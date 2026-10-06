//! Dauerauftrag mit Tresor (contracts/standing_order.sil) in der Skript-Engine:
//! Zahlung nur zum Termin, genau an den Empfänger, Fortsetzung mit dem
//! nächsten Termin; Kündigen und Auffüllen nur durch den Absender; die
//! Kalenderrechnung des Vertrags gegen chrono (src/standing.rs).

mod common;

use chrono::NaiveDate;
use common::{bytecode, execute_input_with_covenants, push_redeem_script};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::tx::{
    CovenantBinding, MutableTransaction, ScriptPublicKey, Transaction, TransactionId, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_lending_protocol::standing::{self, DAY_MS};
use kaspa_txscript::opcodes::codes::OpTrue;
use kaspa_txscript::pay_to_script_hash_script;
use rand::{Rng, RngCore, SeedableRng, rngs::StdRng, thread_rng};
use secp256k1::{Keypair, Message, Secp256k1, SecretKey};
use silverscript_abi::{ArtifactValue, SilAbiArtifact, encode_contract_entry_sig_script};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

const SRC: &str = include_str!("../../contracts/standing_order.sil");
const COV: Hash = Hash::from_bytes([0x5a; 32]);
const COV2: Hash = Hash::from_bytes([0x5b; 32]);
const E8: i64 = 100_000_000;
const AMOUNT: i64 = 10 * E8;
const MAX_FEE: i64 = 1_000_000; // 0,01 KAS
const FUND: i64 = 100 * E8;



fn key() -> Keypair {
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
fn ms(y: i32, m: u32, d: u32, h: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, 0, 0).unwrap().and_utc().timestamp_millis()
}
fn p2pk(k: &Keypair) -> ScriptPublicKey {
    let mut s = vec![0x20];
    s.extend(xonly(k));
    s.push(0xac);
    ScriptPublicKey::new(0, s.into())
}

#[derive(Clone, Copy)]
struct Order {
    anchor: i64,
    period: i64,
    next_due: i64,
    left: i64,
}

struct Env {
    owner: Keypair,
    recipient: Keypair,
    /// beim Anlegen hinterlegte Nachricht (Payload jeder Zahlung)
    payload: Vec<u8>,
}

fn sha(b: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha256::digest(b).to_vec()
}

impl Env {
    fn new() -> Self {
        Env { owner: key(), recipient: key(), payload: vec![] }
    }
    fn with_message(payload: &[u8]) -> Self {
        Env { payload: payload.to_vec(), ..Env::new() }
    }
    fn art(&self, o: Order) -> SilAbiArtifact {
        compile_to_sil_abi_artifact_with_options(
            SRC,
            &[
                ArtifactValue::Bytes(xonly(&self.owner)),
                ArtifactValue::Bytes(xonly(&self.recipient)),
                ArtifactValue::Int(AMOUNT),
                ArtifactValue::Int(o.anchor),
                ArtifactValue::Int(o.period),
                ArtifactValue::Int(MAX_FEE),
                ArtifactValue::Bytes(sha(&self.payload)),
                ArtifactValue::Int(o.next_due),
                ArtifactValue::Int(o.left),
            ],
            CompileOptions::default(),
        )
        .expect("StandingOrder kompiliert")
    }
}

/// Monatlich zum 1., 08:00 UTC, 5 Zahlungen offen
fn monthly() -> Order {
    Order { anchor: 1, period: 0, next_due: ms(2027, 1, 1, 8), left: 5 }
}
fn next(o: Order) -> Order {
    Order { next_due: standing::following(o.next_due, o.anchor, o.period), left: if o.left > 0 { o.left - 1 } else { o.left }, ..o }
}

struct In {
    art: SilAbiArtifact,
    value: i64,
    cov: Hash,
    entry: &'static str,
    signer: Option<Keypair>,
}

fn cov_out(art: &SilAbiArtifact, value: i64, auth: u16, cov: Hash) -> TransactionOutput {
    TransactionOutput { value: value as u64, script_public_key: pay_to_script_hash_script(&bytecode(art)), covenant: Some(CovenantBinding { authorizing_input: auth, covenant_id: cov }) }
}
fn plain(spk: ScriptPublicKey, value: i64) -> TransactionOutput {
    TransactionOutput { value: value as u64, script_public_key: spk, covenant: None }
}
fn anyone(value: i64) -> TransactionOutput {
    plain(ScriptPublicKey::new(0, vec![OpTrue].into()), value)
}

/// Führt jeden Eingang aus; lock_time = Zeitpunkt der Tx (Unix-ms), ohne Payload
fn execute(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64) -> Vec<Result<(), String>> {
    execute_p(inputs, outputs, lock_time, &[])
}

/// wie execute, mit Payload der Tx
fn execute_p(inputs: Vec<In>, outputs: Vec<TransactionOutput>, lock_time: u64, payload: &[u8]) -> Vec<Result<(), String>> {
    let entries: Vec<UtxoEntry> = inputs.iter().map(|i| UtxoEntry::new(i.value as u64, pay_to_script_hash_script(&bytecode(&i.art)), 0, false, Some(i.cov))).collect();
    let op = |i: usize| TransactionOutpoint { transaction_id: TransactionId::from_bytes([0x70 + i as u8; 32]), index: i as u32 };
    let bare: Vec<TransactionInput> = (0..inputs.len()).map(|i| TransactionInput::new_with_compute_budget(op(i), vec![], 0, 0)).collect();
    let unsigned = Transaction::new(1, bare, outputs.clone(), lock_time, Default::default(), 0, payload.to_vec());
    let mut fin = vec![];
    for (idx, i) in inputs.iter().enumerate() {
        let mut args = vec![];
        if let Some(k) = i.signer {
            let mtx = MutableTransaction::with_entries(unsigned.clone(), entries.clone());
            let h = calc_schnorr_signature_hash(&mtx.as_verifiable(), idx, SIG_HASH_ALL, &SigHashReusedValuesUnsync::new());
            let mut s = k.sign_schnorr(Message::from_digest_slice(h.as_bytes().as_slice()).unwrap()).as_ref().to_vec();
            s.push(SIG_HASH_ALL.to_u8());
            args.push(ArtifactValue::Bytes(s));
        }
        let mut script = encode_contract_entry_sig_script(&i.art, "StandingOrder", i.entry, &args).expect("sigscript");
        script.extend_from_slice(&push_redeem_script(&bytecode(&i.art)));
        fin.push(TransactionInput::new_with_compute_budget(op(idx), script, 0, 0));
    }
    let tx = Transaction::new(1, fin, outputs, lock_time, Default::default(), 0, payload.to_vec());
    (0..tx.inputs.len()).map(|i| execute_input_with_covenants(tx.clone(), entries.clone(), i).map_err(|e| format!("{e:?}"))).collect()
}

/// Standardzahlung: Ausgang 0 an den Empfänger, Ausgang 1 Fortsetzung
struct Pay {
    at: i64,
    to: Option<ScriptPublicKey>,
    amount: i64,
    keep: i64,
    next: Option<Order>,
    /// Payload der Zahlung (Standard: die hinterlegte Nachricht)
    payload: Option<Vec<u8>>,
}
impl Pay {
    fn new(o: Order) -> Self {
        Pay { at: o.next_due, to: None, amount: AMOUNT, keep: FUND - AMOUNT - MAX_FEE, next: None, payload: None }
    }
    fn run(&self, e: &Env, o: Order) -> Vec<Result<(), String>> {
        let art = e.art(o);
        let n = self.next.unwrap_or(next(o));
        execute_p(
            vec![In { art: art.clone(), value: FUND, cov: COV, entry: "pay", signer: None }],
            vec![plain(self.to.clone().unwrap_or(p2pk(&e.recipient)), self.amount), cov_out(&e.art(n), self.keep, 0, COV)],
            self.at as u64,
            self.payload.as_deref().unwrap_or(&e.payload),
        )
    }
}

fn ok(r: &[Result<(), String>]) -> bool {
    r.iter().all(|x| x.is_ok())
}

#[test]
fn zahlung_zum_termin() {
    let e = Env::new();
    let o = monthly();
    assert!(ok(&Pay::new(o).run(&e, o)), "pünktlich");
    let mut late = Pay::new(o);
    late.at = o.next_due + 40 * DAY_MS;
    assert!(ok(&late.run(&e, o)), "später geht auch (Rückstand)");
    let mut early = Pay::new(o);
    early.at = o.next_due - 1;
    assert!(early.run(&e, o)[0].is_err(), "1 ms zu früh");
}

#[test]
fn nur_an_den_empfaenger_und_genau_der_betrag() {
    let e = Env::new();
    let o = monthly();
    let mut p = Pay::new(o);
    p.to = Some(p2pk(&key()));
    assert!(p.run(&e, o)[0].is_err(), "fremder Empfänger");
    // Fortsetzung jeweils korrekt, nur der Zahlungsbetrag weicht ab
    for amount in [AMOUNT - 1, AMOUNT + 1] {
        let mut p = Pay::new(o);
        p.amount = amount;
        assert!(p.run(&e, o)[0].is_err(), "Betrag {amount}");
    }
}

#[test]
fn hoechstens_betrag_plus_gebuehr_verlaesst_den_tresor() {
    let e = Env::new();
    let o = monthly();
    let mut p = Pay::new(o);
    p.keep = FUND - AMOUNT - MAX_FEE - 1;
    assert!(p.run(&e, o)[0].is_err());
    p.keep = FUND - AMOUNT; // mehr behalten ist erlaubt (Gebühr von anderswo)
    assert!(ok(&p.run(&e, o)));
}

#[test]
fn fortsetzung_mit_naechstem_termin() {
    let e = Env::new();
    let o = monthly();
    let n = next(o);
    assert_eq!(n.next_due, ms(2027, 2, 1, 8));
    for (label, bad) in [
        ("Termin bleibt", Order { next_due: o.next_due, ..n }),
        ("Termin zu weit", Order { next_due: n.next_due + 1, ..n }),
        ("Anzahl bleibt", Order { left: o.left, ..n }),
        ("unbegrenzt gemacht", Order { left: -1, ..n }),
    ] {
        let mut p = Pay::new(o);
        p.next = Some(bad);
        assert!(p.run(&e, o)[0].is_err(), "{label}");
    }
}

#[test]
fn unbegrenzt_bleibt_unbegrenzt_und_nach_der_letzten_ist_schluss() {
    let e = Env::new();
    let inf = Order { left: -1, ..monthly() };
    assert_eq!(next(inf).left, -1);
    assert!(ok(&Pay::new(inf).run(&e, inf)));
    let last = Order { left: 1, ..monthly() };
    assert_eq!(next(last).left, 0);
    assert!(ok(&Pay::new(last).run(&e, last)), "letzte Zahlung");
    // left = 0: Fortsetzung genau so, wie sie ohne die Regel aussähe
    let done = Order { left: 0, ..monthly() };
    assert_eq!(next(done).left, 0);
    assert!(Pay::new(done).run(&e, done)[0].is_err(), "nichts mehr offen");
}

#[test]
fn ohne_fortsetzung_oder_mit_zweiter_scheitert() {
    let e = Env::new();
    let o = monthly();
    let art = e.art(o);
    let run = |outs: Vec<TransactionOutput>| execute(vec![In { art: art.clone(), value: FUND, cov: COV, entry: "pay", signer: None }], outs, o.next_due as u64);
    let r = run(vec![plain(p2pk(&e.recipient), AMOUNT), anyone(FUND - AMOUNT - MAX_FEE)]);
    assert!(r[0].is_err(), "Rest ohne Covenant weggeleitet");
    let n = e.art(next(o));
    let r = run(vec![plain(p2pk(&e.recipient), AMOUNT), cov_out(&n, FUND - AMOUNT - MAX_FEE, 0, COV), cov_out(&n, 1, 0, COV)]);
    assert!(r[0].is_err(), "zweite Fortsetzung");
}

#[test]
fn zwei_tresore_teilen_sich_keinen_ausgang() {
    // Beide Tresore zahlen denselben Betrag an denselben Empfänger. Zeigten
    // beide auf Ausgang 0, bekäme er einmal, was zweimal abgebucht wird.
    let e = Env::new();
    let o = monthly();
    let a1 = e.art(o);
    let n = e.art(next(o));
    let keep = FUND - AMOUNT - MAX_FEE;
    let inputs = || {
        vec![
            In { art: a1.clone(), value: FUND, cov: COV, entry: "pay", signer: None },
            In { art: a1.clone(), value: FUND, cov: COV2, entry: "pay", signer: None },
        ]
    };
    // ehrlich: Ausgang 0 und 1 an den Empfänger, Fortsetzungen dahinter
    let honest = execute(
        inputs(),
        vec![plain(p2pk(&e.recipient), AMOUNT), plain(p2pk(&e.recipient), AMOUNT), cov_out(&n, keep, 0, COV), cov_out(&n, keep, 1, COV2)],
        o.next_due as u64,
    );
    assert!(ok(&honest), "Gegenprobe: {honest:?}");
    // Angriff: nur ein Zahlungsausgang, der Rest an den Auslöser
    let attack = execute(
        inputs(),
        vec![plain(p2pk(&e.recipient), AMOUNT), cov_out(&n, keep, 0, COV), cov_out(&n, keep, 1, COV2), anyone(AMOUNT)],
        o.next_due as u64,
    );
    assert!(attack[0].is_err() || attack[1].is_err(), "{attack:?}");
    assert!(attack[1].is_err(), "Tresor 2 muss seinen eigenen Ausgang 1 verlangen: {attack:?}");
}

#[test]
fn kuendigen_nur_durch_den_absender() {
    let e = Env::new();
    let o = monthly();
    let run = |k: Keypair| execute(vec![In { art: e.art(o), value: FUND, cov: COV, entry: "cancel", signer: Some(k) }], vec![anyone(FUND)], 0);
    assert!(ok(&run(e.owner)), "Absender");
    assert!(run(e.recipient)[0].is_err(), "Empfänger darf nicht kündigen");
    assert!(run(key())[0].is_err(), "Fremder");
    // Kündigen darf die Covenant-ID nicht weiterleben lassen
    let r = execute(vec![In { art: e.art(o), value: FUND, cov: COV, entry: "cancel", signer: Some(e.owner) }], vec![cov_out(&e.art(o), FUND, 0, COV)], 0);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn auffuellen_nur_durch_den_absender_und_nur_mehr() {
    let e = Env::new();
    let o = monthly();
    let run = |k: Keypair, value: i64, state: Order| {
        execute(vec![In { art: e.art(o), value: FUND, cov: COV, entry: "topUp", signer: Some(k) }], vec![cov_out(&e.art(state), value, 0, COV)], 0)
    };
    assert!(ok(&run(e.owner, FUND + E8, o)));
    assert!(run(key(), FUND + E8, o)[0].is_err(), "Fremder");
    assert!(run(e.owner, FUND, o)[0].is_err(), "nicht mehr als vorher");
    assert!(run(e.owner, FUND + E8, Order { next_due: o.next_due - 1, ..o })[0].is_err(), "Termin vorziehen");
    assert!(run(e.owner, FUND + E8, Order { left: 99, ..o })[0].is_err(), "Anzahl ändern");
}

// -------------------------------------------------------- Kalender ----

fn probe() -> SilAbiArtifact {
    let a = SRC.find("// CAL-BEGIN").unwrap();
    let b = SRC.find("// CAL-END").unwrap();
    let src = format!(
        "pragma silverscript ^0.1.0;
contract CalProbe() {{
    int constant DAY_MS = 86400000;
{}
    entry next(int due, int anchor, int period, int expected) {{ require(following(due, anchor, period) == expected); }}
}}",
        &SRC[a..b]
    );
    compile_to_sil_abi_artifact_with_options(&src, &[], CompileOptions::default()).expect("CalProbe kompiliert")
}

fn run_probe(p: &SilAbiArtifact, args: &[i64]) -> bool {
    let args: Vec<ArtifactValue> = args.iter().map(|&v| ArtifactValue::Int(v)).collect();
    let mut s = encode_contract_entry_sig_script(p, "CalProbe", "next", &args).unwrap();
    s.extend_from_slice(&push_redeem_script(&bytecode(p)));
    let input = TransactionInput::new_with_compute_budget(TransactionOutpoint { transaction_id: TransactionId::from_bytes([9; 32]), index: 0 }, s, 0, 0);
    let tx = Transaction::new(1, vec![input], vec![anyone(1)], 0, Default::default(), 0, vec![]);
    let utxo = UtxoEntry::new(1_000, pay_to_script_hash_script(&bytecode(p)), 0, false, None);
    execute_input_with_covenants(tx, vec![utxo], 0).is_ok()
}

fn check(p: &SilAbiArtifact, due: i64, anchor: i64, period: i64) {
    let want = standing::following(due, anchor, period);
    assert!(run_probe(p, &[due, anchor, period, want]), "following({due}, {anchor}, {period}) sollte {want} sein");
    assert!(!run_probe(p, &[due, anchor, period, want + 1]), "falscher Wert angenommen");
}

#[test]
fn kalender_grenzfaelle() {
    let p = probe();
    for (due, anchor) in [
        (ms(2027, 1, 31, 8), 31),
        (ms(2027, 2, 28, 8), 31),
        (ms(2028, 1, 31, 23), 31),
        (ms(2028, 2, 29, 0), 29),
        (ms(2027, 12, 15, 12), 15),
        (ms(2099, 12, 31, 0), 31),
        (ms(2100, 1, 29, 0), 29), // 2100: kein Schaltjahr
        (ms(2000, 1, 1, 0), 1),
        (ms(2000, 2, 29, 0), 30), // 2000: Schaltjahr
        (ms(2199, 11, 30, 0), 31),
    ] {
        check(&p, due, anchor, 0);
    }
    check(&p, ms(2027, 3, 3, 0), 0, 7 * DAY_MS);
    check(&p, ms(2027, 3, 3, 0) + 12_345, 0, DAY_MS);
}

#[test]
fn kalender_zufallswerte() {
    let p = probe();
    let mut rng = StdRng::seed_from_u64(20260929);
    for _ in 0..120 {
        let due = rng.gen_range(ms(1986, 1, 1, 0)..standing::MAX_TIME - 40 * DAY_MS);
        let anchor = rng.gen_range(1..=31);
        check(&p, due, anchor, 0);
    }
}

// ------------------------------------------------ nach dem Mutationstest ----

/// Zahlung aus einem Tresor mit Wert `fund` und frei gewählter Fortsetzung
fn pay_with(e: &Env, o: Order, fund: i64, cont: Vec<TransactionOutput>) -> Vec<Result<(), String>> {
    let mut outs = vec![plain(p2pk(&e.recipient), AMOUNT)];
    outs.extend(cont);
    execute(vec![In { art: e.art(o), value: fund, cov: COV, entry: "pay", signer: None }], outs, o.next_due as u64)
}

#[test]
fn leerer_tresor_zahlt_nicht_mit_nullfortsetzung() {
    // keep > 0: genau Betrag + Gebühr im Tresor ließe eine Fortsetzung mit 0 sompi
    let e = Env::new();
    let o = monthly();
    let r = pay_with(&e, o, AMOUNT + MAX_FEE, vec![cov_out(&e.art(next(o)), 0, 0, COV)]);
    assert!(r[0].is_err(), "{r:?}");
    let ok_ = pay_with(&e, o, AMOUNT + MAX_FEE + 1, vec![cov_out(&e.art(next(o)), 1, 0, COV)]);
    assert!(ok(&ok_), "Gegenprobe: {ok_:?}");
}

#[test]
fn intervall_null_zahlt_nicht_endlos() {
    // due > nextDue: mit Intervall 0 ließe sich alles sofort auszahlen
    let e = Env::new();
    let o = Order { anchor: 0, period: 0, ..monthly() };
    assert_eq!(next(o).next_due, o.next_due);
    assert!(Pay::new(o).run(&e, o)[0].is_err());
    let w = Order { anchor: 0, period: 7 * DAY_MS, ..monthly() };
    assert!(ok(&Pay::new(w).run(&e, w)), "Gegenprobe wöchentlich");
}

#[test]
fn termine_hoechstens_bis_2200() {
    let e = Env::new();
    let o = Order { anchor: 15, next_due: ms(2199, 12, 15, 0), ..monthly() };
    assert!(next(o).next_due > standing::MAX_TIME);
    assert!(Pay::new(o).run(&e, o)[0].is_err());
    let o = Order { anchor: 15, next_due: ms(2199, 11, 15, 0), ..monthly() };
    assert!(ok(&Pay::new(o).run(&e, o)), "Gegenprobe");
}

#[test]
fn auffuellen_ohne_zweite_fortsetzung() {
    // topUp: genau eine Fortsetzung; eine zweite unter derselben Covenant-ID
    // mit beliebigem Skript darf nicht entstehen
    let e = Env::new();
    let o = monthly();
    let r = execute(
        vec![In { art: e.art(o), value: FUND, cov: COV, entry: "topUp", signer: Some(e.owner) }],
        vec![cov_out(&e.art(o), FUND + E8, 0, COV), TransactionOutput { covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: COV }), ..anyone(E8) }],
        0,
    );
    assert!(r[0].is_err(), "{r:?}");
}

