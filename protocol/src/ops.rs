//! Protokoll-Aktionen als fertige Transaktionen. Jede Funktion liefert die
//! gebaute Tx und den neuen Stand der betroffenen Covenant-UTXOs; die
//! aufrufende Seite (Simulator oder Netz) wendet beides erst an, wenn die Tx
//! angenommen wurde.

use crate::contracts::*;
use crate::math;
use crate::txb::{Built, Draft, In, Signer, Unlock, build, build_ext, build_with_payload, outpoint};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::hashing::covenant_id::covenant_id;
use kaspa_consensus_core::tx::{CovenantBinding, ScriptPublicKey, TransactionOutpoint, TransactionOutput, UtxoEntry};
use kaspa_txscript::opcodes::codes::OpCheckSig;
use kaspa_txscript::script_builder::ScriptBuilder;
use secp256k1::{Keypair, Message};
use serde::{Deserialize, Serialize};
use silverscript_abi::ArtifactValue;

/// KAS in jedem GHOST-Token-UTXO. Klein hält die gebundenen KAS gering,
/// groß hält die Speichermasse neuer Ausgänge gering (KIP-9). Gemessen in
/// tests/e2e_tests.rs.
pub const TOKEN_VALUE: u64 = 100_000_000; // 1 KAS (0,3 KAS ergab 307 000 g Speichermasse je Vault-Tx, e2e 28.09.2026)
/// Dauerhafte Einzel-UTXOs bekommen mehr KAS: jede neu erzeugte Ausgabe
/// kostet etwa 4·10^12/Wert Gramm Speichermasse (gemessen: 1 KAS → 40 000 g,
/// Blocklimit 500 000 g). Orakel und Minter-Zweig werden bei jeder
/// Vault-Aktion neu erzeugt.
pub const ORACLE_VALUE: u64 = 1_000_000_000; // 10 KAS
pub const FACTORY_VALUE: u64 = 1_000_000_000; // 10 KAS
pub const ROOT_VALUE: u64 = 1_000_000_000; // 10 KAS, GHOST-Wurzel-Minter
pub const BRANCH_VALUE: u64 = 300_000_000; // 3 KAS je Vault-Minter-Zweig
/// Register-Haupt-UTXO (Version 4); fährt bei jedem Preis-Update mit
pub const REGISTER_VALUE: u64 = 1_000_000_000; // 10 KAS
/// KAS in einem Austausch-Ticket (bekommt zurück, wer es aktiviert oder aufräumt)
pub const TICKET_VALUE: u64 = 100_000_000; // 1 KAS

/// KAS in den dauerhaften Covenant-UTXOs eines Deployments. Sie bleiben für
/// immer gebunden; jede Fortsetzung trägt denselben Betrag weiter.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CovValues {
    pub register: u64,
    pub oracle: u64,
    pub factory: u64,
    pub root: u64,
}

impl Default for CovValues {
    fn default() -> Self {
        Self { register: REGISTER_VALUE, oracle: ORACLE_VALUE, factory: FACTORY_VALUE, root: ROOT_VALUE }
    }
}

impl CovValues {
    /// Je 1 KAS (Speichermasse einer 1-KAS-Ausgabe ≈ 40 000 g, Blockgrenze 500 000 g).
    /// So legt ghostctl v4 an, Probe wie echtes Deployment (Mainnet-Probe 05.10.2026)
    pub fn small() -> Self {
        Self { register: 100_000_000, oracle: 100_000_000, factory: 100_000_000, root: 100_000_000 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tracked<S> {
    pub outpoint: TransactionOutpoint,
    pub value: u64,
    pub cov: Hash,
    pub state: S,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultRec {
    #[serde(with = "hex_bytes")]
    pub owner: Vec<u8>,
    /// Zustand: geprägte GHOST, Zins, Index der letzten Abrechnung (Version 3)
    pub vault: Tracked<VaultState>,
    pub branch: Tracked<GhostTok>,
    /// von Dritten verändert, Zustand unbekannt (store::resync)
    #[serde(default)]
    pub stale: bool,
}

/// Angekündigter Austausch (offenes Ticket), bis er aktiviert oder abgesagt ist
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rotation {
    pub ticket: Tracked<RegisterState>,
    /// der angekündigte Satz (das Ticket trägt nur seinen Hash)
    pub set: SignerSet,
    /// der mit angekündigte Notfallsatz
    #[serde(default)]
    pub fallback: Option<SignerSet>,
    pub emergency: bool,
    /// DAA, ab dem das Ticket aktiviert werden darf (Aufnahme + Wartezeit, geschätzt)
    pub ready_daa: u64,
}

/// Alles, was man zum Weiterarbeiten braucht (wird als JSON gespeichert).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Deployment {
    pub network: String,
    /// Unterzeichner-Register (Version 4)
    pub register_params: RegisterParams,
    pub register: Tracked<RegisterState>,
    /// aktueller Unterzeichner-Satz (Schlüssel öffentlich; im Register steht sein Hash)
    pub signer_set: SignerSet,
    /// Notfallsatz, falls einer festgelegt ist
    #[serde(default)]
    pub fallback_set: Option<SignerSet>,
    #[serde(default)]
    pub rotation: Option<Rotation>,
    /// abgesagte, ersetzte oder verfallene Tickets: ihre KAS holt `clear_ticket`
    /// zurück (Audit 14 N-1: vorher fielen sie aus der Zustandsdatei)
    #[serde(default)]
    pub old_tickets: Vec<Tracked<RegisterState>>,
    /// Register-nonce, bei der eine Änderung von außen erkannt wurde (fremde
    /// Ankündigung, Absage oder Notfall-Ankündigung, Audit 14 M-3/M5)
    #[serde(default)]
    pub foreign_change: Option<i64>,
    /// Der Unterzeichner-Satz im Register ist ein anderer als in dieser Datei:
    /// Preis-Updates gehen erst wieder mit der Datei des aktivierenden Rechners
    #[serde(default)]
    pub signers_unknown: bool,
    pub oracle_params: OracleParams,
    pub oracle: Tracked<OracleState>,
    pub factory_params: FactoryParams,
    pub factory: Tracked<FactoryState>,
    pub ghost_root: Option<Tracked<GhostTok>>,
    pub vault_params: Option<VaultParams>,
    pub vaults: Vec<VaultRec>,
    /// bekannte GHOST-UTXOs von Nutzern
    pub tokens: Vec<Tracked<GhostTok>>,
    /// Tauschpool KAS/GHOST, falls angelegt (contracts/ghost_pool.sil)
    #[serde(default)]
    pub pool: Option<crate::pool::PoolRec>,
    /// angelegter, noch nicht initialisierter Pool (pool-open Schritt 1)
    #[serde(default)]
    pub pool_pending: Option<crate::pool::PoolPending>,
    /// bekannte Anteils-Token (Pool-Anteile) von Nutzern
    #[serde(default)]
    pub lp_tokens: Vec<Tracked<GhostTok>>,
    /// Pool-Reserve beim letzten Abgleich nicht bestimmbar (wird nicht gespeichert)
    #[serde(skip)]
    pub pool_unresolved: Option<String>,
}

/// KAS eines P2PK-Kontos zum Bezahlen von Gebühren und Einlagen. Signiert
/// von einem Schlüssel oder einer Browser-Wallet (txb::Signer).
pub struct Funds {
    pub key: Signer,
    pub utxos: Vec<(TransactionOutpoint, UtxoEntry)>,
}

pub fn xonly(k: &Keypair) -> Vec<u8> {
    k.x_only_public_key().0.serialize().to_vec()
}

pub fn p2pk_spk(xonly: &[u8]) -> ScriptPublicKey {
    let script = ScriptBuilder::new().add_data(xonly).unwrap().add_op(OpCheckSig).unwrap().drain();
    ScriptPublicKey::new(0, script.into())
}

impl Funds {
    pub fn new(key: impl Into<Signer>, utxos: Vec<(TransactionOutpoint, UtxoEntry)>) -> Self {
        Funds { key: key.into(), utxos }
    }
    pub(crate) fn inputs(&self) -> Vec<In> {
        self.utxos
            .iter()
            .map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: self.key } })
            .collect()
    }
    pub(crate) fn change(&self) -> ScriptPublicKey {
        p2pk_spk(&self.key.xonly())
    }
}

