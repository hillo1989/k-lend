//! Version 5 (docs/v5-entwurf.md): Transaktionen für Register v5 und Orakel
//! v5 – Deployment, Preis-Update, Wächter-Sperre, Ankündigung, Absage,
//! Aktivierung, Einfrieren. Noch ohne Anbindung an ghostctl, Agent und Seite
//! (folgt nach Freigabe des Entwurfs); genutzt von tests/v5_sim_tests.rs.
//!
//! Die relative Sperre des Registers (Mindestabstand minGapDaa, Ticket-Alter)
//! setzt jede Funktion als Sequenz des Register-Eingangs (txb::build_ext).
//! Ob sie im Konsens schon erfüllt ist, prüft der Simulator bzw. der Node.

use crate::contracts::*;
use crate::ops::{Funds, Tracked, cov_in, cov_out, genesis_output, track, xonly};
use crate::txb::{Built, Draft, In, Unlock, build, build_ext};
use kaspa_consensus_core::config::params::Params;
use secp256k1::{Keypair, Message};
use serde::{Deserialize, Serialize};
use silverscript_abi::ArtifactValue;

/// Register und Orakel v5
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Feed5 {
    pub register_params: RegisterV5Params,
    pub register: Tracked<RegisterV5State>,
    pub signer_set: SignerSet,
    pub fallback_set: Option<SignerSet>,
    pub guards: Option<GuardSet>,
    pub oracle_params: OracleV5Params,
    pub oracle: Tracked<OracleV5State>,
}

/// Grenzen der Orakel-Parameter (Template-Konstanten, nicht nachschärfbar)
pub fn check_oracle_params(p: &OracleV5Params) -> Result<(), String> {
    if !(1..=10_000).contains(&p.jump_bps) {
        return Err(format!("jump_bps {} außerhalb 1..=10 000", p.jump_bps));
    }
    if p.ref_after < 1 {
        return Err("ref_after muss ≥ 1 sein".into());
    }
    Ok(())
}

/// Register-Genesis: genau ein Ausgang mit der neuen ID (wie v4)
pub fn deploy_register(p: &RegisterV5Params, s: RegisterV5State, value: u64, fund: &Funds, net: &Params) -> Result<(Built, Tracked<RegisterV5State>), String> {
    let first = fund.utxos.first().ok_or("keine Funding-UTXO")?.0;
    let (out, id) = genesis_output(&register_v5(p, &s), value, 0, first, 0);
    let b = build(Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: fund.change(), lock_time: 0 }, net)?;
    crate::ops::check_register_genesis(&b.tx, &id)?;
    Ok((b.clone(), track(&b, 0, id, s)))
}

pub fn deploy_oracle(p: &OracleV5Params, s: OracleV5State, value: u64, fund: &Funds, net: &Params) -> Result<(Built, Tracked<OracleV5State>), String> {
    check_oracle_params(p)?;
    let first = fund.utxos.first().ok_or("keine Funding-UTXO")?.0;
    let (out, id) = genesis_output(&oracle_v5(p, &s), value, 0, first, 0);
    let b = build(Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: fund.change(), lock_time: 0 }, net)?;
    Ok((b.clone(), track(&b, 0, id, s)))
}

fn reg_bounds(rp: &RegisterV5Params) -> RegisterParams {
    RegisterParams {
        deployer: rp.deployer.clone(),
        min_signers: rp.min_signers,
        min_threshold: rp.min_threshold,
        rot_delay_daa: rp.rot_delay_daa,
        emerg_after_daa: rp.emerg_after_daa,
        emerg_delay_daa: rp.emerg_delay_daa,
    }
}

