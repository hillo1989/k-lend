//! Daueraufträge: KAS oder GHOST in festen Abständen senden, optional mit
//! Nachricht (z. B. „Miete Oktober“).
//!
//! Die Aufträge liegen in `deployments/<netz>-abos.json` neben der
//! Zustandsdatei. Die Datei enthält KEINE Schlüssel, nur den Pfad der
//! Schlüsseldatei des Absenders.
//!
//! Sicherheit gegen Doppelzahlung:
//! - Ein Lauf (`run`) hält eine eigene Sperre; zwei gleichzeitige Läufe
//!   (Agent und Seite) arbeiten nacheinander, der zweite findet nichts Fälliges.
//! - Der Termin wird VOR dem Bauen und Senden fortgeschrieben und die Zahlung
//!   als `inflight` festgehalten, samt TXID, sobald die Tx gebaut ist. Bricht
//!   der Lauf danach ab, klärt der nächste Lauf am Netz, ob sie angenommen
//!   wurde, und sendet im Zweifel NICHT erneut.
//! - Verpasste Termine (Rechner war aus) werden genau einmal nachgeholt; die
//!   übrigen gelten als übersprungen und stehen so im Verlauf.
//! - Scheitert eine Zahlung sicher ohne Senden, folgt ein neuer Versuch nach
//!   einer Wartezeit; nach `MAX_ATTEMPTS` Versuchen wird der Auftrag pausiert.

use crate::store;
use chrono::{Days, Months, NaiveDate};
use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Höchstlänge der Nachricht in Zeichen (UTF-8 höchstens 4 Byte je Zeichen,
/// passt also immer in txb::MAX_PAYLOAD)
pub const MAX_MESSAGE_CHARS: usize = 100;
/// Versuche je Termin, danach wird pausiert
pub const MAX_ATTEMPTS: u32 = 3;
/// Wartezeit vor dem nächsten Versuch, je bisherigem Fehlversuch
pub const RETRY_WAIT_SECS: u64 = 600;
/// Einträge im Verlauf je Auftrag
pub const MAX_HISTORY: usize = 100;
/// Längster Abstand in Tagen (10 Jahre)
pub const MAX_INTERVAL_DAYS: u32 = 3650;

// -------------------------------------------------------------- Datenmodell ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Asset {
    #[serde(rename = "KAS")]
    Kas,
    #[serde(rename = "GHOST")]
    Ghost,
}

impl Asset {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "kas" => Ok(Asset::Kas),
            "ghost" | "ghst" => Ok(Asset::Ghost),
            _ => Err(format!("--asset: KAS oder GHOST erwartet, nicht „{s}“")),
        }
    }
    pub fn unit(self) -> &'static str {
        match self {
            Asset::Kas => "KAS",
            Asset::Ghost => "GHOST",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Every {
    Daily,
    Weekly,
    Monthly,
}

/// "daily" | "weekly" | "monthly" | {"days": n}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Interval {
    Named(Every),
    Days { days: u32 },
}

impl Interval {
    /// täglich/daily, wöchentlich/weekly, monatlich/monthly oder eine Zahl (Tage)
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim().to_lowercase();
        let named = match t.as_str() {
            "daily" | "täglich" | "taeglich" => Some(Every::Daily),
            "weekly" | "wöchentlich" | "woechentlich" => Some(Every::Weekly),
            "monthly" | "monatlich" => Some(Every::Monthly),
            _ => None,
        };
        if let Some(e) = named {
            return Ok(Interval::Named(e));
        }
        let digits = t.strip_suffix('d').unwrap_or(&t);
        match digits.parse::<u32>() {
            Ok(n) if (1..=MAX_INTERVAL_DAYS).contains(&n) => Ok(Interval::Days { days: n }),
            _ => Err(format!("--interval: daily, weekly, monthly oder Anzahl Tage (1–{MAX_INTERVAL_DAYS}) erwartet, nicht „{s}“")),
        }
    }
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Interval::Named(Every::Daily) => write!(f, "täglich"),
            Interval::Named(Every::Weekly) => write!(f, "wöchentlich"),
            Interval::Named(Every::Monthly) => write!(f, "monatlich"),
            Interval::Days { days } => write!(f, "alle {days} Tage"),
        }
    }
}

/// Zahlung, die gerade läuft oder bei einem Abbruch lief (Journal)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InFlight {
    /// Terminnummer und -datum
    pub seq: u32,
    pub date: NaiveDate,
    /// dabei übersprungene (verpasste) Termine
    pub skipped: u32,
    /// Fehlversuche für diesen Termin vor diesem Versuch
    pub attempts: u32,
    /// Terminnummer vor dem Fortschreiben (für den Abbruch durch den Nutzer)
    pub prev_seq: u32,
    pub since: String,
    /// erst gesetzt, wenn die Tx fertig gebaut ist; gesendet wird nur danach
    #[serde(default)]
    pub txid: Option<String>,
    #[serde(default)]
    pub outputs: Vec<(u32, ScriptPublicKey)>,
    #[serde(default)]
    pub inputs: Vec<(TransactionOutpoint, ScriptPublicKey)>,
}

