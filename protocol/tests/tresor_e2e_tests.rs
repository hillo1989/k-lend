//! Dauerauftrag mit Tresor im Simulator: echte Transaktionen aus
//! src/tresor.rs gegen contracts/standing_order.sil, mit Zeit-Locktime gegen
//! die simulierte Past Median Time (src/sim.rs), Speichermasse und
//! Mindestgebühr. Dazu die Messungen, auf denen MIN_AMOUNT, MIN_KEEP und
//! DEFAULT_MAX_FEE beruhen (Ausgabe mit `-- --nocapture`).

use chrono::NaiveDate;
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint, TransactionOutput, UtxoEntry};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::ops::{Tracked, p2pk_spk, xonly};
use kaspa_lending_protocol::sim::Sim;
use kaspa_lending_protocol::standing::{self, DAY_MS};
use kaspa_lending_protocol::tresor::{self, TresorCode, TresorRec};
use kaspa_lending_protocol::txb::{self, Built, Draft, In, Unlock};
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Secp256k1, SecretKey};

const E8: u64 = 100_000_000;

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

fn ms(y: i32, m: u32, d: u32, h: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, 0, 0).unwrap().and_utc().timestamp_millis()
}

struct World {
    sim: Sim,
    owner: Keypair,
    recipient: Keypair,
    p: TresorParams,
    /// beim Anlegen hinterlegte Nachricht (wie `tresor open`): Beschreibung,
    /// öffentlich?, verschlüsselte Fassung (Hex)
    message: String,
    onchain: bool,
    sealed: String,
}

impl World {
    /// Miete: 10 KAS monatlich am 1., 08:00 UTC, ohne Nachricht
    fn new() -> Self {
        let (owner, recipient) = (key(), key());
        let mut sim = Sim::new();
        sim.faucet(&owner, 1_000 * E8);
        let p = TresorParams {
            owner: xonly(&owner),
            recipient: xonly(&recipient),
            amount: 10 * E8 as i64,
            anchor_day: 1,
            period_ms: 0,
            max_fee: tresor::DEFAULT_MAX_FEE,
            payload_hash: payload_hash(&[]),
        };
        World { sim, owner, recipient, p, message: String::new(), onchain: false, sealed: String::new() }
    }
    /// mit öffentlicher Nachricht (Klartext in jeder Zahlung)
    fn public(text: &str) -> Self {
        let mut w = Self::new();
        (w.message, w.onchain) = (text.into(), true);
        w.p.payload_hash = payload_hash(text.as_bytes());
        w
    }
    /// mit Nachricht, einmal an den Empfänger verschlüsselt (wie `tresor open`)
    fn sealed(text: &str) -> Self {
        let mut w = Self::new();
        let blob = kaspa_lending_protocol::message::encrypt(&w.p.recipient, text).unwrap();
        w.message = text.into();
        w.sealed = faster_hex::hex_string(&blob);
        w.p.payload_hash = payload_hash(&blob);
        w
    }
    /// Tresor-Eintrag, wie ihn `tresor open` speichert
    fn rec(&self, t: &Tracked<TresorState>) -> TresorRec {
        let mut rec = TresorRec::new(self.p.clone(), t.clone(), self.message.clone(), self.onchain, None, "x");
        rec.sealed = self.sealed.clone();
        rec
    }
    fn first(&self, left: i64) -> TresorState {
        TresorState { next_due: ms(2027, 2, 1, 8), left }
    }
    fn open(&mut self, left: i64, fund: u64) -> Tracked<TresorState> {
        let (b, t) = tresor::open(&self.p, &self.first(left), fund, &self.sim.funds(&self.owner), &self.sim.params).expect("anlegen");
        self.sim.submit(&b).expect("Genesis angenommen");
        report("Tresor anlegen", &b);
        t
    }
    fn at(&mut self, t: i64) {
        if t as u64 > self.sim.now_ms {
            self.sim.set_time(t as u64);
        }
    }
    fn pay(&mut self, t: &Tracked<TresorState>, payload: &[u8]) -> Result<(Built, Tracked<TresorState>), String> {
        let r = tresor::pay(&self.p, t, payload, None, &self.sim.params)?;
        self.sim.submit(&r.built)?;
        Ok((r.built, r.next))
    }
}

fn report(label: &str, b: &Built) {
    println!(
        "{label}: Gebühr {} sompi ({:.5} KAS), compute {} g, transient {} g, storage {} g, {} In / {} Out, Payload {} B",
        b.fee,
        b.fee as f64 / 1e8,
        b.compute_mass,
        b.transient_mass,
        b.storage_mass,
        b.tx.inputs.len(),
        b.tx.outputs.len(),
        b.tx.payload.len()
    );
}

/// Tresor-UTXO direkt anlegen (für Messungen mit Werten, die `open` ablehnt)
fn plant(sim: &mut Sim, p: &TresorParams, s: TresorState, value: u64, n: u8) -> Tracked<TresorState> {
    let (mut h, mut c) = ([0xeeu8; 32], [0xccu8; 32]);
    h[0] = n;
    c[0] = n;
    let (op, cov) = (TransactionOutpoint::new(Hash::from_bytes(h), 0), Hash::from_bytes(c));
    sim.utxos.insert(op, UtxoEntry::new(value, spk(&standing_order(p, &s)), sim.daa, false, Some(cov)));
    Tracked { outpoint: op, value, cov, state: s }
}

/// Zahlung von Hand, ohne die Mindestwerte von tresor::pay (nur für Messungen
/// und Angriffe): Gebühr aus dem Tresor, Fortsetzung = Wert − Betrag − fee
fn raw_pay(p: &TresorParams, t: &Tracked<TresorState>, fee: u64, net: &kaspa_consensus_core::config::params::Params) -> Result<Built, String> {
    let art = standing_order(p, &t.state);
    let ns = tresor::next_state(p, &t.state);
    let input = In { outpoint: t.outpoint, entry: UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)), unlock: Unlock::Entry { art, entry: "pay", args: vec![], sig_at: None } };
    let cont = TransactionOutput {
        value: t.value - p.amount as u64 - fee,
        script_public_key: spk(&standing_order(p, &ns)),
        covenant: Some(kaspa_consensus_core::tx::CovenantBinding { authorizing_input: 0, covenant_id: t.cov }),
    };
    let pay = TransactionOutput { value: p.amount as u64, script_public_key: p2pk_spk(&p.recipient), covenant: None };
    txb::build(Draft { inputs: vec![input], outputs: vec![pay, cont], change_spk: p2pk_spk(&p.recipient), lock_time: t.state.next_due as u64 }, net)
}

// ------------------------------------------------------------ Simulator ----

#[test]
fn zeit_locktime_gegen_die_past_median_time() {
    // Konsens: final erst bei lock_time < PMT; DAA-Locktimes wie bisher
    let w = World::new();
    let mut sim = w.sim;
    let f = sim.funds(&w.owner);
    let net = sim.params.clone();
    let mk = |lock_time: u64, seq: u64| {
        let inputs = f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (w.owner).into() } }).collect();
        let mut b = txb::build(Draft { inputs, outputs: vec![TransactionOutput { value: E8, script_public_key: p2pk_spk(&xonly(&w.recipient)), covenant: None }], change_spk: p2pk_spk(&xonly(&w.owner)), lock_time }, &net).unwrap();
        if seq != 0 {
            // sequence gehört nicht zur Signatur-Prüfung des Simulators (nur Locktime)
            b.tx.inputs[0].sequence = seq;
        }
        b
    };
    let now = sim.now_ms;
    assert!(sim.submit(&mk(now, 0)).unwrap_err().contains("Past Median Time"), "lock_time == PMT ist noch nicht final");
    assert!(sim.submit(&mk(now + 60_000, 0)).is_err(), "Zukunft");
    // alle Eingänge final (sequence = u64::MAX): Locktime zählt nicht
    let b = mk(now + 60_000, u64::MAX);
    let r = sim.submit(&b);
    assert!(r.as_ref().err().is_none_or(|e| !e.contains("Locktime")), "sequence MAX hebt die Locktime auf: {r:?}");
    let mut sim2 = Sim::new();
    sim2.faucet(&w.owner, 1_000 * E8);
    let f2 = sim2.funds(&w.owner);
    let inputs = f2.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (w.owner).into() } }).collect();
    let ok = txb::build(Draft { inputs, outputs: vec![TransactionOutput { value: E8, script_public_key: p2pk_spk(&xonly(&w.recipient)), covenant: None }], change_spk: p2pk_spk(&xonly(&w.owner)), lock_time: sim2.now_ms - 1 }, &sim2.params).unwrap();
    sim2.submit(&ok).expect("lock_time < PMT ist final");
    // DAA-Locktime unverändert
    let mut sim3 = Sim::new();
    sim3.faucet(&w.owner, 1_000 * E8);
    let f3 = sim3.funds(&w.owner);
    let inputs = f3.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (w.owner).into() } }).collect();
    let daa = txb::build(Draft { inputs, outputs: vec![TransactionOutput { value: E8, script_public_key: p2pk_spk(&xonly(&w.recipient)), covenant: None }], change_spk: p2pk_spk(&xonly(&w.owner)), lock_time: sim3.daa }, &sim3.params).unwrap();
    assert!(sim3.submit(&daa).unwrap_err().contains("DAA"), "DAA-Locktime == DAA bleibt abgelehnt");
}

#[test]
fn zahlung_zum_termin_nicht_frueher_und_genau_der_betrag() {
    let mut w = World::new();
    let t = w.open(12, 12 * (10 * E8 + tresor::DEFAULT_MAX_FEE as u64) + E8);
    assert_eq!(t.value, 12 * (10 * E8 + 1_000_000) + E8);
    // zu früh: der Simulator steht am 1.1.2027, fällig ist der 1.2.2027 08:00
    let early = tresor::pay(&w.p, &t, &[], None, &w.sim.params).expect("bauen geht, der Konsens entscheidet");
    assert_eq!(early.built.tx.lock_time, t.state.next_due as u64, "lock_time = Termin");
    assert!(early.built.tx.inputs.iter().all(|i| i.sequence != u64::MAX), "CLTV verlangt einen nicht finalen Eingang");
    assert!(w.sim.submit(&early.built).unwrap_err().contains("Past Median Time"));
    w.at(t.state.next_due);
    assert!(w.sim.submit(&early.built).is_err(), "genau am Termin ist die PMT noch nicht darüber");
    w.at(t.state.next_due + 1);
    let before = w.sim.balance(&w.recipient);
    w.sim.submit(&early.built).expect("ab Termin + 1 ms angenommen");
    report("Tresor-Zahlung", &early.built);
    assert_eq!(w.sim.balance(&w.recipient) - before, 10 * E8, "Empfänger bekommt genau den Betrag");
    let n = early.next;
    assert!(early.fee_from_tresor);
    assert!(t.value - n.value <= 10 * E8 + tresor::DEFAULT_MAX_FEE as u64, "höchstens Betrag + Höchstgebühr");
    assert_eq!(t.value - n.value - 10 * E8, early.built.fee, "Differenz = Netzgebühr");
    assert_eq!(n.state, TresorState { next_due: ms(2027, 3, 1, 8), left: 11 });
    assert_eq!(n.outpoint.index, 1, "Fortsetzung an Ausgang 1");
    assert_eq!(early.built.tx.outputs[0].script_public_key, p2pk_spk(&w.p.recipient), "Ausgang 0 an den Empfänger");
    // dieselbe Zahlung zweimal: die zweite scheitert an der verbrauchten UTXO
    assert!(w.sim.submit(&early.built).unwrap_err().contains("nicht vorhanden"));
    // der nächste Termin ist noch nicht erreicht
    assert!(w.pay(&n, &[]).is_err());
}

