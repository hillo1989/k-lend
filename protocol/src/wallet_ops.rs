//! Alle Nutzeraktionen von Version 4 mit einer Browser-Wallet signieren
//! (KasWare `signPskt`, Kastle `signTx`), ohne Schlüssel auf dem Server.
//! Verallgemeinert die Wallet-Probe (src/wallet.rs, docs/wallet-probe.md).
//!
//! Ablauf
//! 1. `build_plan`: Die Aktion wird ZWEIMAL mit denselben ops/pool-Funktionen
//!    gebaut wie bei ghostctl mit Schlüsseldatei:
//!    - Messkopie: Der Nutzer-Schlüssel ist überall (Vaults, Token, Anteile,
//!      eigene KAS, Empfänger = man selbst) durch den Ersatzschlüssel
//!      `wallet::mirror_key` ersetzt, der lokal echt signiert. Er ist bei
//!      jedem Bau neu und zufällig; was ihm vorher schon gehörte, bekommt in
//!      der Messkopie einen neutralen Besitzer (Audit 17 A17-1). Daraus kommen
//!      Skript-Einheiten, Compute-Budgets und Gebühr (Skript-Einheiten hängen
//!      nicht vom Schlüssel ab, Test `einheiten_unabhaengig_vom_schluessel`).
//!    - Echte Tx: Signierer `txb::Signer::Wallet(x)`, Budgets und Gebühr aus
//!      der Messkopie (`txb::with_wallet_fill`), an jeder Signaturstelle ein
//!      Platzhalter. Beide müssen in Beträgen, Größen und Massen gleich sein.
//!    Die Wallet bekommt die echte Tx mit LEEREN Signaturskripten (Safe JSON)
//!    und die Liste der zu signierenden Eingänge: eigene P2PK-Eingänge
//!    (Gebühr, Einlagen), Covenant-Eingänge mit Besitzersignatur (`sig_at`,
//!    z. B. Vault mint/repay/deposit/withdraw/close) und KCC20-Eingänge mit
//!    Signatur (Leader/Delegate eigener GHOST bzw. Pool-Anteile).
//! 2. `submit`: Der Plan kam durch den Browser und ist NICHT vertrauenswürdig.
//!    Er wird aus dem aktuellen Zustand (deployments/<netz>.json) und den im
//!    Plan genannten eigenen UTXOs neu gebaut und muss bitgleich sein; weiter
//!    verwendet wird nur der Neubau, nie Inhalte des Plans. Die Antwort der
//!    Wallet muss bis auf Signaturskripte, Compute-Budgets und Speichermasse
//!    gleich sein (diese drei deckt der v1-Sighash nicht ab, Kommentar in
//!    txb.rs). Je Eingang: erster 65-Byte-Push, Hashtype 0x01, Schnorr-Prüfung
//!    gegen die Adresse (`wallet::read_sigs`). Dann wird die Aktion ein drittes
//!    Mal gebaut, jetzt mit den echten Signaturen an den Signaturstellen;
//!    Budgets werden nachgemessen (erhöht, falls nötig, solange die Gebühr
//!    reicht; dann erneut gebaut, damit Tx-ID und Folgezustand passen),
//!    Speichermasse gesetzt und alles lokal wie der Konsens geprüft
//!    (check_scripts, check_standard_sig_ops, Blockgrenzen, Mindestgebühr).
//!
//! Annahmen über die Wallets stehen in src/wallet.rs; zusätzlich hier:
//! - Die Wallet signiert Eingänge, deren Signaturskript leer ist, und lässt
//!   die übrigen (fremde Covenant-Eingänge wie Orakel, Factory, Pool) leer
//!   oder unverändert. Füllt sie sie doch, wird das nur vermerkt und
//!   überschrieben (der Sighash deckt Signaturskripte nicht ab).
//! - Kastle bekommt für Covenant-Eingänge `scripts` mit dem Redeem-Skript;
//!   für Leader/Delegate der KCC20-Token ist das das Token-Skript. Ob Kastle
//!   mehrere solcher Einträge in einer Tx annimmt, ist nicht belegt.
//!
//! Tresore (Daueraufträge, contracts/standing_order.sil, src/tresor.rs):
//! tresor-open, tresor-topup und tresor-cancel laufen denselben Weg, nur ist
//! der Zustand nicht deployments/<netz>.json, sondern die Tresor-Datei
//! (`TresorBasis`, deployments/<netz>-tresore.json). Besitzer eines über die
//! Wallet angelegten Tresors ist der x-only-Schlüssel der Wallet; in der
//! Messkopie wird er wie alles andere durch den Ersatzschlüssel ersetzt.
//! Auffüllen und Kündigen signiert die Wallet am Tresor-Eingang (Zweige
//! `topUp`/`cancel`, Besitzersignatur an Position 0). Zahlen (`pay`) braucht
//! keine Signatur und bleibt beim Agenten. Nachrichten nur öffentlich oder
//! keine: eine verschlüsselte Fassung wäre beim Neubau in `submit` nur aus
//! dem Plan (also vom Browser) zu übernehmen, und ob sie zur Beschreibung
//! passt, könnte der Server ohne Schlüssel des Empfängers nicht prüfen.

use crate::contracts::*;
use crate::ops::{self, Deployment, Funds, p2pk_spk};
use crate::pool;
use crate::tresor::{self, TresorFile, TresorHist, TresorRec};
use crate::txb::{Built, Signer, WalletBuilt, WalletFill, check_block_limits, check_scripts, check_standard_sig_ops, masses, min_fee, run_input, with_wallet_fill};
use crate::wallet::{self, InputReport, MAX_WALLET_INPUTS, Report, SafeTx, SignSpec, address_of_xonly, from_safe, strict_hex, to_safe, xonly_of_address};
use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::mass::{ComputeBudget, ScriptUnits};
use kaspa_consensus_core::tx::{TransactionOutpoint, TransactionOutput, UtxoEntry};
use serde::{Deserialize, Serialize};

pub const ACTION_PLAN_KIND: &str = "ghost-wallet-action:1";
/// Größter Betrag in Einheiten (sompi bzw. GHOST·1e8), den ein Plan nennt
pub const MAX_UNITS: i64 = 1_000_000_000_000_000_000;

