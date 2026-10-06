//! Tests für contracts/signer_register_v4.sil (Unterzeichner-Register) gegen
//! die echte Skript-Engine, zusammen mit dem Orakel v4. Jede Regel hat eine
//! Gegenprobe (dieselbe Tx ehrlich geht durch); Angriffe aus
//! docs/v4-entwurf.md: Austausch-Eintrag + Orakel-Update in einer Tx,
//! Wartezeit umgehen, Ticket zerstören oder fälschen, Notfallweg zu früh.

mod v4common;

use kaspa_consensus_core::Hash;
use silverscript_abi::ArtifactValue;
use v4common::*;

// ------------------------------------------------------------ Bausteine ----

fn keys_arg(s: &Satz) -> ArtifactValue {
    ArtifactValue::Array(s.pubs().into_iter().map(ArtifactValue::Bytes).collect())
}

/// Ankündigung eines neuen Satzes; jedes Feld lässt sich verbiegen.
#[derive(Clone)]
struct Propose {
    new: Satz,
    new_fb: [u8; 32],
    kind: u8,
    to: [u8; 32],
    emerg: bool,
    /// signierender Satz (Standard: Hauptsatz bzw. Notfallsatz)
    auth: Option<Satz>,
    signers: Option<Vec<usize>>,
    payload: Option<Vec<u8>>,
    signed_nonce: Option<i64>,
    signed_ann: Option<Vec<u8>>,
    signed_tag: Option<u8>,
    /// als n übergeben (Standard: Anzahl der Schlüssel)
    new_n_arg: Option<i64>,
    main_out: Option<RSt>,
    ticket_out: Option<RSt>,
    main_value: Option<i64>,
    no_ticket: bool,
    extra_ticket: bool,
    lock: Option<i64>,
    with_oracle: bool,
}

impl Propose {
    fn new(new: Satz) -> Self {
        let fb = Satz::new(5, 3, 4).hash();
        Self {
            new,
            new_fb: fb,
            kind: 0,
            to: [0x99; 32],
            emerg: false,
            auth: None,
            signers: None,
            payload: None,
            signed_nonce: None,
            signed_ann: None,
            signed_tag: None,
            new_n_arg: None,
            main_out: None,
            ticket_out: None,
            main_value: None,
            no_ticket: false,
            extra_ticket: false,
            lock: None,
            with_oracle: false,
        }
    }
    fn emergency(new: Satz) -> Self {
        Self { emerg: true, ..Self::new(new) }
    }
    fn ann(&self) -> Vec<u8> {
        announcement(&self.new.hash(), &self.new_fb, self.kind, &self.to)
    }
    fn main(&self, w: &World) -> RSt {
        RSt { nonce: w.reg.nonce + 1, emerg: self.emerg, ..w.reg.clone() }
    }
    fn ticket(&self, w: &World) -> RSt {
        RSt {
            ticket: true,
            set: self.new.hash(),
            fb: self.new_fb,
            pay_kind: self.kind,
            pay_to: self.to,
            nonce: w.reg.nonce + 1,
            emerg: self.emerg,
            ..w.reg.clone()
        }
    }
    fn auth_satz(&self, w: &World) -> Satz {
        self.auth.clone().unwrap_or_else(|| if self.emerg { w.fb.clone().expect("Notfallsatz") } else { w.set.clone() })
    }
    fn run(&self, w: &World) -> Vec<Result<(), String>> {
        let auth = self.auth_satz(w);
        let tag = self.signed_tag.unwrap_or(if self.emerg { TAG_EMERG } else { TAG_ROTATE });
        let digest =
            rot_digest(REG_COV, tag, self.signed_nonce.unwrap_or(w.reg.nonce + 1), &self.signed_ann.clone().unwrap_or_else(|| self.ann()));
        let who = self.signers.clone().unwrap_or_else(|| Satz::first(auth.t_rot));
        let mut args = vec![
            i(self.new_n_arg.unwrap_or(self.new.n())),
            i(self.new.t),
            i(self.new.t_rot),
            keys_arg(&self.new),
            b(&self.new_fb),
            ArtifactValue::Byte(self.kind),
            b(&self.to),
            ArtifactValue::Bool(self.emerg),
        ];
        args.extend(auth.args());
        args.extend(auth.quorum(digest, &who));
        let mut inputs = vec![w.reg_in(&w.reg, "propose", args)];
        let main = self.main_out.clone().unwrap_or_else(|| self.main(w));
        let mut outputs = vec![cov_out(&w.reg_art(&main), self.main_value.unwrap_or(REG_V), 0, REG_COV)];
        if !self.no_ticket {
            outputs.push(w.reg_out(&self.ticket_out.clone().unwrap_or_else(|| self.ticket(w)), 0));
        }
        if self.extra_ticket {
            outputs.push(w.reg_out(&self.ticket(w), 0));
        }
        if self.with_oracle {
            // Umgehungsversuch: Orakel-Update ohne Preis-Signaturen in derselben Tx
            let next = oracle_next(w.oracle, w.oracle.kas_usd * 2, w.oracle.daa + 600, w.oracle.rate);
            inputs.push(w.oracle_in(w.oracle, "update", vec![i(next.kas_usd), i(next.daa), i(next.rate)]));
            outputs.push(w.oracle_out(next, 1));
        }
        let lock = self.lock.unwrap_or(if self.emerg { w.reg.last_daa + w.rcfg.emerg_after } else { w.oracle.daa + 600 });
        execute(inputs, outputs, lock as u64, self.payload.clone().unwrap_or_else(|| self.ann()))
    }
}

/// Aktivierung: Haupt-UTXO (activate) + Ticket (settle) mit Alter `age`
fn activate_tx(w: &World, ticket: &RSt, age: i64, main_out: Option<RSt>) -> Vec<Result<(), String>> {
    let out = main_out.unwrap_or_else(|| activated(w, ticket));
    let mut tin = w.reg_in(ticket, "settle", vec![i(0)]);
    tin.sequence = age as u64;
    execute(vec![w.reg_in(&w.reg, "activate", vec![i(1)]), tin], vec![w.reg_out(&out, 0)], 0, vec![])
}

fn activated(w: &World, t: &RSt) -> RSt {
    RSt { set: t.set, fb: t.fb, pay_kind: t.pay_kind, pay_to: t.pay_to, nonce: w.reg.nonce + 1, emerg: false, ..w.reg.clone() }
}

/// Welt nach einer erfolgreichen Ankündigung: (Welt, Ticket)
fn proposed(w: &World, p: &Propose) -> (World, RSt) {
    let r = p.run(w);
    assert!(all_ok(&r), "Ankündigung: {r:?}");
    let mut w2 = w.clone();
    w2.reg = p.main(w);
    (w2, p.ticket(w))
}

