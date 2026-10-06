//! Register v5 (contracts/signer_register_v5.sil) mit Orakel v5 gegen die
//! echte Skript-Engine. Jede neue Regel aus docs/v5-entwurf.md mit Angriff und
//! ehrlicher Gegenprobe: A20a-1 (Replay), Mindestabstand, Wächter-Sperre,
//! Notfallsatz unter Sperre, A20a-2 (Absage mit tRot), Aufheben der Sperre.

mod v5common;

use silverscript_abi::ArtifactValue;
use v5common::*;

// ------------------------------------------------------------ Bausteine ----

#[derive(Clone)]
struct Propose {
    new: Satz,
    new_fb: [u8; 32],
    new_guard: [u8; 32],
    emerg: bool,
    auth: Option<Satz>,
    signers: Option<Vec<usize>>,
    lock: Option<i64>,
    seq: Option<u64>,
    payload: Option<Vec<u8>>,
}

impl Propose {
    fn new(new: Satz) -> Self {
        Self { new, new_fb: [0x31; 32], new_guard: [0x32; 32], emerg: false, auth: None, signers: None, lock: None, seq: None, payload: None }
    }
    fn emergency(new: Satz) -> Self {
        Self { emerg: true, ..Self::new(new) }
    }
    fn ann(&self) -> Vec<u8> {
        announcement(&self.new.hash(), &self.new_fb, &self.new_guard)
    }
    fn main(&self, w: &World) -> RSt {
        RSt { nonce: w.reg.nonce + 1, emerg: self.emerg, ..w.reg.clone() }
    }
    fn ticket(&self, w: &World) -> RSt {
        RSt {
            ticket: true,
            set: self.new.hash(),
            fb: self.new_fb,
            guard: self.new_guard,
            nonce: w.reg.nonce + 1,
            emerg: self.emerg,
            locked: false,
            ..w.reg.clone()
        }
    }
    fn run(&self, w: &World) -> Vec<Result<(), String>> {
        let auth = self.auth.clone().unwrap_or_else(|| if self.emerg { w.fb.clone().unwrap() } else { w.set.clone() });
        let tag = if self.emerg { TAG_EMERG } else { TAG_ROTATE };
        let digest = rot_digest(REG_COV, tag, w.reg.nonce + 1, &self.ann());
        let who = self.signers.clone().unwrap_or_else(|| Satz::first(auth.t_rot));
        let mut args = vec![
            i(self.new.n()),
            i(self.new.t),
            i(self.new.t_rot),
            ArtifactValue::Array(self.new.pubs().into_iter().map(ArtifactValue::Bytes).collect()),
            b(&self.new_fb),
            b(&self.new_guard),
            ArtifactValue::Bool(self.emerg),
        ];
        args.extend(auth.args());
        args.extend(auth.quorum(digest, &who));
        let mut rin = w.reg_in(&w.reg, "propose", args);
        if let Some(s) = self.seq {
            rin.sequence = s;
        }
        let lock = self.lock.unwrap_or(w.oracle.daa + 600);
        execute(vec![rin], vec![w.reg_out(&self.main(w), 0), w.reg_out(&self.ticket(w), 0)], lock as u64, self.payload.clone().unwrap_or_else(|| self.ann()))
    }
}

fn proposed(w: &World, p: &Propose) -> (World, RSt) {
    let r = p.run(w);
    assert!(all_ok(&r), "Ankündigung: {r:?}");
    let mut w2 = w.clone();
    w2.reg = p.main(w);
    (w2, p.ticket(w))
}

fn activated(w: &World, t: &RSt) -> RSt {
    RSt { set: t.set, fb: t.fb, guard: t.guard, nonce: w.reg.nonce + 1, emerg: false, locked: false, ..w.reg.clone() }
}

fn activate_tx(w: &World, ticket: &RSt, age: i64, out: Option<RSt>) -> Vec<Result<(), String>> {
    let out = out.unwrap_or_else(|| activated(w, ticket));
    let mut tin = w.reg_in(ticket, "settle", vec![i(0)]);
    tin.sequence = age as u64;
    execute(vec![w.reg_in(&w.reg, "activate", vec![i(1)]), tin], vec![w.reg_out(&out, 0)], 0, vec![])
}

