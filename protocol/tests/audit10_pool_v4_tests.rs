//! POOL V4: Kopie von audit10_pool_engine.rs für contracts/ghost_pool_v4.sil mit dem Orakel v4
//! (price_oracle_v4.sil). Alle v3-Tests laufen unverändert gegen v4; die
//! v4-Tests (eingefrorenes Orakel, stopWhenFrozen) stehen am Ende.
//!
//! Audit 10 (Opus) – offener Pool: Engine-Belege des Prüfers (audit/10-opus-pool.md),
//! u. a. die tragende Regel L214 (freshGenesis) mit Mutanten-Gegenprobe. Harness aus pool_tests.rs
//! übernommen (Zeilen 6–38, 69–72, 129–525, 873–905), eigene Tests am Ende.
#![allow(dead_code)]
mod common;

use common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::covenant_id::covenant_id;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::tx::{
    CovenantBinding, MutableTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput,
    UtxoEntry,
};
use kaspa_txscript::opcodes::codes::OpTrue;
use kaspa_txscript::pay_to_script_hash_script;
use rand::{Rng, RngCore, SeedableRng, rngs::StdRng, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_covenant_decl_sig_script, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};
use std::collections::BTreeMap;

const POOL_SRC: &str = include_str!("../../contracts/ghost_pool_v4.sil");
const KCC20_SRC: &str = include_str!("../../contracts/ghost_token.sil");
const ORACLE_SRC: &str = include_str!("../../contracts/price_oracle_v4.sil");
const ORACLE_COV: Hash = Hash::from_bytes([0x0c; 32]);
/// KAS-Preis, bei dem der Test-Pool (X/Y = 10 000/460) genau 1 USD je GHOST hat
const KAS_USD_1: i64 = 4_600_000;
/// Band der allgemeinen Tests: so weit, dass es nur Preise über 101 USD sperrt
const WIDE_BAND: i64 = 1_000_000;
const ORACLE_V: i64 = 10 * E8;

const GHOST_COV: Hash = Hash::from_bytes([0x0b; 32]);
const POOL_COV: Hash = Hash::from_bytes([0x0e; 32]);
const LP_COV: Hash = Hash::from_bytes([0x0f; 32]);
const KCC20_MAX_INS: i64 = 3;
const KCC20_MAX_OUTS: i64 = 2;
const ID_PUBKEY: u8 = 0x00;
const ID_COV: u8 = 0x02;
const E8: i64 = 100_000_000;
const FEE: i64 = 30; // 0,3 %
const TOKV: i64 = E8; // KAS-Wert einer Token-UTXO

fn i(v: i64) -> ArtifactValue {
    ArtifactValue::Int(v)
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

fn kcc20(owner: &[u8], typ: u8, amount: i64, minter: bool) -> SilAbiArtifact {
    compile_to_sil_abi_artifact_with_options(
        KCC20_SRC,
        &[ArtifactValue::Bytes(owner.to_vec()), i(amount), ArtifactValue::Byte(typ), ArtifactValue::Bool(minter), i(KCC20_MAX_INS), i(KCC20_MAX_OUTS)],
        CompileOptions::default(),
    )
    .expect("KCC20 kompiliert")
}

struct Env {
    creator: Keypair,
    tpl: (Vec<u8>, Vec<u8>, Vec<u8>),
    /// Audit 10: Vertragsquelle (für Mutanten-Gegenproben austauschbar)
    src: String,
    /// Kursband (Pool v2): Orakel mit diesem KAS-Preis, Bandbreite in bps
    kas_usd: i64,
    band: i64,
    /// v4: Orakel eingefroren
    frozen: std::cell::Cell<bool>,
    /// v4: Konstruktor stopWhenFrozen
    stop_when_frozen: bool,
}

impl Env {
    fn new() -> Self {
        Self { creator: random_keypair(), tpl: compiled_template_parts_and_hash(&kcc20(&[0; 32], ID_COV, 0, true)), src: POOL_SRC.to_string(), kas_usd: KAS_USD_1, band: WIDE_BAND, frozen: std::cell::Cell::new(false), stop_when_frozen: true }
    }
    fn oracle(&self) -> SilAbiArtifact {
        // v4: Register-ID, Zinsrahmen, Frist; Zustand mit frozen und lastRateDaa
        compile_to_sil_abi_artifact_with_options(
            ORACLE_SRC,
            &[
                ArtifactValue::Bytes(vec![0x5e; 32]),
                i(1_000_000_000),
                i(15_854_896),
                i(36_000),
                i(72_000),
                i(self.kas_usd),
                i(1_000),
                i(0),
                i(0),
                i(1_000_000_000),
                ArtifactValue::Bool(self.frozen.get()),
                i(1_000),
            ],
            CompileOptions::default(),
        )
        .expect("Orakel kompiliert")
    }
    fn otpl(&self) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        compiled_template_parts_and_hash(&self.oracle())
    }
    /// Pool mit Kursband `band` bps bei KAS-Preis `kas_usd`
    fn with_band(band: i64, kas_usd: i64) -> Self {
        let mut e = Self::new();
        e.band = band;
        e.kas_usd = kas_usd;
        e
    }
    fn pool(&self, lp: Hash, initialized: bool) -> SilAbiArtifact {
        let (kp, ks, kh) = &self.tpl;
        compile_to_sil_abi_artifact_with_options(
            &self.src,
            &[
                ArtifactValue::Bytes(GHOST_COV.as_bytes().to_vec()),
                i(kp.len() as i64),
                i(ks.len() as i64),
                ArtifactValue::Bytes(kh.clone()),
                i(FEE),
                ArtifactValue::Bytes(xonly(&self.creator)),
                ArtifactValue::Bytes(ORACLE_COV.as_bytes().to_vec()),
                i(self.otpl().0.len() as i64),
                i(self.otpl().1.len() as i64),
                ArtifactValue::Bytes(self.otpl().2),
                i(self.band),
                ArtifactValue::Bool(self.stop_when_frozen),
                ArtifactValue::Bytes(lp.as_bytes().to_vec()),
                ArtifactValue::Bool(initialized),
            ],
            CompileOptions::default(),
        )
        .expect("Pool kompiliert")
    }
}

#[derive(Clone)]
struct Tok {
    owner: Vec<u8>,
    typ: u8,
    amount: i64,
    minter: bool,
}

impl Tok {
    /// GHOST-Reserve bzw. sonstiger pool-eigener Token
    fn pool(amount: i64) -> Self {
        Self { owner: POOL_COV.as_bytes().to_vec(), typ: ID_COV, amount, minter: false }
    }
    /// Anteils-Minter des Pools mit S Anteilen
    fn minter(total: i64) -> Self {
        Self { owner: POOL_COV.as_bytes().to_vec(), typ: ID_COV, amount: total, minter: true }
    }
    fn to(k: &Keypair, amount: i64) -> Self {
        Self { owner: xonly(k), typ: ID_PUBKEY, amount, minter: false }
    }
    fn arg(&self) -> ArtifactValue {
        BTreeMap::from([
            ("ownerIdentifier".to_string(), ArtifactValue::Bytes(self.owner.clone())),
            ("identifierType".to_string(), ArtifactValue::Byte(self.typ)),
            ("amount".to_string(), i(self.amount)),
            ("isMinter".to_string(), ArtifactValue::Bool(self.minter)),
        ])
        .into()
    }
    fn art(&self) -> SilAbiArtifact {
        kcc20(&self.owner, self.typ, self.amount, self.minter)
    }
}

