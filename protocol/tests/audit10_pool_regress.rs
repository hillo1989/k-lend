//! Regressionstests zu Audit 10 (Opus), offener Pool: die Befunde A10-P-1,
//! -2, -4 und -5 dürfen nicht zurückkommen. Grundlage sind die Simulator-Belege
//! des Prüfers (audit/10-opus-pool.md), mit umgedrehter Erwartung.

use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{TransactionId, TransactionOutpoint};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::ops::{p2pk_spk, self, Deployment, TOKEN_VALUE, Tracked, xonly};
use kaspa_lending_protocol::pool::{self, POOL_FEE_BPS, PoolParams, PoolRec, Swap};
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
    dep: Deployment,
}

impl World {
    fn apply(&mut self, label: &str, r: Result<(Built, Deployment), String>) {
        let (b, d) = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.dep = d;
    }
    fn pool(&mut self, label: &str, r: Result<pool::PoolTx, String>) {
        let t = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        self.sim.submit(&t.built).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.dep = pool::apply(&self.dep, &t);
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
    let owner = key();
    w.sim.faucet(&owner, 20_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&owner), 10_000 * E8 as u64, &w.sim.funds(&owner), &net);
    w.apply("Vault eröffnen", r);
    let r = ops::mint(&w.dep, 0, &owner, 150 * E8, &xonly(&owner), &w.sim.funds(&owner), &net);
    w.apply("150 GHOST prägen", r);
    (w, owner)
}

/// Pool wie im Mainnet-Plan: 1 KAS + 0,04 GHOST gesperrt (1 GHOST = 25 KAS)
fn open_pool(w: &mut World, owner: &Keypair) {
    let net = w.sim.params.clone();
    let gcov = w.dep.vault_params.as_ref().unwrap().ghost_cov;
    // weites Band: diese Tests prüfen Einlegen/Abziehen mit großen Kursbewegungen
    let r = pool::pool_create(gcov, pool::PoolBand::of(&w.dep, 1_000_000), owner, &w.sim.funds(owner), &net);
    w.pool("Pool-Genesis", r);
    let pend = w.dep.pool_pending.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(owner), 2);
    let r = pool::pool_init(&pend, Some(&pool::OracleRef::of(&w.dep)), owner, &mine, 4_000_000, &w.sim.funds(owner), &net);
    w.pool("Pool-init", r);
}

/// Marktwert in sompi beim fairen Kurs 1 GHOST = 25 KAS (25 sompi je Einheit)
fn val(kas: i64, ghost: i64) -> i64 {
    kas + ghost * 25
}

/// A10-P-1: Hat ein Angreifer den kleinen Pool verschoben, bricht eine Einlage
/// zum alten Verhältnis ab, statt den Überschuss zu verschenken.
#[test]
fn einlage_nach_kursverschiebung_bricht_ab() {
    let (mut w, owner) = setup();
    let net = w.sim.params.clone();
    open_pool(&mut w, &owner);
    let att = key();
    w.sim.faucet(&att, 6_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&att), 4_000 * E8 as u64, &w.sim.funds(&att), &net);
    w.apply("Vault Angreifer", r);
    let r = ops::mint(&w.dep, 1, &att, 20 * E8, &xonly(&att), &w.sim.funds(&att), &net);
    w.apply("20 GHOST Angreifer", r);
    // Angreifer verschiebt den 1-KAS-Pool mit 9 KAS
    let p = w.dep.pool.clone().unwrap();
    let (t, _) = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &att, &[], Swap::Buy { kas: 9 * E8, min_ghost: 1 }, &w.sim.funds(&att), &net).unwrap();
    w.pool("Angreifer kauft", Ok(t));
    // Opfer legt zum alten Verhältnis ein (wie pool-open Schritt 3)
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let e = pool::add(&p, &owner, &mine, 999 * E8, 3_996_000_000, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net).err().expect("muss abbrechen");
    assert!(e.contains("weicht"), "{e}");
}