fn cancel_tx(w: &World, satz: &Satz, who: &[usize], seq: Option<u64>) -> Vec<Result<(), String>> {
    let mut args = satz.args();
    args.extend(satz.quorum(tag_digest(REG_COV, TAG_CANCEL, w.reg.nonce + 1), who));
    let mut rin = w.reg_in(&w.reg, "cancel", args);
    if let Some(s) = seq {
        rin.sequence = s;
    }
    let out = RSt { nonce: w.reg.nonce + 1, emerg: false, ..w.reg.clone() };
    execute(vec![rin], vec![w.reg_out(&out, 0)], (w.oracle.daa + 600) as u64, vec![])
}

// ================================================== A20a-1: kein Replay ----

#[test]
fn a20a_1_replay_der_preis_signatur_mit_read_scheitert() {
    // ehrliches Update, danach steht ein Notfall-Ticket offen; die
    // Signatur-Bytes des Updates werden mit Orakel-read wiederholt
    let mut w = World::standard();
    let a = Attest::next(&w);
    let quorum = a.quorum_of(&w);
    a.apply(&mut w);
    let mut w2 = w.clone();
    w2.reg.emerg = true;
    w2.reg.nonce = 6;
    let mut replay = Attest::new(a.kas, a.daa, a.rate);
    replay.quorum = Some(quorum);
    replay.arg_seq = Some(w2.oracle.seq); // signiert war seq = jetzige seq
    replay.oracle_entry = "read";
    replay.oracle_out = Some(w2.oracle);
    replay.reg_out = Some(RSt { nonce: 7, emerg: false, last_daa: a.daa, ..w2.reg.clone() });
    let r = replay.run(&w2);
    assert!(r[0].is_err() && r[1].is_ok(), "v5: Register lehnt ab (seq ≠ Eingang + 1): {r:?}");
    // Gegenprobe: ein echtes Update entwertet das Notfall-Ticket weiterhin
    let real = Attest::next(&w2);
    let out = real.expected(&w2);
    assert_eq!(w2.reg_after_price(real.daa).nonce, 7);
    assert!(all_ok(&real.run(&w2)), "{out:?}");
}

#[test]
fn a20a_4_seq_muss_eingang_plus_eins_sein() {
    // Signatur über die JETZIGE seq (wie im Replay) mit read: v4-Test prüfte
    // nur die falsche Signatur; hier stimmt die Signatur, es scheitert an seq
    let w = World::standard();
    let mut a = Attest::next(&w);
    a.signed_seq = Some(w.oracle.seq);
    a.arg_seq = Some(w.oracle.seq);
    a.oracle_entry = "read";
    a.oracle_out = Some(w.oracle);
    let r = a.run(&w);
    assert!(r[0].is_err() && r[1].is_ok(), "{r:?}");
    // seq + 2 (signiert und behauptet): Register lehnt ab
    let mut c = Attest::next(&w);
    c.signed_seq = Some(w.oracle.seq + 2);
    c.arg_seq = Some(w.oracle.seq + 2);
    c.oracle_out = Some(OSt { seq: w.oracle.seq + 2, ..c.expected(&w) });
    assert!(c.run(&w)[0].is_err());
}

// ======================================================= Mindestabstand ----

#[test]
fn update_braucht_mindestalter_der_haupt_utxo() {
    let w = World::standard();
    let mut a = Attest::next(&w);
    a.reg_seq = Some(w.rcfg.min_gap as u64 - 1);
    let r = a.run(&w);
    assert!(r[0].is_err() && r[1].is_ok(), "eine DAA zu früh: {r:?}");
    a.reg_seq = Some(w.rcfg.min_gap as u64);
    assert!(all_ok(&a.run(&w)));
    // auch bei großem Rückstand des Orakels (oracleDaa weit hinter der
    // Kette): ohne Alter kein Update – das Problem aus A20e-1
    let mut back = Attest::new(w.oracle.kas_usd, w.oracle.daa + 600, w.oracle.rate);
    back.reg_seq = Some(0);
    assert!(back.run(&w)[0].is_err());
}

#[test]
fn ankuendigung_und_absage_brauchen_mindestalter() {
    let w = World::standard();
    let mut p = Propose::new(Satz::new(3, 2, 2));
    p.seq = Some(w.rcfg.min_gap as u64 - 1);
    assert!(p.run(&w)[0].is_err());
    p.seq = None;
    assert!(all_ok(&p.run(&w)));
    assert!(cancel_tx(&w, &w.set, &[0, 1], Some(w.rcfg.min_gap as u64 - 1))[0].is_err());
    assert!(all_ok(&cancel_tx(&w, &w.set, &[0, 1], None)));
}

