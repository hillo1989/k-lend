//! GHOST-Token Version 2 (contracts/ghost_token.sil): keine negativen Beträge.
//! Audit 1 V-01 (28.09.2026) hatte mit dem unveränderten KCC20 gezeigt, dass
//! 1 Einheit in [+D, −(D−1)] aufgeteilt werden konnte.

use kaspa_consensus_core::tx::{CovenantBinding, TransactionOutput};
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math::E8;
use kaspa_lending_protocol::ops::{p2pk_spk, self, Deployment, xonly};
use kaspa_lending_protocol::sim::{Sim, test_register_params};
use kaspa_lending_protocol::txb::{Draft, In, Unlock, build};
use secp256k1::{Keypair, Secp256k1};

fn key() -> Keypair {
    Keypair::new(&Secp256k1::new(), &mut rand::thread_rng())
}

/// Deployment + ein Vault des Nutzers mit 10 GHOST auf einem Token-UTXO.
fn world() -> (Sim, Deployment, Keypair) {
    let mut sim = Sim::new();
    let deployer = key();
    let user = key();
    sim.faucet(&deployer, 1_000 * E8 as u64);
    sim.faucet(&user, 5_000 * E8 as u64);
    let net = sim.params.clone();
    let feed = sim.deploy_feed(&deployer, test_register_params(&deployer), SignerSet { keys: (0..5).map(|_| xonly(&key())).collect(), t: 3, t_rot: 3 }, 4_000_000, 0, 1_000_000_000).expect("Register und Orakel");
    let op = feed.oracle_params.clone();
    let o = feed.oracle.clone();
    let fp = FactoryParams { deployer: xonly(&deployer), ghost_tpl: ghost_template() };
    let (b, f) = ops::deploy_factory(&fp, 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let (b, f2, root, vp) = ops::init_factory(&op, &o, &fp, &f, &deployer, (20_000, 15_000, 1_000, NO_DEBT_LIMIT), spk_bytes(&p2pk_spk(&xonly(&deployer))), 100_000_000, &sim.funds(&deployer), &net).unwrap();
    sim.submit(&b).unwrap();
    let mut dep = Deployment { network: "sim".into(), register_params: feed.register_params.clone(), register: feed.register.clone(), signer_set: feed.signer_set.clone(), fallback_set: None, rotation: None, old_tickets: vec![], foreign_change: None, signers_unknown: false, oracle_params: op, oracle: o, factory_params: fp, factory: f2, ghost_root: Some(root), vault_params: Some(vp), vaults: vec![], tokens: vec![], pool: None, pool_pending: None, lp_tokens: vec![], pool_unresolved: None };
    let (b, d) = ops::open_vault(&dep, &xonly(&user), 2_000 * E8 as u64, &sim.funds(&user), &net).unwrap();
    sim.submit(&b).unwrap();
    dep = d;
    let (b, d) = ops::mint(&dep, 0, &user, 10 * E8, &xonly(&user), &sim.funds(&user), &net).unwrap();
    sim.submit(&b).unwrap();
    (sim, d, user)
}

/// Überweisung des einzigen Token-UTXOs mit frei gewählten Ausgangsbeträgen.
fn split(sim: &Sim, dep: &Deployment, user: &Keypair, amounts: &[i64]) -> Result<(), String> {
    let t = &dep.tokens[0];
    let art = t.state.artifact();
    let outs: Vec<GhostTok> = amounts.iter().map(|&a| GhostTok::to_pubkey(&xonly(user), a)).collect();
    let states = outs.iter().map(GhostTok::arg).collect();
    let mut inputs = vec![In {
        outpoint: t.outpoint,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Leader { art: art.clone(), new_states: states, signer: Some((*user).into()) },
    }];
    for (op, e) in &sim.funds(user).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (*user).into() } });
    }
    let outputs = outs
        .iter()
        .map(|o| TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&o.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) })
        .collect();
    build(Draft { inputs, outputs, change_spk: ops::p2pk_spk(&xonly(user)), lock_time: 0 }, &sim.params).map(|_| ())
}

#[test]
fn aufteilen_mit_positiven_betraegen_geht() {
    let (sim, dep, user) = world();
    assert_eq!(dep.tokens[0].state.amount, 10 * E8);
    split(&sim, &dep, &user, &[4 * E8, 6 * E8]).expect("ehrliche Aufteilung");
}

#[test]
fn negativer_ausgang_wird_abgelehnt() {
    // [+1 000 000 GHOST, −999 990 GHOST] – Summe stimmt, war im alten KCC20 gültig
    let (sim, dep, user) = world();
    let d = 1_000_000 * E8;
    let e = split(&sim, &dep, &user, &[d, 10 * E8 - d]).expect_err("negativer Betrag muss scheitern");
    assert!(e.contains("Input 0"), "der Token selbst muss ablehnen: {e}");
}

#[test]
fn null_ausgang_bleibt_erlaubt() {
    // amount >= 0: ein 0-Ausgang ist harmlos (keine Wertschöpfung)
    let (sim, dep, user) = world();
    split(&sim, &dep, &user, &[10 * E8, 0]).expect("0 ist erlaubt");
}

