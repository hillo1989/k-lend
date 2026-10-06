//! Nachricht im Payload normaler Transaktionen (send/transfer --onchain-message).
//!
//! Befund im Code von rusty-kaspa a41a333 (dieselbe Revision wie Cargo.toml):
//! - Konsens, Prüfung in Isolation: keine Payload-Regel für Nicht-Coinbase-Tx;
//!   der eigene Test setzt `tx.payload = vec![0]` auf eine native Tx und
//!   erwartet Ok (consensus/src/processes/transaction_validator/
//!   tx_validation_in_isolation.rs, Zeile 393–395). Begrenzt sind nur
//!   Coinbase-Payloads (max_coinbase_payload_len).
//! - Masse: jedes Payload-Byte zählt in die geschätzte Größe
//!   (consensus/core/src/mass/mod.rs, Zeile 37–38) und damit in Compute-
//!   (mass_per_tx_byte) und transiente Masse (× TRANSIENT_BYTE_TO_MASS_FACTOR = 4).
//! - Signatur: payload_hash geht in den Sighash ein, sobald der Payload nicht
//!   leer ist (consensus/core/src/hashing/sighash.rs, Zeile 184–195).
//! - Mempool: die eigenen Tests bauen native P2PK-Tx mit Payload als
//!   Standard-Tx (mining/src/toccata_transient_mass_activation_tests.rs, 507–519).
//!
//! Die Tests hier schicken Tx mit Payload durch dieselbe Prüfung wie alle
//! übrigen (src/sim.rs: Skripte mit Compute-Budget, Speichermasse,
//! Mindestgebühr) und messen, dass Gebühr und Masse den Payload decken.

use kaspa_consensus_core::tx::TransactionOutput;
use kaspa_lending_protocol::abo;
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::message;
use kaspa_lending_protocol::ops::{self, Deployment, p2pk_spk, xonly};
use kaspa_lending_protocol::sim::{Sim, test_register_params};
use kaspa_lending_protocol::txb::{self, Draft, In, Unlock, check_scripts, masses, min_fee};
use rand::{RngCore, thread_rng};
use secp256k1::{Keypair, Parity, Secp256k1, SecretKey};

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

/// KAS-Überweisung wie `ghostctl send`, optional mit Payload
fn kas_send(sim: &Sim, from: &Keypair, to: &Keypair, value: u64, payload: &[u8]) -> Result<txb::Built, String> {
    let f = sim.funds(from);
    let inputs = f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (*from).into() } }).collect();
    let out = TransactionOutput { value, script_public_key: p2pk_spk(&xonly(to)), covenant: None };
    txb::build_with_payload(Draft { inputs, outputs: vec![out], change_spk: p2pk_spk(&xonly(from)), lock_time: 0 }, payload, &sim.params)
}

#[test]
fn kas_mit_nachricht_wird_angenommen_und_die_gebuehr_deckt_den_payload() {
    let mut sim = Sim::new();
    let (a, b) = (key(), key());
    sim.faucet(&a, 100 * E8 as u64);
    let msg = "Miete Oktober – Wohnung 3 (Grüße!)";
    let payload = abo::message_payload(msg).expect("gültige Nachricht");
    assert_eq!(payload, msg.as_bytes(), "Payload = Nachricht in UTF-8");

    let plain = kas_send(&sim, &a, &b, 5 * E8 as u64, &[]).unwrap();
    let with = kas_send(&sim, &a, &b, 5 * E8 as u64, &payload).unwrap();
    assert_eq!(with.tx.payload, payload);
    assert!(plain.tx.payload.is_empty());

    // Masse wächst genau um die Payload-Bytes: 1 g Compute, 4 g transient je Byte
    let n = payload.len() as u64;
    assert_eq!(with.compute_mass - plain.compute_mass, n * sim.params.mass_per_tx_byte, "Compute-Masse je Payload-Byte");
    assert_eq!(with.transient_mass - plain.transient_mass, n * 4, "transiente Masse je Payload-Byte");
    let need = min_fee(with.compute_mass, with.transient_mass);
    assert!(with.fee >= need, "Gebühr {} deckt Mindestgebühr {need} mit Payload", with.fee);
    assert!(with.fee > plain.fee, "Payload kostet etwas mehr Gebühr");
    // unabhängig nachgerechnet mit dem MassCalculator von rusty-kaspa
    let (c, t, _) = masses(&with.tx, &with.entries, &sim.params).unwrap();
    assert_eq!((c, t), (with.compute_mass, with.transient_mass));

    // Simulator: Skripte (Signatur über den Payload), Speichermasse, Mindestgebühr
    sim.submit(&with).expect("Tx mit Payload angenommen");
    assert_eq!(sim.balance(&b), 5 * E8 as u64);
    println!(
        "KAS mit {n} Byte Nachricht: Gebühr {} sompi (ohne: {}), compute {} g, transient {} g",
        with.fee, plain.fee, with.compute_mass, with.transient_mass
    );
}