enum Call {
    Entry { art: SilAbiArtifact, entry: &'static str, args: Vec<ArtifactValue>, sig_by: Option<(usize, Keypair)> },
    Leader { art: SilAbiArtifact, new_states: Vec<ArtifactValue>, sig_by: Option<Keypair> },
    Delegate { art: SilAbiArtifact, sig_by: Keypair },
    Plain,
}

struct In {
    utxo: UtxoEntry,
    call: Call,
    op: TransactionOutpoint,
}

fn cov_utxo(art: &SilAbiArtifact, value: i64, cov: Hash) -> UtxoEntry {
    UtxoEntry::new(value as u64, pay_to_script_hash_script(&bytecode(art)), 0, false, Some(cov))
}

fn cov_out(art: &SilAbiArtifact, value: i64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput {
        value: value as u64,
        script_public_key: pay_to_script_hash_script(&bytecode(art)),
        covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }),
    }
}

fn plain_spk() -> ScriptPublicKey {
    ScriptPublicKey::new(0, vec![OpTrue].into())
}

/// Outpoint aus Tx `tx` (ein Byte als Kennung) mit Index `i`
fn op(tx: u8, i: u32) -> TransactionOutpoint {
    TransactionOutpoint { transaction_id: TransactionId::from_bytes([tx; 32]), index: i }
}
/// Die letzte Pool-Tx: Pool (0), GHOST-Reserve (1) und Anteils-Minter (2) stammen daraus
const POOL_TX: u8 = 0x50;

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

fn execute(inputs: Vec<In>, outputs: Vec<TransactionOutput>) -> Vec<Result<(), String>> {
    let entries: Vec<UtxoEntry> = inputs.iter().map(|i| i.utxo.clone()).collect();
    let ops: Vec<TransactionOutpoint> = inputs.iter().map(|i| i.op).collect();
    let bare: Vec<TransactionInput> = ops.iter().map(|o| TransactionInput::new_with_compute_budget(*o, vec![], 0, 0)).collect();
    let unsigned = Transaction::new(1, bare, outputs.clone(), 0, Default::default(), 0, vec![]);
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
            Call::Leader { art, new_states, sig_by } => {
                let sig = sig_by.map(|k| sign(&unsigned, &entries, idx, &k)).unwrap_or(vec![0; 65]);
                let args = vec![ArtifactValue::Array(new_states), ArtifactValue::Bytes(sig), ArtifactValue::Byte(0)];
                let mut s = encode_contract_covenant_decl_sig_script(&art, &contract_name(&art), "transfer", true, &args).expect("leader");
                s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
                s
            }
            Call::Delegate { art, sig_by } => {
                let args = vec![ArtifactValue::Bytes(sign(&unsigned, &entries, idx, &sig_by)), ArtifactValue::Byte(0)];
                let mut s = encode_contract_covenant_decl_sig_script(&art, &contract_name(&art), "transfer", false, &args).expect("delegate");
                s.extend_from_slice(&push_redeem_script(&bytecode(&art)));
                s
            }
            Call::Plain => vec![],
        };
        final_inputs.push(TransactionInput::new_with_compute_budget(ops[idx], script, 0, 0));
    }
    let tx = Transaction::new(1, final_inputs, outputs, 0, Default::default(), 0, vec![]);
    (0..tx.inputs.len()).map(|i| execute_input_with_covenants(tx.clone(), entries.clone(), i).map_err(|e| format!("{e:?}"))).collect()
}

fn all_ok(r: &[Result<(), String>]) -> bool {
    r.iter().all(|x| x.is_ok())
}

// ---------------------------------------------------------- Referenz ----

fn fee_of(d: i64) -> i64 {
    (d as u128 * FEE as u128).div_ceil(10_000) as i64
}

/// Genau die Tauschregel des Vertrags, in u128
fn swap_ok(x: i64, y: i64, x2: i64, y2: i64) -> bool {
    let xa = if x2 > x { x2 - fee_of(x2 - x) } else { x2 };
    let ya = if y2 > y { y2 - fee_of(y2 - y) } else { y2 };
    xa as u128 * ya as u128 >= x as u128 * y as u128
}

fn max_ghost_out(x: i64, y: i64, dx: i64) -> i64 {
    let (mut lo, mut hi) = (0, y - 1);
    while lo < hi {
        let m = (lo + hi + 1) / 2;
        if swap_ok(x, y, x + dx, y - m) { lo = m } else { hi = m - 1 }
    }
    lo
}

fn max_kas_out(x: i64, y: i64, dy: i64) -> i64 {
    let (mut lo, mut hi) = (0, x - 1);
    while lo < hi {
        let m = (lo + hi + 1) / 2;
        if swap_ok(x, y, x - m, y + dy) { lo = m } else { hi = m - 1 }
    }
    lo
}

/// Größte Anteilszahl für eine Einlage (dx, dy): min(⌊S·dx/x⌋, ⌊S·dy/y⌋)
fn max_shares(s: i64, x: i64, y: i64, dx: i64, dy: i64) -> i64 {
    ((s as u128 * dx as u128 / x as u128).min(s as u128 * dy as u128 / y as u128)) as i64
}

/// Größte Auszahlung für m von S Anteilen: (⌊x·m/S⌋, ⌊y·m/S⌋)
fn max_payout(s: i64, x: i64, y: i64, m: i64) -> (i64, i64) {
    ((x as u128 * m as u128 / s as u128) as i64, (y as u128 * m as u128 / s as u128) as i64)
}

// ------------------------------------------------------------- Pool-Tx ----

const X: i64 = 10_000 * E8; // 10 000 KAS im Pool
const Y: i64 = 460 * E8; // 460 GHOST → 0,046 USD je KAS
const S: i64 = 20_000 * E8; // Anteile insgesamt (davon ein Teil gesperrt)
const MINV: i64 = E8; // KAS-Wert der Minter-UTXO

