//! Signieren mit Browser-Wallets (KasWare, Kastle) – Machbarkeitsprobe.
//!
//! ghostctl baut die Transaktion wie sonst, aber ohne Schlüsseldatei: Der
//! Signierer ist ein Wallet-Konto (Kaspa-Adresse = x-only-Pubkey). Die Wallet
//! bekommt die unsignierte Tx im „Safe JSON“ von kaspa-wasm
//! (`Transaction.serializeToSafeJSON`, rusty-kaspa a41a333
//! consensus/client/src/serializable/string.rs) und gibt sie signiert in
//! derselben Form zurück:
//! - Kastle: `kastle.signTx(networkId, txJson, scripts)` mit
//!   `scripts: [{inputIndex, scriptHex, signType: "All"}]` für Covenant-Eingänge;
//!   Signaturskript dann `createInputSignature(…)` (= `0x41 ‖ sig64 ‖ hashtype`,
//!   rusty-kaspa wallet/core/src/wasm/signer.rs, consensus/core/src/sign.rs
//!   `sign_input`) gefolgt vom Push des Redeem-Skripts (forbole/kastle
//!   lib/wallet/sign-script.ts). Eigene P2PK-Eingänge signiert Kastle danach
//!   mit `signTransaction` (Signaturskript `0x41 ‖ sig64 ‖ 0x01`).
//! - KasWare: `kasware.signPskt({txJsonString, options: {signInputs:
//!   [{index, sighashType: 1}]}})`; die Wallet liest mit
//!   `Transaction.deserializeFromSafeJSON` und gibt `serializeToSafeJSON()`
//!   zurück (kasware-wallet/extension src/background/controller/wallet.ts).
//!   Welche Form das Signaturskript eines fremden Eingangs bekommt, ist nicht
//!   belegt (privater kaspa-wasm-Fork) – `attach` nimmt deshalb den ersten
//!   Push mit 65 Byte, egal was folgt.
//!
//! Zur Reihenfolge: Bei Tx-Version 1 deckt der Sighash weder das
//! Compute-Budget der Eingänge noch die Speichermasse ab (rusty-kaspa
//! consensus/core/src/hashing/sighash.rs, Testvektoren
//! „native-v1-all-0-modify-compute-budget-*“; geprüft in
//! tests/wallet_probe_tests.rs). Budgets und Masse dürfen also nach dem
//! Signieren gesetzt werden. Die Gebühr dagegen steckt in den Ausgängen und
//! muss vorher feststehen. Gemessen wird mit einem Ersatzschlüssel gleicher
//! Länge (`mirror`): Skript-Einheiten hängen nicht vom Schlüssel ab (Test
//! `einheiten_unabhaengig_vom_schluessel`); `attach` misst trotzdem nach und
//! erhöht ein Budget, falls nötig, solange die Gebühr reicht.
//!
//! Die Probe (docs/wallet-probe.md) nutzt einen Tresor (standing_order.sil),
//! dessen Absender UND Empfänger das Wallet-Konto ist. ghostctl lehnt das bei
//! `tresor open` ab (`check_params`: „Empfänger ist der Absender selbst“), der
//! Vertrag nicht. Die Probe-Tresore stehen deshalb nur in einer eigenen
//! Probe-Datei (`Probe`), nie in deployments/.

use crate::contracts::*;
use crate::ops::{Tracked, genesis_output, p2pk_spk};
use crate::standing::DAY_MS;
use crate::tresor::{self, MIN_AMOUNT, MIN_MAX_FEE};
use crate::txb::{
    Built, Draft, In, MIN_CHANGE, Unlock, build, check_block_limits, check_scripts, masses, min_fee, push_redeem, run_input,
};
use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::constants::LOCK_TIME_THRESHOLD;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::mass::{ComputeBudget, ScriptUnits};
use kaspa_consensus_core::subnets::SubnetworkId;
use kaspa_consensus_core::tx::{
    CovenantBinding, MutableTransaction, ScriptPublicKey, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput, UtxoEntry,
};
use kaspa_txscript::script_builder::ScriptBuilder;
use kaspa_txscript::standard::extract_script_pub_key_address;
use secp256k1::{Keypair, Secp256k1, SecretKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use silverscript_abi::{ArtifactValue, encode_contract_entry_sig_script};

/// Betrag der einen Probe-Zahlung (kleinster, den ghostctl zulässt)
pub const PROBE_AMOUNT: i64 = MIN_AMOUNT; // 1 KAS
/// Startguthaben des Probe-Tresors (Recherche Abschnitt 5.3: ≈ 1,5 KAS)
pub const PROBE_FUND: u64 = 150_000_000;
/// Höchstgebühr je Zahlung: die kleinste, die ghostctl zulässt
pub const PROBE_MAX_FEE: i64 = MIN_MAX_FEE; // 0,004 KAS
/// So viel muss nach Betrag und Höchstgebühr mindestens im Tresor bleiben,
/// damit die Fortsetzung bei einer Zahlung (Sicherheitsnetz) kein winziger
/// Ausgang wird (Speichermasse, txb::check_block_limits)
pub const PROBE_MIN_REST: i64 = 25_000_000; // 0,25 KAS
/// Abstand der Termine (nur eine Zahlung, also nur Form)
pub const PROBE_PERIOD_MS: i64 = DAY_MS;
/// Startguthaben höchstens (Kleinstbeträge)
pub const PROBE_MAX_FUND: u64 = 1_000_000_000; // 10 KAS
/// Erster Termin: frühestens/spätestens so viele Minuten nach jetzt
pub const PROBE_DUE_MIN: i64 = 10;
pub const PROBE_DUE_MAX: i64 = 7 * 24 * 60;
/// Für die Trockenprobe: erfundene Tresor-UTXO
pub const DRY_VALUE: u64 = PROBE_FUND;
const DRY_TXID: [u8; 32] = [0xd7; 32];
const DRY_COV: [u8; 32] = [0xc0; 32];
/// Kennung der Dateien
pub const PLAN_KIND: &str = "ghost-wallet-plan:1";
pub const PROBE_KIND: &str = "ghost-wallet-probe:1";
/// höchstens so viele Wallet-UTXOs als Eingänge (der Dialog soll lesbar bleiben)
pub const MAX_WALLET_INPUTS: usize = 8;

// ------------------------------------------------------------ Safe JSON ----

/// u64 als Text (Safe JSON); beim Lesen auch als Zahl angenommen
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct U64Str(pub u64);

impl Serialize for U64Str {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for U64Str {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::String(s) if s.is_empty() => Ok(U64Str(0)),
            serde_json::Value::String(s) => s.parse().map(U64Str).map_err(serde::de::Error::custom),
            serde_json::Value::Number(n) => n.as_u64().map(U64Str).ok_or_else(|| serde::de::Error::custom("keine ganze Zahl ≥ 0")),
            _ => Err(serde::de::Error::custom("Zahl oder Text erwartet")),
        }
    }
}