/// Eine Nutzeraktion mit allen Werten ausgerechnet (keine Vorgaben wie „ganze
/// Schuld“ oder „1 % unter Kurs“ mehr – die löst ghostctl beim Bauen auf),
/// damit der Neubau in `submit` dieselbe Tx ergibt. Beträge in Einheiten.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    OpenVault { kas: u64 },
    Mint { vault: usize, ghost: i64 },
    Repay { vault: usize, ghost: i64 },
    Deposit { vault: usize, kas: u64 },
    Withdraw { vault: usize, keep: u64 },
    Close { vault: usize },
    Redeem { vault: usize, ghost: i64 },
    Liquidate { vault: usize, ghost: i64 },
    Sweep { vault: usize },
    /// KAS an eine Adresse; `payload` = fertige Nachricht (verschlüsselt oder öffentlich)
    Send {
        to: String,
        kas: u64,
        #[serde(default, with = "strict_hex")]
        payload: Vec<u8>,
    },
    /// GHOST an eine Schnorr-Adresse oder einen x-only-Pubkey (64 Hex)
    Transfer {
        to: String,
        ghost: i64,
        #[serde(default, with = "strict_hex")]
        payload: Vec<u8>,
    },
    /// Kaufen (`kas` gesetzt) oder Verkaufen (`ghost` gesetzt) mit Mindestbetrag
    Swap {
        #[serde(default)]
        kas: Option<i64>,
        #[serde(default)]
        ghost: Option<i64>,
        min: i64,
    },
    #[serde(rename_all = "camelCase")]
    PoolAdd { kas: i64, ghost: i64, min_shares: i64 },
    #[serde(rename_all = "camelCase")]
    PoolRemove { shares: i64, min_kas: i64, min_ghost: i64 },
    /// Tresor anlegen: `fund` sompi aus der Wallet in einen neuen Tresor,
    /// Besitzer = Wallet; Termine wie `ghostctl tresor open` ausgerechnet
    #[serde(rename_all = "camelCase")]
    TresorOpen {
        /// Empfänger: Schnorr-Adresse (kaspa:q…) oder x-only-Pubkey (64 Hex)
        to: String,
        /// Betrag je Zahlung (sompi)
        amount: i64,
        /// 1–31 = monatlich an diesem Tag, 0 = festes Intervall `period_ms`
        anchor_day: i64,
        period_ms: i64,
        /// erster Termin (Unix-ms, UTC)
        first_due: i64,
        /// Anzahl der Zahlungen, −1 = unbegrenzt
        count: i64,
        /// Startguthaben (sompi)
        fund: u64,
        /// Höchstgebühr je Zahlung aus dem Tresor (sompi)
        max_fee: i64,
        /// öffentliche Nachricht jeder Zahlung (Klartext) oder leer
        #[serde(default)]
        message: String,
    },
    /// Tresor auffüllen (`tresor` = volle Covenant-ID, 64 Hex)
    TresorTopup { tresor: String, kas: u64 },
    /// Tresor kündigen: Rest abzüglich Gebühr an den Besitzer
    TresorCancel { tresor: String },
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::OpenVault { .. } => "open-vault",
            Action::Mint { .. } => "mint",
            Action::Repay { .. } => "repay",
            Action::Deposit { .. } => "deposit",
            Action::Withdraw { .. } => "withdraw",
            Action::Close { .. } => "close",
            Action::Redeem { .. } => "redeem",
            Action::Liquidate { .. } => "liquidate",
            Action::Sweep { .. } => "sweep",
            Action::Send { .. } => "send",
            Action::Transfer { .. } => "transfer",
            Action::Swap { .. } => "swap",
            Action::PoolAdd { .. } => "pool-add",
            Action::PoolRemove { .. } => "pool-remove",
            Action::TresorOpen { .. } => "tresor-open",
            Action::TresorTopup { .. } => "tresor-topup",
            Action::TresorCancel { .. } => "tresor-cancel",
        }
    }
    /// Tresor-Aktion (gebaut aus der Tresor-Datei statt aus deployments/<netz>.json)?
    pub fn is_tresor(&self) -> bool {
        matches!(self, Action::TresorOpen { .. } | Action::TresorTopup { .. } | Action::TresorCancel { .. })
    }
    /// Braucht eigene KAS der Wallet? Kündigen nicht: die Gebühr kommt aus dem Tresor.
    pub fn needs_funding(&self) -> bool {
        !matches!(self, Action::TresorCancel { .. })
    }
    /// Deutsche Bezeichnung für Journal und Meldungen
    pub fn label(&self) -> &'static str {
        match self {
            Action::OpenVault { .. } => "Vault eröffnen (Wallet)",
            Action::Mint { .. } => "GHOST prägen (Wallet)",
            Action::Repay { .. } => "Tilgen (Wallet)",
            Action::Deposit { .. } => "Einzahlen (Wallet)",
            Action::Withdraw { .. } => "Abheben (Wallet)",
            Action::Close { .. } => "Schließen (Wallet)",
            Action::Redeem { .. } => "Rücknahme (Wallet)",
            Action::Liquidate { .. } => "Liquidieren (Wallet)",
            Action::Sweep { .. } => "Auflösen (Wallet)",
            Action::Send { .. } => "KAS senden (Wallet)",
            Action::Transfer { .. } => "GHOST senden (Wallet)",
            Action::Swap { .. } => "Tauschen (Wallet)",
            Action::PoolAdd { .. } => "Liquidität einlegen (Wallet)",
            Action::PoolRemove { .. } => "Liquidität abziehen (Wallet)",
            Action::TresorOpen { .. } => "Tresor anlegen (Wallet)",
            Action::TresorTopup { .. } => "Tresor auffüllen (Wallet)",
            Action::TresorCancel { .. } => "Tresor kündigen (Wallet)",
        }
    }
    /// Beträge und Nachrichtenlänge grob prüfen (genauer prüfen ops/pool)
    pub fn check(&self) -> Result<(), String> {
        let pos = |v: i64, what: &str| if v > 0 && v <= MAX_UNITS { Ok(()) } else { Err(format!("{what}: Betrag außerhalb 1 … 10^18 Einheiten")) };
        let posu = |v: u64, what: &str| pos(i64::try_from(v).unwrap_or(-1), what);
        match self {
            Action::OpenVault { kas } | Action::Deposit { kas, .. } => posu(*kas, "KAS"),
            Action::Withdraw { keep, .. } => posu(*keep, "verbleibende Sicherheit"),
            Action::Mint { ghost, .. } | Action::Repay { ghost, .. } | Action::Redeem { ghost, .. } | Action::Liquidate { ghost, .. } => pos(*ghost, "GHOST"),
            Action::Close { .. } | Action::Sweep { .. } => Ok(()),
            Action::Send { kas, payload, .. } => {
                posu(*kas, "KAS")?;
                check_payload(payload)
            }
            Action::Transfer { ghost, payload, .. } => {
                pos(*ghost, "GHOST")?;
                check_payload(payload)
            }
            Action::Swap { kas, ghost, min } => {
                match (kas, ghost) {
                    (Some(k), None) => pos(*k, "KAS")?,
                    (None, Some(g)) => pos(*g, "GHOST")?,
                    _ => return Err("Tauschen: entweder KAS oder GHOST angeben".into()),
                }
                pos(*min, "Mindestbetrag")
            }
            Action::PoolAdd { kas, ghost, min_shares } => {
                pos(*kas, "KAS")?;
                pos(*ghost, "GHOST")?;
                pos(*min_shares, "Mindestanteile")
            }
            Action::PoolRemove { shares, min_kas, min_ghost } => {
                pos(*shares, "Anteile")?;
                if *min_kas < 0 || *min_ghost < 0 || *min_kas > MAX_UNITS || *min_ghost > MAX_UNITS {
                    return Err("Mindestbeträge außerhalb 0 … 10^18".into());
                }
                Ok(())
            }
            Action::TresorOpen { to, amount, fund, max_fee, count, message, .. } => {
                if to.len() > 200 {
                    return Err("Empfänger zu lang".into());
                }
                pos(*amount, "Betrag je Zahlung")?;
                posu(*fund, "Startguthaben")?;
                pos(*max_fee, "Höchstgebühr")?;
                if *count != -1 && !(1..=MAX_TRESOR_COUNT).contains(count) {
                    return Err(format!("Anzahl: 1 bis {MAX_TRESOR_COUNT} Zahlungen oder unbegrenzt"));
                }
                // genau der Text, der in jede Zahlung kommt (tresor::bound_payload kürzt)
                if message.trim() != message {
                    return Err("Nachricht: ohne Leerzeichen am Anfang oder Ende".into());
                }
                crate::abo::check_message(message)?;
                check_payload(message.as_bytes())
            }
            Action::TresorTopup { tresor, kas } => {
                check_tresor_id(tresor)?;
                posu(*kas, "KAS")
            }
            Action::TresorCancel { tresor } => check_tresor_id(tresor),
        }
    }
}

/// Höchstzahl der Zahlungen eines Tresors über die Wallet (wie die Seite)
pub const MAX_TRESOR_COUNT: i64 = 9_999;

/// Tresor-Kennung im Plan: volle Covenant-ID (64 Hex, klein) – eindeutig,
/// anders als die 8 Zeichen der Liste
fn check_tresor_id(id: &str) -> Result<(), String> {
    if id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        Ok(())
    } else {
        Err("Tresor: volle Covenant-ID (64 Hex-Zeichen) erwartet".into())
    }
}

fn check_payload(p: &[u8]) -> Result<(), String> {
    if p.len() > crate::txb::MAX_PAYLOAD {
        return Err(format!("Nachricht zu lang ({} Byte, höchstens {})", p.len(), crate::txb::MAX_PAYLOAD));
    }
    Ok(())
}

/// Ergebnis eines Baus: Tx, Folgezustand, Angaben für die Anzeige
pub struct Done<N = Deployment> {
    pub built: Built,
    pub next: N,
    pub info: serde_json::Value,
}