/// Eine Pool-Tx mit frei wählbaren Teilen. Aufbau:
/// Eingänge  [0] Pool, [1] GHOST-Leader (Reserve), [2] Anteils-Leader (Minter),
///           GHOST-Delegates, Anteils-Delegates, Händler-KAS
/// Ausgänge  Pool-Fortsetzung(en), GHOST-Ausgänge, Anteils-Ausgänge, KAS
struct Tx {
    entry: &'static str,
    x: i64,
    x2: i64,
    in_lp: Hash,
    in_init: bool,
    next_lp: Hash,
    pool_outputs: usize,
    signer: Option<Keypair>,
    // GHOST
    reserve: Option<(Tok, TransactionOutpoint, Option<Keypair>)>,
    ghost_ins: Vec<(Tok, Option<Keypair>)>,
    ghost_outs: Vec<Tok>,
    ghost_claimed: Option<Vec<Tok>>,
    reserve_out_value: Option<i64>,
    // Anteile
    minter: Option<(Tok, TransactionOutpoint)>,
    lp_ins: Vec<(Tok, Option<Keypair>)>,
    lp_outs: Vec<Tok>,
    lp_claimed: Option<Vec<Tok>>,
    minter_out_value: i64,
    // nur init: Genesis-Ausgang des Anteils-Tokens
    lp_genesis: Option<Tok>,
    lp_genesis_twice: bool,
    /// Kursband-Test: Orakel unter einer anderen Covenant-ID
    oracle_cov: Option<Hash>,
}

impl Tx {
    fn base(entry: &'static str, x: i64, x2: i64) -> Self {
        Self {
            entry,
            x,
            x2,
            in_lp: LP_COV,
            in_init: true,
            next_lp: LP_COV,
            pool_outputs: 1,
            signer: None,
            reserve: Some((Tok::pool(Y), op(POOL_TX, 1), None)),
            ghost_ins: vec![],
            ghost_outs: vec![],
            ghost_claimed: None,
            reserve_out_value: None,
            minter: Some((Tok::minter(S), op(POOL_TX, 2))),
            lp_ins: vec![],
            lp_outs: vec![Tok::minter(S)],
            lp_claimed: None,
            minter_out_value: MINV,
            lp_genesis: None,
            lp_genesis_twice: false,
            oracle_cov: None,
        }
    }

    /// KAS gegen GHOST
    fn buy(dx: i64, dy: i64) -> Self {
        let mut t = Self::base("swap", X, X + dx);
        t.ghost_outs = vec![Tok::pool(Y - dy), Tok::to(&random_keypair(), dy)];
        t
    }

    /// GHOST gegen KAS
    fn sell(dy: i64, dx: i64) -> Self {
        let trader = random_keypair();
        let mut t = Self::base("swap", X, X - dx);
        t.ghost_ins = vec![(Tok::to(&trader, dy), Some(trader))];
        t.ghost_outs = vec![Tok::pool(Y + dy)];
        t
    }

    /// Einlage (dx, dy) für m Anteile
    fn add(dx: i64, dy: i64, m: i64) -> Self {
        let lp = random_keypair();
        let mut t = Self::base("add", X, X + dx);
        t.ghost_ins = vec![(Tok::to(&lp, dy), Some(lp))];
        t.ghost_outs = vec![Tok::pool(Y + dy)];
        t.lp_outs = vec![Tok::minter(S + m), Tok::to(&lp, m)];
        t
    }

    /// Inhaber mit `have` Anteilen verbrennt m und bekommt (dx, dy)
    fn remove(have: i64, m: i64, dx: i64, dy: i64) -> Self {
        let lp = random_keypair();
        let mut t = Self::base("remove", X, X - dx);
        t.ghost_outs = if dy > 0 { vec![Tok::pool(Y - dy), Tok::to(&lp, dy)] } else { vec![Tok::pool(Y)] };
        t.lp_ins = vec![(Tok::to(&lp, have), Some(lp))];
        t.lp_outs = if have > m { vec![Tok::minter(S - m), Tok::to(&lp, have - m)] } else { vec![Tok::minter(S - m)] };
        t
    }

    fn run(&self, env: &Env) -> Vec<Result<(), String>> {
        let pool = env.pool(self.in_lp, self.in_init);
        let next = env.pool(self.next_lp, true);
        let ghost_claimed: Vec<ArtifactValue> = self.ghost_claimed.as_ref().unwrap_or(&self.ghost_outs).iter().map(Tok::arg).collect();
        let ghost_actual: Vec<ArtifactValue> = self.ghost_outs.iter().map(Tok::arg).collect();
        let lp_claimed: Vec<ArtifactValue> = self.lp_claimed.as_ref().unwrap_or(&self.lp_outs).iter().map(Tok::arg).collect();
        let lp_actual: Vec<ArtifactValue> = self.lp_outs.iter().map(Tok::arg).collect();

        // Ausgänge zuerst: ihre Indizes braucht init
        let mut outputs = vec![];
        for _ in 0..self.pool_outputs {
            outputs.push(cov_out(&next, self.x2, 0, POOL_COV));
        }
        let ghost_leader: u16 = 1;
        for (j, t) in self.ghost_outs.iter().enumerate() {
            let v = if j == 0 { self.reserve_out_value.unwrap_or(TOKV) } else { TOKV };
            outputs.push(cov_out(&t.art(), v, ghost_leader, GHOST_COV));
        }
        let lp_leader: u16 = if self.reserve.is_some() || !self.ghost_ins.is_empty() { 2 } else { 1 };
        for (j, t) in self.lp_outs.iter().enumerate() {
            let v = if j == 0 { self.minter_out_value } else { TOKV };
            outputs.push(cov_out(&t.art(), v, lp_leader, self.in_lp));
        }
        let mut lp_out_idx = 0i64;
        if let Some(g) = &self.lp_genesis {
            // Genesis-ID wie im Konsens: aus dem autorisierenden Eingang (Pool) und dem Ausgang
            let n = if self.lp_genesis_twice { 2 } else { 1 };
            let mut bare = vec![];
            for k in 0..n {
                bare.push((outputs.len() + k) as u32);
            }
            let proto = TransactionOutput { value: MINV as u64, script_public_key: pay_to_script_hash_script(&bytecode(&g.art())), covenant: None };
            let id = covenant_id(op(POOL_TX, 0), bare.iter().map(|&k| (k, &proto)));
            lp_out_idx = outputs.len() as i64;
            for _ in 0..n {
                let mut o = proto.clone();
                o.covenant = Some(CovenantBinding { authorizing_input: 0, covenant_id: id });
                outputs.push(o);
            }
        }
        // Kursband: swap und init lesen das Orakel (Eingang vor dem Funding)
        let with_oracle = self.entry == "swap" || self.entry == "init";
        let n_ghost_in = self.reserve.is_some() as usize + self.ghost_ins.len();
        let oracle_idx = 1 + n_ghost_in + self.minter.is_some() as usize + self.lp_ins.len();
        let oracle_art = env.oracle();
        if with_oracle {
            outputs.push(cov_out(&oracle_art, ORACLE_V, oracle_idx as u16, self.oracle_cov.unwrap_or(ORACLE_COV)));
        }
        outputs.push(TransactionOutput { value: E8 as u64, script_public_key: plain_spk(), covenant: None });

        // Einstieg des Pools
        let (args, sig_by) = match self.entry {
            "init" => {
                let (kp, ks, _) = &env.tpl;
                (
                    vec![i(lp_out_idx), i(oracle_idx as i64), ArtifactValue::Bytes(kp.clone()), ArtifactValue::Bytes(ks.clone()), ArtifactValue::Array(ghost_claimed)],
                    Some((5, self.signer.unwrap_or_else(random_keypair))),
                )
            }
            "swap" => (vec![i(oracle_idx as i64), ArtifactValue::Array(ghost_claimed), ArtifactValue::Array(lp_claimed)], None),
            _ => (vec![ArtifactValue::Array(ghost_claimed), ArtifactValue::Array(lp_claimed)], None),
        };
        let mut inputs = vec![In { utxo: cov_utxo(&pool, self.x, POOL_COV), call: Call::Entry { art: pool.clone(), entry: self.entry, args, sig_by }, op: op(POOL_TX, 0) }];
        // GHOST-Leader: Reserve oder (bei init) der erste Token des Gründers
        let mut ghost_ins = self.ghost_ins.clone();
        if let Some((t, o, k)) = &self.reserve {
            inputs.push(In { utxo: cov_utxo(&t.art(), TOKV, GHOST_COV), call: Call::Leader { art: t.art(), new_states: ghost_actual.clone(), sig_by: *k }, op: *o });
        } else if !ghost_ins.is_empty() {
            let (t, k) = ghost_ins.remove(0);
            inputs.push(In { utxo: cov_utxo(&t.art(), TOKV, GHOST_COV), call: Call::Leader { art: t.art(), new_states: ghost_actual.clone(), sig_by: k }, op: op(0x60, 0) });
        }
        if let Some((t, o)) = &self.minter {
            inputs.push(In { utxo: cov_utxo(&t.art(), MINV, self.in_lp), call: Call::Leader { art: t.art(), new_states: lp_actual.clone(), sig_by: None }, op: *o });
        }
        for (n, (t, k)) in ghost_ins.iter().enumerate() {
            let call = Call::Delegate { art: t.art(), sig_by: k.unwrap_or_else(random_keypair) };
            inputs.push(In { utxo: cov_utxo(&t.art(), TOKV, GHOST_COV), call, op: op(0x61 + n as u8, 0) });
        }
        for (n, (t, k)) in self.lp_ins.iter().enumerate() {
            let call = Call::Delegate { art: t.art(), sig_by: k.unwrap_or_else(random_keypair) };
            inputs.push(In { utxo: cov_utxo(&t.art(), TOKV, self.in_lp), call, op: op(0x68 + n as u8, 0) });
        }
        if with_oracle {
            assert_eq!(inputs.len(), oracle_idx, "Orakel-Index");
            let art = oracle_art.clone();
            inputs.push(In { utxo: cov_utxo(&art, ORACLE_V, self.oracle_cov.unwrap_or(ORACLE_COV)), call: Call::Entry { art, entry: "read", args: vec![], sig_by: None }, op: op(0x71, 0) });
        }
        inputs.push(In { utxo: UtxoEntry::new(1_000 * E8 as u64, plain_spk(), 0, false, None), call: Call::Plain, op: op(0x70, 0) });
        execute(inputs, outputs)
    }
}

