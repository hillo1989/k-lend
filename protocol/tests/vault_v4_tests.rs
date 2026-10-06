//! VAULT V4: Kopie von vault_tests.rs für contracts/stable_vault_v4.sil mit dem
//! Orakel v4 (price_oracle_v4.sil). Alle v3-Tests laufen unverändert gegen v4
//! (Zinsziel = P2PK der Zinskasse wie in v3); die v4-Tests (Einfrieren,
//! Zinsziel-Varianten) stehen am Ende.
//!
//! Ganze Transaktionen mit StableVault + RiskOracle + GHOST (KCC20 aus
//! vendor/silverscript, v1.0.0), ausgeführt in der Skript-Engine. Jeder Input
//! der Transaktion wird ausgeführt; ein Angriff gilt als abgewehrt, wenn
//! mindestens ein Input scheitert — und wir prüfen, WELCHER.

mod common;

use common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::tx::{
    CovenantBinding, MutableTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_lending_protocol::{contracts::VaultState, math};
use kaspa_txscript::pay_to_script_hash_script;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_covenant_decl_sig_script, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};
use std::collections::BTreeMap;

const VAULT_SRC: &str = include_str!("../../contracts/stable_vault_v4.sil");
const ORACLE_SRC: &str = include_str!("../../contracts/price_oracle_v4.sil");
/// GHOST-Token Version 2 (KCC20 + Betragsprüfung, Audit 1 V-01)
const KCC20_SRC: &str = include_str!("../../contracts/ghost_token.sil");

const ORACLE_COV: Hash = Hash::from_bytes([0x0a; 32]);
const GHOST_COV: Hash = Hash::from_bytes([0x0b; 32]);
const VAULT_COV: Hash = Hash::from_bytes([0x0c; 32]);
const OTHER_COV: Hash = Hash::from_bytes([0x0d; 32]);

const MCR: i64 = 20_000; // 200 %
const LIQ: i64 = 15_000; // 150 %
const BONUS: i64 = 1_000; // 10 %
const KCC20_MAX_INS: i64 = 3;
const KCC20_MAX_OUTS: i64 = 2;
const ID_PUBKEY: u8 = 0x00;
const ID_COV: u8 = 0x02;
const E8: i64 = 100_000_000;
const DUST: i64 = 20_000_000;

// ------------------------------------------------------ Referenzrechnung ----

/// Vault-Zustand (Version 3): geprägte Schuld, aufgelaufener Zins (USD × 1e8)
/// und der Orakelindex, bis zu dem der Zins abgerechnet ist.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct St {
    debt: i64,
    interest: i64,
    at: i64,
}

/// Nur Schuld, Zins bis zum Test-Index INDEX abgerechnet (kein Zuwachs)
impl From<i64> for St {
    fn from(debt: i64) -> Self {
        St { debt, interest: 0, at: INDEX }
    }
}

impl St {
    fn vs(self) -> VaultState {
        VaultState { debt: self.debt, interest: self.interest, index_at: self.at }
    }
    /// Zins bis `index` (stable_vault.sil accrued)
    fn accrued(self, index: i64) -> i64 {
        math::accrued(&self.vs(), index)
    }
    /// Schuld + Zins bis `index`
    fn owed(self, index: i64) -> i64 {
        self.debt + self.accrued(index)
    }
    /// Zustand nach einer Aktion mit neuer Schuld `debt` (Zins abgerechnet)
    fn then(self, debt: i64, index: i64) -> St {
        St { debt, interest: self.accrued(index), at: index }
    }
}

fn value_of(coll: i64, price: i64) -> i64 {
    (coll as u128 * price as u128 / E8 as u128) as i64
}
fn healthy(coll: i64, owed: i64, price: i64, ratio: i64) -> bool {
    owed == 0 || value_of(coll, price) as u128 >= (owed as u128 * ratio as u128).div_ceil(10_000)
}

/// Folgezustand, den der Vault bei `entry` mit neuer Schuld `new_debt` verlangt:
/// bei jeder Aktion (auch Liquidation) wird der Zins abgerechnet und bleibt stehen.
fn after(_entry: &str, s: St, new_debt: i64, index: i64) -> St {
    s.then(new_debt, index)
}

// ------------------------------------------------------------- Umgebung ----

fn random_keypair() -> Keypair {
    let secp = Secp256k1::new();
    let mut sk = [0u8; 32];
    loop {
        thread_rng().fill_bytes(&mut sk);
        if let Ok(s) = SecretKey::from_slice(&sk) {
            return Keypair::from_secret_key(&secp, &s);
        }
    }
}

fn xonly(k: &Keypair) -> Vec<u8> {
    k.x_only_public_key().0.serialize().to_vec()
}

#[derive(Clone, Copy)]
struct Oracle {
    kas_usd: i64,
    index: i64,
}

struct Env {
    committee: Vec<Vec<u8>>,
    oracle: Oracle,
    oracle_tpl: (Vec<u8>, Vec<u8>, Vec<u8>),
    ghost_tpl: (Vec<u8>, Vec<u8>, Vec<u8>),
    owner: Keypair,
    /// Zinskasse (Deployer)
    treasury: Keypair,
    /// Höchstschuld je Vault (Standard: praktisch unbegrenzt)
    max_debt: i64,
    /// v4: Orakel eingefroren (gilt für jedes oracle_art dieses Env)
    frozen: std::cell::Cell<bool>,
    /// v4: Zinsziel als scriptPubKey-Bytes (Standard: P2PK der Zinskasse wie v3)
    interest_spk: Vec<u8>,
}

impl Env {
    fn new(oracle: Oracle) -> Self {
        let committee: Vec<Vec<u8>> = (0..5).map(|_| xonly(&random_keypair())).collect();
        let mut e = Self {
            committee,
            oracle,
            oracle_tpl: Default::default(),
            ghost_tpl: Default::default(),
            owner: random_keypair(),
            treasury: random_keypair(),
            max_debt: i64::MAX,
            frozen: std::cell::Cell::new(false),
            interest_spk: vec![],
        };
        e.interest_spk = spk_bytes(&p2pk_out(&e.treasury, 0).script_public_key);
        e.oracle_tpl = compiled_template_parts_and_hash(&e.oracle_art(oracle));
        e.ghost_tpl = compiled_template_parts_and_hash(&e.ghost(vec![0; 32], ID_COV, 0, true));
        e
    }

    fn oracle_art(&self, o: Oracle) -> SilAbiArtifact {
        // v4: Register-ID statt Komitee (Schlüssel spielen beim Lesen keine Rolle)
        let _ = &self.committee;
        let mut args: Vec<ArtifactValue> = vec![ArtifactValue::Bytes(vec![0x5e; 32])];
        args.extend([1_000_000_000i64, 15_854_896, 36_000, 72_000, o.kas_usd, 1_000_000, 1, 158_548_959, o.index].map(ArtifactValue::Int));
        args.push(ArtifactValue::Bool(self.frozen.get()));
        args.push(ArtifactValue::Int(1_000_000));
        compile_to_sil_abi_artifact_with_options(ORACLE_SRC, &args, CompileOptions::default()).expect("Orakel kompiliert")
    }

    fn ghost(&self, owner: Vec<u8>, typ: u8, amount: i64, minter: bool) -> SilAbiArtifact {
        compile_to_sil_abi_artifact_with_options(
            KCC20_SRC,
            &[
                ArtifactValue::Bytes(owner),
                ArtifactValue::Int(amount),
                ArtifactValue::Byte(typ),
                ArtifactValue::Bool(minter),
                ArtifactValue::Int(KCC20_MAX_INS),
                ArtifactValue::Int(KCC20_MAX_OUTS),
            ],
            CompileOptions::default(),
        )
        .expect("KCC20 kompiliert")
    }

    fn vault_with(&self, oracle_cov: Hash, owner: Vec<u8>, s: impl Into<St>) -> SilAbiArtifact {
        let s = s.into();
        let (op, os, oh) = &self.oracle_tpl;
        let (kp, ks, kh) = &self.ghost_tpl;
        compile_to_sil_abi_artifact_with_options(
            VAULT_SRC,
            &[
                ArtifactValue::Bytes(oracle_cov.as_bytes().to_vec()),
                ArtifactValue::Int(op.len() as i64),
                ArtifactValue::Int(os.len() as i64),
                ArtifactValue::Bytes(oh.clone()),
                ArtifactValue::Bytes(GHOST_COV.as_bytes().to_vec()),
                ArtifactValue::Int(kp.len() as i64),
                ArtifactValue::Int(ks.len() as i64),
                ArtifactValue::Bytes(kh.clone()),
                ArtifactValue::Int(MCR),
                ArtifactValue::Int(LIQ),
                ArtifactValue::Int(BONUS),
                ArtifactValue::Int(self.max_debt),
                ArtifactValue::Bytes(self.interest_spk.clone()),
                ArtifactValue::Bytes(owner),
                ArtifactValue::Int(s.debt),
                ArtifactValue::Int(s.interest),
                ArtifactValue::Int(s.at),
            ],
            CompileOptions::default(),
        )
        .expect("Vault kompiliert")
    }

    fn vault(&self, s: impl Into<St>) -> SilAbiArtifact {
        self.vault_with(ORACLE_COV, xonly(&self.owner), s)
    }
}


// ------------------------------------------------------- Tx-Bausteine ----

fn ghost_state(owner: &[u8], typ: u8, amount: i64, minter: bool) -> ArtifactValue {
    BTreeMap::from([
        ("ownerIdentifier".to_string(), ArtifactValue::Bytes(owner.to_vec())),
        ("identifierType".to_string(), ArtifactValue::Byte(typ)),
        ("amount".to_string(), ArtifactValue::Int(amount)),
        ("isMinter".to_string(), ArtifactValue::Bool(minter)),
    ])
    .into()
}

#[derive(Clone)]
struct Tok {
    owner: Vec<u8>,
    typ: u8,
    amount: i64,
    minter: bool,
}

impl Tok {
    fn minter() -> Self {
        Self { owner: VAULT_COV.as_bytes().to_vec(), typ: ID_COV, amount: 0, minter: true }
    }
    fn to(k: &Keypair, amount: i64) -> Self {
        Self { owner: xonly(k), typ: ID_PUBKEY, amount, minter: false }
    }
    fn arg(&self) -> ArtifactValue {
        ghost_state(&self.owner, self.typ, self.amount, self.minter)
    }
}

/// Wie ein Input seine Signatur-Skript-Argumente bekommt.
enum Call {
    /// Einstiegspunkt mit Argumenten; `sig_at` = Position, an der die
    /// Besitzer-Signatur (über die fertige Tx) eingesetzt wird.
    Entry { art: SilAbiArtifact, entry: &'static str, args: Vec<ArtifactValue>, sig_by: Option<(usize, Keypair)> },
    /// KCC20-Leader (transfer) bzw. -Delegate
    GhostLeader { art: SilAbiArtifact, new_states: Vec<ArtifactValue>, sig_by: Option<Keypair> },
    GhostDelegate { art: SilAbiArtifact, sig_by: Keypair },
}

struct In {
    utxo: UtxoEntry,
    call: Call,
}

fn utxo(art: &SilAbiArtifact, value: i64, cov: Hash) -> UtxoEntry {
    UtxoEntry::new(value as u64, pay_to_script_hash_script(&bytecode(art)), 0, false, Some(cov))
}

fn out(art: &SilAbiArtifact, value: i64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput {
        value: value as u64,
        script_public_key: pay_to_script_hash_script(&bytecode(art)),
        covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }),
    }
}

fn plain_out(value: i64) -> TransactionOutput {
    TransactionOutput {
        value: value as u64,
        script_public_key: kaspa_consensus_core::tx::ScriptPublicKey::new(0, vec![kaspa_txscript::opcodes::codes::OpTrue].into()),
        covenant: None,
    }
}

fn outpoint(i: usize) -> TransactionOutpoint {
    TransactionOutpoint { transaction_id: TransactionId::from_bytes([0x40 + i as u8; 32]), index: i as u32 }
}

fn sign(tx: &Transaction, entries: &[UtxoEntry], idx: usize, k: &Keypair) -> Vec<u8> {
    let mtx = MutableTransaction::with_entries(tx.clone(), entries.to_vec());
    let reused = SigHashReusedValuesUnsync::new();
    let h = calc_schnorr_signature_hash(&mtx.as_verifiable(), idx, SIG_HASH_ALL, &reused);
    let mut s = k.sign_schnorr(Message::from_digest_slice(h.as_bytes().as_slice()).unwrap()).as_ref().to_vec();
    s.push(SIG_HASH_ALL.to_u8());
    s
}

fn contract_name(a: &SilAbiArtifact) -> String {
    a.contracts.keys().next().unwrap().clone()
}

/// Baut die Tx, signiert, führt JEDEN Input aus. Ergebnis je Input.
fn execute(inputs: Vec<In>, outputs: Vec<TransactionOutput>) -> Vec<Result<(), String>> {
    execute_lt(inputs, outputs, 0)
}

/// wie execute, mit Locktime (Audit 12: Tresor-Eingänge prüfen tx.time)
fn execute_lt(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64) -> Vec<Result<(), String>> {
    let entries: Vec<UtxoEntry> = inputs.iter().map(|i| i.utxo.clone()).collect();
    let bare: Vec<TransactionInput> =
        (0..inputs.len()).map(|i| TransactionInput::new_with_compute_budget(outpoint(i), vec![], 0, 0)).collect();
    let unsigned = Transaction::new(1, bare, outputs.clone(), lock_time, Default::default(), 0, vec![]);

    let mut final_inputs = vec![];
    for (idx, input) in inputs.into_iter().enumerate() {
        let script = match input.call {
            Call::Entry { art, entry, mut args, sig_by } => {
                if let Some((pos, k)) = sig_by {
                    args.insert(pos, ArtifactValue::Bytes(sign(&unsigned, &entries, idx, &k)));
                }
                let mut s = encode_contract_entry_sig_script(&art, &contract_name(&art), entry, &args).expect("sigscript");
                s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
                s
            }
            Call::GhostLeader { art, new_states, sig_by } => {
                let sig = sig_by.map(|k| sign(&unsigned, &entries, idx, &k)).unwrap_or(vec![0; 65]);
                let args = vec![ArtifactValue::Array(new_states), ArtifactValue::Bytes(sig), ArtifactValue::Byte(0)];
                let mut s = encode_contract_covenant_decl_sig_script(&art, &contract_name(&art), "transfer", true, &args).expect("leader");
                s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
                s
            }
            Call::GhostDelegate { art, sig_by } => {
                let args = vec![ArtifactValue::Bytes(sign(&unsigned, &entries, idx, &sig_by)), ArtifactValue::Byte(0)];
                let mut s = encode_contract_covenant_decl_sig_script(&art, &contract_name(&art), "transfer", false, &args).expect("delegate");
                s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
                s
            }
        };
        final_inputs.push(TransactionInput::new_with_compute_budget(outpoint(idx), script, 0, 0));
    }
    let tx = Transaction::new(1, final_inputs, outputs, lock_time, Default::default(), 0, vec![]);
    (0..tx.inputs.len()).map(|i| execute_input_with_covenants(tx.clone(), entries.clone(), i).map_err(|e| format!("{e:?}"))).collect()
}

