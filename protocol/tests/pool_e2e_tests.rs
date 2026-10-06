//! Tauschpool auf der lokalen Simulationskette (src/sim.rs): Anlegen, Tauschen
//! in beide Richtungen, Liquidität ändern, Auflösen – jede Tx mit Masse,
//! Gebühr und Compute-Budget wie im Netz, gebaut von src/pool.rs.
//! Messwerte: cargo test --test pool_e2e_tests -- --nocapture

use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::ops::{p2pk_spk, self, Deployment, xonly};
use kaspa_lending_protocol::pool::{self, POOL_FEE_BPS, Swap};
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

fn report(label: &str, b: &Built) {
    println!(
        "{label:<24} Inputs {:>2} | compute {:>7} g | storage {:>7} g | Gebühr {:.4} KAS",
        b.tx.inputs.len(),
        b.compute_mass,
        b.storage_mass,
        b.fee as f64 / 1e8
    );
}

struct World {
    sim: Sim,
    dep: Deployment,
}

impl World {
    fn apply(&mut self, label: &str, r: Result<(Built, Deployment), String>) {
        let (b, d) = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        report(label, &b);
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.dep = d;
    }
    fn pool(&mut self, label: &str, r: Result<pool::PoolTx, String>) {
        let t = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        report(label, &t.built);
        self.sim.submit(&t.built).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        // Audit 9 P-1: Aus den Signaturskripten jeder Pool-Tx müssen sich die neuen
        // Beträge von Reserve und Minter finden lassen (so bestimmt ghostctl sie
        // nach Aktionen Dritter)
        if let Some(p) = &t.pool {
            let scripts: Vec<Vec<u8>> = t.built.tx.inputs.iter().map(|i| i.signature_script.clone()).collect();
            let c = pool::amount_candidates(&scripts);
            assert!(c.contains(&p.ghost()), "{label}: Reserve {} nicht unter {} Kandidaten", p.ghost(), c.len());
            assert!(c.contains(&p.shares()), "{label}: Anteile {} nicht unter {} Kandidaten", p.shares(), c.len());
        }
        self.dep = pool::apply(&self.dep, &t);
    }
    fn ghost_of(&self, k: &Keypair) -> i64 {
        self.dep.tokens.iter().filter(|t| t.state.owner == xonly(k)).map(|t| t.state.amount).sum()
    }
    fn lp_of(&self, k: &Keypair) -> i64 {
        self.dep.lp_tokens.iter().filter(|t| t.state.owner == xonly(k)).map(|t| t.state.amount).sum()
    }
}

fn setup() -> (World, Keypair) {
    let mut sim = Sim::new();
    let deployer = key();
    let committee: Vec<Keypair> = (0..5).map(|_| key()).collect();
    sim.faucet(&deployer, 1_000 * E8 as u64);
    let net = sim.params.clone();
    let feed = sim.deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: committee.iter().map(xonly).collect(), t: 3, t_rot: 3 }, 4_000_000, 0, 1_000_000_000).expect("Register und Orakel");
    let op = feed.oracle_params.clone();
    let oracle_t = feed.oracle.clone();
    let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
    let (b, factory_t) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let (b, f2, root, vp) = ops::init_factory(&op, &oracle_t, &fp, &factory_t, &deployer, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
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
    let mut w = World { sim, dep };
    // Besitzer: Vault mit 10 000 KAS, 150 GHOST geprägt
    let owner = key();
    w.sim.faucet(&owner, 20_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&owner), 10_000 * E8 as u64, &w.sim.funds(&owner), &net);
    w.apply("Vault eröffnen", r);
    let r = ops::mint(&w.dep, 0, &owner, 150 * E8, &xonly(&owner), &w.sim.funds(&owner), &net);
    w.apply("150 GHOST prägen", r);
    (w, owner)
}

