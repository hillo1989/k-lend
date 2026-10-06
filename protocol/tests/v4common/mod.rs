//! Gemeinsame Bausteine der v4-Tests (Register, Orakel v4, Zinskasse v4):
//! Kompilieren, Digests wie in den Verträgen, Transaktionen mit Payload,
//! Locktime und Sequenz (für this.ageDaa) bauen und JEDEN Input in der echten
//! Skript-Engine (rusty-kaspa a41a333) ausführen.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
pub mod common;

use common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::tx::{
    CovenantBinding, MutableTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_txscript::pay_to_script_hash_script;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

pub const ORACLE_SRC: &str = include_str!("../../../contracts/price_oracle_v4.sil");
pub const REGISTER_SRC: &str = include_str!("../../../contracts/signer_register_v4.sil");
pub const KASSE_SRC: &str = include_str!("../../../contracts/zinskasse_v4.sil");

pub const REG_COV: Hash = Hash::from_bytes([0x5e; 32]);
pub const ORACLE_COV: Hash = Hash::from_bytes([0x0a; 32]);
pub const OTHER_COV: Hash = Hash::from_bytes([0x0d; 32]);

pub const E8: i64 = 100_000_000;
pub const REG_V: i64 = 10 * E8;
pub const ORACLE_V: i64 = 10 * E8;
pub const TICKET_V: i64 = E8;

pub const TAG_PRICE: u8 = 0x01;
pub const TAG_ROTATE: u8 = 0x02;
pub const TAG_CANCEL: u8 = 0x03;
pub const TAG_EMERG: u8 = 0x04;

/// 1 Tag bei 10 BPS
pub const DAY: i64 = 864_000;

// ---------------------------------------------------------------- Schlüssel ----

pub fn random_keypair() -> Keypair {
    let secp = Secp256k1::new();
    let mut sk = [0u8; 32];
    loop {
        thread_rng().fill_bytes(&mut sk);
        if let Ok(s) = SecretKey::from_slice(&sk) {
            return Keypair::from_secret_key(&secp, &s);
        }
    }
}

pub fn xonly(k: &Keypair) -> Vec<u8> {
    k.x_only_public_key().0.serialize().to_vec()
}

/// `x as byte[8]` in SilverScript: Betrag little-endian, Vorzeichen im höchsten Bit
pub fn num8(v: i64) -> [u8; 8] {
    let mut b = v.unsigned_abs().to_le_bytes();
    if v < 0 {
        b[7] |= 0x80;
    }
    b
}

pub fn sha(m: &[u8]) -> [u8; 32] {
    Sha256::digest(m).into()
}

pub fn i(v: i64) -> ArtifactValue {
    ArtifactValue::Int(v)
}

pub fn b(v: &[u8]) -> ArtifactValue {
    ArtifactValue::Bytes(v.to_vec())
}

/// sha256(n ‖ t ‖ tRot ‖ k0 ‖ … ‖ k(n−1)) wie hashSet im Register
pub fn set_hash(n: i64, t: i64, t_rot: i64, keys: &[Vec<u8>]) -> [u8; 32] {
    let mut m = vec![];
    for v in [n, t, t_rot] {
        m.extend_from_slice(&num8(v));
    }
    for k in keys {
        m.extend_from_slice(k);
    }
    sha(&m)
}

/// Ein Unterzeichner-Satz
#[derive(Clone)]
pub struct Satz {
    pub keys: Vec<Keypair>,
    pub t: i64,
    pub t_rot: i64,
}

impl Satz {
    pub fn new(n: usize, t: i64, t_rot: i64) -> Self {
        Self { keys: (0..n).map(|_| random_keypair()).collect(), t, t_rot }
    }
    pub fn n(&self) -> i64 {
        self.keys.len() as i64
    }
    pub fn pubs(&self) -> Vec<Vec<u8>> {
        self.keys.iter().map(xonly).collect()
    }
    pub fn hash(&self) -> [u8; 32] {
        set_hash(self.n(), self.t, self.t_rot, &self.pubs())
    }
    /// Argumente n, t, tRot, keys
    pub fn args(&self) -> Vec<ArtifactValue> {
        vec![i(self.n()), i(self.t), i(self.t_rot), ArtifactValue::Array(self.pubs().into_iter().map(ArtifactValue::Bytes).collect())]
    }
    /// sigs, idx: Signaturen der Schlüssel `who` (in dieser Reihenfolge) über digest
    pub fn quorum(&self, digest: [u8; 32], who: &[usize]) -> Vec<ArtifactValue> {
        // Index außerhalb des Satzes: mit einem fremden Schlüssel signieren
        let key = |j: usize| self.keys.get(j).copied().unwrap_or_else(random_keypair);
        let sigs = who.iter().map(|&j| ArtifactValue::Bytes(key(j).sign_schnorr(Message::from_digest(digest)).as_ref().to_vec())).collect();
        let idx = who.iter().map(|&j| i(j as i64)).collect();
        vec![ArtifactValue::Array(sigs), ArtifactValue::Array(idx)]
    }
    /// die ersten k Indizes
    pub fn first(k: i64) -> Vec<usize> {
        (0..k as usize).collect()
    }
}

// ------------------------------------------------------------------ Orakel ----

#[derive(Clone, Copy, Debug)]
pub struct OCfg {
    pub max_rate: i64,
    pub rate_step: i64,
    pub rate_gap: i64,
    pub freeze_after: i64,
}

/// Vorschläge aus docs/v4-entwurf.md: 20 %/Jahr, 0,5 Punkte, 1 h, 2 h
pub const OCFG: OCfg = OCfg { max_rate: 634_195_839, rate_step: 15_854_896, rate_gap: 36_000, freeze_after: 72_000 };

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OSt {
    pub kas_usd: i64,
    pub daa: i64,
    pub seq: i64,
    pub rate: i64,
    pub index: i64,
    pub frozen: bool,
    pub last_rate_daa: i64,
}

pub const OSTART: OSt =
    OSt { kas_usd: 4_000_000, daa: 1_000_000, seq: 7, rate: 158_548_959, index: 1_000_000_000, frozen: false, last_rate_daa: 1_000_000 };

pub fn oracle_art(reg_cov: Hash, c: OCfg, s: OSt) -> SilAbiArtifact {
    let args = vec![
        b(reg_cov.as_bytes().as_slice()),
        i(c.max_rate),
        i(c.rate_step),
        i(c.rate_gap),
        i(c.freeze_after),
        i(s.kas_usd),
        i(s.daa),
        i(s.seq),
        i(s.rate),
        i(s.index),
        ArtifactValue::Bool(s.frozen),
        i(s.last_rate_daa),
    ];
    compile_to_sil_abi_artifact_with_options(ORACLE_SRC, &args, CompileOptions::default()).expect("Orakel v4 kompiliert")
}

/// Index-Fortschreibung wie im Vertrag (None bei Überlauf)
pub fn next_index(prev: OSt, new_daa: i64) -> Option<i64> {
    let delta = new_daa - prev.daa;
    let growth = prev.rate.checked_mul(delta)? / 1_000_000_000;
    prev.index.checked_add(prev.index.checked_mul(growth)? / 1_000_000_000)
}

/// Folgezustand eines ehrlichen Updates
pub fn oracle_next(prev: OSt, kas: i64, daa: i64, rate: i64) -> OSt {
    OSt {
        kas_usd: kas,
        daa,
        seq: prev.seq + 1,
        rate,
        index: next_index(prev, daa).expect("kein Überlauf"),
        frozen: false,
        last_rate_daa: if rate != prev.rate { daa } else { prev.last_rate_daa },
    }
}

pub fn price_digest(oracle_cov: Hash, kas: i64, daa: i64, seq: i64, rate: i64) -> [u8; 32] {
    let mut m = oracle_cov.as_bytes().to_vec();
    m.push(TAG_PRICE);
    for v in [kas, daa, seq, rate] {
        m.extend_from_slice(&num8(v));
    }
    sha(&m)
}

// ---------------------------------------------------------------- Register ----

#[derive(Clone, Copy, Debug)]
pub struct RCfg {
    pub min_n: i64,
    pub min_t: i64,
    pub rot_delay: i64,
    pub emerg_after: i64,
    pub emerg_delay: i64,
}

/// Vorschläge: 5 Schlüssel, t ≥ 3, 14 Tage, 30 Tage, 14 Tage
pub const RCFG: RCfg = RCfg { min_n: 5, min_t: 3, rot_delay: 14 * DAY, emerg_after: 30 * DAY, emerg_delay: 14 * DAY };
/// Alleinbetrieb/Mainnet-Probe: ab 1-von-1, 1 h Wartezeiten
pub const RCFG_SOLO: RCfg = RCfg { min_n: 1, min_t: 1, rot_delay: 36_000, emerg_after: 36_000, emerg_delay: 36_000 };

#[derive(Clone, Debug, PartialEq)]
pub struct RSt {
    pub ticket: bool,
    pub set: [u8; 32],
    pub fb: [u8; 32],
    pub pay_kind: u8,
    pub pay_to: [u8; 32],
    pub nonce: i64,
    pub emerg: bool,
    pub last_daa: i64,
    pub oracle_cov: [u8; 32],
    pub oracle_tpl: Vec<u8>,
    pub pre: i64,
    pub suf: i64,
    pub init: bool,
}

pub fn register_art(deployer: &[u8], c: RCfg, s: &RSt) -> SilAbiArtifact {
    let args = vec![
        b(deployer),
        i(c.min_n),
        i(c.min_t),
        i(c.rot_delay),
        i(c.emerg_after),
        i(c.emerg_delay),
        ArtifactValue::Bool(s.ticket),
        b(&s.set),
        b(&s.fb),
        ArtifactValue::Byte(s.pay_kind),
        b(&s.pay_to),
        i(s.nonce),
        ArtifactValue::Bool(s.emerg),
        i(s.last_daa),
        b(&s.oracle_cov),
        b(&s.oracle_tpl),
        i(s.pre),
        i(s.suf),
        ArtifactValue::Bool(s.init),
    ];
    compile_to_sil_abi_artifact_with_options(REGISTER_SRC, &args, CompileOptions::default()).expect("Register v4 kompiliert")
}

/// Ankündigung (Payload) newSet ‖ newFb ‖ newKind ‖ newTo
pub fn announcement(set: &[u8; 32], fb: &[u8; 32], kind: u8, to: &[u8; 32]) -> Vec<u8> {
    let mut v = set.to_vec();
    v.extend_from_slice(fb);
    v.push(kind);
    v.extend_from_slice(to);
    v
}

pub fn rot_digest(reg_cov: Hash, tag: u8, nonce: i64, ann: &[u8]) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(tag);
    m.extend_from_slice(&num8(nonce));
    m.extend_from_slice(ann);
    sha(&m)
}