#[test]
fn rueckstand_zwei_termine_nacheinander_durch_fremden_ohne_kas() {
    let mut w = World::new();
    let t = w.open(-1, 50 * E8);
    let fremder = key();
    assert_eq!(w.sim.balance(&fremder), 0, "Auslöser hat keine KAS");
    // zwei Monate verpasst
    w.at(ms(2027, 3, 15, 0));
    tresor::due_now(&w.p, &t, w.sim.now_ms as i64, false).expect("fällig");
    let (b1, t1) = w.pay(&t, &[]).expect("Februar");
    assert_eq!(b1.tx.inputs.len(), 1, "keine Eingänge des Auslösers");
    assert_eq!(b1.tx.outputs.len(), 2, "kein Wechselgeld");
    let (_, t2) = w.pay(&t1, &[]).expect("März");
    assert_eq!(t2.state, TresorState { next_due: ms(2027, 4, 1, 8), left: -1 }, "unbegrenzt bleibt unbegrenzt");
    assert!(tresor::due_now(&w.p, &t2, w.sim.now_ms as i64, false).is_err(), "April noch nicht");
    assert!(w.pay(&t2, &[]).is_err());
    assert_eq!(w.sim.balance(&w.recipient), 20 * E8);
    assert_eq!(w.sim.balance(&fremder), 0);
}

#[test]
fn letzte_zahlung_dann_ist_schluss_und_nur_der_absender_kuendigt() {
    let mut w = World::new();
    let t = w.open(2, 2 * (10 * E8 + 1_000_000) + E8);
    w.at(ms(2027, 4, 2, 0));
    let (_, t1) = w.pay(&t, &[]).unwrap();
    let (_, t2) = w.pay(&t1, &[]).unwrap();
    assert_eq!(t2.state.left, 0);
    assert!(t2.value >= E8, "Rest bleibt stehen: {}", t2.value);
    assert!(tresor::due_now(&w.p, &t2, w.sim.now_ms as i64, false).is_err());
    assert!(tresor::pay(&w.p, &t2, &[], None, &w.sim.params).is_err(), "Werkzeug lehnt ab");
    // Vertrag lehnt ab, auch von Hand gebaut (Tresor mit left 0 und genug Guthaben)
    w.at(ms(2027, 5, 2, 0));
    let done = plant(&mut w.sim, &w.p, TresorState { next_due: ms(2027, 4, 1, 8), left: 0 }, 50 * E8, 9);
    let e = raw_pay(&w.p, &done, 1_000_000, &w.sim.params).unwrap_err();
    assert!(e.contains("Input 0"), "left 0: Skript scheitert: {e}");
    // Kündigen: Empfänger und Fremde nicht (Werkzeug und Vertrag)
    assert!(tresor::cancel(&w.p, &t2, &w.recipient, &w.sim.params).is_err());
    let art = standing_order(&w.p, &t2.state);
    let forged = txb::build(
        Draft {
            inputs: vec![In { outpoint: t2.outpoint, entry: UtxoEntry::new(t2.value, spk(&art), 0, false, Some(t2.cov)), unlock: Unlock::Entry { art, entry: "cancel", args: vec![], sig_at: Some((0, (w.recipient).into())) } }],
            outputs: vec![TransactionOutput { value: t2.value - 1_000_000, script_public_key: p2pk_spk(&xonly(&w.recipient)), covenant: None }],
            change_spk: p2pk_spk(&xonly(&w.recipient)),
            lock_time: 0,
        },
        &w.sim.params,
    );
    assert!(forged.is_err(), "Signatur des Empfängers reicht nicht");
    let before = w.sim.balance(&w.owner);
    let b = tresor::cancel(&w.p, &t2, &w.owner, &w.sim.params).expect("Absender");
    report("Tresor kündigen", &b);
    w.sim.submit(&b).expect("Kündigung angenommen");
    assert_eq!(w.sim.balance(&w.owner) - before, t2.value - b.fee, "Rest abzüglich Gebühr an den Absender");
    assert!(b.fee < 1_000_000);
    assert!(!w.sim.utxos.contains_key(&t2.outpoint));
}

#[test]
fn auffuellen_nur_der_absender_termine_bleiben() {
    let mut w = World::new();
    let t = w.open(3, 25 * E8);
    let fremder = key();
    w.sim.faucet(&fremder, 100 * E8);
    assert!(tresor::topup(&w.p, &t, &fremder, 5 * E8, &w.sim.funds(&fremder), &w.sim.params).is_err());
    let (b, t1) = tresor::topup(&w.p, &t, &w.owner, 10 * E8, &w.sim.funds(&w.owner), &w.sim.params).expect("auffüllen");
    report("Tresor auffüllen", &b);
    w.sim.submit(&b).expect("angenommen");
    assert_eq!(t1.value, t.value + 10 * E8);
    assert_eq!(t1.state, t.state, "Termine bleiben");
    assert_eq!(t1.cov, t.cov);
    w.at(ms(2027, 2, 1, 9));
    w.pay(&t1, &[]).expect("zahlt nach dem Auffüllen");
}

#[test]
fn gebuehr_vom_ausloeser_wenn_der_tresor_knapp_ist() {
    let mut w = World::new();
    // 10 KAS + 1 KAS + 0,005: nach Betrag und Höchstgebühr blieben < 1 KAS
    let first = w.first(1);
    let t = plant(&mut w.sim, &w.p, first, 11 * E8 + 500_000, 1);
    w.at(ms(2027, 2, 1, 9));
    assert!(tresor::pay(&w.p, &t, &[], None, &w.sim.params).unwrap_err().contains("--key"));
    let helfer = key();
    w.sim.faucet(&helfer, 5 * E8);
    let r = tresor::pay(&w.p, &t, &[], Some(&w.sim.funds(&helfer)), &w.sim.params).expect("mit eigener Gebühr");
    assert!(!r.fee_from_tresor);
    assert_eq!(r.next.value, t.value - 10 * E8, "Tresor verliert nur den Betrag");
    w.sim.submit(&r.built).expect("angenommen");
    report("Tresor-Zahlung, Gebühr vom Auslöser", &r.built);
    assert_eq!(w.sim.balance(&helfer), 5 * E8 - r.built.fee);
}

#[test]
fn nachfuehren_und_import_nach_zahlungen_durch_dritte() {
    let mut w = World::public("Miete Whg. 3");
    let t = w.open(-1, 40 * E8);
    let rec = w.rec(&t);
    let code = TresorCode::of("mainnet", &rec).encode();
    assert!(code.starts_with(tresor::CODE_PREFIX));
    // Dritte zahlen zwei Termine, der Code bleibt beim alten Stand
    w.at(ms(2027, 3, 2, 0));
    let (_, t1) = w.pay(&t, &rec.payload()).unwrap();
    let (_, t2) = w.pay(&t1, &rec.payload()).unwrap();
    let c = TresorCode::decode(&code).expect("Code lesbar");
    assert_eq!(c.cov, t.cov);
    assert_eq!(c.state, t.state);
    assert_eq!(c.message, "Miete Whg. 3");
    let start = Tracked { outpoint: tresor::parse_outpoint(&c.outpoint).unwrap(), value: 0, cov: c.cov, state: c.state };
    let sim = &w.sim;
    let lookup = |s: &ScriptPublicKey| -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        Ok(sim.utxos.iter().filter(|(_, e)| &e.script_public_key == s).map(|(o, e)| (*o, e.clone())).collect())
    };
    let found = tresor::locate(&c.params, &start, sim.now_ms as i64, lookup).unwrap().expect("gefunden");
    assert_eq!(found.outpoint, t2.outpoint);
    assert_eq!(found.value, t2.value);
    assert_eq!(found.state, t2.state);
    // falsche Parameter (anderer Betrag) finden nichts
    let mut bad = c.params.clone();
    bad.amount += 1;
    assert!(tresor::locate(&bad, &start, sim.now_ms as i64, lookup).unwrap().is_none());
    // fremde Covenant-ID bei gleichem Skript zählt nicht
    let other = Tracked { cov: Hash::from_bytes([7; 32]), ..start.clone() };
    assert!(tresor::locate(&c.params, &other, sim.now_ms as i64, lookup).unwrap().is_none());
    // ohne erreichte Termine sucht er keine Nachfolger (weniger Abfragen)
    assert_eq!(tresor::candidates(&c.params, &t2.state, sim.now_ms as i64).len(), 1);
    // nach der Kündigung: nichts mehr auffindbar
    let b = tresor::cancel(&w.p, &t2, &w.owner, &w.sim.params).unwrap();
    w.sim.submit(&b).unwrap();
    let sim = &w.sim;
    let lookup = |s: &ScriptPublicKey| -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        Ok(sim.utxos.iter().filter(|(_, e)| &e.script_public_key == s).map(|(o, e)| (*o, e.clone())).collect())
    };
    assert!(tresor::locate(&c.params, &start, sim.now_ms as i64, lookup).unwrap().is_none());
}

#[test]
fn nachricht_im_payload_jeder_zahlung() {
    let mut w = World::public("Miete Oktober – Whg. 3");
    let t = w.open(-1, 40 * E8);
    let mut rec = w.rec(&t);
    w.at(ms(2027, 2, 1, 9));
    let (b, _) = w.pay(&t, &rec.payload()).expect("mit Nachricht");
    assert_eq!(b.tx.payload, "Miete Oktober – Whg. 3".as_bytes());
    assert!(b.fee <= w.p.max_fee as u64);
    rec.onchain = false;
    assert!(rec.payload().is_empty(), "ohne öffentliche und ohne verschlüsselte Nachricht kein Payload");
}

#[test]
fn verschluesselte_nachricht_in_jeder_zahlung_nur_fuer_den_empfaenger() {
    use kaspa_lending_protocol::message::{self, Found};
    // Anlegen ohne Häkchen: einmal an den Empfänger verschlüsselt
    let mut w = World::sealed("Miete Oktober");
    let t = w.open(-1, 40 * E8);
    let rec = w.rec(&t);
    // der Tresor-Code trägt die verschlüsselte Fassung, der Empfänger übernimmt sie
    let c = TresorCode::decode(&TresorCode::of("sim", &rec).encode()).expect("Code");
    assert_eq!(c.sealed, rec.sealed);
    let mut beschaedigt = c.clone();
    beschaedigt.sealed = "00".repeat(40);
    assert!(TresorCode::decode(&beschaedigt.encode()).is_err(), "kaputte Fassung wird abgelehnt");
    // ein Fremder löst aus: die Zahlung trägt genau diese Fassung
    w.at(ms(2027, 2, 1, 9));
    let (b, t2) = w.pay(&t, &rec.payload()).expect("mit verschlüsselter Nachricht");
    assert!(message::is_encrypted(&b.tx.payload));
    assert!(b.fee <= w.p.max_fee as u64, "Gebühr {} bleibt unter maxFee", b.fee);
    let rk = SecretKey::from_keypair(&w.recipient);
    assert_eq!(message::read(&rk, &b.tx.payload), Some(Found::Private("Miete Oktober".into())));
    let ok = SecretKey::from_keypair(&w.owner);
    assert_eq!(message::read(&ok, &b.tx.payload), Some(Found::Unreadable), "der Absender liest sie nur lokal");
    // nächster Termin: dieselbe Fassung
    w.at(ms(2027, 3, 1, 9));
    let (b2, _) = w.pay(&t2, &rec.payload()).expect("zweite Zahlung");
    assert_eq!(b2.tx.payload, b.tx.payload);
}

#[test]
fn skript_ohne_kompilieren_stimmt() {
    let w = World::new();
    let shape = tresor::TresorShape::of(&w.p);
    for s in [w.first(5), TresorState { next_due: ms(2031, 7, 1, 8), left: -1 }, TresorState { next_due: standing::MAX_TIME, left: 0 }] {
        assert_eq!(shape.spk(&s), spk(&standing_order(&w.p, &s)), "{s:?}");
    }
    let weekly = TresorParams { anchor_day: 0, period_ms: 7 * DAY_MS, ..w.p.clone() };
    let s = TresorState { next_due: ms(2027, 1, 4, 0), left: 3 };
    assert_eq!(tresor::TresorShape::of(&weekly).spk(&s), spk(&standing_order(&weekly, &s)));
    // mit gebundener Nachricht: der Hash steht im Skript, die Form trifft ihn
    // genau, und ein anderer Hash ergibt ein anderes Skript
    for m in [World::public("Miete Whg. 3"), World::sealed("Miete Oktober")] {
        let shape = tresor::TresorShape::of(&m.p);
        let s = m.first(12);
        assert_eq!(shape.spk(&s), spk(&standing_order(&m.p, &s)), "{}", m.message);
        assert_ne!(shape.spk(&s), tresor::TresorShape::of(&TresorParams { payload_hash: w.p.payload_hash.clone(), ..m.p.clone() }).spk(&s));
        // und die Erkennung im Eingang liest ihn zurück
        assert_eq!(tresor::parse_script(&bytecode(&standing_order(&m.p, &s))), Some((m.p.clone(), s)));
    }
}

