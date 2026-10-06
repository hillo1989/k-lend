//! Alle Nutzeraktionen von Version 4 über den Wallet-Weg (src/wallet_ops.rs):
//! build_plan → Wallet signiert (nachgebildet mit einem Testschlüssel, wie
//! KasWare `signPskt` bzw. Kastle `signTx`) → submit → Simulator (sim.rs).
//! Dazu Gegenproben: falscher Schlüssel, verändertes Feld, fehlende Signatur,
//! falscher Hashtype, veränderter Plan, veralteter Plan.

use kaspa_addresses::Prefix;
use kaspa_consensus_core::hashing::sighash_type::{SIG_HASH_ALL, SIG_HASH_NONE, SigHashType};
use kaspa_consensus_core::sign::sign_input;
use kaspa_consensus_core::tx::{PopulatedTransaction, Transaction, UtxoEntry};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::{self, E8};
use kaspa_lending_protocol::ops::{self, Deployment, p2pk_spk, xonly};
use kaspa_lending_protocol::pool;
use kaspa_lending_protocol::sim::{Sim, test_register_params};
use kaspa_lending_protocol::txb::Built;
use kaspa_lending_protocol::wallet::{self as w, SafeTx};
use kaspa_lending_protocol::wallet_ops::{self as wo, Action, ActionPlan, Submitted};
use kaspa_txscript::script_builder::ScriptBuilder;
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

const NET: &str = "mainnet";
const P: Prefix = Prefix::Mainnet;

fn addr(k: &Keypair) -> String {
    w::address_of_xonly(&xonly(k), P)
}

#[derive(Clone, Copy, Debug)]
enum Wallet {
    KasWare,
    Kastle,
}

fn parse_req_tx(json: &str) -> (Transaction, Vec<UtxoEntry>) {
    let s: SafeTx = serde_json::from_str(json).unwrap();
    w::from_safe(&s).unwrap()
}

/// Antwort wie eine Wallet: JSON-Text mit dem Safe JSON
fn reply(tx: &Transaction, e: &[UtxoEntry]) -> String {
    serde_json::to_string(&serde_json::to_string(&w::to_safe(tx, e, P)).unwrap()).unwrap()
}

/// KasWare nachgebildet: signPskt({txJsonString, options:{signInputs}}),
/// je Index `0x41 ‖ sig64 ‖ hashtype`
fn kasware_sign(plan: &ActionPlan, k: &Keypair, ty: SigHashType) -> String {
    let req = plan.kasware();
    let (mut tx, e) = parse_req_tx(req["txJsonString"].as_str().unwrap());
    let sk = k.secret_bytes();
    for s in req["options"]["signInputs"].as_array().unwrap() {
        assert_eq!(s["sighashType"], 1);
        let idx = s["index"].as_u64().unwrap() as usize;
        assert!(tx.inputs[idx].signature_script.is_empty());
        tx.inputs[idx].signature_script = sign_input(&PopulatedTransaction::new(&tx, e.clone()), idx, &sk, ty);
    }
    reply(&tx, &e)
}

/// Kastle nachgebildet: signTx(networkId, txJson, scripts): Covenant-Eingänge
/// mit Redeem-Skript dahinter, danach eigene P2PK-Eingänge
fn kastle_sign(plan: &ActionPlan, k: &Keypair) -> String {
    let req = plan.kastle();
    let (mut tx, e) = parse_req_tx(req["txJson"].as_str().unwrap());
    let sk = k.secret_bytes();
    for s in req["scripts"].as_array().unwrap() {
        let idx = s["inputIndex"].as_u64().unwrap() as usize;
        let mut script = sign_input(&PopulatedTransaction::new(&tx, e.clone()), idx, &sk, SIG_HASH_ALL);
        let redeem = w::strict_hex::decode(s["scriptHex"].as_str().unwrap()).unwrap();
        script.extend(ScriptBuilder::with_flags(kaspa_lending_protocol::txb::engine_flags()).add_data(&redeem).unwrap().drain());
        tx.inputs[idx].signature_script = script;
    }
    let own = p2pk_spk(&xonly(k));
    for i in 0..tx.inputs.len() {
        if tx.inputs[i].signature_script.is_empty() && e[i].script_public_key == own {
            tx.inputs[i].signature_script = sign_input(&PopulatedTransaction::new(&tx, e.clone()), i, &sk, SIG_HASH_ALL);
        }
    }
    reply(&tx, &e)
}

struct World {
    sim: Sim,
    dep: Deployment,
    deployer: Keypair,
    committee: Vec<Keypair>,
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new();
        let deployer = key();
        let committee: Vec<Keypair> = (0..5).map(|_| key()).collect();
        sim.faucet(&deployer, 1_000 * E8 as u64);
        let net = sim.params.clone();
        let feed = sim
            .deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: committee.iter().map(xonly).collect(), t: 3, t_rot: 3 }, 4_000_000, 158_548_959, 1_000_000_000)
            .expect("Register und Orakel");
        let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
        let (b, factory) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
        sim.submit(&b).unwrap();
        let (b, f2, root, vp) = ops::init_factory(
            &feed.oracle_params,
            &feed.oracle,
            &fp,
            &factory,
            &deployer,
            (20_000, 15_000, 1_000, NO_DEBT_LIMIT),
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
        World { sim, dep, deployer, committee }
    }

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

    fn utxos(&self, k: &Keypair) -> Vec<(kaspa_consensus_core::tx::TransactionOutpoint, UtxoEntry)> {
        self.sim.funds(k).utxos
    }

    /// Plan bauen; geht als JSON durch den Browser
    fn plan(&self, k: &Keypair, a: &Action) -> Result<ActionPlan, String> {
        let (plan, _) = wo::build_plan(&self.dep, a, &addr(k), NET, &self.utxos(k), P, &self.sim.params)?;
        Ok(ActionPlan::decode(&serde_json::to_string(&plan).unwrap()).unwrap())
    }

    fn submit(&self, plan: &ActionPlan, signed: &str) -> Result<Submitted, String> {
        wo::submit(&self.dep, plan, &w::parse_signed(signed).unwrap(), NET, P, &self.sim.params)
    }

    /// Ganzer Wallet-Weg; der Simulator muss die Tx annehmen
    fn by_wallet(&mut self, label: &str, k: &Keypair, a: Action, wl: Wallet) -> Submitted {
        let plan = self.plan(k, &a).unwrap_or_else(|e| panic!("{label}: Plan: {e}"));
        assert!(plan.signers.iter().all(|s| s.index < plan.tx.inputs.len()));
        assert!(plan.tx.inputs.iter().all(|i| i.signature_script.is_empty()), "{label}: Wallet bekommt leere Signaturskripte");
        let signed = match wl {
            Wallet::KasWare => kasware_sign(&plan, k, SIG_HASH_ALL),
            Wallet::Kastle => kastle_sign(&plan, k),
        };
        // Größen für die Grenzen der Seite (server/walletActions.ts: Plan, Antwort je 320 kB)
        let plan_len = serde_json::to_string(&plan).unwrap().len();
        assert!(plan_len < 320 * 1024 && signed.len() < 320 * 1024, "{label}: Plan {plan_len} B, Antwort {} B", signed.len());
        // ganze Anfrage an /api/wallet/submit: unter dem Limit der Seite
        // (WALLET_BODY_LIMIT 768 kB) und damit unter dem des Webservers (1 MB, A17-5)
        let body = serde_json::json!({ "network": NET, "plan": plan, "signed": signed, "send": true, "confirmMainnet": true }).to_string().len();
        assert!(body < 768 * 1024, "{label}: submit-Anfrage {body} B");
        let s = self.submit(&plan, &signed).unwrap_or_else(|e| panic!("{label}: submit: {e}"));
        assert!(s.report.valid, "{label}: ungültig: {:?} {:?}", s.report.error, s.report.inputs);
        let b = s.built.clone().unwrap();
        println!(
            "{label:<26} {:?} Eingänge {:>2} (Wallet {:>2}) | compute {:>7} g | storage {:>7} g | Gebühr {:.5} KAS | Plan {:>3} kB",
            wl,
            b.tx.inputs.len(),
            plan.signers.len(),
            b.compute_mass,
            b.storage_mass,
            b.fee as f64 / 1e8,
            plan_len / 1024
        );
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: Simulator lehnt ab: {e}"));
        self.dep = s.next.clone().unwrap();
        s
    }

    fn ghost_of(&self, k: &Keypair) -> i64 {
        self.dep.tokens.iter().filter(|t| t.state.owner == xonly(k)).map(|t| t.state.amount).sum()
    }
    fn lp_of(&self, k: &Keypair) -> i64 {
        self.dep.lp_tokens.iter().filter(|t| t.state.owner == xonly(k)).map(|t| t.state.amount).sum()
    }
    fn vault_of(&self, k: &Keypair) -> Vec<usize> {
        self.dep.vaults.iter().enumerate().filter(|(_, v)| v.owner == xonly(k)).map(|(i, _)| i).collect()
    }
}

/// Betreiber mit Schlüsseldatei: Vault, 150 GHOST, Pool mit 1 000 KAS / 40 GHOST
fn with_pool() -> (World, Keypair) {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let owner = key();
    w.sim.faucet(&owner, 20_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&owner), 10_000 * E8 as u64, &w.sim.funds(&owner), &net);
    w.apply("Vault Betreiber", r);
    let r = ops::mint(&w.dep, 0, &owner, 150 * E8, &xonly(&owner), &w.sim.funds(&owner), &net);
    w.apply("150 GHOST", r);
    let gcov = w.dep.vault_params.as_ref().unwrap().ghost_cov;
    let r = pool::pool_create(gcov, pool::PoolBand::of(&w.dep, 1_000_000), &owner, &w.sim.funds(&owner), &net);
    w.pool("Pool-Genesis", r);
    let pend = w.dep.pool_pending.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let r = pool::pool_init(&pend, Some(&pool::OracleRef::of(&w.dep)), &owner, &mine, 4_000_000, &w.sim.funds(&owner), &net);
    w.pool("Pool-init", r);
    let p = w.dep.pool.clone().unwrap();
    let mine = pool::own(&w.dep.tokens, &xonly(&owner), 2);
    let (t, _, _) = pool::add(&p, &owner, &mine, 999 * E8, 3_996_000_000, 1, pool::ADD_TOL_BPS, &w.sim.funds(&owner), &net).unwrap();
    w.pool("Einlegen Betreiber", Ok(t));
    (w, owner)
}