/// Termin, dessen Zahlung sicher nicht gesendet wurde und erneut versucht wird
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Retry {
    pub seq: u32,
    pub date: NaiveDate,
    pub attempts: u32,
    pub skipped: u32,
    /// frühester nächster Versuch (Unix-Sekunden)
    pub after: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hist {
    /// Termin
    pub date: NaiveDate,
    /// Zeitpunkt der Ausführung (lokale Zeit)
    pub at: String,
    pub amount: String,
    #[serde(default)]
    pub txid: Option<String>,
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub skipped: u32,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Abo {
    pub id: String,
    /// Pfad der Schlüsseldatei des Absenders (keine Geheimnisse in dieser Datei)
    pub key: String,
    pub asset: Asset,
    /// Adresse, x-only-Pubkey (nur GHOST) oder Schlüsseldatei wie bei send/transfer
    pub to: String,
    /// Betrag als Dezimaltext mit Punkt, z. B. "12.5"
    pub amount: String,
    #[serde(default)]
    pub message: String,
    /// Nachricht zusätzlich öffentlich in den Payload der Tx
    #[serde(default)]
    pub onchain: bool,
    /// Nachricht verschlüsselt an den Empfänger in den Payload (src/message.rs),
    /// bei jeder Ausführung neu verschlüsselt. Fehlt bei Aufträgen von vor der
    /// Verschlüsselung (false): dort bleibt die Nachricht wie vereinbart nur lokal.
    #[serde(default)]
    pub encrypt: bool,
    pub interval: Interval,
    pub start: NaiveDate,
    #[serde(default)]
    pub end: Option<NaiveDate>,
    /// Anzahl der Termine insgesamt
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub pause_reason: Option<String>,
    /// Nummer des nächsten Termins (0 = start)
    #[serde(default)]
    pub seq: u32,
    /// nächster Termin; null = alle Termine erledigt
    pub next_due: Option<NaiveDate>,
    #[serde(default)]
    pub inflight: Option<InFlight>,
    #[serde(default)]
    pub retry: Option<Retry>,
    #[serde(default)]
    pub history: Vec<Hist>,
    pub created: String,
    /// vom Nutzer beendet (liegt dann im Archiv)
    #[serde(default)]
    pub ended: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AboFile {
    pub version: u32,
    pub network: String,
    pub abos: Vec<Abo>,
    /// beendete Aufträge; nichts wird hart gelöscht
    #[serde(default)]
    pub archive: Vec<Abo>,
}

impl AboFile {
    pub fn empty(network: &str) -> Self {
        Self { version: 1, network: network.into(), abos: vec![], archive: vec![] }
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Abo> {
        self.abos.iter_mut().chain(self.archive.iter_mut()).find(|a| a.id == id)
    }
    pub fn get(&self, id: &str) -> Option<&Abo> {
        self.abos.iter().chain(self.archive.iter()).find(|a| a.id == id)
    }
}

// ----------------------------------------------------------- Terminrechnung ----

impl Abo {
    /// Datum des Termins Nr. `n`, immer vom Start aus gerechnet: monatlich am
    /// 31. ergibt 28./29. Februar und danach wieder den 31. (chrono setzt bei
    /// zu kurzen Monaten den letzten Tag).
    pub fn occurrence(&self, n: u32) -> Option<NaiveDate> {
        match self.interval {
            Interval::Named(Every::Daily) => self.start.checked_add_days(Days::new(n as u64)),
            Interval::Named(Every::Weekly) => self.start.checked_add_days(Days::new(7 * n as u64)),
            Interval::Days { days } => self.start.checked_add_days(Days::new(days as u64 * n as u64)),
            Interval::Named(Every::Monthly) => self.start.checked_add_months(Months::new(n)),
        }
    }

    /// Termin Nr. `n`, sofern er noch zum Plan gehört (Ende, Anzahl)
    pub fn due_at(&self, n: u32) -> Option<NaiveDate> {
        if self.count.is_some_and(|c| n >= c) {
            return None;
        }
        let d = self.occurrence(n)?;
        if self.end.is_some_and(|e| d > e) {
            return None;
        }
        Some(d)
    }

    pub fn refresh(&mut self) {
        self.next_due = self.due_at(self.seq);
    }

    /// Heute fällige Ausführung: (Terminnummer, Datum, übersprungene Termine).
    /// Sind mehrere Termine verpasst, wird nur der letzte fällige ausgeführt.
    pub fn due(&self, today: NaiveDate) -> Option<(u32, NaiveDate, u32)> {
        let first = self.due_at(self.seq)?;
        if first > today {
            return None;
        }
        let mut m = self.seq;
        while let Some(d) = self.due_at(m + 1) {
            if d > today {
                break;
            }
            m += 1;
        }
        Some((m, self.due_at(m)?, m - self.seq))
    }

    /// Erster Termin ab `today` (für das Fortsetzen nach einer Pause)
    pub fn first_from(&self, today: NaiveDate) -> u32 {
        let mut n = self.seq;
        while let Some(d) = self.due_at(n) {
            if d >= today {
                break;
            }
            n += 1;
        }
        n
    }

    /// Die nächsten `k` Termine ab dem aktuellen
    pub fn upcoming(&self, k: usize) -> Vec<NaiveDate> {
        (self.seq..).map_while(|n| self.due_at(n)).take(k).collect()
    }

    pub fn status(&self) -> &'static str {
        if self.ended.is_some() {
            "beendet"
        } else if self.inflight.is_some() {
            "läuft"
        } else if self.paused {
            "pausiert"
        } else if self.next_due.is_none() && self.retry.is_none() {
            "abgeschlossen"
        } else {
            "aktiv"
        }
    }

    fn push(&mut self, h: Hist) {
        self.history.push(h);
        if self.history.len() > MAX_HISTORY {
            let cut = self.history.len() - MAX_HISTORY;
            self.history.drain(..cut);
        }
    }
}

// ------------------------------------------------------------ Eingaben prüfen ----

/// Zeichen, die eine Nachricht nie enthalten darf (Audit 12, A12-11): alles,
/// was unsichtbar ist oder Text anders anzeigt, als er ist. Gleiche Tabelle in
/// app/src/lib/abo.ts und app/server/actions.ts; die Seite prüft sie in
/// Tests gegen die Unicode-Eigenschaften ihrer JavaScript-Engine.
/// - Cf (Formatzeichen): Richtungswechsel, Null-Breite (auch ZWJ/ZWNJ – in
///   einer einzeiligen Zahlungsnachricht entbehrlich), Soft Hyphen, Tag-Zeichen
///   U+E0001/U+E0020–E007F (unsichtbarer ASCII-Text), Mongolisch U+180E,
///   Invisible Operators U+2061–2064, Interlinear U+FFF9–FFFB u. a.
/// - Default_Ignorable_Code_Point (Unicode: „standardmäßig unsichtbar“):
///   U+034F, Hangul-Füller U+115F/1160/3164/FFA0, Khmer U+17B4/17B5,
///   Variation Selectors U+180B–180F, U+FE00–FE0F, U+E0100–E01EF und der
///   ganze reservierte Bereich U+E0000–E0FFF, U+FFF0–FFF8.
/// - Zl/Zp: Zeilen- und Absatztrenner U+2028/2029.
/// - Co (Private Use): U+E000–F8FF und die Ebenen 15/16 – Zeichen ohne
///   festgelegte Gestalt, je nach Schrift unsichtbar oder ein beliebiges Bild.
/// - Nichtzeichen U+FDD0–FDEF und U+xFFFE/xFFFF.
///
/// Erlaubt bleibt, was normale Texte brauchen: Buchstaben aller Schriften mit
/// kombinierenden Akzenten, Leerzeichen, Satzzeichen, Emoji – und VS16/VS15
/// (U+FE0F/FE0E) direkt hinter einem Bildzeichen, denn ohne VS16 lassen sich
/// ❤️, ☺️ oder ✔️ nicht schreiben. Hinter Buchstaben und Ziffern wäre der
/// Variation Selector unsichtbar und bleibt verboten (Tastenkappen wie 1️⃣
/// gehen daher nicht).
const BAD_CHARS: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x206F),
    (0x3164, 0x3164),
    (0xE000, 0xF8FF),
    (0xFDD0, 0xFDEF),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
    (0xF0000, 0x10FFFF),
];

/// Bildzeichen, hinter denen VS15/VS16 stehen dürfen (Emoji-Darstellung)
fn is_pictograph(c: char) -> bool {
    matches!(c as u32, 0x00A9 | 0x00AE | 0x203C | 0x2049 | 0x2100..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x1F000..=0x1FAFF)
}

/// Verbotenes Zeichen? `prev` = Zeichen davor (für VS15/VS16)
fn is_bad_char(c: char, prev: Option<char>) -> bool {
    let u = c as u32;
    if c.is_control() || u & 0xFFFE == 0xFFFE {
        return true;
    }
    if matches!(u, 0xFE0E | 0xFE0F) && prev.is_some_and(is_pictograph) {
        return false;
    }
    BAD_CHARS.iter().any(|&(a, b)| (a..=b).contains(&u))
}

/// Nachricht prüfen: höchstens 100 Zeichen, keine Steuer-, Format- oder
/// unsichtbaren Zeichen (Liste bei `BAD_CHARS`). Leer ist erlaubt (= keine
/// Nachricht).
pub fn check_message(m: &str) -> Result<(), String> {
    let n = m.chars().count();
    if n > MAX_MESSAGE_CHARS {
        return Err(format!("Nachricht zu lang: {n} Zeichen (höchstens {MAX_MESSAGE_CHARS})"));
    }
    if has_bad_char(m) {
        return Err("Nachricht enthält Steuerzeichen oder unsichtbare Zeichen (z. B. Zeilenumbruch) – bitte nur eine Zeile normalen Text".into());
    }
    Ok(())
}

/// Enthält der Text ein Zeichen, das `check_message` ablehnt (ohne die Länge)?
/// Der Eingang nennt damit den richtigen Grund, warum er einen Text nicht zeigt.
pub fn has_bad_char(m: &str) -> bool {
    let mut prev = None;
    m.chars().any(|c| {
        let bad = is_bad_char(c, prev);
        prev = Some(c);
        bad
    })
}

/// Gespeicherte Nachricht so, wie sie heute gesendet werden darf: Zeichen, die
/// `check_message` inzwischen ablehnt, fallen weg (Leerraum wie U+2028 wird zum
/// Leerzeichen). Ältere Daueraufträge wurden mit dem schwächeren Filter
/// angelegt; ohne diese Bereinigung scheiterte jede Ausführung – die Zahlung
/// selbst – an einem Tastenkappen-Emoji oder einem Zeilentrenner
/// (Nachprüfung A12-11). Das Ergebnis besteht `check_message` immer.
pub fn sendable_message(m: &str) -> String {
    let mut out = String::new();
    let mut prev = None;
    for c in m.chars() {
        let c = if c.is_whitespace() && is_bad_char(c, prev) { ' ' } else { c };
        if !is_bad_char(c, prev) {
            out.push(c);
            prev = Some(c);
        }
    }
    out.trim().chars().take(MAX_MESSAGE_CHARS).collect::<String>().trim_end().to_string()
}

/// Nachricht als Payload (UTF-8) für die Tx
pub fn message_payload(m: &str) -> Result<Vec<u8>, String> {
    check_message(m)?;
    if m.trim().is_empty() {
        return Err("Leere Nachricht – ohne Nachricht nichts in die Transaktion schreiben".into());
    }
    Ok(m.as_bytes().to_vec())
}

/// Payload einer Ausführung: öffentlich, verschlüsselt (je Aufruf mit neuem
/// Ephemeral-Schlüssel und Nonce) oder leer. `recipient` = x-only-Pubkey des
/// Empfängers, None bei Adressen ohne Schnorr-Schlüssel (P2SH, ECDSA).
/// Gesendet wird die Nachricht nach heutigem Filter (`sendable_message`);
/// bleibt davon nichts übrig, geht die Zahlung ohne Nachricht.
pub fn payload(a: &Abo, recipient: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let m = sendable_message(&a.message);
    if (a.onchain || a.encrypt) && !m.is_empty() {
        crate::message::payload_for(&m, a.onchain, recipient)
    } else {
        Ok(vec![])
    }
}

/// Betrag "12.5" → Einheiten (1e-8); > 0, höchstens 8 Nachkommastellen
pub fn parse_amount(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (w, f) = t.split_once('.').unwrap_or((t, ""));
    let ok = !w.is_empty() && w.len() <= 11 && w.bytes().all(|b| b.is_ascii_digit()) && f.len() <= 8 && f.bytes().all(|b| b.is_ascii_digit()) && (!t.contains('.') || !f.is_empty());
    if !ok {
        return Err(format!("Betrag „{s}“: Zahl mit Punkt und höchstens 8 Nachkommastellen erwartet"));
    }
    let units = w.parse::<u64>().map_err(|e| e.to_string())? * 100_000_000 + format!("{f:0<8}").parse::<u64>().map_err(|e| e.to_string())?;
    if units == 0 {
        return Err("Betrag muss größer als 0 sein".into());
    }
    if units > 10_000_000_000 * 100_000_000 {
        return Err("Betrag ist unrealistisch groß".into());
    }
    Ok(units)
}

/// Einheiten → "12.5"
pub fn fmt_amount(units: u64) -> String {
    let (w, f) = (units / 100_000_000, units % 100_000_000);
    if f == 0 {
        return w.to_string();
    }
    format!("{w}.{}", format!("{f:08}").trim_end_matches('0'))
}

pub struct NewAbo {
    pub key: String,
    pub asset: Asset,
    pub to: String,
    pub amount: String,
    pub message: String,
    pub onchain: bool,
    pub interval: Interval,
    pub start: NaiveDate,
    pub end: Option<NaiveDate>,
    pub count: Option<u32>,
}

/// Neuen Auftrag prüfen und anlegen (Empfänger und Schlüssel prüft der Aufrufer)
pub fn new_abo(n: NewAbo, today: NaiveDate, now: &str, id: String) -> Result<Abo, String> {
    let units = parse_amount(&n.amount)?;
    let message = n.message.trim().to_string();
    check_message(&message)?;
    if n.onchain && message.is_empty() {
        return Err("Öffentliche Nachricht gewählt, aber keine Nachricht angegeben".into());
    }
    if n.start < today {
        return Err(format!("Start {} liegt in der Vergangenheit", n.start));
    }
    if let Some(e) = n.end {
        if e < n.start {
            return Err(format!("Ende {e} liegt vor dem Start {}", n.start));
        }
    }
    if n.count == Some(0) {
        return Err("Anzahl muss mindestens 1 sein".into());
    }
    let mut a = Abo {
        id,
        key: n.key,
        asset: n.asset,
        to: n.to.trim().to_string(),
        amount: fmt_amount(units),
        encrypt: !n.onchain && !message.is_empty(),
        message,
        onchain: n.onchain,
        interval: n.interval,
        start: n.start,
        end: n.end,
        count: n.count,
        paused: false,
        pause_reason: None,
        seq: 0,
        next_due: None,
        inflight: None,
        retry: None,
        history: vec![],
        created: now.into(),
        ended: None,
    };
    a.refresh();
    if a.next_due.is_none() {
        return Err("Mit diesem Ende gibt es keinen einzigen Termin".into());
    }
    Ok(a)
}

// ------------------------------------------------------------------ Datei ----

/// deployments/mainnet.json → deployments/mainnet-abos.json
pub fn path_for(state: &Path) -> PathBuf {
    let stem = state.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "state".into());
    state.with_file_name(format!("{stem}-abos.json"))
}

