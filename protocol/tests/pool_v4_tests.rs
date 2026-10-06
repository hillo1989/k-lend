//! POOL V4: Kopie von pool_tests.rs für contracts/ghost_pool_v4.sil mit dem Orakel v4
//! (price_oracle_v4.sil). Alle v3-Tests laufen unverändert gegen v4; die
//! v4-Tests (eingefrorenes Orakel, stopWhenFrozen) stehen am Ende.
//!
//! GhostPool (contracts/ghost_pool.sil, offen mit Anteils-Token): Rechen-
//! funktionen gegen exakte u128-Rechnung und ganze Transaktionen (init, swap,
//! add, remove) mit GHOST und Anteilen (beide KCC20 v2) in der Skript-Engine.
//! Ein Angriff gilt als abgewehrt, wenn der Pool-Input scheitert.

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

// ------------------------------------------------------------ Mathe-Teil ----

fn probe() -> SilAbiArtifact {
    let start = POOL_SRC.find("// MATH-BEGIN").expect("MATH-BEGIN");
    let end = POOL_SRC.find("// MATH-END").expect("MATH-END");
    let math = &POOL_SRC[start..end];
    let src = format!(
        "pragma silverscript ^0.1.0;
contract MathProbe() {{
    int constant LIMB = 2147483648;
    int constant HALF = 4611686018427387904;
{math}
    entry up(int a, int b, int d, int expected) {{ require(mulDivUp(a, b, d) == expected); }}
    entry prod(int a, int b, int hi, int lo) {{ (int h, int l) = mul(a, b); require(h == hi); require(l == lo); }}
    entry ge(int a, int b, int c, int d, bool expected) {{ require(productGe(a, b, c, d) == expected); }}
}}"
    );
    compile_to_sil_abi_artifact_with_options(&src, &[], CompileOptions::default()).expect("MathProbe kompiliert")
}

fn run_probe(p: &SilAbiArtifact, entry: &str, args: &[ArtifactValue]) -> bool {
    let mut s = encode_contract_entry_sig_script(p, "MathProbe", entry, args).expect("sigscript");
    s.extend_from_slice(&push_redeem_script(&bytecode(p)));
    let input = TransactionInput::new_with_compute_budget(TransactionOutpoint { transaction_id: TransactionId::from_bytes([3; 32]), index: 0 }, s, 0, 0);
    let out = TransactionOutput { value: 1, script_public_key: ScriptPublicKey::new(0, vec![OpTrue].into()), covenant: None };
    let tx = Transaction::new(1, vec![input], vec![out], 0, Default::default(), 0, vec![]);
    let utxo = UtxoEntry::new(1_000, pay_to_script_hash_script(&bytecode(p)), 0, false, None);
    execute_input_with_covenants(tx, vec![utxo], 0).is_ok()
}

fn i(v: i64) -> ArtifactValue {
    ArtifactValue::Int(v)
}

const LIMIT: i64 = (1 << 60) - 1;

fn check_prod(p: &SilAbiArtifact, a: i64, b: i64) {
    let n = a as u128 * b as u128;
    let (hi, lo) = ((n >> 62) as i64, (n & ((1u128 << 62) - 1)) as i64);
    assert!(run_probe(p, "prod", &[i(a), i(b), i(hi), i(lo)]), "mul({a},{b}) sollte ({hi},{lo}) sein");
    assert!(!run_probe(p, "prod", &[i(a), i(b), i(hi), i(lo + 1)]), "mul({a},{b}): falsches lo akzeptiert");
    assert!(!run_probe(p, "prod", &[i(a), i(b), i(hi + 1), i(lo)]), "mul({a},{b}): falsches hi akzeptiert");
}

fn check_ge(p: &SilAbiArtifact, a: i64, b: i64, c: i64, d: i64) {
    let e = a as u128 * b as u128 >= c as u128 * d as u128;
    assert!(run_probe(p, "ge", &[i(a), i(b), i(c), i(d), ArtifactValue::Bool(e)]), "productGe({a},{b},{c},{d}) sollte {e} sein");
    assert!(!run_probe(p, "ge", &[i(a), i(b), i(c), i(d), ArtifactValue::Bool(!e)]), "productGe({a},{b},{c},{d}): Gegenteil akzeptiert");
}