fn all_ok(r: &[Result<(), String>]) -> bool {
    r.iter().all(|x| x.is_ok())
}

fn oracle_read(env: &Env, cov: Hash, o: Oracle) -> In {
    In {
        utxo: utxo(&env.oracle_art(o), E8, cov),
        call: Call::Entry { art: env.oracle_art(o), entry: "read", args: vec![], sig_by: None },
    }
}

// ------------------------------------------------------------ Szenarien ----

const PRICE: i64 = 4_000_000; // 0,04 USD
const INDEX: i64 = 1_050_000_000; // Zinsindex des Orakels (5 % seit Start)
const INDEX0: i64 = 1_000_000_000; // Startindex
const COLL: i64 = 10_000 * E8; // 10 000 KAS = 400 USD

fn env() -> Env {
    Env::new(Oracle { kas_usd: PRICE, index: INDEX })
}

/// Größter Betrag, der bei `coll` und Zustand `s` noch prägbar ist (Referenz).
fn max_mint(coll: i64, s: St, o: Oracle) -> i64 {
    let owed = s.owed(o.index);
    let (mut lo, mut hi) = (0i64, value_of(coll, o.kas_usd));
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if healthy(coll, owed + mid, o.kas_usd, MCR) { lo = mid } else { hi = mid - 1 }
    }
    lo
}

struct Mint {
    amount: i64,
    recipient_amount: i64,
    new_state: Option<St>,
    signer: Option<Keypair>,
    oracle_cov: Hash,
    oracle: Oracle,
    extra_minter_to: Option<Vec<u8>>,
    coll: i64,
    /// abweichende Fortsetzung des Minter-Zweigs (Angriffstests)
    minter_out: Option<Tok>,
    /// abweichender GHOST-Eingang 0 (Leader) statt des eigenen Minter-Zweigs
    minter_in: Option<Tok>,
}

impl Mint {
    fn new(env: &Env, amount: i64) -> Self {
        Self {
            amount,
            recipient_amount: amount,
            new_state: None,
            signer: Some(env.owner),
            oracle_cov: ORACLE_COV,
            oracle: env.oracle,
            extra_minter_to: None,
            coll: COLL,
            minter_out: None,
            minter_in: None,
        }
    }

    fn run(&self, env: &Env, s: impl Into<St>) -> Vec<Result<(), String>> {
        let s = s.into();
        let recipient = random_keypair();
        let new_state = self.new_state.unwrap_or(s.then(s.debt + self.amount, self.oracle.index));
        let minter_in = self.minter_in.clone().unwrap_or_else(Tok::minter);
        let mut outs = vec![self.minter_out.clone().unwrap_or_else(Tok::minter), Tok::to(&recipient, self.recipient_amount)];
        if let Some(to) = &self.extra_minter_to {
            outs[1] = Tok { owner: to.clone(), typ: ID_PUBKEY, amount: self.recipient_amount, minter: true };
        }
        let out_args: Vec<ArtifactValue> = outs.iter().map(Tok::arg).collect();
        let vault = env.vault_with(self.oracle_cov, xonly(&env.owner), s);
        let next_vault = env.vault_with(self.oracle_cov, xonly(&env.owner), new_state);
        let signer = self.signer.unwrap_or_else(random_keypair);

        let inputs = vec![
            In {
                utxo: utxo(&vault, self.coll, VAULT_COV),
                call: Call::Entry {
                    art: vault.clone(),
                    entry: "mint",
                    args: vec![ArtifactValue::Int(self.amount), ArtifactValue::Int(1), ArtifactValue::Array(out_args.clone())],
                    sig_by: Some((3, signer)),
                },
            },
            oracle_read(env, self.oracle_cov, self.oracle),
            In {
                utxo: utxo(&env.ghost(minter_in.owner.clone(), minter_in.typ, minter_in.amount, minter_in.minter), 1_000, GHOST_COV),
                call: Call::GhostLeader { art: env.ghost(minter_in.owner.clone(), minter_in.typ, minter_in.amount, minter_in.minter), new_states: out_args, sig_by: None },
            },
        ];
        let outputs = vec![
            out(&next_vault, self.coll, 0, VAULT_COV),
            out(&env.oracle_art(self.oracle), E8, 1, self.oracle_cov),
            out(&env.ghost(outs[0].owner.clone(), outs[0].typ, outs[0].amount, outs[0].minter), 1_000, 2, GHOST_COV),
            out(&env.ghost(outs[1].owner.clone(), outs[1].typ, outs[1].amount, outs[1].minter), 1_000, 2, GHOST_COV),
        ];
        execute(inputs, outputs)
    }
}

#[test]
fn mint_bis_zur_mindestquote_geht_durch() {
    let e = env();
    let m = max_mint(COLL, St::default(), e.oracle);
    assert_eq!(m, 200 * E8, "400 USD Sicherheit bei 200 % tragen genau 200 GHOST");
    let r = Mint::new(&e, m).run(&e, 0);
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn mint_eine_einheit_ueber_der_mindestquote_scheitert_am_vault() {
    let e = env();
    let m = max_mint(COLL, St::default(), e.oracle) + 1;
    let r = Mint::new(&e, m).run(&e, 0);
    assert!(r[0].is_err() && r[1].is_ok() && r[2].is_ok(), "nur der Vault darf ablehnen: {r:?}");
}

#[test]
fn mint_bis_zur_obergrenze_je_vault() {
    // Mainnet: höchstens 50 GHOST je Vault; genug Sicherheit, damit nur die Grenze zählt
    let mut e = env();
    e.max_debt = 50 * E8;
    let cap = |amount: i64, s: St| {
        let mut m = Mint::new(&e, amount);
        m.coll = 1_000_000 * E8;
        m.run(&e, s)
    };
    assert!(all_ok(&cap(50 * E8, St::default())), "bis zur Grenze geht");
    let r = cap(50 * E8 + 1, St::default());
    assert!(r[0].is_err() && r[1].is_ok() && r[2].is_ok(), "eine Einheit darüber scheitert nur am Vault: {r:?}");
    // In mehreren Schritten ebenso: 30 GHOST vorhanden, 25 dazu wären über der Grenze
    assert!(all_ok(&cap(10 * E8, St::from(30 * E8))));
    assert!(cap(25 * E8, St::from(30 * E8))[0].is_err());
}

#[test]
fn mint_ohne_besitzer_signatur_scheitert() {
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.signer = None; // fremder Schlüssel
    let r = m.run(&e, 0);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn mint_mehr_ausgeben_als_verbucht_scheitert() {
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.recipient_amount = 1_000 * E8; // Empfänger bekommt mehr, als als Schuld gebucht wird
    let r = m.run(&e, 0);
    assert!(r[0].is_err() && r[2].is_ok(), "KCC20 selbst hält das nicht auf, der Vault muss: {r:?}");
}

#[test]
fn mint_ohne_schuldbuchung_scheitert() {
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.new_state = Some(St::from(0));
    let r = m.run(&e, 0);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn mint_darf_keinen_zweiten_minter_erzeugen() {
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.extra_minter_to = Some(xonly(&random_keypair()));
    let r = m.run(&e, 0);
    assert!(r[0].is_err() && r[2].is_ok(), "KCC20 erlaubt es dem Minter-Leader, der Vault muss es verbieten: {r:?}");
}

#[test]
fn mint_mit_falschem_orakel_scheitert() {
    let e = env();
    // Angreifer baut ein eigenes Orakel (andere Covenant-ID) mit 100-fachem Preis;
    // der Vault ist auf ORACLE_COV festgelegt.
    let r = run_mint_with_fake_oracle(&e, 1_000 * E8, Oracle { kas_usd: PRICE * 100, index: INDEX });
    assert!(r[0].is_err(), "{r:?}");
}

/// Vault erwartet ORACLE_COV, Tx liefert ein Orakel unter OTHER_COV.
fn run_mint_with_fake_oracle(e: &Env, amount: i64, fake: Oracle) -> Vec<Result<(), String>> {
    let recipient = random_keypair();
    let outs = [Tok::minter(), Tok::to(&recipient, amount)];
    let out_args: Vec<ArtifactValue> = outs.iter().map(Tok::arg).collect();
    let vault = e.vault(0);
    let next_vault = e.vault(St { debt: amount, interest: 0, at: fake.index });
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let inputs = vec![
        In {
            utxo: utxo(&vault, COLL, VAULT_COV),
            call: Call::Entry {
                art: vault.clone(),
                entry: "mint",
                args: vec![ArtifactValue::Int(amount), ArtifactValue::Int(1), ArtifactValue::Array(out_args.clone())],
                sig_by: Some((3, e.owner)),
            },
        },
        oracle_read(e, OTHER_COV, fake),
        In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: out_args, sig_by: None } },
    ];
    let outputs = vec![
        out(&next_vault, COLL, 0, VAULT_COV),
        out(&e.oracle_art(fake), E8, 1, OTHER_COV),
        out(&minter, 1_000, 2, GHOST_COV),
        out(&e.ghost(outs[1].owner.clone(), ID_PUBKEY, amount, false), 1_000, 2, GHOST_COV),
    ];
    execute(inputs, outputs)
}

#[test]
fn mint_mit_echtem_orakel_und_gleichem_betrag_als_gegenprobe() {
    // Gegenprobe zu mint_mit_falschem_orakel_scheitert: mit dem echten Orakel
    // ist derselbe Betrag bei echtem Preis zu hoch — bei 100-fachem Preis aber
    // nicht. Der Unterschied liegt also nur an der Orakel-Herkunft.
    assert!(!healthy(COLL, 1_000 * E8, PRICE, MCR));
    assert!(healthy(COLL, 1_000 * E8, PRICE * 100, MCR));
}

// ------------------------------------------ deposit / withdraw / close ----

#[allow(clippy::too_many_arguments)]
fn vault_only(e: &Env, entry: &'static str, args: Vec<ArtifactValue>, sig_pos: Option<usize>, s: impl Into<St>, coll_in: i64, coll_out: Option<i64>, with_oracle: bool, sneaky_mint: bool) -> Vec<Result<(), String>> {
    let s = s.into();
    let vault = e.vault(s);
    // withdraw rechnet den Zins bis zum Orakelindex ab, deposit lässt den Zustand stehen
    let next = if entry == "withdraw" { s.then(s.debt, e.oracle.index) } else { s };
    let mut inputs = vec![In {
        utxo: utxo(&vault, coll_in, VAULT_COV),
        call: Call::Entry { art: vault.clone(), entry, args, sig_by: sig_pos.map(|p| (p, e.owner)) },
    }];
    let mut outputs = vec![];
    if let Some(c) = coll_out {
        outputs.push(out(&e.vault(next), c, 0, VAULT_COV));
    }
    if with_oracle {
        inputs.push(oracle_read(e, ORACLE_COV, e.oracle));
        outputs.push(out(&e.oracle_art(e.oracle), E8, (inputs.len() - 1) as u16, ORACLE_COV));
    }
    if sneaky_mint {
        // Angriff: neben der harmlosen Vault-Aktion den Minter-Zweig des Vaults
        // mitnehmen und 1 Mio GHOST an sich selbst prägen.
        let thief = random_keypair();
        let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
        let loot = Tok::to(&thief, 1_000_000 * E8);
        let states = vec![Tok::minter().arg(), loot.arg()];
        inputs.push(In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states, sig_by: None } });
        let a = (inputs.len() - 1) as u16;
        outputs.push(out(&minter, 1_000, a, GHOST_COV));
        outputs.push(out(&e.ghost(loot.owner.clone(), ID_PUBKEY, loot.amount, false), 1_000, a, GHOST_COV));
    }
    outputs.push(plain_out(1));
    execute(inputs, outputs)
}

#[test]
fn deposit_geht_durch() {
    let e = env();
    let r = vault_only(&e, "deposit", vec![], Some(0), 5 * E8, COLL, Some(COLL + E8), false, false);
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn deposit_mit_heimlichem_praegen_scheitert_am_vault() {
    // Der zentrale Angriff: KCC20 prüft beim Covenant-Besitzer nur, dass der
    // Vault in der Tx ist. Ohne noGhost() im Vault ginge das durch.
    let e = env();
    let r = vault_only(&e, "deposit", vec![], Some(0), 5 * E8, COLL, Some(COLL + E8), false, true);
    assert!(r[0].is_err(), "Vault muss ablehnen: {r:?}");
    assert!(r[1].is_ok(), "KCC20 allein würde das Prägen erlauben — genau darum prüft der Vault: {r:?}");
}

#[test]
fn deposit_darf_nicht_abziehen() {
    let e = env();
    let r = vault_only(&e, "deposit", vec![], Some(0), 5 * E8, COLL, Some(COLL - 1), false, false);
    assert!(r[0].is_err(), "{r:?}");
}

/// kleinste Sicherheit, bei der `owed` die Quote `ratio` hält (Referenz)
fn min_coll(owed: i64, price: i64, ratio: i64) -> i64 {
    let (mut lo, mut hi) = (0i64, COLL);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if healthy(mid, owed, price, ratio) { hi = mid } else { lo = mid + 1 }
    }
    lo
}

#[test]
fn withdraw_bis_zur_mindestquote() {
    let e = env();
    let debt = 50 * E8; // braucht ≥ 100 USD = 2 500 KAS
    let min = min_coll(debt, PRICE, MCR);
    assert_eq!(min, 2_500 * E8);
    let ok = vault_only(&e, "withdraw", vec![ArtifactValue::Int(min), ArtifactValue::Int(1)], Some(2), debt, COLL, Some(min), true, false);
    assert!(all_ok(&ok), "{ok:?}");
    let bad = vault_only(&e, "withdraw", vec![ArtifactValue::Int(min - 1), ArtifactValue::Int(1)], Some(2), debt, COLL, Some(min - 1), true, false);
    assert!(bad[0].is_err() && bad[1].is_ok(), "{bad:?}");
}

