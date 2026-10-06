//! Offener Tauschpool KAS/GHOST mit Anteils-Token (contracts/ghost_pool.sil):
//! Vorlage, exakte Rechnung, Transaktionen und das Auffinden des aktuellen
//! Stands.
//!
//! Die Pool-UTXO hat nach `init` eine feste Adresse, jeder findet sie über den
//! Node. GHOST-Reserve und Anteils-Minter sind KCC20-Token, deren Skript den
//! Betrag enthält. Beide entstehen in jeder Pool-Tx neu, stammen also aus
//! derselben Tx wie die Pool-UTXO. Nach Aktionen Dritter liest `resync` die
//! Beträge aus dieser Tx (REST-API) und bestätigt jeden Treffer am Node.

use crate::contracts::*;
use crate::net::Net;
use crate::ops::{Deployment, Funds, TOKEN_VALUE, Tracked, cov_in, cov_out, genesis_output, track};
use crate::txb::{Built, Draft, In, Signer, Unlock, build};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint, TransactionOutput};
use serde::{Deserialize, Serialize};
use silverscript_abi::ArtifactValue;
use std::time::Duration;

/// Version 4: Orakel-Layout v4, stopWhenFrozen
pub const POOL_SRC: &str = include_str!("../../contracts/ghost_pool_v4.sil");
/// Pools ohne Kursband (angelegt vor dem 29.09.2026) laufen mit diesem Vertrag weiter
pub const POOL_V1_SRC: &str = include_str!("../../contracts/ghost_pool_v1.sil");
/// Kursband neuer Pools: GHOST-Preis 1 USD ± 3 %
pub const POOL_BAND_BPS: i64 = 300;
pub const POOL_FEE_BPS: i64 = 30; // 0,3 %
/// Der Pool behält immer mindestens 1 KAS (Vertrag: MIN_KAS). So viel geht
/// bei init als gesperrte Mindestliquidität hinein.
pub const POOL_MIN_KAS: u64 = 100_000_000;
/// KAS-Wert der Anteils-Minter-UTXO (wird bei jeder Pool-Tx neu erzeugt)
pub const MINTER_VALUE: u64 = 100_000_000;
const MAX_RESERVE: i64 = 10_000_000_000_000_000; // Vertrag: MAX_KAS / MAX_GHOST
const MAX_SHARES: i64 = (1 << 60) - 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoolParams {
    pub ghost_cov: Hash,
    /// KCC20-Vorlage (GHOST und Anteile)
    pub tpl: Template,
    pub fee_bps: i64,
    #[serde(with = "hex_bytes")]
    pub creator: Vec<u8>,
    /// Kursband (Version 2). None = Pool der Version 1 ohne Band.
    #[serde(default)]
    pub band: Option<PoolBand>,
}

/// Kursband: der Pool liest beim Tauschen das Orakel und hält den GHOST-Preis
/// bei 1 USD ± band_bps (contracts/ghost_pool.sil, checkBand)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoolBand {
    pub oracle_cov: Hash,
    pub oracle_tpl: Template,
    pub band_bps: i64,
    /// Version 4: bei eingefrorenem Orakel kein Tausch und kein init
    /// (Entscheidung des Nutzers, 04.10.2026: Pool sperren)
    #[serde(default = "yes")]
    pub stop_when_frozen: bool,
}

fn yes() -> bool {
    true
}

impl PoolBand {
    /// Band aus den Vault-Parametern eines Deployments (gleiches Orakel)
    pub fn of(d: &Deployment, band_bps: i64) -> Option<Self> {
        d.vault_params.as_ref().map(|vp| Self { oracle_cov: vp.oracle_cov, oracle_tpl: vp.oracle_tpl.clone(), band_bps, stop_when_frozen: true })
    }
}

/// Orakel, das eine Pool-Tx mit Kursband per read() mitliest
#[derive(Clone, Debug)]
pub struct OracleRef {
    pub params: OracleParams,
    pub t: Tracked<OracleState>,
}

impl OracleRef {
    pub fn of(d: &Deployment) -> Self {
        Self { params: d.oracle_params.clone(), t: d.oracle.clone() }
    }
    fn read(&self, auth: u16) -> (In, TransactionOutput) {
        let art = oracle(&self.params, &self.t.state);
        (
            In { outpoint: self.t.outpoint, entry: cov_in(&self.t.outpoint, self.t.value, &art, self.t.cov), unlock: Unlock::Entry { art: art.clone(), entry: "read", args: vec![], sig_at: None } },
            cov_out(&art, self.t.value, auth, self.t.cov),
        )
    }
}

/// Pool v4 mit stopWhenFrozen: kein Tausch gegen einen veralteten Preis
fn frozen_stop(b: &PoolBand, o: &OracleRef) -> Result<(), String> {
    if b.stop_when_frozen && o.t.state.frozen {
        return Err("Der Pool ist gesperrt: Das Orakel ist eingefroren (kein aktueller Preis). Einlegen und Abziehen gehen weiter.".into());
    }
    Ok(())
}

/// Genau die Bandregel des Vertrags (checkBand): nachher ≥ 1 − band oder
/// gestiegen, und nachher ≤ 1 + band oder gefallen. Preis = x2/y2 · kas_usd/1e8;
/// gestiegen/gefallen wie im Vertrag an den Reserven abgelesen.
pub fn band_ok(x: i64, y: i64, x2: i64, y2: i64, kas_usd: i64, band_bps: i64) -> bool {
    band_rule(x2, y2, kas_usd, band_bps, x2 >= x && y2 <= y, x2 <= x && y2 >= y)
}

/// Startkurs im Band (init: weder „gestiegen“ noch „gefallen“ zählt)
pub fn band_start_ok(x: i64, y: i64, kas_usd: i64, band_bps: i64) -> bool {
    band_rule(x, y, kas_usd, band_bps, false, false)
}