#[test]
fn mul_ist_exakt_an_den_grenzen_und_zufaellig() {
    let p = probe();
    for (a, b) in [(0, 0), (1, 1), (LIMIT, LIMIT), (LIMIT, 1), ((1 << 31) - 1, (1 << 31) - 1), (1 << 31, 1 << 31), (1 << 59, 3), (10_000_000_000_000_000, 10_000_000_000_000_000)]
    {
        check_prod(&p, a, b);
    }
    let mut r = StdRng::seed_from_u64(7);
    for _ in 0..40 {
        check_prod(&p, r.gen_range(0..=LIMIT), r.gen_range(0..=LIMIT));
    }
}

#[test]
fn product_ge_ist_exakt_auch_bei_gleichem_hi() {
    let p = probe();
    // gleiche Produkte, knapp darüber und darunter
    check_ge(&p, LIMIT, LIMIT, LIMIT, LIMIT);
    check_ge(&p, LIMIT, LIMIT - 1, LIMIT, LIMIT);
    check_ge(&p, 6, 4, 3, 8);
    check_ge(&p, 1 << 40, 1 << 30, (1 << 40) + 1, (1 << 30) - 1);
    let mut r = StdRng::seed_from_u64(11);
    for _ in 0..30 {
        let (a, b) = (r.gen_range(1..=LIMIT), r.gen_range(1..=LIMIT));
        // Nachbarprodukt mit gleichem hi-Anteil: nur lo entscheidet
        check_ge(&p, a, b, a, b.saturating_sub(1).max(1));
        check_ge(&p, a, b, r.gen_range(1..=LIMIT), r.gen_range(1..=LIMIT));
    }
}

#[test]
fn mul_div_up_rundet_auf() {
    let p = probe();
    for (a, b, d) in [(1, 30, 10_000), (10_000, 30, 10_000), (10_001, 30, 10_000), (10_000_000_000_000_000, 30, 10_000), (0, 30, 10_000)] {
        let e = (a as u128 * b as u128).div_ceil(d as u128) as i64;
        assert!(run_probe(&p, "up", &[i(a), i(b), i(d), i(e)]));
        assert!(!run_probe(&p, "up", &[i(a), i(b), i(d), i(e + 1)]));
    }
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
        Self { creator: random_keypair(), tpl: compiled_template_parts_and_hash(&kcc20(&[0; 32], ID_COV, 0, true)), kas_usd: KAS_USD_1, band: WIDE_BAND, frozen: std::cell::Cell::new(false), stop_when_frozen: true }
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
            POOL_SRC,
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
            outputs.push(cov_out(&oracle_art, ORACLE_V, oracle_idx as u16, ORACLE_COV));
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
            inputs.push(In { utxo: cov_utxo(&art, ORACLE_V, ORACLE_COV), call: Call::Entry { art, entry: "read", args: vec![], sig_by: None }, op: op(0x71, 0) });
        }
        inputs.push(In { utxo: UtxoEntry::new(1_000 * E8 as u64, plain_spk(), 0, false, None), call: Call::Plain, op: op(0x70, 0) });
        execute(inputs, outputs)
    }
}

// ------------------------------------------------------------- Tauschen ----

#[test]
fn kaufen_bis_zum_maximum_geht_durch() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    assert!(dy > 454 * E8 / 100 && dy < 4_555 * E8 / 1000, "dy = {dy}");
    let r = Tx::buy(dx, dy).run(&e);
    assert!(all_ok(&r), "{r:?}");
    let r = Tx::buy(dx, dy + 1).run(&e);
    assert!(r[0].is_err() && r[1].is_ok() && r[2].is_ok(), "eine Einheit zu viel scheitert nur am Pool: {r:?}");
}

