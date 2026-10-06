//! Zinsregel des GHOST-Agenten (Version 3), robust gegen Einzelmessungen
//! (Audit 11 A11-O-1, A11-O-8, A11-O-9).
//!
//! - Je Orakel-Runde eine Messung des GHOST-Kurses aus dem FRISCH abgeglichenen
//!   Pool (Reserve KAS/GHOST × KAS-Marktpreis). Ist der Pool-Stand unbekannt
//!   (`pool_unresolved`) oder hat der Pool weniger als `MIN_POOL_GHOST`, wird
//!   nicht gemessen und der Zins nicht geändert.
//! - Entschieden wird über den Median der Messungen der letzten Stunde, und nur,
//!   wenn mindestens `MIN_SAMPLES` davon vorliegen UND älteste und jüngste
//!   mindestens `MIN_SPAN_SECS` (45 min) auseinanderliegen. Ein einzelner Tausch
//!   über eine Rundengrenze bewegt den Median nicht. Wer ihn verschieben will,
//!   muss bei regelmäßigen Messungen (alle 240 s) mehr als die Hälfte der
//!   Messungen im Fenster stellen, also den Kurs mindestens 24 min halten.
//!   Vorher genügten 6 Messungen in 20 min, davon 4 manipuliert (Audit 12
//!   A12-12). Fehlen Messungen (Agent aus, Pool unter der Mindestliquidität),
//!   zählen die vorhandenen: Lücken gewichtet der Median nicht.
//! - Die letzte Zinsänderung aus der Zukunft (Uhr lief einmal vor) zählt ab
//!   dem Zeitpunkt, an dem sie bemerkt wird; vorher wartete die Zinsregel bis
//!   zu diesem Zeitpunkt, bei +10 Jahren also für immer (Audit 12 A12-3).
//! - Totzone + nur bei Handel (Entscheidung des Betreibers 06.10.2026, Audit 20
//!   A20e-6): Der Zins ändert sich nur, wenn der Median AUSSERHALB der Totzone
//!   0,97–1,03 USD liegt (= Kursband des Pools, `math::RATE_ZONE`) UND der Pool
//!   im Messfenster gehandelt wurde. „Gehandelt“ heißt: Das Tauschverhältnis
//!   des Pools selbst (KAS-Reserve ÷ GHOST-Reserve, ohne den KAS-Marktpreis)
//!   hat sich zwischen aufeinanderfolgenden Messungen im Fenster zusammen um
//!   mindestens `MIN_TRADE_MOVE` (2 %) bewegt. Das Verhältnis ändert sich nur
//!   durch Tausch (Einlegen/Abziehen halten es bis auf Rundung), bei einem
//!   Produktpool entsprechen 2 % etwa 1 % der Reserve als Umsatz – im kleinsten
//!   messbaren Pool (10 GHOST) ≈ 0,1 GHOST. Ohne Handel bildet die Messung
//!   nur die KAS-Bewegung ab (Ratsche A20e-6), das zählt nicht. Ein Tausch hin
//!   und zurück innerhalb von 4 min ist unsichtbar und zählt ebenfalls nicht;
//!   außerhalb des Bands lässt der Pool nur Tausche Richtung Band zu, ein
//!   vorgetäuschter Umsatz schiebt den Kurs also in die Totzone.
//! - Messungen und der Zeitpunkt der letzten Zinsänderung liegen in
//!   `deployments/<netz>-zins.json` neben der Zustandsdatei, geschrieben unter
//!   einer eigenen Dateisperre. So gilt „höchstens eine Zinsänderung je Stunde“
//!   auch über Neustarts des Agenten und für ein parallel laufendes
//!   `oracle-feed` (vorher lag der Takt nur im Speicher des Prozesses).