pub mod strict_hex {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn decode(s: &str) -> Option<Vec<u8>> {
        if s.len() % 2 != 0 {
            return None;
        }
        let mut out = vec![0u8; s.len() / 2];
        faster_hex::hex_decode(s.as_bytes(), &mut out).ok()?;
        Some(out)
    }
    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&faster_hex::hex_string(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).ok_or_else(|| serde::de::Error::custom("ungültiges Hex"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeUtxo {
    /// Adresse (nur zur Anzeige in der Wallet; beim Lesen ignoriert)
    #[serde(default)]
    pub address: Option<serde_json::Value>,
    pub amount: U64Str,
    pub script_public_key: ScriptPublicKey,
    pub block_daa_score: U64Str,
    pub is_coinbase: bool,
    #[serde(default)]
    pub covenant_id: Option<Hash>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeInput {
    pub transaction_id: Hash,
    pub index: u32,
    pub sequence: U64Str,
    #[serde(default)]
    pub sig_op_count: u8,
    #[serde(default)]
    pub compute_budget: u16,
    #[serde(with = "strict_hex")]
    pub signature_script: Vec<u8>,
    pub utxo: SafeUtxo,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SafeCovenant {
    pub authorizing_input: u16,
    pub covenant_id: Hash,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeOutput {
    pub value: U64Str,
    pub script_public_key: ScriptPublicKey,
    #[serde(default)]
    pub covenant: Option<SafeCovenant>,
}

/// Transaktion im Safe JSON von kaspa-wasm (Feldnamen wie
/// consensus/client/src/serializable/string.rs, `SerializableTransaction`)
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeTx {
    pub id: Hash,
    pub version: u16,
    pub inputs: Vec<SafeInput>,
    pub outputs: Vec<SafeOutput>,
    pub subnetwork_id: SubnetworkId,
    pub lock_time: U64Str,
    pub gas: U64Str,
    #[serde(default, alias = "mass")]
    pub storage_mass: U64Str,
    #[serde(with = "strict_hex")]
    pub payload: Vec<u8>,
}

/// Tx + UTXOs → Safe JSON (Adressen zur Anzeige in der Wallet)
pub fn to_safe(tx: &Transaction, entries: &[UtxoEntry], prefix: Prefix) -> SafeTx {
    SafeTx {
        id: tx.id(),
        version: tx.version,
        inputs: tx
            .inputs
            .iter()
            .zip(entries)
            .map(|(i, e)| SafeInput {
                transaction_id: i.previous_outpoint.transaction_id,
                index: i.previous_outpoint.index,
                sequence: U64Str(i.sequence),
                sig_op_count: i.compute_commit.sig_op_count().unwrap_or(0),
                compute_budget: i.compute_commit.compute_budget().unwrap_or(0),
                signature_script: i.signature_script.clone(),
                utxo: SafeUtxo {
                    address: extract_script_pub_key_address(&e.script_public_key, prefix).ok().map(|a| serde_json::Value::String(a.to_string())),
                    amount: U64Str(e.amount),
                    script_public_key: e.script_public_key.clone(),
                    block_daa_score: U64Str(e.block_daa_score),
                    is_coinbase: e.is_coinbase,
                    covenant_id: e.covenant_id,
                },
            })
            .collect(),
        outputs: tx
            .outputs
            .iter()
            .map(|o| SafeOutput {
                value: U64Str(o.value),
                script_public_key: o.script_public_key.clone(),
                covenant: o.covenant.map(|c| SafeCovenant { authorizing_input: c.authorizing_input, covenant_id: c.covenant_id }),
            })
            .collect(),
        subnetwork_id: tx.subnetwork_id,
        lock_time: U64Str(tx.lock_time),
        gas: U64Str(tx.gas),
        storage_mass: U64Str(tx.storage_mass()),
        payload: tx.payload.clone(),
    }
}

/// Safe JSON → Tx + UTXOs. Nur Version 1 (Compute-Budget statt Sigop-Zahl).
pub fn from_safe(s: &SafeTx) -> Result<(Transaction, Vec<UtxoEntry>), String> {
    if s.version != 1 {
        return Err(format!("Tx-Version {} – erwartet 1 (Toccata)", s.version));
    }
    let mut inputs = vec![];
    let mut entries = vec![];
    for (i, x) in s.inputs.iter().enumerate() {
        if x.sig_op_count != 0 {
            return Err(format!("Eingang {i}: sigOpCount gesetzt – bei Version 1 nicht erlaubt"));
        }
        inputs.push(TransactionInput::new_with_compute_budget(
            TransactionOutpoint::new(x.transaction_id, x.index),
            x.signature_script.clone(),
            x.sequence.0,
            x.compute_budget,
        ));
        entries.push(UtxoEntry::new(x.utxo.amount.0, x.utxo.script_public_key.clone(), x.utxo.block_daa_score.0, x.utxo.is_coinbase, x.utxo.covenant_id));
    }
    let outputs = s
        .outputs
        .iter()
        .map(|o| TransactionOutput {
            value: o.value.0,
            script_public_key: o.script_public_key.clone(),
            covenant: o.covenant.as_ref().map(|c| CovenantBinding { authorizing_input: c.authorizing_input, covenant_id: c.covenant_id }),
        })
        .collect();
    let tx = Transaction::new(s.version, inputs, outputs, s.lock_time.0, s.subnetwork_id, s.gas.0, s.payload.clone());
    tx.set_storage_mass(s.storage_mass.0);
    let mut tx = tx;
    tx.finalize();
    Ok((tx, entries))
}

/// Antwort einer Wallet lesen: das Safe-JSON-Objekt selbst oder ein
/// JSON-Text, der es enthält (signTx/signPskt geben einen String zurück)
pub fn parse_signed(text: &str) -> Result<SafeTx, String> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| format!("Wallet-Antwort ist kein JSON: {e}"))?;
    let v = match v {
        serde_json::Value::String(s) => serde_json::from_str(&s).map_err(|e| format!("Wallet-Antwort (Text) ist kein JSON: {e}"))?,
        v => v,
    };
    serde_json::from_value(v).map_err(|e| format!("Wallet-Antwort hat nicht die Form einer Kaspa-Tx (Safe JSON): {e}"))
}

// ------------------------------------------------------------- Adressen ----

/// Kaspa-Adresse des Wallet-Kontos → x-only-Pubkey (nur Schnorr-P2PK)
pub fn xonly_of_address(addr: &str, prefix: Prefix) -> Result<Vec<u8>, String> {
    let a = Address::try_from(addr.trim()).map_err(|_| "keine gültige Kaspa-Adresse".to_string())?;
    if a.prefix != prefix {
        return Err(format!("Adresse gehört zu einem anderen Netz ({})", a.prefix));
    }
    if a.version != Version::PubKey || a.payload.len() != 32 {
        return Err("nur Schnorr-Adressen (kaspa:q…) werden unterstützt – ECDSA- und Skript-Adressen nicht".into());
    }
    secp256k1::XOnlyPublicKey::from_slice(&a.payload).map_err(|_| "Adresse enthält keinen gültigen Schlüssel".to_string())?;
    Ok(a.payload.to_vec())
}

pub fn address_of_xonly(x: &[u8], prefix: Prefix) -> String {
    Address::new(prefix, Version::PubKey, x).to_string()
}

// ------------------------------------------------------- Probe-Tresor ----

/// Ersatzschlüssel zum Messen: bei jedem Bau neu und zufällig (Audit 17
/// A17-1). Er signiert nur lokal die Messkopie, nie eine Tx, die hinausgeht.
/// Früher war er fest (0x42…) und damit öffentlich bekannt: GHOST an diesen
/// Schlüssel landeten in der Messkopie bei jedem Nutzer als „eigene“ Token und
/// sperrten alle Wallet-Aktionen mit GHOST. Zusätzlich neutralisiert
/// `wallet_ops::mirror_dep` alles, was dem Ersatzschlüssel schon gehört.
pub fn mirror_key() -> Keypair {
    if let Some(sk) = MIRROR_OVERRIDE.with(|c| c.get()) {
        return Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&sk).expect("Ersatzschlüssel (Test)"));
    }
    Keypair::new(&Secp256k1::new(), &mut secp256k1::rand::thread_rng())
}

/// x-only-Schlüssel eines Ersatzschlüssels
pub fn mirror_x_of(k: &Keypair) -> Vec<u8> {
    k.x_only_public_key().0.serialize().to_vec()
}

thread_local! {
    static MIRROR_OVERRIDE: std::cell::Cell<Option<[u8; 32]>> = const { std::cell::Cell::new(None) };
}

/// Nur für Tests (Gegenprobe A17-1): in diesem Thread einen bekannten
/// Ersatzschlüssel verwenden, als hätte ein Angreifer ihn erraten
#[doc(hidden)]
pub fn with_mirror_key<R>(sk: [u8; 32], f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            MIRROR_OVERRIDE.with(|c| c.set(None));
        }
    }
    MIRROR_OVERRIDE.with(|c| c.set(Some(sk)));
    let _r = Reset;
    f()
}