#[test]
fn verkaufen_bis_zum_maximum_geht_durch() {
    let e = Env::new();
    let dy = 5 * E8;
    let dx = max_kas_out(X, Y, dy);
    assert!(all_ok(&Tx::sell(dy, dx).run(&e)));
    let r = Tx::sell(dy, dx + 1).run(&e);
    assert!(r[0].is_err() && r[1].is_ok(), "{r:?}");
}

#[test]
fn ohne_gebuehr_zu_tauschen_scheitert() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = (Y as u128 * dx as u128 / (X + dx) as u128) as i64;
    assert!(dy > max_ghost_out(X, Y, dx));
    assert!(Tx::buy(dx, dy).run(&e)[0].is_err());
}

#[test]
fn spende_geht_ghost_nehmen_ohne_kas_nicht() {
    let e = Env::new();
    let mut t = Tx::buy(E8, 0);
    t.ghost_outs = vec![Tok::pool(Y)];
    assert!(all_ok(&t.run(&e)), "Spende");
    assert!(Tx::buy(0, 1).run(&e)[0].is_err(), "GHOST ohne KAS");
}

#[test]
fn kas_der_reserve_und_des_minters_bleiben() {
    let e = Env::new();
    let dx = 100 * E8;
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.reserve_out_value = Some(TOKV / 10);
    assert!(t.run(&e)[0].is_err(), "Reserve-UTXO");
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.minter_out_value = MINV / 10;
    assert!(t.run(&e)[0].is_err(), "Minter-UTXO");
}

#[test]
fn geschenkter_token_als_reserve_scheitert() {
    // 1-Einheiten-Token des Pools aus fremder Tx als Reserve → y = 1, der Pool wäre leer
    let e = Env::new();
    let attacker = random_keypair();
    let mut t = Tx::base("swap", X, 10 * E8);
    t.reserve = Some((Tok::pool(1), op(0x31, 0), None));
    t.ghost_ins = vec![(Tok::to(&attacker, 1_000_000 - 1), Some(attacker))];
    t.ghost_outs = vec![Tok::pool(1_000_000)];
    assert!(swap_ok(X, 1, 10 * E8, 1_000_000), "ohne Herkunftsprüfung ginge die Rechnung auf");
    let r = t.run(&e);
    assert!(r[0].is_err() && r[1].is_ok() && r[3].is_ok(), "nur der Pool darf ablehnen: {r:?}");
}

#[test]
fn fremder_token_aus_derselben_tx_als_reserve_scheitert() {
    let e = Env::new();
    let trader = random_keypair();
    let mut t = Tx::base("swap", X, 10 * E8);
    t.reserve = Some((Tok::to(&trader, 1), op(POOL_TX, 3), Some(trader)));
    t.ghost_ins = vec![(Tok::to(&trader, 1_000_000 - 1), Some(trader))];
    t.ghost_outs = vec![Tok::pool(1_000_000)];
    let r = t.run(&e);
    assert!(r[0].is_err() && r[1].is_ok(), "{r:?}");
}

#[test]
fn minter_als_reserve_scheitert() {
    let e = Env::new();
    let mut t = Tx::buy(E8, 0);
    t.reserve = Some((Tok { minter: true, ..Tok::pool(Y) }, op(POOL_TX, 1), None));
    t.ghost_outs = vec![Tok::pool(Y), Tok::to(&random_keypair(), 1_000 * E8)]; // aus dem Nichts
    let r = t.run(&e);
    assert!(r[0].is_err() && r[1].is_ok(), "KCC20 lässt den Minter prägen, der Pool muss ablehnen: {r:?}");
}

#[test]
fn zweiter_pool_token_rein_oder_raus_scheitert() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    let mut t = Tx::buy(dx, dy);
    t.ghost_ins = vec![(Tok::pool(E8), None)];
    t.ghost_outs[1].amount += E8;
    assert!(t.run(&e)[0].is_err(), "Geschenk als zweiter Eingang");
    let mut t = Tx::buy(dx, dy);
    t.ghost_outs[1] = Tok::pool(dy);
    assert!(t.run(&e)[0].is_err(), "zweiter Pool-Token als Ausgang");
    let mut t = Tx::buy(dx, dy);
    t.ghost_outs[0] = Tok::to(&random_keypair(), Y - dy);
    assert!(t.run(&e)[0].is_err(), "Reserve nicht an den Pool");
}

