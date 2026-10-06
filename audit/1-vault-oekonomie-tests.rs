#![allow(dead_code, unused_imports)]
//! AUDIT-Tests (Kopie der Helfer aus vault_tests.rs). Ganze Transaktionen mit StableVault + RiskOracle + GHOST (KCC20 aus
//! vendor/silverscript, v1.0.0), ausgeführt in der Skript-Engine. Jeder Input
//! der Transaktion wird ausgeführt; ein Angriff gilt als abgewehrt, wenn
//! mindestens ein Input scheitert — und wir prüfen, WELCHER.

mod common;

use common::{bytecode, compiled_template_parts_and_hash, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::tx::{
    CovenantBinding, MutableTransaction, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_txscript::pay_to_script_hash_script;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_covenant_decl_sig_script, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};
use std::collections::BTreeMap;

const VAULT_SRC: &str = include_str!("../../contracts/stable_vault.sil");
const ORACLE_SRC: &str = include_str!("../../contracts/risk_oracle.sil");
const KCC20_SRC: &str = include_str!("../../vendor/silverscript/silverscript-lang/tests/examples/kcc20.sil");

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

fn debt_of(shares: i64, index: i64) -> i64 {
    (shares as u128 * index as u128).div_ceil(1_000_000_000) as i64
}
fn shares_for(m: i64, index: i64, up: bool) -> i64 {
    let n = m as u128 * 1_000_000_000;
    (if up { n.div_ceil(index as u128) } else { n / index as u128 }) as i64
}
fn value_of(coll: i64, price: i64) -> i64 {
    (coll as u128 * price as u128 / E8 as u128) as i64
}
fn healthy(coll: i64, shares: i64, price: i64, index: i64, ratio: i64) -> bool {
    shares == 0 || value_of(coll, price) as u128 >= (debt_of(shares, index) as u128 * ratio as u128).div_ceil(10_000)
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
}

impl Env {
    fn new(oracle: Oracle) -> Self {
        let committee: Vec<Vec<u8>> = (0..5).map(|_| xonly(&random_keypair())).collect();
        let mut e = Self { committee, oracle, oracle_tpl: Default::default(), ghost_tpl: Default::default(), owner: random_keypair() };
        e.oracle_tpl = compiled_template_parts_and_hash(&e.oracle_art(oracle));
        e.ghost_tpl = compiled_template_parts_and_hash(&e.ghost(vec![0; 32], ID_COV, 0, true));
        e
    }

    fn oracle_art(&self, o: Oracle) -> SilAbiArtifact {
        let mut args: Vec<ArtifactValue> = self.committee.iter().cloned().map(ArtifactValue::Bytes).collect();
        args.extend([3i64, 1_000_000_000, o.kas_usd, 1_000_000, 1, 158_548_959, o.index].map(ArtifactValue::Int));
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

    fn vault_with(&self, oracle_cov: Hash, owner: Vec<u8>, shares: i64) -> SilAbiArtifact {
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
                ArtifactValue::Bytes(owner),
                ArtifactValue::Int(shares),
            ],
            CompileOptions::default(),
        )
        .expect("Vault kompiliert")
    }

    fn vault(&self, shares: i64) -> SilAbiArtifact {
        self.vault_with(ORACLE_COV, xonly(&self.owner), shares)
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
    let entries: Vec<UtxoEntry> = inputs.iter().map(|i| i.utxo.clone()).collect();
    let bare: Vec<TransactionInput> =
        (0..inputs.len()).map(|i| TransactionInput::new_with_compute_budget(outpoint(i), vec![], 0, 0)).collect();
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
    let tx = Transaction::new(1, final_inputs, outputs, 0, Default::default(), 0, vec![]);
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
const INDEX: i64 = 1_050_000_000; // 5 % aufgelaufen
const COLL: i64 = 10_000 * E8; // 10 000 KAS = 400 USD

fn env() -> Env {
    Env::new(Oracle { kas_usd: PRICE, index: INDEX })
}

/// Größter Betrag, der bei `coll` und `shares` noch prägbar ist (Referenz).
fn max_mint(coll: i64, shares: i64, o: Oracle) -> i64 {
    let (mut lo, mut hi) = (0i64, value_of(coll, o.kas_usd));
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if healthy(coll, shares + shares_for(mid, o.index, true), o.kas_usd, o.index, MCR) { lo = mid } else { hi = mid - 1 }
    }
    lo
}
fn vault_only(e: &Env, entry: &'static str, args: Vec<ArtifactValue>, sig_pos: Option<usize>, shares: i64, coll_in: i64, coll_out: Option<i64>, with_oracle: bool, sneaky_mint: bool) -> Vec<Result<(), String>> {
    let vault = e.vault(shares);
    let mut inputs = vec![In {
        utxo: utxo(&vault, coll_in, VAULT_COV),
        call: Call::Entry { art: vault.clone(), entry, args, sig_by: sig_pos.map(|p| (p, e.owner)) },
    }];
    let mut outputs = vec![];
    if let Some(c) = coll_out {
        outputs.push(out(&vault, c, 0, VAULT_COV));
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
/// Verbrennt GHOST aus einem Token-Input des Zahlers; optional Wechselgeld.
fn burn_tx(e: &Env, entry: &'static str, shares: i64, new_shares: Option<i64>, coll: i64, coll_out: Option<i64>, pay_in: i64, change: i64, o: Oracle) -> Vec<Result<(), String>> {
    burn_tx_ext(e, entry, shares, new_shares, coll, coll_out, pay_in, change, o, None)
}

fn burn_tx_ext(e: &Env, entry: &'static str, shares: i64, new_shares: Option<i64>, coll: i64, coll_out: Option<i64>, pay_in: i64, change: i64, o: Oracle, rogue: Option<TransactionOutput>) -> Vec<Result<(), String>> {
    let payer = random_keypair();
    let vault = e.vault(shares);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pay = e.ghost(xonly(&payer), ID_PUBKEY, pay_in, false);
    let mut out_toks = vec![Tok::minter()];
    if change > 0 {
        out_toks.push(Tok::to(&payer, change));
    }
    let out_args: Vec<ArtifactValue> = out_toks.iter().map(Tok::arg).collect();
    let oracle_art = e.oracle_art(o);
    let inputs = vec![
        In {
            utxo: utxo(&vault, coll, VAULT_COV),
            call: Call::Entry { art: vault.clone(), entry, args: vec![ArtifactValue::Int(1), ArtifactValue::Array(out_args.clone())], sig_by: None },
        },
        In { utxo: utxo(&oracle_art, E8, ORACLE_COV), call: Call::Entry { art: oracle_art.clone(), entry: "read", args: vec![], sig_by: None } },
        In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: out_args, sig_by: None } },
        In { utxo: utxo(&pay, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pay.clone(), sig_by: payer } },
    ];
    let mut outputs = vec![];
    if let (Some(c), Some(s)) = (coll_out, new_shares) {
        outputs.push(out(&e.vault(s), c, 0, VAULT_COV));
    }
    if let Some(r) = rogue {
        outputs.push(r);
    }
    outputs.push(out(&oracle_art, E8, 1, ORACLE_COV));
    outputs.push(out(&minter, 1_000, 2, GHOST_COV));
    if change > 0 {
        outputs.push(out(&e.ghost(xonly(&payer), ID_PUBKEY, change, false), 1_000, 2, GHOST_COV));
    }
    outputs.push(plain_out(1)); // Liquidator-/Gebühren-Ausgang
    execute(inputs, outputs)
}

// ============================================================ AUDIT-FÄLLE ====

fn seize(coll: i64, debt: i64, price: i64) -> i64 {
    let claim = (debt as u128 * (10_000 + BONUS) as u128).div_ceil(10_000) as i64;
    if value_of(coll, price) > claim { (claim as u128 * E8 as u128).div_ceil(price as u128) as i64 } else { coll }
}

/// A1: Reiner KCC20-Transfer (kein Vault, kein Minter): 10 GHOST -> [+15, -5].
/// KCC20 prüft nur totalIn == totalOut, kein amount >= 0.
#[test]
fn a1_kcc20_transfer_mit_negativem_ausgang() {
    let e = env();
    let attacker = random_keypair();
    let tok_in = e.ghost(xonly(&attacker), ID_PUBKEY, 10 * E8, false);
    let outs = vec![Tok::to(&attacker, 15 * E8), Tok::to(&attacker, -5 * E8)];
    let out_args: Vec<ArtifactValue> = outs.iter().map(Tok::arg).collect();
    let r = execute(
        vec![In { utxo: utxo(&tok_in, 1_000, GHOST_COV), call: Call::GhostLeader { art: tok_in.clone(), new_states: out_args, sig_by: Some(attacker) } }],
        vec![
            out(&e.ghost(xonly(&attacker), ID_PUBKEY, 15 * E8, false), 1_000, 0, GHOST_COV),
            out(&e.ghost(xonly(&attacker), ID_PUBKEY, -5 * E8, false), 1_000, 0, GHOST_COV),
        ],
    );
    println!("A1 KCC20-Transfer 10 -> [+15, -5]: {r:?}");
    // Gegenprobe: normale Aufteilung 10 -> [+7, +3] ist gültig
    let outs2 = vec![Tok::to(&attacker, 7 * E8), Tok::to(&attacker, 3 * E8)];
    let out_args2: Vec<ArtifactValue> = outs2.iter().map(Tok::arg).collect();
    let r2 = execute(
        vec![In { utxo: utxo(&tok_in, 1_000, GHOST_COV), call: Call::GhostLeader { art: tok_in.clone(), new_states: out_args2, sig_by: Some(attacker) } }],
        vec![
            out(&e.ghost(xonly(&attacker), ID_PUBKEY, 7 * E8, false), 1_000, 0, GHOST_COV),
            out(&e.ghost(xonly(&attacker), ID_PUBKEY, 3 * E8, false), 1_000, 0, GHOST_COV),
        ],
    );
    println!("A1 Gegenprobe 10 -> [+7, +3]: {r2:?}");
    assert!(all_ok(&r2), "{r2:?}");
    // Ergebnis wird nur protokolliert; Bewertung im Bericht
    println!("A1 ERGEBNIS: negativer Ausgang {}", if all_ok(&r) { "AKZEPTIERT" } else { "abgelehnt" });
}

/// A1b: Ein negatives Token als Input in repay: der Vault summiert Inputs ohne Vorzeichenprüfung.
#[test]
fn a1b_negatives_token_als_repay_input() {
    let e = env();
    let shares = 100 * E8;
    let payer = random_keypair();
    let vault = e.vault(shares);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let pos = e.ghost(xonly(&payer), ID_PUBKEY, 30 * E8, false);
    let neg = e.ghost(xonly(&payer), ID_PUBKEY, -20 * E8, false);
    // totalIn = 10 GHOST, kein Wechselgeld -> burned = 10
    let burned = 10 * E8;
    let new = shares - shares_for(burned, INDEX, false);
    let out_args = vec![Tok::minter().arg()];
    let oracle_art = e.oracle_art(e.oracle);
    let r = execute(
        vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "repay", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(out_args.clone())], sig_by: None } },
            In { utxo: utxo(&oracle_art, E8, ORACLE_COV), call: Call::Entry { art: oracle_art.clone(), entry: "read", args: vec![], sig_by: None } },
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: out_args, sig_by: None } },
            In { utxo: utxo(&pos, 1_000, GHOST_COV), call: Call::GhostDelegate { art: pos.clone(), sig_by: payer } },
            In { utxo: utxo(&neg, 1_000, GHOST_COV), call: Call::GhostDelegate { art: neg.clone(), sig_by: payer } },
        ],
        vec![out(&e.vault(new), COLL, 0, VAULT_COV), out(&oracle_art, E8, 1, ORACLE_COV), out(&minter, 1_000, 2, GHOST_COV), plain_out(1)],
    );
    println!("A1b repay mit Inputs [+30, -20] als 10 verbucht: {r:?}");
    println!("A1b ERGEBNIS: {}", if all_ok(&r) { "AKZEPTIERT (Vault prüft Vorzeichen der Inputs nicht)" } else { "abgelehnt" });
}

