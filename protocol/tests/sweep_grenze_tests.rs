//! Audit 12 A12-2: Auflösen (sweep) kleiner Zombie-Vaults im Simulator.
//! Ausgabe der Messwerte: cargo test --release --test sweep_grenze_tests -- --nocapture

use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::{self, E8};
use kaspa_lending_protocol::ops::{self, Deployment, Funds, p2pk_spk, xonly};
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
}

fn setup() -> World {
    let mut sim = Sim::new();
    let deployer = key();
    let committee: Vec<Keypair> = (0..5).map(|_| key()).collect();
    sim.faucet(&deployer, 1_000 * E8 as u64);
    let net = sim.params.clone();
    let feed = sim.deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: committee.iter().map(xonly).collect(), t: 3, t_rot: 3 }, 4_000_000, 158_548_959, 1_000_000_000).expect("Register und Orakel");
    let op = feed.oracle_params.clone();
    let oracle_t = feed.oracle.clone();
    let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
    let (b, factory_t) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).expect("Factory-Genesis baut");
    sim.submit(&b).expect("Factory-Genesis angenommen");
    let (b, f2, root, vp) =
        ops::init_factory(&op, &oracle_t, &fp, &factory_t, &deployer, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &net).expect("Init baut");
    sim.submit(&b).expect("Init angenommen");
    let dep = Deployment {
        network: "sim".into(),
        register_params: feed.register_params.clone(), register: feed.register.clone(), signer_set: feed.signer_set.clone(), fallback_set: None, rotation: None, old_tickets: vec![], foreign_change: None, signers_unknown: false, oracle_params: op,
        oracle: oracle_t,
        factory_params: fp,
        factory: f2,
        ghost_root: Some(root),
        vault_params: Some(vp),
        vaults: vec![],
        tokens: vec![],
        pool: None,
        pool_pending: None,
        lp_tokens: vec![],
        pool_unresolved: None,
    };
    World { sim, deployer, committee, dep }
}

impl World {
    fn apply(&mut self, label: &str, r: Result<(Built, Deployment), String>) {
        let (b, d) = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.dep = d;
    }
    fn oracle(&mut self, kas_usd: i64, rate: i64, daa: u64) {
        let signers: Vec<(usize, Keypair)> = [0usize, 2, 4].iter().map(|&i| (i, self.committee[i])).collect();
        self.sim.advance(daa);
        let at = self.sim.daa - 1;
        let net = self.sim.params.clone();
        let r = ops::oracle_update(&self.dep, &signers, kas_usd, rate, at, &self.sim.funds(&self.deployer), &net);
        self.apply("Orakel-Update", r);
    }
}

/// Zombie-Vaults wie in der Nachprüfung (review12_sweep.rs): prägen, ein Jahr
/// Höchstsatz, Schuld tilgen (der Zins bleibt), Sicherheit auf `targets`
/// senken, dann zwei Preishalbierungen – danach zehrt der Zins jeden auf.
fn zombies(w: &mut World, targets: &[u64]) -> Vec<Keypair> {
    let net = w.sim.params.clone();
    let mut users = vec![];
    for &t in targets {
        let user = key();
        w.sim.faucet(&user, 1_000 * E8 as u64);
        let r = ops::open_vault(&w.dep, &xonly(&user), 100 * E8 as u64, &w.sim.funds(&user), &net);
        w.apply("Vault", r);
        let i = w.dep.vaults.iter().position(|v| v.owner == xonly(&user)).unwrap();
        // 0,04·T GHOST: nach einem Jahr Höchstsatz trägt T den Zins noch bei
        // 200 %, nach zwei Preishalbierungen nicht mehr
        let r = ops::mint(&w.dep, i, &user, (t as i128 * 4 / 100) as i64, &xonly(&user), &w.sim.funds(&user), &net);
        w.apply("Prägen", r);
        users.push(user);
    }
    let max_rate = w.dep.oracle_params.max_rate;
    w.oracle(4_000_000, max_rate, 700);
    w.oracle(4_000_000, max_rate, 315_360_000);
    for (u, &t) in users.iter().zip(targets) {
        let i = w.dep.vaults.iter().position(|v| v.owner == xonly(u)).unwrap();
        let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, x)| x.state.owner == xonly(u)).map(|(i, _)| i).collect();
        let debt = w.dep.vaults[i].vault.state.debt;
        let r = ops::repay(&w.dep, i, u, &toks, debt, &w.sim.funds(u), &net);
        w.apply("Tilgen", r);
        let i = w.dep.vaults.iter().position(|v| v.owner == xonly(u)).unwrap();
        let r = ops::withdraw(&w.dep, i, u, t, &p2pk_spk(&xonly(u)), &w.sim.funds(u), &net);
        w.apply(&format!("Senken auf {:.4} KAS", t as f64 / 1e8), r);
    }
    let rate = w.dep.oracle.state.stable_rate;
    w.oracle(2_000_000, rate, 700);
    w.oracle(1_000_000, rate, 700);
    users
}