use crate::math;
use crate::store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Messfenster für den Median (1 Stunde)
pub const WINDOW_SECS: u64 = 3600;
/// Mindestzahl Messungen im Fenster, bevor der Zins sich ändern darf
pub const MIN_SAMPLES: usize = 6;
/// Mindestabstand zweier Messungen: Neustarts (alle 30 s) oder ein zweiter
/// Prozess füllen das Fenster sonst in Minuten mit fast gleichen Werten
pub const SAMPLE_GAP_SECS: u64 = 240;
/// Die Messungen im Fenster müssen sich über mindestens 45 min verteilen
/// (älteste bis jüngste). Mit SAMPLE_GAP_SECS allein reichten 6 Messungen in
/// 20 min (Audit 12 A12-12).
pub const MIN_SPAN_SECS: u64 = 2700;
/// Höchstens eine Zinsänderung je Stunde
pub const EVERY_SECS: u64 = 3600;
/// Mindestliquidität des Pools: 10 GHOST Reserve (bei 1 USD also 10 USD je
/// Seite). Das ist keine Kostenhürde gegen Manipulation (die bilden Median und
/// Takt), sondern schließt Pools aus, deren Kurs nichts aussagt: Der Startpool
/// des Umzugs hat 0,25 GHOST, dort verschiebt schon ein Tausch um Cent-Beträge
/// den Kurs um Prozente, und ohne Handel bildet er nur die KAS-Bewegung ab.
/// 10 GHOST sind ein Fünftel eines vollen Mainnet-Vaults (50 GHOST), also
/// ein Pool, in den jemand echtes Kapital gelegt hat.
pub const MIN_POOL_GHOST: i64 = 10 * math::E8;
/// Mindestbewegung des Tauschverhältnisses im Messfenster, damit der Pool als
/// gehandelt gilt (Summe der Beträge der logarithmischen Änderungen zwischen
/// aufeinanderfolgenden Messungen): 2 % ≈ 1 % der Reserve als Umsatz
pub const MIN_TRADE_MOVE: f64 = 0.02;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    /// Unix-Sekunden
    pub at: u64,
    pub ghost_usd: f64,
    /// Tauschverhältnis des Pools (KAS-Reserve ÷ GHOST-Reserve) bei der
    /// Messung; ältere Messungen haben es nicht (zählen dann nicht als Handel)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_ratio: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLog {
    pub network: String,
    #[serde(default)]
    pub samples: Vec<Sample>,
    /// Zeitpunkt (Unix-Sekunden) der letzten gesendeten oder vorgemerkten Zinsänderung
    #[serde(default)]
    pub last_change: Option<u64>,
}

/// Was die Zinsregel in dieser Runde tut
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// letzte Zinsänderung vor weniger als einer Stunde: noch `secs_left` warten
    Wait { secs_left: u64 },
    /// zu wenige Messungen im Fenster
    TooFew { have: usize },
    /// genug Messungen, aber zu kurz beisammen (älteste bis jüngste < MIN_SPAN_SECS)
    TooShort { have: usize, span_secs: u64 },
    /// Median im Zielband (oder Rahmen erreicht): Zins bleibt
    Keep { median: f64, have: usize },
    /// Median außerhalb der Totzone, aber der Pool wurde im Fenster nicht
    /// (genug) gehandelt: Zins bleibt (A20e-6). `moved` = Bewegung des
    /// Tauschverhältnisses im Fenster
    NoTrade { median: f64, have: usize, moved: f64 },
    /// Zins ändern
    Change { median: f64, have: usize, next: f64 },
}

/// deployments/mainnet.json → deployments/mainnet-zins.json
pub fn path_for(state: &Path) -> PathBuf {
    let stem = state.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "state".into());
    state.with_file_name(format!("{stem}-zins.json"))
}

/// Median; bei gerader Anzahl der Mittelwert der beiden mittleren Werte
pub fn median(v: &[f64]) -> Option<f64> {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return None;
    }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    Some(if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 })
}

/// Tauschverhältnis des Pools: KAS-Reserve ÷ GHOST-Reserve
pub fn pool_ratio(x: i64, y: i64) -> Option<f64> {
    let r = x as f64 / y as f64;
    (y > 0 && x > 0 && r.is_finite()).then_some(r)
}