/// A2: Liquidation bei Unterdeckung (coll·p < Schuld) ist nur als Vollverbrennung möglich
/// und für den Liquidator ein Verlustgeschäft. Kein Teilpfad, kein Bad-Debt-Mechanismus.
#[test]
fn a2_liquidation_bei_unterdeckung_ist_verlustgeschaeft() {
    let shares = 150 * E8;
    let debt = debt_of(shares, INDEX); // 157,5 GHOST
    let p = 1_500_000; // 0,015 USD: 10 000 KAS = 150 USD < 157,5
    let o = Oracle { kas_usd: p, index: INDEX };
    let e = Env::new(o);
    assert!(value_of(COLL, p) < debt);
    assert_eq!(seize(COLL, debt, p), COLL);
    let short = burn_tx(&e, "liquidate", shares, None, COLL, None, debt - 1, 0, o);
    assert!(short[0].is_err(), "Teilverbrennung muss scheitern: {short:?}");
    let partial_keep = burn_tx(&e, "liquidate", shares, Some(0), COLL, Some(E8), debt, 0, o);
    assert!(partial_keep[0].is_err(), "Rest im Vault muss scheitern: {partial_keep:?}");
    let full = burn_tx(&e, "liquidate", shares, None, COLL, None, debt, 0, o);
    assert!(all_ok(&full), "{full:?}");
    println!(
        "A2: Liquidator verbrennt {debt} Einheiten = {:.2} USD, erhält {COLL} sompi = {:.2} USD -> Verlust {:.2} USD; Teilverbrennung abgelehnt",
        debt as f64 / 1e8,
        value_of(COLL, p) as f64 / 1e8,
        (debt - value_of(COLL, p)) as f64 / 1e8
    );
}