#[test]
fn payload_ist_von_der_signatur_gedeckt() {
    // Wer den Payload nachträglich ändert, macht die Signatur ungültig: die
    // Nachricht gehört fest zur Zahlung (sighash.rs payload_hash)
    let mut sim = Sim::new();
    let (a, b) = (key(), key());
    sim.faucet(&a, 100 * E8 as u64);
    let built = kas_send(&sim, &a, &b, 2 * E8 as u64, b"Miete").unwrap();
    check_scripts(&built.tx, &built.entries).expect("unverändert gültig");
    let mut forged = built.tx.clone();
    forged.payload = b"Mietx".to_vec();
    forged.finalize();
    assert!(check_scripts(&forged, &built.entries).is_err(), "geänderter Payload muss die Signatur brechen");
    let mut stripped = built.tx.clone();
    stripped.payload.clear();
    stripped.finalize();
    assert!(check_scripts(&stripped, &built.entries).is_err(), "entfernter Payload muss die Signatur brechen");
    let mut forged_built = built.clone();
    forged_built.tx = forged;
    assert!(sim.submit(&forged_built).is_err());
    sim.submit(&built).expect("Original angenommen");
}

#[test]
fn laengste_nachricht_passt_und_zu_lange_wird_abgelehnt() {
    let mut sim = Sim::new();
    let (a, b) = (key(), key());
    sim.faucet(&a, 100 * E8 as u64);
    // 100 Zeichen mit 4 Byte UTF-8 (Emoji) = 400 Byte Klartext, verschlüsselt
    // 464 Byte = die Höchstlänge
    let longest: String = "🏠".repeat(abo::MAX_MESSAGE_CHARS);
    let p = abo::message_payload(&longest).unwrap();
    assert_eq!(p.len(), 400);
    let built = kas_send(&sim, &a, &b, E8 as u64, &p).unwrap();
    sim.submit(&built).expect("400 Byte Payload angenommen");
    let e = message::encrypt(&xonly(&b), &longest).unwrap();
    assert_eq!(e.len(), txb::MAX_PAYLOAD);
    let built = kas_send(&sim, &a, &b, E8 as u64, &e).unwrap();
    sim.submit(&built).expect("464 Byte verschlüsselter Payload angenommen");
    assert!(abo::message_payload(&"x".repeat(abo::MAX_MESSAGE_CHARS + 1)).is_err());
    assert!(kas_send(&sim, &a, &b, E8 as u64, &vec![b'x'; txb::MAX_PAYLOAD + 1]).is_err());
}

#[test]
fn nachricht_ohne_steuerzeichen() {
    assert!(abo::message_payload("Miete\nOktober").is_err());
    assert!(abo::message_payload("a\u{0}b").is_err());
    assert!(abo::message_payload("\u{202e}gnudnewrebÜ").is_err(), "Richtungszeichen täuschen");
    assert!(abo::message_payload("Miete Oktober 2026, Whg. 3 – danke!").is_ok());
    assert!(abo::message_payload("").is_err(), "leer = keine Nachricht");
}