// ---------------------------------------------------------------- init ----

fn init_tx(env: &Env, x2: i64, reserve: i64, shares: i64) -> Tx {
    let mut t = Tx::base("init", 10 * E8, x2);
    t.in_init = false;
    t.in_lp = Hash::from_bytes([0; 32]);
    t.signer = Some(env.creator);
    t.reserve = None;
    t.minter = None;
    t.lp_outs = vec![];
    t.ghost_ins = vec![(Tok::to(&env.creator, 100 * E8), Some(env.creator))];
    t.ghost_outs = vec![Tok::pool(reserve), Tok::to(&env.creator, 100 * E8 - reserve)];
    t.lp_genesis = Some(Tok::minter(shares));
    t
}

/// Die Fortsetzung muss die echte Genesis-ID tragen; die kennt man erst beim Bau.
fn run_init(env: &Env, mut t: Tx) -> Vec<Result<(), String>> {
    // Genesis-ID vorab ausrechnen (gleiche Rechnung wie in run())
    let n_ghost = t.ghost_outs.len();
    let idx = t.pool_outputs + n_ghost;
    let g = t.lp_genesis.clone().unwrap();
    let proto = TransactionOutput { value: MINV as u64, script_public_key: pay_to_script_hash_script(&bytecode(&g.art())), covenant: None };
    let n = if t.lp_genesis_twice { 2 } else { 1 };
    let ids: Vec<u32> = (0..n).map(|k| (idx + k) as u32).collect();
    let id = covenant_id(op(POOL_TX, 0), ids.iter().map(|&k| (k, &proto)));
    if t.next_lp == LP_COV {
        t.next_lp = id;
    }
    let _ = env;
    t.run(env)
}


// ======================================================= Audit 10 (Opus) ====

use kaspa_lending_protocol::pool as prs;

const MAX_KAS: i64 = 10_000_000_000_000_000;
const MAX_SHARES: i64 = (1 << 60) - 1;

/// Pool-Tx auf einem frei gewählten Stand (S, x, y)
fn at(entry: &'static str, s: i64, x: i64, y: i64) -> Tx {
    let mut t = Tx::base(entry, x, x);
    t.reserve = Some((Tok::pool(y), op(POOL_TX, 1), None));
    t.minter = Some((Tok::minter(s), op(POOL_TX, 2)));
    t.ghost_outs = vec![Tok::pool(y)];
    t.lp_outs = vec![Tok::minter(s)];
    t
}

fn add_at(s: i64, x: i64, y: i64, dx: i64, dy: i64, m: i64) -> Tx {
    let lp = random_keypair();
    let mut t = at("add", s, x, y);
    t.x2 = x + dx;
    t.ghost_ins = vec![(Tok::to(&lp, dy), Some(lp))];
    t.ghost_outs = vec![Tok::pool(y + dy)];
    t.lp_outs = vec![Tok::minter(s + m), Tok::to(&lp, m)];
    t
}

fn remove_at(s: i64, x: i64, y: i64, have: i64, m: i64, dx: i64, dy: i64) -> Tx {
    let lp = random_keypair();
    let mut t = at("remove", s, x, y);
    t.x2 = x - dx;
    t.ghost_outs = if dy > 0 { vec![Tok::pool(y - dy), Tok::to(&lp, dy)] } else { vec![Tok::pool(y)] };
    t.lp_ins = vec![(Tok::to(&lp, have), Some(lp))];
    t.lp_outs = if have > m { vec![Tok::minter(s - m), Tok::to(&lp, have - m)] } else { vec![Tok::minter(s - m)] };
    t
}

