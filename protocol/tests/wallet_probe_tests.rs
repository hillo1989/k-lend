//! Browser-Wallet-Probe (src/wallet.rs): export → fremd signieren wie Kastle
//! bzw. KasWare → attach → Simulator. Die Wallets werden mit einem
//! Testschlüssel nachgebildet:
//! - Kastle `signTx(networkId, txJson, scripts)`: für jeden Eintrag in
//!   `scripts` `createInputSignature(tx, idx, key, All)` (= rusty-kaspa
//!   consensus/core/src/sign.rs `sign_input`, `0x41 ‖ sig64 ‖ hashtype`) und
//!   dahinter der Push von `scriptHex`; danach eigene P2PK-Eingänge mit
//!   `signTransaction` (forbole/kastle lib/wallet/sign-script.ts).
//! - KasWare `signPskt({txJsonString, options:{signInputs}})`: je genanntem
//!   Index `0x41 ‖ sig64 ‖ hashtype` (Annahme wie upstream, der Fork ist nicht
//!   öffentlich).
//! Beide lesen und schreiben das Safe JSON von kaspa-wasm.

use kaspa_addresses::Prefix;
use kaspa_consensus_core::config::params::MAINNET_PARAMS;
use kaspa_consensus_core::hashing::sighash_type::{SIG_HASH_ALL, SigHashType};
use kaspa_consensus_core::sign::sign_input;
use kaspa_consensus_core::tx::{PopulatedTransaction, TransactionOutpoint, UtxoEntry};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::ops::{Tracked, p2pk_spk, xonly};
use kaspa_lending_protocol::sim::Sim;
use kaspa_lending_protocol::tresor;
use kaspa_lending_protocol::wallet::{self as w, Plan, SafeTx};
use kaspa_txscript::script_builder::ScriptBuilder;
use secp256k1::{Keypair, Secp256k1, SecretKey};

const E8: u64 = 100_000_000;

fn key(n: u8) -> Keypair {
    Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
}

fn parse_req_tx(json: &str) -> (kaspa_consensus_core::tx::Transaction, Vec<UtxoEntry>) {
    let s: SafeTx = serde_json::from_str(json).unwrap();
    w::from_safe(&s).unwrap()
}

/// Antwort wie eine Wallet: JSON-Text (String) mit dem Safe JSON
fn reply(tx: &kaspa_consensus_core::tx::Transaction, e: &[UtxoEntry]) -> String {
    serde_json::to_string(&serde_json::to_string(&w::to_safe(tx, e, Prefix::Mainnet)).unwrap()).unwrap()
}

/// Kastle nachgebildet (signTx mit scripts)
fn kastle_sign(plan: &Plan, k: &Keypair, ty: SigHashType) -> String {
    let req = plan.kastle();
    assert_eq!(req["networkId"], "mainnet");
    let (mut tx, e) = parse_req_tx(req["txJson"].as_str().unwrap());
    let sk = k.secret_bytes();
    for s in req["scripts"].as_array().unwrap() {
        assert_eq!(s["signType"], "All");
        let idx = s["inputIndex"].as_u64().unwrap() as usize;
        assert!(tx.inputs[idx].signature_script.is_empty(), "already carries a signatureScript");
        let sig = sign_input(&PopulatedTransaction::new(&tx, e.clone()), idx, &sk, ty);
        let redeem = w::strict_hex::decode(s["scriptHex"].as_str().unwrap()).unwrap();
        let mut script = sig;
        // Push wie Kastles pushDataHex (kanonisch, auch über 520 Byte für v1-Covenants)
        script.extend(ScriptBuilder::with_flags(kaspa_lending_protocol::txb::engine_flags()).add_data(&redeem).unwrap().drain());
        tx.inputs[idx].signature_script = script;
    }
    let own = p2pk_spk(&xonly(k));
    for i in 0..tx.inputs.len() {
        if tx.inputs[i].signature_script.is_empty() && e[i].script_public_key == own {
            let sig = sign_input(&PopulatedTransaction::new(&tx, e.clone()), i, &sk, SIG_HASH_ALL);
            tx.inputs[i].signature_script = sig;
        }
    }
    reply(&tx, &e)
}

/// KasWare nachgebildet (signPskt mit signInputs)
fn kasware_sign(plan: &Plan, k: &Keypair) -> String {
    let req = plan.kasware();
    let (mut tx, e) = parse_req_tx(req["txJsonString"].as_str().unwrap());
    let sk = k.secret_bytes();
    for s in req["options"]["signInputs"].as_array().unwrap() {
        assert_eq!(s["sighashType"], 1);
        let idx = s["index"].as_u64().unwrap() as usize;
        let sig = sign_input(&PopulatedTransaction::new(&tx, e.clone()), idx, &sk, SIG_HASH_ALL);
        tx.inputs[idx].signature_script = sig;
    }
    reply(&tx, &e)
}

