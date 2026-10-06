//! Zustand von der Kette nachführen, auch wenn ihn ein anderer Rechner
//! verändert hat (Audit 10, A10-A-3).
//!
//! Der Node findet UTXOs nur über ihre Adresse, und die hängt bei Orakel und
//! Vault vom Zustand ab. Nach einem fremden Orakel-Update oder Prägen/Tilgen/
//! Liquidieren kennt ghostctl die neue Adresse also nicht. Lösung:
//!
//! 1. Über den Adressverlauf der REST-API die Tx finden, die die bekannte UTXO
//!    ausgegeben hat, und darin den Ausgang derselben Covenant-ID.
//! 2. Den neuen Zustand aus der Tx rekonstruieren: Kandidaten aus den
//!    Argumenten des Vertragsaufrufs, jeweils das Skript gebaut und mit dem
//!    Ausgang verglichen. Nur ein exakt passender Kandidat zählt.
//! 3. Am Ende bestätigt der Node: Die UTXO liegt dort, mit dieser Covenant-ID
//!    und genau diesem Skript. Eine falsche REST-Antwort führt daher nur zu
//!    einem Fehler, nie zu einem falschen Zustand.
//!
//! Neue Vaults findet `discover_vaults` über den Verlauf der Factory, deren
//! Adresse nach `init` fest ist.
//!
//! Skripte werden ohne Kompilieren gebaut: Der Zustand liegt als Block fester
//! Breite im Bytecode (Template-Präfix + Zustand + Suffix). Ein Kompilieren
//! kostet ~200 ms, so ein Skript Mikrosekunden; das erlaubt viele Kandidaten.

use crate::contracts::*;
use crate::math;
use crate::ops::{Deployment, Tracked, VaultRec};
use crate::pool::{amount_candidates, rest_base};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{Transaction, TransactionOutpoint};
use kaspa_txscript::pay_to_script_hash_script;
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

// ------------------------------------------------------------ Tx-Ansicht ----

/// Eingang einer Tx: ausgegebener Outpoint und Signaturskript
#[derive(Clone, Debug)]
pub struct TxIn {
    pub prev: TransactionOutpoint,
    pub sig: Vec<u8>,
}

/// Ausgang einer Tx: Skript (P2SH-Skriptbytes), Wert, Covenant-ID
#[derive(Clone, Debug)]
pub struct TxOut {
    pub index: u32,
    pub value: u64,
    pub spk: Vec<u8>,
    pub cov: Option<Hash>,
}

#[derive(Clone, Debug)]
pub struct TxView {
    pub id: Hash,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
}

impl From<&Transaction> for TxView {
    fn from(tx: &Transaction) -> Self {
        Self {
            id: tx.id(),
            inputs: tx.inputs.iter().map(|i| TxIn { prev: i.previous_outpoint, sig: i.signature_script.clone() }).collect(),
            outputs: tx
                .outputs
                .iter()
                .enumerate()
                .map(|(n, o)| TxOut { index: n as u32, value: o.value, spk: o.script_public_key.script().to_vec(), cov: o.covenant.map(|c| c.covenant_id) })
                .collect(),
        }
    }
}

// ------------------------------------------------- Skripte ohne Kompilieren ----

/// Bytecode = Präfix + Zustand + Suffix (Template::of)
#[derive(Clone)]
pub struct Shape {
    prefix: Vec<u8>,
    suffix: Vec<u8>,
}

impl Shape {
    pub fn of(a: &Artifact) -> Self {
        let t = Template::of(a);
        Self { prefix: t.prefix, suffix: t.suffix }
    }
    pub fn code(&self, state: &[u8]) -> Vec<u8> {
        [self.prefix.as_slice(), state, self.suffix.as_slice()].concat()
    }
    /// P2SH-Skriptbytes zu diesem Zustand
    pub fn spk(&self, state: &[u8]) -> Vec<u8> {
        pay_to_script_hash_script(&self.code(state)).script().to_vec()
    }
    /// Zustandsblock aus einem Redeem-Skript dieser Form, sonst None
    pub fn state_of<'a>(&self, code: &'a [u8]) -> Option<&'a [u8]> {
        if code.len() <= self.prefix.len() + self.suffix.len() || !code.starts_with(&self.prefix) || !code.ends_with(&self.suffix) {
            return None;
        }
        Some(&code[self.prefix.len()..code.len() - self.suffix.len()])
    }
}

fn push_int(out: &mut Vec<u8>, v: i64) {
    out.push(0x08);
    out.extend_from_slice(&script_num8(v));
}

fn push_bool(out: &mut Vec<u8>, v: bool) {
    out.extend_from_slice(&[0x01, v as u8]);
}