/// Vault-Aktionen, Senden, Überweisen, Tauschen und Pool – alles mit der
/// Browser-Wallet; abwechselnd KasWare und Kastle
#[test]
fn alle_aktionen_ueber_die_wallet() {
    let (mut w, owner) = with_pool();
    let u = key();
    w.sim.faucet(&u, 10_000 * E8 as u64);
    w.sim.faucet(&u, 500 * E8 as u64);

    // Vault
    w.by_wallet("open-vault", &u, Action::OpenVault { kas: 3_000 * E8 as u64 }, Wallet::KasWare);
    let v = *w.vault_of(&u).first().expect("Vault des Nutzers");
    w.by_wallet("mint", &u, Action::Mint { vault: v, ghost: 50 * E8 }, Wallet::Kastle);
    assert_eq!(w.ghost_of(&u), 50 * E8);
    w.by_wallet("deposit", &u, Action::Deposit { vault: v, kas: 100 * E8 as u64 }, Wallet::KasWare);
    assert_eq!(w.dep.vaults[v].vault.value, 3_100 * E8 as u64);
    w.by_wallet("withdraw", &u, Action::Withdraw { vault: v, keep: 3_000 * E8 as u64 }, Wallet::Kastle);
    assert_eq!(w.dep.vaults[v].vault.value, 3_000 * E8 as u64);

    // Überweisen (mit öffentlicher Nachricht) und KAS senden
    let payload = kaspa_lending_protocol::message::payload_for("Danke!", true, None).unwrap();
    w.by_wallet("transfer", &u, Action::Transfer { to: faster_hex::hex_string(&xonly(&owner)), ghost: 5 * E8, payload: payload.clone() }, Wallet::KasWare);
    assert_eq!(w.ghost_of(&u), 45 * E8);
    let before = w.sim.balance(&owner);
    w.by_wallet("send", &u, Action::Send { to: addr(&owner), kas: 10 * E8 as u64, payload: vec![] }, Wallet::Kastle);
    assert_eq!(w.sim.balance(&owner) - before, 10 * E8 as u64);

    // Tauschen in beide Richtungen
    let p = w.dep.pool.clone().unwrap();
    let q = pool::ghost_out(p.kas(), p.ghost(), 10 * E8, p.params.fee_bps);
    let s = w.by_wallet("swap kaufen", &u, Action::Swap { kas: Some(10 * E8), ghost: None, min: q }, Wallet::KasWare);
    assert_eq!(s.info["out"].as_f64().unwrap(), q as f64 / 1e8);
    assert_eq!(w.ghost_of(&u), 45 * E8 + q);
    let p = w.dep.pool.clone().unwrap();
    let kq = pool::kas_out(p.kas(), p.ghost(), E8, p.params.fee_bps);
    w.by_wallet("swap verkaufen", &u, Action::Swap { kas: None, ghost: Some(E8), min: kq }, Wallet::Kastle);
    assert_eq!(w.ghost_of(&u), 44 * E8 + q);

    // Pool: einlegen, halb abziehen
    let p = w.dep.pool.clone().unwrap();
    let dy = 2 * E8;
    let dx = (dy as i128 * p.kas() as i128 / p.ghost() as i128) as i64 + 1;
    let m = pool::shares_for_deposit(p.shares(), p.kas(), p.ghost(), dx, dy);
    w.by_wallet("pool-add", &u, Action::PoolAdd { kas: dx, ghost: dy, min_shares: m }, Wallet::KasWare);
    assert_eq!(w.lp_of(&u), m);
    let p = w.dep.pool.clone().unwrap();
    let (x2, y2) = pool::payout_for(p.shares(), p.kas(), p.ghost(), m / 2);
    w.by_wallet("pool-remove", &u, Action::PoolRemove { shares: m / 2, min_kas: x2, min_ghost: y2 }, Wallet::Kastle);
    assert_eq!(w.lp_of(&u), m - m / 2);

    // Tilgen, Rücknahme am Vault des Betreibers
    let debt = w.dep.vaults[v].vault.state.debt;
    w.by_wallet("repay", &u, Action::Repay { vault: v, ghost: 10 * E8 }, Wallet::KasWare);
    assert_eq!(w.dep.vaults[v].vault.state.debt, debt - 10 * E8);
    let ov = w.vault_of(&owner)[0];
    let coll = w.dep.vaults[ov].vault.value;
    let s = w.by_wallet("redeem", &u, Action::Redeem { vault: ov, ghost: 2 * E8 }, Wallet::Kastle);
    let paid = math::redeem_paid(2 * E8, w.dep.oracle.state.kas_usd) as u64;
    assert_eq!(w.dep.vaults[ov].vault.value, coll - paid);
    assert_eq!(s.info["kas"].as_f64().unwrap(), paid as f64 / 1e8);

    // Zweiter Vault ohne Schuld: schließen
    w.by_wallet("open-vault 2", &u, Action::OpenVault { kas: 200 * E8 as u64 }, Wallet::Kastle);
    let v2 = *w.vault_of(&u).iter().find(|&&i| w.dep.vaults[i].vault.state.debt == 0).unwrap();
    let n = w.dep.vaults.len();
    w.by_wallet("close", &u, Action::Close { vault: v2 }, Wallet::KasWare);
    assert_eq!(w.dep.vaults.len(), n - 1);
}

/// Liquidieren und Auflösen (sweep) über die Wallet: Zombie-Vault wie in
/// e2e_tests::keeper_loest_zombie_vault_zugunsten_der_zinskasse_auf
#[test]
fn liquidieren_und_aufloesen_ueber_die_wallet() {
    let mut w = World::new();
    let net = w.sim.params.clone();
    let user = key();
    let keeper = key();
    w.sim.faucet(&user, 20_000 * E8 as u64);
    w.sim.faucet(&keeper, 200_000 * E8 as u64);
    let r = ops::open_vault(&w.dep, &xonly(&user), 5_000 * E8 as u64, &w.sim.funds(&user), &net);
    w.apply("Vault Nutzer", r);
    let r = ops::mint(&w.dep, 0, &user, 90 * E8, &xonly(&user), &w.sim.funds(&user), &net);
    w.apply("90 GHOST", r);
    // Keeper mit Browser-Wallet besorgt sich GHOST
    w.by_wallet("open-vault (Keeper)", &keeper, Action::OpenVault { kas: 150_000 * E8 as u64 }, Wallet::KasWare);
    let kv = w.vault_of(&keeper)[0];
    w.by_wallet("mint (Keeper)", &keeper, Action::Mint { vault: kv, ghost: 200 * E8 }, Wallet::KasWare);
    // Höchstzins über drei Jahre
    let signers: Vec<(usize, Keypair)> = [0usize, 2, 4].iter().map(|&i| (i, w.committee[i])).collect();
    for years in 0..4 {
        w.sim.advance(if years == 0 { 700 } else { 315_360_000 });
        let daa = w.sim.daa - 1;
        let r = ops::oracle_update(&w.dep, &signers, 4_000_000, w.dep.oracle_params.max_rate, daa, &w.sim.funds(&w.deployer), &net);
        w.apply("Orakel-Update", r);
    }
    let z = w.vault_of(&user)[0];
    let debt = w.dep.vaults[z].vault.state.debt;
    w.by_wallet("liquidate", &keeper, Action::Liquidate { vault: z, ghost: debt }, Wallet::Kastle);
    let z = w.vault_of(&user)[0];
    assert_eq!(w.dep.vaults[z].vault.state.debt, 0);
    let rest = w.dep.vaults[z].vault.value as i64;
    let (price, index) = (w.dep.oracle.state.kas_usd, w.dep.oracle.state.stable_index);
    assert!(math::sweepable(rest, &w.dep.vaults[z].vault.state, price, index));
    let before = w.sim.balance(&w.deployer);
    w.by_wallet("sweep", &keeper, Action::Sweep { vault: z }, Wallet::KasWare);
    assert_eq!(w.sim.balance(&w.deployer) - before, (rest - math::SWEEP_FEE) as u64);
    assert!(w.vault_of(&user).is_empty());
}

// ---------------------------------------------------------- Gegenproben ----

/// Mint: Vault-Eingang mit Besitzersignatur (sig_at), Token-Ausgang, P2PK
fn mint_setup() -> (World, Keypair, ActionPlan) {
    let mut w = World::new();
    let u = key();
    w.sim.faucet(&u, 5_000 * E8 as u64);
    w.by_wallet("open-vault", &u, Action::OpenVault { kas: 2_000 * E8 as u64 }, Wallet::KasWare);
    let plan = w.plan(&u, &Action::Mint { vault: 0, ghost: 10 * E8 }).unwrap();
    assert!(plan.signers.iter().any(|s| s.kind == "entry" && s.entry.as_deref() == Some("mint")), "Vault-Eingang signiert die Wallet");
    (w, u, plan)
}

fn rejected(s: Result<Submitted, String>, what: &str) -> String {
    match s {
        Ok(s) => {
            assert!(!s.report.valid && s.built.is_none(), "{what}: hätte abgelehnt werden müssen");
            s.report.error.unwrap_or_default()
        }
        Err(e) => e,
    }
}

#[test]
fn falscher_schluessel_wird_abgelehnt() {
    let (w, _, plan) = mint_setup();
    let fremd = key();
    let e = rejected(w.submit(&plan, &kasware_sign(&plan, &fremd, SIG_HASH_ALL)), "fremder Schlüssel");
    assert!(e.contains("Signatur fehlt oder ist ungültig"), "{e}");
}

#[test]
fn veraendertes_feld_in_der_wallet_antwort_wird_abgelehnt() {
    let (w, u, plan) = mint_setup();
    let signed = kasware_sign(&plan, &u, SIG_HASH_ALL);
    let mut v: serde_json::Value = serde_json::from_str(&serde_json::from_str::<String>(&signed).unwrap()).unwrap();
    // Wechselgeld um 1 sompi verringern (mehr Gebühr) – Ausgänge deckt die Signatur ab
    let last = v["outputs"].as_array().unwrap().len() - 1;
    let val: u64 = v["outputs"][last]["value"].as_str().unwrap().parse().unwrap();
    v["outputs"][last]["value"] = serde_json::json!((val - 1).to_string());
    let e = rejected(w.submit(&plan, &v.to_string()), "Betrag verändert");
    assert!(e.contains("verändert") && e.contains("Betrag"), "{e}");
    // Locktime
    let mut v2: serde_json::Value = serde_json::from_str(&serde_json::from_str::<String>(&signed).unwrap()).unwrap();
    v2["lockTime"] = serde_json::json!("5");
    let e = rejected(w.submit(&plan, &v2.to_string()), "Locktime verändert");
    assert!(e.contains("Locktime"), "{e}");
    // Budget und Speichermasse dürfen abweichen (nicht signiert, überschrieben)
    let mut v3: serde_json::Value = serde_json::from_str(&serde_json::from_str::<String>(&signed).unwrap()).unwrap();
    v3["inputs"][0]["computeBudget"] = serde_json::json!(1);
    v3["storageMass"] = serde_json::json!("0");
    let s = w.submit(&plan, &v3.to_string()).unwrap();
    assert!(s.report.valid, "{:?}", s.report.error);
    assert!(s.report.ignored.iter().any(|x| x.contains("Compute-Budget")) && s.report.ignored.iter().any(|x| x.contains("Speichermasse")));
}