/// Eine Messung aus dem frisch abgeglichenen Pool: Ok(GHOST in USD) oder
/// Err(Grund, warum nicht gemessen wird). `pool` = (KAS-Reserve, GHOST-Reserve).
pub fn measure(pool: Option<(i64, i64)>, unresolved: Option<&str>, market: i64) -> Result<f64, String> {
    let Some((x, y)) = pool else { return Err("kein Tauschpool".into()) };
    if let Some(e) = unresolved {
        return Err(format!("Pool-Stand nicht abgleichbar ({e})"));
    }
    if y < MIN_POOL_GHOST {
        return Err(format!("Pool hat nur {:.8} GHOST (Mindestliquidität {} GHOST)", y as f64 / 1e8, MIN_POOL_GHOST / math::E8));
    }
    let g = crate::pool::ghost_usd(x, y, market);
    if !g.is_finite() || g <= 0.0 {
        return Err("GHOST-Kurs nicht bestimmbar".into());
    }
    Ok(g)
}

impl RateLog {
    pub fn new(network: &str) -> Self {
        RateLog { network: network.into(), ..Default::default() }
    }

    /// Messungen außerhalb des Fensters (und aus der Zukunft) verwerfen. Eine
    /// letzte Zinsänderung aus der Zukunft (Uhr lief vor) wird auf `now`
    /// gekappt: Die Zinsregel wartet dann noch eine Stunde ab jetzt, nicht bis
    /// zu diesem Zeitpunkt (Audit 12 A12-3).
    pub fn prune(&mut self, now: u64) {
        self.samples.retain(|s| s.at <= now && now - s.at < WINDOW_SECS);
        if self.last_change.is_some_and(|t| t > now) {
            self.last_change = Some(now);
        }
    }

    /// Messung ohne Tauschverhältnis aufnehmen (zählt nicht als Handel)
    pub fn record(&mut self, now: u64, ghost_usd: f64) -> bool {
        self.record_pool(now, ghost_usd, None)
    }

    /// Messung samt Tauschverhältnis des Pools aufnehmen, wenn die letzte
    /// mindestens SAMPLE_GAP_SECS zurückliegt. true = aufgenommen.
    pub fn record_pool(&mut self, now: u64, ghost_usd: f64, pool_ratio: Option<f64>) -> bool {
        self.prune(now);
        if !ghost_usd.is_finite() || ghost_usd <= 0.0 {
            return false;
        }
        if self.samples.last().is_some_and(|s| now - s.at < SAMPLE_GAP_SECS) {
            return false;
        }
        let pool_ratio = pool_ratio.filter(|r| r.is_finite() && *r > 0.0);
        self.samples.push(Sample { at: now, ghost_usd, pool_ratio });
        true
    }

    /// Bewegung des Tauschverhältnisses im Fenster: Summe |ln(r_i / r_i−1)|
    /// über aufeinanderfolgende Messungen mit Verhältnis (A20e-6)
    pub fn traded_move(&self, now: u64) -> f64 {
        let r: Vec<f64> = self.samples.iter().filter(|s| s.at <= now && now - s.at < WINDOW_SECS).filter_map(|s| s.pool_ratio).collect();
        r.windows(2).map(|w| (w[1] / w[0]).ln().abs()).filter(|m| m.is_finite()).sum()
    }

    /// Messungen im Fenster
    pub fn window(&self, now: u64) -> Vec<f64> {
        self.samples.iter().filter(|s| s.at <= now && now - s.at < WINDOW_SECS).map(|s| s.ghost_usd).collect()
    }

    /// Sekunden bis zur nächsten erlaubten Zinsänderung (0 = jetzt erlaubt).
    /// Eine Zeit in der Zukunft (Uhr zurückgestellt) zählt vorsichtig ab `now`;
    /// `prune` (bei jeder Messung und Vormerkung) kappt sie dauerhaft auf `now`.
    pub fn wait_secs(&self, now: u64) -> u64 {
        match self.last_change {
            Some(t) if t > now => EVERY_SECS,
            Some(t) => EVERY_SECS.saturating_sub(now - t),
            None => 0,
        }
    }