fn push_32(out: &mut Vec<u8>, v: &[u8]) {
    out.push(0x20);
    out.extend_from_slice(v);
}

/// Zustand des Orakels v4: 5 Zahlen, frozen, lastRateDaa (56 Byte)
pub fn oracle_state_bytes(s: &OracleState) -> Vec<u8> {
    let mut out = vec![];
    for v in [s.kas_usd, s.oracle_daa, s.seq, s.stable_rate, s.stable_index] {
        push_int(&mut out, v);
    }
    push_bool(&mut out, s.frozen);
    push_int(&mut out, s.last_rate_daa);
    out
}

/// Zustand einer Register-UTXO (209 Byte, Reihenfolge wie im Vertrag)
pub fn register_state_bytes(s: &RegisterState) -> Vec<u8> {
    let mut out = vec![];
    push_bool(&mut out, s.ticket);
    push_32(&mut out, &s.set_hash);
    push_32(&mut out, &s.fb_hash);
    out.extend_from_slice(&[0x01, s.pay_kind]);
    push_32(&mut out, &s.pay_to);
    push_int(&mut out, s.nonce);
    push_bool(&mut out, s.emerg);
    push_int(&mut out, s.last_daa);
    push_32(&mut out, &s.oracle_cov);
    push_32(&mut out, &s.oracle_tpl);
    push_int(&mut out, s.oracle_pre);
    push_int(&mut out, s.oracle_suf);
    push_bool(&mut out, s.initialized);
    out
}

/// Zustand eines Vaults (Version 3): 0x20 + Besitzer, dann debt, interest,
/// indexAt je 0x08 + 8 Byte
pub fn vault_state_bytes(owner: &[u8], st: &VaultState) -> Vec<u8> {
    let mut out = vec![0x20];
    out.extend_from_slice(owner);
    for v in [st.debt, st.interest, st.index_at] {
        push_int(&mut out, v);
    }
    out
}

/// Zustand eines GHOST-Tokens: Besitzer, Typ, Betrag, Minter
pub fn ghost_state_bytes(t: &GhostTok) -> Vec<u8> {
    let mut out = vec![0x20];
    out.extend_from_slice(&t.owner);
    out.extend_from_slice(&[0x01, t.typ]);
    push_int(&mut out, t.amount);
    out.extend_from_slice(&[0x01, t.minter as u8]);
    out
}

fn read_int(b: &[u8]) -> Option<i64> {
    if b.len() != 9 || b[0] != 0x08 {
        return None;
    }
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[1..]);
    let neg = a[7] & 0x80 != 0;
    a[7] &= 0x7f;
    let v = i64::from_le_bytes(a);
    Some(if neg { -v } else { v })
}

fn read_bool(b: &[u8]) -> Option<bool> {
    match b {
        [0x01, 0] => Some(false),
        [0x01, 1] => Some(true),
        _ => None,
    }
}

fn read_32(b: &[u8]) -> Option<Vec<u8>> {
    (b.len() == 33 && b[0] == 0x20).then(|| b[1..].to_vec())
}

fn parse_oracle_state(b: &[u8]) -> Option<OracleState> {
    if b.len() != 56 {
        return None;
    }
    let f = |k: usize| read_int(&b[9 * k..9 * k + 9]);
    Some(OracleState {
        kas_usd: f(0)?,
        oracle_daa: f(1)?,
        seq: f(2)?,
        stable_rate: f(3)?,
        stable_index: f(4)?,
        frozen: read_bool(&b[45..47])?,
        last_rate_daa: read_int(&b[47..56])?,
    })
}

/// Gegenstück zu register_state_bytes
pub fn parse_register_state(b: &[u8]) -> Option<RegisterState> {
    if b.len() != 209 {
        return None;
    }
    let mut at = 0usize;
    let mut take = |n: usize| {
        let x = &b[at..at + n];
        at += n;
        x
    };
    Some(RegisterState {
        ticket: read_bool(take(2))?,
        set_hash: read_32(take(33))?,
        fb_hash: read_32(take(33))?,
        pay_kind: match take(2) {
            [0x01, k] => *k,
            _ => return None,
        },
        pay_to: read_32(take(33))?,
        nonce: read_int(take(9))?,
        emerg: read_bool(take(2))?,
        last_daa: read_int(take(9))?,
        oracle_cov: read_32(take(33))?,
        oracle_tpl: read_32(take(33))?,
        oracle_pre: read_int(take(9))?,
        oracle_suf: read_int(take(9))?,
        initialized: read_bool(take(2))?,
    })
}

fn parse_ghost_amount(b: &[u8]) -> Option<i64> {
    if b.len() != 46 || b[0] != 0x20 || b[33] != 0x01 || b[44] != 0x01 {
        return None;
    }
    read_int(&b[35..44])
}