#[test]
fn fehlende_signatur_wird_abgelehnt() {
    let (w, u, plan) = mint_setup();
    let signed = kasware_sign(&plan, &u, SIG_HASH_ALL);
    for sp in &plan.signers {
        let mut v: serde_json::Value = serde_json::from_str(&serde_json::from_str::<String>(&signed).unwrap()).unwrap();
        v["inputs"][sp.index]["signatureScript"] = serde_json::json!("");
        let s = w.submit(&plan, &v.to_string()).unwrap();
        assert!(!s.report.valid && s.built.is_none(), "Eingang {} ohne Signatur angenommen", sp.index);
        assert!(s.report.inputs.iter().any(|i| i.index == sp.index && !i.signed), "Bericht nennt Eingang {}", sp.index);
    }
}

#[test]
fn falscher_hashtype_wird_abgelehnt() {
    let (w, u, plan) = mint_setup();
    let s = w.submit(&plan, &kasware_sign(&plan, &u, SIG_HASH_NONE)).unwrap();
    assert!(!s.report.valid && s.built.is_none());
    assert!(s.report.inputs.iter().all(|i| i.hash_type == Some(SIG_HASH_NONE.to_u8()) && !i.sig_valid), "{:?}", s.report.inputs);
}

#[test]
fn veraenderter_plan_wird_abgelehnt() {
    let (w, u, plan) = mint_setup();
    // Betrag in der Aktion erhöht: der Neubau ergibt eine andere Tx
    let mut p2 = plan.clone();
    p2.action = Action::Mint { vault: 0, ghost: 11 * E8 };
    let e = w.submit(&p2, &kasware_sign(&p2, &u, SIG_HASH_ALL)).err().expect("anderer Betrag");
    assert!(e.contains("Plan passt nicht"), "{e}");
    // Ausgang in der unsignierten Tx umgeleitet (die Wallet signiert, was im Plan steht)
    let mut p3 = plan.clone();
    let fremd = key();
    let last = p3.tx.outputs.len() - 1;
    p3.tx.outputs[last].script_public_key = p2pk_spk(&xonly(&fremd));
    let e = w.submit(&p3, &kasware_sign(&p3, &u, SIG_HASH_ALL)).err().expect("Wechselgeld umgeleitet");
    assert!(e.contains("Plan passt nicht"), "{e}");
    // Liste der zu signierenden Eingänge gekürzt
    let mut p4 = plan.clone();
    p4.signers.pop();
    let e = w.submit(&p4, &kasware_sign(&p4, &u, SIG_HASH_ALL)).err().expect("Signierliste verändert");
    assert!(e.contains("Plan passt nicht"), "{e}");
    // anderes Netz
    let mut p5 = plan.clone();
    p5.network = "testnet-10".into();
    assert!(w.submit(&p5, &kasware_sign(&plan, &u, SIG_HASH_ALL)).is_err());
    // das Original geht
    assert!(w.submit(&plan, &kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap().report.valid);
}

#[test]
fn veralteter_plan_wird_abgelehnt() {
    let (mut w, u, plan) = mint_setup();
    let signed = kasware_sign(&plan, &u, SIG_HASH_ALL);
    // jemand anderes nutzt inzwischen das Orakel (eigener Vault)
    let other = key();
    w.sim.faucet(&other, 3_000 * E8 as u64);
    let net = w.sim.params.clone();
    let r = ops::open_vault(&w.dep, &xonly(&other), 1_000 * E8 as u64, &w.sim.funds(&other), &net);
    w.apply("fremder Vault", r);
    let r = ops::mint(&w.dep, 1, &other, E8, &xonly(&other), &w.sim.funds(&other), &net);
    w.apply("fremdes mint", r);
    let e = w.submit(&plan, &signed).err().expect("Orakel bewegt");
    assert!(e.contains("Plan passt nicht"), "{e}");
}

/// Nur der Besitzer: mint am fremden Vault baut schon keinen Plan
#[test]
fn fremder_vault_baut_keinen_plan() {
    let (w, _, _) = mint_setup();
    let fremd = key();
    let mut w = w;
    w.sim.faucet(&fremd, 100 * E8 as u64);
    let e = w.plan(&fremd, &Action::Mint { vault: 0, ghost: E8 }).err().unwrap();
    assert!(e.contains("gehört nicht"), "{e}");
    let e = w.plan(&fremd, &Action::Withdraw { vault: 0, keep: E8 as u64 }).err().unwrap();
    assert!(e.contains("gehört nicht"), "{e}");
}

/// Die Wallet bekommt dieselben Budgets wie ghostctl mit Schlüsseldatei,
/// und die gesendete Tx ist bis auf Signaturen und Schlüssel gleich groß
#[test]
fn messkopie_entspricht_dem_bau_mit_schluessel() {
    let mut w = World::new();
    let u = key();
    w.sim.faucet(&u, 5_000 * E8 as u64);
    w.by_wallet("open-vault", &u, Action::OpenVault { kas: 2_000 * E8 as u64 }, Wallet::KasWare);
    let net = w.sim.params.clone();
    let (direct, _) = ops::mint(&w.dep, 0, &u, 10 * E8, &xonly(&u), &w.sim.funds(&u), &net).unwrap();
    let plan = w.plan(&u, &Action::Mint { vault: 0, ghost: 10 * E8 }).unwrap();
    let s = w.submit(&plan, &kastle_sign(&plan, &u)).unwrap();
    let b = s.built.unwrap();
    assert_eq!(b.budgets, direct.budgets);
    assert_eq!(b.fee, direct.fee);
    assert_eq!((b.compute_mass, b.storage_mass), (direct.compute_mass, direct.storage_mass));
    assert_eq!(b.tx.outputs, direct.tx.outputs, "gleiche Ausgänge wie mit Schlüsseldatei");
    assert!(!s.report.budgets_raised);
}

/// Aktion, deren eigene GHOST auf zu vielen UTXOs liegen: klare Meldung
#[test]
fn zu_viele_ghost_utxos_meldet_zusammenfuehren() {
    let mut w = World::new();
    let u = key();
    w.sim.faucet(&u, 5_000 * E8 as u64);
    w.by_wallet("open-vault", &u, Action::OpenVault { kas: 3_000 * E8 as u64 }, Wallet::KasWare);
    for n in 0..3 {
        w.by_wallet(&format!("mint {n}"), &u, Action::Mint { vault: 0, ghost: 10 * E8 }, Wallet::KasWare);
    }
    let e = w.plan(&u, &Action::Repay { vault: 0, ghost: 25 * E8 }).err().unwrap();
    assert!(e.contains("zusammenführen"), "{e}");
    // zusammenführen: an sich selbst, dann geht es
    w.by_wallet("transfer an sich", &u, Action::Transfer { to: addr(&u), ghost: 30 * E8, payload: vec![] }, Wallet::Kastle);
    w.by_wallet("repay", &u, Action::Repay { vault: 0, ghost: 25 * E8 }, Wallet::KasWare);
    assert_eq!(w.dep.vaults[0].vault.state.debt, 5 * E8);
}

// ------------------------------------------------------------ Audit 17 ----

/// A17-1: Der Ersatzschlüssel ist bei jedem Bau neu und zufällig, nie der
/// früher feste, öffentlich bekannte 0x42…
#[test]
fn ersatzschluessel_ist_je_bau_zufaellig() {
    let old = Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[0x42; 32]).unwrap());
    let (a, b) = (w::mirror_key(), w::mirror_key());
    assert_ne!(a.secret_bytes(), b.secret_bytes(), "zwei Baue, zwei Schlüssel");
    assert_ne!(a.secret_bytes(), old.secret_bytes());
    assert_ne!(b.secret_bytes(), old.secret_bytes());
}

/// Nutzer A (Vault 0) und V (Vault 1) mit je 10 GHOST
fn zwei_nutzer() -> (World, Keypair, Keypair) {
    let mut w = World::new();
    let (a, v) = (key(), key());
    w.sim.faucet(&a, 5_000 * E8 as u64);
    w.sim.faucet(&v, 5_000 * E8 as u64);
    w.by_wallet("open-vault A", &a, Action::OpenVault { kas: 2_000 * E8 as u64 }, Wallet::KasWare);
    w.by_wallet("mint A", &a, Action::Mint { vault: 0, ghost: 10 * E8 }, Wallet::KasWare);
    w.by_wallet("open-vault V", &v, Action::OpenVault { kas: 2_000 * E8 as u64 }, Wallet::KasWare);
    w.by_wallet("mint V", &v, Action::Mint { vault: 1, ghost: 10 * E8 }, Wallet::KasWare);
    (w, a, v)
}

/// Angriff aus Audit 17 (A17-1) nachgestellt: A überweist Staub-GHOST an den
/// Ersatzschlüssel. Danach muss V weiter überweisen, tilgen und zurücknehmen
/// können – mit dem alten festen Schlüssel 0x42 und auch dann, wenn der
/// Angreifer den Ersatzschlüssel des Baus kennt (zweite Linie: mirror_dep
/// neutralisiert, was dem Ersatzschlüssel vorher gehörte).
#[test]
fn a17_1_staub_an_den_ersatzschluessel_sperrt_niemanden() {
    // a) der früher feste, öffentlich bekannte Schlüssel
    let (mut w, a, v) = zwei_nutzer();
    let old = Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[0x42; 32]).unwrap());
    w.by_wallet("Staub an 0x42", &a, Action::Transfer { to: faster_hex::hex_string(&xonly(&old)), ghost: 1, payload: vec![] }, Wallet::KasWare);
    w.by_wallet("transfer V", &v, Action::Transfer { to: addr(&a), ghost: E8, payload: vec![] }, Wallet::KasWare);
    // b) der Angreifer kennt den Ersatzschlüssel dieses Baus (bzw. hat ihn erraten)
    let k = key();
    let (mut w, a, v) = zwei_nutzer();
    let sk = k.secret_bytes();
    w::with_mirror_key(sk, || {
        assert_eq!(w::mirror_key().secret_bytes(), sk);
        w.by_wallet("Staub an Ersatzschlüssel", &a, Action::Transfer { to: faster_hex::hex_string(&xonly(&k)), ghost: 1, payload: vec![] }, Wallet::KasWare);
        w.by_wallet("transfer V", &v, Action::Transfer { to: addr(&a), ghost: E8, payload: vec![] }, Wallet::KasWare);
        w.by_wallet("repay V", &v, Action::Repay { vault: 1, ghost: 2 * E8 }, Wallet::Kastle);
        w.by_wallet("redeem V", &v, Action::Redeem { vault: 0, ghost: E8 }, Wallet::KasWare);
    });
    assert_eq!(w.ghost_of(&v), 6 * E8);
    // der Staub liegt weiter beim Ersatzschlüssel
    assert_eq!(w.ghost_of(&k), 1);
}

