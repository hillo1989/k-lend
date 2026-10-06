//! Dauerauftrag mit Tresor (contracts/standing_order.sil): KAS liegen in
//! einem Covenant und werden zu festen Terminen an einen festen Empfänger
//! gezahlt – auch wenn der Rechner des Absenders aus ist. Auslösen darf jeder
//! (meist der Empfänger oder ein GHOST-Agent), aber nur ab dem Termin und nur
//! genau so, wie der Vertrag es vorschreibt.
//!
//! Die Nachricht bindet der Vertrag (Audit 12, A12-1 im Vertrag): Der
//! Parameter `payloadHash` ist der sha256 des Payloads, den jede Zahlung
//! tragen muss – die beim Anlegen hinterlegte Nachricht (öffentlich im
//! Klartext oder die einmal an den Empfänger verschlüsselte Fassung), ohne
//! Nachricht der leere Payload. Wer auslöst, kann sie weder weglassen noch
//! ersetzen; ändern lässt sie sich nur mit einem neuen Tresor.
//!
//! Was der Vertrag NICHT bindet (A12-8): den Rest der Höchstgebühr. Je Zahlung
//! dürfen `amount + maxFee` aus dem Tresor; was davon nicht als Netzgebühr
//! gebraucht wird, darf der Auslöser behalten. ghostctl selbst nimmt nur die
//! nötige Gebühr und lässt den Rest im Tresor.
//!
//! Tresore sind unabhängig von GHOST-Deployments und liegen in
//! `deployments/<netz>-tresore.json` (keine Schlüssel, nur der Pfad der
//! Schlüsseldatei des Absenders, falls hier angelegt).
//!
//! Nachführen ohne REST-API: Der Zustand nach einer Zahlung ist vollständig
//! bestimmt (nächster Termin, eine Zahlung weniger). Aus dem bekannten Zustand
//! lassen sich also alle möglichen Nachfolger berechnen – aber nur für Termine,
//! die schon erreicht sind, denn früher lässt der Vertrag nicht zahlen. Für
//! jeden Kandidaten fragt ghostctl den Node nach der Adresse; die UTXO mit
//! dieser Covenant-ID und genau diesem Skript ist der aktuelle Tresor.
//!
//! Teilen mit dem Empfänger: der „Tresor-Code“ (Parameter, Covenant-ID,
//! Zustand, Outpoint). Eine P2SH-Adresse verrät das Skript erst, wenn die UTXO
//! ausgegeben wird; aus der Covenant-ID allein ließen sich die Parameter eines
//! frischen Tresors also gar nicht bestimmen. Der Code wird vor der Übernahme
//! am Node geprüft (Skript-Hash + Covenant-ID); ein falscher Code findet
//! schlicht keine UTXO.

use crate::chain::Shape;
use crate::contracts::*;
use crate::ops::{Funds, Tracked, genesis_output, p2pk_spk, track};
use crate::standing;
use crate::txb::{Built, Draft, FEE_MARGIN_PERMILLE, In, MIN_CHANGE, Signer, Unlock, build_with_payload, min_fee};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::constants::LOCK_TIME_THRESHOLD;
use kaspa_consensus_core::tx::{CovenantBinding, ScriptPublicKey, TransactionOutpoint, TransactionOutput, UtxoEntry};
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Kleinster Betrag je Zahlung. Jeder neue Ausgang kostet Speichermasse
/// ≈ 10^12 / Wert Gramm (KIP-9); gemessen in tests/tresor_e2e_tests.rs.
pub const MIN_AMOUNT: i64 = 100_000_000; // 1 KAS
/// Kleinste Fortsetzung, die ghostctl beim Auslösen lässt: so viel bleibt nach
/// jeder Zahlung durch ghostctl (Agent, Seite) mindestens im Tresor. Der
/// Vertrag selbst verlangt nur einen Rest > 0 nach Betrag und Höchstgebühr; ein
/// fremder Auslöser kann also auch mit weniger Rest zahlen (A12-8).
pub const MIN_KEEP: i64 = 100_000_000; // 1 KAS
/// Reserve im Startguthaben-Vorschlag (bleibt nach der letzten Zahlung übrig)
pub const RESERVE: i64 = MIN_KEEP;
/// Standard-Höchstgebühr je Zahlung. Gemessen (tests/tresor_e2e_tests.rs):
/// Mindestgebühr einer Zahlung 0,0025 KAS, mit 400 Byte Nachricht 0,0033 KAS
/// (transiente Masse bestimmt die Gebühr) – rund dreifache Reserve für Zeiten
/// mit höheren Gebühren. Den ungenutzten Teil darf ein fremder Auslöser
/// behalten (A12-8), je Zahlung also höchstens diese 0,01 KAS.
pub const DEFAULT_MAX_FEE: i64 = 1_000_000; // 0,01 KAS
/// Obergrenze für die Höchstgebühr, die ghostctl beim Anlegen zulässt
pub const MAX_MAX_FEE: i64 = 10_000_000; // 0,1 KAS
/// Untergrenze für die Höchstgebühr beim Anlegen (A13-tresor-4). Gemessen
/// (tests/tresor_e2e_tests.rs, `a13f_mindest_hoechstgebuehr_…`): eine Zahlung
/// mit dem längsten Payload (MAX_PAYLOAD, 464 Byte) kostet im ungünstigsten
/// Fall (1 KAS Betrag, 1 KAS Fortsetzung, längste Skripte) 359.100 sompi.
/// Darunter wäre der Tresor ohne eigenen Schlüssel des Auslösers nie zahlbar,
/// gälte aber als fällig; 400.000 sompi lassen gut 10 % Aufschlag.
pub const MIN_MAX_FEE: i64 = 400_000; // 0,004 KAS
/// Past Median Time läuft der Uhr etwa 2¼ min hinterher (sim.rs); offline gilt
/// ein Termin erst so viel später als fällig
pub const PMT_LAG_MS: i64 = 180_000;
/// Höchstzahl nachgerechneter Termine beim Nachführen
pub const MAX_FOLLOW: usize = 2_000;
/// Einträge im Verlauf je Tresor
pub const MAX_HISTORY: usize = 50;
/// Beim Abgleich nicht gefunden („missing“): die Automatik sieht in diesem
/// Abstand erneut nach, eine Woche lang (ein kurzer Aussetzer des Nodes soll
/// einen Tresor nicht für immer stilllegen; A12-16). Danach nur noch der
/// Abgleich von Hand (`tresor sync`, „Aktualisieren“).
pub const MISSING_RECHECK_MS: i64 = 3_600_000;
pub const MISSING_RECHECK_FOR_MS: i64 = 7 * standing::DAY_MS;
/// Beim automatischen erneuten Nachsehen sucht ghostctl nur so viele Zustände
/// ab dem letzten bekannten; die volle Suche (MAX_FOLLOW) läuft beim ersten
/// Fehlen und beim Abgleich von Hand. Sonst kostete ein gekündigter Tresor mit
/// altem Termin eine Woche lang stündlich MAX_FOLLOW Node-Abfragen (A12-16).
pub const MISSING_RECHECK_FOLLOW: usize = 32;
/// Import: höchstens so viele erreichte, aber nicht gezahlte Termine
/// (Rückstand, gut ein Jahr täglicher Zahlungen). Ein Code mit Termin 1985 und
/// täglichem Intervall hätte sonst MAX_FOLLOW Kandidaten je Abgleich (A12-16).
pub const MAX_IMPORT_BACKLOG: usize = 400;
/// Wartezeit der Automatik nach einem Fehlschlag
pub const RETRY_AFTER_ERROR_MS: i64 = 15 * 60_000;
/// Präfix des teilbaren Tresor-Codes. Version 2: Vertrag mit gebundener
/// Nachricht (`payloadHash`); Codes der Version 1 gehören zum alten Vertrag
/// und werden abgelehnt (im Mainnet gab es noch keine Tresore).
pub const CODE_PREFIX: &str = "ghost-tresor:2:";
/// Präfix der alten Codes (Vertrag ohne gebundene Nachricht)
pub const OLD_CODE_PREFIX: &str = "ghost-tresor:1:";
/// Über die Browser-Wallet (öffentliche Seite) höchstens so viele laufende
/// Tresore je Besitzer in der Tresor-Datei des Servers (wallet_ops). Audit 19
/// A19-3: vorher 20. Zehn reichen für Miete, Abos und Sparpläne einer Person;
/// wer die Datei füllen will, braucht so mindestens 100 Adressen.
pub const MAX_WALLET_PER_OWNER: usize = 10;
/// Höchstzahl der Tresore in der Tresor-Datei, die den Agenten bald etwas
/// kosten (`TresorRec::busy`: laufend, zahlbar, innerhalb BUSY_HORIZON_MS
/// fällig); ab hier lehnt ein Anlegen über die Browser-Wallet ab. Ab dieser
/// Zahl von Einträgen fallen außerdem beendete Wallet-Tresore heraus.
/// Jeder belegte Platz kostet den Agenten Abfragen am Node und eine Sendung,
/// sobald er fällig ist; die Datei soll sich nicht beliebig füllen lassen.
pub const MAX_FILE_TRESORE: usize = 1_000;
/// Belegt zählt ein Tresor nur, wenn sein nächster Termin höchstens so weit
/// voraus liegt (A19-3). 32 Tage: monatliche Tresore zählen immer, jährliche
/// nur im Monat vor dem Termin. Tresore mit Termin 2199 belegten vorher
/// dauerhaft einen Platz, ohne je fällig zu werden. Wer die 1 000 Plätze
/// sperren will, muss jetzt 1 000 Tresore mit je mindestens Betrag +
/// Höchstgebühr + 1 KAS (≈ 2 KAS) bereithalten, die jeden Monat zahlen –
/// an wen auch immer, aber jede Zahlung kostet ihn die Netzgebühr und
/// danach ein Auffüllen, sonst ist der Tresor leer und zählt nicht mehr.
pub const BUSY_HORIZON_MS: i64 = 32 * standing::DAY_MS;
/// Höchstzahl aller Einträge (auch ruhende und leere), gegen eine Datei, die
/// beim Laden jeder Runde Megabytes groß wird: 3 000 Einträge mit kurzem
/// Verlauf sind etwa 6 MB. Ruhende Wallet-Tresore nach dem Muster aus A19-3
/// zu füllen, kostet so mindestens 3 000 × 2 KAS ≈ 6 000 KAS gebunden und
/// 300 Adressen (MAX_WALLET_PER_OWNER).
pub const MAX_FILE_ALL: usize = 3_000;
/// Erster Termin eines Wallet-Tresors höchstens so weit nach der Past Median
/// Time (A19-1/A19-3). Ein Jahr deckt jährliche Zahlungen ab; ein Termin
/// 2199 hält keinen Platz mehr bis dahin fest.
pub const MAX_FIRST_DUE_AHEAD_MS: i64 = 366 * standing::DAY_MS;
/// Suche am Node für die öffentliche Seite (Auffüllen und Kündigen über die
/// Browser-Wallet, A19-2): höchstens so viele Zustände ab dem bekannten. Der
/// Agent führt die Datei bei jeder fälligen Zahlung nach; mehr als 64
/// Zahlungen, die ein anderer ausgelöst hat, ohne dass der Agent sie sah,
/// kommen praktisch nicht vor. Vorher bis MAX_FOLLOW (2 000) je Anfrage.
pub const PUBLIC_FOLLOW: usize = 64;

// ------------------------------------------------------------ Terminlogik ----

/// Zustand nach einer Zahlung (wie pay() im Vertrag)
pub fn next_state(p: &TresorParams, s: &TresorState) -> TresorState {
    TresorState { next_due: standing::following(s.next_due, p.anchor_day, p.period_ms), left: if s.left > 0 { s.left - 1 } else { s.left } }
}

/// Parameter und Startzustand prüfen (Regeln des Vertrags plus Mindestwerte
/// gegen die Speichermasse)
pub fn check_params(p: &TresorParams, first: &TresorState) -> Result<(), String> {
    for (x, what) in [(&p.owner, "Absender"), (&p.recipient, "Empfänger")] {
        secp256k1::XOnlyPublicKey::from_slice(x).map_err(|_| format!("{what}: kein gültiger Schnorr-Schlüssel"))?;
    }
    if p.owner == p.recipient {
        return Err("Empfänger ist der Absender selbst".into());
    }
    if p.payload_hash.len() != 32 {
        return Err("Nachricht: Hash des Payloads muss 32 Byte lang sein".into());
    }
    if p.amount < MIN_AMOUNT {
        return Err(format!("Betrag je Zahlung: mindestens {} KAS (kleinere Ausgänge sind zu schwer für einen Block)", MIN_AMOUNT / 100_000_000));
    }
    if p.max_fee < MIN_MAX_FEE || p.max_fee > MAX_MAX_FEE {
        return Err(format!(
            "Höchstgebühr: {} bis {} KAS (darunter trägt der Tresor die Netzgebühr einer Zahlung nicht sicher)",
            MIN_MAX_FEE as f64 / 1e8,
            MAX_MAX_FEE as f64 / 1e8
        ));
    }
    match (p.anchor_day, p.period_ms) {
        (1..=31, 0) => {}
        (0, per) if (standing::DAY_MS..=3650 * standing::DAY_MS).contains(&per) => {}
        _ => return Err("Intervall: monatlich (Tag 1–31) oder 1 bis 3650 Tage".into()),
    }
    if first.next_due < LOCK_TIME_THRESHOLD as i64 || first.next_due > standing::MAX_TIME {
        return Err("Erster Termin: zwischen 1985 und 2200".into());
    }
    if p.anchor_day > 0 {
        // der erste Termin selbst liegt auf dem Ankertag (oder dem Monatsletzten)
        let d = chrono::DateTime::from_timestamp_millis(first.next_due).ok_or("Erster Termin ungültig")?;
        use chrono::Datelike;
        if d.day() as i64 != p.anchor_day {
            return Err("Monatlich: der erste Termin muss auf dem gewählten Kalendertag liegen".into());
        }
    }
    if first.left == 0 || first.left < -1 {
        return Err("Anzahl: mindestens 1 (oder unbegrenzt)".into());
    }
    Ok(())
}

/// Erster Termin beim Anlegen (`tresor open`, Wallet): nicht mehr als einen
/// Tag vor der Past Median Time `pmt`. Der Vertrag selbst erlaubt Termine ab
/// 1985; ein alter Termin mit kurzem Intervall wäre ein Rückstand, den der
/// Agent Runde für Runde nachzahlt (A19-1).
pub fn check_first_due(first: i64, pmt: i64) -> Result<(), String> {
    if first < pmt - standing::DAY_MS {
        return Err(format!("Erster Termin {} liegt in der Vergangenheit", fmt_time(first)));
    }
    Ok(())
}

/// Erster Termin eines Wallet-Tresors: nicht in der Vergangenheit und
/// höchstens MAX_FIRST_DUE_AHEAD_MS voraus. Gilt in `wallet_ops::run_tresor`,
/// also für build UND den Neubau in submit – die Aktion stammt dort aus dem
/// Plan des Browsers (Audit 19 A19-1).
pub fn check_wallet_first_due(first: i64, pmt: i64) -> Result<(), String> {
    check_first_due(first, pmt)?;
    if first > pmt + MAX_FIRST_DUE_AHEAD_MS {
        return Err(format!("Erster Termin {}: höchstens ein Jahr im Voraus", fmt_time(first)));
    }
    Ok(())
}