/// Parameter des Probe-Tresors: Absender = Empfänger = Wallet-Konto, eine
/// Zahlung zu 1 KAS, Höchstgebühr 0,004 KAS, ohne Nachricht
pub fn probe_params(owner: &[u8], first_due: i64) -> Result<(TresorParams, TresorState), String> {
    let p = TresorParams {
        owner: owner.to_vec(),
        recipient: owner.to_vec(),
        amount: PROBE_AMOUNT,
        anchor_day: 0,
        period_ms: PROBE_PERIOD_MS,
        max_fee: PROBE_MAX_FEE,
        payload_hash: payload_hash(&[]),
    };
    let s = TresorState { next_due: first_due, left: 1 };
    // alle übrigen Regeln von `tresor open`; nur „Empfänger = Absender“ ist
    // hier gewollt, deshalb mit einem anderen gültigen Empfänger geprüft
    let mut q = p.clone();
    q.recipient = mirror_x_of(&mirror_key());
    if q.recipient == q.owner {
        q.recipient = faster_hex_decode32("c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5");
    }
    tresor::check_params(&q, &s)?;
    Ok((p, s))
}

fn faster_hex_decode32(h: &str) -> Vec<u8> {
    strict_hex::decode(h).expect("fester Hex-Wert")
}

/// Ein Probe-Tresor (eigene Datei der Seite bzw. des Nutzers, nie deployments/)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub kind: String,
    pub network: String,
    pub cov: Hash,
    pub params: TresorParams,
    pub state: TresorState,
    /// "txid:index" der Tresor-UTXO, wie zuletzt bekannt
    pub outpoint: String,
    pub value: u64,
    /// erfundene UTXO der Trockenprobe (nie senden)
    #[serde(default)]
    pub fictional: bool,
}