/// A17-1, zweite Linie direkt: mirror_dep gibt Vorbesitz des Ersatzschlüssels
/// einem neutralen Besitzer, der weder Nutzer noch Ersatzschlüssel ist
#[test]
fn messkopie_neutralisiert_vorbesitz_des_ersatzschluessels() {
    let (mut w, a, v) = zwei_nutzer();
    let k = key();
    w.by_wallet("Staub", &a, Action::Transfer { to: faster_hex::hex_string(&xonly(&k)), ghost: 1, payload: vec![] }, Wallet::KasWare);
    let md = wo::mirror_dep(&w.dep, &xonly(&v), &xonly(&k));
    let of = |d: &Deployment, x: &[u8]| d.tokens.iter().filter(|t| t.state.owner == x).map(|t| t.state.amount).sum::<i64>();
    assert_eq!(of(&md, &xonly(&k)), 10 * E8, "nur die GHOST von V gehören in der Messkopie dem Ersatzschlüssel");
    assert_eq!(of(&md, &xonly(&v)), 0);
    assert_eq!(md.tokens.len(), w.dep.tokens.len());
    assert!(md.vaults[1].owner == xonly(&k) && md.vaults[0].owner == xonly(&a));
}

/// Lücke aus Audit 17 (RM3): Messkopie und echte Tx werden verglichen; eine
/// abweichende Messkopie bricht den Bau ab
#[test]
fn messkopie_gleich_echte_tx() {
    let (w, _, v) = zwei_nutzer();
    let net = w.sim.params.clone();
    let f = w.sim.funds(&v);
    let (b10, _) = ops::mint(&w.dep, 1, &v, 10 * E8, &xonly(&v), &f, &net).unwrap();
    let (b10b, _) = ops::mint(&w.dep, 1, &v, 10 * E8, &xonly(&v), &f, &net).unwrap();
    assert!(wo::same_shape(&b10, &b10b).is_ok());
    // gleiche Form, anderer Betrag eines Ausgangs (Einzahlen 100 bzw. 101 KAS)
    let (d100, _) = ops::deposit(&w.dep, 1, &v, 100 * E8 as u64, &f, &net).unwrap();
    let (d101, _) = ops::deposit(&w.dep, 1, &v, 101 * E8 as u64, &f, &net).unwrap();
    let e = wo::same_shape(&d100, &d101).unwrap_err();
    assert!(e.contains("weichen ab"), "{e}");
    // andere Aktion, andere Zahl der Ein-/Ausgänge
    assert!(wo::same_shape(&b10, &d100).is_err());
    let mut fee = b10b.clone();
    fee.fee += 1;
    assert!(wo::same_shape(&b10, &fee).unwrap_err().contains("Gebühr"));
    let mut m = b10b.clone();
    m.compute_mass += 1;
    assert!(wo::same_shape(&b10, &m).unwrap_err().contains("Masse"));
    let mut c = b10b.clone();
    c.change_index = None;
    assert!(wo::same_shape(&b10, &c).unwrap_err().contains("Wechselgeld"));
}

/// A17-4: Die Vorprüfung ohne Netz und ohne Sperre weist Müll-Signaturen und
/// fremde Schlüssel ab; eine echte Signatur kommt durch
#[test]
fn vorpruefung_ohne_netz_weist_muell_signaturen_ab() {
    let (_w, u, plan) = mint_setup();
    let good = w::parse_signed(&kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap();
    assert_eq!(wo::precheck(&plan, &good, NET, P).unwrap().error, None);
    let fremd = w::parse_signed(&kasware_sign(&plan, &key(), SIG_HASH_ALL)).unwrap();
    assert!(wo::precheck(&plan, &fremd, NET, P).unwrap().error.unwrap().contains("ungültig"));
    let none = w::parse_signed(&kasware_sign(&plan, &u, SIG_HASH_NONE)).unwrap();
    assert!(wo::precheck(&plan, &none, NET, P).unwrap().error.is_some());
    // Müll im Signaturskript
    let mut v: serde_json::Value = serde_json::from_str(&serde_json::from_str::<String>(&kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap()).unwrap();
    for sp in &plan.signers {
        v["inputs"][sp.index]["signatureScript"] = serde_json::json!(format!("41{}01", "ab".repeat(64)));
    }
    let junk = w::parse_signed(&v.to_string()).unwrap();
    assert!(wo::precheck(&plan, &junk, NET, P).unwrap().error.is_some());
    // Plan einer fremden Adresse (Schlüssel passt nicht zur Adresse) bzw. falsches Netz
    let mut p2 = plan.clone();
    p2.owner = xonly(&key());
    assert!(wo::precheck(&p2, &good, NET, P).is_err());
    assert!(wo::precheck(&plan, &good, "testnet-10", P).is_err());
}

/// Audit 20 A20b-4: aufgeblasene Pläne (Signer-Einträge doppelt, mehr als
/// Eingänge, viele Ausgänge) scheitern an der Form, bevor ein Sighash
/// gerechnet wird – in Mikrosekunden statt 0,8 s
#[test]
fn a20b_4_aufgeblasener_plan_scheitert_vor_der_signaturpruefung() {
    let (_w, u, plan) = mint_setup();
    let good = w::parse_signed(&kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap();
    assert_eq!(wo::precheck(&plan, &good, NET, P).unwrap().error, None);
    // derselbe Eingang 5 000-mal
    let mut big = plan.clone();
    let first = big.signers[0].clone();
    big.signers = vec![first.clone(); 5_000];
    let t = std::time::Instant::now();
    let e = wo::precheck(&big, &good, NET, P).unwrap_err();
    assert!(e.contains("zu signierende Eingänge"), "{e}");
    assert!(t.elapsed() < std::time::Duration::from_millis(50), "{:?}", t.elapsed());
    // doppelt, aber nicht mehr als Eingänge
    let mut dup = plan.clone();
    dup.signers = vec![first.clone(), first.clone()];
    assert!(wo::precheck(&dup, &good, NET, P).unwrap_err().contains("mehrfach"));
    // Index außerhalb
    let mut out = plan.clone();
    out.signers[0].index = 99;
    assert!(wo::precheck(&out, &good, NET, P).unwrap_err().contains("gibt es nicht"));
    // 700 zusätzliche Ausgänge im Plan
    let mut wide = plan.clone();
    let (mut tx, e) = w::from_safe(&wide.tx).unwrap();
    let o = tx.outputs[0].clone();
    tx.outputs.extend(std::iter::repeat_n(o, 700));
    wide.tx = w::to_safe(&tx, &e, P);
    assert!(wo::precheck(&wide, &good, NET, P).unwrap_err().contains("Ausgänge"));
    // Wallet-Antwort mit anderer Zahl von Ausgängen: Bericht, kein Sighash
    let (mut stx, se) = w::from_safe(&good).unwrap();
    let o = stx.outputs[0].clone();
    stx.outputs.push(o);
    let rep = wo::precheck(&plan, &w::to_safe(&stx, &se, P), NET, P).unwrap();
    assert!(rep.error.unwrap().contains("verändert"));
}

/// Ausgänge beim Namen: Vault eröffnen, prägen, tilgen (Anzeige vor dem Signieren)
#[test]
fn ausgaenge_werden_beim_namen_genannt() {
    let mut w = World::new();
    let u = key();
    w.sim.faucet(&u, 5_000 * E8 as u64);
    let whats = |w: &World, p: &ActionPlan| -> Vec<String> {
        p.describe_outputs_in(Some(&w.dep), P).iter().map(|o| o["what"].as_str().unwrap().to_string()).collect()
    };
    let open = w.plan(&u, &Action::OpenVault { kas: 200 * E8 as u64 }).unwrap();
    let names = whats(&w, &open);
    assert!(names.contains(&"Factory (läuft weiter)".to_string()), "{names:?}");
    assert!(names.contains(&"GHOST-Wurzel (läuft weiter)".to_string()), "{names:?}");
    assert!(names.contains(&"Dein neuer Vault – Sicherheit 200 KAS".to_string()), "{names:?}");
    assert!(names.contains(&"Minter-Zweig des neuen Vaults – 3 KAS, bleiben dauerhaft gebunden".to_string()), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("Covenant")), "{names:?}");
    w.by_wallet("open-vault", &u, Action::OpenVault { kas: 200 * E8 as u64 }, Wallet::KasWare);
    let mint = w.plan(&u, &Action::Mint { vault: 0, ghost: E8 }).unwrap();
    let names = whats(&w, &mint);
    for want in ["Dein Vault 1 – Sicherheit 200 KAS", "Orakel (läuft weiter)", "Minter-Zweig deines Vaults 1 (läuft weiter)"] {
        assert!(names.contains(&want.to_string()), "{want}: {names:?}");
    }
    assert!(names.iter().any(|n| n.starts_with("GHOST-Token")), "{names:?}");
}

// ------------------------------------------------------------- Tresore ----
//
// Daueraufträge mit Tresor (contracts/standing_order.sil) über die Wallet:
// anlegen, auffüllen, kündigen; Zustand ist die Tresor-Datei (TresorBasis).
// Zahlen bleibt beim Agenten (tresor::pay_round ohne Nutzerschlüssel).

use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint};
use kaspa_lending_protocol::tresor::{self, TresorFile};
use wo::{Basis, TresorBasis};

/// 2027-02-01 08:00 UTC (der Simulator beginnt am 2027-01-01)
const FIRST_DUE: i64 = 1_801_468_800_000;

struct TresorWorld {
    sim: Sim,
    basis: TresorBasis,
}

impl TresorWorld {
    fn new() -> Self {
        let sim = Sim::new();
        let pmt = sim.now_ms as i64;
        TresorWorld { sim, basis: TresorBasis { file: TresorFile::empty(NET), now: "2027-01-01 10:00".into(), pmt } }
    }
    fn plan(&self, k: &Keypair, a: &Action) -> Result<ActionPlan, String> {
        let (plan, _) = wo::build_plan(&self.basis, a, &addr(k), NET, &self.sim.funds(k).utxos, P, &self.sim.params)?;
        Ok(ActionPlan::decode(&serde_json::to_string(&plan).unwrap()).unwrap())
    }
    fn submit(&self, plan: &ActionPlan, signed: &str) -> Result<Submitted<TresorBasis>, String> {
        wo::submit(&self.basis, plan, &w::parse_signed(signed).unwrap(), NET, P, &self.sim.params)
    }
    /// Ganzer Wallet-Weg; der Simulator muss die Tx annehmen, danach gilt der
    /// Folgezustand (wie die Übernahme über das Journal bei Annahme)
    fn by_wallet(&mut self, label: &str, k: &Keypair, a: Action, wl: Wallet) -> Submitted<TresorBasis> {
        let plan = self.plan(k, &a).unwrap_or_else(|e| panic!("{label}: Plan: {e}"));
        assert!(plan.tx.inputs.iter().all(|i| i.signature_script.is_empty()), "{label}: Wallet bekommt leere Signaturskripte");
        let signed = match wl {
            Wallet::KasWare => kasware_sign(&plan, k, SIG_HASH_ALL),
            Wallet::Kastle => kastle_sign(&plan, k),
        };
        let body = serde_json::json!({ "network": NET, "plan": plan, "signed": signed, "send": true, "confirmMainnet": true }).to_string().len();
        assert!(body < 768 * 1024, "{label}: submit-Anfrage {body} B");
        let s = self.submit(&plan, &signed).unwrap_or_else(|e| panic!("{label}: submit: {e}"));
        assert!(s.report.valid, "{label}: ungültig: {:?} {:?}", s.report.error, s.report.inputs);
        let b = s.built.clone().unwrap();
        println!(
            "{label:<22} {wl:?} Eingänge {} (Wallet {}) | Gebühr {:.5} KAS | Anfrage {} kB",
            b.tx.inputs.len(),
            plan.signers.len(),
            b.fee as f64 / 1e8,
            body / 1024
        );
        self.sim.submit(&b).unwrap_or_else(|e| panic!("{label}: Simulator lehnt ab: {e}"));
        self.basis = s.next.clone().unwrap();
        s
    }
    /// Wert der UTXO des Tresors `i` im Simulator (Outpoint, Covenant-ID und
    /// Skript wie in der Datei), None = keine
    fn utxo_of(&self, i: usize) -> Option<u64> {
        let r = &self.basis.file.tresore[i];
        let spk = tresor::TresorShape::of(&r.params).spk(&r.utxo.state);
        self.sim.utxos.iter().find(|(o, e)| **o == r.utxo.outpoint && e.covenant_id == Some(r.utxo.cov) && e.script_public_key == spk).map(|(_, e)| e.amount)
    }
}

/// Miete: 10 KAS monatlich am 1., 08:00 UTC
fn miete(to: &Keypair, count: i64, fund: u64, message: &str) -> Action {
    Action::TresorOpen { to: addr(to), amount: 10 * E8, anchor_day: 1, period_ms: 0, first_due: FIRST_DUE, count, fund, max_fee: tresor::DEFAULT_MAX_FEE, message: message.into() }
}

fn cov_of(tw: &TresorWorld, i: usize) -> String {
    tw.basis.file.tresore[i].utxo.cov.to_string()
}

fn whats(tw: &TresorWorld, p: &ActionPlan) -> Vec<String> {
    p.describe_tresor_outputs(Some(&tw.basis.file), P).iter().map(|o| o["what"].as_str().unwrap().to_string()).collect()
}

/// Agent im Simulator (tresor::pay_round, ohne Schlüssel des Nutzers)
struct AgentIo<'a> {
    sim: &'a mut Sim,
    key: Option<Keypair>,
}

