//! Version 4 über ops (wie ghostctl sie baut) auf der Simulationskette:
//! Preis-Update über das Register, Einfrieren und Auftauen, Austausch der
//! Unterzeichner mit Wartezeit (relative Sperre wie im Konsens), Absage,
//! Notfallweg und das Nachführen von Register und Orakel über die Kette.

use kaspa_lending_protocol::chain::{self, MemChain, Shapes, TxView};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::ops::{self, Deployment, p2pk_spk, xonly};
use kaspa_lending_protocol::sim::{Sim, test_register_params};
use kaspa_lending_protocol::txb::Built;
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Secp256k1, SecretKey};

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

struct World {
    sim: Sim,
    deployer: Keypair,
    committee: Vec<Keypair>,
    dep: Deployment,
    log: Vec<TxView>,
}

/// Wartezeiten der Testparameter (sim::test_register_params): je 1 h
const WAIT: u64 = 36_000;

impl World {
    /// 3-von-5-Satz, Zinsschritt wie im echten Deployment (0,5 Punkte, 1 h)
    fn new() -> Self {
        Self::with_set(5, 3, 4)
    }
    /// Satz mit `n` Schlüsseln, Preis-Schwelle `t`, Austausch-Schwelle `t_rot`
    fn with_set(n: usize, t: i64, t_rot: i64) -> Self {
        let mut sim = Sim::new();
        let deployer = key();
        let committee: Vec<Keypair> = (0..n).map(|_| key()).collect();
        sim.faucet(&deployer, 1_000 * E8 as u64);
        let net = sim.params.clone();
        let set = SignerSet { keys: committee.iter().map(xonly).collect(), t, t_rot };
        let feed = sim.deploy_feed_with(&deployer, test_register_params(&deployer), set, 4_000_000, 0, (634_195_839, 15_854_897, WAIT as i64)).expect("Register und Orakel");
        let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
        let (b, factory) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
        sim.submit(&b).unwrap();
        let (b, f2, root, vp) = ops::init_factory(
            &feed.oracle_params,
            &feed.oracle,
            &fp,
            &factory,
            &deployer,
            (20_000, 15_000, 1_000, PROBE_MAX_DEBT),
            spk_bytes(&p2pk_spk(&xonly(&deployer))),
            100_000_000,
            &sim.funds(&deployer),
            &net,
        )
        .unwrap();
        sim.submit(&b).unwrap();
        let mut dep = feed.deployment(fp, f2);
        dep.ghost_root = Some(root);
        dep.vault_params = Some(vp);
        World { sim, deployer, committee, dep, log: vec![] }
    }
    fn apply(&mut self, label: &str, r: Result<(Built, Deployment), String>) {
        let (b, d) = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.log.push(TxView::from(&b.tx));
        self.dep = d;
    }
    fn update(&self, keys: &[Keypair], kas_usd: i64, rate: i64) -> Result<(Built, Deployment), String> {
        let signers = ops::signers_of(&self.dep.signer_set, keys);
        ops::oracle_update(&self.dep, &signers, kas_usd, rate, self.sim.daa - 1, &self.sim.funds(&self.deployer), &self.sim.params)
    }
    fn price(&mut self, kas_usd: i64) {
        self.sim.advance(700);
        let r = self.update(&self.committee[..3.min(self.committee.len())].to_vec(), kas_usd, self.dep.oracle.state.stable_rate);
        self.apply("Preis-Update", r);
    }
    fn funds(&self) -> ops::Funds {
        self.sim.funds(&self.deployer)
    }
}

#[test]
fn preis_nur_mit_quorum_des_aktuellen_satzes() {
    let mut w = World::new();
    w.sim.advance(700);
    let e = w.update(&w.committee[..2].to_vec(), 4_100_000, 0).err().expect("2 von 3 reichen nicht");
    assert!(e.contains("3 Unterschrift(en) nötig"), "{e}");
    let strangers: Vec<Keypair> = (0..3).map(|_| key()).collect();
    assert!(w.update(&strangers, 4_100_000, 0).is_err(), "fremde Schlüssel");
    let r = w.update(&w.committee[2..5].to_vec(), 4_100_000, 0);
    w.apply("Update mit Schlüsseln 2, 3, 4", r);
    assert_eq!(w.dep.oracle.state.kas_usd, 4_100_000);
    assert_eq!(w.dep.oracle.state.seq, 1);
    assert_eq!(w.dep.register.state.last_daa, w.dep.oracle.state.oracle_daa, "Register merkt sich den Preis-DAA");
    // Vertragsgrenzen vorab mit lesbarer Meldung
    w.sim.advance(700);
    let e = w.update(&w.committee[..3].to_vec(), 9_000_000, 0).err().unwrap();
    assert!(e.contains("×2/÷2"), "{e}");
    let e = w.update(&w.committee[..3].to_vec(), 4_100_000, 2 * 15_854_896).err().unwrap();
    assert!(e.contains("Zinsschritt zu groß") || e.contains("erst ab DAA"), "{e}");
}

