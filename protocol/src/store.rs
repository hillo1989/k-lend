//! Zustandsdatei robust halten (Audit 4 F1–F4, Audit 3 O-1/O-3, 28.09.2026):
//! - Dateisperre, damit Orakel-Feed, Seite und Terminal sich nicht gegenseitig
//!   Zustände überschreiben.
//! - Journal der gerade gesendeten Tx: Bricht ghostctl nach dem Senden ab
//!   (Zeitlimit, Absturz), klärt der nächste Aufruf, ob die Tx angenommen wurde,
//!   und übernimmt dann den neuen Zustand.
//! - Nachladen: Hat ein Dritter eine Covenant-UTXO ausgegeben, ohne ihren
//!   Zustand zu ändern (Orakel-read, fremdes Einzahlen), liegt sie unter
//!   derselben Adresse mit derselben Covenant-ID neu. Dann wird nur der
//!   Outpoint (und Betrag) nachgeführt.

use crate::chain;
use crate::contracts::*;
use crate::net::Net;
use crate::ops::{Deployment, Tracked};
use kaspa_consensus_core::Hash;
use kaspa_consensus_core::tx::{ScriptPublicKey, Transaction, TransactionOutpoint};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ Sperre ----

/// Exklusive Sperre über `File::try_lock` (flock). Das Betriebssystem gibt sie
/// frei, sobald der Prozess endet – es gibt keine verwaisten Sperren mehr, die
/// zwei Prozesse gleichzeitig "aufräumen" könnten (Fix-Review N-2: mit einer
/// reinen Sperrdatei bekamen in 5 von 12 Läufen zwei Aufrufer die Sperre).
pub struct Lock {
    _file: std::fs::File,
}

pub fn lock(state: &Path, wait: Duration) -> Result<Lock, String> {
    let path = state.with_extension("lock");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&path).map_err(|e| format!("{}: {e}", file_name(&path)))?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Lock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => {
                if start.elapsed() > wait {
                    return Err(format!("Eine andere ghostctl-Instanz arbeitet gerade (Sperre {}). Bitte gleich noch einmal versuchen.", file_name(&path)));
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("Sperre {}: {e}", file_name(&path))),
        }
    }
}

// ----------------------------------------------------------------- Journal ----

#[derive(Serialize, Deserialize)]
pub struct Pending {
    pub action: String,
    pub txid: String,
    /// Ausgänge der Tx (Index, Skript) – sichtbar = angenommen
    pub outputs: Vec<(u32, ScriptPublicKey)>,
    /// erster Input (Outpoint, Skript) – noch unverbraucht = nicht angenommen
    pub first_input: (TransactionOutpoint, ScriptPublicKey),
    /// alle Inputs (Fix-Review 8 NEU-4); ältere Journale haben nur first_input
    #[serde(default)]
    pub inputs: Vec<(TransactionOutpoint, ScriptPublicKey)>,
    /// eigener Wechselgeld-Ausgang; nur wir können ihn ausgeben (gilt NICHT
    /// für Wallet-Tx, dort gehört er dem Besucher)
    #[serde(default)]
    pub change_output: Option<u32>,
    /// Tx einer Browser-Wallet (ghostctl wallet submit): Eingänge und
    /// Wechselgeld gehören dem Besucher; Annahme wird über die Tx-ID geklärt
    /// (Audit 17 A17-2)
    #[serde(default)]
    pub wallet: bool,
    /// Beträge der Ausgänge (gleiche Reihenfolge wie `outputs`); ältere
    /// Journale haben sie nicht
    #[serde(default)]
    pub values: Vec<u64>,
    /// Zieldatei und neuer Inhalt bei Annahme
    pub target: Option<PathBuf>,
    pub next: Option<serde_json::Value>,
}

/// Wallet-Journal mit noch sichtbaren Eingängen: so lange nicht verwerfen
/// (Audit 18 G-2, verzögerter UTXO-Index nach der Aufnahme in einen Block)
pub const WALLET_UNSPENT_GRACE: Duration = Duration::from_secs(60);

pub fn pending_path(state: &Path) -> PathBuf {
    state.with_extension("pending.json")
}

pub fn write_pending(
    state: &Path,
    action: &str,
    tx: &Transaction,
    input_spks: &[ScriptPublicKey],
    change_output: Option<u32>,
    target: Option<PathBuf>,
    next: Option<serde_json::Value>,
    wallet: bool,
) -> Result<(), String> {
    let p = Pending {
        action: action.into(),
        txid: tx.id().to_string(),
        outputs: tx.outputs.iter().enumerate().map(|(i, o)| (i as u32, o.script_public_key.clone())).collect(),
        first_input: (tx.inputs[0].previous_outpoint, input_spks[0].clone()),
        inputs: tx.inputs.iter().zip(input_spks).map(|(i, s)| (i.previous_outpoint, s.clone())).collect(),
        change_output,
        wallet,
        values: tx.outputs.iter().map(|o| o.value).collect(),
        target,
        next,
    };
    atomic_write(&pending_path(state), &serde_json::to_string_pretty(&p).unwrap())
}

pub fn clear_pending(state: &Path) {
    let _ = std::fs::remove_file(pending_path(state));
}

pub fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Was `resolve_pending` über das Netz wissen muss: Node (UTXOs, Mempool) und
/// REST-API (Annahme einer Tx, wer einen Outpoint ausgab). Im Betrieb `Net`,
/// im Test eine Attrappe.
#[allow(async_fn_in_trait)]
pub trait JournalNet {
    async fn exists(&self, spk: &ScriptPublicKey, op: &TransactionOutpoint) -> Result<bool, String>;
    /// Ok(true)/Ok(false) = im Mempool bzw. sicher nicht; Err = nicht befragbar
    async fn in_mempool(&self, txid: Hash) -> Result<bool, String>;
    /// REST: Ok(Some(angenommen?)) oder Ok(None) = Tx unbekannt
    async fn tx_accepted(&self, txid: Hash) -> Result<Option<bool>, String>;
    /// REST: angenommene Tx, die `op` (Skript `spk`) ausgegeben hat
    async fn accepted_spender(&self, spk: &ScriptPublicKey, op: &TransactionOutpoint) -> Result<Option<chain::TxView>, String>;
}