/// Zustand, aus dem eine Aktion gebaut wird: deployments/<netz>.json
/// (`Deployment`: Vaults, GHOST, Pool) oder die Tresor-Datei (`TresorBasis`).
/// Beide liefern ihre Messkopie und den Folgezustand.
pub trait Basis: Clone {
    /// Messkopie: der Nutzer `user` überall durch den Ersatzschlüssel
    /// `mirror` ersetzt; was schon `mirror` gehörte, bekommt einen neutralen
    /// Besitzer (A17-1)
    fn mirror(&self, user: &[u8], mirror: &[u8]) -> Self;
    /// Aktion bauen; `sub` ersetzt x-only-Empfänger (nur in der Messkopie)
    fn run(&self, a: &Action, who: Signer, fund: &Funds, prefix: Prefix, net: &Params, sub: &dyn Fn(&[u8]) -> Vec<u8>) -> Result<Done<Self>, String>;
}

impl Basis for Deployment {
    fn mirror(&self, user: &[u8], mirror: &[u8]) -> Self {
        mirror_dep(self, user, mirror)
    }
    fn run(&self, a: &Action, who: Signer, fund: &Funds, prefix: Prefix, net: &Params, sub: &dyn Fn(&[u8]) -> Vec<u8>) -> Result<Done, String> {
        run_action(self, a, who, fund, prefix, net, sub)
    }
}

/// Zustand der Tresor-Aktionen: die Tresor-Datei des Servers (ghostctl hat
/// den betroffenen Tresor vorher am Node nachgeführt, `follow_for_wallet`),
/// die Zeit für den Verlauf des Folgezustands (geht nicht in die Tx ein) und
/// die Past Median Time `pmt` (Unix-ms) für die Regeln zum ersten Termin und
/// zu den Plätzen der Datei – in build UND im Neubau von submit (A19-1)
#[derive(Clone, Debug)]
pub struct TresorBasis {
    pub file: TresorFile,
    pub now: String,
    pub pmt: i64,
}

impl Basis for TresorBasis {
    fn mirror(&self, user: &[u8], mirror: &[u8]) -> Self {
        let nobody = neutral_owner(user, mirror);
        let mut m = self.clone();
        for r in m.file.tresore.iter_mut() {
            if r.params.owner == mirror {
                r.params.owner = nobody.clone();
            } else if r.params.owner == user {
                r.params.owner = mirror.to_vec();
            }
        }
        m
    }
    fn run(&self, a: &Action, who: Signer, fund: &Funds, prefix: Prefix, net: &Params, sub: &dyn Fn(&[u8]) -> Vec<u8>) -> Result<Done<Self>, String> {
        run_tresor(self, a, who, fund, prefix, net, sub)
    }
}

// ----------------------------------------------------------- Aktionen ----

fn vault_ok(d: &Deployment, i: usize) -> Result<(), String> {
    let v = d.vaults.get(i).ok_or(format!("Vault {i} gibt es nicht (vorhanden: {})", d.vaults.len()))?;
    if v.stale {
        return Err(format!("Vault {i} wurde von Dritten verändert; sein Zustand ist unbekannt. Aktionen darauf sind gesperrt."));
    }
    Ok(())
}

fn owned(d: &Deployment, i: usize, x: &[u8]) -> Result<(), String> {
    vault_ok(d, i)?;
    if d.vaults[i].owner != x {
        return Err(format!("Vault {i} gehört nicht zu dieser Adresse; diese Aktion darf nur der Besitzer ausführen."));
    }
    Ok(())
}

/// Die `max` größten eigenen GHOST-UTXOs (Indizes in d.tokens); reichen sie
/// nicht für `need`, obwohl insgesamt genug da ist, muss erst zusammengeführt
/// werden (eine Tx, die die Wallet getrennt signiert)
fn own_tokens(d: &Deployment, x: &[u8], max: usize, need: i64) -> Result<Vec<usize>, String> {
    let mut v: Vec<usize> = d.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == x && t.state.typ == ID_PUBKEY && !t.state.minter).map(|(i, _)| i).collect();
    v.sort_by_key(|&i| std::cmp::Reverse(d.tokens[i].state.amount));
    let total: i64 = v.iter().map(|&i| d.tokens[i].state.amount).sum();
    v.truncate(max);
    let top: i64 = v.iter().map(|&i| d.tokens[i].state.amount).sum();
    if top >= need {
        return Ok(v);
    }
    if total >= need {
        return Err(format!(
            "Die GHOST liegen auf mehr als {max} UTXOs verteilt. Bitte zuerst zusammenführen: GHOST senden (transfer) an die eigene Adresse, dann diese Aktion erneut."
        ));
    }
    Err(format!("zu wenig GHOST: {:.8} vorhanden (laut Zustand), {:.8} nötig", total as f64 / 1e8, need as f64 / 1e8))
}

/// KAS-Empfänger: Adresse des Netzes → Skript; x-only-Pubkey, falls P2PK-Schnorr
fn kas_target(prefix: Prefix, to: &str) -> Result<(kaspa_consensus_core::tx::ScriptPublicKey, Option<Vec<u8>>), String> {
    let a = Address::try_from(to.trim()).map_err(|_| "Empfänger: keine gültige Kaspa-Adresse".to_string())?;
    if a.prefix != prefix {
        return Err(format!("Empfänger: Adresse gehört zu einem anderen Netz ({})", a.prefix));
    }
    let x = (a.version == Version::PubKey && a.payload.len() == 32).then(|| a.payload.to_vec());
    Ok((kaspa_txscript::pay_to_address_script(&a), x))
}

/// GHOST-Empfänger → x-only-Pubkey (Schnorr-Adresse oder 64 Hex), muss ein
/// Punkt auf secp256k1 sein (sonst sind die GHOST verloren, A10-A-8)
pub fn ghost_target(prefix: Prefix, to: &str) -> Result<Vec<u8>, String> {
    let t = to.trim();
    let x = if t.contains(':') {
        xonly_of_address(t, prefix).map_err(|e| format!("Empfänger: {e}"))?
    } else {
        let b = strict_hex::decode(t).filter(|b| b.len() == 32).ok_or("Empfänger: Kaspa-Adresse (kaspa:q…) oder 64 Hex-Zeichen")?;
        secp256k1::XOnlyPublicKey::from_slice(&b).map_err(|_| "Empfänger: kein gültiger Schnorr-Schlüssel")?;
        b
    };
    Ok(x)
}