/// Startguthaben-Vorschlag: n × (Betrag + Höchstgebühr) + Reserve; bei
/// unbegrenzt None (frei wählbar)
pub fn suggested_fund(p: &TresorParams, left: i64) -> Option<i64> {
    (left > 0).then(|| left * (p.amount + p.max_fee) + RESERVE)
}

/// Wie viele Zahlungen das Guthaben ohne eigene Gebühr des Auslösers noch trägt
pub fn payments_covered(p: &TresorParams, value: u64) -> i64 {
    ((value as i64 - MIN_KEEP) / (p.amount + p.max_fee)).max(0)
}

/// Trägt der Tresor die nächste Zahlung samt Höchstgebühr selbst (wie `pay`
/// ohne eigenen Schlüssel: danach bleibt mindestens MIN_KEEP)?
pub fn fee_from_tresor(p: &TresorParams, value: u64) -> bool {
    value as i64 - p.amount - p.max_fee >= MIN_KEEP
}

/// Lässt sich die nächste Zahlung überhaupt auslösen? Ohne eigenen Schlüssel
/// nur, wenn der Tresor die Gebühr trägt; mit Schlüssel zahlt der Auslöser
/// sie, der Tresor braucht dann nur Betrag + MIN_KEEP (wie `pay`).
pub fn payable(p: &TresorParams, value: u64, with_key: bool) -> bool {
    if with_key { value as i64 - p.amount >= MIN_KEEP } else { fee_from_tresor(p, value) }
}

/// Kann jetzt (Past Median Time `pmt_ms`) gezahlt werden? Err = Grund.
/// `with_key`: der Auslöser kann die Gebühr selbst zahlen.
pub fn due_now(p: &TresorParams, t: &Tracked<TresorState>, pmt_ms: i64, with_key: bool) -> Result<(), String> {
    if t.state.left == 0 {
        return Err("alle Zahlungen erledigt – der Absender kann den Rest mit Kündigen abholen".into());
    }
    if t.state.next_due >= pmt_ms {
        return Err(format!("nächster Termin {} ist noch nicht erreicht", fmt_time(t.state.next_due)));
    }
    if !payable(p, t.value, true) {
        return Err(format!("Guthaben reicht nicht mehr für eine Zahlung und {} KAS Rest – der Absender muss auffüllen", MIN_KEEP / 100_000_000));
    }
    if !payable(p, t.value, with_key) {
        return Err(format!(
            "der Tresor trägt die Netzgebühr nicht mehr (es blieben weniger als {} KAS) – auslösen nur mit eigenem Schlüssel, der die Gebühr zahlt",
            MIN_KEEP / 100_000_000
        ));
    }
    Ok(())
}

/// Intervall als Text: „monatlich am 1.“, „wöchentlich“, „alle 14 Tage“ …
pub fn interval_text(p: &TresorParams) -> String {
    let day = standing::DAY_MS;
    match (p.anchor_day, p.period_ms) {
        (d, _) if d > 0 => format!("monatlich am {d}."),
        (_, x) if x == day => "täglich".into(),
        (_, x) if x == 7 * day => "wöchentlich".into(),
        (_, x) if x % day == 0 => format!("alle {} Tage", x / day),
        (_, x) => format!("alle {} min", x / 60_000),
    }
}

/// Unix-ms → "2027-01-01 08:00 UTC"
pub fn fmt_time(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_else(|| ms.to_string())
}

// ----------------------------------------------------- Skripte und Formen ----

/// Zustandsblock im Bytecode: nextDue, left je 0x08 + 8 Byte (wie das Orakel)
pub fn state_bytes(s: &TresorState) -> Vec<u8> {
    let mut out = vec![];
    for v in [s.next_due, s.left] {
        out.push(0x08);
        out.extend_from_slice(&script_num8(v));
    }
    out
}

/// Form (Bytecode ohne Zustand) eines Tresors; einmal kompiliert, danach
/// Skripte zu beliebigen Zuständen in Mikrosekunden
pub struct TresorShape(Shape);

impl TresorShape {
    pub fn of(p: &TresorParams) -> Self {
        Self(Shape::of(&standing_order(p, &TresorState { next_due: LOCK_TIME_THRESHOLD as i64, left: -1 })))
    }
    pub fn spk(&self, s: &TresorState) -> ScriptPublicKey {
        ScriptPublicKey::new(0, self.0.spk(&state_bytes(s)).into())
    }
}

/// Tresor-Automatik (`tresor pay` ohne ID): nach einem Sendefehler mit dem
/// nächsten Tresor weitermachen? Ja – meist hat ein anderer Auslöser schneller
/// gezahlt –, außer das Journal ist noch offen (die Tx ist vielleicht im Netz;
/// weitere Sendungen erst nach dessen Klärung) oder der Nutzer hat abgebrochen.
pub fn continue_after_send_error(err: &str, journal_open: bool) -> bool {
    err != "abgebrochen" && !journal_open
}

/// Redeem-Skript eines P2SH-Eingangs: letzter Push des Signaturskripts (das
/// nur aus Pushes besteht); None bei anderem Aufbau
pub fn redeem_script(sig: &[u8]) -> Option<Vec<u8>> {
    let mut last = None;
    let mut i = 0usize;
    while i < sig.len() {
        let op = sig[i];
        i += 1;
        let len = match op {
            0x00 | 0x4f | 0x51..=0x60 => {
                last = None;
                continue;
            }
            0x01..=0x4b => op as usize,
            0x4c => {
                i += 1;
                *sig.get(i - 1)? as usize
            }
            0x4d => {
                i += 2;
                u16::from_le_bytes([*sig.get(i - 2)?, *sig.get(i - 1)?]) as usize
            }
            0x4e => {
                i += 4;
                u32::from_le_bytes([*sig.get(i - 4)?, *sig.get(i - 3)?, *sig.get(i - 2)?, *sig.get(i - 1)?]) as usize
            }
            _ => return None,
        };
        last = Some(sig.get(i..i.checked_add(len)?)?.to_vec());
        i += len;
    }
    last
}

/// Ein Befehl im Skript: Opcode und bei Pushes die Daten
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tok {
    op: u8,
    data: Vec<u8>,
}

fn tokens(code: &[u8]) -> Option<Vec<Tok>> {
    let mut out = vec![];
    let mut i = 0usize;
    while i < code.len() {
        let op = code[i];
        i += 1;
        let len = match op {
            0x01..=0x4b => op as usize,
            0x4c => {
                i += 1;
                *code.get(i - 1)? as usize
            }
            0x4d => {
                i += 2;
                u16::from_le_bytes([*code.get(i - 2)?, *code.get(i - 1)?]) as usize
            }
            0x4e => {
                i += 4;
                u32::from_le_bytes([*code.get(i - 4)?, *code.get(i - 3)?, *code.get(i - 2)?, *code.get(i - 1)?]) as usize
            }
            _ => 0,
        };
        out.push(Tok { op, data: code.get(i..i.checked_add(len)?)?.to_vec() });
        i += len;
    }
    Some(out)
}

/// Zahl in Skript-Kodierung: little-endian, Vorzeichen im höchsten Bit
fn script_num(b: &[u8]) -> Option<i64> {
    if b.len() > 8 {
        return None;
    }
    let mut a = [0u8; 8];
    a[..b.len()].copy_from_slice(b);
    let neg = b.last().is_some_and(|x| x & 0x80 != 0);
    if neg {
        a[b.len() - 1] &= 0x7f;
    }
    let v = i64::from_le_bytes(a);
    Some(if neg { -v } else { v })
}

/// Zahl eines Push-Befehls (OP_0, OP_1NEGATE, OP_1–OP_16 oder Datenpush)
fn tok_num(t: &Tok) -> Option<i64> {
    match t.op {
        0x00 => Some(0),
        0x4f => Some(-1),
        0x51..=0x60 => Some((t.op - 0x50) as i64),
        0x01..=0x4e => script_num(&t.data),
        _ => None,
    }
}

/// Bauplan eines standing_order-Skripts: Vergleichsrumpf und die Befehle, die
/// Parameter tragen (0 owner, 1 recipient, 2 payloadHash, 3 amount,
/// 4 anchorDay, 5 periodMs, 6 maxFee) oder von der Skriptlänge abhängen
/// (LOOSE: Längen und Abstände, die sich mit der Länge der Parameter-Pushes
/// verschieben)
struct Layout {
    prefix: Vec<u8>,
    body: Vec<Tok>,
    slots: Vec<(usize, usize)>,
}

/// Parameter mit 32-Byte-Push (owner, recipient, payloadHash)
const BYTE_SLOTS: usize = 3;
/// Zahl-Parameter (amount, anchorDay, periodMs, maxFee)
const INT_SLOTS: usize = 4;
/// Kennung der längenabhängigen Stellen im Bauplan
const LOOSE: usize = BYTE_SLOTS + INT_SLOTS;

/// Aus zwei Übersetzungen mit verschiedenen Parametern: wo stehen sie im Rumpf?
/// None, falls der Compiler Parameter anders als je einen Push einsetzt – dann
/// erkennt `parse_script` nichts (sicher, nur ohne Erkennung).
fn layout() -> Option<&'static Layout> {
    static L: std::sync::OnceLock<Option<Layout>> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let s = TresorState { next_due: LOCK_TIME_THRESHOLD as i64, left: -1 };
        let p1 = TresorParams {
            owner: vec![0x11; 32],
            recipient: vec![0x22; 32],
            amount: 1_234_567_891,
            anchor_day: 23,
            period_ms: 987_654_321,
            max_fee: 7_654_321,
            payload_hash: vec![0x55; 32],
        };
        let p2 = TresorParams {
            owner: vec![0x33; 32],
            recipient: vec![0x44; 32],
            amount: 2_345_678_912,
            anchor_day: 29,
            period_ms: 876_543_219,
            max_fee: 6_543_217,
            payload_hash: vec![0x66; 32],
        };
        let (t1, t2) = (Template::of(&standing_order(&p1, &s)), Template::of(&standing_order(&p2, &s)));
        let (a, b) = (tokens(&t1.suffix)?, tokens(&t2.suffix)?);
        if t1.prefix != t2.prefix || a.len() != b.len() {
            return None;
        }
        let ints = |p: &TresorParams| [p.amount, p.anchor_day, p.period_ms, p.max_fee];
        let mut slots = vec![];
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            if x == y {
                continue;
            }
            let bytes = |p: &TresorParams| [p.owner.clone(), p.recipient.clone(), p.payload_hash.clone()];
            let which = match (0..BYTE_SLOTS).find(|&k| x.data == bytes(&p1)[k] && y.data == bytes(&p2)[k]) {
                Some(k) => k,
                None => {
                    let (n1, n2) = (tok_num(x)?, tok_num(y)?);
                    (0..INT_SLOTS).find(|&k| ints(&p1)[k] == n1 && ints(&p2)[k] == n2).map_or(LOOSE, |k| k + BYTE_SLOTS)
                }
            };
            slots.push((i, which));
        }
        (0..LOOSE).all(|k| slots.iter().any(|&(_, w)| w == k)).then_some(Layout { prefix: t1.prefix, body: a, slots })
    })
    .as_ref()
}

/// Ist `code` (Redeem-Skript) ein standing_order-Tresor? Dann Parameter und
/// Zustand. Gelesen aus den Befehlen und zur Sicherheit neu übersetzt und
/// byte-genau verglichen: ein fremdes Skript mit ähnlichem Aufbau gilt nie als
/// Tresor. Für den Eingang (A12-1), auch bei Tresoren, die dieser Rechner nicht kennt.
pub fn parse_script(code: &[u8]) -> Option<(TresorParams, TresorState)> {
    const STATE_LEN: usize = 18; // state_bytes: 2 × (0x08 + 8 Byte)
    if code.len() < 400 || code.len() > 2_000 {
        return None; // Signaturen und andere Verträge schnell aussortieren
    }
    let l = layout()?;
    let rest = code.strip_prefix(l.prefix.as_slice())?;
    if rest.len() < STATE_LEN || rest[0] != 0x08 || rest[9] != 0x08 {
        return None;
    }
    let state = TresorState { next_due: script_num(&rest[1..9])?, left: script_num(&rest[10..18])? };
    let body = tokens(&rest[STATE_LEN..])?;
    if body.len() != l.body.len() {
        return None;
    }
    let mut keys: [Option<Vec<u8>>; BYTE_SLOTS] = [None, None, None];
    let mut ints: [Option<i64>; INT_SLOTS] = [None; INT_SLOTS];
    for (i, (t, r)) in body.iter().zip(&l.body).enumerate() {
        match l.slots.iter().find(|(j, _)| *j == i) {
            None if t != r => return None,
            None => {}
            Some(&(_, LOOSE)) => tok_num(t).map(|_| ())?, // längenabhängig: prüft die Neuübersetzung
            Some(&(_, w)) if w < BYTE_SLOTS => {
                if t.op != 0x20 || keys[w].as_ref().is_some_and(|k| *k != t.data) {
                    return None;
                }
                keys[w] = Some(t.data.clone());
            }
            Some(&(_, w)) => {
                let n = tok_num(t)?;
                if ints[w - BYTE_SLOTS].is_some_and(|x| x != n) {
                    return None;
                }
                ints[w - BYTE_SLOTS] = Some(n);
            }
        }
    }
    let [owner, recipient, payload_hash] = keys;
    let [amount, anchor_day, period_ms, max_fee] = ints;
    let p = TresorParams {
        owner: owner?,
        recipient: recipient?,
        amount: amount?,
        anchor_day: anchor_day?,
        period_ms: period_ms?,
        max_fee: max_fee?,
        payload_hash: payload_hash?,
    };
    // Gegenprobe über die Form (je Parametersatz einmal übersetzt)
    static SHAPES: std::sync::Mutex<Vec<(TresorParams, TresorShape)>> = std::sync::Mutex::new(vec![]);
    let mut shapes = SHAPES.lock().unwrap_or_else(|e| e.into_inner());
    if !shapes.iter().any(|(q, _)| *q == p) {
        if shapes.len() >= 32 {
            shapes.remove(0);
        }
        shapes.push((p.clone(), TresorShape::of(&p)));
    }
    let (_, shape) = shapes.iter().find(|(q, _)| *q == p)?;
    (shape.0.code(&state_bytes(&state)) == code).then_some((p, state))
}

/// Selektor des Zweigs `pay` im Signaturskript (Push des Dispatch-Tags, ohne
/// Argumente); hängt nicht von den Parametern ab
fn pay_selector() -> Option<&'static [u8]> {
    static SEL: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    SEL.get_or_init(|| {
        let art = standing_order(&layout_sample(), &TresorState { next_due: LOCK_TIME_THRESHOLD as i64, left: -1 });
        silverscript_abi::encode_contract_entry_sig_script(&art, &contract_name(&art), "pay", &[]).ok()
    })
    .as_deref()
}

fn layout_sample() -> TresorParams {
    TresorParams { owner: vec![0x11; 32], recipient: vec![0x22; 32], amount: MIN_AMOUNT, anchor_day: 1, period_ms: 0, max_fee: DEFAULT_MAX_FEE, payload_hash: payload_hash(&[]) }
}

/// Tresor-Eingang, der den Zweig `pay` nimmt: das Signaturskript ist genau der
/// Selektor von `pay` und das Redeem-Skript, wie `tresor::pay` es baut. Dann
/// Parameter und Zustand, sonst None – auch bei `cancel` und `topUp`: die
/// laufen mit der Signatur des Besitzers durch dasselbe Redeem-Skript und
/// dürfen beliebige Ausgänge haben, eine Zahlung laut Vertrag sind sie nicht
/// (Nachprüfung A12-1). Ein anders kodiertes Signaturskript für `pay` gilt
/// ebenfalls nicht als Tresor-Zahlung (sicher, nur ohne Erkennung).
pub fn pay_input(sig: &[u8]) -> Option<(TresorParams, TresorState)> {
    let code = redeem_script(sig)?;
    let found = parse_script(&code)?;
    let push = kaspa_txscript::script_builder::ScriptBuilder::with_flags(crate::txb::engine_flags()).add_data(&code).ok()?.drain();
    (sig == [pay_selector()?, push.as_slice()].concat()).then_some(found)
}

