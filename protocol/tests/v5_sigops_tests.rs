//! Version 5: Mempool-Standardregel MAX_STANDARD_P2SH_SIG_OPS (15 je
//! P2SH-Eingang, statisch gezählt) und Skriptgrößen aller v5-Verträge.
//! Register v5 enthält drei Quorum-Stellen (je 4) plus init und guardLock.

use kaspa_consensus_core::Hash;
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::pool::{PoolBand, PoolParams};
use kaspa_lending_protocol::txb::{MAX_STANDARD_P2SH_SIG_OPS, push_redeem};
use kaspa_txscript::post_toccata_p2sh_sig_scanner;

fn sig_ops(a: &Artifact) -> u64 {
    post_toccata_p2sh_sig_scanner(&push_redeem(a), &spk(a))
}

fn all() -> Vec<(&'static str, Artifact)> {
    let h = Hash::from_bytes([7; 32]);
    let k = vec![9u8; 32];
    let set = SignerSet { keys: vec![k.clone()], t: 1, t_rot: 1 };
    let rp = RegisterV5Params { deployer: k.clone(), min_signers: 1, min_threshold: 1, rot_delay_daa: 1, emerg_after_daa: 1, emerg_delay_daa: 1, min_gap_daa: 3_000 };
    let op = OracleV5Params { reg_cov: h, max_rate: 1, rate_step: 1, rate_gap_daa: 1, freeze_after_daa: 1, jump_bps: 2_500, ref_after: 6 };
    let os = OracleV5State::genesis(4_000_000, 1, 0);
    let otpl = Template::of(&oracle_v5(&op, &os));
    let vp = VaultV5Params {
        base: VaultParams {
            oracle_cov: h,
            oracle_tpl: otpl.clone(),
            ghost_cov: h,
            ghost_tpl: ghost_template(),
            mcr_bps: 20_000,
            liq_bps: 15_000,
            bonus_bps: 1_000,
            max_debt: 1,
            interest_spk: spk_bytes(&kaspa_lending_protocol::ops::p2pk_spk(&k)),
        },
        settle_after_daa: 60 * DAY_DAA,
        settle_fee_bps: 500,
    };
    let pp = PoolParams { ghost_cov: h, tpl: ghost_template(), fee_bps: 30, creator: k.clone(), band: Some(PoolBand { oracle_cov: h, oracle_tpl: otpl, band_bps: 300, stop_when_frozen: true }) };
    vec![
        ("Register v5", register_v5(&rp, &RegisterV5State::genesis(&set, None, None, 1))),
        ("Orakel v5", oracle_v5(&op, &os)),
        ("Vault v5", vault_v5(&vp, &k, &VaultState::default())),
        ("Pool v5", pool_v5(&pp, &h, true).unwrap()),
    ]
}

#[test]
fn v5_vertragsskripte_bleiben_unter_der_standardgrenze() {
    let mut bad = vec![];
    for (name, a) in all() {
        let n = sig_ops(&a);
        println!("{name:<12} {n:>3} Signaturprüfungen (statisch), Redeem-Skript {} B", bytecode(&a).len());
        if n > MAX_STANDARD_P2SH_SIG_OPS {
            bad.push(format!("{name}: {n}"));
        }
    }
    assert!(bad.is_empty(), "über {MAX_STANDARD_P2SH_SIG_OPS}: {bad:?}");
}

#[test]
fn v5_signaturpruefungen_genau_gezaehlt() {
    let a = all();
    let n: Vec<u64> = a.iter().map(|(_, x)| sig_ops(x)).collect();
    // Register: 3 Quorum-Stellen · 4 + init (checkSig) + guardLock (checkMsgSig)
    assert_eq!(n[0], 14, "Register v5");
    assert_eq!(n[1], 0, "Orakel v5 prüft keine Signaturen");
    assert_eq!(n[2], 5, "Vault v5 wie v4 (settle ohne Signatur)");
    assert_eq!(n[3], 1, "Pool v5 wie v4");
}

#[test]
fn v5_groessen_gegen_v4() {
    let h = Hash::from_bytes([7; 32]);
    let k = vec![9u8; 32];
    let a = all();
    let op4 = OracleParams { reg_cov: h, max_rate: 1, rate_step: 1, rate_gap_daa: 1, freeze_after_daa: 1 };
    let os4 = OracleState { kas_usd: 1, oracle_daa: 1, seq: 1, stable_rate: 0, stable_index: 1, frozen: false, last_rate_daa: 1 };
    let vp4 = VaultParams {
        oracle_cov: h,
        oracle_tpl: Template::of(&oracle(&op4, &os4)),
        ghost_cov: h,
        ghost_tpl: ghost_template(),
        mcr_bps: 20_000,
        liq_bps: 15_000,
        bonus_bps: 1_000,
        max_debt: 1,
        interest_spk: spk_bytes(&kaspa_lending_protocol::ops::p2pk_spk(&k)),
    };
    let rp4 = RegisterParams { deployer: k.clone(), min_signers: 1, min_threshold: 1, rot_delay_daa: 1, emerg_after_daa: 1, emerg_delay_daa: 1 };
    let set = SignerSet { keys: vec![k.clone()], t: 1, t_rot: 1 };
    let v4 = [
        bytecode(&register(&rp4, &RegisterState::genesis(&set, None, 0, [0; 32], 1))).len(),
        bytecode(&oracle(&op4, &os4)).len(),
        bytecode(&vault(&vp4, &k, &VaultState::default())).len(),
    ];
    for (i, name) in ["Register", "Orakel", "Vault"].iter().enumerate() {
        let v5 = bytecode(&a[i].1).len();
        println!("{name:<9} v4 {:>6} B  v5 {v5:>6} B  ({:+} B)", v4[i], v5 as i64 - v4[i] as i64);
    }
    // Das Orakel fährt in jeder Vault- und Pool-Tx mit: höchstens 200 B mehr als v4
    assert!(bytecode(&a[1].1).len() <= v4[1] + 200);
}