// ------------------------------------------------ nach Mutationstest v2 ----

/// Überweisung mit frei wählbaren Ausgängen UND wählbarem Besitzer-Typ.
fn transfer_states(sim: &Sim, dep: &Deployment, user: &Keypair, outs: Vec<GhostTok>) -> Result<(), String> {
    let t = &dep.tokens[0];
    let art = t.state.artifact();
    let states = outs.iter().map(GhostTok::arg).collect();
    let mut inputs = vec![In {
        outpoint: t.outpoint,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Leader { art: art.clone(), new_states: states, signer: Some((*user).into()) },
    }];
    for (op, e) in &sim.funds(user).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (*user).into() } });
    }
    let outputs = outs
        .iter()
        .map(|o| TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&o.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) })
        .collect();
    build(Draft { inputs, outputs, change_spk: ops::p2pk_spk(&xonly(user)), lock_time: 0 }, &sim.params).map(|_| ())
}

#[test]
fn ueberweisung_darf_keine_menge_erzeugen() {
    // L50 (Mengenerhaltung): 10 GHOST rein, 11 raus
    let (sim, dep, user) = world();
    assert!(split(&sim, &dep, &user, &[5 * E8, 5 * E8]).is_ok(), "Gegenprobe");
    let e = split(&sim, &dep, &user, &[6 * E8, 5 * E8]).expect_err("11 aus 10");
    assert!(e.contains("Input 0"), "{e}");
}

#[test]
fn minter_zweig_ohne_seinen_vault_nicht_nutzbar() {
    // L32: der Minter-Zweig gehört der Vault-Covenant-ID; ohne Vault-Input in
    // der Tx darf er nicht prägen (sonst Prägen ohne Schuldbuchung)
    let (sim, dep, user) = world();
    let br = &dep.vaults[0].branch;
    let art = br.state.artifact();
    let loot = GhostTok::to_pubkey(&xonly(&user), 1_000 * E8);
    let states = vec![br.state.arg(), loot.arg()];
    let mut inputs = vec![In {
        outpoint: br.outpoint,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(br.value, spk(&art), 0, false, Some(br.cov)),
        unlock: Unlock::Leader { art: art.clone(), new_states: states, signer: None },
    }];
    for (op, e) in &sim.funds(&user).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (user).into() } });
    }
    let outputs = vec![
        TransactionOutput { value: br.value, script_public_key: spk(&art), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: br.cov }) },
        TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&loot.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: br.cov }) },
    ];
    let e = build(Draft { inputs, outputs, change_spk: ops::p2pk_spk(&xonly(&user)), lock_time: 0 }, &sim.params).expect_err("Prägen ohne Vault");
    assert!(e.contains("Input 0"), "{e}");
}

#[test]
fn unbekannter_besitzertyp_ist_nicht_ausgebbar() {
    // L34: GHOST an Besitzer-Typ 3 (gibt es nicht) → danach für niemanden ausgebbar
    let (mut sim, dep, user) = world();
    let net = sim.params.clone();
    let odd = GhostTok { owner: xonly(&user), typ: 3, amount: 10 * E8, minter: false };
    transfer_states(&sim, &dep, &user, vec![odd.clone()]).expect("Überweisung an Typ 3 ist selbst erlaubt");
    // tatsächlich ausführen und dann versuchen, den Typ-3-Token auszugeben
    let t = &dep.tokens[0];
    let art = t.state.artifact();
    let mut inputs = vec![In {
        outpoint: t.outpoint,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Leader { art: art.clone(), new_states: vec![odd.arg()], signer: Some((user).into()) },
    }];
    for (op, e) in &sim.funds(&user).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (user).into() } });
    }
    let out = TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&odd.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) };
    let b = build(Draft { inputs, outputs: vec![out], change_spk: ops::p2pk_spk(&xonly(&user)), lock_time: 0 }, &net).unwrap();
    sim.submit(&b).unwrap();
    let odd_op = kaspa_consensus_core::tx::TransactionOutpoint { transaction_id: b.tx.id(), index: 0 };
    let odd_art = odd.artifact();
    let thief = key();
    sim.faucet(&thief, 10 * E8 as u64);
    let back = GhostTok::to_pubkey(&xonly(&thief), 10 * E8);
    let mut inputs = vec![In {
        outpoint: odd_op,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(ops::TOKEN_VALUE, spk(&odd_art), 0, false, Some(t.cov)),
        unlock: Unlock::Leader { art: odd_art.clone(), new_states: vec![back.arg()], signer: None },
    }];
    for (op, e) in &sim.funds(&thief).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (thief).into() } });
    }
    let out = TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&back.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) };
    let e = build(Draft { inputs, outputs: vec![out], change_spk: ops::p2pk_spk(&xonly(&thief)), lock_time: 0 }, &net).expect_err("Typ 3 darf niemand ausgeben");
    assert!(e.contains("Input 0"), "{e}");
}

