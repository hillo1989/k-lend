//! Gemeinsame Bausteine der v5-Tests (Register v5, Orakel v5): Kompilieren,
//! Digests wie in den Verträgen, Transaktionen mit Payload, Locktime und
//! Sequenz (this.ageDaa) bauen und JEDEN Input in der echten Skript-Engine
//! (rusty-kaspa a41a333) ausführen. Vorlage: tests/v4common/mod.rs.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
pub mod common;

use common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{
    CovenantBinding, ScriptPublicKey, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_txscript::pay_to_script_hash_script;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

pub const ORACLE_SRC: &str = include_str!("../../../contracts/price_oracle_v5.sil");
pub const REGISTER_SRC: &str = include_str!("../../../contracts/signer_register_v5.sil");

pub const REG_COV: Hash = Hash::from_bytes([0x5e; 32]);
pub const ORACLE_COV: Hash = Hash::from_bytes([0x0a; 32]);
pub const OTHER_COV: Hash = Hash::from_bytes([0x0d; 32]);

pub const E8: i64 = 100_000_000;
pub const REG_V: i64 = E8;
pub const ORACLE_V: i64 = E8;
pub const TICKET_V: i64 = E8;

pub const TAG_PRICE: u8 = 0x01;
pub const TAG_ROTATE: u8 = 0x02;
pub const TAG_CANCEL: u8 = 0x03;
pub const TAG_EMERG: u8 = 0x04;
pub const TAG_LOCK: u8 = 0x05;

/// 1 Tag bei 10 BPS
pub const DAY: i64 = 864_000;

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
    pub fn args(&self) -> Vec<ArtifactValue> {
        vec![i(self.n()), i(self.t), i(self.t_rot), ArtifactValue::Array(self.pubs().into_iter().map(ArtifactValue::Bytes).collect())]
    }
    pub fn quorum(&self, digest: [u8; 32], who: &[usize]) -> Vec<ArtifactValue> {
        let key = |j: usize| self.keys.get(j).copied().unwrap_or_else(random_keypair);
        let sigs = who.iter().map(|&j| ArtifactValue::Bytes(key(j).sign_schnorr(Message::from_digest(digest)).as_ref().to_vec())).collect();
        let idx = who.iter().map(|&j| i(j as i64)).collect();
        vec![ArtifactValue::Array(sigs), ArtifactValue::Array(idx)]
    }
    pub fn first(k: i64) -> Vec<usize> {
        (0..k as usize).collect()
    }
}

/// Wächter: jeder einzelne darf sperren
#[derive(Clone)]
pub struct Waechter {
    pub keys: Vec<Keypair>,
}

impl Waechter {
    pub fn new(n: usize) -> Self {
        Self { keys: (0..n).map(|_| random_keypair()).collect() }
    }
    pub fn hash(&self) -> [u8; 32] {
        let mut m = num8(self.keys.len() as i64).to_vec();
        for k in &self.keys {
            m.extend_from_slice(&xonly(k));
        }
        sha(&m)
    }
    /// gn, gkeys, gi, gs: Wächter `who` signiert digest
    pub fn args(&self, digest: [u8; 32], who: usize, signer: &Keypair) -> Vec<ArtifactValue> {
        vec![
            i(self.keys.len() as i64),
            ArtifactValue::Array(self.keys.iter().map(|k| ArtifactValue::Bytes(xonly(k))).collect()),
            i(who as i64),
            ArtifactValue::Bytes(signer.sign_schnorr(Message::from_digest(digest)).as_ref().to_vec()),
        ]
    }
}

// ------------------------------------------------------------------ Orakel ----

#[derive(Clone, Copy, Debug)]
pub struct OCfg {
    pub max_rate: i64,
    pub rate_step: i64,
    pub rate_gap: i64,
    pub freeze_after: i64,
    pub jump_bps: i64,
    pub ref_after: i64,
}

/// Vorschläge aus docs/v5-entwurf.md
pub const OCFG: OCfg = OCfg { max_rate: 634_195_839, rate_step: 15_854_896, rate_gap: 36_000, freeze_after: 72_000, jump_bps: 2_500, ref_after: 6 };

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OSt {
    pub kas_usd: i64,
    pub daa: i64,
    pub seq: i64,
    pub rate: i64,
    pub index: i64,
    pub frozen: bool,
    pub last_rate_daa: i64,
    pub ref_usd: i64,
    pub cand_usd: i64,
    pub cand_seq: i64,
}

pub const OSTART: OSt = OSt {
    kas_usd: 4_000_000,
    daa: 1_000_000,
    seq: 7,
    rate: 158_548_959,
    index: 1_000_000_000,
    frozen: false,
    last_rate_daa: 1_000_000,
    ref_usd: 4_000_000,
    cand_usd: 4_000_000,
    cand_seq: 7,
};