pub(crate) fn cov_out(art: &Artifact, value: u64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput { value, script_public_key: spk(art), covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }) }
}

pub(crate) fn cov_in(t_out: &TransactionOutpoint, value: u64, art: &Artifact, cov: Hash) -> UtxoEntry {
    let _ = t_out;
    UtxoEntry::new(value, spk(art), 0, false, Some(cov))
}

/// Genesis-ID einer einzelnen Ausgabe, autorisiert vom Input mit `auth_outpoint`.
fn genesis_id(auth_outpoint: TransactionOutpoint, index: u32, out: &TransactionOutput) -> Hash {
    covenant_id(auth_outpoint, std::iter::once((index, out)))
}

pub(crate) fn genesis_output(art: &Artifact, value: u64, auth: u16, auth_outpoint: TransactionOutpoint, index: u32) -> (TransactionOutput, Hash) {
    let mut o = TransactionOutput { value, script_public_key: spk(art), covenant: None };
    let id = genesis_id(auth_outpoint, index, &o);
    o.covenant = Some(CovenantBinding { authorizing_input: auth, covenant_id: id });
    (o, id)
}

pub(crate) fn track<S>(b: &Built, index: u32, cov: Hash, state: S) -> Tracked<S> {
    Tracked { outpoint: outpoint(&b.tx, index), value: b.tx.outputs[index as usize].value, cov, state }
}

// ------------------------------------------------------------------ Genesis ----

/// Register-Genesis (Version 4, Schritt 1). Die Tx hat genau EINEN Ausgang mit
/// der neuen Register-ID: Das Orakel prüft nur die ID, nicht das Template, ein
/// zweiter Genesis-Ausgang mit fremdem Skript wäre eine Hintertür
/// (docs/v4-entwurf.md 2, Schritt 1). `check_register_genesis` prüft das nach.
pub fn deploy_register(p: &RegisterParams, s: RegisterState, value: u64, fund: &Funds, net: &Params) -> Result<(Built, Tracked<RegisterState>), String> {
    let first = fund.utxos.first().ok_or("keine Funding-UTXO")?.0;
    let (out, id) = genesis_output(&register(p, &s), value, 0, first, 0);
    let b = build(Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: fund.change(), lock_time: 0 }, net)?;
    check_register_genesis(&b.tx, &id)?;
    let t = track(&b, 0, id, s);
    Ok((b, t))
}

/// Genau ein Ausgang trägt die Register-ID (öffentlich nachprüfbar an der Genesis-Tx)
pub fn check_register_genesis(tx: &kaspa_consensus_core::tx::Transaction, reg_cov: &Hash) -> Result<(), String> {
    let n = tx.outputs.iter().filter(|o| o.covenant.is_some_and(|c| c.covenant_id == *reg_cov)).count();
    if n != 1 {
        return Err(format!("Register-Genesis mit {n} Ausgängen der Register-ID (erlaubt: genau 1)"));
    }
    Ok(())
}

/// Orakel-Genesis (Schritt 2): kennt die Register-ID als Konstante
pub fn deploy_oracle(p: &OracleParams, s: OracleState, value: u64, fund: &Funds, net: &Params) -> Result<(Built, Tracked<OracleState>), String> {
    let first = fund.utxos.first().ok_or("keine Funding-UTXO")?.0;
    let (out, id) = genesis_output(&oracle(p, &s), value, 0, first, 0);
    let b = build(Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: fund.change(), lock_time: 0 }, net)?;
    let t = track(&b, 0, id, s);
    Ok((b, t))
}

/// Register-init (Schritt 3, Deployer-Signatur): Orakel-ID und -Template
/// festlegen; prüft den Startsatz gegen Hash und Grenzen. Danach hat der
/// Deployer keine Rechte mehr.
#[allow(clippy::too_many_arguments)]
pub fn init_register(
    rp: &RegisterParams,
    reg: &Tracked<RegisterState>,
    set: &SignerSet,
    deployer: &Keypair,
    op: &OracleParams,
    oracle_t: &Tracked<OracleState>,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Tracked<RegisterState>), String> {
    set.check_bounds(rp)?;
    if set.hash().as_slice() != reg.state.set_hash.as_slice() {
        return Err("Startsatz passt nicht zum Hash im Register".into());
    }
    let otpl = Template::of(&oracle(op, &oracle_t.state));
    let next = RegisterState {
        oracle_cov: oracle_t.cov.as_bytes().to_vec(),
        oracle_tpl: otpl.hash.clone(),
        oracle_pre: otpl.prefix.len() as i64,
        oracle_suf: otpl.suffix.len() as i64,
        initialized: true,
        ..reg.state.clone()
    };
    let cur = register(rp, &reg.state);
    let mut args = set.args();
    args.extend([
        ArtifactValue::Bytes(oracle_t.cov.as_bytes().to_vec()),
        ArtifactValue::Bytes(otpl.hash),
        ArtifactValue::Int(otpl.prefix.len() as i64),
        ArtifactValue::Int(otpl.suffix.len() as i64),
    ]);
    let mut inputs = vec![In { outpoint: reg.outpoint, entry: cov_in(&reg.outpoint, reg.value, &cur, reg.cov), unlock: Unlock::Entry { art: cur, entry: "init", args, sig_at: Some((8, deployer.into())) } }];
    inputs.extend(fund.inputs());
    let outputs = vec![cov_out(&register(rp, &next), reg.value, 0, reg.cov)];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let t = track(&b, 0, reg.cov, next);
    Ok((b, t))
}

pub fn deploy_factory(p: &FactoryParams, value: u64, fund: &Funds, net: &Params) -> Result<(Built, Tracked<FactoryState>), String> {
    let first = fund.utxos.first().ok_or("keine Funding-UTXO")?.0;
    let s = FactoryState::uninitialized();
    let (out, id) = genesis_output(&factory(p, &s), value, 0, first, 0);
    let b = build(Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: fund.change(), lock_time: 0 }, net)?;
    let t = track(&b, 0, id, s);
    Ok((b, t))
}

