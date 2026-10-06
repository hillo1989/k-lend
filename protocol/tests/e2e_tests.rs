//! Kompletter Lebenszyklus auf der lokalen Simulationskette (src/sim.rs):
//! jede Tx mit Compute-Budget, Speichermasse und Mindestgebühr wie im Netz.
//! Ausgabe der Messwerte: cargo test --test e2e_tests -- --nocapture

use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::{self, E8};
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

fn report(label: &str, b: &Built) {
    let bytes: usize = b.tx.inputs.iter().map(|i| i.signature_script.len()).sum::<usize>() + b.tx.outputs.len() * 60;
    println!(
        "{label:<22} Inputs {:>2} | Budget {:?} | compute {:>7} g | transient {:>7} g | storage {:>7} g | Gebühr {:>9} sompi ({:.4} KAS) | ~{} B",
        b.tx.inputs.len(),
        b.budgets,
        b.compute_mass,
        b.transient_mass,
        b.storage_mass,
        b.fee,
        b.fee as f64 / 1e8,
        bytes
    );
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
    report("Factory-Genesis", &b);
    sim.submit(&b).expect("Factory-Genesis angenommen");

    let (b, f2, root, vp) =
        ops::init_factory(&op, &oracle_t, &fp, &factory_t, &deployer, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &net).expect("Init baut");
    report("Factory-Init + GHOST", &b);
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
        report(label, &b);
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: abgelehnt: {e}"));
        self.dep = d;
    }
    fn price(&mut self, kas_usd: i64) {
        let signers: Vec<(usize, Keypair)> = [0usize, 2, 4].iter().map(|&i| (i, self.committee[i])).collect();
        self.sim.advance(700); // Orakel v2.1: Mindestabstand 600 DAA
        let daa = self.sim.daa - 1; // lock_time muss < DAA sein
        let net = self.sim.params.clone();
        let r = ops::oracle_update(&self.dep, &signers, kas_usd, self.dep.oracle.state.stable_rate, daa, &self.sim.funds(&self.deployer), &net);
        self.apply("Orakel-Update", r);
    }
}