impl JournalNet for Net {
    async fn exists(&self, spk: &ScriptPublicKey, op: &TransactionOutpoint) -> Result<bool, String> {
        Net::exists(self, spk, op).await
    }
    async fn in_mempool(&self, txid: Hash) -> Result<bool, String> {
        Net::in_mempool(self, txid).await
    }
    async fn tx_accepted(&self, txid: Hash) -> Result<Option<bool>, String> {
        let network = self.network.to_string();
        tokio::task::spawn_blocking(move || chain::tx_accepted(&network, &txid.to_string())).await.map_err(|e| e.to_string())?
    }
    async fn accepted_spender(&self, spk: &ScriptPublicKey, op: &TransactionOutpoint) -> Result<Option<chain::TxView>, String> {
        let (network, script, op) = (self.network.to_string(), spk.script().to_vec(), *op);
        tokio::task::spawn_blocking(move || chain::accepted_spender(&network, &script, &op)).await.map_err(|e| e.to_string())?
    }
}

/// Wie lange ein Wallet-Journal ohne klare Auskunft (REST-API nicht erreichbar
/// oder noch nicht so weit) stehen bleibt, bevor es verworfen wird (A17-2).
/// Bis dahin scheitern Abgleiche mit „unklar“, danach nie mehr.
pub const WALLET_JOURNAL_GRACE: Duration = Duration::from_secs(180);

/// Klärt eine offene Tx. Ok(Some(meldung)) wenn etwas übernommen/verworfen wurde.
pub async fn resolve_pending(net: &Net, state: &Path) -> Result<Option<String>, String> {
    resolve_pending_with(net, state, WALLET_JOURNAL_GRACE).await
}

/// `resolve_pending` mit beliebiger Netz-Auskunft und Frist für Wallet-Journale
pub async fn resolve_pending_with(net: &impl JournalNet, state: &Path, grace: Duration) -> Result<Option<String>, String> {
    let path = pending_path(state);
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    let p: Pending = serde_json::from_str(&text).map_err(|e| format!("Journal {}: {e}", file_name(&path)))?;
    let txid: Hash = p.txid.parse().map_err(|_| "Journal: ungültige TXID".to_string())?;
    let outpoint = |i: u32| TransactionOutpoint { transaction_id: txid, index: i };
    // 1. Irgendein Ausgang sichtbar → sicher angenommen
    let mut accepted = false;
    for (i, spk) in &p.outputs {
        if net.exists(spk, &outpoint(*i)).await? {
            accepted = true;
            break;
        }
    }
    if !accepted {
        // 2. Noch im Mempool → warten. Ist der Mempool nicht befragbar (Fix-Review
        //    8 NEU-5: anderer Fehlertext eines Nodes), fallen die Entscheidungen
        //    trotzdem, sofern sie vom Mempool nicht abhängen (Schritt 4).
        let mempool = net.in_mempool(txid).await;
        if matches!(mempool, Ok(true)) {
            return Err(format!("Die letzte Transaktion ({}, {}) ist noch unterwegs. Bitte kurz warten.", p.action, p.txid));
        }
        // 3. Irgendein Eingang noch unverbraucht → nicht angenommen, denn eine
        //    angenommene Tx verbraucht alle (Fix-Review 8 NEU-4: vorher nur der
        //    erste; eine verdrängte Tx ohne Wechselgeld blockierte dann alles)
        let inputs = if p.inputs.is_empty() { vec![p.first_input.clone()] } else { p.inputs.clone() };
        let mut unspent = false;
        for (op, spk) in &inputs {
            if net.exists(spk, op).await? {
                unspent = true;
                break;
            }
        }
        if unspent && p.wallet {
            // Audit 18 G-2: Kurz nach der Aufnahme in einen Block kann der
            // UTXO-Index die Eingänge noch zeigen, obwohl die Tx angenommen ist.
            // Wallet-Journale daher erst nach WALLET_UNSPENT_GRACE und nur
            // verwerfen, wenn die REST-API die Annahme nicht bestätigt.
            let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).unwrap_or(Duration::MAX);
            if let Ok(Some(true)) = net.tx_accepted(txid).await {
                take_over(state, &p, None)?;
                return Ok(Some(format!("Letzte Wallet-Transaktion ({}, {}) war angenommen (REST-API) – Zustand übernommen.", p.action, p.txid)));
            }
            if age < grace.min(WALLET_UNSPENT_GRACE) {
                return Err(format!("Die letzte Wallet-Transaktion ({}, {}) ist noch nicht geklärt. Bitte kurz warten.", p.action, p.txid));
            }
            clear_pending(state);
            return Ok(Some(format!("Letzte Wallet-Transaktion ({}) wurde nicht angenommen – verworfen.", p.action)));
        }
        if unspent {
            if let Err(e) = mempool {
                // Im Mempool sind die Eingänge ebenfalls unverbraucht: ohne Auskunft warten
                return Err(format!("Unklar, ob die letzte Transaktion ({}, {}) noch unterwegs ist: {e}. Bitte später erneut versuchen.", p.action, p.txid));
            }
            clear_pending(state);
            return Ok(Some(format!("Letzte Transaktion ({}) wurde nicht angenommen – verworfen.", p.action)));
        }
        // 4. Alle Eingänge verbraucht, aber kein Ausgang sichtbar. Entweder wurde
        //    die Tx angenommen und alle Ausgänge sind schon weiterverwendet, oder
        //    FREMDE Tx haben ihre Eingänge verbraucht (z. B. Orakel-read).
        //    Wallet-Tx: Wechselgeld und eigene Eingänge gehören dem Besucher, der
        //    sie jederzeit selbst ausgeben kann – dort entscheidet die Tx-ID
        //    (A17-2, `resolve_wallet`).
        if p.wallet {
            return resolve_wallet(net, state, &path, &p, txid, &inputs, grace).await;
        }
        //    Sonst entscheidet der eigene Wechselgeld-Ausgang: den kann nur unser
        //    Schlüssel ausgeben, und solange das Journal offen ist, tut das kein
        //    anderer ghostctl-Aufruf (Fix-Review N-1).
        match p.change_output {
            Some(_) => {
                clear_pending(state);
                return Ok(Some(format!(
                    "Letzte Transaktion ({}) wurde von einer anderen verdrängt (eigenes Wechselgeld nicht vorhanden) – verworfen.",
                    p.action
                )));
            }
            None => {
                return Err(format!(
                    "Unklar, ob die letzte Transaktion ({}, {}) angenommen wurde (kein eigener Wechselgeld-Ausgang). Bitte im Explorer prüfen und danach {} löschen.",
                    p.action,
                    p.txid,
                    file_name(&path)
                ));
            }
        }
    }
    take_over(state, &p, None)?;
    Ok(Some(format!("Letzte Transaktion ({}, {}) war angenommen – Zustand übernommen.", p.action, p.txid)))
}