#[test]
fn withdraw_nur_durch_besitzer() {
    let e = env();
    let r = vault_only(&e, "withdraw", vec![ArtifactValue::Int(COLL / 2), ArtifactValue::Int(1), ArtifactValue::Bytes(vec![1; 65])], None, 0, COLL, Some(COLL / 2), true, false);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn close_nur_ohne_schuld() {
    let e = env();
    let ok = vault_only(&e, "close", vec![ArtifactValue::Int(1)], Some(1), 0, COLL, None, true, false);
    assert!(all_ok(&ok), "{ok:?}");
    let bad = vault_only(&e, "close", vec![ArtifactValue::Int(1)], Some(1), 1, COLL, None, true, false);
    assert!(bad[0].is_err(), "{bad:?}");
}

// ------------------------------------------ repay / redeem / liquidate ----

/// Verbrennt GHOST aus einem Token-Input des Zahlers; optional Wechselgeld.
/// `new_debt`: Schuld der Fortsetzung, der Zins wird wie im Vault abgerechnet.
#[allow(clippy::too_many_arguments)]
fn burn_tx(e: &Env, entry: &'static str, s: impl Into<St>, new_debt: Option<i64>, coll: i64, coll_out: Option<i64>, pay_in: i64, change: i64, o: Oracle) -> Vec<Result<(), String>> {
    burn_tx_ext(e, entry, s, new_debt, coll, coll_out, pay_in, change, o, None)
}

#[allow(clippy::too_many_arguments)]
fn burn_tx_ext(e: &Env, entry: &'static str, s: impl Into<St>, new_debt: Option<i64>, coll: i64, coll_out: Option<i64>, pay_in: i64, change: i64, o: Oracle, rogue: Option<TransactionOutput>) -> Vec<Result<(), String>> {
    let s = s.into();
    burn_tx_full(e, entry, s, new_debt.map(|d| after(entry, s, d, o.index)), coll, coll_out, pay_in, change, o, rogue, None)
}

/// wie burn_tx_ext, aber mit frei gewähltem Folgezustand und verbiegbarem
/// Betragsargument von liquidate/redeem
#[allow(clippy::too_many_arguments)]
fn burn_tx_full(e: &Env, entry: &'static str, s: St, new: Option<St>, coll: i64, coll_out: Option<i64>, pay_in: i64, change: i64, o: Oracle, rogue: Option<TransactionOutput>, burn_arg: Option<i64>) -> Vec<Result<(), String>> {
    let payer = random_keypair();
    let vault = e.vault(s);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, pay_in, false);
    let mut out_toks = vec![Tok::minter()];
    if change > 0 {
        out_toks.push(Tok::to(&payer, change));
    }
    let out_args: Vec<ArtifactValue> = out_toks.iter().map(Tok::arg).collect();
    let oracle_art = e.oracle_art(o);
    let amount = ArtifactValue::Int(burn_arg.unwrap_or(pay_in - change));
    let inputs = vec![
        In {
            utxo: utxo(&vault, coll, VAULT_COV),
            call: Call::Entry {
                art: vault.clone(),
                entry,
                args: match entry {
                    // liquidate(oracleIdx, burn, outStates) / redeem(oracleIdx, amount, outStates)
                    "liquidate" | "redeem" => vec![ArtifactValue::Int(1), amount, ArtifactValue::Array(out_args.clone())],
                    // repay(oracleIdx, outStates, sig)
                    _ => vec![ArtifactValue::Int(1), ArtifactValue::Array(out_args.clone())],
                },
                // repay nur durch den Besitzer (v2.1); Rücknahme und Liquidation für jeden
                sig_by: if entry == "repay" { Some((2, e.owner)) } else { None },
            },
        },
        In { utxo: utxo(&oracle_art, E8, ORACLE_COV), call: Call::Entry { art: oracle_art.clone(), entry: "read", args: vec![], sig_by: None } },
        In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: out_args, sig_by: None } },
        In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
    ];
    let mut outputs = vec![];
    if let (Some(c), Some(n)) = (coll_out, new) {
        outputs.push(out(&e.vault(n), c, 0, VAULT_COV));
    }
    if let Some(r) = rogue {
        outputs.push(r);
    }
    outputs.push(out(&oracle_art, E8, 1, ORACLE_COV));
    outputs.push(out(&minter, 1_000, 2, GHOST_COV));
    if change > 0 {
        outputs.push(out(&e.ghost(xonly(&payer), ID_PUBKEY, change, false), 1_000, 2, GHOST_COV));
    }
    outputs.push(plain_out(1)); // Ausgang für Liquidator / Rücknehmer / Gebühr
    execute(inputs, outputs)
}

#[test]
fn repay_teilweise() {
    let e = env();
    let debt = 100 * E8;
    let burn = 30 * E8;
    let r = burn_tx(&e, "repay", debt, Some(debt - burn), COLL, Some(COLL), burn + 5 * E8, 5 * E8, e.oracle);
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn repay_teilweise_mit_zu_hoher_restschuld_scheitert() {
    let e = env();
    let debt = 100 * E8;
    let burn = 30 * E8;
    let r = burn_tx(&e, "repay", debt, Some(debt - burn + 1), COLL, Some(COLL), burn, 0, e.oracle);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn repay_teilweise_mit_zu_niedriger_restschuld_scheitert() {
    let e = env();
    let debt = 100 * E8;
    let burn = 30 * E8;
    let r = burn_tx(&e, "repay", debt, Some(debt - burn - 1), COLL, Some(COLL), burn, 0, e.oracle);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn repay_vollstaendig_und_ueberzahlung() {
    let e = env();
    let debt = 100 * E8;
    let ok = burn_tx(&e, "repay", debt, Some(0), COLL, Some(COLL), debt, 0, e.oracle);
    assert!(all_ok(&ok), "{ok:?}");
    let over = burn_tx(&e, "repay", debt, Some(0), COLL, Some(COLL), debt + 1, 0, e.oracle);
    assert!(over[0].is_err(), "Überzahlung muss abgelehnt werden: {over:?}");
}

/// Preis, bei dem der Vault gerade unter die Liquidationsschwelle fällt.
fn crash_price(coll: i64, s: impl Into<St>) -> i64 {
    let owed = s.into().owed(INDEX);
    let mut p = PRICE;
    while healthy(coll, owed, p, LIQ) {
        p -= 1_000;
    }
    p
}

fn seize(coll: i64, burn: i64, price: i64) -> i64 {
    let claim = (burn as u128 * (10_000 + BONUS) as u128).div_ceil(10_000) as i64;
    if value_of(coll, price) > claim { (claim as u128 * E8 as u128).div_ceil(price as u128) as i64 } else { coll }
}

#[test]
fn liquidation_eines_gesunden_vaults_scheitert() {
    let e = env();
    let debt = 100 * E8;
    let rest = COLL - seize(COLL, debt, PRICE);
    let r = burn_tx(&e, "liquidate", debt, Some(0), COLL, Some(rest), debt, 0, e.oracle);
    assert!(r[0].is_err() && r[1].is_ok(), "{r:?}");
}

#[test]
fn liquidation_unter_der_schwelle() {
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let rest = COLL - seize(COLL, debt, p);
    assert!(rest >= DUST);
    let r = burn_tx(&e2, "liquidate", debt, Some(0), COLL, Some(rest), debt, 0, o);
    assert!(all_ok(&r), "{r:?}");
    // Gegenprobe: 1 sompi mehr beim Liquidator (weniger Rest) wird abgelehnt
    let greedy = burn_tx(&e2, "liquidate", debt, Some(0), COLL, Some(rest - 1), debt, 0, o);
    assert!(greedy[0].is_err(), "{greedy:?}");
    // Gegenprobe: nicht die ganze Schuld verbrannt
    let short = burn_tx(&e2, "liquidate", debt, Some(0), COLL, Some(rest), debt - 1, 0, o);
    assert!(short[0].is_err(), "{short:?}");
}

#[test]
fn liquidation_bei_totalausfall_nimmt_alles_und_beendet_den_vault() {
    let e = env();
    let debt = 150 * E8;
    let p = 100_000; // 0,001 USD: Sicherheit deckt die Schuld nicht mehr
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    assert_eq!(seize(COLL, debt, p), COLL);
    let r = burn_tx(&e2, "liquidate", debt, None, COLL, None, debt, 0, o);
    assert!(all_ok(&r), "{r:?}");
}

// ============================================================================
// Nachgezogen nach dem Mutationstest vom 28.09.2026: jede Regel, deren Entfernen
// vorher keinen Test rot gemacht hat, bekommt einen Fall, den NUR sie abfängt.
// ============================================================================

fn rogue_vault_out(value: i64) -> TransactionOutput {
    // beliebiges Skript (OpTrue) unter der Covenant-ID des Vaults
    TransactionOutput { covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: VAULT_COV }), ..plain_out(value) }
}

/// Ausgang an die Zinskasse (P2PK: <32 Byte Schlüssel> OP_CHECKSIG), unabhängig
/// von ops::p2pk_spk nachgebaut
/// scriptPubKey so, wie SilverScript tx.outputs[i].scriptPubKey liefert:
/// Version (2 Byte, LE) + Skript
fn spk_bytes(spk: &ScriptPublicKey) -> Vec<u8> {
    let mut v = spk.version().to_le_bytes().to_vec();
    v.extend_from_slice(spk.script());
    v
}

fn p2pk_out(k: &Keypair, value: i64) -> TransactionOutput {
    let mut script = vec![0x20];
    script.extend(xonly(k));
    script.push(0xac);
    TransactionOutput { value: value as u64, script_public_key: ScriptPublicKey::new(0, script.into()), covenant: None }
}

/// close(oracleIdx 1, sig): Input 0 Vault, Input 1 Orakel; Ausgang 0 ist die
/// Orakel-Fortsetzung, danach `rest` – die Zinskasse steht an Ausgang 1
/// (direkt hinter dem Vault-Eingang 0).
fn close_tx(e: &Env, s: impl Into<St>, coll: i64, signer: Keypair, rest: Vec<TransactionOutput>) -> Vec<Result<(), String>> {
    let vault = e.vault(s);
    let mut outputs = vec![out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV)];
    outputs.extend(rest);
    execute(
        vec![
            In {
                utxo: utxo(&vault, coll, VAULT_COV),
                call: Call::Entry { art: vault.clone(), entry: "close", args: vec![ArtifactValue::Int(1)], sig_by: Some((1, signer)) },
            },
            oracle_read(e, ORACLE_COV, e.oracle),
        ],
        outputs,
    )
}

#[test]
fn close_nur_durch_besitzer() {
    // L203: sonst schließt jeder einen schuldenfreien Vault und nimmt die KAS
    let e = env();
    let ok = close_tx(&e, 0, COLL, e.owner, vec![plain_out(COLL)]);
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
    let r = close_tx(&e, 0, COLL, random_keypair(), vec![plain_out(COLL)]);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn close_darf_die_covenant_id_nicht_weiterleben_lassen() {
    // L206: eine Fortsetzung mit beliebigem Skript unter VAULT_COV würde den
    // Minter-Zweig des Vaults für jeden benutzbar machen
    let e = env();
    let r = close_tx(&e, 0, COLL, e.owner, vec![rogue_vault_out(E8), plain_out(COLL - E8)]);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn totalliquidation_darf_die_covenant_id_nicht_weiterleben_lassen() {
    // L255: wie oben, aber durch einen Liquidator
    let e = env();
    let debt = 150 * E8;
    let o = Oracle { kas_usd: 100_000, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let ok = burn_tx(&e2, "liquidate", debt, None, COLL, None, debt, 0, o);
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
    let r = burn_tx_ext(&e2, "liquidate", debt, None, COLL, None, debt, 0, o, Some(rogue_vault_out(E8)));
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn repay_darf_nicht_praegen() {
    // L225: ohne burned > 0 könnte der Besitzer über repay GHOST prägen, ohne
    // dass die Mindestquote geprüft wird.
    let e = env();
    let debt = 10 * E8;
    let minted = 50 * E8;
    // Zustand genau so, wie ihn der Vault ohne die Regel berechnen würde
    let r = burn_tx(&e, "repay", debt, Some(debt + minted), COLL, Some(COLL), 1, 1 + minted, e.oracle);
    assert!(r[0].is_err(), "{r:?}");
    assert!(r[2].is_ok() && r[3].is_ok(), "KCC20 lässt das Prägen zu, nur der Vault verhindert es: {r:?}");
}

#[test]
fn mint_mit_negativem_betrag_scheitert() {
    // L212: negativer "mint" wäre ein Tilgen ohne die Regeln von repay.
    // Mit Wechselgeld-Ausgang, damit die Regel "genau 2 GHOST-Ausgänge" nicht vorher greift.
    let e = env();
    let debt = 10 * E8;
    let burn = 3 * E8;
    let change = 2 * E8;
    let payer = random_keypair();
    let vault = e.vault(debt);
    let next = e.vault(debt - burn);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, burn + change, false);
    let outs = vec![Tok::minter().arg(), Tok::to(&payer, change).arg()];
    let r = execute(
        vec![
            In {
                utxo: utxo(&vault, COLL, VAULT_COV),
                call: Call::Entry {
                    art: vault.clone(),
                    entry: "mint",
                    args: vec![ArtifactValue::Int(-burn), ArtifactValue::Int(1), ArtifactValue::Array(outs.clone())],
                    sig_by: Some((3, e.owner)),
                },
            },
            oracle_read(&e, ORACLE_COV, e.oracle),
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: outs, sig_by: None } },
            In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
        ],
        vec![
            out(&next, COLL, 0, VAULT_COV),
            out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV),
            out(&minter, 1_000, 2, GHOST_COV),
            out(&e.ghost(xonly(&payer), ID_PUBKEY, change, false), 1_000, 2, GHOST_COV),
        ],
    );
    assert!(r[0].is_err(), "{r:?}");
    assert!(r[2].is_ok() && r[3].is_ok(), "Token-Seite ist gültig, nur der Vault lehnt ab: {r:?}");
}

fn mint_with_minter_out(t: Tok) -> Vec<Result<(), String>> {
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.minter_out = Some(t);
    m.run(&e, 0)
}

#[test]
fn minter_fortsetzung_an_fremden_covenant_scheitert() {
    // L169: sonst wandert das Prägerecht an einen anderen Covenant
    let r = mint_with_minter_out(Tok { owner: OTHER_COV.as_bytes().to_vec(), typ: ID_COV, amount: 0, minter: true });
    assert!(r[0].is_err() && r[2].is_ok(), "{r:?}");
}

#[test]
fn minter_fortsetzung_mit_falschem_besitzertyp_scheitert() {
    // L168: gleiche Bytes, aber als Pubkey statt Covenant-ID
    let r = mint_with_minter_out(Tok { owner: VAULT_COV.as_bytes().to_vec(), typ: ID_PUBKEY, amount: 0, minter: true });
    assert!(r[0].is_err() && r[2].is_ok(), "{r:?}");
}

#[test]
fn minter_fortsetzung_ohne_minterrecht_scheitert() {
    // L167: Vault verlöre sein Prägerecht (Schuld wäre nicht mehr tilgbar)
    let r = mint_with_minter_out(Tok { owner: VAULT_COV.as_bytes().to_vec(), typ: ID_COV, amount: 0, minter: false });
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn minter_fortsetzung_mit_guthaben_scheitert() {
    // L170: Minter-Zweig hält keine GHOST
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.minter_out = Some(Tok { owner: VAULT_COV.as_bytes().to_vec(), typ: ID_COV, amount: 2 * E8, minter: true });
    m.recipient_amount = 8 * E8; // Summe bleibt 10 GHOST
    let r = m.run(&e, 0);
    assert!(r[0].is_err() && r[2].is_ok(), "{r:?}");
}

#[test]
fn doppelte_fortsetzung_scheitert() {
    // L118: zweite Vault-UTXO mit derselben Covenant-ID
    let e = env();
    let vault = e.vault(5 * E8);
    let r = execute(
        vec![In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "deposit", args: vec![], sig_by: Some((0, e.owner)) } }],
        vec![out(&vault, COLL + E8, 0, VAULT_COV), out(&e.vault(0), E8, 0, VAULT_COV)],
    );
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn deposit_ueber_der_obergrenze_scheitert() {
    // L115: oberhalb von 1e8 KAS gelten die Überlaufgrenzen der Rechnung nicht mehr
    let e = env();
    let max = 10_000_000_000_000_000i64;
    let ok = vault_only(&e, "deposit", vec![], Some(0), 0, COLL, Some(max), false, false);
    assert!(all_ok(&ok), "{ok:?}");
    let bad = vault_only(&e, "deposit", vec![], Some(0), 0, COLL, Some(max + 1), false, false);
    assert!(bad[0].is_err(), "{bad:?}");
}