fn band_rule(x2: i64, y2: i64, kas_usd: i64, band_bps: i64, rises: bool, falls: bool) -> bool {
    if y2 <= 0 || kas_usd <= 0 {
        return false;
    }
    let k = kas_usd as i128 * 10_000;
    let high = 100_000_000i128 * (10_000 + band_bps as i128);
    let low = if band_bps < 10_000 { 100_000_000i128 * (10_000 - band_bps as i128) } else { 0 };
    (rises || x2 as i128 * k >= y2 as i128 * low) && (falls || y2 as i128 * high >= x2 as i128 * k)
}

/// GHOST-Preis des Pools in USD (Anzeige)
pub fn ghost_usd(x: i64, y: i64, kas_usd: i64) -> f64 {
    if y <= 0 { 0.0 } else { x as f64 / y as f64 * kas_usd as f64 / 1e8 }
}

/// Stand des Pools: Pool-UTXO (Wert = KAS-Reserve), GHOST-Reserve, Anteils-Minter
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolRec {
    pub params: PoolParams,
    pub lp_cov: Hash,
    pub pool: Tracked<()>,
    pub reserve: Tracked<GhostTok>,
    pub minter: Tracked<GhostTok>,
}

impl PoolRec {
    pub fn kas(&self) -> i64 {
        self.pool.value as i64
    }
    pub fn ghost(&self) -> i64 {
        self.reserve.state.amount
    }
    pub fn shares(&self) -> i64 {
        self.minter.state.amount
    }
}

/// Angelegter, noch nicht initialisierter Pool (zwischen den beiden Schritten von pool-open)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolPending {
    pub params: PoolParams,
    pub pool: Tracked<()>,
}

pub fn pool_artifact(p: &PoolParams, lp_cov: &Hash, initialized: bool) -> Artifact {
    let mut args = vec![
        ArtifactValue::Bytes(p.ghost_cov.as_bytes().to_vec()),
        ArtifactValue::Int(p.tpl.prefix.len() as i64),
        ArtifactValue::Int(p.tpl.suffix.len() as i64),
        ArtifactValue::Bytes(p.tpl.hash.clone()),
        ArtifactValue::Int(p.fee_bps),
        ArtifactValue::Bytes(p.creator.clone()),
    ];
    let src = match &p.band {
        None => POOL_V1_SRC,
        Some(b) => {
            args.extend([
                ArtifactValue::Bytes(b.oracle_cov.as_bytes().to_vec()),
                ArtifactValue::Int(b.oracle_tpl.prefix.len() as i64),
                ArtifactValue::Int(b.oracle_tpl.suffix.len() as i64),
                ArtifactValue::Bytes(b.oracle_tpl.hash.clone()),
                ArtifactValue::Int(b.band_bps),
                ArtifactValue::Bool(b.stop_when_frozen),
            ]);
            POOL_SRC
        }
    };
    args.extend([ArtifactValue::Bytes(lp_cov.as_bytes().to_vec()), ArtifactValue::Bool(initialized)]);
    compile(src, args)
}

pub fn reserve_tok(pool_cov: &Hash, amount: i64) -> GhostTok {
    GhostTok { owner: pool_cov.as_bytes().to_vec(), typ: ID_COV, amount, minter: false }
}

pub fn minter_tok(pool_cov: &Hash, shares: i64) -> GhostTok {
    GhostTok { owner: pool_cov.as_bytes().to_vec(), typ: ID_COV, amount: shares, minter: true }
}

// ------------------------------------------------------------- Rechnung ----

fn fee_of(d: i64, fee_bps: i64) -> i64 {
    ((d as i128 * fee_bps as i128 + 9_999) / 10_000) as i64
}

/// Genau die Regel des Vertrags (swap): (x'−fee)·(y'−fee) ≥ x·y
pub fn swap_ok(x: i64, y: i64, x2: i64, y2: i64, fee_bps: i64) -> bool {
    if x2 < POOL_MIN_KAS as i64 || y2 <= 0 || x2 > MAX_RESERVE || y2 > MAX_RESERVE {
        return false;
    }
    let xa = if x2 > x { x2 - fee_of(x2 - x, fee_bps) } else { x2 };
    let ya = if y2 > y { y2 - fee_of(y2 - y, fee_bps) } else { y2 };
    xa as i128 * ya as i128 >= x as i128 * y as i128
}

fn max_out(ok: impl Fn(i64) -> bool, hi: i64) -> i64 {
    let (mut lo, mut hi) = (0i64, hi.max(0));
    while lo < hi {
        let m = lo + (hi - lo + 1) / 2;
        if ok(m) { lo = m } else { hi = m - 1 }
    }
    lo
}

/// Reserven nach einem Tausch von `amount` (Kauf: sompi, Verkauf: GHOST-Einheiten)
pub fn after_swap(x: i64, y: i64, fee_bps: i64, buy: bool, amount: i64) -> (i64, i64) {
    if buy {
        (x + amount, y - ghost_out(x, y, amount, fee_bps))
    } else {
        (x - kas_out(x, y, amount, fee_bps), y + amount)
    }
}

/// Größter Einsatz, den das Kursband gerade zulässt (0 = Richtung gesperrt)
pub fn band_max_in(x: i64, y: i64, fee_bps: i64, kas_usd: i64, band_bps: i64, buy: bool) -> i64 {
    let hi = if buy { MAX_RESERVE - x } else { MAX_RESERVE - y };
    max_out(
        |a| {
            let (x2, y2) = after_swap(x, y, fee_bps, buy, a);
            (x2, y2) != (x, y) && band_ok(x, y, x2, y2, kas_usd, band_bps)
        },
        hi,
    )
}

/// Größte GHOST-Menge für `dx` sompi
pub fn ghost_out(x: i64, y: i64, dx: i64, fee_bps: i64) -> i64 {
    if dx <= 0 || y <= 1 {
        return 0;
    }
    max_out(|m| swap_ok(x, y, x + dx, y - m, fee_bps), y - 1)
}

/// Größte KAS-Menge (sompi) für `dy` GHOST-Einheiten
pub fn kas_out(x: i64, y: i64, dy: i64, fee_bps: i64) -> i64 {
    if dy <= 0 || x <= POOL_MIN_KAS as i64 {
        return 0;
    }
    max_out(|m| swap_ok(x, y, x - m, y + dy, fee_bps), x - POOL_MIN_KAS as i64)
}