    pub fn decide(&self, now: u64, current_pct: f64) -> Decision {
        let secs_left = self.wait_secs(now);
        if secs_left > 0 {
            return Decision::Wait { secs_left };
        }
        let w = self.window(now);
        if w.len() < MIN_SAMPLES {
            return Decision::TooFew { have: w.len() };
        }
        let at: Vec<u64> = self.samples.iter().filter(|s| s.at <= now && now - s.at < WINDOW_SECS).map(|s| s.at).collect();
        let span_secs = at.iter().max().unwrap() - at.iter().min().unwrap();
        if span_secs < MIN_SPAN_SECS {
            return Decision::TooShort { have: w.len(), span_secs };
        }
        let median = median(&w).expect("Fenster nicht leer");
        match math::rate_next(current_pct, median) {
            // außerhalb der Totzone: nur, wenn im Fenster gehandelt wurde (A20e-6)
            Some(next) => {
                let moved = self.traded_move(now);
                if moved + 1e-12 < MIN_TRADE_MOVE {
                    Decision::NoTrade { median, have: w.len(), moved }
                } else {
                    Decision::Change { median, have: w.len(), next }
                }
            }
            None => Decision::Keep { median, have: w.len() },
        }
    }

    /// Zinsänderung vormerken, bevor gesendet wird. None = in der letzten Stunde
    /// schon geändert (z. B. von einem anderen Prozess). Some(vorher) erlaubt
    /// `release`, falls sicher nichts gesendet wurde.
    pub fn reserve(&mut self, now: u64) -> Option<Option<u64>> {
        self.prune(now);
        if self.wait_secs(now) > 0 {
            return None;
        }
        let prev = self.last_change;
        self.last_change = Some(now);
        Some(prev)
    }

    /// Vormerkung zurücknehmen (nur die eigene, erkennbar am Zeitpunkt)
    pub fn release(&mut self, reserved_at: u64, prev: Option<u64>) {
        if self.last_change == Some(reserved_at) {
            self.last_change = prev;
        }
    }

    /// Zins von Hand gesetzt (`oracle-update --rate`): Der Takt beginnt jetzt,
    /// auch wenn die letzte Änderung weniger als eine Stunde zurückliegt (die
    /// Hand geht vor). Rückgabe wie bei `reserve` der vorige Zeitpunkt, für
    /// `release`. Vorher setzte ein Zins von Hand den Takt nicht, und die
    /// Zinsregel des Agenten durfte ihn sofort wieder ändern (Audit 12
    /// A12-17 c, Restpunkt C-O1).
    pub fn set_by_hand(&mut self, now: u64) -> Option<u64> {
        self.prune(now);
        let prev = self.last_change;
        self.last_change = Some(now);
        prev
    }

    /// Neues Deployment (`deploy --rate`): Messungen eines früheren Pools
    /// sagen über den neuen nichts, der Takt beginnt mit dem Startzins.
    pub fn restart(&mut self, now: u64) {
        self.samples.clear();
        self.last_change = Some(now);
    }
}

pub fn load(path: &Path, network: &str) -> Result<RateLog, String> {
    let log = match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str::<RateLog>(&t).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => RateLog::new(network),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if log.network != network {
        return Err(format!("{} gehört zum Netz {}, gewählt ist {network}", path.display(), log.network));
    }
    Ok(log)
}

