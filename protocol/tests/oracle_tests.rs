//! Tests für contracts/risk_oracle.sil gegen die echte Kaspa-Skript-Engine
//! (rusty-kaspa a41a333). Jeder Angriffstest prüft, dass GENAU die gemeinte
//! Regel greift: Die Gut-Variante derselben Transaktion muss durchgehen.

mod common;

use common::{COV_A, COV_B, bytecode, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{CovenantBinding, Transaction, TransactionInput, TransactionOutpoint, TransactionId, TransactionOutput, UtxoEntry};
use kaspa_txscript::pay_to_script_hash_script;
use kaspa_txscript_errors::TxScriptError;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

const SOURCE: &str = include_str!("../../contracts/risk_oracle.sil");
const RATE_SCALE: i64 = 1_000_000_000;
const UTXO_VALUE: u64 = 100_000_000; // 1 KAS im Orakel
const MAX_RATE: i64 = 1_000_000_000; // ≈ 31,5 %/Jahr bei 10 BPS

#[derive(Clone, Copy, Debug, PartialEq)]
struct OracleState {
    kas_usd: i64,
    oracle_daa: i64,
    seq: i64,
    stable_rate: i64,
    stable_index: i64,
}

const START: OracleState = OracleState {
    kas_usd: 4_000_000,         // 0,04 USD
    oracle_daa: 1_000_000,
    seq: 7,
    stable_rate: 158_548_959,   // ≈ 5 %/Jahr
    stable_index: 1_000_000_000,
};

struct Committee {
    keys: Vec<Keypair>,
}

impl Committee {
    fn new() -> Self {
        Self { keys: (0..5).map(|_| random_keypair()).collect() }
    }
    fn pubkeys(&self) -> Vec<Vec<u8>> {
        self.keys.iter().map(|k| k.x_only_public_key().0.serialize().to_vec()).collect()
    }
}

fn random_keypair() -> Keypair {
    let secp = Secp256k1::new();
    let mut sk = [0u8; 32];
    loop {
        thread_rng().fill_bytes(&mut sk);
        if let Ok(secret) = SecretKey::from_slice(&sk) {
            return Keypair::from_secret_key(&secp, &secret);
        }
    }
}

fn compile(committee: &Committee, threshold: i64, s: OracleState) -> SilAbiArtifact {
    let mut args: Vec<ArtifactValue> = committee.pubkeys().into_iter().map(ArtifactValue::Bytes).collect();
    args.extend([
        ArtifactValue::Int(threshold),
        ArtifactValue::Int(MAX_RATE),
        ArtifactValue::Int(s.kas_usd),
        ArtifactValue::Int(s.oracle_daa),
        ArtifactValue::Int(s.seq),
        ArtifactValue::Int(s.stable_rate),
        ArtifactValue::Int(s.stable_index),
    ]);
    compile_to_sil_abi_artifact_with_options(SOURCE, &args, CompileOptions::default()).expect("risk_oracle.sil kompiliert")
}

/// Referenzrechnung des Index, genau wie im Vertrag (Ganzzahl, abgerundet).
/// Liefert None, wo auch der Vertrag überlaufen muss.
fn next_index_checked(prev: OracleState, new_daa: i64) -> Option<i64> {
    let delta = new_daa - prev.oracle_daa;
    let growth = prev.stable_rate.checked_mul(delta)? / RATE_SCALE;
    prev.stable_index.checked_add(prev.stable_index.checked_mul(growth)? / RATE_SCALE)
}

fn next_index(prev: OracleState, new_daa: i64) -> i64 {
    next_index_checked(prev, new_daa).expect("kein Überlauf in der Referenzrechnung")
}

/// Nachricht wie im Vertrag: covId ‖ preis ‖ daa ‖ seq ‖ rate, je 8 Byte LE.
/// Kodierung wie SilverScript `x as byte[8]` (OpNum2Bin): Betrag little-endian,
/// Vorzeichen im höchsten Bit – NICHT Zweierkomplement. Für x ≥ 0 identisch mit
/// to_le_bytes (Mutationstest v2: negative Werte waren falsch kodiert).
fn script_num8(v: i64) -> [u8; 8] {
    let mut b = v.unsigned_abs().to_le_bytes();
    if v < 0 {
        b[7] |= 0x80;
    }
    b
}

fn oracle_digest(cov_id: Hash, kas_usd: i64, daa: i64, seq: i64, rate: i64) -> [u8; 32] {
    let mut m = cov_id.as_bytes().to_vec();
    for v in [kas_usd, daa, seq, rate] {
        m.extend_from_slice(&script_num8(v));
    }
    Sha256::digest(&m).into()
}

fn sign(key: &Keypair, digest: [u8; 32]) -> Vec<u8> {
    key.sign_schnorr(Message::from_digest(digest)).as_ref().to_vec()
}

fn sigscript(artifact: &SilAbiArtifact, entry: &str, args: &[ArtifactValue]) -> Vec<u8> {
    let name = artifact.contracts.keys().next().unwrap().clone();
    let mut s = encode_contract_entry_sig_script(artifact, &name, entry, args).expect("sigscript");
    s.extend_from_slice(&push_redeem_script(&bytecode(artifact)));
    s
}

fn input(sig_script: Vec<u8>) -> TransactionInput {
    TransactionInput::new_with_compute_budget(
        TransactionOutpoint { transaction_id: TransactionId::from_bytes([9; 32]), index: 0 },
        sig_script,
        0,
        0,
    )
}

fn oracle_utxo(artifact: &SilAbiArtifact, cov: Hash) -> UtxoEntry {
    UtxoEntry::new(UTXO_VALUE, pay_to_script_hash_script(&bytecode(artifact)), 0, false, Some(cov))
}

fn continuation(artifact: &SilAbiArtifact, value: u64, cov: Hash) -> TransactionOutput {
    TransactionOutput {
        value,
        script_public_key: pay_to_script_hash_script(&bytecode(artifact)),
        covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: cov }),
    }
}