/// Baut eine Aktion mit dem Signierer `who` und den KAS `fund`. `sub` ersetzt
/// x-only-Empfänger (nur die Messkopie ersetzt den eigenen Schlüssel).
fn run_action(d: &Deployment, a: &Action, who: Signer, fund: &Funds, prefix: Prefix, net: &Params, sub: &dyn Fn(&[u8]) -> Vec<u8>) -> Result<Done, String> {
    a.check()?;
    let me = who.xonly();
    let plain = |r: Result<(Built, Deployment), String>, info: serde_json::Value| r.map(|(built, next)| Done { built, next, info });
    let pool_rec = |d: &Deployment| -> Result<pool::PoolRec, String> {
        if let Some(e) = &d.pool_unresolved {
            return Err(format!("Pool-Stand unbekannt: {e}"));
        }
        d.pool.clone().ok_or_else(|| "In diesem Netz gibt es noch keinen Pool.".to_string())
    };
    match a {
        Action::OpenVault { kas } => {
            let r = ops::open_vault(d, &me, *kas, fund, net);
            plain(r, serde_json::json!({ "vault": d.vaults.len() }))
        }
        Action::Mint { vault, ghost } => {
            owned(d, *vault, &me)?;
            plain(ops::mint(d, *vault, who, *ghost, &me, fund, net), serde_json::json!({}))
        }
        Action::Repay { vault, ghost } => {
            owned(d, *vault, &me)?;
            let need = (*ghost).min(d.vaults[*vault].vault.state.debt);
            let mine = own_tokens(d, &me, (GHOST_MAX_INS - 1) as usize, need)?;
            plain(ops::repay(d, *vault, who, &mine, *ghost, fund, net), serde_json::json!({}))
        }
        Action::Deposit { vault, kas } => {
            owned(d, *vault, &me)?;
            plain(ops::deposit(d, *vault, who, *kas, fund, net), serde_json::json!({}))
        }
        Action::Withdraw { vault, keep } => {
            owned(d, *vault, &me)?;
            let have = d.vaults[*vault].vault.value;
            if *keep >= have {
                return Err(format!("Die verbleibende Sicherheit muss kleiner als die aktuelle ({:.8} KAS) sein; zum Nachschießen „Einzahlen“", have as f64 / 1e8));
            }
            plain(ops::withdraw(d, *vault, who, *keep, &p2pk_spk(&sub(&me)), fund, net), serde_json::json!({ "kas": (have - keep) as f64 / 1e8 }))
        }
        Action::Close { vault } => {
            owned(d, *vault, &me)?;
            let r = ops::close(d, *vault, who, &p2pk_spk(&sub(&me)), fund, net);
            plain(r, serde_json::json!({ "interestKas": ops::close_fee(d, *vault) as f64 / 1e8 }))
        }
        Action::Redeem { vault, ghost } => {
            vault_ok(d, *vault)?;
            let mine = own_tokens(d, &me, (GHOST_MAX_INS - 1) as usize, *ghost)?;
            let paid = crate::math::redeem_paid(*ghost, d.oracle.state.kas_usd);
            plain(ops::redeem(d, *vault, who, &mine, *ghost, fund, net), serde_json::json!({ "kas": paid as f64 / 1e8 }))
        }
        Action::Liquidate { vault, ghost } => {
            vault_ok(d, *vault)?;
            let burn = (*ghost).min(d.vaults[*vault].vault.state.debt);
            let mine = own_tokens(d, &me, (GHOST_MAX_INS - 1) as usize, burn)?;
            plain(ops::liquidate(d, *vault, who, &mine, burn, fund, net), serde_json::json!({}))
        }
        Action::Sweep { vault } => {
            vault_ok(d, *vault)?;
            plain(ops::sweep(d, *vault, fund, net), serde_json::json!({}))
        }
        Action::Send { to, kas, payload } => {
            let (mut spk, x) = kas_target(prefix, to)?;
            if let Some(x) = x {
                spk = p2pk_spk(&sub(&x));
            }
            let out = TransactionOutput { value: *kas, script_public_key: spk, covenant: None };
            let b = crate::txb::build_with_payload(
                crate::txb::Draft { inputs: fund.inputs(), outputs: vec![out], change_spk: p2pk_spk(&me), lock_time: 0 },
                payload,
                net,
            )?;
            Ok(Done { built: b, next: d.clone(), info: serde_json::json!({}) })
        }
        Action::Transfer { to, ghost, payload } => {
            let to_x = sub(&ghost_target(prefix, to)?);
            let mine = own_tokens(d, &me, GHOST_MAX_INS as usize, *ghost)?;
            plain(ops::transfer_with_payload(d, who, &mine, &to_x, *ghost, payload, fund, net), serde_json::json!({}))
        }
        Action::Swap { kas, ghost, min } => {
            let rec = pool_rec(d)?;
            let oref = pool::OracleRef::of(d);
            let (t, out) = match (kas, ghost) {
                (Some(k), None) => pool::swap(&rec, Some(&oref), who, &[], pool::Swap::Buy { kas: *k, min_ghost: *min }, fund, net)?,
                (None, Some(g)) => {
                    let idx = own_tokens(d, &me, (GHOST_MAX_INS - 1) as usize, *g)?;
                    let mine: Vec<_> = idx.iter().map(|&i| d.tokens[i].clone()).collect();
                    pool::swap(&rec, Some(&oref), who, &mine, pool::Swap::Sell { ghost: *g, min_kas: *min }, fund, net)?
                }
                _ => return Err("Tauschen: entweder KAS oder GHOST angeben".into()),
            };
            Ok(Done { next: pool::apply(d, &t), built: t.built, info: serde_json::json!({ "out": out as f64 / 1e8 }) })
        }
        Action::PoolAdd { kas, ghost, min_shares } => {
            let rec = pool_rec(d)?;
            let idx = own_tokens(d, &me, (GHOST_MAX_INS - 1) as usize, *ghost)?;
            let mine: Vec<_> = idx.iter().map(|&i| d.tokens[i].clone()).collect();
            let (t, m, (dx, dy)) = pool::add(&rec, who, &mine, *kas, *ghost, *min_shares, pool::ADD_TOL_BPS, fund, net)?;
            Ok(Done { next: pool::apply(d, &t), built: t.built, info: serde_json::json!({ "shares": m, "kas": dx as f64 / 1e8, "ghost": dy as f64 / 1e8 }) })
        }
        Action::PoolRemove { shares, min_kas, min_ghost } => {
            let rec = pool_rec(d)?;
            let mine = pool::own(&d.lp_tokens, &me, 2);
            let (t, (dx, dy)) = pool::remove(&rec, who, &mine, *shares, *min_kas, *min_ghost, fund, net)?;
            Ok(Done { next: pool::apply(d, &t), built: t.built, info: serde_json::json!({ "kas": dx as f64 / 1e8, "ghost": dy as f64 / 1e8 }) })
        }
        Action::TresorOpen { .. } | Action::TresorTopup { .. } | Action::TresorCancel { .. } => {
            Err("Tresor-Aktionen werden aus der Tresor-Datei gebaut, nicht aus dem GHOST-Zustand".into())
        }
    }
}

/// Tresor der Datei über die volle Covenant-ID; nur laufende Tresore des
/// Besitzers `me` (Auffüllen und Kündigen verlangt der Vertrag ohnehin mit
/// dessen Signatur – hier früh und verständlich abgewiesen)
pub fn own_tresor(f: &TresorFile, id: &str, me: &[u8]) -> Result<usize, String> {
    let i = f.tresore.iter().position(|r| r.utxo.cov.to_string() == id).ok_or_else(|| format!("Tresor {} ist auf diesem Server nicht bekannt", &id[..8.min(id.len())]))?;
    let r = &f.tresore[i];
    if r.params.owner != me {
        return Err(format!("Tresor {} gehört nicht zu dieser Adresse; auffüllen und kündigen darf nur der Besitzer.", r.id));
    }
    if r.ended.is_some() {
        return Err(format!("Tresor {} ist schon gekündigt", r.id));
    }
    if r.missing.is_some() {
        return Err(format!("Tresor {} ist am Node nicht auffindbar (gekündigt?)", r.id));
    }
    Ok(i)
}

/// Tresor der Datei für Auffüllen bzw. Kündigen über die Browser-Wallet am
/// Node nachführen (ghostctl `wallet build | submit`, ändert nur `file` im
/// Speicher). Audit 19 A19-2: erst der Besitzer (`own_tresor` mit `owner`,
/// x-only aus Adresse bzw. Plan), dann die Suche, und die höchstens über
/// tresor::PUBLIC_FOLLOW Zustände. Fremde, gekündigte und schon als fehlend
/// markierte Tresore kosten so keine einzige Node-Abfrage. Anlegen sucht nichts.
pub async fn follow_for_wallet(io: &mut impl tresor::TresorIo, file: &mut TresorFile, a: &Action, owner: &[u8], pmt: i64, now: &str) -> Result<Option<String>, String> {
    let (Action::TresorTopup { tresor: id, .. } | Action::TresorCancel { tresor: id }) = a else { return Ok(None) };
    check_tresor_id(id)?;
    let i = own_tresor(file, id, owner)?;
    tresor::follow(io, &mut file.tresore[i], pmt, now, tresor::Search::Public).await
}

fn tresor_hist(now: &str, action: &str, b: &Built) -> TresorHist {
    TresorHist { at: now.into(), action: action.into(), txid: Some(b.tx.id().to_string()), due: None, note: None }
}

