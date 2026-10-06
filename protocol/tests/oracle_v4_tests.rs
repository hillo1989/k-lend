//! Tests für contracts/price_oracle_v4.sil gegen die echte Skript-Engine.
//! Jedes Update läuft als ganze Tx mit dem Unterzeichner-Register
//! (signer_register_v4.sil). Jede Regel hat eine Gegenprobe: dieselbe Tx mit
//! ehrlichen Werten geht durch, und es scheitert der gemeinte Eingang.

mod v4common;

use v4common::*;

const REG: usize = 0;
const ORA: usize = 1;

fn w() -> World {
    World::standard()
}

/// nur das Orakel lehnt ab (das Register akzeptiert die signierten Werte)
fn only_oracle_fails(r: &[Result<(), String>]) -> bool {
    r[REG].is_ok() && r[ORA].is_err()
}

#[test]
fn ehrliches_update_geht_durch() {
    let w = w();
    let a = Attest::new(4_100_000, w.oracle.daa + 600, w.oracle.rate);
    let r = a.run(&w);
    assert!(all_ok(&r), "{r:?}");
    let e = a.expected(&w);
    assert_eq!(e.seq, w.oracle.seq + 1);
    assert!(e.index > w.oracle.index, "Index wächst mit dem alten Satz");
}

#[test]
fn update_ohne_register_scheitert() {
    // Kern von Variante C: das Orakel prüft keine Signaturen, sondern verlangt
    // genau einen Register-Eingang
    let w = w();
    let mut a = Attest::new(8_000_000, w.oracle.daa + 600, w.oracle.rate);
    a.with_register = false;
    let r = a.run(&w);
    assert!(r[0].is_err(), "Orakel allein: {r:?}");
    a.with_register = true;
    assert!(all_ok(&a.run(&w)), "Gegenprobe mit Register");
}

#[test]
fn preisgrenzen_wie_v3() {
    let mut w = w();
    let d = w.oracle.daa + 600;
    let rate = w.oracle.rate;
    // ×2 / ÷2 je Update
    assert!(all_ok(&Attest::new(8_000_000, d, rate).run(&w)));
    assert!(only_oracle_fails(&Attest::new(8_000_001, d, rate).run(&w)));
    assert!(all_ok(&Attest::new(2_000_000, d, rate).run(&w)));
    assert!(only_oracle_fails(&Attest::new(1_999_999, d, rate).run(&w)));
    // absolute Grenzen 0,00001 USD und 900 USD
    w.oracle.kas_usd = 1_500;
    assert!(all_ok(&Attest::new(1_000, d, rate).run(&w)));
    assert!(only_oracle_fails(&Attest::new(999, d, rate).run(&w)));
    w.oracle.kas_usd = 60_000_000_000;
    assert!(all_ok(&Attest::new(90_000_000_000, d, rate).run(&w)));
    assert!(only_oracle_fails(&Attest::new(90_000_000_001, d, rate).run(&w)));
}

#[test]
fn mindestabstand_und_keine_zukunft() {
    let w = w();
    let rate = w.oracle.rate;
    let k = w.oracle.kas_usd;
    assert!(all_ok(&Attest::new(k, w.oracle.daa + 600, rate).run(&w)));
    assert!(only_oracle_fails(&Attest::new(k, w.oracle.daa + 599, rate).run(&w)));
    // Preis aus der Zukunft: Locktime unter newOracleDaa
    let mut a = Attest::new(k, w.oracle.daa + 1_000, rate);
    a.lock_time = Some((w.oracle.daa + 999) as u64);
    let r = a.run(&w);
    assert!(r[ORA].is_err(), "{r:?}");
    a.lock_time = Some((w.oracle.daa + 1_000) as u64);
    assert!(all_ok(&a.run(&w)));
}

#[test]
fn zins_im_rahmen() {
    let mut w = w();
    let k = w.oracle.kas_usd;
    // Zinsänderung erst rateGap nach der letzten
    w.oracle.last_rate_daa = w.oracle.daa;
    let step = w.ocfg.rate_step;
    let early = w.oracle.daa + w.ocfg.rate_gap - 1;
    let on_time = w.oracle.daa + w.ocfg.rate_gap;
    assert!(only_oracle_fails(&Attest::new(k, early, w.oracle.rate + 1).run(&w)), "zu früh");
    assert!(all_ok(&Attest::new(k, early, w.oracle.rate).run(&w)), "gleicher Satz braucht keinen Abstand");
    assert!(all_ok(&Attest::new(k, on_time, w.oracle.rate + step).run(&w)), "ein Schritt hinauf");
    assert!(only_oracle_fails(&Attest::new(k, on_time, w.oracle.rate + step + 1).run(&w)), "Schritt zu groß hinauf");
    assert!(all_ok(&Attest::new(k, on_time, w.oracle.rate - step).run(&w)), "ein Schritt hinab");
    assert!(only_oracle_fails(&Attest::new(k, on_time, w.oracle.rate - step - 1).run(&w)), "Schritt zu groß hinab");
    // Rahmen 0 … maxRate
    w.oracle.rate = 5;
    w.oracle.last_rate_daa = 0;
    assert!(all_ok(&Attest::new(k, on_time, 0).run(&w)));
    assert!(only_oracle_fails(&Attest::new(k, on_time, -1).run(&w)));
    w.oracle.rate = w.ocfg.max_rate - 5;
    assert!(all_ok(&Attest::new(k, on_time, w.ocfg.max_rate).run(&w)));
    assert!(only_oracle_fails(&Attest::new(k, on_time, w.ocfg.max_rate + 1).run(&w)));
}

