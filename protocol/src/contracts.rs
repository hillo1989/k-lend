//! Verträge kompilieren und ihre Zustände als ABI-Werte darstellen.
//! Quellen werden eingebettet, damit Tests, CLI und Deployment dieselben
//! Bytes verwenden.

use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::ScriptPublicKey;
use kaspa_txscript::pay_to_script_hash_script;
use serde::{Deserialize, Serialize};
use silverscript_abi::{ArtifactValue, SilAbiArtifact};
use silverscript_lang::compiler::{CompileOptions, compile_to_sil_abi_artifact_with_options};
use std::collections::BTreeMap;

/// Version 4: Preis-Orakel ohne Signaturprüfung, das Quorum prüft das Register
pub const ORACLE_SRC: &str = include_str!("../../contracts/price_oracle_v4.sil");
/// Unterzeichner-Register (Version 4): Satz-Hash, Preis-Quorum, Austausch über Ticket
pub const REGISTER_SRC: &str = include_str!("../../contracts/signer_register_v4.sil");
pub const VAULT_SRC: &str = include_str!("../../contracts/stable_vault_v4.sil");
pub const FACTORY_SRC: &str = include_str!("../../contracts/vault_factory.sil");
/// GHOST: KCC20 aus SilverScript v1.0.0 plus Betragsprüfung (Version 2, Audit 1 V-01).
pub const GHOST_SRC: &str = include_str!("../../contracts/ghost_token.sil");
/// Dauerauftrag mit Tresor (unabhängig von GHOST-Deployments)
pub const STANDING_SRC: &str = include_str!("../../contracts/standing_order.sil");

pub const ID_PUBKEY: u8 = 0x00;
pub const ID_COV: u8 = 0x02;
/// Schleifengrenzen des GHOST-Tokens; müssen zu MAX_GHOST_INS/OUTS im Vault passen.
pub const GHOST_MAX_INS: i64 = 3;
pub const GHOST_MAX_OUTS: i64 = 2;

pub type Artifact = SilAbiArtifact;

pub fn compile(src: &str, args: Vec<ArtifactValue>) -> Artifact {
    compile_to_sil_abi_artifact_with_options(src, &args, CompileOptions::default()).expect("Vertrag kompiliert")
}

pub fn contract_name(a: &Artifact) -> String {
    a.contracts.keys().next().expect("ein Vertrag").clone()
}

pub fn bytecode(a: &Artifact) -> Vec<u8> {
    a.contracts.values().next().expect("ein Vertrag").compiled.bytecode.clone()
}

pub fn spk(a: &Artifact) -> ScriptPublicKey {
    pay_to_script_hash_script(&bytecode(a))
}

/// Template = Bytecode ohne den Zustandsbereich, plus dessen Hash.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Template {
    #[serde(with = "hex_bytes")]
    pub prefix: Vec<u8>,
    #[serde(with = "hex_bytes")]
    pub suffix: Vec<u8>,
    #[serde(with = "hex_bytes")]
    pub hash: Vec<u8>,
}

impl Template {
    pub fn of(a: &Artifact) -> Self {
        let c = a.contracts.values().next().expect("ein Vertrag");
        let span = c.compiled.state_span;
        let code = &c.compiled.bytecode;
        Self {
            prefix: code[..span.offset].to_vec(),
            suffix: code[span.offset + span.len..].to_vec(),
            hash: c.compiled.template_hash.to_vec(),
        }
    }
}

fn obj(fields: Vec<(&str, ArtifactValue)>) -> ArtifactValue {
    ArtifactValue::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<BTreeMap<_, _>>())
}

fn hash_bytes(h: &Hash) -> ArtifactValue {
    ArtifactValue::Bytes(h.as_bytes().to_vec())
}

// ------------------------------------------------------------------ Orakel ----

/// Feste Parameter des Preis-Orakels v4 (contracts/price_oracle_v4.sil)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleParams {
    /// Covenant-ID des Unterzeichner-Registers
    pub reg_cov: Hash,
    /// Obergrenze des Zinssatzes (je DAA × 1e18)
    pub max_rate: i64,
    /// größter Zinsschritt je Änderung
    pub rate_step: i64,
    /// Mindestabstand zweier Zinsänderungen (DAA)
    pub rate_gap_daa: i64,
    /// ohne Update so lange → jeder darf einfrieren (DAA)
    pub freeze_after_daa: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleState {
    pub kas_usd: i64,
    pub oracle_daa: i64,
    pub seq: i64,
    pub stable_rate: i64,
    pub stable_index: i64,
    /// eingefroren (veralteter Preis): Vault und Pool sperren
    pub frozen: bool,
    /// oracleDaa der letzten Zinsänderung
    pub last_rate_daa: i64,
}