/// Mögliche Zustände ab `from`, in der Reihenfolge der Zahlungen: nur solche,
/// deren Vorgänger bis `pmt_ms` (+ Sicherheitsabstand) fällig war
pub fn candidates(p: &TresorParams, from: &TresorState, pmt_ms: i64) -> Vec<TresorState> {
    candidates_max(p, from, pmt_ms, MAX_FOLLOW)
}

/// Wie `candidates`, höchstens `max` Zustände
pub fn candidates_max(p: &TresorParams, from: &TresorState, pmt_ms: i64, max: usize) -> Vec<TresorState> {
    let mut out = vec![*from];
    let mut s = *from;
    while out.len() < max.min(MAX_FOLLOW) && s.left != 0 && s.next_due < pmt_ms + PMT_LAG_MS {
        let n = next_state(p, &s);
        if n.next_due <= s.next_due || n.next_due > standing::MAX_TIME {
            break;
        }
        out.push(n);
        s = n;
    }
    out
}

/// Treffer zu einem Kandidaten: genau eine UTXO mit dieser Covenant-ID und
/// genau diesem Skript
fn pick(hits: Vec<(TransactionOutpoint, UtxoEntry)>, t: &Tracked<TresorState>, spk: &ScriptPublicKey, s: TresorState) -> Result<Option<Tracked<TresorState>>, String> {
    let hits: Vec<_> = hits.into_iter().filter(|(_, e)| e.covenant_id == Some(t.cov) && e.script_public_key == *spk).collect();
    match hits.as_slice() {
        [] => Ok(None),
        [(op, e)] => Ok(Some(Tracked { outpoint: *op, value: e.amount, cov: t.cov, state: s })),
        _ => Err("mehrere UTXOs derselben Tresor-Covenant – das lässt der Vertrag nicht zu".into()),
    }
}

/// Aktuelle UTXO eines Tresors finden. `lookup` liefert die UTXOs zu einem
/// Skript (Node oder Simulator). Ok(None) = keine UTXO mehr (gekündigt).
pub fn locate(
    p: &TresorParams,
    t: &Tracked<TresorState>,
    pmt_ms: i64,
    mut lookup: impl FnMut(&ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String>,
) -> Result<Option<Tracked<TresorState>>, String> {
    let shape = TresorShape::of(p);
    for s in candidates(p, &t.state, pmt_ms) {
        let spk = shape.spk(&s);
        if let Some(found) = pick(lookup(&spk)?, t, &spk, s)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Wie `locate`, über `io` (Node in ghostctl) und höchstens `max` Zustände
pub async fn locate_io(io: &mut impl TresorIo, p: &TresorParams, t: &Tracked<TresorState>, pmt_ms: i64, max: usize) -> Result<Option<Tracked<TresorState>>, String> {
    let shape = TresorShape::of(p);
    for s in candidates_max(p, &t.state, pmt_ms, max) {
        let spk = shape.spk(&s);
        if let Some(found) = pick(io.utxos(&spk).await?, t, &spk, s)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

// ----------------------------------------------------------- Transaktionen ----

/// Tresor-Eingang; `signer` = Besitzer bei `topUp` und `cancel` (Schlüssel
/// oder Browser-Wallet), bei `pay` keiner
pub(crate) fn tresor_input(p: &TresorParams, t: &Tracked<TresorState>, entry: &'static str, signer: Option<Signer>) -> In {
    let art = standing_order(p, &t.state);
    In {
        outpoint: t.outpoint,
        entry: UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Entry { art, entry, args: vec![], sig_at: signer.map(|k| (0, k)) },
    }
}

pub(crate) fn cont(p: &TresorParams, s: &TresorState, value: u64, cov: Hash) -> TransactionOutput {
    TransactionOutput { value, script_public_key: spk(&standing_order(p, s)), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: cov }) }
}

/// Baut zweimal: erst mit `probe` sompi Gebühr, dann genau mit der nötigen
/// (die Gebühr hängt nicht von den Beträgen ab, nur von der Größe). Mit
/// Browser-Wallet (wallet_ops) genau einmal, mit der Gebühr der Messkopie.
pub(crate) fn build_exact_fee(mk: impl Fn(u64) -> Draft, probe: u64, payload: &[u8], net: &Params) -> Result<Built, String> {
    if let Some(fee) = crate::txb::wallet_fill_fee() {
        return build_with_payload(mk(fee), payload, net);
    }
    let b1 = build_with_payload(mk(probe), payload, net)?;
    // Mindestgebühr der fertigen Tx (mit endgültigen Budgets) plus Aufschlag wie txb::build
    let need = min_fee(b1.compute_mass, b1.transient_mass) * (1000 + FEE_MARGIN_PERMILLE) / 1000;
    if need >= probe || b1.change_index.is_some() {
        return Ok(b1);
    }
    match build_with_payload(mk(need), payload, net) {
        Ok(b2) if b2.change_index.is_none() => Ok(b2),
        _ => Ok(b1),
    }
}

/// Tresor anlegen: `fund` sompi aus den KAS des Absenders in den Vertrag
pub fn open(p: &TresorParams, first: &TresorState, fund: u64, funds: &Funds, net: &Params) -> Result<(Built, Tracked<TresorState>), String> {
    check_params(p, first)?;
    if (fund as i64) < p.amount + p.max_fee + MIN_KEEP {
        return Err(format!(
            "Startguthaben zu klein: mindestens Betrag + Höchstgebühr + {} KAS Reserve = {:.8} KAS",
            MIN_KEEP / 100_000_000,
            (p.amount + p.max_fee + MIN_KEEP) as f64 / 1e8
        ));
    }
    let first_in = funds.utxos.first().ok_or("keine KAS für das Startguthaben")?.0;
    let (out, id) = genesis_output(&standing_order(p, first), fund, 0, first_in, 0);
    let inputs = funds.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: funds.key } }).collect();
    let b = crate::txb::build(Draft { inputs, outputs: vec![out], change_spk: p2pk_spk(&funds.key.xonly()), lock_time: 0 }, net)?;
    let t = track(&b, 0, id, *first);
    Ok((b, t))
}

/// Ergebnis einer Zahlung
#[derive(Debug)]
pub struct Paid {
    pub built: Built,
    pub next: Tracked<TresorState>,
    /// Gebühr kam aus dem Tresor (der Auslöser brauchte keine eigenen KAS)
    pub fee_from_tresor: bool,
}

/// Zahlung des fälligen Termins. Tresor = Eingang 0, Empfänger = Ausgang 0,
/// Fortsetzung = Ausgang 1. Die Gebühr kommt aus dem Tresor (höchstens
/// maxFee), sonst – falls `funds` angegeben – aus den KAS des Auslösers.
/// `payload` muss genau der beim Anlegen gebundene sein (sha256 = payloadHash,
/// `TresorRec::payload`); einen anderen lehnt schon ghostctl ab, nicht erst der
/// Vertrag. Prüft NICHT die Uhr (das tut der Vertrag über lock_time = nextDue).
pub fn pay(p: &TresorParams, t: &Tracked<TresorState>, payload: &[u8], funds: Option<&Funds>, net: &Params) -> Result<Paid, String> {
    if payload_hash(payload) != p.payload_hash {
        return Err("Nachricht passt nicht zum Tresor: jede Zahlung trägt genau die beim Anlegen hinterlegte Nachricht".into());
    }
    if t.state.left == 0 {
        return Err("alle Zahlungen erledigt".into());
    }
    let ns = next_state(p, &t.state);
    if ns.next_due <= t.state.next_due || ns.next_due > standing::MAX_TIME {
        return Err("kein gültiger nächster Termin".into());
    }
    let value = t.value as i64;
    let pay_out = TransactionOutput { value: p.amount as u64, script_public_key: p2pk_spk(&p.recipient), covenant: None };
    // Locktime genau auf den Termin: CLTV verlangt nextDue ≤ lock_time, der
    // Konsens lock_time < Past Median Time – so früh wie möglich gültig
    let lock_time = t.state.next_due as u64;
    let track_next = |b: &Built| Tracked { outpoint: crate::txb::outpoint(&b.tx, 1), value: b.tx.outputs[1].value, cov: t.cov, state: ns };
    let why = if value - p.amount - p.max_fee >= MIN_KEEP {
        let mk = |fee: u64| Draft {
            inputs: vec![tresor_input(p, t, "pay", None)],
            outputs: vec![pay_out.clone(), cont(p, &ns, (value - p.amount) as u64 - fee, t.cov)],
            // Rest < MIN_CHANGE, es entsteht nie ein Wechselgeld-Ausgang
            change_spk: p2pk_spk(&p.recipient),
            lock_time,
        };
        match build_exact_fee(mk, p.max_fee as u64, payload, net) {
            Ok(b) if b.change_index.is_none() && (b.fee as i64) <= p.max_fee => {
                let next = track_next(&b);
                return Ok(Paid { built: b, next, fee_from_tresor: true });
            }
            Ok(_) => "Gebühr über der Höchstgebühr".to_string(),
            Err(e) => e,
        }
    } else {
        format!("es blieben weniger als {} KAS im Tresor", MIN_KEEP / 100_000_000)
    };
    let Some(funds) = funds else {
        return Err(format!("Zahlung aus dem Tresor allein nicht möglich ({why}); mit eigenem Schlüssel (--key) zahlt der Auslöser die Gebühr"));
    };
    if value - p.amount < MIN_KEEP {
        return Err(format!(
            "Guthaben {:.8} KAS reicht nicht für eine Zahlung von {:.8} KAS und {} KAS Rest – der Absender muss auffüllen",
            value as f64 / 1e8,
            p.amount as f64 / 1e8,
            MIN_KEEP / 100_000_000
        ));
    }
    let mut inputs = vec![tresor_input(p, t, "pay", None)];
    inputs.extend(funds.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: funds.key } }));
    let b = build_with_payload(
        Draft { inputs, outputs: vec![pay_out, cont(p, &ns, (value - p.amount) as u64, t.cov)], change_spk: p2pk_spk(&funds.key.xonly()), lock_time },
        payload,
        net,
    )?;
    let next = track_next(&b);
    Ok(Paid { built: b, next, fee_from_tresor: false })
}

/// Absender legt `add` sompi nach (Termine bleiben). `owner` = Schlüssel des
/// Absenders oder seine Browser-Wallet (wallet_ops)
pub fn topup(p: &TresorParams, t: &Tracked<TresorState>, owner: impl Into<Signer>, add: u64, funds: &Funds, net: &Params) -> Result<(Built, Tracked<TresorState>), String> {
    let owner: Signer = owner.into();
    if owner.xonly() != p.owner {
        return Err("Auffüllen darf nur der Absender dieses Tresors".into());
    }
    if add == 0 {
        return Err("Betrag zum Auffüllen muss größer als 0 sein".into());
    }
    let mut inputs = vec![tresor_input(p, t, "topUp", Some(owner))];
    inputs.extend(funds.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: funds.key } }));
    let b = crate::txb::build(Draft { inputs, outputs: vec![cont(p, &t.state, t.value + add, t.cov)], change_spk: p2pk_spk(&funds.key.xonly()), lock_time: 0 }, net)?;
    let next = track(&b, 0, t.cov, t.state);
    Ok((b, next))
}

/// Absender kündigt: alles (abzüglich Gebühr) an den Absender, der Tresor
/// endet. `owner` = Schlüssel des Absenders oder seine Browser-Wallet
pub fn cancel(p: &TresorParams, t: &Tracked<TresorState>, owner: impl Into<Signer>, net: &Params) -> Result<Built, String> {
    let owner: Signer = owner.into();
    if owner.xonly() != p.owner {
        return Err("Kündigen darf nur der Absender dieses Tresors".into());
    }
    let to = p2pk_spk(&p.owner);
    let mk = |fee: u64| Draft {
        inputs: vec![tresor_input(p, t, "cancel", Some(owner))],
        outputs: vec![TransactionOutput { value: t.value - fee, script_public_key: to.clone(), covenant: None }],
        change_spk: to.clone(),
        lock_time: 0,
    };
    let probe = (t.value / 2).min(MIN_CHANGE / 4);
    build_exact_fee(mk, probe, &[], net)
}

// -------------------------------------------------------------- Tresor-Code ----

/// Inhalt des teilbaren Codes (JSON, dann base64url)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TresorCode {
    pub network: String,
    pub cov: Hash,
    pub params: TresorParams,
    pub state: TresorState,
    /// Outpoint beim Erstellen des Codes ("txid:index"); nur ein Startpunkt,
    /// die aktuelle UTXO sucht ghostctl selbst
    pub outpoint: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub onchain: bool,
    /// Nachricht verschlüsselt an den Empfänger (Hex, Format message.rs), wenn
    /// nicht öffentlich: kommt unverändert in jede Zahlung
    #[serde(default)]
    pub sealed: String,
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let vals: Vec<u32> = s.bytes().map(|b| B64.iter().position(|&x| x == b).map(|p| p as u32)).collect::<Option<_>>()?;
    if vals.len() % 4 == 1 {
        return None;
    }
    let mut out = vec![];
    for c in vals.chunks(4) {
        let n = c.iter().enumerate().fold(0u32, |acc, (i, v)| acc | v << (18 - 6 * i));
        for i in 0..c.len() - 1 {
            out.push((n >> (16 - 8 * i) & 0xff) as u8);
        }
    }
    Some(out)
}

pub fn outpoint_text(op: &TransactionOutpoint) -> String {
    format!("{}:{}", op.transaction_id, op.index)
}

pub fn parse_outpoint(s: &str) -> Result<TransactionOutpoint, String> {
    let (id, idx) = s.split_once(':').ok_or("Outpoint: txid:index erwartet")?;
    Ok(TransactionOutpoint::new(Hash::from_str(id).map_err(|_| "Outpoint: ungültige TXID")?, idx.parse().map_err(|_| "Outpoint: ungültiger Index")?))
}

/// Verschlüsselte Tresor-Nachricht: leer oder Hex im Format von message.rs
pub fn check_sealed(hex: &str) -> Result<(), String> {
    if hex.is_empty() {
        return Ok(());
    }
    let raw = faster_hex_decode(hex).ok_or("Tresor-Code: verschlüsselte Nachricht beschädigt")?;
    if !crate::message::is_encrypted(&raw) || raw.len() < crate::message::OVERHEAD || raw.len() > crate::message::MAX_ENCRYPTED {
        return Err("Tresor-Code: verschlüsselte Nachricht beschädigt".into());
    }
    Ok(())
}

/// Zeichen, die ghostctl schon vor der Verschärfung des Filters (A12-11) in
/// Nachrichten abgelehnt hat – seit es Tresore gibt (Steuer-, Richtungs- und
/// Null-Breite-Zeichen). Kein Tresor konnte sie in der Beschreibung haben.
fn legacy_bad_char(c: char) -> bool {
    c.is_control() || matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{061C}' | '\u{00AD}')
}