/// Datenpushes auf oberster Ebene (Signaturskript: Argumente …, Redeem-Skript zuletzt)
fn top_pushes(script: &[u8]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    let mut i = 0usize;
    while i < script.len() {
        let op = script[i];
        i += 1;
        let len = match op {
            0x00 => {
                out.push(vec![]);
                continue;
            }
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
        out.push(script[i..i + len].to_vec());
        i += len;
    }
    out
}

/// Redeem-Skript eines P2SH-Eingangs (letzter Push)
fn redeem(sig: &[u8]) -> Option<Vec<u8>> {
    top_pushes(sig).pop()
}

// ------------------------------------------------------- Rekonstruktion ----

/// Formen der Verträge eines Deployments (einmal kompiliert)
pub struct Shapes {
    pub oracle: Shape,
    pub register: Shape,
    pub vault: Option<Shape>,
    pub ghost: Option<Shape>,
}

impl Shapes {
    pub fn of(d: &Deployment) -> Self {
        Self {
            oracle: Shape::of(&oracle(&d.oracle_params, &d.oracle.state)),
            register: Shape::of(&register(&d.register_params, &d.register.state)),
            vault: d.vault_params.as_ref().map(|vp| Shape::of(&vault(vp, &[0u8; 32], &VaultState::default()))),
            ghost: d.vault_params.as_ref().map(|_| Shape::of(&GhostTok::to_pubkey(&[0u8; 32], 0).artifact())),
        }
    }
}

/// Ergebnis eines Schritts: Covenant geht weiter (Ausgang, neuer Zustand) oder endet
pub enum Next<S> {
    Continues(TxOut, S),
    Ended,
}

fn cov_output<'a>(tx: &'a TxView, cov: &Hash) -> Result<Option<&'a TxOut>, String> {
    let outs: Vec<&TxOut> = tx.outputs.iter().filter(|o| o.cov.as_ref() == Some(cov)).collect();
    match outs.as_slice() {
        [] => Ok(None),
        [o] => Ok(Some(o)),
        _ => Err(format!("Tx {}: mehrere Ausgänge derselben Covenant", tx.id)),
    }
}

/// Orakel: aus der Tx, die `cur` ausgibt, den neuen Zustand bestimmen.
/// read() lässt ihn unverändert, freeze() setzt nur frozen, update(newKasUsd,
/// newOracleDaa, newStableRate) ändert ihn wie in price_oracle_v4.sil.
pub fn oracle_next(sh: &Shapes, p: &OracleParams, cur: &OracleState, prev: &TransactionOutpoint, cov: &Hash, tx: &TxView) -> Result<Next<OracleState>, String> {
    let Some(out) = cov_output(tx, cov)? else {
        return Err(format!("Tx {}: Orakel ohne Fortsetzung – das lässt der Vertrag nicht zu", tx.id));
    };
    for s in [*cur, OracleState { frozen: true, ..*cur }] {
        if out.spk == sh.oracle.spk(&oracle_state_bytes(&s)) {
            return Ok(Next::Continues(out.clone(), s));
        }
    }
    let inp = tx.inputs.iter().find(|i| &i.prev == prev).ok_or(format!("Tx {}: Orakel-Eingang fehlt", tx.id))?;
    let nums = amount_candidates(std::slice::from_ref(&inp.sig));
    let prices: Vec<i64> = nums.iter().copied().filter(|&v| (1_000..=90_000_000_000).contains(&v) && v * 2 >= cur.kas_usd && v <= cur.kas_usd * 2).collect();
    let daas: Vec<i64> = nums.iter().copied().filter(|&v| v >= cur.oracle_daa + 600 && v <= cur.oracle_daa + 10_000_000_000).collect();
    let mut rates: Vec<i64> = nums.iter().copied().filter(|&v| v <= p.max_rate).collect();
    rates.extend([0, cur.stable_rate]);
    rates.sort_unstable();
    rates.dedup();
    for &d in &daas {
        for &k in &prices {
            for &r in &rates {
                let Some(s) = oracle_next_state(cur, k, d, r) else { continue };
                if out.spk == sh.oracle.spk(&oracle_state_bytes(&s)) {
                    return Ok(Next::Continues(out.clone(), s));
                }
            }
        }
    }
    Err(format!("Tx {}: neuer Orakelzustand nicht rekonstruierbar", tx.id))
}