#[test]
fn withdraw_auf_null_scheitert() {
    // L114: leeren heißt close, nicht eine 0-sompi-Fortsetzung
    let e = env();
    let r = vault_only(&e, "withdraw", vec![ArtifactValue::Int(0), ArtifactValue::Int(1)], Some(2), 0, COLL, Some(0), true, false);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn schuld_ueber_der_obergrenze_scheitert() {
    // bei 500 USD/KAS und 1e8 KAS Sicherheit wäre mehr Schuld gesund,
    // als die Rechnung sicher abbilden kann (MAX_DEBT = 1e9 GHOST)
    let o = Oracle { kas_usd: 50_000_000_000, index: INDEX };
    let e = Env::new(o);
    let coll = 10_000_000_000_000_000i64;
    let max_debt = 100_000_000_000_000_000i64;
    for (amount, should_pass) in [(max_debt, true), (max_debt + 1, false)] {
        assert!(healthy(coll, amount, o.kas_usd, MCR), "Fall muss an der Quote vorbei gesund sein");
        let mut m = Mint::new(&e, amount);
        m.coll = coll;
        let r = m.run(&e, 0);
        assert_eq!(r[0].is_ok(), should_pass, "amount {amount}: {r:?}");
    }
}

/// Größen für die Gebührenabschätzung. Ausgabe: cargo test --test vault_tests groessen -- --nocapture
#[test]
fn groessen() {
    let e = env();
    let v = e.vault(0);
    let k = e.ghost(vec![0; 32], ID_COV, 0, true);
    println!(
        "StableVault: Redeem-Skript {} B | KCC20: {} B | Orakel: {} B",
        bytecode(&v).len(),
        bytecode(&k).len(),
        bytecode(&e.oracle_art(e.oracle)).len()
    );
}

// ============================================================================
// Version 2 (nach dem Fable-Audit vom 28.09.2026)
// ============================================================================

#[test]
fn v2_einzahlen_nur_durch_besitzer() {
    // Audit 1 V-04: vorher konnte jeder die Vault-UTXO mit +1 sompi ausgeben
    let e = env();
    let vault = e.vault(5 * E8);
    let fremd = random_keypair();
    let r = execute(
        vec![In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "deposit", args: vec![], sig_by: Some((0, fremd)) } }],
        vec![out(&vault, COLL + E8, 0, VAULT_COV), plain_out(1)],
    );
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn tilgung_einer_einheit_senkt_die_schuld_um_eine_einheit() {
    // Audit 1 V-05 (in v2 tilgte 1 Einheit 0 Anteile): in v3 zählt jede Einheit
    let e = env();
    let debt = 100 * E8;
    let ok = burn_tx(&e, "repay", debt, Some(debt - 1), COLL, Some(COLL), 1, 0, e.oracle);
    assert!(all_ok(&ok), "{ok:?}");
    // Gegenprobe: verbrennen ohne Schuldabbau wird abgelehnt
    let r = burn_tx(&e, "repay", debt, Some(debt), COLL, Some(COLL), 1, 0, e.oracle);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn v2_teil_liquidation() {
    // Audit 1 V-02: Liquidator muss nicht die ganze Schuld aufbringen
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let burn = 40 * E8;
    let rest = COLL - seize(COLL, burn, p);
    let r = burn_tx(&e2, "liquidate", debt, Some(debt - burn), COLL, Some(rest), burn, 0, o);
    assert!(all_ok(&r), "{r:?}");
    // Gegenprobe: zu wenig Schuld abgebaut → abgelehnt
    let bad = burn_tx(&e2, "liquidate", debt, Some(debt - burn + 1), COLL, Some(rest), burn, 0, o);
    assert!(bad[0].is_err(), "{bad:?}");
}

#[test]
fn v2_teil_liquidation_ohne_schuldabbau_scheitert() {
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let rest = COLL - seize(COLL, 1, p);
    let r = burn_tx(&e2, "liquidate", debt, Some(debt), COLL, Some(rest), 1, 0, o);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn v2_liquidation_bei_unterdeckung_lohnt_sich_und_bucht_aus() {
    // Audit 1 V-02: Deckung unter 100 %. Vorher musste die GANZE Schuld
    // verbrannt werden (Verlustgeschäft). Jetzt: burn ≈ Wert/1,1 für die ganze
    // Sicherheit, Vault endet, Restschuld ist ausgebucht.
    let e = env();
    let debt = 150 * E8;
    let p = 1_000_000; // 0,01 USD → 10 000 KAS = 100 USD < Schuld
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let value = value_of(COLL, p);
    let burn = (value as u128 * 10_000 / 11_000) as i64 + 1; // claim ≥ Wert
    assert!(burn < debt);
    assert_eq!(seize(COLL, burn, p), COLL);
    let r = burn_tx(&e2, "liquidate", debt, None, COLL, None, burn, 0, o);
    assert!(all_ok(&r), "{r:?}");
    // Gegenprobe: mehr als die Schuld verbrennen ist nicht erlaubt
    let over = burn_tx(&e2, "liquidate", debt, None, COLL, None, debt + 1, 0, o);
    assert!(over[0].is_err(), "{over:?}");
}

// ------------------------------------------------ nach Test-Audit 5 ----

/// repay/deposit mit einer GHOST-Gruppe OHNE Ausgänge: der KCC20-Leader
/// (Minter-Zweig) wäre damit vernichtet.
fn with_empty_ghost_group(entry: &'static str) -> Vec<Result<(), String>> {
    let e = env();
    let debt = 10 * E8;
    let vault = e.vault(debt);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let (args, sig_by, with_oracle) = if entry == "repay" {
        (vec![ArtifactValue::Int(1), ArtifactValue::Array(vec![])], Some((2, e.owner)), true)
    } else {
        (vec![], Some((0, e.owner)), false)
    };
    let mut inputs = vec![In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry, args, sig_by } }];
    let mut outputs = vec![out(&vault, if entry == "deposit" { COLL + E8 } else { COLL }, 0, VAULT_COV)];
    if with_oracle {
        inputs.push(oracle_read(&e, ORACLE_COV, e.oracle));
        outputs.push(out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV));
    }
    inputs.push(In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: vec![], sig_by: None } });
    outputs.push(plain_out(1));
    execute(inputs, outputs)
}

#[test]
fn repay_ohne_ghost_ausgang_vernichtet_keinen_minter_zweig() {
    // L145 (nOut >= 1): 1 GHOST verbrennen und die GHOST-Gruppe OHNE Ausgänge
    // lassen – der Minter-Zweig des Vaults wäre vernichtet, der Vault könnte
    // nie mehr tilgen oder liquidiert werden.
    let e = env();
    let debt = 10 * E8;
    let payer = random_keypair();
    let vault = e.vault(debt);
    let next = e.vault(debt - E8);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, E8, false);
    let r = execute(
        vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "repay", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(vec![])], sig_by: Some((2, e.owner)) } },
            oracle_read(&e, ORACLE_COV, e.oracle),
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: vec![], sig_by: None } },
            In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
        ],
        vec![out(&next, COLL, 0, VAULT_COV), out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV), plain_out(1)],
    );
    assert!(r[0].is_err(), "Vault muss ablehnen: {r:?}");
    assert!(r[2].is_ok() && r[3].is_ok(), "die Token-Seite allein erlaubt es: {r:?}");
}

#[test]
fn einzahlen_mit_leerer_ghost_gruppe_scheitert() {
    // L132 allein (ohne L133): deposit + Minter-Leader ohne Ausgänge
    let r = with_empty_ghost_group("deposit");
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn liquidationsgrenze_auf_die_einheit_genau() {
    // need = ceil(Schuld · 1,5): bei ungerader Schuld unterscheiden sich Auf-
    // und Abrunden um eine Einheit (mulDivUp in healthy).
    let e = env();
    let debt = 100 * E8 + 1;
    let need = (debt as u128 * 15_000).div_ceil(10_000) as i64; // = floor + 1
    // 1 KAS Sicherheit: Wert = Preis (in GHOST-Einheiten), also exakt einstellbar
    let coll = E8;
    let price = need - 1;
    assert_eq!(value_of(coll, price), need - 1, "Wert muss exakt need−1 sein");
    assert!(!healthy(coll, debt, price, LIQ), "Referenz: ungesund");
    let o = Oracle { kas_usd: price, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    // Liquidator erhält Schuld·1,1, der Rest (≥ 0,2 KAS) bleibt schuldenfrei im Vault
    let rest = coll - seize(coll, debt, price);
    assert!(rest >= DUST);
    let r = burn_tx(&e2, "liquidate", debt, Some(0), coll, Some(rest), debt, 0, o);
    assert!(all_ok(&r), "bei Wert = need−1 muss Liquidation erlaubt sein: {r:?}");
    // Gegenprobe eine Einheit darüber (Wert = need): gesund, keine Liquidation
    let o2 = Oracle { kas_usd: need, index: INDEX };
    let e3 = Env { oracle: o2, ..env() };
    let rest2 = coll - seize(coll, debt, need);
    let r2 = burn_tx(&e3, "liquidate", debt, Some(0), coll, Some(rest2), debt, 0, o2);
    assert!(r2[0].is_err(), "bei Wert = need ist der Vault gesund: {r2:?}");
}

#[test]
fn liquidation_mit_falschem_burn_argument_scheitert() {
    // burn-Argument muss der tatsächlich verbrannten Menge entsprechen,
    // sonst gäbe es die Sicherheit für die volle Schuld gegen einen Bruchteil
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let rest = COLL - seize(COLL, debt, p);
    let s = St::from(debt);
    let done = Some(after("liquidate", s, 0, INDEX));
    // behauptet volle Schuld, verbrennt aber nur 1 GHOST
    let r = burn_tx_full(&e2, "liquidate", s, done, COLL, Some(rest), E8, 0, o, None, Some(debt));
    assert!(r[0].is_err(), "{r:?}");
    // Gegenprobe: ehrlich
    let ok = burn_tx_full(&e2, "liquidate", s, done, COLL, Some(rest), debt, 0, o, None, Some(debt));
    assert!(all_ok(&ok), "{ok:?}");
}

#[test]
fn kleiner_rest_nur_bei_voller_tilgung() {
    // Mutationstest v2: Vault mit < 0,2 KAS Sicherheit. Vorher beendete jede
    // Liquidation (auch mit 1 Einheit) den Vault und buchte die ganze Schuld aus.
    let e = env();
    let coll = 10_000_000; // 0,1 KAS
    let debt = 315_000;
    assert!(!healthy(coll, debt, PRICE, LIQ), "Szenario muss liquidierbar sein");
    let claim_full = (debt as u128 * 11_000).div_ceil(10_000) as i64;
    assert!(value_of(coll, PRICE) > claim_full, "Sicherheit deckt den Anspruch – kein Ausbuchen");
    // Teilbetrag, Vault soll enden → abgelehnt
    let r = burn_tx(&e, "liquidate", debt, None, coll, None, debt / 3, 0, e.oracle);
    assert!(r[0].is_err(), "Teilbetrag darf den Vault nicht beenden: {r:?}");
    // volle Schuld → Vault endet, Rest geht an den Liquidator
    let ok = burn_tx(&e, "liquidate", debt, None, coll, None, debt, 0, e.oracle);
    assert!(all_ok(&ok), "{ok:?}");
}

// ------------------------------------------------ nach Fix-Review (v2.1) ----

/// repay mit wählbarem Unterzeichner und wählbarem KAS-Wert der Minter-Fortsetzung
fn repay_variant(signer: Keypair, branch_out_value: i64) -> Vec<Result<(), String>> {
    let e = env();
    let debt = 10 * E8;
    let payer = random_keypair();
    let vault = e.vault(debt);
    let next = e.vault(debt - E8);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, E8, false);
    let states = vec![Tok::minter().arg()];
    execute(
        vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "repay", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(states.clone())], sig_by: Some((2, signer)) } },
            oracle_read(&e, ORACLE_COV, e.oracle),
            In { utxo: utxo(&minter, 300_000_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states, sig_by: None } },
            In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
        ],
        vec![out(&next, COLL, 0, VAULT_COV), out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV), out(&minter, branch_out_value, 2, GHOST_COV), plain_out(1)],
    )
}

#[test]
fn v21_repay_nur_durch_besitzer() {
    // Fix-Review N-4: Fremdtilgung sperrte den Besitzer aus
    let e = env();
    let r_ok = burn_tx(&e, "repay", 10 * E8, Some(9 * E8), COLL, Some(COLL), E8, 0, e.oracle);
    assert!(all_ok(&r_ok), "Besitzer darf tilgen: {r_ok:?}");
    let r = repay_variant(random_keypair(), 300_000_000);
    assert!(r[0].is_err(), "Fremder darf nicht tilgen: {r:?}");
}

#[test]
fn v21_minter_zweig_behaelt_seine_kas() {
    // Fix-Review N-11: vorher ließen sich die 3 KAS des Zweigs beim Tilgen abziehen.
    // Hier signiert der Besitzer korrekt, nur der Zweig-Wert ist verbogen.
    let e = env();
    let debt = 10 * E8;
    let payer = random_keypair();
    let vault = e.vault(debt);
    let next = e.vault(debt - E8);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, E8, false);
    let states = vec![Tok::minter().arg()];
    let run = |branch_out: i64| {
        execute(
            vec![
                In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "repay", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(states.clone())], sig_by: Some((2, e.owner)) } },
                oracle_read(&e, ORACLE_COV, e.oracle),
                In { utxo: utxo(&minter, 300_000_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states.clone(), sig_by: None } },
                In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
            ],
            vec![out(&next, COLL, 0, VAULT_COV), out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV), out(&minter, branch_out, 2, GHOST_COV), plain_out(1)],
        )
    };
    let ok = run(300_000_000);
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
    let bad = run(1_000);
    assert!(bad[0].is_err(), "Zweig-KAS dürfen nicht abfließen: {bad:?}");
}

#[test]
fn v21_kleiner_rest_ohne_weiterleben_der_covenant_id() {
    // Fix-Review N-12: im Zweig "Rest < 0,2 KAS, volle Tilgung" darf keine
    // Fortsetzung entstehen (sonst Covenant-ID mit beliebigem Skript = Minter-Hintertür)
    let e = env();
    let coll = 10_000_000;
    let debt = 315_000;
    let ok = burn_tx(&e, "liquidate", debt, None, coll, None, debt, 0, e.oracle);
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
    let r = burn_tx_ext(&e, "liquidate", debt, None, coll, None, debt, 0, e.oracle, Some(rogue_vault_out(1_000_000)));
    assert!(r[0].is_err(), "{r:?}");
}

// ============================================================================
// Version 3: Zins als eigener Posten, Zinskasse, Rücknahme zu 1 USD
// ============================================================================