fn cancel_tx(w: &World, satz: &Satz, who: &[usize], out: Option<RSt>, with_oracle: bool) -> Vec<Result<(), String>> {
    let mut args = satz.args();
    args.extend(satz.quorum(cancel_digest(REG_COV, w.reg.nonce + 1), who));
    let mut inputs = vec![w.reg_in(&w.reg, "cancel", args)];
    let mut outputs = vec![w.reg_out(&out.unwrap_or(RSt { nonce: w.reg.nonce + 1, emerg: false, ..w.reg.clone() }), 0)];
    if with_oracle {
        let next = oracle_next(w.oracle, w.oracle.kas_usd * 2, w.oracle.daa + 600, w.oracle.rate);
        inputs.push(w.oracle_in(w.oracle, "update", vec![i(next.kas_usd), i(next.daa), i(next.rate)]));
        outputs.push(w.oracle_out(next, 1));
    }
    execute(inputs, outputs, (w.oracle.daa + 600) as u64, vec![])
}

fn witness_tx(w: &World, out: RSt, value: i64) -> Vec<Result<(), String>> {
    execute(vec![w.reg_in(&w.reg, "witness", vec![])], vec![cov_out(&w.reg_art(&out), value, 0, REG_COV)], 0, vec![])
}

// ============================================================ attestPrice ----

#[test]
fn preis_mit_quorum_des_hauptsatzes() {
    let w = World::standard();
    let r = Attest::next(&w).run(&w);
    assert!(all_ok(&r), "{r:?}");
    // jede Auswahl von t Schlüsseln, aufsteigend
    let mut a = Attest::next(&w);
    a.signers = Some(vec![2, 4, 5, 6]);
    assert!(all_ok(&a.run(&w)));
}

#[test]
fn preis_mit_zu_wenigen_doppelten_oder_falsch_sortierten_signaturen_scheitert() {
    let w = World::standard();
    for who in [vec![0, 1, 2], vec![0, 0, 1, 2], vec![0, 2, 1, 3], vec![3, 2, 1, 0], vec![0, 1, 2, 7]] {
        let mut a = Attest::next(&w);
        a.signers = Some(who.clone());
        let r = a.run(&w);
        assert!(r[0].is_err() && r[1].is_ok(), "{who:?}: {r:?}");
    }
    // mehr Signaturen als t: Anzahl muss genau t sein
    let mut a = Attest::next(&w);
    a.signers = Some(vec![0, 1, 2, 3, 4]);
    assert!(a.run(&w)[0].is_err());
}

#[test]
fn preis_mit_fremdem_satz_oder_falschen_schluesseln_scheitert() {
    let w = World::standard();
    // fremder Satz signiert und gibt seine eigenen Schlüssel an: Hash passt nicht
    let mut a = Attest::next(&w);
    a.satz = Some(Satz::new(7, 4, 4));
    assert!(a.run(&w)[0].is_err());
    // fremde Signaturen, aber Schlüssel des echten Satzes: Signaturen falsch
    let mut b2 = Attest::next(&w);
    b2.satz = Some(Satz::new(7, 4, 4));
    b2.args_satz = Some(w.set.clone());
    assert!(b2.run(&w)[0].is_err());
    // echter Satz mit anderer Schwelle angegeben (t = 1): Hash passt nicht
    let mut weak = w.set.clone();
    weak.t = 1;
    let mut c = Attest::next(&w);
    c.satz = Some(weak);
    assert!(c.run(&w)[0].is_err());
    // Notfallsatz darf keine Preise setzen
    let mut d = Attest::next(&w);
    d.satz = w.fb.clone();
    assert!(d.run(&w)[0].is_err());
}

#[test]
fn signatur_bindet_preis_seq_und_orakel() {
    let w = World::standard();
    let fs: [fn(&mut Attest); 3] = [
        |a: &mut Attest| a.signed_kas = Some(a.kas + 1),
        |a: &mut Attest| a.signed_seq = Some(9),
        |a: &mut Attest| a.signed_cov = Some(Hash::from_bytes([0x33; 32])),
    ];
    for f in fs {
        let mut a = Attest::next(&w);
        f(&mut a);
        let r = a.run(&w);
        assert!(r[0].is_err(), "{r:?}");
    }
}

#[test]
fn orakel_ausgang_muss_dem_signierten_preis_entsprechen() {
    // Umgehung: Quorum signiert Preis P, das Orakel-Update setzt aber P'
    // (gültig im Orakelrahmen). Das Register verlangt den signierten Ausgang.
    let w = World::standard();
    let mut a = Attest::next(&w);
    let other = oracle_next(w.oracle, a.kas * 2, a.daa, a.rate);
    a.oracle_args = Some((other.kas_usd, other.daa, other.rate));
    a.oracle_out = Some(other);
    let r = a.run(&w);
    assert!(r[0].is_err() && r[1].is_ok(), "nur das Register hält das auf: {r:?}");
}

#[test]
fn register_bleibt_bei_preis_unveraendert_bis_auf_last_daa() {
    let w = World::standard();
    let a = Attest::next(&w);
    let good = w.reg_after_price(a.daa);
    for bad in [
        RSt { last_daa: w.reg.last_daa, ..good.clone() },
        RSt { set: [1; 32], ..good.clone() },
        RSt { nonce: good.nonce + 1, ..good.clone() },
        RSt { pay_to: [1; 32], ..good.clone() },
        RSt { ticket: true, ..good.clone() },
    ] {
        let mut x = a.clone();
        x.reg_out = Some(bad);
        assert!(x.run(&w)[0].is_err());
    }
}

#[test]
fn preis_vor_init_oder_vom_ticket_scheitert() {
    let mut w = World::standard();
    w.reg.init = false;
    let mut a = Attest::next(&w);
    a.reg_out = Some(RSt { last_daa: a.daa, ..w.reg.clone() });
    assert!(a.run(&w)[0].is_err(), "vor init");
    let mut w = World::standard();
    w.reg.ticket = true;
    let mut a = Attest::next(&w);
    a.reg_out = Some(RSt { last_daa: a.daa, ..w.reg.clone() });
    assert!(a.run(&w)[0].is_err(), "Ticket ist kein Register");
}

#[test]
fn register_kas_bleiben_im_register() {
    let w = World::standard();
    let a = Attest::next(&w);
    let e = a.expected(&w);
    let mut args = vec![i(a.kas), i(a.daa), i(e.seq), i(a.rate), i(e.index), i(e.last_rate_daa)];
    args.extend(w.set.args());
    args.extend(w.set.quorum(price_digest(ORACLE_COV, a.kas, a.daa, e.seq, a.rate), &Satz::first(4)));
    let run = |v: i64| {
        execute(
            vec![w.reg_in(&w.reg, "attestPrice", args.clone()), w.oracle_in(w.oracle, "update", vec![i(a.kas), i(a.daa), i(a.rate)])],
            vec![cov_out(&w.reg_art(&w.reg_after_price(a.daa)), v, 0, REG_COV), w.oracle_out(e, 1)],
            a.daa as u64,
            vec![],
        )
    };
    assert!(all_ok(&run(REG_V)));
    assert!(run(REG_V - 1)[0].is_err());
}