#[test]
fn zins_hoechstens_ein_schritt_je_stunde() {
    let mut w = World::new();
    w.sim.advance(WAIT + 10);
    let step = 15_854_896;
    let r = w.update(&w.committee[..3].to_vec(), 4_000_000, step);
    w.apply("Zins +0,5 Punkte", r);
    assert_eq!(w.dep.oracle.state.last_rate_daa, w.dep.oracle.state.oracle_daa);
    w.sim.advance(700);
    let e = w.update(&w.committee[..3].to_vec(), 4_000_000, 2 * step).err().unwrap();
    assert!(e.contains("erst ab DAA"), "{e}");
    w.sim.advance(WAIT);
    let r = w.update(&w.committee[..3].to_vec(), 4_000_000, 2 * step);
    w.apply("nach einer Stunde der nächste Schritt", r);
}

#[test]
fn einfrieren_sperrt_und_preis_taut_auf() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let user = key();
    w.sim.faucet(&user, 5_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&user), 1_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault eröffnen", r);
    let r = ops::mint(&w.dep, 0, &user, 2 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("2 GHOST prägen", r);

    let after = w.dep.oracle_params.freeze_after_daa as u64;
    let e = ops::oracle_freeze(&w.dep, w.sim.daa - 1, &w.funds(), &net).err().unwrap();
    assert!(e.contains("Einfrieren erst ab"), "{e}");
    w.sim.advance(after + 100);
    // jeder darf einfrieren, auch ein Fremder
    let r = ops::oracle_freeze(&w.dep, w.sim.daa - 1, &w.sim.funds(&user), &net);
    w.apply("Einfrieren", r);
    assert!(w.dep.oracle.state.frozen);

    let e = ops::mint(&w.dep, 0, &user, E8, &xonly(&user), &w.sim.funds(&user), &net).err().unwrap();
    assert!(e.contains("eingefroren"), "{e}");
    let e = ops::withdraw(&w.dep, 0, &user, 900 * E8 as u64, &p2pk_spk(&xonly(&user)), &w.sim.funds(&user), &net).err().unwrap();
    assert!(e.contains("eingefroren"), "{e}");
    let toks: Vec<usize> = (0..w.dep.tokens.len()).collect();
    for (what, r) in [
        ("Einlösen", ops::redeem(&w.dep, 0, &user, &toks, E8, &w.sim.funds(&user), &net)),
        ("Liquidieren", ops::liquidate(&w.dep, 0, &user, &toks, E8, &w.sim.funds(&user), &net)),
        ("Auflösen", ops::sweep(&w.dep, 0, &w.sim.funds(&user), &net)),
    ] {
        let e = r.err().unwrap_or_else(|| panic!("{what} muss gesperrt sein"));
        assert!(e.contains("eingefroren"), "{what}: {e}");
    }
    assert!(ops::sweep_candidates(&w.dep, 4_000_000).is_empty());
    // Gesperrt ist nur, was den Preis braucht: Einzahlen und Tilgen gehen
    let r = ops::deposit(&w.dep, 0, &user, 10 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Einzahlen trotz Einfrieren", r);
    let toks: Vec<usize> = (0..w.dep.tokens.len()).collect();
    let r = ops::repay(&w.dep, 0, &user, &toks, E8, &w.sim.funds(&user), &net);
    w.apply("Tilgen trotz Einfrieren", r);
    // Gegenprobe ohne Vorprüfung: mit vorgetäuschtem frischem Orakel baut die
    // Tx zwar, die Kette kennt diese Orakel-UTXO aber nicht
    let mut fake = w.dep.clone();
    fake.oracle.state.frozen = false;
    let (b, _) = ops::mint(&fake, 0, &user, E8, &xonly(&user), &w.sim.funds(&user), &net).expect("baut lokal");
    assert!(w.sim.submit(&b).is_err(), "abgelehnt");

    w.price(4_000_000);
    assert!(!w.dep.oracle.state.frozen, "Preis taut auf");
    let r = ops::mint(&w.dep, 0, &user, E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("Prägen nach dem Auftauen", r);
}

#[test]
fn austausch_erst_nach_der_wartezeit() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let solo = key();
    let new_set = SignerSet { keys: vec![xonly(&solo)], t: 1, t_rot: 1 };
    // Austausch-Schwelle 4: drei Schlüssel reichen nicht
    let e = ops::propose(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[..3]), &new_set, None, false, w.sim.daa - 1, &w.funds(), &net).err().unwrap();
    assert!(e.contains("4 Unterschrift(en) nötig"), "{e}");
    let (b, d) = ops::propose(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[..4]), &new_set, None, false, w.sim.daa - 1, &w.funds(), &net).unwrap();
    // die Ankündigung steht im Payload der Tx (öffentlich)
    let ann = announcement(&new_set.hash(), &[0; 32], w.dep.register.state.pay_kind, &w.dep.register.state.pay_to.clone().try_into().unwrap());
    assert_eq!(b.tx.payload, ann);
    w.apply("Ankündigung", Ok((b, d)));
    let rot = w.dep.rotation.clone().expect("Ticket");
    assert!(rot.ticket.state.ticket);
    assert_eq!(rot.ticket.state.set_hash, new_set.hash().to_vec());

    // eine DAA zu früh: der Node (hier der Simulator) lehnt die relative Sperre ab
    w.sim.advance(WAIT - 20);
    let (b, _) = ops::activate_rotation(&w.dep, &w.funds(), &net).expect("baut (Skript sieht nur die Sequenz)");
    let e = w.sim.submit(&b).err().expect("zu früh");
    assert!(e.contains("relative Sperre"), "{e}");
    // Preis-Updates des alten Satzes laufen in der Wartezeit weiter
    w.price(4_100_000);
    w.sim.advance(40);
    let r = ops::activate_rotation(&w.dep, &w.funds(), &net);
    w.apply("Aktivieren nach der Wartezeit", r);
    assert_eq!(w.dep.signer_set, new_set);
    assert!(w.dep.rotation.is_none());
    assert_eq!(w.dep.register.state.set_hash, new_set.hash().to_vec());

    // alter Satz kann nicht mehr, neuer allein schon
    w.sim.advance(700);
    assert!(w.update(&w.committee.clone(), 4_200_000, 0).is_err(), "alter Satz");
    let r = w.update(&[solo], 4_200_000, 0);
    w.apply("Update durch den neuen Satz", r);
}