/// Neue Anteile für eine Einlage (dx, dy): min(⌊S·dx/x⌋, ⌊S·dy/y⌋) (Vertrag: add)
pub fn shares_for_deposit(s: i64, x: i64, y: i64, dx: i64, dy: i64) -> i64 {
    if x <= 0 || y <= 0 || dx < 0 || dy < 0 {
        return 0;
    }
    ((s as i128 * dx as i128 / x as i128).min(s as i128 * dy as i128 / y as i128)) as i64
}

/// Auszahlung für m von S Anteilen: (⌊x·m/S⌋, ⌊y·m/S⌋) (Vertrag: remove)
pub fn payout_for(s: i64, x: i64, y: i64, m: i64) -> (i64, i64) {
    if s <= 0 {
        return (0, 0);
    }
    ((x as i128 * m as i128 / s as i128) as i64, (y as i128 * m as i128 / s as i128) as i64)
}

// ------------------------------------------------------ Transaktionen ----

fn tok_input(t: &Tracked<GhostTok>, unlock: Unlock) -> In {
    let art = t.state.artifact();
    In { outpoint: t.outpoint, entry: cov_in(&t.outpoint, t.value, &art, t.cov), unlock }
}

/// Ergebnis einer Pool-Tx: neuer Pool-Stand, neue eigene Token (GHOST bzw.
/// Anteile) und verbrauchte eigene Token (Outpoints)
pub struct PoolTx {
    pub built: Built,
    pub pool: Option<PoolRec>,
    pub pending: Option<PoolPending>,
    pub new_ghost: Vec<Tracked<GhostTok>>,
    pub new_lp: Vec<Tracked<GhostTok>>,
    pub spent: Vec<TransactionOutpoint>,
    /// neue Orakel-UTXO, wenn die Tx das Orakel mitgelesen hat (Kursband)
    pub oracle: Option<Tracked<OracleState>>,
}

/// Schritt 1 von pool-open: Pool-Genesis (nicht initialisiert) mit 1 KAS
pub fn pool_create(ghost_cov: Hash, band: Option<PoolBand>, creator: impl Into<Signer>, fund: &Funds, net: &Params) -> Result<PoolTx, String> {
    let creator: Signer = creator.into();
    let params = PoolParams { ghost_cov, tpl: ghost_template(), fee_bps: POOL_FEE_BPS, creator: creator.xonly(), band };
    let art = pool_artifact(&params, &Hash::from_bytes([0; 32]), false);
    let first = fund.utxos.first().ok_or("keine Funding-UTXO")?.0;
    let (out, pool_cov) = genesis_output(&art, POOL_MIN_KAS, 0, first, 0);
    let b = build(Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: fund.change(), lock_time: 0 }, net)?;
    let pending = PoolPending { params, pool: track(&b, 0, pool_cov, ()) };
    Ok(PoolTx { built: b, pool: None, pending: Some(pending), new_ghost: vec![], new_lp: vec![], spent: vec![], oracle: None })
}

/// Schritt 2 von pool-open: Anteils-Token anlegen (S0 = 1 KAS in sompi,
/// gesperrt) und `ghost` Einheiten als erste Reserve einlegen. Beides bleibt
/// für immer im Pool.
pub fn pool_init(p: &PoolPending, oracle_ref: Option<&OracleRef>, creator: impl Into<Signer>, my_tokens: &[Tracked<GhostTok>], ghost: i64, fund: &Funds, net: &Params) -> Result<PoolTx, String> {
    let creator: Signer = creator.into();
    if creator.xonly() != p.params.creator {
        return Err("Nur der Gründer darf den Pool initialisieren.".into());
    }
    let have: i64 = my_tokens.iter().map(|t| t.state.amount).sum();
    if ghost <= 0 || my_tokens.is_empty() || my_tokens.len() > GHOST_MAX_INS as usize || have < ghost {
        return Err(format!("zu wenig GHOST in höchstens {GHOST_MAX_INS} Token: {:.8} vorhanden", have as f64 / 1e8));
    }
    let pool_cov = p.pool.cov;
    let x0 = p.pool.value as i64;
    if let Some(b) = &p.params.band {
        let o = oracle_ref.ok_or("Pool mit Kursband braucht das Orakel")?;
        frozen_stop(b, o)?;
        if !band_start_ok(x0, ghost, o.t.state.kas_usd, b.band_bps) {
            return Err(format!(
                "Startkurs {:.4} USD je GHOST liegt nicht im Band 1 USD ± {:.1} % (Orakel {:.6} USD je KAS)",
                ghost_usd(x0, ghost, o.t.state.kas_usd),
                b.band_bps as f64 / 100.0,
                o.t.state.kas_usd as f64 / 1e8
            ));
        }
    }
    let reserve = reserve_tok(&pool_cov, ghost);
    let mut gouts = vec![reserve.clone()];
    if have > ghost {
        gouts.push(GhostTok::to_pubkey(&creator.xonly(), have - ghost));
    }
    // Ausgänge: [0] Pool, [1..] GHOST, dann die Genesis des Anteils-Minters
    let lp_idx = 1 + gouts.len() as u32;
    let minter = minter_tok(&pool_cov, x0);
    let (lp_out, lp_cov) = genesis_output(&minter.artifact(), MINTER_VALUE, 0, p.pool.outpoint, lp_idx);
    let art_in = pool_artifact(&p.params, &Hash::from_bytes([0; 32]), false);
    let art_out = pool_artifact(&p.params, &lp_cov, true);
    let states: Vec<ArtifactValue> = gouts.iter().map(GhostTok::arg).collect();
    // Orakel-Eingang (nur mit Band): nach Pool und eigenen Token, vor dem Funding
    let oracle_idx = 1 + my_tokens.len();
    let mut args = vec![ArtifactValue::Int(lp_idx as i64)];
    if p.params.band.is_some() {
        args.push(ArtifactValue::Int(oracle_idx as i64));
    }
    args.extend([
        ArtifactValue::Bytes(p.params.tpl.prefix.clone()),
        ArtifactValue::Bytes(p.params.tpl.suffix.clone()),
        ArtifactValue::Array(states.clone()),
    ]);
    let sig_pos = args.len();
    let mut inputs = vec![In {
        outpoint: p.pool.outpoint,
        entry: cov_in(&p.pool.outpoint, p.pool.value, &art_in, pool_cov),
        unlock: Unlock::Entry { art: art_in.clone(), entry: "init", args, sig_at: Some((sig_pos, creator)) },
    }];
    for (n, t) in my_tokens.iter().enumerate() {
        let art = t.state.artifact();
        let unlock = if n == 0 { Unlock::Leader { art, new_states: states.clone(), signer: Some(creator) } } else { Unlock::Delegate { art, signer: creator } };
        inputs.push(tok_input(t, unlock));
    }
    let mut oracle_out = None;
    if p.params.band.is_some() {
        let (i, o) = oracle_ref.ok_or("Pool mit Kursband braucht das Orakel")?.read(oracle_idx as u16);
        inputs.push(i);
        oracle_out = Some(o);
    }
    inputs.extend(fund.inputs());
    let mut outputs = vec![cov_out(&art_out, x0 as u64, 0, pool_cov)];
    for o in &gouts {
        outputs.push(cov_out(&o.artifact(), TOKEN_VALUE, 1, p.params.ghost_cov));
    }
    outputs.push(lp_out);
    let oracle_pos = outputs.len() as u32;
    if let Some(o) = oracle_out {
        outputs.push(o);
    }
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let oracle_next = oracle_ref.filter(|_| p.params.band.is_some()).map(|o| track(&b, oracle_pos, o.t.cov, o.t.state));
    let rec = PoolRec {
        params: p.params.clone(),
        lp_cov,
        pool: track(&b, 0, pool_cov, ()),
        reserve: track(&b, 1, p.params.ghost_cov, reserve),
        minter: track(&b, lp_idx, lp_cov, minter),
    };
    let new_ghost = if gouts.len() > 1 { vec![track(&b, 2, p.params.ghost_cov, gouts[1].clone())] } else { vec![] };
    Ok(PoolTx { built: b, pool: Some(rec), pending: None, new_ghost, new_lp: vec![], spent: my_tokens.iter().map(|t| t.outpoint).collect(), oracle: oracle_next })
}