/// Sperre eines ganzen Laufs (eigene Datei, unabhängig von der Dateisperre)
fn run_lock_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!("{stem}-run.json"))
}

pub fn load(path: &Path, network: &str) -> Result<AboFile, String> {
    let f = match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str::<AboFile>(&t).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => AboFile::empty(network),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if f.network != network {
        return Err(format!("{} gehört zum Netz {}, gewählt ist {network}", path.display(), f.network));
    }
    Ok(f)
}

/// Unter Dateisperre lesen, ändern, atomar schreiben
pub fn update<R>(path: &Path, network: &str, f: impl FnOnce(&mut AboFile) -> Result<R, String>) -> Result<R, String> {
    let _lock = store::lock(path, Duration::from_secs(30))?;
    let mut file = load(path, network)?;
    let r = f(&mut file)?;
    for a in file.abos.iter_mut().chain(file.archive.iter_mut()) {
        a.refresh();
    }
    store::atomic_write(path, &serde_json::to_string_pretty(&file).unwrap())?;
    Ok(r)
}

/// Einen Auftrag ändern (auch im Archiv), unter Dateisperre
fn edit(path: &Path, network: &str, id: &str, f: impl FnOnce(&mut Abo)) -> Result<(), String> {
    update(path, network, |file| {
        let a = file.get_mut(id).ok_or_else(|| format!("Dauerauftrag {id} nicht gefunden"))?;
        f(a);
        Ok(())
    })
}

pub fn pause(path: &Path, network: &str, id: &str) -> Result<Abo, String> {
    update(path, network, |file| {
        let a = file.abos.iter_mut().find(|a| a.id == id).ok_or_else(|| format!("Dauerauftrag {id} nicht gefunden (oder schon beendet)"))?;
        a.paused = true;
        a.pause_reason = Some("vom Nutzer pausiert".into());
        Ok(a.clone())
    })
}