/// A2b: Zone 100 % < Quote < 110 %: Sicherheit deckt die Schuld, aber nicht Schuld+Bonus.
/// Der Besitzer verliert die gesamte Sicherheit (kein Rest), obwohl er solvent war.
#[test]
fn a2b_zone_zwischen_100_und_110_prozent_nimmt_alles() {
    let shares = 150 * E8;
    let debt = debt_of(shares, INDEX); // 157,5 GHOST
    let p = 1_650_000; // 0,0165 USD: Wert 165 USD, Quote 104,8 %
    let o = Oracle { kas_usd: p, index: INDEX };
    let e = Env::new(o);
    let value = value_of(COLL, p);
    let claim = (debt as u128 * 11_000 / 10_000) as i64;
    assert!(value > debt && value < claim, "value {value} debt {debt} claim {claim}");
    assert_eq!(seize(COLL, debt, p), COLL);
    let keep = burn_tx(&e, "liquidate", shares, Some(0), COLL, Some(DUST), debt, 0, o);
    assert!(keep[0].is_err(), "{keep:?}");
    let full = burn_tx(&e, "liquidate", shares, None, COLL, None, debt, 0, o);
    assert!(all_ok(&full), "{full:?}");
    println!("A2b: Quote {:.1} %: Besitzer verliert {COLL} sompi ({:.2} USD) für {:.2} USD Schuld; Liquidator-Gewinn nur {:.2} USD (Bonus durch Sicherheit gedeckelt)",
        value as f64 / debt as f64 * 100.0, value as f64 / 1e8, debt as f64 / 1e8, (value - debt) as f64 / 1e8);
}