#[test]
fn preis_ohne_orakel_scheitert() {
    let w = World::standard();
    let mut a = Attest::next(&w);
    a.with_oracle = false;
    assert!(a.run(&w)[0].is_err());
}

#[test]
fn preis_mit_orakel_read_statt_update_scheitert() {
    // Register verlangt seq + 1 im Orakel-Ausgang, read lässt den Zustand stehen
    let w = World::standard();
    let mut a = Attest::next(&w);
    a.oracle_entry = "read";
    a.oracle_out = Some(w.oracle);
    a.arg_seq = Some(w.oracle.seq);
    let r = a.run(&w);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn notfall_ankuendigung_verfaellt_durch_preis_update() {
    let mut w = World::standard();
    w.reg.emerg = true;
    w.reg.nonce = 5;
    let a = Attest::next(&w);
    let good = w.reg_after_price(a.daa);
    assert_eq!((good.nonce, good.emerg), (6, false));
    assert!(all_ok(&a.run(&w)));
    let mut keep = a.clone();
    keep.reg_out = Some(RSt { nonce: 5, emerg: false, last_daa: a.daa, ..w.reg.clone() });
    assert!(keep.run(&w)[0].is_err(), "nonce muss steigen");
    // reguläre Ankündigung bleibt bestehen (nonce unverändert)
    w.reg.emerg = false;
    let mut bump = Attest::next(&w);
    bump.reg_out = Some(RSt { nonce: 6, last_daa: bump.daa, ..w.reg.clone() });
    assert!(bump.run(&w)[0].is_err());
    assert!(all_ok(&Attest::next(&w).run(&w)));
}

// ================================================================== init ----

fn genesis(w: &World) -> RSt {
    RSt { init: false, oracle_cov: [0; 32], oracle_tpl: vec![0; 32], pre: 0, suf: 0, ..w.reg.clone() }
}

fn init_tx(w: &World, g0: &RSt, satz: &Satz, out: RSt, signer: Option<&secp256k1::Keypair>, with_oracle: bool) -> Vec<Result<(), String>> {
    let mut args = satz.args();
    args.extend([b(&w.reg.oracle_cov), b(&w.reg.oracle_tpl), i(w.reg.pre), i(w.reg.suf)]);
    let mut inp = w.reg_in(g0, "init", args);
    let k = signer.copied().unwrap_or_else(random_keypair);
    if let Call::Entry { sig_at, .. } = &mut inp.call {
        *sig_at = Some((8, k));
    }
    let mut inputs = vec![inp];
    let mut outputs = vec![w.reg_out(&out, 0)];
    if with_oracle {
        let next = oracle_next(w.oracle, w.oracle.kas_usd * 2, w.oracle.daa + 600, w.oracle.rate);
        inputs.push(w.oracle_in(w.oracle, "update", vec![i(next.kas_usd), i(next.daa), i(next.rate)]));
        outputs.push(w.oracle_out(next, 1));
    }
    execute(inputs, outputs, (w.oracle.daa + 600) as u64, vec![])
}

#[test]
fn init_einmalig_vom_deployer() {
    let w = World::standard();
    let g = genesis(&w);
    let ok = init_tx(&w, &g, &w.set, w.reg.clone(), Some(&w.deployer), false);
    assert!(all_ok(&ok), "{ok:?}");
    assert!(init_tx(&w, &g, &w.set, w.reg.clone(), None, false)[0].is_err(), "fremde Signatur");
    assert!(init_tx(&w, &w.reg.clone(), &w.set, w.reg.clone(), Some(&w.deployer), false)[0].is_err(), "zweites init");
    let gt = RSt { ticket: true, ..g.clone() };
    assert!(init_tx(&w, &gt, &w.set, w.reg.clone(), Some(&w.deployer), false)[0].is_err(), "Ticket-Genesis");
    // Ausgang muss genau die Orakel-Daten tragen und initialisiert sein
    assert!(init_tx(&w, &g, &w.set, RSt { pre: w.reg.pre + 1, ..w.reg.clone() }, Some(&w.deployer), false)[0].is_err());
    assert!(init_tx(&w, &g, &w.set, RSt { init: false, ..w.reg.clone() }, Some(&w.deployer), false)[0].is_err());
    assert!(init_tx(&w, &g, &w.set, RSt { set: [3; 32], ..w.reg.clone() }, Some(&w.deployer), false)[0].is_err());
}

#[test]
fn init_prueft_den_startsatz() {
    let w = World::standard();
    // Schlüssel passen nicht zum Hash der Genesis
    let other = Satz::new(7, 4, 4);
    assert!(init_tx(&w, &genesis(&w), &other, w.reg.clone(), Some(&w.deployer), false)[0].is_err());
    // Startsatz außerhalb der Grenzen (n < 5 bei RCFG) – Hash passt, Grenzen nicht
    let mut small = World::new(Satz::new(4, 3, 3), None, RCFG);
    small.deployer = w.deployer;
    let ok = World::new(Satz::new(5, 3, 3), None, RCFG);
    let mut ok2 = ok.clone();
    ok2.deployer = w.deployer;
    assert!(all_ok(&init_tx(&ok2, &genesis(&ok2), &ok2.set, ok2.reg.clone(), Some(&w.deployer), false)));
    assert!(init_tx(&small, &genesis(&small), &small.set, small.reg.clone(), Some(&w.deployer), false)[0].is_err());
}

#[test]
fn init_mit_orakel_update_scheitert() {
    let w = World::standard();
    let r = init_tx(&w, &genesis(&w), &w.set, w.reg.clone(), Some(&w.deployer), true);
    assert!(r[0].is_err(), "{r:?}");
}

// =============================================================== propose ----

#[test]
fn ankuendigung_legt_ticket_an() {
    let w = World::standard();
    let p = Propose::new(Satz::new(7, 4, 4));
    let r = p.run(&w);
    assert!(all_ok(&r), "{r:?}");
}

#[test]
fn ankuendigung_braucht_austausch_schwelle() {
    // tRot > t: 5 Schlüssel, Preis 3, Austausch 4
    let w = World::new(Satz::new(5, 3, 4), Some(Satz::new(5, 3, 4)), RCFG);
    let mut p = Propose::new(Satz::new(7, 4, 4));
    p.signers = Some(Satz::first(3)); // t = 3 reicht nicht, tRot = 4
    assert!(p.run(&w)[0].is_err());
    p.signers = Some(vec![0, 2, 3, 4]);
    assert!(all_ok(&p.run(&w)));
    p.signers = Some(vec![0, 3, 2, 4]);
    assert!(p.run(&w)[0].is_err(), "absteigend");
    p.signers = Some(vec![0, 2, 3, 5]);
    assert!(p.run(&w)[0].is_err(), "Index ≥ n");
    let mut q = Propose::new(Satz::new(7, 4, 4));
    q.auth = Some(Satz::new(5, 3, 4));
    assert!(q.run(&w)[0].is_err(), "fremder Satz");
}

#[test]
fn ankuendigung_prueft_die_grenzen_des_neuen_satzes() {
    let w = World::standard();
    // (n, t, tRot): gültig an der Grenze / eins daneben
    let cases = [
        ((5, 3, 3), true),
        ((4, 3, 3), false), // n < minSigners
        ((7, 4, 4), true),
        ((8, 5, 5), false), // n > 7 (und Schwelle > 4)
        ((7, 4, 5), false), // tRot > 4 (Mempool-Standardregel)
        ((5, 3, 4), true),
        ((5, 3, 5), false), // tRot > 4
        ((6, 3, 3), false), // 2t ≤ n
        ((7, 4, 3), false), // tRot < t
        ((7, 4, 8), false), // tRot > n
    ];
    for ((n, t, tr), ok) in cases {
        let r = Propose::new(Satz::new(n, t, tr)).run(&w);
        assert_eq!(all_ok(&r), ok, "({n},{t},{tr}): {r:?}");
        if !ok {
            assert!(r[0].is_err());
        }
    }
    // t < minThreshold bei sonst gültigem Satz: minThreshold 4 einstellen
    let mut w4 = World::standard();
    w4.rcfg.min_t = 4;
    let w4 = World { reg: w4.reg.clone(), ..w4 };
    assert!(all_ok(&Propose::new(Satz::new(5, 4, 4)).run(&w4)));
    assert!(Propose::new(Satz::new(5, 3, 4)).run(&w4)[0].is_err(), "t < minThreshold");
    // angegebenes n passt nicht zur Zahl der Schlüssel
    let mut p = Propose::new(Satz::new(7, 4, 4));
    p.new_n_arg = Some(6);
    assert!(p.run(&w)[0].is_err());
}

#[test]
fn ankuendigung_muss_im_payload_stehen() {
    let w = World::standard();
    let mut p = Propose::new(Satz::new(7, 4, 4));
    p.payload = Some(vec![]);
    assert!(p.run(&w)[0].is_err(), "ohne Payload");
    let mut wrong = p.ann();
    wrong[70] ^= 1;
    p.payload = Some(wrong);
    assert!(p.run(&w)[0].is_err(), "anderer Payload");
    let mut longer = p.ann();
    longer.push(0);
    p.payload = Some(longer);
    assert!(p.run(&w)[0].is_err(), "Payload mit Anhang");
}

#[test]
fn signatur_bindet_ankuendigung_nonce_und_art() {
    let w = World::standard();
    let p = Propose::new(Satz::new(7, 4, 4));
    let mut a = p.clone();
    a.signed_nonce = Some(w.reg.nonce);
    assert!(a.run(&w)[0].is_err(), "alte nonce (Wiederholung)");
    let mut b2 = p.clone();
    let mut ann = p.ann();
    ann[64] = 1; // anderes Zinsziel unterschrieben
    b2.signed_ann = Some(ann);
    assert!(b2.run(&w)[0].is_err());
    let mut c = p.clone();
    c.signed_tag = Some(TAG_EMERG);
    assert!(c.run(&w)[0].is_err(), "Notfall-Signatur gilt nicht regulär");
}

#[test]
fn ankuendigung_ausgaenge_genau_haupt_und_ticket() {
    let w = World::standard();
    let p = Propose::new(Satz::new(7, 4, 4));
    let mut a = p.clone();
    a.no_ticket = true;
    assert!(a.run(&w)[0].is_err(), "ohne Ticket");
    let mut b2 = p.clone();
    b2.extra_ticket = true;
    assert!(b2.run(&w)[0].is_err(), "zwei Tickets");
    let mut c = p.clone();
    c.main_out = Some(RSt { nonce: w.reg.nonce, ..p.main(&w) });
    assert!(c.run(&w)[0].is_err(), "nonce nicht erhöht");
    let mut d = p.clone();
    d.main_out = Some(RSt { set: p.new.hash(), ..p.main(&w) });
    assert!(d.run(&w)[0].is_err(), "Satz sofort übernommen");
    let mut e = p.clone();
    e.ticket_out = Some(RSt { emerg: true, ..p.ticket(&w) });
    assert!(e.run(&w)[0].is_err(), "Ticket mit kürzerer Notfall-Frist");
    let mut f = p.clone();
    f.ticket_out = Some(RSt { ticket: false, ..p.ticket(&w) });
    assert!(f.run(&w)[0].is_err(), "zweite Haupt-UTXO");
    let mut g = p.clone();
    g.main_value = Some(REG_V - 1);
    assert!(g.run(&w)[0].is_err(), "KAS abgezogen");
    let mut h = p.clone();
    h.main_out = Some(RSt { emerg: true, ..p.main(&w) });
    assert!(h.run(&w)[0].is_err(), "Haupt-UTXO als Notfall markiert");
}

// ========================================================= Notfallweg ----

#[test]
fn notfall_erst_nach_langer_stille() {
    let w = World::standard();
    let p = Propose::emergency(Satz::new(7, 4, 4));
    assert!(all_ok(&p.run(&w)), "nach emergAfter");
    let mut early = p.clone();
    early.lock = Some(w.reg.last_daa + w.rcfg.emerg_after - 1);
    assert!(early.run(&w)[0].is_err(), "eine DAA zu früh");
}

#[test]
fn notfall_nur_durch_den_notfallsatz() {
    let w = World::standard();
    let mut p = Propose::emergency(Satz::new(7, 4, 4));
    p.auth = Some(w.set.clone());
    assert!(p.run(&w)[0].is_err(), "Hauptsatz ist nicht der Notfallsatz");
    let mut q = Propose::emergency(Satz::new(7, 4, 4));
    q.signers = Some(Satz::first(3)); // Notfallsatz 5/3/4: tRot = 4
    assert!(q.run(&w)[0].is_err());
    // ohne Notfallsatz (fbHash = 0) gibt es keinen Notfallweg
    let solo = World::new(Satz::new(5, 3, 4), None, RCFG);
    let mut r = Propose::emergency(Satz::new(7, 4, 4));
    r.auth = Some(Satz::new(5, 3, 4));
    assert!(r.run(&solo)[0].is_err());
}

#[test]
fn notfallsatz_wird_bei_benutzung_auf_grenzen_geprueft() {
    // Ein Notfallsatz steht nur als Hash im Zustand. Ein Satz mit tRot = 0
    // dürfte sonst ohne Signatur ankündigen.
    let bad_fb = Satz::new(5, 0, 0);
    let w = World::new(Satz::new(7, 4, 4), Some(bad_fb.clone()), RCFG);
    let mut p = Propose::emergency(Satz::new(7, 4, 4));
    p.signers = Some(vec![]);
    assert!(p.run(&w)[0].is_err(), "Notfallsatz 0-von-5 abgelehnt");
    // Gegenprobe: gültiger Notfallsatz, ehrliche Signaturen
    let good = World::new(Satz::new(7, 4, 4), Some(Satz::new(5, 3, 4)), RCFG);
    assert!(all_ok(&Propose::emergency(Satz::new(7, 4, 4)).run(&good)));
}

#[test]
fn notfall_ticket_wartet_emerg_delay_und_verfaellt_durch_update() {
    let mut cfg = RCFG;
    cfg.emerg_delay = 20 * DAY; // anders als rotDelay, damit die Frist unterscheidbar ist
    let w = World::new(Satz::new(7, 4, 4), Some(Satz::new(5, 3, 4)), cfg);
    let p = Propose::emergency(Satz::new(7, 4, 4));
    let (w2, t) = proposed(&w, &p);
    assert!(w2.reg.emerg);
    assert!(activate_tx(&w2, &t, cfg.rot_delay, None)[1].is_err(), "reguläre Frist reicht nicht");
    assert!(activate_tx(&w2, &t, cfg.emerg_delay - 1, None)[1].is_err());
    assert!(all_ok(&activate_tx(&w2, &t, cfg.emerg_delay, None)));
    // ein Preis-Update des alten Hauptsatzes lässt das Ticket verfallen
    let a = Attest::next(&w2);
    assert!(all_ok(&a.run(&w2)));
    let mut w3 = w2.clone();
    w3.reg = w2.reg_after_price(a.daa);
    let r = activate_tx(&w3, &t, cfg.emerg_delay, None);
    assert!(r[0].is_err(), "verfallenes Ticket: {r:?}");
}

// ============================================================= activate ----

#[test]
fn austausch_erst_nach_der_wartezeit() {
    let w = World::standard();
    let p = Propose::new(Satz::new(7, 4, 4));
    let (w2, t) = proposed(&w, &p);
    let early = activate_tx(&w2, &t, w.rcfg.rot_delay - 1, None);
    assert!(early[0].is_ok() && early[1].is_err(), "nur das Ticket prüft die Zeit: {early:?}");
    let ok = activate_tx(&w2, &t, w.rcfg.rot_delay, None);
    assert!(all_ok(&ok), "{ok:?}");
}

#[test]
fn nach_dem_austausch_gilt_nur_der_neue_satz() {
    let w = World::standard();
    let new = Satz::new(7, 4, 4);
    let p = Propose::new(new.clone());
    let (w2, t) = proposed(&w, &p);
    assert!(all_ok(&activate_tx(&w2, &t, w.rcfg.rot_delay, None)));
    let mut w3 = w2.clone();
    w3.reg = activated(&w2, &t);
    assert_eq!(w3.reg.set, new.hash());
    assert_eq!(w3.reg.pay_to, p.to);
    // alter Satz kann keine Preise mehr setzen, neuer schon
    assert!(Attest::next(&w3).run(&w3)[0].is_err());
    w3.set = new;
    assert!(all_ok(&Attest::next(&w3).run(&w3)));
}

#[test]
fn aktivierter_zustand_ist_festgelegt() {
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let good = activated(&w2, &t);
    for bad in [
        RSt { set: w.reg.set, ..good.clone() },
        RSt { fb: [0; 32], ..good.clone() },
        RSt { pay_kind: 1, ..good.clone() },
        RSt { nonce: good.nonce - 1, ..good.clone() },
        RSt { emerg: true, ..good.clone() },
        RSt { last_daa: good.last_daa + 1, ..good.clone() },
    ] {
        let r = activate_tx(&w2, &t, w.rcfg.rot_delay, Some(bad));
        assert!(r[0].is_err() && r[1].is_err(), "{r:?}");
    }
}

#[test]
fn abgelaufenes_ticket_aktiviert_nicht() {
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    // abgesagt
    let c = cancel_tx(&w2, &w2.set, &Satz::first(4), None, false);
    assert!(all_ok(&c), "{c:?}");
    let mut w3 = w2.clone();
    w3.reg.nonce += 1;
    let r = activate_tx(&w3, &t, w.rcfg.rot_delay, Some(RSt { nonce: w3.reg.nonce + 1, ..activated(&w3, &t) }));
    assert!(r[0].is_err() && r[1].is_ok(), "Ticket ist abgelaufen, die Haupt-UTXO lehnt ab: {r:?}");
}

#[test]
fn neue_ankuendigung_entwertet_die_alte() {
    let w = World::standard();
    let (w2, t1) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let (w3, t2) = proposed(&w2, &Propose::new(Satz::new(5, 3, 3)));
    assert!(activate_tx(&w3, &t1, w.rcfg.rot_delay, None)[0].is_err());
    assert!(all_ok(&activate_tx(&w3, &t2, w.rcfg.rot_delay, None)));
}

#[test]
fn ticket_muss_von_der_register_covenant_sein() {
    // gefälschtes Ticket: gleiches Skript, andere Covenant-ID (vom Angreifer
    // frei angelegt) – mit eigenem Satz und ohne Wartezeit
    let w = World::standard();
    let p = Propose::new(Satz::new(5, 3, 3));
    let mut w2 = w.clone();
    w2.reg = p.main(&w);
    let t = p.ticket(&w);
    let fake = entry_in(&w2.reg_art(&t), TICKET_V, OTHER_COV, "settle", vec![i(0)]);
    let r = execute(vec![w2.reg_in(&w2.reg, "activate", vec![i(1)]), fake], vec![w2.reg_out(&activated(&w2, &t), 0)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
    // Gegenprobe mit echtem Ticket
    assert!(all_ok(&activate_tx(&w2, &t, w.rcfg.rot_delay, None)));
}

#[test]
fn activate_zeigt_auf_die_haupt_utxo_selbst() {
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let mut tin = w2.reg_in(&t, "settle", vec![i(0)]);
    tin.sequence = w.rcfg.rot_delay as u64;
    let r = execute(vec![w2.reg_in(&w2.reg, "activate", vec![i(0)]), tin], vec![w2.reg_out(&activated(&w2, &t), 0)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
}

// ================================================================= settle ----

#[test]
fn lebendes_ticket_laesst_sich_nicht_zerstoeren() {
    // Angriff: das reife Ticket in einer witness-Tx verbrauchen, damit niemand
    // aktivieren kann
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let mut tin = w2.reg_in(&t, "settle", vec![i(0)]);
    tin.sequence = (w.rcfg.rot_delay * 2) as u64;
    let r = execute(vec![w2.reg_in(&w2.reg, "witness", vec![]), tin], vec![w2.reg_out(&w2.reg, 0)], 0, vec![]);
    assert!(r[0].is_ok() && r[1].is_err(), "{r:?}");
}

#[test]
fn abgelaufenes_ticket_darf_jeder_aufraeumen() {
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let mut w3 = w2.clone();
    w3.reg.nonce += 1; // z. B. nach cancel
    let r = execute(vec![w3.reg_in(&w3.reg, "witness", vec![]), w3.reg_in(&t, "settle", vec![i(0)])], vec![w3.reg_out(&w3.reg, 0)], 0, vec![]);
    assert!(all_ok(&r), "ohne Wartezeit: {r:?}");
}

#[test]
fn ticket_braucht_die_haupt_utxo() {
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    // Angriff: das lebende Ticket gegen ein ABGELAUFENES zweites Ticket statt
    // gegen die Haupt-UTXO prüfen lassen (dann zählt es als „abgelaufen")
    let stale = RSt { nonce: 0, ..t.clone() };
    let r = execute(
        vec![w2.reg_in(&t, "settle", vec![i(1)]), w2.reg_in(&stale, "settle", vec![i(0)])],
        vec![plain_out(1)],
        0,
        vec![],
    );
    assert!(r[0].is_err(), "{r:?}");
    // auf sich selbst zeigen
    let r = execute(vec![w2.reg_in(&t, "settle", vec![i(0)])], vec![plain_out(1)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
    // Haupt-UTXO einer fremden Covenant (Angreifer-Kopie mit anderer nonce)
    let fake_main = entry_in(&w2.reg_art(&RSt { nonce: 99, ..w2.reg.clone() }), REG_V, OTHER_COV, "witness", vec![]);
    let r = execute(vec![w2.reg_in(&t, "settle", vec![i(1)]), fake_main], vec![plain_out(1)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn ticket_erzeugt_keine_register_ausgaenge() {
    // Angriff: abgelaufenes Ticket legt beim Aufräumen eine zweite „Haupt-UTXO"
    // mit eigenem Satz an
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let mut w3 = w2.clone();
    w3.reg.nonce += 1;
    let evil = RSt { ticket: false, set: Satz::new(5, 3, 3).hash(), ..w3.reg.clone() };
    let r = execute(
        vec![w3.reg_in(&w3.reg, "witness", vec![]), w3.reg_in(&t, "settle", vec![i(0)])],
        vec![w3.reg_out(&w3.reg, 0), w3.reg_out(&evil, 1)],
        0,
        vec![],
    );
    assert!(r[1].is_err(), "{r:?}");
}

#[test]
fn ticket_kann_keine_hauptfunktionen() {
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let mut wt = w2.clone();
    wt.reg = t.clone();
    let r = witness_tx(&wt, t.clone(), TICKET_V);
    assert!(r[0].is_err(), "witness vom Ticket: {r:?}");
}

// ================================================================= cancel ----

#[test]
fn absage_mit_preis_schwelle() {
    let w = World::standard();
    let (w2, _) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    assert!(all_ok(&cancel_tx(&w2, &w2.set, &Satz::first(4), None, false)));
    assert!(cancel_tx(&w2, &w2.set, &Satz::first(3), None, false)[0].is_err(), "t − 1");
    assert!(cancel_tx(&w2, &w2.set, &[1, 0, 2, 3], None, false)[0].is_err(), "unsortiert");
    let fremd = Satz::new(7, 4, 4);
    assert!(cancel_tx(&w2, &fremd, &Satz::first(4), None, false)[0].is_err(), "fremder Satz");
    // Ausgang: nonce + 1, Notfall-Merker gelöscht, sonst unverändert
    assert!(cancel_tx(&w2, &w2.set, &Satz::first(4), Some(w2.reg.clone()), false)[0].is_err());
    assert!(cancel_tx(&w2, &w2.set, &Satz::first(4), Some(RSt { nonce: w2.reg.nonce + 1, set: [1; 32], ..w2.reg.clone() }), false)[0].is_err());
    let mut we = w2.clone();
    we.reg.emerg = true;
    assert!(all_ok(&cancel_tx(&we, &we.set, &Satz::first(4), None, false)), "Hauptsatz sagt Notfall ab");
    assert!(cancel_tx(&we, &we.set, &Satz::first(4), Some(RSt { nonce: we.reg.nonce + 1, ..we.reg.clone() }), false)[0].is_err());
}

#[test]
fn absage_signatur_gilt_nur_fuer_diese_nonce() {
    let w = World::standard();
    let mut args = w.set.args();
    args.extend(w.set.quorum(cancel_digest(REG_COV, w.reg.nonce), &Satz::first(4)));
    let r = execute(
        vec![w.reg_in(&w.reg, "cancel", args)],
        vec![w.reg_out(&RSt { nonce: w.reg.nonce + 1, ..w.reg.clone() }, 0)],
        0,
        vec![],
    );
    assert!(r[0].is_err());
}

// ================================================================ witness ----

#[test]
fn witness_laesst_alles_unveraendert() {
    let w = World::standard();
    assert!(all_ok(&witness_tx(&w, w.reg.clone(), REG_V)));
    assert!(witness_tx(&w, RSt { nonce: 1, ..w.reg.clone() }, REG_V)[0].is_err());
    assert!(witness_tx(&w, RSt { pay_kind: 1, ..w.reg.clone() }, REG_V)[0].is_err());
    assert!(witness_tx(&w, w.reg.clone(), REG_V - 1)[0].is_err());
    let mut g = w.clone();
    g.reg.init = false;
    assert!(witness_tx(&g, g.reg.clone(), REG_V)[0].is_err(), "vor init");
}

// ======================================== Umgehung: Eintrag + Orakel-Update ----

#[test]
fn kein_austausch_eintrag_neben_einem_orakel_update() {
    // Kernangriff aus dem Entwurf: Das Orakel verlangt nur einen Register-
    // Eingang. Jeder Register-Eintrag außer attestPrice muss deshalb selbst ein
    // Orakel in der Tx verbieten – sonst ginge ein Update ohne Preis-Signaturen.
    let w = World::standard();
    let next = oracle_next(w.oracle, w.oracle.kas_usd * 2, w.oracle.daa + 600, w.oracle.rate);
    let upd = |w: &World| w.oracle_in(w.oracle, "update", vec![i(next.kas_usd), i(next.daa), i(next.rate)]);
    let lock = (w.oracle.daa + 600) as u64;

    // witness
    let r = execute(vec![w.reg_in(&w.reg, "witness", vec![]), upd(&w)], vec![w.reg_out(&w.reg, 0), w.oracle_out(next, 1)], lock, vec![]);
    assert!(r[0].is_err() && r[1].is_ok(), "witness: das Orakel allein ließe es zu: {r:?}");
    // propose (regulär und Notfall)
    let mut p = Propose::new(Satz::new(7, 4, 4));
    p.with_oracle = true;
    let r = p.run(&w);
    assert!(r[0].is_err() && r[1].is_ok(), "propose: {r:?}");
    let mut pe = Propose::emergency(Satz::new(7, 4, 4));
    pe.with_oracle = true;
    let mut we = w.clone();
    we.oracle.daa = w.reg.last_daa; // Orakel-Update muss zur Locktime passen
    pe.lock = Some(w.reg.last_daa + w.rcfg.emerg_after);
    let r = pe.run(&we);
    assert!(r[0].is_err(), "Notfall: {r:?}");
    // cancel
    let r = cancel_tx(&w, &w.set, &Satz::first(4), None, true);
    assert!(r[0].is_err() && r[1].is_ok(), "cancel: {r:?}");
    // init
    let r = init_tx(&w, &genesis(&w), &w.set, w.reg.clone(), Some(&w.deployer), true);
    assert!(r[0].is_err() && r[1].is_ok(), "init: {r:?}");
    // activate: zwei Register-Eingänge, das Orakel lehnt schon selbst ab
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let mut tin = w2.reg_in(&t, "settle", vec![i(0)]);
    tin.sequence = w.rcfg.rot_delay as u64;
    let r = execute(
        vec![w2.reg_in(&w2.reg, "activate", vec![i(1)]), tin, upd(&w2)],
        vec![w2.reg_out(&activated(&w2, &t), 0), w2.oracle_out(next, 2)],
        lock,
        vec![],
    );
    assert!(r[0].is_err() && r[2].is_err(), "activate: {r:?}");
    // Gegenprobe: ehrliches Update mit attestPrice
    assert!(all_ok(&Attest::new(next.kas_usd, next.daa, next.rate).run(&w)));
}

// ========================================================= Alleinbetrieb ----

#[test]
fn alleinbetrieb_eins_von_eins_mit_kurzen_fristen() {
    // Mainnet-Probe: ein eigener Schlüssel, Wartezeit 1 h, später Austausch
    // gegen unabhängige Unterzeichner ohne neues Deployment
    let me = Satz::new(1, 1, 1);
    let w = World::new(me.clone(), Some(Satz::new(1, 1, 1)), RCFG_SOLO);
    assert!(all_ok(&Attest::next(&w).run(&w)), "Preis mit 1 Signatur");
    // mit den Vorschlagsgrenzen (n ≥ 5) wäre 1-von-1 als Startsatz unzulässig
    let mut strict = World::new(me.clone(), None, RCFG);
    strict.reg.init = false;
    let g0 = strict.reg.clone();
    strict.reg.init = true;
    assert!(init_tx(&strict, &g0, &me, strict.reg.clone(), Some(&strict.deployer), false)[0].is_err());
    // Austausch auf 7 unabhängige Schlüssel
    let indep = Satz::new(7, 4, 4);
    let p = Propose::new(indep.clone());
    let (w2, t) = proposed(&w, &p);
    assert!(activate_tx(&w2, &t, RCFG_SOLO.rot_delay - 1, None)[1].is_err());
    assert!(all_ok(&activate_tx(&w2, &t, RCFG_SOLO.rot_delay, None)));
    let mut w3 = w2.clone();
    w3.reg = activated(&w2, &t);
    w3.set = indep;
    assert!(all_ok(&Attest::next(&w3).run(&w3)));
}

#[test]
fn alleinbetrieb_drei_von_fuenf_eigene_schluessel() {
    let w = World::new(Satz::new(5, 3, 4), None, RCFG);
    assert!(all_ok(&Attest::next(&w).run(&w)));
    let mut a = Attest::next(&w);
    a.signers = Some(vec![1, 4]);
    assert!(a.run(&w)[0].is_err());
}

#[test]
fn sieben_schluessel_volle_schleife() {
    let w = World::new(Satz::new(7, 4, 4), None, RCFG);
    let mut a = Attest::next(&w);
    a.signers = Some(vec![0, 2, 4, 6]);
    assert!(all_ok(&a.run(&w)));
    let p = Propose::new(Satz::new(7, 4, 4));
    let r = p.run(&w);
    assert!(all_ok(&r), "{r:?}");
}

/// Ein Satz mit Schwelle 5 (aus einer alten Datei oder von Hand gehasht) kann
/// nie Preise setzen: checkQuorum verlangt need ≤ 4 ausdrücklich, statt sich
/// auf die Laufzeitprüfung der Schleife zu verlassen (TUTORIAL „For Loops“)
#[test]
fn schwelle_ueber_vier_wird_abgelehnt() {
    let mut w = World::new(Satz::new(7, 5, 5), None, RCFG);
    w.reg.set = w.set.hash();
    let a = Attest::next(&w);
    let r = a.run(&w);
    assert!(r[0].is_err(), "{r:?}");
}

// ============================================================== Messung ----

#[test]
fn groessen_und_massen() {
    use kaspa_consensus_core::config::params::MAINNET_PARAMS;
    use kaspa_consensus_core::tx::{TransactionId, TransactionOutpoint};
    use kaspa_lending_protocol::txb::{self, Draft, In as TIn, Unlock};
    let w = World::standard();
    let art = w.reg_art(&w.reg);
    let (pre, suf, _) = tpl(&art);
    let code = v4common::common::bytecode(&art);
    println!("Register v4: Redeem-Skript {} B, Zustand {} B", code.len(), code.len() - pre.len() - suf.len());
    let op = |k: u8| TransactionOutpoint { transaction_id: TransactionId::from_bytes([k; 32]), index: 0 };
    let fee_in = |k: u8| TIn { outpoint: op(k), entry: plain_in(10 * E8).utxo, unlock: Unlock::Raw(vec![]) };
    let to_tin = |k: u8, x: In| match x.call {
        Call::Entry { art, entry, args, sig_at } => TIn { outpoint: op(k), entry: x.utxo, unlock: Unlock::Entry { art, entry, args, sig_at: sig_at.map(|(p, k)| (p, k.into())) } },
        Call::Raw(s) => TIn { outpoint: op(k), entry: x.utxo, unlock: Unlock::Raw(s) },
    };
    // Update-Tx (Register + Orakel + Gebührenzahler) je Satzgröße
    for (n, t) in [(1usize, 1i64), (5, 3), (7, 4)] {
        let w = World::new(Satz::new(n, t, t), None, RCFG_SOLO);
        let a = Attest::next(&w);
        let e = a.expected(&w);
        let mut args = vec![i(a.kas), i(a.daa), i(e.seq), i(a.rate), i(e.index), i(e.last_rate_daa)];
        args.extend(w.set.args());
        args.extend(w.set.quorum(price_digest(ORACLE_COV, a.kas, a.daa, e.seq, a.rate), &Satz::first(t)));
        let d = Draft {
            inputs: vec![
                to_tin(1, w.reg_in(&w.reg, "attestPrice", args)),
                to_tin(2, w.oracle_in(w.oracle, "update", vec![i(a.kas), i(a.daa), i(a.rate)])),
                fee_in(3),
            ],
            outputs: vec![w.reg_out(&w.reg_after_price(a.daa), 0), w.oracle_out(e, 1)],
            change_spk: opt_true_spk(),
            lock_time: a.daa as u64,
        };
        let bt = txb::build(d, &MAINNET_PARAMS).expect("Update baut");
        println!(
            "Update {n} Schlüssel/{t} Sig.: Tx {} B, compute {} g, transient {} g, storage {} g, Gebühr {} sompi ({:.4} KAS), Einheiten {:?}",
            bt.tx.inputs.iter().map(|x| x.signature_script.len()).sum::<usize>(),
            bt.compute_mass,
            bt.transient_mass,
            bt.storage_mass,
            bt.fee,
            bt.fee as f64 / 1e8,
            bt.used_units
        );
    }
    // Ankündigung (7er-Satz, tRot 4) mit Payload
    let p = Propose::new(Satz::new(7, 4, 4));
    let auth = w.set.clone();
    let mut args = vec![i(7), i(4), i(4), keys_arg(&p.new), b(&p.new_fb), ArtifactValue::Byte(0), b(&p.to), ArtifactValue::Bool(false)];
    args.extend(auth.args());
    args.extend(auth.quorum(rot_digest(REG_COV, TAG_ROTATE, w.reg.nonce + 1, &p.ann()), &Satz::first(4)));
    let d = Draft {
        inputs: vec![to_tin(1, w.reg_in(&w.reg, "propose", args)), fee_in(3)],
        outputs: vec![w.reg_out(&p.main(&w), 0), w.reg_out(&p.ticket(&w), 0)],
        change_spk: opt_true_spk(),
        lock_time: 0,
    };
    let bt = txb::build_with_payload(d, &p.ann(), &MAINNET_PARAMS).expect("Ankündigung baut");
    println!("Ankündigung 7/5: compute {} g, transient {} g, storage {} g, Gebühr {:.4} KAS", bt.compute_mass, bt.transient_mass, bt.storage_mass, bt.fee as f64 / 1e8);
    // witness (Zinskasse lesen)
    let d = Draft {
        inputs: vec![to_tin(1, w.reg_in(&w.reg, "witness", vec![])), fee_in(3)],
        outputs: vec![w.reg_out(&w.reg, 0)],
        change_spk: opt_true_spk(),
        lock_time: 0,
    };
    let bt = txb::build(d, &MAINNET_PARAMS).expect("witness baut");
    println!("witness: compute {} g, transient {} g, Gebühr {:.4} KAS", bt.compute_mass, bt.transient_mass, bt.fee as f64 / 1e8);
}

#[test]
fn keine_zweite_haupt_utxo() {
    // Angriff: neben der Fortsetzung eine zweite Haupt-UTXO mit eigenem Satz
    // anlegen (witness ohne Signatur, attestPrice mit Preis-Quorum)
    let w = World::standard();
    let evil = RSt { set: Satz::new(5, 3, 3).hash(), ..w.reg.clone() };
    let r = execute(vec![w.reg_in(&w.reg, "witness", vec![])], vec![w.reg_out(&w.reg, 0), w.reg_out(&evil, 0)], 0, vec![]);
    assert!(r[0].is_err(), "witness: {r:?}");
    let mut a = Attest::next(&w);
    a.reg_out = Some(w.reg_after_price(a.daa));
    let base = a.run(&w);
    assert!(all_ok(&base), "Gegenprobe: {base:?}");
}

#[test]
fn ticket_wird_nicht_zur_haupt_utxo() {
    // Angriff: ein Ticket (mit angekündigtem Satz) führt witness aus und setzt
    // sich als Haupt-UTXO fort – das wäre eine zweite Haupt-UTXO mit dem neuen
    // Satz, ohne Wartezeit
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(5, 3, 3)));
    let as_main = RSt { ticket: false, ..t.clone() };
    let r = execute(vec![w2.reg_in(&t, "witness", vec![])], vec![cov_out(&w2.reg_art(&as_main), TICKET_V, 0, REG_COV)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
    // Gegenprobe: die echte Haupt-UTXO darf witness
    assert!(all_ok(&witness_tx(&w2, w2.reg.clone(), REG_V)));
}

#[test]
fn witness_initialisiert_nicht() {
    // Angriff: die Genesis ohne init (ohne Deployer-Signatur und Satzprüfung)
    // per witness als initialisiert fortsetzen
    let w = World::standard();
    let g = genesis(&w);
    let mut wg = w.clone();
    wg.reg = g.clone();
    let r = witness_tx(&wg, RSt { init: true, ..g.clone() }, REG_V);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn init_nicht_aus_einem_ticket() {
    let w = World::standard();
    let gt = RSt { ticket: true, ..genesis(&w) };
    let mut args = w.set.args();
    args.extend([b(&w.reg.oracle_cov), b(&w.reg.oracle_tpl), i(w.reg.pre), i(w.reg.suf)]);
    let mut inp = w.reg_in(&gt, "init", args);
    if let Call::Entry { sig_at, .. } = &mut inp.call {
        *sig_at = Some((8, w.deployer));
    }
    // gleicher Betrag wie das Ticket, damit nur isTicket zählt
    let r = execute(vec![inp], vec![cov_out(&w.reg_art(&w.reg), TICKET_V, 0, REG_COV)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
}

#[test]
fn activate_ohne_ticket_ist_keine_freie_absage() {
    // Angriff: activate zeigt auf die Haupt-UTXO selbst und erhöht so ohne
    // Signatur die nonce – jede Ankündigung ließe sich damit endlos entwerten
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(7, 4, 4)));
    let bumped = RSt { nonce: w2.reg.nonce + 1, emerg: false, ..w2.reg.clone() };
    let r = execute(vec![w2.reg_in(&w2.reg, "activate", vec![i(0)])], vec![w2.reg_out(&bumped, 0)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
    assert!(all_ok(&activate_tx(&w2, &t, w.rcfg.rot_delay, None)), "Gegenprobe mit Ticket");
}

#[test]
fn haupt_utxo_laesst_sich_nicht_per_settle_vernichten() {
    // Angriff: die Haupt-UTXO als „Ticket“ ausgeben (settle, keine Ausgänge)
    // – das Register wäre für immer weg
    let w = World::standard();
    let mut inp = w.reg_in(&w.reg, "settle", vec![i(0)]);
    inp.sequence = (w.rcfg.rot_delay * 10) as u64;
    let r = execute(vec![inp], vec![plain_out(1)], 0, vec![]);
    assert!(r[0].is_err(), "{r:?}");
}