#[test]
fn zinsaenderung_setzt_last_rate_daa() {
    let mut w = w();
    w.oracle.last_rate_daa = 0;
    let d = w.oracle.daa + 600;
    let new_rate = w.oracle.rate + 1;
    let ok = Attest::new(w.oracle.kas_usd, d, new_rate);
    assert_eq!(ok.expected(&w).last_rate_daa, d);
    assert!(all_ok(&ok.run(&w)));
    // Ausgang mit altem lastRateDaa: das Orakel lehnt ab (Register nimmt das Argument)
    let mut bad = ok.clone();
    bad.oracle_out = Some(OSt { last_rate_daa: 0, ..ok.expected(&w) });
    assert!(only_oracle_fails(&bad.run(&w)));
    // ohne Änderung bleibt lastRateDaa stehen
    let same = Attest::new(w.oracle.kas_usd, d, w.oracle.rate);
    assert_eq!(same.expected(&w).last_rate_daa, 0);
    let mut moved = same.clone();
    moved.oracle_out = Some(OSt { last_rate_daa: d, ..same.expected(&w) });
    assert!(only_oracle_fails(&moved.run(&w)));
}

#[test]
fn index_und_seq_rechnet_das_orakel() {
    let w = w();
    let a = Attest::new(w.oracle.kas_usd, w.oracle.daa + 36_000, w.oracle.rate);
    let e = a.expected(&w);
    for bad in [OSt { index: e.index + 1, ..e }, OSt { index: e.index - 1, ..e }, OSt { seq: e.seq + 1, ..e }] {
        let mut x = a.clone();
        x.oracle_out = Some(bad);
        x.arg_seq = Some(bad.seq);
        let r = x.run(&w);
        // falsche seq signiert das Quorum nicht → auch das Register scheitert
        assert!(r[ORA].is_err(), "{bad:?}: {r:?}");
    }
}

#[test]
fn update_taut_auf_und_setzt_frozen_false() {
    let mut w = w();
    w.oracle.frozen = true;
    let a = Attest::next(&w);
    assert!(!a.expected(&w).frozen);
    assert!(all_ok(&a.run(&w)), "Update taut auf");
    // Ausgang bleibt eingefroren: Orakel und Register lehnen ab
    let mut x = a.clone();
    x.oracle_out = Some(OSt { frozen: true, ..a.expected(&w) });
    let r = x.run(&w);
    assert!(r[ORA].is_err() && r[REG].is_err(), "{r:?}");
}

#[test]
fn fortsetzung_behaelt_den_betrag() {
    let w = w();
    let a = Attest::next(&w);
    let e = a.expected(&w);
    let reg_out = w.reg_out(&w.reg_after_price(a.daa), 0);
    let mk = |value: i64| {
        let mut args = vec![i(a.kas), i(a.daa), i(e.seq), i(a.rate), i(e.index), i(e.last_rate_daa)];
        args.extend(w.set.args());
        args.extend(w.set.quorum(price_digest(ORACLE_COV, a.kas, a.daa, e.seq, a.rate), &Satz::first(w.set.t)));
        execute(
            vec![w.reg_in(&w.reg, "attestPrice", args), w.oracle_in(w.oracle, "update", vec![i(a.kas), i(a.daa), i(a.rate)])],
            vec![reg_out.clone(), cov_out(&w.oracle_art(e), value, 1, ORACLE_COV)],
            a.daa as u64,
            vec![],
        )
    };
    assert!(all_ok(&mk(ORACLE_V)));
    let r = mk(ORACLE_V - 1);
    assert!(r[ORA].is_err() && r[REG].is_ok(), "{r:?}");
}