// ================================================================ Wächter ----

#[test]
fn jeder_waechter_sperrt_sofort_ohne_mindestalter() {
    let w = World::standard();
    for who in 0..2 {
        let r = w.lock_tx(who, None);
        assert!(all_ok(&r), "Wächter {who}: {r:?}");
    }
}

#[test]
fn sperre_braucht_einen_echten_waechter() {
    let w = World::standard();
    let d = tag_digest(REG_COV, TAG_LOCK, w.reg.nonce + 1);
    let out = RSt { nonce: w.reg.nonce + 1, locked: true, ..w.reg.clone() };
    let run = |args: Vec<ArtifactValue>| {
        let mut x = w.reg_in(&w.reg, "guardLock", args);
        x.sequence = 0;
        execute(vec![x], vec![w.reg_out(&out, 0)], 0, vec![])
    };
    // fremder Schlüssel signiert
    assert!(run(w.guard.args(d, 0, &random_keypair()))[0].is_err());
    // Index außerhalb
    assert!(run(w.guard.args(d, 2, &w.guard.keys[0]))[0].is_err());
    // fremde Wächterliste (Hash passt nicht)
    let other = Waechter::new(2);
    assert!(run(other.args(d, 0, &other.keys[0]))[0].is_err());
    // Hauptsatz ist kein Wächter
    let s = Waechter { keys: w.set.keys.clone() };
    assert!(run(s.args(d, 0, &s.keys[0]))[0].is_err());
    // alte Sperr-Signatur (nonce) gilt nicht
    let old = tag_digest(REG_COV, TAG_LOCK, w.reg.nonce);
    assert!(run(w.guard.args(old, 0, &w.guard.keys[0]))[0].is_err());
    // Gegenprobe
    assert!(all_ok(&run(w.guard.args(d, 1, &w.guard.keys[1]))));
}

#[test]
fn ohne_waechter_keine_sperre() {
    let mut w = World::standard();
    w.reg.guard = [0; 32];
    assert!(w.lock_tx(0, Some(RSt { nonce: w.reg.nonce + 1, locked: true, ..w.reg.clone() }))[0].is_err());
}

#[test]
fn sperre_setzt_genau_locked_und_nonce() {
    let w = World::standard();
    let good = RSt { nonce: w.reg.nonce + 1, emerg: false, locked: true, ..w.reg.clone() };
    for bad in [
        RSt { locked: false, ..good.clone() },
        RSt { nonce: w.reg.nonce, ..good.clone() },
        RSt { set: [1; 32], ..good.clone() },
        RSt { guard: [1; 32], ..good.clone() },
        RSt { fb: [1; 32], ..good.clone() },
    ] {
        assert!(w.lock_tx(0, Some(bad))[0].is_err());
    }
    assert!(all_ok(&w.lock_tx(0, Some(good))));
}

#[test]
fn sperre_neben_einem_orakel_update_scheitert() {
    // Umgehung: guardLock prüft das Orakel nicht – stünde ein Orakel-update
    // daneben, käme ein Preis ohne Signaturen durch. noOracle verhindert das.
    let w = World::standard();
    let d = tag_digest(REG_COV, TAG_LOCK, w.reg.nonce + 1);
    let mut x = w.reg_in(&w.reg, "guardLock", w.guard.args(d, 0, &w.guard.keys[0]));
    x.sequence = 0;
    let fake = w.next(w.oracle.kas_usd + 1, w.oracle.daa + 600, w.oracle.rate);
    let r = execute(
        vec![x, w.oracle_in(w.oracle, "update", vec![i(fake.kas_usd), i(fake.daa), i(fake.rate)])],
        vec![w.reg_out(&RSt { nonce: w.reg.nonce + 1, locked: true, ..w.reg.clone() }, 0), w.oracle_out(fake, 1)],
        fake.daa as u64,
        vec![],
    );
    assert!(r[0].is_err(), "Register muss ablehnen: {r:?}");
}

#[test]
fn keine_zweite_sperre() {
    let w = World::standard().locked();
    assert!(w.lock_tx(0, None)[0].is_err());
}