#[test]
fn kompletter_lebenszyklus() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let user = key();
    let liq = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    w.sim.faucet(&liq, 50_000 * E8 as u64);

    // Vault eröffnen: 10 000 KAS
    let r = ops::open_vault(&w.dep, &xonly(&user), 10_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault eröffnen", r);
    assert_eq!(w.dep.vaults.len(), 1);

    // 150 GHOST prägen (bei 0,04 USD sind 400 USD Sicherheit → max ~200)
    let r = ops::mint(&w.dep, 0, &user, 150 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("Prägen 150 GHOST", r);
    assert_eq!(w.dep.tokens.len(), 1);

    // zu viel prägen wird schon beim Bauen abgelehnt
    assert!(ops::mint(&w.dep, 0, &user, 100 * E8, &xonly(&user), &w.sim.funds(&user), &net).is_err());

    // Orakel aktualisieren (3 von 5)
    w.price(4_100_000);
    assert_eq!(w.dep.oracle.state.seq, 1);

    // 50 GHOST tilgen
    let r = ops::repay(&w.dep, 0, &user, &[0], 50 * E8, &w.sim.funds(&user), &net);
    w.apply("Tilgen 50 GHOST", r);

    // Liquidator: eigener Vault, prägt 200 GHOST
    let r = ops::open_vault(&w.dep, &xonly(&liq), 40_000 * E8 as u64, &w.sim.funds(&liq), &net);
    w.apply("Vault 2 eröffnen", r);
    let r = ops::mint(&w.dep, 1, &liq, 200 * E8, &xonly(&liq), &w.sim.funds(&liq), &net);
    w.apply("Prägen 200 GHOST", r);

    // Preissturz: Vault 1 fällt unter 150 %
    let st = w.dep.vaults[0].vault.state;
    let mut p = 4_100_000;
    while math::healthy(w.dep.vaults[0].vault.value as i64, math::owed(&st, w.dep.oracle.state.stable_index), p, 15_000) {
        p -= 100_000;
    }
    // Orakel v2 erlaubt je Update höchstens ÷2: Absturz in Schritten
    while w.dep.oracle.state.kas_usd / 2 > p {
        let step = w.dep.oracle.state.kas_usd / 2 + 1;
        w.price(step);
    }
    w.price(p);
    let liq_before = w.sim.balance(&liq);
    let liq_tok = w.dep.tokens.iter().position(|t| t.state.owner == xonly(&liq)).unwrap();
    let full_debt = w.dep.vaults[0].vault.state.debt;
    let r = ops::liquidate(&w.dep, 0, &liq, &[liq_tok], full_debt, &w.sim.funds(&liq), &net);
    w.apply("Liquidation", r);
    let gained = w.sim.balance(&liq) as i64 - liq_before as i64;
    println!("Liquidator: +{:.2} KAS (abzüglich Gebühr) bei Preis {:.4} USD", gained as f64 / 1e8, p as f64 / 1e8);
    assert!(gained > 0, "Liquidation muss sich lohnen");

    // Vault 2 (jetzt Index 0 oder 1, je nachdem ob Vault 1 endete)
    let v2 = w.dep.vaults.iter().position(|v| v.owner == xonly(&liq)).unwrap();
    let r = ops::deposit(&w.dep, v2, &liq, 1_000 * E8 as u64, &w.sim.funds(&liq), &net);
    w.apply("Einzahlen", r);
    let r = ops::withdraw(&w.dep, v2, &liq, 35_000 * E8 as u64, &p2pk_spk(&xonly(&liq)), &w.sim.funds(&liq), &net);
    w.apply("Abheben", r);

    // Veralteter Orakel-Stand: alte Deployment-Kopie verweist auf verbrauchte UTXO
    let stale = w.dep.clone();
    w.price(p + 1_000_000);
    let r = ops::withdraw(&stale, v2, &liq, 34_000 * E8 as u64, &p2pk_spk(&xonly(&liq)), &w.sim.funds(&liq), &net).unwrap();
    assert!(w.sim.submit(&r.0).is_err(), "alter Orakelstand darf nicht mehr nutzbar sein");

    // GHOST überweisen: Nutzer schickt 1 GHOST an den Liquidator
    let user_tok = w.dep.tokens.iter().position(|t| t.state.owner == xonly(&user)).unwrap();
    let r = ops::transfer(&w.dep, &user, &[user_tok], &xonly(&liq), E8, &w.sim.funds(&user), &net);
    w.apply("GHOST überweisen", r);

    // Vault 3: prägen, voll tilgen (Version 3: Schuld = genau die geprägten GHOST), schließen
    let r = ops::open_vault(&w.dep, &xonly(&liq), 5_000 * E8 as u64, &w.sim.funds(&liq), &net);
    w.apply("Vault 3 eröffnen", r);
    let v3 = w.dep.vaults.len() - 1;
    let r = ops::mint(&w.dep, v3, &liq, 10 * E8, &xonly(&liq), &w.sim.funds(&liq), &net);
    w.apply("Prägen 10 GHOST", r);
    let debt = w.dep.vaults[v3].vault.state.debt;
    assert_eq!(debt, 10 * E8, "Schuld wächst nicht mit dem Zins");
    let mut toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&liq)).map(|(i, _)| i).collect();
    toks.sort_by_key(|&i| -w.dep.tokens[i].state.amount);
    toks.truncate(2);
    let r = ops::repay(&w.dep, v3, &liq, &toks, debt, &w.sim.funds(&liq), &net);
    w.apply("Voll tilgen", r);
    assert_eq!(w.dep.vaults[v3].vault.state.debt, 0);
    let before = w.sim.balance(&liq);
    let r = ops::close(&w.dep, v3, &liq, &p2pk_spk(&xonly(&liq)), &w.sim.funds(&liq), &net);
    w.apply("Schließen", r);
    assert!(w.sim.balance(&liq) > before + 4_999 * E8 as u64, "Sicherheit zurück");
}

