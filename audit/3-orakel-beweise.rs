//! Audit-Beweise Orakel (nur in der Scratch-Kopie). Ausgabe:
//! cargo test --test audit_oracle -- --nocapture --test-threads=1

use kaspa_consensus_core::tx::{CovenantBinding, TransactionOutput, UtxoEntry};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::{self, E8};
use kaspa_lending_protocol::ops::{self, Deployment, xonly};
use kaspa_lending_protocol::sim::Sim;
use kaspa_lending_protocol::txb::{Built, Draft, In, Unlock, build, check_scripts};
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
        "  {label:<26} Inputs {:>2} | compute {:>6} g | transient {:>6} g | storage {:>6} g | Gebühr {:>7} sompi ({:.5} KAS) | ~{} B",
        b.tx.inputs.len(),
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

/// Wie e2e_tests::setup(), mit den Mainnet-Parametern aus deployments/mainnet.json
/// (threshold 3, max_rate 1e9, rate 5 % p. a., Index 1e9).
fn setup(with_factory: bool) -> World {
    let mut sim = Sim::new();
    let deployer = key();
    let committee: Vec<Keypair> = (0..5).map(|_| key()).collect();
    sim.faucet(&deployer, 1_000 * E8 as u64);
    let net = sim.params.clone();
    let op = OracleParams { signers: committee.iter().map(xonly).collect(), threshold: 3, max_rate: 1_000_000_000 };
    let os = OracleState { kas_usd: 4_774_050, oracle_daa: sim.daa as i64, seq: 0, stable_rate: 158_548_960, stable_index: 1_000_000_000 };
    let (b, oracle_t) = ops::deploy_oracle(&op, os, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
    let (b, factory_t) = ops::deploy_factory(&fp, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let (factory, root, vp) = if with_factory {
        let (b, f2, root, vp) =
            ops::init_factory(&op, &oracle_t, &fp, &factory_t, &deployer, (20_000, 15_000, 1_000), &sim.funds(&deployer), &net).unwrap();
        sim.submit(&b).unwrap();
        (f2, Some(root), Some(vp))
    } else {
        (factory_t, None, None)
    };
    let dep = Deployment {
        network: "sim".into(),
        oracle_params: op,
        oracle: oracle_t,
        factory_params: fp,
        factory,
        ghost_root: root,
        vault_params: vp,
        vaults: vec![],
        tokens: vec![],
    };
    World { sim, deployer, committee, dep }
}

impl World {
    fn signers(&self) -> Vec<(usize, Keypair)> {
        [0usize, 1, 2].iter().map(|&i| (i, self.committee[i])).collect()
    }
    /// Update wie ghostctl::oracle_update (DAA = aktuell, Signierer 0,1,2), nicht gesendet.
    fn update_tx(&self, dep: &Deployment, kas_usd: i64, rate: i64, daa: u64) -> Result<(Built, Deployment), String> {
        ops::oracle_update(dep, &self.signers(), kas_usd, rate, daa, &self.sim.funds(&self.deployer), &self.sim.params.clone())
    }
}

/// read()-Transaktion eines beliebigen Dritten (baut nach, was ops::oracle_read_input privat tut).
fn stranger_read(dep: &Deployment, sim: &Sim, stranger: &Keypair) -> Result<Built, String> {
    let o = &dep.oracle;
    let art = oracle(&dep.oracle_params, &o.state);
    let mut inputs = vec![In {
        outpoint: o.outpoint,
        entry: UtxoEntry::new(o.value, spk(&art), 0, false, Some(o.cov)),
        unlock: Unlock::Entry { art: art.clone(), entry: "read", args: vec![], sig_at: None },
    }];
    let funds = sim.funds(stranger);
    inputs.extend(funds.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: *stranger } }));
    let out = TransactionOutput {
        value: o.value,
        script_public_key: spk(&art),
        covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: o.cov }),
    };
    build(Draft { inputs, outputs: vec![out], change_spk: ops::p2pk_spk(&xonly(stranger)), lock_time: 0 }, &sim.params.clone())
}