#[test]
fn v3_zins_waechst_mit_dem_index_und_wird_beim_praegen_verbucht() {
    let e = env();
    // 100 GHOST seit Index 1,00 → bei 1,05 sind 5 USD Zins aufgelaufen
    let s = St { debt: 100 * E8, interest: 0, at: INDEX0 };
    assert_eq!(s.accrued(INDEX), 5 * E8);
    let ok = Mint::new(&e, 10 * E8).run(&e, s);
    assert!(all_ok(&ok), "{ok:?}");
    // Gegenprobe: Zins weggelassen
    let mut m = Mint::new(&e, 10 * E8);
    m.new_state = Some(St { debt: 110 * E8, interest: 0, at: INDEX });
    assert!(m.run(&e, s)[0].is_err(), "Zins darf nicht verschwinden");
    // Gegenprobe: Index nicht nachgezogen (Zins würde doppelt zählen)
    m.new_state = Some(St { debt: 110 * E8, interest: 5 * E8, at: INDEX0 });
    assert!(m.run(&e, s)[0].is_err(), "indexAt muss auf den Orakelindex");
    // Gegenprobe: Zins als Schuld gebucht (v2-Fehler: Schuld ohne GHOST)
    m.new_state = Some(St { debt: 115 * E8, interest: 0, at: INDEX });
    assert!(m.run(&e, s)[0].is_err(), "Zins ist keine GHOST-Schuld");
}

#[test]
fn v3_zins_zaehlt_fuer_die_mindestquote() {
    let e = env();
    // 400 USD Sicherheit tragen 200 USD; 100 GHOST + 50 USD Zins → 50 GHOST frei
    let s = St { debt: 100 * E8, interest: 50 * E8, at: INDEX };
    assert_eq!(max_mint(COLL, s, e.oracle), 50 * E8);
    assert_eq!(max_mint(COLL, St::from(100 * E8), e.oracle), 100 * E8, "ohne Zins wären es 100");
    assert!(all_ok(&Mint::new(&e, 50 * E8).run(&e, s)));
    let r = Mint::new(&e, 50 * E8 + 1).run(&e, s);
    assert!(r[0].is_err() && r[1].is_ok() && r[2].is_ok(), "{r:?}");
}

#[test]
fn v3_zins_zaehlt_fuer_die_auszahlung() {
    let e = env();
    let s = St { debt: 50 * E8, interest: 25 * E8, at: INDEX };
    let min = min_coll(75 * E8, PRICE, MCR); // 150 USD = 3 750 KAS
    assert_eq!(min, 3_750 * E8);
    let arg = |c: i64| vec![ArtifactValue::Int(c), ArtifactValue::Int(1)];
    assert!(all_ok(&vault_only(&e, "withdraw", arg(min), Some(2), s, COLL, Some(min), true, false)));
    let bad = vault_only(&e, "withdraw", arg(min - 1), Some(2), s, COLL, Some(min - 1), true, false);
    assert!(bad[0].is_err(), "{bad:?}");
}

#[test]
fn v3_obergrenze_gilt_nur_fuer_die_gepraegte_schuld() {
    // 50 GHOST je Vault begrenzen die umlaufenden GHOST, nicht den Zins
    let mut e = env();
    e.max_debt = 50 * E8;
    let s = St { debt: 30 * E8, interest: 10 * E8, at: INDEX };
    let run = |amount: i64| {
        let mut m = Mint::new(&e, amount);
        m.coll = 1_000_000 * E8;
        m.run(&e, s)
    };
    assert!(all_ok(&run(20 * E8)));
    assert!(run(20 * E8 + 1)[0].is_err());
}

#[test]
fn v3_tilgen_laesst_den_zins_stehen() {
    let e = env();
    // 10 GHOST + 3 USD Zins seit Index 1,00 → bei 1,05: 3 + 13 · 5 % = 3,65 USD
    let s = St { debt: 10 * E8, interest: 3 * E8, at: INDEX0 };
    assert_eq!(s.accrued(INDEX), 365 * E8 / 100);
    let ok = burn_tx(&e, "repay", s, Some(0), COLL, Some(COLL), 10 * E8, 0, e.oracle);
    assert!(all_ok(&ok), "{ok:?}");
    // Gegenprobe: Zins beim Tilgen gestrichen
    let gone = burn_tx_full(&e, "repay", s, Some(St { debt: 0, interest: 0, at: INDEX }), COLL, Some(COLL), 10 * E8, 0, e.oracle, None, None);
    assert!(gone[0].is_err(), "{gone:?}");
    // Zins lässt sich nicht mit GHOST tilgen: mehr als die Schuld verbrennen scheitert
    let over = burn_tx(&e, "repay", s, Some(0), COLL, Some(COLL), 13 * E8, 0, e.oracle);
    assert!(over[0].is_err(), "{over:?}");
}

/// Zins in sompi beim Schließen (aufgerundet, höchstens die Sicherheit)
fn close_fee(s: St, coll: i64, price: i64) -> i64 {
    ((s.accrued(INDEX) as u128 * E8 as u128).div_ceil(price as u128) as i64).min(coll)
}

#[test]
fn v3_schliessen_zahlt_den_zins_an_die_zinskasse() {
    let e = env();
    let s = St { debt: 0, interest: 2 * E8, at: INDEX };
    let fee = close_fee(s, COLL, PRICE);
    assert_eq!(fee, 50 * E8, "2 USD bei 0,04 USD/KAS = 50 KAS");
    let ok = close_tx(&e, s, COLL, e.owner, vec![p2pk_out(&e.treasury, fee), plain_out(COLL - fee)]);
    assert!(all_ok(&ok), "{ok:?}");
    // Gegenproben: zu wenig, falscher Empfänger, gar keine Zahlung
    let short = close_tx(&e, s, COLL, e.owner, vec![p2pk_out(&e.treasury, fee - 1), plain_out(COLL - fee + 1)]);
    assert!(short[0].is_err(), "{short:?}");
    let thief = random_keypair();
    let other = close_tx(&e, s, COLL, e.owner, vec![p2pk_out(&thief, fee), plain_out(COLL - fee)]);
    assert!(other[0].is_err(), "{other:?}");
    let none = close_tx(&e, s, COLL, e.owner, vec![plain_out(COLL)]);
    assert!(none[0].is_err(), "{none:?}");
    // Kassen-Ausgang an anderer Stelle (hinter dem Rest) zählt nicht
    let moved = close_tx(&e, s, COLL, e.owner, vec![plain_out(COLL - fee), p2pk_out(&e.treasury, fee)]);
    assert!(moved[0].is_err(), "{moved:?}");
}

#[test]
fn v3_schliessen_rechnet_den_zins_bis_zum_orakelindex() {
    // Offener Zins verzinst sich bis zum Orakelindex mit, auch ohne Schuld
    // (Audit 11 A11-V-5): 2 USD seit Index 1,00 → 2,10 USD bei 1,05
    let e = env();
    let s = St { debt: 0, interest: 2 * E8, at: INDEX0 };
    assert_eq!(s.accrued(INDEX), 21 * E8 / 10);
    let fee = close_fee(s, COLL, PRICE);
    assert_eq!(fee, 5_250_000_000, "2,10 USD bei 0,04 USD/KAS = 52,5 KAS");
    let short = close_tx(&e, s, COLL, e.owner, vec![p2pk_out(&e.treasury, 50 * E8), plain_out(COLL - 50 * E8)]);
    assert!(short[0].is_err(), "nur der verbuchte Zins reicht nicht: {short:?}");
    let ok = close_tx(&e, s, COLL, e.owner, vec![p2pk_out(&e.treasury, fee), plain_out(COLL - fee)]);
    assert!(all_ok(&ok), "{ok:?}");
}

#[test]
fn v3_zins_unter_staub_wird_erlassen() {
    let e = env();
    // 0,007 USD bei 0,04 USD/KAS = 0,175 KAS < 0,2 KAS → erlassen
    let small = St { debt: 0, interest: 700_000, at: INDEX };
    assert!(close_fee(small, COLL, PRICE) < DUST);
    let ok = close_tx(&e, small, COLL, e.owner, vec![plain_out(COLL)]);
    assert!(all_ok(&ok), "{ok:?}");
    // genau an der Grenze (0,008 USD = 0,2 KAS) muss gezahlt werden
    let edge = St { debt: 0, interest: 800_000, at: INDEX };
    assert_eq!(close_fee(edge, COLL, PRICE), DUST);
    let bad = close_tx(&e, edge, COLL, e.owner, vec![plain_out(COLL)]);
    assert!(bad[0].is_err(), "{bad:?}");
    let paid = close_tx(&e, edge, COLL, e.owner, vec![p2pk_out(&e.treasury, DUST), plain_out(COLL - DUST)]);
    assert!(all_ok(&paid), "{paid:?}");
}

#[test]
fn v3_zins_hoechstens_die_sicherheit() {
    // Übersteigt der Zins die Sicherheit, geht alles an die Kasse — nicht mehr
    let e = env();
    let coll = E8; // 1 KAS = 0,04 USD
    let s = St { debt: 0, interest: E8, at: INDEX }; // 1 USD Zins
    assert_eq!(close_fee(s, coll, PRICE), coll);
    let ok = close_tx(&e, s, coll, e.owner, vec![p2pk_out(&e.treasury, coll)]);
    assert!(all_ok(&ok), "{ok:?}");
    let short = close_tx(&e, s, coll, e.owner, vec![p2pk_out(&e.treasury, coll - 1), plain_out(1)]);
    assert!(short[0].is_err(), "{short:?}");
}

/// KAS für `amount` GHOST bei der Rücknahme (stable_vault.sil redeem)
fn redeem_paid(amount: i64, price: i64) -> i64 {
    let usd = (amount as u128 * 9_900 / 10_000) as i64;
    (usd as u128 * E8 as u128 / price as u128) as i64
}

#[test]
fn v3_ruecknahme_zahlt_einen_dollar_je_ghost() {
    let e = env();
    let debt = 100 * E8;
    let amount = 10 * E8;
    let paid = redeem_paid(amount, PRICE);
    assert_eq!(paid, 24_750_000_000, "10 GHOST × 0,99 USD / 0,04 USD = 247,5 KAS");
    // jeder darf zurückgeben (keine Signatur des Besitzers)
    let ok = burn_tx(&e, "redeem", debt, Some(debt - amount), COLL, Some(COLL - paid), amount, 0, e.oracle);
    assert!(all_ok(&ok), "{ok:?}");
    // mit Wechselgeld
    let ch = burn_tx(&e, "redeem", debt, Some(debt - amount), COLL, Some(COLL - paid), amount + 3 * E8, 3 * E8, e.oracle);
    assert!(all_ok(&ch), "{ch:?}");
    // Gegenproben: 1 sompi zu viel entnommen, Schuld nicht gesenkt, zu stark gesenkt
    let greedy = burn_tx(&e, "redeem", debt, Some(debt - amount), COLL, Some(COLL - paid - 1), amount, 0, e.oracle);
    assert!(greedy[0].is_err(), "{greedy:?}");
    let keep = burn_tx(&e, "redeem", debt, Some(debt), COLL, Some(COLL - paid), amount, 0, e.oracle);
    assert!(keep[0].is_err(), "{keep:?}");
    let wipe = burn_tx(&e, "redeem", debt, Some(debt - amount - 1), COLL, Some(COLL - paid), amount, 0, e.oracle);
    assert!(wipe[0].is_err(), "{wipe:?}");
}

#[test]
fn v3_ruecknahme_mit_falschem_betrag_scheitert() {
    // behauptet 10 GHOST, verbrennt aber nur 5
    let e = env();
    let s = St::from(100 * E8);
    let next = Some(s.then(90 * E8, INDEX));
    let paid = redeem_paid(10 * E8, PRICE);
    let r = burn_tx_full(&e, "redeem", s, next, COLL, Some(COLL - paid), 5 * E8, 0, e.oracle, None, Some(10 * E8));
    assert!(r[0].is_err(), "{r:?}");
    let ok = burn_tx_full(&e, "redeem", s, next, COLL, Some(COLL - paid), 10 * E8, 0, e.oracle, None, Some(10 * E8));
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
}