/// pool.rs (shares_for_deposit / payout_for) = Vertrag an der Grenze, auch bei
/// großem S; und: Einlegen + sofort Abziehen bringt nie mehr zurück.
#[test]
fn a10_rechnung_pool_rs_gleich_vertrag_bei_add_und_remove() {
    let e = Env::new();
    let mut r = StdRng::seed_from_u64(10);
    let mut n = 0;
    while n < 6 {
        let s = r.gen_range(E8..MAX_SHARES / 4);
        let x = r.gen_range(E8..MAX_KAS / 4);
        let y = r.gen_range(1..MAX_KAS / 4);
        let dx = r.gen_range(1..x);
        let dy = r.gen_range(1..y.max(2));
        let m = prs::shares_for_deposit(s, x, y, dx, dy);
        if m <= 0 || s as i128 + m as i128 > MAX_SHARES as i128 {
            continue;
        }
        n += 1;
        assert!(all_ok(&add_at(s, x, y, dx, dy, m).run(&e)), "add m s={s} x={x} y={y} dx={dx} dy={dy}");
        let rr = add_at(s, x, y, dx, dy, m + 1).run(&e);
        assert!(rr[0].is_err(), "add m+1 muss am Pool scheitern: {rr:?}");
        // sofort wieder abziehen: nie mehr als eingelegt (exakt in u128)
        let (px, py) = prs::payout_for(s + m, x + dx, y + dy, m);
        assert!(px <= dx && py <= dy, "Rundungsgewinn: ({px},{py}) > ({dx},{dy})");
        // Abziehen an der Grenze
        let mr = r.gen_range(1..s / 2);
        let (qx, qy) = prs::payout_for(s, x, y, mr);
        if x - qx < E8 || qy >= y {
            continue;
        }
        assert!(all_ok(&remove_at(s, x, y, mr, mr, qx, qy).run(&e)), "remove s={s} m={mr}");
        assert!(remove_at(s, x, y, mr, mr, qx + 1, qy).run(&e)[0].is_err(), "remove +1 sompi");
        assert!(remove_at(s, x, y, mr, mr, qx, qy + 1).run(&e)[0].is_err(), "remove +1 Einheit");
    }
}

/// S bis genau MAX_SHARES = 2^60−1 und Tausch/Abziehen bei S = MAX_SHARES,
/// Reserven nahe 1e16: keine Überläufe in mul()/productGe.
#[test]
fn a10_grenzen_max_shares_und_max_reserven() {
    let e = Env::new();
    let s = MAX_SHARES - 1_000;
    assert!(all_ok(&add_at(s, X, Y, 1, 1, 1_000).run(&e)), "genau MAX_SHARES");
    let r = add_at(s, X, Y, 1, 1, 1_001).run(&e);
    assert!(r[0].is_err() && r[2].is_ok(), "MAX_SHARES + 1 scheitert nur am Pool: {r:?}");
    // alles zugleich groß
    let (x, y, s) = (MAX_KAS - 1_000_000_000_000, MAX_KAS - 1_000_000_000_000, MAX_SHARES);
    let dx = 1_000_000_000_000;
    let dy = max_ghost_out(x, y, dx);
    let mut t = at("swap", s, x, y);
    t.x2 = x + dx;
    t.ghost_outs = vec![Tok::pool(y - dy), Tok::to(&random_keypair(), dy)];
    assert!(all_ok(&t.run(&e)), "Tausch an der Obergrenze");
    let mut t2 = at("swap", s, x, y);
    t2.x2 = x + dx;
    t2.ghost_outs = vec![Tok::pool(y - dy - 1), Tok::to(&random_keypair(), dy + 1)];
    assert!(t2.run(&e)[0].is_err(), "+1 Einheit an der Obergrenze");
    let m = s / 2;
    let (qx, qy) = prs::payout_for(s, x, y, m);
    assert!(all_ok(&remove_at(s, x, y, m, m, qx, qy).run(&e)), "Abziehen bei S = MAX_SHARES");
    assert!(remove_at(s, x, y, m, m, qx + 1, qy).run(&e)[0].is_err());
    // S > 1e16 ist ein gültiger Vertragszustand (vgl. A10-P-2)
    assert!(s > MAX_KAS);
}

/// Inflations-/Spendeangriff: Wegen S0 = Start-KAS gesperrter Anteile verliert
/// ein Opfer höchstens den Wert eines Anteils, der Angreifer fast die ganze Spende.
#[test]
fn a10_spendeangriff_lohnt_nicht() {
    let (s0, x0, y0) = (E8, E8, 4_000_000i64); // Pool nach init wie im Mainnet-Plan
    // Angreifer legt minimal ein (1 Anteil) und spendet dann D = 1000 KAS
    let m_a = prs::shares_for_deposit(s0, x0, y0, 1, 1);
    let (s1, x1, y1) = (s0 + m_a.max(1), x0 + 1, y0 + 1);
    let d = 1_000 * E8;
    let x2 = x1 + d;
    // Opfer legt 10 KAS + passende GHOST ein
    let vx = 10 * E8;
    let vy = (vx as i128 * y1 as i128 / x2 as i128) as i64 + 1;
    let m_v = prs::shares_for_deposit(s1, x2, y1, vx, vy);
    let (s3, x3, y3) = (s1 + m_v, x2 + vx, y1 + vy);
    let (got_x, _) = prs::payout_for(s3, x3, y3, m_v);
    let victim_loss = vx - got_x;
    let (att_x, _) = prs::payout_for(s3, x3, y3, m_a.max(1));
    let attacker_loss = d + 1 - att_x;
    println!("Opfer-Verlust {victim_loss} sompi, Angreifer-Verlust {attacker_loss} sompi, Anteile Opfer {m_v}");
    assert!(m_v > 0);
    assert!(victim_loss <= x3 / s3 + 1, "Opfer verliert höchstens ~1 Anteil");
    assert!(attacker_loss > d * 99 / 100, "Spende geht an die gesperrten S0");
}

/// A10-P-4: Ist ein Anteil weniger als 1 sompi wert (Kurs hat sich seit init
/// verschoben), kann der letzte Einleger nicht alles abziehen: pool.rs lehnt
/// ab, der Vertrag erlaubt aber die Auszahlung bis 1 KAS Rest.
#[test]
fn a10_letzter_einleger_und_1_kas_rest() {
    let e = Env::new();
    let (s, x, y) = (1_000 * E8, 286 * E8, 140 * E8); // x/S = 0,286 sompi je Anteil
    let have = s - E8; // alles außer den gesperrten S0
    let (px, py) = prs::payout_for(s, x, y, have);
    assert!(x - px < E8, "voller Anteil ließe weniger als 1 KAS");
    assert!(remove_at(s, x, y, have, have, px, py).run(&e)[0].is_err());
    // gekappt: KAS bis 1 KAS Rest, GHOST voll → vom Vertrag erlaubt
    let capped = x - E8;
    assert!(all_ok(&remove_at(s, x, y, have, have, capped, py).run(&e)), "gekappte Auszahlung ist gültig");
    println!("festsitzend ohne Kappung: {} sompi", px - capped);
}

/// A10-P-7 (Info): Anteils-Token, deren Besitzer die Covenant-ID der Anteile
/// (oder von GHOST) ist, kann jeder in einer Pool-Tx verbrennen und auszahlen –
/// KCC20 prüft bei Covenant-Besitz nur, dass ein Eingang dieser ID dabei ist.
#[test]
fn a10_token_im_besitz_der_anteils_id_ist_fuer_jeden_frei() {
    let e = Env::new();
    let m = S / 10;
    let (dx, dy) = max_payout(S, X, Y, m);
    let mut t = Tx::remove(m, m, dx, dy);
    t.lp_ins = vec![(Tok { owner: LP_COV.as_bytes().to_vec(), typ: ID_COV, amount: m, minter: false }, Some(random_keypair()))];
    let r = t.run(&e);
    assert!(all_ok(&r), "fremder Unterzeichner verbrennt die Anteile und nimmt die Auszahlung: {r:?}");
}