#[test]
fn absage_entwertet_das_ticket() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let thief = key();
    let set = SignerSet { keys: vec![xonly(&thief)], t: 1, t_rot: 1 };
    let r = ops::propose(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[1..5]), &set, None, false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Ankündigung", r);
    let ticket = w.dep.rotation.clone().unwrap().ticket;
    let r = ops::cancel_rotation(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[..3]), &w.funds(), &net);
    w.apply("Absage", r);
    assert!(w.dep.rotation.is_none());
    assert_eq!(w.dep.old_tickets.len(), 1, "Ticket bleibt zum Aufräumen vorgemerkt (Audit 14 N-1)");
    w.sim.advance(WAIT * 2);
    let e = ops::activate_rotation(&w.dep, &w.funds(), &net).err().unwrap();
    assert!(e.contains("keine Ankündigung offen"), "{e}");
    // auch ohne Vorprüfung: das echte Ticket trägt die alte nonce, ein Ticket
    // mit passender nonce gibt es auf der Kette nicht
    let mut fake = w.dep.clone();
    let mut tk = ticket.clone();
    tk.state.nonce = fake.register.state.nonce;
    fake.rotation = Some(ops::Rotation { ticket: tk, set: set.clone(), fallback: None, emergency: false, ready_daa: 0 });
    if let Ok((b, _)) = ops::activate_rotation(&fake, &w.funds(), &net) {
        assert!(w.sim.submit(&b).is_err(), "abgelehnt");
    }
    // aufräumen: die KAS des Tickets kommen zurück
    let before = w.sim.balance(&w.deployer);
    let r = ops::clear_ticket(&w.dep, &ticket, &w.funds(), &net);
    w.apply("Ticket aufräumen", r);
    assert!(w.sim.balance(&w.deployer) > before + ops::TICKET_VALUE / 2);
    assert!(w.dep.old_tickets.is_empty());
    assert_eq!(w.dep.signer_set.keys, w.committee.iter().map(xonly).collect::<Vec<_>>());
}

