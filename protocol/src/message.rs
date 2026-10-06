//! Nachricht im Payload einer Überweisung: öffentlich (Klartext) oder
//! verschlüsselt an den Empfänger.
//!
//! Verschlüsselt (Format "GHM\x01", 64 Byte Zusatz):
//!
//! ```text
//! Magic "GHM\x01" (4) || E_x (32) || Nonce (12) || Ciphertext || Tag (16)
//! ```
//!
//! - Empfänger-Schlüssel P: x-only-Pubkey der Schnorr-Adresse (kaspa:q…, P2PK),
//!   also genau das, was jede Kaspa-Adresse ohnehin öffentlich enthält.
//! - Je Nachricht ein frischer Ephemeral-Schlüssel e (BIP340-Stil: bei
//!   ungerader y-Koordinate negiert, damit E = e·G = lift_x(E_x)).
//! - ECDH nur über die x-Koordinate: shared = x(e · lift_x(P)). Der Empfänger
//!   rechnet x(sk · lift_x(E)). lift_x(P) ist ±sk·G, je nach Vorzeichen des
//!   Empfänger-Schlüssels; ±Q haben dieselbe x-Koordinate, daher stimmt das
//!   Geheimnis in beiden Fällen (Test `empfaenger_mit_ungerader_y`).
//! - Schlüssel = BLAKE2b-256("GHOST-Nachricht v1" || shared_x || E_x || P_x).
//!   E_x und P_x binden den Schlüssel an genau dieses Paar.
//! - ChaCha20-Poly1305, zufälliger 12-Byte-Nonce, Associated Data = Magic || E_x.
//!   Der Schlüssel ist schon je Nachricht neu; der Nonce ist Zusatzschutz,
//!   falls ein Zufallsgenerator einmal denselben Ephemeral-Schlüssel liefert.
//!
//! Öffentlich bleiben: dass es eine Nachricht gibt, ihre Länge (±0 Byte, kein
//! Auffüllen), Absender, Empfänger und Betrag der Tx. Wer später an den
//! geheimen Schlüssel des Empfängers kommt, kann alle alten Nachrichten an ihn
//! lesen (keine Vorwärtssicherheit auf Empfängerseite). Der Absender kann
//! seine eigene verschlüsselte Nachricht nicht wieder entschlüsseln (e ist
//! verworfen); er sieht sie nur im lokalen Verlauf.
//!
//! Klartext-Nachrichten können nie mit dem Magic beginnen: `\x01` ist ein
//! Steuerzeichen und wird von `abo::check_message` abgelehnt.
//!
//! Nicht gegeben: Absender-Echtheit. Jeder kann an eine Adresse verschlüsseln.
//! Der Eingang (`inbox_entry`) zeigt deshalb als „von“ nur die
//! Schlüssel-Adressen unter den Eingängen der Tx. Tresor-Zahlungen erkennt er
//! am Zweig `pay` samt Ausgang an mich. Ihre Nachricht bindet der Vertrag
//! (payloadHash, Audit 12, A12-1 im Vertrag): Sie wurde beim Anlegen
//! hinterlegt, wer auslöst, kann sie nicht ändern. Den Besitzer nennt der
//! Eingang nur bei hier übernommenen Tresoren (anlegen kann jeder einen Tresor
//! mit beliebigem Besitzer, ohne dessen Signatur). Die Tx selbst kommt von der REST-API; `node_verdict`
//! gleicht sie mit dem Block am Node ab, solange der Node ihn noch hat (A12-19).

use crate::abo::{MAX_MESSAGE_CHARS, check_message, has_bad_char, message_payload};
use crate::contracts::TresorParams;
use crate::tresor::TresorRec;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use kaspa_consensus_core::tx::ScriptPublicKey;
use rand::RngCore;
use secp256k1::{Parity, PublicKey, SECP256K1, SecretKey, XOnlyPublicKey, ecdh};

pub const MAGIC: [u8; 4] = *b"GHM\x01";
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
/// Zusatzbytes einer verschlüsselten Nachricht: Magic, E_x, Nonce, Tag
pub const OVERHEAD: usize = MAGIC.len() + 32 + NONCE_LEN + TAG_LEN;
/// Längste verschlüsselte Nachricht: 100 Zeichen à 4 Byte UTF-8 + Zusatz
pub const MAX_ENCRYPTED: usize = MAX_MESSAGE_CHARS * 4 + OVERHEAD;
const KDF_DOMAIN: &[u8] = b"GHOST-Nachricht v1";

/// Fehlertext, wenn der Empfänger keine normale Schnorr-Adresse hat
pub const NOT_P2PK: &str = "verschlüsselt nur an normale Kaspa-Adressen – Nachricht öffentlich oder weglassen";

/// Payload im Format einer verschlüsselten Nachricht (beginnt mit dem Magic)
pub fn is_encrypted(payload: &[u8]) -> bool {
    payload.starts_with(&MAGIC)
}

/// x(scalar · point), konstante Laufzeit (libsecp256k1 ECDH)
fn shared_x(point: &PublicKey, scalar: &SecretKey) -> [u8; 32] {
    let xy = ecdh::shared_secret_point(point, scalar);
    let mut x = [0u8; 32];
    x.copy_from_slice(&xy[..32]);
    x
}

fn derive_key(shared: &[u8; 32], e_x: &[u8], p_x: &[u8]) -> [u8; 32] {
    let h = blake2b_simd::Params::new().hash_length(32).to_state().update(KDF_DOMAIN).update(shared).update(e_x).update(p_x).finalize();
    let mut k = [0u8; 32];
    k.copy_from_slice(h.as_bytes());
    k
}

/// Verschlüsseln mit festem Ephemeral-Schlüssel und Nonce (nur für Tests;
/// im Betrieb `encrypt`)
fn encrypt_with(recipient_xonly: &[u8], text: &str, mut e: SecretKey, nonce: [u8; NONCE_LEN]) -> Result<Vec<u8>, String> {
    check_message(text)?;
    let p = XOnlyPublicKey::from_slice(recipient_xonly).map_err(|_| "Empfänger ist kein gültiger Schnorr-Schlüssel".to_string())?;
    let (e_x, parity) = e.x_only_public_key(SECP256K1);
    if parity == Parity::Odd {
        e = e.negate(); // gleiche x-Koordinate, jetzt gerade y: E = lift_x(E_x)
    }
    let e_x = e_x.serialize();
    let key = derive_key(&shared_x(&p.public_key(Parity::Even), &e), &e_x, &p.serialize());
    let mut out = Vec::with_capacity(OVERHEAD + text.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&e_x);
    let ct = ChaCha20Poly1305::new(Key::from_slice(&key))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: text.as_bytes(), aad: &out })
        .map_err(|_| "Verschlüsseln fehlgeschlagen".to_string())?;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Nachricht an den x-only-Pubkey des Empfängers verschlüsseln. Jeder Aufruf
/// nimmt einen neuen Ephemeral-Schlüssel und Nonce aus dem Zufall des
/// Betriebssystems.
pub fn encrypt(recipient_xonly: &[u8], text: &str) -> Result<Vec<u8>, String> {
    let mut rng = rand::rngs::OsRng;
    let e = SecretKey::new(&mut rng);
    let mut nonce = [0u8; NONCE_LEN];
    rng.fill_bytes(&mut nonce);
    encrypt_with(recipient_xonly, text, e, nonce)
}

/// Mit dem geheimen Schlüssel des Empfängers entschlüsseln. None bei falschem
/// Schlüssel, verändertem Payload oder fremdem Format.
pub fn decrypt(sk: &SecretKey, payload: &[u8]) -> Option<String> {
    if !is_encrypted(payload) || payload.len() < OVERHEAD {
        return None;
    }
    let (head, rest) = payload.split_at(MAGIC.len() + 32);
    let (nonce, ct) = rest.split_at(NONCE_LEN);
    let e_x = &head[MAGIC.len()..];
    let e = XOnlyPublicKey::from_slice(e_x).ok()?;
    let (p_x, _) = sk.x_only_public_key(SECP256K1);
    let key = derive_key(&shared_x(&e.public_key(Parity::Even), sk), e_x, &p_x.serialize());
    let pt = ChaCha20Poly1305::new(Key::from_slice(&key)).decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: head }).ok()?;
    String::from_utf8(pt).ok()
}

/// x-only-Pubkey eines Schnorr-P2PK-Skripts (`OP_DATA_32 <x> OP_CHECKSIG`),
/// sonst None (P2SH, ECDSA-P2PK, Covenants)
pub fn recipient_of_spk(spk: &ScriptPublicKey) -> Option<[u8; 32]> {
    let s = spk.script();
    if spk.version() != 0 || s.len() != 34 || s[0] != 0x20 || s[33] != 0xac {
        return None;
    }
    XOnlyPublicKey::from_slice(&s[1..33]).ok()?;
    let mut x = [0u8; 32];
    x.copy_from_slice(&s[1..33]);
    Some(x)
}

/// Payload einer Überweisung aus Nachricht und Häkchen:
/// - keine Nachricht → kein Payload (öffentlich ohne Nachricht ist ein Fehler)
/// - `public` → Klartext wie bisher
/// - sonst verschlüsselt an `recipient` (x-only); ohne P2PK-Empfänger Fehler
pub fn payload_for(message: &str, public: bool, recipient: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let m = message.trim();
    check_message(m)?;
    if m.is_empty() {
        if public {
            return Err("Öffentliche Nachricht gewählt, aber keine Nachricht angegeben".into());
        }
        return Ok(vec![]);
    }
    if public {
        return message_payload(m);
    }
    encrypt(recipient.ok_or(NOT_P2PK)?, m)
}

/// Was ein Empfänger in einem Payload findet
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// verschlüsselt an diesen Schlüssel, entschlüsselt
    Private(String),
    /// verschlüsselt, aber nicht für diesen Schlüssel oder verändert
    Unreadable,
    /// öffentlicher Klartext
    Public(String),
    /// Text (öffentlich oder entschlüsselt), den `check_message` ablehnt: die
    /// Zahlung erscheint, der Text nicht – auch nach einer Verschärfung des
    /// Filters verschwindet sie so nicht aus dem Eingang (Nachprüfung A12-11).
    /// Der Grund steht dabei (Restpunkt: auch zu lange Texte landen hier, etwa
    /// JSON einer anderen Anwendung, und sind nicht „unzulässige Zeichen“).
    Invalid(Rejected),
}

/// Warum ein gelesener Text nicht angezeigt wird (`Found::Invalid`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    /// länger als MAX_MESSAGE_CHARS Zeichen, sonst zulässig
    TooLong,
    /// unsichtbare, Steuer- oder Formatzeichen (Liste bei `abo::BAD_CHARS`)
    BadChars,
    /// beides
    TooLongAndBadChars,
}

/// Grund, aus dem `check_message` den Text ablehnt; None = zulässig
pub fn rejected(t: &str) -> Option<Rejected> {
    match (t.chars().count() > MAX_MESSAGE_CHARS, has_bad_char(t)) {
        (false, false) => None,
        (true, false) => Some(Rejected::TooLong),
        (false, true) => Some(Rejected::BadChars),
        (true, true) => Some(Rejected::TooLongAndBadChars),
    }
}

impl Found {
    /// Angezeigter Text, falls es einen gibt
    pub fn text(&self) -> Option<&str> {
        match self {
            Found::Private(t) | Found::Public(t) => Some(t),
            Found::Unreadable | Found::Invalid(_) => None,
        }
    }
}

