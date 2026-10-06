//! Exakte Referenz der Vertragsrechnung (u128) für stable_vault.sil Version 3.
//! Die Vertragsversion ist in tests/vault_math_tests.rs und tests/vault_tests.rs
//! gegen genau diese Formeln geprüft.

use crate::contracts::VaultState;

pub const E8: i64 = 100_000_000;

pub fn value_of(coll: i64, price: i64) -> i64 {
    (coll as u128 * price as u128 / E8 as u128) as i64
}

/// Zinswachstum × 1e9 (stable_vault.sil growth): ⌈(to − from)·1e9/from⌉,
/// gedeckelt auf MAX_GROWTH (Index ×10 je Abrechnung)
pub const MAX_GROWTH: i64 = 9_000_000_000;
pub fn growth(from: i64, to: i64) -> i64 {
    let g = ((to - from) as u128 * 1_000_000_000).div_ceil(from as u128);
    g.min(MAX_GROWTH as u128) as i64
}

/// Aufgelaufener Zins bis zum Orakelindex `index` (stable_vault.sil accrued):
/// interest + ⌈(debt + interest)·growth/1e9⌉ – offener Zins verzinst sich mit
/// (Audit 11 A11-V-5); ohne Schuld und Zins wächst nichts.
pub fn accrued(s: &VaultState, index: i64) -> i64 {
    let base = s.debt + s.interest;
    if base > 0 && index > s.index_at && s.index_at > 0 {
        s.interest + (base as u128 * growth(s.index_at, index) as u128).div_ceil(1_000_000_000) as i64
    } else {
        s.interest
    }
}

/// Offen insgesamt: GHOST-Schuld + Zins (USD × 1e8, 1 GHOST ≙ 1 USD)
pub fn owed(s: &VaultState, index: i64) -> i64 {
    s.debt + accrued(s, index)
}

/// Zustand nach der Abrechnung bis `index` (Zins festgeschrieben)
pub fn settled(s: &VaultState, index: i64) -> VaultState {
    VaultState { debt: s.debt, interest: accrued(s, index), index_at: index }
}

/// Sicherheit·Preis ≥ offen·Quote (nichts offen ist immer gesund)
pub fn healthy(coll: i64, owed: i64, price: i64, ratio_bps: i64) -> bool {
    owed == 0 || value_of(coll, price) as u128 >= (owed as u128 * ratio_bps as u128).div_ceil(10_000)
}

/// Größter prägbarer Betrag: ⌈o·m/1e4⌉ ≤ v ⇔ o ≤ ⌊v·1e4/m⌋
pub fn max_mint(coll: i64, s: &VaultState, price: i64, index: i64, mcr_bps: i64) -> i64 {
    let max_owed = (value_of(coll, price) as u128 * 10_000 / mcr_bps as u128) as i64;
    (max_owed - owed(s, index)).max(0)
}

/// Wie viel GHOST die Höchstschuld je Vault noch zulässt (mint: newDebt ≤ maxDebt)
pub fn cap_room(s: &VaultState, max_debt: i64) -> i64 {
    if max_debt >= i64::MAX / 4 {
        return i64::MAX;
    }
    (max_debt - s.debt).max(0)
}

/// Rücknahme: so viel bleibt als Ausgleich beim Vault-Besitzer (1 %)
pub const REDEEM_FEE_BPS: i64 = 100;
/// Kleinste Rücknahme (1 GHOST), außer der ganzen Schuld
pub const MIN_REDEEM: i64 = 100_000_000;

/// KAS (sompi) für die Rücknahme von `amount` GHOST (stable_vault.sil redeem)
pub fn redeem_paid(amount: i64, price: i64) -> i64 {
    let usd = amount as u128 * (10_000 - REDEEM_FEE_BPS) as u128 / 10_000;
    (usd * E8 as u128 / price as u128) as i64
}

/// Nimmt redeem() `amount` an? Rückgabe: (sompi an den Einlöser, neuer Zustand)
pub fn redemption(coll: i64, s: &VaultState, amount: i64, price: i64, index: i64, liq_bps: i64) -> Option<(i64, VaultState)> {
    if amount <= 0 || amount > s.debt || (amount < MIN_REDEEM && amount != s.debt) || !healthy(coll, owed(s, index), price, liq_bps) {
        return None;
    }
    let paid = redeem_paid(amount, price);
    if paid <= 0 || coll - paid < DUST {
        return None;
    }
    let st = settled(s, index);
    Some((paid, VaultState { debt: s.debt - amount, ..st }))
}