/// A10-P-1: Es wird nur genommen, was zu den Anteilen passt; der Rest bleibt beim Nutzer.
#[test]
fn einlage_nimmt_nur_den_passenden_betrag() {
    let (mut w, owner) = setup();
    let net = w.sim.params.clone();
    open_pool(&mut w, &owner);
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    // 0,5 % mehr GHOST als passend: innerhalb des Bands, der Überschuss bleibt beim Nutzer
    let (kas, ghost) = (100 * E8, 402_000_000i64);
    let (t, m, (dx, dy)) = pool::add(&p, &owner, &mine, kas, ghost, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net).expect("baut");
    assert_eq!(dx, kas);
    assert!(dy < ghost && dy >= 400_000_000, "nur passende GHOST: {dy}");
    w.pool("Einlegen", Ok(t));
    let p = w.dep.pool.clone().unwrap();
    assert_eq!(p.ghost(), 4_000_000 + dy);
    assert!(m > 0);
}

/// A10-P-2: Anteile über 1e16 werden als Kandidaten gefunden
#[test]
fn anteile_ueber_1e16_werden_gefunden() {
    let mut sim = Sim::new();
    let net = sim.params.clone();
    let trader = key();
    sim.faucet(&trader, 1_000 * E8 as u64);
    let pool_cov = Hash::from_bytes([0x0e; 32]);
    let lp_cov = Hash::from_bytes([0x0f; 32]);
    let ghost_cov = Hash::from_bytes([0x0b; 32]);
    let txid = TransactionId::from_bytes([0x50; 32]);
    let s_big: i64 = 20_000_000_000_000_000;
    let rec = PoolRec {
        params: PoolParams { ghost_cov, tpl: ghost_template(), fee_bps: POOL_FEE_BPS, creator: xonly(&key()), band: None },
        lp_cov,
        pool: Tracked { outpoint: TransactionOutpoint { transaction_id: txid, index: 0 }, value: 1_000 * E8 as u64, cov: pool_cov, state: () },
        reserve: Tracked { outpoint: TransactionOutpoint { transaction_id: txid, index: 1 }, value: TOKEN_VALUE, cov: ghost_cov, state: pool::reserve_tok(&pool_cov, 40 * E8) },
        minter: Tracked { outpoint: TransactionOutpoint { transaction_id: txid, index: 2 }, value: pool::MINTER_VALUE, cov: lp_cov, state: pool::minter_tok(&pool_cov, s_big) },
    };
    let (t, _) = pool::swap(&rec, None, &trader, &[], Swap::Buy { kas: 10 * E8, min_ghost: 1 }, &sim.funds(&trader), &net).expect("gültige Pool-Tx bei S = 2e16");
    let c = pool::amount_candidates(&t.built.tx.inputs.iter().map(|i| i.signature_script.clone()).collect::<Vec<_>>());
    assert!(c.contains(&t.pool.unwrap().ghost()));
    assert!(c.contains(&s_big), "S = 2e16 muss unter den Kandidaten sein");
}

/// A10-P-4/-5: nach einer Kursverschiebung kommt der Einleger ganz heraus
/// (KAS-Seite auf die 1-KAS-Grenze gekappt), ein Mini-Abzug ohne Auszahlung wird abgelehnt.
#[test]
fn abziehen_an_den_raendern() {
    let (mut w, owner) = setup();
    let net = w.sim.params.clone();
    open_pool(&mut w, &owner);
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let (t, _, _) = pool::add(&p, &owner, &mine, 999 * E8, 3_996_000_000, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net).unwrap();
    w.pool("Einlegen", Ok(t));
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let (t, _) = pool::swap(&p, Some(&pool::OracleRef::of(&w.dep)), &owner, &mine, Swap::Sell { ghost: 100 * E8, min_kas: 1 }, &w.sim.funds(&owner), &net).unwrap();
    w.pool("Verkauf", Ok(t));
    let p = w.dep.pool.clone().unwrap();
    assert!(p.kas() < p.shares(), "Anteil < 1 sompi wert");
    let lp = pool::own(&w.dep.lp_tokens, &xonly(&owner), 2);
    let have = w.lp_of(&owner);
    assert!(pool::remove(&p, &owner, &lp, 1, 0, 0, &w.sim.funds(&owner), &net).is_err(), "1 Anteil → nichts, abgelehnt");
    let (t, (dx, _)) = pool::remove(&p, &owner, &lp, have, 0, 0, &w.sim.funds(&owner), &net).expect("100 % geht");
    assert_eq!(dx, p.kas() - pool::POOL_MIN_KAS as i64, "bis auf 1 KAS");
    w.pool("Abziehen 100 %", Ok(t));
    assert_eq!(w.lp_of(&owner), 0);
    assert_eq!(w.dep.pool.clone().unwrap().kas(), pool::POOL_MIN_KAS as i64);
}