impl tresor::TresorIo for AgentIo<'_> {
    async fn utxos(&mut self, s: &ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        Ok(self.sim.utxos.iter().filter(|(_, e)| e.script_public_key == *s).map(|(o, e)| (*o, e.clone())).collect())
    }
    async fn funds(&mut self) -> Option<ops::Funds> {
        self.key.map(|k| self.sim.funds(&k))
    }
    async fn send(&mut self, _: &str, b: &Built, _: &TresorFile) -> Result<(), String> {
        self.sim.submit(b)
    }
    fn save(&mut self, _: &TresorFile) -> Result<(), String> {
        Ok(())
    }
    fn journal_open(&self) -> bool {
        false
    }
    fn dry_run(&self) -> bool {
        false
    }
    fn now_ms(&self) -> i64 {
        self.sim.now_ms as i64
    }
    fn say(&mut self, _: &str) {}
}

/// Anlegen, auffüllen, Zahlung durch den Agenten, kündigen – alles, was der
/// Nutzer signiert, über die Browser-Wallet (KasWare und Kastle); Messkopie
/// und echte Tx gleich, Ergebnis wie mit Schlüsseldatei
#[tokio::test]
async fn tresor_anlegen_auffuellen_zahlen_kuendigen_ueber_die_wallet() {
    let mut tw = TresorWorld::new();
    let (u, empf, agent) = (key(), key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.sim.faucet(&agent, 5 * E8 as u64);

    // anlegen: 3 Zahlungen, Startguthaben 3 × (10 + 0,01) + 1 KAS, öffentliche Nachricht
    let fund = 3 * (10 * E8 as u64 + tresor::DEFAULT_MAX_FEE as u64) + E8 as u64;
    let plan = tw.plan(&u, &miete(&empf, 3, fund, "Miete Whg. 3")).unwrap();
    let names = whats(&tw, &plan);
    assert!(names.contains(&format!("Dein neuer Tresor – 31,03 KAS, zahlt 10 KAS monatlich am 1. an {}", addr(&empf))), "{names:?}");
    assert!(plan.signers.iter().all(|s| s.kind == "p2pk"), "anlegen: nur eigene KAS");
    let s = tw.by_wallet("tresor-open", &u, miete(&empf, 3, fund, "Miete Whg. 3"), Wallet::KasWare);
    assert_eq!(tw.basis.file.tresore.len(), 1);
    let r = tw.basis.file.tresore[0].clone();
    assert!(r.wallet && r.key.is_none(), "Wallet-Tresor ohne Schlüsseldatei");
    assert_eq!((r.params.owner.clone(), r.params.recipient.clone()), (xonly(&u), xonly(&empf)), "Besitzer = Wallet");
    assert_eq!((r.message.as_str(), r.onchain), ("Miete Whg. 3", true));
    assert_eq!(r.params.payload_hash, payload_hash(b"Miete Whg. 3"), "Nachricht im Vertrag gebunden");
    assert_eq!(r.message_check(None), tresor::MessageCheck::Bound);
    assert_eq!(r.history.last().unwrap().txid, Some(s.built.as_ref().unwrap().tx.id().to_string()));
    assert_eq!(s.info["tresor"], r.id);
    assert_eq!(s.info["covered"], 3);
    assert_eq!(tw.utxo_of(0), Some(fund), "Tresor-UTXO im Netz");

    // gleiche Tx wie `tresor open` mit Schlüsseldatei
    let mut d = Sim::new();
    d.faucet(&u, 500 * E8 as u64);
    let direct = tresor::open(&r.params, &TresorState { next_due: FIRST_DUE, left: 3 }, fund, &d.funds(&u), &d.params).unwrap().0;
    let b = s.built.as_ref().unwrap();
    assert_eq!((b.fee, b.budgets.clone(), b.compute_mass, b.storage_mass), (direct.fee, direct.budgets.clone(), direct.compute_mass, direct.storage_mass));
    assert_eq!(b.tx.outputs, direct.tx.outputs, "gleiche Ausgänge wie mit Schlüsseldatei");

    // auffüllen (Kastle): Tresor-Eingang mit Besitzersignatur + eigene KAS
    let id = cov_of(&tw, 0);
    let a = Action::TresorTopup { tresor: id.clone(), kas: 5 * E8 as u64 };
    let plan = tw.plan(&u, &a).unwrap();
    assert!(plan.signers.iter().any(|s| s.kind == "entry" && s.entry.as_deref() == Some("topUp") && s.index == 0), "{:?}", plan.signers);
    let names = whats(&tw, &plan);
    assert!(names[0].starts_with(&format!("Dein Tresor {} – 36,03 KAS, zahlt 10 KAS monatlich am 1. an kaspa:", r.id)), "{names:?}");
    let s = tw.by_wallet("tresor-topup", &u, a, Wallet::Kastle);
    assert_eq!(tw.utxo_of(0), Some(fund + 5 * E8 as u64));
    assert_eq!(tw.basis.file.tresore[0].utxo.value, fund + 5 * E8 as u64, "Datei nachgeführt");
    assert_eq!(tw.basis.file.tresore[0].history.last().unwrap().action, "topup");
    assert_eq!(s.info["covered"], 3);

    // Zahlung zum Termin: der Agent mit eigenem Schlüssel, Gebühr aus dem Tresor
    tw.sim.set_time(FIRST_DUE as u64 + 10 * 60_000);
    let (params, pmt) = (tw.sim.params.clone(), tw.sim.now_ms as i64 - tresor::PMT_LAG_MS);
    let before = (tw.sim.balance(&empf), tw.sim.balance(&agent));
    let mut file = tw.basis.file.clone();
    let reps = tresor::pay_round(&mut AgentIo { sim: &mut tw.sim, key: Some(agent) }, &mut file, None, true, pmt, "x", &params).await.unwrap();
    assert_eq!(reps.iter().map(|r| r.paid).collect::<Vec<_>>(), vec![true], "{reps:?}");
    assert_eq!(tw.sim.balance(&empf) - before.0, 10 * E8 as u64, "Empfänger bekommt den Betrag");
    assert_eq!(tw.sim.balance(&agent), before.1, "der Agent zahlt keine Gebühr");
    assert_eq!(file.tresore[0].utxo.state.left, 2);
    tw.basis.file = file;
    let rest = tw.basis.file.tresore[0].utxo.value;
    assert!(rest > fund + 5 * E8 as u64 - 10 * E8 as u64 - tresor::DEFAULT_MAX_FEE as u64, "nur die nötige Gebühr");

    // kündigen (KasWare): nur der Tresor-Eingang, Gebühr aus dem Tresor
    let a = Action::TresorCancel { tresor: id.clone() };
    let plan = tw.plan(&u, &a).unwrap();
    assert_eq!(plan.signers.len(), 1, "keine eigenen KAS nötig: {:?}", plan.signers);
    assert_eq!(plan.signers[0].entry.as_deref(), Some("cancel"));
    assert_eq!(whats(&tw, &plan), vec!["Rest des Tresors zurück an die Wallet".to_string()]);
    let before = tw.sim.balance(&u);
    let s = tw.by_wallet("tresor-cancel", &u, a, Wallet::KasWare);
    let b = s.built.unwrap();
    assert_eq!(tw.sim.balance(&u) - before, rest - b.fee, "Rest zurück an den Besitzer");
    assert!(tw.basis.file.tresore[0].ended.is_some(), "in der Datei beendet");
    assert!(tw.utxo_of(0).is_none());
    // gleiche Tx wie `tresor cancel` mit Schlüsseldatei
    let r = &tw.basis.file.tresore[0];
    let direct = tresor::cancel(&r.params, &r.utxo, &u, &tw.sim.params).unwrap();
    assert_eq!((direct.tx.outputs.clone(), direct.fee), (b.tx.outputs.clone(), b.fee));
    // danach nichts mehr
    let e = tw.plan(&u, &Action::TresorCancel { tresor: id.clone() }).unwrap_err();
    assert!(e.contains("schon gekündigt"), "{e}");
}

/// Neubau bitgleich, sonst abgelehnt; fehlende oder fremde Signatur
/// abgelehnt; einen Folgezustand (also einen Eintrag in der Tresor-Datei)
/// gibt es nur bei gültiger Signatur, und build/submit ändern die Datei nicht
#[test]
fn tresor_plan_wird_neu_gebaut_und_geprueft() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    let a = miete(&empf, 2, 25 * E8 as u64, "");
    let plan = tw.plan(&u, &a).unwrap();
    let before = serde_json::to_string(&tw.basis.file).unwrap();
    let ok = tw.submit(&plan, &kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap();
    assert!(ok.report.valid && ok.next.as_ref().unwrap().file.tresore.len() == 1);
    assert_eq!(serde_json::to_string(&tw.basis.file).unwrap(), before, "build und submit ändern die Datei nicht");
    for (what, signed) in [("fremder Schlüssel", kasware_sign(&plan, &key(), SIG_HASH_ALL)), ("Hashtype NONE", kasware_sign(&plan, &u, SIG_HASH_NONE))] {
        let s = tw.submit(&plan, &signed).unwrap();
        assert!(!s.report.valid && s.built.is_none() && s.next.is_none(), "{what}");
    }
    let mut v: serde_json::Value = serde_json::from_str(&serde_json::from_str::<String>(&kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap()).unwrap();
    v["inputs"][0]["signatureScript"] = serde_json::json!("");
    let s = tw.submit(&plan, &v.to_string()).unwrap();
    assert!(!s.report.valid && s.next.is_none(), "ohne Signatur");
    // veränderter Plan: anderer Empfänger, Startguthaben, Anzahl, Nachricht
    let fremd = key();
    for (what, a2) in [
        ("Empfänger", miete(&fremd, 2, 25 * E8 as u64, "")),
        ("Startguthaben", miete(&empf, 2, 26 * E8 as u64, "")),
        ("Anzahl", miete(&empf, -1, 25 * E8 as u64, "")),
        ("Nachricht", miete(&empf, 2, 25 * E8 as u64, "Miete")),
    ] {
        let mut p2 = plan.clone();
        p2.action = a2;
        let e = tw.submit(&p2, &kasware_sign(&p2, &u, SIG_HASH_ALL)).err().unwrap_or_else(|| panic!("{what}: angenommen"));
        assert!(e.contains("Plan passt nicht"), "{what}: {e}");
    }
    // Ausgang umgeleitet
    let mut p3 = plan.clone();
    p3.tx.outputs[0].script_public_key = p2pk_spk(&xonly(&fremd));
    assert!(tw.submit(&p3, &kasware_sign(&p3, &u, SIG_HASH_ALL)).err().unwrap().contains("Plan passt nicht"));
    // Tresor-Aktion nicht gegen den GHOST-Zustand und umgekehrt
    let gw = World::new();
    assert!(wo::build_plan(&gw.dep, &a, &addr(&u), NET, &tw.sim.funds(&u).utxos, P, &tw.sim.params).unwrap_err().contains("Tresor-Datei"));
    assert!(tw.plan(&u, &Action::Send { to: addr(&empf), kas: E8 as u64, payload: vec![] }).unwrap_err().contains("GHOST-Zustand"));
}

/// Nur der Besitzer: ein Fremder (auch der Empfänger) bekommt für Auffüllen
/// und Kündigen keinen Plan; seine Signatur auf dem Plan des Besitzers ist
/// ungültig, ein auf ihn umgeschriebener Plan scheitert beim Neubau
#[test]
fn fremder_kann_tresor_weder_kuendigen_noch_auffuellen() {
    let mut tw = TresorWorld::new();
    let (u, empf, fremd) = (key(), key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.sim.faucet(&fremd, 500 * E8 as u64);
    tw.sim.faucet(&empf, 10 * E8 as u64);
    tw.by_wallet("tresor-open", &u, miete(&empf, 2, 25 * E8 as u64, ""), Wallet::KasWare);
    let id = cov_of(&tw, 0);
    for k in [&fremd, &empf] {
        for a in [Action::TresorCancel { tresor: id.clone() }, Action::TresorTopup { tresor: id.clone(), kas: E8 as u64 }] {
            let e = tw.plan(k, &a).unwrap_err();
            assert!(e.contains("gehört nicht zu dieser Adresse"), "{e}");
        }
    }
    let plan = tw.plan(&u, &Action::TresorCancel { tresor: id.clone() }).unwrap();
    let s = tw.submit(&plan, &kastle_sign(&plan, &fremd)).unwrap();
    assert!(!s.report.valid && s.next.is_none());
    let mut p2 = plan.clone();
    p2.owner = xonly(&fremd);
    p2.address = addr(&fremd);
    let e = tw.submit(&p2, &kasware_sign(&p2, &fremd, SIG_HASH_ALL)).err().unwrap();
    assert!(e.contains("gehört nicht"), "{e}");
    let plan = tw.plan(&u, &Action::TresorTopup { tresor: id.clone(), kas: E8 as u64 }).unwrap();
    let s = tw.submit(&plan, &kasware_sign(&plan, &fremd, SIG_HASH_ALL)).unwrap();
    assert!(!s.report.valid && s.next.is_none());
    assert_eq!(tw.utxo_of(0), Some(25 * E8 as u64), "Tresor unberührt");
    assert!(tw.plan(&u, &Action::TresorCancel { tresor: "ab".repeat(32) }).unwrap_err().contains("nicht bekannt"));
    assert!(tw.plan(&u, &Action::TresorCancel { tresor: id[..8].into() }).unwrap_err().contains("64 Hex"));
}

/// Veralteter Plan: zahlt der Agent zwischen Bauen und Senden, passt der
/// Kündigungsplan nicht mehr (anderer Tresor-Ausgang) – neu bauen geht
#[tokio::test]
async fn tresor_veralteter_plan_nach_zahlung_abgelehnt() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.by_wallet("tresor-open", &u, miete(&empf, 3, 40 * E8 as u64, ""), Wallet::KasWare);
    let id = cov_of(&tw, 0);
    let plan = tw.plan(&u, &Action::TresorCancel { tresor: id.clone() }).unwrap();
    let signed = kasware_sign(&plan, &u, SIG_HASH_ALL);
    tw.sim.set_time(FIRST_DUE as u64 + 10 * 60_000);
    let (params, pmt) = (tw.sim.params.clone(), tw.sim.now_ms as i64 - tresor::PMT_LAG_MS);
    let mut file = tw.basis.file.clone();
    let reps = tresor::pay_round(&mut AgentIo { sim: &mut tw.sim, key: None }, &mut file, None, false, pmt, "x", &params).await.unwrap();
    assert!(reps[0].paid, "{reps:?}");
    tw.basis.file = file;
    let e = tw.submit(&plan, &signed).err().unwrap();
    assert!(e.contains("Plan passt nicht"), "{e}");
    tw.by_wallet("tresor-cancel neu", &u, Action::TresorCancel { tresor: id }, Wallet::Kastle);
}

/// A17-1 für Tresore: Gehört ein Tresor schon dem Ersatzschlüssel dieses
/// Baus, bekommt er in der Messkopie einen neutralen Besitzer und stört nicht
#[test]
fn tresor_messkopie_neutralisiert_vorbesitz_des_ersatzschluessels() {
    let mut tw = TresorWorld::new();
    let (u, empf, k) = (key(), key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.sim.faucet(&k, 500 * E8 as u64);
    tw.by_wallet("tresor-open k", &k, miete(&empf, 2, 25 * E8 as u64, ""), Wallet::KasWare);
    tw.by_wallet("tresor-open u", &u, miete(&empf, 2, 25 * E8 as u64, ""), Wallet::KasWare);
    let id = cov_of(&tw, 1);
    let m = tw.basis.mirror(&xonly(&u), &xonly(&k));
    assert_eq!(m.file.tresore[1].params.owner, xonly(&k), "Tresor des Nutzers in der Messkopie beim Ersatzschlüssel");
    assert!(m.file.tresore[0].params.owner != xonly(&k) && m.file.tresore[0].params.owner != xonly(&u), "Vorbesitz neutral");
    w::with_mirror_key(k.secret_bytes(), || {
        tw.by_wallet("tresor-topup u", &u, Action::TresorTopup { tresor: id.clone(), kas: E8 as u64 }, Wallet::KasWare);
        tw.by_wallet("tresor-cancel u", &u, Action::TresorCancel { tresor: id.clone() }, Wallet::Kastle);
    });
    assert!(tw.basis.file.tresore[1].ended.is_some() && tw.basis.file.tresore[0].ended.is_none());
}

/// Grenzen gegen eine vollgeschriebene Tresor-Datei und Eingaben: höchstens
/// MAX_WALLET_PER_OWNER laufende je Besitzer; ist die Datei voll, fallen
/// gekündigte Wallet-Tresore heraus, sonst abgelehnt; Prüfungen wie `tresor open`
#[test]
fn tresor_grenzen_und_eingaben() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.by_wallet("tresor-open", &u, miete(&empf, 2, 25 * E8 as u64, ""), Wallet::KasWare);
    let r = tw.basis.file.tresore[0].clone();
    for n in 1..tresor::MAX_WALLET_PER_OWNER {
        let mut x = r.clone();
        x.utxo.cov = kaspa_consensus_core::Hash::from_bytes([n as u8; 32]);
        tw.basis.file.tresore.push(x);
    }
    let base = || miete(&empf, 2, 25 * E8 as u64, "");
    let e = tw.plan(&u, &base()).unwrap_err();
    assert!(e.contains("laufende Tresore"), "{e}");
    tw.basis.file.tresore[3].ended = Some("x".into());
    assert!(tw.plan(&u, &base()).is_ok(), "ein gekündigter zählt nicht");
    // Datei voll
    let cov = |n: usize| {
        let mut h = [7u8; 32];
        h[..8].copy_from_slice(&(n as u64).to_le_bytes());
        kaspa_consensus_core::Hash::from_bytes(h)
    };
    let mut f = TresorFile::empty(NET);
    for n in 0..tresor::MAX_FILE_TRESORE {
        let mut x = r.clone();
        x.params.owner = xonly(&empf);
        x.utxo.cov = cov(n);
        f.tresore.push(x);
    }
    tw.basis.file = f;
    assert!(tw.plan(&u, &base()).unwrap_err().contains("kein Platz"));
    tw.basis.file.tresore[5].ended = Some("x".into());
    let plan = tw.plan(&u, &base()).unwrap();
    let s = tw.submit(&plan, &kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap();
    let next = s.next.unwrap().file;
    assert_eq!(next.tresore.len(), tresor::MAX_FILE_TRESORE, "der gekündigte ist ersetzt");
    assert!(next.tresore.iter().all(|r| r.ended.is_none()));
    tw.basis.file = TresorFile::empty(NET);
    // Eingaben wie `tresor open`; Nachricht nur so, wie sie in jede Zahlung kommt
    let bad = |a: Action| tw.plan(&u, &a).unwrap_err();
    let with = |f: &dyn Fn(&mut i64, &mut i64, &mut i64, &mut i64, &mut i64)| {
        let mut a = base();
        if let Action::TresorOpen { amount, max_fee, count, anchor_day, first_due, .. } = &mut a {
            f(amount, max_fee, count, anchor_day, first_due);
        }
        a
    };
    assert!(bad(with(&|a, _, _, _, _| *a = E8 - 1)).contains("mindestens 1 KAS"));
    assert!(bad(with(&|_, m, _, _, _| *m = tresor::MIN_MAX_FEE - 1)).contains("Höchstgebühr"));
    assert!(bad(with(&|_, m, _, _, _| *m = tresor::MAX_MAX_FEE + 1)).contains("Höchstgebühr"));
    assert!(bad(with(&|_, _, c, _, _| *c = 0)).contains("Anzahl"));
    assert!(bad(with(&|_, _, c, _, _| *c = -2)).contains("Anzahl"));
    assert!(bad(with(&|_, _, _, d, _| *d = 2)).contains("Kalendertag"));
    assert!(bad(with(&|_, _, _, d, t| {
        *d = 0;
        *t = 1_000
    }))
    .contains("Intervall"));
    assert!(bad(miete(&empf, 2, 25 * E8 as u64, " Miete")).contains("Leerzeichen"));
    assert!(bad(miete(&empf, 2, 25 * E8 as u64, "a\nb")).contains("Nachricht"));
    assert!(bad(miete(&u, 2, 25 * E8 as u64, "")).contains("Absender selbst"));
    assert!(bad(miete(&empf, 2, 10 * E8 as u64, "")).contains("Startguthaben zu klein"));
    let mut a = base();
    if let Action::TresorOpen { to, .. } = &mut a {
        *to = w::address_of_xonly(&xonly(&empf), Prefix::Testnet);
    }
    assert!(bad(a).contains("Netz"), "Adresse eines anderen Netzes");
}

// ------------------------------------------------------------- Audit 19 ----
// Tresor mit Browser-Wallet: Terminregel auch in submit (A19-1), Agent
// reihum (A19-1), Besitzer vor der Suche und begrenzte Suche (A19-2),
// Plätze der Tresor-Datei (A19-3), Kurz-ID (A19-9).

use kaspa_lending_protocol::standing::DAY_MS;

/// 1 KAS täglich, unbegrenzt, kleinste Höchstgebühr (der Fall aus dem Audit)
fn taeglich(to: &Keypair, first_due: i64, fund: u64) -> Action {
    Action::TresorOpen { to: addr(to), amount: E8, anchor_day: 0, period_ms: DAY_MS, first_due, count: -1, fund, max_fee: tresor::MIN_MAX_FEE, message: String::new() }
}

/// Agent im Simulator mit Zähler: Node-Abfragen, höchstens `sends` Sendungen
/// je Runde (danach „abgebrochen“ wie beim Zeitlimit), gespeicherte Stände;
/// Abfragen zum Skript `hang` antworten nie (langsamer Node)
struct PruefIo<'a> {
    sim: &'a mut Sim,
    lookups: usize,
    sends: usize,
    hang: Option<ScriptPublicKey>,
    saved: Vec<TresorFile>,
}

impl<'a> PruefIo<'a> {
    fn new(sim: &'a mut Sim, sends: usize) -> Self {
        PruefIo { sim, lookups: 0, sends, hang: None, saved: vec![] }
    }
}

impl tresor::TresorIo for PruefIo<'_> {
    async fn utxos(&mut self, s: &ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        if self.hang.as_ref() == Some(s) {
            std::future::pending::<()>().await;
        }
        self.lookups += 1;
        Ok(self.sim.utxos.iter().filter(|(_, e)| e.script_public_key == *s).map(|(o, e)| (*o, e.clone())).collect())
    }
    async fn funds(&mut self) -> Option<ops::Funds> {
        None
    }
    async fn send(&mut self, _: &str, b: &Built, _: &TresorFile) -> Result<(), String> {
        if self.sends == 0 {
            return Err("abgebrochen".into());
        }
        self.sends -= 1;
        self.sim.submit(b)
    }
    fn save(&mut self, f: &TresorFile) -> Result<(), String> {
        self.saved.push(f.clone());
        Ok(())
    }
    fn journal_open(&self) -> bool {
        false
    }
    fn dry_run(&self) -> bool {
        false
    }
    fn now_ms(&self) -> i64 {
        self.sim.now_ms as i64
    }
    fn say(&mut self, _: &str) {}
}

/// A19-1 (hoch), genau der Fall aus dem Audit: Ein selbst gebauter Plan mit
/// erstem Termin 2000-01-01 und täglichem Intervall, von der eigenen Wallet
/// signiert. Die Regel „nicht in der Vergangenheit“ stand nur im build-Pfad
/// von ghostctl; submit baute die Aktion aus dem Plan bitgleich nach und nahm
/// sie an (≈ 2 000 Termine Rückstand). Jetzt prüft run_tresor – also build
/// UND der Neubau in submit – mit der Past Median Time; dazu höchstens ein
/// Jahr voraus.
#[test]
fn a19_1_vergangener_termin_im_submit_abgelehnt() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    let first_due: i64 = 946_684_800_000; // 2000-01-01 00:00 UTC
    let a = taeglich(&empf, first_due, 100 * E8 as u64);
    assert!(tw.plan(&u, &a).unwrap_err().contains("Vergangenheit"), "build lehnt ab");
    // der Angreifer baut den Plan selbst (hier: build_plan mit einer Uhr im
    // Jahr 2000 – die Tx hängt nur von Aktion und Funding ab) und signiert
    let mut alt = tw.basis.clone();
    alt.pmt = first_due;
    let (plan, _) = wo::build_plan(&alt, &a, &addr(&u), NET, &tw.sim.funds(&u).utxos, P, &tw.sim.params).unwrap();
    let plan = ActionPlan::decode(&serde_json::to_string(&plan).unwrap()).unwrap();
    let signed = kasware_sign(&plan, &u, SIG_HASH_ALL);
    let damals = wo::submit(&alt, &plan, &w::parse_signed(&signed).unwrap(), NET, P, &tw.sim.params).unwrap();
    assert!(damals.report.valid, "Plan und Signatur an sich gültig");
    let p = TresorParams { owner: vec![], recipient: vec![], amount: E8, anchor_day: 0, period_ms: DAY_MS, max_fee: 0, payload_hash: vec![] };
    assert!(tresor::backlog(&p, &TresorState { next_due: first_due, left: -1 }, tw.basis.pmt) > 1_000, "das wäre der Rückstand");
    let e = tw.submit(&plan, &signed).err().expect("submit darf den Termin 2000 nicht annehmen");
    assert!(e.contains("Vergangenheit"), "{e}");
    assert!(tw.basis.file.tresore.is_empty());
    // Grenzen: ein Tag zurück (heute 00:00 UTC aus jeder Zeitzone) geht, ein Jahr voraus auch
    let now = tw.basis.pmt;
    assert!(tw.plan(&u, &taeglich(&empf, now - DAY_MS, 100 * E8 as u64)).is_ok());
    assert!(tw.plan(&u, &taeglich(&empf, now - DAY_MS - 1, 100 * E8 as u64)).unwrap_err().contains("Vergangenheit"));
    assert!(tw.plan(&u, &taeglich(&empf, now + tresor::MAX_FIRST_DUE_AHEAD_MS, 100 * E8 as u64)).is_ok());
    assert!(tw.plan(&u, &taeglich(&empf, now + tresor::MAX_FIRST_DUE_AHEAD_MS + 1, 100 * E8 as u64)).unwrap_err().contains("ein Jahr"));
    // auch die Obergrenze gilt im Neubau von submit (Termin 2199 aus A19-3)
    let weit: i64 = 7_226_582_400_000; // 2199-01-01
    let mut zukunft = tw.basis.clone();
    zukunft.pmt = weit - DAY_MS;
    let b = taeglich(&empf, weit, 100 * E8 as u64);
    let (plan, _) = wo::build_plan(&zukunft, &b, &addr(&u), NET, &tw.sim.funds(&u).utxos, P, &tw.sim.params).unwrap();
    let plan = ActionPlan::decode(&serde_json::to_string(&plan).unwrap()).unwrap();
    let e = tw.submit(&plan, &kasware_sign(&plan, &u, SIG_HASH_ALL)).err().expect("Termin 2199 abgelehnt");
    assert!(e.contains("ein Jahr"), "{e}");
}