pub fn oracle_art(reg_cov: Hash, c: OCfg, s: OSt) -> SilAbiArtifact {
    let args = vec![
        b(reg_cov.as_bytes().as_slice()),
        i(c.max_rate),
        i(c.rate_step),
        i(c.rate_gap),
        i(c.freeze_after),
        i(c.jump_bps),
        i(c.ref_after),
        i(s.kas_usd),
        i(s.daa),
        i(s.seq),
        i(s.rate),
        i(s.index),
        ArtifactValue::Bool(s.frozen),
        i(s.last_rate_daa),
        i(s.ref_usd),
        i(s.cand_usd),
        i(s.cand_seq),
    ];
    compile_to_sil_abi_artifact_with_options(ORACLE_SRC, &args, CompileOptions::default()).expect("Orakel v5 kompiliert")
}

pub fn next_index(prev: OSt, new_daa: i64) -> Option<i64> {
    let delta = new_daa - prev.daa;
    let growth = prev.rate.checked_mul(delta)? / 1_000_000_000;
    prev.index.checked_add(prev.index.checked_mul(growth)? / 1_000_000_000)
}

/// Folgezustand eines ehrlichen Updates (wie price_oracle_v5.sil)
pub fn oracle_next(c: OCfg, prev: OSt, kas: i64, daa: i64, rate: i64) -> OSt {
    let seq = prev.seq + 1;
    let rot = seq - prev.cand_seq >= c.ref_after;
    OSt {
        kas_usd: kas,
        daa,
        seq,
        rate,
        index: next_index(prev, daa).expect("kein Überlauf"),
        frozen: false,
        last_rate_daa: if rate != prev.rate { daa } else { prev.last_rate_daa },
        ref_usd: if rot { prev.cand_usd } else { prev.ref_usd },
        cand_usd: if rot { kas } else { prev.cand_usd },
        cand_seq: if rot { seq } else { prev.cand_seq },
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
    pub min_gap: i64,
}

/// Vorschläge v5: ab 3 Schlüssel / t ≥ 2, 14 Tage, 30 Tage, 7 Tage, 5 min
pub const RCFG: RCfg = RCfg { min_n: 3, min_t: 2, rot_delay: 14 * DAY, emerg_after: 30 * DAY, emerg_delay: 7 * DAY, min_gap: 3_000 };
/// Alleinbetrieb: ab 1-von-1
pub const RCFG_SOLO: RCfg = RCfg { min_n: 1, min_t: 1, ..RCFG };

#[derive(Clone, Debug, PartialEq)]
pub struct RSt {
    pub ticket: bool,
    pub set: [u8; 32],
    pub fb: [u8; 32],
    pub guard: [u8; 32],
    pub nonce: i64,
    pub emerg: bool,
    pub locked: bool,
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
        i(c.min_gap),
        ArtifactValue::Bool(s.ticket),
        b(&s.set),
        b(&s.fb),
        b(&s.guard),
        i(s.nonce),
        ArtifactValue::Bool(s.emerg),
        ArtifactValue::Bool(s.locked),
        i(s.last_daa),
        b(&s.oracle_cov),
        b(&s.oracle_tpl),
        i(s.pre),
        i(s.suf),
        ArtifactValue::Bool(s.init),
    ];
    compile_to_sil_abi_artifact_with_options(REGISTER_SRC, &args, CompileOptions::default()).expect("Register v5 kompiliert")
}

/// Ankündigung (Payload) newSet ‖ newFb ‖ newGuard
pub fn announcement(set: &[u8; 32], fb: &[u8; 32], guard: &[u8; 32]) -> Vec<u8> {
    let mut v = set.to_vec();
    v.extend_from_slice(fb);
    v.extend_from_slice(guard);
    v
}

pub fn rot_digest(reg_cov: Hash, tag: u8, nonce: i64, ann: &[u8]) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(tag);
    m.extend_from_slice(&num8(nonce));
    m.extend_from_slice(ann);
    sha(&m)
}

pub fn tag_digest(reg_cov: Hash, tag: u8, nonce: i64) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(tag);
    m.extend_from_slice(&num8(nonce));
    sha(&m)
}

// ------------------------------------------------------- Tx-Bausteine ----