/// Fortsetzen: Termine während der Pause werden nicht nachgeholt; weiter geht
/// es mit dem ersten Termin ab heute.
pub fn resume(path: &Path, network: &str, id: &str, today: NaiveDate, now: &str) -> Result<Abo, String> {
    update(path, network, |file| {
        let a = file.abos.iter_mut().find(|a| a.id == id).ok_or_else(|| format!("Dauerauftrag {id} nicht gefunden (oder schon beendet)"))?;
        if !a.paused {
            return Ok(a.clone());
        }
        let from = a.first_from(today);
        let left_out = from - a.seq;
        let retry_dropped = a.retry.take().is_some();
        if left_out > 0 || retry_dropped {
            let date = a.due_at(a.seq).or(a.occurrence(a.seq)).unwrap_or(today);
            let n = left_out + retry_dropped as u32;
            a.push(Hist {
                date,
                at: now.into(),
                amount: a.amount.clone(),
                txid: None,
                ok: false,
                error: None,
                skipped: n,
                note: Some(format!("{n} Termin(e) während der Pause ausgelassen")),
            });
        }
        a.seq = from;
        a.paused = false;
        a.pause_reason = None;
        a.refresh();
        Ok(a.clone())
    })
}

/// Beenden: ins Archiv, nichts wird gelöscht. Eine gerade laufende Zahlung
/// wird noch zu Ende geklärt (inflight bleibt erhalten).
pub fn remove(path: &Path, network: &str, id: &str, now: &str) -> Result<Abo, String> {
    update(path, network, |file| {
        let i = file.abos.iter().position(|a| a.id == id).ok_or_else(|| format!("Dauerauftrag {id} nicht gefunden (oder schon beendet)"))?;
        let mut a = file.abos.remove(i);
        a.ended = Some(now.into());
        a.paused = true;
        a.pause_reason = Some("beendet".into());
        file.archive.push(a.clone());
        Ok(a)
    })
}

pub fn add(path: &Path, network: &str, a: Abo) -> Result<Abo, String> {
    update(path, network, |file| {
        if file.get(&a.id).is_some() {
            return Err(format!("ID {} ist schon vergeben", a.id));
        }
        file.abos.push(a.clone());
        Ok(a)
    })
}

/// Lokaler Verlauf gesendeter Transaktionen samt Nachricht
/// (deployments/<netz>-txlog.jsonl, eine JSON-Zeile je Sendung)
pub fn log_tx(state: &Path, entry: &serde_json::Value) -> Result<(), String> {
    use std::io::Write;
    let stem = state.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "state".into());
    let path = state.with_file_name(format!("{stem}-txlog.jsonl"));
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(f, "{entry}").map_err(|e| format!("{}: {e}", path.display()))
}

/// Gibt es etwas zu tun (ohne Netz, für Agent und Seite)?
pub fn needs_run(file: &AboFile, today: NaiveDate, now_unix: u64) -> bool {
    file.abos.iter().chain(file.archive.iter()).any(|a| {
        a.inflight.is_some() || (a.ended.is_none() && !a.paused && (a.due(today).is_some() || a.retry.as_ref().is_some_and(|r| r.after <= now_unix)))
    })
}

// ------------------------------------------------------------------- Lauf ----

/// Ausgang einer unterbrochenen Zahlung
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// ein Ausgang der Tx ist sichtbar
    Accepted,
    /// sicher nicht angenommen (Eingang noch unverbraucht, nicht im Mempool)
    NotSent,
    /// noch im Mempool bzw. derzeit nicht klärbar
    Pending,
    /// nicht mehr klärbar – NICHT erneut senden
    Unknown(String),
}

#[derive(Debug, Clone)]
pub struct SubmitError {
    pub msg: String,
    /// die Tx kann das Netz erreicht haben
    pub maybe_sent: bool,
    /// Nutzer hat die Rückfrage verneint
    pub declined: bool,
}

/// Was von einer gebauten Tx für das Journal gebraucht wird
pub struct TxInfo {
    pub txid: String,
    pub fee: u64,
    pub outputs: Vec<(u32, ScriptPublicKey)>,
    pub inputs: Vec<(TransactionOutpoint, ScriptPublicKey)>,
}

/// Anbindung an Netz oder Simulator
#[allow(async_fn_in_trait)]
pub trait Payer {
    type Tx;
    fn info(tx: &Self::Tx) -> TxInfo;
    /// Zahlung bauen und vollständig prüfen, NICHT senden
    async fn prepare(&mut self, a: &Abo, units: u64) -> Result<Self::Tx, String>;
    /// Senden und auf Bestätigung warten
    async fn submit(&mut self, a: &Abo, tx: &Self::Tx) -> Result<(), SubmitError>;
    /// Unterbrochene Zahlung klären
    async fn check(&mut self, a: &Abo, f: &InFlight) -> Check;
}

pub struct RunOpts {
    pub today: NaiveDate,
    /// lokale Zeit für den Verlauf
    pub now: String,
    pub now_unix: u64,
    pub dry_run: bool,
    /// Wartezeit auf einen anderen, laufenden Lauf
    pub lock_wait: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub id: String,
    pub ok: bool,
    /// eine Zahlung wurde gesendet (bzw. im Probelauf gebaut)
    pub paid: bool,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub txid: Option<String>,
}

struct Job {
    seq: u32,
    date: NaiveDate,
    skipped: u32,
    attempts: u32,
    prev_seq: u32,
    note: Option<String>,
}

fn skip_note(skipped: u32) -> Option<String> {
    (skipped > 0).then(|| format!("{skipped} Termin(e) übersprungen (verpasst, nur einmal nachgeholt)"))
}

/// Fehlschlag ohne Senden: Verlauf, neuer Versuch später oder Pause
fn fail(a: &mut Abo, j: &Job, err: &str, o: &RunOpts) -> String {
    a.inflight = None;
    let attempts = j.attempts + 1;
    a.push(Hist { date: j.date, at: o.now.clone(), amount: a.amount.clone(), txid: None, ok: false, error: Some(err.into()), skipped: j.skipped, note: j.note.clone() });
    if attempts >= MAX_ATTEMPTS {
        a.retry = None;
        a.paused = true;
        a.pause_reason = Some(format!("{MAX_ATTEMPTS} Fehlversuche für den Termin {}: {err}", j.date));
        format!("Termin {} nach {MAX_ATTEMPTS} Versuchen gescheitert ({err}) – Auftrag pausiert", j.date)
    } else {
        a.retry = Some(Retry { seq: j.seq, date: j.date, attempts, skipped: j.skipped, after: o.now_unix + RETRY_WAIT_SECS * attempts as u64 });
        format!("Termin {}: nicht gesendet ({err}) – Versuch {attempts}/{MAX_ATTEMPTS}, nächster frühestens in {} min", j.date, RETRY_WAIT_SECS * attempts as u64 / 60)
    }
}

fn job_from_inflight(f: &InFlight) -> Job {
    Job { seq: f.seq, date: f.date, skipped: f.skipped, attempts: f.attempts, prev_seq: f.prev_seq, note: skip_note(f.skipped) }
}