pub fn cancel_digest(reg_cov: Hash, nonce: i64) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(TAG_CANCEL);
    m.extend_from_slice(&num8(nonce));
    sha(&m)
}

// ------------------------------------------------------------- Zinskasse ----

pub fn kasse_art(reg_cov: Hash, reg_tpl: &(Vec<u8>, Vec<u8>, Vec<u8>)) -> SilAbiArtifact {
    let args = vec![b(reg_cov.as_bytes().as_slice()), i(reg_tpl.0.len() as i64), i(reg_tpl.1.len() as i64), b(&reg_tpl.2)];
    compile_to_sil_abi_artifact_with_options(KASSE_SRC, &args, CompileOptions::default()).expect("Zinskasse v4 kompiliert")
}

// ------------------------------------------------------- Tx-Bausteine ----

pub enum Call {
    /// Einstiegspunkt; `sig_at` = Position der Schnorr-Signatur über die Tx
    Entry { art: SilAbiArtifact, entry: &'static str, args: Vec<ArtifactValue>, sig_at: Option<(usize, Keypair)> },
    /// fertiges Sigscript (OpTrue-Eingänge: leer)
    Raw(Vec<u8>),
}

pub struct In {
    pub utxo: UtxoEntry,
    pub call: Call,
    /// Sequenz des Eingangs (relative Sperre für this.ageDaa)
    pub sequence: u64,
}

pub fn entry_in(art: &SilAbiArtifact, value: i64, cov: Hash, entry: &'static str, args: Vec<ArtifactValue>) -> In {
    In { utxo: cov_utxo(art, value, cov), call: Call::Entry { art: art.clone(), entry, args, sig_at: None }, sequence: 0 }
}

pub fn cov_utxo(art: &SilAbiArtifact, value: i64, cov: Hash) -> UtxoEntry {
    UtxoEntry::new(value as u64, pay_to_script_hash_script(&bytecode(art)), 0, false, Some(cov))
}

/// P2SH-Eingang ohne Covenant (Zinskasse)
pub fn p2sh_utxo(art: &SilAbiArtifact, value: i64) -> UtxoEntry {
    UtxoEntry::new(value as u64, pay_to_script_hash_script(&bytecode(art)), 0, false, None)
}

pub fn opt_true_spk() -> ScriptPublicKey {
    ScriptPublicKey::new(0, vec![kaspa_txscript::opcodes::codes::OpTrue].into())
}

/// Eingang ohne Skriptprüfung (Gebührenzahler)
pub fn plain_in(value: i64) -> In {
    In { utxo: UtxoEntry::new(value as u64, opt_true_spk(), 0, false, None), call: Call::Raw(vec![]), sequence: 0 }
}

pub fn cov_out(art: &SilAbiArtifact, value: i64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput {
        value: value as u64,
        script_public_key: pay_to_script_hash_script(&bytecode(art)),
        covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }),
    }
}