/// Register: aus der Tx, die die Haupt-UTXO `cur` ausgibt, den neuen Zustand
/// der Haupt-UTXO bestimmen. Kandidaten je Eintrag (signer_register_v4.sil):
/// witness (unverändert), attestPrice (lastDaa = neuer Preis-DAA, nonce + 1 bei
/// offenem Notfall), propose (nonce + 1, emerg je nach Weg), cancel (nonce + 1),
/// activate (Satz, Notfallsatz und Zinsziel aus dem Ticket-Eingang).
pub fn register_next(sh: &Shapes, cur: &RegisterState, prev: &TransactionOutpoint, cov: &Hash, tx: &TxView) -> Result<Next<RegisterState>, String> {
    let outs: Vec<&TxOut> = tx.outputs.iter().filter(|o| o.cov.as_ref() == Some(cov)).collect();
    if outs.is_empty() {
        return Err(format!("Tx {}: Register ohne Fortsetzung – das lässt der Vertrag nicht zu", tx.id));
    }
    let mut cands = vec![cur.clone()];
    let bump = RegisterState { nonce: cur.nonce + 1, emerg: false, ..cur.clone() };
    cands.push(bump.clone());
    cands.push(RegisterState { emerg: true, ..bump.clone() });
    if let Some(inp) = tx.inputs.iter().find(|i| &i.prev == prev) {
        for d in amount_candidates(std::slice::from_ref(&inp.sig)).into_iter().filter(|&d| d > cur.last_daa) {
            cands.push(RegisterState { last_daa: d, emerg: false, ..cur.clone() });
            cands.push(RegisterState { last_daa: d, ..bump.clone() });
        }
    }
    for t in tx.inputs.iter().filter_map(|i| redeem(&i.sig)).filter_map(|c| sh.register.state_of(&c).and_then(parse_register_state)).filter(|t| t.ticket) {
        cands.push(RegisterState { set_hash: t.set_hash, fb_hash: t.fb_hash, pay_kind: t.pay_kind, pay_to: t.pay_to, ..bump.clone() });
    }
    for o in &outs {
        for c in &cands {
            if o.spk == sh.register.spk(&register_state_bytes(c)) {
                return Ok(Next::Continues((*o).clone(), c.clone()));
            }
        }
    }
    Err(format!("Tx {}: neuer Register-Zustand nicht rekonstruierbar", tx.id))
}

/// Vault: aus der Tx, die `prev` ausgibt, den neuen Zustand bestimmen
/// (stable_vault.sil Version 3). Mit Orakel-Eingang wird der Zins bis zu dessen
/// Index abgerechnet (indexAt = Index); die Schuld ändert sich um den Betrag des
/// Aufrufs: mint +amount, repay −verbrannt, redeem −amount, liquidate −burn
/// (Zins bleibt stehen), withdraw ±0. deposit lässt alles gleich, close und
/// sweep beenden.
pub fn vault_next(sh: &Shapes, owner: &[u8], st: &VaultState, prev: &TransactionOutpoint, cov: &Hash, tx: &TxView) -> Result<Next<VaultState>, String> {
    let vs = sh.vault.as_ref().ok_or("Deployment ohne Vault-Parameter")?;
    let Some(out) = cov_output(tx, cov)? else {
        return Ok(Next::Ended);
    };
    let spk_of = |s: &VaultState| vs.spk(&vault_state_bytes(owner, s));
    if out.spk == spk_of(st) {
        return Ok(Next::Continues(out.clone(), *st));
    }
    let inp = tx.inputs.iter().find(|i| &i.prev == prev).ok_or(format!("Tx {}: Vault-Eingang fehlt", tx.id))?;
    // Zinsindex aus dem Orakel-Eingang derselben Tx (read())
    let mut indices: Vec<i64> = tx
        .inputs
        .iter()
        .filter_map(|i| redeem(&i.sig))
        .filter_map(|c| sh.oracle.state_of(&c).and_then(parse_oracle_state))
        .map(|s| s.stable_index)
        .collect();
    indices.dedup();
    if indices.is_empty() {
        return Err(format!("Tx {}: Zustand geändert, aber kein Orakel-Eingang", tx.id));
    }
    // Beträge aus dem Aufruf (mint: amount, liquidate: burn, redeem: amount)
    let mut amounts = amount_candidates(std::slice::from_ref(&inp.sig));
    // repay: verbrannt = GHOST-Eingänge − GHOST-Ausgänge. Eingänge exakt aus den
    // Redeem-Skripten, die Ausgangsbeträge stehen als ein Push von 8-Byte-Zahlen
    // im Aufruf (outStates, feldweise) – jede solche Folge ist ein Kandidat.
    if let Some(gs) = &sh.ghost {
        let ghost_in: i64 = tx.inputs.iter().filter_map(|i| redeem(&i.sig)).filter_map(|c| gs.state_of(&c).and_then(parse_ghost_amount)).sum();
        if ghost_in > 0 {
            let mut out_sums = vec![0i64];
            for d in top_pushes(&inp.sig).iter().filter(|d| !d.is_empty() && d.len() % 8 == 0 && d.len() <= 64) {
                let parts: Option<Vec<i64>> = d.chunks(8).map(|c| read_int(&[&[0x08], c].concat())).collect();
                if let Some(ps) = parts {
                    if ps.iter().all(|&v| v >= 0) {
                        out_sums.push(ps.iter().sum());
                    }
                }
            }
            amounts.extend(out_sums.iter().filter(|&&o| o < ghost_in).map(|&o| ghost_in - o));
        }
    }
    amounts.sort_unstable();
    amounts.dedup();
    for &idx in &indices {
        let acc = math::accrued(st, idx);
        let mut debts = vec![st.debt];
        for &a in &amounts {
            if let Some(n) = st.debt.checked_add(a) {
                debts.push(n);
            }
            if a <= st.debt {
                debts.push(st.debt - a);
            }
        }
        // jede Fortsetzung rechnet den Zins bis zum Orakelindex ab und lässt ihn stehen
        for &d in &debts {
            let cand = VaultState { debt: d, interest: acc, index_at: idx };
            if out.spk == spk_of(&cand) {
                return Ok(Next::Continues(out.clone(), cand));
            }
        }
    }
    Err(format!("Tx {}: neuer Vault-Zustand nicht rekonstruierbar", tx.id))
}

