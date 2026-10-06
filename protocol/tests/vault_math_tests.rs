//! Prüft die Rechenfunktionen aus contracts/stable_vault.sil (Block zwischen
//! MATH-BEGIN und MATH-END, wörtlich herausgelöst) gegen exakte u128-Rechnung
//! in Rust — mit Grenzwerten und Zufallswerten, ausgeführt in der Skript-Engine.

mod common;

use common::{bytecode, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::tx::{ScriptPublicKey, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry};
use kaspa_txscript::opcodes::codes::OpTrue;
use kaspa_txscript::pay_to_script_hash_script;
use rand::{Rng, SeedableRng, rngs::StdRng};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

const VAULT: &str = include_str!("../../contracts/stable_vault.sil");

fn probe() -> SilAbiArtifact {
    let start = VAULT.find("// MATH-BEGIN").expect("MATH-BEGIN");
    let end = VAULT.find("// MATH-END").expect("MATH-END");
    let math = &VAULT[start..end];
    let src = format!(
        "pragma silverscript ^0.1.0;
contract MathProbe() {{
    int constant INDEX_SCALE = 1000000000;
    int constant MAX_GROWTH = 9000000000;
{math}
    entry down(int a, int b, int d, int expected) {{ require(mulDivDown(a, b, d) == expected); }}
    entry up(int a, int b, int d, int expected) {{ require(mulDivUp(a, b, d) == expected); }}
    entry accr(int d, int from, int to, int expected) {{ require(accrual(d, from, to) == expected); }}
}}"
    );
    compile_to_sil_abi_artifact_with_options(&src, &[], CompileOptions::default()).expect("MathProbe kompiliert")
}

fn run(p: &SilAbiArtifact, entry: &str, args: &[ArtifactValue]) -> bool {
    let mut s = encode_contract_entry_sig_script(p, "MathProbe", entry, args).expect("sigscript");
    s.extend_from_slice(&push_redeem_script(&bytecode(p)));
    let input = TransactionInput::new_with_compute_budget(
        TransactionOutpoint { transaction_id: TransactionId::from_bytes([3; 32]), index: 0 },
        s,
        0,
        0,
    );
    let out = TransactionOutput { value: 1, script_public_key: ScriptPublicKey::new(0, vec![OpTrue].into()), covenant: None };
    let tx = Transaction::new(1, vec![input], vec![out], 0, Default::default(), 0, vec![]);
    let utxo = UtxoEntry::new(1_000, pay_to_script_hash_script(&bytecode(p)), 0, false, None);
    execute_input_with_covenants(tx, vec![utxo], 0).is_ok()
}

// ------------------------------------------------ exakte Referenz (u128) ----

fn ref_down(a: i64, b: i64, d: i64) -> i64 {
    (a as u128 * b as u128 / d as u128) as i64
}
fn ref_up(a: i64, b: i64, d: i64) -> i64 {
    (a as u128 * b as u128).div_ceil(d as u128) as i64
}
/// Referenz für accrual (stable_vault.sil Version 3) = math::accrued ohne Altzins
fn ref_accrual(d: i64, from: i64, to: i64) -> i64 {
    kaspa_lending_protocol::math::accrued(&kaspa_lending_protocol::contracts::VaultState { debt: d, interest: 0, index_at: from }, to)
}

fn i(v: i64) -> ArtifactValue {
    ArtifactValue::Int(v)
}

/// Prüft Gleichheit mit der Referenz UND dass ein um 1 falscher Wert abgelehnt
/// wird (sonst würde der Test auch bei einem immer wahren require bestehen).
fn check(p: &SilAbiArtifact, entry: &str, args: Vec<ArtifactValue>, expected: i64) {
    let mut good = args.clone();
    good.push(i(expected));
    assert!(run(p, entry, &good), "{entry}{args:?} sollte {expected} ergeben");
    let mut bad = args.clone();
    bad.push(i(expected + 1));
    assert!(!run(p, entry, &bad), "{entry}{args:?}: falscher Wert {} wurde akzeptiert", expected + 1);
}

// ------------------------------------------------------------------ Tests ----

const MAX_COLLATERAL: i64 = 10_000_000_000_000_000;

#[test]
fn mul_div_grenzfaelle() {
    let p = probe();
    // Sicherheitswert: größte Sicherheit × Preis 920 USD/KAS
    for (a, b, d) in [
        (0, 4_000_000, 100_000_000),
        (1, 1, 100_000_000),
        (MAX_COLLATERAL, 92_000_000_000, 100_000_000),
        (MAX_COLLATERAL - 1, 4_000_000, 100_000_000),
        (123_456_789_012_345, 30_000, 10_000),
        (99_999_999, 92_000_000_000, 100_000_000), // größter Rest × Preisgrenze
    ] {
        check(&p, "down", vec![i(a), i(b), i(d)], ref_down(a, b, d));
        check(&p, "up", vec![i(a), i(b), i(d)], ref_up(a, b, d));
    }
}

/// Oberhalb von d·b > i64::MAX bricht die Engine ab (kein falscher Wert).
/// Für die Sicherheitsbewertung heißt das: Preis ≤ 920 USD/KAS (Test 28.09.2026).
#[test]
fn mul_div_ueber_der_preisgrenze_bricht_ab() {
    let p = probe();
    let (a, b, d) = (99_999_999i64, 99_999_999_999i64, 100_000_000i64);
    assert!((a % d).checked_mul(b).is_none(), "Fall muss wirklich überlaufen");
    assert!(!run(&p, "down", &[i(a), i(b), i(d), i(ref_down(a, b, d))]));
}

const MAX_DEBT: i64 = 100_000_000_000_000_000;

#[test]
fn zins_grenzfaelle() {
    let p = probe();
    for (d, from, to) in [
        (0, 1_000_000_000, 2_000_000_000),                 // ohne Schuld kein Zins
        (1, 1_000_000_000, 1_000_000_000),                 // ohne Zuwachs kein Zins
        (1, 1_000_000_000, 1_000_000_001),                 // aufrunden: 1 Einheit
        (100 * 100_000_000, 1_000_000_000, 1_050_000_000), // +5 %
        (MAX_DEBT, 1_000_000_000, 10_000_000_000),         // größte Schuld, Index ×10
        (MAX_DEBT, 5_000_000_000, 5_000_000_001),
        (123_456_789_123, 1_052_345_678, 1_300_000_001),
        (5, 0, 1_000_000_000),                             // indexAt 0 (neuer Vault): nichts
        (5, 2_000_000_000, 1_000_000_000),                 // Index kleiner: nichts
        (MAX_DEBT, 1_000_000_000, 50_000_000_000),         // über dem Deckel: Index ×10 zählt
        (MAX_DEBT, 9_000_000_000_000, 9_000_000_000_001),  // größter exakter Index
        (MAX_DEBT, 2_000_000_000_000, 7_000_000_000_000),
    ] {
        check(&p, "accr", vec![i(d), i(from), i(to)], ref_accrual(d, from, to));
    }
}

#[test]
fn zufallswerte_gegen_u128() {
    let p = probe();
    let mut rng = StdRng::seed_from_u64(20260928);
    for _ in 0..150 {
        let coll = rng.gen_range(0..=MAX_COLLATERAL);
        let price = rng.gen_range(1..=92_000_000_000i64);
        check(&p, "down", vec![i(coll), i(price), i(100_000_000)], ref_down(coll, price, 100_000_000));

        let debt = rng.gen_range(0..=1_000_000_000_000_000_000i64);
        let bps = rng.gen_range(10_000..=30_000i64);
        check(&p, "up", vec![i(debt), i(bps), i(10_000)], ref_up(debt, bps, 10_000));

        let d = rng.gen_range(0..=MAX_DEBT);
        let from = rng.gen_range(1_000_000_000..=10_000_000_000i64);
        let to = from + rng.gen_range(0..=from * 12); // auch über den Deckel (×10)
        check(&p, "accr", vec![i(d), i(from), i(to)], ref_accrual(d, from, to));
    }
}
