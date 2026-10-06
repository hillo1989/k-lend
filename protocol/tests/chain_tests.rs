//! Zustand von der Kette nachführen (src/chain.rs, Audit 10 A10-A-3): Ein
//! Rechner mit altem Stand muss Orakel-Updates, Prägen/Tilgen/Liquidieren und
//! neue Vaults anderer rekonstruieren – exakt, am Skript bestätigt.

use kaspa_lending_protocol::chain::{self, MemChain, Next, Shapes, TxView};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::ops::{self, Deployment, p2pk_spk, xonly};
use kaspa_lending_protocol::sim::{Sim, test_register_params};
use kaspa_lending_protocol::txb::Built;
use rand::{Rng, RngCore, SeedableRng, rngs::StdRng, thread_rng};
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
    /// alle Tx seit Beginn der Aufzeichnung (der „fremde“ Verlauf)
    log: Vec<TxView>,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new();
        let deployer = key();
        let committee: Vec<Keypair> = (0..5).map(|_| key()).collect();
        sim.faucet(&deployer, 1_000 * E8 as u64);
        let net = sim.params.clone();
        let feed = sim.deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: committee.iter().map(xonly).collect(), t: 3, t_rot: 3 }, 4_000_000, 158_548_959, 1_000_000_000).expect("Register und Orakel");
        let op = feed.oracle_params.clone();
        let oracle_t = feed.oracle.clone();
        let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
        let (b, factory_t) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
        sim.submit(&b).unwrap();
        let (b, f2, root, vp) = ops::init_factory(&op, &oracle_t, &fp, &factory_t, &deployer, (20_000, 15_000, 1_000, 5_000_000_000), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &net).unwrap();
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
        World { sim, deployer, committee, dep, log: vec![] }
    }
    fn apply(&mut self, label: &str, r: Result<(Built, Deployment), String>) {
        let (b, d) = r.unwrap_or_else(|e| panic!("{label}: baut nicht: {e}"));
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.log.push(TxView::from(&b.tx));
        self.dep = d;
    }
    fn price(&mut self, kas_usd: i64, rate: i64) {
        let signers: Vec<(usize, Keypair)> = [0usize, 2, 4].iter().map(|&i| (i, self.committee[i])).collect();
        self.sim.advance(700);
        let daa = self.sim.daa - 1;
        let net = self.sim.params.clone();
        let r = ops::oracle_update(&self.dep, &signers, kas_usd, rate, daa, &self.sim.funds(&self.deployer), &net);
        self.apply("Orakel-Update", r);
    }
    fn toks(&self, k: &Keypair) -> Vec<usize> {
        let mut v: Vec<usize> = self.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(k)).map(|(i, _)| i).collect();
        v.sort_by_key(|&i| std::cmp::Reverse(self.dep.tokens[i].state.amount));
        v.truncate(2);
        v
    }
}