/// Neuer Vault aus einer openVault-Tx der Factory: Besitzer aus den Argumenten,
/// Vault-Ausgang (Schuld 0) und Minter-Zweig (minter_of(vid)) am Skript bestätigt.
pub fn vault_from_open(sh: &Shapes, root_cov: &Hash, factory_prev: &TransactionOutpoint, known: &[Hash], tx: &TxView) -> Option<VaultRec> {
    let vs = sh.vault.as_ref()?;
    let gs = sh.ghost.as_ref()?;
    let inp = tx.inputs.iter().find(|i| &i.prev == factory_prev)?;
    let owners: Vec<Vec<u8>> = top_pushes(&inp.sig).into_iter().filter(|d| d.len() == 32).collect();
    for o in tx.outputs.iter().filter(|o| o.cov.is_some_and(|c| !known.contains(&c) && c != *root_cov)) {
        let vid = o.cov.unwrap();
        let Some(owner) = owners.iter().find(|w| o.spk == vs.spk(&vault_state_bytes(w, &VaultState::default()))) else { continue };
        let branch_state = GhostTok::minter_of(&vid);
        let b = tx.outputs.iter().find(|b| b.cov == Some(*root_cov) && b.spk == gs.spk(&ghost_state_bytes(&branch_state)))?;
        return Some(VaultRec {
            owner: owner.clone(),
            vault: Tracked { outpoint: TransactionOutpoint::new(tx.id, o.index), value: o.value, cov: vid, state: VaultState::default() },
            branch: Tracked { outpoint: TransactionOutpoint::new(tx.id, b.index), value: b.value, cov: *root_cov, state: branch_state },
            stale: false,
        });
    }
    None
}

// --------------------------------------------------------- REST-Verlauf ----

#[derive(Deserialize)]
struct HTx {
    transaction_id: String,
    #[serde(default)]
    inputs: Option<Vec<HIn>>,
    #[serde(default)]
    outputs: Option<Vec<HOut>>,
}
#[derive(Deserialize)]
struct HIn {
    previous_outpoint_hash: String,
    previous_outpoint_index: serde_json::Value,
    #[serde(default)]
    signature_script: Option<String>,
}
#[derive(Deserialize)]
struct HOut {
    index: u32,
    amount: u64,
    script_public_key: String,
    #[serde(default)]
    covenant_id: Option<String>,
}

fn hex(s: &str) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; s.len() / 2];
    faster_hex::hex_decode(s.as_bytes(), &mut b).map_err(|e| format!("REST-API: Hex ({e})"))?;
    Ok(b)
}