/// Keeper-Agent (ghostctl agent): Die Entscheidung math::keeper_burn liquidiert
/// nur bei Unterdeckung nach Orakel UND Markt, und die Liquidation ist für den
/// Keeper nie ein Verlust – auch wenn die Sicherheit die Schuld nicht mehr deckt.
#[test]
fn keeper_entscheidung_und_liquidation() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let user = key();
    let keeper = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    w.sim.faucet(&keeper, 50_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&user), 10_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault", r);
    let r = ops::mint(&w.dep, 0, &user, 150 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("150 GHOST", r);
    // Keeper besorgt sich GHOST über einen eigenen Vault
    let r = ops::open_vault(&w.dep, &xonly(&keeper), 40_000 * E8 as u64, &w.sim.funds(&keeper), &net);
    w.apply("Keeper-Vault", r);
    let r = ops::mint(&w.dep, 1, &keeper, 300 * E8, &xonly(&keeper), &w.sim.funds(&keeper), &net);
    w.apply("300 GHOST", r);
    let vp = w.dep.vault_params.clone().unwrap();
    let decide = |w: &World, market: i64| {
        let v = &w.dep.vaults[0];
        let o = &w.dep.oracle.state;
        math::keeper_burn(v.vault.value as i64, &v.vault.state, o.kas_usd, market, o.stable_index, vp.liq_bps, vp.bonus_bps)
    };
    assert_eq!(decide(&w, 4_000_000), None, "gesunder Vault");
    // Absturz auf 0,02 USD: 10 000 KAS = 200 USD, Schuld 150 → 133 %
    let mut p = 4_000_000;
    while p > 2_000_000 {
        p = (p / 2).max(2_000_000);
        w.price(p);
    }
    assert_eq!(decide(&w, 4_000_000), None, "Markt sagt gesund → kein Liquidieren mit veraltetem Orakel");
    let burn = decide(&w, 2_000_000).expect("liquidierbar");
    let debt = w.dep.vaults[0].vault.state.debt;
    assert_eq!(burn, debt, "Sicherheit deckt Schuld + Bonus → ganze Schuld");
    let before = w.sim.funds(&keeper).utxos.iter().map(|(_, e)| e.amount).sum::<u64>();
    let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&keeper)).map(|(i, _)| i).collect();
    let r = ops::liquidate(&w.dep, 0, &keeper, &toks, burn, &w.sim.funds(&keeper), &net);
    w.apply("Keeper liquidiert", r);
    let after = w.sim.funds(&keeper).utxos.iter().map(|(_, e)| e.amount).sum::<u64>();
    let gained_usd = math::value_of(after as i64 - before as i64, w.dep.oracle.state.kas_usd);
    assert!(gained_usd > burn, "Keeper bekommt mehr Wert, als er an GHOST verbrennt: {gained_usd} > {burn}");
}

/// Version 3 durchgehend: Zins läuft mit dem Orakelindex auf, jemand Fremdes
/// gibt GHOST zu 1 USD zurück, der Besitzer tilgt und schließt, und die
/// Zinskasse (Deployer) bekommt genau die Zinsgebühr in KAS.
#[test]
fn zins_ruecknahme_und_zinskasse() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let user = key();
    let other = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    w.sim.faucet(&other, 20_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&user), 10_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault", r);
    let r = ops::mint(&w.dep, 0, &user, 100 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("100 GHOST", r);

    // 5 % p. a. (Startsatz des Orakels), 1/10 Jahr = 31 536 000 DAA → Index +0,5 %
    w.sim.advance(31_536_000);
    w.price(4_000_000);
    let index = w.dep.oracle.state.stable_index;
    assert!(index > 1_004_900_000 && index < 1_005_100_000, "Index {index}");
    let st = w.dep.vaults[0].vault.state;
    assert_eq!(st.debt, 100 * E8, "Schuld wächst nicht");
    let acc = math::accrued(&st, index);
    assert!(acc > 49 * E8 / 100 && acc < 51 * E8 / 100, "≈ 0,5 USD Zins, war {acc}");

    // Fremder prägt sich GHOST und gibt 10 davon an den Vault des Nutzers zurück
    let r = ops::open_vault(&w.dep, &xonly(&other), 10_000 * E8 as u64, &w.sim.funds(&other), &net);
    w.apply("Vault 2", r);
    let r = ops::mint(&w.dep, 1, &other, 20 * E8, &xonly(&other), &w.sim.funds(&other), &net);
    w.apply("20 GHOST", r);
    let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&other)).map(|(i, _)| i).collect();
    let paid = math::redeem_paid(10 * E8, w.dep.oracle.state.kas_usd);
    assert_eq!(paid, 24_750_000_000, "10 GHOST × 0,99 USD / 0,04 USD");
    let vault_before = w.dep.vaults[0].vault.value;
    let other_before = w.sim.balance(&other);
    let r = ops::redeem(&w.dep, 0, &other, &toks, 10 * E8, &w.sim.funds(&other), &net);
    let fee = r.as_ref().map(|(b, _)| b.fee).unwrap_or(0);
    w.apply("Rücknahme 10 GHOST", r);
    assert_eq!(w.sim.balance(&other) as i64 - other_before as i64, paid - fee as i64, "Rücknehmer bekommt die KAS abzüglich Netzgebühr");
    let v = &w.dep.vaults[0];
    assert_eq!(v.vault.value, vault_before - paid as u64);
    assert_eq!(v.vault.state.debt, 90 * E8);
    assert_eq!(v.vault.state.interest, acc, "Zins abgerechnet und stehen gelassen");

    // Besitzer tilgt den Rest (hat noch 100 GHOST) und schließt
    let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&user)).map(|(i, _)| i).collect();
    let r = ops::repay(&w.dep, 0, &user, &toks, 90 * E8, &w.sim.funds(&user), &net);
    w.apply("Tilgen 90 GHOST", r);
    assert_eq!(w.dep.vaults[0].vault.state.debt, 0);
    let interest_fee = ops::close_fee(&w.dep, 0);
    let expect = (acc as u128 * E8 as u128).div_ceil(w.dep.oracle.state.kas_usd as u128) as i64;
    assert_eq!(interest_fee, expect, "≈ 12,5 KAS");
    assert!(interest_fee >= math::DUST);
    let treasury_before = w.sim.balance(&w.deployer);
    let r = ops::close(&w.dep, 0, &user, &p2pk_spk(&xonly(&user)), &w.sim.funds(&user), &net);
    w.apply("Schließen mit Zins", r);
    assert_eq!(w.sim.balance(&w.deployer) - treasury_before, interest_fee as u64, "Zinskasse bekommt genau die Zinsgebühr");
    assert_eq!(w.dep.vaults.len(), 1, "nur noch der Vault des Fremden");
}