/// Payload einer eingegangenen Tx deuten. None = keine Nachricht (leer,
/// nur Leerraum oder Binärdaten einer anderen Anwendung). Auch entschlüsselter
/// Text muss die Regeln für Nachrichten erfüllen (keine Steuer- und
/// Formatzeichen, höchstens MAX_MESSAGE_CHARS Zeichen), denn der Absender kann
/// beliebige Bytes verschlüsseln; sonst `Invalid` mit dem Grund.
pub fn read(sk: &SecretKey, payload: &[u8]) -> Option<Found> {
    if payload.is_empty() {
        return None;
    }
    if is_encrypted(payload) {
        return Some(match decrypt(sk, payload) {
            Some(t) if t.trim().is_empty() => Found::Unreadable,
            Some(t) => match rejected(&t) {
                None => Found::Private(t),
                Some(why) => Found::Invalid(why),
            },
            None => Found::Unreadable,
        });
    }
    match std::str::from_utf8(payload) {
        Ok(t) if t.trim().is_empty() => None,
        Ok(t) => Some(match rejected(t) {
            None => Found::Public(t.to_string()),
            Some(why) => Found::Invalid(why),
        }),
        Err(_) => None,
    }
}

// ------------------------------------------- Eingang (ghostctl messages) ----

/// Wie weit eine Tresor-Nachricht geprüft ist. Der Vertrag bindet den Payload
/// jeder Zahlung (sha256 = payloadHash, A12-1 im Vertrag): Wer auslöst, kann
/// keine eigene Nachricht einsetzen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TresorCheck {
    /// beim Anlegen hinterlegt (vom Vertrag erzwungen), Tresor hier übernommen,
    /// und der Text ist genau die Beschreibung, die die Tresor-Liste zeigt
    AsStored,
    /// beim Anlegen hinterlegt (vom Vertrag erzwungen): Der Payload passt zum
    /// Hash im Vertrag. Tresor hier nicht übernommen (Besitzer nicht geprüft)
    /// oder sein Code beschreibt die Nachricht anders, als sie lautet
    Bound,
    /// passt NICHT zum Hash im Vertrag. Im Netz lehnt der Vertrag eine solche
    /// Zahlung ab; so etwas kann nur aus falschen Daten der REST-API stammen
    /// (ohne Gegenprüfung am Node)
    Inserted,
}

/// Woher eine eingegangene Zahlung kommt – und was das über ihre Nachricht sagt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// nur Eingänge mit Schlüssel-Adressen (die Nachricht setzt, wer die Tx baut)
    Direct,
    /// Zahlung eines Tresors laut Vertrag: ein Eingang nimmt den Zweig `pay`
    /// eines standing_order-Tresors an diesen Schlüssel, am selben Index steht
    /// der Ausgang an ihn mit genau dem Betrag. `id` und `owner` nur bei einem
    /// Tresor, dessen Code hier übernommen oder der hier angelegt wurde:
    /// `owner` ist bloß ein Parameter des Skripts – anlegen und auslösen kann
    /// jeder, ohne Signatur des Besitzers (Nachprüfung A12-1).
    Tresor { id: Option<String>, owner: Option<[u8; 32]>, check: TresorCheck },
    /// aus einem Vertrag (Skript-Adresse unter den Eingängen), auch Kündigen
    /// oder Auffüllen eines Tresors mit Ausgang an diesen Schlüssel
    Contract,
}

/// Woher der Inhalt der Zahlung stammt
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// nur von der öffentlichen REST-API
    Rest,
    /// am Node gegengeprüft: dieselbe Tx (Payload, Eingänge, Ausgänge) im Block
    Node,
}

/// Eine eingegangene Nachricht
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inbox {
    pub txid: String,
    /// Blockzeit in ms seit 1970, falls bekannt
    pub time_ms: Option<u64>,
    /// Betrag an diesen Schlüssel in 1e-8 (sompi bzw. GHOST-Einheiten); bei
    /// einer Tresor-Zahlung der Betrag laut Vertrag (= Ausgang an mich am
    /// Index des Tresors)
    pub amount: u64,
    pub unit: &'static str,
    /// Schlüssel-Adressen der Eingänge, soweit die API sie auflöst – nie der
    /// Besitzer eines Tresors (der steht in `origin`), nie die eigene Adresse
    pub from: Vec<String>,
    pub found: Found,
    pub origin: Origin,
    pub source: Source,
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut b = vec![0u8; s.len() / 2];
    faster_hex::hex_decode(s.as_bytes(), &mut b).ok()?;
    Some(b)
}

/// Adresse eines Schlüssels (P2PK, Schnorr oder ECDSA), nicht eines Skripts
fn is_key_address(a: &str) -> bool {
    kaspa_addresses::Address::try_from(a).is_ok_and(|x| x.version != kaspa_addresses::Version::ScriptHash)
}

/// Adresse eines Skripts (P2SH)
fn is_script_address(a: &str) -> bool {
    kaspa_addresses::Address::try_from(a).is_ok_and(|x| x.version == kaspa_addresses::Version::ScriptHash)
}

/// Ausgang `i` im REST-JSON: über das Feld `index`, sonst die Stelle in der Liste
fn output_at(tx: &serde_json::Value, i: usize) -> Option<&serde_json::Value> {
    let outs = tx["outputs"].as_array()?;
    if outs.iter().any(|o| !o["index"].is_null()) {
        return outs.iter().find(|o| o["index"].as_u64() == Some(i as u64));
    }
    outs.get(i)
}

/// Ein- oder Ausgänge im REST-JSON in der Reihenfolge der Tx: nach dem Feld
/// `index` (api.kaspa.org nennt es bei beiden), ohne das Feld in
/// Listenreihenfolge. None, wenn die Indizes nicht genau 0..n sind – eine
/// solche Angabe passt zu keiner Tx (zweite Nachprüfung: `same_as_node` und
/// `tresor_payment` hingen von der Listenreihenfolge ab).
fn in_index_order(list: &[serde_json::Value]) -> Option<Vec<&serde_json::Value>> {
    if list.iter().all(|x| x["index"].is_null()) {
        return Some(list.iter().collect());
    }
    let mut v: Vec<(u64, &serde_json::Value)> = list.iter().map(|x| x["index"].as_u64().map(|i| (i, x))).collect::<Option<_>>()?;
    v.sort_by_key(|(i, _)| *i);
    v.iter().enumerate().all(|(n, (i, _))| *i == n as u64).then(|| v.into_iter().map(|(_, x)| x).collect())
}

/// Zahlt dieser Ausgang genau `amount` an meinen Schlüssel? Geprüft werden
/// Skript und Adresse, soweit die API sie nennt (mindestens eines davon).
fn pays_me(o: &serde_json::Value, my_addr: &str, my_spk_hex: &str, amount: u64) -> bool {
    let (script, addr) = (o["script_public_key"].as_str(), o["script_public_key_address"].as_str());
    o["amount"].as_u64() == Some(amount)
        && (script.is_some() || addr.is_some())
        && script.is_none_or(|s| s.eq_ignore_ascii_case(my_spk_hex))
        && addr.is_none_or(|a| a == my_addr)
}

/// Tresor-Zahlung an mich laut Vertrag, sonst None. Verlangt:
/// - ein Eingang nimmt den Zweig `pay` eines standing_order-Tresors
///   (`tresor::pay_input`: Signaturskript genau Selektor + Redeem-Skript) mit
///   Empfänger = ich; soweit die API die Adresse des ausgegebenen Outpoints
///   nennt, passt die P2SH-Adresse zum Skript;
/// - am selben Index steht der Ausgang an mich mit genau dem Betrag (so
///   verlangt es `pay`).
///
/// `cancel` und `topUp` laufen mit der Signatur des Besitzers durch dasselbe
/// Redeem-Skript und dürfen beliebige Ausgänge haben: Wer seinen Tresor mit
/// Betrag 10 000 KAS kündigt und dabei 0,3 KAS an mich schickt, ist keine
/// Tresor-Zahlung über 10 000 KAS (Nachprüfung A12-1).
fn tresor_payment(tx: &serde_json::Value, prefix: kaspa_addresses::Prefix, my_x: &[u8; 32], my_addr: &str) -> Option<TresorParams> {
    let my_spk = faster_hex::hex_string(crate::ops::p2pk_spk(my_x).script());
    for (i, input) in in_index_order(tx["inputs"].as_array()?)?.into_iter().enumerate() {
        let Some(sig) = input["signature_script"].as_str().and_then(hex_bytes) else { continue };
        let Some((p, _)) = crate::tresor::pay_input(&sig) else { continue };
        let Ok(amount) = u64::try_from(p.amount) else { continue };
        if p.recipient != my_x {
            continue;
        }
        if let Some(a) = input["previous_outpoint_address"].as_str() {
            let code = crate::tresor::redeem_script(&sig)?;
            let spk = kaspa_txscript::pay_to_script_hash_script(&code);
            let want = kaspa_txscript::standard::extract_script_pub_key_address(&spk, prefix).map(|x| x.to_string()).ok();
            if want.as_deref() != Some(a) {
                continue;
            }
        }
        if !output_at(tx, i).is_some_and(|o| pays_me(o, my_addr, &my_spk, amount)) {
            continue;
        }
        return Some(p);
    }
    None
}

/// Nachricht einer Tresor-Zahlung prüfen: zuerst gegen den Hash im Vertrag
/// (`p.payload_hash`, gilt auch für Tresore, die hier niemand übernommen hat),
/// dann gegen die hier übernommenen Tresore mit genau diesen Parametern: „wie
/// im Tresor-Code“ nur, wenn der Payload die hinterlegte Fassung ist UND für
/// diesen Schlüssel den Text ergibt, den die Tresor-Liste zeigt (ein Code,
/// dessen `sealed` etwas anderes enthält als `message`, macht die Nachricht
/// nicht zur Beschreibung der Liste). Besitzer und ID nur bei bekannten
/// Tresoren. `text` = gelesener Text, auch wenn er nicht angezeigt wird
/// (`raw_text`).
fn check_tresor(p: &TresorParams, payload: &[u8], text: Option<&str>, known: &[TresorRec]) -> (Option<String>, Option<[u8; 32]>, TresorCheck) {
    let bound = crate::contracts::payload_hash(payload) == p.payload_hash;
    let same: Vec<&TresorRec> = known.iter().filter(|r| &r.params == p).collect();
    let Some(first) = same.first() else {
        return (None, None, if bound { TresorCheck::Bound } else { TresorCheck::Inserted });
    };
    let owner = <[u8; 32]>::try_from(p.owner.as_slice()).ok();
    if !bound {
        return (Some(first.id.clone()), owner, TresorCheck::Inserted);
    }
    match same.iter().find(|r| r.payload() == payload && text.map(str::trim) == Some(r.message.trim())) {
        Some(r) => (Some(r.id.clone()), owner, TresorCheck::AsStored),
        None => (Some(first.id.clone()), owner, TresorCheck::Bound),
    }
}

/// Text eines Payloads ohne die Regeln von `check_message` (entschlüsselt bzw.
/// UTF-8), nur für den Abgleich mit der hinterlegten Nachricht
fn raw_text(sk: &SecretKey, payload: &[u8]) -> Option<String> {
    if is_encrypted(payload) {
        decrypt(sk, payload)
    } else {
        String::from_utf8(payload.to_vec()).ok()
    }
}

