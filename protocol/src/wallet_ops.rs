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

use crate::contracts::*;
use crate::ops::{self, Deployment, Funds, p2pk_spk};
use crate::pool;
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
        }
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
        }
    }
}

fn check_payload(p: &[u8]) -> Result<(), String> {
    if p.len() > crate::txb::MAX_PAYLOAD {
        return Err(format!("Nachricht zu lang ({} Byte, höchstens {})", p.len(), crate::txb::MAX_PAYLOAD));
    }
    Ok(())
}

/// Ergebnis eines Baus: Tx, Folgezustand, Angaben für die Anzeige
pub struct Done {
    pub built: Built,
    pub next: Deployment,
    pub info: serde_json::Value,
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

fn measure(d: &Deployment, a: &Action, user: &[u8], funding: &[(TransactionOutpoint, UtxoEntry)], prefix: Prefix, net: &Params) -> Result<Measured, String> {
    let mk = wallet::mirror_key();
    let mx = wallet::mirror_x_of(&mk);
    if user == mx.as_slice() {
        return Err("Diese Adresse ist der Ersatzschlüssel der Messkopie und kann nicht signieren".into());
    }
    let md = mirror_dep(d, user, &mx);
    let mspk = p2pk_spk(&mx);
    let mf = Funds::new(mk, funding.iter().map(|(o, e)| (*o, UtxoEntry::new(e.amount, mspk.clone(), e.block_daa_score, e.is_coinbase, None))).collect());
    let user = user.to_vec();
    let sub = move |x: &[u8]| if x == user.as_slice() { mx.clone() } else { x.to_vec() };
    let done = run_action(&md, a, Signer::Key(mk), &mf, prefix, net, &sub)?;
    let b = done.built;
    Ok(Measured { budgets: b.budgets.clone(), fee: b.fee - b.donated, used_units: b.used_units.clone(), built: b })
}

fn build_real(d: &Deployment, a: &Action, user: &[u8; 32], funding: &[(TransactionOutpoint, UtxoEntry)], prefix: Prefix, net: &Params, fill: WalletFill) -> Result<(Done, WalletBuilt), String> {
    let f = Funds::new(Signer::Wallet(*user), funding.to_vec());
    let (r, wb) = with_wallet_fill(fill, || run_action(d, a, Signer::Wallet(*user), &f, prefix, net, &|x: &[u8]| x.to_vec()));
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
        let Ok((tx, _)) = from_safe(&self.tx) else { return vec![] };
        let own = p2pk_spk(&self.owner);
        let kas = |v: u64| {
            let s = format!("{:.8}", v as f64 / 1e8);
            let s = s.trim_end_matches('0').trim_end_matches('.').replace('.', ",");
            format!("{s} KAS")
        };
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
                    format!("Dein Vault {i} – Sicherheit {}", kas(o.value))
                } else {
                    format!("Vault {i} (läuft weiter)")
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
                if let Some((i, _)) = d.vaults.iter().enumerate().find(|(_, v)| spk(&v.branch.state.artifact()) == o.script_public_key) {
                    return format!("Minter-Zweig von Vault {i} (läuft weiter)");
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
        tx.outputs
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let what = if o.covenant.is_some() {
                    covenant_name(o)
                } else if o.script_public_key == own {
                    if Some(i as u32) == self.change_index { "Wechselgeld an die Wallet".into() } else { "an die Wallet".into() }
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

struct Planned {
    plan: ActionPlan,
    /// angeforderte Gebühr der Messkopie
    fee_req: u64,
    info: serde_json::Value,
}

fn plan_with_funding(
    d: &Deployment,
    a: &Action,
    address: &str,
    network: &str,
    funding: &[(TransactionOutpoint, UtxoEntry)],
    prefix: Prefix,
    net: &Params,
) -> Result<Planned, String> {
    let owner = xonly_of_address(address, prefix)?;
    let user: [u8; 32] = owner.as_slice().try_into().map_err(|_| "Adresse: Schlüssel nicht 32 Byte")?;
    if funding.is_empty() {
        return Err(format!("keine KAS auf {address} (Gebühr und Einlagen kommen aus der Wallet)"));
    }
    if funding.len() > MAX_WALLET_INPUTS {
        return Err(format!("höchstens {MAX_WALLET_INPUTS} eigene UTXOs je Tx"));
    }
    let own = p2pk_spk(&owner);
    if funding.iter().any(|(_, e)| e.script_public_key != own || e.covenant_id.is_some() || e.is_coinbase) {
        return Err("Eingänge für Gebühr/Einlagen müssen normale KAS dieser Adresse sein".into());
    }
    let m = measure(d, a, &owner, funding, prefix, net)?;
    let fill = WalletFill { budgets: m.budgets.clone(), fee: m.fee, sigs: vec![] };
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
pub fn build_plan(
    d: &Deployment,
    a: &Action,
    address: &str,
    network: &str,
    utxos: &[(TransactionOutpoint, UtxoEntry)],
    prefix: Prefix,
    net: &Params,
) -> Result<(ActionPlan, serde_json::Value), String> {
    let owner = xonly_of_address(address, prefix)?;
    let funding = select_funding(&owner, utxos);
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

pub struct Submitted {
    /// fertige, lokal geprüfte Tx (nur wenn alles gültig ist)
    pub built: Option<Built>,
    /// Folgezustand bei Annahme (für das Journal)
    pub next: Option<Deployment>,
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
    let mut rep = Report { planned_units: plan.used_units.clone(), ..Default::default() };
    let (stx, se) = match from_safe(signed) {
        Ok(x) => x,
        Err(e) => {
            rep.error = Some(format!("Wallet-Antwort: {e}"));
            return Ok(rep);
        }
    };
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

/// Wallet-Antwort übernehmen. Err = Plan unbrauchbar (veraltet, verändert,
/// falsches Netz); Ok mit `built: None` = Signatur der Wallet ungültig
/// (Grund im Bericht).
pub fn submit(d: &Deployment, plan: &ActionPlan, signed: &SafeTx, network: &str, prefix: Prefix, net: &Params) -> Result<Submitted, String> {
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
fn finish(
    d: &Deployment,
    plan: &ActionPlan,
    user: &[u8; 32],
    funding: &[(TransactionOutpoint, UtxoEntry)],
    mut budgets: Vec<u16>,
    fee_req: u64,
    sigs: Vec<Option<Vec<u8>>>,
    prefix: Prefix,
    net: &Params,
    rep: &mut Report,
) -> Result<Done, String> {
    for round in 0..2 {
        let fill = WalletFill { budgets: budgets.clone(), fee: fee_req, sigs: sigs.clone() };
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