fn attach(plan: &Plan, signed: &str) -> (Option<kaspa_lending_protocol::txb::Built>, w::Report) {
    // Plan geht als JSON durch Browser und Datei
    let plan = Plan::decode(&serde_json::to_string(plan).unwrap()).unwrap();
    w::attach(&plan, &w::parse_signed(signed).unwrap(), &MAINNET_PARAMS).unwrap()
}

const NOW: i64 = 1_798_761_600_000; // sim::START_MS

#[test]
fn safe_json_form_wie_kaspa_wasm() {
    let k = key(7);
    let plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    let v: serde_json::Value = serde_json::from_str(plan.kastle()["txJson"].as_str().unwrap()).unwrap();
    // Feldnamen und Zahlen als Text wie consensus/client/src/serializable/string.rs
    for f in ["id", "version", "inputs", "outputs", "subnetworkId", "lockTime", "gas", "storageMass", "payload"] {
        assert!(v.get(f).is_some(), "Feld {f} fehlt");
    }
    let i = &v["inputs"][0];
    for f in ["transactionId", "index", "sequence", "sigOpCount", "computeBudget", "signatureScript", "utxo"] {
        assert!(i.get(f).is_some(), "Eingang: Feld {f} fehlt");
    }
    assert!(i["sequence"].is_string() && i["utxo"]["amount"].is_string() && v["lockTime"].is_string());
    assert_eq!(i["signatureScript"], "");
    assert_eq!(i["sigOpCount"], 0);
    assert!(i["computeBudget"].as_u64().unwrap() > 0, "Budget aus der Messung");
    assert!(i["utxo"]["covenantId"].is_string());
    assert!(i["utxo"]["scriptPublicKey"].as_str().unwrap().starts_with("0000aa20"), "P2SH mit Version 0000");
    assert_eq!(v["version"], 1);
    // Rückweg: Zahlen dürfen auch als Zahl kommen, „mass“ als alter Name
    let mut v2 = v.clone();
    v2["lockTime"] = serde_json::json!(0);
    let sm = v2["storageMass"].clone();
    v2.as_object_mut().unwrap().remove("storageMass");
    v2["mass"] = sm;
    let a = w::parse_signed(&v.to_string()).unwrap();
    let b = w::parse_signed(&v2.to_string()).unwrap();
    assert_eq!(w::from_safe(&a).unwrap().0.id(), w::from_safe(&b).unwrap().0.id());
    assert_eq!(w::from_safe(&a).unwrap().0.storage_mass(), w::from_safe(&b).unwrap().0.storage_mass());
    // Version 0 (Form aus dem rusty-kaspa-Test) wird gelesen, aber abgelehnt
    let v0 = r#"{"id":"0000000000000000000000000000000000000000000000000000000000000000","version":0,"inputs":[{"transactionId":"0101010101010101010101010101010101010101010101010101010101010101","index":0,"sequence":"0","sigOpCount":1,"signatureScript":"01","utxo":{"amount":"1","scriptPublicKey":"000001","blockDaaScore":"0","isCoinbase":false}}],"outputs":[],"subnetworkId":"0000000000000000000000000000000000000000","lockTime":"0","gas":"0","mass":"1","payload":""}"#;
    let s0 = w::parse_signed(v0).unwrap();
    assert!(w::from_safe(&s0).unwrap_err().contains("Version 0"));
}

#[test]
fn sighash_v1_deckt_budget_und_speichermasse_nicht_ab() {
    let k = key(9);
    let plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    let (tx, e) = w::from_safe(&plan.tx).unwrap();
    let h0 = w::sighash_all(&tx, &e, 0);
    let mut t2 = tx.clone();
    t2.inputs[0] = kaspa_consensus_core::tx::TransactionInput::new_with_compute_budget(tx.inputs[0].previous_outpoint, vec![], tx.inputs[0].sequence, 999);
    t2.set_storage_mass(tx.storage_mass() + 12_345);
    assert_eq!(h0, w::sighash_all(&t2, &e, 0), "Budget und Speichermasse sind nicht signiert");
    let mut t3 = tx.clone();
    t3.outputs[0].value -= 1;
    assert_ne!(h0, w::sighash_all(&t3, &e, 0), "Ausgänge sind signiert");
    let mut t4 = tx.clone();
    t4.payload = vec![1];
    assert_ne!(h0, w::sighash_all(&t4, &e, 0), "Payload ist signiert");
}