/// Eine Tx im JSON der REST-API (api.kaspa.org, Felder `transaction_id`,
/// `block_time`, `is_accepted`, `payload` als Hex, `inputs[].previous_outpoint_address`/
/// `signature_script`, `outputs[].script_public_key(_address)`/`amount`) für den
/// Schlüssel `sk` mit Adresse `my_addr` auswerten. Nur angenommene, eingehende
/// Tx mit Nachricht; eigene Sendungen (ein Eingang von `my_addr`) zählen nicht –
/// außer einer Tresor-Zahlung an mich, deren Gebühr ich selbst getragen habe.
/// `ghost` = Betrag der GHOST an diesen Schlüssel (aus den Token-UTXOs der
/// Zustandsdatei); ohne ihn zählen die KAS-Ausgänge an `my_addr`.
/// Tresor-Zahlungen werden erkannt und ihre Nachricht gegen den Hash im
/// Vertrag geprüft; mit der Beschreibung bekannter Tresore vergleicht erst
/// `inbox_entry_with`.
pub fn inbox_entry(tx: &serde_json::Value, my_addr: &str, sk: &SecretKey, ghost: Option<u64>) -> Option<Inbox> {
    inbox_entry_with(tx, my_addr, sk, ghost, &[])
}

/// Wie `inbox_entry`; `known` = Tresore dieses Rechners
/// (deployments/<netz>-tresore.json), gegen deren hinterlegte Nachricht eine
/// Tresor-Zahlung geprüft wird.
pub fn inbox_entry_with(tx: &serde_json::Value, my_addr: &str, sk: &SecretKey, ghost: Option<u64>, known: &[TresorRec]) -> Option<Inbox> {
    if tx["is_accepted"] == serde_json::Value::Bool(false) {
        return None;
    }
    let payload = match tx["payload"].as_str() {
        Some(h) if !h.is_empty() => hex_bytes(h)?,
        _ => return None,
    };
    let found = read(sk, &payload)?;
    let empty = vec![];
    let inputs = tx["inputs"].as_array().unwrap_or(&empty);
    let in_addrs: Vec<&str> = inputs.iter().filter_map(|i| i["previous_outpoint_address"].as_str()).collect();
    let me = kaspa_addresses::Address::try_from(my_addr).ok()?;
    let my_x = sk.x_only_public_key(SECP256K1).0.serialize();
    let mut from: Vec<String> = vec![];
    for a in &in_addrs {
        if is_key_address(a) && *a != my_addr && !from.iter().any(|f| f == a) {
            from.push(a.to_string());
        }
    }
    let entry = |amount: u64, unit: &'static str, from: Vec<String>, found: Found, origin: Origin| Inbox {
        txid: tx["transaction_id"].as_str().unwrap_or("").to_string(),
        time_ms: tx["block_time"].as_u64(),
        amount,
        unit,
        from,
        found,
        origin,
        source: Source::Rest,
    };
    // Tresor-Zahlung an mich (auch wenn ich die Gebühr gezahlt habe): Betrag
    // laut Vertrag, der Ausgang an mich kann sonst mein Wechselgeld enthalten
    if ghost.is_none() {
        if let Some(p) = tresor_payment(tx, me.prefix, &my_x, my_addr) {
            // Abgleich mit dem Text der Tresor-Liste; bei `Invalid` mit dem
            // gelesenen, nicht angezeigten Text: Ein Tresor, der vor der
            // Verschärfung des Filters übernommen wurde, darf solche Zeichen
            // enthalten, seine echte Zahlung ist dann nicht „eingefügt“
            // (zweite Nachprüfung, Altdaten)
            let text = match &found {
                Found::Invalid(_) => raw_text(sk, &payload),
                f => f.text().map(str::to_string),
            };
            let (id, owner, check) = check_tresor(&p, &payload, text.as_deref(), known);
            return Some(entry(p.amount as u64, "KAS", from, found, Origin::Tresor { id, owner, check }));
        }
    }
    if in_addrs.contains(&my_addr) {
        return None;
    }
    let (amount, unit) = match ghost {
        Some(g) => (g, "GHOST"),
        None => {
            let outputs = tx["outputs"].as_array().unwrap_or(&empty);
            let sum: u64 = outputs.iter().filter(|o| o["script_public_key_address"].as_str() == Some(my_addr)).filter_map(|o| o["amount"].as_u64()).sum();
            if sum == 0 {
                return None;
            }
            (sum, "KAS")
        }
    };
    // KAS aus einem Vertrag (GHOST liegen ohnehin in Token-Covenants)
    let origin = if ghost.is_none() && in_addrs.iter().any(|a| is_script_address(a)) { Origin::Contract } else { Origin::Direct };
    Some(entry(amount, unit, from, found, origin))
}

// ------------------------------------------------- Abgleich am Node (A12-19) ----

/// Eine Tx, wie der Node sie im Block führt
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeTx {
    pub payload: Vec<u8>,
    /// Signaturskripte der Eingänge (darin das Redeem-Skript eines Tresors)
    pub sigs: Vec<Vec<u8>>,
    /// Ausgänge: Betrag und Skript
    pub outputs: Vec<(u64, Vec<u8>)>,
}

/// Stimmt die Tx der REST-API mit der Fassung des Nodes überein? Verglichen
/// wird alles, was der Eingang auswertet: Payload, Signaturskripte (Tresor-
/// Erkennung) und Ausgänge (Betrag). Ob die Tx angenommen wurde und von
/// welchen Adressen die Eingänge stammen, sagt der Block nicht – das bleibt
/// Angabe der REST-API. Ein- und Ausgänge in der Reihenfolge des Felds `index`.
pub fn same_as_node(rest: &serde_json::Value, node: &NodeTx) -> bool {
    let empty = vec![];
    let payload = match rest["payload"].as_str() {
        Some(h) => hex_bytes(h),
        None => Some(vec![]),
    };
    let sigs: Option<Vec<Vec<u8>>> = in_index_order(rest["inputs"].as_array().unwrap_or(&empty))
        .and_then(|v| v.iter().map(|i| hex_bytes(i["signature_script"].as_str().unwrap_or(""))).collect());
    let outputs: Option<Vec<(u64, Vec<u8>)>> = in_index_order(rest["outputs"].as_array().unwrap_or(&empty))
        .and_then(|v| v.iter().map(|o| Some((o["amount"].as_u64()?, hex_bytes(o["script_public_key"].as_str()?)?))).collect());
    payload.as_ref() == Some(&node.payload) && sigs.as_ref() == Some(&node.sigs) && outputs.as_ref() == Some(&node.outputs)
}

/// Was der Node zu den Blöcken sagt, die die REST-API für eine Tx nennt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeLookup {
    /// die Tx, wie sie in einem dieser Blöcke steht
    Found(NodeTx),
    /// ein genannter Block liegt vollständig am Node, die Tx steht aber in keinem
    NotInBlock,
    /// der Node kennt keinen der genannten Blöcke, nicht einmal den Kopf
    UnknownBlock,
    /// nicht prüfbar: Blockinhalt gelöscht (Pruning), Fehler, Zeitüberschreitung
    Unavailable,
}

/// Aufbewahrung am Node: Zeit des Pruning-Punkts (jüngere Blöcke hat der Node
/// vollständig) und Past Median Time (so weit ist er)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub pruning_ms: u64,
    pub pmt_ms: u64,
}

/// Sicherheitsabstand zu beiden Grenzen der Aufbewahrung
pub const RETENTION_MARGIN_MS: u64 = 3_600_000;

/// Urteil über eine Tx der REST-API nach dem Blick auf den Node:
/// Ok(true) = am Node geprüft, Ok(false) = nicht prüfbar (bleibt „laut
/// REST-API“), Err = ausblenden, mit Grund. Einen Block, dessen Zeit laut
/// REST-API sicher im Aufbewahrungsfenster liegt, muss der Node kennen; nennt
/// die API einen erfundenen Block, fällt das so auf (Nachprüfung A12-19).
pub fn node_verdict(rest: &serde_json::Value, lookup: &NodeLookup, keep: Option<Retention>) -> Result<bool, String> {
    match lookup {
        NodeLookup::Found(n) if same_as_node(rest, n) => Ok(true),
        NodeLookup::Found(_) => Err("Angaben der REST-API weichen vom Block am Node ab".into()),
        NodeLookup::NotInBlock => Err("die REST-API nennt einen Block, in dem die Tx am Node nicht steht".into()),
        NodeLookup::UnknownBlock => match (rest["block_time"].as_u64(), keep) {
            (Some(t), Some(k)) if t > k.pruning_ms + RETENTION_MARGIN_MS && t + RETENTION_MARGIN_MS < k.pmt_ms => {
                Err("die REST-API nennt einen Block, den der Node nicht kennt, obwohl er ihn haben müsste".into())
            }
            _ => Ok(false),
        },
        NodeLookup::Unavailable => Ok(false),
    }
}

// ------------------------------------------------ Ablauf ghostctl messages ----

/// Ein- und Ausgabe von `inbox`: REST-API und Node in ghostctl, Attrappen im Test
#[allow(async_fn_in_trait)]
pub trait InboxIo {
    /// Verlauf der Adresse als REST-JSON, neueste zuerst, höchstens `limit`
    fn address_txs(&mut self, address: &str, limit: usize) -> Result<Vec<serde_json::Value>, String>;
    /// eine Tx als REST-JSON
    fn tx(&mut self, txid: &str) -> Result<serde_json::Value, String>;
    /// mit einem Node verbinden; false = keiner erreichbar
    async fn connect(&mut self) -> bool;
    /// die Tx in den Blöcken, die die REST-API nennt (`block_hash`)
    async fn node_tx(&mut self, rest: &serde_json::Value) -> NodeLookup;
    /// Aufbewahrungsfenster des Nodes, falls abrufbar
    async fn retention(&mut self) -> Option<Retention>;
    /// Zeitrahmen (in Tests kürzer)
    fn budget(&self) -> InboxBudget {
        INBOX_BUDGET
    }
}

/// Höchstzahl der Nachrichten, die `inbox` am Node gegenprüft (je ein Block)
pub const NODE_CHECKS: usize = 30;

/// Zeitrahmen von `inbox`, ab dem Start gerechnet. Die Seite wartet höchstens
/// 180 s auf ghostctl (READ_TIMEOUT_MS in app/server/api.ts).
/// - Jede REST-Abfrage endet nach 20 s (`chain::http`); der Verlauf (200 Tx,
///   4 Seiten) braucht so höchstens 80 s.
/// - GHOST-Tx: neue Abrufe nur bis `ghost`, der letzte endet ≤ 20 s danach.
/// - Node: Verbinden, Aufbewahrungsfenster und alle Abfragen zusammen
///   höchstens `node`, nie über `total` hinaus.
///
/// Zusammen ≤ 80 s + 60 s = 140 s, mit dem Trennen in ghostctl (5 s) 145 s.
/// Vorher war nur die Schleife begrenzt:
/// Verbinden 30 s, Aufbewahrung 2 × 10 s, 60 s und eine letzte Abfrage mit
/// bis zu 3 × 10 s, dazu die REST-Abrufe (zweite Nachprüfung).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxBudget {
    pub ghost: std::time::Duration,
    pub node: std::time::Duration,
    pub total: std::time::Duration,
}
pub const INBOX_BUDGET: InboxBudget = InboxBudget {
    ghost: std::time::Duration::from_secs(60),
    node: std::time::Duration::from_secs(60),
    total: std::time::Duration::from_secs(150),
};