#[test]
fn anlegen_prueft_die_eingaben() {
    let mut w = World::new();
    let f = w.sim.funds(&w.owner);
    let net = w.sim.params.clone();
    let first = w.first(12);
    let bad = |p: TresorParams, s: TresorState, fund: u64| tresor::open(&p, &s, fund, &f, &net).is_err();
    assert!(bad(TresorParams { amount: E8 as i64 - 1, ..w.p.clone() }, first, 100 * E8), "Betrag < 1 KAS");
    assert!(bad(TresorParams { recipient: w.p.owner.clone(), ..w.p.clone() }, first, 100 * E8), "an sich selbst");
    assert!(bad(TresorParams { recipient: vec![0xff; 32], ..w.p.clone() }, first, 100 * E8), "kein Punkt auf der Kurve");
    assert!(bad(TresorParams { anchor_day: 0, period_ms: 0, ..w.p.clone() }, first, 100 * E8), "Intervall 0");
    assert!(bad(TresorParams { anchor_day: 32, ..w.p.clone() }, first, 100 * E8), "Tag 32");
    assert!(bad(TresorParams { max_fee: 0, ..w.p.clone() }, first, 100 * E8), "Höchstgebühr 0");
    assert!(bad(TresorParams { max_fee: tresor::MAX_MAX_FEE + 1, ..w.p.clone() }, first, 100 * E8), "Höchstgebühr zu groß");
    assert!(bad(TresorParams { payload_hash: vec![0; 31], ..w.p.clone() }, first, 100 * E8), "Hash der Nachricht zu kurz");
    assert!(bad(TresorParams { payload_hash: vec![], ..w.p.clone() }, first, 100 * E8), "ohne Hash der Nachricht");
    assert!(bad(w.p.clone(), TresorState { next_due: ms(2027, 2, 2, 8), left: 12 }, 100 * E8), "erster Termin nicht am Ankertag");
    assert!(bad(w.p.clone(), TresorState { left: 0, ..first }, 100 * E8), "Anzahl 0");
    assert!(bad(w.p.clone(), first, 11 * E8), "Startguthaben < Betrag + Gebühr + Reserve");
    assert_eq!(tresor::suggested_fund(&w.p, 12), Some(12 * (10 * E8 as i64 + 1_000_000) + E8 as i64));
    assert_eq!(tresor::suggested_fund(&w.p, -1), None);
    assert_eq!(tresor::payments_covered(&w.p, 12 * (10 * E8 + 1_000_000) + E8), 12);
    let _ = w.open(12, 11 * E8 + 1_000_000);
}

// ------------------------------------------------------------ Messungen ----

#[test]
fn messung_gebuehr_und_mindestwerte() {
    let w = World::new();
    let mut sim = w.sim;
    sim.set_time(ms(2027, 2, 2, 0) as u64);
    let first = TresorState { next_due: ms(2027, 2, 1, 8), left: -1 };

    // Gebühr einer Zahlung ohne und mit 100 Zeichen Nachricht (4 Byte je Zeichen: 400 Byte)
    // (die Nachricht ist im Vertrag gebunden: je Nachricht ein eigener Tresor)
    let bound = |text: &str| TresorParams { payload_hash: payload_hash(text.as_bytes()), ..w.p.clone() };
    let t = plant(&mut sim, &w.p, first, 50 * E8, 1);
    let plain = tresor::pay(&w.p, &t, &[], None, &sim.params).unwrap().built;
    let long = "€".repeat(100);
    let (p3, p4) = (bound(&long), bound(&"𝄞".repeat(100)));
    let t3 = plant(&mut sim, &p3, first, 50 * E8, 250);
    let with = tresor::pay(&p3, &t3, long.as_bytes(), None, &sim.params).unwrap().built;
    report("Messung Zahlung ohne Nachricht", &plain);
    report("Messung Zahlung mit 300-Byte-Nachricht", &with);
    let max = "𝄞".repeat(100);
    let t4 = plant(&mut sim, &p4, first, 50 * E8, 251);
    let with4 = tresor::pay(&p4, &t4, max.as_bytes(), None, &sim.params).unwrap().built;
    report("Messung Zahlung mit 400-Byte-Nachricht", &with4);
    // gemessen 28.–29.09.2026: 0,0025 KAS ohne, 0,0033 KAS mit 400 Byte – Faktor 3 Luft
    assert!(with4.fee <= tresor::DEFAULT_MAX_FEE as u64 / 2, "Gebühr {} unter der Hälfte von 0,01 KAS", with4.fee);
    sim.submit(&with4).expect("mit längster Nachricht angenommen");

    // Kleinster Betrag, den der Simulator annimmt (Fortsetzung groß)
    let mut lo = 1u64;
    let mut hi = E8;
    let mut n = 2u8;
    while hi - lo > E8 / 1000 {
        let mid = (lo + hi) / 2;
        let p = TresorParams { amount: mid as i64, ..w.p.clone() };
        let t = plant(&mut sim, &p, first, 100 * E8, n);
        n = n.wrapping_add(1);
        let ok = raw_pay(&p, &t, 1_000_000, &sim.params).and_then(|b| sim.submit(&b)).is_ok();
        if ok { hi = mid } else { lo = mid }
    }
    println!("Messung: kleinster angenommener Betrag ≈ {:.4} KAS (Fortsetzung 100 KAS)", hi as f64 / 1e8);
    let min_amount = hi;

    // Kleinste Fortsetzung bei 1 KAS Betrag
    let mut lo = 1u64;
    let mut hi = E8;
    while hi - lo > E8 / 1000 {
        let mid = (lo + hi) / 2;
        let p = TresorParams { amount: E8 as i64, ..w.p.clone() };
        let t = plant(&mut sim, &p, first, E8 + 1_000_000 + mid, n);
        n = n.wrapping_add(1);
        let ok = raw_pay(&p, &t, 1_000_000, &sim.params).and_then(|b| sim.submit(&b)).is_ok();
        if ok { hi = mid } else { lo = mid }
    }
    println!("Messung: kleinste angenommene Fortsetzung ≈ {:.4} KAS (Betrag 1 KAS)", hi as f64 / 1e8);
    let min_keep = hi;

    // Speichermasse bei den gewählten Mindestwerten: 1 KAS Betrag, 1 KAS Fortsetzung
    let p = TresorParams { amount: tresor::MIN_AMOUNT, ..w.p.clone() };
    let t = plant(&mut sim, &p, first, (tresor::MIN_AMOUNT + tresor::MIN_KEEP + p.max_fee) as u64, n);
    let b = tresor::pay(&p, &t, &[], None, &sim.params).expect("Mindestwerte");
    report("Messung Zahlung 1 KAS / Fortsetzung 1 KAS", &b.built);
    sim.submit(&b.built).expect("angenommen");
    assert!(b.built.storage_mass <= txb::MAX_BLOCK_STORAGE_MASS / 5, "deutlicher Abstand zum Blocklimit");
    assert!((min_amount as i64) < tresor::MIN_AMOUNT && (min_keep as i64) < tresor::MIN_KEEP, "Mindestwerte mit Sicherheitsabstand");
}

// ------------------------------------------- Audit 12 (Nachprüfung, Opus) ----

/// Tx im Format der REST-API (wie message.rs erwartet): Signaturskripte und
/// Adressen der ausgegebenen Outpoints aus dem gebauten Tx
fn rest_of(b: &Built) -> serde_json::Value {
    let prefix = kaspa_addresses::Prefix::Mainnet;
    let addr = |s: &ScriptPublicKey| kaspa_txscript::standard::extract_script_pub_key_address(s, prefix).unwrap().to_string();
    serde_json::json!({
        "transaction_id": b.tx.id().to_string(),
        "block_time": 1u64,
        "is_accepted": true,
        "payload": faster_hex::hex_string(&b.tx.payload),
        "inputs": b.tx.inputs.iter().zip(&b.entries).map(|(i, e)| serde_json::json!({
            "previous_outpoint_address": addr(&e.script_public_key),
            "signature_script": faster_hex::hex_string(&i.signature_script),
        })).collect::<Vec<_>>(),
        "outputs": b.tx.outputs.iter().map(|o| serde_json::json!({
            "amount": o.value,
            "script_public_key": faster_hex::hex_string(o.script_public_key.script()),
            "script_public_key_address": addr(&o.script_public_key),
        })).collect::<Vec<_>>(),
    })
}

fn address(x: &[u8]) -> String {
    kaspa_addresses::Address::new(kaspa_addresses::Prefix::Mainnet, kaspa_addresses::Version::PubKey, x).to_string()
}

/// Zahlung von Hand wie ein fremder Auslöser: Gebühr aus dem Tresor (höchstens
/// maxFee, der Rest geht an `change`), eigene Eingänge aus `extra`, beliebiger
/// Payload. Err, wenn schon das Skript beim Bauen scheitert (txb prüft jeden
/// Eingang mit der Skript-Engine, wie der Konsens)
fn foreign_pay(w: &World, t: &Tracked<TresorState>, extra: Option<Keypair>, change: &[u8], payload: &[u8]) -> Result<Built, String> {
    let art = standing_order(&w.p, &t.state);
    let ns = tresor::next_state(&w.p, &t.state);
    let mut inputs = vec![In {
        outpoint: t.outpoint,
        entry: UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Entry { art, entry: "pay", args: vec![], sig_at: None },
    }];
    if let Some(k) = extra {
        let f = w.sim.funds(&k);
        inputs.extend(f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (k).into() } }));
    }
    let cont = TransactionOutput {
        value: t.value - w.p.amount as u64 - w.p.max_fee as u64,
        script_public_key: spk(&standing_order(&w.p, &ns)),
        covenant: Some(kaspa_consensus_core::tx::CovenantBinding { authorizing_input: 0, covenant_id: t.cov }),
    };
    let pay = TransactionOutput { value: w.p.amount as u64, script_public_key: p2pk_spk(&w.p.recipient), covenant: None };
    txb::build_with_payload(Draft { inputs, outputs: vec![pay, cont], change_spk: p2pk_spk(change), lock_time: t.state.next_due as u64 }, payload, &w.sim.params)
}

/// Lehnt der Vertrag diese Zahlung ab (beim Bauen oder im Simulator)?
fn rejected(w: &mut World, b: Result<Built, String>) -> Option<String> {
    match b {
        Err(e) => Some(e),
        Ok(b) => w.sim.submit(&b).err(),
    }
}

/// Dieselbe Tx mit anderem Payload im Format der REST-API – so etwas kann nur
/// eine falsche Antwort der REST-API sein, im Netz lehnt der Vertrag sie ab
fn rest_with_payload(b: &Built, payload: &[u8]) -> serde_json::Value {
    let mut r = rest_of(b);
    r["payload"] = faster_hex::hex_string(payload).into();
    r
}