impl HTx {
    fn view(self) -> Result<TxView, String> {
        let id = Hash::from_str(&self.transaction_id).map_err(|e| format!("REST-API: Tx-ID ({e})"))?;
        let mut inputs = vec![];
        for i in self.inputs.unwrap_or_default() {
            let idx = match &i.previous_outpoint_index {
                serde_json::Value::String(s) => s.parse::<u32>().map_err(|e| format!("REST-API: Index ({e})"))?,
                serde_json::Value::Number(n) => n.as_u64().ok_or("REST-API: Index")? as u32,
                _ => return Err("REST-API: Index fehlt".into()),
            };
            let prev = TransactionOutpoint::new(Hash::from_str(&i.previous_outpoint_hash).map_err(|e| format!("REST-API: Outpoint ({e})"))?, idx);
            inputs.push(TxIn { prev, sig: hex(i.signature_script.as_deref().unwrap_or(""))? });
        }
        let mut outputs = vec![];
        for o in self.outputs.unwrap_or_default() {
            let cov = match o.covenant_id.as_deref() {
                Some(c) if !c.is_empty() => Some(Hash::from_str(c).map_err(|e| format!("REST-API: Covenant-ID ({e})"))?),
                _ => None,
            };
            outputs.push(TxOut { index: o.index, value: o.amount, spk: hex(&o.script_public_key)?, cov });
        }
        Ok(TxView { id, inputs, outputs })
    }
}

fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .resolver(|addr: &str| -> std::io::Result<Vec<std::net::SocketAddr>> {
            use std::net::ToSocketAddrs;
            let mut v: Vec<_> = addr.to_socket_addrs()?.collect();
            v.sort_by_key(|a| a.is_ipv6()); // im lokalen Netz war IPv6 gestört
            Ok(v)
        })
        .build()
}

/// Alle Tx einer Adresse (neueste zuerst), seitenweise; höchstens `max` Stück
pub fn address_history(network: &str, address: &str, max: usize) -> Result<Vec<TxView>, String> {
    const PAGE: usize = 100;
    let agent = http();
    let mut all = vec![];
    let mut offset = 0;
    while offset < max {
        let url = format!("{}/addresses/{}/full-transactions?limit={PAGE}&offset={offset}&resolve_previous_outpoints=no", rest_base(network), address);
        let page: Vec<HTx> = agent.get(&url).call().map_err(|e| format!("REST-API: {e}"))?.into_json().map_err(|e| format!("REST-API: {e}"))?;
        let n = page.len();
        for t in page {
            all.push(t.view()?);
        }
        if n < PAGE {
            break;
        }
        offset += PAGE;
    }
    Ok(all)
}

/// Tx einer Adresse als rohes REST-JSON (neueste zuerst), mit Adressen und
/// Beträgen der ausgegebenen Outpoints (`resolve_previous_outpoints=light`)
/// und dem Payload als Hex (Feld `payload`, bei leerem Payload `null`;
/// gemessen am 29.09.2026 an api.kaspa.org). Für `ghostctl messages`.
pub fn address_txs_json(network: &str, address: &str, max: usize) -> Result<Vec<serde_json::Value>, String> {
    const PAGE: usize = 50;
    let agent = http();
    let mut all = vec![];
    let mut offset = 0;
    while offset < max {
        let n = PAGE.min(max - offset);
        let url = format!("{}/addresses/{}/full-transactions?limit={n}&offset={offset}&resolve_previous_outpoints=light", rest_base(network), address);
        let page: Vec<serde_json::Value> = agent.get(&url).call().map_err(|e| format!("REST-API: {e}"))?.into_json().map_err(|e| format!("REST-API: {e}"))?;
        let got = page.len();
        all.extend(page);
        if got < n {
            break;
        }
        offset += n;
    }
    Ok(all)
}

/// Eine Tx als rohes REST-JSON (gleiche Felder wie `address_txs_json`)
pub fn tx_json(network: &str, txid: &str) -> Result<serde_json::Value, String> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("TXID: 64 Hex-Zeichen erwartet".into());
    }
    let url = format!("{}/transactions/{txid}?inputs=true&outputs=true&resolve_previous_outpoints=light", rest_base(network));
    http().get(&url).call().map_err(|e| format!("REST-API: {e}"))?.into_json().map_err(|e| format!("REST-API: {e}"))
}

/// Wurde die Tx in einem Block angenommen (REST, Feld `is_accepted`)?
/// Ok(None) = der REST-API unbekannt (404), Ok(Some(false)) = bekannt, aber
/// (noch) nicht angenommen. Für das Journal einer Wallet-Tx (store.rs).
pub fn tx_accepted(network: &str, txid: &str) -> Result<Option<bool>, String> {
    if txid.len() != 64 || !txid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("TXID: 64 Hex-Zeichen erwartet".into());
    }
    let url = format!("{}/transactions/{txid}?inputs=false&outputs=false", rest_base(network));
    match http().get(&url).call() {
        Ok(r) => {
            let v: serde_json::Value = r.into_json().map_err(|e| format!("REST-API: {e}"))?;
            Ok(Some(v["is_accepted"] == serde_json::Value::Bool(true)))
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(e) => Err(format!("REST-API: {e}")),
    }
}