pub fn plain_out(value: i64) -> TransactionOutput {
    TransactionOutput { value: value as u64, script_public_key: opt_true_spk(), covenant: None }
}

pub fn p2pk_spk(xonly: &[u8]) -> ScriptPublicKey {
    let mut script = vec![0x20];
    script.extend_from_slice(xonly);
    script.push(0xac);
    ScriptPublicKey::new(0, script.into())
}

pub fn p2pk_out(xonly: &[u8], value: i64) -> TransactionOutput {
    TransactionOutput { value: value as u64, script_public_key: p2pk_spk(xonly), covenant: None }
}

pub fn spk_out(spk: ScriptPublicKey, value: i64) -> TransactionOutput {
    TransactionOutput { value: value as u64, script_public_key: spk, covenant: None }
}

/// scriptPubKey wie tx.outputs[i].scriptPubKey: Version (LE) + Skript
pub fn spk_bytes(spk: &ScriptPublicKey) -> Vec<u8> {
    let mut v = spk.version().to_le_bytes().to_vec();
    v.extend_from_slice(spk.script());
    v
}

/// Skript-Hash einer P2SH-Ausgabe (blake2b des Redeem-Skripts)
pub fn p2sh_hash(redeem: &[u8]) -> [u8; 32] {
    let spk = pay_to_script_hash_script(redeem);
    let s = spk.script();
    // OP_BLAKE2B OP_DATA_32 <32> OP_EQUAL
    s[2..34].try_into().unwrap()
}

