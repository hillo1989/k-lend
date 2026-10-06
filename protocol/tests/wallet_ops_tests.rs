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
    for want in ["Dein Vault 0 – Sicherheit 200 KAS", "Orakel (läuft weiter)", "Minter-Zweig von Vault 0 (läuft weiter)"] {
        assert!(names.contains(&want.to_string()), "{want}: {names:?}");
    }
    assert!(names.iter().any(|n| n.starts_with("GHOST-Token")), "{names:?}");
}