#[test]
fn falsch_genannte_ausgaenge_scheitern() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    let mut t = Tx::buy(dx, dy * 2);
    t.ghost_claimed = Some(vec![Tok::pool(Y - dy), t.ghost_outs[1].clone()]);
    assert!(t.run(&e)[0].is_err(), "GHOST");
    let mut t = Tx::buy(dx, dy);
    t.lp_claimed = Some(vec![Tok::minter(S + 1)]);
    assert!(t.run(&e)[0].is_err(), "Anteile");
}

#[test]
fn tauschen_laesst_die_anteile_unveraendert() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    // Anteile mitprägen
    let mut t = Tx::buy(dx, dy);
    t.lp_outs = vec![Tok::minter(S + E8), Tok::to(&random_keypair(), E8)];
    let r = t.run(&e);
    assert!(r[0].is_err() && r[2].is_ok(), "Minter-Leader prägt, der Pool muss ablehnen: {r:?}");
    // Anteile eines Inhabers mitverbrauchen
    let mut t = Tx::buy(dx, dy);
    let h = random_keypair();
    t.lp_ins = vec![(Tok::to(&h, E8), Some(h))];
    t.lp_outs = vec![Tok::minter(S), Tok::to(&h, E8)];
    assert!(t.run(&e)[0].is_err(), "Inhaber-Anteile im Tausch");
}

#[test]
fn minter_aus_fremder_tx_oder_fehlend_scheitert() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    let mut t = Tx::buy(dx, dy);
    t.minter = Some((Tok::minter(S), op(0x33, 0)));
    assert!(t.run(&e)[0].is_err(), "Minter aus fremder Tx");
    let mut t = Tx::buy(dx, dy);
    t.minter = None;
    t.lp_outs = vec![];
    assert!(t.run(&e)[0].is_err(), "ohne Minter");
}

#[test]
fn falscher_zustand_der_fortsetzung_scheitert() {
    let e = Env::new();
    let dx = 100 * E8;
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.next_lp = Hash::from_bytes([0x44; 32]);
    assert!(t.run(&e)[0].is_err(), "andere Anteils-ID");
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.pool_outputs = 2;
    assert!(t.run(&e)[0].is_err(), "zwei Fortsetzungen");
}

#[test]
fn pool_behaelt_1_kas_und_etwas_ghost() {
    let e = Env::new();
    let x = E8;
    let mut t = Tx::sell(100 * E8, max_kas_out(x, Y, 100 * E8));
    t.x = x;
    t.x2 = x - max_kas_out(x, Y, 100 * E8);
    assert!(t.x2 < E8);
    assert!(t.run(&e)[0].is_err(), "unter 1 KAS");
    assert!(Tx::buy(1_000_000 * E8, Y).run(&e)[0].is_err(), "Reserve 0");
}

#[test]
fn nicht_initialisiert_kein_tausch() {
    let e = Env::new();
    let dx = 100 * E8;
    let mut t = Tx::buy(dx, max_ghost_out(X, Y, dx));
    t.in_init = false;
    assert!(t.run(&e)[0].is_err());
}

#[test]
fn zufaellige_taeusche_stimmen_mit_der_referenz() {
    let e = Env::new();
    let mut r = StdRng::seed_from_u64(5);
    for _ in 0..4 {
        if r.gen_bool(0.5) {
            let dx = r.gen_range(1..X);
            let dy = max_ghost_out(X, Y, dx);
            if dy == 0 {
                continue;
            }
            assert!(all_ok(&Tx::buy(dx, dy).run(&e)), "buy dx={dx}");
            assert!(Tx::buy(dx, dy + 1).run(&e)[0].is_err(), "buy +1 dx={dx}");
        } else {
            let dy = r.gen_range(1..Y);
            let dx = max_kas_out(X, Y, dy);
            if X - dx < E8 || dx == 0 {
                continue;
            }
            assert!(all_ok(&Tx::sell(dy, dx).run(&e)), "sell dy={dy}");
            assert!(Tx::sell(dy, dx + 1).run(&e)[0].is_err(), "sell +1 dy={dy}");
        }
    }
}