fn outpoint(i: usize) -> TransactionOutpoint {
    TransactionOutpoint { transaction_id: TransactionId::from_bytes([0x40 + i as u8; 32]), index: i as u32 }
}

fn sign_tx(tx: &Transaction, entries: &[UtxoEntry], idx: usize, k: &Keypair) -> Vec<u8> {
    let mtx = MutableTransaction::with_entries(tx.clone(), entries.to_vec());
    let reused = SigHashReusedValuesUnsync::new();
    let h = calc_schnorr_signature_hash(&mtx.as_verifiable(), idx, SIG_HASH_ALL, &reused);
    let mut s = k.sign_schnorr(Message::from_digest_slice(h.as_bytes().as_slice()).unwrap()).as_ref().to_vec();
    s.push(SIG_HASH_ALL.to_u8());
    s
}

pub fn contract_name(a: &SilAbiArtifact) -> String {
    a.contracts.keys().next().unwrap().clone()
}

/// Baut die Tx (Locktime, Payload), signiert, führt JEDEN Input aus.
pub fn execute(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64, payload: Vec<u8>) -> Vec<Result<(), String>> {
    let (tx, entries) = build(inputs, outputs, lock_time, payload);
    (0..tx.inputs.len()).map(|i| execute_input_with_covenants(tx.clone(), entries.clone(), i).map_err(|e| format!("{e:?}"))).collect()
}