#[test]
fn ghost_mit_nachricht_wird_angenommen() {
    // GHOST-Überweisung (KCC20-Covenant) mit Payload: der Vertrag prüft den
    // Payload nicht, die Tx bleibt gültig
    let mut sim = Sim::new();
    let deployer = key();
    let committee: Vec<Keypair> = (0..5).map(|_| key()).collect();
    let (user, friend) = (key(), key());
    sim.faucet(&deployer, 1_000 * E8 as u64);
    sim.faucet(&user, 20_000 * E8 as u64);
    let net = sim.params.clone();
    let feed = sim.deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: committee.iter().map(xonly).collect(), t: 3, t_rot: 3 }, 4_000_000, 0, 1_000_000_000).expect("Register und Orakel");
    let op = feed.oracle_params.clone();
    let oracle_t = feed.oracle.clone();
    let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
    let (b, factory_t) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let (b, f2, root, vp) = ops::init_factory(&op, &oracle_t, &fp, &factory_t, &deployer, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let mut dep = Deployment {
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
    let (b, d) = ops::open_vault(&dep, &xonly(&user), 10_000 * E8 as u64, &sim.funds(&user), &net).unwrap();
    sim.submit(&b).unwrap();
    dep = d;
    let (b, d) = ops::mint(&dep, 0, &user, 50 * E8, &xonly(&user), &sim.funds(&user), &net).unwrap();
    sim.submit(&b).unwrap();
    dep = d;

    let payload = abo::message_payload("Taschengeld September").unwrap();
    let (plain, _) = ops::transfer(&dep, &user, &[0], &xonly(&friend), 5 * E8, &sim.funds(&user), &net).unwrap();
    let (b, d) = ops::transfer_with_payload(&dep, &user, &[0], &xonly(&friend), 5 * E8, &payload, &sim.funds(&user), &net).unwrap();
    assert_eq!(b.tx.payload, payload);
    let n = payload.len() as u64;
    assert_eq!(b.compute_mass - plain.compute_mass, n * net.mass_per_tx_byte);
    assert_eq!(b.transient_mass - plain.transient_mass, n * 4);
    assert!(b.fee >= min_fee(b.compute_mass, b.transient_mass));
    sim.submit(&b).expect("GHOST-Überweisung mit Payload angenommen");
    let got: i64 = d.tokens.iter().filter(|t| t.state.owner == xonly(&friend)).map(|t| t.state.amount).sum();
    assert_eq!(got, 5 * E8);
    println!("GHOST mit {n} Byte Nachricht: Gebühr {} sompi (ohne: {})", b.fee, plain.fee);

    // verschlüsselt an den GHOST-Empfänger (sein x-only-Pubkey ist der Token-Besitzer)
    let enc = message::payload_for("Taschengeld Oktober", false, Some(&xonly(&friend))).unwrap();
    let mine: Vec<usize> = d.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(&user)).map(|(i, _)| i).collect();
    let (b2, d2) = ops::transfer_with_payload(&d, &user, &mine, &xonly(&friend), 2 * E8, &enc, &sim.funds(&user), &net).unwrap();
    assert!(b2.fee >= min_fee(b2.compute_mass, b2.transient_mass));
    sim.submit(&b2).expect("GHOST-Überweisung mit verschlüsselter Nachricht angenommen");
    let got: i64 = d2.tokens.iter().filter(|t| t.state.owner == xonly(&friend)).map(|t| t.state.amount).sum();
    assert_eq!(got, 7 * E8);
    assert_eq!(message::decrypt(&SecretKey::from_keypair(&friend), &b2.tx.payload).as_deref(), Some("Taschengeld Oktober"));
    assert_eq!(message::decrypt(&SecretKey::from_keypair(&user), &b2.tx.payload), None);
    println!("GHOST mit {} Byte verschlüsselter Nachricht: Gebühr {} sompi", enc.len(), b2.fee);
}

