//! Mempool-Standardregel MAX_STANDARD_P2SH_SIG_OPS (15 je P2SH-Eingang,
//! statisch gezählt): jedes Vertragsskript muss darunter bleiben. Die
//! Mainnet-Probe vom 05.10.2026 lief mit dem Register (28) in diese Regel.

use kaspa_consensus_core::Hash;
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::pool::{PoolBand, PoolParams, pool_artifact};
use kaspa_lending_protocol::txb::{MAX_STANDARD_P2SH_SIG_OPS, push_redeem};
use kaspa_txscript::post_toccata_p2sh_sig_scanner;

fn sig_ops(a: &Artifact) -> u64 {
    post_toccata_p2sh_sig_scanner(&push_redeem(a), &spk(a))
}

#[test]
fn jedes_vertragsskript_bleibt_unter_der_standardgrenze() {
    let h = Hash::from_bytes([7; 32]);
    let k = vec![9u8; 32];
    let set = SignerSet { keys: vec![k.clone()], t: 1, t_rot: 1 };
    let rp = RegisterParams { deployer: k.clone(), min_signers: 1, min_threshold: 1, rot_delay_daa: 1, emerg_after_daa: 1, emerg_delay_daa: 1 };
    let op = OracleParams { reg_cov: h, max_rate: 1, rate_step: 1, rate_gap_daa: 1, freeze_after_daa: 1 };
    let os = OracleState { kas_usd: 1, oracle_daa: 1, seq: 1, stable_rate: 0, stable_index: 1, frozen: false, last_rate_daa: 1 };
    let otpl = Template::of(&oracle(&op, &os));
    let vp = VaultParams {
        oracle_cov: h,
        oracle_tpl: otpl.clone(),
        ghost_cov: h,
        ghost_tpl: ghost_template(),
        mcr_bps: 20_000,
        liq_bps: 15_000,
        bonus_bps: 1_000,
        max_debt: 1,
        interest_spk: spk_bytes(&kaspa_lending_protocol::ops::p2pk_spk(&k)),
    };
    let pp = PoolParams { ghost_cov: h, tpl: ghost_template(), fee_bps: 30, creator: k.clone(), band: Some(PoolBand { oracle_cov: h, oracle_tpl: otpl, band_bps: 300, stop_when_frozen: true }) };
    let tp = TresorParams { owner: k.clone(), recipient: k.clone(), amount: 1, anchor_day: 1, period_ms: 0, max_fee: 1, payload_hash: vec![0; 32] };
    let all = [
        ("Register", register(&rp, &RegisterState::genesis(&set, None, 0, [0; 32], 1))),
        ("Orakel v4", oracle(&op, &os)),
        ("Vault v4", vault(&vp, &k, &VaultState::default())),
        ("Factory", factory(&FactoryParams { deployer: k.clone(), ghost_tpl: ghost_template() }, &FactoryState::uninitialized())),
        ("GHOST-Token", GhostTok::minter_of(&h).artifact()),
        ("Pool v4", pool_artifact(&pp, &h, true)),
        ("Tresor", standing_order(&tp, &TresorState { next_due: 1, left: 1 })),
    ];
    let mut bad = vec![];
    for (name, a) in &all {
        let n = sig_ops(a);
        println!("{name:<12} {n:>3} Signaturprüfungen (statisch)");
        if n > MAX_STANDARD_P2SH_SIG_OPS {
            bad.push(format!("{name}: {n}"));
        }
    }
    assert!(bad.is_empty(), "über {MAX_STANDARD_P2SH_SIG_OPS}: {bad:?}");
}