/// Einmalige Initialisierung: GHOST-Genesis + Vault-Template festlegen.
/// `interest_spk`: Zinsziel des Vaults (spk_bytes, z. B. P2PK der eigenen Adresse).
#[allow(clippy::too_many_arguments)]
pub fn init_factory(
    oracle_params: &OracleParams,
    oracle_t: &Tracked<OracleState>,
    fp: &FactoryParams,
    f: &Tracked<FactoryState>,
    deployer: &Keypair,
    // (Mindestquote, Liquidationsschwelle, Bonus) in bps und Höchstschuld je Vault
    ratios: (i64, i64, i64, i64),
    interest_spk: Vec<u8>,
    root_value: u64,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Tracked<FactoryState>, Tracked<GhostTok>, VaultParams), String> {
    spk_from_bytes(&interest_spk)?;
    let root = GhostTok::minter_of(&f.cov);
    // Ausgang 1 = GHOST-Genesis, autorisiert von Input 0 (Factory)
    let (ghost_out, gid) = genesis_output(&root.artifact(), root_value, 0, f.outpoint, 1);
    let oracle_tpl = Template::of(&oracle(oracle_params, &oracle_t.state));
    let vp = VaultParams {
        oracle_cov: oracle_t.cov,
        oracle_tpl,
        ghost_cov: gid,
        ghost_tpl: fp.ghost_tpl.clone(),
        mcr_bps: ratios.0,
        liq_bps: ratios.1,
        bonus_bps: ratios.2,
        max_debt: ratios.3,
        interest_spk,
    };
    let vault_hash = Template::of(&vault(&vp, &[0; 32], &VaultState::default())).hash;
    let next = FactoryState { ghost_cov: gid, vault_hash: vault_hash.clone(), initialized: true };
    let cur = factory(fp, &f.state);
    let mut inputs = vec![In {
        outpoint: f.outpoint,
        entry: cov_in(&f.outpoint, f.value, &cur, f.cov),
        unlock: Unlock::Entry {
            art: cur,
            entry: "init",
            args: vec![
                ArtifactValue::Bytes(vault_hash),
                ArtifactValue::Int(1),
                ArtifactValue::Bytes(fp.ghost_tpl.prefix.clone()),
                ArtifactValue::Bytes(fp.ghost_tpl.suffix.clone()),
            ],
            sig_at: Some((4, deployer.into())),
        },
    }];
    inputs.extend(fund.inputs());
    let outputs = vec![cov_out(&factory(fp, &next), f.value, 0, f.cov), ghost_out];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let nf = track(&b, 0, f.cov, next);
    let nr = track(&b, 1, gid, root);
    Ok((b, nf, nr, vp))
}

// -------------------------------------------------------------- openVault ----

pub fn open_vault(dep: &Deployment, owner: &[u8], collateral: u64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let vp = dep.vault_params.as_ref().ok_or("Factory nicht initialisiert")?;
    let root = dep.ghost_root.as_ref().ok_or("kein GHOST-Wurzel-Minter")?;
    let f = &dep.factory;
    let fart = factory(&dep.factory_params, &f.state);
    let rart = root.state.artifact();
    let zero = VaultState::default();
    let vtpl = Template::of(&vault(vp, owner, &zero));
    // Ausgang 1 = Vault-Genesis, autorisiert von Input 0 (Factory)
    let (vault_out, vid) = genesis_output(&vault(vp, owner, &zero), collateral, 0, f.outpoint, 1);
    let branch = GhostTok::minter_of(&vid);
    let states = vec![root.state.arg(), branch.arg()];
    let mut inputs = vec![
        In {
            outpoint: f.outpoint,
            entry: cov_in(&f.outpoint, f.value, &fart, f.cov),
            unlock: Unlock::Entry {
                art: fart.clone(),
                entry: "openVault",
                args: vec![
                    ArtifactValue::Bytes(owner.to_vec()),
                    ArtifactValue::Int(1),
                    ArtifactValue::Bytes(vtpl.prefix),
                    ArtifactValue::Bytes(vtpl.suffix),
                    ArtifactValue::Array(states.clone()),
                ],
                sig_at: None,
            },
        },
        In { outpoint: root.outpoint, entry: cov_in(&root.outpoint, root.value, &rart, root.cov), unlock: Unlock::Leader { art: rart.clone(), new_states: states, signer: None } },
    ];
    inputs.extend(fund.inputs());
    let outputs = vec![
        cov_out(&fart, f.value, 0, f.cov),
        vault_out,
        cov_out(&rart, root.value, 1, root.cov),
        cov_out(&branch.artifact(), BRANCH_VALUE, 1, root.cov),
    ];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.factory = track(&b, 0, f.cov, f.state.clone());
    d.ghost_root = Some(track(&b, 2, root.cov, root.state.clone()));
    d.vaults.push(VaultRec { owner: owner.to_vec(), vault: track(&b, 1, vid, zero), branch: track(&b, 3, root.cov, branch), stale: false });
    Ok((b, d))
}

// ------------------------------------------------------------- Orakel-Teil ----

fn oracle_read_input(dep: &Deployment) -> (In, TransactionOutput) {
    let o = &dep.oracle;
    let art = oracle(&dep.oracle_params, &o.state);
    (
        In { outpoint: o.outpoint, entry: cov_in(&o.outpoint, o.value, &art, o.cov), unlock: Unlock::Entry { art: art.clone(), entry: "read", args: vec![], sig_at: None } },
        cov_out(&art, o.value, 0, o.cov), // auth wird vom Aufrufer gesetzt
    )
}

fn set_auth(mut o: TransactionOutput, auth: u16) -> TransactionOutput {
    if let Some(c) = o.covenant.as_mut() {
        c.authorizing_input = auth;
    }
    o
}

fn register_in(dep: &Deployment, entry: &'static str, args: Vec<ArtifactValue>) -> In {
    let r = &dep.register;
    let art = register(&dep.register_params, &r.state);
    In { outpoint: r.outpoint, entry: cov_in(&r.outpoint, r.value, &art, r.cov), unlock: Unlock::Entry { art, entry, args, sig_at: None } }
}

fn register_out(dep: &Deployment, s: &RegisterState, auth: u16) -> TransactionOutput {
    cov_out(&register(&dep.register_params, s), dep.register.value, auth, dep.register.cov)
}

/// sigs, idx: Signaturen der Schlüssel `signers` (Index im Satz, Schlüssel) über
/// `digest`, Indizes aufsteigend; jeder Schlüssel muss zum Satz passen
fn quorum(set: &SignerSet, signers: &[(usize, Keypair)], need: i64, digest: [u8; 32]) -> Result<Vec<ArtifactValue>, String> {
    let mut sorted = signers.to_vec();
    sorted.sort_by_key(|(i, _)| *i);
    sorted.dedup_by_key(|(i, _)| *i);
    if (sorted.len() as i64) < need {
        return Err(format!("{need} Unterschrift(en) nötig, {} vorhanden", sorted.len()));
    }
    sorted.truncate(need as usize);
    for (i, k) in &sorted {
        if set.keys.get(*i).map(|x| x.as_slice()) != Some(xonly(k).as_slice()) {
            return Err(format!("Schlüssel {i} gehört nicht zum Unterzeichner-Satz"));
        }
    }
    let sigs = sorted.iter().map(|(_, k)| ArtifactValue::Bytes(k.sign_schnorr(Message::from_digest(digest)).as_ref().to_vec())).collect();
    let idx = sorted.iter().map(|(i, _)| ArtifactValue::Int(*i as i64)).collect();
    Ok(vec![ArtifactValue::Array(sigs), ArtifactValue::Array(idx)])
}

/// Signierer eines Satzes aus vorhandenen Schlüsseln (Index im Satz, Schlüssel)
pub fn signers_of(set: &SignerSet, keys: &[Keypair]) -> Vec<(usize, Keypair)> {
    keys.iter().filter_map(|k| set.keys.iter().position(|x| *x == xonly(k)).map(|i| (i, *k))).collect()
}