/// Abgleich pool.rs ↔ app/src/lib/poolMath.ts: exportiert Zufallsfälle nach
/// $AUDIT10_EXPORT (JSON); geprüft von audit/a10_poolmath_check.mjs.
#[test]
fn a10_export_faelle_fuer_poolmath_ts() {
    let Ok(path) = std::env::var("AUDIT10_EXPORT") else { return };
    let mut r = StdRng::seed_from_u64(1010);
    let mut rows = vec![];
    for k in 0..500 {
        let big = k % 5 == 0;
        let x = if big { r.gen_range(E8..MAX_KAS) } else { r.gen_range(E8..100_000 * E8) };
        let y = if big { r.gen_range(1..MAX_KAS) } else { r.gen_range(1..10_000 * E8) };
        let s = if big { r.gen_range(1..MAX_SHARES) } else { r.gen_range(E8..100_000 * E8) };
        let dx = r.gen_range(1..x);
        let dy = r.gen_range(1..y.max(2));
        let m = r.gen_range(0..s);
        let g = prs::ghost_out(x, y, dx, FEE);
        let kx = prs::kas_out(x, y, dy, FEE);
        let sh = prs::shares_for_deposit(s, x, y, dx, dy);
        let (px, py) = prs::payout_for(s, x, y, m);
        rows.push(format!(
            "{{\"x\":\"{x}\",\"y\":\"{y}\",\"s\":\"{s}\",\"dx\":\"{dx}\",\"dy\":\"{dy}\",\"m\":\"{m}\",\"ghostOut\":\"{g}\",\"kasOut\":\"{kx}\",\"shares\":\"{sh}\",\"px\":\"{px}\",\"py\":\"{py}\"}}"
        ));
    }
    std::fs::write(&path, format!("[{}]", rows.join(","))).unwrap();
}

/// A10-P-8 (Info, Teilbeleg): Der Vertrag setzt voraus, dass die Inhaber
/// zusammen höchstens S − S0 Anteile halten. Das garantiert nur die Herkunft
/// (Genesis mit initialized = false, init über den Vertrag). Ein Inhaber-Token
/// über mehr als S (aus einer gefälschten Genesis) zieht fast alles ab.
#[test]
fn a10_vertrag_vertraut_der_herkunft_der_anteile() {
    let e = Env::new();
    let s = 2 * E8;
    let have = 1_000_000 * E8; // mehr Anteile als S – in einem echten Pool unmöglich
    let m = s - s / 1000; // 99,9 % (1 KAS muss bleiben)
    let (dx, dy) = max_payout(s, X, Y, m);
    let r = remove_at(s, X, Y, have, m, dx, dy).run(&e);
    assert!(all_ok(&r), "{r:?}");
    println!("Abzug mit {have} Anteilen bei S = {s}: {:.2} KAS und {:.2} GHOST von {:.2} / {:.2}", dx as f64 / 1e8, dy as f64 / 1e8, X as f64 / 1e8, Y as f64 / 1e8);
}


// ---------------------------------------------- Mutanten-Gegenproben ----

fn mutant(line: &str) -> Env {
    let mut e = Env::new();
    assert!(e.src.contains(line), "Zeile nicht gefunden: {line}");
    e.src = e.src.replacen(line, "require(true);", 1);
    e
}

/// init, dessen „Genesis“ in Wahrheit die Fortsetzung eines schon bestehenden
/// KCC20-Tokens T ist (Minter gehört vorher dem Gründer). Liefert die
/// Ergebnisse aller Eingänge.
fn init_with_existing_token(e: &Env) -> Vec<Result<(), String>> {
    let t_id = Hash::from_bytes([0x7a; 32]);
    let x2 = 20 * E8;
    let c = e.creator;
    let pool_in = e.pool(Hash::from_bytes([0; 32]), false);
    let pool_out = e.pool(t_id, true);
    let g_in = Tok::to(&c, 100 * E8);
    let g_outs = vec![Tok::pool(5 * E8), Tok::to(&c, 95 * E8)];
    let t_in = Tok { owner: xonly(&c), typ: ID_PUBKEY, amount: 1, minter: true };
    let t_out = Tok::minter(x2);
    let outputs = vec![
        cov_out(&pool_out, x2, 0, POOL_COV),
        cov_out(&g_outs[0].art(), TOKV, 1, GHOST_COV),
        cov_out(&g_outs[1].art(), TOKV, 1, GHOST_COV),
        cov_out(&t_out.art(), MINV, 2, t_id),
        cov_out(&e.oracle(), ORACLE_V, 3, ORACLE_COV), // Kursband: Orakel (Eingang 3)
        TransactionOutput { value: E8 as u64, script_public_key: plain_spk(), covenant: None },
    ];
    let (kp, ks, _) = &e.tpl;
    let g_states: Vec<ArtifactValue> = g_outs.iter().map(Tok::arg).collect();
    let inputs = vec![
        In {
            utxo: cov_utxo(&pool_in, 10 * E8, POOL_COV),
            call: Call::Entry {
                art: pool_in.clone(),
                entry: "init",
                args: vec![i(3), i(3), ArtifactValue::Bytes(kp.clone()), ArtifactValue::Bytes(ks.clone()), ArtifactValue::Array(g_states.clone())],
                sig_by: Some((5, c)),
            },
            op: op(POOL_TX, 0),
        },
        In { utxo: cov_utxo(&g_in.art(), TOKV, GHOST_COV), call: Call::Leader { art: g_in.art(), new_states: g_states, sig_by: Some(c) }, op: op(0x60, 0) },
        In { utxo: cov_utxo(&t_in.art(), MINV, t_id), call: Call::Leader { art: t_in.art(), new_states: vec![t_out.arg()], sig_by: Some(c) }, op: op(0x61, 0) },
        In { utxo: cov_utxo(&e.oracle(), ORACLE_V, ORACLE_COV), call: Call::Entry { art: e.oracle(), entry: "read", args: vec![], sig_by: None }, op: op(0x71, 0) },
        In { utxo: UtxoEntry::new(1_000 * E8 as u64, plain_spk(), 0, false, None), call: Call::Plain, op: op(0x70, 0) },
    ];
    execute(inputs, outputs)
}

/// Mutationslücke L214 (`OpCovInputCount(id) == 0` in freshGenesis) ist
/// tragend: Ohne sie übernimmt init einen bestehenden Token als Anteils-Token –
/// dessen früher ausgegebene Anteile wären ungedeckt (vgl. A10-P-7).
/// Kein bestehender Test deckt das ab.
#[test]
fn a10_gegenprobe_l214_init_mit_bestehendem_token() {
    let r = init_with_existing_token(&Env::new());
    assert!(r[0].is_err() && r[1].is_ok() && r[2].is_ok(), "Original lehnt ab (nur der Pool): {r:?}");
    let r = init_with_existing_token(&mutant("require(OpCovInputCount(id) == 0);"));
    assert!(all_ok(&r), "Mutante L214 nimmt den bestehenden Token an: {r:?}");
}

