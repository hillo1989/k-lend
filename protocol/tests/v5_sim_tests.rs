//! Version 5 im Simulator (Konsensprüfung der relativen Sperre wie
//! check_sequence_lock, Speichermasse, Mindestgebühr, Mempool-Regel 15
//! Signaturprüfungen): Mindestabstand trotz DAA-Rückstand, Zeitachse eines
//! Angreifers mit gestohlenem 1-von-1-Schlüssel (A20e-1), Wächter-Sperre und
//! Notfall-Austausch (A20e-2), Massen und Gebühren.

use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::ops::xonly;
use kaspa_lending_protocol::sim::Sim;
use kaspa_lending_protocol::txb::Built;
use kaspa_lending_protocol::v5::{self, Feed5};
use secp256k1::{Keypair, Secp256k1, SecretKey};

const E8: u64 = 100_000_000;
const GAP: i64 = 3_000; // 5 min
const PRICE: i64 = 4_335_083; // Live-Preis Audit 20e

fn key(seed: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[seed; 32]).unwrap())
}

fn rp(deployer: &Keypair) -> RegisterV5Params {
    RegisterV5Params {
        deployer: xonly(deployer),
        min_signers: 1,
        min_threshold: 1,
        rot_delay_daa: 14 * DAY_DAA,
        emerg_after_daa: 30 * DAY_DAA,
        emerg_delay_daa: 7 * DAY_DAA,
        min_gap_daa: GAP,
    }
}

fn op() -> OracleV5Params {
    OracleV5Params { reg_cov: kaspa_consensus_core::Hash::from_bytes([0; 32]), max_rate: 634_195_839, rate_step: 15_854_896, rate_gap_daa: HOUR_DAA, freeze_after_daa: 2 * HOUR_DAA, jump_bps: 2_500, ref_after: 6 }
}

struct W {
    sim: Sim,
    feed: Feed5,
    payer: Keypair,
    signers: Vec<Keypair>,
    fb: Keypair,
    guards: Vec<Keypair>,
}

/// 1 von 1 (heute), Notfallsatz 1 Schlüssel, zwei Wächter
fn setup(n_signers: usize, t: i64) -> W {
    let mut sim = Sim::new();
    let deployer = key(1);
    let payer = key(2);
    sim.faucet(&deployer, 100 * E8);
    for _ in 0..4 {
        sim.faucet(&payer, 1_000 * E8);
    }
    let signers: Vec<Keypair> = (0..n_signers).map(|i| key(10 + i as u8)).collect();
    let set = SignerSet { keys: signers.iter().map(xonly).collect(), t, t_rot: t };
    let fb = key(30);
    let fbs = SignerSet { keys: vec![xonly(&fb)], t: 1, t_rot: 1 };
    let guards = vec![key(40), key(41)];
    let gs = GuardSet { keys: guards.iter().map(xonly).collect() };
    let (feed, _) = v5::deploy_feed(&mut sim, &deployer, rp(&deployer), set, Some(fbs), Some(gs), op(), PRICE, 0, E8).expect("Deployment v5");
    W { sim, feed, payer, signers, fb, guards }
}

impl W {
    fn signers_of(&self) -> Vec<(usize, Keypair)> {
        self.signers.iter().copied().enumerate().collect()
    }
    fn update(&mut self, kas: i64) -> Result<Built, String> {
        let daa = self.sim.daa - 1;
        let (b, f) = v5::attest(&self.feed, &self.signers_of(), kas, 0, daa, &self.sim.funds(&self.payer), &self.sim.params.clone())?;
        self.sim.submit(&b)?;
        self.feed = f;
        Ok(b)
    }
}

fn fee(b: &Built) -> String {
    format!("{:.4} KAS (compute {} g, transient {} g, storage {} g)", b.fee as f64 / 1e8, b.compute_mass, b.transient_mass, b.storage_mass)
}