/// Nur der Dateiname: Meldungen erreichen über die Wallet-Routen die
/// öffentliche Seite, absolute Pfade gehören nicht dorthin (A17-8)
fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Folgezustand übernehmen und Journal löschen. `swap` = (alte, neue Tx-ID),
/// wenn ein Zwilling statt unserer Tx angenommen wurde.
fn take_over(state: &Path, p: &Pending, swap: Option<(&str, &str)>) -> Result<(), String> {
    if let (Some(target), Some(next)) = (&p.target, &p.next) {
        let mut next = next.clone();
        if let Some((old, new)) = swap {
            replace_txid(&mut next, old, new);
        }
        atomic_write(target, &serde_json::to_string_pretty(&next).unwrap())?;
    }
    clear_pending(state);
    Ok(())
}

/// Tx-ID in allen Texten eines JSON-Werts ersetzen (Outpoints des Folgezustands)
fn replace_txid(v: &mut serde_json::Value, old: &str, new: &str) {
    match v {
        serde_json::Value::String(s) if s.contains(old) => *s = s.replace(old, new),
        serde_json::Value::Array(a) => a.iter_mut().for_each(|x| replace_txid(x, old, new)),
        serde_json::Value::Object(o) => o.values_mut().for_each(|x| replace_txid(x, old, new)),
        _ => {}
    }
}

/// Zwilling: andere Tx-ID, aber dieselben Eingänge und genau dieselben Ausgänge
/// (Skript und Betrag je Index). Entsteht, wenn jemand Compute-Budget oder
/// Speichermasse ändert – beides deckt der Sighash nicht ab (wallet.rs) –,
/// und hat dieselbe Wirkung wie unsere Tx.
fn is_twin(t: &chain::TxView, p: &Pending) -> bool {
    let key = |o: &TransactionOutpoint| (o.transaction_id.as_bytes(), o.index);
    let mut a: Vec<_> = t.inputs.iter().map(|i| key(&i.prev)).collect();
    let mut b: Vec<_> = p.inputs.iter().map(|(o, _)| key(o)).collect();
    a.sort();
    b.sort();
    if a != b || p.values.len() != p.outputs.len() || t.outputs.len() != p.outputs.len() {
        return false;
    }
    p.outputs.iter().zip(&p.values).all(|((i, spk), v)| t.outputs.iter().any(|o| o.index == *i && o.value == *v && o.spk == spk.script()))
}

/// Schritt 4 für Wallet-Tx: angenommen laut REST → übernehmen; ein Zwilling
/// angenommen → übernehmen mit dessen Tx-ID; eine andere Tx hat einen Eingang
/// ausgegeben → verworfen; keine klare Auskunft → nach `grace` verworfen. Der
/// Zustand der Covenants (Orakel, Register, Vaults, Pool) wird danach ohnehin
/// über `resync` von der Kette nachgeführt; ein Wallet-Journal sperrt so nie
/// dauerhaft Orakel, Keeper und Status.
async fn resolve_wallet(
    net: &impl JournalNet,
    state: &Path,
    path: &Path,
    p: &Pending,
    txid: Hash,
    inputs: &[(TransactionOutpoint, ScriptPublicKey)],
    grace: Duration,
) -> Result<Option<String>, String> {
    let mut doubt: Option<String> = None;
    match net.tx_accepted(txid).await {
        Ok(Some(true)) => {
            take_over(state, p, None)?;
            return Ok(Some(format!("Letzte Wallet-Transaktion ({}, {}) war angenommen (REST-API) – Zustand übernommen.", p.action, p.txid)));
        }
        Ok(_) => {}
        Err(e) => doubt = Some(e),
    }
    if doubt.is_none() {
        // eigene P2PK-Eingänge zuerst: kurzer Verlauf (Covenant-Adressen haben lange)
        let mut order: Vec<_> = inputs.iter().collect();
        order.sort_by_key(|(_, s)| !(s.script().len() == 34 && s.script().last() == Some(&0xac)));
        for (op, spk) in order.into_iter().take(3) {
            match net.accepted_spender(spk, op).await {
                Ok(Some(t)) if t.id == txid => {
                    take_over(state, p, None)?;
                    return Ok(Some(format!("Letzte Wallet-Transaktion ({}, {}) war angenommen – Zustand übernommen.", p.action, p.txid)));
                }
                Ok(Some(t)) if is_twin(&t, p) => {
                    let new = t.id.to_string();
                    take_over(state, p, Some((&p.txid, &new)))?;
                    return Ok(Some(format!(
                        "Statt der letzten Wallet-Transaktion ({}) wurde eine gleichwertige mit anderer Tx-ID angenommen ({new}) – Zustand übernommen.",
                        p.action
                    )));
                }
                Ok(Some(t)) => {
                    clear_pending(state);
                    return Ok(Some(format!(
                        "Letzte Wallet-Transaktion ({}) wurde von {} verdrängt – verworfen; der Zustand wird von der Kette nachgeführt.",
                        p.action, t.id
                    )));
                }
                Ok(None) => {}
                Err(e) => {
                    doubt = Some(e);
                    break;
                }
            }
        }
    }
    let age = std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).unwrap_or(Duration::MAX);
    if age >= grace {
        clear_pending(state);
        return Ok(Some(format!(
            "Letzte Wallet-Transaktion ({}, {}): Annahme nach {} s nicht feststellbar{} – Journal verworfen; der Zustand wird von der Kette nachgeführt.",
            p.action,
            p.txid,
            grace.as_secs(),
            doubt.map(|e| format!(" ({e})")).unwrap_or_default()
        )));
    }
    Err(format!(
        "Unklar, ob die letzte Wallet-Transaktion ({}, {}) angenommen wurde{}. Wird spätestens nach {} s automatisch geklärt – bitte kurz warten.",
        p.action,
        p.txid,
        doubt.map(|e| format!(" ({e})")).unwrap_or_default(),
        grace.as_secs()
    ))
}