#[test]
fn einheiten_unabhaengig_vom_schluessel() {
    // gleiche Aktion (Tresor kündigen), zwei verschiedene Besitzer
    let mut units = vec![];
    for n in [3u8, 4, 5] {
        let k = key(n);
        let (p, s) = w::probe_params(&xonly(&k), NOW + 3_600_000).unwrap();
        let t = Tracked { outpoint: TransactionOutpoint::new([0xab; 32].into(), 0), value: 150_000_000, cov: [0xcd; 32].into(), state: s };
        let b = tresor::cancel(&p, &t, &k, &MAINNET_PARAMS).unwrap();
        units.push((b.used_units.clone(), b.budgets.clone(), b.compute_mass, b.transient_mass, b.storage_mass, b.fee, b.tx.inputs[0].signature_script.len()));
    }
    assert_eq!(units[0], units[1]);
    assert_eq!(units[1], units[2]);
    // und die Messkopie (Ersatzschlüssel) trifft die echte Wallet-Signatur genau
    let k = key(6);
    let plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    let (b, rep) = attach(&plan, &kastle_sign(&plan, &k, SIG_HASH_ALL));
    let b = b.expect("gültig");
    assert!(!rep.budgets_raised);
    assert_eq!(rep.used_units, plan.used_units);
    assert_eq!(b.fee, plan.fee);
    eprintln!("Kündigen: {} Einheiten, Budget {:?}, Gebühr {} sompi, Masse c/t/s {}/{}/{}", rep.used_units[0], rep.budgets, b.fee, b.compute_mass, b.transient_mass, b.storage_mass);
}

#[test]
fn stufe0_trockenprobe_kastle_und_kasware() {
    let k = key(11);
    let plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    assert!(plan.dry_only && plan.probe.fictional);
    assert_eq!(plan.signers.len(), 1);
    assert_eq!(plan.signers[0].kind, "tresor");
    // Kastle-Anfrage: scripts mit dem Redeem-Skript des Tresors
    let ks = plan.kastle();
    let redeem = ks["scripts"][0]["scriptHex"].as_str().unwrap();
    assert_eq!(redeem, faster_hex::hex_string(&bytecode(&standing_order(&plan.probe.params, &plan.probe.state))));
    for signed in [kastle_sign(&plan, &k, SIG_HASH_ALL), kasware_sign(&plan, &k)] {
        let (b, rep) = attach(&plan, &signed);
        assert!(rep.valid, "{rep:?}");
        assert!(rep.inputs[0].sig_valid && rep.inputs[0].hash_type == Some(1));
        let b = b.unwrap();
        kaspa_lending_protocol::txb::check_scripts(&b.tx, &b.entries).unwrap();
        // alles an die Wallet zurück
        assert_eq!(b.tx.outputs.len(), 1);
        assert_eq!(b.tx.outputs[0].script_public_key, p2pk_spk(&xonly(&k)));
        assert_eq!(b.tx.outputs[0].value, w::DRY_VALUE - b.fee);
    }
}

#[test]
fn ungueltige_signaturen_werden_erkannt() {
    let k = key(12);
    let plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    // falscher Schlüssel
    let (b, rep) = attach(&plan, &kastle_sign(&plan, &key(13), SIG_HASH_ALL));
    assert!(b.is_none() && !rep.valid && !rep.inputs[0].sig_valid);
    // anderer Hashtype
    let (b, rep) = attach(&plan, &kastle_sign(&plan, &k, SigHashType::from_u8(0x81).unwrap()));
    assert!(b.is_none() && rep.inputs[0].hash_type == Some(0x81));
    assert!(rep.inputs[0].note.contains("0x81"));
    // Wallet signiert stumm nicht (Kastle ohne scripts, Issue #353)
    let (tx, e) = w::from_safe(&plan.tx).unwrap();
    let (b, rep) = attach(&plan, &reply(&tx, &e));
    assert!(b.is_none() && !rep.inputs[0].signed && rep.inputs[0].note.contains("NICHT signiert"));
    // Wallet verändert einen Ausgang
    let signed: String = serde_json::from_str(&kastle_sign(&plan, &k, SIG_HASH_ALL)).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&signed).unwrap();
    v["outputs"][0]["value"] = "1000".into();
    let (b, rep) = attach(&plan, &v.to_string());
    assert!(b.is_none() && rep.changed.iter().any(|c| c.contains("Ausgang 0: Betrag")), "{rep:?}");
    // Budget und Speichermasse verändert: nicht signiert, wird überschrieben
    let mut v: serde_json::Value = serde_json::from_str(&signed).unwrap();
    v["inputs"][0]["computeBudget"] = 1.into();
    v["storageMass"] = "1".into();
    let (b, rep) = attach(&plan, &v.to_string());
    assert!(b.is_some() && rep.valid, "{rep:?}");
    assert_eq!(rep.ignored.len(), 2);
}