/// Neuer Preis/Zins (Version 4): t Unterzeichner des aktuellen Satzes
/// signieren, das Register (attestPrice) prüft das Quorum, das Orakel (update)
/// die Grenzen. `new_daa` wird auch Locktime (tx.daa ≥ newOracleDaa).
pub fn oracle_update(
    dep: &Deployment,
    signers: &[(usize, Keypair)],
    kas_usd: i64,
    rate: i64,
    new_daa: u64,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Deployment), String> {
    let o = &dep.oracle;
    let r = &dep.register;
    if !r.state.initialized || r.state.ticket {
        return Err("Register nicht initialisiert".into());
    }
    check_update(&dep.oracle_params, &o.state, kas_usd, new_daa as i64, rate)?;
    let next = oracle_next_state(&o.state, kas_usd, new_daa as i64, rate).ok_or("Index-Überlauf")?;
    let digest = oracle_digest(&o.cov, kas_usd, next.oracle_daa, next.seq, rate);
    let mut args = vec![
        ArtifactValue::Int(next.kas_usd),
        ArtifactValue::Int(next.oracle_daa),
        ArtifactValue::Int(next.seq),
        ArtifactValue::Int(next.stable_rate),
        ArtifactValue::Int(next.stable_index),
        ArtifactValue::Int(next.last_rate_daa),
    ];
    args.extend(dep.signer_set.args());
    args.extend(quorum(&dep.signer_set, signers, dep.signer_set.t, digest)?);
    let reg_next = RegisterState { nonce: if r.state.emerg { r.state.nonce + 1 } else { r.state.nonce }, emerg: false, last_daa: next.oracle_daa, ..r.state.clone() };
    let cur = oracle(&dep.oracle_params, &o.state);
    let mut inputs = vec![
        register_in(dep, "attestPrice", args),
        In {
            outpoint: o.outpoint,
            entry: cov_in(&o.outpoint, o.value, &cur, o.cov),
            unlock: Unlock::Entry {
                art: cur,
                entry: "update",
                args: vec![ArtifactValue::Int(kas_usd), ArtifactValue::Int(new_daa as i64), ArtifactValue::Int(rate)],
                sig_at: None,
            },
        },
    ];
    inputs.extend(fund.inputs());
    let outputs = vec![register_out(dep, &reg_next, 0), cov_out(&oracle(&dep.oracle_params, &next), o.value, 1, o.cov)];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: new_daa }, net)?;
    let mut d = dep.clone();
    d.register = track(&b, 0, r.cov, reg_next);
    d.oracle = track(&b, 1, o.cov, next);
    if r.state.emerg {
        // ein Preis-Update entwertet ein offenes Notfall-Ticket
        retire_rotation(&mut d);
    }
    Ok((b, d))
}

/// Grenzen von price_oracle_v4.sil update() vorab, mit lesbarer Meldung
pub fn check_update(p: &OracleParams, cur: &OracleState, kas_usd: i64, daa: i64, rate: i64) -> Result<(), String> {
    if !(1_000..=90_000_000_000).contains(&kas_usd) {
        return Err(format!("Preis {:.8} USD außerhalb der Vertragsgrenzen (0,00001–900 USD)", kas_usd as f64 / 1e8));
    }
    if kas_usd * 2 < cur.kas_usd || kas_usd > cur.kas_usd * 2 {
        return Err(format!("Preissprung {:.6} → {:.6} USD ist größer als ×2/÷2 je Update", cur.kas_usd as f64 / 1e8, kas_usd as f64 / 1e8));
    }
    if daa < cur.oracle_daa + 600 {
        return Err("letztes Update ist zu frisch (Vertrag verlangt ≈ 1 min Abstand)".into());
    }
    if !(0..=p.max_rate).contains(&rate) {
        return Err(format!("Zins außerhalb 0 … {:.2} % p. a.", p.max_rate as f64 * 315_360_000.0 / 1e16));
    }
    if rate != cur.stable_rate {
        if daa < cur.last_rate_daa + p.rate_gap_daa {
            return Err(format!("Zins darf sich erst ab DAA {} wieder ändern (höchstens einmal je {} DAA)", cur.last_rate_daa + p.rate_gap_daa, p.rate_gap_daa));
        }
        if (rate - cur.stable_rate).abs() > p.rate_step {
            return Err(format!("Zinsschritt zu groß: höchstens {:.2} Prozentpunkte je Änderung", p.rate_step as f64 * 315_360_000.0 / 1e16));
        }
    }
    Ok(())
}

/// Friert ein veraltetes Orakel ein (jeder darf, ab oracleDaa + freezeAfterDaa).
/// `daa` = Locktime, muss unter dem aktuellen DAA der Kette liegen.
pub fn oracle_freeze(dep: &Deployment, daa: u64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let o = &dep.oracle;
    let due = o.state.oracle_daa + dep.oracle_params.freeze_after_daa;
    if (daa as i64) < due {
        return Err(format!("Einfrieren erst ab DAA {due} (Preis von DAA {}, Frist {} DAA)", o.state.oracle_daa, dep.oracle_params.freeze_after_daa));
    }
    let next = OracleState { frozen: true, ..o.state };
    let cur = oracle(&dep.oracle_params, &o.state);
    let mut inputs = vec![In { outpoint: o.outpoint, entry: cov_in(&o.outpoint, o.value, &cur, o.cov), unlock: Unlock::Entry { art: cur, entry: "freeze", args: vec![], sig_at: None } }];
    inputs.extend(fund.inputs());
    let outputs = vec![cov_out(&oracle(&dep.oracle_params, &next), o.value, 0, o.cov)];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: daa }, net)?;
    let mut d = dep.clone();
    d.oracle = track(&b, 0, o.cov, next);
    Ok((b, d))
}