#[test]
fn notfallweg_nur_nach_stille_und_verfaellt_durch_preis() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let fb: Vec<Keypair> = (0..3).map(|_| key()).collect();
    let fb_set = SignerSet { keys: fb.iter().map(xonly).collect(), t: 2, t_rot: 2 };
    // 1. Notfallsatz per regulärem Austausch festlegen (Satz bleibt)
    let same = w.dep.signer_set.clone();
    let r = ops::propose(&w.dep, &ops::signers_of(&same, &w.committee[..4]), &same, Some(&fb_set), false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Notfallsatz ankündigen", r);
    w.sim.advance(WAIT + 10);
    let r = ops::activate_rotation(&w.dep, &w.funds(), &net);
    w.apply("Notfallsatz aktiv", r);
    assert_eq!(w.dep.fallback_set.as_ref(), Some(&fb_set));
    w.price(4_000_000);

    // 2. zu früh nach dem letzten Preis
    let rescue = key();
    let rescue_set = SignerSet { keys: vec![xonly(&rescue)], t: 1, t_rot: 1 };
    let e = ops::propose(&w.dep, &ops::signers_of(&fb_set, &fb), &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net).err().unwrap();
    assert!(e.contains("Notfallweg erst ab"), "{e}");
    // 3. nach der Stille, aber Orakel nicht eingefroren: abgelehnt (A20a-1)
    w.sim.advance(WAIT + 10);
    let e = ops::propose(&w.dep, &ops::signers_of(&fb_set, &fb), &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net).err().unwrap();
    assert!(e.contains("eingefroren"), "{e}");
    // eingefroren: Notfall-Ankündigung, ein Preis-Update entwertet sie
    w.sim.advance(w.dep.oracle_params.freeze_after_daa as u64);
    let r = ops::oracle_freeze(&w.dep, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Einfrieren", r);
    let r = ops::propose(&w.dep, &ops::signers_of(&fb_set, &fb), &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Notfall-Ankündigung", r);
    assert!(w.dep.register.state.emerg);
    w.price(4_000_000);
    assert!(w.dep.rotation.is_none(), "Preis-Update entwertet das Notfall-Ticket");
    assert!(!w.dep.register.state.emerg);
    // 4. wieder Stille, diesmal bleibt der Hauptsatz still
    w.sim.advance(w.dep.oracle_params.freeze_after_daa as u64 + 10);
    let r = ops::oracle_freeze(&w.dep, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Einfrieren 2", r);
    let r = ops::propose(&w.dep, &ops::signers_of(&fb_set, &fb), &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Notfall-Ankündigung 2", r);
    w.sim.advance(WAIT + 10);
    let r = ops::activate_rotation(&w.dep, &w.funds(), &net);
    w.apply("Notfall-Austausch aktiv", r);
    assert_eq!(w.dep.signer_set, rescue_set);
    assert_eq!(w.dep.fallback_set, None);
    w.sim.advance(700);
    let r = w.update(&[rescue], 4_000_000, 0);
    w.apply("Rettungssatz setzt Preise", r);
}

/// Ein Rechner mit altem Stand führt Register und Orakel über die Kette nach:
/// Preis-Updates, Einfrieren, Ankündigung und Aktivierung
#[test]
fn kette_fuehrt_register_und_orakel_nach() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let old = w.dep.clone();
    w.log.clear();
    w.price(4_100_000);
    w.sim.advance(w.dep.oracle_params.freeze_after_daa as u64 + 10);
    let r = ops::oracle_freeze(&w.dep, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Einfrieren", r);
    let new_set = SignerSet { keys: w.committee[..3].iter().map(xonly).collect(), t: 2, t_rot: 2 };
    let r = ops::propose(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[..4]), &new_set, None, false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Ankündigung", r);
    w.price(4_200_000);
    w.sim.advance(WAIT + 10);
    let r = ops::activate_rotation(&w.dep, &w.funds(), &net);
    w.apply("Aktivieren", r);
    w.sim.advance(700);
    let r = w.update(&w.committee[..2].to_vec(), 4_300_000, 0);
    w.apply("Update durch den neuen Satz", r);

    let sh = Shapes::of(&old);
    let mut h = MemChain { txs: w.log.clone() };
    let (op, value, s) = chain::follow_register(&mut h, &sh, &old.register).expect("Register verfolgbar");
    assert_eq!((op, value, s), (w.dep.register.outpoint, w.dep.register.value, w.dep.register.state.clone()));
    let (op, value, s, updates) = chain::follow_oracle(&mut h, &sh, &old.oracle_params, &old.oracle).expect("Orakel verfolgbar");
    assert_eq!((op, value, s), (w.dep.oracle.outpoint, w.dep.oracle.value, w.dep.oracle.state));
    assert_eq!(updates, 4, "3 Preise und das Einfrieren");
}

#[test]
fn genesis_mit_zweitem_register_ausgang_wird_abgelehnt() {
    let w = World::new();
    let b = ops::deploy_register(&w.dep.register_params, w.dep.register.state.clone(), ops::REGISTER_VALUE, &w.funds(), &w.sim.params).unwrap().0;
    let mut tx = b.tx.clone();
    let cov = tx.outputs[0].covenant.unwrap().covenant_id;
    ops::check_register_genesis(&tx, &cov).expect("genau ein Ausgang");
    let extra = tx.outputs[0].clone();
    tx.outputs.push(extra);
    assert!(ops::check_register_genesis(&tx, &cov).is_err());
}

#[test]
fn satz_grenzen() {
    let rp = RegisterParams { deployer: vec![0; 32], min_signers: 1, min_threshold: 1, rot_delay_daa: 1, emerg_after_daa: 1, emerg_delay_daa: 1 };
    let k = |n: u8| vec![n; 32];
    let ok = |keys: Vec<Vec<u8>>, t, r| SignerSet { keys, t, t_rot: r }.check_bounds(&rp);
    assert!(ok(vec![k(1)], 1, 1).is_ok());
    assert!(ok(vec![k(1), k(2)], 1, 1).is_err(), "2t > n");
    assert!(ok(vec![k(1), k(2), k(3)], 2, 3).is_ok());
    assert!(ok(vec![k(1), k(2), k(3)], 2, 1).is_err(), "tRot ≥ t");
    assert!(ok(vec![k(1), k(1), k(3)], 2, 2).is_err(), "doppelter Schlüssel");
    assert!(ok((1..=10).map(k).collect(), 6, 6).is_err(), "höchstens 9");
    assert!(ok(vec![], 1, 1).is_err());
}

/// Eine zweite Ankündigung ersetzt die erste; deren Ticket bleibt aufräumbar (Audit 14 N-1)
#[test]
fn ersetzte_ankuendigung_bleibt_aufraeumbar() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let a = SignerSet { keys: vec![xonly(&key())], t: 1, t_rot: 1 };
    let b = SignerSet { keys: vec![xonly(&key())], t: 1, t_rot: 1 };
    let signers = ops::signers_of(&w.dep.signer_set, &w.committee[..4]);
    let r = ops::propose(&w.dep, &signers, &a, None, false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Ankündigung A", r);
    let r = ops::propose(&w.dep, &signers, &b, None, false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Ankündigung B ersetzt A", r);
    assert_eq!(w.dep.old_tickets.len(), 1);
    assert_eq!(w.dep.rotation.as_ref().unwrap().set, b);
    let old = w.dep.old_tickets[0].clone();
    let r = ops::clear_ticket(&w.dep, &old, &w.funds(), &net);
    w.apply("Ticket A aufräumen", r);
    w.sim.advance(WAIT + 700);
    let r = ops::activate_rotation(&w.dep, &w.funds(), &net);
    w.apply("B aktivieren", r);
    assert_eq!(w.dep.signer_set, b);
}

/// Zinsziel: Version big-endian wie rusty-kaspa (Audit 14 N-3)
#[test]
fn zinsziel_bytes_wie_der_konsens() {
    use kaspa_consensus_core::tx::ScriptPublicKey;
    let s = ScriptPublicKey::new(0x0102, vec![0xaa, 0xbb].into());
    assert_eq!(spk_bytes(&s), vec![0x01, 0x02, 0xaa, 0xbb]);
    assert_eq!(spk_from_bytes(&spk_bytes(&s)).unwrap(), s);
}

/// Register-Abgleich nach dem Nachführen (store::reconcile_register, Audit 14
/// H-2/M-3, Audit 15 G-6): was ein anderer Rechner oder ein Dritter getan hat
#[test]
fn abgleich_des_registers_nach_aenderungen_von_aussen() {
    use kaspa_lending_protocol::store::reconcile_register;
    let mut w = World::new();
    let net = w.sim.params.clone();
    let solo = key();
    let new_set = SignerSet { keys: vec![xonly(&solo)], t: 1, t_rot: 1 };
    let signers = ops::signers_of(&w.dep.signer_set, &w.committee[..4]);

    // 1. nonce unverändert: nichts zu melden
    let mut d = w.dep.clone();
    assert!(reconcile_register(&mut d, &w.dep.register.state.clone()).is_empty());

    // 2. fremde Ankündigung (anderer Rechner mit denselben Schlüsseln): Alarm
    let mut other = w.dep.clone();
    let (b, od) = ops::propose(&other, &signers, &new_set, None, false, w.sim.daa - 1, &w.funds(), &net).unwrap();
    w.sim.submit(&b).unwrap();
    other = od;
    let mut d = w.dep.clone();
    let before = d.register.state.clone();
    d.register = other.register.clone();
    let notes = reconcile_register(&mut d, &before);
    assert_eq!(d.foreign_change, Some(other.register.state.nonce), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("von außen")));
    assert!(!d.signers_unknown);

    // 3. die eigene Ankündigung (Datei `other`) wird von einem Dritten aktiviert:
    //    neuer Satz übernommen, kein Fehlalarm
    w.sim.advance(WAIT + 700);
    let (b, act) = ops::activate_rotation(&other, &w.funds(), &net).unwrap();
    w.sim.submit(&b).unwrap();
    let mut mine = other.clone();
    let before = mine.register.state.clone();
    mine.register = act.register.clone();
    let notes = reconcile_register(&mut mine, &before);
    assert_eq!(mine.signer_set, new_set, "{notes:?}");
    assert!(mine.rotation.is_none() && mine.foreign_change.is_none() && !mine.signers_unknown);

    // 4. dieselbe Aktivierung, aber die Datei hatte einen anderen Notfallsatz
    //    angekündigt: nicht übernehmen, Preis-Updates sperren (Audit 15 G-6)
    let mut wrong = other.clone();
    wrong.rotation.as_mut().unwrap().fallback = Some(SignerSet { keys: vec![vec![7; 32]], t: 1, t_rot: 1 });
    let before = wrong.register.state.clone();
    wrong.register = act.register.clone();
    reconcile_register(&mut wrong, &before);
    assert!(wrong.signers_unknown);
    assert_ne!(wrong.signer_set, new_set);

    // 5. ein Rechner ohne die Ankündigung sieht einen fremden Satz: gesperrt
    let mut stranger = w.dep.clone();
    let before = stranger.register.state.clone();
    stranger.register = act.register.clone();
    let notes = reconcile_register(&mut stranger, &before);
    assert!(stranger.signers_unknown && notes.iter().any(|n| n.contains("nicht kennt")));
}

/// Eine eigene Ankündigung, die von außen abgesagt wird, wandert zu den
/// aufzuräumenden Tickets; eine durch ein Preis-Update verfallene
/// Notfall-Ankündigung ist kein Alarm (Audit 15)
#[test]
fn abgleich_absage_und_verfallener_notfall() {
    use kaspa_lending_protocol::store::reconcile_register;
    let mut w = World::new();
    let net = w.sim.params.clone();
    let set = SignerSet { keys: vec![xonly(&key())], t: 1, t_rot: 1 };
    let r = ops::propose(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[..4]), &set, None, false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Ankündigung", r);
    // ein anderer Rechner sagt ab
    let (b, cancelled) = ops::cancel_rotation(&w.dep, &ops::signers_of(&w.dep.signer_set, &w.committee[..3]), &w.funds(), &net).unwrap();
    w.sim.submit(&b).unwrap();
    let mut d = w.dep.clone();
    let before = d.register.state.clone();
    d.register = cancelled.register.clone();
    reconcile_register(&mut d, &before);
    assert!(d.rotation.is_none());
    assert_eq!(d.old_tickets.len(), 1, "Ticket bleibt aufräumbar");

    // verfallene Notfall-Ankündigung: emerg vorher an, Preis-Update setzt nonce + 1
    let mut d = w.dep.clone();
    let mut before = d.register.state.clone();
    before.emerg = true;
    d.register.state.nonce = before.nonce + 1;
    d.register.state.emerg = false;
    d.register.state.last_daa = before.last_daa + 700;
    let notes = reconcile_register(&mut d, &before);
    assert!(d.foreign_change.is_none(), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("verfallen")));
}

/// Audit 20, Notfallsatz (Entscheidung des Betreibers) und A20a-1: der ganze
/// Weg von GHOST-Notfallsatz.command mit dem Live-Aufbau (1 von 1, kein
/// Notfallsatz). Ankündigung „gleicher Hauptsatz + neuer Notfallsatz“, vom
/// bisherigen Unterzeichner signiert → Wartezeit (zu früh lehnt die Kette ab)
/// → Aktivieren → Hauptsatz setzt weiter Preise (auch ein Rechner, der die
/// Ankündigung nicht kennt, wie der Server) → Notfallweg nur nach Stille UND
/// bei eingefrorenem Orakel → nach der Notfall-Wartezeit übernimmt der
/// Rettungssatz.
#[test]
fn a20_notfallsatz_nachtragen_und_notfallweg() {
    use kaspa_lending_protocol::store::reconcile_register;
    let mut w = World::with_set(1, 1, 1);
    let net = w.sim.params.clone();
    let main = w.committee[0];
    let server = w.dep.clone();
    assert_eq!(w.dep.fallback_set, None);
    assert_eq!(w.dep.register.state.fb_hash, vec![0u8; 32], "wie live: kein Notfallsatz");

    // 1. Notfall-Schlüssel (committee-keygen --count 1) und Ankündigung mit
    //    unverändertem Hauptsatz (signers propose --same-set --fallback-keys …)
    let cold = key();
    let fb_set = SignerSet { keys: vec![xonly(&cold)], t: 1, t_rot: 1 };
    let same = w.dep.signer_set.clone();
    // der Notfall-Schlüssel allein kann nichts ankündigen
    assert!(ops::propose(&w.dep, &ops::signers_of(&same, &[cold]), &same, Some(&fb_set), false, w.sim.daa - 1, &w.funds(), &net).is_err());
    let r = ops::propose(&w.dep, &ops::signers_of(&same, &[main]), &same, Some(&fb_set), false, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Notfallsatz ankündigen", r);
    let rot = w.dep.rotation.clone().expect("Ankündigung offen");
    assert_eq!(rot.set, same);
    assert_eq!(rot.fallback.as_ref(), Some(&fb_set));

    // 2. Preis-Updates laufen während der Wartezeit weiter (der Agent) – ein
    //    reguläres Ticket verfällt dadurch nicht
    w.price(4_050_000);
    assert!(w.dep.rotation.is_some(), "reguläre Ankündigung bleibt");
    // zu früh: die Kette lehnt ab
    let (b, _) = ops::activate_rotation(&w.dep, &w.funds(), &net).expect("baut");
    assert!(w.sim.submit(&b).is_err(), "Aktivieren vor Ablauf der Wartezeit");

    // 3. nach der Wartezeit aktivieren (darf jeder – hier ein Dritter): die
    //    Datei des Ankündigenden übernimmt den Notfallsatz beim Abgleich, obwohl
    //    der Satz gleich bleibt (vorher galt die Ankündigung dann als abgesagt)
    w.sim.advance(WAIT + 700);
    let stranger = key();
    w.sim.faucet(&stranger, 10 * E8 as u64);
    let (b, act) = ops::activate_rotation(&w.dep, &w.sim.funds(&stranger), &net).expect("baut");
    w.sim.submit(&b).expect("Notfallsatz aktivieren");
    let before = w.dep.register.state.clone();
    w.dep.register = act.register.clone();
    let notes = reconcile_register(&mut w.dep, &before);
    assert!(notes.iter().any(|n| n.contains("aktiviert")), "{notes:?}");
    assert!(w.dep.rotation.is_none() && w.dep.old_tickets.is_empty() && w.dep.foreign_change.is_none(), "{notes:?}");
    assert_eq!(w.dep.signer_set, same, "Hauptsatz unverändert");
    assert_eq!(w.dep.fallback_set.as_ref(), Some(&fb_set));
    assert_eq!(w.dep.register.state.fb_hash, fb_set.hash().to_vec());

    // 4. Hauptsatz setzt weiter Preise; ein Rechner ohne die Ankündigung
    //    (Server-Datei) führt das Register nach und kann ebenfalls updaten
    w.price(4_100_000);
    let mut srv = server.clone();
    let before = srv.register.state.clone();
    srv.register = w.dep.register.clone();
    srv.oracle = w.dep.oracle.clone();
    let notes = reconcile_register(&mut srv, &before);
    assert!(!srv.signers_unknown, "Hauptsatz bekannt: {notes:?}");
    w.sim.advance(700);
    let (b, d) = ops::oracle_update(&srv, &ops::signers_of(&srv.signer_set, &[main]), 4_150_000, srv.oracle.state.stable_rate, w.sim.daa - 1, &w.funds(), &net).expect("Server-Stand baut");
    w.sim.submit(&b).expect("Preis-Update vom Server-Stand");
    assert_eq!(d.fallback_set, None, "der Server kennt nur den Hash des Notfallsatzes");
    // der Mac (dessen Datei die Ankündigung kennt) führt weiter
    let fb_known = w.dep.fallback_set.clone();
    w.dep = d;
    w.dep.fallback_set = fb_known;

    // 5. Notfallweg: nicht ohne Stille, nicht ohne Einfrieren (A20a-1)
    let rescue = key();
    let rescue_set = SignerSet { keys: vec![xonly(&rescue)], t: 1, t_rot: 1 };
    let cold_signers = ops::signers_of(&fb_set, &[cold]);
    let e = ops::propose(&w.dep, &cold_signers, &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net).err().unwrap();
    assert!(e.contains("Notfallweg erst ab"), "{e}");
    w.sim.advance(WAIT + 10);
    let e = ops::propose(&w.dep, &cold_signers, &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net).err().unwrap();
    assert!(e.contains("eingefroren"), "Stille allein reicht nicht: {e}");
    // Gegenprobe: nur das Einfrieren fehlt – mit eingefrorenem Orakel baut sie
    // (das Register selbst prüft das Orakel nicht, deshalb prüft es ops::propose)
    let mut unchecked = w.dep.clone();
    unchecked.oracle.state.frozen = true;
    let (b, _) = ops::propose(&unchecked, &cold_signers, &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net).unwrap_or_else(|e| panic!("ohne Prüfung: {e}"));
    assert!(!b.tx.inputs.is_empty());
    w.sim.advance(w.dep.oracle_params.freeze_after_daa as u64);
    let r = ops::oracle_freeze(&w.dep, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Einfrieren nach der Stille", r);
    let r = ops::propose(&w.dep, &cold_signers, &rescue_set, None, true, w.sim.daa - 1, &w.funds(), &net);
    w.apply("Notfall-Ankündigung", r);
    assert!(w.dep.register.state.emerg);
    // das eingefrorene Orakel kann nur ein echtes Update des Hauptsatzes auftauen;
    // ohne das läuft die Notfall-Wartezeit ab
    w.sim.advance(WAIT + 700);
    let r = ops::activate_rotation(&w.dep, &w.funds(), &net);
    w.apply("Rettungssatz aktivieren", r);
    assert_eq!(w.dep.signer_set, rescue_set);
    // der alte Hauptsatz kann keine Preise mehr setzen, der Rettungssatz schon
    w.sim.advance(700);
    assert!(w.update(&[main], 4_000_000, w.dep.oracle.state.stable_rate).is_err());
    let r = w.update(&[rescue], 4_000_000, w.dep.oracle.state.stable_rate);
    w.apply("Rettungssatz setzt Preise", r);
    assert!(!w.dep.oracle.state.frozen, "Preis taut auf");
}