/// Gemeinsamer Bau für swap/add/remove.
/// Eingänge: [0] Pool, [1] Reserve (GHOST-Leader), [2] Minter (Anteils-Leader),
///           eigene GHOST (Delegates), eigene Anteile (Delegates), Funding.
/// Ausgänge: [0] Pool, GHOST (Reserve + eigener), Anteile (Minter + eigene).
#[allow(clippy::too_many_arguments)]
fn pool_tx(
    rec: &PoolRec,
    entry: &'static str,
    who: Signer,
    my_ghost: &[Tracked<GhostTok>],
    my_lp: &[Tracked<GhostTok>],
    new_kas: i64,
    new_ghost: i64,
    my_ghost_out: i64,
    new_shares: i64,
    my_lp_out: i64,
    oracle_ref: Option<&OracleRef>,
    fund: &Funds,
    net: &Params,
) -> Result<PoolTx, String> {
    if my_ghost.len() + 1 > GHOST_MAX_INS as usize || my_lp.len() + 1 > GHOST_MAX_INS as usize {
        return Err(format!("höchstens {} eigene Token-UTXOs je Seite", GHOST_MAX_INS - 1));
    }
    let pool_cov = rec.pool.cov;
    let art = pool_artifact(&rec.params, &rec.lp_cov, true);
    let reserve = reserve_tok(&pool_cov, new_ghost);
    let mut gouts = vec![reserve.clone()];
    if my_ghost_out > 0 {
        gouts.push(GhostTok::to_pubkey(&who.xonly(), my_ghost_out));
    }
    let minter = minter_tok(&pool_cov, new_shares);
    let mut louts = vec![minter.clone()];
    if my_lp_out > 0 {
        louts.push(GhostTok::to_pubkey(&who.xonly(), my_lp_out));
    }
    let gstates: Vec<ArtifactValue> = gouts.iter().map(GhostTok::arg).collect();
    let lstates: Vec<ArtifactValue> = louts.iter().map(GhostTok::arg).collect();
    // Kursband: swap liest das Orakel (Eingang nach den eigenen Token, vor dem Funding)
    let with_oracle = entry == "swap" && rec.params.band.is_some();
    let oracle_idx = 3 + my_ghost.len() + my_lp.len();
    let mut args = vec![];
    if with_oracle {
        args.push(ArtifactValue::Int(oracle_idx as i64));
    }
    args.extend([ArtifactValue::Array(gstates.clone()), ArtifactValue::Array(lstates.clone())]);
    let mut inputs = vec![
        In {
            outpoint: rec.pool.outpoint,
            entry: cov_in(&rec.pool.outpoint, rec.pool.value, &art, pool_cov),
            unlock: Unlock::Entry { art: art.clone(), entry, args, sig_at: None },
        },
        tok_input(&rec.reserve, Unlock::Leader { art: rec.reserve.state.artifact(), new_states: gstates, signer: None }),
        tok_input(&rec.minter, Unlock::Leader { art: rec.minter.state.artifact(), new_states: lstates, signer: None }),
    ];
    for t in my_ghost.iter().chain(my_lp.iter()) {
        inputs.push(tok_input(t, Unlock::Delegate { art: t.state.artifact(), signer: who }));
    }
    let mut oracle_out = None;
    if with_oracle {
        let (i, o) = oracle_ref.ok_or("Pool mit Kursband braucht das Orakel")?.read(oracle_idx as u16);
        inputs.push(i);
        oracle_out = Some(o);
    }
    inputs.extend(fund.inputs());
    let mut outputs: Vec<TransactionOutput> = vec![cov_out(&art, new_kas as u64, 0, pool_cov)];
    for (j, o) in gouts.iter().enumerate() {
        outputs.push(cov_out(&o.artifact(), if j == 0 { rec.reserve.value } else { TOKEN_VALUE }, 1, rec.params.ghost_cov));
    }
    let lp_first = outputs.len() as u32;
    for (j, o) in louts.iter().enumerate() {
        outputs.push(cov_out(&o.artifact(), if j == 0 { rec.minter.value } else { TOKEN_VALUE }, 2, rec.lp_cov));
    }
    let oracle_pos = outputs.len() as u32;
    if let Some(o) = oracle_out {
        outputs.push(o);
    }
    let b = build(Draft { inputs, outputs, change_spk: fund.change(), lock_time: 0 }, net)?;
    let oracle_next = if with_oracle { oracle_ref.map(|o| track(&b, oracle_pos, o.t.cov, o.t.state)) } else { None };
    let next = PoolRec {
        params: rec.params.clone(),
        lp_cov: rec.lp_cov,
        pool: track(&b, 0, pool_cov, ()),
        reserve: track(&b, 1, rec.params.ghost_cov, reserve),
        minter: track(&b, lp_first, rec.lp_cov, minter),
    };
    let new_ghost_t = gouts.iter().enumerate().skip(1).map(|(j, o)| track(&b, 1 + j as u32, rec.params.ghost_cov, o.clone())).collect();
    let new_lp_t = louts.iter().enumerate().skip(1).map(|(j, o)| track(&b, lp_first + j as u32, rec.lp_cov, o.clone())).collect();
    let spent = my_ghost.iter().chain(my_lp.iter()).map(|t| t.outpoint).collect();
    Ok(PoolTx { built: b, pool: Some(next), pending: None, new_ghost: new_ghost_t, new_lp: new_lp_t, spent, oracle: oracle_next })
}