/// A3: Zins ohne Gegenwert. 100 GHOST bei Index 1,0 geprägt; bei Index 1,05 reichen
/// die 100 GHOST nicht zum Schließen. Die fehlenden 5 GHOST existieren nirgends.
#[test]
fn a3_zins_ohne_gegenwert_vault_nicht_schliessbar() {
    let e = env();
    let shares = 100 * E8;
    let debt = debt_of(shares, INDEX);
    let minted = 100 * E8;
    let new = shares - shares_for(minted, INDEX, false);
    assert!(new > 0);
    let r = burn_tx(&e, "repay", shares, Some(new), COLL, Some(COLL), minted, 0, e.oracle);
    assert!(all_ok(&r), "{r:?}");
    let r0 = burn_tx(&e, "repay", shares, Some(0), COLL, Some(COLL), minted, 0, e.oracle);
    assert!(r0[0].is_err(), "auf 0 mit 100 GHOST muss scheitern: {r0:?}");
    let c = vault_only(&e, "close", vec![], Some(0), new, COLL, None, false, false);
    assert!(c[0].is_err(), "close mit Restschuld muss scheitern: {c:?}");
    // Besitzer kann höchstens bis zur Mindestquote abheben: 2 × Restschuld bleibt gebunden
    let mut lo = 0i64;
    let mut hi = COLL;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if healthy(mid, new, PRICE, INDEX, MCR) { hi = mid } else { lo = mid + 1 }
    }
    let w = vault_only(&e, "withdraw", vec![ArtifactValue::Int(lo), ArtifactValue::Int(1)], Some(2), new, COLL, Some(lo), true, false);
    assert!(all_ok(&w), "{w:?}");
    println!(
        "A3: Schuld {debt} Einheiten; nach Verbrennen aller 100 GHOST bleiben {new} Anteile = {} Einheiten; close scheitert; mindestens {lo} sompi ({:.2} KAS) bleiben gebunden",
        debt_of(new, INDEX),
        lo as f64 / 1e8
    );
}