/// Audit 11 A11-V-4: Nach einer Liquidation bleibt ein Vault mit Schuld 0
/// stehen, dessen Zins mehr wert ist als der Rest. Der Keeper (ghostctl agent)
/// löst ihn per sweep auf: Die Zinskasse bekommt die Sicherheit abzüglich
/// SWEEP_FEE (0,1 KAS). Gemessen: Die Netzgebühr der sweep-Tx ist ≈ 0,055 KAS
/// (der Vault-Eingang trägt das ganze Vertragsskript); den Keeper kostet das
/// nichts, er bekommt den Überschuss als Wechselgeld.
#[test]
fn keeper_loest_zombie_vault_zugunsten_der_zinskasse_auf() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let user = key();
    let keeper = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    w.sim.faucet(&keeper, 200_000 * E8 as u64);
    // Nutzer: 5 000 KAS à 0,04 USD = 200 USD, 90 GHOST
    let r = ops::open_vault(&w.dep, &xonly(&user), 5_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault Nutzer", r);
    let r = ops::mint(&w.dep, 0, &user, 90 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("90 GHOST", r);
    // Keeper besorgt sich GHOST über einen gut gedeckten eigenen Vault
    let r = ops::open_vault(&w.dep, &xonly(&keeper), 150_000 * E8 as u64, &w.sim.funds(&keeper), &net);
    w.apply("Keeper-Vault", r);
    let r = ops::mint(&w.dep, 1, &keeper, 200 * E8, &xonly(&keeper), &w.sim.funds(&keeper), &net);
    w.apply("200 GHOST", r);

    // Höchstsatz des Orakels (31,5 % p. a.) über drei Jahre: der Zins übersteigt die Schuld
    let signers: Vec<(usize, Keypair)> = [0usize, 2, 4].iter().map(|&i| (i, w.committee[i])).collect();
    for years in 0..4 {
        w.sim.advance(if years == 0 { 700 } else { 315_360_000 });
        let daa = w.sim.daa - 1;
        let r = ops::oracle_update(&w.dep, &signers, 4_000_000, w.dep.oracle_params.max_rate, daa, &w.sim.funds(&w.deployer), &net);
        w.apply("Orakel-Update (Zins)", r);
    }
    let vp = w.dep.vault_params.clone().unwrap();
    let (price, index) = (w.dep.oracle.state.kas_usd, w.dep.oracle.state.stable_index);
    let st = w.dep.vaults[0].vault.state;
    let coll = w.dep.vaults[0].vault.value as i64;
    let interest = math::accrued(&st, index);
    println!("Index {:.4}, Zins Nutzer-Vault {:.2} USD auf {:.0} GHOST Schuld", index as f64 / 1e9, interest as f64 / 1e8, st.debt as f64 / 1e8);
    assert!(!math::healthy(coll, math::owed(&st, index), price, vp.liq_bps), "Vault ist durch den Zins unterdeckt");
    assert!(!math::sweepable(coll, &st, price, index), "mit Schuld nicht auflösbar");

    // Keeper liquidiert die ganze Schuld; der Rest bleibt mit dem ganzen Zins stehen
    let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&keeper)).map(|(i, _)| i).collect();
    let (_, after) = math::liquidation(coll, &st, st.debt, price, index, vp.liq_bps, vp.bonus_bps).expect("liquidierbar");
    let after = after.expect("Rest ≥ DUST bleibt als Vault");
    let r = ops::liquidate(&w.dep, 0, &keeper, &toks, st.debt, &w.sim.funds(&keeper), &net);
    w.apply("Liquidation", r);
    let z = w.dep.vaults.iter().position(|v| v.owner == xonly(&user)).expect("Rest-Vault");
    let zv = w.dep.vaults[z].vault.clone();
    assert_eq!(zv.state, after);
    assert_eq!(zv.state.debt, 0);
    let rest = zv.value as i64;
    println!("Rest {:.2} KAS = {:.2} USD, Zins {:.2} USD", rest as f64 / 1e8, math::value_of(rest, price) as f64 / 1e8, math::accrued(&zv.state, index) as f64 / 1e8);
    assert!(math::sweepable(rest, &zv.state, price, index), "Zins ≥ Rest: auflösbar");
    // gesunde Vaults (Keeper-Vault) lassen sich nicht auflösen
    let kv = w.dep.vaults.iter().position(|v| v.owner == xonly(&keeper)).unwrap();
    assert!(ops::sweep(&w.dep, kv, &w.sim.funds(&keeper), &net).is_err());

    // Auflösen: jeder darf, hier der Keeper; die Gebühr trägt der Vault
    let treasury_before = w.sim.balance(&w.deployer);
    let keeper_before = w.sim.balance(&keeper);
    let r = ops::sweep(&w.dep, z, &w.sim.funds(&keeper), &net);
    let fee = r.as_ref().map(|(b, _)| b.fee).unwrap_or(0);
    w.apply("Auflösen (sweep)", r);
    assert_eq!(w.sim.balance(&w.deployer) - treasury_before, (rest - math::SWEEP_FEE) as u64, "Zinskasse bekommt die Sicherheit abzüglich SWEEP_FEE");
    assert!((fee as i64) < math::SWEEP_FEE, "SWEEP_FEE deckt die Netzgebühr ({fee} sompi)");
    let gain = w.sim.balance(&keeper) as i64 - keeper_before as i64;
    assert_eq!(gain, math::SWEEP_FEE - fee as i64, "Keeper bekommt den Überschuss, legt nichts drauf");
    println!("Netzgebühr {:.4} KAS, Überschuss an den Keeper {:.4} KAS", fee as f64 / 1e8, gain as f64 / 1e8);
    assert!(w.dep.vaults.iter().all(|v| v.owner != xonly(&user)), "Vault ist beendet");
    // die Orakel-Fortsetzung stimmt: ein weiteres Update geht durch
    w.price(4_100_000);
}