/// Senden unter der Sperre, Warten ohne (Audit 17 A17-6): `send` läuft, solange
/// `lock` gehalten wird (das Journal ist dann schon geschrieben); danach wird
/// die Sperre freigegeben – auch wenn `send` scheitert – und erst dann läuft
/// `wait`. So wartet der Agent nie auf die Bestätigung einer Wallet-Tx.
/// Ergebnis: Tx-ID und Ergebnis des Wartens.
pub async fn send_then_wait<T, S, W, F>(lock: Lock, send: S, wait: W) -> Result<(T, Result<(), String>), String>
where
    S: std::future::Future<Output = Result<T, String>>,
    W: FnOnce() -> F,
    F: std::future::Future<Output = Result<(), String>>,
{
    let sent = send.await;
    drop(lock);
    let id = sent?;
    let waited = wait().await;
    Ok((id, waited))
}

// --------------------------------------------------------------- Nachladen ----

/// Führt einen verschobenen Covenant-UTXO nach. Ok(true) wenn gefunden/aktuell.
async fn follow<S>(net: &Net, t: &mut Tracked<S>, script: &ScriptPublicKey) -> Result<bool, String> {
    if net.exists(script, &t.outpoint).await? {
        return Ok(true);
    }
    let addr = net.address_of_spk(script)?;
    let cands: Vec<_> = net.utxos(&addr).await?.into_iter().filter(|(_, e)| e.covenant_id == Some(t.cov) && &e.script_public_key == script).collect();
    if cands.len() == 1 {
        t.outpoint = cands[0].0;
        t.value = cands[0].1.amount;
        return Ok(true);
    }
    Ok(false)
}

/// Bestätigt am Node: Die UTXO `op` liegt unter `script` und trägt die Covenant
/// von `t`. Dann übernimmt `t` Outpoint und Wert.
async fn confirm<S>(net: &Net, t: &mut Tracked<S>, script: &ScriptPublicKey, op: TransactionOutpoint) -> Result<bool, String> {
    let addr = net.address_of_spk(script)?;
    let hit = net.utxos(&addr).await?.into_iter().find(|(o, e)| *o == op && e.covenant_id == Some(t.cov) && &e.script_public_key == script);
    Ok(match hit {
        Some((o, e)) => {
            t.outpoint = o;
            t.value = e.amount;
            true
        }
        None => false,
    })
}

/// Orakel über die Kette nachführen (fremde Updates, Audit 10 A10-A-3)
async fn chain_oracle(net: &Net, d: &mut Deployment) -> Result<u32, String> {
    let (network, params, start) = (d.network.clone(), d.oracle_params.clone(), d.oracle.clone());
    let sh = chain::Shapes::of(d);
    let (op, _value, state, updates) = tokio::task::spawn_blocking(move || {
        let mut h = chain::History::new(&network)?;
        chain::follow_oracle(&mut h, &sh, &params, &start)
    })
    .await
    .map_err(|e| e.to_string())??;
    let script = spk(&oracle(&d.oracle_params, &state));
    let mut t = d.oracle.clone();
    if !confirm(net, &mut t, &script, op).await? {
        return Err("Die REST-API nennt eine Orakel-UTXO, die der Node nicht hat (noch nicht synchron?) – später erneut".into());
    }
    t.state = state;
    d.oracle = t;
    Ok(updates)
}

/// Register-Haupt-UTXO über die Kette nachführen (Preis-Updates, Austausch,
/// witness durch Dritte)
async fn chain_register(net: &Net, d: &mut Deployment) -> Result<(), String> {
    let (network, start) = (d.network.clone(), d.register.clone());
    let sh = chain::Shapes::of(d);
    let (op, _value, state) = tokio::task::spawn_blocking(move || {
        let mut h = chain::History::new(&network)?;
        chain::follow_register(&mut h, &sh, &start)
    })
    .await
    .map_err(|e| e.to_string())??;
    let script = spk(&register(&d.register_params, &state));
    let mut t = d.register.clone();
    if !confirm(net, &mut t, &script, op).await? {
        return Err("Die REST-API nennt eine Register-UTXO, die der Node nicht hat (noch nicht synchron?) – später erneut".into());
    }
    t.state = state;
    d.register = t;
    Ok(())
}

/// Vault über die Kette nachführen. Ok(None) = Vault beendet (geschlossen oder ganz liquidiert).
async fn chain_vault(net: &Net, d: &Deployment, i: usize) -> Result<Option<Tracked<VaultState>>, String> {
    let vp = d.vault_params.clone().ok_or("nicht initialisiert")?;
    let (network, owner, start) = (d.network.clone(), d.vaults[i].owner.clone(), d.vaults[i].vault.clone());
    let sh = chain::Shapes::of(d);
    let o2 = owner.clone();
    let r = tokio::task::spawn_blocking(move || {
        let mut h = chain::History::new(&network)?;
        chain::follow_vault(&mut h, &sh, &o2, &start)
    })
    .await
    .map_err(|e| e.to_string())??;
    let Some((op, _value, st)) = r else { return Ok(None) };
    let script = spk(&vault(&vp, &owner, &st));
    let mut t = d.vaults[i].vault.clone();
    if !confirm(net, &mut t, &script, op).await? {
        return Err("Die REST-API nennt eine Vault-UTXO, die der Node nicht hat (noch nicht synchron?)".into());
    }
    t.state = st;
    Ok(Some(t))
}