/// Richtung eines Tauschs
#[derive(Clone, Copy, Debug)]
pub enum Swap {
    /// `kas` sompi gegen mindestens `min_ghost` Einheiten
    Buy { kas: i64, min_ghost: i64 },
    /// `ghost` Einheiten gegen mindestens `min_kas` sompi
    Sell { ghost: i64, min_kas: i64 },
}

pub fn swap(rec: &PoolRec, oracle_ref: Option<&OracleRef>, trader: impl Into<Signer>, my_tokens: &[Tracked<GhostTok>], s: Swap, fund: &Funds, net: &Params) -> Result<(PoolTx, i64), String> {
    let (x, y, f, sh) = (rec.kas(), rec.ghost(), rec.params.fee_bps, rec.shares());
    // Kursband vorab prüfen, damit der Nutzer eine klare Meldung samt
    // Höchstbetrag bekommt statt eines abgelehnten Skripts
    if let Some(b) = &rec.params.band {
        let o = oracle_ref.ok_or("Pool mit Kursband braucht das Orakel")?;
        frozen_stop(b, o)?;
        let k = o.t.state.kas_usd;
        let buy = matches!(s, Swap::Buy { .. });
        let amount = match s {
            Swap::Buy { kas, .. } => kas,
            Swap::Sell { ghost, .. } => ghost,
        };
        let (x2, y2) = after_swap(x, y, f, buy, amount);
        if !band_ok(x, y, x2, y2, k, b.band_bps) {
            let max = band_max_in(x, y, f, k, b.band_bps, buy);
            let unit = if buy { "KAS" } else { "GHOST" };
            return Err(if max > 0 {
                format!(
                    "Kursband: Der Tausch würde GHOST auf {:.4} USD schieben (erlaubt 1 USD ± {:.1} %). Höchstens {:.8} {unit} sind gerade möglich.",
                    ghost_usd(x2, y2, k),
                    b.band_bps as f64 / 100.0,
                    max as f64 / 1e8
                )
            } else {
                format!(
                    "Kursband: In diese Richtung ist der Pool gerade gesperrt (GHOST {:.4} USD, erlaubt 1 USD ± {:.1} %). Er öffnet wieder, wenn jemand in die Gegenrichtung tauscht oder sich der KAS-Preis bewegt.",
                    ghost_usd(x, y, k),
                    b.band_bps as f64 / 100.0
                )
            });
        }
    }
    match s {
        Swap::Buy { kas, min_ghost } => {
            let out = ghost_out(x, y, kas, f);
            if out <= 0 {
                return Err("Betrag zu klein für einen Tausch".into());
            }
            if out < min_ghost {
                return Err(format!(
                    "Der Kurs hat sich verschoben: nur {:.8} GHOST statt mindestens {:.8}. Bitte neu prüfen.",
                    out as f64 / 1e8,
                    min_ghost as f64 / 1e8
                ));
            }
            let t = pool_tx(rec, "swap", trader.into(), &[], &[], x + kas, y - out, out, sh, 0, oracle_ref, fund, net)?;
            Ok((t, out))
        }
        Swap::Sell { ghost, min_kas } => {
            let have: i64 = my_tokens.iter().map(|t| t.state.amount).sum();
            if my_tokens.is_empty() || have < ghost {
                return Err(format!("zu wenig GHOST: {:.8} vorhanden", have as f64 / 1e8));
            }
            let out = kas_out(x, y, ghost, f);
            if out <= 0 {
                return Err("Betrag zu klein für einen Tausch".into());
            }
            if out < min_kas {
                return Err(format!(
                    "Der Kurs hat sich verschoben: nur {:.8} KAS statt mindestens {:.8}. Bitte neu prüfen.",
                    out as f64 / 1e8,
                    min_kas as f64 / 1e8
                ));
            }
            let t = pool_tx(rec, "swap", trader.into(), my_tokens, &[], x - out, y + ghost, have - ghost, sh, 0, oracle_ref, fund, net)?;
            Ok((t, out))
        }
    }
}

/// Kursband beim Einlegen in bps: so viel darf das Verhältnis des Pools vom
/// eingegebenen Verhältnis abweichen (wie amountAMin/amountBMin bei Uniswap)
pub const ADD_TOL_BPS: i64 = 100;