/// Ein Update-Versuch; jedes Feld lässt sich für Angriffstests verbiegen.
struct Update {
    new_kas_usd: i64,
    new_daa: i64,
    new_rate: i64,
    signers: Vec<usize>,
    /// was die Signierer tatsächlich unterschreiben (Standard = korrekt)
    signed_kas_usd: Option<i64>,
    signed_seq: Option<i64>,
    signed_cov: Option<Hash>,
    /// was im Ausgang steht (Standard = korrekt fortgeschriebener Zustand)
    out_state: Option<OracleState>,
    out_value: u64,
    lock_time: Option<u64>,
}

impl Update {
    fn honest(new_kas_usd: i64, new_daa: i64, new_rate: i64) -> Self {
        Self {
            new_kas_usd,
            new_daa,
            new_rate,
            signers: vec![0, 2, 4],
            signed_kas_usd: None,
            signed_seq: None,
            signed_cov: None,
            out_state: None,
            out_value: UTXO_VALUE,
            lock_time: None,
        }
    }

    fn expected_state(&self, prev: OracleState) -> OracleState {
        OracleState { stable_index: next_index(prev, self.new_daa), ..self.expected_state_unchecked(prev) }
    }

    /// Alles außer dem Index (für Überlauftests, wo es keinen gültigen Index gibt).
    fn expected_state_unchecked(&self, prev: OracleState) -> OracleState {
        OracleState {
            kas_usd: self.new_kas_usd,
            oracle_daa: self.new_daa,
            seq: prev.seq + 1,
            stable_rate: self.new_rate,
            stable_index: 0,
        }
    }

    fn run(&self, c: &Committee, prev: OracleState) -> Result<(), TxScriptError> {
        let current = compile(c, 3, prev);
        let next = compile(c, 3, self.out_state.unwrap_or_else(|| self.expected_state(prev)));
        let digest = oracle_digest(
            self.signed_cov.unwrap_or(COV_A),
            self.signed_kas_usd.unwrap_or(self.new_kas_usd),
            self.new_daa,
            self.signed_seq.unwrap_or(prev.seq + 1),
            self.new_rate,
        );
        let sigs: Vec<ArtifactValue> = self.signers.iter().map(|&i| ArtifactValue::Bytes(sign(&c.keys[i], digest))).collect();
        let idx: Vec<ArtifactValue> = self.signers.iter().map(|&i| ArtifactValue::Int(i as i64)).collect();
        let script = sigscript(
            &current,
            "update",
            &[
                ArtifactValue::Int(self.new_kas_usd),
                ArtifactValue::Int(self.new_daa),
                ArtifactValue::Int(self.new_rate),
                ArtifactValue::Array(sigs),
                ArtifactValue::Array(idx),
            ],
        );
        let tx = Transaction::new(
            1,
            vec![input(script)],
            vec![continuation(&next, self.out_value, COV_A)],
            self.lock_time.unwrap_or(self.new_daa as u64),
            Default::default(),
            0,
            vec![],
        );
        execute_input_with_covenants(tx, vec![oracle_utxo(&current, COV_A)], 0)
    }
}