/// Führt alle fälligen, nicht pausierten Aufträge aus (je Auftrag höchstens
/// eine Zahlung je Lauf). Mehrfach aufrufen ist harmlos.
pub async fn run<P: Payer>(path: &Path, network: &str, o: &RunOpts, p: &mut P) -> Result<Vec<Report>, String> {
    let mut out = vec![];
    // Probeläufe ändern nichts und brauchen die Sperre nicht
    let _run = if o.dry_run { None } else { Some(store::lock(&run_lock_path(path), o.lock_wait).map_err(|_| "Ein anderer Lauf der Daueraufträge ist gerade aktiv – er führt die fälligen aus.".to_string())?) };
    let ids: Vec<String> = {
        let f = load(path, network)?;
        f.abos.iter().chain(f.archive.iter()).map(|a| a.id.clone()).collect()
    };
    for id in ids {
        let rep = |ok: bool, paid: bool, text: String, txid: Option<String>| Report { id: id.clone(), ok, paid, text, txid };

        // 1. Unterbrochene Zahlung klären, bevor irgendetwas neu gesendet wird
        let Some(a) = load(path, network)?.get(&id).cloned() else { continue };
        if let Some(f) = a.inflight.clone() {
            if o.dry_run {
                out.push(rep(true, false, "Unterbrochene Zahlung – wird beim nächsten echten Lauf geklärt".into(), f.txid.clone()));
                continue;
            }
            let verdict = if f.txid.is_none() { Check::NotSent } else { p.check(&a, &f).await };
            let j = job_from_inflight(&f);
            match verdict {
                Check::Pending => {
                    out.push(rep(true, false, format!("Zahlung für {} ist noch unterwegs", f.date), f.txid.clone()));
                    continue;
                }
                Check::Accepted => {
                    edit(path, network, &id, |a| {
                        a.inflight = None;
                        a.push(Hist { date: j.date, at: o.now.clone(), amount: a.amount.clone(), txid: f.txid.clone(), ok: true, error: None, skipped: j.skipped, note: Some("nach Unterbrechung als angenommen bestätigt".into()) });
                    })?;
                    out.push(rep(true, true, format!("Zahlung für {} war angenommen (nach Unterbrechung geklärt)", f.date), f.txid.clone()));
                }
                Check::NotSent => {
                    let mut msg = String::new();
                    edit(path, network, &id, |a| msg = fail(a, &j, "unterbrochen, bevor gesendet wurde", o))?;
                    out.push(rep(false, false, msg, None));
                }
                Check::Unknown(e) => {
                    edit(path, network, &id, |a| {
                        a.inflight = None;
                        a.push(Hist {
                            date: j.date,
                            at: o.now.clone(),
                            amount: a.amount.clone(),
                            txid: f.txid.clone(),
                            ok: false,
                            error: Some(format!("Ausgang unklar ({e}) – nicht wiederholt, bitte im Explorer prüfen")),
                            skipped: j.skipped,
                            note: None,
                        });
                    })?;
                    out.push(rep(false, false, format!("Zahlung für {}: Ausgang unklar ({e}) – wird NICHT wiederholt", f.date), f.txid.clone()));
                }
            }
        }

        // 2. Fällig?
        let Some(a) = load(path, network)?.get(&id).cloned() else { continue };
        if a.paused || a.ended.is_some() || a.inflight.is_some() {
            continue;
        }
        let job = match (a.due(o.today), &a.retry) {
            (Some((seq, date, skipped)), retry) => {
                // ein offener Wiederholungsversuch wird vom neueren Termin abgelöst
                let skipped = skipped + retry.is_some() as u32;
                Job { seq, date, skipped, attempts: 0, prev_seq: a.seq, note: skip_note(skipped) }
            }
            (None, Some(r)) if r.after <= o.now_unix => Job { seq: r.seq, date: r.date, skipped: r.skipped, attempts: r.attempts, prev_seq: a.seq, note: skip_note(r.skipped) },
            _ => continue,
        };
        let units = match parse_amount(&a.amount) {
            Ok(u) => u,
            Err(e) => {
                let mut msg = String::new();
                edit(path, network, &id, |a| msg = fail(a, &Job { attempts: MAX_ATTEMPTS, ..job }, &e, o))?;
                out.push(rep(false, false, msg, None));
                continue;
            }
        };
        let what = format!("{} {} → {}", a.amount, a.asset.unit(), a.to);

        if o.dry_run {
            match p.prepare(&a, units).await {
                Ok(tx) => {
                    let i = P::info(&tx);
                    let extra = job.note.as_deref().map(|n| format!(", {n}")).unwrap_or_default();
                    out.push(rep(true, true, format!("Termin {}: würde {what} senden, Gebühr {} KAS{extra} – Probelauf", job.date, fmt_amount(i.fee)), Some(i.txid)));
                }
                Err(e) => out.push(rep(false, false, format!("Termin {}: {what} ließe sich nicht senden: {e}", job.date), None)),
            }
            continue;
        }

        // 3. Journal: Termin fortschreiben, BEVOR gebaut und gesendet wird
        edit(path, network, &id, |a| {
            a.retry = None;
            a.seq = a.seq.max(job.seq + 1);
            a.inflight = Some(InFlight {
                seq: job.seq,
                date: job.date,
                skipped: job.skipped,
                attempts: job.attempts,
                prev_seq: job.prev_seq,
                since: o.now.clone(),
                txid: None,
                outputs: vec![],
                inputs: vec![],
            });
        })?;

        // 4. Bauen (inkl. Prüfung gegen die Verträge)
        let tx = match p.prepare(&a, units).await {
            Ok(tx) => tx,
            Err(e) => {
                let mut msg = String::new();
                edit(path, network, &id, |a| msg = fail(a, &job, &e, o))?;
                out.push(rep(false, false, msg, None));
                continue;
            }
        };
        let info = P::info(&tx);
        edit(path, network, &id, |a| {
            if let Some(f) = a.inflight.as_mut() {
                f.txid = Some(info.txid.clone());
                f.outputs = info.outputs.clone();
                f.inputs = info.inputs.clone();
            }
        })?;

        // 5. Senden – erst jetzt, da die TXID im Journal steht
        match p.submit(&a, &tx).await {
            Ok(()) => {
                edit(path, network, &id, |a| {
                    a.inflight = None;
                    a.push(Hist { date: job.date, at: o.now.clone(), amount: a.amount.clone(), txid: Some(info.txid.clone()), ok: true, error: None, skipped: job.skipped, note: job.note.clone() });
                })?;
                let extra = job.note.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
                out.push(rep(true, true, format!("Termin {}: {what} gesendet{extra}", job.date), Some(info.txid)));
            }
            Err(e) if e.maybe_sent => {
                // Journal bleibt: der nächste Lauf klärt am Netz, nichts wird doppelt gesendet
                out.push(rep(false, false, format!("Termin {}: gesendet, aber nicht bestätigt ({}) – der nächste Lauf klärt es", job.date, e.msg), Some(info.txid)));
            }
            Err(e) if e.declined => {
                edit(path, network, &id, |a| {
                    a.inflight = None;
                    a.seq = job.prev_seq;
                    if job.attempts > 0 {
                        a.retry = Some(Retry { seq: job.seq, date: job.date, attempts: job.attempts, skipped: job.skipped, after: 0 });
                    }
                })?;
                out.push(rep(true, false, format!("Termin {}: abgebrochen, nichts gesendet – bleibt fällig", job.date), None));
            }
            Err(e) => {
                let mut msg = String::new();
                edit(path, network, &id, |a| msg = fail(a, &job, &e.msg, o))?;
                out.push(rep(false, false, msg, None));
            }
        }
    }
    Ok(out)
}