/// A4: Jeder Fremde kann die Vault-UTXO ausgeben: repay mit 1 Einheit (Anteile unverändert,
/// weil floor(1e9/index) = 0) und deposit mit 1 sompi — beide ohne Signatur gültig.
#[test]
fn a4_fremde_tilgung_und_einzahlung_ohne_signatur() {
    let e = env();
    let shares = 100 * E8;
    assert_eq!(shares_for(1, INDEX, false), 0);
    let r = burn_tx(&e, "repay", shares, Some(shares), COLL, Some(COLL), 1, 0, e.oracle);
    assert!(all_ok(&r), "{r:?}");
    let d = vault_only(&e, "deposit", vec![], None, shares, COLL, Some(COLL + 1), false, false);
    assert!(all_ok(&d), "{d:?}");
    // Wie viel GHOST geht beim Teiltilgen verloren? Größter Betrag mit sharesFor == 0:
    let mut lost = 0;
    while shares_for(lost + 1, INDEX, false) == 0 { lost += 1; }
    println!("A4: repay(1 Einheit) durch Fremden gültig, Anteile bleiben {shares}; bis {lost} Einheiten je Teiltilgung verfallen; deposit(+1 sompi) durch Fremden gültig");
}

/// A5: Rest < DUST (0,2 KAS) geht komplett an den Liquidator; eine Fortsetzung mit dem Rest wird abgelehnt.
#[test]
fn a5_dust_rest_geht_an_liquidator() {
    let e = env();
    let shares = 150 * E8;
    let debt = debt_of(shares, INDEX);
    let p = PRICE;
    let claim = (debt as u128 * 11_000).div_ceil(10_000) as i64;
    let sz = (claim as u128 * E8 as u128).div_ceil(p as u128) as i64;
    let coll = sz + DUST / 2;
    assert!(!healthy(coll, shares, p, INDEX, LIQ));
    assert_eq!(seize(coll, debt, p), sz);
    let rest = coll - sz;
    assert!(rest > 0 && rest < DUST);
    let keep = burn_tx(&e, "liquidate", shares, Some(0), coll, Some(rest), debt, 0, e.oracle);
    assert!(keep[0].is_err(), "{keep:?}");
    let all = burn_tx(&e, "liquidate", shares, None, coll, None, debt, 0, e.oracle);
    assert!(all_ok(&all), "{all:?}");
    println!("A5: coll {coll} sompi, seize {sz}, Rest {rest} sompi (< DUST {DUST}) geht an den Liquidator, Vault endet");
}