/// Einlegen: höchstens `kas` sompi und `ghost` Einheiten für mindestens
/// `min_shares` Anteile. Genommen wird nur, was zu den Anteilen passt
/// (⌈m·x/S⌉, ⌈m·y/S⌉); weicht der Poolkurs um mehr als `tol_bps` vom
/// eingegebenen Verhältnis ab, bricht es ab (Audit 10, A10-P-1: vorher wurde
/// der Überschuss verschenkt, und Einlegen zu einem verschobenen Kurs verlor
/// danach über die Arbitrage). Rückgabe: Tx, neue Anteile, (KAS, GHOST) genommen.
pub fn add(rec: &PoolRec, who: impl Into<Signer>, my_tokens: &[Tracked<GhostTok>], kas: i64, ghost: i64, min_shares: i64, tol_bps: i64, fund: &Funds, net: &Params) -> Result<(PoolTx, i64, (i64, i64)), String> {
    let (x, y, sh) = (rec.kas(), rec.ghost(), rec.shares());
    let have: i64 = my_tokens.iter().map(|t| t.state.amount).sum();
    if kas <= 0 || ghost <= 0 {
        return Err("Einlegen braucht KAS und GHOST".into());
    }
    if my_tokens.is_empty() || have < ghost {
        return Err(format!("zu wenig GHOST: {:.8} vorhanden", have as f64 / 1e8));
    }
    let m = shares_for_deposit(sh, x, y, kas, ghost);
    if m <= 0 {
        return Err("Einlage zu klein für einen Anteil".into());
    }
    if m < min_shares.max(1) {
        return Err("Der Pool hat sich verschoben: weniger Anteile als erwartet. Bitte neu prüfen.".into());
    }
    if sh as i128 + m as i128 > MAX_SHARES as i128 {
        return Err("Anteils-Obergrenze des Vertrags erreicht".into());
    }
    // Vertrag: m·x ≤ S·Δx und m·y ≤ S·Δy → kleinste passende Beträge; da
    // m ≤ S·kas/x und m ≤ S·ghost/y, liegen sie nie über der Eingabe
    let up = |a: i64| ((m as i128 * a as i128 + sh as i128 - 1) / sh as i128) as i64;
    let (dx, dy) = (up(x), up(y));
    let low = |got: i64, want: i64| (got as i128) * 10_000 < want as i128 * (10_000 - tol_bps.clamp(0, 10_000)) as i128;
    if low(dx, kas) || low(dy, ghost) {
        return Err(format!(
            "Der Kurs des Pools weicht um mehr als {:.1} % von deinem Verhältnis ab: passend wären {:.8} KAS zu {:.8} GHOST. Bitte neu prüfen.",
            tol_bps as f64 / 100.0,
            dx as f64 / 1e8,
            dy as f64 / 1e8
        ));
    }
    if x + dx > MAX_RESERVE || y + dy > MAX_RESERVE {
        return Err("Reserve-Obergrenze des Vertrags erreicht".into());
    }
    let t = pool_tx(rec, "add", who.into(), my_tokens, &[], x + dx, y + dy, have - dy, sh + m, m, None, fund, net)?;
    Ok((t, m, (dx, dy)))
}

/// Abziehen: `m` eigene Anteile verbrennen, dafür den Anteil m/S beider Reserven.
/// Rückgabe: Tx und (KAS, GHOST) der Auszahlung.
pub fn remove(rec: &PoolRec, who: impl Into<Signer>, my_lp: &[Tracked<GhostTok>], m: i64, min_kas: i64, min_ghost: i64, fund: &Funds, net: &Params) -> Result<(PoolTx, (i64, i64)), String> {
    let (x, y, sh) = (rec.kas(), rec.ghost(), rec.shares());
    let have: i64 = my_lp.iter().map(|t| t.state.amount).sum();
    if m <= 0 || my_lp.is_empty() || have < m {
        return Err(format!("zu wenig Anteile: {have} vorhanden"));
    }
    // KAS-Seite auf die 1-KAS-Grenze kappen: der Vertrag begrenzt jede Seite
    // einzeln, sonst käme der letzte Einleger nach Kursbewegungen nicht ganz
    // heraus (Audit 10, A10-P-4)
    let (dx0, dy) = payout_for(sh, x, y, m);
    let dx = dx0.min(x - POOL_MIN_KAS as i64).max(0);
    if dx == 0 && dy == 0 {
        // sonst verbrennt die Tx Anteile für nichts (A10-P-5)
        return Err("Zu wenige Anteile für eine Auszahlung".into());
    }
    if dy >= y {
        return Err("Die GHOST-Reserve darf nicht leer werden".into());
    }
    if dx < min_kas || dy < min_ghost {
        return Err("Der Pool hat sich verschoben: weniger Auszahlung als erwartet. Bitte neu prüfen.".into());
    }
    let t = pool_tx(rec, "remove", who.into(), &[], my_lp, x - dx, y - dy, dy, sh - m, have - m, None, fund, net)?;
    Ok((t, (dx, dy)))
}

/// Übernimmt eine Pool-Tx in den Stand: verbrauchte eigene Token weg, neue dazu
pub fn apply(dep: &Deployment, t: &PoolTx) -> Deployment {
    let mut d = dep.clone();
    d.tokens.retain(|x| !t.spent.contains(&x.outpoint));
    d.lp_tokens.retain(|x| !t.spent.contains(&x.outpoint));
    // eigene neue GHOST des Handelnden, mit Obergrenze je Besitzer (A20b-1)
    for g in &t.new_ghost {
        crate::ops::track_token(&mut d, g.clone(), true);
    }
    d.lp_tokens.extend(t.new_lp.iter().cloned());
    if t.pool.is_some() || t.pending.is_none() {
        d.pool = t.pool.clone();
    }
    d.pool_pending = t.pending.clone();
    d.pool_unresolved = None;
    if let Some(o) = &t.oracle {
        d.oracle = o.clone();
    }
    d
}