// ---------------------------------------------------------------------------
// Beweis A: Ein Dritter gibt das Orakel per read() aus. Danach scheitert das
// Komitee-Update, das auf dem gespeicherten Outpoint aufbaut (= Zustandsdatei
// von ghostctl). Kosten des Angriffs = Gebühr der read()-Tx.
#[test]
fn a_fremdes_read_macht_zustandsdatei_und_update_unbrauchbar() {
    let mut w = setup(false);
    let stranger = key();
    w.sim.faucet(&stranger, 1 * E8 as u64);

    let b = stranger_read(&w.dep, &w.sim, &stranger).expect("Dritter kann read() bauen");
    report("read() durch Dritten", &b);
    w.sim.submit(&b).expect("read() durch Dritten wird angenommen");
    println!("  Dritter zahlte {} sompi Gebühr; Orakel-Outpoint ist jetzt {}:{}", b.fee, b.tx.id(), 0);

    // Komitee (ghostctl) baut auf der alten Zustandsdatei: Bau gelingt, Netz lehnt ab.
    let daa = w.sim.daa;
    let (ub, _) = w.update_tx(&w.dep, 4_800_000, w.dep.oracle.state.stable_rate, daa).expect("Update baut lokal (kennt Kette nicht)");
    let err = w.sim.submit(&ub).expect_err("Update auf altem Outpoint muss scheitern");
    println!("  Komitee-Update auf altem Outpoint: {err}");
    assert!(err.contains("nicht vorhanden"));
}

// ---------------------------------------------------------------------------
// Beweis B: Locktime-Domänenwechsel. Ein Komitee kann newOracleDaa über die
// LOCK_TIME_THRESHOLD (5e11) heben; die Skript-Engine akzeptiert das, wenn
// tx.lock_time ebenfalls ein Zeitstempel (ms) ist. Danach zählt Δ in
// Millisekunden statt DAA (100× schnellere Verzinsung bei gleichem maxRate).
#[test]
fn b_locktime_domaenenwechsel_wird_von_der_engine_akzeptiert() {
    let mut w = setup(false);
    let daa = w.sim.daa;
    // 1) Satz auf 0, sonst überläuft rate·Δ beim Sprung (das ist die einzige Hürde).
    let (b, d1) = w.update_tx(&w.dep, 4_774_050, 0, daa).unwrap();
    w.sim.submit(&b).unwrap();
    // 2) Sprung in die Zeitstempel-Domäne: 1,7e12 ms ≈ 2023, also längst „vergangen“.
    let ts = 1_700_000_000_000u64;
    // ERGEBNIS: Der SilverScript-Compiler senkt `require(tx.daa >= x)` als
    // `x OpWithin(0, LOCK_TIME_THRESHOLD) OpVerify OpCheckLockTimeVerify` ab
    // (compile/statement.rs:381-391). Ein Wert >= 5e11 scheitert daher schon
    // vor dem CLTV mit VerifyError. Der Domänenwechsel ist NICHT möglich.
    let r = w.update_tx(&d1, 4_774_050, 1_000_000_000, ts);
    println!("  Update mit oracleDaa = {ts} (>= LOCK_TIME_THRESHOLD): {}", r.as_ref().err().unwrap_or(&"BAUT".into()));
    assert!(r.is_err(), "Compiler-Schutz (OpWithin) muss greifen");
    // Kontrolle: knapp unterhalb der Schwelle geht es durch (Engine), Sim lehnt nur wegen DAA-Vergleich ab
    let below = kaspa_txscript::LOCK_TIME_THRESHOLD - 1;
    let (b2, _) = w.update_tx(&d1, 4_774_050, 1_000_000_000, below).expect("baut");
    check_scripts(&b2.tx, &b2.entries).expect("knapp unter der Schwelle akzeptiert die Engine");
    println!("  Kontrolle: oracleDaa = {below} (DAA-Domäne, weit in der Zukunft) wird von der Engine akzeptiert; die Tx wäre erst bei diesem DAA-Score final");
}