/// Register-init (Deployer): Orakel-ID und -Template festlegen
#[allow(clippy::too_many_arguments)]
pub fn init_register(
    rp: &RegisterV5Params,
    reg: &Tracked<RegisterV5State>,
    set: &SignerSet,
    deployer: &Keypair,
    op: &OracleV5Params,
    oracle_t: &Tracked<OracleV5State>,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Tracked<RegisterV5State>), String> {
    set.check_bounds(&reg_bounds(rp))?;
    if set.hash().as_slice() != reg.state.set_hash.as_slice() {
        return Err("Startsatz passt nicht zum Hash im Register".into());
    }
    let otpl = Template::of(&oracle_v5(op, &oracle_t.state));
    let next = RegisterV5State {
        oracle_cov: oracle_t.cov.as_bytes().to_vec(),
        oracle_tpl: otpl.hash.clone(),
        oracle_pre: otpl.prefix.len() as i64,
        oracle_suf: otpl.suffix.len() as i64,
        initialized: true,
        ..reg.state.clone()
    };
    let cur = register_v5(rp, &reg.state);
    let mut args = set.args();
    args.extend([
        ArtifactValue::Bytes(oracle_t.cov.as_bytes().to_vec()),
        ArtifactValue::Bytes(otpl.hash),
        ArtifactValue::Int(otpl.prefix.len() as i64),
        ArtifactValue::Int(otpl.suffix.len() as i64),
    ]);
    let mut inputs = vec![In { outpoint: reg.outpoint, entry: cov_in(&reg.outpoint, reg.value, &cur, reg.cov), unlock: Unlock::Entry { art: cur, entry: "init", args, sig_at: Some((8, deployer.into())) } }];
    inputs.extend(fund.inputs());
    let b = build(Draft { inputs, outputs: vec![cov_out(&register_v5(rp, &next), reg.value, 0, reg.cov)], change_spk: fund.change(), lock_time: 0 }, net)?;
    Ok((b.clone(), track(&b, 0, reg.cov, next)))
}

fn register_in(f: &Feed5, entry: &'static str, args: Vec<ArtifactValue>) -> In {
    let r = &f.register;
    let art = register_v5(&f.register_params, &r.state);
    In { outpoint: r.outpoint, entry: cov_in(&r.outpoint, r.value, &art, r.cov), unlock: Unlock::Entry { art, entry, args, sig_at: None } }
}

fn register_out(f: &Feed5, s: &RegisterV5State) -> kaspa_consensus_core::tx::TransactionOutput {
    cov_out(&register_v5(&f.register_params, s), f.register.value, 0, f.register.cov)
}

/// sigs, idx: `need` Signaturen (aufsteigende Indizes) der Schlüssel `signers` über digest
fn quorum(set: &SignerSet, signers: &[(usize, Keypair)], need: i64, digest: [u8; 32]) -> Result<Vec<ArtifactValue>, String> {
    let mut s = signers.to_vec();
    s.sort_by_key(|(i, _)| *i);
    s.dedup_by_key(|(i, _)| *i);
    if (s.len() as i64) < need {
        return Err(format!("{need} Unterschrift(en) nötig, {} vorhanden", s.len()));
    }
    s.truncate(need as usize);
    for (i, k) in &s {
        if set.keys.get(*i).map(|x| x.as_slice()) != Some(xonly(k).as_slice()) {
            return Err(format!("Schlüssel {i} gehört nicht zum Satz"));
        }
    }
    let sigs = s.iter().map(|(_, k)| ArtifactValue::Bytes(k.sign_schnorr(Message::from_digest(digest)).as_ref().to_vec())).collect();
    let idx = s.iter().map(|(i, _)| ArtifactValue::Int(*i as i64)).collect();
    Ok(vec![ArtifactValue::Array(sigs), ArtifactValue::Array(idx)])
}

/// Sequenz des Register-Eingangs für Einträge des Hauptsatzes
fn gap(f: &Feed5) -> u64 {
    f.register_params.min_gap_daa as u64
}