/// Beschreibung in einem Tresor-Code: höchstens MAX_MESSAGE_CHARS Zeichen und
/// nichts, was ghostctl nie angenommen hat. Was erst der strengere Filter
/// ablehnt (Tastenkappe, VS16 hinter Buchstaben, U+2028, Private Use …),
/// konnte ein älterer Tresor enthalten; sein Code wird übernommen, die
/// Beschreibung bleibt als Vergleichstext für den Eingang unverändert und wird
/// nur bereinigt angezeigt (`TresorRec::shown_message`). Vorher lehnte der
/// Import solche Codes ab (Restpunkt zu A12-11).
pub fn check_code_message(m: &str) -> Result<(), String> {
    let n = m.chars().count();
    if n > crate::abo::MAX_MESSAGE_CHARS {
        return Err(format!("Tresor-Code: Beschreibung zu lang ({n} Zeichen, höchstens {})", crate::abo::MAX_MESSAGE_CHARS));
    }
    if m.chars().any(legacy_bad_char) {
        return Err("Tresor-Code: Beschreibung mit Steuer- oder Richtungszeichen".into());
    }
    Ok(())
}

/// Payload jeder Zahlung aus den Angaben eines Tresors: öffentliche Nachricht
/// im Klartext, sonst die beim Anlegen verschlüsselte Fassung, sonst leer
pub fn bound_payload(message: &str, onchain: bool, sealed: &str) -> Vec<u8> {
    if onchain && !message.trim().is_empty() { message.trim().as_bytes().to_vec() } else { faster_hex_decode(sealed).unwrap_or_default() }
}

/// Passen Nachricht bzw. verschlüsselte Fassung zum Hash im Vertrag? Sonst
/// trügen die Zahlungen etwas anderes, als Code oder Liste zeigen – ghostctl
/// könnte sie gar nicht auslösen (der Vertrag lehnt jeden anderen Payload ab).
/// Dazu die Form (A13-tresor-1 bis 3): öffentlich heißt Beschreibung im
/// Klartext und keine verschlüsselte Fassung; sonst gibt es eine
/// Beschreibung genau dann, wenn es eine verschlüsselte Fassung gibt. Ohne
/// diese Regel passte zu einem Tresor ohne Nachricht (leerer Payload) jede
/// erfundene Beschreibung, und ein öffentlicher Tresor ohne Text trüge ein
/// Chiffrat, das die Liste nicht zeigt.
pub fn check_bound(p: &TresorParams, message: &str, onchain: bool, sealed: &str) -> Result<(), String> {
    let has_text = !message.trim().is_empty();
    if onchain && (!has_text || !sealed.is_empty()) {
        return Err("Tresor-Code: öffentliche Nachricht ohne Text oder mit verschlüsselter Fassung".into());
    }
    if !onchain && has_text != !sealed.is_empty() {
        return Err(if has_text {
            "Tresor-Code: Beschreibung ohne Nachricht in den Zahlungen (die Zahlungen dieses Tresors tragen keine)".into()
        } else {
            "Tresor-Code: verschlüsselte Nachricht ohne Beschreibung".into()
        });
    }
    if payload_hash(&bound_payload(message, onchain, sealed)) != p.payload_hash {
        return Err("Tresor-Code: Nachricht passt nicht zum Vertrag (der Tresor verlangt in jeder Zahlung eine andere)".into());
    }
    Ok(())
}

/// Nachricht beim Anlegen binden (`tresor open`): `payload` ist der Payload
/// jeder Zahlung (öffentlich der Klartext, sonst die einmal an den Empfänger
/// verschlüsselte Fassung, ohne Nachricht leer). Setzt payloadHash in `p` und
/// liefert die verschlüsselte Fassung für den Tresor-Eintrag (Hex, öffentlich
/// leer). Err, wenn Beschreibung und Payload nicht zusammenpassen.
pub fn bind_message(p: &mut TresorParams, message: &str, onchain: bool, payload: &[u8]) -> Result<String, String> {
    let sealed = if onchain { String::new() } else { faster_hex::hex_string(payload) };
    p.payload_hash = payload_hash(payload);
    check_bound(p, message, onchain, &sealed).map_err(|_| "Nachricht und Payload des Tresors passen nicht zusammen".to_string())?;
    Ok(sealed)
}

/// Wie sicher trägt jede Zahlung die Beschreibung eines Tresors?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageCheck {
    /// keine Nachricht
    None,
    /// öffentlich: der Klartext selbst ist im Vertrag gebunden
    Bound,
    /// verschlüsselt, hier angelegt oder mit dem Empfängerschlüssel geprüft
    Checked,
    /// verschlüsselt, Beschreibung nur laut Tresor-Code (nicht geprüft)
    Unchecked,
    /// die Zahlungen tragen etwas anderes als die Beschreibung
    Mismatch,
}

/// Verschlüsselte Nachricht mit dem Schlüssel des Empfängers gegen die
/// Beschreibung prüfen: Some(true) = gleich, Some(false) = abweichend oder
/// nicht lesbar, None = nichts zu prüfen oder kein passender Schlüssel
pub fn sealed_matches(p: &TresorParams, message: &str, onchain: bool, sealed: &str, sk: Option<&SecretKey>) -> Option<bool> {
    let sk = sk?;
    if onchain || sealed.is_empty() || sk.x_only_public_key(secp256k1::SECP256K1).0.serialize().as_slice() != p.recipient.as_slice() {
        return None;
    }
    let text = faster_hex_decode(sealed).and_then(|raw| crate::message::decrypt(sk, &raw));
    Some(text.is_some_and(|t| t.trim() == message.trim()))
}

fn check_sealed_text(r: &TresorRec, sk: Option<&SecretKey>) -> Option<bool> {
    sealed_matches(&r.params, &r.message, r.onchain, &r.sealed, sk)
}

fn faster_hex_decode(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut out = vec![0u8; hex.len() / 2];
    faster_hex::hex_decode(hex.as_bytes(), &mut out).ok()?;
    Some(out)
}

impl TresorCode {
    pub fn of(network: &str, r: &TresorRec) -> Self {
        Self {
            network: network.into(),
            cov: r.utxo.cov,
            params: r.params.clone(),
            state: r.utxo.state,
            outpoint: outpoint_text(&r.utxo.outpoint),
            message: r.message.clone(),
            onchain: r.onchain,
            sealed: r.sealed.clone(),
        }
    }
    pub fn encode(&self) -> String {
        format!("{CODE_PREFIX}{}", b64_encode(serde_json::to_string(self).unwrap().as_bytes()))
    }
    /// Code lesen und auf Form prüfen (nicht am Node – das tut `locate`).
    /// Dieselben Regeln wie beim Anlegen (`check_params`), nur der Zustand darf
    /// fortgeschritten sein (Anzahl 0, Termin nicht mehr auf dem Starttag):
    /// ein präparierter Code mit 1 ms Intervall oder 1 sompi Betrag kostete
    /// sonst bis zu MAX_FOLLOW Node-Abfragen je Abgleich (A12-16).
    pub fn decode(code: &str) -> Result<Self, String> {
        if code.trim().starts_with(OLD_CODE_PREFIX) {
            return Err("Alter Tresor-Code (ghost-tresor:1:): Dieser Tresor läuft mit dem alten Vertrag, der die Nachricht nicht bindet, und wird nicht mehr unterstützt. Bitte beim Absender einen neuen Tresor anfordern.".into());
        }
        let body = code.trim().strip_prefix(CODE_PREFIX).ok_or("Kein Tresor-Code (er beginnt mit „ghost-tresor:2:“)")?;
        if body.len() > 4_000 {
            return Err("Tresor-Code zu lang".into());
        }
        let raw = b64_decode(body).ok_or("Tresor-Code beschädigt (unvollständig kopiert?)")?;
        let c: TresorCode = serde_json::from_slice(&raw).map_err(|_| "Tresor-Code beschädigt (unvollständig kopiert?)".to_string())?;
        parse_outpoint(&c.outpoint)?;
        check_code_message(&c.message)?;
        check_sealed(&c.sealed)?;
        let p = &c.params;
        for (x, what) in [(&p.owner, "Absender"), (&p.recipient, "Empfänger")] {
            secp256k1::XOnlyPublicKey::from_slice(x).map_err(|_| format!("Tresor-Code: {what} ist kein gültiger Schlüssel"))?;
        }
        let interval_ok = match (p.anchor_day, p.period_ms) {
            (1..=31, 0) => true,
            (0, per) => (standing::DAY_MS..=3650 * standing::DAY_MS).contains(&per),
            _ => false,
        };
        let s = &c.state;
        if p.owner == p.recipient
            || p.payload_hash.len() != 32
            || p.amount < MIN_AMOUNT
            || !(1..=MAX_MAX_FEE).contains(&p.max_fee)
            || !interval_ok
            || s.left < -1
            || s.next_due < LOCK_TIME_THRESHOLD as i64
            || s.next_due > standing::MAX_TIME
        {
            return Err("Tresor-Code: ungültige Werte".into());
        }
        check_bound(p, &c.message, c.onchain, &c.sealed)?;
        Ok(c)
    }
}