// ------------------------------------------- Audit 12 (Nachprüfung, Opus) ----

/// A12-8 (übernommen, dokumentiert): Die Höchstgebühr muss nicht ins Netz
/// gehen – der Vertrag lässt sie an einen beliebigen Ausgang fließen (hier: an
/// den Auslöser). Bewusst nicht im Vertrag gebunden (siehe audit/12-stand-a.md);
/// Seite und Hilfe sagen es jetzt.
#[test]
fn a12_ausloeser_darf_den_rest_der_hoechstgebuehr_behalten() {
    let e = Env::new();
    let o = monthly();
    let keep = FUND - AMOUNT - MAX_FEE;
    let r = pay_with(&e, o, FUND, vec![cov_out(&e.art(next(o)), keep, 0, COV), anyone(MAX_FEE)]);
    assert!(ok(&r), "{r:?}");
}

/// A12 (übernommen): Grenzfälle der Termine – Anker > 31 wirkt wie
/// Monatsletzter, Anker negativ wie Intervall; der Vertrag lässt beides zu,
/// ghostctl nicht (check_params, TresorCode::decode, A12-16)
#[test]
fn a12_anker_ausserhalb_1_bis_31() {
    let p = probe();
    // Anker 40 → Monatsletzter
    let due = ms(2027, 1, 31, 8);
    assert!(run_probe(&p, &[due, 40, 0, ms(2027, 2, 28, 8)]));
    // Anker −1 mit Intervall 7 Tage → wie Anker 0
    assert!(run_probe(&p, &[due, -1, 7 * DAY_MS, due + 7 * DAY_MS]));
}