/// Preis-Update: attestPrice (Mindestalter) + Orakel update
pub fn attest(f: &Feed5, signers: &[(usize, Keypair)], kas_usd: i64, rate: i64, new_daa: u64, fund: &Funds, net: &Params) -> Result<(Built, Feed5), String> {
    let (r, o) = (&f.register, &f.oracle);
    if r.state.locked {
        return Err("Register gesperrt (Wächter): keine Preis-Updates bis zum Notfall-Austausch".into());
    }
    if !jump_ok(o.state.kas_usd, kas_usd, f.oracle_params.jump_bps) {
        return Err("Preissprung über der Grenze".into());
    }
    let next = oracle_v5_next_state(&f.oracle_params, &o.state, kas_usd, new_daa as i64, rate).ok_or("Index-Überlauf")?;
    let digest = oracle_digest(&o.cov, kas_usd, next.oracle_daa, next.seq, rate);
    let mut args: Vec<ArtifactValue> = [next.kas_usd, next.oracle_daa, next.seq, next.stable_rate, next.stable_index, next.last_rate_daa, next.ref_kas_usd, next.cand_kas_usd, next.cand_seq]
        .into_iter()
        .map(ArtifactValue::Int)
        .collect();
    args.extend(f.signer_set.args());
    args.extend(quorum(&f.signer_set, signers, f.signer_set.t, digest)?);
    let reg_next = RegisterV5State { nonce: if r.state.emerg { r.state.nonce + 1 } else { r.state.nonce }, emerg: false, last_daa: next.oracle_daa, ..r.state.clone() };
    let cur = oracle_v5(&f.oracle_params, &o.state);
    let mut inputs = vec![
        register_in(f, "attestPrice", args),
        In {
            outpoint: o.outpoint,
            entry: cov_in(&o.outpoint, o.value, &cur, o.cov),
            unlock: Unlock::Entry { art: cur, entry: "update", args: vec![ArtifactValue::Int(kas_usd), ArtifactValue::Int(new_daa as i64), ArtifactValue::Int(rate)], sig_at: None },
        },
    ];
    inputs.extend(fund.inputs());
    let outputs = vec![register_out(f, &reg_next), cov_out(&oracle_v5(&f.oracle_params, &next), o.value, 1, o.cov)];
    let b = build_ext(Draft { inputs, outputs, change_spk: fund.change(), lock_time: new_daa }, &[], &[gap(f)], net)?;
    let mut n = f.clone();
    n.register = track(&b, 0, r.cov, reg_next);
    n.oracle = track(&b, 1, o.cov, next);
    Ok((b, n))
}

/// Sperre durch Wächter `who` (kein Mindestalter, kein Orakel)
pub fn guard_lock(f: &Feed5, who: usize, key: &Keypair, fund: &Funds, net: &Params) -> Result<(Built, Feed5), String> {
    let g = f.guards.as_ref().ok_or("keine Wächter festgelegt")?;
    if g.keys.get(who).map(|x| x.as_slice()) != Some(xonly(key).as_slice()) {
        return Err(format!("Schlüssel ist nicht Wächter {who}"));
    }
    let r = &f.register;
    if r.state.locked {
        return Err("schon gesperrt".into());
    }
    let d = lock_digest(&r.cov, r.state.nonce + 1);
    let mut args = g.args();
    args.push(ArtifactValue::Int(who as i64));
    args.push(ArtifactValue::Bytes(key.sign_schnorr(Message::from_digest(d)).as_ref().to_vec()));
    let next = RegisterV5State { nonce: r.state.nonce + 1, emerg: false, locked: true, ..r.state.clone() };
    let mut inputs = vec![register_in(f, "guardLock", args)];
    inputs.extend(fund.inputs());
    let b = build(Draft { inputs, outputs: vec![register_out(f, &next)], change_spk: fund.change(), lock_time: 0 }, net)?;
    let mut n = f.clone();
    n.register = track(&b, 0, r.cov, next);
    Ok((b, n))
}