#[test]
fn verschluesselte_nachricht_wird_angenommen_und_der_empfaenger_liest_sie() {
    // Empfänger mit ungerader y-Koordinate: die Adresse trägt nur x
    let mut sim = Sim::new();
    let a = key();
    let b = loop {
        let k = key();
        if k.x_only_public_key().1 == Parity::Odd {
            break k;
        }
    };
    let c = key();
    sim.faucet(&a, 100 * E8 as u64);
    let msg = "Miete Oktober – Wohnung 3";
    // Empfänger-Schlüssel wie ghostctl aus dem Ausgangsskript (P2PK)
    let to = message::recipient_of_spk(&p2pk_spk(&xonly(&b))).expect("P2PK");
    let payload = message::payload_for(msg, false, Some(&to)).unwrap();
    assert!(message::is_encrypted(&payload));
    assert_eq!(payload.len(), msg.len() + message::OVERHEAD);

    let plain = kas_send(&sim, &a, &b, 3 * E8 as u64, &[]).unwrap();
    let public = kas_send(&sim, &a, &b, 3 * E8 as u64, msg.as_bytes()).unwrap();
    let built = kas_send(&sim, &a, &b, 3 * E8 as u64, &payload).unwrap();
    let n = payload.len() as u64;
    assert_eq!(built.compute_mass - plain.compute_mass, n * sim.params.mass_per_tx_byte);
    assert_eq!(built.transient_mass - plain.transient_mass, n * 4);
    assert_eq!(built.transient_mass - public.transient_mass, message::OVERHEAD as u64 * 4, "Verschlüsselung kostet 64 Byte mehr");
    assert!(built.fee >= min_fee(built.compute_mass, built.transient_mass));
    let (cm, tm, _) = masses(&built.tx, &built.entries, &sim.params).unwrap();
    assert_eq!((cm, tm), (built.compute_mass, built.transient_mass));

    // Simulator: Signatur deckt den verschlüsselten Payload, Tx angenommen
    sim.submit(&built).expect("Tx mit verschlüsseltem Payload angenommen");
    assert_eq!(sim.balance(&b), 3 * E8 as u64);

    // Empfänger liest aus der angenommenen Tx; Absender und Dritte nicht
    let tx = &built.tx;
    let sk = |k: &Keypair| SecretKey::from_keypair(k);
    assert_eq!(message::decrypt(&sk(&b), &tx.payload).as_deref(), Some(msg));
    assert_eq!(message::read(&sk(&b), &tx.payload), Some(message::Found::Private(msg.into())));
    assert_eq!(message::decrypt(&sk(&a), &tx.payload), None, "Absender kann nicht entschlüsseln");
    assert_eq!(message::decrypt(&sk(&c), &tx.payload), None);
    // Manipulation am Payload bricht die Signatur (wie beim Klartext)
    let mut forged = tx.clone();
    forged.payload[60] ^= 1;
    forged.finalize();
    assert!(check_scripts(&forged, &built.entries).is_err());
    println!(
        "KAS mit {} Zeichen Nachricht: ohne {} sompi | öffentlich {} Byte {} sompi | verschlüsselt {n} Byte {} sompi; compute {} g, transient {} g",
        msg.chars().count(),
        plain.fee,
        msg.len(),
        public.fee,
        built.fee,
        built.compute_mass,
        built.transient_mass
    );
    let longest = message::encrypt(&to, &"🏠".repeat(abo::MAX_MESSAGE_CHARS)).unwrap();
    let big = kas_send(&sim, &a, &b, 3 * E8 as u64, &longest).unwrap();
    println!(
        "KAS mit längster verschlüsselter Nachricht ({} Byte): Gebühr {} sompi, compute {} g, transient {} g, storage {} g",
        longest.len(),
        big.fee,
        big.compute_mass,
        big.transient_mass,
        big.storage_mass
    );
    sim.submit(&big).expect("längste verschlüsselte Nachricht angenommen");
    assert_eq!(message::decrypt(&sk(&b), &big.tx.payload).map(|t| t.chars().count()), Some(abo::MAX_MESSAGE_CHARS));
}

#[test]
fn verschluesselt_nur_an_schnorr_adressen() {
    // P2SH-Empfänger: klare Fehlermeldung statt Klartext oder stiller Verlust
    let p2sh = kaspa_txscript::pay_to_script_hash_script(&[0x51]);
    assert_eq!(message::recipient_of_spk(&p2sh), None);
    assert_eq!(message::payload_for("Miete", false, None).unwrap_err(), message::NOT_P2PK);
    assert_eq!(message::payload_for("Miete", true, None).unwrap(), b"Miete", "öffentlich geht");
    assert!(message::payload_for("", false, None).unwrap().is_empty(), "ohne Nachricht kein Payload");
}

// ------------------------------------------------ Audit 12, A12-11 (Filter) ----

/// Dieselben Fälle wie die Seite (app/src/lib/audit12a-nachrichten.test.ts):
/// unsichtbare und täuschende Zeichen werden abgelehnt, normale Texte (auch
/// Emoji mit VS16, Akzente, andere Schriften) bleiben erlaubt.
#[test]
fn a12_nachrichtenfilter_gleiche_faelle_wie_die_seite() {
    let data: serde_json::Value = serde_json::from_str(include_str!("data/nachrichtenfilter.json")).unwrap();
    for c in data["cases"].as_array().unwrap() {
        let (name, text, ok) = (c["name"].as_str().unwrap(), c["text"].as_str().unwrap(), c["ok"].as_bool().unwrap());
        assert_eq!(abo::check_message(text).is_ok(), ok, "{name}: {text:?}");
        if ok && !text.is_empty() {
            // auch der Weg über Payload und Verschlüsselung
            assert!(abo::message_payload(text).is_ok(), "{name}");
        }
    }
}