/// Vaults suchen, die über die Factory eröffnet wurden und hier fehlen
/// (andere Rechner). Übernimmt sie mit aktuellem Stand; beendete bleiben weg.
pub async fn discover(net: &Net, d: &mut Deployment) -> Result<Vec<String>, String> {
    if d.vault_params.is_none() {
        return Ok(vec![]);
    }
    let network = d.network.clone();
    let snapshot = d.clone();
    let sh = chain::Shapes::of(d);
    let found = tokio::task::spawn_blocking(move || {
        let mut h = chain::History::new(&network)?;
        chain::discover_vaults(&mut h, &sh, &snapshot)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut notes = vec![];
    for v in found {
        d.vaults.push(v);
        let i = d.vaults.len() - 1;
        match chain_vault(net, d, i).await {
            Ok(Some(t)) => {
                d.vaults[i].vault = t;
                let bs = spk(&d.vaults[i].branch.state.artifact());
                let mut br = d.vaults[i].branch.clone();
                if !follow(net, &mut br, &bs).await? {
                    d.vaults.pop();
                    notes.push("Neuer Vault gefunden, sein Minter-Zweig aber nicht – übersprungen".into());
                    continue;
                }
                d.vaults[i].branch = br;
                notes.push(format!(
                    "Vault {i} eines anderen Besitzers gefunden ({}…): {:.2} KAS, Schuld {:.8} GHOST",
                    faster_hex::hex_string(&d.vaults[i].owner)[..12].to_string(),
                    d.vaults[i].vault.value as f64 / 1e8,
                    d.vaults[i].vault.state.debt as f64 / 1e8
                ));
            }
            Ok(None) => {
                d.vaults.pop(); // schon wieder geschlossen
            }
            Err(e) => {
                d.vaults.pop();
                notes.push(format!("Neuer Vault gefunden, aber nicht nachführbar: {e}"));
            }
        }
    }
    Ok(notes)
}

/// Register nach dem Nachführen mit der Zustandsdatei abgleichen (ohne Netz,
/// damit testbar; Audit 15). `before` = Zustand der Haupt-UTXO vorher.
/// - Austausch aktiviert (jeder darf das, Audit 14 H-2): mit der eigenen
///   Ankündigung übernehmen (Satz UND Notfallsatz müssen passen, Audit 15 G-6),
///   sonst nur Preis-Updates sperren – Vaults und Pool brauchen den Satz nicht.
/// - nonce von außen verändert: fremde Ankündigung, Absage oder Notfall
///   (Audit 14 M-3). Verfällt eine Notfall-Ankündigung durch ein Preis-Update
///   (anderer Betreiber-Rechner), ist das kein Alarm (Audit 15).
pub fn reconcile_register(d: &mut Deployment, before: &RegisterState) -> Vec<String> {
    let mut notes = vec![];
    let now = d.register.state.clone();
    let mut adopted = false;
    if now.set_hash != before.set_hash {
        let mine = d.rotation.clone().filter(|r| {
            r.set.hash().as_slice() == now.set_hash.as_slice() && r.fallback.as_ref().map(|f| f.hash().to_vec()).unwrap_or(vec![0; 32]) == now.fb_hash
        });
        if let Some(rot) = mine {
            d.signer_set = rot.set;
            d.fallback_set = rot.fallback;
            d.rotation = None;
            d.foreign_change = None;
            adopted = true;
            notes.push("Der angekündigte Austausch der Unterzeichner ist aktiviert – neuer Satz übernommen.".into());
        }
    }
    let known = d.signer_set.hash().as_slice() == now.set_hash.as_slice();
    if !known && !d.signers_unknown {
        notes.push(
            "ACHTUNG: Im Register steht ein Unterzeichner-Satz, den diese Zustandsdatei nicht kennt (Austausch von einem anderen Rechner). Preis-Updates gehen erst mit der Datei des aktivierenden Rechners wieder; Vaults und Pool sind nicht betroffen.".into(),
        );
    }
    d.signers_unknown = !known;
    if now.nonce == before.nonce || adopted {
        return notes;
    }
    if let Some(rot) = &d.rotation {
        if rot.ticket.state.nonce != now.nonce {
            notes.push("Die angekündigte Schlüssel-Änderung ist abgesagt oder überholt – Ticket zum Aufräumen vorgemerkt (`signers clear`).".into());
            let r = d.rotation.take().unwrap();
            d.old_tickets.push(r.ticket);
        }
    }
    // Preis-Update eines anderen Betreiber-Rechners hat ein Notfall-Ticket entwertet
    let emergency_lapsed = before.emerg && !now.emerg && now.nonce == before.nonce + 1 && now.last_daa > before.last_daa;
    if emergency_lapsed {
        notes.push("Eine Notfall-Ankündigung ist durch ein Preis-Update verfallen.".into());
        d.foreign_change = None;
    } else if known && d.foreign_change != Some(now.nonce) {
        d.foreign_change = Some(now.nonce);
        notes.push(format!(
            "ACHTUNG: Im Register wurde von außen ein Austausch angekündigt oder abgesagt (nonce {}{}). Eine fremde Ankündigung lässt sich mit `ghostctl signers cancel` absagen, solange die Wartezeit läuft.",
            now.nonce,
            if now.emerg { ", Notfallweg" } else { "" }
        ));
    }
    notes
}

/// Gleicht alle verfolgten UTXOs mit dem Netz ab. Liefert Hinweise. Was ein
/// anderer Rechner verändert hat (Orakel-Update, Prägen, Tilgen, Liquidieren),
/// wird über die Kette nachgeführt; nur wenn das scheitert, bleibt ein Vault
/// als `stale` gesperrt.
pub async fn resync(net: &Net, d: &mut Deployment) -> Result<Vec<String>, String> {
    let mut notes = vec![];
    let oscript = spk(&oracle(&d.oracle_params, &d.oracle.state));
    let before = d.oracle.outpoint;
    if !follow(net, &mut d.oracle, &oscript).await? {
        match chain_oracle(net, d).await {
            Ok(n) => notes.push(format!(
                "Orakel von der Kette nachgeführt: {n} fremde(s) Update(s), jetzt {:.6} USD (Update Nr. {})",
                d.oracle.state.kas_usd as f64 / 1e8,
                d.oracle.state.seq
            )),
            Err(e) => {
                return Err(format!(
                    "Orakel-UTXO nicht auffindbar, und das Nachführen über die Kette scheiterte: {e}. Bitte später erneut oder Zustandsdatei prüfen."
                ));
            }
        }
    }
    if d.oracle.outpoint != before {
        notes.push("Orakel-UTXO wurde von Dritten neu erzeugt (read) – nachgeführt.".into());
    }
    let rscript = spk(&register(&d.register_params, &d.register.state));
    let before = d.register.state.clone();
    if !follow(net, &mut d.register, &rscript).await? {
        chain_register(net, d).await.map_err(|e| format!("Register-UTXO nicht auffindbar, und das Nachführen über die Kette scheiterte: {e}"))?;
        notes.push("Register von der Kette nachgeführt.".into());
    }
    // Tickets, die nicht mehr existieren (von Dritten aufgeräumt, Audit 15 G-4)
    let mut keep = vec![];
    for t in std::mem::take(&mut d.old_tickets) {
        let ts = spk(&register(&d.register_params, &t.state));
        if net.exists(&ts, &t.outpoint).await? {
            keep.push(t);
        }
    }
    d.old_tickets = keep;
    notes.extend(reconcile_register(d, &before));
    let fscript = spk(&factory(&d.factory_params, &d.factory.state));
    if !follow(net, &mut d.factory, &fscript).await? {
        notes.push("Factory-UTXO nicht auffindbar.".into());
    }
    if let Some(root) = d.ghost_root.as_mut() {
        let rs = spk(&root.state.artifact());
        if !follow(net, root, &rs).await? {
            notes.push("GHOST-Wurzel-Minter nicht auffindbar.".into());
        }
    }
    if let Some(vp) = d.vault_params.clone() {
        let mut ended = vec![];
        for i in 0..d.vaults.len() {
            let vs = spk(&vault(&vp, &d.vaults[i].owner, &d.vaults[i].vault.state));
            let bs = spk(&d.vaults[i].branch.state.artifact());
            let mut v_ok = follow(net, &mut d.vaults[i].vault, &vs).await?;
            if !v_ok {
                // von anderen verändert: über die Kette nachführen
                match chain_vault(net, d, i).await {
                    Ok(Some(t)) => {
                        notes.push(format!(
                            "Vault {i} von der Kette nachgeführt (von anderen verändert): Schuld {:.8} → {:.8} GHOST",
                            d.vaults[i].vault.state.debt as f64 / 1e8,
                            t.state.debt as f64 / 1e8
                        ));
                        d.vaults[i].vault = t;
                        v_ok = true;
                    }
                    Ok(None) => {
                        notes.push(format!("Vault {i} wurde geschlossen oder ganz liquidiert – entfernt."));
                        ended.push(i);
                        continue;
                    }
                    Err(e) => {
                        if !d.vaults[i].stale {
                            notes.push(format!("Vault {i} wurde von Dritten verändert und ist nicht nachführbar ({e}) – gesperrt."));
                        }
                    }
                }
            }
            let v = &mut d.vaults[i];
            let b_ok = follow(net, &mut v.branch, &bs).await?;
            if v_ok && b_ok {
                if v.stale {
                    notes.push(format!("Vault {i} ist wieder aktuell."));
                }
                v.stale = false;
            } else {
                v.stale = true;
            }
        }
        for i in ended.into_iter().rev() {
            d.vaults.remove(i);
        }
    }
    let mut keep = vec![];
    for mut t in std::mem::take(&mut d.tokens) {
        let ts = spk(&t.state.artifact());
        if follow(net, &mut t, &ts).await? {
            keep.push(t);
        } else {
            notes.push(format!("GHOST-UTXO ({:.8}) nicht mehr vorhanden – entfernt.", t.state.amount as f64 / 1e8));
        }
    }
    d.tokens = keep;
    Ok(notes)
}

/// Wartet, bis eine Tx angenommen ist: irgendein Ausgang sichtbar. Solange die
/// Tx im Mempool liegt, wird bis `max` weiter gewartet.
pub async fn wait_accepted(net: &Net, tx: &Transaction, base: Duration, max: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        for (i, o) in tx.outputs.iter().enumerate() {
            if net.exists(&o.script_public_key, &TransactionOutpoint { transaction_id: tx.id(), index: i as u32 }).await? {
                return Ok(());
            }
        }
        let el = start.elapsed();
        // Netzfehler bei der Mempool-Abfrage: weiter warten statt aufgeben
        let in_pool = net.in_mempool(tx.id()).await.unwrap_or(true);
        if el > max || (el > base && !in_pool) {
            return Err(format!(
                "Tx {} nach {:?} nicht bestätigt. Der nächste ghostctl-Aufruf prüft automatisch, ob sie doch angenommen wurde.",
                tx.id(),
                el
            ));
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sperre_ist_exklusiv_und_wird_freigegeben() {
        // Fix-Review N-2: zweiter Aufrufer darf die Sperre nicht bekommen
        let dir = std::env::temp_dir().join(format!("ghost-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("x.json");
        let a = lock(&state, Duration::from_millis(0)).expect("erste Sperre");
        assert!(lock(&state, Duration::from_millis(400)).is_err(), "zweite Sperre muss warten/scheitern");
        drop(a);
        lock(&state, Duration::from_millis(0)).expect("nach Freigabe wieder frei");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------ Audit 17 A17-2 / A17-6 ----

    use kaspa_consensus_core::subnets::SUBNETWORK_ID_NATIVE;
    use kaspa_consensus_core::tx::{TransactionInput, TransactionOutput};

    /// Netz-Attrappe: sichtbare UTXOs, Mempool, REST-Auskünfte
    struct Fake {
        utxos: Vec<TransactionOutpoint>,
        mempool: Result<bool, String>,
        accepted: Result<Option<bool>, String>,
        spender: Result<Option<chain::TxView>, String>,
    }

    impl JournalNet for Fake {
        async fn exists(&self, _spk: &ScriptPublicKey, op: &TransactionOutpoint) -> Result<bool, String> {
            Ok(self.utxos.contains(op))
        }
        async fn in_mempool(&self, _txid: Hash) -> Result<bool, String> {
            self.mempool.clone()
        }
        async fn tx_accepted(&self, _txid: Hash) -> Result<Option<bool>, String> {
            self.accepted.clone()
        }
        async fn accepted_spender(&self, _spk: &ScriptPublicKey, _op: &TransactionOutpoint) -> Result<Option<chain::TxView>, String> {
            self.spender.clone()
        }
    }

    /// Weg: Ausgänge weiterverwendet, Eingänge verbraucht, nicht im Mempool,
    /// REST kennt die Tx nicht
    fn gone() -> Fake {
        Fake { utxos: vec![], mempool: Ok(false), accepted: Ok(None), spender: Ok(None) }
    }

    fn p2pk(b: u8) -> ScriptPublicKey {
        let mut s = vec![0x20];
        s.extend([b; 32]);
        s.push(0xac);
        ScriptPublicKey::new(0, s.into())
    }

    /// Wallet-Tx OHNE Wechselgeld: Vault-Eingang + Eingang des Besuchers,
    /// Ausgänge Vault (Covenant-Skript) und Empfänger
    fn wallet_tx(budget: u16) -> Transaction {
        let ins = vec![
            TransactionInput::new_with_compute_budget(TransactionOutpoint::new(Hash::from_bytes([1; 32]), 0), vec![], 0, budget),
            TransactionInput::new_with_compute_budget(TransactionOutpoint::new(Hash::from_bytes([2; 32]), 3), vec![], 0, 0),
        ];
        let outs = vec![
            TransactionOutput { value: 5_000, script_public_key: ScriptPublicKey::new(0, vec![0xaa, 0x20, 9, 9, 0x87].into()), covenant: None },
            TransactionOutput { value: 7_000, script_public_key: p2pk(3), covenant: None },
        ];
        Transaction::new(1, ins, outs, 0, SUBNETWORK_ID_NATIVE, 0, vec![])
    }

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Zustandsdatei „alt“ und ein Journal mit Folgezustand (Outpoint der Tx darin)
    fn journal(name: &str, wallet: bool) -> (Dir, PathBuf, Transaction) {
        let dir = std::env::temp_dir().join(format!("ghost-a17-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = dir.join("mainnet.json");
        std::fs::write(&state, r#"{"stand":"alt"}"#).unwrap();
        let tx = wallet_tx(7);
        let next = serde_json::json!({ "stand": "neu", "vault": { "outpoint": TransactionOutpoint::new(tx.id(), 0) } });
        let spks = vec![ScriptPublicKey::new(0, vec![0xaa, 0x20, 1, 1, 0x87].into()), p2pk(2)];
        write_pending(&state, "Tilgen (Wallet)", &tx, &spks, None, Some(state.clone()), Some(next), wallet).unwrap();
        (Dir(dir), state, tx)
    }

    fn stand(state: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(state).unwrap()).unwrap()
    }

    /// Audit 18 G-2: Eingänge noch sichtbar (verzögerter UTXO-Index), aber die
    /// Tx ist angenommen – nicht verwerfen, sondern übernehmen bzw. warten
    #[tokio::test]
    async fn wallet_journal_mit_sichtbaren_eingaengen_wird_nicht_zu_frueh_verworfen() {
        let (_d, state, tx) = journal("g2", true);
        let first = tx.inputs[0].previous_outpoint;
        let sichtbar = |accepted| Fake { utxos: vec![first], mempool: Ok(false), accepted, spender: Ok(None) };
        // REST bestätigt: Folgezustand übernommen
        let m = resolve_pending_with(&sichtbar(Ok(Some(true))), &state, Duration::from_secs(3600)).await.unwrap().unwrap();
        assert!(m.contains("angenommen"), "{m}");
        assert_eq!(stand(&state)["stand"], "neu");
        // REST weiß nichts: innerhalb der Frist warten (Fehler), nichts verwerfen
        let (_d2, state2, tx2) = journal("g2b", true);
        let first2 = tx2.inputs[0].previous_outpoint;
        let net = Fake { utxos: vec![first2], mempool: Ok(false), accepted: Ok(None), spender: Ok(None) };
        assert!(resolve_pending_with(&net, &state2, Duration::from_secs(3600)).await.is_err());
        assert!(pending_path(&state2).exists(), "Journal bleibt in der Frist");
        // nach der Frist verworfen
        let m = resolve_pending_with(&net, &state2, Duration::ZERO).await.unwrap().unwrap();
        assert!(m.contains("nicht angenommen"), "{m}");
        assert_eq!(stand(&state2)["stand"], "alt");
    }

    #[tokio::test]
    async fn liegengebliebenes_wallet_journal_blockiert_den_naechsten_abgleich_nicht() {
        // A17-2: Wallet-Tx ohne Wechselgeld, alles weg, REST weiß nichts
        let (_d, state, _) = journal("liegen", true);
        // innerhalb der Frist: kurz „unklar“, Journal bleibt
        let e = resolve_pending_with(&gone(), &state, Duration::from_secs(3600)).await.unwrap_err();
        assert!(e.contains("automatisch geklärt"), "{e}");
        assert!(pending_path(&state).exists());
        // nach der Frist: verworfen, alter Stand bleibt, nächster Abgleich frei
        let m = resolve_pending_with(&gone(), &state, Duration::ZERO).await.unwrap().unwrap();
        assert!(m.contains("verworfen"), "{m}");
        assert!(!pending_path(&state).exists());
        assert_eq!(stand(&state)["stand"], "alt");
        assert_eq!(resolve_pending_with(&gone(), &state, Duration::ZERO).await.unwrap(), None);
        // REST nicht erreichbar: ebenso nur bis zur Frist
        let (_d2, state, _) = journal("liegen-rest", true);
        let down = Fake { accepted: Err("REST-API: Zeitlimit".into()), ..gone() };
        assert!(resolve_pending_with(&down, &state, Duration::from_secs(3600)).await.is_err());
        assert!(resolve_pending_with(&down, &state, Duration::ZERO).await.unwrap().unwrap().contains("verworfen"));
        assert!(!pending_path(&state).exists());
    }

    #[tokio::test]
    async fn wallet_journal_angenommen_laut_rest_uebernimmt_den_folgezustand() {
        let (_d, state, _) = journal("rest", true);
        let f = Fake { accepted: Ok(Some(true)), ..gone() };
        let m = resolve_pending_with(&f, &state, Duration::from_secs(3600)).await.unwrap().unwrap();
        assert!(m.contains("angenommen"), "{m}");
        assert_eq!(stand(&state)["stand"], "neu");
        assert!(!pending_path(&state).exists());
    }

    #[tokio::test]
    async fn wallet_journal_zwilling_und_verdraengung() {
        // Zwilling (andere Tx-ID, gleiche Ein- und Ausgänge): Zustand mit dessen Tx-ID
        let (_d, state, tx) = journal("zwilling", true);
        let mut twin = chain::TxView::from(&wallet_tx(9));
        twin.id = Hash::from_bytes([9; 32]);
        let f = Fake { spender: Ok(Some(twin.clone())), ..gone() };
        let m = resolve_pending_with(&f, &state, Duration::from_secs(3600)).await.unwrap().unwrap();
        assert!(m.contains("gleichwertige"), "{m}");
        let s = stand(&state);
        assert_eq!(s["stand"], "neu");
        let text = s.to_string();
        assert!(text.contains(&twin.id.to_string()) && !text.contains(&tx.id().to_string()), "{text}");
        // eine andere Tx (anderer Betrag) gab den Eingang aus: sofort verworfen
        let (_d2, state, _) = journal("verdraengt", true);
        let mut other = wallet_tx(7);
        other.outputs[1].value = 6_000;
        other.finalize();
        let f = Fake { spender: Ok(Some(chain::TxView::from(&other))), ..gone() };
        let m = resolve_pending_with(&f, &state, Duration::from_secs(3600)).await.unwrap().unwrap();
        assert!(m.contains("verdrängt"), "{m}");
        assert_eq!(stand(&state)["stand"], "alt");
        // Ausgang sichtbar: wie immer angenommen
        let (_d3, state, tx) = journal("sichtbar", true);
        let f = Fake { utxos: vec![TransactionOutpoint::new(tx.id(), 1)], ..gone() };
        resolve_pending_with(&f, &state, Duration::from_secs(3600)).await.unwrap().unwrap();
        assert_eq!(stand(&state)["stand"], "neu");
        // noch im Mempool: warten, Journal bleibt
        let (_d4, state, _) = journal("mempool", true);
        let f = Fake { mempool: Ok(true), ..gone() };
        assert!(resolve_pending_with(&f, &state, Duration::ZERO).await.unwrap_err().contains("unterwegs"));
        assert!(pending_path(&state).exists());
    }

    #[tokio::test]
    async fn journal_mit_schluesseldatei_bleibt_beim_alten_verfahren() {
        // ohne Wallet-Merkmal und ohne Wechselgeld: wie bisher Rückfrage an den Betreiber
        let (_d, state, _) = journal("schluessel", false);
        let e = resolve_pending_with(&gone(), &state, Duration::ZERO).await.unwrap_err();
        assert!(e.contains("im Explorer prüfen") && e.contains("mainnet.pending.json"), "{e}");
        assert!(!e.contains(&*std::env::temp_dir().to_string_lossy()), "kein absoluter Pfad: {e}");
    }

    #[tokio::test]
    async fn senden_haelt_die_sperre_warten_nicht() {
        // A17-6: während des Wartens auf die Bestätigung ist die Sperre frei
        let dir = std::env::temp_dir().join(format!("ghost-a17-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _d = Dir(dir.clone());
        let state = dir.join("x.json");
        let l = lock(&state, Duration::ZERO).unwrap();
        let st = state.clone();
        let (id, waited) = send_then_wait(
            l,
            async {
                // beim Senden ist sie gehalten
                assert!(lock(&st, Duration::ZERO).is_err(), "Sperre beim Senden gehalten");
                Ok("txid")
            },
            || async {
                // während des Wartens bekommt ein anderer (Agent) die Sperre sofort
                lock(&st, Duration::ZERO).map(|_| ())
            },
        )
        .await
        .unwrap();
        assert_eq!(id, "txid");
        waited.expect("Agent bekommt die Sperre während des Wartens");
        // Senden scheitert: Sperre trotzdem frei, nicht gewartet
        let l = lock(&state, Duration::ZERO).unwrap();
        let r = send_then_wait(l, async { Err::<(), _>("abgelehnt".to_string()) }, || async { panic!("nicht warten") }).await;
        assert!(r.is_err());
        lock(&state, Duration::ZERO).expect("Sperre nach Fehlschlag frei");
    }

    #[test]
    fn sperrmeldung_ohne_absoluten_pfad() {
        // A17-8: Meldungen erreichen die öffentliche Seite
        let dir = std::env::temp_dir().join(format!("ghost-a17-pfad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _d = Dir(dir.clone());
        let state = dir.join("mainnet.json");
        let _a = lock(&state, Duration::ZERO).unwrap();
        let e = lock(&state, Duration::from_millis(10)).err().unwrap();
        assert!(e.contains("mainnet.lock") && !e.contains(&*dir.to_string_lossy()), "{e}");
    }
}