/// Tresor-Aktion bauen (wie ghostctl tresor open/topup/cancel, Signierer
/// `who`); Folgezustand = Tresor-Datei mit dem neuen bzw. geänderten Eintrag
fn run_tresor(b: &TresorBasis, a: &Action, who: Signer, fund: &Funds, prefix: Prefix, net: &Params, sub: &dyn Fn(&[u8]) -> Vec<u8>) -> Result<Done<TresorBasis>, String> {
    a.check()?;
    let me = who.xonly();
    let mut next = b.clone();
    match a {
        Action::TresorOpen { to, amount, anchor_day, period_ms, first_due, count, fund: value, max_fee, message } => {
            let recipient = sub(&ghost_target(prefix, to)?);
            // nur öffentlich oder keine Nachricht (Modulkommentar)
            let onchain = !message.is_empty();
            let mut p = TresorParams { owner: me.clone(), recipient, amount: *amount, anchor_day: *anchor_day, period_ms: *period_ms, max_fee: *max_fee, payload_hash: vec![] };
            tresor::bind_message(&mut p, message, onchain, message.as_bytes())?;
            let s0 = TresorState { next_due: *first_due, left: *count };
            tresor::check_params(&p, &s0)?;
            // A19-1: hier und nicht nur in ghostctl build – `first_due` kommt
            // in submit aus dem Plan des Browsers
            tresor::check_wallet_first_due(*first_due, b.pmt)?;
            next.file.make_room_for_wallet(&me, b.pmt)?;
            let (built, t) = tresor::open(&p, &s0, *value, fund, net)?;
            let mut rec = TresorRec::new(p, t, message.clone(), onchain, None, &b.now);
            rec.wallet = true;
            rec.push(tresor_hist(&b.now, "open", &built));
            let info = serde_json::json!({
                "tresor": rec.id,
                "covenantId": rec.utxo.cov.to_string(),
                "value": rec.utxo.value as f64 / 1e8,
                "covered": tresor::payments_covered(&rec.params, rec.utxo.value),
                "nextDue": first_due,
                "interval": tresor::interval_text(&rec.params),
            });
            next.file.upsert(rec);
            Ok(Done { built, next, info })
        }
        Action::TresorTopup { tresor: id, kas } => {
            let i = own_tresor(&b.file, id, &me)?;
            let r = &b.file.tresore[i];
            let (built, t) = tresor::topup(&r.params, &r.utxo, who, *kas, fund, net)?;
            let info = serde_json::json!({ "tresor": r.id, "value": t.value as f64 / 1e8, "covered": tresor::payments_covered(&r.params, t.value) });
            let nr = &mut next.file.tresore[i];
            nr.utxo = t;
            nr.retry_after = None;
            nr.last_error = None;
            nr.push(tresor_hist(&b.now, "topup", &built));
            Ok(Done { built, next, info })
        }
        Action::TresorCancel { tresor: id } => {
            let i = own_tresor(&b.file, id, &me)?;
            let r = &b.file.tresore[i];
            let built = tresor::cancel(&r.params, &r.utxo, who, net)?;
            let back: u64 = built.tx.outputs.iter().map(|o| o.value).sum();
            let info = serde_json::json!({ "tresor": r.id, "kas": back as f64 / 1e8 });
            let nr = &mut next.file.tresore[i];
            nr.ended = Some(b.now.clone());
            nr.push(tresor_hist(&b.now, "cancel", &built));
            Ok(Done { built, next, info })
        }
        _ => Err("Diese Aktion wird aus dem GHOST-Zustand gebaut, nicht aus der Tresor-Datei".into()),
    }
}

// --------------------------------------------------------- Messkopie ----

/// Neutraler Besitzer für die Messkopie: ein gültiger x-only-Schlüssel, der
/// weder Nutzer noch Ersatzschlüssel ist (x von G, 2G, 3G)
fn neutral_owner(user: &[u8], mirror: &[u8]) -> Vec<u8> {
    const CANDIDATES: [&str; 3] = [
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
        "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9",
    ];
    CANDIDATES.iter().map(|h| strict_hex::decode(h).expect("fester Hex-Wert")).find(|x| x.as_slice() != user && x.as_slice() != mirror).expect("drei Kandidaten, höchstens zwei belegt")
}

/// Zustand mit dem Ersatzschlüssel an Stelle des Nutzers (Vault-Besitz,
/// GHOST, Pool-Anteile). Gleiche Längen, also gleiche Massen und Einheiten.
/// Was vor dem Ersetzen schon dem Ersatzschlüssel gehört (fremde Vaults,
/// Token, Anteile), bekommt einen neutralen Besitzer, damit die Messkopie
/// genau die Eingänge des Nutzers wählt wie der echte Bau (A17-1). Eigene
/// KAS der Messkopie sind nur die umgeschriebenen UTXOs des Nutzers
/// (`measure`); andere UTXOs des Ersatzschlüssels kommen nie hinein.
pub fn mirror_dep(d: &Deployment, user: &[u8], mirror: &[u8]) -> Deployment {
    let nobody = neutral_owner(user, mirror);
    let mut m = d.clone();
    for v in m.vaults.iter_mut() {
        if v.owner == mirror {
            v.owner = nobody.clone();
        } else if v.owner == user {
            v.owner = mirror.to_vec();
        }
    }
    for t in m.tokens.iter_mut().chain(m.lp_tokens.iter_mut()) {
        if t.state.typ != ID_PUBKEY {
            continue;
        }
        if t.state.owner == mirror {
            t.state.owner = nobody.clone();
        } else if t.state.owner == user {
            t.state.owner = mirror.to_vec();
        }
    }
    m
}

/// Eigene KAS: Schnorr-P2PK der Adresse, ohne Covenant, keine Coinbase; die
/// größten zuerst, höchstens MAX_WALLET_INPUTS (der Dialog bleibt lesbar)
pub fn select_funding(owner: &[u8], utxos: &[(TransactionOutpoint, UtxoEntry)]) -> Vec<(TransactionOutpoint, UtxoEntry)> {
    let own = p2pk_spk(owner);
    let mut v: Vec<_> = utxos.iter().filter(|(_, e)| e.script_public_key == own && e.covenant_id.is_none() && !e.is_coinbase).cloned().collect();
    v.sort_by_key(|(o, e)| (std::cmp::Reverse(e.amount), o.transaction_id, o.index));
    v.truncate(MAX_WALLET_INPUTS);
    v
}

struct Measured {
    budgets: Vec<u16>,
    /// angeforderte Gebühr (ohne verschenkten Rest)
    fee: u64,
    used_units: Vec<u64>,
    built: Built,
}

fn measure<B: Basis>(d: &B, a: &Action, user: &[u8], funding: &[(TransactionOutpoint, UtxoEntry)], prefix: Prefix, net: &Params) -> Result<Measured, String> {
    let mk = wallet::mirror_key();
    let mx = wallet::mirror_x_of(&mk);
    if user == mx.as_slice() {
        return Err("Diese Adresse ist der Ersatzschlüssel der Messkopie und kann nicht signieren".into());
    }
    let md = d.mirror(user, &mx);
    let mspk = p2pk_spk(&mx);
    let mf = Funds::new(mk, funding.iter().map(|(o, e)| (*o, UtxoEntry::new(e.amount, mspk.clone(), e.block_daa_score, e.is_coinbase, None))).collect());
    let user = user.to_vec();
    let sub = move |x: &[u8]| if x == user.as_slice() { mx.clone() } else { x.to_vec() };
    let done = md.run(a, Signer::Key(mk), &mf, prefix, net, &sub)?;
    let b = done.built;
    Ok(Measured { budgets: b.budgets.clone(), fee: b.fee - b.donated, used_units: b.used_units.clone(), built: b })
}

fn build_real<B: Basis>(d: &B, a: &Action, user: &[u8; 32], funding: &[(TransactionOutpoint, UtxoEntry)], prefix: Prefix, net: &Params, fill: WalletFill) -> Result<(Done<B>, WalletBuilt), String> {
    let f = Funds::new(Signer::Wallet(*user), funding.to_vec());
    let (r, wb) = with_wallet_fill(fill, || d.run(a, Signer::Wallet(*user), &f, prefix, net, &|x: &[u8]| x.to_vec()));
    let done = r?;
    if !wb.used || wb.inputs.is_empty() {
        return Err("Aktion ohne Wallet-Signatur gebaut".into());
    }
    Ok((done, wb))
}

/// Messkopie und echte Tx müssen bis auf Schlüssel gleich sein
pub fn same_shape(m: &Built, r: &Built) -> Result<(), String> {
    let bad = |what: &str| Err(format!("Messkopie und echte Tx weichen ab ({what}) – Bau abgebrochen"));
    if m.tx.inputs.len() != r.tx.inputs.len() || m.tx.outputs.len() != r.tx.outputs.len() {
        return bad("Zahl der Ein-/Ausgänge");
    }
    if m.tx.outputs.iter().zip(&r.tx.outputs).any(|(a, b)| a.value != b.value || a.script_public_key.script().len() != b.script_public_key.script().len()) {
        return bad("Ausgänge");
    }
    if m.tx.inputs.iter().zip(&r.tx.inputs).any(|(a, b)| a.signature_script.len() != b.signature_script.len() || a.previous_outpoint != b.previous_outpoint || a.sequence != b.sequence) {
        return bad("Eingänge");
    }
    if m.change_index != r.change_index || m.tx.payload != r.tx.payload || m.tx.lock_time != r.tx.lock_time {
        return bad("Wechselgeld/Payload/Locktime");
    }
    if (m.compute_mass, m.transient_mass, m.storage_mass) != (r.compute_mass, r.transient_mass, r.storage_mass) {
        return bad("Masse");
    }
    if m.fee != r.fee {
        return bad("Gebühr");
    }
    Ok(())
}