// ------------------------------------------------------------- Einlegen ----

#[test]
fn einlegen_bis_zur_anteilsgrenze() {
    let e = Env::new();
    // 1 000 KAS und 46 GHOST = 10 % beider Reserven → 10 % der Anteile
    let (dx, dy) = (1_000 * E8, 46 * E8);
    let m = max_shares(S, X, Y, dx, dy);
    assert_eq!(m, S / 10);
    assert!(all_ok(&Tx::add(dx, dy, m).run(&e)));
    let r = Tx::add(dx, dy, m + 1).run(&e);
    assert!(r[0].is_err() && r[2].is_ok(), "ein Anteil zu viel scheitert nur am Pool: {r:?}");
    // Ungleiche Einlage: die kleinere Seite zählt
    let m2 = max_shares(S, X, Y, dx, dy / 2);
    assert_eq!(m2, S / 20);
    assert!(all_ok(&Tx::add(dx, dy / 2, m2).run(&e)));
    assert!(Tx::add(dx, dy / 2, m2 + 1).run(&e)[0].is_err());
}

#[test]
fn einlegen_ueber_der_kas_obergrenze_scheitert() {
    // MAX_KAS = 1e8 KAS; Einlage so gewählt, dass nur die Obergrenze greift
    let e = Env::new();
    let max: i64 = 10_000_000_000_000_000;
    let dx = max - X + 1;
    let dy = (Y as i128 * dx as i128 / X as i128) as i64 + 1;
    let m = max_shares(S, X, Y, dx, dy);
    assert!(m > 0);
    let r = Tx::add(dx, dy, m).run(&e);
    assert!(r[0].is_err() && r[2].is_ok(), "{r:?}");
    let r = Tx::add(dx - 1, dy, max_shares(S, X, Y, dx - 1, dy)).run(&e);
    assert!(all_ok(&r), "genau an der Grenze geht: {r:?}");
}

#[test]
fn einlegen_nur_einer_seite_bringt_keine_anteile() {
    let e = Env::new();
    let mut t = Tx::add(1_000 * E8, 1, 1);
    t.ghost_ins = vec![];
    t.ghost_outs = vec![Tok::pool(Y)];
    assert!(t.run(&e)[0].is_err(), "nur KAS");
    let mut t = Tx::add(0, 46 * E8, 1);
    t.x2 = X;
    assert!(t.run(&e)[0].is_err(), "nur GHOST");
}

#[test]
fn einlegen_mengenerhaltung_der_anteile() {
    let e = Env::new();
    let (dx, dy) = (1_000 * E8, 46 * E8);
    let m = max_shares(S, X, Y, dx, dy);
    // Minter zählt m, der Einleger bekommt mehr
    let mut t = Tx::add(dx, dy, m);
    t.lp_outs[1].amount = m + 1;
    let r = t.run(&e);
    assert!(r[0].is_err() && r[2].is_ok(), "KCC20 prüft beim Minter-Leader nichts, der Pool schon: {r:?}");
    // zweiter Minter als Ausgang
    let mut t = Tx::add(dx, dy, m);
    t.lp_outs[1].minter = true;
    assert!(t.run(&e)[0].is_err(), "zweiter Minter");
    // Anteile an den Pool selbst
    let mut t = Tx::add(dx, dy, m);
    t.lp_outs[1] = Tok { minter: false, ..Tok::minter(m) };
    assert!(t.run(&e)[0].is_err(), "Anteile an den Pool");
}

#[test]
fn einlegen_ohne_fremde_anteile() {
    let e = Env::new();
    let (dx, dy) = (1_000 * E8, 46 * E8);
    let m = max_shares(S, X, Y, dx, dy);
    let h = random_keypair();
    let mut t = Tx::add(dx, dy, m);
    t.lp_ins = vec![(Tok::to(&h, E8), Some(h))];
    t.lp_outs = vec![Tok::minter(S + m), Tok::to(&h, m + E8)];
    assert!(t.run(&e)[0].is_err());
}