/// KAS (sompi), die ein Liquidator erhält — wie stable_vault.sil liquidate().
pub fn seize(coll: i64, debt: i64, price: i64, bonus_bps: i64) -> i64 {
    let claim = (debt as u128 * (10_000 + bonus_bps) as u128).div_ceil(10_000) as i64;
    if value_of(coll, price) > claim { (claim as u128 * E8 as u128).div_ceil(price as u128) as i64 } else { coll }
}

pub const DUST: i64 = 20_000_000;

/// Netzgebühr, die eine Auflösung zugunsten der Zinskasse tragen darf (sweep)
pub const SWEEP_FEE: i64 = 10_000_000;

/// Kleinster Betrag an die Zinskasse, mit dem sich eine Auflösung sicher bauen
/// lässt (Audit 12 A12-2). Der Kassen-Ausgang (Sicherheit − SWEEP_FEE) ist ein
/// eigener Ausgang; unter ≈ 0,02 KAS ist seine Speichermasse (KIP-9,
/// ≈ 10^12/Betrag Gramm) größer als ein Block erlaubt, bei Sicherheit ≤
/// SWEEP_FEE gäbe es ihn gar nicht. Gemessen in tests/sweep_grenze_tests.rs;
/// mit Abstand zur Grenze 0,025 KAS wie SMALL_OUTPUT auf der Seite.
pub const SWEEP_MIN_TREASURY: i64 = 2_500_000;

/// Zinsgebühr beim Schließen in sompi: ⌈Zins·1e8/Preis⌉, höchstens die
/// Sicherheit. Sättigend in u128 gerechnet, nie negativ (Audit 12 A12-14:
/// vorher schnitt `as i64` einen Wert über i64 ins Negative ab).
pub fn interest_fee(coll: i64, s: &VaultState, price: i64, index: i64) -> i64 {
    let fee = (accrued(s, index) as u128 * E8 as u128).div_ceil(price as u128);
    fee.min(coll.max(0) as u128) as i64
}

/// Zinsgebühr genau wie stable_vault.sil interestFee: None, wenn der Vertrag
/// dabei über 64 Bit läuft (NumberTooBig). Dann scheitern close und sweep, bis
/// der KAS-Preis steigt. Die Grenze folgt nicht aus MAX_DEBT/MAX_INTEREST,
/// sondern hängt vom Preis ab: ⌈Zins·1e8/Preis⌉ > i64::MAX, beim Tiefstpreis
/// des Orakels (0,00001 USD) ab ≈ 922 000 USD Zins (Audit 12 A12-14).
pub fn interest_fee_checked(coll: i64, s: &VaultState, price: i64, index: i64) -> Option<i64> {
    let fee = (accrued(s, index) as u128 * E8 as u128).div_ceil(price as u128);
    i64::try_from(fee).ok().map(|f| f.min(coll))
}

/// Lässt stable_vault.sil sweep den Vault zu? Schuld 0 und der Zins zehrt die
/// ganze Sicherheit auf (der Vertrag allein, ohne Blick auf die Tx-Größe).
pub fn sweep_allowed(coll: i64, s: &VaultState, price: i64, index: i64) -> bool {
    s.debt == 0 && interest_fee_checked(coll, s, price, index) == Some(coll)
}

/// Darf jeder den Vault zugunsten der Zinskasse auflösen (sweep), UND lässt
/// sich die Tx bauen? Wie `sweep_allowed`, zusätzlich muss nach SWEEP_FEE
/// mindestens SWEEP_MIN_TREASURY für die Kasse bleiben (Audit 12 A12-2: ein
/// Zombie-Vault unter 0,1 KAS ließ ops::sweep überlaufen und blockierte den
/// Keeper für alle anderen).
pub fn sweepable(coll: i64, s: &VaultState, price: i64, index: i64) -> bool {
    coll >= SWEEP_FEE + SWEEP_MIN_TREASURY && sweep_allowed(coll, s, price, index)
}

/// Mindestgewinn des Keepers zum Marktpreis in bps (Audit 10, A10-A-2)
pub const KEEPER_MARGIN_BPS: i64 = 200;