/// A12 (übernommen): Termin unter LOCK_TIME_THRESHOLD (Anlegen mit nextDue in
/// ms < 5e11) ist nie zahlbar – der Tresor bleibt, bis der Absender kündigt;
/// ghostctl lässt solche Termine weder beim Anlegen noch beim Import zu
#[test]
fn a12_termin_unter_der_zeitschwelle_zahlt_nie() {
    let e = Env::new();
    let o = Order { anchor: 0, period: 7 * DAY_MS, next_due: 1_000_000, left: 3 };
    let r = Pay::new(o).run(&e, o);
    assert!(r[0].is_err(), "{r:?}");
    let mut p = Pay::new(o);
    p.at = ms(2027, 1, 1, 0);
    assert!(p.run(&e, o)[0].is_err(), "auch mit Zeit-Locktime nicht");
}

// ------------------------------------- Nachricht fest gebunden (A12-1) ----

#[test]
fn nachricht_ist_an_jede_zahlung_gebunden() {
    // eine an den Empfänger verschlüsselte Nachricht (Format egal: gebunden
    // sind die Bytes)
    let mut blob = b"GHM\x01".to_vec();
    blob.extend((0..87u8).map(|i| i.wrapping_mul(37)));
    let e = Env::with_message(&blob);
    let o = monthly();
    assert!(ok(&Pay::new(o).run(&e, o)), "mit der hinterlegten Nachricht");
    let mut wrong = blob.clone();
    wrong[40] ^= 1;
    for (label, p) in [("ein Bit anders", wrong), ("leer", vec![]), ("ein Byte länger", [blob.clone(), vec![0]].concat()), ("Text des Auslösers", b"Zahlung storniert".to_vec())] {
        let mut pay = Pay::new(o);
        pay.payload = Some(p);
        assert!(pay.run(&e, o)[0].is_err(), "{label}");
    }
}

#[test]
fn ohne_nachricht_bleibt_der_payload_leer() {
    let e = Env::new();
    let o = monthly();
    assert!(ok(&Pay::new(o).run(&e, o)));
    let mut pay = Pay::new(o);
    pay.payload = Some(b"Hallo".to_vec());
    assert!(pay.run(&e, o)[0].is_err(), "Auslöser darf keinen Text anhängen");
}

#[test]
fn laengste_nachricht_passt() {
    // 100 Zeichen à 4 Byte + 64 Byte Verschlüsselung = 464 Byte (txb::MAX_PAYLOAD)
    let blob: Vec<u8> = (0..464u32).map(|i| (i * 7 % 251) as u8).collect();
    let e = Env::with_message(&blob);
    let o = monthly();
    assert!(ok(&Pay::new(o).run(&e, o)));
    let mut pay = Pay::new(o);
    pay.payload = Some(blob[..463].to_vec());
    assert!(pay.run(&e, o)[0].is_err());
}