/// Audit 12 A12-18 (Vault-Teil): Rücknahme der ganzen, winzigen Schuld eines
/// Vaults (0,001 GHOST bei 0,04 USD = 0,02475 KAS). Anders als beim Schließen
/// und Abheben (A11-O-12) bekommt der Rücknehmer keinen eigenen Ausgang: Die
/// KAS fließen in sein Wechselgeld. Die Tx ist also nicht zu schwer und wird
/// angenommen, nur ist die Auszahlung kleiner als die Netzgebühr. Darauf weist
/// die Vorprüfung der Seite hin (precheck.ts, audit12-vault.test.ts).
#[test]
fn a12_ruecknahme_mit_winziger_auszahlung_baut() {
    let mut w = setup();
    let net = w.sim.params.clone();
    let (owner, redeemer) = (key(), key());
    w.sim.faucet(&owner, 1_000 * E8 as u64);
    w.sim.faucet(&redeemer, 10 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&owner), 100 * E8 as u64, &w.sim.funds(&owner), &net);
    w.apply("Vault", r);
    let r = ops::mint(&w.dep, 0, &owner, 100_000, &xonly(&redeemer), &w.sim.funds(&owner), &net);
    w.apply("0,001 GHOST an den Rücknehmer", r);
    let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&redeemer)).map(|(i, _)| i).collect();
    let paid = math::redeem_paid(100_000, w.dep.oracle.state.kas_usd) as u64;
    assert_eq!(paid, 2_475_000);
    let r = ops::redeem(&w.dep, 0, &redeemer, &toks, 100_000, &w.sim.funds(&redeemer), &net);
    let b = r.as_ref().map(|(b, _)| b.clone()).expect("Rücknahme baut");
    assert!(b.tx.outputs.iter().all(|o| o.value != paid), "kein eigener Ausgang in Höhe der Auszahlung");
    assert!(b.fee > paid, "Netzgebühr {} sompi > Auszahlung {paid} sompi", b.fee);
    w.apply("Rücknahme 0,001 GHOST", r);
    println!("Auszahlung {:.5} KAS im Wechselgeld, Netzgebühr {:.5} KAS, storage {} g", paid as f64 / 1e8, b.fee as f64 / 1e8, b.storage_mass);
    assert_eq!(w.dep.vaults[0].vault.state.debt, 0);
}