fn read(c: &Committee, prev: OracleState, out_state: OracleState, out_value: u64) -> Result<(), TxScriptError> {
    let current = compile(c, 3, prev);
    let next = compile(c, 3, out_state);
    let script = sigscript(&current, "read", &[]);
    let tx = Transaction::new(1, vec![input(script)], vec![continuation(&next, out_value, COV_A)], 0, Default::default(), 0, vec![]);
    execute_input_with_covenants(tx, vec![oracle_utxo(&current, COV_A)], 0)
}

fn good() -> Update {
    Update::honest(4_200_000, START.oracle_daa + 600, 200_000_000)
}

// ---------------------------------------------------------------- update ----

#[test]
fn update_mit_drei_gueltigen_signaturen_geht_durch() {
    let c = Committee::new();
    good().run(&c, START).expect("ehrliches Update");
}

#[test]
fn index_waechst_mit_altem_satz() {
    // Plausibilität der Referenzrechnung: 600 DAA ≈ 1 min bei 5 %/Jahr
    let idx = next_index(START, START.oracle_daa + 600);
    assert_eq!(idx, 1_000_000_095, "1e9 · 1,585e-10 · 600 ≈ 95");
}

#[test]
fn falscher_index_im_ausgang_wird_abgelehnt() {
    let c = Committee::new();
    let mut u = good();
    let mut s = u.expected_state(START);
    s.stable_index += 1;
    u.out_state = Some(s);
    assert!(u.run(&c, START).is_err());
}

#[test]
fn nur_zwei_signaturen_reichen_nicht() {
    let c = Committee::new();
    let mut u = good();
    u.signers = vec![0, 1];
    assert!(u.run(&c, START).is_err());
}

// Mutationstest 28.09.2026: nur_zwei_... bleibt auch ohne die Längenprüfung
// rot, weil die Schleife dann über das Array-Ende liest. Die Längenprüfung
// selbst deckt erst dieser Fall ab:
#[test]
fn vier_signaturen_bei_schwelle_drei_werden_abgelehnt() {
    let c = Committee::new();
    let mut u = good();
    u.signers = vec![0, 1, 2, 3];
    assert!(u.run(&c, START).is_err());
}

#[test]
fn derselbe_signierer_zaehlt_nicht_doppelt() {
    let c = Committee::new();
    let mut u = good();
    u.signers = vec![1, 1, 3];
    assert!(u.run(&c, START).is_err());
}

#[test]
fn absteigende_signierer_reihenfolge_wird_abgelehnt() {
    let c = Committee::new();
    let mut u = good();
    u.signers = vec![4, 2, 0];
    assert!(u.run(&c, START).is_err());
}

#[test]
fn signierter_preis_muss_eingereichtem_preis_entsprechen() {
    let c = Committee::new();
    let mut u = good();
    u.signed_kas_usd = Some(3_000_000);
    assert!(u.run(&c, START).is_err());
}

#[test]
fn signatur_fuer_alte_seq_ist_kein_replay() {
    let c = Committee::new();
    let mut u = good();
    u.signed_seq = Some(START.seq); // Nachricht aus dem vorigen Update
    assert!(u.run(&c, START).is_err());
}

#[test]
fn signatur_fuer_anderes_orakel_gilt_nicht() {
    let c = Committee::new();
    let mut u = good();
    u.signed_cov = Some(COV_B);
    assert!(u.run(&c, START).is_err());
}

#[test]
fn fremdes_komitee_wird_abgelehnt() {
    let c = Committee::new();
    let fremd = Committee::new();
    // Vertrag mit Komitee c, Signaturen von fremd
    let current = compile(&c, 3, START);
    let u = good();
    let next = compile(&c, 3, u.expected_state(START));
    let digest = oracle_digest(COV_A, u.new_kas_usd, u.new_daa, START.seq + 1, u.new_rate);
    let sigs = [0, 2, 4].iter().map(|&i| ArtifactValue::Bytes(sign(&fremd.keys[i], digest))).collect();
    let script = sigscript(
        &current,
        "update",
        &[
            ArtifactValue::Int(u.new_kas_usd),
            ArtifactValue::Int(u.new_daa),
            ArtifactValue::Int(u.new_rate),
            ArtifactValue::Array(sigs),
            ArtifactValue::Array(vec![0.into(), 2.into(), 4.into()]),
        ],
    );
    let tx = Transaction::new(1, vec![input(script)], vec![continuation(&next, UTXO_VALUE, COV_A)], u.new_daa as u64, Default::default(), 0, vec![]);
    assert!(execute_input_with_covenants(tx, vec![oracle_utxo(&current, COV_A)], 0).is_err());
}