/// A12-1 im Vertrag (vorher: „untergeschobene Nachricht im Eingang erkannt“):
/// Ein fremder Auslöser konnte die Nachricht einer Tresor-Zahlung ersetzen;
/// der Eingang warnte nur bei übernommenen Tresoren. Jetzt bindet der Vertrag
/// den Payload (payloadHash): ghostctl baut keine solche Zahlung, der Konsens
/// (Simulator) lehnt eine von Hand gebaute ab, und der Eingang erkennt die
/// gebundene Nachricht auch ohne übernommenen Tresor-Code.
#[test]
fn a12_untergeschobene_nachricht_im_eingang_erkannt() {
    use kaspa_lending_protocol::message::{self, Found, Origin, TresorCheck};
    let mut w = World::sealed("Miete Februar");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    let forged = message::encrypt(&xonly(&w.recipient), "Miete ab Maerz bitte an neue Adresse melden").unwrap();
    let sk = SecretKey::from_keypair(&w.recipient);
    let me = address(&w.p.recipient);
    w.at(t.state.next_due + 10 * 60_000);
    let (b1, t2) = w.pay(&t, &rec.payload()).expect("ehrlich, wie ghostctl");
    w.at(t2.state.next_due + 10 * 60_000);
    // fremde Nachricht: ghostctl lehnt ab, der Vertrag ebenso
    assert!(tresor::pay(&w.p, &t2, &forged, None, &w.sim.params).unwrap_err().contains("beim Anlegen hinterlegte Nachricht"));
    let b2 = foreign_pay(&w, &t2, None, &w.p.recipient, &forged);
    assert!(rejected(&mut w, b2).is_some(), "Vertrag lehnt die fremde Nachricht ab");
    for (label, payload) in [("leer", vec![]), ("ein Byte mehr", [rec.payload(), vec![0]].concat())] {
        let b = foreign_pay(&w, &t2, None, &w.p.recipient, &payload);
        assert!(rejected(&mut w, b).is_some(), "{label}");
    }
    // Gegenprobe: von Hand mit der hinterlegten Fassung geht es
    let b = foreign_pay(&w, &t2, None, &w.p.recipient, &rec.payload());
    assert_eq!(rejected(&mut w, b), None, "hinterlegte Nachricht angenommen");
    let known = [rec.clone()];
    let e1 = message::inbox_entry_with(&rest_of(&b1), &me, &sk, None, &known).unwrap();
    // meldet die REST-API die Zahlung mit anderem Payload, passt er nicht zum Vertrag
    let e2 = message::inbox_entry_with(&rest_with_payload(&b1, &forged), &me, &sk, None, &known).unwrap();
    println!("echt: {:?} {:?}\nfalsche REST-Daten: {:?} {:?}", e1.found, e1.origin, e2.found, e2.origin);
    let owner: [u8; 32] = w.p.owner.clone().try_into().unwrap();
    assert_eq!(e1.found, Found::Private("Miete Februar".into()));
    assert_eq!(e1.origin, Origin::Tresor { id: Some(rec.id.clone()), owner: Some(owner), check: TresorCheck::AsStored });
    assert_eq!(e2.origin, Origin::Tresor { id: Some(rec.id.clone()), owner: Some(owner), check: TresorCheck::Inserted });
    assert_eq!((e1.amount, e1.unit), (10 * E8, "KAS"));
    // „von“ = Schlüssel-Adressen unter den Eingängen: hier keine (Gebühr aus dem
    // Tresor); der Besitzer steht nur in der Herkunft, und nur bei übernommenem Tresor
    assert!(e1.from.is_empty(), "{:?}", e1.from);
    // Tresor auf diesem Rechner unbekannt: erkannt, Nachricht vom Vertrag
    // erzwungen (beim Anlegen hinterlegt), Besitzer nicht genannt
    let e3 = message::inbox_entry(&rest_of(&b1), &me, &sk, None).unwrap();
    assert_eq!(e3.found, Found::Private("Miete Februar".into()));
    assert_eq!(e3.origin, Origin::Tresor { id: None, owner: None, check: TresorCheck::Bound });
    let e4 = message::inbox_entry(&rest_with_payload(&b1, &forged), &me, &sk, None).unwrap();
    assert_eq!(e4.origin, Origin::Tresor { id: None, owner: None, check: TresorCheck::Inserted });
    // die hinterlegte Fassung wiederholt (Replay) in einer normalen Sendung:
    // kein Tresor-Eingang, also auch kein „wie hinterlegt“
    let dritter = key();
    w.sim.faucet(&dritter, 5 * E8);
    let f = w.sim.funds(&dritter);
    let inputs = f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (dritter).into() } }).collect();
    let replay = txb::build_with_payload(
        Draft {
            inputs,
            outputs: vec![TransactionOutput { value: E8, script_public_key: p2pk_spk(&w.p.recipient), covenant: None }],
            change_spk: p2pk_spk(&xonly(&dritter)),
            lock_time: 0,
        },
        &rec.payload(),
        &w.sim.params,
    )
    .unwrap();
    let e5 = message::inbox_entry_with(&rest_of(&replay), &me, &sk, None, &known).unwrap();
    assert_eq!(e5.origin, Origin::Direct);
    assert_eq!(e5.from, vec![address(&xonly(&dritter))]);
}

/// A12-1 im Vertrag, A12-8 (übernommen): Ein beliebiger Auslöser zahlt den
/// fälligen Termin und behält den Rest der Höchstgebühr (das bindet der
/// Vertrag weiterhin nicht). Eine eigene Nachricht kann er nicht mehr
/// einsetzen: Der Vertrag lehnt jeden anderen Payload ab (vorher nahm der
/// Konsens die Zahlung mit fremdem Payload an, und nur der Eingang warnte).
/// „von“ zeigt die Eingänge (hier den Auslöser), den Besitzer nur als Angabe
/// des übernommenen Tresor-Codes.
#[test]
fn a12_fremder_ausloeser_kann_keine_eigene_nachricht_setzen_behaelt_aber_den_gebuehrenrest() {
    use kaspa_lending_protocol::message::{self, Found, Origin, TresorCheck};
    let mut w = World::sealed("Miete Februar");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    w.at(t.state.next_due + 10 * 60_000);
    let attacker = key();
    w.sim.faucet(&attacker, 5 * E8);
    let before = w.sim.balance(&attacker);
    let text = "Kontowechsel: bitte ab sofort an kaspa:qz0000 zurueckzahlen";
    let forged = message::encrypt(&xonly(&w.recipient), text).unwrap();
    let b = foreign_pay(&w, &t, Some(attacker), &xonly(&attacker), &forged);
    let e = rejected(&mut w, b).expect("Vertrag lehnt die fremde Nachricht ab");
    println!("fremde Nachricht: {e}");
    assert_eq!(w.sim.balance(&w.recipient), 0, "nichts gezahlt");
    // mit der hinterlegten Nachricht zahlt er – und behält den Gebührenrest
    let b = foreign_pay(&w, &t, Some(attacker), &xonly(&attacker), &rec.payload()).expect("baut");
    w.sim.submit(&b).expect("mit der hinterlegten Nachricht angenommen");
    let gain = w.sim.balance(&attacker) as i64 - before as i64;
    println!("Auslöser: Netzgebühr {} sompi, Gewinn {gain} sompi ({:.5} KAS) aus maxFee {}", b.fee, gain as f64 / 1e8, w.p.max_fee);
    assert!(gain > 0 && gain <= w.p.max_fee, "Auslöser behält höchstens maxFee − Netzgebühr");
    assert_eq!(w.sim.balance(&w.recipient), 10 * E8, "Empfänger bekam den Termin");
    let sk = SecretKey::from_keypair(&w.recipient);
    let e = message::inbox_entry_with(&rest_of(&b), &address(&w.p.recipient), &sk, None, &[rec]).unwrap();
    assert_eq!(e.found, Found::Private("Miete Februar".into()));
    assert!(matches!(e.origin, Origin::Tresor { check: TresorCheck::AsStored, .. }), "{:?}", e.origin);
    assert_eq!(e.from, vec![address(&xonly(&attacker))], "„von“ = Eingänge der Tx, hier der Auslöser");
    assert_eq!(e.amount, 10 * E8);
}

/// A12-1 im Vertrag: Tresor ohne Nachricht – jede Zahlung hat einen leeren
/// Payload, im Eingang erscheint also kein Text; ein Auslöser kann keinen
/// anhängen.
#[test]
fn a12_ohne_nachricht_bleibt_jede_zahlung_ohne_text() {
    use kaspa_lending_protocol::message;
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    assert!(rec.payload().is_empty());
    assert_eq!(w.p.payload_hash, payload_hash(&[]));
    w.at(t.state.next_due + 10 * 60_000);
    assert!(tresor::pay(&w.p, &t, b"Hallo", None, &w.sim.params).is_err(), "ghostctl hängt nichts an");
    let b = foreign_pay(&w, &t, None, &w.p.recipient, b"Hallo");
    assert!(rejected(&mut w, b).is_some(), "Vertrag lehnt Text ab");
    let (b, _) = w.pay(&t, &rec.payload()).expect("ohne Payload");
    assert!(b.tx.payload.is_empty());
    let sk = SecretKey::from_keypair(&w.recipient);
    assert_eq!(message::inbox_entry(&rest_of(&b), &address(&w.p.recipient), &sk, None), None, "kein Text, kein Eintrag im Eingang");
    assert_eq!(w.sim.balance(&w.recipient), 10 * E8);
}

/// A12-10: Trägt der Tresor die Gebühr nicht mehr und zahlt sie der Empfänger
/// mit dem eigenen Schlüssel, hat die Tx einen Eingang von ihm. Vorher galt sie
/// darum als eigene Sendung und die Tresor-Nachricht verschwand aus dem Eingang.
#[test]
fn a12_gebuehr_vom_eigenen_schluessel_nachricht_bleibt_im_eingang() {
    use kaspa_lending_protocol::message::{self, Found, Origin, TresorCheck};
    let mut w = World::sealed("Miete März");
    let first = w.first(2);
    let t = plant(&mut w.sim, &w.p, first, 11 * E8 + 500_000, 3);
    let rec = w.rec(&t);
    w.at(ms(2027, 2, 1, 9));
    assert!(tresor::due_now(&w.p, &t, w.sim.now_ms as i64, false).is_err(), "Tresor allein trägt die Gebühr nicht");
    tresor::due_now(&w.p, &t, w.sim.now_ms as i64, true).expect("mit eigener Gebühr");
    w.sim.faucet(&w.recipient, 3 * E8);
    let r = tresor::pay(&w.p, &t, &rec.payload(), Some(&w.sim.funds(&w.recipient)), &w.sim.params).expect("mit eigener Gebühr");
    assert!(!r.fee_from_tresor);
    w.sim.submit(&r.built).expect("angenommen");
    let me = address(&w.p.recipient);
    let rest = rest_of(&r.built);
    assert!(rest["inputs"].as_array().unwrap().iter().any(|i| i["previous_outpoint_address"] == me.as_str()), "Eingang vom Empfänger");
    let sk = SecretKey::from_keypair(&w.recipient);
    let e = message::inbox_entry_with(&rest, &me, &sk, None, std::slice::from_ref(&rec)).expect("bleibt im Eingang");
    assert_eq!(e.found, Found::Private("Miete März".into()));
    assert!(matches!(e.origin, Origin::Tresor { check: TresorCheck::AsStored, .. }));
    assert_eq!(e.amount, 10 * E8, "Betrag laut Vertrag, ohne das eigene Wechselgeld");
    assert!(e.from.is_empty(), "die eigene Adresse steht nicht unter „von“");
    // eine gewöhnliche eigene Sendung bleibt draußen
    let f = w.sim.funds(&w.recipient);
    let inputs = f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (w.recipient).into() } }).collect();
    let own = txb::build_with_payload(
        Draft {
            inputs,
            outputs: vec![TransactionOutput { value: E8, script_public_key: p2pk_spk(&w.p.owner), covenant: None }],
            change_spk: p2pk_spk(&w.p.recipient),
            lock_time: 0,
        },
        &rec.payload(),
        &w.sim.params,
    )
    .unwrap();
    assert_eq!(message::inbox_entry_with(&rest_of(&own), &me, &sk, None, &[]), None);
}

/// A12-1: Öffentliche Tresor-Nachricht: gleicher Klartext = wie hinterlegt;
/// ein anderer Klartext kommt nicht mehr durch den Vertrag (nur falsche
/// REST-Daten könnten ihn zeigen: „passt nicht zum Vertrag“)
#[test]
fn a12_oeffentliche_tresor_nachricht_geprueft() {
    use kaspa_lending_protocol::message::{self, Found, Origin, TresorCheck};
    let mut w = World::public("Miete Whg. 3");
    let t = w.open(-1, 40 * E8);
    let rec = w.rec(&t);
    w.at(ms(2027, 3, 2, 0));
    let (b1, t1) = w.pay(&t, &rec.payload()).unwrap();
    let other = b"Miete Whg. 3 - neue IBAN folgt";
    assert!(w.pay(&t1, other).is_err(), "ghostctl setzt nur die gebundene Nachricht");
    let b = foreign_pay(&w, &t1, None, &w.p.recipient, other);
    assert!(rejected(&mut w, b).is_some(), "Vertrag lehnt den anderen Klartext ab");
    let sk = SecretKey::from_keypair(&w.recipient);
    let me = address(&w.p.recipient);
    let e1 = message::inbox_entry_with(&rest_of(&b1), &me, &sk, None, std::slice::from_ref(&rec)).unwrap();
    let e2 = message::inbox_entry_with(&rest_with_payload(&b1, other), &me, &sk, None, std::slice::from_ref(&rec)).unwrap();
    assert_eq!(e1.found, Found::Public("Miete Whg. 3".into()));
    assert!(matches!(e1.origin, Origin::Tresor { check: TresorCheck::AsStored, .. }));
    assert!(matches!(e2.origin, Origin::Tresor { check: TresorCheck::Inserted, .. }));
}

