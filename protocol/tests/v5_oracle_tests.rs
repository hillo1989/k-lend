//! Orakel v5 (contracts/price_oracle_v5.sil): Sprunggrenze, Referenzpreis,
//! Einfrieren; zusammen mit dem Register v5 in der Skript-Engine.

mod v5common;

use v5common::*;

#[test]
fn sprunggrenze_genau_an_der_grenze() {
    let w = World::standard();
    let p = w.oracle.kas_usd; // 4 000 000, j = 25 %
    let up = p * 12_500 / 10_000;
    let down = p * 10_000 / 12_500;
    for (kas, ok) in [(up, true), (up + 1, false), (down, true), (down - 1, false)] {
        let a = Attest::new(kas, w.oracle.daa + 600, w.oracle.rate);
        let r = a.run(&w);
        assert_eq!(all_ok(&r), ok, "{kas}: {r:?}");
        if !ok {
            assert!(r[1].is_err(), "das Orakel selbst lehnt ab: {r:?}");
        }
    }
}

#[test]
fn v4_sprung_entspricht_jump_10000() {
    let mut w = World::standard();
    w.ocfg.jump_bps = 10_000;
    let otpl = tpl(&w.oracle_art(w.oracle));
    w.reg.oracle_tpl = otpl.2;
    let p = w.oracle.kas_usd;
    assert!(all_ok(&Attest::new(p * 2, w.oracle.daa + 600, w.oracle.rate).run(&w)));
    assert!(!all_ok(&Attest::new(p * 2 + 1, w.oracle.daa + 600, w.oracle.rate).run(&w)));
    assert!(all_ok(&Attest::new(p / 2, w.oracle.daa + 600, w.oracle.rate).run(&w)));
    assert!(!all_ok(&Attest::new(p / 2 - 1, w.oracle.daa + 600, w.oracle.rate).run(&w)));
}

#[test]
fn referenzpreis_folgt_erst_nach_ref_after_updates() {
    // Angreifer senkt den Preis Update für Update um 25 %. Der Referenzpreis
    // bleibt ehrlich, bis ein gefälschter Kandidat ref_after Updates alt ist.
    let mut w = World::standard();
    let honest = w.oracle.kas_usd;
    let k = w.ocfg.ref_after;
    // ungünstigster Fall: der erste gefälschte Preis wird sofort Kandidat
    w.oracle.cand_seq = w.oracle.seq + 1 - k;
    let mut refs = vec![];
    for n in 1..=(2 * k) {
        let kas = (w.oracle.kas_usd * 10_000 + 12_499) / 12_500;
        Attest::new(kas, w.oracle.daa + 600, w.oracle.rate).apply(&mut w);
        refs.push((n, w.oracle.kas_usd, w.oracle.ref_usd));
    }
    for &(n, cur, r) in &refs {
        if n <= k {
            assert_eq!(r, honest, "Update {n}: Referenz noch ehrlich");
        } else {
            assert!(r < honest, "Update {n}: Referenz gefälscht");
        }
        assert!(cur < honest);
    }
    // erste gefälschte Referenz = Preis des ersten gefälschten Updates (÷1,25)
    assert_eq!(refs[k as usize].2, (honest * 10_000 + 12_499) / 12_500);
}

#[test]
fn referenzfelder_rechnet_das_orakel_selbst() {
    // Register übernimmt ref/cand aus den Argumenten; das Orakel prüft sie
    let w = World::standard();
    let a = Attest::next(&w);
    let e = a.expected(&w);
    for bad in [OSt { ref_usd: e.ref_usd + 1, ..e }, OSt { cand_usd: e.cand_usd + 1, ..e }, OSt { cand_seq: e.cand_seq + 1, ..e }] {
        let mut x = a.clone();
        x.oracle_out = Some(bad);
        let r = x.run(&w);
        assert!(r[0].is_ok() && r[1].is_err(), "{r:?}");
    }
    // Rotation fällig: Kandidat wird Referenz, neuer Preis Kandidat
    let mut w2 = w.clone();
    w2.oracle.cand_usd = 5_000_000;
    w2.oracle.cand_seq = w2.oracle.seq + 1 - w2.ocfg.ref_after;
    let a2 = Attest::new(4_100_000, w2.oracle.daa + 600, w2.oracle.rate);
    let e2 = a2.expected(&w2);
    assert_eq!((e2.ref_usd, e2.cand_usd, e2.cand_seq), (5_000_000, 4_100_000, w2.oracle.seq + 1));
    assert!(all_ok(&a2.run(&w2)));
    let mut keep = a2.clone();
    keep.oracle_out = Some(OSt { ref_usd: w2.oracle.ref_usd, cand_usd: w2.oracle.cand_usd, cand_seq: w2.oracle.cand_seq, ..e2 });
    assert!(keep.run(&w2)[1].is_err(), "Rotation darf nicht ausbleiben");
}

#[test]
fn update_ohne_register_scheitert() {
    let w = World::standard();
    let mut a = Attest::next(&w);
    let e = a.expected(&w);
    a.oracle_out = Some(e);
    let r = execute(vec![w.oracle_in(w.oracle, "update", vec![i(a.kas), i(a.daa), i(a.rate)])], vec![w.oracle_out(e, 0)], a.daa as u64, vec![]);
    assert!(r[0].is_err());
    a.with_oracle = true;
}

#[test]
fn read_und_freeze_lassen_die_referenz_stehen() {
    let mut w = World::standard();
    w.oracle.ref_usd = 3_900_000;
    w.oracle.cand_usd = 4_100_000;
    let r = execute(vec![w.oracle_in(w.oracle, "read", vec![])], vec![w.oracle_out(w.oracle, 0)], 0, vec![]);
    assert!(all_ok(&r));
    let fr = OSt { frozen: true, ..w.oracle };
    let lock = (w.oracle.daa + w.ocfg.freeze_after) as u64;
    assert!(all_ok(&execute(vec![w.oracle_in(w.oracle, "freeze", vec![])], vec![w.oracle_out(fr, 0)], lock, vec![])));
    assert!(!all_ok(&execute(vec![w.oracle_in(w.oracle, "freeze", vec![])], vec![w.oracle_out(fr, 0)], lock - 1, vec![])));
    let moved = OSt { ref_usd: 1, ..fr };
    assert!(!all_ok(&execute(vec![w.oracle_in(w.oracle, "freeze", vec![])], vec![w.oracle_out(moved, 0)], lock, vec![])));
    // Update taut auf
    let mut wf = w.clone();
    wf.oracle = fr;
    let a = Attest::new(fr.kas_usd, fr.daa + w.ocfg.freeze_after + 600, fr.rate);
    assert!(!a.expected(&wf).frozen);
    assert!(all_ok(&a.run(&wf)));
}