/// Die `max` größten eigenen Token (Pubkey-Besitz) aus `list`
pub fn own(list: &[Tracked<GhostTok>], owner: &[u8], max: usize) -> Vec<Tracked<GhostTok>> {
    let mut v: Vec<Tracked<GhostTok>> = list.iter().filter(|t| t.state.owner == owner && t.state.typ == ID_PUBKEY && !t.state.minter).cloned().collect();
    v.sort_by_key(|t| std::cmp::Reverse(t.state.amount));
    v.truncate(max);
    v
}

// ---------------------------------------------------- Reserve auffinden ----

/// Basis-URL der öffentlichen REST-API (nur zum Auffinden, Treffer werden
/// über den Node bestätigt)
pub fn rest_base(network: &str) -> &'static str {
    if network == "mainnet" { "https://api.kaspa.org" } else { "https://api-tn10.kaspa.org" }
}

/// Alle Datenpushes eines Skripts, verschachtelte Skripte (Redeem-Skripte)
/// eine Ebene tief mit
fn pushes(script: &[u8], out: &mut Vec<Vec<u8>>, depth: u8) {
    let mut i = 0usize;
    while i < script.len() {
        let op = script[i];
        i += 1;
        let len = match op {
            0x01..=0x4b => op as usize,
            0x4c if i < script.len() => {
                i += 1;
                script[i - 1] as usize
            }
            0x4d if i + 1 < script.len() => {
                i += 2;
                u16::from_le_bytes([script[i - 2], script[i - 1]]) as usize
            }
            0x4e if i + 3 < script.len() => {
                i += 4;
                u32::from_le_bytes([script[i - 4], script[i - 3], script[i - 2], script[i - 1]]) as usize
            }
            0x51..=0x60 => {
                out.push(vec![op - 0x50]);
                continue;
            }
            _ => continue,
        };
        if i + len > script.len() {
            break;
        }
        let data = script[i..i + len].to_vec();
        i += len;
        if depth > 0 && data.len() > 8 {
            pushes(&data, out, depth - 1);
        }
        out.push(data);
    }
}

/// Skriptzahl (little endian, Vorzeichenbit im höchsten Byte), nur positive
fn script_num(b: &[u8]) -> Option<i64> {
    if b.is_empty() || b.len() > 8 {
        return None;
    }
    let mut v: u64 = 0;
    for (n, &byte) in b.iter().enumerate() {
        let byte = if n == b.len() - 1 { byte & 0x7f } else { byte };
        v |= (byte as u64) << (8 * n);
    }
    // bis MAX_SHARES: Anteile können über 1e16 liegen (Audit 10, A10-P-2);
    // jeder Treffer wird ohnehin am Skript und am Node bestätigt
    if b[b.len() - 1] & 0x80 != 0 || v == 0 || v > MAX_SHARES as u64 {
        return None;
    }
    Some(v as i64)
}

/// Kandidaten für Token-Beträge aus den Signaturskripten einer Tx.
/// Die ABI legt Struct-Arrays feldweise ab: alle `amount`-Werte eines Arrays
/// stehen in EINEM Push als Folge von 8-Byte-Zahlen (silverscript-abi
/// `push_struct_array_fields`). Solche Pushes werden zusätzlich im 8-Byte-Raster
/// gelesen (Audit 9 P-1: sonst fehlte der neue Betrag bei jedem Kauf).
pub fn amount_candidates(sig_scripts: &[Vec<u8>]) -> Vec<i64> {
    let mut all = vec![];
    for s in sig_scripts {
        pushes(s, &mut all, 2);
    }
    let mut c: Vec<i64> = all.iter().filter_map(|d| script_num(d)).collect();
    for d in all.iter().filter(|d| d.len() >= 8 && d.len() % 8 == 0) {
        c.extend(d.chunks(8).filter_map(script_num));
    }
    c.sort_unstable();
    c.dedup();
    c
}

#[derive(Deserialize)]
struct RestTx {
    #[serde(default)]
    inputs: Vec<RestIn>,
    #[serde(default)]
    outputs: Vec<RestOut>,
}
#[derive(Deserialize)]
struct RestIn {
    #[serde(default)]
    signature_script: Option<String>,
}
#[derive(Deserialize)]
struct RestOut {
    script_public_key: String,
    #[serde(default)]
    covenant_id: Option<String>,
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .resolver(|addr: &str| -> std::io::Result<Vec<std::net::SocketAddr>> {
            use std::net::ToSocketAddrs;
            let mut v: Vec<_> = addr.to_socket_addrs()?.collect();
            v.sort_by_key(|a| a.is_ipv6());
            Ok(v)
        })
        .build()
}

/// Bestimmt aus der Tx `txid` (REST-API) die Beträge von GHOST-Reserve und
/// Anteils-Minter: Kandidaten aus den Signaturskripten, Skript nachgerechnet
/// und mit dem Ausgang der passenden Covenant verglichen. Rückgabe: (GHOST, Anteile).
pub fn find_amounts_rest(network: &str, txid: &Hash, pool_cov: &Hash, ghost_cov: &Hash, lp_cov: &Hash) -> Result<(i64, i64), String> {
    let url = format!("{}/transactions/{}?inputs=true&outputs=true&resolve_previous_outpoints=no", rest_base(network), txid);
    let tx: RestTx = http_agent().get(&url).call().map_err(|e| format!("REST-API: {e}"))?.into_json().map_err(|e| format!("REST-API: {e}"))?;
    let scripts: Vec<Vec<u8>> = tx
        .inputs
        .iter()
        .filter_map(|i| i.signature_script.as_ref())
        .filter_map(|h| {
            let mut b = vec![0u8; h.len() / 2];
            faster_hex::hex_decode(h.as_bytes(), &mut b).ok().map(|_| b)
        })
        .collect();
    let cands = amount_candidates(&scripts);
    let find = |cov: &Hash, make: &dyn Fn(i64) -> GhostTok| -> Option<i64> {
        let cov_hex = cov.to_string();
        for o in tx.outputs.iter().filter(|o| o.covenant_id.as_deref() == Some(cov_hex.as_str())) {
            let got = o.script_public_key.to_lowercase();
            for &a in &cands {
                let s = spk(&make(a).artifact());
                if got == spk_hex(&s) || got == faster_hex::hex_string(s.script()) {
                    return Some(a);
                }
            }
        }
        None
    };
    let g = find(ghost_cov, &|a| reserve_tok(pool_cov, a)).ok_or(format!("GHOST-Reserve in Tx {txid} nicht gefunden ({} Kandidaten)", cands.len()))?;
    let s = find(lp_cov, &|a| minter_tok(pool_cov, a)).ok_or(format!("Anteils-Minter in Tx {txid} nicht gefunden ({} Kandidaten)", cands.len()))?;
    Ok((g, s))
}