pub fn build(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64, payload: Vec<u8>) -> (Transaction, Vec<UtxoEntry>) {
    let entries: Vec<UtxoEntry> = inputs.iter().map(|i| i.utxo.clone()).collect();
    let bare: Vec<TransactionInput> =
        inputs.iter().enumerate().map(|(i, x)| TransactionInput::new_with_compute_budget(outpoint(i), vec![], x.sequence, 0)).collect();
    let unsigned = Transaction::new(1, bare, outputs.clone(), lock_time, Default::default(), 0, payload.clone());
    let mut final_inputs = vec![];
    for (idx, input) in inputs.into_iter().enumerate() {
        let script = match input.call {
            Call::Entry { art, entry, mut args, sig_at } => {
                if let Some((pos, k)) = sig_at {
                    args.insert(pos, ArtifactValue::Bytes(sign_tx(&unsigned, &entries, idx, &k)));
                }
                let mut s = encode_contract_entry_sig_script(&art, &contract_name(&art), entry, &args).expect("sigscript");
                s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
                s
            }
            Call::Raw(s) => s,
        };
        final_inputs.push(TransactionInput::new_with_compute_budget(outpoint(idx), script, input.sequence, 0));
    }
    (Transaction::new(1, final_inputs, outputs, lock_time, Default::default(), 0, payload), entries)
}

pub fn all_ok(r: &[Result<(), String>]) -> bool {
    r.iter().all(|x| x.is_ok())
}

pub fn tpl(art: &SilAbiArtifact) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    compiled_template_parts_and_hash(art)
}

// ------------------------------------------------------------------- Welt ----

/// Ein fertig initialisiertes System aus Register und Orakel
#[derive(Clone)]
pub struct World {
    pub deployer: Keypair,
    pub rcfg: RCfg,
    pub ocfg: OCfg,
    pub set: Satz,
    pub fb: Option<Satz>,
    pub reg: RSt,
    pub oracle: OSt,
}

impl World {
    pub fn new(set: Satz, fb: Option<Satz>, rcfg: RCfg) -> Self {
        let ocfg = OCFG;
        let otpl = tpl(&oracle_art(REG_COV, ocfg, OSTART));
        let reg = RSt {
            ticket: false,
            set: set.hash(),
            fb: fb.as_ref().map(|f| f.hash()).unwrap_or([0; 32]),
            pay_kind: 0,
            pay_to: [0x77; 32],
            nonce: 0,
            emerg: false,
            last_daa: OSTART.daa,
            oracle_cov: ORACLE_COV.as_bytes(),
            oracle_tpl: otpl.2.clone(),
            pre: otpl.0.len() as i64,
            suf: otpl.1.len() as i64,
            init: true,
        };
        Self { deployer: random_keypair(), rcfg, ocfg, set, fb, reg, oracle: OSTART }
    }

    /// Vorschlagswerte: 7 Schlüssel, 4 / 4 (höchstens 4 Signaturen je Prüfung,
    /// Mempool-Standardregel), Notfallsatz 5 Schlüssel 3 / 4
    pub fn standard() -> Self {
        Self::new(Satz::new(7, 4, 4), Some(Satz::new(5, 3, 4)), RCFG)
    }

    pub fn reg_art(&self, s: &RSt) -> SilAbiArtifact {
        register_art(&xonly(&self.deployer), self.rcfg, s)
    }

    pub fn oracle_art(&self, s: OSt) -> SilAbiArtifact {
        oracle_art(REG_COV, self.ocfg, s)
    }

    pub fn reg_in(&self, s: &RSt, entry: &'static str, args: Vec<ArtifactValue>) -> In {
        entry_in(&self.reg_art(s), if s.ticket { TICKET_V } else { REG_V }, REG_COV, entry, args)
    }

    pub fn reg_out(&self, s: &RSt, auth: u16) -> TransactionOutput {
        cov_out(&self.reg_art(s), if s.ticket { TICKET_V } else { REG_V }, auth, REG_COV)
    }

    pub fn oracle_in(&self, s: OSt, entry: &'static str, args: Vec<ArtifactValue>) -> In {
        entry_in(&self.oracle_art(s), ORACLE_V, ORACLE_COV, entry, args)
    }

    pub fn oracle_out(&self, s: OSt, auth: u16) -> TransactionOutput {
        cov_out(&self.oracle_art(s), ORACLE_V, auth, ORACLE_COV)
    }

    /// Haupt-UTXO nach einem Preis-Update
    pub fn reg_after_price(&self, daa: i64) -> RSt {
        let mut r = self.reg.clone();
        r.last_daa = daa;
        if r.emerg {
            r.nonce += 1;
            r.emerg = false;
        }
        r
    }
}