#[test]
fn unter_sperre_kein_preis_keine_ankuendigung_keine_absage() {
    let w = World::standard().locked();
    let mut a = Attest::next(&w);
    a.reg_out = Some(RSt { last_daa: a.daa, locked: false, ..w.reg.clone() });
    assert!(a.run(&w)[0].is_err(), "Preis unter Sperre");
    a.reg_out = Some(RSt { last_daa: a.daa, ..w.reg.clone() });
    assert!(a.run(&w)[0].is_err(), "Preis unter Sperre (Sperre bleibt)");
    assert!(Propose::new(Satz::new(3, 2, 2)).run(&w)[0].is_err(), "Hauptsatz kündigt an");
    assert!(cancel_tx(&w, &w.set, &[0, 1], None)[0].is_err(), "Absage");
}

#[test]
fn sperre_entwertet_offene_tickets() {
    // der Dieb hat einen eigenen Satz angekündigt; der Wächter sperrt
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(3, 2, 2)));
    assert!(all_ok(&w2.lock_tx(0, None)));
    let w3 = w2.locked();
    let r = activate_tx(&w3, &t, w.rcfg.rot_delay, None);
    assert!(!all_ok(&r), "Ticket des Diebs nach der Sperre: {r:?}");
}

#[test]
fn notfallsatz_kuendigt_unter_sperre_sofort_an() {
    let w = World::standard();
    // ohne Sperre: erst nach Stille
    let p = Propose::emergency(Satz::new(3, 2, 2));
    assert!(p.run(&w)[0].is_err(), "ohne Sperre und ohne Stille");
    let mut q = p.clone();
    q.lock = Some(w.reg.last_daa + w.rcfg.emerg_after);
    assert!(all_ok(&q.run(&w)), "nach Stille wie v4");
    // unter Sperre: sofort, Sperre bleibt bis zur Aktivierung
    let wl = w.locked();
    let mut m = p.clone();
    let r = m.run(&wl);
    assert!(all_ok(&r), "{r:?}");
    assert!(m.main(&wl).locked);
    // Hauptsatz darf unter Sperre auch nicht als „Notfall“ ankündigen
    m.auth = Some(w.set.clone());
    assert!(m.run(&wl)[0].is_err());
}

#[test]
fn a20a_2_dieb_mit_allen_hauptschluesseln_kann_den_notfall_austausch_nicht_aufhalten() {
    // 1 von 1 (heute): Dieb und Betreiber halten denselben Schlüssel.
    let w = World::solo();
    assert!(all_ok(&w.lock_tx(1, None)));
    let wl = w.locked();
    let neu = Satz::new(3, 2, 2);
    let (w2, t) = proposed(&wl, &Propose::emergency(neu.clone()));
    // Dieb: Absage, Gegen-Ankündigung, Preis – alles abgelehnt
    assert!(cancel_tx(&w2, &w.set, &[0], None)[0].is_err());
    assert!(Propose::new(Satz::new(1, 1, 1)).run(&w2)[0].is_err());
    let mut a = Attest::next(&w2);
    a.reg_out = Some(RSt { last_daa: a.daa, ..w2.reg.clone() });
    assert!(a.run(&w2)[0].is_err());
    // zu früh
    assert!(!all_ok(&activate_tx(&w2, &t, w.rcfg.emerg_delay - 1, None)));
    // nach emergDelay: neuer Satz, Sperre aufgehoben
    let r = activate_tx(&w2, &t, w.rcfg.emerg_delay, None);
    assert!(all_ok(&r), "{r:?}");
    let act = activated(&w2, &t);
    assert!(!act.locked && act.set == neu.hash());
    // die Aktivierung muss die Sperre aufheben (gesperrt bleiben ist kein gültiger Ausgang)
    assert!(!all_ok(&activate_tx(&w2, &t, w.rcfg.emerg_delay, Some(RSt { locked: true, ..act.clone() }))));
    // danach setzt der neue Satz Preise, der alte nicht
    let mut w3 = w2.clone();
    w3.reg = act;
    w3.set = neu;
    assert!(all_ok(&Attest::next(&w3).run(&w3)));
    let mut old = Attest::next(&w3);
    old.satz = Some(w.set.clone());
    assert!(old.run(&w3)[0].is_err());
}

#[test]
fn a20a_2_absage_braucht_trot() {
    // t = 3 < tRot = 4: drei Schlüssel (Preis-Quorum) können nicht mehr absagen
    let w = World::new(Satz::new(5, 3, 4), Some(Satz::new(1, 1, 1)), RCFG_SOLO);
    let (w2, _) = proposed(&w, &Propose::new(Satz::new(3, 2, 2)));
    assert!(cancel_tx(&w2, &w.set, &[0, 1, 2], None)[0].is_err(), "t Signaturen");
    assert!(all_ok(&cancel_tx(&w2, &w.set, &[0, 1, 2, 3], None)), "tRot Signaturen");
}