#[test]
fn v3_ruecknahme_hoechstens_die_schuld() {
    let e = env();
    let debt = 5 * E8;
    let ok = burn_tx(&e, "redeem", debt, Some(0), COLL, Some(COLL - redeem_paid(debt, PRICE)), debt, 0, e.oracle);
    assert!(all_ok(&ok), "ganze Schuld zurücknehmen geht: {ok:?}");
    let paid = redeem_paid(debt + 1, PRICE);
    let r = burn_tx_full(&e, "redeem", St::from(debt), Some(St::from(0)), COLL, Some(COLL - paid), debt + 1, 0, e.oracle, None, None);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn v3_ruecknahme_nur_ab_der_liquidationsschwelle() {
    // Darunter sind Liquidatoren dran (mit Bonus); sonst ließe sich ein
    // kranker Vault billiger ausräumen als liquidieren.
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let amount = 10 * E8;
    let sick = Oracle { kas_usd: p, index: INDEX };
    let e_sick = Env { oracle: sick, ..env() };
    let r = burn_tx(&e_sick, "redeem", debt, Some(debt - amount), COLL, Some(COLL - redeem_paid(amount, p)), amount, 0, sick);
    assert!(r[0].is_err(), "{r:?}");
    // Gegenprobe: 0,00001 USD höher ist der Vault gesund, Rücknahme geht
    let fine = Oracle { kas_usd: p + 1_000, index: INDEX };
    assert!(healthy(COLL, debt, fine.kas_usd, LIQ));
    let e_fine = Env { oracle: fine, ..e };
    let ok = burn_tx(&e_fine, "redeem", debt, Some(debt - amount), COLL, Some(COLL - redeem_paid(amount, fine.kas_usd)), amount, 0, fine);
    assert!(all_ok(&ok), "{ok:?}");
}

#[test]
fn v3_ruecknahme_laesst_mindestens_staub_im_vault() {
    // 100 USD/KAS: 0,3 KAS = 30 USD tragen 20 GHOST bei 150 %
    let price = 10_000_000_000;
    let o = Oracle { kas_usd: price, index: INDEX };
    let e = Env::new(o);
    let coll = 30_000_000;
    let debt = 20 * E8;
    assert!(healthy(coll, debt, price, LIQ));
    // alles zurück: 0,198 KAS an den Rücknehmer, Rest 0,102 KAS < 0,2 KAS
    let paid = redeem_paid(debt, price);
    assert!(coll - paid < DUST);
    let r = burn_tx(&e, "redeem", debt, Some(0), coll, Some(coll - paid), debt, 0, o);
    assert!(r[0].is_err(), "{r:?}");
    // Gegenprobe: 5 GHOST, Rest ≥ 0,2 KAS
    let part = 5 * E8;
    let paid = redeem_paid(part, price);
    assert!(coll - paid >= DUST);
    let ok = burn_tx(&e, "redeem", debt, Some(debt - part), coll, Some(coll - paid), part, 0, o);
    assert!(all_ok(&ok), "{ok:?}");
}

#[test]
fn v3_ruecknahme_mindestens_ein_ghost_oder_die_ganze_schuld() {
    // Audit 11 A11-V-3: sonst bewegt jeder mit 2 Einheiten fremde Vaults
    let e = env();
    let debt = 100 * E8;
    for (amount, ok) in [(E8 - 1, false), (2, false), (E8, true)] {
        let r = burn_tx(&e, "redeem", debt, Some(debt - amount), COLL, Some(COLL - redeem_paid(amount, PRICE)), amount, 0, e.oracle);
        assert_eq!(r[0].is_ok(), ok, "{amount}: {r:?}");
    }
    // kleiner Vault: die ganze Schuld (0,5 GHOST) geht
    let r = burn_tx(&e, "redeem", E8 / 2, Some(0), COLL, Some(COLL - redeem_paid(E8 / 2, PRICE)), E8 / 2, 0, e.oracle);
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn v3_ruecknahme_verbucht_den_zins() {
    let e = env();
    let s = St { debt: 100 * E8, interest: 0, at: INDEX0 };
    let amount = 10 * E8;
    let paid = redeem_paid(amount, PRICE);
    let ok = burn_tx(&e, "redeem", s, Some(90 * E8), COLL, Some(COLL - paid), amount, 0, e.oracle);
    assert!(all_ok(&ok), "{ok:?}");
    let lost = burn_tx_full(&e, "redeem", s, Some(St { debt: 90 * E8, interest: 0, at: INDEX }), COLL, Some(COLL - paid), amount, 0, e.oracle, None, None);
    assert!(lost[0].is_err(), "Zins darf bei der Rücknahme nicht verschwinden: {lost:?}");
}

#[test]
fn v3_zins_macht_liquidierbar() {
    // Schuld allein wäre gedeckt, Schuld + Zins nicht → Liquidation erlaubt
    let e = env();
    let with = St { debt: 150 * E8, interest: 30 * E8, at: INDEX };
    let p = crash_price(COLL, with);
    assert!(healthy(COLL, 150 * E8, p, LIQ), "ohne Zins gesund");
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let rest = COLL - seize(COLL, 150 * E8, p);
    let ok = burn_tx(&e2, "liquidate", with, Some(0), COLL, Some(rest), 150 * E8, 0, o);
    assert!(all_ok(&ok), "{ok:?}");
    // Gegenprobe: derselbe Vault ohne Zins ist nicht liquidierbar
    let r = burn_tx(&e2, "liquidate", 150 * E8, Some(0), COLL, Some(rest), 150 * E8, 0, o);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn v3_teil_liquidation_laesst_den_zins_stehen() {
    // Der Zins gehört der Zinskasse; der Liquidator tilgt nur geprägte GHOST.
    // (Die erste Fassung kürzte den Zins anteilig – 30e8 · 110e8 lief dabei
    // über 64 Bit; dieser Test hat es gefunden.)
    let e = env();
    let s = St { debt: 150 * E8, interest: 30 * E8, at: INDEX0 };
    let acc = s.accrued(INDEX); // 30 + (150 + 30) · 5 % = 39 USD
    assert_eq!(acc, 39 * E8);
    let p = crash_price(COLL, s);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let burn = 40 * E8;
    let rest = COLL - seize(COLL, burn, p);
    let next = St { debt: 110 * E8, interest: acc, at: INDEX };
    assert_eq!(after("liquidate", s, 110 * E8, INDEX), next);
    let ok = burn_tx(&e2, "liquidate", s, Some(110 * E8), COLL, Some(rest), burn, 0, o);
    assert!(all_ok(&ok), "{ok:?}");
    for interest in [0, 30 * E8, 375 * E8 / 10, acc - 1] {
        let bad = burn_tx_full(&e2, "liquidate", s, Some(St { interest, ..next }), COLL, Some(rest), burn, 0, o, None, None);
        assert!(bad[0].is_err(), "Zins {interest}: {bad:?}");
    }
    // volle Liquidation mit Rest: schuldenfrei, Zins bleibt bis zum Schließen
    let rest_full = COLL - seize(COLL, 150 * E8, p);
    let full = burn_tx(&e2, "liquidate", s, Some(0), COLL, Some(rest_full), 150 * E8, 0, o);
    assert!(all_ok(&full), "{full:?}");
    let wiped = burn_tx_full(&e2, "liquidate", s, Some(St { debt: 0, interest: 0, at: INDEX }), COLL, Some(rest_full), 150 * E8, 0, o, None, None);
    assert!(wiped[0].is_err(), "{wiped:?}");
}

// ============================================================================
// Nach dem Mutationstest von Version 3 (29.09.2026): Regeln, die nur sie selbst
// abfangen. Jeder Test prüft, dass genau der Vault (Input 0) ablehnt.
// ============================================================================

#[test]
fn v3_zins_ueber_der_rechengrenze_scheitert() {
    // L155: MAX_INTEREST (1e9 USD) hält die Rechnungen überlauffrei. Mit
    // 1e8 KAS bei 500 USD ist der Vault an der Quote vorbei gesund.
    let o = Oracle { kas_usd: 50_000_000_000, index: INDEX };
    let e = Env::new(o);
    let coll = 10_000_000_000_000_000i64;
    let max = 100_000_000_000_000_000i64;
    let arg = |c: i64| vec![ArtifactValue::Int(c), ArtifactValue::Int(1)];
    for (interest, ok) in [(max, true), (max + 1, false)] {
        let s = St { debt: 0, interest, at: INDEX };
        assert!(healthy(coll - E8, interest, o.kas_usd, MCR));
        let r = vault_only(&e, "withdraw", arg(coll - E8), Some(2), s, coll, Some(coll - E8), true, false);
        assert_eq!(r[0].is_ok(), ok, "Zins {interest}: {r:?}");
    }
}

#[test]
fn withdraw_darf_nicht_einzahlen() {
    // L237: withdraw mit mehr Sicherheit als vorher wäre ein deposit ohne dessen Regeln
    let e = env();
    let r = vault_only(&e, "withdraw", vec![ArtifactValue::Int(COLL + E8), ArtifactValue::Int(1)], Some(2), 10 * E8, COLL, Some(COLL + E8), true, false);
    assert!(r[0].is_err(), "{r:?}");
    let ok = vault_only(&e, "withdraw", vec![ArtifactValue::Int(COLL - E8), ArtifactValue::Int(1)], Some(2), 10 * E8, COLL, Some(COLL - E8), true, false);
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
}

/// Prägen mit einem anderen GHOST-Eingang 0 als dem eigenen Minter-Zweig
fn mint_with_leader(t: Tok) -> Vec<Result<(), String>> {
    let e = env();
    let mut m = Mint::new(&e, 10 * E8);
    m.minter_in = Some(t);
    m.run(&e, 0)
}

#[test]
fn praegen_nur_ueber_den_eigenen_minter_zweig() {
    let own = VAULT_COV.as_bytes().to_vec();
    assert!(all_ok(&mint_with_leader(Tok::minter())), "Gegenprobe");
    // L197: Minter-Zweig eines anderen Covenants
    let r = mint_with_leader(Tok { owner: OTHER_COV.as_bytes().to_vec(), typ: ID_COV, amount: 0, minter: true });
    assert!(r[0].is_err(), "fremder Minter: {r:?}");
    // L196: gleiche Bytes, aber als Pubkey-Besitzer
    let r = mint_with_leader(Tok { owner: own.clone(), typ: ID_PUBKEY, amount: 0, minter: true });
    assert!(r[0].is_err(), "falscher Besitzertyp: {r:?}");
    // L195: gewöhnlicher Token (kein Minter) unter der eigenen ID
    let r = mint_with_leader(Tok { owner: own, typ: ID_COV, amount: 0, minter: false });
    assert!(r[0].is_err(), "kein Minter: {r:?}");
}

#[test]
fn tilgen_mit_zweitem_minter_eingang_scheitert() {
    // L199: nur Eingang 0 darf ein Minter sein. Ein zweiter Minter-Zweig
    // (hier der eines anderen Covenants) würde dessen Prägerecht mitbewegen.
    let e = env();
    let payer = random_keypair();
    let vault = e.vault(10 * E8);
    let next = e.vault(9 * E8);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let foreign = e.ghost(OTHER_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, E8, false);
    let states = vec![Tok::minter().arg()];
    let run = |with_foreign: bool| {
        let mut inputs = vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "repay", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(states.clone())], sig_by: Some((2, e.owner)) } },
            oracle_read(&e, ORACLE_COV, e.oracle),
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states.clone(), sig_by: None } },
        ];
        if with_foreign {
            inputs.push(In { utxo: utxo(&foreign, 1_000, GHOST_COV), call: Call::GhostDelegate { art: foreign.clone(), sig_by: payer } });
        }
        inputs.push(In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } });
        execute(inputs, vec![out(&next, COLL, 0, VAULT_COV), out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV), out(&minter, 1_000, 2, GHOST_COV), plain_out(1)])
    };
    assert!(all_ok(&run(false)), "Gegenprobe");
    let r = run(true);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn tilgen_mit_leerem_ghost_ausgang_scheitert() {
    // L214: gewöhnliche GHOST-Ausgänge brauchen einen Betrag > 0
    let e = env();
    let payer = random_keypair();
    let vault = e.vault(10 * E8);
    let next = e.vault(9 * E8);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, E8, false);
    let run = |change: i64| {
        let states = vec![Tok::minter().arg(), Tok::to(&payer, change).arg()];
        execute(
            vec![
                In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "repay", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(states.clone())], sig_by: Some((2, e.owner)) } },
                oracle_read(&e, ORACLE_COV, e.oracle),
                In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states, sig_by: None } },
                In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
            ],
            vec![
                out(&next, COLL, 0, VAULT_COV),
                out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV),
                out(&minter, 1_000, 2, GHOST_COV),
                out(&e.ghost(xonly(&payer), ID_PUBKEY, change, false), 1_000, 2, GHOST_COV),
            ],
        )
    };
    let r = run(0);
    assert!(r[0].is_err(), "{r:?}");
}

/// mulDivUp/mulDivDown wie die Skript-Engine (Division und Rest schneiden
/// Richtung 0 ab, wie i64 in Rust) – auch für negative Werte
fn script_mul_div_up(a: i64, b: i64, d: i64) -> i64 {
    (a / d) * b + ((a % d) * b + d - 1) / d
}

#[test]
fn liquidation_mit_negativem_betrag_praegt_nicht() {
    // L319: burn < 0 hieße „GHOST prägen per liquidate“. Der Zustand ist hier
    // genau so gebaut, wie ihn der Vault ohne die Regel verlangen würde.
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let minted = 10 * E8;
    let burn = -minted;
    let claim = script_mul_div_up(burn, 10_000 + BONUS, 10_000);
    let seize = script_mul_div_up(claim, E8, p);
    let rest = COLL - seize;
    assert!(rest > COLL, "der Angreifer müsste KAS nachlegen");
    let s = St::from(debt);
    let next = St { debt: debt - burn, interest: 0, at: INDEX };
    // Zahler ohne Eingang: 1 Einheit rein, 1 + minted heraus
    let r = burn_tx_full(&e2, "liquidate", s, Some(next), COLL, Some(rest), 1, 1 + minted, o, None, Some(burn));
    assert!(r[0].is_err(), "{r:?}");
    assert!(r[2].is_ok() && r[3].is_ok(), "die Token-Seite ließe das Prägen zu: {r:?}");
}

#[test]
fn ruecknahme_mit_negativem_betrag_praegt_nicht() {
    // L295/L304: amount < 0 hieße „GHOST prägen per redeem“ (beide Regeln fangen
    // es ab, deshalb überlebt jede einzeln den Mutationstest)
    let e = env();
    let minted = 10 * E8;
    let s = St::from(100 * E8);
    let r = burn_tx_full(&e, "redeem", s, Some(St::from(110 * E8)), COLL, Some(COLL), 1, 1 + minted, e.oracle, None, Some(-minted));
    assert!(r[0].is_err(), "{r:?}");
}

// ============================================================================
// Audit 11 (Opus, 29.09.2026): Befunde A11-V-1 … V-10 und ihre Behebung
// ============================================================================

const VAULT2_COV: Hash = Hash::from_bytes([0x0e; 32]);

/// Zwei Vaults (Eingänge 0 und 1) schließen in einer Tx; Orakel ist Eingang 2.
/// Die Kasse von Vault i muss an Ausgang i + 1 stehen.
fn close_two(e: &Env, s1: St, s2: St, outputs_after_oracle: Vec<TransactionOutput>) -> Vec<Result<(), String>> {
    let v1 = e.vault(s1);
    let v2 = e.vault(s2);
    let mut outputs = outputs_after_oracle;
    outputs.push(out(&e.oracle_art(e.oracle), E8, 2, ORACLE_COV));
    execute(
        vec![
            In { utxo: utxo(&v1, COLL, VAULT_COV), call: Call::Entry { art: v1.clone(), entry: "close", args: vec![ArtifactValue::Int(2)], sig_by: Some((1, e.owner)) } },
            In { utxo: utxo(&v2, COLL, VAULT2_COV), call: Call::Entry { art: v2.clone(), entry: "close", args: vec![ArtifactValue::Int(2)], sig_by: Some((1, e.owner)) } },
            oracle_read(e, ORACLE_COV, e.oracle),
        ],
        outputs,
    )
}

#[test]
fn a11_zwei_vaults_teilen_sich_keine_zinszahlung() {
    // A11-V-1: vorher zeigten beide Vaults auf denselben Kassen-Ausgang, die
    // Kasse bekam nur die größere Einzelgebühr
    let e = env();
    let s1 = St { debt: 0, interest: 2 * E8, at: INDEX };
    let s2 = St { debt: 0, interest: 2 * E8 + 1, at: INDEX };
    let (f1, f2) = (close_fee(s1, COLL, PRICE), close_fee(s2, COLL, PRICE));
    let honest = close_two(&e, s1, s2, vec![plain_out(2 * COLL - f1 - f2), p2pk_out(&e.treasury, f1), p2pk_out(&e.treasury, f2)]);
    assert!(all_ok(&honest), "ehrlich: {honest:?}");
    // Angriff: ein Kassen-Ausgang mit max(f1, f2) an Stelle 1, an Stelle 2 geht der Rest weg
    let shared = close_two(&e, s1, s2, vec![plain_out(E8), p2pk_out(&e.treasury, f2), plain_out(2 * COLL - f2 - E8)]);
    assert!(shared[1].is_err(), "Vault 2 muss seinen eigenen Kassen-Ausgang verlangen: {shared:?}");
}

#[test]
fn a11_preis_null_nur_durch_l165_gesperrt() {
    // A11-V-6: ohne kasUsd > 0 nähme ein Liquidator mit 1 Einheit GHOST die
    // ganze Sicherheit eines gesunden Vaults
    let o = Oracle { kas_usd: 0, index: INDEX };
    let e = Env::new(o);
    let debt = 10 * E8;
    assert!(healthy(COLL, debt, PRICE, LIQ));
    let r = burn_tx(&e, "liquidate", debt, None, COLL, None, 1, 0, o);
    assert!(r[0].is_err(), "Vault muss ablehnen: {r:?}");
}