#[test]
fn mindestabstand_setzt_der_konsens_durch_auch_bei_rueckstand() {
    let mut w = setup(1, 1);
    // großer Rückstand: Orakel 1 h hinter der Kette (v4: 6 Updates sofort)
    w.sim.advance(HOUR_DAA as u64);
    let p = w.feed.oracle.state.kas_usd;
    w.update(p).expect("erstes Update nach langer Pause");
    // nach 70 s (Orakel-Abstand 600 DAA erfüllt): die Register-UTXO ist zu
    // jung → der Konsens lehnt ab (die Skripte allein lassen es zu)
    w.sim.advance(700);
    let e = w.update(p).unwrap_err();
    assert!(e.contains("relative Sperre"), "{e}");
    w.sim.advance(GAP as u64 - 700 - 20);
    assert!(w.update(p).unwrap_err().contains("relative Sperre"));
    w.sim.advance(20);
    w.update(p).expect("nach 5 min");
}

/// Zeitachse eines Diebs mit dem einzigen Schlüssel: jedes Update so früh wie
/// möglich, jeweils −25 %. Gezählt wird, ab wann auch der Referenzpreis
/// gefälscht ist (erst dann bewegt sich Geld, Zwei-Preis-Regel).
#[test]
fn a20e_1_zeitachse_des_angreifers() {
    let mut w = setup(1, 1);
    // ungünstigster Fall: die Referenz rotiert genau beim ersten gefälschten
    // Update (seq 6) – vorher fünf ehrliche Updates
    for _ in 0..5 {
        w.sim.advance(GAP as u64);
        w.update(PRICE).expect("ehrlich");
    }
    w.sim.advance(HOUR_DAA as u64); // Rückstand wie im Audit
    let start = w.sim.daa;
    let mut table = vec![];
    let mut first_fake_ref = None;
    let mut liquidatable_200 = None; // Vault bei 200 %: zu beiden Preisen < 150 %
    for n in 1..=19 {
        let cur = w.feed.oracle.state.kas_usd;
        let next = (cur * 10_000 + 12_499) / 12_500;
        w.update(next).expect("Update des Angreifers");
        let o = w.feed.oracle.state;
        let minutes = (w.sim.daa - start) as f64 / 600.0;
        table.push((n, minutes, o.kas_usd, o.ref_kas_usd));
        if o.ref_kas_usd < PRICE && first_fake_ref.is_none() {
            first_fake_ref = Some(minutes);
        }
        // 200 % → < 150 % heißt Preis < 0,75 · ehrlich, zu BEIDEN Preisen
        if o.ref_kas_usd.max(o.kas_usd) * 4 < PRICE * 3 && liquidatable_200.is_none() {
            liquidatable_200 = Some(minutes);
        }
        w.sim.advance(GAP as u64);
    }
    for (n, m, c, r) in &table {
        println!("Update {n:>2} nach {m:>5.1} min: aktuell {:.5} USD, Referenz {:.5} USD", *c as f64 / 1e8, *r as f64 / 1e8);
    }
    let fr = first_fake_ref.expect("Referenz irgendwann gefälscht");
    let lq = liquidatable_200.expect("irgendwann liquidierbar");
    println!("Referenz gefälscht ab {fr:.1} min, Vault mit 200 % liquidierbar ab {lq:.1} min (v4: 13 Updates ohne Wartezeit, < 15 min für alles)");
    // Fenster des Wächters: mindestens ref_after · Mindestabstand = 30 min
    assert!(fr >= 30.0, "{fr}");
    assert!(lq >= 55.0, "{lq}");
}

