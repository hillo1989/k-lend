//! Dauerauftrag mit Tresor (contracts/standing_order.sil): Terminrechnung wie
//! im Vertrag, aber unabhängig davon mit chrono gerechnet – die Tests prüfen
//! beide gegeneinander.

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};

pub const DAY_MS: i64 = 86_400_000;
/// Termine höchstens bis 2200 (Vertrag: MAX_TIME)
pub const MAX_TIME: i64 = 7_258_118_400_000;

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let first_next = NaiveDate::from_ymd_opt(ny, nm, 1).expect("Datum");
    first_next.pred_opt().expect("Datum").day()
}

/// Termin nach `due` (Unix-ms, UTC): monatlich am Tag min(anchor, Monatslänge)
/// zur selben Uhrzeit, oder `due + period_ms` bei anchor 0.
pub fn following(due: i64, anchor: i64, period_ms: i64) -> i64 {
    if anchor <= 0 {
        return due + period_ms;
    }
    let t: DateTime<Utc> = Utc.timestamp_millis_opt(due).single().expect("Zeit");
    let tod = due.rem_euclid(DAY_MS);
    let (y, m) = (t.year(), t.month());
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let day = (anchor as u32).min(days_in_month(ny, nm));
    let date = NaiveDate::from_ymd_opt(ny, nm, day).expect("Datum");
    date.and_hms_opt(0, 0, 0).expect("Zeit").and_utc().timestamp_millis() + tod
}

/// Die nächsten `n` Termine ab dem ersten (für die Vorschau)
pub fn schedule(first: i64, anchor: i64, period_ms: i64, n: usize) -> Vec<i64> {
    let mut out = Vec::with_capacity(n);
    let mut t = first;
    for _ in 0..n {
        out.push(t);
        t = following(t, anchor, period_ms);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(y: i32, m: u32, d: u32, h: u32) -> i64 {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, 0, 0).unwrap().and_utc().timestamp_millis()
    }

    #[test]
    fn monatsende_und_schaltjahr() {
        // 31. Januar → 28./29. Februar → 31. März (Anker bleibt der 31.)
        assert_eq!(following(ms(2027, 1, 31, 8), 31, 0), ms(2027, 2, 28, 8));
        assert_eq!(following(ms(2027, 2, 28, 8), 31, 0), ms(2027, 3, 31, 8));
        assert_eq!(following(ms(2028, 1, 31, 8), 31, 0), ms(2028, 2, 29, 8));
        assert_eq!(following(ms(2027, 12, 1, 0), 1, 0), ms(2028, 1, 1, 0));
        assert_eq!(following(ms(2100, 1, 29, 0), 29, 0), ms(2100, 2, 28, 0), "2100 ist kein Schaltjahr");
        assert_eq!(following(ms(2027, 3, 3, 0), 0, 7 * DAY_MS), ms(2027, 3, 10, 0));
        assert_eq!(schedule(ms(2027, 1, 30, 0), 30, 0, 3), vec![ms(2027, 1, 30, 0), ms(2027, 2, 28, 0), ms(2027, 3, 30, 0)]);
    }
}