/// A19-1, Agent: fällige Tresore reihum. Ein Tresor mit Rückstand (Agent
/// war aus, oder ein alter Tresor) kam vorher in jeder Runde zuerst dran und
/// verbrauchte die Sendezeit; ein später angelegter kam nie an die Reihe.
/// Hier sendet jede Runde nur einmal (wie beim Zeitlimit des Agenten).
#[tokio::test]
async fn a19_1_agent_bedient_faellige_tresore_reihum() {
    let mut tw = TresorWorld::new();
    let (u, v, empf) = (key(), key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.sim.faucet(&v, 500 * E8 as u64);
    let start = tw.basis.pmt;
    tw.by_wallet("A (alt)", &u, taeglich(&empf, start + 3_600_000, 100 * E8 as u64), Wallet::KasWare);
    tw.sim.set_time((start + 30 * DAY_MS) as u64);
    tw.basis.pmt = tw.sim.now_ms as i64;
    tw.by_wallet("B (neu)", &v, taeglich(&empf, tw.basis.pmt - 3_600_000, 100 * E8 as u64), Wallet::Kastle);
    // beide im Rückstand: A gut 30 Termine, B 5
    tw.sim.set_time((start + 35 * DAY_MS) as u64);
    let (ida, idb) = (tw.basis.file.tresore[0].id.clone(), tw.basis.file.tresore[1].id.clone());
    let params = tw.sim.params.clone();
    let mut file = tw.basis.file.clone();
    let mut paid = vec![];
    for _ in 0..4 {
        let pmt = tw.sim.now_ms as i64 - tresor::PMT_LAG_MS;
        let mut io = PruefIo::new(&mut tw.sim, 1);
        let reps = tresor::pay_round(&mut io, &mut file, None, true, pmt, "x", &params).await.unwrap();
        paid.extend(reps.into_iter().filter(|r| r.paid).map(|r| r.id));
    }
    let n = |id: &str| paid.iter().filter(|p| *p == id).count();
    println!("A19-1 reihum: A (Rückstand {}) {}×, B {}×", tresor::backlog(&file.tresore[0].params, &file.tresore[0].utxo.state, tw.sim.now_ms as i64), n(&ida), n(&idb));
    assert_eq!(paid, vec![ida.clone(), idb.clone(), ida.clone(), idb.clone()], "abwechselnd, nicht immer der erste der Datei");
    assert!(file.tresore.iter().all(|r| r.last_paid_ms.is_some()));
}

/// A19-2: Auffüllen/Kündigen über die Seite prüft den Besitzer, BEVOR am
/// Node gesucht wird, und sucht höchstens PUBLIC_FOLLOW Zustände. Vorher:
/// bis zu 2 000 Abfragen je Anfrage für jede bekannte Tresor-ID.
#[tokio::test]
async fn a19_2_besitzer_vor_der_suche_und_begrenzt() {
    let mut tw = TresorWorld::new();
    let (u, empf, fremd) = (key(), key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    let pmt0 = tw.basis.pmt;
    tw.by_wallet("tresor-open", &u, taeglich(&empf, pmt0 + 3_600_000, 10 * E8 as u64), Wallet::KasWare);
    let id = cov_of(&tw, 0);
    let cancel = Action::TresorCancel { tresor: id.clone() };
    let mut file = tw.basis.file.clone();
    let mut io = PruefIo::new(&mut tw.sim, 0);
    // fremd, unbekannt, Anlegen: keine einzige Abfrage
    let e = wo::follow_for_wallet(&mut io, &mut file, &cancel, &xonly(&fremd), pmt0, "x").await.unwrap_err();
    assert!(e.contains("gehört nicht"), "{e}");
    let e = wo::follow_for_wallet(&mut io, &mut file, &Action::TresorCancel { tresor: "ab".repeat(32) }, &xonly(&u), pmt0, "x").await.unwrap_err();
    assert!(e.contains("nicht bekannt"), "{e}");
    assert!(wo::follow_for_wallet(&mut io, &mut file, &taeglich(&empf, pmt0, 10 * E8 as u64), &xonly(&u), pmt0, "x").await.unwrap().is_none());
    assert_eq!(io.lookups, 0, "vor der Besitzerprüfung keine Node-Abfrage");
    // eigener, laufender Tresor: eine Abfrage
    assert!(wo::follow_for_wallet(&mut io, &mut file, &cancel, &xonly(&u), pmt0, "x").await.unwrap().is_none());
    assert_eq!(io.lookups, 1);
    // am Server vorbei gekündigt, danach 2 500 Tage vergangen (Rückstand > MAX_FOLLOW)
    let r = file.tresore[0].clone();
    let b = tresor::cancel(&r.params, &r.utxo, &u, &io.sim.params).unwrap();
    io.sim.submit(&b).unwrap();
    io.sim.set_time((pmt0 + 2_500 * DAY_MS) as u64);
    let pmt = io.sim.now_ms as i64;
    io.lookups = 0;
    let mut voll = file.clone();
    tresor::follow(&mut io, &mut voll.tresore[0], pmt, "x", tresor::Search::Full).await.unwrap();
    let full = io.lookups;
    io.lookups = 0;
    let note = wo::follow_for_wallet(&mut io, &mut file, &cancel, &xonly(&u), pmt, "x").await.unwrap();
    println!("A19-2 Abfragen je öffentlicher Anfrage: {} statt {full}", io.lookups);
    assert_eq!((full, io.lookups), (tresor::MAX_FOLLOW, tresor::PUBLIC_FOLLOW));
    assert!(note.unwrap().contains("nicht auffindbar") && file.tresore[0].missing.is_some());
    // als fehlend markiert: gar keine Suche mehr
    io.lookups = 0;
    assert!(wo::follow_for_wallet(&mut io, &mut file, &cancel, &xonly(&u), pmt, "x").await.unwrap_err().contains("nicht auffindbar"));
    assert_eq!(io.lookups, 0);
}

/// A19-2, Agent: Was die erste (teure) Suche nach einem verschwundenen
/// Tresor ergibt, wird sofort gesichert. Vorher speicherte pay_round erst vor
/// einer Sendung oder am Ende; brach das Zeitlimit die Runde vorher ab (hier:
/// ein Node, der beim nächsten Tresor nicht antwortet), suchte die nächste
/// Runde wieder alles ab.
#[tokio::test]
async fn a19_2_fehlender_tresor_bleibt_nach_abbruch_markiert() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    let pmt0 = tw.basis.pmt;
    tw.by_wallet("A", &u, taeglich(&empf, pmt0 + 3_600_000, 10 * E8 as u64), Wallet::KasWare);
    // anderer Termin, also anderes Skript als A
    tw.by_wallet("B", &u, taeglich(&empf, pmt0 + 7_200_000, 10 * E8 as u64), Wallet::KasWare);
    let mut file = tw.basis.file.clone();
    let r = file.tresore[0].clone();
    let b = tresor::cancel(&r.params, &r.utxo, &u, &tw.sim.params).unwrap();
    tw.sim.submit(&b).unwrap();
    tw.sim.set_time((pmt0 + 3 * DAY_MS) as u64);
    let (params, pmt) = (tw.sim.params.clone(), tw.sim.now_ms as i64 - tresor::PMT_LAG_MS);
    let hang = tresor::TresorShape::of(&file.tresore[1].params).spk(&file.tresore[1].utxo.state);
    let mut io = PruefIo::new(&mut tw.sim, 1);
    io.hang = Some(hang);
    let round = tresor::pay_round(&mut io, &mut file, None, true, pmt, "x", &params);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(300), round).await.is_err(), "Runde hängt am zweiten Tresor");
    let saved = io.saved.last().expect("vor dem Abbruch gesichert");
    assert!(saved.tresore[0].missing.is_some() && saved.tresore[0].retry_after.is_some(), "Markierung „missing“ gesichert");
    assert!(saved.tresore[1].missing.is_none());
}