// --------------------------------------------------------------- Plan ----

/// Nummer eines Vaults unter den Vaults desselben Besitzers (1, 2, 3 …), wie die
/// Seite sie zeigt: jeder Nutzer zählt seine eigenen Vaults, nur zur Anzeige
pub fn own_number(d: &Deployment, owner: &[u8], index: usize) -> usize {
    d.vaults.iter().take(index + 1).filter(|v| v.owner == owner).count()
}

/// Kurzform einer Covenant-ID für fremde Vaults (erste 8 Hex-Zeichen)
fn short_cov(c: &impl std::fmt::Display) -> String {
    c.to_string().chars().take(8).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPlan {
    pub kind: String,
    pub network: String,
    pub address: String,
    #[serde(with = "strict_hex")]
    pub owner: Vec<u8>,
    pub action: Action,
    /// unsignierte Tx, alle Signaturskripte leer (Budgets und Speichermasse aus der Messung)
    pub tx: SafeTx,
    pub signers: Vec<SignSpec>,
    pub fee: u64,
    #[serde(default)]
    pub change_index: Option<u32>,
    pub used_units: Vec<u64>,
}

impl ActionPlan {
    pub fn decode(text: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| format!("Plan ist kein JSON: {e}"))?;
        let v = match v.get("plan") {
            Some(p) if v.get("kind").is_none() => p.clone(),
            _ => v,
        };
        let p: ActionPlan = serde_json::from_value(v).map_err(|e| format!("Plan beschädigt: {e}"))?;
        if p.kind != ACTION_PLAN_KIND {
            return Err("kein Aktionsplan von ghostctl wallet build".into());
        }
        Ok(p)
    }

    /// Anfrage an Kastle: signTx(networkId, txJson, scripts) – scripts für
    /// alle Covenant-Eingänge (Redeem-Skript), P2PK-Eingänge signiert Kastle selbst
    pub fn kastle(&self) -> serde_json::Value {
        let scripts: Vec<_> = self
            .signers
            .iter()
            .filter(|s| s.kind != "p2pk")
            .map(|s| serde_json::json!({ "inputIndex": s.index, "scriptHex": s.redeem_hex, "signType": "All" }))
            .collect();
        serde_json::json!({ "networkId": self.network, "txJson": serde_json::to_string(&self.tx).unwrap(), "scripts": scripts })
    }

    /// Anfrage an KasWare: signPskt({txJsonString, options:{signInputs}})
    pub fn kasware(&self) -> serde_json::Value {
        let sign: Vec<_> = self.signers.iter().map(|s| serde_json::json!({ "index": s.index, "sighashType": 1 })).collect();
        serde_json::json!({ "txJsonString": serde_json::to_string(&self.tx).unwrap(), "options": { "signInputs": sign } })
    }

    /// Wofür jeder Ausgang ist (Anzeige vor dem Signieren), ohne Zustand
    pub fn describe_outputs(&self, prefix: Prefix) -> Vec<serde_json::Value> {
        self.describe_outputs_in(None, prefix)
    }

    /// Wie `describe_outputs`, mit dem Deployment: Covenant-Ausgänge werden beim
    /// Namen genannt (Orakel, Factory, GHOST-Wurzel, Vault, Minter-Zweig,
    /// GHOST-Token, Pool) statt nur „Vertrag (Covenant)“
    pub fn describe_outputs_in(&self, d: Option<&Deployment>, prefix: Prefix) -> Vec<serde_json::Value> {
        let kas = kas_text;
        let covenant_name = |o: &TransactionOutput| -> String {
            let Some(d) = d else { return "Vertrag (Covenant)".into() };
            let cov = o.covenant.as_ref().map(|c| c.covenant_id).unwrap_or_default();
            if cov == d.oracle.cov {
                return "Orakel (läuft weiter)".into();
            }
            if cov == d.register.cov {
                return "Unterzeichner-Register (läuft weiter)".into();
            }
            if cov == d.factory.cov {
                return "Factory (läuft weiter)".into();
            }
            if let Some((i, v)) = d.vaults.iter().enumerate().find(|(_, v)| v.vault.cov == cov) {
                return if v.owner == self.owner {
                    format!("Dein Vault {} – Sicherheit {}", own_number(d, &self.owner, i), kas(o.value))
                } else {
                    format!("Fremder Vault {} (läuft weiter)", short_cov(&v.vault.cov))
                };
            }
            if let Some(p) = &d.pool {
                if cov == p.pool.cov {
                    return "Tauschpool (KAS-Reserve)".into();
                }
                if cov == p.lp_cov {
                    return "Pool-Anteile (Token)".into();
                }
            }
            let ghost = d.ghost_root.as_ref().map(|r| r.cov);
            if Some(cov) == ghost {
                if d.ghost_root.as_ref().is_some_and(|r| spk(&r.state.artifact()) == o.script_public_key) {
                    return "GHOST-Wurzel (läuft weiter)".into();
                }
                if let Some((i, v)) = d.vaults.iter().enumerate().find(|(_, v)| spk(&v.branch.state.artifact()) == o.script_public_key) {
                    return if v.owner == self.owner {
                        format!("Minter-Zweig deines Vaults {} (läuft weiter)", own_number(d, &self.owner, i))
                    } else {
                        format!("Minter-Zweig des fremden Vaults {} (läuft weiter)", short_cov(&v.vault.cov))
                    };
                }
                if matches!(self.action, Action::OpenVault { .. }) && o.value == ops::BRANCH_VALUE {
                    return format!("Minter-Zweig des neuen Vaults – {}, bleiben dauerhaft gebunden", kas(o.value));
                }
                return format!("GHOST-Token – {} stecken darin und kommen beim Weitergeben bzw. Tilgen zurück", kas(o.value));
            }
            if matches!(self.action, Action::OpenVault { .. }) {
                return format!("Dein neuer Vault – Sicherheit {}", kas(o.value));
            }
            "Vertrag (Covenant)".into()
        };
        self.describe_with(prefix, &covenant_name)
    }

    /// Wie `describe_outputs_in` für Tresor-Aktionen, mit der Tresor-Datei:
    /// „Dein Tresor – N KAS, zahlt X KAS monatlich am 1. an kaspa:…“
    pub fn describe_tresor_outputs(&self, f: Option<&TresorFile>, prefix: Prefix) -> Vec<serde_json::Value> {
        let pays = |amount: i64, p: &TresorParams| format!("zahlt {} {} an {}", kas_text(amount as u64), tresor::interval_text(p), address_of_xonly(&p.recipient, prefix));
        let covenant_name = |o: &TransactionOutput| -> String {
            if let Action::TresorOpen { to, amount, anchor_day, period_ms, .. } = &self.action {
                let p = TresorParams { owner: vec![], recipient: vec![], amount: *amount, anchor_day: *anchor_day, period_ms: *period_ms, max_fee: 0, payload_hash: vec![] };
                return format!("Dein neuer Tresor – {}, zahlt {} {} an {}", kas_text(o.value), kas_text(*amount as u64), tresor::interval_text(&p), to.trim());
            }
            let cov = o.covenant.as_ref().map(|c| c.covenant_id).unwrap_or_default();
            match f.and_then(|f| f.tresore.iter().find(|r| r.utxo.cov == cov)) {
                Some(r) if r.params.owner == self.owner => format!("Dein Tresor {} – {}, {}", r.id, kas_text(o.value), pays(r.params.amount, &r.params)),
                Some(r) => format!("Tresor {} (läuft weiter)", r.id),
                None => "Vertrag (Covenant)".into(),
            }
        };
        self.describe_with(prefix, &covenant_name)
    }

    fn describe_with(&self, prefix: Prefix, covenant_name: &dyn Fn(&TransactionOutput) -> String) -> Vec<serde_json::Value> {
        let Ok((tx, _)) = from_safe(&self.tx) else { return vec![] };
        let own = p2pk_spk(&self.owner);
        tx.outputs
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let what = if o.covenant.is_some() {
                    covenant_name(o)
                } else if o.script_public_key == own {
                    if Some(i as u32) == self.change_index {
                        "Wechselgeld an die Wallet".into()
                    } else if matches!(self.action, Action::TresorCancel { .. }) {
                        "Rest des Tresors zurück an die Wallet".into()
                    } else {
                        "an die Wallet".into()
                    }
                } else {
                    "andere Adresse".to_string()
                };
                serde_json::json!({
                    "index": i,
                    "sompi": o.value,
                    "kas": o.value as f64 / 1e8,
                    "address": kaspa_txscript::standard::extract_script_pub_key_address(&o.script_public_key, prefix).map(|a| a.to_string()).unwrap_or_else(|_| "?".into()),
                    "what": what,
                })
            })
            .collect()
    }
}