impl Probe {
    pub fn tracked(&self) -> Result<Tracked<TresorState>, String> {
        Ok(Tracked { outpoint: tresor::parse_outpoint(&self.outpoint)?, value: self.value, cov: self.cov, state: self.state })
    }
    /// Datei lesen und auf Form prüfen
    pub fn decode(text: &str, network: &str) -> Result<Self, String> {
        let p: Probe = serde_json::from_str(text.trim()).map_err(|e| format!("Probe-Datei beschädigt: {e}"))?;
        if p.kind != PROBE_KIND {
            return Err("keine Probe-Datei (kind fehlt)".into());
        }
        if p.network != network {
            return Err(format!("Probe gehört zum Netz {}, gewählt ist {network}", p.network));
        }
        tresor::parse_outpoint(&p.outpoint)?;
        let (q, _) = probe_params(&p.params.owner, p.state.next_due.max(LOCK_TIME_THRESHOLD as i64))?;
        if q != p.params {
            return Err("Probe-Datei: Parameter sind nicht die eines Probe-Tresors".into());
        }
        if !(0..=1).contains(&p.state.left) {
            return Err("Probe-Datei: ungültiger Zustand".into());
        }
        Ok(p)
    }
}

// ----------------------------------------------------------------- Plan ----

/// Welcher Eingang wie signiert wird
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SignSpec {
    pub index: usize,
    /// "p2pk" (eigene KAS der Wallet) oder "tresor" (Covenant-Eingang)
    pub kind: String,
    /// Einstiegspunkt im Vertrag (nur tresor)
    #[serde(default)]
    pub entry: Option<String>,
    /// Position der Signatur im Argument-Array des Einstiegspunkts
    #[serde(default)]
    pub arg_pos: usize,
    /// Redeem-Skript (Hex, nur tresor) – Kastle braucht es als scriptHex
    #[serde(default)]
    pub redeem_hex: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub kind: String,
    /// dry-cancel | tresor-open | tresor-cancel
    pub stage: String,
    pub network: String,
    pub address: String,
    #[serde(with = "strict_hex")]
    pub owner: Vec<u8>,
    /// unsignierte Tx (Budgets und Speichermasse aus der Messung)
    pub tx: SafeTx,
    pub signers: Vec<SignSpec>,
    /// betroffener Probe-Tresor (bei tresor-open: Outpoint wird erst nach dem Signieren bekannt)
    pub probe: Probe,
    /// nie senden (Trockenprobe gegen eine erfundene UTXO)
    pub dry_only: bool,
    pub fee: u64,
    #[serde(default)]
    pub change_index: Option<u32>,
    pub used_units: Vec<u64>,
}

impl Plan {
    pub fn decode(text: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| format!("Plan ist kein JSON: {e}"))?;
        // auch die ganze Ausgabe von export-unsigned (Feld "plan")
        let v = match v.get("plan") {
            Some(p) if v.get("kind").is_none() => p.clone(),
            _ => v,
        };
        let p: Plan = serde_json::from_value(v).map_err(|e| format!("Plan beschädigt: {e}"))?;
        if p.kind != PLAN_KIND {
            return Err("kein Signierplan von ghostctl wallet export-unsigned".into());
        }
        if !matches!(p.stage.as_str(), "dry-cancel" | "tresor-open" | "tresor-cancel") {
            return Err("Plan: unbekannte Stufe".into());
        }
        Ok(p)
    }

    /// Anfrage an Kastle: signTx(networkId, txJson, scripts)
    pub fn kastle(&self) -> serde_json::Value {
        let scripts: Vec<_> = self
            .signers
            .iter()
            .filter(|s| s.kind == "tresor")
            .map(|s| serde_json::json!({ "inputIndex": s.index, "scriptHex": s.redeem_hex, "signType": "All" }))
            .collect();
        serde_json::json!({ "networkId": self.network, "txJson": serde_json::to_string(&self.tx).unwrap(), "scripts": scripts })
    }

    /// Anfrage an KasWare: signPskt({txJsonString, options:{signInputs}})
    pub fn kasware(&self) -> serde_json::Value {
        let sign: Vec<_> = self.signers.iter().map(|s| serde_json::json!({ "index": s.index, "sighashType": 1 })).collect();
        serde_json::json!({ "txJsonString": serde_json::to_string(&self.tx).unwrap(), "options": { "signInputs": sign } })
    }
}

/// Aus der Messkopie (mit Ersatzschlüssel signiert) die unsignierte echte Tx:
/// gleiche Tx, Signaturskripte leer, echte UTXOs
fn plan_from_mirror(
    stage: &str,
    network: &str,
    prefix: Prefix,
    owner: &[u8],
    mirror: Built,
    entries: Vec<UtxoEntry>,
    signers: Vec<SignSpec>,
    probe: Probe,
) -> Plan {
    let mut tx = mirror.tx.clone();
    for i in tx.inputs.iter_mut() {
        i.signature_script.clear();
    }
    tx.finalize();
    Plan {
        kind: PLAN_KIND.into(),
        stage: stage.into(),
        network: network.into(),
        address: address_of_xonly(owner, prefix),
        owner: owner.to_vec(),
        tx: to_safe(&tx, &entries, prefix),
        signers,
        dry_only: probe.fictional,
        probe,
        fee: mirror.fee,
        change_index: mirror.change_index,
        used_units: mirror.used_units,
    }
}

fn cancel_plan(stage: &str, network: &str, prefix: Prefix, probe: Probe, entry_real: UtxoEntry, net: &Params) -> Result<Plan, String> {
    let p = &probe.params;
    let t = probe.tracked()?;
    let real = standing_order(p, &t.state);
    if entry_real.script_public_key != spk(&real) || entry_real.covenant_id != Some(t.cov) || entry_real.amount != t.value {
        return Err("Tresor-UTXO passt nicht zu den Parametern".into());
    }
    let mirror = mirror_key();
    let mut pm = p.clone();
    pm.owner = mirror_x_of(&mirror);
    let to = p2pk_spk(&p.owner);
    let mk = |fee: u64| Draft {
        inputs: vec![tresor::tresor_input(&pm, &t, "cancel", Some(mirror.into()))],
        outputs: vec![TransactionOutput { value: t.value - fee, script_public_key: to.clone(), covenant: None }],
        change_spk: to.clone(),
        lock_time: 0,
    };
    let b = tresor::build_exact_fee(mk, (t.value / 2).min(MIN_CHANGE / 4), &[], net)?;
    let signers = vec![SignSpec {
        index: 0,
        kind: "tresor".into(),
        entry: Some("cancel".into()),
        arg_pos: 0,
        redeem_hex: Some(faster_hex::hex_string(&bytecode(&real))),
    }];
    let owner = p.owner.clone();
    Ok(plan_from_mirror(stage, network, prefix, &owner, b, vec![entry_real], signers, probe))
}