/// Kündigt einen neuen Unterzeichner-Satz an (Ticket). Regulär signieren tRot
/// des aktuellen Satzes; `emergency` = Notfallweg, signiert von tRot des
/// Notfallsatzes, erst ab lastDaa + emergAfterDaa (`daa` = Locktime).
/// Der Payload der Tx ist die Ankündigung: der Austausch ist öffentlich.
#[allow(clippy::too_many_arguments)]
pub fn propose(
    dep: &Deployment,
    signers: &[(usize, Keypair)],
    new_set: &SignerSet,
    new_fb: Option<&SignerSet>,
    emergency: bool,
    daa: u64,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Deployment), String> {
    let r = &dep.register;
    let rp = &dep.register_params;
    new_set.check_bounds(rp)?;
    if let Some(fb) = new_fb {
        fb.check_bounds(rp)?;
    }
    let auth = if emergency {
        let due = r.state.last_daa + rp.emerg_after_daa;
        if (daa as i64) < due {
            return Err(format!("Notfallweg erst ab DAA {due} (letzter Preis bei DAA {})", r.state.last_daa));
        }
        dep.fallback_set.as_ref().ok_or("kein Notfallsatz bekannt")?
    } else {
        &dep.signer_set
    };
    if emergency && auth.hash().as_slice() != r.state.fb_hash.as_slice() {
        return Err("Notfallsatz passt nicht zum Register".into());
    }
    let set_h = new_set.hash();
    let fb_h = new_fb.map(|f| f.hash()).unwrap_or([0; 32]);
    let pay_to: [u8; 32] = r.state.pay_to.as_slice().try_into().map_err(|_| "Zinsziel im Register ungültig")?;
    let ann = announcement(&set_h, &fb_h, r.state.pay_kind, &pay_to);
    let nonce = r.state.nonce + 1;
    let digest = rotate_digest(&r.cov, emergency, nonce, &ann);
    let mut args = new_set.args();
    args.extend([ArtifactValue::Bytes(fb_h.to_vec()), ArtifactValue::Byte(r.state.pay_kind), ArtifactValue::Bytes(pay_to.to_vec()), ArtifactValue::Bool(emergency)]);
    args.extend(auth.args());
    args.extend(quorum(auth, signers, auth.t_rot, digest)?);
    let main_next = RegisterState { nonce, emerg: emergency, ..r.state.clone() };
    let ticket = RegisterState { ticket: true, set_hash: set_h.to_vec(), fb_hash: fb_h.to_vec(), nonce, emerg: emergency, ..r.state.clone() };
    let mut inputs = vec![register_in(dep, "propose", args)];
    inputs.extend(fund.inputs());
    let outputs = vec![register_out(dep, &main_next, 0), cov_out(&register(rp, &ticket), TICKET_VALUE, 0, r.cov)];
    let b = build_ext(Draft { inputs, outputs, change_spk: fund.change(), lock_time: if emergency { daa } else { 0 } }, &ann, &[], net)?;
    let mut d = dep.clone();
    d.register = track(&b, 0, r.cov, main_next);
    retire_rotation(&mut d);
    d.foreign_change = None;
    let delay = if emergency { rp.emerg_delay_daa } else { rp.rot_delay_daa };
    // `daa` liegt etwas vor der Aufnahme des Tickets; 600 DAA (1 min) Puffer (Audit 14 N9)
    d.rotation = Some(Rotation { ticket: track(&b, 1, r.cov, ticket), set: new_set.clone(), fallback: new_fb.cloned(), emergency, ready_daa: daa + delay as u64 + 600 });
    Ok((b, d))
}