/// Mutationslücke L291 (add: x2 ≥ x): Ohne sie Anteile prägen und dabei KAS
/// entnehmen? Wird von productGe mit negativem Faktor abgefangen.
#[test]
fn a10_gegenprobe_l291_add_mit_kas_entnahme() {
    for e in [Env::new(), mutant("require(x2 >= x);")] {
        let mut t = Tx::add(0, 46 * E8, S / 20);
        t.x2 = X - 1_000 * E8;
        let r = t.run(&e);
        assert!(r[0].is_err(), "{r:?}");
    }
}

/// Mutationslücken L192/L193 (Minter pool-eigen / Minter-Flag): Anteils-Token
/// eines Inhabers aus der letzten Pool-Tx als „Minter“ – abgefangen, weil
/// Ausgang 0 Minter und pool-eigen sein muss und ein Nicht-Minter-Leader in
/// KCC20 keinen Minter ausgeben darf.
#[test]
fn a10_gegenprobe_l192_l193_inhaber_als_minter() {
    for e in [Env::new(), mutant("require(s.isMinter);")] {
        let h = random_keypair();
        let mut t = Tx::buy(E8, 0);
        t.ghost_outs = vec![Tok::pool(Y)];
        t.minter = Some((Tok::to(&h, E8), op(POOL_TX, 3)));
        t.lp_outs = vec![Tok::minter(E8)];
        let r = t.run(&e);
        println!("Inhaber als Minter: {r:?}");
        assert!(!all_ok(&r), "{r:?}");
    }
}

// ------------------------------------------- Nachtrag: fehlende Regeltests ----
// Empfehlung aus Abschnitt 5 von audit/10-opus-pool.md: L249/L250 (init) und
// MAX_GHOST (L168, L254) mit eigenem Test, jeweils mit Gegenprobe am Mutanten.

/// L249: init darf keinen GHOST-Minter als Eingang nehmen. KCC20 hebt mit einem
/// Minter-Leader die Mengenerhaltung auf – die Reserve käme aus dem Nichts.
#[test]
fn init_mit_ghost_minter_als_eingang_scheitert() {
    let bau = |e: &Env| {
        let mut t = init_tx(e, 20 * E8, 5 * E8, 20 * E8);
        t.ghost_ins = vec![(Tok { minter: true, ..Tok::to(&e.creator, 100 * E8) }, Some(e.creator))];
        // mehr heraus als hinein: nur mit Minter-Leader möglich
        t.ghost_outs = vec![Tok::pool(500 * E8), Tok::to(&e.creator, 100 * E8)];
        t
    };
    let e = Env::new();
    let r = run_init(&e, bau(&e));
    assert!(r[0].is_err(), "Pool muss ablehnen: {r:?}");
    assert!(r[1].is_ok(), "KCC20 allein lässt es zu – sonst prüft der Test die Regel nicht: {r:?}");
    let m = mutant("require(!t.isMinter);");
    let r = run_init(&m, bau(&m));
    assert!(all_ok(&r), "Gegenprobe: ohne L249 geht es durch: {r:?}");
}

/// L250: init darf keine pool-eigenen GHOST (Geschenke an den Pool) als Eingang nehmen
#[test]
fn init_mit_pool_eigenem_ghost_als_eingang_scheitert() {
    let bau = |e: &Env| {
        let mut t = init_tx(e, 20 * E8, 5 * E8, 20 * E8);
        t.ghost_ins = vec![(Tok::pool(100 * E8), None)];
        t.ghost_outs = vec![Tok::pool(5 * E8), Tok::to(&e.creator, 95 * E8)];
        t
    };
    let e = Env::new();
    let r = run_init(&e, bau(&e));
    assert!(r[0].is_err(), "Pool muss ablehnen: {r:?}");
    assert!(r[1].is_ok(), "KCC20 allein lässt es zu (Besitzer-Covenant ist in der Tx): {r:?}");
    let m = mutant("require(!(t.identifierType == IDENTIFIER_COVENANT_ID && t.ownerIdentifier == me));");
    let r = run_init(&m, bau(&m));
    assert!(all_ok(&r), "Gegenprobe: ohne L250 geht es durch: {r:?}");
}

/// L254: init mit einer Reserve über MAX_GHOST (1e8 GHOST) scheitert, genau an der Grenze geht es
#[test]
fn init_ueber_der_ghost_obergrenze_scheitert() {
    const MAX_GHOST: i64 = 10_000_000_000_000_000;
    let bau = |e: &Env, reserve: i64| {
        let mut t = init_tx(e, 20 * E8, 5 * E8, 20 * E8);
        t.ghost_ins = vec![(Tok::to(&e.creator, reserve), Some(e.creator))];
        t.ghost_outs = vec![Tok::pool(reserve)];
        t
    };
    let e = Env::new();
    let r = run_init(&e, bau(&e, MAX_GHOST + 1));
    assert!(r[0].is_err() && r[1].is_ok(), "{r:?}");
    let r = run_init(&e, bau(&e, MAX_GHOST));
    assert!(all_ok(&r), "genau an der Grenze geht: {r:?}");
    let m = mutant("require(reserve <= MAX_GHOST);");
    let r = run_init(&m, bau(&m, MAX_GHOST + 1));
    assert!(all_ok(&r), "Gegenprobe: ohne L254 geht es durch: {r:?}");
}

/// L168: ein Verkauf, der die Reserve über MAX_GHOST hebt, scheitert
/// (hier als Spende ohne KAS-Entnahme, damit nur die Obergrenze greift)
#[test]
fn verkauf_ueber_die_ghost_obergrenze_scheitert() {
    const MAX_GHOST: i64 = 10_000_000_000_000_000;
    let e = Env::new();
    let r = Tx::sell(MAX_GHOST + 1 - Y, 0).run(&e);
    assert!(r[0].is_err() && r[1].is_ok(), "{r:?}");
    let r = Tx::sell(MAX_GHOST - Y, 0).run(&e);
    assert!(all_ok(&r), "genau an der Grenze geht: {r:?}");
    let m = mutant("require(newReserve <= MAX_GHOST);");
    let r = Tx::sell(MAX_GHOST + 1 - Y, 0).run(&m);
    assert!(all_ok(&r), "Gegenprobe: ohne L168 geht es durch: {r:?}");
}

// ------------------------------------------------------------- Kursband ----
// Pool v2 (29.09.2026): Tauschen nur, solange GHOST danach bei 1 USD ± Band
// liegt oder sich in Richtung Band bewegt. Erwartungen exakt aus pool.rs
// (band_max_in/band_ok), jede Regel mit Gegenprobe am Mutanten.

const BAND: i64 = 300; // ± 3 %