/// Ankündigung; `emergency` = durch den Notfallsatz (bei Stille oder Sperre).
/// Rückgabe: Tx, neuer Stand, Ticket
#[allow(clippy::too_many_arguments)]
pub fn propose(
    f: &Feed5,
    emergency: bool,
    signers: &[(usize, Keypair)],
    new_set: &SignerSet,
    new_fb: Option<&SignerSet>,
    new_guards: Option<&GuardSet>,
    lock_time: u64,
    fund: &Funds,
    net: &Params,
) -> Result<(Built, Feed5, Tracked<RegisterV5State>), String> {
    let r = &f.register;
    new_set.check_bounds(&reg_bounds(&f.register_params))?;
    let auth = if emergency { f.fallback_set.clone().ok_or("kein Notfallsatz")? } else { f.signer_set.clone() };
    let set_h = new_set.hash();
    let fb_h = new_fb.map(|s| s.hash()).unwrap_or([0; 32]);
    let g_h = new_guards.map(|g| g.hash()).unwrap_or([0; 32]);
    let ann = announcement_v5(&set_h, &fb_h, &g_h);
    let nonce = r.state.nonce + 1;
    let digest = rotate_digest(&r.cov, emergency, nonce, &ann);
    let mut args = new_set.args();
    args.extend([ArtifactValue::Bytes(fb_h.to_vec()), ArtifactValue::Bytes(g_h.to_vec()), ArtifactValue::Bool(emergency)]);
    args.extend(auth.args());
    args.extend(quorum(&auth, signers, auth.t_rot, digest)?);
    let main = RegisterV5State { nonce, emerg: emergency, ..r.state.clone() };
    let ticket = RegisterV5State { ticket: true, set_hash: set_h.to_vec(), fb_hash: fb_h.to_vec(), guard_hash: g_h.to_vec(), nonce, emerg: emergency, locked: false, ..r.state.clone() };
    let mut inputs = vec![register_in(f, "propose", args)];
    inputs.extend(fund.inputs());
    let outputs = vec![register_out(f, &main), cov_out(&register_v5(&f.register_params, &ticket), crate::ops::TICKET_VALUE, 0, r.cov)];
    let b = build_ext(Draft { inputs, outputs, change_spk: fund.change(), lock_time }, &ann, &[gap(f)], net)?;
    let mut n = f.clone();
    n.register = track(&b, 0, r.cov, main);
    Ok((b.clone(), n, track(&b, 1, r.cov, ticket)))
}

/// Absage (tRot des Hauptsatzes)
pub fn cancel(f: &Feed5, signers: &[(usize, Keypair)], fund: &Funds, net: &Params) -> Result<(Built, Feed5), String> {
    let r = &f.register;
    let digest = cancel_digest(&r.cov, r.state.nonce + 1);
    let mut args = f.signer_set.args();
    args.extend(quorum(&f.signer_set, signers, f.signer_set.t_rot, digest)?);
    let next = RegisterV5State { nonce: r.state.nonce + 1, emerg: false, ..r.state.clone() };
    let mut inputs = vec![register_in(f, "cancel", args)];
    inputs.extend(fund.inputs());
    let b = build_ext(Draft { inputs, outputs: vec![register_out(f, &next)], change_spk: fund.change(), lock_time: 0 }, &[], &[gap(f)], net)?;
    let mut n = f.clone();
    n.register = track(&b, 0, r.cov, next);
    Ok((b, n))
}