/// A6: Minter-Zweig ohne Vault-Input ist unbrauchbar (nach close / Totalliquidation dauerhaft).
#[test]
fn a6_minter_zweig_ohne_vault_unbrauchbar() {
    let e = env();
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let states = vec![Tok::minter().arg()];
    let r = execute(
        vec![In { utxo: utxo(&minter, 3 * E8, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: states, sig_by: None } }],
        vec![out(&minter, 3 * E8 - 1_000, 0, GHOST_COV)],
    );
    println!("A6: Minter-Zweig ohne Vault: {r:?}");
    assert!(r[0].is_err(), "{r:?}");
}

/// A7: Mint + sofortige Volltilgung: burned muss >= geprägt sein (kein Leck durch Rundung).
#[test]
fn a7_mint_repay_rundung_kein_leck() {
    let e = env();
    for amount in [1i64, 7, 999, E8, 12_345_678_901] {
        let s = shares_for(amount, INDEX, true);
        let d = debt_of(s, INDEX);
        assert!(d >= amount, "amount {amount}: Schuld {d} < geprägt");
        // Tilgung mit genau `amount` (falls < Schuld) darf Anteile nicht unter das Rechte Maß senken
        if amount < d {
            let left = s - shares_for(amount, INDEX, false);
            assert!(left >= 1, "Rest-Anteile nach Teiltilgung");
        }
    }
    println!("A7: Rundung bei mint/repay stets zugunsten des Protokolls (Referenzrechnung, Engine-Gleichheit in vault_math_tests)");
}

/// A1c: Kette. Tx1: Angreifer hat 1 Einheit GHOST und erzeugt [+debt, -(debt-1)].
/// Tx2: Der +debt-Token (byte-identisches Skript wie Ausgang 0 von Tx1) liquidiert
/// einen unterdeckten Vault und erhält die gesamte Sicherheit.
#[test]
fn a1c_kette_negativ_split_dann_liquidation() {
    let shares = 150 * E8;
    let debt = debt_of(shares, INDEX);
    let p = 1_500_000;
    let o = Oracle { kas_usd: p, index: INDEX };
    let e = Env::new(o);
    let attacker = random_keypair();
    // Tx1
    let seed = e.ghost(xonly(&attacker), ID_PUBKEY, 1, false);
    let outs = vec![Tok::to(&attacker, debt), Tok::to(&attacker, 1 - debt)];
    let out_args: Vec<ArtifactValue> = outs.iter().map(Tok::arg).collect();
    let big = e.ghost(xonly(&attacker), ID_PUBKEY, debt, false);
    let neg = e.ghost(xonly(&attacker), ID_PUBKEY, 1 - debt, false);
    let tx1_outs = vec![out(&big, E8, 0, GHOST_COV), out(&neg, E8, 0, GHOST_COV)];
    let r1 = execute(
        vec![In { utxo: utxo(&seed, 1_000, GHOST_COV), call: Call::GhostLeader { art: seed.clone(), new_states: out_args, sig_by: Some(attacker) } }],
        tx1_outs.clone(),
    );
    assert!(all_ok(&r1), "Tx1: {r1:?}");
    // Tx2: Liquidation mit dem +debt-Token
    let vault = e.vault(shares);
    let minter = e.ghost(VAULT_COV.as_bytes().to_vec(), ID_COV, 0, true);
    let oracle_art = e.oracle_art(o);
    let out_args2 = vec![Tok::minter().arg()];
    let pay_utxo = utxo(&big, E8, GHOST_COV);
    assert_eq!(pay_utxo.script_public_key, tx1_outs[0].script_public_key, "Token aus Tx1 ist genau der Input von Tx2");
    let r2 = execute(
        vec![
            In { utxo: utxo(&vault, COLL, VAULT_COV), call: Call::Entry { art: vault.clone(), entry: "liquidate", args: vec![ArtifactValue::Int(1), ArtifactValue::Array(out_args2.clone())], sig_by: None } },
            In { utxo: utxo(&oracle_art, E8, ORACLE_COV), call: Call::Entry { art: oracle_art.clone(), entry: "read", args: vec![], sig_by: None } },
            In { utxo: utxo(&minter, 1_000, GHOST_COV), call: Call::GhostLeader { art: minter.clone(), new_states: out_args2, sig_by: None } },
            In { utxo: pay_utxo, call: Call::GhostDelegate { art: big.clone(), sig_by: attacker } },
        ],
        vec![out(&oracle_art, E8, 1, ORACLE_COV), out(&minter, 1_000, 2, GHOST_COV), plain_out(COLL)],
    );
    assert!(all_ok(&r2), "Tx2: {r2:?}");
    println!("A1c: aus 1 Einheit GHOST wurden {debt} Einheiten; damit Liquidation gültig, Angreifer erhält {COLL} sompi Sicherheit");
}