// -------------------------------------- Audit 12, Nachprüfung (Gruppe a) ----

/// Kündigung bzw. Auffüllen durch den Besitzer mit einem zusätzlichen Ausgang
/// an den Empfänger und der Nachricht im Payload (beides erlaubt der Vertrag)
fn owner_tx(p: &TresorParams, t: &Tracked<TresorState>, owner: Keypair, entry: &'static str, to_recipient: u64, payload: &[u8], sim: &Sim) -> Built {
    let art = standing_order(p, &t.state);
    let mut inputs = vec![In { outpoint: t.outpoint, entry: UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)), unlock: Unlock::Entry { art, entry, args: vec![], sig_at: Some((0, (owner).into())) } }];
    let mut outputs = vec![];
    if entry == "topUp" {
        let f = sim.funds(&owner);
        inputs.extend(f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (owner).into() } }));
        outputs.push(TransactionOutput {
            value: t.value + E8,
            script_public_key: spk(&standing_order(p, &t.state)),
            covenant: Some(kaspa_consensus_core::tx::CovenantBinding { authorizing_input: 0, covenant_id: t.cov }),
        });
    }
    outputs.push(TransactionOutput { value: to_recipient, script_public_key: p2pk_spk(&p.recipient), covenant: None });
    txb::build_with_payload(Draft { inputs, outputs, change_spk: p2pk_spk(&p.owner), lock_time: 0 }, payload, &sim.params).expect("baut")
}

/// Nachprüfung A12-1 (Beleg `pruef_kuendigung_mit_kleinem_ausgang_zeigt_vollen_betrag`):
/// Der Besitzer kündigt und schickt dem Empfänger dabei 0,5 KAS mit der
/// hinterlegten Nachricht. Vorher zeigte der Eingang 100 KAS „wie im Tresor
/// hinterlegt“ – jede Tx mit Tresor-Eingang galt als Zahlung laut Vertrag.
/// Ebenso beim Auffüllen. Jetzt: tatsächlicher Betrag, „über einen Vertrag“.
#[test]
fn a12n_kuendigen_und_auffuellen_sind_keine_tresor_zahlung() {
    use kaspa_lending_protocol::message::{self, Origin};
    for entry in ["cancel", "topUp"] {
        let mut w = World::sealed("Miete Februar");
        w.p.amount = 100 * E8 as i64;
        let t = w.open(3, 400 * E8);
        let rec = w.rec(&t);
        let b = owner_tx(&w.p, &t, w.owner, entry, E8 / 2, &rec.payload(), &w.sim);
        w.sim.submit(&b).unwrap_or_else(|e| panic!("{entry} angenommen: {e}"));
        let got: u64 = b.tx.outputs.iter().filter(|o| o.script_public_key == p2pk_spk(&w.p.recipient)).map(|o| o.value).sum();
        let sk = SecretKey::from_keypair(&w.recipient);
        let e = message::inbox_entry_with(&rest_of(&b), &address(&w.p.recipient), &sk, None, std::slice::from_ref(&rec)).expect("im Eingang");
        println!("{entry}: an den Empfänger {got} sompi, Eingang zeigt {} {} {:?}", e.amount, e.unit, e.origin);
        assert_eq!(got, E8 / 2);
        assert_eq!(e.amount, got, "{entry}: tatsächlicher Betrag, nicht der Vertragsbetrag");
        assert_eq!(e.origin, Origin::Contract, "{entry}: keine Tresor-Zahlung, Herkunft der Nachricht nicht prüfbar");
    }
    // auch mit genau dem Vertragsbetrag an Ausgang 0: der Zweig ist cancel, nicht pay
    let mut w = World::sealed("Miete Februar");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    let b = owner_tx(&w.p, &t, w.owner, "cancel", w.p.amount as u64, &rec.payload(), &w.sim);
    w.sim.submit(&b).expect("Kündigung angenommen");
    let e = message::inbox_entry_with(&rest_of(&b), &address(&w.p.recipient), &SecretKey::from_keypair(&w.recipient), None, &[rec]).unwrap();
    assert_eq!(e.origin, Origin::Contract, "Selektor von cancel");
}

/// Nachprüfung A12-1 (Beleg `pruef_ueberzahlung_per_kuendigung`): Überzahlungs-
/// Masche. Der Angreifer legt als Besitzer einen Tresor über 10 000 KAS je
/// Zahlung an (nur 2 KAS darin), kündigt und schickt dem Opfer 0,3 KAS mit
/// „Versehentlich 10000 KAS … bitte zurück“. Vorher zeigte der Eingang 10 000 KAS.
#[test]
fn a12n_ueberzahlungs_masche_per_kuendigung() {
    use kaspa_lending_protocol::message::{self, Found, Origin};
    let (attacker, victim) = (key(), key());
    let mut sim = Sim::new();
    let p = TresorParams {
        owner: xonly(&attacker),
        recipient: xonly(&victim),
        amount: 10_000 * E8 as i64,
        anchor_day: 1,
        period_ms: 0,
        max_fee: tresor::DEFAULT_MAX_FEE,
        payload_hash: payload_hash(&[]),
    };
    let t = plant(&mut sim, &p, TresorState { next_due: ms(2027, 2, 1, 0), left: 1 }, 2 * E8, 7);
    let text = "Versehentlich 10000 KAS an dich - bitte an kaspa:qz0000 zurueck";
    let payload = message::encrypt(&xonly(&victim), text).unwrap();
    let b = owner_tx(&p, &t, attacker, "cancel", 30_000_000, &payload, &sim);
    sim.submit(&b).expect("Kündigung angenommen");
    let sk = SecretKey::from_keypair(&victim);
    let e = message::inbox_entry_with(&rest_of(&b), &address(&p.recipient), &sk, None, &[]).unwrap();
    println!("Opfer bekam 0.3 KAS, Eingang zeigt {} sompi, {:?}", e.amount, e.origin);
    assert_eq!(e.amount, 30_000_000);
    assert_eq!(e.origin, Origin::Contract);
    assert_eq!(e.found, Found::Private(text.into()));
}

/// Nachprüfung A12-1: Tresor-Zahlung nur mit dem Ausgang an mich am Index des
/// Tresors, mit genau dem Betrag. Eine REST-Antwort, die den Ausgang anders
/// nennt, ergibt keine Tresor-Zahlung (sonst stünde ein Betrag da, den laut
/// derselben Antwort niemand an mich gezahlt hat).
#[test]
fn a12n_tresor_zahlung_nur_mit_ausgang_an_mich() {
    use kaspa_lending_protocol::message::{self, Origin};
    let mut w = World::sealed("Miete Februar");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    w.at(t.state.next_due + 10 * 60_000);
    let (b, _) = w.pay(&t, &rec.payload()).unwrap();
    let (sk, me) = (SecretKey::from_keypair(&w.recipient), address(&w.p.recipient));
    let known = std::slice::from_ref(&rec);
    assert!(matches!(message::inbox_entry_with(&rest_of(&b), &me, &sk, None, known).unwrap().origin, Origin::Tresor { .. }));
    // Ausgang 0 laut REST an jemand anderen
    let other = key();
    let mut r = rest_of(&b);
    r["outputs"][0]["script_public_key"] = faster_hex::hex_string(p2pk_spk(&xonly(&other)).script()).into();
    r["outputs"][0]["script_public_key_address"] = address(&xonly(&other)).into();
    assert_eq!(message::inbox_entry_with(&r, &me, &sk, None, known), None, "nichts an mich");
    // Betrag laut REST anders: kein Vertragsbetrag, sondern die Angabe selbst (der Node-Abgleich blendet sie aus)
    let mut r = rest_of(&b);
    r["outputs"][0]["amount"] = (20 * E8).into();
    let e = message::inbox_entry_with(&r, &me, &sk, None, known).unwrap();
    assert_eq!((e.amount, e.origin), (20 * E8, Origin::Contract));
    // Ausgänge mit Feld `index` (wie api.kaspa.org): über das Feld, nicht die Stelle
    let mut r = rest_of(&b);
    let outs = r["outputs"].as_array_mut().unwrap();
    for (i, o) in outs.iter_mut().enumerate() {
        o["index"] = i.into();
    }
    outs.reverse();
    assert!(matches!(message::inbox_entry_with(&r, &me, &sk, None, known).unwrap().origin, Origin::Tresor { .. }));
}

/// Nachprüfung A12-1 (Beleg `pruef_fremder_besitzer_erscheint_als_absender`):
/// Ein Dritter legt einen Tresor mit Besitzer = Alice an (Alice signiert dabei
/// nichts) und hinterlegt beim Anlegen seine eigene Nachricht. Vorher stand
/// Alice als „von“ da, die Seite schrieb „Tresor von <Alice>“. Jetzt: Besitzer
/// nicht genannt; die Nachricht ist die beim Anlegen gebundene – von dem, der
/// angelegt hat, nicht von Alice.
#[test]
fn a12n_fremder_besitzer_nicht_als_absender() {
    use kaspa_lending_protocol::message::{self, Origin, TresorCheck};
    let (alice, recipient, attacker) = (key(), key(), key());
    let mut sim = Sim::new();
    sim.faucet(&attacker, 1_000 * E8);
    let forged = message::encrypt(&xonly(&recipient), "Hier Alice: neue Adresse fuer die Miete ist kaspa:qz0000").unwrap();
    let p = TresorParams {
        owner: xonly(&alice),
        recipient: xonly(&recipient),
        amount: E8 as i64,
        anchor_day: 0,
        period_ms: DAY_MS,
        max_fee: tresor::DEFAULT_MAX_FEE,
        payload_hash: payload_hash(&forged),
    };
    let (b, t) = tresor::open(&p, &TresorState { next_due: ms(2027, 2, 1, 0), left: 1 }, 3 * E8, &sim.funds(&attacker), &sim.params).expect("anlegen mit fremdem Besitzer");
    sim.submit(&b).unwrap();
    sim.set_time(ms(2027, 2, 1, 1) as u64);
    let r = tresor::pay(&p, &t, &forged, None, &sim.params).unwrap();
    sim.submit(&r.built).unwrap();
    let sk = SecretKey::from_keypair(&recipient);
    let e = message::inbox_entry_with(&rest_of(&r.built), &address(&p.recipient), &sk, None, &[]).unwrap();
    println!("von {:?}, Herkunft {:?}", e.from, e.origin);
    assert_eq!(e.origin, Origin::Tresor { id: None, owner: None, check: TresorCheck::Bound });
    assert!(!e.from.contains(&address(&xonly(&alice))), "Alice hat nie signiert");
    // übernimmt der Empfänger den Code, nennt der Eingang den Besitzer – als
    // Angabe dieses Codes (die Seite schreibt „laut Tresor-Code“)
    let mut rec = TresorRec::new(p.clone(), t, String::new(), false, None, "x");
    rec.sealed = faster_hex::hex_string(&forged);
    let e = message::inbox_entry_with(&rest_of(&r.built), &address(&p.recipient), &sk, None, &[rec]).unwrap();
    assert!(matches!(e.origin, Origin::Tresor { owner: Some(o), .. } if o.as_slice() == xonly(&alice).as_slice()));
}