/// A19-3: Plätze der Tresor-Datei. Vorher belegten 1 000 nie fällige
/// Wallet-Tresore (Termin 2199, je ≈ 2 KAS) die Datei dauerhaft. Jetzt zählen
/// nur Tresore, die bald etwas kosten (laufend, zahlbar, binnen 32 Tagen
/// fällig); ruhende zählen nur gegen die viel größere Gesamtgrenze, und aus
/// der vollen Datei fallen gekündigte und seit über einer Woche fehlende
/// Wallet-Tresore – laufende mit Guthaben nie.
#[test]
fn a19_3_ruhende_tresore_belegen_keine_plaetze() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.by_wallet("tresor-open", &u, miete(&empf, 2, 25 * E8 as u64, ""), Wallet::KasWare);
    let r = tw.basis.file.tresore[0].clone();
    let pmt = tw.basis.pmt;
    assert!(r.busy(pmt), "Miete im nächsten Monat belegt einen Platz");
    let fill = |n: usize, f: &dyn Fn(&mut tresor::TresorRec)| {
        let mut file = TresorFile::empty(NET);
        for i in 0..n {
            let mut x = r.clone();
            x.params.owner = xonly(&empf);
            let mut h = [7u8; 32];
            h[..8].copy_from_slice(&(i as u64).to_le_bytes());
            x.utxo.cov = kaspa_consensus_core::Hash::from_bytes(h);
            f(&mut x);
            file.tresore.push(x);
        }
        file
    };
    let base = || miete(&empf, 2, 25 * E8 as u64, "");
    let full = |tw: &TresorWorld| tw.plan(&u, &base()).err().is_some_and(|e| e.contains("kein Platz"));
    tw.basis.file = fill(tresor::MAX_FILE_TRESORE, &|_| {});
    assert!(full(&tw), "1 000 bald fällige: voll");
    tw.basis.file = fill(tresor::MAX_FILE_TRESORE - 1, &|_| {});
    assert!(!full(&tw));
    let ruhend: [(&str, &dyn Fn(&mut tresor::TresorRec)); 4] = [
        ("Termin in 200 Tagen", &|x| x.utxo.state.next_due = pmt + 200 * DAY_MS),
        ("leer", &|x| x.utxo.value = (x.params.amount + x.params.max_fee) as u64),
        ("erledigt", &|x| x.utxo.state.left = 0),
        ("gekündigt", &|x| x.ended = Some("x".into())),
    ];
    for (what, f) in ruhend {
        tw.basis.file = fill(tresor::MAX_FILE_TRESORE, f);
        assert!(!full(&tw), "{what}: ruht, belegt keinen Platz");
    }
    // Gesamtgrenze: ruhende bis MAX_FILE_ALL
    let far = |x: &mut tresor::TresorRec| x.utxo.state.next_due = pmt + 200 * DAY_MS;
    tw.basis.file = fill(tresor::MAX_FILE_ALL, &far);
    assert!(full(&tw), "Gesamtgrenze");
    // ein gekündigter Wallet-Tresor fällt heraus
    tw.basis.file.tresore[7].ended = Some("x".into());
    let plan = tw.plan(&u, &base()).unwrap();
    let next = tw.submit(&plan, &kasware_sign(&plan, &u, SIG_HASH_ALL)).unwrap().next.unwrap().file;
    assert_eq!(next.tresore.len(), tresor::MAX_FILE_ALL);
    assert!(next.tresore.iter().all(|r| r.ended.is_none()), "der gekündigte ist ersetzt");
    // fehlend: erst nach der Woche erneuten Nachsehens
    tw.basis.file = fill(tresor::MAX_FILE_ALL, &far);
    tw.basis.file.tresore[9].missing = Some("x".into());
    tw.basis.file.tresore[9].missing_ms = Some(pmt - tresor::MISSING_RECHECK_FOR_MS + 60_000);
    assert!(full(&tw), "frisch fehlend bleibt");
    tw.basis.file.tresore[9].missing_ms = Some(pmt - tresor::MISSING_RECHECK_FOR_MS - 60_000);
    assert!(!full(&tw), "nach einer Woche fehlend fällt heraus");
    // Tresore des Betreibers (Schlüsseldatei) fallen nie heraus
    tw.basis.file = fill(tresor::MAX_FILE_ALL, &|x| {
        far(x);
        x.wallet = false;
        x.ended = Some("x".into());
    });
    assert!(full(&tw));
    assert_eq!(tresor::MAX_WALLET_PER_OWNER, 10);
}

/// A19-9: Bei mehrdeutiger Kurz-ID nennt `find` die vollen Covenant-IDs
#[test]
fn a19_9_mehrdeutige_kurz_id_nennt_die_vollen() {
    let mut tw = TresorWorld::new();
    let (u, empf) = (key(), key());
    tw.sim.faucet(&u, 500 * E8 as u64);
    tw.by_wallet("tresor-open", &u, miete(&empf, 2, 25 * E8 as u64, ""), Wallet::KasWare);
    let mut f = tw.basis.file.clone();
    let mut x = f.tresore[0].clone();
    let mut h = x.utxo.cov.as_bytes();
    h[31] ^= 1;
    x.utxo.cov = kaspa_consensus_core::Hash::from_bytes(h);
    f.tresore.push(x.clone());
    let e = f.find(&x.id).unwrap_err();
    assert!(e.contains("mehrdeutig") && e.contains(&f.tresore[0].utxo.cov.to_string()) && e.contains(&x.utxo.cov.to_string()), "{e}");
    assert_eq!(f.find(&x.utxo.cov.to_string()).unwrap(), 1, "volle ID eindeutig");
}