// ---------------------------------------------------------------------------
// Beweis C: Der Vertrag kennt keine Preisgrenze. 1 sompi-USD und 1e18 gehen
// durch. Bei 1e18 (10^10 USD/KAS) bricht jede Vault-Rechnung mit NumberTooBig
// ab: Vault mit Schuld kann weder tilgen noch liquidiert noch abheben.
#[test]
fn c_preis_ohne_grenze_kann_vaults_einfrieren() {
    let mut w = setup(true);
    let net = w.sim.params.clone();
    let user = key();
    w.sim.faucet(&user, 300 * E8 as u64);
    let (b, d) = ops::open_vault(&w.dep, &xonly(&user), 150 * E8 as u64, &w.sim.funds(&user), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.dep = d;
    let (b, d) = ops::mint(&w.dep, 0, &user, 1 * E8, &xonly(&user), &w.sim.funds(&user), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.dep = d;
    // zweiter Vault (100 KAS, 1 GHOST), damit genug GHOST für Schuld + Zins da ist
    let (b, d) = ops::open_vault(&w.dep, &xonly(&user), 100 * E8 as u64, &w.sim.funds(&user), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.dep = d;
    let (b, d) = ops::mint(&w.dep, 1, &user, 1 * E8, &xonly(&user), &w.sim.funds(&user), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.dep = d;

    // Preis 1 (= 1e-8 USD/KAS): akzeptiert
    let daa = w.sim.daa;
    let (b, d) = w.update_tx(&w.dep, 1, w.dep.oracle.state.stable_rate, daa).unwrap();
    w.sim.submit(&b).expect("Preis 1 sompi-USD wird angenommen");
    w.dep = d;
    println!("  Orakel akzeptiert kasUsd = 1");
    // Vault 0 ist jetzt „liquidierbar“ zu einem Fantasiepreis: ~1 GHOST Schuld → ganzer Vault (150 KAS)
    let liq = key();
    w.sim.faucet(&liq, 10 * E8 as u64);
    let toks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&user)).map(|(i, _)| i).collect();
    let (b, d) = ops::transfer(&w.dep, &user, &toks, &xonly(&liq), 2 * E8, &w.sim.funds(&user), &net).unwrap();
    w.sim.submit(&b).unwrap();
    w.dep = d;
    let ltoks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&liq)).map(|(i, _)| i).collect();
    let before = w.sim.balance(&liq);
    let (b, d) = ops::liquidate(&w.dep, 0, &liq, &ltoks, &w.sim.funds(&liq), &net).expect("Liquidation baut");
    w.sim.submit(&b).expect("Liquidation zum Fantasiepreis wird angenommen");
    w.dep = d;
    println!(
        "  Liquidation bei kasUsd=1: Liquidator +{:.2} KAS für ~1 GHOST (Sicherheit 150 KAS ≈ 7,16 USD zum echten Kurs)",
        (w.sim.balance(&liq) as i64 - before as i64) as f64 / 1e8
    );

    // Preis 1e18: Vertrag akzeptiert, Vault 1 (100 KAS, 1 GHOST Schuld) friert ein
    let daa = w.sim.daa;
    let (b, d) = w.update_tx(&w.dep, 1_000_000_000_000_000_000, w.dep.oracle.state.stable_rate, daa).unwrap();
    w.sim.submit(&b).expect("Preis 1e18 wird angenommen");
    w.dep = d;
    println!("  Orakel akzeptiert kasUsd = 1e18 (Vault-Grenze laut ARCHITEKTUR.md: 920 USD/KAS = 9,2e10)");
    let vi = w.dep.vaults.iter().position(|v| v.vault.value == 100 * E8 as u64).expect("Vault 1");
    let ltoks: Vec<usize> = w.dep.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&liq)).map(|(i, _)| i).collect();
    // repay/close brauchen den Preis nicht (readOracle prüft nur kasUsd > 0): Ausstieg bleibt möglich
    let r = ops::repay(&w.dep, vi, &liq, &ltoks, E8 / 2, &w.sim.funds(&liq), &net);
    println!("  repay bei kasUsd=1e18: {}", r.as_ref().err().unwrap_or(&"BAUT (Engine akzeptiert)".into()));
    let (b, d) = r.expect("repay geht trotz Fantasiepreis");
    w.sim.submit(&b).expect("repay angenommen");
    w.dep = d;
    let r = ops::withdraw(&w.dep, vi, &user, 90 * E8 as u64, &ops::p2pk_spk(&xonly(&user)), &w.sim.funds(&user), &net);
    println!("  withdraw bei kasUsd=1e18: {}", r.as_ref().err().unwrap_or(&"BAUT".into()));
    assert!(r.is_err(), "withdraw muss scheitern");
    let r = ops::liquidate(&w.dep, vi, &liq, &ltoks, &w.sim.funds(&liq), &net);
    println!("  liquidate bei kasUsd=1e18: {}", r.as_ref().err().unwrap_or(&"BAUT".into()));
    assert!(r.is_err(), "liquidate muss scheitern");
}