/// Skripte ohne Kompilieren = kompilierte Skripte (Orakel, Vault, GHOST-Token)
#[test]
fn skripte_ohne_kompilieren_stimmen() {
    let w = World::new();
    let sh = Shapes::of(&w.dep);
    let vp = w.dep.vault_params.clone().unwrap();
    let mut r = StdRng::seed_from_u64(7);
    for _ in 0..6 {
        let s = OracleState {
            kas_usd: r.gen_range(1_000..90_000_000_000),
            oracle_daa: r.gen_range(0..1_000_000_000_000),
            seq: r.gen_range(0..1_000_000),
            stable_rate: r.gen_range(0..1_000_000_000),
            stable_index: r.gen_range(1_000_000_000..100_000_000_000),
            frozen: r.gen_bool(0.5),
            last_rate_daa: r.gen_range(0..1_000_000_000_000),
        };
        let want = spk(&oracle(&w.dep.oracle_params, &s)).script().to_vec();
        assert_eq!(sh.oracle.spk(&chain::oracle_state_bytes(&s)), want, "Orakel {s:?}");
        // Register v4: jeder Zustand, auch Ticket und Notfall
        let h = |r: &mut StdRng| (0..32).map(|_| r.r#gen()).collect::<Vec<u8>>();
        let rs = RegisterState {
            ticket: r.gen_bool(0.5),
            set_hash: h(&mut r),
            fb_hash: h(&mut r),
            pay_kind: r.r#gen(),
            pay_to: h(&mut r),
            nonce: r.gen_range(0..1_000_000),
            emerg: r.gen_bool(0.5),
            last_daa: r.gen_range(0..1_000_000_000_000),
            oracle_cov: h(&mut r),
            oracle_tpl: h(&mut r),
            oracle_pre: r.gen_range(0..100),
            oracle_suf: r.gen_range(0..10_000),
            initialized: r.gen_bool(0.5),
        };
        let want = spk(&register(&w.dep.register_params, &rs)).script().to_vec();
        assert_eq!(sh.register.spk(&chain::register_state_bytes(&rs)), want, "Register {rs:?}");
        let code = bytecode(&register(&w.dep.register_params, &rs));
        assert_eq!(sh.register.state_of(&code).and_then(chain::parse_register_state), Some(rs.clone()), "Register zurückgelesen");
        let owner: Vec<u8> = (0..32).map(|_| r.r#gen()).collect();
        let st = VaultState { debt: r.gen_range(0..1_000_000_000_000_000), interest: r.gen_range(0..1_000_000_000_000), index_at: r.gen_range(0..100_000_000_000) };
        let want = spk(&vault(&vp, &owner, &st)).script().to_vec();
        assert_eq!(sh.vault.as_ref().unwrap().spk(&chain::vault_state_bytes(&owner, &st)), want, "Vault {st:?}");
        let t = GhostTok { owner: owner.clone(), typ: if r.gen_bool(0.5) { ID_COV } else { ID_PUBKEY }, amount: r.gen_range(0..10_000_000_000_000_000), minter: r.gen_bool(0.5) };
        assert_eq!(sh.ghost.as_ref().unwrap().spk(&chain::ghost_state_bytes(&t)), spk(&t.artifact()).script().to_vec(), "Token {t:?}");
    }
}

/// Ein Rechner mit altem Stand holt alles nach, was andere inzwischen getan haben
#[test]
fn fremde_aenderungen_werden_exakt_nachgefuehrt() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let user = key();
    let other = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    w.sim.faucet(&other, 5_000 * E8 as u64);
    // 2 500 KAS: nach dem Kurssturz auf 0,0105 USD liegt die Quote bei ~90 %
    let r = ops::open_vault(&w.dep, &xonly(&user), 2_500 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault 0 eröffnen", r);
    let r = ops::mint(&w.dep, 0, &user, 40 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("Vault 0 prägt 40", r);

    // ---- Stand des anderen Rechners; alles danach sieht er nicht
    let old = w.dep.clone();
    w.log.clear();

    w.price(4_100_000, 158_548_959);
    let r = ops::mint(&w.dep, 0, &user, 7 * E8 + 123, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("Vault 0 prägt 7,00000123", r);
    let t = w.toks(&user);
    let r = ops::repay(&w.dep, 0, &user, &t, 12 * E8 + 7, &w.sim.funds(&user), &net);
    w.apply("Vault 0 tilgt 12,00000007", r);
    // neuer Vault eines anderen Nutzers
    let r = ops::open_vault(&w.dep, &xonly(&other), 3_000 * E8 as u64, &w.sim.funds(&other), &net);
    w.apply("Vault 1 eröffnen", r);
    let r = ops::mint(&w.dep, 1, &other, 30 * E8, &xonly(&other), &w.sim.funds(&other), &net);
    w.apply("Vault 1 prägt 30", r);
    // Zinssatz ändert sich
    w.price(4_200_000, 300_000_000);
    let r = ops::deposit(&w.dep, 0, &user, 500 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault 0 zahlt 500 KAS ein", r);
    // Zins lässt die Schuld über 30 GHOST wachsen: 1 GHOST dazu (Tilgen dann mit zwei GHOST-Eingängen)
    let t = w.toks(&user);
    let r = ops::transfer(&w.dep, &user, &t, &xonly(&other), E8, &w.sim.funds(&user), &net);
    w.apply("1 GHOST an Nutzer 2", r);
    // Vault 1 tilgt vollständig und schließt
    let debt1 = w.dep.vaults[1].vault.state.debt;
    let t = w.toks(&other);
    let r = ops::repay(&w.dep, 1, &other, &t, debt1, &w.sim.funds(&other), &net);
    w.apply("Vault 1 tilgt alles", r);
    assert_eq!(w.dep.vaults[1].vault.state.debt, 0);
    let vault1 = w.dep.vaults[1].clone();
    let r = ops::close(&w.dep, 1, &other, &p2pk_spk(&xonly(&other)), &w.sim.funds(&other), &net);
    w.apply("Vault 1 schließen", r);
    // Kurssturz, dann Teil-Liquidation von Vault 0
    w.price(2_100_000, 300_000_000);
    w.price(1_050_000, 300_000_000);
    let t = w.toks(&user);
    let r = ops::liquidate(&w.dep, 0, &user, &t, 5 * E8 + 99, &w.sim.funds(&user), &net);
    w.apply("Vault 0 teilweise liquidiert", r);

    let sh = Shapes::of(&old);
    let mut h = MemChain { txs: w.log.clone() };

    // Orakel: 4 Updates, Zins und Index exakt
    let (op, value, s, updates) = chain::follow_oracle(&mut h, &sh, &old.oracle_params, &old.oracle).expect("Orakel verfolgbar");
    assert_eq!((op, value, s), (w.dep.oracle.outpoint, w.dep.oracle.value, w.dep.oracle.state));
    assert_eq!(updates, 4);
    assert_ne!(s.stable_index, old.oracle.state.stable_index, "Index ist gewachsen");

    // Vault 0: Prägen, Tilgen, Einzahlen, Teil-Liquidation
    let v0 = &old.vaults[0];
    let (op, value, shares) = chain::follow_vault(&mut h, &sh, &v0.owner, &v0.vault).expect("Vault verfolgbar").expect("Vault 0 lebt");
    assert_eq!((op, value, shares), (w.dep.vaults[0].vault.outpoint, w.dep.vaults[0].vault.value, w.dep.vaults[0].vault.state));

    // Vault 1 war dem alten Stand unbekannt: über die Factory gefunden …
    let found = chain::discover_vaults(&mut h, &sh, &old).expect("Factory-Verlauf lesbar");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].vault.cov, vault1.vault.cov);
    assert_eq!(found[0].owner, xonly(&other));
    assert_eq!(found[0].branch.cov, vault1.branch.cov);
    assert_eq!(found[0].branch.state, vault1.branch.state);
    // … und bis zum Schließen verfolgt
    assert!(chain::follow_vault(&mut h, &sh, &found[0].owner, &found[0].vault).expect("verfolgbar").is_none(), "Vault 1 ist geschlossen");
}

/// Eine Tx, deren Ausgang zu keinem Kandidaten passt, wird abgelehnt – nie geraten
#[test]
fn gefaelschter_ausgang_wird_nicht_uebernommen() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let user = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&user), 10_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault eröffnen", r);
    let old = w.dep.clone();
    w.log.clear();
    let r = ops::mint(&w.dep, 0, &user, 40 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("Prägen", r);
    w.price(4_100_000, 158_548_959);
    let sh = Shapes::of(&old);
    let v0 = &old.vaults[0];
    let vcov = v0.vault.cov;
    // Vault-Ausgang der Präge-Tx verfälschen (Skript eines Vaults mit anderer Schuld)
    let mut bad = w.log.clone();
    let o = bad[0].outputs.iter_mut().find(|o| o.cov == Some(vcov)).unwrap();
    o.spk = sh.vault.as_ref().unwrap().spk(&chain::vault_state_bytes(&v0.owner, &VaultState { debt: 999_999, ..w.dep.vaults[0].vault.state }));
    let e = chain::follow_vault(&mut MemChain { txs: bad }, &sh, &v0.owner, &v0.vault).err().expect("muss scheitern");
    assert!(e.contains("nicht rekonstruierbar"), "{e}");
    // Orakel-Ausgang des Updates verfälschen
    let mut bad = w.log.clone();
    let ocov = old.oracle.cov;
    let o = bad[1].outputs.iter_mut().find(|o| o.cov == Some(ocov)).unwrap();
    o.spk = sh.oracle.spk(&chain::oracle_state_bytes(&OracleState { kas_usd: 9_999_999, ..w.dep.oracle.state }));
    let e = chain::follow_oracle(&mut MemChain { txs: bad }, &sh, &old.oracle_params, &old.oracle).err().expect("muss scheitern");
    assert!(e.contains("nicht rekonstruierbar"), "{e}");
    // Kontrolle: unverfälscht geht es
    let r = chain::follow_oracle(&mut MemChain { txs: w.log.clone() }, &sh, &old.oracle_params, &old.oracle).unwrap();
    assert_eq!(r.2, w.dep.oracle.state);
    let _ = Next::<i64>::Ended;
}