#[test]
fn daa_muss_steigen() {
    let c = Committee::new();
    let u = Update::honest(4_200_000, START.oracle_daa, 200_000_000);
    assert!(u.run(&c, START).is_err());
}

#[test]
fn preis_aus_der_zukunft_scheitert_an_locktime() {
    let c = Committee::new();
    let mut u = good();
    u.lock_time = Some(u.new_daa as u64 - 1);
    assert!(u.run(&c, START).is_err());
}

#[test]
fn zinssatz_ueber_obergrenze_wird_abgelehnt() {
    let c = Committee::new();
    good_with_rate(&c, MAX_RATE).expect("Obergrenze selbst ist erlaubt");
    assert!(good_with_rate(&c, MAX_RATE + 1).is_err());
}

fn good_with_rate(c: &Committee, rate: i64) -> Result<(), TxScriptError> {
    Update::honest(4_200_000, START.oracle_daa + 600, rate).run(c, START)
}

#[test]
fn update_darf_keine_kas_abziehen() {
    let c = Committee::new();
    let mut u = good();
    u.out_value = UTXO_VALUE - 1;
    assert!(u.run(&c, START).is_err());
}

// ------------------------------------------------------------------ read ----

#[test]
fn read_mit_unveraendertem_zustand_geht_durch() {
    let c = Committee::new();
    read(&c, START, START, UTXO_VALUE).expect("Lesen");
}

#[test]
fn read_darf_preis_nicht_aendern() {
    let c = Committee::new();
    let mut s = START;
    s.kas_usd = 9_000_000;
    assert!(read(&c, START, s, UTXO_VALUE).is_err());
}

#[test]
fn read_darf_keine_kas_abziehen() {
    let c = Committee::new();
    assert!(read(&c, START, START, UTXO_VALUE - 1).is_err());
}

// ------------------------------------------------------------ Überlauf ----

/// Wie lange darf das Orakel schweigen, bevor die Indexrechnung überläuft?
/// Test hält das tatsächliche Verhalten der Engine fest (Fehler statt
/// Wrap-around ist das, was wir brauchen).
#[test]
fn ueberlauf_bei_langer_pause_bricht_ab_statt_umzubrechen() {
    let c = Committee::new();
    let mut prev = START;
    prev.stable_rate = MAX_RATE;
    prev.stable_index = 10_000_000_000; // Index hat sich verzehnfacht
    // growth = Δ (bei rate 1e9); index·growth = 1e10·Δ > i64::MAX ab Δ ≈ 9,2e8
    let ok_daa = prev.oracle_daa + 900_000_000;
    let bad_daa = prev.oracle_daa + 1_000_000_000;
    assert!(next_index_checked(prev, bad_daa).is_none(), "Testfall muss wirklich überlaufen");
    Update::honest(4_200_000, ok_daa, MAX_RATE).run(&c, prev).expect("≈ 2,9 Jahre Pause gehen noch");

    // Ausgang mit umgebrochenem (wrapping) Wert: den darf die Engine nicht akzeptieren
    let mut u = Update::honest(4_200_000, bad_daa, MAX_RATE);
    let growth = MAX_RATE * (bad_daa - prev.oracle_daa) / RATE_SCALE;
    let mut wrapped = u.expected_state_unchecked(prev);
    wrapped.stable_index = prev.stable_index.wrapping_add(prev.stable_index.wrapping_mul(growth) / RATE_SCALE);
    u.out_state = Some(wrapped);
    let err = u.run(&c, prev).expect_err("Überlauf muss scheitern");
    println!("Engine-Fehler bei Überlauf: {err:?}");
    assert!(
        !matches!(err, TxScriptError::VerifyError | TxScriptError::EvalFalse),
        "erwartet Arithmetikfehler, nicht bloß falschen Ausgang: {err:?}"
    );
}

#[test]
fn alte_reihenfolge_waere_frueh_uebergelaufen() {
    // Dokumentiert, warum growth zuerst gerechnet wird: index·rate mit
    // index 1e10 und rate 1e9 sprengt i64 schon bei Δ = 1.
    assert!(10_000_000_000i64.checked_mul(MAX_RATE).is_none());
    let mut prev = START;
    prev.stable_rate = MAX_RATE;
    prev.stable_index = 10_000_000_000;
    assert!(next_index_checked(prev, prev.oracle_daa + 1).is_some());
}

// ---------------------------------------------------------------- Messung ----