pub fn oracle(p: &OracleParams, s: &OracleState) -> Artifact {
    compile(
        ORACLE_SRC,
        vec![
            hash_bytes(&p.reg_cov),
            ArtifactValue::Int(p.max_rate),
            ArtifactValue::Int(p.rate_step),
            ArtifactValue::Int(p.rate_gap_daa),
            ArtifactValue::Int(p.freeze_after_daa),
            ArtifactValue::Int(s.kas_usd),
            ArtifactValue::Int(s.oracle_daa),
            ArtifactValue::Int(s.seq),
            ArtifactValue::Int(s.stable_rate),
            ArtifactValue::Int(s.stable_index),
            ArtifactValue::Bool(s.frozen),
            ArtifactValue::Int(s.last_rate_daa),
        ],
    )
}

/// Kodierung wie SilverScript `x as byte[8]` (OpNum2Bin): Betrag little-endian,
/// Vorzeichen im höchsten Bit – NICHT Zweierkomplement. Für x ≥ 0 identisch mit
/// to_le_bytes (Mutationstest v2: negative Werte waren falsch kodiert).
pub fn script_num8(v: i64) -> [u8; 8] {
    let mut b = v.unsigned_abs().to_le_bytes();
    if v < 0 {
        b[7] |= 0x80;
    }
    b
}

pub const TAG_PRICE: u8 = 0x01;
pub const TAG_ROTATE: u8 = 0x02;
pub const TAG_CANCEL: u8 = 0x03;
pub const TAG_EMERG: u8 = 0x04;

fn sha256(m: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(m).into()
}

/// Nachricht, die die Unterzeichner für ein Preis-Update signieren
/// (signer_register_v4.sil attestPrice): sha256(oracleCov ‖ 0x01 ‖ kasUsd ‖ oracleDaa ‖ seq ‖ rate)
pub fn oracle_digest(oracle_cov: &Hash, kas_usd: i64, daa: i64, seq: i64, rate: i64) -> [u8; 32] {
    let mut m = oracle_cov.as_bytes().to_vec();
    m.push(TAG_PRICE);
    for v in [kas_usd, daa, seq, rate] {
        m.extend_from_slice(&script_num8(v));
    }
    sha256(&m)
}

/// Folgezustand eines Preis-Updates genau wie price_oracle_v4.sil update()
/// (None bei Überlauf des Index)
pub fn oracle_next_state(prev: &OracleState, kas_usd: i64, daa: i64, rate: i64) -> Option<OracleState> {
    Some(OracleState {
        kas_usd,
        oracle_daa: daa,
        seq: prev.seq + 1,
        stable_rate: rate,
        stable_index: oracle_next_index(prev, daa)?,
        frozen: false,
        last_rate_daa: if rate != prev.stable_rate { daa } else { prev.last_rate_daa },
    })
}

/// Index-Fortschreibung genau wie im Vertrag.
pub fn oracle_next_index(prev: &OracleState, new_daa: i64) -> Option<i64> {
    let delta = new_daa - prev.oracle_daa;
    let growth = prev.stable_rate.checked_mul(delta)? / 1_000_000_000;
    prev.stable_index.checked_add(prev.stable_index.checked_mul(growth)? / 1_000_000_000)
}

// ---------------------------------------------------------------- Register ----

/// Feste Parameter des Unterzeichner-Registers (contracts/signer_register_v4.sil).
/// Alle sind Template-Konstanten und nach dem Deployment unveränderlich.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterParams {
    /// nur für init (einmalig), danach ohne Rechte
    #[serde(with = "hex_bytes")]
    pub deployer: Vec<u8>,
    pub min_signers: i64,
    pub min_threshold: i64,
    /// Wartezeit eines regulären Austauschs (DAA, Ticket-Alter)
    pub rot_delay_daa: i64,
    /// Stille, ab der der Notfallsatz vorschlagen darf (DAA ab dem letzten Preis)
    pub emerg_after_daa: i64,
    /// Wartezeit eines Notfall-Austauschs (DAA, Ticket-Alter)
    pub emerg_delay_daa: i64,
}

/// Zustand einer Register-UTXO (Haupt-UTXO oder Ticket)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterState {
    pub ticket: bool,
    #[serde(with = "hex_bytes")]
    pub set_hash: Vec<u8>,
    /// Notfallsatz; 0…0 = keiner
    #[serde(with = "hex_bytes")]
    pub fb_hash: Vec<u8>,
    pub pay_kind: u8,
    #[serde(with = "hex_bytes")]
    pub pay_to: Vec<u8>,
    pub nonce: i64,
    pub emerg: bool,
    pub last_daa: i64,
    #[serde(with = "hex_bytes")]
    pub oracle_cov: Vec<u8>,
    /// Template-Hash des Orakels
    #[serde(with = "hex_bytes")]
    pub oracle_tpl: Vec<u8>,
    pub oracle_pre: i64,
    pub oracle_suf: i64,
    pub initialized: bool,
}