/// Ergebnis von `inbox`
#[derive(Debug, Clone, Default)]
pub struct InboxResult {
    pub address: String,
    /// neueste zuerst
    pub messages: Vec<Inbox>,
    /// GHOST-Eingänge, die sich nicht prüfen ließen
    pub notes: Vec<String>,
    /// alles zur Prüfung der Nachrichten selbst (Node, Tresore)
    pub checks: Vec<String>,
    pub ghost_checked: usize,
    /// ein Node war erreichbar und hat nachgeprüft
    pub node_checked: bool,
    /// ausgeblendet, weil REST-API und Node nicht zusammenpassen
    pub hidden: usize,
}

/// GHOST-Tx (ID, Betrag an mich) einzeln über die REST-API holen und
/// auswerten. Neue Abrufe nur bis `until` (INBOX_BUDGET), der Rest wird als
/// nicht geprüft gemeldet.
fn ghost_receipts(
    io: &mut impl InboxIo,
    todo: Vec<(String, u64)>,
    until: std::time::Instant,
    my_addr: &str,
    sk: &SecretKey,
    known: &[TresorRec],
    out: &mut InboxResult,
) -> Vec<(Inbox, serde_json::Value)> {
    let (n, mut found) = (todo.len(), vec![]);
    for (i, (id, units)) in todo.into_iter().enumerate() {
        if std::time::Instant::now() >= until {
            out.notes.push(format!("{} GHOST-Tx aus Zeitgründen nicht geprüft", n - i));
            break;
        }
        out.ghost_checked += 1;
        match io.tx(&id) {
            Ok(t) => {
                if let Some(e) = inbox_entry_with(&t, my_addr, sk, Some(units), known) {
                    found.push((e, t));
                }
            }
            Err(e) => out.notes.push(format!("GHOST-Tx {}…: {e}", &id[..12])),
        }
    }
    found
}