/// Betrag in KAS mit Komma, ohne überflüssige Nullen („1,5 KAS“)
fn kas_text(v: u64) -> String {
    let s = format!("{:.8}", v as f64 / 1e8);
    let s = s.trim_end_matches('0').trim_end_matches('.').replace('.', ",");
    format!("{s} KAS")
}

struct Planned {
    plan: ActionPlan,
    /// angeforderte Gebühr der Messkopie
    fee_req: u64,
    info: serde_json::Value,
}

fn plan_with_funding<B: Basis>(
    d: &B,
    a: &Action,
    address: &str,
    network: &str,
    funding: &[(TransactionOutpoint, UtxoEntry)],
    prefix: Prefix,
    net: &Params,
) -> Result<Planned, String> {
    let owner = xonly_of_address(address, prefix)?;
    let user: [u8; 32] = owner.as_slice().try_into().map_err(|_| "Adresse: Schlüssel nicht 32 Byte")?;
    if funding.is_empty() && a.needs_funding() {
        return Err(format!("keine KAS auf {address} (Gebühr und Einlagen kommen aus der Wallet)"));
    }
    if !funding.is_empty() && !a.needs_funding() {
        return Err("Diese Aktion nimmt keine eigenen KAS (die Gebühr kommt aus dem Vertrag)".into());
    }
    if funding.len() > MAX_WALLET_INPUTS {
        return Err(format!("höchstens {MAX_WALLET_INPUTS} eigene UTXOs je Tx"));
    }
    let own = p2pk_spk(&owner);
    if funding.iter().any(|(_, e)| e.script_public_key != own || e.covenant_id.is_some() || e.is_coinbase) {
        return Err("Eingänge für Gebühr/Einlagen müssen normale KAS dieser Adresse sein".into());
    }
    let m = measure(d, a, &owner, funding, prefix, net)?;
    let fill = WalletFill { budgets: m.budgets.clone(), fee: m.fee, sigs: vec![], paid: m.built.fee };
    let (done, wb) = build_real(d, a, &user, funding, prefix, net, fill)?;
    same_shape(&m.built, &done.built)?;
    // Jeder zu signierende P2PK-Eingang muss der Adresse gehören
    for wi in &wb.inputs {
        if wi.kind == "p2pk" && done.built.entries[wi.index].script_public_key != own {
            return Err(format!("Eingang {} gehört nicht der Adresse", wi.index));
        }
    }
    let mut tx = done.built.tx.clone();
    for i in tx.inputs.iter_mut() {
        i.signature_script.clear();
    }
    tx.finalize();
    let signers = wb
        .inputs
        .iter()
        .map(|w| SignSpec { index: w.index, kind: w.kind.clone(), entry: w.entry.clone(), arg_pos: w.arg_pos, redeem_hex: w.redeem.as_ref().map(|r| faster_hex::hex_string(r)) })
        .collect();
    let plan = ActionPlan {
        kind: ACTION_PLAN_KIND.into(),
        network: network.into(),
        address: address_of_xonly(&owner, prefix),
        owner,
        action: a.clone(),
        tx: to_safe(&tx, &done.built.entries, prefix),
        signers,
        fee: done.built.fee,
        change_index: done.built.change_index,
        used_units: m.used_units,
    };
    Ok(Planned { plan, fee_req: m.fee, info: done.info })
}

/// Plan bauen: unsignierte Tx für die Wallet (sendet nichts, schreibt nichts).
/// `utxos` = UTXOs der Adresse vom Node.
pub fn build_plan<B: Basis>(
    d: &B,
    a: &Action,
    address: &str,
    network: &str,
    utxos: &[(TransactionOutpoint, UtxoEntry)],
    prefix: Prefix,
    net: &Params,
) -> Result<(ActionPlan, serde_json::Value), String> {
    let owner = xonly_of_address(address, prefix)?;
    let funding = if a.needs_funding() { select_funding(&owner, utxos) } else { vec![] };
    let p = plan_with_funding(d, a, address, network, &funding, prefix, net)?;
    Ok((p.plan, p.info))
}

/// Eigene UTXOs (Gebühr/Einlagen) aus einem Plan: alle Eingänge mit dem
/// P2PK-Skript der Adresse. Der Aufrufer prüft sie am Node (`net`).
pub fn plan_funding(plan: &ActionPlan, prefix: Prefix) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
    let owner = xonly_of_address(&plan.address, prefix)?;
    if owner != plan.owner {
        return Err("Plan: Adresse und Schlüssel passen nicht zusammen".into());
    }
    let (tx, entries) = from_safe(&plan.tx)?;
    let own = p2pk_spk(&owner);
    let v: Vec<_> = tx.inputs.iter().zip(entries).filter(|(_, e)| e.script_public_key == own).map(|(i, e)| (i.previous_outpoint, e)).collect();
    if v.len() > MAX_WALLET_INPUTS {
        return Err(format!("Plan: mehr als {MAX_WALLET_INPUTS} eigene UTXOs"));
    }
    Ok(v)
}

// ------------------------------------------------------------- Submit ----

pub struct Submitted<N = Deployment> {
    /// fertige, lokal geprüfte Tx (nur wenn alles gültig ist)
    pub built: Option<Built>,
    /// Folgezustand bei Annahme (für das Journal)
    pub next: Option<N>,
    pub report: Report,
    pub info: serde_json::Value,
}

/// Vorprüfung vor jedem Netzzugriff und vor der Sperre (Audit 17 A17-4):
/// Passt die Wallet-Antwort zur unsignierten Tx des Plans, und sind alle
/// Signaturen gültige Schnorr-Signaturen des Plan-Schlüssels (Hashtype ALL)?
/// Der Plan ist hier noch NICHT geprüft (das macht `submit` mit dem Neubau);
/// es geht nur darum, Müll-Signaturen und Pläne für fremde Adressen ohne
/// Netz, ohne Sperre und ohne Abgleich abzuweisen. Ok mit `error: None` =
/// weiter zu `submit`; Ok mit Fehler = ungültig (Bericht wie bei `submit`).
pub fn precheck(plan: &ActionPlan, signed: &SafeTx, network: &str, prefix: Prefix) -> Result<Report, String> {
    if plan.kind != ACTION_PLAN_KIND {
        return Err("kein Aktionsplan".into());
    }
    if plan.network != network {
        return Err(format!("Plan gehört zum Netz {}, gewählt ist {network}", plan.network));
    }
    plan_funding(plan, prefix)?;
    if plan.signers.is_empty() {
        return Err("Plan ohne zu signierende Eingänge".into());
    }
    let (utx, ue) = from_safe(&plan.tx)?;
    check_plan_shape(plan, &utx)?;
    let mut rep = Report { planned_units: plan.used_units.clone(), ..Default::default() };
    let (stx, se) = match from_safe(signed) {
        Ok(x) => x,
        Err(e) => {
            rep.error = Some(format!("Wallet-Antwort: {e}"));
            return Ok(rep);
        }
    };
    if stx.inputs.len() != utx.inputs.len() || stx.outputs.len() != utx.outputs.len() {
        rep.error = Some(format!(
            "Die Wallet hat die Tx verändert: {} Eingänge / {} Ausgänge statt {} / {}",
            stx.inputs.len(),
            stx.outputs.len(),
            utx.inputs.len(),
            utx.outputs.len()
        ));
        return Ok(rep);
    }
    let (changed, ignored) = wallet::diff(&utx, &ue, &stx, &se, &plan.signers, false);
    rep.changed = changed;
    rep.ignored = ignored;
    let (inputs, sigs) = wallet::read_sigs(&utx, &ue, &stx, &plan.owner, &plan.signers)?;
    rep.inputs = inputs;
    if !rep.changed.is_empty() {
        rep.error = Some(format!("Die Wallet hat die Tx verändert: {}", rep.changed.join(", ")));
    } else if rep.inputs.iter().any(|i| !i.sig_valid) || sigs.iter().any(Option::is_none) {
        rep.error = Some("Mindestens eine Signatur fehlt oder ist ungültig".into());
    }
    Ok(rep)
}