impl RegisterState {
    /// Genesis: Startsatz, Notfallsatz, Zinsziel; Orakel folgt per init
    pub fn genesis(set: &SignerSet, fb_hash: Option<[u8; 32]>, pay_kind: u8, pay_to: [u8; 32], last_daa: i64) -> Self {
        Self {
            ticket: false,
            set_hash: set.hash().to_vec(),
            fb_hash: fb_hash.unwrap_or([0; 32]).to_vec(),
            pay_kind,
            pay_to: pay_to.to_vec(),
            nonce: 0,
            emerg: false,
            last_daa,
            oracle_cov: vec![0; 32],
            oracle_tpl: vec![0; 32],
            oracle_pre: 0,
            oracle_suf: 0,
            initialized: false,
        }
    }
}

pub fn register(p: &RegisterParams, s: &RegisterState) -> Artifact {
    compile(
        REGISTER_SRC,
        vec![
            ArtifactValue::Bytes(p.deployer.clone()),
            ArtifactValue::Int(p.min_signers),
            ArtifactValue::Int(p.min_threshold),
            ArtifactValue::Int(p.rot_delay_daa),
            ArtifactValue::Int(p.emerg_after_daa),
            ArtifactValue::Int(p.emerg_delay_daa),
            ArtifactValue::Bool(s.ticket),
            ArtifactValue::Bytes(s.set_hash.clone()),
            ArtifactValue::Bytes(s.fb_hash.clone()),
            ArtifactValue::Byte(s.pay_kind),
            ArtifactValue::Bytes(s.pay_to.clone()),
            ArtifactValue::Int(s.nonce),
            ArtifactValue::Bool(s.emerg),
            ArtifactValue::Int(s.last_daa),
            ArtifactValue::Bytes(s.oracle_cov.clone()),
            ArtifactValue::Bytes(s.oracle_tpl.clone()),
            ArtifactValue::Int(s.oracle_pre),
            ArtifactValue::Int(s.oracle_suf),
            ArtifactValue::Bool(s.initialized),
        ],
    )
}

/// Ein Unterzeichner-Satz: n x-only-Schlüssel, Preis-Schwelle t, Austausch-Schwelle tRot
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignerSet {
    #[serde(with = "hex_vec")]
    pub keys: Vec<Vec<u8>>,
    pub t: i64,
    pub t_rot: i64,
}

impl SignerSet {
    pub fn n(&self) -> i64 {
        self.keys.len() as i64
    }
    /// sha256(n ‖ t ‖ tRot ‖ k0 ‖ … ‖ k(n−1)) wie hashSet im Register
    pub fn hash(&self) -> [u8; 32] {
        let mut m = vec![];
        for v in [self.n(), self.t, self.t_rot] {
            m.extend_from_slice(&script_num8(v));
        }
        for k in &self.keys {
            m.extend_from_slice(k);
        }
        sha256(&m)
    }
    /// Grenzen wie checkBounds im Register
    pub fn check_bounds(&self, p: &RegisterParams) -> Result<(), String> {
        let (n, t, r) = (self.n(), self.t, self.t_rot);
        if n < p.min_signers || n > MAX_SIGNERS {
            return Err(format!("Satz braucht {} bis {MAX_SIGNERS} Schlüssel, hat {n}", p.min_signers));
        }
        if t < p.min_threshold || 2 * t <= n || r < t || r > n || r > MAX_QUORUM {
            return Err(format!("Schwellen ungültig: t = {t}, tRot = {r} bei n = {n} (nötig: t ≥ {}, 2t > n, t ≤ tRot ≤ n, tRot ≤ {MAX_QUORUM})", p.min_threshold));
        }
        let mut seen = self.keys.clone();
        seen.sort();
        seen.dedup();
        if seen.len() != self.keys.len() || self.keys.iter().any(|k| k.len() != 32) {
            return Err("Satz enthält doppelte oder ungültige Schlüssel".into());
        }
        Ok(())
    }
    /// Argumente n, t, tRot, keys
    pub fn args(&self) -> Vec<ArtifactValue> {
        vec![
            ArtifactValue::Int(self.n()),
            ArtifactValue::Int(self.t),
            ArtifactValue::Int(self.t_rot),
            ArtifactValue::Array(self.keys.iter().cloned().map(ArtifactValue::Bytes).collect()),
        ]
    }
}

/// Schleifengrenzen des Registers (signer_register_v4.sil): höchstens 7
/// Schlüssel, höchstens 4 Signaturen je Prüfung (Mempool-Standardregel, 15
/// Signaturprüfungen je Eingang)
pub const MAX_SIGNERS: i64 = 7;
pub const MAX_QUORUM: i64 = 4;

/// Ankündigung eines Austauschs (Payload der propose-Tx): newSet ‖ newFb ‖ newKind ‖ newTo
pub fn announcement(set: &[u8; 32], fb: &[u8; 32], kind: u8, to: &[u8; 32]) -> Vec<u8> {
    let mut v = set.to_vec();
    v.extend_from_slice(fb);
    v.push(kind);
    v.extend_from_slice(to);
    v
}