// ------------------------------------------------------------------ Datei ----

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TresorHist {
    /// lokale Zeit
    pub at: String,
    /// open | pay | topup | cancel | import
    pub action: String,
    #[serde(default)]
    pub txid: Option<String>,
    /// bezahlter Termin (Unix-ms)
    #[serde(default)]
    pub due: Option<i64>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TresorRec {
    /// erste 8 Hex-Zeichen der Covenant-ID (auf allen Rechnern gleich)
    pub id: String,
    pub params: TresorParams,
    /// aktuelle UTXO: Outpoint, Wert, Covenant-ID, Zustand
    pub utxo: Tracked<TresorState>,
    /// Beschreibung, z. B. „Miete“; bei `onchain` in jeder Zahlung als Payload
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub onchain: bool,
    /// sonst verschlüsselt an den Empfänger (Hex), einmal beim Anlegen erzeugt
    #[serde(default)]
    pub sealed: String,
    /// Schlüsseldatei des Absenders, wenn hier angelegt (keine Geheimnisse)
    #[serde(default)]
    pub key: Option<String>,
    /// Beim Übernehmen mit dem Schlüssel des Empfängers geprüft: `sealed`
    /// entschlüsselt ergibt genau die Beschreibung (A13-tresor-1). Bei einer
    /// verschlüsselten Nachricht bindet der Vertrag nur `sealed`, nicht den
    /// Text der Beschreibung.
    #[serde(default)]
    pub checked: bool,
    pub created: String,
    /// gekündigt (von hier aus)
    #[serde(default)]
    pub ended: Option<String>,
    /// beim letzten Abgleich nicht auffindbar (von anderswo gekündigt?)
    #[serde(default)]
    pub missing: Option<String>,
    /// seit wann nicht auffindbar (Unix-ms); die Automatik sieht bis
    /// MISSING_RECHECK_FOR_MS danach stündlich erneut nach
    #[serde(default)]
    pub missing_ms: Option<i64>,
    /// letzter Fehler beim automatischen Auslösen und frühester neuer Versuch (Unix-ms)
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub retry_after: Option<i64>,
    #[serde(default)]
    pub history: Vec<TresorHist>,
    /// Über die Browser-Wallet angelegt (öffentliche Seite, wallet_ops): Die
    /// Netzgebühr jeder Zahlung kommt nur aus dem Tresor (Höchstgebühr); ein
    /// Agent zahlt sie nie mit dem eigenen Schlüssel dazu, sonst ließe sich
    /// sein Guthaben über fremde Tresore mit knappem Rest aufbrauchen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wallet: bool,
    /// Letzte Zahlung durch diese Automatik (Unix-ms, Uhr des Rechners): die
    /// Runde bedient fällige Tresore reihum, am längsten nicht bediente zuerst
    /// (Audit 19 A19-1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_paid_ms: Option<i64>,
}

impl TresorRec {
    pub fn new(p: TresorParams, utxo: Tracked<TresorState>, message: String, onchain: bool, key: Option<String>, now: &str) -> Self {
        Self {
            id: id_of(&utxo.cov),
            params: p,
            utxo,
            message,
            onchain,
            sealed: String::new(),
            key,
            checked: false,
            created: now.into(),
            ended: None,
            missing: None,
            missing_ms: None,
            last_error: None,
            retry_after: None,
            history: vec![],
            wallet: false,
            last_paid_ms: None,
        }
    }
    pub fn push(&mut self, h: TresorHist) {
        self.history.push(h);
        if self.history.len() > MAX_HISTORY {
            let cut = self.history.len() - MAX_HISTORY;
            self.history.drain(..cut);
        }
    }
    /// Payload jeder Zahlung: öffentliche Nachricht im Klartext, sonst die beim
    /// Anlegen verschlüsselte Fassung, sonst leer (`bound_payload`; sein
    /// sha256 steht als payloadHash im Vertrag)
    pub fn payload(&self) -> Vec<u8> {
        bound_payload(&self.message, self.onchain, &self.sealed)
    }
    /// Beschreibung zur Anzeige, nach heutigem Filter bereinigt
    /// (`abo::sendable_message`, wie bei Daueraufträgen); `true`, wenn dabei
    /// etwas wegfiel. Gespeichert bleibt die Beschreibung unverändert: Sie ist
    /// der Text, mit dem der Eingang die Zahlungen dieses Tresors vergleicht,
    /// und steht so im Code des Absenders.
    pub fn shown_message(&self) -> (String, bool) {
        let shown = crate::abo::sendable_message(&self.message);
        let cleaned = shown != self.message.trim();
        (shown, cleaned)
    }
    /// Wie sicher trägt jede Zahlung die Beschreibung? `sk`: Schlüssel des
    /// Empfängers, falls vorhanden (`tresor list --key`); ein anderer Schlüssel
    /// zählt nicht. Hier angelegt (`key`) hat der Absender selbst
    /// verschlüsselt; übernommen nur, wenn beim Import geprüft (`checked`).
    pub fn message_check(&self, sk: Option<&SecretKey>) -> MessageCheck {
        if self.message.trim().is_empty() {
            return MessageCheck::None;
        }
        if self.onchain {
            return MessageCheck::Bound;
        }
        if self.sealed.is_empty() {
            return MessageCheck::Mismatch;
        }
        if let Some(r) = check_sealed_text(self, sk) {
            return if r { MessageCheck::Checked } else { MessageCheck::Mismatch };
        }
        if self.checked || self.key.is_some() { MessageCheck::Checked } else { MessageCheck::Unchecked }
    }
    /// Beschreibung sicher gebunden (öffentlich per Hash oder verschlüsselt und geprüft)?
    pub fn message_sure(&self) -> bool {
        matches!(self.message_check(None), MessageCheck::Bound | MessageCheck::Checked)
    }
    pub fn active(&self) -> bool {
        self.ended.is_none() && self.missing.is_none()
    }
    /// Nicht auffindbar, aber noch im Fenster für den automatischen neuen
    /// Versuch und dessen Zeit gekommen (ältere Dateien ohne `missing_ms`:
    /// ja – das Fenster beginnt dann beim nächsten Abgleich)
    pub fn recheck_missing(&self, now_ms: i64) -> bool {
        self.ended.is_none()
            && self.missing.is_some()
            && self.missing_ms.is_none_or(|t| now_ms - t <= MISSING_RECHECK_FOR_MS)
            && self.retry_after.is_none_or(|t| t <= now_ms)
    }
    /// Darf ein Auslöser mit eigenem Schlüssel (`with_key`) bei diesem Tresor
    /// die Gebühr zahlen? Bei Wallet-Tresoren nie (`wallet`).
    pub fn key_may_pay(&self, with_key: bool) -> bool {
        with_key && !self.wallet
    }
    /// Offline-Schätzung mit der Uhr des Rechners: lohnt ein Abgleich mit dem
    /// Node? `with_key`: der Auslöser könnte die Gebühr selbst zahlen (nicht
    /// bei Wallet-Tresoren, `key_may_pay`).
    pub fn looks_due(&self, now_ms: i64, with_key: bool) -> bool {
        let with_key = self.key_may_pay(with_key);
        if self.missing.is_some() {
            return self.recheck_missing(now_ms);
        }
        self.active()
            && self.utxo.state.left != 0
            && self.utxo.state.next_due + PMT_LAG_MS <= now_ms
            && payable(&self.params, self.utxo.value, with_key)
            && self.retry_after.is_none_or(|t| t <= now_ms)
    }
}

impl TresorRec {
    /// Belegt der Tresor einen Platz der Datei (MAX_FILE_TRESORE)? Nur, wenn
    /// er den Agenten bald Abfragen und eine Sendung kostet: läuft, hat noch
    /// Zahlungen, trägt die nächste und ist innerhalb BUSY_HORIZON_MS fällig
    /// (A19-3). Leere, erledigte und weit entfernte Tresore ruhen.
    pub fn busy(&self, now_ms: i64) -> bool {
        self.active()
            && self.utxo.state.left != 0
            && payable(&self.params, self.utxo.value, self.key_may_pay(true))
            && self.utxo.state.next_due <= now_ms + BUSY_HORIZON_MS
    }
    /// Darf ein Wallet-Tresor aus der vollen Datei fallen? Nur, wenn er
    /// gekündigt ist oder seit über MISSING_RECHECK_FOR_MS am Node fehlt (die
    /// Automatik hat ihn dann eine Woche lang stündlich gesucht). Laufende
    /// mit Guthaben nie: ohne Eintrag könnte der Besitzer über die Seite
    /// nicht mehr kündigen.
    fn evictable(&self, now_ms: i64) -> bool {
        self.wallet && (self.ended.is_some() || (self.missing.is_some() && self.missing_ms.is_some_and(|t| now_ms - t > MISSING_RECHECK_FOR_MS)))
    }
}

pub fn id_of(cov: &Hash) -> String {
    cov.to_string()[..8].to_string()
}

/// Version der Tresor-Datei: 2 = Vertrag mit gebundener Nachricht
pub const FILE_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TresorFile {
    pub version: u32,
    pub network: String,
    pub tresore: Vec<TresorRec>,
}

impl TresorFile {
    pub fn empty(network: &str) -> Self {
        Self { version: FILE_VERSION, network: network.into(), tresore: vec![] }
    }
    /// Tresor über ID (8 Hex) oder volle Covenant-ID
    pub fn find(&self, id: &str) -> Result<usize, String> {
        let id = id.trim().to_lowercase();
        let hits: Vec<usize> = self.tresore.iter().enumerate().filter(|(_, r)| r.id == id || r.utxo.cov.to_string() == id).map(|(i, _)| i).collect();
        match hits.as_slice() {
            [i] => Ok(*i),
            [] => Err(format!("Tresor {id} nicht gefunden")),
            // A19-9: die Kurz-ID lässt sich mit ~2^32 Versuchen treffen; die
            // vollen IDs nennen, damit der Betreiber gleich weiterkommt
            many => Err(format!(
                "Tresor {id} ist mehrdeutig – bitte die volle Covenant-ID angeben: {}",
                many.iter().map(|&i| self.tresore[i].utxo.cov.to_string()).collect::<Vec<_>>().join(", ")
            )),
        }
    }
    /// Übernehmen oder (gleiche Covenant-ID) aktualisieren
    pub fn upsert(&mut self, r: TresorRec) -> usize {
        match self.tresore.iter().position(|x| x.utxo.cov == r.utxo.cov) {
            Some(i) => {
                let old = &mut self.tresore[i];
                if old.missing.is_some() {
                    // wieder da: die Wartezeit galt dem erneuten Nachsehen
                    // (wie in `follow`); sonst wartete die Automatik nach dem
                    // erneuten Übernehmen bis zu einer Stunde (Restpunkt A12-16)
                    old.retry_after = None;
                }
                let (sure, fits) = (r.message_sure(), check_bound(&old.params, &r.message, r.onchain, &r.sealed).is_ok());
                old.utxo = r.utxo;
                old.missing = None;
                old.missing_ms = None;
                // Beschreibung nur übernehmen, wenn sie gebunden ist
                // (öffentlich per Hash oder verschlüsselt und beim Import
                // geprüft) und die bisherige es nicht war; eine ungebundene
                // aus einem Code ersetzt nie etwas (A13-tresor-2)
                if sure && fits && !old.message_sure() {
                    old.message = r.message;
                    old.onchain = r.onchain;
                    old.sealed = r.sealed;
                    old.checked = r.checked;
                } else if r.checked && old.message == r.message && old.sealed == r.sealed && old.onchain == r.onchain {
                    old.checked = true;
                }
                i
            }
            None => {
                self.tresore.push(r);
                self.tresore.len() - 1
            }
        }
    }
}

impl TresorFile {
    /// Tresore eines Besitzers (x-only), in der Reihenfolge der Datei
    pub fn of_owner<'a>(&'a self, owner: &'a [u8]) -> impl Iterator<Item = &'a TresorRec> + 'a {
        self.tresore.iter().filter(move |r| r.params.owner == owner)
    }
    /// Platz für einen neuen Wallet-Tresor von `owner` (Grenzen gegen eine
    /// mit Einträgen gefüllte Datei, A19-3): höchstens MAX_WALLET_PER_OWNER
    /// laufende je Besitzer; ab MAX_FILE_TRESORE Einträgen fallen zuerst
    /// beendete Wallet-Tresore heraus (älteste zuerst, `evictable`); danach
    /// abgelehnt, wenn MAX_FILE_TRESORE Tresore belegt sind (`busy`, `now_ms`
    /// = Past Median Time) oder die Datei MAX_FILE_ALL Einträge hat.
    pub fn make_room_for_wallet(&mut self, owner: &[u8], now_ms: i64) -> Result<(), String> {
        let running = self.of_owner(owner).filter(|r| r.ended.is_none()).count();
        if running >= MAX_WALLET_PER_OWNER {
            return Err(format!("Diese Adresse hat schon {running} laufende Tresore (höchstens {MAX_WALLET_PER_OWNER}); bitte erst einen kündigen"));
        }
        while self.tresore.len() >= MAX_FILE_TRESORE {
            match self.tresore.iter().position(|r| r.evictable(now_ms)) {
                Some(i) => {
                    self.tresore.remove(i);
                }
                None => break,
            }
        }
        let busy = self.tresore.iter().filter(|r| r.busy(now_ms)).count();
        if busy >= MAX_FILE_TRESORE || self.tresore.len() >= MAX_FILE_ALL {
            return Err("Auf diesem Server ist gerade kein Platz für weitere Tresore – bitte später erneut versuchen".into());
        }
        Ok(())
    }
}

/// deployments/mainnet.json → deployments/mainnet-tresore.json
pub fn path_for(state: &Path) -> PathBuf {
    let stem = state.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "state".into());
    state.with_file_name(format!("{stem}-tresore.json"))
}

/// Lesen (fehlt die Datei: leer). Schreiben nur unter der Sperre des Aufrufers.
pub fn load(path: &Path, network: &str) -> Result<TresorFile, String> {
    let f = match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str::<TresorFile>(&t).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TresorFile::empty(network),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if f.network != network {
        return Err(format!("{} gehört zum Netz {}, gewählt ist {network}", path.display(), f.network));
    }
    check_file(&f).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(TresorFile { version: FILE_VERSION, ..f })
}

/// Tresore des alten Vertrags (ohne gebundene Nachricht) kann diese Version
/// weder finden noch auslösen: klar melden statt still übergehen
pub fn check_file(f: &TresorFile) -> Result<(), String> {
    match f.tresore.iter().find(|r| r.params.payload_hash.len() != 32) {
        Some(r) => Err(format!(
            "Tresor {} stammt vom alten Vertrag ohne gebundene Nachricht – diese Version kann ihn nicht mehr bedienen. Die Datei bitte beiseitelegen; der Absender kündigt mit der alten Version und legt einen neuen Tresor an.",
            r.id
        )),
        None => Ok(()),
    }
}

pub fn save(path: &Path, f: &TresorFile) -> Result<(), String> {
    crate::store::atomic_write(path, &serde_json::to_string_pretty(f).unwrap())
}

/// Gibt es offline betrachtet etwas auszulösen?
pub fn needs_run(f: &TresorFile, now_ms: i64, with_key: bool) -> bool {
    f.tresore.iter().any(|r| r.looks_due(now_ms, with_key))
}

// ------------------------------------------------ Abläufe (ghostctl tresor) ----