/// Sagt eine Ankündigung ab: t des aktuellen Satzes, nonce + 1 entwertet jedes Ticket
pub fn cancel_rotation(dep: &Deployment, signers: &[(usize, Keypair)], fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let r = &dep.register;
    let nonce = r.state.nonce + 1;
    let mut args = dep.signer_set.args();
    args.extend(quorum(&dep.signer_set, signers, dep.signer_set.t, cancel_digest(&r.cov, nonce))?);
    let next = RegisterState { nonce, emerg: false, ..r.state.clone() };
    let mut inputs = vec![register_in(dep, "cancel", args)];
    inputs.extend(fund.inputs());
    let b = build(Draft { inputs, outputs: vec![register_out(dep, &next, 0)], change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.register = track(&b, 0, r.cov, next);
    // das Ticket bleibt als UTXO liegen (abgelaufen); `clear_ticket` holt seine KAS zurück
    retire_rotation(&mut d);
    d.foreign_change = None;
    Ok((b, d))
}

/// Setzt eine fällige Ankündigung in Kraft (jeder darf): Haupt-UTXO
/// activate(1), Ticket settle(0) mit Sequenz = Wartezeit (this.ageDaa).
pub fn activate_rotation(dep: &Deployment, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let r = &dep.register;
    let rot = dep.rotation.as_ref().ok_or("keine Ankündigung offen (abgesagt, ersetzt oder schon aktiviert?)")?;
    let tk = &rot.ticket;
    if tk.state.nonce != r.state.nonce {
        return Err("Die Ankündigung ist abgesagt oder überholt (nonce passt nicht)".into());
    }
    if rot.set.hash().as_slice() != tk.state.set_hash.as_slice() || rot.fallback.as_ref().map(|f| f.hash().to_vec()).unwrap_or(vec![0; 32]) != tk.state.fb_hash {
        return Err("Zustandsdatei passt nicht zum Ticket (Satz oder Notfallsatz)".into());
    }
    let delay = if tk.state.emerg { dep.register_params.emerg_delay_daa } else { dep.register_params.rot_delay_daa };
    let next = RegisterState {
        set_hash: tk.state.set_hash.clone(),
        fb_hash: tk.state.fb_hash.clone(),
        pay_kind: tk.state.pay_kind,
        pay_to: tk.state.pay_to.clone(),
        nonce: r.state.nonce + 1,
        emerg: false,
        ..r.state.clone()
    };
    let tart = register(&dep.register_params, &tk.state);
    let mut inputs = vec![
        register_in(dep, "activate", vec![ArtifactValue::Int(1)]),
        In { outpoint: tk.outpoint, entry: cov_in(&tk.outpoint, tk.value, &tart, tk.cov), unlock: Unlock::Entry { art: tart, entry: "settle", args: vec![ArtifactValue::Int(0)], sig_at: None } },
    ];
    inputs.extend(fund.inputs());
    let b = build_ext(Draft { inputs, outputs: vec![register_out(dep, &next, 0)], change_spk: fund.change(), lock_time: 0 }, &[], &[0, delay as u64], net)?;
    let mut d = dep.clone();
    d.register = track(&b, 0, r.cov, next);
    d.signer_set = rot.set.clone();
    d.fallback_set = rot.fallback.clone();
    d.rotation = None;
    d.foreign_change = None;
    d.signers_unknown = false;
    Ok((b, d))
}

/// Räumt ein abgesagtes oder überholtes Ticket auf (jeder darf): Haupt-UTXO
/// witness, Ticket settle(0); die KAS des Tickets gehen an `fund`.
pub fn clear_ticket(dep: &Deployment, ticket: &Tracked<RegisterState>, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let r = &dep.register;
    if ticket.state.nonce == r.state.nonce {
        return Err("Das Ticket ist noch gültig – erst absagen oder aktivieren".into());
    }
    let tart = register(&dep.register_params, &ticket.state);
    let mut inputs = vec![
        register_in(dep, "witness", vec![]),
        In { outpoint: ticket.outpoint, entry: cov_in(&ticket.outpoint, ticket.value, &tart, ticket.cov), unlock: Unlock::Entry { art: tart, entry: "settle", args: vec![ArtifactValue::Int(0)], sig_at: None } },
    ];
    inputs.extend(fund.inputs());
    let b = build(Draft { inputs, outputs: vec![register_out(dep, &r.state, 0)], change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.register = track(&b, 0, r.cov, r.state.clone());
    if d.rotation.as_ref().is_some_and(|x| x.ticket.outpoint == ticket.outpoint) {
        d.rotation = None;
    }
    d.old_tickets.retain(|t| t.outpoint != ticket.outpoint);
    Ok((b, d))
}

/// Offene Ankündigung in die Liste der aufzuräumenden Tickets verschieben
fn retire_rotation(d: &mut Deployment) {
    if let Some(r) = d.rotation.take() {
        d.old_tickets.push(r.ticket);
    }
}

/// Fehler, wenn das Orakel eingefroren ist (Vault v4 sperrt die Aktion)
fn not_frozen(dep: &Deployment, what: &str) -> Result<(), String> {
    if dep.oracle.state.frozen {
        return Err(format!("{what} ist gesperrt: Das Orakel ist eingefroren (kein aktueller Preis). Das nächste Preis-Update taut es auf."));
    }
    Ok(())
}

// ------------------------------------------------------------ Vault-Aktionen ----

fn vault_input(dep: &Deployment, i: usize, entry: &'static str, args: Vec<ArtifactValue>, sig_at: Option<(usize, Signer)>) -> Result<(In, Artifact), String> {
    let vp = dep.vault_params.as_ref().ok_or("keine Vault-Parameter")?;
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let art = vault(vp, &v.owner, &v.vault.state);
    Ok((In { outpoint: v.vault.outpoint, entry: cov_in(&v.vault.outpoint, v.vault.value, &art, v.vault.cov), unlock: Unlock::Entry { art: art.clone(), entry, args, sig_at } }, art))
}

fn vault_cont(dep: &Deployment, i: usize, value: u64, st: &VaultState) -> TransactionOutput {
    let v = &dep.vaults[i];
    cov_out(&vault(dep.vault_params.as_ref().unwrap(), &v.owner, st), value, 0, v.vault.cov)
}

fn branch_leader(dep: &Deployment, i: usize, states: Vec<ArtifactValue>) -> In {
    let br = &dep.vaults[i].branch;
    let art = br.state.artifact();
    In { outpoint: br.outpoint, entry: cov_in(&br.outpoint, br.value, &art, br.cov), unlock: Unlock::Leader { art, new_states: states, signer: None } }
}

/// Prägt `amount` GHOST an `recipient` (x-only-Pubkey). Signiert vom Besitzer.
pub fn mint(dep: &Deployment, i: usize, owner: impl Into<Signer>, amount: i64, recipient: &[u8], fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let owner: Signer = owner.into();
    not_frozen(dep, "Prägen")?;
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    let vp = dep.vault_params.as_ref().unwrap();
    let st = math::settled(&v.vault.state, index);
    let new_state = VaultState { debt: st.debt + amount, ..st };
    if !math::healthy(v.vault.value as i64, new_state.debt + new_state.interest, price, vp.mcr_bps) {
        return Err(format!(
            "Mindestquote verletzt: höchstens {:.8} GHOST prägbar",
            math::max_mint(v.vault.value as i64, &v.vault.state, price, index, vp.mcr_bps) as f64 / 1e8
        ));
    }
    if new_state.debt > vp.max_debt {
        return Err(format!(
            "Obergrenze je Vault ({:.0} GHOST): höchstens noch {:.8} GHOST prägbar",
            vp.max_debt as f64 / 1e8,
            math::cap_room(&v.vault.state, vp.max_debt) as f64 / 1e8
        ));
    }
    let tok = GhostTok::to_pubkey(recipient, amount);
    let states = vec![v.branch.state.arg(), tok.arg()];
    let (vin, _) = vault_input(dep, i, "mint", vec![ArtifactValue::Int(amount), ArtifactValue::Int(1), ArtifactValue::Array(states.clone())], Some((3, owner)))?;
    let (oin, oout) = oracle_read_input(dep);
    let mut inputs = vec![vin, oin, branch_leader(dep, i, states)];
    inputs.extend(fund.inputs());
    let outputs = vec![
        vault_cont(dep, i, v.vault.value, &new_state),
        set_auth(oout, 1),
        cov_out(&v.branch.state.artifact(), v.branch.value, 2, v.branch.cov),
        cov_out(&tok.artifact(), TOKEN_VALUE, 2, v.branch.cov),
    ];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.vaults[i].vault = track(&b, 0, v.vault.cov, new_state);
    d.oracle = track(&b, 1, dep.oracle.cov, dep.oracle.state);
    d.vaults[i].branch = track(&b, 2, v.branch.cov, v.branch.state.clone());
    d.tokens.push(track(&b, 3, v.branch.cov, tok));
    Ok((b, d))
}

/// Verbrennt GHOST aus den Token-UTXOs `token_idx` (alle gehören `payer`).
/// `burn` = verbrannte Menge; Rest geht als Wechselgeld-Token an den Zahler.
/// repay/liquidate/redeem; `vault_value` = KAS, die im Vault bleiben (None = er endet).
#[allow(clippy::too_many_arguments)]
fn burn_op(
    dep: &Deployment,
    i: usize,
    entry: &'static str,
    payer: Signer,
    token_idx: &[usize],
    burn: i64,
    new_state: VaultState,
    vault_value: Option<u64>,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Deployment), String> {
    if token_idx.is_empty() || token_idx.len() > (GHOST_MAX_INS - 1) as usize {
        return Err(format!("1 bis {} Token-UTXOs erlaubt", GHOST_MAX_INS - 1));
    }
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let toks: Vec<&Tracked<GhostTok>> = token_idx.iter().map(|&t| dep.tokens.get(t).ok_or("Token unbekannt")).collect::<Result<_, _>>()?;
    let have: i64 = toks.iter().map(|t| t.state.amount).sum();
    let change = have - burn;
    if change < 0 {
        return Err(format!("zu wenig GHOST: {have} vorhanden, {burn} nötig"));
    }
    let mut states = vec![v.branch.state.arg()];
    let change_tok = GhostTok::to_pubkey(&payer.xonly(), change);
    if change > 0 {
        states.push(change_tok.arg());
    }
    let args = if entry == "liquidate" || entry == "redeem" {
        vec![ArtifactValue::Int(1), ArtifactValue::Int(burn), ArtifactValue::Array(states.clone())]
    } else {
        vec![ArtifactValue::Int(1), ArtifactValue::Array(states.clone())]
    };
    // repay nur durch den Besitzer (v2.1): Signatur hinter oracleIdx, outStates
    let sig = if entry == "repay" { Some((2, payer)) } else { None };
    let (vin, _) = vault_input(dep, i, entry, args, sig)?;
    let (oin, oout) = oracle_read_input(dep);
    let mut inputs = vec![vin, oin, branch_leader(dep, i, states)];
    for t in &toks {
        let art = t.state.artifact();
        inputs.push(In { outpoint: t.outpoint, entry: cov_in(&t.outpoint, t.value, &art, t.cov), unlock: Unlock::Delegate { art, signer: payer } });
    }
    inputs.extend(fund.inputs());
    let mut outputs = vec![];
    let has_vault = vault_value.is_some();
    if let Some(val) = vault_value {
        outputs.push(vault_cont(dep, i, val, &new_state));
    }
    let o_idx = outputs.len() as u32;
    outputs.push(set_auth(oout, 1));
    let b_idx = outputs.len() as u32;
    outputs.push(cov_out(&v.branch.state.artifact(), v.branch.value, 2, v.branch.cov));
    if change > 0 {
        outputs.push(cov_out(&change_tok.artifact(), TOKEN_VALUE, 2, v.branch.cov));
    }
    let c_idx = b_idx + 1;
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.oracle = track(&b, o_idx, dep.oracle.cov, dep.oracle.state);
    d.vaults[i].branch = track(&b, b_idx, v.branch.cov, v.branch.state.clone());
    if has_vault {
        d.vaults[i].vault = track(&b, 0, v.vault.cov, new_state);
    }
    let mut spent: Vec<usize> = token_idx.to_vec();
    spent.sort_unstable_by(|a, b| b.cmp(a));
    for t in spent {
        d.tokens.remove(t);
    }
    if change > 0 {
        d.tokens.push(track(&b, c_idx, v.branch.cov, change_tok));
    }
    if !has_vault {
        d.vaults.remove(i);
    }
    Ok((b, d))
}

/// Tilgt `amount` GHOST-Einheiten (höchstens die Schuld). Der Zins bleibt
/// stehen und wird beim Schließen in KAS bezahlt. Nur der Besitzer (v2.1).
pub fn repay(dep: &Deployment, i: usize, payer: impl Into<Signer>, token_idx: &[usize], amount: i64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let burn = amount.min(v.vault.state.debt);
    if burn <= 0 {
        return Err("Keine GHOST-Schuld zu tilgen".into());
    }
    let st = math::settled(&v.vault.state, dep.oracle.state.stable_index);
    burn_op(dep, i, "repay", payer.into(), token_idx, burn, VaultState { debt: st.debt - burn, ..st }, Some(v.vault.value), fund, net)
}

/// Liquidiert `burn` GHOST-Einheiten (≤ Schuld). Reicht die Sicherheit nicht,
/// endet der Vault; Restschuld und Zins sind ausgebucht (stable_vault.sil v3).
pub fn liquidate(dep: &Deployment, i: usize, liquidator: impl Into<Signer>, token_idx: &[usize], burn: i64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    not_frozen(dep, "Liquidieren")?;
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let vp = dep.vault_params.as_ref().unwrap();
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    let s = &v.vault.state;
    if math::healthy(v.vault.value as i64, math::owed(s, index), price, vp.liq_bps) {
        return Err("Vault ist gesund, Liquidation nicht erlaubt".into());
    }
    if burn <= 0 || burn > s.debt {
        return Err(format!("Liquidationsbetrag muss zwischen 1 Einheit und der Schuld ({:.8} GHOST) liegen", s.debt as f64 / 1e8));
    }
    let coll = v.vault.value as i64;
    let (got, after) = math::liquidation(coll, s, burn, price, index, vp.liq_bps, vp.bonus_bps).ok_or_else(|| {
        format!(
            "Der Rest ({:.8} KAS) wäre kleiner als 0,2 KAS – dann muss die ganze Schuld ({:.8} GHOST) verbrannt werden",
            (coll - math::seize(coll, burn, price, vp.bonus_bps)) as f64 / 1e8,
            s.debt as f64 / 1e8
        )
    })?;
    let (state, value) = match after {
        Some(st) => (st, Some((coll - got) as u64)),
        None => (VaultState::default(), None),
    };
    burn_op(dep, i, "liquidate", liquidator.into(), token_idx, burn, state, value, fund, net)
}

/// Rücknahme: `amount` GHOST zurückgeben (verbrannt), dafür KAS im Wert von
/// amount·(1 − 1 %) USD aus dem Vault. Jeder darf, an Vaults ab 150 %.
pub fn redeem(dep: &Deployment, i: usize, redeemer: impl Into<Signer>, token_idx: &[usize], amount: i64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    not_frozen(dep, "Einlösen")?;
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let vp = dep.vault_params.as_ref().unwrap();
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    let coll = v.vault.value as i64;
    let (paid, after) = math::redemption(coll, &v.vault.state, amount, price, index, vp.liq_bps).ok_or_else(|| {
        if amount <= 0 || amount > v.vault.state.debt {
            format!("Rücknahme: 1 Einheit bis zur Schuld des Vaults ({:.8} GHOST)", v.vault.state.debt as f64 / 1e8)
        } else if !math::healthy(coll, math::owed(&v.vault.state, index), price, vp.liq_bps) {
            "Rücknahme nur an Vaults ab der Liquidationsschwelle – dieser ist liquidierbar".to_string()
        } else {
            "Rücknahme zu groß: im Vault müssen mindestens 0,2 KAS bleiben".to_string()
        }
    })?;
    burn_op(dep, i, "redeem", redeemer.into(), token_idx, amount, after, Some((coll - paid) as u64), fund, net)
}

/// Schießt KAS nach; seit Version 2 nur der Besitzer (Audit 1 V-04).
pub fn deposit(dep: &Deployment, i: usize, owner: impl Into<Signer>, add: u64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let (vin, _) = vault_input(dep, i, "deposit", vec![], Some((0, owner.into())))?;
    let mut inputs = vec![vin];
    inputs.extend(fund.inputs());
    let nv = v.vault.value + add;
    let b = build(Draft { inputs, outputs: vec![vault_cont(dep, i, nv, &v.vault.state)], change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.vaults[i].vault = track(&b, 0, v.vault.cov, v.vault.state);
    Ok((b, d))
}

/// Nimmt KAS heraus; `to` bekommt die Differenz. Der Zins wird abgerechnet.
pub fn withdraw(dep: &Deployment, i: usize, owner: impl Into<Signer>, new_coll: u64, to: &ScriptPublicKey, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let vp = dep.vault_params.as_ref().unwrap();
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    let st = math::settled(&v.vault.state, index);
    if st.debt > 0 {
        not_frozen(dep, "Abheben bei offener Schuld")?;
    }
    if !math::healthy(new_coll as i64, st.debt + st.interest, price, vp.mcr_bps) {
        return Err("Nach dem Abheben wäre die Mindestquote unterschritten".into());
    }
    let (vin, _) = vault_input(dep, i, "withdraw", vec![ArtifactValue::Int(new_coll as i64), ArtifactValue::Int(1)], Some((2, owner.into())))?;
    let (oin, oout) = oracle_read_input(dep);
    let mut inputs = vec![vin, oin];
    inputs.extend(fund.inputs());
    let outputs = vec![
        vault_cont(dep, i, new_coll, &st),
        set_auth(oout, 1),
        TransactionOutput { value: v.vault.value - new_coll, script_public_key: to.clone(), covenant: None },
    ];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.vaults[i].vault = track(&b, 0, v.vault.cov, st);
    d.oracle = track(&b, 1, dep.oracle.cov, dep.oracle.state);
    Ok((b, d))
}

/// Zins beim Schließen in sompi (aufgerundet, höchstens die Sicherheit); unter
/// 0,2 KAS erlassen (stable_vault.sil close)
pub fn close_fee(dep: &Deployment, i: usize) -> i64 {
    let v = &dep.vaults[i];
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    let fee = math::interest_fee(v.vault.value as i64, &v.vault.state, price, index);
    if fee >= math::DUST { fee } else { 0 }
}

/// Schließt einen schuldenfreien Vault: der Zins geht in KAS an die Zinskasse,
/// der Rest an `to`.
pub fn close(dep: &Deployment, i: usize, owner: impl Into<Signer>, to: &ScriptPublicKey, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let vp = dep.vault_params.as_ref().unwrap();
    if v.vault.state.debt != 0 {
        return Err(format!("Erst die Schuld tilgen ({:.8} GHOST)", v.vault.state.debt as f64 / 1e8));
    }
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    if math::interest_fee_checked(v.vault.value as i64, &v.vault.state, price, index).is_none() {
        // Rechengrenze des Vertrags (Audit 12 A12-14), nur bei extremem Zins und Tiefstpreis
        return Err("Der Zins ist beim jetzigen KAS-Preis zu groß für die Rechnung des Vertrags (64 Bit); Schließen geht erst bei höherem Preis".into());
    }
    let fee = close_fee(dep, i);
    // Ausgänge: [0] Orakel-Fortsetzung, [1] Zinskasse (wenn Zins ≥ 0,2 KAS; der
    // Vertrag verlangt sie direkt hinter dem Vault-Eingang 0), dann `to`
    let (vin, _) = vault_input(dep, i, "close", vec![ArtifactValue::Int(1)], Some((1, owner.into())))?;
    let (oin, oout) = oracle_read_input(dep);
    let mut inputs = vec![vin, oin];
    inputs.extend(fund.inputs());
    let mut outputs = vec![set_auth(oout, 1)];
    if fee > 0 {
        outputs.push(TransactionOutput { value: fee as u64, script_public_key: spk_from_bytes(&vp.interest_spk)?, covenant: None });
    }
    // frisst der Zins die ganze Sicherheit, gibt es keinen Rest-Ausgang
    if v.vault.value > fee as u64 {
        outputs.push(TransactionOutput { value: v.vault.value - fee as u64, script_public_key: to.clone(), covenant: None });
    }
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.oracle = track(&b, 0, dep.oracle.cov, dep.oracle.state);
    d.vaults.remove(i);
    Ok((b, d))
}

/// Löst einen schuldenfreien Vault auf, dessen Zins die ganze Sicherheit
/// aufzehrt (stable_vault.sil sweep): jeder darf, alles bis auf SWEEP_FEE geht
/// an die Zinskasse. Der Vault trägt bis zu SWEEP_FEE (0,1 KAS) der
/// Netzgebühr; die sweep-Tx kostet ≈ 0,055 KAS (Vertragsskript im Eingang),
/// der Rest geht als Wechselgeld an `fund` (gemessen in tests/e2e_tests.rs).
/// Bei einer Sicherheit bis SWEEP_FEE gibt es keinen Kassen-Ausgang (Fehler
/// statt u64-Unterlauf, Audit 12 A12-2); knapp darüber lehnt der Bau den zu
/// schweren Ausgang ab. Ob sich der Bau sicher lohnt, sagt math::sweepable.
pub fn sweep(dep: &Deployment, i: usize, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    not_frozen(dep, "Auflösen")?;
    let v = dep.vaults.get(i).ok_or("Vault unbekannt")?;
    let vp = dep.vault_params.as_ref().unwrap();
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    if !math::sweep_allowed(v.vault.value as i64, &v.vault.state, price, index) {
        return Err("Vault ist nicht auflösbar: nur mit Schuld 0 und Zins ≥ Sicherheit".into());
    }
    // SWEEP_FEE ist nicht die Netzgebühr: Sie ist der Anteil, den der Vault für
    // das Auflösen trägt; die Netzgebühr (≈ 0,055 KAS) geht davon ab, den Rest
    // bekommt, wer auflöst (Audit 12, Restpunkt B-P1)
    let to_treasury = v.vault.value.checked_sub(math::SWEEP_FEE as u64).filter(|&x| x > 0).ok_or_else(|| {
        format!(
            "Vault ist zu klein zum Auflösen: {:.8} KAS Sicherheit. Vorab gehen {:.1} KAS für das Auflösen ab (SWEEP_FEE: davon die Netzgebühr von etwa 0.055 KAS, den Rest bekommt, wer auflöst); für die Zinskasse bliebe nichts.",
            v.vault.value as f64 / 1e8,
            math::SWEEP_FEE as f64 / 1e8
        )
    })?;
    let (vin, _) = vault_input(dep, i, "sweep", vec![ArtifactValue::Int(1)], None)?;
    let (oin, oout) = oracle_read_input(dep);
    let mut inputs = vec![vin, oin];
    inputs.extend(fund.inputs());
    let outputs = vec![set_auth(oout, 1), TransactionOutput { value: to_treasury, script_public_key: spk_from_bytes(&vp.interest_spk)?, covenant: None }];
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut d = dep.clone();
    d.oracle = track(&b, 0, dep.oracle.cov, dep.oracle.state);
    d.vaults.remove(i);
    Ok((b, d))
}

/// Vaults, die der Keeper in dieser Reihenfolge aufzulösen versucht: nicht
/// veraltet und nach Orakel- UND Marktpreis `math::sweepable` (Audit 11
/// A11-V-4: ein nachlaufendes Orakel löst keinen Vault auf, den sein Besitzer
/// noch mit Gewinn schließen könnte). Scheitert einer, kommt der nächste dran
/// (Audit 12 A12-2: vorher versuchte der Keeper immer nur den ersten).
pub fn sweep_candidates(dep: &Deployment, market: i64) -> Vec<usize> {
    if dep.oracle.state.frozen {
        return vec![];
    }
    let (price, index) = (dep.oracle.state.kas_usd, dep.oracle.state.stable_index);
    dep.vaults
        .iter()
        .enumerate()
        .filter(|(_, v)| {
            let coll = v.vault.value as i64;
            !v.stale && math::sweepable(coll, &v.vault.state, price, index) && math::sweepable(coll, &v.vault.state, market, index)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Überweist `amount` GHOST aus Token-UTXOs von `sender` an `to` (x-only).
/// Normale KCC20-Übertragung: Leader ist der erste Token-Input, Menge bleibt erhalten.
pub fn transfer(dep: &Deployment, sender: impl Into<Signer>, token_idx: &[usize], to: &[u8], amount: i64, fund: &Funds, net: &Params) -> Result<(Built, Deployment), String> {
    transfer_with_payload(dep, sender, token_idx, to, amount, &[], fund, net)
}

/// Wie `transfer`, mit Nutzdaten im Payload der Tx (öffentliche Nachricht).
/// Der GHOST-Vertrag prüft den Payload nicht; er steht nur in der Tx.
#[allow(clippy::too_many_arguments)]
pub fn transfer_with_payload(
    dep: &Deployment,
    sender: impl Into<Signer>,
    token_idx: &[usize],
    to: &[u8],
    amount: i64,
    payload: &[u8],
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Deployment), String> {
    if token_idx.is_empty() || token_idx.len() > GHOST_MAX_INS as usize {
        return Err(format!("1 bis {GHOST_MAX_INS} Token-UTXOs erlaubt"));
    }
    let toks: Vec<&Tracked<GhostTok>> = token_idx.iter().map(|&t| dep.tokens.get(t).ok_or("Token unbekannt")).collect::<Result<_, _>>()?;
    let have: i64 = toks.iter().map(|t| t.state.amount).sum();
    if amount <= 0 || amount > have {
        return Err(format!("Betrag muss zwischen 1 und {have} liegen"));
    }
    let sender: Signer = sender.into();
    let recv = GhostTok::to_pubkey(to, amount);
    let change = GhostTok::to_pubkey(&sender.xonly(), have - amount);
    let mut outs = vec![recv.clone()];
    if change.amount > 0 {
        outs.push(change.clone());
    }
    let states: Vec<ArtifactValue> = outs.iter().map(GhostTok::arg).collect();
    let mut inputs = vec![];
    for (n, t) in toks.iter().enumerate() {
        let art = t.state.artifact();
        let unlock = if n == 0 { Unlock::Leader { art: art.clone(), new_states: states.clone(), signer: Some(sender) } } else { Unlock::Delegate { art: art.clone(), signer: sender } };
        inputs.push(In { outpoint: t.outpoint, entry: cov_in(&t.outpoint, t.value, &art, t.cov), unlock });
    }
    inputs.extend(fund.inputs());
    let cov = toks[0].cov;
    let outputs: Vec<TransactionOutput> = outs.iter().map(|o| cov_out(&o.artifact(), TOKEN_VALUE, 0, cov)).collect();
    let b = build_with_payload(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, payload, net)?;
    let mut d = dep.clone();
    let mut spent: Vec<usize> = token_idx.to_vec();
    spent.sort_unstable_by(|a, b| b.cmp(a));
    for t in spent {
        d.tokens.remove(t);
    }
    for (n, o) in outs.into_iter().enumerate() {
        d.tokens.push(track(&b, n as u32, cov, o));
    }
    Ok((b, d))
}