/// Geldbörsen des Auslösers: keine KAS, eine große UTXO, acht kleine
fn wallets(w: &mut World) -> Vec<(&'static str, Funds)> {
    let big = key();
    w.sim.faucet(&big, 1_000 * E8 as u64);
    let small = key();
    for _ in 0..8 {
        w.sim.faucet(&small, 30_000_000);
    }
    // eine UTXO, bei der das Wechselgeld knapp über MIN_CHANGE (0,2 KAS) landet:
    // kleiner Wechselgeld-Ausgang, kaum Entlastung durch die Eingänge
    let tight = key();
    w.sim.faucet(&tight, 15_500_000);
    vec![
        ("ohne KAS", w.sim.funds(&key())),
        ("1 000 KAS", w.sim.funds(&big)),
        ("8 × 0,3 KAS", w.sim.funds(&small)),
        ("0,155 KAS", w.sim.funds(&tight)),
    ]
}

/// A12-2, Messung: Ab welcher Sicherheit lässt sich die Auflösung bauen?
/// Bisektion (auf 1 000 sompi) über den Wert des Vault-Eingangs; der Vertrag
/// nimmt jede Sicherheit an, deren Zins sie aufzehrt, die Grenze ist allein die
/// Speichermasse des Kassen-Ausgangs (Blockgrenze 500 000 g). Gemessen
/// 29.09.2026: baubar ab 0,1165 KAS (8 × 0,3 KAS) bis 0,1213 KAS (0,155 KAS,
/// Wechselgeld knapp über 0,2 KAS) Sicherheit, also ab ≈ 0,016–0,021 KAS an
/// die Kasse. math::sweepable verlangt 0,025 KAS (SWEEP_MIN_TREASURY), dort
/// höchstens 429 551 g: für jede dieser Geldbörsen auf der sicheren Seite.
#[test]
fn a12_messung_kleinste_aufloesbare_sicherheit() {
    let mut w = setup();
    let us = zombies(&mut w, &[30_000_000]);
    let i = w.dep.vaults.iter().position(|v| v.owner == xonly(&us[0])).unwrap();
    let net = w.sim.params.clone();
    let floor = (math::SWEEP_FEE + math::SWEEP_MIN_TREASURY) as u64;
    for (name, f) in wallets(&mut w) {
        let builds = |coll: u64| {
            // nur bauen (Skripte, Masse, Gebühr), nicht einreichen: Wert und
            // Zins des Vault-Eingangs frei gewählt, der Zins zehrt jede Sicherheit auf
            let mut d = w.dep.clone();
            d.vaults[i].vault.value = coll;
            d.vaults[i].vault.state.interest = 1_000 * E8;
            ops::sweep(&d, i, &f, &net).map(|(b, _)| b.storage_mass)
        };
        let at_floor = builds(floor);
        assert!(at_floor.is_ok(), "{name}: an der Grenze von math::sweepable muss es gehen: {at_floor:?}");
        let (mut lo, mut hi) = (math::SWEEP_FEE as u64, floor);
        while hi - lo > 1_000 {
            let mid = (lo + hi) / 2;
            if builds(mid).is_ok() { hi = mid } else { lo = mid }
        }
        println!(
            "{name:<12}: baubar ab ≈ {hi} sompi ({:.5} KAS, Kasse {:.5} KAS); an der Grenze {:.3} KAS: storage {at_floor:?} g; {lo} sompi: {:?}",
            hi as f64 / 1e8,
            (hi - math::SWEEP_FEE as u64) as f64 / 1e8,
            floor as f64 / 1e8,
            builds(lo)
        );
        assert!(builds(lo).is_err() && lo > math::SWEEP_FEE as u64, "{name}: unterhalb der Messung nicht baubar");
        // bis SWEEP_FEE gibt es keinen Kassen-Ausgang: Fehler statt u64-Unterlauf
        for coll in [1, 9_000_000, math::SWEEP_FEE as u64] {
            let e = builds(coll).unwrap_err();
            assert!(e.contains("zu klein zum Auflösen"), "{name}, {coll} sompi: {e}");
        }
    }
}