/// Ein- und Ausgabe der Tresor-Befehle: in ghostctl Node, Journal und
/// Terminal, in den Tests der Simulator. Nachführen, Import und die Runde der
/// Automatik laufen so in den Tests genau wie in ghostctl.
#[allow(async_fn_in_trait)]
pub trait TresorIo {
    /// UTXOs zu einem Skript (Node: über dessen Adresse)
    async fn utxos(&mut self, spk: &ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String>;
    /// KAS des Auslösers (nur mit eigenem Schlüssel)
    async fn funds(&mut self) -> Option<Funds>;
    /// Senden mit Journal; wird die Tx angenommen, bekommt die Tresor-Datei `next`
    async fn send(&mut self, what: &str, b: &Built, next: &TresorFile) -> Result<(), String>;
    /// Tresor-Datei speichern (im Probelauf nichts)
    fn save(&mut self, f: &TresorFile) -> Result<(), String>;
    /// Journal einer gesendeten Tx noch offen (die Tx ist vielleicht im Netz)?
    fn journal_open(&self) -> bool;
    /// Probelauf: nichts senden, nichts übernehmen
    fn dry_run(&self) -> bool;
    /// Uhr des Rechners (Unix-ms)
    fn now_ms(&self) -> i64;
    /// Meldung für den Nutzer
    fn say(&mut self, line: &str);
}

/// Wie gesucht wird: vollständig (Abgleich von Hand, Import, Auffüllen,
/// Kündigen, eine bestimmte Zahlung) oder in der Runde der Automatik
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Search {
    Full,
    Auto,
    /// Öffentliche Seite (Auffüllen, Kündigen über die Browser-Wallet):
    /// höchstens PUBLIC_FOLLOW Zustände (A19-2)
    Public,
}

/// Tresor über `io` nachführen. Liefert einen Hinweis, wenn sich etwas geändert
/// hat. In der Automatik wird ein nicht auffindbarer Tresor nur über
/// MISSING_RECHECK_FOLLOW Zustände gesucht (A12-16).
pub async fn follow(io: &mut impl TresorIo, r: &mut TresorRec, pmt: i64, now: &str, search: Search) -> Result<Option<String>, String> {
    if r.ended.is_some() {
        return Ok(None);
    }
    let max = match search {
        Search::Auto if r.missing.is_some() => MISSING_RECHECK_FOLLOW,
        Search::Public => PUBLIC_FOLLOW,
        _ => MAX_FOLLOW,
    };
    match locate_io(io, &r.params, &r.utxo, pmt, max).await? {
        Some(t) => {
            let paid = candidates(&r.params, &r.utxo.state, pmt).iter().position(|s| *s == t.state).unwrap_or(0);
            let note = (t.outpoint != r.utxo.outpoint || r.missing.is_some()).then(|| {
                format!(
                    "Tresor {}: {} Zahlung(en) seit dem letzten Abgleich, Guthaben {} KAS, nächster Termin {}",
                    r.id,
                    paid,
                    crate::abo::fmt_amount(t.value),
                    fmt_time(t.state.next_due)
                )
            });
            if r.missing.is_some() {
                // wieder da: die Wartezeit galt dem erneuten Nachsehen
                r.retry_after = None;
            }
            r.utxo = t;
            r.missing = None;
            r.missing_ms = None;
            Ok(note)
        }
        None => {
            let first = r.missing.is_none();
            if first {
                r.missing = Some(now.into());
            }
            // die Automatik sieht stündlich erneut nach (A12-16), eine Woche lang
            let now_ms = io.now_ms();
            r.missing_ms.get_or_insert(now_ms);
            r.retry_after = Some(now_ms + MISSING_RECHECK_MS);
            Ok(first.then(|| format!("Tresor {}: am Node nicht auffindbar – vermutlich vom Absender gekündigt", r.id)))
        }
    }
}

/// Rückstand: erreichte, noch nicht gezahlte Termine ab dem Zustand `s`
pub fn backlog(p: &TresorParams, s: &TresorState, pmt: i64) -> usize {
    candidates(p, s, pmt).len() - 1
}

/// Import nur mit Rückstand bis MAX_IMPORT_BACKLOG (A12-16)
pub fn check_backlog(p: &TresorParams, s: &TresorState, pmt: i64) -> Result<(), String> {
    let n = backlog(p, s, pmt);
    if n > MAX_IMPORT_BACKLOG {
        return Err(format!(
            "Tresor-Code: {} Termine seit {} nicht gezahlt – mehr als {MAX_IMPORT_BACKLOG} übernimmt ghostctl nicht (jeder Abgleich müsste sie einzeln am Node nachsehen)",
            if n + 1 >= MAX_FOLLOW { format!("über {n}") } else { n.to_string() },
            fmt_time(s.next_due)
        ));
    }
    Ok(())
}

/// `tresor import`: Code prüfen, die UTXO am Node suchen, den Rückstand
/// begrenzen und übernehmen. Liefert den Index in der Datei (gespeichert wird
/// nicht). `recipient`: Schlüssel des Empfängers, falls vorhanden (ghostctl
/// sucht ihn in keys/ oder nimmt --key); damit wird eine verschlüsselte
/// Nachricht entschlüsselt und muss die Beschreibung des Codes ergeben, sonst
/// wird der Code abgelehnt (A13-tresor-1). Ein anderer Schlüssel zählt nicht.
pub async fn import(io: &mut impl TresorIo, file: &mut TresorFile, code: &str, network: &str, pmt: i64, now: &str, recipient: Option<&SecretKey>) -> Result<usize, String> {
    let c = TresorCode::decode(code)?;
    if c.network != network {
        return Err(format!("Der Tresor-Code gehört zum Netz {}, gewählt ist {network}", c.network));
    }
    let checked = match sealed_matches(&c.params, &c.message, c.onchain, &c.sealed, recipient) {
        Some(false) => {
            return Err("Tresor-Code: Die Beschreibung weicht von der verschlüsselten Nachricht ab, die jede Zahlung trägt – der Code wurde verändert. Bitte beim Absender einen neuen Code anfordern.".into());
        }
        Some(true) => true,
        None => false,
    };
    let start = Tracked { outpoint: parse_outpoint(&c.outpoint)?, value: 0, cov: c.cov, state: c.state };
    let t = locate_io(io, &c.params, &start, pmt, MAX_FOLLOW)
        .await?
        .ok_or("Zu diesem Tresor-Code gibt es am Node keine UTXO – gekündigt, noch nicht bestätigt oder der Code ist falsch")?;
    check_backlog(&c.params, &t.state, pmt)?;
    let mut rec = TresorRec::new(c.params, t, c.message, c.onchain, None, now);
    rec.sealed = c.sealed;
    rec.checked = checked;
    rec.push(TresorHist { at: now.into(), action: "import".into(), txid: None, due: None, note: None });
    let i = file.upsert(rec);
    if let (shown, true) = file.tresore[i].shown_message() {
        io.say(&format!(
            "Hinweis: Die Beschreibung des Tresors {} enthält Zeichen, die heute nicht mehr erlaubt sind (z. B. Tastenkappen-Emoji oder Zeilentrenner); angezeigt wird sie bereinigt: „{shown}“",
            file.tresore[i].id
        ));
    }
    Ok(i)
}

/// Ergebnis je Tresor in `tresor pay`
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TresorReport {
    pub id: String,
    pub ok: bool,
    pub paid: bool,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txid: Option<String>,
}

/// `tresor pay`: ein bestimmter Tresor (`id`) oder die Runde der Automatik über
/// alle aktiven und die nicht auffindbaren, deren erneutes Nachsehen fällig ist
/// (A12-16). `with_key`: der Auslöser darf die Gebühr mit eigenem Schlüssel
/// zahlen. Nach einem Sendefehler geht die Runde weiter, außer das Journal ist
/// noch offen oder der Nutzer hat abgebrochen.
///
/// Reihenfolge (Audit 19 A19-1): Je Tresor und Runde höchstens ein Termin.
/// Offline fällige Tresore kommen zuerst, und zwar reihum – der am längsten
/// nicht bediente zuerst (`last_paid_ms`), danach die übrigen in
/// Dateireihenfolge. Ein Tresor mit großem Rückstand kann so die Sendezeit
/// einer Runde (Agent: 90 s nach der ersten Sendung) nicht Runde für Runde
/// für sich allein verbrauchen; vorher kam immer der erste der Datei zuerst.
/// Was das Nachführen am Node herausfindet (vor allem `missing` nach der
/// teuren ersten Suche), wird sofort gespeichert und geht nicht verloren,
/// wenn das Zeitlimit die Runde vor dem Ende abbricht (A19-2).
pub async fn pay_round(io: &mut impl TresorIo, file: &mut TresorFile, id: Option<&str>, with_key: bool, pmt: i64, now: &str, net: &Params) -> Result<Vec<TresorReport>, String> {
    let single = id.is_some();
    let idxs: Vec<usize> = match id {
        Some(i) => vec![file.find(i)?],
        None => {
            let now_ms = io.now_ms();
            let mut v: Vec<usize> = (0..file.tresore.len()).filter(|&i| file.tresore[i].active() || file.tresore[i].recheck_missing(now_ms)).collect();
            // stabil: bei Gleichstand bleibt die Dateireihenfolge
            v.sort_by_key(|&i| {
                let r = &file.tresore[i];
                if r.missing.is_none() && r.looks_due(now_ms, with_key) { (0, r.last_paid_ms.unwrap_or(i64::MIN)) } else { (1, 0) }
            });
            v
        }
    };
    let search = if single { Search::Full } else { Search::Auto };
    let mut reports: Vec<TresorReport> = vec![];
    for i in idxs {
        let note = follow(io, &mut file.tresore[i], pmt, now, search).await?;
        if let Some(n) = &note {
            io.say(n);
            // gleich sichern (A19-2): die Suche bis MAX_FOLLOW soll sich nach
            // einem Abbruch der Runde nicht wiederholen
            io.save(file)?;
        }
        let r = file.tresore[i].clone();
        let rep = |ok: bool, paid: bool, text: String, txid: Option<String>| TresorReport { id: r.id.clone(), ok, paid, text, txid };
        if !r.active() {
            if single {
                io.save(file)?;
                return Err(format!("Tresor {} ist gekündigt oder am Node nicht auffindbar", r.id));
            }
            // gemeldet nur beim ersten Mal, nicht bei jedem erneuten Nachsehen
            if note.is_some() {
                reports.push(rep(false, false, "nicht auffindbar oder gekündigt".into(), None));
            }
            continue;
        }
        // Wallet-Tresore zahlen die Gebühr nur selbst (TresorRec::wallet)
        let with_key = r.key_may_pay(with_key);
        if let Err(why) = due_now(&r.params, &r.utxo, pmt, with_key) {
            if single {
                io.save(file)?;
                return Err(format!("Tresor {}: {why}", r.id));
            }
            continue;
        }
        let funds = if with_key { io.funds().await } else { None };
        let due = r.utxo.state.next_due;
        let what = format!("Tresor {}: {} KAS für den Termin {}", r.id, crate::abo::fmt_amount(r.params.amount as u64), fmt_time(due));
        let paid = match pay(&r.params, &r.utxo, &r.payload(), funds.as_ref(), net) {
            Ok(p) => p,
            Err(e) => {
                // später erneut (Agent/Seite), nicht jede Minute
                let rr = &mut file.tresore[i];
                rr.last_error = Some(e.clone());
                rr.retry_after = Some(io.now_ms() + RETRY_AFTER_ERROR_MS);
                io.say(&format!("{what}: {e}"));
                reports.push(rep(false, false, e.clone(), None));
                if single {
                    io.save(file)?;
                    return Err(e);
                }
                continue;
            }
        };
        let mut next = file.clone();
        {
            let nr = &mut next.tresore[i];
            nr.utxo = paid.next.clone();
            nr.last_error = None;
            nr.retry_after = None;
            nr.last_paid_ms = Some(io.now_ms());
            nr.push(TresorHist {
                at: now.into(),
                action: "pay".into(),
                txid: Some(paid.built.tx.id().to_string()),
                due: Some(due),
                note: (!paid.fee_from_tresor).then(|| "Gebühr vom Auslöser".into()),
            });
        }
        if !paid.fee_from_tresor {
            io.say("  Hinweis: der Tresor trägt die Gebühr nicht mehr selbst; sie kommt vom eigenen Schlüssel.");
        }
        io.save(file)?;
        let txid = paid.built.tx.id().to_string();
        match io.send(&what, &paid.built, &next).await {
            Ok(()) => {
                if !io.dry_run() {
                    *file = next;
                }
                reports.push(rep(true, true, format!("{what} gezahlt{}", if io.dry_run() { " (Probelauf)" } else { "" }), Some(txid)));
            }
            Err(e) => {
                // Automatische Läufe warten danach (der nächste klärt das Journal).
                if e != "abgebrochen" {
                    let rr = &mut file.tresore[i];
                    rr.last_error = Some(e.clone());
                    rr.retry_after = Some(io.now_ms() + RETRY_AFTER_ERROR_MS);
                    io.save(file)?;
                }
                reports.push(rep(false, false, format!("{what}: {e}"), Some(txid)));
                if single {
                    return Err(e);
                }
                // Journal noch offen (Tx vielleicht im Netz): weitere Sendungen
                // erst nach dessen Klärung. Sonst – meist hat ein anderer
                // Auslöser schneller gezahlt – weiter mit dem nächsten Tresor
                // (A12-16; vorher beendete jeder Fehler die ganze Runde).
                if !continue_after_send_error(&e, io.journal_open()) {
                    break;
                }
            }
        }
    }
    for r in &reports {
        io.say(&format!("Tresor {}: {}", r.id, r.text));
    }
    if reports.is_empty() {
        io.say("Keine fälligen Tresor-Zahlungen.");
    }
    io.save(file)?;
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_hin_und_zurueck() {
        for n in 0..40usize {
            let data: Vec<u8> = (0..n as u8).map(|i| i.wrapping_mul(37).wrapping_add(n as u8)).collect();
            let e = b64_encode(&data);
            assert!(e.bytes().all(|b| B64.contains(&b)), "nur URL-sichere Zeichen");
            assert_eq!(b64_decode(&e).unwrap(), data, "Länge {n}");
        }
        assert_eq!(b64_encode(b"Miete"), "TWlldGU");
        assert!(b64_decode("A").is_none(), "unvollständig");
        assert!(b64_decode("TWl+dGU").is_none(), "fremdes Zeichen");
    }

    fn x(n: u8) -> Vec<u8> {
        let secp = secp256k1::Secp256k1::new();
        let k = secp256k1::Keypair::from_secret_key(&secp, &secp256k1::SecretKey::from_slice(&[n; 32]).unwrap());
        crate::ops::xonly(&k)
    }

    fn sample() -> TresorRec {
        let text = "Miete „Whg. 3“ – Oktober";
        let p = TresorParams {
            owner: x(1),
            recipient: x(2),
            amount: 10 * 100_000_000,
            anchor_day: 31,
            period_ms: 0,
            max_fee: DEFAULT_MAX_FEE,
            payload_hash: payload_hash(text.as_bytes()),
        };
        let t = Tracked {
            outpoint: TransactionOutpoint::new(Hash::from_bytes([3; 32]), 1),
            value: 50 * 100_000_000,
            cov: Hash::from_bytes([0xab; 32]),
            state: TresorState { next_due: 1_801_353_600_000, left: 12 }, // 2027-01-31 00:00 UTC
        };
        TresorRec::new(p, t, text.into(), true, Some("keys/a.json".into()), "2027-01-01 10:00")
    }

    #[test]
    fn code_hin_und_zurueck() {
        let r = sample();
        let c = TresorCode::of("testnet-10", &r);
        let s = c.encode();
        assert!(s.starts_with(CODE_PREFIX));
        assert!(s[CODE_PREFIX.len()..].bytes().all(|b| B64.contains(&b)), "kopierbar, ohne +/=");
        assert!(!s.contains("keys/"), "der Pfad der Schlüsseldatei gehört nicht in den Code");
        assert_eq!(TresorCode::decode(&s).unwrap(), c);
        assert_eq!(TresorCode::decode(&format!("  {s}\n")).unwrap(), c, "Leerraum beim Kopieren");
        assert_eq!(id_of(&c.cov), "abababab");
        assert_eq!(r.id, "abababab");
        assert_eq!(parse_outpoint(&c.outpoint).unwrap(), r.utxo.outpoint);
    }

    #[test]
    fn code_ablehnen() {
        let good = TresorCode::of("mainnet", &sample());
        let s = good.encode();
        assert!(TresorCode::decode(&s[1..]).is_err(), "Präfix fehlt");
        assert!(TresorCode::decode(&s[..s.len() - 7]).is_err(), "abgeschnitten");
        assert!(TresorCode::decode("ghost-tresor:2:").is_err(), "leer");
        assert!(TresorCode::decode(&format!("{CODE_PREFIX}{}", "A".repeat(5_000))).is_err(), "zu lang");
        // Hash passend zur geänderten Nachricht: abgelehnt wird der Fehler selbst
        let with = |f: &dyn Fn(&mut TresorCode)| {
            let mut c = good.clone();
            f(&mut c);
            c.params.payload_hash = payload_hash(&bound_payload(&c.message, c.onchain, &c.sealed));
            TresorCode::decode(&c.encode())
        };
        assert!(with(&|c| c.params.anchor_day = 32).is_err(), "Tag 32");
        assert!(with(&|c| c.params.period_ms = 5).is_err(), "Tag und Intervall zugleich");
        assert!(with(&|c| c.params.amount = 0).is_err(), "Betrag 0");
        assert!(with(&|c| c.params.recipient = vec![0xff; 32]).is_err(), "kein Schlüssel");
        assert!(with(&|c| c.params.owner = vec![1; 31]).is_err(), "Länge");
        assert!(with(&|c| c.state.left = -2).is_err(), "Anzahl");
        assert!(with(&|c| c.state.next_due = 1_000).is_err(), "Termin im DAA-Bereich");
        assert!(with(&|c| c.message = "a\nb".into()).is_err(), "Steuerzeichen");
        assert!(with(&|c| c.message = "\u{202E}gnuhcer".into()).is_err(), "Richtungswechsel");
        assert!(with(&|c| c.outpoint = "xyz".into()).is_err(), "Outpoint");
        assert!(with(&|c| c.params.anchor_day = 0).is_err(), "Intervall 0");
        assert!(with(&|c| {
            c.params.anchor_day = 0;
            c.params.period_ms = 7 * standing::DAY_MS
        })
        .is_ok());
    }

    #[test]
    fn faellig_offline() {
        let mut r = sample();
        let due = r.utxo.state.next_due;
        assert!(!r.looks_due(due, false), "Past Median Time hinkt nach");
        assert!(r.looks_due(due + PMT_LAG_MS, false));
        r.retry_after = Some(due + PMT_LAG_MS + 1);
        assert!(!r.looks_due(due + PMT_LAG_MS, false), "Wartezeit nach Fehlschlag");
        r.retry_after = None;
        r.missing = Some("x".into());
        r.missing_ms = Some(due);
        r.retry_after = Some(due + PMT_LAG_MS + 1);
        assert!(!r.looks_due(due + PMT_LAG_MS, false), "nicht auffindbar, erneutes Nachsehen noch nicht fällig");
        r.missing = None;
        r.missing_ms = None;
        r.retry_after = None;
        r.utxo.state.left = 0;
        assert!(!r.looks_due(due + PMT_LAG_MS, true), "alles erledigt");
        r.utxo.state.left = 3;
        // 10 KAS + 1 KAS Rest: nur mit eigener Gebühr
        r.utxo.value = 11 * 100_000_000;
        assert!(!r.looks_due(due + PMT_LAG_MS, false));
        assert!(r.looks_due(due + PMT_LAG_MS, true));
        let f = TresorFile { version: 1, network: "mainnet".into(), tresore: vec![r.clone()] };
        assert!(needs_run(&f, due + PMT_LAG_MS, true));
        assert!(!needs_run(&f, due + PMT_LAG_MS, false));
        r.ended = Some("x".into());
        assert!(!r.looks_due(due + PMT_LAG_MS, true), "gekündigt");
    }

    #[test]
    fn kandidaten_nur_fuer_erreichte_termine() {
        let r = sample();
        let (p, s) = (&r.params, r.utxo.state);
        assert_eq!(candidates(p, &s, s.next_due - PMT_LAG_MS), vec![s], "noch nicht fällig: nur der bekannte Zustand");
        let c = candidates(p, &s, s.next_due + 70 * standing::DAY_MS);
        assert_eq!(c.len(), 4, "drei Termine erreicht: {c:?}");
        assert_eq!(c[3].left, 9);
        let last = TresorState { left: 1, ..s };
        let c = candidates(p, &last, s.next_due + 400 * standing::DAY_MS);
        assert_eq!(c.len(), 2, "nach der letzten Zahlung kein weiterer Zustand");
        assert_eq!(c[1].left, 0);
        let inf = TresorState { left: -1, ..s };
        assert_eq!(candidates(p, &inf, standing::MAX_TIME).len(), MAX_FOLLOW, "begrenzt");
    }

    #[test]
    fn datei_und_suche() {
        assert_eq!(path_for(Path::new("deployments/mainnet.json")), Path::new("deployments/mainnet-tresore.json"));
        let mut f = TresorFile::empty("mainnet");
        let r = sample();
        assert_eq!(f.upsert(r.clone()), 0);
        let mut moved = r.clone();
        moved.utxo.value = 1;
        moved.message = String::new();
        assert_eq!(f.upsert(moved), 0, "gleiche Covenant-ID: aktualisieren statt verdoppeln");
        assert_eq!(f.tresore.len(), 1);
        assert_eq!(f.tresore[0].utxo.value, 1);
        assert_eq!(f.tresore[0].message, r.message, "Beschreibung bleibt");
        assert_eq!(f.find("ABABABAB").unwrap(), 0);
        assert_eq!(f.find(&r.utxo.cov.to_string()).unwrap(), 0);
        assert!(f.find("abab").is_err());
        let back: TresorFile = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back.tresore[0].params, r.params);
        assert!(!serde_json::to_string(&f).unwrap().contains("secret"));
    }