/// Stufe 0: Kündigung eines erfundenen Tresors (nichts im Netz, nie senden)
pub fn export_dry_cancel(owner: &[u8], network: &str, prefix: Prefix, now_ms: i64, net: &Params) -> Result<Plan, String> {
    let (p, s) = probe_params(owner, now_ms + 60 * 60_000)?;
    let probe = Probe {
        kind: PROBE_KIND.into(),
        network: network.into(),
        cov: Hash::from_bytes(DRY_COV),
        outpoint: tresor::outpoint_text(&TransactionOutpoint::new(Hash::from_bytes(DRY_TXID), 0)),
        value: DRY_VALUE,
        params: p.clone(),
        state: s,
        fictional: true,
    };
    let entry = UtxoEntry::new(DRY_VALUE, spk(&standing_order(&p, &s)), 0, false, Some(probe.cov));
    cancel_plan("dry-cancel", network, prefix, probe, entry, net)
}

/// Stufe 2: Kündigung des echten Probe-Tresors; `entry` = UTXO vom Node
pub fn export_cancel(probe: &Probe, entry: UtxoEntry, network: &str, prefix: Prefix, net: &Params) -> Result<Plan, String> {
    if probe.fictional {
        return Err("erfundener Tresor – das ist die Trockenprobe (dry-cancel)".into());
    }
    cancel_plan("tresor-cancel", network, prefix, probe.clone(), entry, net)
}

/// Stufe 1: Probe-Tresor anlegen, Startguthaben aus den UTXOs der Wallet
pub fn export_open(
    owner: &[u8],
    utxos: &[(TransactionOutpoint, UtxoEntry)],
    fund: u64,
    first_due: i64,
    network: &str,
    prefix: Prefix,
    net: &Params,
) -> Result<Plan, String> {
    let (p, s) = probe_params(owner, first_due)?;
    if fund > PROBE_MAX_FUND {
        return Err(format!("Startguthaben höchstens {} KAS (Probe mit Kleinstbeträgen)", PROBE_MAX_FUND / 100_000_000));
    }
    if (fund as i64) < p.amount + p.max_fee + PROBE_MIN_REST {
        return Err(format!(
            "Startguthaben zu klein: mindestens Betrag + Höchstgebühr + {} KAS = {:.8} KAS",
            PROBE_MIN_REST as f64 / 1e8,
            (p.amount + p.max_fee + PROBE_MIN_REST) as f64 / 1e8
        ));
    }
    let own = p2pk_spk(owner);
    let mut mine: Vec<_> = utxos.iter().filter(|(_, e)| e.script_public_key == own && e.covenant_id.is_none() && !e.is_coinbase).cloned().collect();
    mine.sort_by_key(|(o, e)| (std::cmp::Reverse(e.amount), o.transaction_id, o.index));
    // so wenige wie möglich: Startguthaben + Gebührenreserve
    let need = fund + MIN_CHANGE / 4;
    let mut chosen = vec![];
    let mut sum = 0u64;
    for u in mine {
        if sum >= need || chosen.len() >= MAX_WALLET_INPUTS {
            break;
        }
        sum += u.1.amount;
        chosen.push(u);
    }
    if sum < need {
        return Err(format!("zu wenig KAS auf der Wallet-Adresse: {:.8} KAS in bis zu {MAX_WALLET_INPUTS} UTXOs, nötig etwa {:.8} KAS", sum as f64 / 1e8, need as f64 / 1e8));
    }
    let art = standing_order(&p, &s);
    let first_in = chosen[0].0;
    let (out, cov) = genesis_output(&art, fund, 0, first_in, 0);
    // Messkopie: gleiche Tx, Eingänge mit P2PK des Ersatzschlüssels
    let mk = mirror_key();
    let mspk = p2pk_spk(&mirror_x_of(&mk));
    let inputs = chosen
        .iter()
        .map(|(op, e)| In { outpoint: *op, entry: UtxoEntry::new(e.amount, mspk.clone(), e.block_daa_score, e.is_coinbase, None), unlock: Unlock::P2pk { signer: mk.into() } })
        .collect();
    let b = build(Draft { inputs, outputs: vec![out], change_spk: own, lock_time: 0 }, net)?;
    let signers = (0..chosen.len()).map(|i| SignSpec { index: i, kind: "p2pk".into(), entry: None, arg_pos: 0, redeem_hex: None }).collect();
    let probe = Probe {
        kind: PROBE_KIND.into(),
        network: network.into(),
        cov,
        params: p,
        state: s,
        // Tx-ID steht erst nach dem Signieren fest (Budgets gehen in die ID ein)
        outpoint: tresor::outpoint_text(&TransactionOutpoint::new(Hash::from_bytes([0; 32]), 0)),
        value: fund,
        fictional: false,
    };
    Ok(plan_from_mirror("tresor-open", network, prefix, owner, b, chosen.into_iter().map(|(_, e)| e).collect(), signers, probe))
}

// --------------------------------------------------------- Signaturen ----

/// Erster Push des Signaturskripts mit genau 65 Byte (Schnorr-Signatur +
/// Hashtype). Kastle/KasWare/rusty-kaspa schreiben `0x41 ‖ 65 Byte`.
pub fn first_sig_push(script: &[u8]) -> Option<Vec<u8>> {
    if script.len() >= 66 && script[0] == 0x41 {
        return Some(script[1..66].to_vec());
    }
    None
}

