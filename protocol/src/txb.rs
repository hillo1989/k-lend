//! Transaktionen bauen: Signaturen, Compute-Budget je Input, Speichermasse,
//! Gebühr. Die Regeln sind aus rusty-kaspa a41a333 übernommen:
//! - Konsens setzt je Input ein Skript-Limit aus dem Compute-Budget
//!   (tx_validation_in_utxo_context.rs, check_scripts_sequential) und bepreist
//!   Signaturprüfungen mit mass_per_sig_op = 1000 g.
//! - Die committete Speichermasse muss exakt der berechneten entsprechen
//!   (check_mass_commitment).
//! - Mindestgebühr = 100 sompi/g · max(compute, transient/2)
//!   (mining/src/mempool/check_transaction_standard.rs, Toccata-Leitfaden).

use crate::contracts::{Artifact, bytecode, contract_name};
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::hashing::sighash::{SigHashReusedValuesUnsync, calc_schnorr_signature_hash};
use kaspa_consensus_core::hashing::sighash_type::SIG_HASH_ALL;
use kaspa_consensus_core::mass::{ComputeBudget, Gram, MassCalculator, ScriptUnits};
use kaspa_consensus_core::subnets::SUBNETWORK_ID_NATIVE;
use kaspa_consensus_core::tx::{VerifiableTransaction,
    MutableTransaction, PopulatedTransaction, ScriptPublicKey, Transaction, TransactionInput, TransactionOutpoint, TransactionOutput,
    UtxoEntry,
};
use kaspa_txscript::caches::Cache;
use kaspa_txscript::covenants::CovenantsContext;
use kaspa_txscript::script_builder::ScriptBuilder;
use kaspa_txscript::{EngineCtx, EngineFlags, TxScriptEngine};
use secp256k1::{Keypair, Message};
use silverscript_abi::{ArtifactValue, encode_contract_covenant_decl_sig_script, encode_contract_entry_sig_script};

/// Mindest-Gebührensatz nach Toccata (Node-Policy, nicht Konsens).
pub const MIN_FEE_SOMPI_PER_GRAM: u64 = 100;
/// Sicherheitsaufschlag auf die Mindestgebühr (Promille).
pub const FEE_MARGIN_PERMILLE: u64 = 50;

/// Wer einen Eingang signiert: ein Schlüssel, den ghostctl hat, oder eine
/// Browser-Wallet, von der nur der x-only-Pubkey bekannt ist (KasWare,
/// Kastle; src/wallet_ops.rs). Bei `Wallet` setzt der Bau einen Platzhalter
/// bzw. die von der Wallet gelieferte Signatur ein (`with_wallet_fill`).
#[derive(Clone, Copy, Debug)]
pub enum Signer {
    Key(Keypair),
    Wallet([u8; 32]),
}

impl Signer {
    pub fn xonly(&self) -> Vec<u8> {
        match self {
            Signer::Key(k) => k.x_only_public_key().0.serialize().to_vec(),
            Signer::Wallet(x) => x.to_vec(),
        }
    }
    pub fn is_wallet(&self) -> bool {
        matches!(self, Signer::Wallet(_))
    }
}

impl From<Keypair> for Signer {
    fn from(k: Keypair) -> Self {
        Signer::Key(k)
    }
}
impl From<&Keypair> for Signer {
    fn from(k: &Keypair) -> Self {
        Signer::Key(*k)
    }
}
impl From<&Signer> for Signer {
    fn from(s: &Signer) -> Self {
        *s
    }
}