    /// A12-1 im Vertrag: `tresor open` bindet genau den Payload, den jede
    /// Zahlung tragen wird (öffentlich, verschlüsselt, ohne Nachricht)
    #[test]
    fn a13_nachricht_beim_anlegen_gebunden() {
        let base = sample().params;
        let mut p = base.clone();
        assert_eq!(bind_message(&mut p, "Miete", true, b"Miete").unwrap(), "");
        assert_eq!(p.payload_hash, payload_hash(b"Miete"));
        let blob = [crate::message::MAGIC.to_vec(), vec![7; 80]].concat();
        let mut p = base.clone();
        assert_eq!(bind_message(&mut p, "Miete", false, &blob).unwrap(), faster_hex::hex_string(&blob));
        assert_eq!(p.payload_hash, payload_hash(&blob));
        let mut p = base.clone();
        assert_eq!(bind_message(&mut p, "", false, &[]).unwrap(), "");
        assert_eq!(p.payload_hash, payload_hash(&[]));
        assert!(bind_message(&mut base.clone(), "Miete", true, b"Miete Mai").is_err(), "Klartext weicht ab");
        // jede Zahlung trägt genau das Gebundene: pay lehnt anderes ab
        let r = sample();
        let net = &kaspa_consensus_core::config::params::MAINNET_PARAMS;
        assert!(pay(&r.params, &r.utxo, b"Miete", None, net).unwrap_err().contains("hinterlegte Nachricht"));
        assert!(pay(&r.params, &r.utxo, &[], None, net).is_err());
        assert_eq!(pay(&r.params, &r.utxo, &r.payload(), None, net).unwrap().built.tx.payload, r.payload());
    }

    // ------------------------------------------------ Audit 12 (Behebungen) ----

    /// A12-7: „fällig“ nur, wenn `pay` auch zahlt – mit 1 KAS Rest (MIN_KEEP).
    /// Vorher galt ein Tresor schon als zahlbar, sobald Betrag + Höchstgebühr
    /// irgendwie gedeckt waren (10,5 KAS für 10 KAS), und `pay` scheiterte dann.
    #[test]
    fn a12_faellig_nur_mit_einem_kas_rest() {
        let r = sample();
        let (p, due) = (&r.params, r.utxo.state.next_due + 1);
        let at = |value: u64| Tracked { value, ..r.utxo.clone() };
        // 10,5 KAS für 10 KAS: gar nicht zahlbar, auch nicht mit eigener Gebühr
        assert!(due_now(p, &at(1_050_000_000), due, true).unwrap_err().contains("1 KAS Rest"));
        assert!(!payable(p, 1_050_000_000, true));
        // 11,005 KAS: nur, wenn der Auslöser die Gebühr zahlt
        assert!(due_now(p, &at(1_100_500_000), due, false).unwrap_err().contains("eigenem Schlüssel"));
        assert!(due_now(p, &at(1_100_500_000), due, true).is_ok());
        assert!(!fee_from_tresor(p, 1_100_500_000));
        // 11,01 KAS: der Tresor trägt alles
        assert!(due_now(p, &at(1_101_000_000), due, false).is_ok());
        assert!(fee_from_tresor(p, 1_101_000_000));
        // dieselbe Grenze wie looks_due und payments_covered
        let mut rr = r.clone();
        rr.utxo.value = 1_100_999_999;
        assert!(!rr.looks_due(due + PMT_LAG_MS, false) && rr.looks_due(due + PMT_LAG_MS, true));
        assert_eq!(payments_covered(p, 1_100_999_999), 0);
        assert_eq!(payments_covered(p, 1_101_000_000), 1);
    }

    /// A12-16: Import prüft wie das Anlegen. Vorher nahm `decode` 1 ms Intervall,
    /// 1 sompi Betrag und 100 KAS Höchstgebühr an (bis zu 2000 Node-Abfragen je Abgleich).
    #[test]
    fn a12_import_prueft_wie_das_anlegen() {
        let good = TresorCode::of("mainnet", &sample());
        let with = |f: &dyn Fn(&mut TresorCode)| {
            let mut c = good.clone();
            f(&mut c);
            TresorCode::decode(&c.encode())
        };
        let period = |ms: i64| {
            move |c: &mut TresorCode| {
                c.params.anchor_day = 0;
                c.params.period_ms = ms;
            }
        };
        assert!(with(&period(1)).is_err(), "1 ms");
        assert!(with(&period(standing::DAY_MS - 1)).is_err(), "unter einem Tag");
        assert!(with(&period(3651 * standing::DAY_MS)).is_err(), "über 10 Jahre");
        assert!(with(&period(standing::DAY_MS)).is_ok(), "täglich");
        assert!(with(&|c| c.params.amount = 1).is_err(), "1 sompi");
        assert!(with(&|c| c.params.amount = MIN_AMOUNT - 1).is_err());
        assert!(with(&|c| c.params.amount = MIN_AMOUNT).is_ok());
        assert!(with(&|c| c.params.max_fee = 100 * 100_000_000).is_err(), "100 KAS Höchstgebühr");
        assert!(with(&|c| c.params.max_fee = MAX_MAX_FEE + 1).is_err());
        assert!(with(&|c| c.params.max_fee = 0).is_err(), "Höchstgebühr 0 zahlt nie");
        assert!(with(&|c| c.params.max_fee = MAX_MAX_FEE).is_ok());
        assert!(with(&|c| c.params.recipient = c.params.owner.clone()).is_err(), "an sich selbst");
        assert!(with(&|c| c.state.next_due = standing::MAX_TIME + 1).is_err(), "nach 2200");
        assert!(with(&|c| c.state.left = 0).is_ok(), "fortgeschrittener Zustand: alle Zahlungen erledigt");
        // Nachführen nach dem Import: höchstens ein Kandidat je erreichtem Tag
        let c = with(&period(standing::DAY_MS)).unwrap();
        let n = candidates(&c.params, &c.state, c.state.next_due + 30 * standing::DAY_MS).len();
        assert!(n <= 32, "{n} Kandidaten für 30 Tage");
    }

    /// A12-16: Ein Tresor, der beim Abgleich fehlte, bleibt für die Automatik
    /// nicht für immer liegen: stündlich erneut, eine Woche lang.
    #[test]
    fn a12_nicht_auffindbar_wird_wieder_gesucht() {
        let mut r = sample();
        let now = 1_900_000_000_000;
        r.missing = Some("2030-03-17 10:00".into());
        r.missing_ms = Some(now - 3_600_000);
        r.retry_after = Some(now - 1);
        assert!(r.recheck_missing(now) && r.looks_due(now, false), "Wartezeit abgelaufen");
        r.retry_after = Some(now + 1);
        assert!(!r.looks_due(now, true), "noch nicht wieder");
        r.retry_after = None;
        r.missing_ms = Some(now - MISSING_RECHECK_FOR_MS - 1);
        assert!(!r.looks_due(now, true), "nach einer Woche nur noch von Hand");
        r.missing_ms = None;
        assert!(r.looks_due(now, false), "ältere Datei ohne Zeitpunkt: einmal nachsehen");
        r.ended = Some("x".into());
        assert!(!r.looks_due(now, true), "gekündigt: nie");
        let f = TresorFile { version: 1, network: "mainnet".into(), tresore: vec![TresorRec { ended: None, ..r.clone() }] };
        assert!(needs_run(&f, now, false));
        // wieder gefunden: upsert löscht die Markierung
        let mut f = f;
        f.upsert(sample());
        assert!(f.tresore[0].missing.is_none() && f.tresore[0].missing_ms.is_none());
    }

    /// A12-16: Nach einem Sendefehler geht die Runde weiter, außer das Journal
    /// ist noch offen (Tx vielleicht im Netz) oder der Nutzer hat abgebrochen.
    #[test]
    fn a12_weiter_nach_sendefehler() {
        assert!(continue_after_send_error("Transaktion abgelehnt: UTXO schon verbraucht", false));
        assert!(!continue_after_send_error("Zeitüberschreitung", true), "Journal offen");
        assert!(!continue_after_send_error("abgebrochen", false));
    }

    /// A12-1: Tresor-Zahlungen am Redeem-Skript erkennen – auch ohne die
    /// Parameter zu kennen – und nichts anderes dafür halten
    #[test]
    fn a12_tresor_am_skript_erkannt() {
        let r = sample();
        for (p, s) in [
            (r.params.clone(), r.utxo.state),
            (
                TresorParams { amount: MIN_AMOUNT, anchor_day: 0, period_ms: standing::DAY_MS, max_fee: 1, ..r.params.clone() },
                TresorState { next_due: 1_900_000_000_000, left: -1 },
            ),
            (TresorParams { amount: 123_456_789_012, max_fee: MAX_MAX_FEE, ..r.params.clone() }, TresorState { left: 0, ..r.utxo.state }),
        ] {
            let code = bytecode(&standing_order(&p, &s));
            assert_eq!(parse_script(&code), Some((p.clone(), s)));
            // Signaturskript des Eingangs: Argumente, dann das Redeem-Skript
            let sig = [vec![0x51], vec![0x4d, (code.len() & 0xff) as u8, (code.len() >> 8) as u8], code.clone()].concat();
            assert_eq!(redeem_script(&sig), Some(code.clone()));
            for i in [0, 1, 20, code.len() / 2, code.len() - 1] {
                let mut bad = code.clone();
                bad[i] ^= 0x01;
                assert_eq!(parse_script(&bad), None, "Byte {i} verändert");
            }
            assert_eq!(parse_script(&code[..code.len() - 1]), None);
        }
        // andere Verträge und Signaturen sind keine Tresore
        let token = bytecode(&GhostTok::to_pubkey(&[7u8; 32], 5).artifact());
        assert_eq!(parse_script(&token), None);
        let minter = bytecode(&GhostTok::minter_of(&Hash::from_bytes([9; 32])).artifact());
        assert_eq!(parse_script(&minter), None);
        assert_eq!(parse_script(&[0u8; 64]), None);
        assert_eq!(redeem_script(&[0x40]), None, "abgeschnitten");
        assert_eq!(redeem_script(&[0xac]), None, "kein Push");
    }

    // ------------------------------------- Audit 12, Nachprüfung (Gruppe a) ----

    /// Befehl wieder zu Bytes (Umkehrung von `tokens`)
    fn untoken(t: &Tok) -> Vec<u8> {
        match t.op {
            0x01..=0x4b => [vec![t.op], t.data.clone()].concat(),
            0x4c => [vec![0x4c, t.data.len() as u8], t.data.clone()].concat(),
            0x4d => [vec![0x4d], (t.data.len() as u16).to_le_bytes().to_vec(), t.data.clone()].concat(),
            0x4e => [vec![0x4e], (t.data.len() as u32).to_le_bytes().to_vec(), t.data.clone()].concat(),
            op => vec![op],
        }
    }

    /// Nachprüfung zu A12-1: `parse_script` liest die längenabhängigen Stellen
    /// (Sprungweiten, Längen) nur als irgendeine Zahl. Erst die Neuübersetzung
    /// mit byte-genauem Vergleich lehnt ein Skript ab, das genau dort abweicht –
    /// ohne sie gälte es als Tresor mit den gelesenen Parametern.
    #[test]
    fn a12n_laengenabhaengige_stellen_nur_ueber_die_neuuebersetzung() {
        let r = sample();
        let (p, s) = (r.params.clone(), r.utxo.state);
        let code = bytecode(&standing_order(&p, &s));
        let l = layout().expect("Bauplan");
        let rest = &code[l.prefix.len()..];
        let body = tokens(&rest[18..]).unwrap();
        // Probe: aus den Befehlen lassen sich die Bytes genau zurückbauen
        assert_eq!([l.prefix.clone(), rest[..18].to_vec(), body.iter().flat_map(untoken).collect()].concat(), code);
        let loose: Vec<usize> = l.slots.iter().filter(|&&(_, w)| w == LOOSE).map(|&(i, _)| i).collect();
        assert!(!loose.is_empty(), "das Skript hat längenabhängige Stellen");
        for i in loose {
            let mut b = body.clone();
            match b[i].op {
                0x01..=0x4e if !b[i].data.is_empty() => b[i].data[0] ^= 0x01,
                0x51..=0x5f => b[i].op += 1,
                op => b[i].op = if op == 0x60 { 0x5f } else { 0x52 },
            }
            assert!(tok_num(&b[i]).is_some(), "weiterhin eine Zahl");
            let forged = [l.prefix.clone(), rest[..18].to_vec(), b.iter().flat_map(untoken).collect()].concat();
            assert_eq!(forged.len(), code.len());
            assert_ne!(forged, code);
            assert_eq!(parse_script(&forged), None, "Stelle {i} verändert: kein Tresor");
        }
        assert_eq!(parse_script(&code), Some((p, s)));
    }

    /// Nachprüfung zu A12-1: nur der Zweig `pay` ist eine Tresor-Zahlung;
    /// `cancel` und `topUp` (mit Signatur des Besitzers) nicht
    #[test]
    fn a12n_nur_der_zweig_pay() {
        let r = sample();
        let art = standing_order(&r.params, &r.utxo.state);
        let code = bytecode(&art);
        let push = kaspa_txscript::script_builder::ScriptBuilder::with_flags(crate::txb::engine_flags()).add_data(&code).unwrap().drain();
        let entry = |name: &str, args: Vec<silverscript_abi::ArtifactValue>| {
            [silverscript_abi::encode_contract_entry_sig_script(&art, &contract_name(&art), name, &args).unwrap(), push.clone()].concat()
        };
        let sig = || silverscript_abi::ArtifactValue::Bytes(vec![7; 65]);
        let pay = entry("pay", vec![]);
        assert_eq!(pay_input(&pay), Some((r.params.clone(), r.utxo.state)));
        assert_eq!(redeem_script(&entry("cancel", vec![sig()])), Some(code.clone()), "dasselbe Redeem-Skript …");
        assert_eq!(parse_script(&code).map(|x| x.0), Some(r.params.clone()), "… und derselbe Tresor …");
        assert_eq!(pay_input(&entry("cancel", vec![sig()])), None, "… aber kein pay");
        assert_eq!(pay_input(&entry("topUp", vec![sig()])), None);
        // pay mit zusätzlichem Push davor: nicht, wie tresor::pay baut
        assert_eq!(pay_input(&[vec![0x51], pay.clone()].concat()), None);
        assert_eq!(pay_input(&push), None, "ohne Selektor");
    }

    /// Ein- und Ausgabe für die Abläufe: UTXOs aus einer Liste, Abfragen gezählt
    struct Fake {
        utxos: Vec<(TransactionOutpoint, UtxoEntry)>,
        lookups: usize,
        now: i64,
        said: Vec<String>,
    }
    impl TresorIo for Fake {
        async fn utxos(&mut self, spk: &ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
            self.lookups += 1;
            Ok(self.utxos.iter().filter(|(_, e)| e.script_public_key == *spk).cloned().collect())
        }
        async fn funds(&mut self) -> Option<Funds> {
            None
        }
        async fn send(&mut self, _: &str, _: &Built, _: &TresorFile) -> Result<(), String> {
            Err("nicht im Test".into())
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
            self.now
        }
        fn say(&mut self, line: &str) {
            self.said.push(line.into());
        }
    }

    /// Tresor mit täglichem Intervall und Termin 1985 (Angriff aus der Nachprüfung)
    fn old_daily() -> TresorRec {
        let mut r = sample();
        r.params.anchor_day = 0;
        r.params.period_ms = standing::DAY_MS;
        r.utxo.state = TresorState { next_due: 500_000_000_000 + standing::DAY_MS, left: -1 };
        r
    }
    const NOW: i64 = 1_790_640_000_000; // 2026-09-29