#[test]
fn manipulierter_plan_wird_abgelehnt() {
    let k = key(14);
    let mut plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    let signed = kastle_sign(&plan, &k, SIG_HASH_ALL);
    // Ausgang an eine fremde Adresse
    plan.tx.outputs[0].script_public_key = p2pk_spk(&xonly(&key(15)));
    let err = w::attach(&plan, &w::parse_signed(&signed).unwrap(), &MAINNET_PARAMS).unwrap_err();
    assert!(err.contains("Ausgang 0"), "{err}");
    // Trockenprobe als echt ausgegeben
    let mut plan = w::export_dry_cancel(&xonly(&k), "mainnet", Prefix::Mainnet, NOW, &MAINNET_PARAMS).unwrap();
    plan.dry_only = false;
    assert!(w::attach(&plan, &w::parse_signed(&signed).unwrap(), &MAINNET_PARAMS).unwrap_err().contains("Trockenprobe"));
}

fn wallet_utxos(sim: &Sim, k: &Keypair) -> Vec<(TransactionOutpoint, UtxoEntry)> {
    sim.funds(k).utxos
}

/// Stufe 1 im Simulator: KasWare signiert die Anlage
fn open_probe(sim: &mut Sim, k: &Keypair) -> w::Probe {
    let due = sim.now_ms as i64 + 3_600_000;
    let plan = w::export_open(&xonly(k), &wallet_utxos(sim, k), w::PROBE_FUND, due, "mainnet", Prefix::Mainnet, &sim.params).unwrap();
    assert!(!plan.dry_only);
    assert!(plan.signers.iter().all(|s| s.kind == "p2pk"));
    let outs = w::describe_outputs(&plan, Prefix::Mainnet);
    assert!(outs.iter().all(|o| o["what"] != "FREMDE Adresse"));
    // v1-Feld beim Rückweg verloren (Covenant-Bindung des Ausgangs): erkannt
    let lost: String = serde_json::from_str(&kasware_sign(&plan, k)).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&lost).unwrap();
    v["outputs"][0]["covenant"] = serde_json::Value::Null;
    let (_, rep) = attach(&plan, &v.to_string());
    assert!(!rep.valid && rep.changed.iter().any(|c| c.contains("Covenant-Bindung")), "{rep:?}");
    let (b, rep) = attach(&plan, &kasware_sign(&plan, k));
    assert!(rep.valid, "{rep:?}");
    assert!(!rep.budgets_raised);
    let b = b.unwrap();
    sim.submit(&b).unwrap();
    let probe = w::probe_after(&plan, &b);
    let op = tresor::parse_outpoint(&probe.outpoint).unwrap();
    let e = sim.utxos.get(&op).expect("Tresor-UTXO");
    assert_eq!(e.amount, w::PROBE_FUND);
    assert_eq!(e.covenant_id, Some(probe.cov));
    eprintln!("Anlegen: Gebühr {} sompi, {} Eingänge", b.fee, b.tx.inputs.len());
    // Probe-Datei: Rundweg
    let text = serde_json::to_string(&probe).unwrap();
    assert_eq!(w::Probe::decode(&text, "mainnet").unwrap(), probe);
    assert!(w::Probe::decode(&text, "testnet-10").is_err());
    probe
}

fn current(sim: &Sim, probe: &w::Probe) -> UtxoEntry {
    sim.utxos.get(&tresor::parse_outpoint(&probe.outpoint).unwrap()).unwrap().clone()
}

#[test]
fn stufe1_und_2_im_simulator() {
    let k = key(21);
    let mut sim = Sim::new();
    sim.faucet(&k, E8); // 1 KAS
    sim.faucet(&k, 4 * E8); // 4 KAS
    let before = sim.balance(&k);
    let probe = open_probe(&mut sim, &k);
    // Stufe 2: Kastle signiert den Covenant-Eingang
    let plan = w::export_cancel(&probe, current(&sim, &probe), "mainnet", Prefix::Mainnet, &sim.params).unwrap();
    assert_eq!(plan.kastle()["scripts"].as_array().unwrap().len(), 1);
    let (b, rep) = attach(&plan, &kastle_sign(&plan, &k, SIG_HASH_ALL));
    assert!(rep.valid, "{rep:?}");
    let b = b.unwrap();
    sim.submit(&b).unwrap();
    let after = sim.balance(&k);
    let cost = before - after;
    eprintln!("Stufe 1+2 kosten zusammen {cost} sompi");
    assert!(cost < E8 / 100, "nur Gebühren (< 0,01 KAS), war {cost}");
    // KasWare kann Stufe 2 ebenso (falls sie fremde Eingänge signiert)
    let probe = open_probe(&mut sim, &k);
    let plan = w::export_cancel(&probe, current(&sim, &probe), "mainnet", Prefix::Mainnet, &sim.params).unwrap();
    let (b, _) = attach(&plan, &kasware_sign(&plan, &k));
    sim.submit(&b.unwrap()).unwrap();
}