/// A11-V-7: drei GHOST-Ausgänge (Minter, Wechselgeld, 1 Mio GHOST an einen Dieb)
fn redeem_with_loot(e: &Env, loot: i64) -> Vec<Result<(), String>> {
    let payer = random_keypair();
    let thief = random_keypair();
    let debt = 100 * E8;
    let amount = 10 * E8;
    let s = St::from(debt);
    let vault = e.vault(s);
    let next = e.vault(s.then(debt - amount, INDEX));
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, amount + E8, false);
    let toks = vec![Tok::minter(), Tok::to(&payer, E8), Tok::to(&thief, loot)];
    let args: Vec<ArtifactValue> = toks.iter().map(Tok::arg).collect();
    let paid = redeem_paid(amount, PRICE);
    execute(
        vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "redeem", args: vec![ArtifactValue::Int(1), ArtifactValue::Int(amount), ArtifactValue::Array(args.clone())], sig_by: None } },
            oracle_read(e, ORACLE_COV, e.oracle),
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: args, sig_by: None } },
            In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
        ],
        vec![
            out(&next, COLL - paid, 0, VAULT_COV),
            out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV),
            out(&minter, 1_000, 2, GHOST_COV),
            out(&e.ghost(xonly(&payer), ID_PUBKEY, E8, false), 1_000, 2, GHOST_COV),
            out(&e.ghost(xonly(&thief), ID_PUBKEY, loot, false), 1_000, 2, GHOST_COV),
            plain_out(1),
        ],
    )
}

/// A11-V-7 ist eine Zweitsicherung (Audit 12 A12-15): Drei GHOST-Ausgänge
/// lehnt der Vault auch OHNE `require(nOut <= MAX_GHOST_OUTS)` ab (seine
/// Schleife über die Ausgänge ist auf MAX_GHOST_OUTS begrenzt), und der
/// KCC20-Leader lehnt sie ebenfalls ab. Die Mutante M03 (Regel → true)
/// überlebt deshalb; dieser Test belegt die Ablehnung, nicht die Regel allein.
#[test]
fn a11_drei_ghost_ausgaenge_bei_ruecknahme_zweitsicherung() {
    let r = redeem_with_loot(&env(), 1_000_000 * E8);
    println!("drei GHOST-Ausgänge: {r:?}");
    assert!(r[0].is_err(), "der Vault selbst muss ablehnen: {r:?}");
    assert!(r[2].is_err(), "und der KCC20-Leader ebenso: {r:?}");
}

#[test]
fn a11_gegenprobe_falsches_orakel_als_tx() {
    // A11-V-10: dieselben 1000 GHOST gehen mit dem ECHTEN Orakel bei
    // 100-fachem Preis durch – der Unterschied liegt nur an der Herkunft
    let e = env();
    let mut m = Mint::new(&e, 1_000 * E8);
    m.oracle = Oracle { kas_usd: PRICE * 100, index: INDEX };
    assert!(all_ok(&m.run(&e, 0)));
}

/// sweep(oracleIdx 1): Input 0 Vault, Input 1 Orakel; Ausgang 0 Orakel,
/// Ausgang 1 Kasse, danach `rest`
fn sweep_tx(e: &Env, s: St, coll: i64, rest: Vec<TransactionOutput>) -> Vec<Result<(), String>> {
    let vault = e.vault(s);
    let mut outputs = vec![out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV)];
    outputs.extend(rest);
    execute(
        vec![
            In { utxo: utxo(&vault, coll, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "sweep", args: vec![ArtifactValue::Int(1)], sig_by: None } },
            oracle_read(e, ORACLE_COV, e.oracle),
        ],
        outputs,
    )
}

const SWEEP_FEE: i64 = 10_000_000;

#[test]
fn a11_zombie_vault_wird_zugunsten_der_kasse_aufgeloest() {
    // A11-V-4: nach einer Liquidation Schuld 0, Zins > Sicherheit – der
    // Besitzer bekäme nichts mehr und schlösse nie. Jetzt darf jeder auflösen.
    let e = env();
    let coll = 5_000 * E8; // 200 USD
    let s = St { debt: 100 * E8, interest: 150 * E8, at: INDEX };
    let rest = coll - seize(coll, 100 * E8, PRICE);
    assert!(all_ok(&burn_tx(&e, "liquidate", s, Some(0), coll, Some(rest), 100 * E8, 0, e.oracle)));
    let zombie = St { debt: 0, interest: 150 * E8, at: INDEX };
    assert_eq!(close_fee(zombie, rest, PRICE), rest);
    let ok = sweep_tx(&e, zombie, rest, vec![p2pk_out(&e.treasury, rest - SWEEP_FEE)]);
    assert!(all_ok(&ok), "jeder löst auf: {ok:?}");
    // Gegenproben: weniger an die Kasse, falscher Empfänger, Kasse an falscher Stelle
    let less = sweep_tx(&e, zombie, rest, vec![p2pk_out(&e.treasury, rest - SWEEP_FEE - 1), plain_out(1)]);
    assert!(less[0].is_err(), "{less:?}");
    let other = sweep_tx(&e, zombie, rest, vec![p2pk_out(&random_keypair(), rest - SWEEP_FEE)]);
    assert!(other[0].is_err(), "{other:?}");
    let moved = sweep_tx(&e, zombie, rest, vec![plain_out(1), p2pk_out(&e.treasury, rest - SWEEP_FEE)]);
    assert!(moved[0].is_err(), "{moved:?}");
}

#[test]
fn a11_auflösen_nur_ohne_schuld_und_wenn_der_zins_alles_aufzehrt() {
    let e = env();
    // Zins kleiner als die Sicherheit: Sache des Besitzers (close)
    let s = St { debt: 0, interest: 2 * E8, at: INDEX };
    assert!(close_fee(s, COLL, PRICE) < COLL);
    let r = sweep_tx(&e, s, COLL, vec![p2pk_out(&e.treasury, COLL - SWEEP_FEE)]);
    assert!(r[0].is_err(), "{r:?}");
    // mit Schuld: nie, auch wenn der Zins die Sicherheit übersteigt
    let s = St { debt: E8, interest: 1_000 * E8, at: INDEX };
    let r = sweep_tx(&e, s, COLL, vec![p2pk_out(&e.treasury, COLL - SWEEP_FEE)]);
    assert!(r[0].is_err(), "{r:?}");
    // keine Fortsetzung unter der Covenant-ID
    let z = St { debt: 0, interest: 1_000 * E8, at: INDEX };
    let r = sweep_tx(&e, z, COLL, vec![p2pk_out(&e.treasury, COLL - SWEEP_FEE), rogue_vault_out(1)]);
    assert!(r[0].is_err(), "{r:?}");
    // Gegenprobe ohne Prägen; heimliches Prägen beim Auflösen prüft
    // a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault (noGhost)
    let ok = sweep_tx(&e, z, COLL, vec![p2pk_out(&e.treasury, COLL - SWEEP_FEE)]);
    assert!(all_ok(&ok), "Gegenprobe: {ok:?}");
}

#[test]
fn a11_zins_unabhaengig_von_der_abrechnungshaeufigkeit() {
    // A11-V-5: mit Zinseszins auf den offenen Zins ist einmal abrechnen ≈
    // stündlich abrechnen. Vorher: 11,07 gegen 10,00 USD. Jede Abrechnung rundet
    // den Zuwachs auf (zugunsten der Kasse), daher stündlich minimal mehr.
    use kaspa_lending_protocol::contracts::{OracleState, oracle_next_index};
    let rate = (0.20f64 / 315_360_000.0 * 1e18).round() as i64;
    let mut o = OracleState { kas_usd: PRICE, oracle_daa: 0, seq: 0, stable_rate: rate, stable_index: 1_000_000_000, frozen: false, last_rate_daa: 0 };
    let start = VaultState { debt: 50 * E8, interest: 0, index_at: o.stable_index };
    let mut hourly = start;
    for _ in 0..8_760 {
        let idx = oracle_next_index(&o, o.oracle_daa + 36_000).unwrap();
        o = OracleState { oracle_daa: o.oracle_daa + 36_000, stable_index: idx, ..o };
        hourly = math::settled(&hourly, idx);
    }
    let once = math::accrued(&start, o.stable_index);
    let diff = hourly.interest - once;
    assert!(diff >= 0 && diff * 10_000 <= once, "einmal {once}, stündlich {}, Abstand {diff} (höchstens 0,01 %)", hourly.interest);
}

// ------------------------------------------------------------- Audit 12 ----

const TRESOR_SRC: &str = include_str!("../../contracts/standing_order.sil");
const TRESOR_COV: Hash = Hash::from_bytes([0x1f; 32]);

/// A12-15: sweep darf jeder auslösen. Ohne noGhost() in sweep könnte der
/// Auslöser den Minter-Zweig des Vaults mitnehmen und beliebig GHOST prägen
/// (KCC20 prüft beim Covenant-Besitzer nur, dass der Vault in der Tx vorkommt).
/// Vor Audit 12 sicherte kein Test diese Regel (Mutante M04 überlebte).
#[test]
fn a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault() {
    let e = env();
    let z = St { debt: 0, interest: 1_000 * E8, at: INDEX };
    let vault = e.vault(z);
    let thief = random_keypair();
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let loot = Tok::to(&thief, 1_000_000 * E8);
    let states = vec![Tok::minter().arg(), loot.arg()];
    let r = execute(
        vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "sweep", args: vec![ArtifactValue::Int(1)], sig_by: None } },
            oracle_read(&e, ORACLE_COV, e.oracle),
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states, sig_by: None } },
        ],
        vec![
            out(&e.oracle_art(e.oracle), E8, 1, ORACLE_COV),
            p2pk_out(&e.treasury, COLL - SWEEP_FEE),
            out(&minter, 1_000, 2, GHOST_COV),
            out(&e.ghost(loot.owner.clone(), ID_PUBKEY, loot.amount, false), 1_000, 2, GHOST_COV),
        ],
    );
    println!("sweep + Prägen: {r:?}");
    assert!(r[2].is_ok(), "KCC20 allein ließe das Prägen zu: {r:?}");
    assert!(r[0].is_err(), "der Vault muss ablehnen: {r:?}");
}

/// A12-15 (Mutante M08 `interestFee >= coll − 1`): Auflösen auf den sompi
/// genau an der Grenze. Zehrt der Zins die Sicherheit genau auf, geht es; bei
/// 1 sompi mehr Sicherheit (Zinsgebühr = Sicherheit − 1) lehnt der Vault ab.
#[test]
fn a12_aufloesen_grenze_auf_ein_sompi_genau() {
    let e = env();
    let z = St { debt: 0, interest: 150 * E8, at: INDEX };
    let fee = close_fee(z, i64::MAX, PRICE); // ⌈Zins·1e8/Preis⌉ ohne Deckel: 3 750 KAS
    assert!(fee > SWEEP_FEE);
    let at = sweep_tx(&e, z, fee, vec![p2pk_out(&e.treasury, fee - SWEEP_FEE)]);
    assert!(all_ok(&at), "Zinsgebühr = Sicherheit: {at:?}");
    let above = sweep_tx(&e, z, fee + 1, vec![p2pk_out(&e.treasury, fee + 1 - SWEEP_FEE)]);
    assert!(above[0].is_err(), "Zinsgebühr = Sicherheit − 1 sompi: {above:?}");
    // der Rust-Spiegel zieht dieselbe Grenze
    assert!(math::sweep_allowed(fee, &z.vs(), PRICE, INDEX) && math::sweepable(fee, &z.vs(), PRICE, INDEX));
    assert!(!math::sweep_allowed(fee + 1, &z.vs(), PRICE, INDEX) && !math::sweepable(fee + 1, &z.vs(), PRICE, INDEX));
}

/// A12: Vault an Eingang 1 (Orakel an 0): die Kasse muss an Ausgang 2 stehen
#[test]
fn a12_kasse_direkt_hinter_dem_vault_auch_an_eingang_1() {
    let e = env();
    let orc = || out(&e.oracle_art(e.oracle), E8, 0, ORACLE_COV);
    // close
    let s = St { debt: 0, interest: 2 * E8, at: INDEX };
    let fee = close_fee(s, COLL, PRICE);
    let v = e.vault(s);
    let close = |outs: Vec<TransactionOutput>| {
        execute(
            vec![
                oracle_read(&e, ORACLE_COV, e.oracle),
                In { utxo: utxo(&v, COLL, VAULT_COV), call: Call::Entry { art: v.clone(), entry: "close", args: vec![ArtifactValue::Int(0)], sig_by: Some((1, e.owner)) } },
            ],
            outs,
        )
    };
    let ok = close(vec![orc(), plain_out(COLL - fee), p2pk_out(&e.treasury, fee)]);
    assert!(all_ok(&ok), "Kasse an Ausgang 2: {ok:?}");
    let wrong = close(vec![orc(), p2pk_out(&e.treasury, fee), plain_out(COLL - fee)]);
    assert!(wrong[1].is_err(), "Kasse an Ausgang 1 zählt nicht: {wrong:?}");
    // sweep
    let z = St { debt: 0, interest: 1_000 * E8, at: INDEX };
    let zv = e.vault(z);
    let sweep = |outs: Vec<TransactionOutput>| {
        execute(
            vec![
                oracle_read(&e, ORACLE_COV, e.oracle),
                In { utxo: utxo(&zv, COLL, VAULT_COV), call: Call::Entry { art: zv.clone(), entry: "sweep", args: vec![ArtifactValue::Int(0)], sig_by: None } },
            ],
            outs,
        )
    };
    let ok = sweep(vec![orc(), plain_out(1), p2pk_out(&e.treasury, COLL - SWEEP_FEE)]);
    assert!(all_ok(&ok), "{ok:?}");
    let wrong = sweep(vec![orc(), p2pk_out(&e.treasury, COLL - SWEEP_FEE), plain_out(1)]);
    assert!(wrong[1].is_err(), "{wrong:?}");
}

fn tresor_art(owner: &[u8], recipient: &[u8], amount: i64, max_fee: i64, next_due: i64, left: i64) -> SilAbiArtifact {
    compile_to_sil_abi_artifact_with_options(
        TRESOR_SRC,
        &[
            ArtifactValue::Bytes(owner.to_vec()),
            ArtifactValue::Bytes(recipient.to_vec()),
            ArtifactValue::Int(amount),
            ArtifactValue::Int(1),
            ArtifactValue::Int(0),
            ArtifactValue::Int(max_fee),
            // payloadHash: Tresor ohne Nachricht (die Tx hier hat keinen Payload)
            ArtifactValue::Bytes(kaspa_lending_protocol::contracts::payload_hash(&[])),
            ArtifactValue::Int(next_due),
            ArtifactValue::Int(left),
        ],
        CompileOptions::default(),
    )
    .expect("StandingOrder kompiliert")
}