/// Signierte Nachricht einer Ankündigung: sha256(regCov ‖ Tag ‖ nonce ‖ Ankündigung)
pub fn rotate_digest(reg_cov: &Hash, emergency: bool, nonce: i64, ann: &[u8]) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(if emergency { TAG_EMERG } else { TAG_ROTATE });
    m.extend_from_slice(&script_num8(nonce));
    m.extend_from_slice(ann);
    sha256(&m)
}

/// Signierte Nachricht einer Absage: sha256(regCov ‖ 0x03 ‖ nonce)
pub fn cancel_digest(reg_cov: &Hash, nonce: i64) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(TAG_CANCEL);
    m.extend_from_slice(&script_num8(nonce));
    sha256(&m)
}

// ------------------------------------------------------------------- GHOST ----

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GhostTok {
    #[serde(with = "hex_bytes")]
    pub owner: Vec<u8>,
    pub typ: u8,
    pub amount: i64,
    pub minter: bool,
}

impl GhostTok {
    pub fn minter_of(cov: &Hash) -> Self {
        Self { owner: cov.as_bytes().to_vec(), typ: ID_COV, amount: 0, minter: true }
    }
    pub fn to_pubkey(xonly: &[u8], amount: i64) -> Self {
        Self { owner: xonly.to_vec(), typ: ID_PUBKEY, amount, minter: false }
    }
    pub fn arg(&self) -> ArtifactValue {
        obj(vec![
            ("ownerIdentifier", ArtifactValue::Bytes(self.owner.clone())),
            ("identifierType", ArtifactValue::Byte(self.typ)),
            ("amount", ArtifactValue::Int(self.amount)),
            ("isMinter", ArtifactValue::Bool(self.minter)),
        ])
    }
    pub fn artifact(&self) -> Artifact {
        compile(
            GHOST_SRC,
            vec![
                ArtifactValue::Bytes(self.owner.clone()),
                ArtifactValue::Int(self.amount),
                ArtifactValue::Byte(self.typ),
                ArtifactValue::Bool(self.minter),
                ArtifactValue::Int(GHOST_MAX_INS),
                ArtifactValue::Int(GHOST_MAX_OUTS),
            ],
        )
    }
}

pub fn ghost_template() -> Template {
    Template::of(&GhostTok::minter_of(&Hash::from_bytes([0; 32])).artifact())
}

// -------------------------------------------------------------------- Vault ----

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VaultParams {
    pub oracle_cov: Hash,
    pub oracle_tpl: Template,
    pub ghost_cov: Hash,
    pub ghost_tpl: Template,
    pub mcr_bps: i64,
    pub liq_bps: i64,
    pub bonus_bps: i64,
    /// Höchstschuld je Vault beim Prägen, in GHOST-Einheiten
    #[serde(default = "default_max_debt")]
    pub max_debt: i64,
    /// Zinsziel (Version 4): scriptPubKey als Version (2 Byte BE) + Skript;
    /// bekommt den Zins beim Schließen und Auflösen in KAS
    #[serde(with = "hex_bytes")]
    pub interest_spk: Vec<u8>,
}

/// Zustand eines Vaults (Version 3, in v4 unverändert): geprägte GHOST, aufgelaufener Zins
/// (USD × 1e8) und Zinsindex der letzten Abrechnung
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultState {
    pub debt: i64,
    pub interest: i64,
    pub index_at: i64,
}

/// Mainnet-Start: 50 GHOST je Vault, Anzahl der Vaults unbegrenzt (Nutzer, 28.09.2026)
pub const MAINNET_MAX_DEBT: i64 = 5_000_000_000;
/// Mainnet-Probe v4: höchstens 5 GHOST je Vault (Nutzer, 04.10.2026)
pub const PROBE_MAX_DEBT: i64 = 500_000_000;
/// 1 Stunde bei 10 BPS
pub const HOUR_DAA: i64 = 36_000;
/// 1 Tag bei 10 BPS
pub const DAY_DAA: i64 = 864_000;
/// Praktisch unbegrenzt (Tests, alte Zustandsdateien vor der Grenze)
pub const NO_DEBT_LIMIT: i64 = i64::MAX;

fn default_max_debt() -> i64 {
    NO_DEBT_LIMIT
}