/// A12-2: Ein Zombie-Vault unter 0,1 KAS ließ ops::sweep überlaufen
/// (`value − SWEEP_FEE` in u64), und der Keeper versuchte immer nur den
/// ersten auflösbaren Vault. So blieben alle anderen liegen. Jetzt gelten nur
/// baubare Vaults als auflösbar, und der Keeper nimmt sie der Reihe nach.
#[test]
fn a12_kleiner_zombie_blockiert_das_aufloesen_nicht() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let targets: [u64; 5] = [9_000_000, 11_000_000, 12_500_000, 20_000_000, 30_000_000];
    let users = zombies(&mut w, &targets);
    let (price, index) = (w.dep.oracle.state.kas_usd, w.dep.oracle.state.stable_index);
    let idx = |w: &World, u: &Keypair| w.dep.vaults.iter().position(|v| v.owner == xonly(u)).unwrap();
    for (u, &t) in users.iter().zip(&targets) {
        let v = &w.dep.vaults[idx(&w, u)].vault;
        assert_eq!(v.value, t);
        assert!(math::sweep_allowed(t as i64, &v.state, price, index), "{t}: der Vertrag ließe es zu");
        assert_eq!(math::sweepable(t as i64, &v.state, price, index), t >= 12_500_000, "{t} sompi");
    }
    let keeper = key();
    w.sim.faucet(&keeper, 1_000 * E8 as u64);
    // die beiden kleinen: sauberer Fehler, kein Überlauf
    let e = ops::sweep(&w.dep, idx(&w, &users[0]), &w.sim.funds(&keeper), &net).unwrap_err();
    assert!(e.contains("zu klein zum Auflösen"), "{e}");
    let e = ops::sweep(&w.dep, idx(&w, &users[1]), &w.sim.funds(&keeper), &net).unwrap_err();
    assert!(e.contains("zu groß für einen Block"), "{e}");
    // Kandidaten des Keepers: ohne die beiden kleinen, in Reihenfolge der Vaults
    let list = ops::sweep_candidates(&w.dep, price);
    let want: Vec<usize> = users[2..].iter().map(|u| idx(&w, u)).collect();
    assert_eq!(list, want);
    // liegt der Marktpreis weit darüber, löst der Keeper nichts auf (A11-V-4)
    assert!(ops::sweep_candidates(&w.dep, price * 100).is_empty());
    // wie keeper_round: je Runde einer, bis keiner mehr übrig ist
    let mut rounds = 0;
    while let Some(&i) = ops::sweep_candidates(&w.dep, price).first() {
        let coll = w.dep.vaults[i].vault.value;
        let treasury = w.sim.balance(&w.deployer);
        let (b, d) = ops::sweep(&w.dep, i, &w.sim.funds(&keeper), &net).expect("Kandidat baut");
        w.sim.submit(&b).expect("Kandidat angenommen");
        w.dep = d;
        assert_eq!(w.sim.balance(&w.deployer) - treasury, coll - math::SWEEP_FEE as u64);
        println!("Vault mit {:.3} KAS aufgelöst: storage {} g, Gebühr {:.4} KAS", coll as f64 / 1e8, b.storage_mass, b.fee as f64 / 1e8);
        rounds += 1;
    }
    assert_eq!(rounds, 3);
    // die beiden kleinen bleiben liegen, niemand muss sie anfassen
    assert_eq!(w.dep.vaults.iter().filter(|v| v.vault.value < 12_500_000).count(), 2);
}

