//! Tests für contracts/zinskasse_v4.sil: Weiterleiten des Zinses an das
//! Zinsziel, das im Unterzeichner-Register steht (änderbar nur über
//! propose → Wartezeit → activate). Jede Regel mit Gegenprobe.

mod v4common;

use kaspa_consensus_core::tx::TransactionOutput;
use silverscript_abi::SilAbiArtifact;
use v4common::*;

const KV: i64 = 3 * E8; // eine Zins-Zahlung

fn kasse(w: &World) -> SilAbiArtifact {
    kasse_art(REG_COV, &tpl(&w.reg_art(&w.reg)))
}

fn kasse_in(w: &World, reg_idx: i64, value: i64) -> In {
    let art = kasse(w);
    In { utxo: p2sh_utxo(&art, value), call: Call::Entry { art, entry: "forward", args: vec![i(reg_idx)], sig_at: None }, sequence: 0 }
}

/// Register (witness) an 0, Zinskasse an 1, Zahlung an Ausgang 1
fn forward_tx(w: &World, pay: TransactionOutput) -> Vec<Result<(), String>> {
    execute(vec![w.reg_in(&w.reg, "witness", vec![]), kasse_in(w, 0, KV)], vec![w.reg_out(&w.reg, 0), pay], 0, vec![])
}

fn world(kind: u8, to: [u8; 32]) -> World {
    let mut w = World::standard();
    w.reg.pay_kind = kind;
    w.reg.pay_to = to;
    w
}

#[test]
fn weiterleiten_an_p2pk() {
    let k = random_keypair();
    let to: [u8; 32] = xonly(&k).try_into().unwrap();
    let w = world(0, to);
    assert!(all_ok(&forward_tx(&w, p2pk_out(&to, KV))));
    // Gegenproben: anderer Empfänger, zu wenig, P2SH desselben Werts
    assert!(forward_tx(&w, p2pk_out(&xonly(&random_keypair()), KV))[1].is_err());
    assert!(forward_tx(&w, p2pk_out(&to, KV - 1))[1].is_err());
    assert!(forward_tx(&w, spk_out(kaspa_txscript::pay_to_script_hash_script(&to), KV))[1].is_err());
}

#[test]
fn weiterleiten_an_p2sh_und_verbrennen() {
    for redeem in [vec![0x51u8], vec![0x00]] {
        let h = p2sh_hash(&redeem);
        let w = world(1, h);
        let pay = spk_out(kaspa_txscript::pay_to_script_hash_script(&redeem), KV);
        assert!(all_ok(&forward_tx(&w, pay)));
        assert!(forward_tx(&w, p2pk_out(&h, KV))[1].is_err(), "P2PK statt P2SH");
        assert!(forward_tx(&w, spk_out(kaspa_txscript::pay_to_script_hash_script(&[0x52]), KV))[1].is_err());
    }
}

#[test]
fn unbekannte_art_haelt_das_geld_fest() {
    let to = [0x42; 32];
    let w = world(2, to);
    assert!(forward_tx(&w, p2pk_out(&to, KV))[1].is_err());
    assert!(forward_tx(&w, spk_out(kaspa_txscript::pay_to_script_hash_script(&[0x51]), KV))[1].is_err());
}

#[test]
fn zahlung_am_index_des_eigenen_eingangs() {
    let to = [0x11; 32];
    let w = world(1, to);
    let pay = |v| spk_out(kaspa_txscript::pay_to_script_hash_script(&[0x51]), v);
    let w = World { reg: RSt { pay_to: p2sh_hash(&[0x51]), ..w.reg.clone() }, ..w };
    // an anderer Stelle zählt nicht
    let r = execute(vec![w.reg_in(&w.reg, "witness", vec![]), kasse_in(&w, 0, KV)], vec![w.reg_out(&w.reg, 0), plain_out(1), pay(KV)], 0, vec![]);
    assert!(r[1].is_err(), "{r:?}");
    // zwei Kassen-Eingänge dürfen sich keinen Ausgang teilen (A11-V-1)
    let two = |outs: Vec<TransactionOutput>| {
        execute(vec![w.reg_in(&w.reg, "witness", vec![]), kasse_in(&w, 0, KV), kasse_in(&w, 0, KV)], outs, 0, vec![])
    };
    let shared = two(vec![w.reg_out(&w.reg, 0), pay(KV), plain_out(1)]);
    assert!(shared[2].is_err(), "{shared:?}");
    assert!(all_ok(&two(vec![w.reg_out(&w.reg, 0), pay(KV), pay(KV)])), "Gegenprobe: je ein Ausgang");
}