/// Jeder Codepunkt zwischen zwei Buchstaben: abgelehnt genau in den Bereichen
/// der gemeinsamen Liste (die Seite prüft dieselbe Liste zusätzlich gegen die
/// Unicode-Eigenschaften Cc, Cf, Co, Zl, Zp, Default_Ignorable_Code_Point und
/// Noncharacter_Code_Point ihrer JavaScript-Engine)
#[test]
fn a12_nachrichtenfilter_alle_codepunkte() {
    let data: serde_json::Value = serde_json::from_str(include_str!("data/nachrichtenfilter.json")).unwrap();
    let want: Vec<String> = data["ranges"].as_array().unwrap().iter().map(|r| r.as_str().unwrap().to_string()).collect();
    let mut got: Vec<(u32, u32)> = vec![];
    for u in 0..=0x10FFFFu32 {
        let Some(c) = char::from_u32(u) else { continue }; // Surrogate gibt es in Rust-Text nicht
        if abo::check_message(&format!("a{c}b")).is_err() {
            match got.last_mut() {
                Some((_, b)) if *b + 1 == u => *b = u,
                _ => got.push((u, u)),
            }
        }
    }
    let got: Vec<String> = got.iter().map(|(a, b)| format!("{a:04X}-{b:04X}")).collect();
    assert_eq!(got, want);
}

/// Nachprüfung zu A12-11: Der strengere Filter gilt auch für gespeicherte
/// Daueraufträge. Vorher scheiterte jede Ausführung eines älteren Auftrags mit
/// Tastenkappen-Emoji, U+2028 oder Private-Use-Zeichen an `check_message` –
/// die Zahlung selbst blieb aus. Jetzt geht die Nachricht nach heutigem Filter
/// (unzulässige Zeichen weg, Zeilentrenner als Leerzeichen).
#[test]
fn a12n_aeltere_dauerauftraege_zahlen_weiter() {
    let data: serde_json::Value = serde_json::from_str(include_str!("data/nachrichtenfilter.json")).unwrap();
    for c in data["cases"].as_array().unwrap() {
        let (name, text, ok) = (c["name"].as_str().unwrap(), c["text"].as_str().unwrap(), c["ok"].as_bool().unwrap());
        let s = abo::sendable_message(text);
        assert!(abo::check_message(&s).is_ok(), "{name}: {s:?}");
        if ok {
            assert_eq!(s, text.trim(), "{name}: erlaubter Text bleibt unverändert");
        }
    }
    for (old, now) in [
        ("Miete\u{2028}Mai", "Miete Mai"),
        ("Platz 1\u{fe0f}\u{20e3}", "Platz 1\u{20e3}"),
        ("Danke \u{2764}\u{fe0f}", "Danke \u{2764}\u{fe0f}"),
        ("\u{2764}\u{200b}\u{fe0f}", "\u{2764}\u{fe0f}"),
        ("a\u{e000}b\u{e0041}", "ab"),
        ("\u{e000}", ""),
    ] {
        assert_eq!(abo::sendable_message(old), now, "{old:?}");
    }
    let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
    let new = |onchain: bool| abo::NewAbo {
        key: "keys/a.json".into(),
        asset: abo::Asset::Kas,
        to: "kaspa:qx".into(),
        amount: "10".into(),
        message: "Miete".into(),
        onchain,
        interval: abo::Interval::parse("monatlich").unwrap(),
        start: today,
        end: None,
        count: Some(3),
    };
    // öffentlich: so gespeichert, wie ein älterer ghostctl ihn angelegt hat
    let mut a = abo::new_abo(new(true), today, "x", "1".into()).unwrap();
    a.message = "Miete\u{2028}Mai".into();
    assert!(abo::check_message(&a.message).is_err());
    assert_eq!(abo::payload(&a, None).expect("zahlt weiter"), b"Miete Mai");
    // verschlüsselt
    let r = key();
    let mut a = abo::new_abo(new(false), today, "x", "2".into()).unwrap();
    a.message = "Platz 1\u{fe0f}\u{20e3}".into();
    let p = abo::payload(&a, Some(&xonly(&r))).expect("zahlt weiter");
    assert_eq!(message::decrypt(&SecretKey::from_keypair(&r), &p).as_deref(), Some("Platz 1\u{20e3}"));
    // bleibt nichts übrig: Zahlung ohne Nachricht
    a.message = "\u{e000}".into();
    assert!(abo::payload(&a, Some(&xonly(&r))).unwrap().is_empty());
}