// --------------------------------------------------- Rechen-Sonde (wie vault_math_tests) ----

fn probe() -> SilAbiArtifact {
    let start = VAULT_SRC.find("// MATH-BEGIN").unwrap();
    let end = VAULT_SRC.find("// MATH-END").unwrap();
    let math = &VAULT_SRC[start..end];
    let src = format!(
        "pragma silverscript ^0.1.0;
contract MathProbe() {{
    int constant INDEX_SCALE = 1000000000;
{math}
    entry debt(int shares, int index, int expected) {{ require(debtOf(shares, index) == expected); }}
    entry shares(int m, int index, bool roundUp, int expected) {{ require(sharesFor(m, index, roundUp) == expected); }}
}}"
    );
    compile_to_sil_abi_artifact_with_options(&src, &[], CompileOptions::default()).expect("MathProbe")
}

fn probe_run(p: &SilAbiArtifact, entry: &str, args: &[ArtifactValue]) -> Result<(), String> {
    let mut s = encode_contract_entry_sig_script(p, "MathProbe", entry, args).expect("sigscript");
    s.extend_from_slice(&push_redeem_script(&bytecode(p)));
    let input = TransactionInput::new_with_compute_budget(outpoint(0), s, 0, 0);
    let tx = Transaction::new(1, vec![input], vec![plain_out(1)], 0, Default::default(), 0, vec![]);
    let u = UtxoEntry::new(1_000, pay_to_script_hash_script(&bytecode(p)), 0, false, None);
    execute_input_with_covenants(tx, vec![u], 0).map_err(|e| format!("{e:?}"))
}

/// A8: Reale Überlaufgrenze von debtOf bei MAX_DEBT_SHARES: q·index mit q = 1e8.
#[test]
fn a8_debtof_ueberlauf_bei_max_shares() {
    let p = probe();
    let max_shares = 100_000_000_000_000_000i64;
    for idx in [90_000_000_000i64, 92_000_000_000, 93_000_000_000, 1_000_000_000_000, 9_200_000_000_000] {
        let expected = debt_of(max_shares, idx);
        let r = probe_run(&p, "debt", &[ArtifactValue::Int(max_shares), ArtifactValue::Int(idx), ArtifactValue::Int(expected)]);
        println!("A8: debtOf(1e17, {idx}) [Index {}x]: {r:?}", idx / 1_000_000_000);
    }
    // sharesFor: Grenze 9,2e12 laut Kommentar
    for idx in [9_000_000_000_000i64, 9_300_000_000_000] {
        let m = 1_000_000_000_000i64;
        let expected = shares_for(m, idx, true);
        let r = probe_run(&p, "shares", &[ArtifactValue::Int(m), ArtifactValue::Int(idx), ArtifactValue::Bool(true), ArtifactValue::Int(expected)]);
        println!("A8: sharesFor(1e12, {idx}) [Index {}x]: {r:?}", idx / 1_000_000_000);
    }
}