#[test]
fn ziel_kommt_nur_aus_der_echten_haupt_utxo() {
    let to = [0x11; 32];
    let w = world(0, to);
    let evil = [0x66; 32];
    // ohne Register: Eingang 0 ist ein gewöhnlicher Eingang
    let r = execute(vec![plain_in(E8), kasse_in(&w, 0, KV)], vec![plain_out(1), p2pk_out(&to, KV)], 0, vec![]);
    assert!(r[1].is_err(), "{r:?}");
    // gefälschtes Register (eigene Covenant-ID, eigenes Ziel)
    let fake = RSt { pay_to: evil, ..w.reg.clone() };
    let r = execute(
        vec![entry_in(&w.reg_art(&fake), REG_V, OTHER_COV, "witness", vec![]), kasse_in(&w, 0, KV)],
        vec![cov_out(&w.reg_art(&fake), REG_V, 0, OTHER_COV), p2pk_out(&evil, KV)],
        0,
        vec![],
    );
    assert!(r[1].is_err(), "{r:?}");
    // Ticket mit angekündigtem (noch nicht wirksamem) Ziel
    let ticket = RSt { ticket: true, pay_to: evil, ..w.reg.clone() };
    let mut w2 = w.clone();
    w2.reg.nonce = 1;
    let ticket = RSt { nonce: 0, ..ticket };
    let run = |idx: i64, to: &[u8; 32]| {
        execute(
            vec![w2.reg_in(&w2.reg, "witness", vec![]), w2.reg_in(&ticket, "settle", vec![i(0)]), kasse_in(&w2, idx, KV)],
            vec![w2.reg_out(&w2.reg, 0), plain_out(1), p2pk_out(to, KV)],
            0,
            vec![],
        )
    };
    let r = run(1, &evil);
    assert!(r[2].is_err(), "Ziel aus dem Ticket: {r:?}");
    assert!(all_ok(&run(0, &to)), "Gegenprobe: Ziel aus der Haupt-UTXO");
}

#[test]
fn groesse() {
    let w = World::standard();
    println!("Zinskasse v4: Redeem-Skript {} B", v4common::common::bytecode(&kasse(&w)).len());
}

#[test]
fn nie_an_die_kasse_selbst() {
    // Zinsziel = die Kasse selbst (P2SH ihres eigenen Skripts): ein Vault, der
    // beim Schließen an die Kasse zahlen muss, könnte sonst den Ausgang einer
    // Weiterleitung als seine Zinszahlung mitbenutzen
    let mut w = world(1, [0; 32]);
    let own = v4common::common::bytecode(&kasse(&w));
    w.reg.pay_to = p2sh_hash(&own);
    let r = forward_tx(&w, spk_out(kaspa_txscript::pay_to_script_hash_script(&own), KV));
    assert!(r[1].is_err(), "{r:?}");
    let w2 = world(1, p2sh_hash(&[0x51]));
    assert!(all_ok(&forward_tx(&w2, spk_out(kaspa_txscript::pay_to_script_hash_script(&[0x51]), KV))));
}

#[test]
fn unbekannte_art_auch_nicht_als_p2sh() {
    // payKind 2 mit payTo = Hash eines Skripts: weder P2PK noch P2SH erlaubt
    let h = p2sh_hash(&[0x51]);
    let w = world(2, h);
    let r = forward_tx(&w, spk_out(kaspa_txscript::pay_to_script_hash_script(&[0x51]), KV));
    assert!(r[1].is_err(), "{r:?}");
    assert!(all_ok(&forward_tx(&world(1, h), spk_out(kaspa_txscript::pay_to_script_hash_script(&[0x51]), KV))), "Gegenprobe Art 1");
}