/// Gibt den Token-UTXO 0 mit beliebigem Unterzeichner aus.
fn spend_as(sim: &mut Sim, dep: &Deployment, signer: &Keypair, outs: Vec<GhostTok>) -> Result<(), String> {
    let t = &dep.tokens[0];
    let art = t.state.artifact();
    let states = outs.iter().map(GhostTok::arg).collect();
    sim.faucet(signer, 10 * E8 as u64);
    let mut inputs = vec![In {
        outpoint: t.outpoint,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Leader { art: art.clone(), new_states: states, signer: Some((*signer).into()) },
    }];
    for (op, e) in &sim.funds(signer).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (*signer).into() } });
    }
    let outputs = outs
        .iter()
        .map(|o| TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&o.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) })
        .collect();
    build(Draft { inputs, outputs, change_spk: ops::p2pk_spk(&xonly(signer)), lock_time: 0 }, &sim.params).map(|_| ())
}

#[test]
fn fremde_ghost_nicht_ohne_signatur_des_besitzers() {
    // L27: ohne die Signaturprüfung könnte jeder fremde GHOST ausgeben
    let (mut sim, dep, user) = world();
    let thief = key();
    spend_as(&mut sim, &dep, &user, vec![GhostTok::to_pubkey(&xonly(&user), 10 * E8)]).expect("Gegenprobe: Besitzer darf");
    let e = spend_as(&mut sim, &dep, &thief, vec![GhostTok::to_pubkey(&xonly(&thief), 10 * E8)]).expect_err("Dieb darf nicht");
    assert!(e.contains("Input 0"), "{e}");
}

#[test]
fn halter_darf_keinen_minter_zweig_erzeugen() {
    // L64: ein gewöhnlicher Halter erzeugt einen Minter-Zweig für sich selbst –
    // der könnte danach unbegrenzt prägen
    let (mut sim, dep, user) = world();
    let rogue = GhostTok { owner: xonly(&user), typ: ID_PUBKEY, amount: 10 * E8, minter: true };
    let e = spend_as(&mut sim, &dep, &user, vec![rogue]).expect_err("Minter aus normalem Token");
    assert!(e.contains("Input 0"), "{e}");
}

#[test]
fn skript_besitz_nur_mit_passendem_input() {
    // L30: GHOST, die einem Skript-Hash gehören, sind nur mit einem Input dieses
    // Skripts ausgebbar. Hier: Besitzer = Hash eines OpTrue-Skripts, danach
    // versucht ein Fremder die GHOST ohne diesen Input auszugeben.
    use kaspa_txscript::opcodes::codes::OpTrue;
    let (mut sim, dep, user) = world();
    let net = sim.params.clone();
    let redeem = vec![OpTrue];
    let hash = {
        use blake2b_simd::Params;
        Params::new().hash_length(32).to_state().update(&redeem).finalize().as_bytes().to_vec()
    };
    let owned = GhostTok { owner: hash, typ: 0x01, amount: 10 * E8, minter: false };
    // Überweisung an den Skript-Hash (vom echten Besitzer)
    let t = &dep.tokens[0];
    let art = t.state.artifact();
    let mut inputs = vec![In {
        outpoint: t.outpoint,
        entry: kaspa_consensus_core::tx::UtxoEntry::new(t.value, spk(&art), 0, false, Some(t.cov)),
        unlock: Unlock::Leader { art: art.clone(), new_states: vec![owned.arg()], signer: Some((user).into()) },
    }];
    for (op, e) in &sim.funds(&user).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (user).into() } });
    }
    let out = TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&owned.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) };
    let b = build(Draft { inputs, outputs: vec![out], change_spk: ops::p2pk_spk(&xonly(&user)), lock_time: 0 }, &net).unwrap();
    sim.submit(&b).unwrap();
    // Fremder gibt sie ohne Skript-Input aus
    let thief = key();
    sim.faucet(&thief, 10 * E8 as u64);
    let o_art = owned.artifact();
    let loot = GhostTok::to_pubkey(&xonly(&thief), 10 * E8);
    let mut inputs = vec![In {
        outpoint: kaspa_consensus_core::tx::TransactionOutpoint { transaction_id: b.tx.id(), index: 0 },
        entry: kaspa_consensus_core::tx::UtxoEntry::new(ops::TOKEN_VALUE, spk(&o_art), 0, false, Some(t.cov)),
        // Zeuge zeigt auf Input 1 (die P2PK-Gebühr des Diebs) statt auf das Skript
        unlock: Unlock::Leader { art: o_art.clone(), new_states: vec![loot.arg()], signer: None },
    }];
    for (op, e) in &sim.funds(&thief).utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (thief).into() } });
    }
    let out = TransactionOutput { value: ops::TOKEN_VALUE, script_public_key: spk(&loot.artifact()), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: t.cov }) };
    let e = build(Draft { inputs, outputs: vec![out], change_spk: ops::p2pk_spk(&xonly(&thief)), lock_time: 0 }, &net).expect_err("ohne Skript-Input");
    assert!(e.contains("Input 0"), "{e}");
}