/// Sighash (SIGHASH_ALL) eines Eingangs
pub fn sighash_all(tx: &Transaction, entries: &[UtxoEntry], idx: usize) -> [u8; 32] {
    let mtx = MutableTransaction::with_entries(tx.clone(), entries.to_vec());
    let reused = SigHashReusedValuesUnsync::new();
    calc_schnorr_signature_hash(&mtx.as_verifiable(), idx, SIG_HASH_ALL, &reused).as_bytes()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputReport {
    pub index: usize,
    pub kind: String,
    /// Signaturskript vorhanden
    pub signed: bool,
    /// Länge des Signaturskripts von der Wallet
    pub script_len: usize,
    /// Hashtype-Byte der Signatur (1 = ALL)
    pub hash_type: Option<u8>,
    /// Schnorr-Signatur passt zum Wallet-Schlüssel und zu dieser Tx
    pub sig_valid: bool,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// alles gültig: die Tx ginge so durch die Skriptprüfung
    pub valid: bool,
    /// Felder, die die Wallet verändert hat (außer Signaturskripten)
    pub changed: Vec<String>,
    /// verändert, aber nicht signiert und deshalb überschrieben
    pub ignored: Vec<String>,
    pub inputs: Vec<InputReport>,
    pub planned_units: Vec<u64>,
    pub used_units: Vec<u64>,
    pub budgets: Vec<u16>,
    pub budgets_raised: bool,
    pub fee: u64,
    pub min_fee: u64,
    pub compute_mass: u64,
    pub transient_mass: u64,
    pub storage_mass: u64,
    pub error: Option<String>,
}

/// Der Plan geht durch den Browser: vor dem Übernehmen nachprüfen, dass er
/// nur das tut, was eine Probe darf (alles an das Wallet-Konto oder in den
/// Probe-Tresor dieses Kontos)
pub fn check_plan(plan: &Plan, tx: &Transaction, entries: &[UtxoEntry]) -> Result<(), String> {
    let own = p2pk_spk(&plan.owner);
    let pr = &plan.probe;
    if pr.params.owner != plan.owner || pr.params.recipient != plan.owner {
        return Err("Plan: Tresor gehört nicht dem Wallet-Konto".into());
    }
    probe_params(&plan.owner, pr.state.next_due).and_then(|(p, _)| if p == pr.params { Ok(()) } else { Err("Plan: keine Probe-Parameter".into()) })?;
    if plan.dry_only != pr.fictional {
        return Err("Plan: Trockenprobe-Kennung widersprüchlich".into());
    }
    let tresor_spk = spk(&standing_order(&pr.params, &pr.state));
    for (i, o) in tx.outputs.iter().enumerate() {
        let ok = match o.covenant {
            Some(c) => plan.stage == "tresor-open" && i == 0 && o.script_public_key == tresor_spk && c.covenant_id == pr.cov && o.value == pr.value,
            None => o.script_public_key == own,
        };
        if !ok {
            return Err(format!("Plan: Ausgang {i} geht nicht an das Wallet-Konto bzw. den Probe-Tresor – abgelehnt"));
        }
    }
    if plan.signers.len() != tx.inputs.len() || plan.signers.iter().enumerate().any(|(i, s)| s.index != i) {
        return Err("Plan: jeder Eingang braucht genau eine Signatur der Wallet".into());
    }
    for (s, e) in plan.signers.iter().zip(entries) {
        let ok = match s.kind.as_str() {
            "p2pk" => e.script_public_key == own,
            "tresor" => plan.stage != "tresor-open" && e.script_public_key == tresor_spk && e.covenant_id == Some(pr.cov),
            _ => false,
        };
        if !ok {
            return Err(format!("Plan: Eingang {} gehört nicht dem Wallet-Konto", s.index));
        }
    }
    Ok(())
}

/// Unterschiede zwischen unsignierter Tx `u` und Wallet-Antwort `s` außer den
/// Signaturskripten der zu signierenden Eingänge. `strict_unsigned`: ein
/// Signaturskript an einem Eingang, den die Wallet nicht signieren sollte,
/// gilt als Veränderung (Probe); sonst wird es nur vermerkt und überschrieben
/// (wallet_ops: der Sighash deckt Signaturskripte nicht ab).
pub(crate) fn diff(u: &Transaction, ue: &[UtxoEntry], s: &Transaction, se: &[UtxoEntry], signers: &[SignSpec], strict_unsigned: bool) -> (Vec<String>, Vec<String>) {
    let mut changed = vec![];
    let mut ignored = vec![];
    if u.version != s.version {
        changed.push("Version".into());
    }
    if u.inputs.len() != s.inputs.len() {
        changed.push(format!("Anzahl Eingänge {} → {}", u.inputs.len(), s.inputs.len()));
        return (changed, ignored);
    }
    for (i, (a, b)) in u.inputs.iter().zip(&s.inputs).enumerate() {
        if a.previous_outpoint != b.previous_outpoint {
            changed.push(format!("Eingang {i}: Outpoint"));
        }
        if a.sequence != b.sequence {
            changed.push(format!("Eingang {i}: Sequence"));
        }
        if a.compute_commit != b.compute_commit {
            ignored.push(format!("Eingang {i}: Compute-Budget"));
        }
        if !signers.iter().any(|x| x.index == i) && !b.signature_script.is_empty() {
            if strict_unsigned {
                changed.push(format!("Eingang {i}: unerwartetes Signaturskript"));
            } else {
                ignored.push(format!("Eingang {i}: Signaturskript (nicht zu signieren, überschrieben)"));
            }
        }
    }
    for (i, (a, b)) in ue.iter().zip(se).enumerate() {
        if a.amount != b.amount || a.script_public_key != b.script_public_key || a.covenant_id != b.covenant_id {
            changed.push(format!("Eingang {i}: UTXO (Betrag/Skript/Covenant)"));
        }
    }
    if u.outputs.len() != s.outputs.len() {
        changed.push(format!("Anzahl Ausgänge {} → {}", u.outputs.len(), s.outputs.len()));
    } else {
        for (i, (a, b)) in u.outputs.iter().zip(&s.outputs).enumerate() {
            if a.value != b.value {
                changed.push(format!("Ausgang {i}: Betrag"));
            }
            if a.script_public_key != b.script_public_key {
                changed.push(format!("Ausgang {i}: Empfänger-Skript"));
            }
            if a.covenant != b.covenant {
                changed.push(format!("Ausgang {i}: Covenant-Bindung"));
            }
        }
    }
    if u.lock_time != s.lock_time {
        changed.push("Locktime".into());
    }
    if u.subnetwork_id != s.subnetwork_id {
        changed.push("Subnetz".into());
    }
    if u.gas != s.gas {
        changed.push("Gas".into());
    }
    if u.payload != s.payload {
        changed.push("Payload".into());
    }
    if u.storage_mass() != s.storage_mass() {
        ignored.push("Speichermasse".into());
    }
    (changed, ignored)
}

/// Signaturen der Wallet herauslesen (erster 65-Byte-Push je zu signierendem
/// Eingang) und prüfen: Hashtype 0x01 (ALL), Schnorr-Signatur gegen den
/// Schlüssel `owner` und den Sighash der UNSIGNIERTEN Tx `utx`. Gemeinsam für
/// die Probe (`attach`) und alle Aktionen (wallet_ops::submit).
pub fn read_sigs(utx: &Transaction, ue: &[UtxoEntry], stx: &Transaction, owner: &[u8], signers: &[SignSpec]) -> Result<(Vec<InputReport>, Vec<Option<Vec<u8>>>), String> {
    let owner = secp256k1::XOnlyPublicKey::from_slice(owner).map_err(|_| "Plan: ungültiger Schlüssel")?;
    let mut reports = vec![];
    let mut sigs = vec![];
    for sp in signers {
        if sp.index >= utx.inputs.len() {
            return Err(format!("Plan: Eingang {} gibt es nicht", sp.index));
        }
        let script = stx.inputs.get(sp.index).map(|i| i.signature_script.clone()).unwrap_or_default();
        let sig = first_sig_push(&script);
        let hash_type = sig.as_ref().map(|s| s[64]);
        let mut sig_valid = false;
        let note = match &sig {
            None if script.is_empty() => "Wallet hat diesen Eingang NICHT signiert (Signaturskript leer)".to_string(),
            None => "Signaturskript ohne 65-Byte-Signatur am Anfang".to_string(),
            Some(s) if s[64] != SIG_HASH_ALL.to_u8() => format!("Hashtype {:#04x} statt 0x01 (ALL) – abgelehnt", s[64]),
            Some(s) => {
                let h = sighash_all(utx, ue, sp.index);
                let ok = secp256k1::schnorr::Signature::from_slice(&s[..64])
                    .ok()
                    .and_then(|sg| secp256k1::Message::from_digest_slice(&h).ok().map(|m| sg.verify(&m, &owner).is_ok()))
                    .unwrap_or(false);
                sig_valid = ok;
                if ok { "Signatur passt zum Wallet-Schlüssel".into() } else { "Signatur passt NICHT (anderer Schlüssel oder andere Tx-Regeln)".into() }
            }
        };
        reports.push(InputReport { index: sp.index, kind: sp.kind.clone(), signed: !script.is_empty(), script_len: script.len(), hash_type, sig_valid, note });
        sigs.push(sig);
    }
    Ok((reports, sigs))
}

/// Wallet-Antwort übernehmen: prüfen, Signaturskripte bauen, Budgets und
/// Masse setzen, lokal wie der Konsens prüfen. Ok((Some(Tx), Bericht)) nur,
/// wenn alles gültig ist; sonst Bericht mit Grund.
pub fn attach(plan: &Plan, signed: &SafeTx, net: &Params) -> Result<(Option<Built>, Report), String> {
    let (utx, ue) = from_safe(&plan.tx)?;
    check_plan(plan, &utx, &ue)?;
    let mut rep = Report { planned_units: plan.used_units.clone(), ..Default::default() };
    let (stx, se) = match from_safe(signed) {
        Ok(x) => x,
        Err(e) => {
            rep.error = Some(format!("Wallet-Antwort: {e}"));
            return Ok((None, rep));
        }
    };
    let (changed, ignored) = diff(&utx, &ue, &stx, &se, &plan.signers, true);
    rep.changed = changed;
    rep.ignored = ignored;
    let (inputs, sigs) = read_sigs(&utx, &ue, &stx, &plan.owner, &plan.signers)?;
    rep.inputs = inputs;
    if !rep.changed.is_empty() {
        rep.error = Some(format!("Die Wallet hat die Tx verändert: {}", rep.changed.join(", ")));
        return Ok((None, rep));
    }
    if rep.inputs.iter().any(|i| !i.sig_valid) {
        rep.error = Some("Mindestens eine Signatur fehlt oder ist ungültig".into());
        return Ok((None, rep));
    }
    // Signaturskripte bauen
    let mut tx = utx.clone();
    for (sp, sig) in plan.signers.iter().zip(sigs) {
        let sig = sig.expect("geprüft");
        let script = match sp.kind.as_str() {
            "p2pk" => {
                if ue[sp.index].script_public_key != p2pk_spk(&plan.owner) {
                    return Err(format!("Plan: Eingang {} gehört nicht dem Wallet-Konto", sp.index));
                }
                ScriptBuilder::new().add_data(&sig).map_err(|e| e.to_string())?.drain()
            }
            "tresor" => {
                let pr = &plan.probe;
                if pr.params.owner != plan.owner {
                    return Err("Plan: Tresor gehört nicht dem Wallet-Konto".into());
                }
                let art = standing_order(&pr.params, &pr.state);
                if ue[sp.index].script_public_key != spk(&art) {
                    return Err(format!("Plan: Eingang {} ist nicht dieser Tresor", sp.index));
                }
                let entry = sp.entry.as_deref().ok_or("Plan: Einstiegspunkt fehlt")?;
                if !matches!(entry, "cancel" | "topUp") || sp.arg_pos != 0 {
                    return Err("Plan: Einstiegspunkt nicht unterstützt".into());
                }
                let mut s = encode_contract_entry_sig_script(&art, &contract_name(&art), entry, &[ArtifactValue::Bytes(sig)]).map_err(|e| format!("Sigscript: {e:?}"))?;
                s.extend(push_redeem(&art));
                s
            }
            k => return Err(format!("Plan: unbekannte Eingangsart {k}")),
        };
        tx.inputs[sp.index].signature_script = script;
    }
    // Einheiten messen, Budgets (nicht signiert) bei Bedarf erhöhen
    let mut budgets = vec![];
    let mut used = vec![];
    for i in 0..tx.inputs.len() {
        let u = match run_input(&tx, &ue, i, ScriptUnits(u64::MAX)) {
            Ok(u) => u,
            Err(e) => {
                rep.error = Some(format!("Skriptprüfung: {e}"));
                return Ok((None, rep));
            }
        };
        let need = ComputeBudget::checked_covering_script_units(ScriptUnits(u)).map(|b| b.value()).ok_or("Budget > u16")?;
        let have = tx.inputs[i].compute_commit.compute_budget().unwrap_or(0);
        if need > have {
            rep.budgets_raised = true;
        }
        budgets.push(need.max(have));
        used.push(u);
    }
    let inputs = tx
        .inputs
        .iter()
        .zip(&budgets)
        .map(|(i, b)| TransactionInput::new_with_compute_budget(i.previous_outpoint, i.signature_script.clone(), i.sequence, *b))
        .collect();
    let tx = Transaction::new(tx.version, inputs, tx.outputs.clone(), tx.lock_time, tx.subnetwork_id, tx.gas, tx.payload.clone());
    let (compute, transient, storage) = masses(&tx, &ue, net)?;
    tx.set_storage_mass(storage);
    let mut tx = tx;
    tx.finalize();
    let total_in: u64 = ue.iter().map(|e| e.amount).sum();
    let total_out: u64 = tx.outputs.iter().map(|o| o.value).sum();
    let fee = total_in.checked_sub(total_out).ok_or("Ausgänge > Eingänge")?;
    rep.used_units = used.clone();
    rep.budgets = budgets.clone();
    rep.fee = fee;
    rep.min_fee = min_fee(compute, transient);
    rep.compute_mass = compute;
    rep.transient_mass = transient;
    rep.storage_mass = storage;
    let checks = (|| -> Result<(), String> {
        if fee < rep.min_fee {
            return Err(format!("Gebühr {fee} sompi < Mindestgebühr {} sompi", rep.min_fee));
        }
        check_block_limits(compute, transient, storage)?;
        check_scripts(&tx, &ue)
    })();
    if let Err(e) = checks {
        rep.error = Some(e);
        return Ok((None, rep));
    }
    rep.valid = true;
    let built = Built {
        tx,
        entries: ue,
        fee,
        compute_mass: compute,
        transient_mass: transient,
        storage_mass: storage,
        budgets,
        used_units: used,
        donated: 0,
        change_index: plan.change_index,
    };
    Ok((Some(built), rep))
}

/// Probe-Tresor nach `tresor-open`: Outpoint aus der fertigen Tx
pub fn probe_after(plan: &Plan, b: &Built) -> Probe {
    let mut p = plan.probe.clone();
    if plan.stage == "tresor-open" {
        p.outpoint = tresor::outpoint_text(&TransactionOutpoint::new(b.tx.id(), 0));
        p.value = b.tx.outputs[0].value;
    }
    p
}

/// Sicherheitsnetz: die eine Zahlung des Probe-Tresors auslösen (darf jeder,
/// ohne Signatur). Die 1 KAS gehen an den Empfänger = das Wallet-Konto; die
/// Gebühr kommt aus dem Tresor (höchstens maxFee). Anders als `tresor pay`
/// ohne die 1-KAS-Regel (MIN_KEEP): Der Vertrag verlangt nur einen Rest > 0.
pub fn probe_pay(probe: &Probe, net: &Params) -> Result<Built, String> {
    if probe.fictional {
        return Err("erfundener Tresor – nichts zu zahlen".into());
    }
    let p = &probe.params;
    let t = probe.tracked()?;
    if t.state.left == 0 {
        return Err("die Zahlung ist schon erfolgt – den Rest holt nur das Kündigen (Stufe 2)".into());
    }
    let ns = tresor::next_state(p, &t.state);
    let keep = t.value as i64 - p.amount - p.max_fee;
    if keep <= 0 {
        return Err("Guthaben reicht nicht für Betrag und Höchstgebühr".into());
    }
    let pay_out = TransactionOutput { value: p.amount as u64, script_public_key: p2pk_spk(&p.recipient), covenant: None };
    let mk = |fee: u64| Draft {
        inputs: vec![tresor::tresor_input(p, &t, "pay", None)],
        outputs: vec![pay_out.clone(), tresor::cont(p, &ns, t.value - p.amount as u64 - fee, t.cov)],
        change_spk: p2pk_spk(&p.recipient),
        lock_time: t.state.next_due as u64,
    };
    let b = tresor::build_exact_fee(mk, p.max_fee as u64, &[], net)?;
    if b.change_index.is_some() || b.fee as i64 > p.max_fee {
        return Err("Gebühr über der Höchstgebühr".into());
    }
    Ok(b)
}

/// Probe nach einer Zahlung fortschreiben
pub fn probe_after_pay(probe: &Probe, b: &Built) -> Probe {
    let mut p = probe.clone();
    p.state = tresor::next_state(&probe.params, &probe.state);
    p.outpoint = tresor::outpoint_text(&TransactionOutpoint::new(b.tx.id(), 1));
    p.value = b.tx.outputs[1].value;
    p
}

/// Wofür jeder Ausgang ist (Anzeige vor dem Signieren)
pub fn describe_outputs(plan: &Plan, prefix: Prefix) -> Vec<serde_json::Value> {
    let Ok((tx, _)) = from_safe(&plan.tx) else { return vec![] };
    let own = p2pk_spk(&plan.owner);
    tx.outputs
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let what = if o.covenant.is_some() {
                "Probe-Tresor (Covenant, gehört dem Vertrag)"
            } else if o.script_public_key == own {
                if Some(i as u32) == plan.change_index { "Wechselgeld an die Wallet" } else { "zurück an die Wallet" }
            } else {
                "FREMDE Adresse"
            };
            serde_json::json!({
                "index": i,
                "sompi": o.value,
                "kas": o.value as f64 / 1e8,
                "address": extract_script_pub_key_address(&o.script_public_key, prefix).map(|a| a.to_string()).unwrap_or_else(|_| "?".into()),
                "what": what,
            })
        })
        .collect()
}