// ------------------------------------------------------------------ Tests ----

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{p2pk_spk, xonly};
    use crate::sim::Sim;
    use crate::txb::{Built, Draft, In, Unlock, build_with_payload};
    use kaspa_consensus_core::tx::TransactionOutput;
    use secp256k1::{Keypair, Secp256k1, SecretKey};
    use std::sync::{Arc, Mutex};

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn abo(interval: Interval, start: &str) -> Abo {
        let n = NewAbo {
            key: "keys/x.json".into(),
            asset: Asset::Kas,
            to: "keys/y.json".into(),
            amount: "2".into(),
            message: "Miete".into(),
            onchain: false,
            interval,
            start: d(start),
            end: None,
            count: None,
        };
        new_abo(n, d(start), "t0", "a1".into()).unwrap()
    }

    const MONTHLY: Interval = Interval::Named(Every::Monthly);

    #[test]
    fn monatlich_am_31_mit_monatsende() {
        let a = abo(MONTHLY, "2027-01-31");
        let dates: Vec<String> = (0..6).map(|n| a.occurrence(n).unwrap().to_string()).collect();
        assert_eq!(dates, ["2027-01-31", "2027-02-28", "2027-03-31", "2027-04-30", "2027-05-31", "2027-06-30"]);
        // vom Start aus gerechnet: nach dem Februar wieder der 31., nicht der 28.
        let a = abo(MONTHLY, "2027-12-31");
        assert_eq!(a.occurrence(2).unwrap(), d("2028-02-29"), "Schaltjahr 2028");
        assert_eq!(a.occurrence(3).unwrap(), d("2028-03-31"));
        assert_eq!(a.occurrence(14).unwrap(), d("2029-02-28"));
    }

    #[test]
    fn schaltjahr_taeglich_und_29_februar() {
        let a = abo(Interval::Named(Every::Daily), "2028-02-28");
        assert_eq!(a.occurrence(1).unwrap(), d("2028-02-29"));
        assert_eq!(a.occurrence(2).unwrap(), d("2028-03-01"));
        let a = abo(MONTHLY, "2028-02-29");
        assert_eq!(a.occurrence(12).unwrap(), d("2029-02-28"));
        assert_eq!(a.occurrence(48).unwrap(), d("2032-02-29"));
        let a = abo(Interval::Days { days: 14 }, "2028-02-20");
        assert_eq!(a.occurrence(1).unwrap(), d("2028-03-05"));
        let a = abo(Interval::Named(Every::Weekly), "2027-12-27");
        assert_eq!(a.occurrence(1).unwrap(), d("2028-01-03"));
    }

    #[test]
    fn nachholen_nur_einmal() {
        let mut a = abo(MONTHLY, "2027-01-15");
        assert_eq!(a.due(d("2027-01-14")), None);
        assert_eq!(a.due(d("2027-01-15")), Some((0, d("2027-01-15"), 0)));
        // Rechner war bis Mitte April aus: nur der letzte fällige Termin, 3 übersprungen
        assert_eq!(a.due(d("2027-04-20")), Some((3, d("2027-04-15"), 3)));
        a.seq = 4;
        a.refresh();
        assert_eq!(a.next_due, Some(d("2027-05-15")), "nächster Termin liegt in der Zukunft");
        assert_eq!(a.due(d("2027-04-20")), None);
    }

    #[test]
    fn ende_und_anzahl() {
        let mut a = abo(MONTHLY, "2027-01-31");
        a.count = Some(3);
        assert_eq!(a.upcoming(10), vec![d("2027-01-31"), d("2027-02-28"), d("2027-03-31")]);
        // lange aus: höchstens der letzte Termin des Plans
        assert_eq!(a.due(d("2030-01-01")), Some((2, d("2027-03-31"), 2)));
        a.seq = 3;
        a.refresh();
        assert_eq!(a.next_due, None);
        assert_eq!(a.status(), "abgeschlossen");

        let mut b = abo(Interval::Named(Every::Weekly), "2027-01-01");
        b.end = Some(d("2027-01-20"));
        assert_eq!(b.upcoming(10), vec![d("2027-01-01"), d("2027-01-08"), d("2027-01-15")]);
        // Ende inklusive
        b.end = Some(d("2027-01-15"));
        assert_eq!(b.upcoming(10).len(), 3);
        // Anzahl und Ende: was zuerst greift
        b.count = Some(2);
        assert_eq!(b.upcoming(10).len(), 2);
    }

    #[test]
    fn eingaben_pruefen() {
        assert_eq!(parse_amount("12.5").unwrap(), 1_250_000_000);
        assert_eq!(parse_amount("0.00000001").unwrap(), 1);
        for bad in ["0", "0.0", "-1", "1,5", "1.", ".5", "1.123456789", "abc", ""] {
            assert!(parse_amount(bad).is_err(), "{bad}");
        }
        assert_eq!(fmt_amount(1_250_000_000), "12.5");
        assert_eq!(fmt_amount(300_000_000), "3");
        assert_eq!(Interval::parse("monatlich").unwrap(), MONTHLY);
        assert_eq!(Interval::parse("14").unwrap(), Interval::Days { days: 14 });
        assert!(Interval::parse("0").is_err());
        assert_eq!(serde_json::to_string(&MONTHLY).unwrap(), "\"monthly\"");
        assert_eq!(serde_json::to_string(&Interval::Days { days: 10 }).unwrap(), "{\"days\":10}");
        assert_eq!(serde_json::from_str::<Interval>("{\"days\":10}").unwrap(), Interval::Days { days: 10 });
        let base = || NewAbo {
            key: "k".into(),
            asset: Asset::Ghost,
            to: "t".into(),
            amount: "1".into(),
            message: String::new(),
            onchain: false,
            interval: MONTHLY,
            start: d("2027-01-01"),
            end: None,
            count: None,
        };
        let today = d("2027-01-01");
        assert!(new_abo(NewAbo { start: d("2026-12-31"), ..base() }, today, "", "x".into()).is_err(), "Start in der Vergangenheit");
        assert!(new_abo(NewAbo { end: Some(d("2026-12-31")), ..base() }, today, "", "x".into()).is_err());
        assert!(new_abo(NewAbo { count: Some(0), ..base() }, today, "", "x".into()).is_err());
        assert!(new_abo(NewAbo { onchain: true, ..base() }, today, "", "x".into()).is_err(), "öffentlich ohne Nachricht");
        assert!(new_abo(NewAbo { message: "a\nb".into(), ..base() }, today, "", "x".into()).is_err());
        assert!(new_abo(NewAbo { message: "x".repeat(101), ..base() }, today, "", "x".into()).is_err());
        let ok = new_abo(NewAbo { message: "  Miete  ".into(), ..base() }, today, "", "x".into()).unwrap();
        assert_eq!(ok.message, "Miete");
        assert!(ok.encrypt && !ok.onchain, "ohne Häkchen: verschlüsselt");
        let public = new_abo(NewAbo { message: "Miete".into(), onchain: true, ..base() }, today, "", "x".into()).unwrap();
        assert!(!public.encrypt && public.onchain);
        let none = new_abo(base(), today, "", "x".into()).unwrap();
        assert!(!none.encrypt && !none.onchain, "ohne Nachricht nichts");
        // Aufträge von vor der Verschlüsselung (ohne Feld): Nachricht bleibt lokal
        let mut old = serde_json::to_value(&ok).unwrap();
        old.as_object_mut().unwrap().remove("encrypt");
        let old: Abo = serde_json::from_value(old).unwrap();
        assert!(!old.encrypt);
        assert_eq!(payload(&old, None).unwrap(), Vec::<u8>::new());
        assert_eq!(payload(&public, None).unwrap(), b"Miete");
        assert_eq!(payload(&ok, None).unwrap_err(), crate::message::NOT_P2PK);
        assert_eq!(ok.next_due, Some(d("2027-01-01")));
    }

    // ---- Lauf gegen den Simulator: echte KAS-Tx, Skripte, Gebühr

    fn key() -> Keypair {
        let mut b = [7u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
        Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&b).unwrap())
    }

    /// Simulator als Netz; zählt, wie oft wirklich gezahlt wurde
    #[derive(Clone)]
    struct SimPayer {
        sim: Arc<Mutex<Sim>>,
        from: Keypair,
        to: Keypair,
        paid: Arc<Mutex<Vec<String>>>,
        /// Ausfälle: vor dem Bauen hängen bzw. nach dem Senden hängen (Absturz)
        hang_prepare: bool,
        hang_after_submit: bool,
        fail_prepare: Option<String>,
        /// Payloads der angenommenen Tx
        payloads: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl SimPayer {
        fn new() -> Self {
            let mut sim = Sim::new();
            let from = key();
            sim.faucet(&from, 1_000 * 100_000_000);
            Self { sim: Arc::new(Mutex::new(sim)), from, to: key(), paid: Arc::default(), hang_prepare: false, hang_after_submit: false, fail_prepare: None, payloads: Arc::default() }
        }
        fn received(&self) -> u64 {
            self.sim.lock().unwrap().balance(&self.to)
        }
    }

    impl Payer for SimPayer {
        type Tx = Built;
        fn info(b: &Built) -> TxInfo {
            TxInfo {
                txid: b.tx.id().to_string(),
                fee: b.fee,
                outputs: b.tx.outputs.iter().enumerate().map(|(i, o)| (i as u32, o.script_public_key.clone())).collect(),
                inputs: b.tx.inputs.iter().zip(&b.entries).map(|(i, e)| (i.previous_outpoint, e.script_public_key.clone())).collect(),
            }
        }
        async fn prepare(&mut self, a: &Abo, units: u64) -> Result<Built, String> {
            if self.hang_prepare {
                std::future::pending::<()>().await;
            }
            if let Some(e) = &self.fail_prepare {
                return Err(e.clone());
            }
            let sim = self.sim.lock().unwrap();
            let f = sim.funds(&self.from);
            let inputs = f.utxos.iter().map(|(op, e)| In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: self.from.into() } }).collect();
            let out = TransactionOutput { value: units, script_public_key: p2pk_spk(&xonly(&self.to)), covenant: None };
            let payload = payload(a, Some(&xonly(&self.to)))?;
            build_with_payload(Draft { inputs, outputs: vec![out], change_spk: p2pk_spk(&xonly(&self.from)), lock_time: 0 }, &payload, &sim.params)
        }
        async fn submit(&mut self, _a: &Abo, b: &Built) -> Result<(), SubmitError> {
            self.sim.lock().unwrap().submit(b).map_err(|e| SubmitError { msg: e, maybe_sent: false, declined: false })?;
            self.paid.lock().unwrap().push(b.tx.id().to_string());
            self.payloads.lock().unwrap().push(b.tx.payload.clone());
            if self.hang_after_submit {
                std::future::pending::<()>().await;
            }
            Ok(())
        }
        async fn check(&mut self, _a: &Abo, f: &InFlight) -> Check {
            let sim = self.sim.lock().unwrap();
            let txid: kaspa_consensus_core::Hash = f.txid.as_ref().unwrap().parse().unwrap();
            if f.outputs.iter().any(|(i, _)| sim.utxos.contains_key(&TransactionOutpoint { transaction_id: txid, index: *i })) {
                return Check::Accepted;
            }
            if f.inputs.iter().any(|(op, _)| sim.utxos.contains_key(op)) {
                return Check::NotSent;
            }
            Check::Unknown("nicht sichtbar".into())
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ghost-abo-{name}-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("sim-abos.json")
    }

    fn opts(today: &str, unix: u64) -> RunOpts {
        RunOpts { today: d(today), now: format!("{today} 08:00"), now_unix: unix, dry_run: false, lock_wait: Duration::from_secs(20) }
    }

    fn setup(name: &str, onchain: bool) -> (PathBuf, SimPayer) {
        let path = tmp(name);
        let mut a = abo(MONTHLY, "2027-01-31");
        a.onchain = onchain;
        a.encrypt = !onchain;
        add(&path, "sim", a).unwrap();
        (path, SimPayer::new())
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn zweimal_hintereinander_nur_eine_zahlung() {
        let (path, mut p) = setup("zweimal", true);
        let rt = rt();
        let r1 = rt.block_on(run(&path, "sim", &opts("2027-01-31", 1), &mut p)).unwrap();
        assert!(r1[0].ok && r1[0].paid, "{r1:?}");
        let r2 = rt.block_on(run(&path, "sim", &opts("2027-01-31", 2), &mut p)).unwrap();
        assert!(r2.is_empty(), "nichts mehr fällig: {r2:?}");
        assert_eq!(p.paid.lock().unwrap().len(), 1);
        assert_eq!(p.received(), 200_000_000);
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert_eq!(a.next_due, Some(d("2027-02-28")));
        assert_eq!(a.history.len(), 1);
        assert!(a.history[0].ok && a.history[0].txid.is_some());
        // nächster Monat: wieder genau eine
        rt.block_on(run(&path, "sim", &opts("2027-02-28", 3), &mut p)).unwrap();
        rt.block_on(run(&path, "sim", &opts("2027-03-01", 4), &mut p)).unwrap();
        assert_eq!(p.paid.lock().unwrap().len(), 2);
    }

    #[test]
    fn verschluesselt_je_ausfuehrung_neu() {
        // ohne Häkchen: jede Zahlung trägt die Nachricht verschlüsselt, jedes
        // Mal mit neuem Ephemeral-Schlüssel und Nonce; nur der Empfänger liest sie
        let (path, mut p) = setup("verschluesselt", false);
        let rt = rt();
        rt.block_on(run(&path, "sim", &opts("2027-01-31", 1), &mut p)).unwrap();
        rt.block_on(run(&path, "sim", &opts("2027-02-28", 2), &mut p)).unwrap();
        let pl = p.payloads.lock().unwrap().clone();
        assert_eq!(pl.len(), 2);
        let to_sk = SecretKey::from_keypair(&p.to);
        let from_sk = SecretKey::from_keypair(&p.from);
        for x in &pl {
            assert!(crate::message::is_encrypted(x));
            assert_eq!(crate::message::decrypt(&to_sk, x).as_deref(), Some("Miete"));
            assert_eq!(crate::message::decrypt(&from_sk, x), None, "Absender liest nur im lokalen Verlauf");
        }
        assert_ne!(pl[0][4..48], pl[1][4..48], "neuer Ephemeral-Schlüssel und Nonce je Ausführung");
        // öffentlich: Klartext
        let (path, mut p) = setup("oeffentlich", true);
        rt.block_on(run(&path, "sim", &opts("2027-01-31", 1), &mut p)).unwrap();
        assert_eq!(p.payloads.lock().unwrap()[0], b"Miete");
    }

    #[test]
    fn gleichzeitig_angestossen_nur_eine_zahlung() {
        // Agent und Seite stoßen gleichzeitig an: Sperre + fortgeschriebener Termin
        for round in 0..3 {
            let (path, p) = setup(&format!("parallel{round}"), false);
            let threads: Vec<_> = (0..4)
                .map(|i| {
                    let (path, mut p) = (path.clone(), p.clone());
                    std::thread::spawn(move || rt().block_on(run(&path, "sim", &opts("2027-02-01", 10 + i), &mut p)))
                })
                .collect();
            for t in threads {
                t.join().unwrap().unwrap();
            }
            assert_eq!(p.paid.lock().unwrap().len(), 1, "Runde {round}: genau eine Zahlung");
            assert_eq!(p.received(), 200_000_000);
        }
    }

    #[test]
    fn nachholen_im_lauf_mit_vermerk() {
        let (path, mut p) = setup("nachholen", false);
        // Rechner war vom 31.1. bis 5.5. aus
        let r = rt().block_on(run(&path, "sim", &opts("2027-05-05", 1), &mut p)).unwrap();
        assert!(r[0].text.contains("3 Termin(e) übersprungen"), "{r:?}");
        assert_eq!(p.paid.lock().unwrap().len(), 1, "genau einmal nachgeholt");
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert_eq!(a.next_due, Some(d("2027-05-31")), "nächster Termin in der Zukunft");
        assert_eq!(a.history[0].date, d("2027-04-30"));
        assert_eq!(a.history[0].skipped, 3);
    }

    #[test]
    fn absturz_nach_dem_senden_keine_doppelzahlung() {
        let (path, mut p) = setup("absturz-senden", false);
        let rt = rt();
        // Lauf 1 sendet und „stürzt ab“, bevor er das Ergebnis schreibt
        let mut crashing = SimPayer { hang_after_submit: true, ..p.clone() };
        let r = rt.block_on(async { tokio::time::timeout(Duration::from_millis(500), run(&path, "sim", &opts("2027-01-31", 1), &mut crashing)).await });
        assert!(r.is_err(), "Lauf hängt nach dem Senden (Absturz)");
        assert_eq!(p.paid.lock().unwrap().len(), 1);
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert!(a.inflight.as_ref().is_some_and(|f| f.txid.is_some()), "Journal mit TXID vorhanden");
        assert_eq!(a.next_due, Some(d("2027-02-28")), "Termin schon vor dem Senden fortgeschrieben");
        // Lauf 2 (gleicher Tag, auch nach Ablauf jeder Wartezeit): klärt, sendet NICHT erneut
        let r = rt.block_on(run(&path, "sim", &opts("2027-01-31", 99_999), &mut p)).unwrap();
        assert!(r[0].ok && r[0].text.contains("angenommen"), "{r:?}");
        assert_eq!(p.paid.lock().unwrap().len(), 1, "keine Doppelzahlung");
        assert_eq!(p.received(), 200_000_000);
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert!(a.inflight.is_none() && a.history.len() == 1 && a.history[0].ok);
        // ist der Ausgang nicht mehr klärbar, wird ebenfalls nicht erneut gesendet
        let (path2, p2) = setup("absturz-unklar", false);
        let mut crashing = SimPayer { hang_after_submit: true, ..p2.clone() };
        let _ = rt.block_on(async { tokio::time::timeout(Duration::from_millis(500), run(&path2, "sim", &opts("2027-01-31", 1), &mut crashing)).await });
        // alle Ausgänge weg (z. B. vom Empfänger weitergegeben): unklar
        update(&path2, "sim", |f| {
            f.abos[0].inflight.as_mut().unwrap().outputs.clear();
            Ok(())
        })
        .unwrap();
        let mut p2b = p2.clone();
        let r = rt.block_on(run(&path2, "sim", &opts("2027-01-31", 99_999), &mut p2b)).unwrap();
        assert!(!r[0].ok && r[0].text.contains("NICHT wiederholt"), "{r:?}");
        let r = rt.block_on(run(&path2, "sim", &opts("2027-01-31", 199_999), &mut p2b)).unwrap();
        assert!(r.is_empty());
        assert_eq!(p2.paid.lock().unwrap().len(), 1);
    }

    #[test]
    fn absturz_nach_dem_journal_genau_eine_zahlung() {
        let (path, mut p) = setup("absturz-journal", false);
        let rt = rt();
        // Lauf 1 schreibt das Journal und hängt beim Bauen (nichts gesendet)
        let mut crashing = SimPayer { hang_prepare: true, ..p.clone() };
        let r = rt.block_on(async { tokio::time::timeout(Duration::from_millis(500), run(&path, "sim", &opts("2027-01-31", 1_000), &mut crashing)).await });
        assert!(r.is_err());
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert!(a.inflight.as_ref().is_some_and(|f| f.txid.is_none()));
        assert_eq!(p.paid.lock().unwrap().len(), 0);
        // Lauf 2: sicher nicht gesendet → neuer Versuch nach der Wartezeit
        rt.block_on(run(&path, "sim", &opts("2027-01-31", 1_001), &mut p)).unwrap();
        assert_eq!(p.paid.lock().unwrap().len(), 0, "Wartezeit vor dem neuen Versuch");
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert_eq!(a.retry.as_ref().map(|r| r.attempts), Some(1));
        rt.block_on(run(&path, "sim", &opts("2027-01-31", 1_001 + RETRY_WAIT_SECS), &mut p)).unwrap();
        rt.block_on(run(&path, "sim", &opts("2027-02-01", 1_002 + RETRY_WAIT_SECS), &mut p)).unwrap();
        assert_eq!(p.paid.lock().unwrap().len(), 1, "genau eine Zahlung für den Termin");
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert!(a.retry.is_none() && a.inflight.is_none());
        assert_eq!(a.history.iter().filter(|h| h.ok).count(), 1);
    }

    #[test]
    fn drei_fehlversuche_dann_pause() {
        let (path, p) = setup("fehler", false);
        let mut bad = SimPayer { fail_prepare: Some("keine KAS".into()), ..p.clone() };
        let rt = rt();
        let mut t = 1_000;
        for _ in 0..MAX_ATTEMPTS {
            rt.block_on(run(&path, "sim", &opts("2027-01-31", t), &mut bad)).unwrap();
            t += RETRY_WAIT_SECS * MAX_ATTEMPTS as u64;
        }
        let a = load(&path, "sim").unwrap().abos[0].clone();
        assert!(a.paused, "nach {MAX_ATTEMPTS} Versuchen pausiert");
        assert!(a.pause_reason.as_ref().unwrap().contains("keine KAS"));
        assert_eq!(a.history.iter().filter(|h| !h.ok).count(), MAX_ATTEMPTS as usize);
        assert_eq!(a.status(), "pausiert");
        // pausiert: nichts mehr, auch nicht mit funktionierendem Netz
        let mut good = p.clone();
        let r = rt.block_on(run(&path, "sim", &opts("2027-03-31", t), &mut good)).unwrap();
        assert!(r.is_empty());
        // Fortsetzen: Termine der Pause nicht nachholen, weiter ab heute
        let a = resume(&path, "sim", "a1", d("2027-03-15"), "t").unwrap();
        assert_eq!(a.next_due, Some(d("2027-03-31")));
        assert!(a.history.last().unwrap().note.as_ref().unwrap().contains("Pause"));
        rt.block_on(run(&path, "sim", &opts("2027-03-31", t), &mut good)).unwrap();
        assert_eq!(p.paid.lock().unwrap().len(), 1);
    }

    #[test]
    fn neuer_termin_loest_wiederholung_ab() {
        let (path, p) = setup("abloesen", false);
        let rt = rt();
        let mut bad = SimPayer { fail_prepare: Some("Node weg".into()), ..p.clone() };
        rt.block_on(run(&path, "sim", &opts("2027-01-31", 1), &mut bad)).unwrap();
        // der Februar-Termin ist fällig, bevor der Januar erneut versucht wurde: eine Zahlung
        let mut good = p.clone();
        let r = rt.block_on(run(&path, "sim", &opts("2027-02-28", 2), &mut good)).unwrap();
        assert!(r[0].text.contains("1 Termin(e) übersprungen"), "{r:?}");
        assert_eq!(p.paid.lock().unwrap().len(), 1);
    }

    #[test]
    fn probelauf_aendert_nichts_und_beenden_archiviert() {
        let (path, mut p) = setup("probe", true);
        let before = std::fs::read_to_string(&path).unwrap();
        let o = RunOpts { dry_run: true, ..opts("2027-06-01", 1) };
        let r = rt().block_on(run(&path, "sim", &o, &mut p)).unwrap();
        assert!(r[0].ok && r[0].text.contains("Probelauf"), "{r:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "Probelauf schreibt nichts");
        assert_eq!(p.paid.lock().unwrap().len(), 0);
        pause(&path, "sim", "a1").unwrap();
        assert!(!needs_run(&load(&path, "sim").unwrap(), d("2027-06-01"), 1));
        let a = remove(&path, "sim", "a1", "t").unwrap();
        assert_eq!(a.status(), "beendet");
        let f = load(&path, "sim").unwrap();
        assert!(f.abos.is_empty() && f.archive.len() == 1, "nichts gelöscht, nur archiviert");
        assert!(remove(&path, "sim", "a1", "t").is_err());
        assert!(load(&path, "mainnet").is_err(), "falsches Netz");
    }
}