/// Nimmt stable_vault.sil liquidate() `burn` an? Rückgabe: (sompi an den
/// Liquidator, Zustand danach – None, wenn der Vault endet). Spiegelt die drei
/// Zweige des Vertrags; bleibt ein Vault übrig, bleibt auch sein Zins stehen.
pub fn liquidation(coll: i64, s: &VaultState, burn: i64, price: i64, index: i64, liq_bps: i64, bonus_bps: i64) -> Option<(i64, Option<VaultState>)> {
    let owed_interest = accrued(s, index);
    if healthy(coll, s.debt + owed_interest, price, liq_bps) || burn <= 0 || burn > s.debt {
        return None;
    }
    let got = seize(coll, burn, price, bonus_bps);
    if got == coll {
        return Some((got, None));
    }
    if coll - got < DUST {
        return (burn == s.debt).then_some((got, None));
    }
    // der Zins bleibt stehen (stable_vault.sil liquidate)
    Some((got, Some(VaultState { debt: s.debt - burn, interest: owed_interest, index_at: index })))
}

/// Lohnt sich die Liquidation zum Marktpreis? Der Vertrag rechnet mit dem
/// Orakelpreis; liegt der Markt darunter, kann der Keeper sonst verlieren.
#[allow(clippy::too_many_arguments)]
pub fn keeper_profitable(coll: i64, s: &VaultState, burn: i64, oracle_price: i64, market_price: i64, index: i64, liq_bps: i64, bonus_bps: i64) -> bool {
    match liquidation(coll, s, burn, oracle_price, index, liq_bps, bonus_bps) {
        Some((got, _)) => value_of(got, market_price) as i128 * 10_000 >= burn as i128 * (10_000 + KEEPER_MARGIN_BPS) as i128,
        None => false,
    }
}

/// Entscheidung des Keeper-Agenten für einen Vault: wie viele GHOST verbrennen?
/// None = nicht liquidieren. Liquidiert wird nur, wenn der Vault nach dem
/// Orakelpreis UND nach dem Marktpreis unter der Liquidationsschwelle liegt,
/// und höchstens Wert/(1+Bonus) (aufgerundet, Audit 10 A10-A-1). Die Menge
/// muss der Vertrag annehmen, und zum Marktpreis muss KEEPER_MARGIN_BPS
/// Gewinn bleiben (A10-A-2).
pub fn keeper_burn(coll: i64, s: &VaultState, oracle_price: i64, market_price: i64, index: i64, liq_bps: i64, bonus_bps: i64) -> Option<i64> {
    let o = owed(s, index);
    if s.debt == 0 || healthy(coll, o, oracle_price, liq_bps) || healthy(coll, o, market_price, liq_bps) {
        return None;
    }
    let value = value_of(coll, oracle_price) as i128;
    let d = (10_000 + bonus_bps) as i128;
    let fair = ((value * 10_000 + d - 1) / d) as i64;
    let burn = s.debt.min(fair);
    (burn > 0 && keeper_profitable(coll, s, burn, oracle_price, market_price, index, liq_bps, bonus_bps)).then_some(burn)
}

/// Zinsregel des Agenten (Version 3), in % p. a.: Liegt GHOST unter 0,995 USD,
/// steigt der Zins um RATE_STEP_PCT (Schulden werden teurer, Schuldner kaufen
/// GHOST und tilgen), über 1,005 USD sinkt er (Prägen lohnt sich wieder).
/// Grenzen RATE_MIN_PCT … RATE_MAX_PCT. None = unverändert lassen.
pub const RATE_STEP_PCT: f64 = 0.5;
pub const RATE_MAX_PCT: f64 = 20.0;
/// Grundzins (Entscheidung des Betreibers 06.10.2026): Die Regel senkt nie
/// darunter. Liegt der Zins darunter (Start mit 0 %), hebt `rate_floor_step`
/// ihn in Vertragsschritten (0,5 Punkte, höchstens einmal je Stunde) an.
pub const RATE_MIN_PCT: f64 = 2.0;