#[test]
fn sicherheitsnetz_zahlung_dann_kuendigen() {
    let k = key(22);
    let mut sim = Sim::new();
    sim.faucet(&k, 3 * E8);
    let probe = open_probe(&mut sim, &k);
    // vor dem Termin lehnt der Vertrag ab
    let b = w::probe_pay(&probe, &sim.params).unwrap();
    assert!(sim.submit(&b).is_err());
    sim.set_time(probe.state.next_due as u64 + 60_000);
    let before = sim.balance(&k);
    sim.submit(&b).unwrap();
    assert_eq!(sim.balance(&k) - before, w::PROBE_AMOUNT as u64, "1 KAS an die eigene Adresse");
    assert!(b.fee <= w::PROBE_MAX_FEE as u64);
    let next = w::probe_after_pay(&probe, &b);
    assert_eq!(next.state.left, 0);
    assert!(next.value >= (w::PROBE_FUND as i64 - w::PROBE_AMOUNT - w::PROBE_MAX_FEE) as u64);
    eprintln!("Zahlung: Gebühr {} sompi, Rest im Tresor {} sompi", b.fee, next.value);
    assert!(w::probe_pay(&next, &sim.params).is_err(), "nur eine Zahlung");
    // den Rest holt die Wallet-Signatur
    let plan = w::export_cancel(&next, current(&sim, &next), "mainnet", Prefix::Mainnet, &sim.params).unwrap();
    let (b, rep) = attach(&plan, &kastle_sign(&plan, &k, SIG_HASH_ALL));
    assert!(rep.valid, "{rep:?}");
    sim.submit(&b.unwrap()).unwrap();
}

#[test]
fn anlegen_grenzen_und_adressen() {
    let k = key(23);
    let mut sim = Sim::new();
    sim.faucet(&k, E8);
    let due = sim.now_ms as i64 + 3_600_000;
    let x = xonly(&k);
    // zu wenig KAS
    assert!(w::export_open(&x, &wallet_utxos(&sim, &k), w::PROBE_FUND, due, "mainnet", Prefix::Mainnet, &sim.params).unwrap_err().contains("zu wenig KAS"));
    sim.faucet(&k, 20 * E8);
    // Startguthaben zu klein / zu groß
    assert!(w::export_open(&x, &wallet_utxos(&sim, &k), 120_000_000, due, "mainnet", Prefix::Mainnet, &sim.params).is_err());
    assert!(w::export_open(&x, &wallet_utxos(&sim, &k), 11 * E8, due, "mainnet", Prefix::Mainnet, &sim.params).is_err());
    // fremde UTXOs werden nie genommen
    let other = key(24);
    sim.faucet(&other, 50 * E8);
    let mut all = wallet_utxos(&sim, &k);
    all.extend(wallet_utxos(&sim, &other));
    let plan = w::export_open(&x, &all, w::PROBE_FUND, due, "mainnet", Prefix::Mainnet, &sim.params).unwrap();
    assert!(w::from_safe(&plan.tx).unwrap().1.iter().all(|e| e.script_public_key == p2pk_spk(&x)));
    // Adresse ↔ x-only
    let a = w::address_of_xonly(&x, Prefix::Mainnet);
    eprintln!("Testadresse (Schlüssel 23): {a}");
    assert_eq!(w::xonly_of_address(&a, Prefix::Mainnet).unwrap(), x);
    assert!(w::xonly_of_address(&a, Prefix::Testnet).is_err());
    assert!(w::xonly_of_address("kaspa:qqqq", Prefix::Mainnet).is_err());
    let ecdsa = kaspa_addresses::Address::new(Prefix::Mainnet, kaspa_addresses::Version::PubKeyECDSA, &[2u8; 33]).to_string();
    assert!(w::xonly_of_address(&ecdsa, Prefix::Mainnet).unwrap_err().contains("Schnorr"));
}