pub fn vault(p: &VaultParams, owner: &[u8], st: &VaultState) -> Artifact {
    compile(
        VAULT_SRC,
        vec![
            hash_bytes(&p.oracle_cov),
            ArtifactValue::Int(p.oracle_tpl.prefix.len() as i64),
            ArtifactValue::Int(p.oracle_tpl.suffix.len() as i64),
            ArtifactValue::Bytes(p.oracle_tpl.hash.clone()),
            hash_bytes(&p.ghost_cov),
            ArtifactValue::Int(p.ghost_tpl.prefix.len() as i64),
            ArtifactValue::Int(p.ghost_tpl.suffix.len() as i64),
            ArtifactValue::Bytes(p.ghost_tpl.hash.clone()),
            ArtifactValue::Int(p.mcr_bps),
            ArtifactValue::Int(p.liq_bps),
            ArtifactValue::Int(p.bonus_bps),
            ArtifactValue::Int(p.max_debt),
            ArtifactValue::Bytes(p.interest_spk.clone()),
            ArtifactValue::Bytes(owner.to_vec()),
            ArtifactValue::Int(st.debt),
            ArtifactValue::Int(st.interest),
            ArtifactValue::Int(st.index_at),
        ],
    )
}

/// scriptPubKey als Bytes wie tx.outputs[i].scriptPubKey im Vertrag: Version
/// (2 Byte big-endian, wie rusty-kaspa txscript/src/lib.rs to_bytes) + Skript
/// (Audit 14 N-3: vorher little-endian, bei Version 0 gleich)
pub fn spk_bytes(s: &ScriptPublicKey) -> Vec<u8> {
    let mut v = s.version().to_be_bytes().to_vec();
    v.extend_from_slice(s.script());
    v
}

/// Gegenstück zu spk_bytes
pub fn spk_from_bytes(b: &[u8]) -> Result<ScriptPublicKey, String> {
    if b.len() < 3 {
        return Err("Zinsziel: scriptPubKey zu kurz".into());
    }
    Ok(ScriptPublicKey::new(u16::from_be_bytes([b[0], b[1]]), b[2..].to_vec().into()))
}

// ------------------------------------------------------------------ Factory ----

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactoryParams {
    #[serde(with = "hex_bytes")]
    pub deployer: Vec<u8>,
    pub ghost_tpl: Template,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FactoryState {
    pub ghost_cov: Hash,
    #[serde(with = "hex_bytes")]
    pub vault_hash: Vec<u8>,
    pub initialized: bool,
}

impl FactoryState {
    pub fn uninitialized() -> Self {
        Self { ghost_cov: Hash::from_bytes([0; 32]), vault_hash: vec![0; 32], initialized: false }
    }
}

pub fn factory(p: &FactoryParams, s: &FactoryState) -> Artifact {
    compile(
        FACTORY_SRC,
        vec![
            ArtifactValue::Bytes(p.deployer.clone()),
            ArtifactValue::Int(p.ghost_tpl.prefix.len() as i64),
            ArtifactValue::Int(p.ghost_tpl.suffix.len() as i64),
            ArtifactValue::Bytes(p.ghost_tpl.hash.clone()),
            hash_bytes(&s.ghost_cov),
            ArtifactValue::Bytes(s.vault_hash.clone()),
            ArtifactValue::Bool(s.initialized),
        ],
    )
}

// ----------------------------------------------------- Dauerauftrag-Tresor ----

/// Feste Parameter eines Tresors (contracts/standing_order.sil)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TresorParams {
    /// Absender (x-only-Pubkey): kündigt und füllt auf
    #[serde(with = "hex_bytes")]
    pub owner: Vec<u8>,
    /// Empfänger (x-only-Pubkey): bekommt jede Zahlung als P2PK-Ausgang
    #[serde(with = "hex_bytes")]
    pub recipient: Vec<u8>,
    /// Betrag je Zahlung in sompi
    pub amount: i64,
    /// 1–31: monatlich an diesem Kalendertag (UTC); 0: festes Intervall
    pub anchor_day: i64,
    /// Intervall in ms bei anchor_day 0
    pub period_ms: i64,
    /// höchstens so viel Netzgebühr je Zahlung aus dem Tresor (sompi)
    pub max_fee: i64,
    /// sha256 des Payloads, den jede Zahlung tragen muss: die beim Anlegen
    /// hinterlegte Nachricht (öffentlich im Klartext oder die einmal an den
    /// Empfänger verschlüsselte Fassung), ohne Nachricht der Hash des leeren
    /// Payloads. Der Vertrag erzwingt ihn (Audit 12, A12-1 im Vertrag). Fehlt
    /// er (Tresor-Datei des alten Vertrags), bleibt er leer und
    /// `tresor::check_file` lehnt die Datei ab.
    #[serde(default, with = "hex_bytes")]
    pub payload_hash: Vec<u8>,
}

/// sha256 eines Payloads, wie `pay()` im Tresor-Vertrag ihn vergleicht
pub fn payload_hash(payload: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha256::digest(payload).to_vec()
}

/// Zustand eines Tresors: nächster Termin (Unix-ms, UTC) und verbleibende
/// Zahlungen (−1 = unbegrenzt)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TresorState {
    pub next_due: i64,
    pub left: i64,
}