// ------------------------------------------------------------- Abziehen ----

#[test]
fn abziehen_bis_zum_anteil() {
    let e = Env::new();
    let m = S / 10;
    let (dx, dy) = max_payout(S, X, Y, m);
    assert_eq!((dx, dy), (X / 10, Y / 10));
    assert!(all_ok(&Tx::remove(m, m, dx, dy).run(&e)));
    let r = Tx::remove(m, m, dx + 1, dy).run(&e);
    assert!(r[0].is_err() && r[2].is_ok(), "1 sompi zu viel: {r:?}");
    assert!(Tx::remove(m, m, dx, dy + 1).run(&e)[0].is_err(), "1 GHOST-Einheit zu viel");
    // Teil der eigenen Anteile, Rest bleibt beim Inhaber
    let (dx2, dy2) = max_payout(S, X, Y, m / 2);
    assert!(all_ok(&Tx::remove(m, m / 2, dx2, dy2).run(&e)));
}

#[test]
fn abziehen_braucht_die_signatur_des_inhabers() {
    let e = Env::new();
    let m = S / 10;
    let (dx, dy) = max_payout(S, X, Y, m);
    let mut t = Tx::remove(m, m, dx, dy);
    let (tok, _) = t.lp_ins[0].clone();
    t.lp_ins = vec![(tok, None)]; // fremder Schlüssel
    let r = t.run(&e);
    assert!(!all_ok(&r), "{r:?}");
}

#[test]
fn abziehen_mengenerhaltung_der_anteile() {
    let e = Env::new();
    let m = S / 10;
    let (dx, dy) = max_payout(S, X, Y, m);
    // Minter sinkt um m, der Inhaber behält aber etwas
    let mut t = Tx::remove(m, m, dx, dy);
    t.lp_outs = vec![Tok::minter(S - m), t.lp_ins[0].0.clone()];
    t.lp_outs[1].amount = 1;
    assert!(t.run(&e)[0].is_err());
    // Anteile verbrennen, ohne dass S sinkt → keine Auszahlung erlaubt
    let mut t = Tx::remove(m, m, dx, dy);
    t.lp_outs = vec![Tok::minter(S)];
    assert!(t.run(&e)[0].is_err());
}