/// Restpunkt B-P1 (A12-2, Text): Die Meldung bei zu kleiner Sicherheit nannte
/// SWEEP_FEE „die Netzgebühr des Auflösens“. Die Netzgebühr ist aber nur ein
/// Teil davon (≈ 0,055 KAS), den Rest bekommt, wer auflöst – wie auf der Seite.
#[test]
fn a12_bp1_zu_klein_meldung_nennt_sweep_fee_nicht_netzgebuehr() {
    let mut w = setup();
    let us = zombies(&mut w, &[9_000_000]);
    let i = w.dep.vaults.iter().position(|v| v.owner == xonly(&us[0])).unwrap();
    let keeper = key();
    w.sim.faucet(&keeper, 10 * E8 as u64);
    let net = w.sim.params.clone();
    let e = ops::sweep(&w.dep, i, &w.sim.funds(&keeper), &net).unwrap_err();
    assert!(e.contains("zu klein zum Auflösen: 0.09000000 KAS Sicherheit"), "{e}");
    assert!(!e.contains("Netzgebühr des Auflösens"), "SWEEP_FEE ist nicht die Netzgebühr: {e}");
    assert!(e.contains("0.1 KAS für das Auflösen ab") && e.contains("den Rest bekommt, wer auflöst"), "{e}");
    assert!(e.contains("für die Zinskasse bliebe nichts"), "{e}");
}

/// A12-14, Nachprüfung: An der Rechengrenze des Vertrags (⌈Zins·1e8/Preis⌉ >
/// i64::MAX, beim Tiefstpreis 0,00001 USD ab ≈ 922 000 USD Zins) meldet
/// ops::close den Grund, statt eine Tx zu bauen, die an NumberTooBig scheitert.
/// Genau an der Grenze geht Schließen noch: die ganze Sicherheit an die Kasse.
#[test]
fn a12_schliessen_an_der_rechengrenze_meldet_den_grund() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let user = key();
    w.sim.faucet(&user, 1_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&user), 100 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault", r);
    let i = w.dep.vaults.iter().position(|v| v.owner == xonly(&user)).unwrap();
    let coll = w.dep.vaults[i].vault.value;
    // nur bauen: Tiefstpreis und Zins im Abbild frei gesetzt
    let at = |interest: i64| {
        let mut d = w.dep.clone();
        d.oracle.state.kas_usd = 1_000;
        let index = d.oracle.state.stable_index;
        d.vaults[i].vault.state.interest = interest;
        d.vaults[i].vault.state.index_at = index;
        d
    };
    let to = p2pk_spk(&xonly(&user));
    let edge = i64::MAX / 100_000;
    for interest in [edge + 1, 1_000_000 * E8] {
        let e = ops::close(&at(interest), i, &user, &to, &w.sim.funds(&user), &net).map(|_| ()).unwrap_err();
        assert!(e.contains("zu groß für die Rechnung des Vertrags"), "Zins {interest}: {e}");
    }
    let d = at(edge);
    assert_eq!(ops::close_fee(&d, i), coll as i64);
    let (b, _) = ops::close(&d, i, &user, &to, &w.sim.funds(&user), &net).expect("an der Grenze baut Schließen");
    assert_eq!(b.tx.outputs[1].value, coll, "die ganze Sicherheit an die Zinskasse");
}