pub fn standing_order(p: &TresorParams, s: &TresorState) -> Artifact {
    compile(
        STANDING_SRC,
        vec![
            ArtifactValue::Bytes(p.owner.clone()),
            ArtifactValue::Bytes(p.recipient.clone()),
            ArtifactValue::Int(p.amount),
            ArtifactValue::Int(p.anchor_day),
            ArtifactValue::Int(p.period_ms),
            ArtifactValue::Int(p.max_fee),
            ArtifactValue::Bytes(p.payload_hash.clone()),
            ArtifactValue::Int(s.next_due),
            ArtifactValue::Int(s.left),
        ],
    )
}

// =============================================================== Version 5 ----
//
// Verträge nach Audit 20 (docs/v5-entwurf.md). Noch ohne Anbindung an
// ghostctl; Bau der Transaktionen für Tests in crate::v5.

pub const ORACLE_V5_SRC: &str = include_str!("../../contracts/price_oracle_v5.sil");
pub const REGISTER_V5_SRC: &str = include_str!("../../contracts/signer_register_v5.sil");
pub const VAULT_V5_SRC: &str = include_str!("../../contracts/stable_vault_v5.sil");
pub const POOL_V5_SRC: &str = include_str!("../../contracts/ghost_pool_v5.sil");

pub const TAG_LOCK: u8 = 0x05;
/// Höchstzahl der Wächter (signer_register_v5.sil MAX_GUARDS)
pub const MAX_GUARDS: i64 = 3;