/// Nachprüfung A12-1 (klein): „wie hinterlegt“ verglich nur mit `sealed` aus dem
/// Tresor-Code. Ein Code, dessen `sealed` etwas anderes enthält als die
/// angezeigte Beschreibung `message`, machte die Nachricht zu „wie im
/// Tresor-Code“. Jetzt muss sie für diesen Schlüssel auch den Text der Liste
/// ergeben; sonst bleibt nur „beim Anlegen hinterlegt“ (vom Vertrag erzwungen,
/// die Beschreibung im Code stimmt aber nicht).
#[test]
fn a12n_hinterlegt_heisst_auch_derselbe_text() {
    use kaspa_lending_protocol::message::{self, Origin, TresorCheck};
    let mut w = World::sealed("Neue Adresse: kaspa:qz0000");
    let t = w.open(3, 100 * E8);
    let mut rec = w.rec(&t);
    rec.message = "Miete Februar".into();
    // der Code passt zum Vertrag (Hash der verschlüsselten Fassung), nur die Beschreibung lügt
    assert!(TresorCode::decode(&TresorCode::of("mainnet", &rec).encode()).is_ok());
    w.at(t.state.next_due + 10 * 60_000);
    let (b, _) = w.pay(&t, &rec.payload()).unwrap();
    let e = message::inbox_entry_with(&rest_of(&b), &address(&w.p.recipient), &SecretKey::from_keypair(&w.recipient), None, &[rec]).unwrap();
    assert!(matches!(e.origin, Origin::Tresor { check: TresorCheck::Bound, .. }), "{:?}", e.origin);
}

/// Ein- und Ausgabe der Tresor-Abläufe gegen den Simulator – wie ghostctl gegen den Node
struct SimIo<'a> {
    sim: &'a mut Sim,
    key: Option<Keypair>,
    lookups: usize,
    /// so viele Sendungen scheitern zuerst (ein anderer Auslöser war schneller)
    fail_sends: usize,
    journal_open: bool,
    sent: Vec<String>,
}

impl tresor::TresorIo for SimIo<'_> {
    async fn utxos(&mut self, s: &ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        self.lookups += 1;
        Ok(self.sim.utxos.iter().filter(|(_, e)| e.script_public_key == *s).map(|(o, e)| (*o, e.clone())).collect())
    }
    async fn funds(&mut self) -> Option<kaspa_lending_protocol::ops::Funds> {
        self.key.map(|k| self.sim.funds(&k))
    }
    async fn send(&mut self, what: &str, b: &Built, _next: &tresor::TresorFile) -> Result<(), String> {
        if self.fail_sends > 0 {
            self.fail_sends -= 1;
            return Err("Node lehnt ab: UTXO schon verbraucht".into());
        }
        self.sim.submit(b)?;
        self.sent.push(what.into());
        Ok(())
    }
    fn save(&mut self, _: &tresor::TresorFile) -> Result<(), String> {
        Ok(())
    }
    fn journal_open(&self) -> bool {
        self.journal_open
    }
    fn dry_run(&self) -> bool {
        false
    }
    fn now_ms(&self) -> i64 {
        self.sim.now_ms as i64
    }
    fn say(&mut self, _: &str) {}
}

/// Zwei fällige Tresore in einer Tresor-Datei
fn two_due() -> (World, tresor::TresorFile) {
    let mut w = World::new();
    let (t1, t2) = (w.open(3, 100 * E8), w.open(3, 100 * E8));
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(TresorRec::new(w.p.clone(), t1.clone(), String::new(), false, None, "x"));
    f.upsert(TresorRec::new(w.p.clone(), t2, String::new(), false, None, "x"));
    w.at(t1.state.next_due + 10 * 60_000);
    (w, f)
}

/// Nachprüfung A12-16: Die Runde der Automatik (`tresor pay` ohne ID) läuft
/// jetzt in der Bibliothek und wird hier gegen den Simulator gefahren. Ein
/// Sendefehler beim ersten Tresor (anderer Auslöser schneller) hält den
/// zweiten nicht auf; ist das Journal noch offen, endet die Runde.
#[tokio::test]
async fn a12n_runde_geht_nach_sendefehler_weiter() {
    let (mut w, mut f) = two_due();
    let (params, pmt) = (w.sim.params.clone(), w.sim.now_ms as i64 - tresor::PMT_LAG_MS);
    let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 1, journal_open: false, sent: vec![] };
    let reps = tresor::pay_round(&mut io, &mut f, None, false, pmt, "x", &params).await.unwrap();
    println!("{reps:?}");
    assert_eq!(reps.iter().map(|r| r.paid).collect::<Vec<_>>(), vec![false, true], "zweiter Tresor trotzdem gezahlt");
    assert_eq!(io.sent.len(), 1);
    assert!(f.tresore[0].retry_after.is_some() && f.tresore[0].last_error.is_some(), "der erste wartet");
    assert_eq!(f.tresore[1].utxo.state.left, 2, "der zweite ist nachgeführt");
    // Journal offen: weitere Sendungen erst nach dessen Klärung
    let (mut w, mut f) = two_due();
    let (params, pmt) = (w.sim.params.clone(), w.sim.now_ms as i64 - tresor::PMT_LAG_MS);
    let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 1, journal_open: true, sent: vec![] };
    let reps = tresor::pay_round(&mut io, &mut f, None, false, pmt, "x", &params).await.unwrap();
    assert_eq!(reps.len(), 1, "Runde nach dem ersten Fehler beendet");
    assert!(io.sent.is_empty());
}

/// Nachprüfung A12-16: Die Automatik nimmt nicht auffindbare Tresore wieder
/// in die Runde, sobald das erneute Nachsehen fällig ist (vorher: nie mehr),
/// und sucht sie dabei nur begrenzt.
#[tokio::test]
async fn a12n_runde_sieht_fehlende_wieder_nach() {
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    w.at(t.state.next_due + 10 * 60_000);
    let now = w.sim.now_ms as i64;
    let mut rec = TresorRec::new(w.p.clone(), t, String::new(), false, None, "x");
    // beim letzten Abgleich nicht gefunden (Aussetzer des Nodes), Wartezeit um
    rec.missing = Some("vor einer Stunde".into());
    rec.missing_ms = Some(now - tresor::MISSING_RECHECK_MS);
    rec.retry_after = Some(now - 1);
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(rec);
    assert!(f.tresore[0].missing.is_some() && !f.tresore[0].active(), "nicht aktiv, nur zum erneuten Nachsehen");
    let params = w.sim.params.clone();
    let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
    let reps = tresor::pay_round(&mut io, &mut f, None, false, now - tresor::PMT_LAG_MS, "x", &params).await.unwrap();
    assert_eq!(reps.len(), 1, "{reps:?}");
    assert!(reps[0].paid, "wiedergefunden und gezahlt");
    assert!(f.tresore[0].missing.is_none());
    assert!(io.lookups <= tresor::MISSING_RECHECK_FOLLOW);
}

/// Zweite Nachprüfung A12-16 (Rückbau R20 überlebte): Die Runde der Automatik
/// sieht einen verschwundenen Tresor höchstens stündlich nach. Ohne die
/// Wartezeit beim Fehlen suchte jede Runde (Agent, Zeitgeber der Seite) erneut
/// – eine Woche lang, statt 168-mal.
#[tokio::test]
async fn a12p_runde_sucht_fehlende_hoechstens_stuendlich() {
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    w.at(t.state.next_due + 10 * 60_000);
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(TresorRec::new(w.p.clone(), t.clone(), String::new(), false, None, "x"));
    // der Absender kündigt: am Node nicht mehr auffindbar
    let b = tresor::cancel(&w.p, &t, &w.owner, &w.sim.params).expect("kündigen");
    w.sim.submit(&b).expect("Kündigung angenommen");
    let (params, now) = (w.sim.params.clone(), w.sim.now_ms as i64);
    let mut lookups = vec![];
    for later in [0, 60_000, 30 * 60_000, tresor::MISSING_RECHECK_MS - 1, tresor::MISSING_RECHECK_MS] {
        w.sim.set_time((now + later) as u64);
        let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
        tresor::pay_round(&mut io, &mut f, None, false, now + later - tresor::PMT_LAG_MS, "x", &params).await.unwrap();
        lookups.push(io.lookups);
    }
    println!("Node-Abfragen je Runde (0 s, 1 min, 30 min, 1 h − 1 ms, 1 h): {lookups:?}");
    assert!(f.tresore[0].missing.is_some());
    assert!(lookups[0] > 0, "erstes Fehlen: gesucht");
    assert_eq!(lookups[1..4], [0, 0, 0], "innerhalb der Stunde: keine Abfrage");
    assert!(lookups[4] > 0 && lookups[4] <= tresor::MISSING_RECHECK_FOLLOW, "nach einer Stunde: begrenzt nachgesehen");
}

// ------------------------------------------- Audit 12, Restpunkte (Gruppe a) ----