/// Die ANGENOMMENE Tx, die den Outpoint `op` (Skript `spk`) ausgegeben hat,
/// laut REST-Verlauf der Adresse (neueste zuerst, höchstens 300 Tx). Für das
/// Journal einer Wallet-Tx (store.rs): eigene Tx, Zwilling oder Verdrängung.
pub fn accepted_spender(network: &str, spk: &[u8], op: &TransactionOutpoint) -> Result<Option<TxView>, String> {
    const PAGE: usize = 100;
    let prefix = kaspa_addresses::Prefix::from(crate::net::network_id(network)?);
    let s = kaspa_consensus_core::tx::ScriptPublicKey::new(0, spk.to_vec().into());
    let addr = kaspa_txscript::standard::extract_script_pub_key_address(&s, prefix).map_err(|e| e.to_string())?;
    let agent = http();
    for page in 0..3 {
        let url = format!("{}/addresses/{}/full-transactions?limit={PAGE}&offset={}&resolve_previous_outpoints=no", rest_base(network), addr, page * PAGE);
        let txs: Vec<serde_json::Value> = agent.get(&url).call().map_err(|e| format!("REST-API: {e}"))?.into_json().map_err(|e| format!("REST-API: {e}"))?;
        let n = txs.len();
        for v in txs {
            if v["is_accepted"] != serde_json::Value::Bool(true) {
                continue;
            }
            let t: HTx = serde_json::from_value(v).map_err(|e| format!("REST-API: {e}"))?;
            let t = t.view()?;
            if t.inputs.iter().any(|i| &i.prev == op) {
                return Ok(Some(t));
            }
        }
        if n < PAGE {
            break;
        }
    }
    Ok(None)
}

/// Quelle für den Verlauf: REST im Betrieb, eine Tx-Liste im Test
pub trait Chain {
    /// Tx, die `op` ausgibt (die UTXO hatte das Skript `spk`), sonst None
    fn spender(&mut self, spk: &[u8], op: &TransactionOutpoint) -> Result<Option<TxView>, String>;
    /// Alle Tx, die das Skript `spk` betreffen, neueste zuerst
    fn txs(&mut self, spk: &[u8]) -> Result<Vec<TxView>, String>;
}

/// Verlauf mehrerer Adressen über die REST-API, je Adresse einmal geladen
pub struct History<'a> {
    network: &'a str,
    prefix: kaspa_addresses::Prefix,
    cache: HashMap<String, Vec<TxView>>,
}

impl<'a> History<'a> {
    pub fn new(network: &'a str) -> Result<Self, String> {
        let prefix = kaspa_addresses::Prefix::from(crate::net::network_id(network)?);
        Ok(Self { network, prefix, cache: HashMap::new() })
    }
    fn address(&self, spk: &[u8]) -> Result<String, String> {
        let s = kaspa_consensus_core::tx::ScriptPublicKey::new(0, spk.to_vec().into());
        kaspa_txscript::standard::extract_script_pub_key_address(&s, self.prefix).map(|a| a.to_string()).map_err(|e| e.to_string())
    }
    fn load(&mut self, spk: &[u8]) -> Result<&Vec<TxView>, String> {
        let addr = self.address(spk)?;
        if !self.cache.contains_key(&addr) {
            let h = address_history(self.network, &addr, 2_000)?;
            self.cache.insert(addr.clone(), h);
        }
        Ok(&self.cache[&addr])
    }
}

impl Chain for History<'_> {
    fn spender(&mut self, spk: &[u8], op: &TransactionOutpoint) -> Result<Option<TxView>, String> {
        Ok(self.load(spk)?.iter().find(|t| t.inputs.iter().any(|i| &i.prev == op)).cloned())
    }
    fn txs(&mut self, spk: &[u8]) -> Result<Vec<TxView>, String> {
        Ok(self.load(spk)?.clone())
    }
}

/// Verlauf aus einer Liste von Tx (Simulator-Tests), älteste zuerst übergeben
pub struct MemChain {
    pub txs: Vec<TxView>,
}

impl Chain for MemChain {
    fn spender(&mut self, _spk: &[u8], op: &TransactionOutpoint) -> Result<Option<TxView>, String> {
        Ok(self.txs.iter().find(|t| t.inputs.iter().any(|i| &i.prev == op)).cloned())
    }
    fn txs(&mut self, spk: &[u8]) -> Result<Vec<TxView>, String> {
        let outs: HashMap<TransactionOutpoint, Vec<u8>> =
            self.txs.iter().flat_map(|t| t.outputs.iter().map(move |o| (TransactionOutpoint::new(t.id, o.index), o.spk.clone()))).collect();
        let mut v: Vec<TxView> = self
            .txs
            .iter()
            .filter(|t| t.outputs.iter().any(|o| o.spk == spk) || t.inputs.iter().any(|i| outs.get(&i.prev).is_some_and(|s| s == spk)))
            .cloned()
            .collect();
        v.reverse();
        Ok(v)
    }
}