/// Nächster Schritt zum Grundzins, wenn der Zins darunter liegt (unabhängig vom
/// GHOST-Kurs und von der Pool-Liquidität). None = Grundzins erreicht.
pub fn rate_floor_step(current_pct: f64) -> Option<f64> {
    if !current_pct.is_finite() || current_pct >= RATE_MIN_PCT - 1e-9 {
        return None;
    }
    let cur = (current_pct.max(0.0) / RATE_STEP_PCT).round() * RATE_STEP_PCT;
    let next = (cur + RATE_STEP_PCT).min(RATE_MIN_PCT);
    ((next - current_pct).abs() > 1e-9).then_some(next)
}
pub fn rate_next(current_pct: f64, ghost_usd: f64) -> Option<f64> {
    if !ghost_usd.is_finite() || ghost_usd <= 0.0 || !current_pct.is_finite() {
        return None;
    }
    // erst auf Rahmen und Raster bringen, dann einen Schritt: ein Satz außerhalb
    // (von Hand gesetzt) ergab sonst Schritte wie +0,27 oder senkte bei „rauf“
    // (Audit 11 A11-O-10)
    let cur = ((current_pct.clamp(0.0, RATE_MAX_PCT)) / RATE_STEP_PCT).round() * RATE_STEP_PCT;
    let next = if ghost_usd < 0.995 {
        (cur + RATE_STEP_PCT).min(RATE_MAX_PCT)
    } else if ghost_usd > 1.005 {
        // nie unter den Grundzins; liegt er schon darunter, hebt rate_floor_step an
        if cur <= RATE_MIN_PCT + 1e-9 {
            return None;
        }
        (cur - RATE_STEP_PCT).max(RATE_MIN_PCT)
    } else {
        return None;
    };
    ((next - current_pct).abs() > 1e-9).then_some(next)
}

#[cfg(test)]
mod keeper_tests {
    use super::*;

    const I0: i64 = 1_000_000_000;
    fn debt(d: i64) -> VaultState {
        VaultState { debt: d, interest: 0, index_at: I0 }
    }

    #[test]
    fn keeper_burn_ist_nie_ein_verlust() {
        let e8 = 100_000_000i64;
        let coll = 10_000 * e8;
        let s = debt(150 * e8);
        // Unterdeckung: 10 000 KAS à 0,012 USD = 120 USD < 150 GHOST Schuld
        let b = keeper_burn(coll, &s, 1_200_000, 1_200_000, I0, 15_000, 1_000).expect("liquidierbar");
        assert!(b < s.debt, "nur so viel, wie die Sicherheit deckt");
        let (got, _) = liquidation(coll, &s, b, 1_200_000, I0, 15_000, 1_000).expect("Vertrag nimmt an");
        assert!(value_of(got, 1_200_000) as i128 * 10_000 >= b as i128 * 10_200, "kein Verlust");
        assert!(b > 108 * e8, "trotzdem fast der ganze Wert: {b}");
        assert_eq!(keeper_burn(coll, &s, 1_200_000, 4_000_000, I0, 15_000, 1_000), None, "gesund nach Markt");
        assert_eq!(keeper_burn(coll, &debt(0), 1_200_000, 1_200_000, I0, 15_000, 1_000), None, "keine Schuld");
    }

    /// A10-A-1: im ganzen Band 100–110 % nimmt der Vertrag die Menge des Keepers an
    #[test]
    fn keeper_burn_wird_im_band_unter_110_prozent_nie_abgelehnt() {
        let e8 = 100_000_000i64;
        let coll = 1_000 * e8;
        let s = debt(20 * e8);
        let mut n = 0;
        for price in (2_000_000..2_240_000).step_by(7) {
            if let Some(b) = keeper_burn(coll, &s, price, price, I0, 15_000, 1_000) {
                let got = liquidation(coll, &s, b, price, I0, 15_000, 1_000);
                assert!(got.is_some(), "Vertrag lehnt ab bei Preis {price}, burn {b}");
                assert!(value_of(got.unwrap().0, price) >= b, "Verlust bei Preis {price}");
                n += 1;
            }
        }
        assert!(n > 30_000, "Band geprüft: {n}");
    }

    /// A10-A-2: Orakel über dem Markt – kein Verlustgeschäft zum Marktpreis
    #[test]
    fn keeper_liquidiert_nicht_mit_verlust_zum_marktpreis() {
        let e8 = 100_000_000i64;
        let coll = 10_000 * e8;
        let s = debt(150 * e8);
        assert_eq!(keeper_burn(coll, &s, 2_000_000, 1_500_000, I0, 15_000, 1_000), None);
        assert!(keeper_burn(coll, &s, 2_000_000, 1_950_000, I0, 15_000, 1_000).is_some());
    }

    /// Obergrenze: ohne Grenze unbegrenzt, sonst max_debt − debt
    #[test]
    fn cap_room_je_vault() {
        assert_eq!(cap_room(&debt(0), i64::MAX), i64::MAX);
        assert_eq!(cap_room(&debt(12), 5_000_000_000), 4_999_999_988);
        assert_eq!(cap_room(&debt(6_000_000_000), 5_000_000_000), 0);
    }