/// Kauf bis genau an die obere Bandgrenze geht, 1 sompi mehr nicht
#[test]
fn kaufen_nur_bis_zur_oberen_bandgrenze() {
    let e = Env::with_band(BAND, KAS_USD_1);
    let dx = prs::band_max_in(X, Y, FEE, KAS_USD_1, BAND, true);
    assert!(dx > 0 && dx < X / 10, "sinnvoller Höchstbetrag: {dx}");
    let r = Tx::buy(dx, max_ghost_out(X, Y, dx)).run(&e);
    assert!(all_ok(&r), "an der Grenze: {r:?}");
    let r = Tx::buy(dx + 1, max_ghost_out(X, Y, dx + 1)).run(&e);
    assert!(r[0].is_err() && r[1].is_ok(), "1 sompi darüber: {r:?}");
    let m = Env { band: BAND, ..mutant("require(falls || productGe(y2, highRef, x2, k));") };
    let r = Tx::buy(dx + 1, max_ghost_out(X, Y, dx + 1)).run(&m);
    assert!(all_ok(&r), "Gegenprobe ohne obere Grenze: {r:?}");
}

/// Verkauf bis genau an die untere Bandgrenze geht, darüber nicht
#[test]
fn verkaufen_nur_bis_zur_unteren_bandgrenze() {
    let e = Env::with_band(BAND, KAS_USD_1);
    let dy = prs::band_max_in(X, Y, FEE, KAS_USD_1, BAND, false);
    assert!(dy > 0 && dy < Y / 10, "sinnvoller Höchstbetrag: {dy}");
    let r = Tx::sell(dy, max_kas_out(X, Y, dy)).run(&e);
    assert!(all_ok(&r), "an der Grenze: {r:?}");
    let r = Tx::sell(dy + 1, max_kas_out(X, Y, dy + 1)).run(&e);
    assert!(r[0].is_err() && r[1].is_ok(), "1 Einheit darüber: {r:?}");
    let m = Env { band: BAND, ..mutant("require(rises || productGe(x2, k, y2, lowRef));") };
    let r = Tx::sell(dy + 1, max_kas_out(X, Y, dy + 1)).run(&m);
    assert!(all_ok(&r), "Gegenprobe ohne untere Grenze: {r:?}");
}

/// Liegt der Kurs außerhalb (KAS-Preis hat sich bewegt), ist nur die Richtung
/// zurück ins Band frei – und auch sie darf nicht auf der anderen Seite hinaus
#[test]
fn ausserhalb_des_bands_nur_richtung_band() {
    // KAS fällt um 10 %: GHOST steht bei 0,90 USD, unter dem Band
    let e = Env::with_band(BAND, KAS_USD_1 * 9 / 10);
    let k = e.kas_usd;
    // weiter verkaufen (Kurs fällt): gesperrt, schon der kleinste Betrag
    assert_eq!(prs::band_max_in(X, Y, FEE, k, BAND, false), 0);
    let r = Tx::sell(E8 / 100, max_kas_out(X, Y, E8 / 100)).run(&e);
    assert!(r[0].is_err(), "Verkauf unter dem Band: {r:?}");
    // kaufen (Kurs steigt) geht, auch ohne das Band zu erreichen …
    let small = 10 * E8;
    assert!(prs::band_ok(X, Y, X + small, Y - max_ghost_out(X, Y, small), k, BAND));
    assert!(all_ok(&Tx::buy(small, max_ghost_out(X, Y, small)).run(&e)), "Richtung Band");
    // … aber nicht über die obere Grenze hinaus
    let dx = prs::band_max_in(X, Y, FEE, k, BAND, true);
    assert!(all_ok(&Tx::buy(dx, max_ghost_out(X, Y, dx)).run(&e)), "bis zur oberen Grenze");
    let r = Tx::buy(dx + 1, max_ghost_out(X, Y, dx + 1)).run(&e);
    assert!(r[0].is_err(), "über das Band hinaus: {r:?}");
}

/// Der Pool liest nur das echte Orakel (Covenant-ID)
#[test]
fn fremdes_orakel_zaehlt_nicht() {
    let e = Env::with_band(BAND, KAS_USD_1);
    let dx = 10 * E8;
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.oracle_cov = Some(Hash::from_bytes([0x0d; 32]));
    let r = t.run(&e);
    assert!(r[0].is_err(), "Orakel mit anderer Covenant-ID: {r:?}");
    let m = Env { band: BAND, ..mutant("require(OpInputCovenantId(oracleIdx) == oracleCovId);") };
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.oracle_cov = Some(Hash::from_bytes([0x0d; 32]));
    assert!(all_ok(&t.run(&m)), "Gegenprobe: ohne Prüfung zählt es");
}

/// init verlangt einen Startkurs im Band
#[test]
fn init_nur_mit_startkurs_im_band() {
    // init_tx: 20 KAS und 5 GHOST ⇒ 4 KAS je GHOST; 1 USD bei 0,25 USD je KAS
    let fair = 25_000_000;
    let ok = Env::with_band(BAND, fair);
    assert!(all_ok(&run_init(&ok, init_tx(&ok, 20 * E8, 5 * E8, 20 * E8))), "Start bei 1 USD");
    let near = Env::with_band(BAND, fair * 1029 / 1000);
    assert!(all_ok(&run_init(&near, init_tx(&near, 20 * E8, 5 * E8, 20 * E8))), "Start bei 1,029 USD");
    for k in [fair * 1031 / 1000, fair * 969 / 1000] {
        let e = Env::with_band(BAND, k);
        let r = run_init(&e, init_tx(&e, 20 * E8, 5 * E8, 20 * E8));
        assert!(r[0].is_err(), "Start außerhalb bei KAS {k}: {r:?}");
    }
}

/// Die Rust-Regel (pool.rs) trifft den Vertrag: zufällige Täusche bei
/// zufälligem KAS-Preis, angenommen genau dann, wenn band_ok sagt
#[test]
fn bandregel_in_pool_rs_gleich_vertrag() {
    let mut rng = StdRng::seed_from_u64(29);
    let mut both = (0, 0);
    for _ in 0..24 {
        let k = KAS_USD_1 * rng.gen_range(900..1100) / 1000;
        let e = Env::with_band(BAND, k);
        let buy = rng.gen_bool(0.5);
        let (x2, y2, t) = if buy {
            let dx = rng.gen_range(E8..X / 20);
            let dy = max_ghost_out(X, Y, dx);
            (X + dx, Y - dy, Tx::buy(dx, dy))
        } else {
            let dy = rng.gen_range(E8 / 10..Y / 20);
            let dx = max_kas_out(X, Y, dy);
            (X - dx, Y + dy, Tx::sell(dy, dx))
        };
        let want = prs::band_ok(X, Y, x2, y2, k, BAND);
        let r = t.run(&e);
        assert_eq!(r[0].is_ok(), want, "k={k} buy={buy} x2={x2} y2={y2}: {r:?}");
        if want { both.0 += 1 } else { both.1 += 1 }
    }
    assert!(both.0 > 3 && both.1 > 3, "beide Fälle getroffen: {both:?}");
}
