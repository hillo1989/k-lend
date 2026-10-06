//! Prüft das Auffinden von Token-Beträgen (src/pool.rs) an einer ECHTEN
//! Mainnet-Tx über die öffentliche REST-API. Braucht Netz, daher ignoriert:
//!   cargo test --test rest_live_tests -- --ignored
//! Tx: Prägung von 1 GHOST im Mainnet-Probelauf v1 (deployments/mainnet-v1.json),
//! Token an Ausgang 3 mit der v1-Vorlage (unverändertes KCC20-Beispiel).

use kaspa_lending_protocol::contracts::spk;
use kaspa_lending_protocol::pool::amount_candidates;
use silverscript_abi::ArtifactValue;
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};

const V1_KCC20: &str = include_str!("../../vendor/silverscript/silverscript-lang/tests/examples/kcc20.sil");
const TX: &str = "368cc2d8791d1b9a8e21cd2005ffd78f77d074045df5aec20a3f6a67d8745151";
const OWNER: &str = "ed2fa9eb8d521bd5d643d28afcd67e307bfc4bd4a39f29daf631e7d07bc3ab55";

#[test]
#[ignore]
fn betrag_und_skript_aus_echter_tx() {
    let url = format!("https://api.kaspa.org/transactions/{TX}?inputs=true&outputs=true&resolve_previous_outpoints=no");
    let v: serde_json::Value = ureq::get(&url).call().expect("REST").into_json().expect("JSON");
    let scripts: Vec<Vec<u8>> = v["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["signature_script"].as_str())
        .map(|h| {
            let mut b = vec![0u8; h.len() / 2];
            faster_hex::hex_decode(h.as_bytes(), &mut b).unwrap();
            b
        })
        .collect();
    let cands = amount_candidates(&scripts);
    assert!(cands.contains(&100_000_000), "1 GHOST unter den Kandidaten: {} Kandidaten", cands.len());

    let mut owner = vec![0u8; 32];
    faster_hex::hex_decode(OWNER.as_bytes(), &mut owner).unwrap();
    let art = compile_to_sil_abi_artifact_with_options(
        V1_KCC20,
        &[ArtifactValue::Bytes(owner), ArtifactValue::Int(100_000_000), ArtifactValue::Byte(0), ArtifactValue::Bool(false), ArtifactValue::Int(3), ArtifactValue::Int(2)],
        CompileOptions::default(),
    )
    .expect("v1-KCC20 kompiliert");
    let want = faster_hex::hex_string(spk(&art).script());
    let out3 = v["outputs"].as_array().unwrap().iter().find(|o| o["index"] == 3).unwrap();
    assert_eq!(out3["script_public_key"].as_str().unwrap(), want, "nachgerechnetes Token-Skript = Ausgang 3");
    // Nur ein Kandidat passt
    let hits = cands
        .iter()
        .filter(|&&a| {
            let mut o = vec![0u8; 32];
            faster_hex::hex_decode(OWNER.as_bytes(), &mut o).unwrap();
            let art = compile_to_sil_abi_artifact_with_options(
                V1_KCC20,
                &[ArtifactValue::Bytes(o), ArtifactValue::Int(a), ArtifactValue::Byte(0), ArtifactValue::Bool(false), ArtifactValue::Int(3), ArtifactValue::Int(2)],
                CompileOptions::default(),
            )
            .unwrap();
            faster_hex::hex_string(spk(&art).script()) == want
        })
        .count();
    assert_eq!(hits, 1);
    println!("{} Kandidaten, genau einer passt", cands.len());
}

/// Payload im Adressverlauf der REST-API (gemessen 29.09.2026): Feld `payload`
/// als Hex-Text, bei leerem Payload `null`; Eingänge mit
/// `previous_outpoint_address`. Adresse mit Payload-Tx einer fremden
/// Anwendung (Subnetz 97b1…); mit fremdem Schlüssel ist nichts lesbar.
///   cargo test --release --offline --test rest_live_tests -- --ignored payload
#[test]
#[ignore]
fn payload_im_adressverlauf() {
    use kaspa_lending_protocol::{chain, message};
    let addr = "kaspa:qrcxpm930jhc2tha2k5ep4xxyd0757qf0qj5aazzu0z8q6rgd2kkx59tjyjcl";
    let txs = chain::address_txs_json("mainnet", addr, 5).expect("REST");
    assert!(!txs.is_empty());
    let with_payload = txs.iter().filter(|t| t["payload"].as_str().is_some_and(|p| !p.is_empty())).count();
    println!("{} Tx, davon {with_payload} mit Payload (Hex)", txs.len());
    assert!(txs.iter().all(|t| t["payload"].is_null() || t["payload"].is_string()));
    assert!(txs.iter().all(|t| t["inputs"].as_array().is_some_and(|i| i.iter().all(|i| i["previous_outpoint_address"].is_string() || i["previous_outpoint_address"].is_null()))));
    let sk = secp256k1::SecretKey::new(&mut rand::thread_rng());
    for t in &txs {
        // eigene Sendungen der Adresse bzw. fremde Binärdaten: kein Eintrag
        assert_eq!(message::inbox_entry(t, addr, &sk, None), None);
    }
    let one = chain::tx_json("mainnet", txs[0]["transaction_id"].as_str().unwrap()).expect("REST Tx");
    assert_eq!(one["payload"], txs[0]["payload"]);
}