/// Liest die Datei unter ihrer Sperre, wendet `f` an und schreibt sie zurück
/// (atomar). Sperre: deployments/<netz>-zins.lock (flock, wie die Zustandsdatei).
pub fn update<R>(path: &Path, network: &str, f: impl FnOnce(&mut RateLog) -> R) -> Result<R, String> {
    let _lock = store::lock(path, Duration::from_secs(10))?;
    let mut log = load(path, network)?;
    let r = f(&mut log);
    store::atomic_write(path, &serde_json::to_string_pretty(&log).unwrap())?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_790_000_000;
    /// sechs Messungen im Abstand STEP liegen genau MIN_SPAN_SECS auseinander
    const STEP: u64 = MIN_SPAN_SECS / 5;

    /// Tauschverhältnis eines gehandelten Pools: schwankt je Messung um 1 %
    fn traded(i: usize) -> Option<f64> {
        Some(25.0 * if i % 2 == 0 { 1.0 } else { 1.01 })
    }

    /// Messungen eines gehandelten Pools (Tauschverhältnis bewegt sich)
    fn log_with(values: &[f64], start: u64, step: u64) -> RateLog {
        let mut l = RateLog::new("mainnet");
        for (i, v) in values.iter().enumerate() {
            assert!(l.record_pool(start + i as u64 * step, *v, traded(i)));
        }
        l
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ghost-zins-{name}-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("mainnet.json")
    }

    #[test]
    fn median_ungerade_gerade_und_ausreisser() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[1.0]), Some(1.0));
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        // ein einzelner Ausreißer (Tausch über die Rundengrenze) bewegt nichts
        assert_eq!(median(&[1.0, 1.0, 1.0, 1.02, 1.0, 1.0, 1.0]), Some(1.0));
        assert_eq!(median(&[1.0, f64::NAN, 1.0]), Some(1.0));
    }

    #[test]
    fn messung_nur_aus_bekanntem_pool_mit_mindestliquiditaet() {
        let e8 = math::E8;
        // 250 KAS / 10 GHOST bei 0,04 USD = 1,00 USD
        assert!((measure(Some((250 * e8, 10 * e8)), None, 4_000_000).unwrap() - 1.0).abs() < 1e-12);
        assert!(measure(Some((250 * e8, 10 * e8 - 1)), None, 4_000_000).unwrap_err().contains("Mindestliquidität"));
        // Startpool des Umzugs (0,25 GHOST): keine Messung
        assert!(measure(Some((625_000_000, 25_000_000)), None, 4_000_000).is_err());
        assert!(measure(Some((250 * e8, 10 * e8)), Some("Node nicht synchron"), 4_000_000).unwrap_err().contains("nicht abgleichbar"));
        assert!(measure(None, None, 4_000_000).is_err());
    }

    #[test]
    fn mindestens_sechs_messungen_im_fenster() {
        // 5 Messungen weit unter 1 USD: noch keine Änderung
        let l = log_with(&[0.9; 5], T0, STEP);
        let now = T0 + 4 * STEP;
        assert_eq!(l.decide(now, 3.0), Decision::TooFew { have: 5 });
        // die sechste entscheidet (sechs Messungen über 45 min)
        let mut l = l;
        assert!(l.record_pool(now + STEP, 0.9, traded(5)));
        assert_eq!(l.decide(now + STEP, 3.0), Decision::Change { median: 0.9, have: 6, next: 3.5 });
        // in der Totzone ±3 %: bleibt
        let l = log_with(&[1.029, 0.971, 1.0, 1.0, 1.02, 0.98], T0, STEP);
        assert!(matches!(l.decide(T0 + 5 * STEP, 3.0), Decision::Keep { .. }));
    }

    #[test]
    fn median_statt_einzelmessung() {
        // Audit-Fall A11-O-1: ein Kauf hebt den Kurs für eine Runde über die Totzone
        let l = log_with(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.04], T0, STEP);
        assert!(matches!(l.decide(T0 + 5 * STEP, 3.0), Decision::Keep { .. }), "eine Messung darf den Zins nicht bewegen");
        // hält der Kurs die Mehrheit der Messungen über 45 min, folgt der Zins
        let l = log_with(&[1.0, 1.0, 1.04, 1.04, 1.04, 1.04], T0, STEP);
        assert_eq!(l.decide(T0 + 5 * STEP, 3.0), Decision::Change { median: 1.04, have: 6, next: 2.5 });
    }

    #[test]
    fn fenster_und_abstand() {
        let mut l = RateLog::new("mainnet");
        assert!(l.record(T0, 1.0));
        // Neustart nach 30 s: keine zweite Messung
        assert!(!l.record(T0 + 30, 1.0));
        assert!(!l.record(T0 + SAMPLE_GAP_SECS - 1, 1.0));
        assert!(l.record(T0 + SAMPLE_GAP_SECS, 1.0));
        assert!(!l.record(T0 + 2 * SAMPLE_GAP_SECS, f64::NAN));
        // nach einer Stunde fallen alte Messungen heraus
        l.prune(T0 + WINDOW_SECS);
        assert_eq!(l.samples.len(), 1);
        assert_eq!(l.window(T0 + WINDOW_SECS + SAMPLE_GAP_SECS), Vec::<f64>::new());
    }

    #[test]
    fn takt_hoechstens_einmal_je_stunde() {
        let mut l = log_with(&[0.9; 6], T0, STEP);
        let now = T0 + 5 * STEP;
        assert!(matches!(l.decide(now, 3.0), Decision::Change { .. }));
        let prev = l.reserve(now).expect("frei");
        assert_eq!(prev, None);
        // gleich danach (Neustart, zweiter Prozess): warten
        assert_eq!(l.decide(now + 30, 3.5), Decision::Wait { secs_left: EVERY_SECS - 30 });
        assert_eq!(l.reserve(now + 30), None);
        // nach einer Stunde wieder frei, mit neuen Messungen
        for i in 1..=12 {
            l.record_pool(now + i * 300, 0.9, traded(i as usize));
        }
        assert!(matches!(l.decide(now + EVERY_SECS, 3.5), Decision::Change { next, .. } if next == 4.0));
        // zurückgestellte Uhr: vorsichtig warten
        assert!(matches!(l.decide(now - 100, 3.5), Decision::Wait { .. }));
        // Vormerkung zurücknehmen, wenn nichts gesendet wurde
        let mut l2 = RateLog::new("mainnet");
        l2.last_change = Some(T0);
        let p = l2.reserve(T0 + EVERY_SECS).unwrap();
        l2.release(T0 + EVERY_SECS, p);
        assert_eq!(l2.last_change, Some(T0));
    }

    #[test]
    fn datei_ueberlebt_neustart_und_sperrt_parallele_prozesse() {
        let state = tmp("takt");
        let path = path_for(&state);
        assert!(path.ends_with("mainnet-zins.json"));
        for i in 0..6 {
            update(&path, "mainnet", |l| l.record_pool(T0 + i * 300, 0.9, traded(i as usize))).unwrap();
        }
        // „Neustart“: frisch geladen, gleiche Messungen
        let l = load(&path, "mainnet").unwrap();
        assert_eq!(l.window(T0 + 1500).len(), 6);
        // zwei Prozesse wollen gleichzeitig ändern: genau einer darf
        let now = T0 + 1500;
        let hs: Vec<_> = (0..4)
            .map(|_| {
                let p = path.clone();
                std::thread::spawn(move || update(&p, "mainnet", |l| l.reserve(now).is_some()).unwrap())
            })
            .collect();
        let granted = hs.into_iter().map(|h| h.join().unwrap()).filter(|&g| g).count();
        assert_eq!(granted, 1, "nur ein Zinsschritt je Stunde");
        assert_eq!(load(&path, "mainnet").unwrap().last_change, Some(now));
        // falsches Netz wird abgelehnt
        assert!(load(&path, "testnet-10").is_err());
        let _ = std::fs::remove_dir_all(state.parent().unwrap());
    }

    /// Audit 12 A12-3: Lief die Uhr einmal vor (hier +10 Jahre) und hat der
    /// Agent dabei eine Zinsänderung vorgemerkt, wartet die Zinsregel nach der
    /// Korrektur nur noch eine Stunde, nicht bis zu jenem Zeitpunkt.
    #[test]
    fn a12_uhr_einmal_vorgestellt_friert_den_takt_nicht_ein() {
        let mut l = RateLog::new("mainnet");
        let wrong = T0 + 10 * 365 * 86_400;
        for i in 0..6 {
            assert!(l.record_pool(wrong - 5 * STEP + i * STEP, 0.9, traded(i as usize)));
        }
        assert!(matches!(l.decide(wrong, 3.0), Decision::Change { .. }));
        assert!(l.reserve(wrong).is_some());
        // Uhr korrigiert: die erste Messung verwirft die Messungen aus der
        // „Zukunft“ und kappt den Takt auf jetzt
        assert!(l.record_pool(T0, 0.9, traded(0)));
        assert_eq!(l.last_change, Some(T0));
        assert_eq!(l.decide(T0, 3.0), Decision::Wait { secs_left: EVERY_SECS });
        // eine Stunde regelmäßig messen: dann ist die Änderung wieder erlaubt
        for i in 1..=15 {
            assert!(l.record_pool(T0 + i * SAMPLE_GAP_SECS, 0.9, traded(i as usize)));
        }
        let now = T0 + 15 * SAMPLE_GAP_SECS;
        assert!(matches!(l.decide(now, 3.0), Decision::Change { next, .. } if next == 3.5), "{:?}", l.decide(now, 3.0));
        assert!(l.reserve(now).is_some());
        // Vormerkung aus der Zukunft ohne neue Messung: reserve kappt ebenso
        let mut l2 = RateLog::new("mainnet");
        l2.last_change = Some(wrong);
        assert_eq!(l2.reserve(T0), None, "erst eine Stunde ab jetzt");
        assert_eq!(l2.last_change, Some(T0));
        assert!(l2.reserve(T0 + EVERY_SECS).is_some());
    }

    /// Audit 12 A12-12: sechs Messungen in 20 min, vier davon manipuliert,
    /// reichten für eine Zinsänderung. Jetzt müssen sie über 45 min verteilt sein.
    #[test]
    fn a12_sechs_messungen_in_zwanzig_minuten_reichen_nicht() {
        let mut l = RateLog::new("mainnet");
        for (i, v) in [1.0, 1.0, 1.04, 1.04, 1.04, 1.04].iter().enumerate() {
            assert!(l.record_pool(T0 + i as u64 * SAMPLE_GAP_SECS, *v, traded(i)));
        }
        let now = T0 + 5 * SAMPLE_GAP_SECS;
        assert_eq!(l.decide(now, 3.0), Decision::TooShort { have: 6, span_secs: 1200 });
    }

    /// Restpunkt C-O1 (A12-17 c): Ein Zins von Hand setzt den Takt. Vorher
    /// durfte die Zinsregel gleich danach wieder ändern, sobald sie genug
    /// Messungen hatte.
    #[test]
    fn a12_co1_zins_von_hand_setzt_den_takt() {
        let mut l = log_with(&[0.9; 6], T0, STEP);
        let now = T0 + 5 * STEP;
        assert!(matches!(l.decide(now, 3.0), Decision::Change { .. }));
        assert_eq!(l.set_by_hand(now), None);
        assert_eq!(l.decide(now, 5.0), Decision::Wait { secs_left: EVERY_SECS });
        // die Hand geht vor: auch 10 min nach einer Änderung der Zinsregel,
        // danach wieder eine volle Stunde
        let mut l = log_with(&[0.9; 6], T0, STEP);
        assert!(l.reserve(now).is_some());
        assert_eq!(l.set_by_hand(now + 600), Some(now));
        assert_eq!(l.decide(now + 600, 5.0), Decision::Wait { secs_left: EVERY_SECS });
        // nichts gesendet: zurück auf den vorigen Stand
        l.release(now + 600, Some(now));
        assert_eq!(l.last_change, Some(now));
        // die Messungen bleiben (derselbe Pool)
        assert_eq!(l.window(now + 600).len(), 6);
    }

    /// Restpunkt C-O1: Ein neues Deployment verwirft die Messungen des alten
    /// Pools (die Zinsdatei heißt nach dem Umzug gleich) und beginnt den Takt.
    #[test]
    fn a12_co1_deployment_beginnt_die_zinsregel_neu() {
        let mut l = log_with(&[0.9; 6], T0, STEP);
        l.last_change = Some(T0 - 2 * EVERY_SECS);
        let now = T0 + 5 * STEP;
        assert!(matches!(l.decide(now, 3.0), Decision::Change { .. }));
        l.restart(now);
        assert!(l.samples.is_empty());
        assert_eq!(l.last_change, Some(now));
        assert_eq!(l.decide(now + EVERY_SECS, 0.0), Decision::TooFew { have: 0 });
    }

    /// A12-12: Bei regelmäßigen Messungen (alle SAMPLE_GAP_SECS) verschiebt ein
    /// Kurs, der 20 min gehalten wird, den Median nie, egal wo er im Fenster
    /// liegt. Erst ab gut 24 min (mehr als die Hälfte der Messungen) folgt der Zins.
    #[test]
    fn a12_zwanzig_minuten_manipulation_bewegen_den_zins_nicht() {
        let gap = SAMPLE_GAP_SECS;
        let run = |start: u64, hold: u64| -> bool {
            // 2 h regelmäßig messen, Kurs 1,006 von start bis start + hold, sonst 1,0
            let mut l = RateLog::new("mainnet");
            let mut changed = false;
            for i in 0..30u64 {
                let t = T0 + i * gap;
                let v = if t >= T0 + start && t <= T0 + start + hold { 1.04 } else { 1.0 };
                assert!(l.record_pool(t, v, traded(i as usize)));
                changed |= matches!(l.decide(t, 3.0), Decision::Change { .. });
            }
            changed
        };
        for start in (0..=3600).step_by(60) {
            assert!(!run(start, 20 * 60), "20 min ab {start} s dürfen den Zins nicht ändern");
        }
        // Gegenprobe: 30 min gehalten ändern ihn
        assert!(run(1800, 30 * 60));
    }

    /// Audit 20 A20e-6 (Entscheidung des Betreibers „Totzone + nur bei Handel“):
    /// Liegt der Median außerhalb der Totzone, ändert sich der Zins nur, wenn
    /// das Tauschverhältnis des Pools sich im Fenster um ≥ 2 % bewegt hat.
    /// Ohne Handel (reine KAS-Drift) bleibt er, in beide Richtungen.
    #[test]
    fn a20e_6_ohne_handel_keine_zinsaenderung() {
        let now = T0 + 5 * STEP;
        // KAS fällt, der Pool wird nicht gehandelt: Messung 0,95, Verhältnis fest
        let mut l = RateLog::new("mainnet");
        for i in 0..6 {
            assert!(l.record_pool(T0 + i * STEP, 0.95, Some(25.0)));
        }
        assert_eq!(l.traded_move(now), 0.0);
        assert!(matches!(l.decide(now, 3.0), Decision::NoTrade { have: 6, .. }), "{:?}", l.decide(now, 3.0));
        // ebenso nach oben (über 1,03) und ohne Verhältnis (alte Messungen)
        let mut l = RateLog::new("mainnet");
        for i in 0..6 {
            assert!(l.record(T0 + i * STEP, 1.05));
        }
        assert!(matches!(l.decide(now, 5.0), Decision::NoTrade { .. }));
        // ein einzelner kleiner Tausch (0,5 %) reicht nicht
        let mut l = RateLog::new("mainnet");
        for i in 0..6 {
            let r = if i < 3 { 25.0 } else { 25.0 * 1.005 };
            assert!(l.record_pool(T0 + i * STEP, 0.95, Some(r)));
        }
        assert!(matches!(l.decide(now, 3.0), Decision::NoTrade { moved, .. } if (moved - 1.005f64.ln()).abs() < 1e-9));
        // Handel von zusammen 2 % (z. B. Käufe Richtung Band): Zins folgt
        let mut l = RateLog::new("mainnet");
        for i in 0..6 {
            let r = 25.0 * (1.0 + 0.005 * i as f64);
            assert!(l.record_pool(T0 + i * STEP, 0.95, Some(r)));
        }
        assert!(l.traded_move(now) >= MIN_TRADE_MOVE);
        assert_eq!(l.decide(now, 3.0), Decision::Change { median: 0.95, have: 6, next: 3.5 });
        // in der Totzone hilft auch viel Handel nicht
        let l = log_with(&[0.975, 1.025, 0.98, 1.02, 1.0, 1.0], T0, STEP);
        assert!(matches!(l.decide(now, 3.0), Decision::Keep { .. }));
    }

    /// Das Tauschverhältnis steht in der Zinsdatei; alte Dateien ohne Feld
    /// lassen sich lesen (Messungen zählen dann nicht als Handel)
    #[test]
    fn a20e_6_zinsdatei_mit_und_ohne_tauschverhaeltnis() {
        let alt = r#"{"network":"mainnet","samples":[{"at":1,"ghostUsd":0.9}],"lastChange":null}"#;
        let l: RateLog = serde_json::from_str(alt).unwrap();
        assert_eq!(l.samples[0].pool_ratio, None);
        let mut l = RateLog::new("mainnet");
        l.record_pool(T0, 0.9, pool_ratio(250 * math::E8, 10 * math::E8));
        let j = serde_json::to_string(&l).unwrap();
        assert!(j.contains("\"poolRatio\":25.0"), "{j}");
        assert_eq!(pool_ratio(1, 0), None);
    }
}