/// A12-13 (Info, dokumentiert): Zwei Verträge mit festem Ausgangsindex in einer
/// Tx. Der Vault an Eingang 0 schließt (Kasse an Ausgang 1), ein Tresor an
/// Eingang 1 zahlt an die Zinskasse (Zahlung an Ausgang 1). EIN Ausgang
/// erfüllt beide: Die fällige Tresor-Zahlung eines Dritten gilt zugleich als
/// Zinszahlung des Vaults, der Vault-Besitzer nimmt die ganze Sicherheit.
/// Der Tresor zahlt wie vereinbart; es verliert die Zinskasse (der Zins des
/// Vaults fehlt, bis zur Höhe der Tresor-Zahlung). Beim Auflösen (sweep)
/// braucht es nicht einmal eine Signatur: Ein Fremder nimmt die Sicherheit
/// eines Zombie-Vaults, die sonst an die Kasse ginge. Möglich nur, wenn ein
/// Tresor an die Zinskasse zahlt, er gerade fällig ist und seine Zahlung
/// mindestens die Zinsgebühr bzw. Sicherheit − SWEEP_FEE erreicht.
#[test]
fn a12_tresor_zahlung_an_die_kasse_deckt_zugleich_den_zins() {
    let e = env();
    let s = St { debt: 0, interest: 2 * E8, at: INDEX };
    let fee = close_fee(s, COLL, PRICE); // 50 KAS
    let v = e.vault(s);
    // Tresor eines Dritten, der monatlich 50 KAS an die Zinskasse zahlt (1. des Monats, 08:00 UTC)
    let due: i64 = 1_801_468_800_000; // 2027-02-01 08:00 UTC
    let next = kaspa_lending_protocol::standing::following(due, 1, 0);
    let (tresor_owner, max_fee, fund) = (random_keypair(), 1_000_000i64, 100 * E8);
    let t = tresor_art(&xonly(&tresor_owner), &xonly(&e.treasury), fee, max_fee, due, -1);
    let tn = tresor_art(&xonly(&tresor_owner), &xonly(&e.treasury), fee, max_fee, next, -1);
    let r = execute_lt(
        vec![
            In { utxo: utxo(&v, COLL, VAULT_COV), call: Call::Entry { art: v.clone(), entry: "close", args: vec![ArtifactValue::Int(2)], sig_by: Some((1, e.owner)) } },
            In { utxo: utxo(&t, fund, TRESOR_COV), call: Call::Entry { art: t.clone(), entry: "pay", args: vec![], sig_by: None } },
            oracle_read(&e, ORACLE_COV, e.oracle),
        ],
        vec![
            plain_out(COLL), // der Vault-Besitzer nimmt die ganze Sicherheit
            p2pk_out(&e.treasury, fee), // ein einziger Kassen-Ausgang für beide
            out(&tn, fund - fee - max_fee, 1, TRESOR_COV),
            out(&e.oracle_art(e.oracle), E8, 2, ORACLE_COV),
        ],
        due as u64,
    );
    println!("Vault-close + Tresor-pay mit geteiltem Ausgang 1: {r:?}");
    assert!(all_ok(&r), "beide Verträge nehmen den geteilten Ausgang an: {r:?}");
    // sweep: ein Fremder löst einen Zombie-Vault mit 50 KAS auf und behält sie
    let zc = fee;
    let zv = e.vault(St { debt: 0, interest: 1_000 * E8, at: INDEX });
    let r = execute_lt(
        vec![
            In { utxo: utxo(&zv, zc, VAULT_COV), call: Call::Entry { art: zv.clone(), entry: "sweep", args: vec![ArtifactValue::Int(2)], sig_by: None } },
            In { utxo: utxo(&t, fund, TRESOR_COV), call: Call::Entry { art: t.clone(), entry: "pay", args: vec![], sig_by: None } },
            oracle_read(&e, ORACLE_COV, e.oracle),
        ],
        vec![
            plain_out(zc), // die Sicherheit an den Fremden
            p2pk_out(&e.treasury, fee), // die Tresor-Zahlung zählt als Kassen-Ausgang (≥ Sicherheit − SWEEP_FEE)
            out(&tn, fund - fee - max_fee, 1, TRESOR_COV),
            out(&e.oracle_art(e.oracle), E8, 2, ORACLE_COV),
        ],
        due as u64,
    );
    println!("Vault-sweep + Tresor-pay mit geteiltem Ausgang 1: {r:?}");
    assert!(all_ok(&r), "{r:?}");
}

/// A12-14: interestFee rechnet im Vertrag (Zins/kasUsd)·1e8 in 64 Bit. Beim
/// Tiefstpreis des Orakels (0,00001 USD) läuft das ab ≈ 922 000 USD Zins über;
/// Schließen und Auflösen scheitern dann, bis der Preis steigt (Vertragsgrenze,
/// dokumentiert; mit 50 GHOST Höchstschuld im Mainnet unerreichbar).
/// math::interest_fee schnitt beim `as i64` ins Negative ab, jetzt rechnet sie
/// sättigend; interest_fee_checked und sweep_allowed folgen dem Vertrag genau.
#[test]
fn a12_zinsgebuehr_rechengrenze_bei_tiefstpreis() {
    let low = Oracle { kas_usd: 1_000, index: INDEX };
    let e = Env::new(low);
    let z = St { debt: 0, interest: 1_000_000 * E8, at: INDEX }; // 1 Mio USD
    let fee_rs = math::interest_fee(COLL, &z.vs(), low.kas_usd, INDEX);
    println!("math::interest_fee bei 1 Mio USD Zins, 0,00001 USD: {fee_rs}");
    assert_eq!(fee_rs, COLL, "sättigend: höchstens die Sicherheit, nie negativ");
    assert_eq!(math::interest_fee_checked(COLL, &z.vs(), low.kas_usd, INDEX), None, "der Vertrag läuft hier über");
    assert!(!math::sweep_allowed(COLL, &z.vs(), low.kas_usd, INDEX) && !math::sweepable(COLL, &z.vs(), low.kas_usd, INDEX));
    let r = sweep_tx(&e, z, COLL, vec![p2pk_out(&e.treasury, COLL - SWEEP_FEE)]);
    println!("sweep bei 1 Mio USD Zins, Preis 0,00001 USD: {r:?}");
    assert!(r[0].is_err(), "{r:?}");
    // genau an der Grenze: ⌈Zins·1e8/Preis⌉ = Zins·1e5 ≤ i64::MAX geht, 1 Einheit mehr nicht
    let edge = i64::MAX / 100_000;
    for (interest, fits) in [(edge, true), (edge + 1, false)] {
        let z = St { debt: 0, interest, at: INDEX };
        assert_eq!(math::interest_fee_checked(COLL, &z.vs(), low.kas_usd, INDEX).is_some(), fits);
        assert_eq!(math::sweep_allowed(COLL, &z.vs(), low.kas_usd, INDEX), fits);
        let r = sweep_tx(&e, z, COLL, vec![p2pk_out(&e.treasury, COLL - SWEEP_FEE)]);
        assert_eq!(all_ok(&r), fits, "Zins {interest}: {r:?}");
    }
}

// ============================================================================
// Version 4: Frischeprüfung (Orakel eingefroren) und Zinsziel
// ============================================================================

/// führt `f` einmal mit frischem und einmal mit eingefrorenem Orakel aus
fn frisch_und_eingefroren(e: &Env, f: impl Fn(&Env) -> Vec<Result<(), String>>) -> (Vec<Result<(), String>>, Vec<Result<(), String>>) {
    e.frozen.set(false);
    let fresh = f(e);
    e.frozen.set(true);
    let frozen = f(e);
    e.frozen.set(false);
    (fresh, frozen)
}

#[test]
fn v4_eingefroren_sperrt_praegen() {
    let e = env();
    let (fresh, frozen) = frisch_und_eingefroren(&e, |e| Mint::new(e, 10 * E8).run(e, 0));
    assert!(all_ok(&fresh), "Gegenprobe frisch: {fresh:?}");
    assert!(frozen[0].is_err() && frozen[1].is_ok() && frozen[2].is_ok(), "nur der Vault lehnt ab: {frozen:?}");
}

#[test]
fn v4_eingefroren_sperrt_ruecknahme() {
    let e = env();
    let (debt, amount) = (100 * E8, 10 * E8);
    let paid = redeem_paid(amount, PRICE);
    let (fresh, frozen) =
        frisch_und_eingefroren(&e, |e| burn_tx(e, "redeem", debt, Some(debt - amount), COLL, Some(COLL - paid), amount, 0, e.oracle));
    assert!(all_ok(&fresh), "{fresh:?}");
    assert!(frozen[0].is_err() && frozen[1].is_ok(), "{frozen:?}");
}

#[test]
fn v4_eingefroren_sperrt_liquidation() {
    let e = env();
    let debt = 150 * E8;
    let p = crash_price(COLL, debt);
    let o = Oracle { kas_usd: p, index: INDEX };
    let e2 = Env { oracle: o, ..e };
    let burn = 40 * E8;
    let rest = COLL - seize(COLL, burn, p);
    let (fresh, frozen) = frisch_und_eingefroren(&e2, |e| burn_tx(e, "liquidate", debt, Some(debt - burn), COLL, Some(rest), burn, 0, o));
    assert!(all_ok(&fresh), "{fresh:?}");
    assert!(frozen[0].is_err() && frozen[1].is_ok(), "{frozen:?}");
}

#[test]
fn v4_eingefroren_sperrt_aufloesen() {
    let e = env();
    let coll = 5_000 * E8;
    let zombie = St { debt: 0, interest: 250 * E8, at: INDEX };
    assert_eq!(close_fee(zombie, coll, PRICE), coll);
    let (fresh, frozen) = frisch_und_eingefroren(&e, |e| sweep_tx(e, zombie, coll, vec![p2pk_out(&e.treasury, coll - SWEEP_FEE)]));
    assert!(all_ok(&fresh), "{fresh:?}");
    assert!(frozen[0].is_err() && frozen[1].is_ok(), "{frozen:?}");
}

#[test]
fn v4_eingefroren_sperrt_auszahlung_nur_mit_schuld() {
    let e = env();
    let w = |e: &Env, debt: i64| {
        vault_only(e, "withdraw", vec![ArtifactValue::Int(COLL / 2), ArtifactValue::Int(1)], Some(2), debt, COLL, Some(COLL / 2), true, false)
    };
    let (fresh, frozen) = frisch_und_eingefroren(&e, |e| w(e, 10 * E8));
    assert!(all_ok(&fresh), "mit Schuld, frisch: {fresh:?}");
    assert!(frozen[0].is_err() && frozen[1].is_ok(), "mit Schuld, eingefroren: {frozen:?}");
    let (fresh0, frozen0) = frisch_und_eingefroren(&e, |e| w(e, 0));
    assert!(all_ok(&fresh0) && all_ok(&frozen0), "ohne Schuld immer: {fresh0:?} {frozen0:?}");
}

#[test]
fn v4_eingefroren_bleiben_tilgen_schliessen_einzahlen() {
    let e = env();
    // tilgen
    let s = St { debt: 10 * E8, interest: 3 * E8, at: INDEX0 };
    let (a, b) = frisch_und_eingefroren(&e, |e| burn_tx(e, "repay", s, Some(0), COLL, Some(COLL), 10 * E8, 0, e.oracle));
    assert!(all_ok(&a) && all_ok(&b), "tilgen: {a:?} {b:?}");
    // schließen (mit Zins an die Kasse)
    let s = St { debt: 0, interest: 2 * E8, at: INDEX };
    let fee = close_fee(s, COLL, PRICE);
    let (a, b) = frisch_und_eingefroren(&e, |e| close_tx(e, s, COLL, e.owner, vec![p2pk_out(&e.treasury, fee), plain_out(COLL - fee)]));
    assert!(all_ok(&a) && all_ok(&b), "schließen: {a:?} {b:?}");
    // einzahlen braucht kein Orakel
    let (a, b) = frisch_und_eingefroren(&e, |e| vault_only(e, "deposit", vec![], Some(0), 5 * E8, COLL, Some(COLL + E8), false, false));
    assert!(all_ok(&a) && all_ok(&b), "einzahlen: {a:?} {b:?}");
}

fn p2sh_out(redeem: &[u8], value: i64) -> TransactionOutput {
    TransactionOutput { value: value as u64, script_public_key: pay_to_script_hash_script(redeem), covenant: None }
}

/// Unspendbares Ziel zum Verbrennen: P2SH eines Skripts aus OP_FALSE
const BURN_SCRIPT: [u8; 1] = [0x00];

#[test]
fn v4_zinsziel_p2sh_zinskasse_und_verbrennen() {
    // Zinsziel als beliebiges scriptPubKey: P2SH (z. B. Zinskasse v4 oder ein
    // späterer Topf) und Verbrennen (P2SH von OP_FALSE)
    let s = St { debt: 0, interest: 2 * E8, at: INDEX };
    let fee = close_fee(s, COLL, PRICE);
    for redeem in [vec![0x51u8, 0x75, 0x51], BURN_SCRIPT.to_vec()] {
        let mut e = env();
        e.interest_spk = spk_bytes(&pay_to_script_hash_script(&redeem));
        let ok = close_tx(&e, s, COLL, e.owner, vec![p2sh_out(&redeem, fee), plain_out(COLL - fee)]);
        assert!(all_ok(&ok), "{ok:?}");
        // Gegenproben: alter P2PK-Empfänger, anderes Skript, zu wenig
        let p2pk = close_tx(&e, s, COLL, e.owner, vec![p2pk_out(&e.treasury, fee), plain_out(COLL - fee)]);
        assert!(p2pk[0].is_err(), "{p2pk:?}");
        let other = close_tx(&e, s, COLL, e.owner, vec![p2sh_out(&[0x51], fee), plain_out(COLL - fee)]);
        assert!(other[0].is_err(), "{other:?}");
        let short = close_tx(&e, s, COLL, e.owner, vec![p2sh_out(&redeem, fee - 1), plain_out(COLL - fee + 1)]);
        assert!(short[0].is_err(), "{short:?}");
    }
}

#[test]
fn v4_groessen() {
    let e = env();
    let v = e.vault(0);
    println!("StableVault v4: Redeem-Skript {} B | Orakel v4: {} B", bytecode(&v).len(), bytecode(&e.oracle_art(e.oracle)).len());
}

#[test]
fn v4_factory_bleibt_unveraendert_nutzbar() {
    // vault_factory.sil bindet nur den Vault-Template-Hash (per init) und das
    // Zustandslayout VaultState { owner, debt, interest, indexAt }. Vault v4
    // hat dasselbe Layout: die Zustandsbytes sind für jeden Zustand identisch
    // mit v3, die Factory kann also unverändert Vaults v4 eröffnen.
    const VAULT_V3_SRC: &str = include_str!("../../contracts/stable_vault.sil");
    let e = env();
    for s in [St::default(), St { debt: 7 * E8, interest: 3, at: INDEX }] {
        let v4 = e.vault(s);
        let mut args: Vec<ArtifactValue> = vec![
            ArtifactValue::Bytes(ORACLE_COV.as_bytes().to_vec()),
            ArtifactValue::Int(1),
            ArtifactValue::Int(1),
            ArtifactValue::Bytes(vec![0; 32]),
            ArtifactValue::Bytes(GHOST_COV.as_bytes().to_vec()),
            ArtifactValue::Int(1),
            ArtifactValue::Int(1),
            ArtifactValue::Bytes(vec![0; 32]),
        ];
        args.extend([MCR, LIQ, BONUS, i64::MAX].map(ArtifactValue::Int));
        args.push(ArtifactValue::Bytes(xonly(&e.treasury)));
        args.extend([ArtifactValue::Bytes(xonly(&e.owner)), ArtifactValue::Int(s.debt), ArtifactValue::Int(s.interest), ArtifactValue::Int(s.at)]);
        let v3 = compile_to_sil_abi_artifact_with_options(VAULT_V3_SRC, &args, CompileOptions::default()).unwrap();
        let state = |a: &SilAbiArtifact| {
            let l = common::state_layout(a);
            bytecode(a)[l.start..l.start + l.len].to_vec()
        };
        assert_eq!(state(&v3), state(&v4), "Zustandsbytes v3 = v4 für {s:?}");
    }
}