#[test]
fn pool_lebenszyklus_im_simulator() {
    let (mut w, owner) = setup();
    let net = w.sim.params.clone();
    let gcov = w.dep.vault_params.as_ref().unwrap().ghost_cov;

    // 1. Anlegen: Genesis mit 1 KAS, init mit 0,04 GHOST (Kurs 0,04) – beides gesperrt
    // weites Band: dieser Test tauscht bewusst große Beträge (Kursband: eigener Test)
    let r = pool::pool_create(gcov, pool::PoolBand::of(&w.dep, 1_000_000), &owner, &w.sim.funds(&owner), &net);
    w.pool("Pool-Genesis", r);
    let pend = w.dep.pool_pending.clone().expect("angelegt");
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let r = pool::pool_init(&pend, Some(&pool::OracleRef::of(&w.dep)), &owner, &mine, 4_000_000, &w.sim.funds(&owner), &net);
    w.pool("Pool-init", r);
    let p = w.dep.pool.clone().expect("initialisiert");
    assert_eq!((p.kas(), p.ghost(), p.shares()), (E8, 4_000_000, E8));
    assert!(w.dep.pool_pending.is_none());
    let pool_spk = spk(&pool::pool_artifact(&p.params, &p.lp_cov, true));

    // 2. Besitzer legt 999 KAS und 39,96 GHOST ein → 99,9 % der Anteile
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let r = pool::add(&p, &owner, &mine, 999 * E8, 3_996_000_000, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net);
    let (t, m, _) = r.expect("add baut");
    assert_eq!(m, 999 * E8);
    w.pool("Einlegen (Besitzer)", Ok(t));
    assert_eq!(w.lp_of(&owner), 999 * E8);
    let p = w.dep.pool.clone().unwrap();
    assert_eq!((p.kas(), p.ghost(), p.shares()), (1_000 * E8, 40 * E8, 1_000 * E8));

    // 3. Händler kauft GHOST für 100 KAS, verkauft die Hälfte zurück
    let trader = key();
    w.sim.faucet(&trader, 500 * E8 as u64);
    let quote = pool::ghost_out(p.kas(), p.ghost(), 100 * E8, POOL_FEE_BPS);
    let (t, out) = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &trader, &[], Swap::Buy { kas: 100 * E8, min_ghost: quote }, &w.sim.funds(&trader), &net).expect("buy baut");
    assert_eq!(out, quote);
    w.pool("KAS → GHOST", Ok(t));
    assert_eq!(w.ghost_of(&trader), quote);
    let p = w.dep.pool.clone().unwrap();
    assert_eq!(spk(&pool::pool_artifact(&p.params, &p.lp_cov, true)), pool_spk, "Pool-Adresse bleibt gleich");
    assert!(pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &trader, &[], Swap::Buy { kas: 100 * E8, min_ghost: quote }, &w.sim.funds(&trader), &net).is_err(), "Kurs verschoben");
    let half = quote / 2;
    let mine = pool::own(&w.dep.tokens, &xonly(&trader), 2);
    let kq = pool::kas_out(p.kas(), p.ghost(), half, POOL_FEE_BPS);
    let (t, _) = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &trader, &mine, Swap::Sell { ghost: half, min_kas: kq }, &w.sim.funds(&trader), &net).expect("sell baut");
    w.pool("GHOST → KAS", Ok(t));
    assert_eq!(w.ghost_of(&trader), quote - half);

    // 4. Zweiter Einleger: Anteile nach dem aktuellen Verhältnis
    let lp2 = key();
    w.sim.faucet(&lp2, 5_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&lp2), 4_000 * E8 as u64, &w.sim.funds(&lp2), &net);
    w.apply("Vault 2", r);
    let r = ops::mint(&w.dep, 1, &lp2, 20 * E8, &xonly(&lp2), &w.sim.funds(&lp2), &net);
    w.apply("20 GHOST prägen", r);
    let p = w.dep.pool.clone().unwrap();
    let dy = 10 * E8;
    let dx = (dy as i128 * p.kas() as i128 / p.ghost() as i128) as i64 + 1;
    let mine = pool::own(&w.dep.tokens, &xonly(&lp2), 2);
    let (t, m2, _) = pool::add(&p, &lp2, &mine, dx, dy, 1, pool::ADD_TOL_BPS, &w.sim.funds(&lp2), &net).expect("add 2 baut");
    assert_eq!(m2, pool::shares_for_deposit(p.shares(), p.kas(), p.ghost(), dx, dy));
    w.pool("Einlegen (zweiter)", Ok(t));
    assert_eq!(w.lp_of(&lp2), m2);

    // 5. Zweiter zieht die Hälfte ab, Besitzer alles – die gesperrten Anteile bleiben
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.lp_tokens, &xonly(&lp2), 2);
    let (dx2, dy2) = pool::payout_for(p.shares(), p.kas(), p.ghost(), m2 / 2);
    let (t, got) = pool::remove(&p, &lp2, &mine, m2 / 2, dx2, dy2, &w.sim.funds(&lp2), &net).expect("remove baut");
    assert_eq!(got, (dx2, dy2));
    w.pool("Abziehen (halb)", Ok(t));
    assert_eq!(w.lp_of(&lp2), m2 - m2 / 2);
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.lp_tokens, &xonly(&owner), 2);
    let (t, _) = pool::remove(&p, &owner, &mine, 999 * E8, 0, 0, &w.sim.funds(&owner), &net).expect("remove Besitzer baut");
    w.pool("Abziehen (Besitzer)", Ok(t));
    assert_eq!(w.lp_of(&owner), 0);
    let p = w.dep.pool.clone().unwrap();
    assert_eq!(p.shares(), E8 + (m2 - m2 / 2), "gesperrte + Anteile des zweiten");
    assert!(p.kas() >= E8);
}