/// Restpunkt zu A12-7 (Rückbau R15 überlebte: `due_now(…, with_key)` in
/// pay_round → `true`): Ohne eigenen Schlüssel lässt die Runde einen Tresor
/// aus, der die Gebühr nicht mehr trägt – dieselbe Regel wie `payParams` auf
/// der Seite. Kein Zahlungsversuch, kein Fehler, keine Wartezeit. Ein
/// bestimmter Tresor ohne Schlüssel nennt den Grund. Mit Schlüssel zahlt der
/// Auslöser die Gebühr.
#[tokio::test]
async fn a13_runde_ohne_schluessel_nur_wenn_der_tresor_die_gebuehr_traegt() {
    let mut w = World::new();
    let first = w.first(3);
    // 10 KAS + 1 KAS + 0,005 KAS: nach Betrag und Höchstgebühr blieben < 1 KAS
    let t = plant(&mut w.sim, &w.p, first, 11 * E8 + 500_000, 1);
    w.at(t.state.next_due + 10 * 60_000);
    let (params, pmt) = (w.sim.params.clone(), w.sim.now_ms as i64 - tresor::PMT_LAG_MS);
    assert!(tresor::due_now(&w.p, &t, pmt, true).is_ok() && tresor::due_now(&w.p, &t, pmt, false).is_err(), "nur mit eigener Gebühr zahlbar");
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(TresorRec::new(w.p.clone(), t.clone(), String::new(), false, None, "x"));
    let id = f.tresore[0].id.clone();
    // Runde der Automatik ohne Schlüssel
    let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
    let reps = tresor::pay_round(&mut io, &mut f, None, false, pmt, "x", &params).await.unwrap();
    assert!(reps.is_empty(), "ausgelassen, nicht versucht: {reps:?}");
    assert!(io.sent.is_empty());
    assert_eq!((f.tresore[0].last_error.clone(), f.tresore[0].retry_after), (None, None), "kein Fehler, keine Wartezeit");
    // ein bestimmter Tresor ohne Schlüssel: der Grund, kein Zahlungsversuch
    let e = tresor::pay_round(&mut io, &mut f, Some(&id), false, pmt, "x", &params).await.unwrap_err();
    assert!(e.contains("trägt die Netzgebühr nicht mehr") && e.contains("eigenem Schlüssel"), "{e}");
    assert_eq!(f.tresore[0].last_error, None);
    assert_eq!(f.tresore[0].utxo.value, t.value, "Tresor unberührt");
    // mit Schlüssel: gezahlt, die Gebühr trägt der Auslöser
    let helfer = key();
    w.sim.faucet(&helfer, 5 * E8);
    let mut io = SimIo { sim: &mut w.sim, key: Some(helfer), lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
    let reps = tresor::pay_round(&mut io, &mut f, None, true, pmt, "x", &params).await.unwrap();
    assert_eq!(reps.iter().map(|r| r.paid).collect::<Vec<_>>(), vec![true], "{reps:?}");
    assert_eq!(f.tresore[0].utxo.value, t.value - 10 * E8, "der Tresor verliert nur den Betrag");
    assert_eq!(f.tresore[0].history.last().and_then(|h| h.note.as_deref()), Some("Gebühr vom Auslöser"));
    assert!(w.sim.balance(&helfer) < 5 * E8);
}

/// Restpunkt zu A12-16 (Rückbau R30 überlebte: `if note.is_some()` → `if
/// true`): Die Runde meldet einen verschwundenen Tresor nur beim ersten Mal,
/// nicht bei jedem stündlichen erneuten Nachsehen eine Woche lang. Bisher war
/// das nur für `follow` getestet, nicht für die Berichte der Runde.
#[tokio::test]
async fn a13_runde_meldet_fehlende_nur_beim_ersten_mal() {
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    w.at(t.state.next_due + 10 * 60_000);
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(TresorRec::new(w.p.clone(), t.clone(), String::new(), false, None, "x"));
    let b = tresor::cancel(&w.p, &t, &w.owner, &w.sim.params).expect("kündigen");
    w.sim.submit(&b).expect("Kündigung angenommen");
    let (params, now) = (w.sim.params.clone(), w.sim.now_ms as i64);
    let (mut gemeldet, mut lookups) = (vec![], vec![]);
    for later in [0, tresor::MISSING_RECHECK_MS, 2 * tresor::MISSING_RECHECK_MS, DAY_MS] {
        w.sim.set_time((now + later) as u64);
        let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
        let reps = tresor::pay_round(&mut io, &mut f, None, false, now + later - tresor::PMT_LAG_MS, "x", &params).await.unwrap();
        gemeldet.push(reps.iter().filter(|r| r.text.contains("nicht auffindbar")).count());
        lookups.push(io.lookups);
    }
    println!("Meldungen je Runde (0 h, 1 h, 2 h, 24 h): {gemeldet:?}, Node-Abfragen {lookups:?}");
    assert!(lookups.iter().all(|&n| n > 0), "jedes Mal nachgesehen: {lookups:?}");
    assert_eq!(gemeldet, [1, 0, 0, 0], "gemeldet nur beim ersten Mal");
    assert!(f.tresore[0].missing.is_some());
}

/// Attrappe für message::inbox: Verlauf aus einer Liste, der Node kennt jede Tx
struct InboxSim {
    txs: Vec<serde_json::Value>,
    node: Vec<kaspa_consensus_core::tx::Transaction>,
}

impl kaspa_lending_protocol::message::InboxIo for InboxSim {
    fn address_txs(&mut self, _: &str, _: usize) -> Result<Vec<serde_json::Value>, String> {
        Ok(self.txs.clone())
    }
    fn tx(&mut self, _: &str) -> Result<serde_json::Value, String> {
        Err("nicht im Test".into())
    }
    async fn connect(&mut self) -> bool {
        true
    }
    async fn node_tx(&mut self, rest: &serde_json::Value) -> kaspa_lending_protocol::message::NodeLookup {
        use kaspa_lending_protocol::message::{NodeLookup, NodeTx};
        match self.node.iter().find(|t| Some(t.id().to_string().as_str()) == rest["transaction_id"].as_str()) {
            Some(t) => NodeLookup::Found(NodeTx {
                payload: t.payload.clone(),
                sigs: t.inputs.iter().map(|i| i.signature_script.clone()).collect(),
                outputs: t.outputs.iter().map(|o| (o.value, o.script_public_key.script().to_vec())).collect(),
            }),
            None => NodeLookup::Unavailable,
        }
    }
    async fn retention(&mut self) -> Option<kaspa_lending_protocol::message::Retention> {
        None
    }
}

/// Nachprüfung A12-1: Der Ablauf von `ghostctl messages` (jetzt message::inbox)
/// prüft Tresor-Nachrichten gegen die Tresor-Datei neben der Zustandsdatei.
/// Vorher stand das nur in ghostctl; ein Aufruf mit leerer Liste statt der
/// bekannten Tresore fiel keinem Test auf.
#[tokio::test]
async fn a12n_eingang_prueft_gegen_die_tresor_datei() {
    use kaspa_lending_protocol::message::{self, Origin, Source, TresorCheck};
    let mut w = World::sealed("Miete Februar");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    w.at(t.state.next_due + 10 * 60_000);
    let (b, _) = w.pay(&t, &rec.payload()).unwrap();
    let dir = std::env::temp_dir().join(format!("ghost-a12n-eingang-{}-{}", std::process::id(), rec.id));
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("mainnet.json");
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(rec.clone());
    tresor::save(&tresor::path_for(&state), &f).unwrap();
    let sk = SecretKey::from_keypair(&w.recipient);
    let owner: [u8; 32] = w.p.owner.clone().try_into().unwrap();
    let mut io = InboxSim { txs: vec![rest_of(&b)], node: vec![b.tx.clone()] };
    let r = message::inbox(&mut io, "mainnet", &state, &sk, 50).await.unwrap();
    assert_eq!(r.messages.len(), 1);
    assert_eq!(r.messages[0].origin, Origin::Tresor { id: Some(rec.id.clone()), owner: Some(owner), check: TresorCheck::AsStored });
    assert_eq!(r.messages[0].source, Source::Node);
    // ohne Tresor-Datei: erkannt, Nachricht vom Vertrag erzwungen, ohne Besitzer
    std::fs::remove_file(tresor::path_for(&state)).unwrap();
    let r = message::inbox(&mut io, "mainnet", &state, &sk, 50).await.unwrap();
    assert_eq!(r.messages[0].origin, Origin::Tresor { id: None, owner: None, check: TresorCheck::Bound });
    std::fs::remove_dir(&dir).unwrap();
}

// ------------------------------------- A12-1 im Vertrag: Tresor-Code v2 ----

/// Tresor-Code Version 2 trägt den Hash der Nachricht (in den Parametern) und
/// geht hin und zurück – öffentlich, verschlüsselt und ohne Nachricht; der
/// Empfänger übernimmt ihn am Simulator und zahlt mit genau dem gebundenen Payload.
#[tokio::test]
async fn a13_tresor_code_v2_hin_und_zurueck() {
    for mut w in [World::public("Miete Whg. 3"), World::sealed("Miete Oktober"), World::new()] {
        let t = w.open(3, 100 * E8);
        let rec = w.rec(&t);
        let code = TresorCode::of("mainnet", &rec).encode();
        assert!(code.starts_with("ghost-tresor:2:"), "{code}");
        let c = TresorCode::decode(&code).expect("lesbar");
        assert_eq!(c, TresorCode::of("mainnet", &rec));
        assert_eq!(c.params.payload_hash, payload_hash(&rec.payload()));
        assert!(serde_json::to_string(&c).unwrap().contains("\"payloadHash\""), "Hash als Hex im Code");
        // übernehmen und auslösen wie der Empfänger
        w.at(t.state.next_due + 10 * 60_000);
        let pmt = w.sim.now_ms as i64 - tresor::PMT_LAG_MS;
        let mut f = tresor::TresorFile::empty("mainnet");
        let params = w.sim.params.clone();
        let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
        let i = tresor::import(&mut io, &mut f, &code, "mainnet", pmt, "x", None).await.expect("übernommen");
        assert_eq!(f.tresore[i].payload(), rec.payload());
        let reps = tresor::pay_round(&mut io, &mut f, None, false, pmt, "x", &params).await.unwrap();
        assert_eq!(reps.iter().map(|r| r.paid).collect::<Vec<_>>(), vec![true], "{} {reps:?}", w.message);
        assert_eq!(w.sim.balance(&w.recipient), 10 * E8);
    }
}

/// Alte Codes (Version 1, Vertrag ohne gebundene Nachricht) werden klar
/// abgelehnt – im Mainnet gab es noch keine Tresore.
#[test]
fn a13_alter_tresor_code_abgelehnt() {
    let mut w = World::public("Miete");
    let t = w.open(3, 100 * E8);
    let v2 = TresorCode::of("mainnet", &w.rec(&t)).encode();
    let v1 = v2.replacen("ghost-tresor:2:", "ghost-tresor:1:", 1);
    let e = TresorCode::decode(&v1).unwrap_err();
    assert!(e.contains("Alter Tresor-Code") && e.contains("neuen Tresor"), "{e}");
    let e = TresorCode::decode(&format!("  {v1}\n")).unwrap_err();
    assert!(e.contains("Alter Tresor-Code"), "auch mit Leerraum: {e}");
    // ein v1-Inhalt (ohne payloadHash) unter dem neuen Präfix: ungültig
    let mut json = serde_json::to_value(TresorCode::of("mainnet", &w.rec(&t))).unwrap();
    json["params"].as_object_mut().unwrap().remove("payloadHash");
    let v1_body = serde_json::from_value::<TresorCode>(json).expect("Feld fehlt: leer");
    assert!(v1_body.params.payload_hash.is_empty());
    assert!(TresorCode::decode(&v1_body.encode()).unwrap_err().contains("ungültige Werte"), "ohne Hash");
}

/// Import: der Hash im Code muss zur Nachricht passen (öffentlich: Klartext,
/// verschlüsselt: die hinterlegte Fassung, ohne: leer), sonst wird der Code
/// abgelehnt – die Zahlungen trügen etwas anderes, als der Code zeigt, und
/// ghostctl könnte sie gar nicht auslösen.
#[tokio::test]
async fn a13_import_mit_falschem_hash_abgelehnt() {
    use kaspa_lending_protocol::message;
    let mut w = World::sealed("Miete Oktober");
    let t = w.open(3, 100 * E8);
    let good = TresorCode::of("mainnet", &w.rec(&t));
    let with = |f: &dyn Fn(&mut TresorCode)| {
        let mut c = good.clone();
        f(&mut c);
        c.encode()
    };
    let other_sealed = faster_hex::hex_string(&message::encrypt(&w.p.recipient, "Miete Oktober").unwrap());
    let bad = [
        ("andere verschlüsselte Fassung desselben Textes", with(&|c| c.sealed = other_sealed.clone())),
        ("verschlüsselte Fassung entfernt", with(&|c| c.sealed = String::new())),
        ("als öffentlich ausgegeben", with(&|c| c.onchain = true)),
        ("Hash verändert", with(&|c| c.params.payload_hash[0] ^= 1)),
    ];
    let pmt = w.sim.now_ms as i64;
    for (label, code) in bad {
        let e = TresorCode::decode(&code).unwrap_err();
        // seit A13-tresor-2/3 scheitern die beiden mittleren schon an der Form
        let form = e.contains("Beschreibung ohne Nachricht") || e.contains("mit verschlüsselter Fassung");
        assert!(e.contains("passt nicht zum Vertrag") || e.contains("ungültige Werte") || form, "{label}: {e}");
        let mut f = tresor::TresorFile::empty("mainnet");
        let mut io = SimIo { sim: &mut w.sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] };
        assert!(tresor::import(&mut io, &mut f, &code, "mainnet", pmt, "x", None).await.is_err(), "{label}");
        assert!(f.tresore.is_empty() && io.lookups == 0, "{label}: abgelehnt, bevor der Node gefragt wird");
    }
    // öffentliche Nachricht: Hash des Klartexts
    let mut p = World::public("Miete Whg. 3");
    let tp = p.open(3, 100 * E8);
    let mut c = TresorCode::of("mainnet", &p.rec(&tp));
    assert!(TresorCode::decode(&c.encode()).is_ok());
    c.message = "Miete Whg. 4".into();
    assert!(TresorCode::decode(&c.encode()).unwrap_err().contains("passt nicht zum Vertrag"));
    // ohne Nachricht: Hash des leeren Payloads; eine öffentliche Beschreibung passt nicht
    let mut n = World::new();
    let tn = n.open(3, 100 * E8);
    let mut c = TresorCode::of("mainnet", &n.rec(&tn));
    assert!(TresorCode::decode(&c.encode()).is_ok());
    (c.message, c.onchain) = ("Miete".into(), true);
    assert!(TresorCode::decode(&c.encode()).is_err());
}

/// Tresor-Datei des alten Vertrags (ohne payloadHash): klar abgelehnt statt
/// still falsch bedient. Eine leere alte Datei geht.
#[test]
fn a13_alte_tresor_datei_abgelehnt() {
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(w.rec(&t));
    let dir = std::env::temp_dir().join(format!("ghost-a13-alte-datei-{}-{}", std::process::id(), f.tresore[0].id));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mainnet-tresore.json");
    let mut json = serde_json::to_value(&f).unwrap();
    json["version"] = 1.into();
    tresor::save(&path, &f).unwrap();
    assert!(tresor::load(&path, "mainnet").is_ok());
    json["tresore"][0]["params"].as_object_mut().unwrap().remove("payloadHash");
    std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
    let e = tresor::load(&path, "mainnet").unwrap_err();
    assert!(e.contains("alten Vertrag"), "{e}");
    std::fs::write(&path, r#"{"version":1,"network":"mainnet","tresore":[]}"#).unwrap();
    assert_eq!(tresor::load(&path, "mainnet").unwrap().version, tresor::FILE_VERSION);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&dir).unwrap();
}