/// Ein Preis-Update (Register attestPrice + Orakel update); jedes Feld lässt
/// sich für Angriffe verbiegen.
#[derive(Clone)]
pub struct Attest {
    pub kas: i64,
    pub daa: i64,
    pub rate: i64,
    /// signierender Satz (Standard: aktueller Satz der Welt)
    pub satz: Option<Satz>,
    /// Satz, dessen Schlüssel als Argument mitkommen (Standard = satz)
    pub args_satz: Option<Satz>,
    pub signers: Option<Vec<usize>>,
    pub signed_kas: Option<i64>,
    pub signed_seq: Option<i64>,
    pub signed_cov: Option<Hash>,
    /// Argumente an attestPrice (Standard = ehrliche Werte)
    pub arg_seq: Option<i64>,
    pub arg_index: Option<i64>,
    pub arg_rate_daa: Option<i64>,
    /// Orakel-Ausgang (Standard = ehrlicher Folgezustand)
    pub oracle_out: Option<OSt>,
    /// Register-Ausgang (Standard = reg_after_price)
    pub reg_out: Option<RSt>,
    pub lock_time: Option<u64>,
    /// Orakel-Eintrag (Standard update)
    pub oracle_entry: &'static str,
    /// Argumente an update (Standard = kas, daa, rate)
    pub oracle_args: Option<(i64, i64, i64)>,
    pub with_register: bool,
    pub with_oracle: bool,
}

impl Attest {
    pub fn new(kas: i64, daa: i64, rate: i64) -> Self {
        Self {
            kas,
            daa,
            rate,
            satz: None,
            args_satz: None,
            signers: None,
            signed_kas: None,
            signed_seq: None,
            signed_cov: None,
            arg_seq: None,
            arg_index: None,
            arg_rate_daa: None,
            oracle_out: None,
            reg_out: None,
            lock_time: None,
            oracle_entry: "update",
            oracle_args: None,
            with_register: true,
            with_oracle: true,
        }
    }

    /// ehrliche Fortschreibung um 600 DAA bei gleichem Preis und Satz
    pub fn next(w: &World) -> Self {
        Self::new(w.oracle.kas_usd, w.oracle.daa + 600, w.oracle.rate)
    }

    pub fn expected(&self, w: &World) -> OSt {
        oracle_next(w.oracle, self.kas, self.daa, self.rate)
    }

    pub fn run(&self, w: &World) -> Vec<Result<(), String>> {
        let satz = self.satz.clone().unwrap_or_else(|| w.set.clone());
        let args_satz = self.args_satz.clone().unwrap_or_else(|| satz.clone());
        let exp = next_index(w.oracle, self.daa).map(|_| self.expected(w));
        let new_seq = w.oracle.seq + 1;
        let digest = price_digest(
            self.signed_cov.unwrap_or(ORACLE_COV),
            self.signed_kas.unwrap_or(self.kas),
            self.daa,
            self.signed_seq.unwrap_or(new_seq),
            self.rate,
        );
        let who = self.signers.clone().unwrap_or_else(|| Satz::first(satz.t));
        let out_state = self.oracle_out.unwrap_or_else(|| exp.expect("Index berechenbar"));
        let mut reg_args = vec![
            i(self.kas),
            i(self.daa),
            i(self.arg_seq.unwrap_or(new_seq)),
            i(self.rate),
            i(self.arg_index.unwrap_or(out_state.index)),
            i(self.arg_rate_daa.unwrap_or(out_state.last_rate_daa)),
        ];
        reg_args.extend(args_satz.args());
        reg_args.extend(satz.quorum(digest, &who));
        let mut inputs = vec![];
        let mut outputs = vec![];
        if self.with_register {
            inputs.push(w.reg_in(&w.reg, "attestPrice", reg_args));
            outputs.push(w.reg_out(&self.reg_out.clone().unwrap_or_else(|| w.reg_after_price(self.daa)), 0));
        }
        if self.with_oracle {
            let (k, d, r) = self.oracle_args.unwrap_or((self.kas, self.daa, self.rate));
            let oargs = if self.oracle_entry == "update" { vec![i(k), i(d), i(r)] } else { vec![] };
            let a = inputs.len() as u16;
            inputs.push(w.oracle_in(w.oracle, self.oracle_entry, oargs));
            outputs.push(w.oracle_out(out_state, a));
        }
        execute(inputs, outputs, self.lock_time.unwrap_or(self.daa as u64), vec![])
    }
}