    /// Zins: Schuld und offener Zins wachsen mit dem Index ab indexAt,
    /// aufgerundet; ohne beides nichts
    #[test]
    fn zins_waechst_mit_dem_index() {
        let s = VaultState { debt: 100 * E8, interest: 7, index_at: I0 };
        assert_eq!(accrued(&s, I0), 7);
        // Index +5 %: ⌈(100 GHOST + 7)·5 %⌉ = 5 GHOST-Wert + 1 Einheit dazu
        assert_eq!(accrued(&s, I0 * 105 / 100), 7 + 5 * E8 + 1);
        // aufrunden: 1 Einheit Indexzuwachs bei 1 GHOST Schuld
        assert_eq!(accrued(&VaultState { debt: E8, interest: 0, index_at: I0 }, I0 + 1), 1);
        // offener Zins ohne Schuld verzinst sich mit (Index ×2 ⇒ ×2)
        assert_eq!(accrued(&VaultState { debt: 0, interest: 9, index_at: I0 }, 2 * I0), 18);
        assert_eq!(accrued(&VaultState { debt: 0, interest: 0, index_at: I0 }, 2 * I0), 0);
        assert_eq!(owed(&s, I0 * 105 / 100), 105 * E8 + 8);
        // prägbar: 1 000 KAS à 0,05 USD = 50 USD bei 200 % → 25 minus offen
        assert_eq!(max_mint(1_000 * E8, &debt(10 * E8), 5_000_000, I0, 20_000), 15 * E8);
    }

    /// Zinsregel: unter 0,995 rauf, über 1,005 runter, dazwischen nichts; Grenzen
    #[test]
    fn zinsregel_folgt_dem_ghost_kurs() {
        assert_eq!(rate_next(0.0, 0.97), Some(0.5));
        assert_eq!(rate_next(3.0, 0.99), Some(3.5));
        assert_eq!(rate_next(3.0, 1.0), None);
        assert_eq!(rate_next(3.0, 1.004), None);
        assert_eq!(rate_next(3.0, 1.02), Some(2.5));
        assert_eq!(rate_next(0.0, 1.02), None, "nicht unter 0");
        assert_eq!(rate_next(2.5, 1.02), Some(2.0));
        assert_eq!(rate_next(2.0, 1.02), None, "nicht unter den Grundzins");
        assert_eq!(rate_next(1.0, 1.02), None, "unter dem Grundzins nicht weiter runter");
        assert_eq!(rate_floor_step(0.0), Some(0.5));
        assert_eq!(rate_floor_step(1.5), Some(2.0));
        assert_eq!(rate_floor_step(1.8), Some(2.0));
        assert_eq!(rate_floor_step(2.0), None);
        assert_eq!(rate_floor_step(5.0), None);
        assert_eq!(rate_next(20.0, 0.5), None, "nicht über 20 %");
        assert_eq!(rate_next(19.8, 0.5), Some(20.0));
        assert_eq!(rate_next(1.0, f64::NAN), None);
        // außerhalb von Raster oder Rahmen: erst einrasten, dann ein Schritt
        assert_eq!(rate_next(3.23, 0.9), Some(3.5));
        assert_eq!(rate_next(31.5, 0.9), Some(20.0), "rauf senkt nicht");
        assert_eq!(rate_next(31.5, 1.1), Some(19.5));
    }

    /// Rücknahme: 1 USD je GHOST minus 1 %, nur ab der Liquidationsschwelle
    #[test]
    fn ruecknahme_zahlt_einen_dollar_je_ghost() {
        let s = debt(10 * E8);
        // 0,05 USD je KAS: 10 GHOST ⇒ 9,90 USD ⇒ 198 KAS
        let (paid, after) = redemption(1_000 * E8, &s, 10 * E8, 5_000_000, I0, 15_000).expect("erlaubt");
        assert_eq!(paid, 198 * E8);
        // unter 1 GHOST nur, wenn es die ganze Schuld ist
        assert!(redemption(1_000 * E8, &s, E8 - 1, 5_000_000, I0, 15_000).is_none());
        assert!(redemption(1_000 * E8, &debt(E8 / 2), E8 / 2, 5_000_000, I0, 15_000).is_some());
        assert_eq!(after.debt, 0);
        // unter 150 % (Liquidatoren sind dran): nicht erlaubt
        assert!(redemption(250 * E8, &s, E8, 5_000_000, I0, 15_000).is_none());
        // mehr als die Schuld: nicht erlaubt
        assert!(redemption(1_000 * E8, &s, 11 * E8, 5_000_000, I0, 15_000).is_none());
    }
}