#[test]
fn abziehen_haelt_1_kas() {
    let e = Env::new();
    // fast alle Anteile (die gesperrten bleiben), aber der Pool ist klein
    let m = S - S / 1000;
    let mut t = Tx::remove(m, m, 0, 0);
    t.x = 2 * E8;
    let (dx, dy) = max_payout(S, 2 * E8, Y, m);
    t.x2 = 2 * E8 - dx;
    t.ghost_outs = vec![Tok::pool(Y - dy), Tok::to(&random_keypair(), dy)];
    assert!(t.x2 < E8);
    assert!(t.run(&e)[0].is_err());
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

#[test]
fn init_legt_anteile_und_reserve_an() {
    let e = Env::new();
    let r = run_init(&e, init_tx(&e, 20 * E8, 5 * E8, 20 * E8));
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn init_nur_einmal_und_nur_vom_gruender() {
    let e = Env::new();
    let mut t = init_tx(&e, 20 * E8, 5 * E8, 20 * E8);
    t.signer = Some(random_keypair());
    assert!(run_init(&e, t)[0].is_err(), "fremde Signatur");
    let mut t = init_tx(&e, 20 * E8, 5 * E8, 20 * E8);
    t.in_init = true;
    assert!(run_init(&e, t)[0].is_err(), "schon initialisiert");
}

#[test]
fn init_prueft_anteile_und_reserve() {
    let e = Env::new();
    assert!(run_init(&e, init_tx(&e, 20 * E8, 5 * E8, 20 * E8 + 1))[0].is_err(), "S0 ≠ Start-KAS");
    let mut t = init_tx(&e, 20 * E8, 5 * E8, 20 * E8);
    t.lp_genesis = Some(Tok { minter: false, ..Tok::minter(20 * E8) });
    assert!(run_init(&e, t)[0].is_err(), "Anteils-Token kein Minter");
    let mut t = init_tx(&e, 20 * E8, 5 * E8, 20 * E8);
    t.lp_genesis_twice = true;
    assert!(run_init(&e, t)[0].is_err(), "zwei Ausgänge mit der neuen ID");
    let mut t = init_tx(&e, 20 * E8, 5 * E8, 20 * E8);
    t.next_lp = Hash::from_bytes([0x44; 32]);
    assert!(run_init(&e, t)[0].is_err(), "Fortsetzung merkt sich eine falsche ID");
    let mut t = init_tx(&e, E8 / 2, 5 * E8, E8 / 2);
    t.x2 = E8 / 2;
    assert!(run_init(&e, t)[0].is_err(), "unter 1 KAS");
    let mut t = init_tx(&e, 20 * E8, 5 * E8, 20 * E8);
    t.ghost_outs = vec![Tok::to(&e.creator, 100 * E8)];
    assert!(run_init(&e, t)[0].is_err(), "keine Reserve an den Pool");
}

#[test]
fn minter_status_der_pool_ausgaenge_ist_fest() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    // Anteils-Minter verliert seinen Status: KCC20 erlaubt das, der Pool nicht,
    // sonst könnte nie wieder jemand Anteile bekommen.
    let mut t = Tx::buy(dx, dy);
    t.lp_outs = vec![Tok { minter: false, ..Tok::minter(S) }];
    let r = t.run(&e);
    assert!(r[0].is_err(), "Minter ohne Minter-Status: {r:?}");
    eprintln!("KCC20 (Anteile, Status fallen lassen): {:?}", r[2]);
    // GHOST-Reserve als Minter ausgeben
    let mut t = Tx::buy(dx, dy);
    t.ghost_outs[0].minter = true;
    let r = t.run(&e);
    assert!(r[0].is_err(), "Reserve als Minter: {r:?}");
    eprintln!("KCC20 (Reserve wird Minter): {:?}", r[1]);
}

// ============================================================================
// Version 4: eingefrorenes Orakel
// ============================================================================

#[test]
fn v4_eingefroren_kein_tausch_wenn_stop_when_frozen() {
    let e = Env::new();
    let dx = 100 * E8;
    let dy = max_ghost_out(X, Y, dx);
    assert!(all_ok(&Tx::buy(dx, dy).run(&e)), "Gegenprobe frisch");
    e.frozen.set(true);
    let r = Tx::buy(dx, dy).run(&e);
    assert!(r[0].is_err() && r[1].is_ok() && r[2].is_ok(), "nur der Pool lehnt ab: {r:?}");
    let sell = Tx::sell(E8, max_kas_out(X, Y, E8)).run(&e);
    assert!(sell[0].is_err(), "auch verkaufen: {sell:?}");
}

#[test]
fn v4_eingefroren_tausch_frei_wenn_eingestellt() {
    let mut e = Env::new();
    e.stop_when_frozen = false;
    e.frozen.set(true);
    let dx = 100 * E8;
    assert!(all_ok(&Tx::buy(dx, max_ghost_out(X, Y, dx)).run(&e)));
}

#[test]
fn v4_eingefroren_einlegen_und_abziehen_bleiben() {
    // add/remove lesen das Orakel nicht
    let e = Env::new();
    e.frozen.set(true);
    let (dx, dy) = (100 * E8, 46 * E8 / 10);
    let m = max_shares(S, X, Y, dx, dy);
    assert!(all_ok(&Tx::add(dx, dy, m).run(&e)));
    let (px, py) = max_payout(S, X, Y, 10 * E8);
    assert!(all_ok(&Tx::remove(10 * E8, 10 * E8, px, py).run(&e)));
}

#[test]
fn v4_eingefroren_kein_init() {
    let e = Env::new();
    assert!(all_ok(&run_init(&e, init_tx(&e, 20 * E8, 5 * E8, 20 * E8))));
    e.frozen.set(true);
    let r = run_init(&e, init_tx(&e, 20 * E8, 5 * E8, 20 * E8));
    assert!(r[0].is_err(), "{r:?}");
}