/// Wie ein Input aufgeschlossen wird.
pub enum Unlock {
    /// Handgeschriebener Einstiegspunkt; `sig_at` = Position im Argument-Array,
    /// an der die Schnorr-Signatur des Signierers eingesetzt wird.
    Entry { art: Artifact, entry: &'static str, args: Vec<ArtifactValue>, sig_at: Option<(usize, Signer)> },
    /// KCC20-Leader (`transfer`), optional mit Signatur des Besitzers.
    Leader { art: Artifact, new_states: Vec<ArtifactValue>, signer: Option<Signer> },
    /// KCC20-Delegate eines Pubkey-eigenen Token-UTXOs.
    Delegate { art: Artifact, signer: Signer },
    /// Normaler P2PK-Input (Gebühren, Einzahlungen).
    P2pk { signer: Signer },
    /// Fertiges Sigscript (z. B. leer für OpTrue-Skripte in Tests).
    Raw(Vec<u8>),
}

impl Unlock {
    /// Signierer dieses Eingangs, wenn er eine Browser-Wallet ist
    pub fn wallet_signer(&self) -> Option<[u8; 32]> {
        let s = match self {
            Unlock::Entry { sig_at, .. } => sig_at.map(|(_, s)| s),
            Unlock::Leader { signer, .. } => *signer,
            Unlock::Delegate { signer, .. } | Unlock::P2pk { signer } => Some(*signer),
            Unlock::Raw(_) => None,
        };
        match s {
            Some(Signer::Wallet(x)) => Some(x),
            _ => None,
        }
    }
    /// Art und Redeem-Skript für die Wallet (Kastle braucht es als scriptHex)
    fn wallet_spec(&self, index: usize) -> WalletInput {
        let (kind, entry, arg_pos, art) = match self {
            Unlock::Entry { art, entry, sig_at, .. } => ("entry", Some(entry.to_string()), sig_at.map(|(p, _)| p).unwrap_or(0), Some(art)),
            Unlock::Leader { art, .. } => ("leader", Some("transfer".to_string()), 1, Some(art)),
            Unlock::Delegate { art, .. } => ("delegate", Some("transfer".to_string()), 0, Some(art)),
            Unlock::P2pk { .. } | Unlock::Raw(_) => ("p2pk", None, 0, None),
        };
        WalletInput { index, kind: kind.into(), entry, arg_pos, redeem: art.map(bytecode) }
    }
}

/// Ein Eingang, den die Browser-Wallet signiert
#[derive(Clone, Debug, PartialEq)]
pub struct WalletInput {
    pub index: usize,
    /// p2pk | entry | leader | delegate
    pub kind: String,
    pub entry: Option<String>,
    /// Position der Signatur im Argument-Array
    pub arg_pos: usize,
    /// Redeem-Skript (nur Covenant-Eingänge)
    pub redeem: Option<Vec<u8>>,
}

/// Vorgaben für einen Bau mit Wallet-Signierern: Budgets und Gebühr aus der
/// Messkopie, Signaturen (65 Byte, Hashtype am Ende) je Eingang, sobald die
/// Wallet geantwortet hat. Ohne Signatur steht ein Platzhalter gleicher
/// Länge im Skript; diese Tx besteht die Skriptprüfung nicht und geht nur
/// (mit geleerten Signaturskripten) an die Wallet.
#[derive(Clone, Debug, Default)]
pub struct WalletFill {
    pub budgets: Vec<u16>,
    pub fee: u64,
    pub sigs: Vec<Option<Vec<u8>>>,
}

/// Was ein Bau mit Wallet-Signierern zurückmeldet
#[derive(Clone, Debug, Default)]
pub struct WalletBuilt {
    pub inputs: Vec<WalletInput>,
    /// Bau ist gelaufen (genau einmal je with_wallet_fill)
    pub used: bool,
}

thread_local! {
    static WALLET_FILL: std::cell::RefCell<Option<(WalletFill, WalletBuilt)>> = const { std::cell::RefCell::new(None) };
}

/// Führt `op` (eine Aktion aus ops/pool) mit Wallet-Vorgaben aus. Der Bau
/// darin nimmt Budgets, Gebühr und Signaturen aus `fill`, statt zu messen,
/// und prüft keine Skripte – das macht der Aufrufer (wallet_ops::finish).
pub fn with_wallet_fill<R>(fill: WalletFill, op: impl FnOnce() -> R) -> (R, WalletBuilt) {
    WALLET_FILL.with(|c| *c.borrow_mut() = Some((fill, WalletBuilt::default())));
    let r = op();
    let wb = WALLET_FILL.with(|c| c.borrow_mut().take()).map(|(_, w)| w).unwrap_or_default();
    (r, wb)
}

/// Platzhalter für eine fehlende Wallet-Signatur (Länge wie Schnorr + Hashtype)
pub const SIG_PLACEHOLDER: [u8; 65] = [0u8; 65];

pub struct In {
    pub outpoint: TransactionOutpoint,
    pub entry: UtxoEntry,
    pub unlock: Unlock,
}

pub struct Draft {
    pub inputs: Vec<In>,
    pub outputs: Vec<TransactionOutput>,
    /// Wohin das Wechselgeld geht (P2PK des Gebührenzahlers).
    pub change_spk: ScriptPublicKey,
    pub lock_time: u64,
}

#[derive(Debug, Clone)]
pub struct Built {
    pub tx: Transaction,
    pub entries: Vec<UtxoEntry>,
    pub fee: u64,
    pub compute_mass: u64,
    pub transient_mass: u64,
    pub storage_mass: u64,
    pub budgets: Vec<u16>,
    pub used_units: Vec<u64>,
    /// Restbetrag unter MIN_CHANGE, der zusätzlich als Gebühr an die Miner geht
    pub donated: u64,
    /// Index des Wechselgeld-Ausgangs (gehört dem Gebührenzahler; Dritte können
    /// ihn nicht ausgeben – store::resolve_pending stützt sich darauf)
    pub change_index: Option<u32>,
}

/// Blockgrenzen (Mainnet/TN10, Toccata): eine Tx darf sie allein nicht
/// überschreiten, sonst nimmt kein Block sie auf (Audit 4 F7: der Simulator
/// hatte eine Tx mit 1 003 783 g Speichermasse akzeptiert).
pub const MAX_BLOCK_COMPUTE_MASS: u64 = 500_000;
pub const MAX_BLOCK_STORAGE_MASS: u64 = 500_000;
pub const MAX_BLOCK_TRANSIENT_MASS: u64 = 1_000_000;

pub fn check_block_limits(compute: u64, transient: u64, storage: u64) -> Result<(), String> {
    if compute > MAX_BLOCK_COMPUTE_MASS || storage > MAX_BLOCK_STORAGE_MASS || transient > MAX_BLOCK_TRANSIENT_MASS {
        return Err(format!(
            "Transaktion zu groß für einen Block (compute {compute} g, storage {storage} g, transient {transient} g). Häufige Ursache: ein sehr kleiner Ausgang – Beträge unter etwa 0,2 KAS vermeiden."
        ));
    }
    Ok(())
}

/// Mempool-Standardregel (rusty-kaspa a41a333,
/// mining/src/mempool/check_transaction_standard.rs MAX_STANDARD_P2SH_SIG_OPS):
/// höchstens so viele Signaturprüfungen je P2SH-Eingang, STATISCH im ganzen
/// Redeem-Skript gezählt (post_toccata_p2sh_sig_scanner: jede OpCheckSig* und
/// OpCheckSigFromStack*, auch in nie ausgeführten Zweigen und ausgerollten
/// Schleifen). Kein Konsens, aber ohne sie leitet kein Node die Tx weiter
/// (Mainnet-Probe 05.10.2026: Register-Init mit 28 abgelehnt).
pub const MAX_STANDARD_P2SH_SIG_OPS: u64 = 15;

/// Prüft die Signaturprüfungen je Eingang wie der Mempool
pub fn check_standard_sig_ops(tx: &Transaction, entries: &[UtxoEntry]) -> Result<(), String> {
    for (i, (input, e)) in tx.inputs.iter().zip(entries).enumerate() {
        let n = kaspa_txscript::post_toccata_p2sh_sig_scanner(&input.signature_script, &e.script_public_key);
        if n > MAX_STANDARD_P2SH_SIG_OPS {
            return Err(format!(
                "Eingang {i}: {n} Signaturprüfungen im Skript, der Mempool erlaubt höchstens {MAX_STANDARD_P2SH_SIG_OPS} je Eingang (Standardregel)"
            ));
        }
    }
    Ok(())
}

/// Wechselgeld darunter wird nicht als eigener Ausgang angelegt (KIP-9:
/// kleine Ausgänge treiben die Speichermasse hoch), sondern verschenkt.
pub const MIN_CHANGE: u64 = 20_000_000;

pub fn engine_flags() -> EngineFlags {
    // mass_per_sig_op = 1000 g in Mainnet- und Testnet-Parametern
    EngineFlags { covenants_enabled: true, sigop_script_units: Gram(1000).into() }
}

/// Führt einen Input aus und liefert die verbrauchten Skript-Einheiten.
pub fn run_input(tx: &Transaction, entries: &[UtxoEntry], idx: usize, limit: ScriptUnits) -> Result<u64, String> {
    let reused = SigHashReusedValuesUnsync::new();
    let cache = Cache::new(1_000);
    let populated = PopulatedTransaction::new(tx, entries.to_vec());
    let cov_ctx = CovenantsContext::from_tx(&populated).map_err(|e| format!("Covenant-Kontext: {e:?}"))?;
    let utxo = populated.utxo(idx).ok_or("Input fehlt")?;
    let mut vm = TxScriptEngine::from_transaction_input_with_script_units_limit(
        &populated,
        &tx.inputs[idx],
        idx,
        utxo,
        EngineCtx::new(&cache).with_reused(&reused).with_covenants_ctx(&cov_ctx),
        engine_flags(),
        limit,
    );
    vm.execute().map_err(|e| format!("Input {idx}: {e:?}"))?;
    Ok(vm.used_script_units().0)
}

/// Prüft alle Inputs mit den Limits, die ihr Compute-Budget erlaubt (wie der Konsens).
pub fn check_scripts(tx: &Transaction, entries: &[UtxoEntry]) -> Result<(), String> {
    for i in 0..tx.inputs.len() {
        run_input(tx, entries, i, tx.inputs[i].compute_commit.allowed_script_units())?;
    }
    Ok(())
}

pub fn masses(tx: &Transaction, entries: &[UtxoEntry], params: &Params) -> Result<(u64, u64, u64), String> {
    let calc = MassCalculator::new_with_consensus_params(params);
    let nc = calc.calc_non_contextual_masses(tx);
    let populated = PopulatedTransaction::new(tx, entries.to_vec());
    let c = calc.calc_contextual_masses(&populated).ok_or("Speichermasse nicht berechenbar")?;
    Ok((nc.compute_mass, nc.transient_mass, c.storage_mass))
}

pub fn min_fee(compute: u64, transient: u64) -> u64 {
    compute.max(transient.div_ceil(2)) * MIN_FEE_SOMPI_PER_GRAM
}

fn signature(tx: &Transaction, entries: &[UtxoEntry], idx: usize, s: &Signer) -> Vec<u8> {
    match s {
        Signer::Key(k) => sign_input(tx, entries, idx, k),
        Signer::Wallet(_) => WALLET_FILL
            .with(|c| c.borrow().as_ref().and_then(|(f, _)| f.sigs.get(idx).cloned().flatten()))
            .unwrap_or_else(|| SIG_PLACEHOLDER.to_vec()),
    }
}

fn sign_input(tx: &Transaction, entries: &[UtxoEntry], idx: usize, k: &Keypair) -> Vec<u8> {
    let mtx = MutableTransaction::with_entries(tx.clone(), entries.to_vec());
    let reused = SigHashReusedValuesUnsync::new();
    let h = calc_schnorr_signature_hash(&mtx.as_verifiable(), idx, SIG_HASH_ALL, &reused);
    let mut s = k.sign_schnorr(Message::from_digest_slice(h.as_bytes().as_slice()).unwrap()).as_ref().to_vec();
    s.push(SIG_HASH_ALL.to_u8());
    s
}

pub fn push_redeem(art: &Artifact) -> Vec<u8> {
    ScriptBuilder::with_flags(engine_flags()).add_data(&bytecode(art)).expect("Redeem-Skript").drain()
}

fn sigscript(unlock: &Unlock, tx: &Transaction, entries: &[UtxoEntry], idx: usize) -> Vec<u8> {
    match unlock {
        Unlock::Entry { art, entry, args, sig_at } => {
            let mut args = args.clone();
            if let Some((pos, k)) = sig_at {
                args.insert(*pos, ArtifactValue::Bytes(signature(tx, entries, idx, k)));
            }
            let mut s = encode_contract_entry_sig_script(art, &contract_name(art), entry, &args).expect("Sigscript");
            s.extend(push_redeem(art));
            s
        }
        Unlock::Leader { art, new_states, signer } => {
            let sig = signer.as_ref().map(|k| signature(tx, entries, idx, k)).unwrap_or(vec![0; 65]);
            let args = vec![ArtifactValue::Array(new_states.clone()), ArtifactValue::Bytes(sig), ArtifactValue::Byte(0)];
            let mut s = encode_contract_covenant_decl_sig_script(art, &contract_name(art), "transfer", true, &args).expect("Leader");
            s.extend(push_redeem(art));
            s
        }
        Unlock::Delegate { art, signer } => {
            let args = vec![ArtifactValue::Bytes(signature(tx, entries, idx, signer)), ArtifactValue::Byte(0)];
            let mut s = encode_contract_covenant_decl_sig_script(art, &contract_name(art), "transfer", false, &args).expect("Delegate");
            s.extend(push_redeem(art));
            s
        }
        Unlock::P2pk { signer } => ScriptBuilder::new().add_data(&signature(tx, entries, idx, signer)).expect("P2PK").drain(),
        Unlock::Raw(s) => s.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn assemble(d: &Draft, payload: &[u8], seqs: &[u64], budgets: &[u16], fee: u64, entries: &[UtxoEntry], params: &Params) -> Result<(Transaction, u64, Option<u32>), String> {
    if d.outputs.iter().any(|o| o.value == 0) {
        return Err("Ausgang mit 0 sompi ist nicht erlaubt".into());
    }
    let total_in: u64 = entries.iter().map(|e| e.amount).sum();
    let total_out: u64 = d.outputs.iter().map(|o| o.value).sum();
    let rest = total_in.checked_sub(total_out).and_then(|r| r.checked_sub(fee)).ok_or_else(|| {
        format!("Zu wenig KAS: Eingänge {total_in}, Ausgänge {total_out}, Gebühr {fee} sompi")
    })?;
    let mut outputs = d.outputs.clone();
    let mut change_index = None;
    let paid_fee = if rest >= MIN_CHANGE {
        change_index = Some(outputs.len() as u32);
        outputs.push(TransactionOutput { value: rest, script_public_key: d.change_spk.clone(), covenant: None });
        fee
    } else {
        fee + rest
    };
    let inputs = d
        .inputs
        .iter()
        .zip(budgets)
        .enumerate()
        .map(|(n, (i, b))| TransactionInput::new_with_compute_budget(i.outpoint, vec![], seqs.get(n).copied().unwrap_or(0), *b))
        .collect();
    // Payload zählt in die Masse (transaction_estimated_serialized_size) und
    // in den Sighash (payload_hash), deshalb vor dem Signieren setzen
    let tx = Transaction::new(1, inputs, outputs, d.lock_time, SUBNETWORK_ID_NATIVE, 0, payload.to_vec());
    let (_, _, storage) = masses(&tx, entries, params)?;
    tx.set_storage_mass(storage);
    // Signaturen über die fertige Tx. Budget und Speichermasse deckt der
    // v1-Sighash nicht ab (tests/wallet_probe_tests.rs), die Reihenfolge
    // schadet aber nicht
    let scripts: Vec<Vec<u8>> = d.inputs.iter().enumerate().map(|(i, input)| sigscript(&input.unlock, &tx, entries, i)).collect();
    let mut tx = tx;
    for (input, s) in tx.inputs.iter_mut().zip(scripts) {
        input.signature_script = s;
    }
    tx.finalize();
    Ok((tx, paid_fee, change_index))
}

/// Baut die Tx so oft neu, bis Budgets und Gebühr stabil sind.
pub fn build(d: Draft, params: &Params) -> Result<Built, String> {
    build_with_payload(d, &[], params)
}

/// Höchstlänge der Nutzdaten (Payload) normaler Transaktionen, die ghostctl
/// baut. Der Konsens (rusty-kaspa a41a333) begrenzt den Payload nicht
/// eigens, er zählt aber jedes Byte in die Masse: 1 g Compute und 4 g
/// transient je Byte (mass/mod.rs). 464 Byte = 100 Zeichen UTF-8 (≤ 400 Byte)
/// plus 64 Byte für die Verschlüsselung (message::OVERHEAD); gemessen in
/// tests/payload_tests.rs.
pub const MAX_PAYLOAD: usize = crate::message::MAX_ENCRYPTED;

/// Wie `build`, mit Nutzdaten (z. B. einer Nachricht in UTF-8) im Payload.
pub fn build_with_payload(d: Draft, payload: &[u8], params: &Params) -> Result<Built, String> {
    build_ext(d, payload, &[], params)
}

/// Wie `build_with_payload`, dazu die Sequenz je Eingang (relative Sperre in
/// DAA, für `this.ageDaa` im Vertrag; fehlende Einträge = 0). Der Konsens
/// nimmt die Tx erst, wenn DAA der UTXO + Sequenz − 1 < DAA des Blocks
/// (rusty-kaspa a41a333, tx_validation_in_utxo_context.rs check_sequence_lock).
pub fn build_ext(d: Draft, payload: &[u8], seqs: &[u64], params: &Params) -> Result<Built, String> {
    if payload.len() > MAX_PAYLOAD {
        return Err(format!("Nutzdaten zu lang: {} Byte (höchstens {MAX_PAYLOAD})", payload.len()));
    }
    let entries: Vec<UtxoEntry> = d.inputs.iter().map(|i| i.entry.clone()).collect();
    if d.inputs.iter().any(|i| i.unlock.wallet_signer().is_some()) {
        return build_for_wallet(&d, payload, seqs, &entries, params);
    }
    let mut budgets = vec![0u16; d.inputs.len()];
    let mut fee = 0u64;
    for _round in 0..6 {
        let (tx, paid, change_index) = assemble(&d, payload, seqs, &budgets, fee, &entries, params)?;
        let mut used = vec![];
        for i in 0..tx.inputs.len() {
            used.push(run_input(&tx, &entries, i, ScriptUnits(u64::MAX))?);
        }
        let need_budgets: Vec<u16> = used
            .iter()
            .map(|u| ComputeBudget::checked_covering_script_units(ScriptUnits(*u)).map(|b| b.value()).ok_or("Budget > u16"))
            .collect::<Result<_, _>>()?;
        let (compute, transient, storage) = masses(&tx, &entries, params)?;
        let need_fee = min_fee(compute, transient) * (1000 + FEE_MARGIN_PERMILLE) / 1000;
        if need_budgets == budgets && paid >= need_fee {
            check_scripts(&tx, &entries)?;
            check_standard_sig_ops(&tx, &entries)?;
            check_block_limits(compute, transient, storage)?;
            return Ok(Built { tx, entries, fee: paid, compute_mass: compute, transient_mass: transient, storage_mass: storage, budgets, used_units: used, donated: paid - fee, change_index });
        }
        budgets = need_budgets;
        fee = fee.max(need_fee);
    }
    Err("Budget/Gebühr konvergiert nicht".into())
}

/// Bau mit Wallet-Signierern: Budgets und Gebühr kommen aus `with_wallet_fill`
/// (gemessen an einer Messkopie mit Ersatzschlüssel, wallet_ops::measure),
/// Signaturen ebenso. Keine Skriptprüfung hier.
fn build_for_wallet(d: &Draft, payload: &[u8], seqs: &[u64], entries: &[UtxoEntry], params: &Params) -> Result<Built, String> {
    let fill = WALLET_FILL.with(|c| {
        let mut b = c.borrow_mut();
        match b.as_mut() {
            Some((f, w)) if !w.used => {
                w.used = true;
                w.inputs = d.inputs.iter().enumerate().filter(|(_, i)| i.unlock.wallet_signer().is_some()).map(|(n, i)| i.unlock.wallet_spec(n)).collect();
                Some(f.clone())
            }
            _ => None,
        }
    });
    let fill = fill.ok_or("Wallet-Signierer ohne Vorgaben (Budgets/Gebühr aus der Messkopie) – nur über wallet_ops bauen")?;
    if fill.budgets.len() != d.inputs.len() {
        return Err("Wallet-Bau: Budgets passen nicht zur Zahl der Eingänge".into());
    }
    let (tx, paid, change_index) = assemble(d, payload, seqs, &fill.budgets, fill.fee, entries, params)?;
    let (compute, transient, storage) = masses(&tx, entries, params)?;
    Ok(Built {
        tx,
        entries: entries.to_vec(),
        fee: paid,
        compute_mass: compute,
        transient_mass: transient,
        storage_mass: storage,
        budgets: fill.budgets.clone(),
        used_units: vec![],
        donated: paid - fill.fee,
        change_index,
    })
}

/// NUR für Angriffstests gegen einen echten Node: baut und signiert ohne
/// Skriptprüfung, mit festen Budgets und fester Gebühr.
pub fn build_unverified(d: Draft, budgets: &[u16], fee: u64, params: &Params) -> Result<Built, String> {
    let entries: Vec<UtxoEntry> = d.inputs.iter().map(|i| i.entry.clone()).collect();
    let (tx, paid, change_index) = assemble(&d, &[], &[], budgets, fee, &entries, params)?;
    let (compute, transient, storage) = masses(&tx, &entries, params)?;
    Ok(Built { tx, entries, fee: paid, compute_mass: compute, transient_mass: transient, storage_mass: storage, budgets: budgets.to_vec(), used_units: vec![], donated: paid - fee, change_index })
}

pub fn outpoint(tx: &Transaction, index: u32) -> TransactionOutpoint {
    TransactionOutpoint { transaction_id: tx.id(), index }
}