/// Höchstzahl der Eingänge und Ausgänge eines Plans: Die Aktionen von
/// ghostctl haben höchstens 8 eigene Eingänge plus wenige Covenant-Eingänge
/// und höchstens etwa 6 Ausgänge; alles darüber ist kein Plan von `build`.
pub const MAX_PLAN_INPUTS: usize = 24;
pub const MAX_PLAN_OUTPUTS: usize = 16;

/// Form des Plans vor jeder Signaturprüfung (Audit 20 A20b-4): Vorher rechnete
/// `precheck` je Signer-Eintrag einen vollen Sighash und eine Schnorr-Prüfung,
/// ohne Zahl und Eindeutigkeit zu prüfen – ein 714-kB-Plan mit 5 000 Einträgen
/// kostete 0,8 s CPU je Aufruf. Jetzt: höchstens so viele Signer-Einträge wie
/// Eingänge, jeder Eingang höchstens einmal, Eingänge und Ausgänge begrenzt.
pub fn check_plan_shape(plan: &ActionPlan, utx: &kaspa_consensus_core::tx::Transaction) -> Result<(), String> {
    let (n_in, n_out) = (utx.inputs.len(), utx.outputs.len());
    if n_in > MAX_PLAN_INPUTS || n_out > MAX_PLAN_OUTPUTS {
        return Err(format!("Plan: {n_in} Eingänge / {n_out} Ausgänge – kein Plan von ghostctl (höchstens {MAX_PLAN_INPUTS} / {MAX_PLAN_OUTPUTS})"));
    }
    if plan.signers.len() > n_in {
        return Err(format!("Plan: {} zu signierende Eingänge bei {n_in} Eingängen", plan.signers.len()));
    }
    let mut seen = vec![false; n_in];
    for sp in &plan.signers {
        if sp.index >= n_in {
            return Err(format!("Plan: Eingang {} gibt es nicht", sp.index));
        }
        if std::mem::replace(&mut seen[sp.index], true) {
            return Err(format!("Plan: Eingang {} steht mehrfach in der Signer-Liste", sp.index));
        }
    }
    Ok(())
}

/// Wallet-Antwort übernehmen. Err = Plan unbrauchbar (veraltet, verändert,
/// falsches Netz); Ok mit `built: None` = Signatur der Wallet ungültig
/// (Grund im Bericht).
pub fn submit<B: Basis>(d: &B, plan: &ActionPlan, signed: &SafeTx, network: &str, prefix: Prefix, net: &Params) -> Result<Submitted<B>, String> {
    if plan.kind != ACTION_PLAN_KIND {
        return Err("kein Aktionsplan".into());
    }
    if plan.network != network {
        return Err(format!("Plan gehört zum Netz {}, gewählt ist {network}", plan.network));
    }
    let funding = plan_funding(plan, prefix)?;
    // Neubau aus dem eigenen Zustand: nur er wird weiter verwendet
    let fresh = plan_with_funding(d, &plan.action, &plan.address, network, &funding, prefix, net)
        .map_err(|e| format!("Plan lässt sich nicht mehr bauen: {e}"))?;
    let same = serde_json::to_value(&fresh.plan).map_err(|e| e.to_string())? == serde_json::to_value(plan).map_err(|e| e.to_string())?;
    if !same {
        return Err("Plan passt nicht zum aktuellen Stand (inzwischen bewegt oder unterwegs verändert) – bitte neu bauen und signieren".into());
    }
    let plan = fresh.plan;
    let mut rep = Report { planned_units: plan.used_units.clone(), ..Default::default() };
    let none = |rep: Report| Ok(Submitted { built: None, next: None, report: rep, info: serde_json::Value::Null });
    let (utx, ue) = from_safe(&plan.tx)?;
    let (stx, se) = match from_safe(signed) {
        Ok(x) => x,
        Err(e) => {
            rep.error = Some(format!("Wallet-Antwort: {e}"));
            return none(rep);
        }
    };
    let (changed, ignored) = wallet::diff(&utx, &ue, &stx, &se, &plan.signers, false);
    rep.changed = changed;
    rep.ignored = ignored;
    let (inputs, sigs) = wallet::read_sigs(&utx, &ue, &stx, &plan.owner, &plan.signers)?;
    rep.inputs = inputs;
    if !rep.changed.is_empty() {
        rep.error = Some(format!("Die Wallet hat die Tx verändert: {}", rep.changed.join(", ")));
        return none(rep);
    }
    if rep.inputs.iter().any(|i: &InputReport| !i.sig_valid) || sigs.iter().any(Option::is_none) {
        rep.error = Some("Mindestens eine Signatur fehlt oder ist ungültig".into());
        return none(rep);
    }
    let mut per_input: Vec<Option<Vec<u8>>> = vec![None; utx.inputs.len()];
    for (sp, s) in plan.signers.iter().zip(sigs) {
        per_input[sp.index] = s;
    }
    let user: [u8; 32] = plan.owner.as_slice().try_into().map_err(|_| "Plan: Schlüssel nicht 32 Byte")?;
    let budgets: Vec<u16> = utx.inputs.iter().map(|i| i.compute_commit.compute_budget().unwrap_or(0)).collect();
    match finish(d, &plan, &user, &funding, budgets, fresh.fee_req, per_input, prefix, net, &mut rep) {
        Ok(done) => {
            rep.valid = true;
            Ok(Submitted { built: Some(done.built), next: Some(done.next), report: rep, info: fresh.info })
        }
        Err(e) => {
            rep.error = Some(e);
            none(rep)
        }
    }
}

/// Mit den Signaturen der Wallet bauen, Budgets nachmessen, prüfen wie der Konsens
#[allow(clippy::too_many_arguments)]
fn finish<B: Basis>(
    d: &B,
    plan: &ActionPlan,
    user: &[u8; 32],
    funding: &[(TransactionOutpoint, UtxoEntry)],
    mut budgets: Vec<u16>,
    fee_req: u64,
    sigs: Vec<Option<Vec<u8>>>,
    prefix: Prefix,
    net: &Params,
    rep: &mut Report,
) -> Result<Done<B>, String> {
    for round in 0..2 {
        let fill = WalletFill { budgets: budgets.clone(), fee: fee_req, sigs: sigs.clone(), paid: plan.fee };
        let (done, _) = build_real(d, &plan.action, user, funding, prefix, net, fill)?;
        let tx = &done.built.tx;
        let ue = &done.built.entries;
        let mut used = vec![];
        let mut need = vec![];
        for i in 0..tx.inputs.len() {
            let u = run_input(tx, ue, i, ScriptUnits(u64::MAX)).map_err(|e| format!("Skriptprüfung: {e}"))?;
            need.push(ComputeBudget::checked_covering_script_units(ScriptUnits(u)).map(|b| b.value()).ok_or("Budget > u16")?);
            used.push(u);
        }
        if need.iter().zip(&budgets).any(|(n, h)| n > h) {
            if round == 1 {
                return Err("Compute-Budget stabilisiert sich nicht".into());
            }
            rep.budgets_raised = true;
            budgets = budgets.iter().zip(&need).map(|(h, n)| *h.max(n)).collect();
            continue;
        }
        let (compute, transient, storage) = masses(tx, ue, net)?;
        if tx.storage_mass() != storage {
            return Err(format!("Speichermasse committet {} ≠ berechnet {storage}", tx.storage_mass()));
        }
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
        if fee < rep.min_fee {
            return Err(format!("Gebühr {fee} sompi < Mindestgebühr {} sompi", rep.min_fee));
        }
        check_block_limits(compute, transient, storage)?;
        check_scripts(tx, ue)?;
        check_standard_sig_ops(tx, ue)?;
        let mut built = done.built;
        built.used_units = used;
        return Ok(Done { built, next: done.next, info: done.info });
    }
    Err("Compute-Budget stabilisiert sich nicht".into())
}