#[test]
fn notfall_ticket_aus_stille_verfaellt_durch_echtes_update() {
    let w = World::standard();
    let mut p = Propose::emergency(Satz::new(3, 2, 2));
    p.lock = Some(w.reg.last_daa + w.rcfg.emerg_after);
    let (w2, t) = proposed(&w, &p);
    let mut w3 = w2.clone();
    Attest::next(&w2).apply(&mut w3);
    assert_eq!((w3.reg.nonce, w3.reg.emerg), (w2.reg.nonce + 1, false));
    assert!(!all_ok(&activate_tx(&w3, &t, w.rcfg.emerg_delay, None)));
}

#[test]
fn ankuendigung_traegt_waechter_im_payload() {
    let w = World::standard();
    let p = Propose::new(Satz::new(3, 2, 2));
    let mut q = p.clone();
    q.payload = Some(announcement(&p.new.hash(), &p.new_fb, &[0x99; 32]));
    assert!(q.run(&w)[0].is_err());
    let (w2, t) = proposed(&w, &p);
    let act = activated(&w2, &t);
    assert_eq!(act.guard, [0x32; 32]);
    assert!(all_ok(&activate_tx(&w2, &t, w.rcfg.rot_delay, None)));
    // Wächter ändern sich nur mit
    assert!(!all_ok(&activate_tx(&w2, &t, w.rcfg.rot_delay, Some(RSt { guard: w.reg.guard, ..act }))));
}

#[test]
fn abgelaufenes_ticket_wird_neben_einem_register_eintrag_aufgeraeumt() {
    // ohne witness: ein abgelaufenes Ticket fährt neben cancel/propose mit.
    // Neben einem Preis-Update geht es NICHT: das Orakel verlangt genau einen
    // Register-Eingang (wie v4 für settle).
    let w = World::standard();
    let (w2, t) = proposed(&w, &Propose::new(Satz::new(3, 2, 2)));
    let mut w3 = w2.clone();
    w3.reg.nonce += 1; // abgesagt
    let mut args = w3.set.args();
    args.extend(w3.set.quorum(tag_digest(REG_COV, TAG_CANCEL, w3.reg.nonce + 1), &[0, 1]));
    let out = RSt { nonce: w3.reg.nonce + 1, ..w3.reg.clone() };
    let r = execute(vec![w3.reg_in(&w3.reg, "cancel", args), w3.reg_in(&t, "settle", vec![i(0)])], vec![w3.reg_out(&out, 0)], 0, vec![]);
    assert!(all_ok(&r), "{r:?}");
    let a = Attest::next(&w3);
    let e = a.expected(&w3);
    let mut reg_args = vec![i(a.kas), i(a.daa), i(e.seq), i(a.rate), i(e.index), i(e.last_rate_daa), i(e.ref_usd), i(e.cand_usd), i(e.cand_seq)];
    reg_args.extend(w3.set.args());
    reg_args.extend(a.quorum_of(&w3));
    let r = execute(
        vec![
            w3.reg_in(&w3.reg, "attestPrice", reg_args),
            w3.oracle_in(w3.oracle, "update", vec![i(a.kas), i(a.daa), i(a.rate)]),
            w3.reg_in(&t, "settle", vec![i(0)]),
        ],
        vec![w3.reg_out(&w3.reg_after_price(a.daa), 0), w3.oracle_out(e, 1)],
        a.daa as u64,
        vec![],
    );
    assert!(r[0].is_ok() && r[1].is_err(), "{r:?}");
}

#[test]
fn witness_gibt_es_nicht_mehr() {
    let w = World::standard();
    let art = w.reg_art(&w.reg);
    let c = art.contracts.values().next().unwrap();
    assert!(c.entry("witness").is_none());
    assert!(c.entry("guardLock").is_some());
}

#[test]
fn preis_braucht_quorum_und_bindet_seq() {
    let w = World::standard();
    assert!(all_ok(&Attest::next(&w).run(&w)));
    let mut a = Attest::next(&w);
    a.signers = Some(vec![0]);
    assert!(a.run(&w)[0].is_err(), "zu wenige");
    let mut f = Attest::next(&w);
    f.satz = w.fb.clone();
    assert!(f.run(&w)[0].is_err(), "Notfallsatz setzt keine Preise");
}