/// Feste Parameter des Preis-Orakels v5
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleV5Params {
    pub reg_cov: Hash,
    pub max_rate: i64,
    pub rate_step: i64,
    pub rate_gap_daa: i64,
    pub freeze_after_daa: i64,
    /// größter Sprung je Update in bps (×(1 + j) bzw. ÷(1 + j)); 10 000 = v4
    pub jump_bps: i64,
    /// Updates, bis ein Preis Referenzpreis wird
    pub ref_after: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OracleV5State {
    pub kas_usd: i64,
    pub oracle_daa: i64,
    pub seq: i64,
    pub stable_rate: i64,
    pub stable_index: i64,
    pub frozen: bool,
    pub last_rate_daa: i64,
    /// Referenzpreis (mindestens ref_after Updates öffentlich)
    pub ref_kas_usd: i64,
    /// Kandidat für den nächsten Referenzpreis und seine seq
    pub cand_kas_usd: i64,
    pub cand_seq: i64,
}

impl OracleV5State {
    /// Genesis: Referenz und Kandidat = Startpreis
    pub fn genesis(kas_usd: i64, daa: i64, rate: i64) -> Self {
        Self {
            kas_usd,
            oracle_daa: daa,
            seq: 0,
            stable_rate: rate,
            stable_index: 1_000_000_000,
            frozen: false,
            last_rate_daa: daa,
            ref_kas_usd: kas_usd,
            cand_kas_usd: kas_usd,
            cand_seq: 0,
        }
    }
}

pub fn oracle_v5(p: &OracleV5Params, s: &OracleV5State) -> Artifact {
    compile(
        ORACLE_V5_SRC,
        vec![
            hash_bytes(&p.reg_cov),
            ArtifactValue::Int(p.max_rate),
            ArtifactValue::Int(p.rate_step),
            ArtifactValue::Int(p.rate_gap_daa),
            ArtifactValue::Int(p.freeze_after_daa),
            ArtifactValue::Int(p.jump_bps),
            ArtifactValue::Int(p.ref_after),
            ArtifactValue::Int(s.kas_usd),
            ArtifactValue::Int(s.oracle_daa),
            ArtifactValue::Int(s.seq),
            ArtifactValue::Int(s.stable_rate),
            ArtifactValue::Int(s.stable_index),
            ArtifactValue::Bool(s.frozen),
            ArtifactValue::Int(s.last_rate_daa),
            ArtifactValue::Int(s.ref_kas_usd),
            ArtifactValue::Int(s.cand_kas_usd),
            ArtifactValue::Int(s.cand_seq),
        ],
    )
}

/// Sprunggrenze wie price_oracle_v5.sil update()
pub fn jump_ok(cur: i64, new: i64, jump_bps: i64) -> bool {
    let (c, n, j) = (cur as i128, new as i128, jump_bps as i128);
    n * 10_000 <= c * (10_000 + j) && n * (10_000 + j) >= c * 10_000
}

/// Folgezustand eines Preis-Updates genau wie price_oracle_v5.sil update()
/// (None bei Überlauf des Index)
pub fn oracle_v5_next_state(p: &OracleV5Params, prev: &OracleV5State, kas_usd: i64, daa: i64, rate: i64) -> Option<OracleV5State> {
    let delta = daa - prev.oracle_daa;
    let growth = prev.stable_rate.checked_mul(delta)? / 1_000_000_000;
    let index = prev.stable_index.checked_add(prev.stable_index.checked_mul(growth)? / 1_000_000_000)?;
    let seq = prev.seq + 1;
    let rotate = seq - prev.cand_seq >= p.ref_after;
    Some(OracleV5State {
        kas_usd,
        oracle_daa: daa,
        seq,
        stable_rate: rate,
        stable_index: index,
        frozen: false,
        last_rate_daa: if rate != prev.stable_rate { daa } else { prev.last_rate_daa },
        ref_kas_usd: if rotate { prev.cand_kas_usd } else { prev.ref_kas_usd },
        cand_kas_usd: if rotate { kas_usd } else { prev.cand_kas_usd },
        cand_seq: if rotate { seq } else { prev.cand_seq },
    })
}

/// Feste Parameter des Registers v5
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterV5Params {
    #[serde(with = "hex_bytes")]
    pub deployer: Vec<u8>,
    pub min_signers: i64,
    pub min_threshold: i64,
    pub rot_delay_daa: i64,
    pub emerg_after_daa: i64,
    pub emerg_delay_daa: i64,
    /// Mindestalter der Haupt-UTXO für attestPrice, propose, cancel (DAA)
    pub min_gap_daa: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisterV5State {
    pub ticket: bool,
    #[serde(with = "hex_bytes")]
    pub set_hash: Vec<u8>,
    #[serde(with = "hex_bytes")]
    pub fb_hash: Vec<u8>,
    /// Wächter; 0…0 = keine
    #[serde(with = "hex_bytes")]
    pub guard_hash: Vec<u8>,
    pub nonce: i64,
    pub emerg: bool,
    /// vom Wächter gesperrt
    pub locked: bool,
    pub last_daa: i64,
    #[serde(with = "hex_bytes")]
    pub oracle_cov: Vec<u8>,
    #[serde(with = "hex_bytes")]
    pub oracle_tpl: Vec<u8>,
    pub oracle_pre: i64,
    pub oracle_suf: i64,
    pub initialized: bool,
}

impl RegisterV5State {
    pub fn genesis(set: &SignerSet, fb_hash: Option<[u8; 32]>, guard_hash: Option<[u8; 32]>, last_daa: i64) -> Self {
        Self {
            ticket: false,
            set_hash: set.hash().to_vec(),
            fb_hash: fb_hash.unwrap_or([0; 32]).to_vec(),
            guard_hash: guard_hash.unwrap_or([0; 32]).to_vec(),
            nonce: 0,
            emerg: false,
            locked: false,
            last_daa,
            oracle_cov: vec![0; 32],
            oracle_tpl: vec![0; 32],
            oracle_pre: 0,
            oracle_suf: 0,
            initialized: false,
        }
    }
}

pub fn register_v5(p: &RegisterV5Params, s: &RegisterV5State) -> Artifact {
    compile(
        REGISTER_V5_SRC,
        vec![
            ArtifactValue::Bytes(p.deployer.clone()),
            ArtifactValue::Int(p.min_signers),
            ArtifactValue::Int(p.min_threshold),
            ArtifactValue::Int(p.rot_delay_daa),
            ArtifactValue::Int(p.emerg_after_daa),
            ArtifactValue::Int(p.emerg_delay_daa),
            ArtifactValue::Int(p.min_gap_daa),
            ArtifactValue::Bool(s.ticket),
            ArtifactValue::Bytes(s.set_hash.clone()),
            ArtifactValue::Bytes(s.fb_hash.clone()),
            ArtifactValue::Bytes(s.guard_hash.clone()),
            ArtifactValue::Int(s.nonce),
            ArtifactValue::Bool(s.emerg),
            ArtifactValue::Bool(s.locked),
            ArtifactValue::Int(s.last_daa),
            ArtifactValue::Bytes(s.oracle_cov.clone()),
            ArtifactValue::Bytes(s.oracle_tpl.clone()),
            ArtifactValue::Int(s.oracle_pre),
            ArtifactValue::Int(s.oracle_suf),
            ArtifactValue::Bool(s.initialized),
        ],
    )
}

/// Wächter: 1 bis 3 x-only-Schlüssel, jeder einzelne darf sperren
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardSet {
    #[serde(with = "hex_vec")]
    pub keys: Vec<Vec<u8>>,
}

impl GuardSet {
    /// sha256(n ‖ g0 ‖ … ‖ g(n−1)) wie guardLock im Register v5
    pub fn hash(&self) -> [u8; 32] {
        let mut m = script_num8(self.keys.len() as i64).to_vec();
        for k in &self.keys {
            m.extend_from_slice(k);
        }
        sha256(&m)
    }
    /// Argumente gn, gkeys
    pub fn args(&self) -> Vec<ArtifactValue> {
        vec![ArtifactValue::Int(self.keys.len() as i64), ArtifactValue::Array(self.keys.iter().cloned().map(ArtifactValue::Bytes).collect())]
    }
}

/// Ankündigung v5 (Payload der propose-Tx): newSet ‖ newFb ‖ newGuard
pub fn announcement_v5(set: &[u8; 32], fb: &[u8; 32], guard: &[u8; 32]) -> Vec<u8> {
    let mut v = set.to_vec();
    v.extend_from_slice(fb);
    v.extend_from_slice(guard);
    v
}

/// Signierte Nachricht einer Sperre: sha256(regCov ‖ 0x05 ‖ nonce)
pub fn lock_digest(reg_cov: &Hash, nonce: i64) -> [u8; 32] {
    let mut m = reg_cov.as_bytes().to_vec();
    m.push(TAG_LOCK);
    m.extend_from_slice(&script_num8(nonce));
    sha256(&m)
}

/// Vault v5: Parameter wie v4 plus Abwicklung
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VaultV5Params {
    pub base: VaultParams,
    /// Abwicklung ab so langer Stille des eingefrorenen Orakels (DAA)
    pub settle_after_daa: i64,
    /// Abschlag der Abwicklung in bps (bleibt im Vault)
    pub settle_fee_bps: i64,
}

pub fn vault_v5(p: &VaultV5Params, owner: &[u8], st: &VaultState) -> Artifact {
    let b = &p.base;
    compile(
        VAULT_V5_SRC,
        vec![
            hash_bytes(&b.oracle_cov),
            ArtifactValue::Int(b.oracle_tpl.prefix.len() as i64),
            ArtifactValue::Int(b.oracle_tpl.suffix.len() as i64),
            ArtifactValue::Bytes(b.oracle_tpl.hash.clone()),
            hash_bytes(&b.ghost_cov),
            ArtifactValue::Int(b.ghost_tpl.prefix.len() as i64),
            ArtifactValue::Int(b.ghost_tpl.suffix.len() as i64),
            ArtifactValue::Bytes(b.ghost_tpl.hash.clone()),
            ArtifactValue::Int(b.mcr_bps),
            ArtifactValue::Int(b.liq_bps),
            ArtifactValue::Int(b.bonus_bps),
            ArtifactValue::Int(b.max_debt),
            ArtifactValue::Bytes(b.interest_spk.clone()),
            ArtifactValue::Int(p.settle_after_daa),
            ArtifactValue::Int(p.settle_fee_bps),
            ArtifactValue::Bytes(owner.to_vec()),
            ArtifactValue::Int(st.debt),
            ArtifactValue::Int(st.interest),
            ArtifactValue::Int(st.index_at),
        ],
    )
}

/// Pool v5: gleiche Konstruktor-Argumente wie Pool v4 (Kursband Pflicht)
pub fn pool_v5(p: &crate::pool::PoolParams, lp_cov: &Hash, initialized: bool) -> Result<Artifact, String> {
    let b = p.band.as_ref().ok_or("Pool v5 braucht ein Kursband")?;
    Ok(compile(
        POOL_V5_SRC,
        vec![
            ArtifactValue::Bytes(p.ghost_cov.as_bytes().to_vec()),
            ArtifactValue::Int(p.tpl.prefix.len() as i64),
            ArtifactValue::Int(p.tpl.suffix.len() as i64),
            ArtifactValue::Bytes(p.tpl.hash.clone()),
            ArtifactValue::Int(p.fee_bps),
            ArtifactValue::Bytes(p.creator.clone()),
            ArtifactValue::Bytes(b.oracle_cov.as_bytes().to_vec()),
            ArtifactValue::Int(b.oracle_tpl.prefix.len() as i64),
            ArtifactValue::Int(b.oracle_tpl.suffix.len() as i64),
            ArtifactValue::Bytes(b.oracle_tpl.hash.clone()),
            ArtifactValue::Int(b.band_bps),
            ArtifactValue::Bool(b.stop_when_frozen),
            ArtifactValue::Bytes(lp_cov.as_bytes().to_vec()),
            ArtifactValue::Bool(initialized),
        ],
    ))
}

// ------------------------------------------------------------ serde-Helfer ----

pub mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&faster_hex::hex_string(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        let mut out = vec![0u8; s.len() / 2];
        faster_hex::hex_decode(s.as_bytes(), &mut out).map_err(serde::de::Error::custom)?;
        Ok(out)
    }
}

pub mod hex_vec {
    use serde::{Deserialize, Deserializer, Serializer, ser::SerializeSeq};
    pub fn serialize<S: Serializer>(v: &[Vec<u8>], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for b in v {
            seq.serialize_element(&faster_hex::hex_string(b))?;
        }
        seq.end()
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Vec<u8>>, D::Error> {
        let v = Vec::<String>::deserialize(d)?;
        v.into_iter()
            .map(|s| {
                let mut out = vec![0u8; s.len() / 2];
                faster_hex::hex_decode(s.as_bytes(), &mut out).map(|_| out).map_err(serde::de::Error::custom)
            })
            .collect()
    }
}