/// Skript wie die REST-API es zeigt: Version (2 Byte, big endian) + Skript, hex
fn spk_hex(s: &ScriptPublicKey) -> String {
    let mut v = s.version().to_be_bytes().to_vec();
    v.extend_from_slice(s.script());
    faster_hex::hex_string(&v)
}

/// UTXO eines pool-eigenen Tokens mit genau diesem Zustand, aus der Tx `txid`,
/// mit der Covenant `cov` (Audit 9 P-2: ohne Covenant-Prüfung zählte ein
/// Köder-Ausgang). KAS-Wert und Outpoint kommen vom Node.
async fn find_tok(net: &Net, tok: GhostTok, cov: Hash, txid: Hash) -> Result<Option<Tracked<GhostTok>>, String> {
    let hits: Vec<_> = net
        .utxos(&net.address_of_spk(&spk(&tok.artifact()))?)
        .await?
        .into_iter()
        .filter(|(o, e)| o.transaction_id == txid && e.covenant_id == Some(cov))
        .collect();
    Ok(match hits.as_slice() {
        [(o, e)] => Some(Tracked { outpoint: *o, value: e.amount, cov, state: tok }),
        _ => None,
    })
}

/// Bringt den Pool-Stand auf die Kette: aktuelle Pool-UTXO über die feste
/// Adresse, Reserve und Minter aus derselben Tx, jedes Mal am Node bestätigt.
/// Einen offenen Pool kann niemand auflösen: Findet der Node keine Pool-UTXO,
/// ist das ein Node-Fehler, und der Stand bleibt unverändert (Audit 10, A10-P-3).
pub async fn resync(net: &Net, network: &str, rec: &PoolRec) -> Result<PoolRec, String> {
    let art = pool_artifact(&rec.params, &rec.lp_cov, true);
    let addr = net.address_of_spk(&spk(&art))?;
    let cands: Vec<_> = net.utxos(&addr).await?.into_iter().filter(|(_, e)| e.covenant_id == Some(rec.pool.cov)).collect();
    let (op, entry) = match cands.as_slice() {
        [] => return Err("Pool-UTXO am Node nicht gefunden (Node nicht synchron?) – Stand bleibt unverändert".into()),
        [one] => one.clone(),
        _ => return Err("mehrere Pool-UTXOs gefunden – der Pool ist nicht eindeutig".into()),
    };
    let (pcov, gcov, lcov, txid) = (rec.pool.cov, rec.params.ghost_cov, rec.lp_cov, op.transaction_id);
    let pool_t = Tracked { outpoint: op, value: entry.amount, cov: pcov, state: () };
    // 1. Alte Beträge (unverändert oder nur KAS bewegt)
    let r = find_tok(net, reserve_tok(&pcov, rec.ghost()), gcov, txid).await?;
    let m = find_tok(net, minter_tok(&pcov, rec.shares()), lcov, txid).await?;
    if let (Some(r), Some(m)) = (r, m) {
        return Ok(PoolRec { params: rec.params.clone(), lp_cov: lcov, pool: pool_t, reserve: r, minter: m });
    }
    // 2. Neue Beträge über die REST-API, jeder Treffer am Node bestätigt
    let net_name = network.to_string();
    let (g, s) = tokio::task::spawn_blocking(move || find_amounts_rest(&net_name, &txid, &pcov, &gcov, &lcov)).await.map_err(|e| e.to_string())??;
    let r = find_tok(net, reserve_tok(&pcov, g), gcov, txid).await?.ok_or(format!("GHOST-Reserve über {:.8} laut REST-API, aber nicht mit GHOST-Covenant am Node", g as f64 / 1e8))?;
    let m = find_tok(net, minter_tok(&pcov, s), lcov, txid).await?.ok_or(format!("Anteils-Minter über {s} laut REST-API, aber nicht mit Anteils-Covenant am Node"))?;
    Ok(PoolRec { params: rec.params.clone(), lp_cov: lcov, pool: pool_t, reserve: r, minter: m })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechnung_wie_im_vertrag() {
        let e8 = 100_000_000i64;
        let (x, y) = (10_000 * e8, 460 * e8);
        let dy = ghost_out(x, y, 100 * e8, POOL_FEE_BPS);
        assert!(swap_ok(x, y, x + 100 * e8, y - dy, POOL_FEE_BPS));
        assert!(!swap_ok(x, y, x + 100 * e8, y - dy - 1, POOL_FEE_BPS));
        assert!(dy > 454 * e8 / 100 && dy < 4_555 * e8 / 1000);
    }

    #[test]
    fn betraege_aus_signaturskripten() {
        // 0x08 + 8 Byte Skriptzahl 4 600 000 000 (= 46 GHOST), dazu Kleinkram
        let n: i64 = 4_600_000_000;
        let mut b = n.to_le_bytes().to_vec();
        while b.len() > 1 && b[b.len() - 1] == 0 && b[b.len() - 2] & 0x80 == 0 {
            b.pop();
        }
        let mut s = vec![b.len() as u8];
        s.extend(&b);
        s.extend([0x51, 0x00, 0x02, 0xff, 0x80]); // OP_1, OP_0, negative Zahl
        let c = amount_candidates(&[s]);
        assert!(c.contains(&n), "{c:?}");
        assert!(c.contains(&1));
        assert!(!c.iter().any(|&v| v <= 0));
    }
}