pub enum Call {
    Entry { art: SilAbiArtifact, entry: &'static str, args: Vec<ArtifactValue> },
    Raw(Vec<u8>),
}

pub struct In {
    pub utxo: UtxoEntry,
    pub call: Call,
    pub sequence: u64,
}

pub fn entry_in(art: &SilAbiArtifact, value: i64, cov: Hash, entry: &'static str, args: Vec<ArtifactValue>) -> In {
    In { utxo: cov_utxo(art, value, cov), call: Call::Entry { art: art.clone(), entry, args }, sequence: 0 }
}

pub fn cov_utxo(art: &SilAbiArtifact, value: i64, cov: Hash) -> UtxoEntry {
    UtxoEntry::new(value as u64, pay_to_script_hash_script(&bytecode(art)), 0, false, Some(cov))
}

pub fn opt_true_spk() -> ScriptPublicKey {
    ScriptPublicKey::new(0, vec![kaspa_txscript::opcodes::codes::OpTrue].into())
}

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

fn outpoint(i: usize) -> TransactionOutpoint {
    TransactionOutpoint { transaction_id: TransactionId::from_bytes([0x40 + i as u8; 32]), index: i as u32 }
}

pub fn contract_name(a: &SilAbiArtifact) -> String {
    a.contracts.keys().next().unwrap().clone()
}

pub fn build(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64, payload: Vec<u8>) -> (Transaction, Vec<UtxoEntry>) {
    let entries: Vec<UtxoEntry> = inputs.iter().map(|i| i.utxo.clone()).collect();
    let mut final_inputs = vec![];
    for (idx, input) in inputs.into_iter().enumerate() {
        let script = match input.call {
            Call::Entry { art, entry, args } => {
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

pub fn execute(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64, payload: Vec<u8>) -> Vec<Result<(), String>> {
    let (tx, entries) = build(inputs, outputs, lock_time, payload);
    (0..tx.inputs.len()).map(|i| execute_input_with_covenants(tx.clone(), entries.clone(), i).map_err(|e| format!("{e:?}"))).collect()
}

pub fn all_ok(r: &[Result<(), String>]) -> bool {
    r.iter().all(|x| x.is_ok())
}

pub fn tpl(art: &SilAbiArtifact) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    compiled_template_parts_and_hash(art)
}

// ------------------------------------------------------------------- Welt ----

#[derive(Clone)]
pub struct World {
    pub deployer: Keypair,
    pub rcfg: RCfg,
    pub ocfg: OCfg,
    pub set: Satz,
    pub fb: Option<Satz>,
    pub guard: Waechter,
    pub reg: RSt,
    pub oracle: OSt,
}

impl World {
    pub fn new(set: Satz, fb: Option<Satz>, rcfg: RCfg) -> Self {
        let ocfg = OCFG;
        let otpl = tpl(&oracle_art(REG_COV, ocfg, OSTART));
        let guard = Waechter::new(2);
        let reg = RSt {
            ticket: false,
            set: set.hash(),
            fb: fb.as_ref().map(|f| f.hash()).unwrap_or([0; 32]),
            guard: guard.hash(),
            nonce: 0,
            emerg: false,
            locked: false,
            last_daa: OSTART.daa,
            oracle_cov: ORACLE_COV.as_bytes(),
            oracle_tpl: otpl.2.clone(),
            pre: otpl.0.len() as i64,
            suf: otpl.1.len() as i64,
            init: true,
        };
        Self { deployer: random_keypair(), rcfg, ocfg, set, fb, guard, reg, oracle: OSTART }
    }

    /// Vorschlag: Hauptsatz 2 von 3, Notfallsatz 1 von 1 (Alleinbetrieb-Grenzen), 2 Wächter
    pub fn standard() -> Self {
        Self::new(Satz::new(3, 2, 2), Some(Satz::new(1, 1, 1)), RCFG_SOLO)
    }

    /// heute: 1 von 1, kein Notfallsatz
    pub fn solo() -> Self {
        Self::new(Satz::new(1, 1, 1), Some(Satz::new(1, 1, 1)), RCFG_SOLO)
    }

    pub fn reg_art(&self, s: &RSt) -> SilAbiArtifact {
        register_art(&xonly(&self.deployer), self.rcfg, s)
    }

    pub fn oracle_art(&self, s: OSt) -> SilAbiArtifact {
        oracle_art(REG_COV, self.ocfg, s)
    }

    /// Register-Eingang; Sequenz = Mindestabstand (Haupt-UTXO alt genug)
    pub fn reg_in(&self, s: &RSt, entry: &'static str, args: Vec<ArtifactValue>) -> In {
        let mut x = entry_in(&self.reg_art(s), if s.ticket { TICKET_V } else { REG_V }, REG_COV, entry, args);
        x.sequence = self.rcfg.min_gap as u64;
        x
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

    pub fn reg_after_price(&self, daa: i64) -> RSt {
        let mut r = self.reg.clone();
        r.last_daa = daa;
        if r.emerg {
            r.nonce += 1;
            r.emerg = false;
        }
        r
    }

    pub fn next(&self, kas: i64, daa: i64, rate: i64) -> OSt {
        oracle_next(self.ocfg, self.oracle, kas, daa, rate)
    }

    /// Sperr-Tx des Wächters `who`; `with_oracle`: Umgehungsversuch mit Orakel-Update
    pub fn lock_tx(&self, who: usize, out: Option<RSt>) -> Vec<Result<(), String>> {
        let d = tag_digest(REG_COV, TAG_LOCK, self.reg.nonce + 1);
        let args = self.guard.args(d, who, &self.guard.keys[who]);
        let out = out.unwrap_or(RSt { nonce: self.reg.nonce + 1, emerg: false, locked: true, ..self.reg.clone() });
        let mut x = self.reg_in(&self.reg, "guardLock", args);
        x.sequence = 0;
        execute(vec![x], vec![self.reg_out(&out, 0)], 0, vec![])
    }

    pub fn locked(&self) -> Self {
        let mut w = self.clone();
        w.reg.nonce += 1;
        w.reg.emerg = false;
        w.reg.locked = true;
        w
    }
}

/// Ein Preis-Update (Register attestPrice + Orakel update)
#[derive(Clone)]
pub struct Attest {
    pub kas: i64,
    pub daa: i64,
    pub rate: i64,
    pub satz: Option<Satz>,
    pub signers: Option<Vec<usize>>,
    pub signed_seq: Option<i64>,
    pub arg_seq: Option<i64>,
    pub oracle_out: Option<OSt>,
    pub reg_out: Option<RSt>,
    pub oracle_entry: &'static str,
    pub oracle_args: Option<(i64, i64, i64)>,
    pub with_oracle: bool,
    /// Sequenz des Register-Eingangs (Standard: Mindestabstand)
    pub reg_seq: Option<u64>,
    /// feste Signatur-Bytes (Replay)
    pub quorum: Option<Vec<ArtifactValue>>,
}

impl Attest {
    pub fn new(kas: i64, daa: i64, rate: i64) -> Self {
        Self {
            kas,
            daa,
            rate,
            satz: None,
            signers: None,
            signed_seq: None,
            arg_seq: None,
            oracle_out: None,
            reg_out: None,
            oracle_entry: "update",
            oracle_args: None,
            with_oracle: true,
            reg_seq: None,
            quorum: None,
        }
    }

    pub fn next(w: &World) -> Self {
        Self::new(w.oracle.kas_usd, w.oracle.daa + 600, w.oracle.rate)
    }

    pub fn expected(&self, w: &World) -> OSt {
        w.next(self.kas, self.daa, self.rate)
    }

    pub fn quorum_of(&self, w: &World) -> Vec<ArtifactValue> {
        let satz = self.satz.clone().unwrap_or_else(|| w.set.clone());
        let digest = price_digest(ORACLE_COV, self.kas, self.daa, self.signed_seq.unwrap_or(w.oracle.seq + 1), self.rate);
        satz.quorum(digest, &self.signers.clone().unwrap_or_else(|| Satz::first(satz.t)))
    }

    pub fn run(&self, w: &World) -> Vec<Result<(), String>> {
        let satz = self.satz.clone().unwrap_or_else(|| w.set.clone());
        let out = self.oracle_out.unwrap_or_else(|| self.expected(w));
        let mut reg_args = vec![
            i(self.kas),
            i(self.daa),
            i(self.arg_seq.unwrap_or(w.oracle.seq + 1)),
            i(self.rate),
            i(out.index),
            i(out.last_rate_daa),
            i(out.ref_usd),
            i(out.cand_usd),
            i(out.cand_seq),
        ];
        reg_args.extend(satz.args());
        reg_args.extend(self.quorum.clone().unwrap_or_else(|| self.quorum_of(w)));
        let mut rin = w.reg_in(&w.reg, "attestPrice", reg_args);
        if let Some(s) = self.reg_seq {
            rin.sequence = s;
        }
        let mut inputs = vec![rin];
        let mut outputs = vec![w.reg_out(&self.reg_out.clone().unwrap_or_else(|| w.reg_after_price(self.daa)), 0)];
        if self.with_oracle {
            let (k, d, r) = self.oracle_args.unwrap_or((self.kas, self.daa, self.rate));
            let oargs = if self.oracle_entry == "update" { vec![i(k), i(d), i(r)] } else { vec![] };
            inputs.push(w.oracle_in(w.oracle, self.oracle_entry, oargs));
            outputs.push(w.oracle_out(out, 1));
        }
        execute(inputs, outputs, self.daa as u64, vec![])
    }

    /// führt aus und schreibt die Welt fort
    pub fn apply(&self, w: &mut World) {
        let r = self.run(w);
        assert!(all_ok(&r), "Update: {r:?}");
        let o = self.expected(w);
        w.reg = w.reg_after_price(self.daa);
        w.oracle = o;
    }
}