/// Größen für die Gebührenabschätzung (ARCHITEKTUR.md Abschnitt 3, Schritt 3).
/// Ausgabe mit: cargo test --test oracle_tests groessen -- --nocapture
#[test]
fn groessen() {
    let c = Committee::new();
    let a = compile(&c, 3, START);
    let redeem = bytecode(&a).len();
    let read_sig = sigscript(&a, "read", &[]).len();
    let u = good();
    let digest = oracle_digest(COV_A, u.new_kas_usd, u.new_daa, START.seq + 1, u.new_rate);
    let sigs = [0, 2, 4].iter().map(|&i| ArtifactValue::Bytes(sign(&c.keys[i], digest))).collect();
    let update_sig = sigscript(
        &a,
        "update",
        &[
            ArtifactValue::Int(u.new_kas_usd),
            ArtifactValue::Int(u.new_daa),
            ArtifactValue::Int(u.new_rate),
            ArtifactValue::Array(sigs),
            ArtifactValue::Array(vec![0.into(), 2.into(), 4.into()]),
        ],
    )
    .len();
    println!("RiskOracle: Redeem-Skript {redeem} B, Sigscript read {read_sig} B, update {update_sig} B");
    assert!(redeem < 10_000, "Redeem-Skript unerwartet groß");
}


// ------------------------------------------------ Version 2: Preisgrenzen ----

fn upd(c: &Committee, prev: OracleState, price: i64) -> Result<(), TxScriptError> {
    Update::honest(price, prev.oracle_daa + 600, prev.stable_rate).run(c, prev)
}

#[test]
fn v2_preis_hoechstens_verdoppeln_oder_halbieren() {
    // Audit 3 O-2: vorher akzeptierte das Orakel jeden Preis bis 1e18
    let c = Committee::new();
    upd(&c, START, START.kas_usd * 2).expect("genau ×2 erlaubt");
    upd(&c, START, START.kas_usd / 2).expect("genau ÷2 erlaubt");
    assert!(upd(&c, START, START.kas_usd * 2 + 1).is_err(), "mehr als ×2");
    assert!(upd(&c, START, START.kas_usd / 2 - 1).is_err(), "weniger als ÷2");
}

#[test]
fn v2_absolute_preisgrenzen() {
    let c = Committee::new();
    let mut low = START;
    low.kas_usd = 1_500;
    upd(&c, low, 1_000).expect("Untergrenze 0,00001 USD erlaubt");
    assert!(upd(&c, low, 999).is_err(), "unter der Untergrenze");
    let mut high = START;
    high.kas_usd = 60_000_000_000;
    upd(&c, high, 90_000_000_000).expect("Obergrenze 900 USD erlaubt");
    assert!(upd(&c, high, 90_000_000_001).is_err(), "über der Obergrenze");
}

#[test]
fn v2_negativer_zinssatz_wird_abgelehnt() {
    // L86 (nach Mutationstest v2 ohne eigenen Test): ein negativer Satz ließe den Index schrumpfen
    let c = Committee::new();
    Update::honest(4_200_000, START.oracle_daa + 600, 0).run(&c, START).expect("Satz 0 erlaubt");
    assert!(Update::honest(4_200_000, START.oracle_daa + 600, -1).run(&c, START).is_err());
}

#[test]
fn read_darf_keine_zweite_fortsetzung_erzeugen() {
    // L40: zwei Orakel-UTXOs mit derselben Covenant-ID = zwei gültige Preise
    let c = Committee::new();
    let current = compile(&c, 3, START);
    let mut s2 = START;
    s2.kas_usd = 9_000_000;
    let fake = compile(&c, 3, s2);
    let script = sigscript(&current, "read", &[]);
    let tx = Transaction::new(
        1,
        vec![input(script)],
        vec![continuation(&current, UTXO_VALUE, COV_A), continuation(&fake, UTXO_VALUE, COV_A)],
        0,
        Default::default(),
        0,
        vec![],
    );
    assert!(execute_input_with_covenants(tx, vec![oracle_utxo(&current, COV_A)], 0).is_err());
}

#[test]
fn v21_mindestabstand_zwischen_updates() {
    // Fix-Review I-3: ×2/÷2 allein bremst nicht in der Zeit
    let c = Committee::new();
    Update::honest(4_200_000, START.oracle_daa + 600, START.stable_rate).run(&c, START).expect("600 DAA reichen");
    assert!(Update::honest(4_200_000, START.oracle_daa + 599, START.stable_rate).run(&c, START).is_err(), "599 DAA zu früh");
}