/// Höchstzahl Schritte je Kette (Schutz gegen Endlosschleifen)
const MAX_STEPS: usize = 5_000;

/// Orakel von `start` bis zur letzten Ausgabe verfolgen (ohne Node).
/// Rückgabe: Outpoint, Wert, Zustand, Zahl der Updates unterwegs.
pub fn follow_oracle(h: &mut impl Chain, sh: &Shapes, p: &OracleParams, start: &Tracked<OracleState>) -> Result<(TransactionOutpoint, u64, OracleState, u32), String> {
    let (mut op, mut value, mut s, mut updates) = (start.outpoint, start.value, start.state, 0u32);
    for _ in 0..MAX_STEPS {
        let spk = sh.oracle.spk(&oracle_state_bytes(&s));
        let Some(tx) = h.spender(&spk, &op)? else {
            return Ok((op, value, s, updates));
        };
        match oracle_next(sh, p, &s, &op, &start.cov, &tx)? {
            Next::Continues(out, ns) => {
                if ns != s {
                    updates += 1;
                }
                op = TransactionOutpoint::new(tx.id, out.index);
                value = out.value;
                s = ns;
            }
            Next::Ended => return Err("Orakel beendet".into()),
        }
    }
    Err("Orakel: zu viele Schritte".into())
}

/// Register-Haupt-UTXO von `start` bis zur letzten Ausgabe verfolgen (ohne Node)
pub fn follow_register(h: &mut impl Chain, sh: &Shapes, start: &Tracked<RegisterState>) -> Result<(TransactionOutpoint, u64, RegisterState), String> {
    let (mut op, mut value, mut s) = (start.outpoint, start.value, start.state.clone());
    for _ in 0..MAX_STEPS {
        let spk = sh.register.spk(&register_state_bytes(&s));
        let Some(tx) = h.spender(&spk, &op)? else {
            return Ok((op, value, s));
        };
        match register_next(sh, &s, &op, &start.cov, &tx)? {
            Next::Continues(out, ns) => {
                op = TransactionOutpoint::new(tx.id, out.index);
                value = out.value;
                s = ns;
            }
            Next::Ended => return Err("Register beendet".into()),
        }
    }
    Err("Register: zu viele Schritte".into())
}

/// Vault von `start` bis zur letzten Ausgabe verfolgen (ohne Node).
/// None = Vault beendet (geschlossen oder ganz liquidiert).
pub fn follow_vault(h: &mut impl Chain, sh: &Shapes, owner: &[u8], start: &Tracked<VaultState>) -> Result<Option<(TransactionOutpoint, u64, VaultState)>, String> {
    let vs = sh.vault.as_ref().ok_or("Deployment ohne Vault-Parameter")?;
    let (mut op, mut value, mut s) = (start.outpoint, start.value, start.state);
    for _ in 0..MAX_STEPS {
        let spk = vs.spk(&vault_state_bytes(owner, &s));
        let Some(tx) = h.spender(&spk, &op)? else {
            return Ok(Some((op, value, s)));
        };
        match vault_next(sh, owner, &s, &op, &start.cov, &tx)? {
            Next::Continues(out, ns) => {
                op = TransactionOutpoint::new(tx.id, out.index);
                value = out.value;
                s = ns;
            }
            Next::Ended => return Ok(None),
        }
    }
    Err("Vault: zu viele Schritte".into())
}

/// Vaults, die über die Factory eröffnet wurden und hier noch fehlen
pub fn discover_vaults(h: &mut impl Chain, sh: &Shapes, d: &Deployment) -> Result<Vec<VaultRec>, String> {
    let Some(root) = d.ghost_root.as_ref() else { return Ok(vec![]) };
    let fspk = spk(&factory(&d.factory_params, &d.factory.state)).script().to_vec();
    let mut known: Vec<Hash> = d.vaults.iter().map(|v| v.vault.cov).collect();
    let mut found = vec![];
    let mut txs = h.txs(&fspk)?;
    txs.reverse(); // älteste zuerst, damit die Reihenfolge der Eröffnung erhalten bleibt
    for tx in &txs {
        // die Factory-Eingänge dieser Tx (Outpoints der Factory-Covenant)
        for i in &tx.inputs {
            if let Some(v) = vault_from_open(sh, &root.cov, &i.prev, &known, tx) {
                known.push(v.vault.cov);
                found.push(v);
            }
        }
    }
    Ok(found)
}