// ---------------------------------------------------------------------------
// Beweis D: Zinsrechnung mit den Mainnet-Konstanten (stable_rate 158 548 960).
#[test]
fn d_zinsrechnung_mainnet_konstanten() {
    let start = OracleState { kas_usd: 4_774_050, oracle_daa: 551_728_492, seq: 0, stable_rate: 158_548_960, stable_index: 1_000_000_000 };
    let year = 315_360_000; // DAA je Jahr bei 10/s
    let one = oracle_next_index(&start, start.oracle_daa + year).unwrap();
    println!("  1 Jahr in einem Update: Index {one} (= {:.4} % p. a.)", (one - start.stable_index) as f64 / 1e7);
    // 5-Minuten-Takt (3000 DAA), ein Jahr lang
    let mut s = start;
    for _ in 0..(year / 3000) {
        s.stable_index = oracle_next_index(&s, s.oracle_daa + 3000).unwrap();
        s.oracle_daa += 3000;
    }
    println!("  1 Jahr im 5-min-Takt: Index {} (= {:.4} % p. a., Rundungsverlust {} ppb)", s.stable_index, (s.stable_index - start.stable_index) as f64 / 1e7, one - s.stable_index);
    // Untergrenze: erst ab Δ ≥ 7 DAA gibt es überhaupt Zuwachs
    assert_eq!(oracle_next_index(&start, start.oracle_daa + 6).unwrap(), start.stable_index);
    // Δ = 1 DAA-Updates würden gar nicht verzinsen (Rundung auf 0), aber nur das Komitee kann updaten.
}

// ---------------------------------------------------------------------------
// Beweis E: Ein signiertes Update ist ein Inhaberpapier bis seq weitergezählt
// ist. Wer das Sigscript (z. B. aus dem Mempool) sieht, kann es mit eigenen
// Gebühren-UTXOs einreichen. Signiert das Komitee für dieselbe seq zweimal
// (Wiederholung nach Zeitüberschreitung), entscheidet der Dritte, welcher
// Preis landet.
#[test]
fn e_signiertes_update_ist_inhaberpapier_bis_seq_steigt() {
    let mut w = setup(false);
    let daa = w.sim.daa;
    // Komitee signiert Preis P1 (Tx bleibt z. B. im Mempool hängen, ghostctl gibt nach 120/180 s auf)
    let (tx1, _) = w.update_tx(&w.dep, 4_000_000, w.dep.oracle.state.stable_rate, daa).unwrap();
    // Komitee signiert später erneut, gleiche seq, Preis P2 (Kurs ist gefallen)
    let (tx2, d2) = w.update_tx(&w.dep, 3_000_000, w.dep.oracle.state.stable_rate, daa + 5).unwrap();

    // Dritter nimmt das Sigscript von tx1 (ohne Tx-Signatur, nur Datasigs) und baut eigene Tx
    let stranger = key();
    w.sim.faucet(&stranger, 1 * E8 as u64);
    let o = &w.dep.oracle;
    let art = oracle(&w.dep.oracle_params, &o.state);
    let mut inputs = vec![In {
        outpoint: o.outpoint,
        entry: UtxoEntry::new(o.value, spk(&art), 0, false, Some(o.cov)),
        unlock: Unlock::Raw(tx1.tx.inputs[0].signature_script.clone()),
    }];
    let funds = w.sim.funds(&stranger);
    inputs.extend(funds.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: stranger } }));
    let out = tx1.tx.outputs[0].clone();
    let replay = build(Draft { inputs, outputs: vec![out], change_spk: ops::p2pk_spk(&xonly(&stranger)), lock_time: tx1.tx.lock_time }, &w.sim.params.clone())
        .expect("Dritter kann das Komitee-Sigscript wiederverwenden");
    w.sim.submit(&replay).expect("Replay durch Dritten wird angenommen");
    println!("  Dritter hat P1 = 0,04 USD installiert (Tx {})", replay.tx.id());
    // Das eigentlich gewollte P2 scheitert jetzt (seq schon verbraucht)
    let err = w.sim.submit(&tx2).expect_err("P2 muss scheitern");
    println!("  Komitee-Update P2 = 0,03 USD danach: {err}");
    let _ = d2;
    let _ = math::E8;
}