#[test]
fn waechter_sperrt_dieb_kommt_nicht_mehr_durch_notfallsatz_tauscht_aus() {
    let mut w = setup(1, 1);
    let net = w.sim.params.clone();
    w.sim.advance(GAP as u64);
    let honest = w.feed.oracle.state.kas_usd;
    w.update(honest * 4 / 5 + 1).expect("Dieb: erstes gefälschtes Update");
    // Wächter 1 sperrt sofort (kein Mindestalter)
    let (b, f) = v5::guard_lock(&w.feed, 1, &w.guards[1], &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).expect("Sperre sofort nach dem Update");
    println!("Sperre: {}", fee(&b));
    w.feed = f;
    assert_eq!(w.feed.oracle.state.ref_kas_usd, honest, "Referenz bleibt ehrlich");
    // Dieb: weitere Updates gehen nicht (Bibliothek und Vertrag, siehe v5_register_tests)
    w.sim.advance(GAP as u64);
    assert!(w.update(honest / 2).is_err());
    // Orakel friert nach 2 h ein (jeder)
    let d0 = w.feed.oracle.state.oracle_daa as u64;
    if w.sim.daa < d0 + 2 * HOUR_DAA as u64 + 1 {
        w.sim.advance(d0 + 2 * HOUR_DAA as u64 + 1 - w.sim.daa);
    }
    let (b, f) = v5::freeze(&w.feed, w.sim.daa - 1, &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.feed = f;
    // Notfallsatz kündigt einen neuen Satz an – ohne 30 Tage Stille
    let new_keys = [key(50), key(51), key(52)];
    let new_set = SignerSet { keys: new_keys.iter().map(xonly).collect(), t: 2, t_rot: 2 };
    let new_guards = GuardSet { keys: vec![xonly(&key(60))] };
    let fbs = w.feed.fallback_set.clone();
    let (b, f, ticket) = v5::propose(&w.feed, true, &[(0, w.fb)], &new_set, fbs.as_ref(), Some(&new_guards), w.sim.daa - 1, &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).expect("Notfall-Ankündigung unter Sperre");
    println!("Notfall-Ankündigung: {}", fee(&b));
    w.feed = f;
    // zu früh: der Konsens lehnt ab
    w.sim.advance(7 * DAY_DAA as u64 - 100);
    let (b, _) = v5::activate(&w.feed, &ticket, &new_set, fbs.as_ref(), Some(&new_guards), &w.sim.funds(&w.payer), &net).unwrap();
    assert!(w.sim.submit(&b).unwrap_err().contains("relative Sperre"));
    w.sim.advance(100);
    let (b, f) = v5::activate(&w.feed, &ticket, &new_set, fbs.as_ref(), Some(&new_guards), &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).expect("Aktivierung nach 7 Tagen");
    println!("Aktivierung: {}", fee(&b));
    w.feed = f;
    assert!(!w.feed.register.state.locked);
    // neuer Satz (2 von 3) taut auf und führt den Preis zurück
    w.signers = new_keys.to_vec();
    w.sim.advance(GAP as u64);
    let cur = w.feed.oracle.state.kas_usd;
    w.update(cur).expect("erstes Update des neuen Satzes");
    assert!(!w.feed.oracle.state.frozen);
}

#[test]
fn massen_und_gebuehren_v5() {
    for (n, t) in [(1usize, 1i64), (3, 2), (7, 4)] {
        let mut w = setup(n, t);
        w.sim.advance(GAP as u64);
        let p = w.feed.oracle.state.kas_usd;
        let b = w.update(p).expect("Update");
        println!("Update {n} Schlüssel / {t} Sig.: {}", fee(&b));
        assert!(b.fee < 3 * E8 / 100, "Update unter 0,03 KAS");
        assert!(b.storage_mass < 100_000);
    }
    let mut w = setup(1, 1);
    let net = w.sim.params.clone();
    w.sim.advance(GAP as u64);
    let new_set = SignerSet { keys: vec![xonly(&key(70))], t: 1, t_rot: 1 };
    let (b, f, _) = v5::propose(&w.feed, false, &w.signers_of(), &new_set, None, None, 0, &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).unwrap();
    println!("Ankündigung (Payload 96 B, Ticket 1 KAS): {}", fee(&b));
    w.feed = f;
    w.sim.advance(GAP as u64);
    let (b, f) = v5::cancel(&w.feed, &w.signers_of(), &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).unwrap();
    println!("Absage: {}", fee(&b));
    w.feed = f;
}

#[test]
fn absage_ohne_mindestalter_lehnt_der_konsens_ab() {
    // Dieb versucht, die Haupt-UTXO mit einer Kette von Absagen zu belegen,
    // damit die Sperre nicht durchkommt: jede Absage braucht 5 min Alter
    let mut w = setup(1, 1);
    let net = w.sim.params.clone();
    w.sim.advance(GAP as u64);
    let (b, f) = v5::cancel(&w.feed, &w.signers_of(), &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.feed = f;
    let (b, _) = v5::cancel(&w.feed, &w.signers_of(), &w.sim.funds(&w.payer), &net).unwrap();
    assert!(w.sim.submit(&b).unwrap_err().contains("relative Sperre"));
    // die Sperre des Wächters geht sofort
    let (b, _) = v5::guard_lock(&w.feed, 0, &w.guards[0], &w.sim.funds(&w.payer), &net).unwrap();
    w.sim.submit(&b).expect("Sperre ohne Wartezeit");
}