/// Kursband (Pool v2) im Simulator mit voller Konsensprüfung: Der frische
/// 1-KAS-Pool lässt sich nicht verschieben, Kaufen geht genau bis zur
/// Bandgrenze, und das mitgelesene Orakel bleibt für Vaults nutzbar.
#[test]
fn kursband_im_simulator() {
    let (mut w, owner) = setup();
    let net = w.sim.params.clone();
    let gcov = w.dep.vault_params.as_ref().unwrap().ghost_cov;
    let band = pool::POOL_BAND_BPS;
    let r = pool::pool_create(gcov, pool::PoolBand::of(&w.dep, band), &owner, &w.sim.funds(&owner), &net);
    w.pool("Pool-Genesis", r);
    let pend = w.dep.pool_pending.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    // Start außerhalb des Bands (1 KAS : 0,05 GHOST = 0,80 USD) wird abgelehnt …
    let e = pool::pool_init(&pend, Some(&pool::OracleRef::of(&w.dep)), &owner, &mine, 5_000_000, &w.sim.funds(&owner), &net).err().expect("außerhalb");
    assert!(e.contains("Band"), "{e}");
    // … bei 1 USD geht es (1 KAS : 0,04 GHOST bei 0,04 USD je KAS)
    let r = pool::pool_init(&pend, Some(&pool::OracleRef::of(&w.dep)), &owner, &mine, 4_000_000, &w.sim.funds(&owner), &net);
    w.pool("Pool-init im Band", r);

    // Angriff aus Audit 10 (A10-P-1): den winzigen Pool mit 9 KAS verschieben – gesperrt
    let att = key();
    w.sim.faucet(&att, 100 * E8 as u64);
    let p = w.dep.pool.clone().unwrap();
    let e = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &att, &[], Swap::Buy { kas: 9 * E8, min_ghost: 1 }, &w.sim.funds(&att), &net).err().expect("gesperrt");
    assert!(e.contains("Kursband"), "{e}");

    // Liquidität dazu, dann kaufen bis zur oberen Grenze – im Konsens angenommen
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let (t, _, _) = pool::add(&p, &owner, &mine, 999 * E8, 3_996_000_000, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net).unwrap();
    w.pool("Einlegen", Ok(t));
    let p = w.dep.pool.clone().unwrap();
    let k = w.dep.oracle.state.kas_usd;
    let dx = pool::band_max_in(p.kas(), p.ghost(), POOL_FEE_BPS, k, band, true);
    assert!(dx > 10 * E8 && dx < 30 * E8, "bei 1 000 KAS im Pool etwa 15 KAS: {dx}");
    let oracle_before = w.dep.oracle.outpoint;
    let r = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &att, &[], Swap::Buy { kas: dx, min_ghost: 1 }, &w.sim.funds(&att), &net).map(|(t, _)| t);
    w.pool("Kauf bis zur Bandgrenze", r);
    assert_ne!(w.dep.oracle.outpoint, oracle_before, "Orakel wurde mitgelesen und nachgeführt");
    let p = w.dep.pool.clone().unwrap();
    let usd = pool::ghost_usd(p.kas(), p.ghost(), k);
    assert!(usd > 1.02 && usd <= 1.03, "GHOST jetzt am oberen Rand: {usd}");
    // weiter kaufen: gesperrt; verkaufen (zurück Richtung 1 USD): frei
    assert!(pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &att, &[], Swap::Buy { kas: E8, min_ghost: 1 }, &w.sim.funds(&att), &net).is_err());
    let mine = pool::own(&w.dep.tokens, &xonly(&att), 2);
    let r = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &att, &mine, Swap::Sell { ghost: w.ghost_of(&att) / 2, min_kas: 1 }, &w.sim.funds(&att), &net).map(|(t, _)| t);
    w.pool("Verkauf zurück ins Band", r);

    // Vault-Aktion mit dem nachgeführten Orakel geht weiter
    let r = ops::mint(&w.dep, 0, &owner, E8, &xonly(&owner), &w.sim.funds(&owner), &net);
    w.apply("Prägen nach Pool-Täuschen", r);
}