#[test]
fn update_mit_zwei_fortsetzungen_scheitert() {
    let w = w();
    let a = Attest::next(&w);
    let e = a.expected(&w);
    let mut args = vec![i(a.kas), i(a.daa), i(e.seq), i(a.rate), i(e.index), i(e.last_rate_daa)];
    args.extend(w.set.args());
    args.extend(w.set.quorum(price_digest(ORACLE_COV, a.kas, a.daa, e.seq, a.rate), &Satz::first(w.set.t)));
    let run = |two: bool| {
        let mut outs = vec![w.reg_out(&w.reg_after_price(a.daa), 0), w.oracle_out(e, 1)];
        if two {
            outs.push(w.oracle_out(e, 1));
        }
        execute(
            vec![w.reg_in(&w.reg, "attestPrice", args.clone()), w.oracle_in(w.oracle, "update", vec![i(a.kas), i(a.daa), i(a.rate)])],
            outs,
            a.daa as u64,
            vec![],
        )
    };
    assert!(all_ok(&run(false)));
    let r = run(true);
    assert!(r[ORA].is_err(), "{r:?}");
}

// ------------------------------------------------------------------ read ----

fn read_tx(w: &World, out: OSt, value: i64, n_out: usize) -> Vec<Result<(), String>> {
    let mut outs = vec![];
    for _ in 0..n_out {
        outs.push(cov_out(&w.oracle_art(out), value, 0, ORACLE_COV));
    }
    execute(vec![w.oracle_in(w.oracle, "read", vec![])], outs, 0, vec![])
}

#[test]
fn lesen_laesst_alles_unveraendert() {
    let mut w = w();
    assert!(all_ok(&read_tx(&w, w.oracle, ORACLE_V, 1)));
    assert!(read_tx(&w, OSt { kas_usd: w.oracle.kas_usd * 2, ..w.oracle }, ORACLE_V, 1)[0].is_err());
    assert!(read_tx(&w, OSt { frozen: true, ..w.oracle }, ORACLE_V, 1)[0].is_err(), "lesen friert nicht ein");
    assert!(read_tx(&w, w.oracle, ORACLE_V - 1, 1)[0].is_err(), "KAS bleiben im Orakel");
    assert!(read_tx(&w, w.oracle, ORACLE_V, 2)[0].is_err(), "genau eine Fortsetzung");
    w.oracle.frozen = true;
    assert!(all_ok(&read_tx(&w, w.oracle, ORACLE_V, 1)), "eingefrorenes Orakel lässt sich lesen");
    assert!(read_tx(&w, OSt { frozen: false, ..w.oracle }, ORACLE_V, 1)[0].is_err(), "lesen taut nicht auf");
}

// ---------------------------------------------------------------- freeze ----

fn freeze_tx(w: &World, out: OSt, lock: i64) -> Vec<Result<(), String>> {
    execute(vec![w.oracle_in(w.oracle, "freeze", vec![])], vec![cov_out(&w.oracle_art(out), ORACLE_V, 0, ORACLE_COV)], lock as u64, vec![])
}

#[test]
fn einfrieren_erst_nach_der_frist_und_durch_jeden() {
    let w = w();
    let due = w.oracle.daa + w.ocfg.freeze_after;
    let frozen = OSt { frozen: true, ..w.oracle };
    assert!(all_ok(&freeze_tx(&w, frozen, due)), "ab der Frist darf jeder einfrieren");
    assert!(freeze_tx(&w, frozen, due - 1)[0].is_err(), "eine DAA zu früh");
    // Ausgang muss genau eingefroren und sonst unverändert sein
    assert!(freeze_tx(&w, w.oracle, due)[0].is_err(), "nicht eingefroren");
    assert!(freeze_tx(&w, OSt { kas_usd: 1_000, ..frozen }, due)[0].is_err(), "Preis verändert");
    assert!(freeze_tx(&w, OSt { daa: due, ..frozen }, due)[0].is_err(), "DAA verändert");
}

#[test]
fn einfrieren_und_wieder_auftauen() {
    // Lebenszyklus: Preis veraltet → eingefroren → nächstes Update taut auf
    // und trägt den Zins für die Pause nach
    let mut w = w();
    let due = w.oracle.daa + w.ocfg.freeze_after;
    assert!(all_ok(&freeze_tx(&w, OSt { frozen: true, ..w.oracle }, due)));
    w.oracle.frozen = true;
    let later = due + 10 * DAY;
    let a = Attest::new(w.oracle.kas_usd, later, w.oracle.rate);
    let e = a.expected(&w);
    assert!(!e.frozen);
    assert_eq!(e.index, next_index(w.oracle, later).unwrap(), "Zins über die ganze Pause");
    assert!(all_ok(&a.run(&w)));
}

#[test]
fn groesse_und_zustand() {
    let w = w();
    let art = w.oracle_art(w.oracle);
    let (pre, suf, _) = tpl(&art);
    let code = v4common::common::bytecode(&art);
    println!(
        "Orakel v4: Redeem-Skript {} B, Zustand {} B (Präfix {} B, Suffix {} B)",
        code.len(),
        code.len() - pre.len() - suf.len(),
        pre.len(),
        suf.len()
    );
    assert!(code.len() < 852, "kleiner als das v3-Orakel (852 B)");
}