/// Aktivierung eines reifen Tickets; hebt eine Sperre auf
pub fn activate(f: &Feed5, ticket: &Tracked<RegisterV5State>, new_set: &SignerSet, new_fb: Option<&SignerSet>, new_guards: Option<&GuardSet>, fund: &Funds, net: &Params) -> Result<(Built, Feed5), String> {
    let r = &f.register;
    let tk = &ticket.state;
    if tk.nonce != r.state.nonce {
        return Err("Ticket abgesagt oder überholt".into());
    }
    let delay = if tk.emerg { f.register_params.emerg_delay_daa } else { f.register_params.rot_delay_daa };
    let next = RegisterV5State { set_hash: tk.set_hash.clone(), fb_hash: tk.fb_hash.clone(), guard_hash: tk.guard_hash.clone(), nonce: r.state.nonce + 1, emerg: false, locked: false, ..r.state.clone() };
    let tart = register_v5(&f.register_params, tk);
    let mut inputs = vec![
        register_in(f, "activate", vec![ArtifactValue::Int(1)]),
        In { outpoint: ticket.outpoint, entry: cov_in(&ticket.outpoint, ticket.value, &tart, ticket.cov), unlock: Unlock::Entry { art: tart, entry: "settle", args: vec![ArtifactValue::Int(0)], sig_at: None } },
    ];
    inputs.extend(fund.inputs());
    let b = build_ext(Draft { inputs, outputs: vec![register_out(f, &next)], change_spk: fund.change(), lock_time: 0 }, &[], &[0, delay as u64], net)?;
    let mut n = f.clone();
    n.register = track(&b, 0, r.cov, next);
    n.signer_set = new_set.clone();
    n.fallback_set = new_fb.cloned();
    n.guards = new_guards.cloned();
    Ok((b, n))
}

/// Einfrieren (jeder, ab oracleDaa + freezeAfterDaa)
pub fn freeze(f: &Feed5, daa: u64, fund: &Funds, net: &Params) -> Result<(Built, Feed5), String> {
    let o = &f.oracle;
    let next = OracleV5State { frozen: true, ..o.state };
    let cur = oracle_v5(&f.oracle_params, &o.state);
    let mut inputs = vec![In { outpoint: o.outpoint, entry: cov_in(&o.outpoint, o.value, &cur, o.cov), unlock: Unlock::Entry { art: cur, entry: "freeze", args: vec![], sig_at: None } }];
    inputs.extend(fund.inputs());
    let b = build(Draft { inputs, outputs: vec![cov_out(&oracle_v5(&f.oracle_params, &next), o.value, 0, o.cov)], change_spk: fund.change(), lock_time: daa }, net)?;
    let mut n = f.clone();
    n.oracle = track(&b, 0, o.cov, next);
    Ok((b, n))
}

/// Register-Genesis, Orakel-Genesis, Register-init (drei Tx) im Simulator
#[allow(clippy::too_many_arguments)]
pub fn deploy_feed(
    sim: &mut crate::sim::Sim,
    deployer: &Keypair,
    rp: RegisterV5Params,
    set: SignerSet,
    fallback: Option<SignerSet>,
    guards: Option<GuardSet>,
    op_template: OracleV5Params,
    kas_usd: i64,
    rate: i64,
    value: u64,
) -> Result<(Feed5, Vec<Built>), String> {
    let net = sim.params.clone();
    let rs = RegisterV5State::genesis(&set, fallback.as_ref().map(|s| s.hash()), guards.as_ref().map(|g| g.hash()), sim.daa as i64);
    let (b1, reg) = deploy_register(&rp, rs, value, &sim.funds(deployer), &net)?;
    sim.submit(&b1)?;
    let op = OracleV5Params { reg_cov: reg.cov, ..op_template };
    let os = OracleV5State::genesis(kas_usd, sim.daa as i64, rate);
    let (b2, oracle) = deploy_oracle(&op, os, value, &sim.funds(deployer), &net)?;
    sim.submit(&b2)?;
    let (b3, reg) = init_register(&rp, &reg, &set, deployer, &op, &oracle, &sim.funds(deployer), &net)?;
    sim.submit(&b3)?;
    Ok((Feed5 { register_params: rp, register: reg, signer_set: set, fallback_set: fallback, guards, oracle_params: op, oracle }, vec![b1, b2, b3]))
}