// ------------------------------------------- Audit 13 (Behebung Tresor) ----

fn a13f_io(sim: &mut Sim) -> SimIo<'_> {
    SimIo { sim, key: None, lookups: 0, fail_sends: 0, journal_open: false, sent: vec![] }
}

/// A13-tresor-4: Untergrenze der Höchstgebühr gegen die gemessene Gebühr einer
/// Zahlung mit dem längsten Payload (MAX_PAYLOAD) im ungünstigsten Fall:
/// kleinster Betrag und kleinste Fortsetzung (Speichermasse), längste Skripte
/// (große Anzahl, Ankertag, Höchstgebühr mit langem Push)
#[test]
fn a13f_mindest_hoechstgebuehr_ueber_der_gemessenen_gebuehr() {
    let w = World::new();
    let mut sim = w.sim;
    let payload: Vec<u8> = (0..txb::MAX_PAYLOAD).map(|i| (i % 251) as u8 + 1).collect();
    let mut worst = 0u64;
    let mut n = 90u8;
    for amount in [tresor::MIN_AMOUNT, 1_000 * E8 as i64, (1i64 << 55) + 12_345] {
        for left in [1i64, -1, 1 << 40] {
            for (anchor_day, period_ms) in [(31i64, 0i64), (0, 3650 * DAY_MS)] {
                for rest in [tresor::MIN_KEEP as u64, 1_000_000 * E8] {
                    let p = TresorParams { amount, anchor_day, period_ms, max_fee: tresor::MAX_MAX_FEE, payload_hash: payload_hash(&payload), ..w.p.clone() };
                    let first = TresorState { next_due: if anchor_day == 31 { ms(2027, 1, 31, 8) } else { ms(2027, 2, 1, 8) }, left };
                    let value = amount as u64 + tresor::MAX_MAX_FEE as u64 + rest;
                    let t = plant(&mut sim, &p, first, value, n);
                    n = n.wrapping_add(1);
                    if first.next_due as u64 + 600_000 > sim.now_ms {
                        sim.set_time(first.next_due as u64 + 600_000);
                    }
                    let b = tresor::pay(&p, &t, &payload, None, &sim.params).expect("baut").built;
                    sim.submit(&b).expect("angenommen");
                    worst = worst.max(b.fee);
                }
            }
        }
    }
    println!("A13 Gebühr einer Zahlung mit {} Byte Payload, ungünstigster Fall: {worst} sompi; Untergrenze {}", txb::MAX_PAYLOAD, tresor::MIN_MAX_FEE);
    // mindestens 10 % Aufschlag auf die gemessene Gebühr
    assert!(tresor::MIN_MAX_FEE as u64 >= worst + worst / 10, "Untergrenze {} zu knapp über {worst}", tresor::MIN_MAX_FEE);
    assert!(tresor::MIN_MAX_FEE <= tresor::DEFAULT_MAX_FEE);
}

/// A13-tresor-4: Höchstgebühr unter der Mindestgebühr wird beim Anlegen
/// abgelehnt (übernommen aus r13_hoechstgebuehr_unter_mindestgebuehr: mit
/// 100.000 sompi war der Tresor ohne eigenen Schlüssel nie zahlbar)
#[test]
fn a13f_hoechstgebuehr_unter_mindestgebuehr_abgelehnt() {
    let mut w = World::public("Miete Whg. 3");
    for (fee, ok) in [(1, false), (100_000, false), (tresor::MIN_MAX_FEE - 1, false), (tresor::MIN_MAX_FEE, true), (tresor::DEFAULT_MAX_FEE, true), (tresor::MAX_MAX_FEE, true), (tresor::MAX_MAX_FEE + 1, false)] {
        w.p.max_fee = fee;
        let r = tresor::check_params(&w.p, &w.first(3));
        assert_eq!(r.is_ok(), ok, "maxFee {fee}: {r:?}");
        if !ok && fee < tresor::MIN_MAX_FEE {
            assert!(r.unwrap_err().contains("0.004"), "Meldung nennt die Untergrenze");
        }
    }
    // mit der Untergrenze und längster Nachricht zahlt der Tresor selbst
    let text = "𝄞".repeat(100);
    let mut w = World::public(&text);
    w.p.max_fee = tresor::MIN_MAX_FEE;
    let t = w.open(3, 100 * E8);
    w.at(t.state.next_due + 10 * 60_000);
    let r = tresor::pay(&w.p, &t, text.as_bytes(), None, &w.sim.params).expect("ohne Schlüssel zahlbar");
    assert!(r.fee_from_tresor);
    w.sim.submit(&r.built).expect("angenommen");
}

/// A13-tresor-2/3: Form der Nachricht im Code (übernommen aus
/// r13_code_erfundene_beschreibung_ohne_nachricht und
/// r13_code_onchain_ohne_text_mit_chiffrat)
#[test]
fn a13f_code_beschreibung_nur_mit_passender_nachricht() {
    // ohne Nachricht: erfundene Beschreibung passte zum leeren Payload
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    assert!(TresorCode::decode(&TresorCode::of("mainnet", &rec).encode()).is_ok(), "echter Code ohne Nachricht");
    let mut c = TresorCode::of("mainnet", &rec);
    c.message = "Miete – ab Maerz neue Kontonummer, siehe Mail".into();
    let e = TresorCode::decode(&c.encode()).unwrap_err();
    assert!(e.contains("Beschreibung ohne Nachricht"), "{e}");
    // verschlüsselt: öffentlich ohne Text mit Chiffrat, Chiffrat ohne Beschreibung
    let mut w = World::sealed("Miete Oktober");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    assert!(TresorCode::decode(&TresorCode::of("mainnet", &rec).encode()).is_ok(), "echter verschlüsselter Code");
    let mut c = TresorCode::of("mainnet", &rec);
    (c.message, c.onchain) = (String::new(), true);
    let e = TresorCode::decode(&c.encode()).unwrap_err();
    assert!(e.contains("öffentliche Nachricht ohne Text"), "{e}");
    let mut c = TresorCode::of("mainnet", &rec);
    c.message = "   ".into();
    let e = TresorCode::decode(&c.encode()).unwrap_err();
    assert!(e.contains("ohne Beschreibung"), "{e}");
    // öffentlich: Text und kein Chiffrat
    let mut w = World::public("Miete Whg. 3");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    assert!(TresorCode::decode(&TresorCode::of("mainnet", &rec).encode()).is_ok(), "echter öffentlicher Code");
    let mut c = TresorCode::of("mainnet", &rec);
    c.sealed = faster_hex::hex_string(&kaspa_lending_protocol::message::encrypt(&w.p.recipient, "x").unwrap());
    let e = TresorCode::decode(&c.encode()).unwrap_err();
    assert!(e.contains("mit verschlüsselter Fassung"), "{e}");
}

/// A13-tresor-1: verschlüsselte Nachricht beim Import mit dem Schlüssel des
/// Empfängers gegen die Beschreibung geprüft (übernommen aus
/// r13_code_beschreibung_bei_verschluesselt_frei)
#[tokio::test]
async fn a13f_import_prueft_verschluesselte_beschreibung() {
    use kaspa_lending_protocol::tresor::MessageCheck;
    let mut w = World::sealed("Miete Oktober");
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    let mut c = TresorCode::of("mainnet", &rec);
    c.message = "Ab jetzt bitte an kaspa:qneu... zurückzahlen".into();
    let forged = c.encode();
    let genuine = TresorCode::of("mainnet", &rec).encode();
    let sk = SecretKey::from_keypair(&w.recipient);
    let other = SecretKey::from_keypair(&w.owner);
    let pmt = w.sim.now_ms as i64;
    // mit dem Empfängerschlüssel: gefälschte Beschreibung abgelehnt
    let mut f = tresor::TresorFile::empty("mainnet");
    let e = tresor::import(&mut a13f_io(&mut w.sim), &mut f, &forged, "mainnet", pmt, "x", Some(&sk)).await.unwrap_err();
    assert!(e.contains("weicht von der verschlüsselten Nachricht ab"), "{e}");
    assert!(f.tresore.is_empty());
    // ohne (oder mit fremdem) Schlüssel: übernommen, aber nicht geprüft
    for k in [None, Some(&other)] {
        let mut f = tresor::TresorFile::empty("mainnet");
        let i = tresor::import(&mut a13f_io(&mut w.sim), &mut f, &forged, "mainnet", pmt, "x", k).await.expect("übernommen");
        let r = &f.tresore[i];
        assert!(!r.checked);
        assert_eq!(r.message_check(None), MessageCheck::Unchecked);
        assert!(!r.message_sure());
        // bei der Anzeige mit dem Empfängerschlüssel: abweichend
        assert_eq!(r.message_check(Some(&sk)), MessageCheck::Mismatch);
        assert_eq!(r.message_check(Some(&other)), MessageCheck::Unchecked, "fremder Schlüssel prüft nichts");
    }
    // echter Code mit Empfängerschlüssel: geprüft und gespeichert
    let mut f = tresor::TresorFile::empty("mainnet");
    let i = tresor::import(&mut a13f_io(&mut w.sim), &mut f, &genuine, "mainnet", pmt, "x", Some(&sk)).await.expect("übernommen");
    assert!(f.tresore[i].checked);
    assert_eq!(f.tresore[i].message_check(None), MessageCheck::Checked);
    // später mit Schlüssel erneut übernommen: der ungeprüfte Eintrag wird geprüft
    let mut f = tresor::TresorFile::empty("mainnet");
    tresor::import(&mut a13f_io(&mut w.sim), &mut f, &genuine, "mainnet", pmt, "x", None).await.unwrap();
    assert_eq!(f.tresore[0].message_check(None), MessageCheck::Unchecked);
    tresor::import(&mut a13f_io(&mut w.sim), &mut f, &genuine, "mainnet", pmt, "x", Some(&sk)).await.unwrap();
    assert_eq!(f.tresore[0].message_check(None), MessageCheck::Checked);
    // öffentlich: gebunden über den Hash; hier angelegt: geprüft (der Absender hat verschlüsselt)
    assert_eq!(World::public("Miete").rec(&t).message_check(None), MessageCheck::Bound);
    let mut own = rec.clone();
    own.key = Some("keys/a.json".into());
    assert_eq!(own.message_check(None), MessageCheck::Checked);
    assert_eq!(World::new().rec(&t).message_check(None), MessageCheck::None);
}

/// A13-tresor-2: upsert übernimmt keine ungebundene Beschreibung aus einem
/// Code; eine geprüfte ersetzt eine ungeprüfte
#[test]
fn a13f_upsert_nur_gebundene_beschreibung() {
    // ohne Nachricht übernommen, dann ein Eintrag mit erfundener Beschreibung
    // (an decode vorbei, z. B. aus einer älteren Datei)
    let mut w = World::new();
    let t = w.open(3, 100 * E8);
    let rec = w.rec(&t);
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(rec.clone());
    let mut forged = rec.clone();
    forged.message = "Miete – ab Maerz neue Kontonummer".into();
    f.upsert(forged.clone());
    assert_eq!(f.tresore[0].message, "", "ungebundene Beschreibung nicht übernommen");
    forged.checked = true; // selbst als „geprüft“ markiert: passt nicht zum leeren Payload
    f.upsert(forged);
    assert_eq!(f.tresore[0].message, "");
    // verschlüsselt: ungeprüfte Fälschung zuerst, dann der geprüfte echte Code
    let mut w = World::sealed("Miete Oktober");
    let t = w.open(3, 100 * E8);
    let genuine = w.rec(&t);
    let mut fake = genuine.clone();
    fake.message = "Neue Adresse".into();
    let mut f = tresor::TresorFile::empty("mainnet");
    f.upsert(fake.clone());
    f.upsert(genuine.clone());
    assert_eq!(f.tresore[0].message, "Neue Adresse", "ungeprüft ersetzt nichts");
    let mut checked = genuine.clone();
    checked.checked = true;
    f.upsert(checked);
    assert_eq!((f.tresore[0].message.as_str(), f.tresore[0].checked), ("Miete Oktober", true), "geprüft ersetzt ungeprüft");
    f.upsert(fake);
    assert_eq!(f.tresore[0].message, "Miete Oktober", "geprüft bleibt");
}