/// Ein Pool der Version 1 (ohne Kursband) bleibt bedienbar – Einlegen und
/// Abziehen wie für den bestehenden Mainnet-Pool nötig
#[test]
fn pool_ohne_kursband_bleibt_bedienbar() {
    let (mut w, owner) = setup();
    let net = w.sim.params.clone();
    let gcov = w.dep.vault_params.as_ref().unwrap().ghost_cov;
    let r = pool::pool_create(gcov, None, &owner, &w.sim.funds(&owner), &net);
    w.pool("Pool-Genesis v1", r);
    let pend = w.dep.pool_pending.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let r = pool::pool_init(&pend, None, &owner, &mine, 4_000_000, &w.sim.funds(&owner), &net);
    w.pool("Pool-init v1", r);
    let p = w.dep.pool.clone().unwrap();
    assert!(p.params.band.is_none());
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let (t, _, _) = pool::add(&p, &owner, &mine, 99 * E8, 396_000_000, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net).unwrap();
    w.pool("Einlegen v1", Ok(t));
    // Tausch ohne Orakel, auch weit weg von 1 USD
    let p = w.dep.pool.clone().unwrap();
    let r = pool::swap(&p, None, &owner, &[], Swap::Buy { kas: 50 * E8, min_ghost: 1 }, &w.sim.funds(&owner), &net).map(|(t, _)| t);
    w.pool("Tausch v1", r);
    // alles abziehen (bis auf die gesperrte Mindestliquidität)
    let p = w.dep.pool.clone().unwrap();
    let lp = pool::own(&w.dep.lp_tokens, &xonly(&owner), 2);
    let have = w.lp_of(&owner);
    let (t, _) = pool::remove(&p, &owner, &lp, have, 0, 0, &w.sim.funds(&owner), &net).unwrap();
    w.pool("Abziehen v1 100 %", Ok(t));
    assert_eq!(w.lp_of(&owner), 0);
}