/// `ghostctl messages`: eingegangene Nachrichten an den Schlüssel `sk`.
/// - KAS: Verlauf der eigenen Adresse über die REST-API.
/// - GHOST: Ein GHOST-Token liegt unter einer Skript-Adresse, die Adresse des
///   Empfängers kommt in der Tx nicht vor. Gefunden werden daher nur Eingänge
///   zu Token-UTXOs, die die Zustandsdatei `state` für diesen Schlüssel kennt.
/// - Tresor-Zahlungen: gegen die hinterlegte Nachricht der Tresore dieses
///   Rechners (`<state>-tresore.json`) geprüft (A12-1).
/// - Ist ein Node erreichbar, wird jede Tx im Block am Node gegengeprüft
///   (A12-19); passt die REST-API nicht dazu, erscheint die Nachricht nicht.
pub async fn inbox(io: &mut impl InboxIo, network: &str, state: &std::path::Path, sk: &SecretKey, limit: usize) -> Result<InboxResult, String> {
    let (start, budget) = (std::time::Instant::now(), io.budget());
    let prefix = kaspa_addresses::Prefix::from(crate::net::network_id(network)?);
    let my_x = sk.x_only_public_key(SECP256K1).0.serialize();
    let my_addr = kaspa_addresses::Address::new(prefix, kaspa_addresses::Version::PubKey, &my_x).to_string();
    let mut out = InboxResult { address: my_addr.clone(), ..Default::default() };
    let known = match crate::tresor::load(&crate::tresor::path_for(state), network) {
        Ok(f) => f.tresore,
        Err(e) => {
            out.checks.push(format!("Tresore nicht lesbar ({e}) – Tresor-Nachrichten nicht mit der hinterlegten Fassung verglichen"));
            vec![]
        }
    };
    let mut found: Vec<(Inbox, serde_json::Value)> = vec![];
    for t in io.address_txs(&my_addr, limit)? {
        if let Some(e) = inbox_entry_with(&t, &my_addr, sk, None, &known) {
            found.push((e, t));
        }
    }
    if let Ok(text) = std::fs::read_to_string(state) {
        match serde_json::from_str::<crate::ops::Deployment>(&text) {
            Ok(d) if d.network == network => {
                // Betrag je Tx: mehrere Token-UTXOs derselben Tx zusammen
                let mut by_tx: Vec<(String, u64)> = vec![];
                for t in d.tokens.iter().filter(|t| t.state.owner == my_x) {
                    let id = t.outpoint.transaction_id.to_string();
                    match by_tx.iter_mut().find(|(i, _)| *i == id) {
                        Some((_, a)) => *a += t.state.amount.max(0) as u64,
                        None => by_tx.push((id, t.state.amount.max(0) as u64)),
                    }
                }
                let todo: Vec<(String, u64)> = by_tx.into_iter().take(50).filter(|(id, _)| !found.iter().any(|(f, _)| f.txid == *id)).collect();
                let more = ghost_receipts(io, todo, start + budget.ghost, &my_addr, sk, &known, &mut out);
                found.extend(more);
            }
            Ok(_) => out.notes.push(format!("Zustandsdatei {} gehört zu einem anderen Netz – GHOST-Eingänge nicht geprüft", state.display())),
            Err(e) => out.notes.push(format!("Zustandsdatei {} unlesbar ({e}) – GHOST-Eingänge nicht geprüft", state.display())),
        }
    }
    found.sort_by_key(|(f, _)| std::cmp::Reverse(f.time_ms.unwrap_or(0)));
    // Abgleich am Node (A12-19): Kaspa-Nodes führen keinen Tx-Index, eine Tx
    // ist nur über ihren Block abrufbar, und das nur, solange der Node ihn hat
    // – alles zusammen im Zeitrahmen, auch eine hängende Abfrage
    if !found.is_empty() {
        let until = (std::time::Instant::now() + budget.node).min(start + budget.total);
        let left = || until.saturating_duration_since(std::time::Instant::now());
        if tokio::time::timeout(left(), io.connect()).await.unwrap_or(false) {
            out.node_checked = true;
            let keep = tokio::time::timeout(left(), io.retention()).await.ok().flatten();
            let (mut kept, mut late) = (vec![], false);
            for (i, (mut f, raw)) in found.into_iter().enumerate() {
                late |= i < NODE_CHECKS && left().is_zero();
                if i < NODE_CHECKS && !late {
                    let lookup = match tokio::time::timeout(left(), io.node_tx(&raw)).await {
                        Ok(l) => l,
                        Err(_) => {
                            late = true;
                            NodeLookup::Unavailable
                        }
                    };
                    match node_verdict(&raw, &lookup, keep) {
                        Ok(true) => f.source = Source::Node,
                        Ok(false) => {}
                        Err(why) => {
                            out.hidden += 1;
                            out.checks.push(format!("Tx {}: {why} – Nachricht nicht angezeigt", f.txid));
                            continue;
                        }
                    }
                }
                kept.push((f, raw));
            }
            if late {
                out.checks.push("Zeitrahmen für den Abgleich am Node erschöpft – weitere Nachrichten nur laut REST-API".into());
            }
            found = kept;
        } else {
            out.checks.push("Kein Node erreichbar – Angaben nur laut REST-API".into());
        }
    }
    out.messages = found.into_iter().map(|(f, _)| f).collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::p2pk_spk;
    use secp256k1::{Keypair, Scalar};

    fn key() -> SecretKey {
        SecretKey::new(&mut rand::thread_rng())
    }
    fn x(sk: &SecretKey) -> [u8; 32] {
        sk.x_only_public_key(SECP256K1).0.serialize()
    }
    /// Schlüssel mit gewünschter Parität der y-Koordinate
    fn key_with(parity: Parity) -> SecretKey {
        loop {
            let k = key();
            if k.x_only_public_key(SECP256K1).1 == parity {
                return k;
            }
        }
    }

    #[test]
    fn hin_und_rueckweg() {
        let sk = key();
        for text in ["Miete Oktober", "Grüße aus Köln – ½ Anteil", "x", "🏠"] {
            let p = encrypt(&x(&sk), text).unwrap();
            assert!(is_encrypted(&p));
            assert_eq!(p.len(), text.len() + OVERHEAD, "Größe = Text + {OVERHEAD} Byte");
            assert_eq!(decrypt(&sk, &p).as_deref(), Some(text));
            assert_eq!(read(&sk, &p), Some(Found::Private(text.into())));
        }
    }

    #[test]
    fn falscher_schluessel_ergibt_none() {
        let (sk, other) = (key(), key());
        let p = encrypt(&x(&sk), "nur für dich").unwrap();
        assert_eq!(decrypt(&other, &p), None);
        assert_eq!(read(&other, &p), Some(Found::Unreadable));
        // auch der negierte Schlüssel (gleiche x-Koordinate) ist derselbe
        // Empfänger – er liest mit, das ist mathematisch dieselbe Adresse
        assert_eq!(decrypt(&sk.negate(), &p).as_deref(), Some("nur für dich"));
    }

    #[test]
    fn manipulation_ergibt_none() {
        let sk = key();
        let p = encrypt(&x(&sk), "Miete Oktober").unwrap();
        // jedes einzelne Byte hinter dem Magic kippen: E, Nonce, Ciphertext, Tag
        for i in MAGIC.len()..p.len() {
            let mut q = p.clone();
            q[i] ^= 0x01;
            assert_eq!(decrypt(&sk, &q), None, "Byte {i} verändert");
        }
        // Magic verändert → kein verschlüsseltes Format mehr
        let mut q = p.clone();
        q[3] = 0x02;
        assert!(!is_encrypted(&q));
        assert_eq!(decrypt(&sk, &q), None);
        // gekürzt, verlängert, nur Kopf
        assert_eq!(decrypt(&sk, &p[..p.len() - 1]), None);
        assert_eq!(decrypt(&sk, &[p.as_slice(), &[0]].concat()), None);
        assert_eq!(decrypt(&sk, &p[..OVERHEAD - 1]), None);
        assert_eq!(decrypt(&sk, &MAGIC), None);
        // E durch einen anderen gültigen Punkt ersetzt
        let mut q = p.clone();
        q[4..36].copy_from_slice(&x(&key()));
        assert_eq!(decrypt(&sk, &q), None);
        // E_x ist kein Punkt der Kurve: einen solchen x-Wert suchen
        let mut bad = [0xffu8; 32];
        while XOnlyPublicKey::from_slice(&bad).is_ok() {
            bad[31] = bad[31].wrapping_sub(1);
        }
        let mut q = p.clone();
        q[4..36].copy_from_slice(&bad);
        assert_eq!(decrypt(&sk, &q), None);
    }

    #[test]
    fn empfaenger_mit_ungerader_y() {
        // Schnorr-Adressen tragen nur x. Der geheime Schlüssel des Empfängers
        // kann zu einem Punkt mit ungerader y gehören; der Absender rechnet
        // mit lift_x(P) = -P. Nur x(·) zählt, daher klappt es trotzdem.
        for parity in [Parity::Odd, Parity::Even] {
            for _ in 0..20 {
                let sk = key_with(parity);
                let p = encrypt(&x(&sk), "Paritätstest").unwrap();
                assert_eq!(decrypt(&sk, &p).as_deref(), Some("Paritätstest"), "{parity:?}");
            }
        }
        // ebenso für einen Ephemeral-Schlüssel mit ungerader y (wird negiert)
        let sk = key_with(Parity::Odd);
        let e = key_with(Parity::Odd);
        let p = encrypt_with(&x(&sk), "E ungerade", e, [7; NONCE_LEN]).unwrap();
        assert_eq!(&p[4..36], &x(&e), "E_x = x(e·G)");
        assert_eq!(decrypt(&sk, &p).as_deref(), Some("E ungerade"));
    }

    #[test]
    fn ecdh_x_unabhaengig_nachgerechnet() {
        // x(e · lift_x(P)) über libsecp256k1-ECDH = x über Punktmultiplikation
        for _ in 0..10 {
            let (sk, e) = (key_with(Parity::Odd), key());
            let lifted = XOnlyPublicKey::from_slice(&x(&sk)).unwrap().public_key(Parity::Even);
            let via_mul = lifted.mul_tweak(SECP256K1, &Scalar::from(e)).unwrap().x_only_public_key().0.serialize();
            assert_eq!(shared_x(&lifted, &e), via_mul);
            // Empfängerseite: sk · E mit dem echten (evtl. ungeraden) sk
            let e_pub = XOnlyPublicKey::from_slice(&x(&e)).unwrap().public_key(Parity::Even);
            assert_eq!(shared_x(&e_pub, &sk), via_mul, "beide Seiten gleich trotz Vorzeichen");
        }
    }

    #[test]
    fn leere_und_lange_texte_und_groesse() {
        let sk = key();
        // leer: kryptografisch möglich (nur Tag), payload_for macht daraus aber keinen Payload
        let p = encrypt(&x(&sk), "").unwrap();
        assert_eq!(p.len(), OVERHEAD);
        assert_eq!(decrypt(&sk, &p).as_deref(), Some(""));
        assert_eq!(read(&sk, &p), Some(Found::Unreadable), "leere Nachricht wird nicht angezeigt");
        assert_eq!(payload_for("   ", false, Some(&x(&sk))).unwrap(), Vec::<u8>::new());
        assert!(payload_for("", true, None).is_err());
        // längste: 100 Zeichen à 4 Byte
        let long = "🏠".repeat(MAX_MESSAGE_CHARS);
        let p = encrypt(&x(&sk), &long).unwrap();
        assert_eq!(p.len(), MAX_ENCRYPTED);
        assert_eq!(MAX_ENCRYPTED, 464);
        assert!(p.len() <= crate::txb::MAX_PAYLOAD, "passt in MAX_PAYLOAD");
        assert_eq!(decrypt(&sk, &p), Some(long));
        assert!(encrypt(&x(&sk), &"x".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
        assert!(encrypt(&x(&sk), "a\nb").is_err());
    }

    #[test]
    fn zufallswerte_je_nachricht_neu() {
        let sk = key();
        let a = encrypt(&x(&sk), "gleich").unwrap();
        let b = encrypt(&x(&sk), "gleich").unwrap();
        assert_ne!(a[4..36], b[4..36], "neuer Ephemeral-Schlüssel");
        assert_ne!(a[36..48], b[36..48], "neuer Nonce");
        assert_ne!(a[48..], b[48..], "anderer Ciphertext");
        // Ciphertext enthält den Klartext nicht
        assert!(!a.windows(6).any(|w| w == b"gleich"));
        let es: std::collections::HashSet<Vec<u8>> = (0..50).map(|_| encrypt(&x(&sk), "z").unwrap()[4..48].to_vec()).collect();
        assert_eq!(es.len(), 50);
    }

    #[test]
    fn klartext_nie_mit_magic() {
        // \x01 ist ein Steuerzeichen: check_message lehnt es ab
        assert!(message_payload("GHM\u{1}hallo").is_err());
        assert!(payload_for("GHM\u{1}hallo", true, None).is_err());
        assert!(!is_encrypted(&message_payload("GHM hallo").unwrap()));
        let sk = key();
        assert_eq!(read(&sk, b"Miete Oktober"), Some(Found::Public("Miete Oktober".into())));
        assert_eq!(read(&sk, &[]), None);
        assert_eq!(read(&sk, &[0x95, 0x78, 0x9c, 0x00]), None, "Binärdaten fremder Anwendungen");
        // Text mit Richtungszeichen: die Zahlung erscheint, der Text nicht
        assert_eq!(read(&sk, "a\u{202e}b".as_bytes()), Some(Found::Invalid(Rejected::BadChars)), "Richtungszeichen");
        // verschlüsselter Steuerzeichen-Text wird nicht angezeigt
        let evil = encrypt_with_unchecked(&x(&sk), "a\nb");
        assert_eq!(decrypt(&sk, &evil).as_deref(), Some("a\nb"));
        assert_eq!(read(&sk, &evil), Some(Found::Invalid(Rejected::BadChars)));
    }

    /// wie encrypt, ohne Textprüfung (ein fremder Absender kann alles schicken)
    fn encrypt_with_unchecked(recipient: &[u8], text: &str) -> Vec<u8> {
        let e = key_with(Parity::Even);
        let e_x = x(&e);
        let p = XOnlyPublicKey::from_slice(recipient).unwrap();
        let k = derive_key(&shared_x(&p.public_key(Parity::Even), &e), &e_x, recipient);
        let mut out = [MAGIC.as_slice(), &e_x].concat();
        let nonce = [1u8; NONCE_LEN];
        let ct = ChaCha20Poly1305::new(Key::from_slice(&k)).encrypt(Nonce::from_slice(&nonce), Payload { msg: text.as_bytes(), aad: &out }).unwrap();
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    fn addr(sk: &SecretKey) -> String {
        kaspa_addresses::Address::new(kaspa_addresses::Prefix::Mainnet, kaspa_addresses::Version::PubKey, &x(sk)).to_string()
    }

    /// Tx im Format der REST-API (Feldnamen wie am 29.09.2026 gemessen)
    fn rest_tx(payload: Option<&[u8]>, from: &[&str], to: &[(&str, u64)], accepted: bool) -> serde_json::Value {
        serde_json::json!({
            "subnetwork_id": "0000000000000000000000000000000000000000",
            "transaction_id": "97b1e7e611ca4f37e6be8cd3a55f9c66d84dc2b788a4a9a253915de555f06e9c",
            "mass": "2069",
            "payload": payload.map(faster_hex::hex_string),
            "block_time": 1790691455495u64,
            "is_accepted": accepted,
            "inputs": from.iter().map(|a| serde_json::json!({ "previous_outpoint_address": a, "previous_outpoint_amount": 5 })).collect::<Vec<_>>(),
            "outputs": to.iter().enumerate().map(|(i, (a, v))| serde_json::json!({ "index": i, "amount": v, "script_public_key_address": a, "script_public_key_type": "pubkey" })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn eingang_auswerten() {
        let (me, alice) = (key(), key());
        let (me_a, alice_a) = (addr(&me), addr(&alice));
        let p2sh = kaspa_addresses::Address::new(kaspa_addresses::Prefix::Mainnet, kaspa_addresses::Version::ScriptHash, &[3u8; 32]).to_string();
        let enc = encrypt(&x(&me), "Miete Oktober").unwrap();
        // verschlüsselt an mich, mit Wechselgeld an Alice
        let t = rest_tx(Some(&enc), &[&alice_a], &[(&me_a, 250_000_000), (&alice_a, 9)], true);
        let e = inbox_entry(&t, &me_a, &me, None).unwrap();
        assert_eq!(e.found, Found::Private("Miete Oktober".into()));
        assert_eq!((e.amount, e.unit), (250_000_000, "KAS"));
        assert_eq!(e.from, vec![alice_a.clone()]);
        assert_eq!(e.time_ms, Some(1790691455495));
        assert_eq!(e.txid.len(), 64);
        // öffentlich
        let t = rest_tx(Some(b"Danke!"), &[&alice_a], &[(&me_a, 1)], true);
        assert_eq!(inbox_entry(&t, &me_a, &me, None).unwrap().found, Found::Public("Danke!".into()));
        // eigene Sendung (Eingang von mir): nicht im Eingang
        let t = rest_tx(Some(&enc), &[&me_a], &[(&alice_a, 1), (&me_a, 7)], true);
        assert_eq!(inbox_entry(&t, &me_a, &me, None), None);
        // nicht angenommen, ohne Payload, ohne Ausgang an mich, fremde Binärdaten
        assert_eq!(inbox_entry(&rest_tx(Some(&enc), &[&alice_a], &[(&me_a, 1)], false), &me_a, &me, None), None);
        assert_eq!(inbox_entry(&rest_tx(None, &[&alice_a], &[(&me_a, 1)], true), &me_a, &me, None), None);
        assert_eq!(inbox_entry(&rest_tx(Some(&enc), &[&alice_a], &[(&alice_a, 1)], true), &me_a, &me, None), None);
        assert_eq!(inbox_entry(&rest_tx(Some(&[0x95, 0x78, 0x9c]), &[&alice_a], &[(&me_a, 1)], true), &me_a, &me, None), None);
        // an jemand anderen verschlüsselt, aber Ausgang an mich: als unlesbar gemeldet
        let other = encrypt(&x(&alice), "geheim").unwrap();
        let t = rest_tx(Some(&other), &[&alice_a], &[(&me_a, 1)], true);
        assert_eq!(inbox_entry(&t, &me_a, &me, None).unwrap().found, Found::Unreadable);
        // GHOST: Betrag aus der Zustandsdatei, P2SH-Eingänge (Token) sind kein Absender
        let t = rest_tx(Some(&enc), &[&p2sh, &alice_a, &alice_a], &[(&p2sh, 100_000_000), (&alice_a, 5)], true);
        let e = inbox_entry(&t, &me_a, &me, Some(5 * 100_000_000)).unwrap();
        assert_eq!((e.amount, e.unit), (500_000_000, "GHOST"));
        assert_eq!(e.from, vec![alice_a]);
    }

    #[test]
    fn empfaenger_aus_skript() {
        let sk = key();
        let kp = Keypair::from_secret_key(SECP256K1, &sk);
        assert_eq!(recipient_of_spk(&p2pk_spk(&x(&sk))), Some(x(&sk)));
        // ECDSA-P2PK (33 Byte + OP_CHECKSIGECDSA)
        let ecdsa = kaspa_txscript::pay_to_address_script(&kaspa_addresses::Address::new(
            kaspa_addresses::Prefix::Mainnet,
            kaspa_addresses::Version::PubKeyECDSA,
            &kp.public_key().serialize(),
        ));
        assert_eq!(recipient_of_spk(&ecdsa), None);
        // P2SH
        let p2sh = kaspa_txscript::pay_to_script_hash_script(&[0x51]);
        assert_eq!(recipient_of_spk(&p2sh), None);
        assert_eq!(payload_for("Miete", false, recipient_of_spk(&p2sh).as_ref().map(|a| a.as_slice())).unwrap_err(), NOT_P2PK);
        // öffentlich geht auch an P2SH
        assert_eq!(payload_for("Miete", true, None).unwrap(), b"Miete");
        // verschlüsselt an P2PK
        let p = payload_for(" Miete ", false, Some(&x(&sk))).unwrap();
        assert_eq!(decrypt(&sk, &p).as_deref(), Some("Miete"));
    }

    /// A12-19: Die Tx der REST-API wird mit dem Block am Node verglichen; jede
    /// Abweichung in Payload, Signaturskripten oder Ausgängen fällt auf
    #[test]
    fn a12_abgleich_mit_dem_node() {
        let me = key();
        let enc = encrypt(&x(&me), "Miete Oktober").unwrap();
        let spk_me = p2pk_spk(&x(&me)).script().to_vec();
        let rest = serde_json::json!({
            "transaction_id": "11".repeat(32),
            "payload": faster_hex::hex_string(&enc),
            "inputs": [ { "signature_script": "4101ab", "previous_outpoint_address": "kaspa:qx" } ],
            "outputs": [ { "amount": 250_000_000u64, "script_public_key": faster_hex::hex_string(&spk_me) } ],
        });
        let node = NodeTx { payload: enc.clone(), sigs: vec![vec![0x41, 0x01, 0xab]], outputs: vec![(250_000_000, spk_me.clone())] };
        assert!(same_as_node(&rest, &node));
        let other = encrypt(&x(&me), "Neue Adresse!").unwrap();
        assert!(!same_as_node(&rest, &NodeTx { payload: other, ..node.clone() }), "anderer Payload");
        assert!(!same_as_node(&rest, &NodeTx { outputs: vec![(1, spk_me.clone())], ..node.clone() }), "anderer Betrag");
        assert!(!same_as_node(&rest, &NodeTx { sigs: vec![vec![0x41, 0x01, 0xac]], ..node.clone() }), "anderes Signaturskript");
        assert!(!same_as_node(&rest, &NodeTx { outputs: vec![], ..node.clone() }), "Ausgang fehlt");
        // leerer Payload: REST liefert null
        let mut r2 = rest.clone();
        r2["payload"] = serde_json::Value::Null;
        assert!(same_as_node(&r2, &NodeTx { payload: vec![], ..node }));
    }

    /// A12-1: KAS aus einem Vertrag, der kein Tresor ist, werden als solche
    /// gekennzeichnet (die Nachricht setzt, wer die Tx gebaut hat); eine
    /// direkte Zahlung bleibt „direkt“ und startet ohne Node-Prüfung
    #[test]
    fn a12_herkunft_vertrag_und_direkt() {
        let (me, alice) = (key(), key());
        let (me_a, alice_a) = (addr(&me), addr(&alice));
        let p2sh = kaspa_addresses::Address::new(kaspa_addresses::Prefix::Mainnet, kaspa_addresses::Version::ScriptHash, &[3u8; 32]).to_string();
        let enc = encrypt(&x(&me), "aus dem Pool").unwrap();
        let t = rest_tx(Some(&enc), &[&p2sh, &alice_a], &[(&me_a, 7)], true);
        let e = inbox_entry(&t, &me_a, &me, None).unwrap();
        assert_eq!(e.origin, Origin::Contract);
        assert_eq!(e.from, vec![alice_a.clone()]);
        assert_eq!(e.source, Source::Rest);
        let t = rest_tx(Some(&enc), &[&alice_a], &[(&me_a, 7)], true);
        assert_eq!(inbox_entry(&t, &me_a, &me, None).unwrap().origin, Origin::Direct);
        // GHOST liegen immer in Token-Covenants: kein Hinweis „Vertrag“
        let t = rest_tx(Some(&enc), &[&p2sh, &alice_a], &[(&p2sh, 1)], true);
        assert_eq!(inbox_entry(&t, &me_a, &me, Some(5)).unwrap().origin, Origin::Direct);
    }

    // ------------------------------------- Audit 12, Nachprüfung (Gruppe a) ----

    /// Nachprüfung zu A12-11: Seit dem strengeren Filter verschwanden
    /// Zahlungen mit solchen Zeichen ganz aus dem Eingang (read → None). Jetzt
    /// erscheinen sie, der Text aber nicht.
    #[test]
    fn a12n_unzulaessige_zeichen_zahlung_bleibt_sichtbar() {
        let (me, alice) = (key(), key());
        let (me_a, alice_a) = (addr(&me), addr(&alice));
        for text in ["Miete\u{2028}Mai", "Platz 1\u{fe0f}\u{20e3}", "a\u{e000}b"] {
            assert!(check_message(text).is_err(), "{text:?} gilt heute als unzulässig");
            let t = rest_tx(Some(text.as_bytes()), &[&alice_a], &[(&me_a, 5)], true);
            let e = inbox_entry(&t, &me_a, &me, None).expect("Zahlung bleibt im Eingang");
            assert_eq!(e.found, Found::Invalid(Rejected::BadChars));
            assert_eq!(e.found.text(), None, "der Text wird nicht gezeigt");
            let enc = encrypt_with_unchecked(&x(&me), text);
            let t = rest_tx(Some(&enc), &[&alice_a], &[(&me_a, 5)], true);
            assert_eq!(inbox_entry(&t, &me_a, &me, None).unwrap().found, Found::Invalid(Rejected::BadChars));
        }
        // Binärdaten fremder Anwendungen und reiner Leerraum bleiben draußen
        assert_eq!(inbox_entry(&rest_tx(Some(&[0xff, 0xfe, 0x00]), &[&alice_a], &[(&me_a, 5)], true), &me_a, &me, None), None);
        assert_eq!(inbox_entry(&rest_tx(Some(b"   "), &[&alice_a], &[(&me_a, 5)], true), &me_a, &me, None), None);
    }

    /// Nachprüfung zu A12-19: Urteil nach dem Blick auf den Node. Neu: Liegt ein
    /// genannter Block vollständig am Node und die Tx steht nicht darin, oder
    /// kennt der Node einen Block nicht, der laut REST-API sicher im
    /// Aufbewahrungsfenster liegt, wird die Nachricht ausgeblendet – vorher
    /// blieb sie „laut REST-API“ sichtbar.
    #[test]
    fn a12n_urteil_am_node() {
        let me = key();
        let spk_me = p2pk_spk(&x(&me)).script().to_vec();
        let enc = encrypt(&x(&me), "Miete").unwrap();
        let day = 86_400_000u64;
        let keep = Retention { pruning_ms: 100 * day, pmt_ms: 102 * day };
        let rest = |time: u64| {
            serde_json::json!({
                "transaction_id": "11".repeat(32),
                "block_time": time,
                "payload": faster_hex::hex_string(&enc),
                "inputs": [ { "signature_script": "41ab" } ],
                "outputs": [ { "amount": 5u64, "script_public_key": faster_hex::hex_string(&spk_me) } ],
            })
        };
        let node = NodeTx { payload: enc.clone(), sigs: vec![vec![0x41, 0xab]], outputs: vec![(5, spk_me.clone())] };
        let r = rest(101 * day);
        assert_eq!(node_verdict(&r, &NodeLookup::Found(node.clone()), Some(keep)), Ok(true));
        assert!(node_verdict(&r, &NodeLookup::Found(NodeTx { outputs: vec![(6, spk_me.clone())], ..node }), Some(keep)).is_err());
        assert!(node_verdict(&r, &NodeLookup::NotInBlock, Some(keep)).unwrap_err().contains("nicht steht"));
        assert!(node_verdict(&r, &NodeLookup::UnknownBlock, Some(keep)).unwrap_err().contains("nicht kennt"));
        assert_eq!(node_verdict(&r, &NodeLookup::Unavailable, Some(keep)), Ok(false), "Inhalt gelöscht oder Fehler: nicht prüfbar");
        // unbekannter Block, aber älter als der Pruning-Punkt, ganz frisch oder ohne Fenster: nicht prüfbar
        assert_eq!(node_verdict(&rest(99 * day), &NodeLookup::UnknownBlock, Some(keep)), Ok(false));
        assert_eq!(node_verdict(&rest(100 * day + RETENTION_MARGIN_MS - 1), &NodeLookup::UnknownBlock, Some(keep)), Ok(false));
        assert_eq!(node_verdict(&rest(102 * day - RETENTION_MARGIN_MS + 1), &NodeLookup::UnknownBlock, Some(keep)), Ok(false), "Node noch nicht so weit");
        assert_eq!(node_verdict(&r, &NodeLookup::UnknownBlock, None), Ok(false));
    }

    /// Attrappe für `inbox`: Verlauf aus einer Liste, Node-Antwort je Tx-ID.
    /// `slow`: dieser Schritt hängt (Node) bzw. dauert (REST) – für den Zeitrahmen.
    struct FakeInbox {
        txs: Vec<serde_json::Value>,
        node: Option<Vec<(String, NodeLookup)>>,
        keep: Option<Retention>,
        asked: usize,
        slow: Option<&'static str>,
        budget: Option<InboxBudget>,
    }
    impl FakeInbox {
        fn new(txs: Vec<serde_json::Value>, node: Option<Vec<(String, NodeLookup)>>, keep: Option<Retention>) -> Self {
            FakeInbox { txs, node, keep, asked: 0, slow: None, budget: None }
        }
        async fn hang(&self, step: &str) {
            if self.slow == Some(step) {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        }
    }
    impl InboxIo for FakeInbox {
        fn address_txs(&mut self, _: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
            if self.slow == Some("history") {
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            Ok(self.txs.iter().take(limit).cloned().collect())
        }
        fn tx(&mut self, _: &str) -> Result<serde_json::Value, String> {
            if self.slow == Some("tx") {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err("nicht im Test".into())
        }
        async fn connect(&mut self) -> bool {
            self.hang("connect").await;
            self.node.is_some()
        }
        async fn node_tx(&mut self, rest: &serde_json::Value) -> NodeLookup {
            self.asked += 1;
            self.hang("node_tx").await;
            let id = rest["transaction_id"].as_str().unwrap_or("");
            self.node.iter().flatten().find(|(t, _)| t == id).map(|(_, l)| l.clone()).unwrap_or(NodeLookup::Unavailable)
        }
        async fn retention(&mut self) -> Option<Retention> {
            self.hang("retention").await;
            self.keep
        }
        fn budget(&self) -> InboxBudget {
            self.budget.unwrap_or(INBOX_BUDGET)
        }
    }

    /// Nachprüfung zu A12-19: Der Abgleich am Node läuft im Ablauf von
    /// `ghostctl messages` (jetzt `inbox`) – vorher stand er nur in ghostctl,
    /// und ein abgeschalteter Abgleich fiel keinem Test auf.
    #[tokio::test]
    async fn a12n_ablauf_eingang_mit_node() {
        let (me, alice) = (key(), key());
        let (me_a, alice_a) = (addr(&me), addr(&alice));
        let spk_me = p2pk_spk(&x(&me)).script().to_vec();
        let day = 86_400_000u64;
        let tx = |n: u8, text: &str| {
            let enc = encrypt(&x(&me), text).unwrap();
            let rest = serde_json::json!({
                "transaction_id": format!("{n:02x}").repeat(32),
                "block_time": 101 * day + n as u64,
                "is_accepted": true,
                "payload": faster_hex::hex_string(&enc),
                "inputs": [ { "previous_outpoint_address": alice_a, "signature_script": "41ab" } ],
                "outputs": [ { "amount": 5u64, "script_public_key": faster_hex::hex_string(&spk_me), "script_public_key_address": me_a } ],
            });
            let node = NodeTx { payload: enc, sigs: vec![vec![0x41, 0xab]], outputs: vec![(5, spk_me.clone())] };
            (rest, node)
        };
        let (t1, n1) = tx(1, "gleich am Node");
        let (t2, mut n2) = tx(2, "REST weicht ab");
        n2.payload = encrypt(&x(&me), "anderer Text").unwrap();
        let (t3, _) = tx(3, "Block ohne diese Tx");
        let (t4, _) = tx(4, "Block gelöscht");
        let (t5, _) = tx(5, "erfundener Block");
        let id = |t: &serde_json::Value| t["transaction_id"].as_str().unwrap().to_string();
        let lookups = vec![
            (id(&t1), NodeLookup::Found(n1)),
            (id(&t2), NodeLookup::Found(n2)),
            (id(&t3), NodeLookup::NotInBlock),
            (id(&t4), NodeLookup::Unavailable),
            (id(&t5), NodeLookup::UnknownBlock),
        ];
        let state = std::env::temp_dir().join("ghost-a12n-kein-deployment").join("mainnet.json");
        let mut io = FakeInbox::new(vec![t1, t2, t3, t4, t5], Some(lookups), Some(Retention { pruning_ms: 100 * day, pmt_ms: 102 * day }));
        let r = inbox(&mut io, "mainnet", &state, &me, 50).await.unwrap();
        assert_eq!(r.address, me_a);
        assert!(r.node_checked);
        assert_eq!(io.asked, 5, "jede Nachricht am Node nachgesehen");
        let shown: Vec<(Option<&str>, Source)> = r.messages.iter().map(|m| (m.found.text(), m.source)).collect();
        assert_eq!(shown, vec![(Some("Block gelöscht"), Source::Rest), (Some("gleich am Node"), Source::Node)], "neueste zuerst, drei ausgeblendet");
        assert_eq!(r.hidden, 3);
        assert_eq!(r.checks.len(), 3, "{:?}", r.checks);
        // ohne Node: alles laut REST-API, mit Hinweis
        let (t1, _) = tx(1, "ohne Node");
        let mut io = FakeInbox::new(vec![t1], None, None);
        let r = inbox(&mut io, "mainnet", &state, &me, 50).await.unwrap();
        assert!(!r.node_checked && io.asked == 0);
        assert_eq!(r.messages[0].source, Source::Rest);
        assert!(r.checks.iter().any(|c| c.contains("Kein Node")));
    }

    // ------------------------------- Audit 12, zweite Nachprüfung (Gruppe a) ----

    /// Tresor an `me`: 10 KAS monatlich, Termin 31.01.2027, jede Zahlung mit `payload`
    fn tresor_an(me: &SecretKey, value: u64, payload: &[u8]) -> (TresorParams, crate::ops::Tracked<crate::contracts::TresorState>) {
        let p = TresorParams {
            owner: x(&key()).to_vec(),
            recipient: x(me).to_vec(),
            amount: 10 * 100_000_000,
            anchor_day: 31,
            period_ms: 0,
            max_fee: crate::tresor::DEFAULT_MAX_FEE,
            payload_hash: crate::contracts::payload_hash(payload),
        };
        let t = crate::ops::Tracked {
            outpoint: kaspa_consensus_core::tx::TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([3; 32]), 1),
            value,
            cov: kaspa_consensus_core::Hash::from_bytes([0xab; 32]),
            state: crate::contracts::TresorState { next_due: 1_801_353_600_000, left: 12 },
        };
        (p, t)
    }

    /// Echte Zahlung `tresor::pay` im Format der REST-API, mit Feld `index` an
    /// Ein- und Ausgängen (wie api.kaspa.org am 29.09.2026)
    fn tresor_zahlung(p: &TresorParams, t: &crate::ops::Tracked<crate::contracts::TresorState>, payload: &[u8], funds: Option<&crate::ops::Funds>) -> serde_json::Value {
        let b = crate::tresor::pay(p, t, payload, funds, &kaspa_consensus_core::config::params::MAINNET_PARAMS).unwrap().built;
        let addr = |s: &ScriptPublicKey| kaspa_txscript::standard::extract_script_pub_key_address(s, kaspa_addresses::Prefix::Mainnet).unwrap().to_string();
        serde_json::json!({
            "transaction_id": b.tx.id().to_string(),
            "block_time": 1_801_353_700_000u64,
            "is_accepted": true,
            "payload": faster_hex::hex_string(&b.tx.payload),
            "inputs": b.tx.inputs.iter().zip(&b.entries).enumerate().map(|(i, (x, e))| serde_json::json!({
                "index": i,
                "previous_outpoint_address": addr(&e.script_public_key),
                "signature_script": faster_hex::hex_string(&x.signature_script),
            })).collect::<Vec<_>>(),
            "outputs": b.tx.outputs.iter().enumerate().map(|(i, o)| serde_json::json!({
                "index": i,
                "amount": o.value,
                "script_public_key": faster_hex::hex_string(o.script_public_key.script()),
                "script_public_key_address": addr(&o.script_public_key),
            })).collect::<Vec<_>>(),
        })
    }

    /// Dieselbe Zahlung mit anderem Payload, wie eine falsche REST-API sie
    /// melden könnte (im Netz lehnt der Vertrag sie ab)
    fn mit_payload(mut tx: serde_json::Value, payload: &[u8]) -> serde_json::Value {
        tx["payload"] = faster_hex::hex_string(payload).into();
        tx
    }

    fn check_of(e: &Inbox) -> Option<TresorCheck> {
        match e.origin {
            Origin::Tresor { check, .. } => Some(check),
            _ => None,
        }
    }

    /// Zweite Nachprüfung (Altdaten): Ein Tresor, der vor der Verschärfung des
    /// Filters übernommen wurde, darf in der Beschreibung heute verbotene
    /// Zeichen haben (Tastenkappe, ZWJ-Emoji, U+2028). Seine echte Zahlung
    /// ergab `Found::Invalid`, der Vergleich mit dem Text schlug fehl, und sie
    /// erschien als „NICHT vom Absender – beim Auslösen eingefügt“.
    #[test]
    fn a12p_altdaten_echte_zahlung_nicht_eingefuegt() {
        let me = key();
        let me_a = addr(&me);
        for (text, public) in [("Miete 1\u{fe0f}\u{20e3}", false), ("Gruß \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}", false), ("Miete\u{2028}Mai", true)] {
            assert!(check_message(text).is_err(), "{text:?} gilt heute als unzulässig");
            let stored = if public { text.as_bytes().to_vec() } else { encrypt_with_unchecked(&x(&me), text) };
            let (p, t) = tresor_an(&me, 50 * 100_000_000, &stored);
            let mut rec = TresorRec::new(p.clone(), t.clone(), text.into(), public, None, "x");
            if !public {
                rec.sealed = faster_hex::hex_string(&stored);
            }
            assert_eq!(rec.payload(), stored);
            let known = std::slice::from_ref(&rec);
            let e = inbox_entry_with(&tresor_zahlung(&p, &t, &stored, None), &me_a, &me, None, known).expect("im Eingang");
            assert_eq!(e.found, Found::Invalid(Rejected::BadChars), "{text:?}: Text nicht angezeigt");
            assert_eq!(check_of(&e), Some(TresorCheck::AsStored), "{text:?}: die hinterlegte Fassung, nicht eingefügt");
            // Gegenprobe: eine andere Nachricht mit solchen Zeichen (nur aus
            // falschen REST-Daten möglich, der Vertrag bindet den Payload) passt
            // nicht zum Hash im Vertrag
            let other = if public { b"Neue Adresse\xe2\x80\xa8kaspa:qz0000".to_vec() } else { encrypt_with_unchecked(&x(&me), "Neue Adresse\u{2028}kaspa:qz0000") };
            let e = inbox_entry_with(&mit_payload(tresor_zahlung(&p, &t, &stored, None), &other), &me_a, &me, None, known).unwrap();
            assert_eq!((check_of(&e), e.found), (Some(TresorCheck::Inserted), Found::Invalid(Rejected::BadChars)));
        }
        // Eine hinterlegte Fassung, deren Text nicht die Beschreibung im Code
        // ist: vom Vertrag erzwungen, aber nicht „wie im Tresor-Code“
        let sealed = encrypt_with_unchecked(&x(&me), "Neue Adresse\u{2028}kaspa:qz0000");
        let (p, t) = tresor_an(&me, 50 * 100_000_000, &sealed);
        let mut rec = TresorRec::new(p.clone(), t.clone(), "Miete Mai".into(), false, None, "x");
        rec.sealed = faster_hex::hex_string(&sealed);
        let e = inbox_entry_with(&tresor_zahlung(&p, &t, &sealed, None), &me_a, &me, None, &[rec]).unwrap();
        assert_eq!(check_of(&e), Some(TresorCheck::Bound));
    }

    /// Zweite Nachprüfung: `same_as_node` verglich Ein- und Ausgänge in
    /// Listenreihenfolge, `output_at` dagegen über das Feld `index`. Liefert
    /// die REST-API eine andere Reihenfolge, hätte der Node-Abgleich echte
    /// Nachrichten als „abweichend“ ausgeblendet.
    #[test]
    fn a12p_node_abgleich_nach_index() {
        let me = key();
        let (spk_me, spk_o) = (p2pk_spk(&x(&me)).script().to_vec(), p2pk_spk(&x(&key())).script().to_vec());
        let enc = encrypt(&x(&me), "Miete").unwrap();
        let node = NodeTx { payload: enc.clone(), sigs: vec![vec![0x41, 1], vec![0x41, 2]], outputs: vec![(5, spk_me.clone()), (7, spk_o.clone())] };
        let rest = |order: [usize; 2], index: bool| {
            let idx = |i: usize| if index { serde_json::json!(i) } else { serde_json::Value::Null };
            serde_json::json!({
                "payload": faster_hex::hex_string(&enc),
                "inputs": order.map(|i| serde_json::json!({ "index": idx(i), "signature_script": faster_hex::hex_string(&node.sigs[i]) })),
                "outputs": order.map(|i| serde_json::json!({ "index": idx(i), "amount": node.outputs[i].0, "script_public_key": faster_hex::hex_string(&node.outputs[i].1) })),
            })
        };
        assert!(same_as_node(&rest([0, 1], true), &node));
        assert!(same_as_node(&rest([1, 0], true), &node), "andere Reihenfolge, Feld index stimmt");
        assert!(same_as_node(&rest([0, 1], false), &node), "ohne index: Listenreihenfolge");
        assert!(!same_as_node(&rest([1, 0], false), &node), "ohne index vertauscht: abweichend");
        // Indizes, die zu keiner Tx passen: doppelt, Lücke, nur teilweise
        let mut r = rest([0, 1], true);
        r["outputs"][1]["index"] = 0.into();
        assert!(!same_as_node(&r, &node), "doppelt");
        let mut r = rest([0, 1], true);
        r["inputs"][1]["index"] = 5.into();
        assert!(!same_as_node(&r, &node), "Lücke");
        let mut r = rest([0, 1], true);
        r["inputs"][1]["index"] = serde_json::Value::Null;
        assert!(!same_as_node(&r, &node), "nur teilweise");
    }

    /// Zweite Nachprüfung: Auch die Tresor-Erkennung nahm die Listenstelle des
    /// Eingangs als Index. Zahlt der Empfänger die Gebühr selbst (zwei
    /// Eingänge) und nennt die REST-API die Eingänge in anderer Reihenfolge,
    /// verschwand die Tresor-Zahlung aus dem Eingang (eigene Sendung).
    #[test]
    fn a12p_tresor_erkennung_nach_index() {
        let me = key();
        let sealed = encrypt(&x(&me), "Miete Mai").unwrap();
        let (p, t) = tresor_an(&me, 11 * 100_000_000 + 500_000, &sealed);
        let kp = secp256k1::Keypair::from_secret_key(SECP256K1, &me);
        let mine = (
            kaspa_consensus_core::tx::TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([9; 32]), 0),
            kaspa_consensus_core::tx::UtxoEntry::new(3 * 100_000_000, p2pk_spk(&x(&me)), 0, false, None),
        );
        let funds = crate::ops::Funds::new(kp, vec![mine]);
        let mut rec = TresorRec::new(p.clone(), t.clone(), "Miete Mai".into(), false, None, "x");
        rec.sealed = faster_hex::hex_string(&sealed);
        let known = std::slice::from_ref(&rec);
        let rest = tresor_zahlung(&p, &t, &rec.payload(), Some(&funds));
        assert_eq!(rest["inputs"].as_array().unwrap().len(), 2, "Gebühr vom eigenen Schlüssel");
        let e = inbox_entry_with(&rest, &addr(&me), &me, None, known).expect("im Eingang");
        assert_eq!((e.amount, check_of(&e)), (10 * 100_000_000, Some(TresorCheck::AsStored)));
        let mut r = rest.clone();
        r["inputs"].as_array_mut().unwrap().reverse();
        let e = inbox_entry_with(&r, &addr(&me), &me, None, known).expect("Eingänge vertauscht, Feld index stimmt: bleibt im Eingang");
        assert_eq!((e.amount, check_of(&e)), (10 * 100_000_000, Some(TresorCheck::AsStored)));
        // Indizes, die zu keiner Tx passen: keine Tresor-Zahlung (eigene Sendung, draußen)
        r["inputs"][0]["index"] = 0.into();
        assert_eq!(inbox_entry_with(&r, &addr(&me), &me, None, known), None);
    }

    /// Zweite Nachprüfung (Rückbau R21 überlebte): Nennt die REST-API für den
    /// Tresor-Eingang eine Adresse, muss sie die P2SH-Adresse des Skripts sein.
    #[test]
    fn a12p_tresor_eingang_mit_fremder_p2sh_adresse() {
        let me = key();
        let (p, t) = tresor_an(&me, 50 * 100_000_000, b"Miete");
        let rec = TresorRec::new(p.clone(), t.clone(), "Miete".into(), true, None, "x");
        let known = std::slice::from_ref(&rec);
        let rest = tresor_zahlung(&p, &t, &rec.payload(), None);
        let me_a = addr(&me);
        assert_eq!(check_of(&inbox_entry_with(&rest, &me_a, &me, None, known).unwrap()), Some(TresorCheck::AsStored));
        // eine andere Skript-Adresse: widersprüchliche Angabe, keine Tresor-Zahlung
        let mut r = rest.clone();
        r["inputs"][0]["previous_outpoint_address"] =
            kaspa_addresses::Address::new(kaspa_addresses::Prefix::Mainnet, kaspa_addresses::Version::ScriptHash, &[3u8; 32]).to_string().into();
        assert_eq!(inbox_entry_with(&r, &me_a, &me, None, known).unwrap().origin, Origin::Contract);
        // ohne Adresse (API nennt sie nicht): nur das Skript zählt
        let mut r = rest.clone();
        r["inputs"][0]["previous_outpoint_address"] = serde_json::Value::Null;
        assert_eq!(check_of(&inbox_entry_with(&r, &me_a, &me, None, known).unwrap()), Some(TresorCheck::AsStored));
    }

    /// Zweite Nachprüfung: `ghostctl messages` konnte länger laufen, als die
    /// Seite wartet (180 s): Verbinden 30 s, Aufbewahrung 2 × 10 s, 60 s
    /// Schleife und eine letzte Abfrage bis 3 × 10 s, dazu die REST-Abrufe.
    /// Jetzt bleibt der ganze Abgleich am Node im Zeitrahmen – auch wenn ein
    /// Schritt hängt –, und die Nachrichten bleiben „laut REST-API“ sichtbar.
    #[tokio::test]
    async fn a12p_eingang_im_zeitrahmen() {
        let me = key();
        let (me_a, alice_a) = (addr(&me), addr(&key()));
        let spk_me = p2pk_spk(&x(&me)).script().to_vec();
        let day = 86_400_000u64;
        let keep = Some(Retention { pruning_ms: 100 * day, pmt_ms: 102 * day });
        let tx = |n: u8| {
            let enc = encrypt(&x(&me), "Miete").unwrap();
            let id = format!("{n:02x}").repeat(32);
            let rest = serde_json::json!({
                "transaction_id": id,
                "block_time": 101 * day + n as u64,
                "is_accepted": true,
                "payload": faster_hex::hex_string(&enc),
                "inputs": [ { "previous_outpoint_address": alice_a, "signature_script": "41ab" } ],
                "outputs": [ { "amount": 5u64, "script_public_key": faster_hex::hex_string(&spk_me), "script_public_key_address": me_a } ],
            });
            (rest, (id, NodeLookup::Found(NodeTx { payload: enc, sigs: vec![vec![0x41, 0xab]], outputs: vec![(5, spk_me.clone())] })))
        };
        let (a, b) = (tx(1), tx(2));
        let state = std::env::temp_dir().join("ghost-a12p-kein-deployment").join("mainnet.json");
        let ms = std::time::Duration::from_millis;
        let short = InboxBudget { ghost: ms(200), node: ms(300), total: ms(5_000) };
        // Gegenprobe: ohne hängenden Schritt beide am Node geprüft
        let mut io = FakeInbox::new(vec![a.0.clone(), b.0.clone()], Some(vec![a.1.clone(), b.1.clone()]), keep);
        io.budget = Some(short);
        let r = inbox(&mut io, "mainnet", &state, &me, 50).await.unwrap();
        assert!(r.messages.iter().all(|m| m.source == Source::Node));
        for step in ["connect", "retention", "node_tx"] {
            let mut io = FakeInbox::new(vec![a.0.clone(), b.0.clone()], Some(vec![a.1.clone(), b.1.clone()]), keep);
            (io.slow, io.budget) = (Some(step), Some(short));
            let t0 = std::time::Instant::now();
            let r = inbox(&mut io, "mainnet", &state, &me, 50).await.unwrap();
            let took = t0.elapsed();
            println!("{step} hängt: fertig nach {took:?}, {:?}", r.checks);
            assert!(took < ms(2_000), "{step}: {took:?} statt höchstens des Zeitrahmens");
            assert_eq!(r.messages.len(), 2, "{step}: nichts ausgeblendet");
            assert!(r.messages.iter().all(|m| m.source == Source::Rest), "{step}: laut REST-API");
            let why = if step == "connect" { "Kein Node" } else { "Zeitrahmen" };
            assert!(r.checks.iter().any(|c| c.starts_with(why)), "{step}: {:?}", r.checks);
        }
        // der Gesamtrahmen gilt ab dem Start: dauert schon der Verlauf zu lange, fragt der Node nichts mehr
        let mut io = FakeInbox::new(vec![a.0.clone(), b.0.clone()], Some(vec![a.1, b.1]), keep);
        (io.slow, io.budget) = (Some("history"), Some(InboxBudget { ghost: ms(100), node: ms(10_000), total: ms(200) }));
        let r = inbox(&mut io, "mainnet", &state, &me, 50).await.unwrap();
        assert_eq!(io.asked, 0);
        assert!(r.messages.iter().all(|m| m.source == Source::Rest));
        assert!(r.checks.iter().any(|c| c.starts_with("Zeitrahmen")), "{:?}", r.checks);
    }

    /// Zweite Nachprüfung: auch die GHOST-Abrufe (bis zu 50, je bis 20 s)
    /// stehen im Zeitrahmen; was nicht mehr drankommt, wird gemeldet
    #[test]
    fn a12p_ghost_abrufe_im_zeitrahmen() {
        let me = key();
        let mut io = FakeInbox::new(vec![], None, None);
        io.slow = Some("tx");
        let todo: Vec<(String, u64)> = (1..=5u8).map(|n| (format!("{n:02x}").repeat(32), 1)).collect();
        let mut out = InboxResult::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(250);
        let found = ghost_receipts(&mut io, todo, until, &addr(&me), &me, &[], &mut out);
        println!("{:?}", out.notes);
        assert!(found.is_empty());
        // Abrufe bei 0, 100 und 200 ms, danach keine neuen (unter Last evtl. einer weniger)
        assert!((2..=3).contains(&out.ghost_checked), "{}", out.ghost_checked);
        let rest = format!("{} GHOST-Tx aus Zeitgründen nicht geprüft", 5 - out.ghost_checked);
        assert!(out.notes.contains(&rest), "{:?}", out.notes);
    }

    // --------------------------------------- Audit 12, Restpunkte (Gruppe a) ----

    /// Restpunkt „Invalid-Text-zu-lang“: `Found::Invalid` entsteht auch bei
    /// Texten über 100 Zeichen, etwa JSON einer anderen Anwendung. ghostctl
    /// und Seite nannten dafür „unzulässige Zeichen“. Jetzt steht der Grund
    /// dabei, öffentlich wie verschlüsselt, auch in einer Zahlung im Eingang.
    #[test]
    fn a13_nicht_angezeigt_mit_dem_richtigen_grund() {
        let (me, alice) = (key(), key());
        let (me_a, alice_a) = (addr(&me), addr(&alice));
        let json = format!("{{\"app\":\"fremd\",\"daten\":\"{}\"}}", "a".repeat(120));
        let long_nl = format!("{}\n{}", "a".repeat(60), "b".repeat(60));
        let cases: Vec<(String, Option<Rejected>)> = vec![
            ("x".repeat(MAX_MESSAGE_CHARS + 1), Some(Rejected::TooLong)),
            (json, Some(Rejected::TooLong)),
            ("🏠".repeat(MAX_MESSAGE_CHARS + 1), Some(Rejected::TooLong)),
            ("a\u{202e}b".into(), Some(Rejected::BadChars)),
            ("{\n  \"app\": \"fremd\"\n}".into(), Some(Rejected::BadChars)),
            (long_nl, Some(Rejected::TooLongAndBadChars)),
            ("🏠".repeat(MAX_MESSAGE_CHARS), None),
            ("Miete Oktober".into(), None),
        ];
        for (text, why) in &cases {
            let t = text.as_str();
            assert_eq!(rejected(t), *why, "{t:?}");
            assert_eq!(rejected(t).is_none(), check_message(t).is_ok(), "{t:?}: dieselbe Regel wie check_message");
            let (public, private) = match why {
                Some(w) => (Found::Invalid(*w), Found::Invalid(*w)),
                None => (Found::Public(t.into()), Found::Private(t.into())),
            };
            assert_eq!(read(&me, t.as_bytes()), Some(public.clone()), "{t:?} öffentlich");
            let enc = encrypt_with_unchecked(&x(&me), t);
            assert_eq!(read(&me, &enc), Some(private.clone()), "{t:?} verschlüsselt");
            let e = inbox_entry(&rest_tx(Some(t.as_bytes()), &[&alice_a], &[(&me_a, 5)], true), &me_a, &me, None).expect("im Eingang");
            assert_eq!(e.found, public, "{t:?} im Eingang");
        }
        // Gleichlauf mit check_message über die gemeinsame Fallsammlung
        let data: serde_json::Value = serde_json::from_str(include_str!("../tests/data/nachrichtenfilter.json")).unwrap();
        for c in data["cases"].as_array().unwrap() {
            let t = c["text"].as_str().unwrap();
            assert_eq!(rejected(t).is_none(), check_message(t).is_ok(), "{}", c["name"]);
            assert_eq!(rejected(t).is_some(), c["ok"] == false, "{}", c["name"]);
        }
    }
}