    /// Nachprüfung zu A12-16: Ein gekündigter Tresor mit altem Termin kostete je
    /// automatischem erneutem Nachsehen MAX_FOLLOW Node-Abfragen, eine Woche
    /// lang stündlich (≈ 336 000). Die Automatik sucht einen fehlenden Tresor
    /// jetzt nur über MISSING_RECHECK_FOLLOW Zustände; der Abgleich von Hand
    /// weiterhin vollständig.
    #[tokio::test]
    async fn a12n_fehlender_tresor_automatik_sucht_begrenzt() {
        let mut r = old_daily();
        let mut io = Fake { utxos: vec![], lookups: 0, now: NOW, said: vec![] };
        // erstes Fehlen (Tresor bekannt, nicht markiert): volle Suche
        assert!(follow(&mut io, &mut r, NOW, "x", Search::Auto).await.unwrap().is_some());
        assert_eq!(io.lookups, MAX_FOLLOW, "erstes Fehlen: vollständig");
        assert!(r.missing.is_some() && r.recheck_missing(NOW + MISSING_RECHECK_MS));
        // die Automatik sieht stündlich nach – jetzt begrenzt
        io.lookups = 0;
        io.now = NOW + MISSING_RECHECK_MS;
        assert!(follow(&mut io, &mut r, NOW + MISSING_RECHECK_MS, "x", Search::Auto).await.unwrap().is_none(), "nur einmal gemeldet");
        assert_eq!(io.lookups, MISSING_RECHECK_FOLLOW);
        let week = (MISSING_RECHECK_FOR_MS / MISSING_RECHECK_MS) as usize;
        println!("Woche erneutes Nachsehen: {} Abfragen statt {}", week * MISSING_RECHECK_FOLLOW, week * MAX_FOLLOW);
        // von Hand: vollständig
        io.lookups = 0;
        follow(&mut io, &mut r, NOW, "x", Search::Full).await.unwrap();
        assert_eq!(io.lookups, MAX_FOLLOW);
        // wieder da (kurzer Aussetzer des Nodes): die begrenzte Suche findet ihn
        let spk = TresorShape::of(&r.params).spk(&r.utxo.state);
        io.utxos = vec![(r.utxo.outpoint, UtxoEntry::new(r.utxo.value, spk, 0, false, Some(r.utxo.cov)))];
        io.lookups = 0;
        assert!(follow(&mut io, &mut r, NOW, "x", Search::Auto).await.unwrap().is_some());
        assert_eq!(io.lookups, 1);
        assert!(r.missing.is_none() && r.retry_after.is_none());
    }

    /// Zweite Nachprüfung zu A12-16 (Rückbau R20 überlebte): Ohne die Wartezeit
    /// beim Fehlen wäre `recheck_missing` in jeder Runde wahr – der Agent und
    /// der Zeitgeber der Seite (`tresorNeedsRun`) sähen den Tresor jede Runde
    /// nach statt stündlich. Der obige Test prüfte nur den Zeitpunkt nach einer
    /// Stunde, der auch ohne Wartezeit gilt.
    #[tokio::test]
    async fn a12p_fehlender_tresor_hoechstens_stuendlich() {
        let mut r = old_daily();
        let mut io = Fake { utxos: vec![], lookups: 0, now: NOW, said: vec![] };
        follow(&mut io, &mut r, NOW, "x", Search::Auto).await.unwrap();
        assert_eq!(r.retry_after, Some(NOW + MISSING_RECHECK_MS));
        let mut f = TresorFile::empty("mainnet");
        f.tresore.push(r.clone());
        for later in [1, 60_000, MISSING_RECHECK_MS - 1] {
            assert!(!r.recheck_missing(NOW + later), "{later} ms danach: noch nicht");
            assert!(!needs_run(&f, NOW + later, true), "{later} ms danach: kein Lauf der Automatik");
        }
        assert!(r.recheck_missing(NOW + MISSING_RECHECK_MS) && needs_run(&f, NOW + MISSING_RECHECK_MS, true));
        // nach jedem erneuten Nachsehen wieder eine Stunde Pause
        io.now = NOW + MISSING_RECHECK_MS;
        follow(&mut io, &mut r, NOW + MISSING_RECHECK_MS, "x", Search::Auto).await.unwrap();
        assert_eq!(r.retry_after, Some(NOW + 2 * MISSING_RECHECK_MS));
        assert!(!r.recheck_missing(NOW + MISSING_RECHECK_MS + 1));
    }

    /// Nachprüfung zu A12-16 (Beleg `pruef_import_alter_termin_2000_abfragen`):
    /// Ein Code mit Termin 1985 und täglichem Intervall ging durch den Import.
    /// Jetzt lehnt der Import einen Rückstand über MAX_IMPORT_BACKLOG ab.
    #[tokio::test]
    async fn a12n_import_mit_altem_termin_abgelehnt() {
        let r = old_daily();
        let code = TresorCode::of("mainnet", &r).encode();
        assert!(TresorCode::decode(&code).is_ok(), "Form in Ordnung, erst der Rückstand fällt auf");
        let spk = TresorShape::of(&r.params).spk(&r.utxo.state);
        let utxo = (r.utxo.outpoint, UtxoEntry::new(r.utxo.value, spk, 0, false, Some(r.utxo.cov)));
        let mut io = Fake { utxos: vec![utxo], lookups: 0, now: NOW, said: vec![] };
        let mut f = TresorFile::empty("mainnet");
        let e = import(&mut io, &mut f, &code, "mainnet", NOW, "x", None).await.unwrap_err();
        assert!(e.contains("nicht gezahlt"), "{e}");
        assert!(f.tresore.is_empty());
        // Grenze: genau MAX_IMPORT_BACKLOG Termine offen geht, einer mehr nicht
        let at = |n: i64| r.utxo.state.next_due + n * standing::DAY_MS - PMT_LAG_MS;
        let n_ok = (1..3_000).find(|&n| backlog(&r.params, &r.utxo.state, at(n)) == MAX_IMPORT_BACKLOG).unwrap();
        assert!(check_backlog(&r.params, &r.utxo.state, at(n_ok)).is_ok());
        assert!(check_backlog(&r.params, &r.utxo.state, at(n_ok + 1)).is_err());
        // ein regulärer Tresor (Termin in der Zukunft) wird übernommen
        let mut fresh = r.clone();
        fresh.utxo.state.next_due = NOW + standing::DAY_MS;
        let spk = TresorShape::of(&fresh.params).spk(&fresh.utxo.state);
        io.utxos = vec![(fresh.utxo.outpoint, UtxoEntry::new(fresh.utxo.value, spk, 0, false, Some(fresh.utxo.cov)))];
        let i = import(&mut io, &mut f, &TresorCode::of("mainnet", &fresh).encode(), "mainnet", NOW, "x", None).await.unwrap();
        assert_eq!(f.tresore[i].utxo.state, fresh.utxo.state);
        assert!(import(&mut io, &mut f, &TresorCode::of("testnet-10", &fresh).encode(), "mainnet", NOW, "x", None).await.is_err(), "anderes Netz");
    }

    // --------------------------------------- Audit 12, Restpunkte (Gruppe a) ----

    /// UTXO eines Tresors, wie der Node sie liefert
    fn utxo_of(r: &TresorRec) -> (TransactionOutpoint, UtxoEntry) {
        let spk = TresorShape::of(&r.params).spk(&r.utxo.state);
        (r.utxo.outpoint, UtxoEntry::new(r.utxo.value, spk, 0, false, Some(r.utxo.cov)))
    }

    /// Restpunkt „Import-alter-Tresor-Codes“: Die Beschreibung eines vor der
    /// Verschärfung des Filters (A12-11) angelegten Tresors darf heute
    /// verbotene Zeichen enthalten (Tastenkappe, VS16 hinter Buchstaben,
    /// U+2028, Private Use). Der Import lehnte seinen Code ab; der Empfänger
    /// konnte ihn weder in die Liste holen noch über die Seite abholen. Jetzt wird er
    /// übernommen: gespeichert wie im Code (Vergleichstext des Eingangs),
    /// angezeigt bereinigt und gekennzeichnet. Was ghostctl nie angenommen
    /// hat, und die Vertragsdaten bleiben streng geprüft.
    #[tokio::test]
    async fn a13_alte_codes_mit_heute_verbotenen_zeichen_uebernommen() {
        let base = sample();
        let pmt = base.utxo.state.next_due - 1;
        for text in ["Miete 1\u{fe0f}\u{20e3}", "Miete\u{2028}Mai", "a\u{e000}b", "Platz A\u{fe0f}", "Gruß \u{1f44d}\u{fe0e}\u{fe0f}"] {
            assert!(crate::abo::check_message(text).is_err(), "{text:?} gilt heute als unzulässig");
            for onchain in [true, false] {
                let mut r = base.clone();
                (r.message, r.onchain) = (text.into(), onchain);
                if !onchain {
                    // verschlüsselt: eine Beschreibung gibt es nur mit
                    // verschlüsselter Fassung (A13-tresor-2); deren Inhalt
                    // prüft ohne Empfängerschlüssel niemand
                    let mut blob = crate::message::MAGIC.to_vec();
                    blob.resize(crate::message::OVERHEAD + text.len(), 7);
                    r.sealed = faster_hex::hex_string(&blob);
                }
                r.params.payload_hash = payload_hash(&r.payload());
                let code = TresorCode::of("mainnet", &r).encode();
                assert_eq!(TresorCode::decode(&code).expect("alter Code lesbar").message, text);
                let mut io = Fake { utxos: vec![utxo_of(&r)], lookups: 0, now: NOW, said: vec![] };
                let mut f = TresorFile::empty("mainnet");
                let i = import(&mut io, &mut f, &code, "mainnet", pmt, "x", None).await.expect("übernommen");
                let t = &f.tresore[i];
                assert_eq!(t.message, text, "gespeichert wie im Code");
                assert_eq!(t.payload(), r.payload(), "dieselbe Nachricht in jeder Zahlung wie beim Absender");
                let (shown, cleaned) = t.shown_message();
                assert!(cleaned, "{text:?}: gekennzeichnet");
                assert_eq!(shown, crate::abo::sendable_message(text));
                assert!(crate::abo::check_message(&shown).is_ok() && shown != text);
                assert!(io.said.iter().any(|s| s.contains("heute nicht mehr erlaubt") && s.contains(&shown)), "{:?}", io.said);
            }
        }
        let mut r = base.clone();
        r.message = "Miete\u{2028}Mai".into();
        assert_eq!(r.shown_message(), ("Miete Mai".into(), true));
        // die Fälle unten scheitern an der Beschreibung selbst, nicht am Hash
        let rebind = |c: &mut TresorCode| c.params.payload_hash = payload_hash(&bound_payload(&c.message, c.onchain, &c.sealed));
        // heute zulässige Beschreibung: unverändert, ohne Kennzeichen und ohne Hinweis
        assert_eq!(base.shown_message(), (base.message.clone(), false));
        let mut io = Fake { utxos: vec![utxo_of(&base)], lookups: 0, now: NOW, said: vec![] };
        import(&mut io, &mut TresorFile::empty("mainnet"), &TresorCode::of("mainnet", &base).encode(), "mainnet", pmt, "x", None).await.unwrap();
        assert!(io.said.is_empty(), "{:?}", io.said);
        // was ghostctl nie angenommen hat, bleibt abgelehnt, ebenso zu lange
        // Texte. Dazu gehört ZWJ (U+200D): Er stand schon im alten Filter, ein
        // ZWJ-Emoji konnte also in keinem Tresor stehen.
        let with = |f: &dyn Fn(&mut TresorCode)| {
            let mut c = TresorCode::of("mainnet", &base);
            f(&mut c);
            rebind(&mut c);
            TresorCode::decode(&c.encode())
        };
        for bad in ["a\nb", "\u{202E}gnuhcer", "a\u{200B}b", "Gruß \u{1f468}\u{200d}\u{1f469}", "a\u{2066}b", "a\u{00AD}b", "a\u{FEFF}"] {
            assert!(with(&|c| c.message = bad.into()).is_err(), "{bad:?}");
        }
        assert!(with(&|c| c.message = "x".repeat(crate::abo::MAX_MESSAGE_CHARS + 1)).is_err(), "zu lang");
        assert!(with(&|c| c.message = "x".repeat(crate::abo::MAX_MESSAGE_CHARS)).is_ok());
        // Vertragsdaten und verschlüsselte Fassung unverändert streng, auch mit alter Beschreibung
        let legacy = |c: &mut TresorCode| c.message = "Miete 1\u{fe0f}\u{20e3}".into();
        assert!(with(&|c| {
            legacy(c);
            c.params.amount = MIN_AMOUNT - 1
        })
        .is_err());
        assert!(with(&|c| {
            legacy(c);
            c.params.max_fee = MAX_MAX_FEE + 1
        })
        .is_err());
        assert!(with(&|c| {
            legacy(c);
            c.sealed = "zz".into()
        })
        .is_err());
    }

    /// Restpunkt „upsert-retry_after“: Übernimmt der Empfänger den Code eines
    /// als fehlend markierten Tresors erneut (er ist wieder da), fiel zwar die
    /// Markierung weg, die Wartezeit des erneuten Nachsehens aber nicht: Die
    /// Automatik (looks_due, auf dem Server tresorNeedsRun) wartete danach
    /// bis zu einer Stunde. Jetzt wie in `follow`.
    #[tokio::test]
    async fn a13_erneut_uebernommen_ohne_wartezeit() {
        let r = sample();
        let now = r.utxo.state.next_due + PMT_LAG_MS + 60_000;
        let pmt = now - PMT_LAG_MS;
        let mut f = TresorFile::empty("mainnet");
        f.upsert(r.clone());
        // beim Abgleich fehlte er (Aussetzer des Nodes): stündliche Wartezeit
        let mut io = Fake { utxos: vec![], lookups: 0, now, said: vec![] };
        follow(&mut io, &mut f.tresore[0], pmt, "x", Search::Auto).await.unwrap();
        assert_eq!(f.tresore[0].retry_after, Some(now + MISSING_RECHECK_MS));
        assert!(!needs_run(&f, now + 1, false));
        // Code erneut übernommen, der Tresor ist wieder am Node
        io.utxos = vec![utxo_of(&r)];
        assert_eq!(import(&mut io, &mut f, &TresorCode::of("mainnet", &r).encode(), "mainnet", pmt, "y", None).await.unwrap(), 0);
        let t = &f.tresore[0];
        assert!(t.missing.is_none() && t.missing_ms.is_none());
        assert_eq!(t.retry_after, None, "Wartezeit des erneuten Nachsehens entfällt");
        assert!(t.looks_due(now + 1, false) && needs_run(&f, now + 1, false), "die Automatik zahlt gleich, nicht erst in einer Stunde");
        // direkt über upsert ebenso
        let mut g = TresorFile::empty("mainnet");
        g.upsert(TresorRec { missing: Some("x".into()), missing_ms: Some(now), retry_after: Some(now + MISSING_RECHECK_MS), ..r.clone() });
        g.upsert(r.clone());
        assert_eq!((g.tresore[0].missing.clone(), g.tresore[0].retry_after), (None, None));
        // die Wartezeit nach einem Fehlschlag beim Auslösen bleibt (der Tresor war nicht verschwunden)
        let mut g = TresorFile::empty("mainnet");
        g.upsert(TresorRec { retry_after: Some(now + RETRY_AFTER_ERROR_MS), ..r.clone() });
        g.upsert(r.clone());
        assert_eq!(g.tresore[0].retry_after, Some(now + RETRY_AFTER_ERROR_MS));
    }
}
