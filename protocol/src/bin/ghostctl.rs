//! ghostctl — GHOST-Protokoll auf Kaspa bedienen (Testnetz TN10 oder Mainnet).
//!
//! Schlüssel liegen als JSON in keys/ (nie committen). Der Zustand der
//! Covenant-UTXOs wird in deployments/<netz>.json mitgeführt.
//! Im Mainnet fragt jede Transaktion vor dem Senden nach Bestätigung.

use clap::{Parser, Subcommand};
use kaspa_lending_protocol::abo;
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::math;
use kaspa_lending_protocol::message;
use kaspa_lending_protocol::net::Net;
use kaspa_lending_protocol::ops::{self, Deployment, Funds, p2pk_spk, xonly};
use kaspa_lending_protocol::pool;
use kaspa_lending_protocol::price;
use kaspa_lending_protocol::rate;
use kaspa_lending_protocol::store;
use kaspa_lending_protocol::tresor::{self, TresorCode, TresorRec};
use kaspa_lending_protocol::txb::Built;
use secp256k1::{Keypair, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

static JSON_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// gebaute/gesendete Transaktionen – auch im Fehlerfall in der JSON-Ausgabe (Audit 4 F5)
static TXS: std::sync::Mutex<Vec<serde_json::Value>> = std::sync::Mutex::new(Vec::new());

/// Menschliche Meldung: im JSON-Modus nach stderr, damit stdout reines JSON bleibt.
macro_rules! say {
    ($($t:tt)*) => {
        if JSON_MODE.load(std::sync::atomic::Ordering::Relaxed) { eprintln!($($t)*) } else { println!($($t)*) }
    };
}

#[derive(Parser)]
#[command(name = "ghostctl", about = "GHOST-Stablecoin auf Kaspa L1 bedienen")]
struct Cli {
    /// mainnet | testnet-10
    #[arg(long, default_value = "mainnet")]
    network: String,
    /// eigener Node, z. B. ws://127.0.0.1:17210 (sonst öffentlicher Resolver)
    #[arg(long)]
    rpc: Option<String>,
    /// Zustandsdatei (Standard: deployments/<netz>.json)
    #[arg(long)]
    state: Option<PathBuf>,
    /// Mainnet-Transaktionen ohne Rückfrage senden
    #[arg(long)]
    ja: bool,
    /// Transaktion bauen und vollständig prüfen, aber NICHT senden
    #[arg(long)]
    dry_run: bool,
    /// Ergebnis als JSON auf stdout (Meldungen gehen nach stderr)
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Neuen Schlüssel erzeugen (Datei darf noch nicht existieren)
    Keygen { out: PathBuf },
    /// Unterzeichner-Schlüssel für das Orakel erzeugen (Version 4: Standard 1,
    /// höchstens 9). Die Datei enthält die Geheimnisse – nur auf dem Rechner,
    /// der Preise signiert.
    CommitteeKeygen {
        out: PathBuf,
        #[arg(long, default_value_t = 1)]
        count: usize,
    },
    /// Adresse und Guthaben eines Schlüssels
    Balance {
        #[arg(long)]
        key: PathBuf,
    },
    /// KAS/USD aus allen Quellen und Median
    Price,
    /// Schlüsseldateien mit Adresse und Guthaben (ohne Geheimnisse)
    Keys {
        #[arg(long, default_value = "keys")]
        dir: PathBuf,
    },
    /// Register + Orakel + Factory + GHOST anlegen (einmalig, Version 4).
    /// Der Zins der Vaults geht an die Adresse von --key.
    Deploy {
        #[arg(long)]
        key: PathBuf,
        /// Unterzeichner-Schlüssel (committee-keygen); alle bilden den Startsatz
        #[arg(long)]
        committee: PathBuf,
        /// Start-Zins p. a. in Prozent. Danach passt ihn der GHOST-Agent (mit
        /// Komitee-Datei) höchstens stündlich an den GHOST-Kurs an, frühestens
        /// eine Stunde nach dem Deployment.
        #[arg(long, default_value_t = 0.0)]
        rate: f64,
        /// Preis-Schwelle t (Standard: Mehrheit der Schlüssel)
        #[arg(long)]
        threshold: Option<i64>,
        /// Mainnet-Probe: Fristen 1 h statt 14 Tage / 2 h / 30 Tage,
        /// höchstens 5 GHOST je Vault
        #[arg(long)]
        probe: bool,
    },
    /// Abgleich mit der Kette: fremde Orakel-Updates und Vault-Änderungen
    /// nachführen und Vaults anderer Besitzer suchen (REST-API + Node)
    Sync,
    /// Übersicht: Orakel, Vaults, GHOST-Guthaben
    Status {
        /// maschinenlesbar (für die Lending-Seite)
        #[arg(long)]
        json: bool,
    },
    /// Veraltetes Orakel einfrieren (jeder darf, sobald die Frist ohne Preis
    /// abgelaufen ist): sperrt Prägen, Einlösen, Liquidieren und den Tausch,
    /// bis wieder ein Preis kommt
    OracleFreeze {
        #[arg(long)]
        key: PathBuf,
    },
    /// Unterzeichner des Orakels: anzeigen, Austausch ankündigen, absagen, aktivieren
    Signers {
        #[command(subcommand)]
        cmd: SignerCmd,
    },
    /// Orakel mit aktuellem Median-Preis (oder --usd) aktualisieren
    OracleUpdate {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        committee: PathBuf,
        #[arg(long)]
        usd: Option<f64>,
        /// Zins p. a. in Prozent von Hand; die Zinsregel des Agenten ändert
        /// ihn frühestens eine Stunde danach
        #[arg(long)]
        rate: Option<f64>,
    },
    /// Orakel dauerhaft betreiben: alle N Sekunden prüfen, aktualisieren nur
    /// bei Preisänderung > --min-change oder Alter > --max-age-min
    OracleFeed {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        committee: PathBuf,
        #[arg(long, default_value_t = 300)]
        interval: u64,
        /// relative Preisänderung, ab der aktualisiert wird (0.005 = 0,5 %;
        /// unter der Rücknahmegebühr von 1 %, Audit 11 A11-V-2)
        #[arg(long, default_value_t = 0.005)]
        min_change: f64,
        /// spätestens nach so vielen Minuten aktualisieren
        #[arg(long, default_value_t = 360.0)]
        max_age_min: f64,
    },
    /// GHOST-Agent im Dauerbetrieb: liquidiert unterdeckte Vaults (nur wenn auch
    /// der Marktpreis sie als unterdeckt zeigt, nie mit Verlust), hält mit
    /// --committee zusätzlich das Orakel frisch, führt fällige Daueraufträge aus
    /// und löst fällige Tresor-Zahlungen aus.
    Agent {
        /// Schlüssel mit KAS für Gebühren und GHOST für Liquidationen
        #[arg(long)]
        key: PathBuf,
        /// Komitee-Datei: dann auch Orakel-Updates (nur für den Betreiber)
        #[arg(long)]
        committee: Option<PathBuf>,
        #[arg(long, default_value_t = 120)]
        interval: u64,
        /// relative Preisänderung, ab der das Orakel aktualisiert wird (0,5 %)
        #[arg(long, default_value_t = 0.005)]
        min_change: f64,
        #[arg(long, default_value_t = 360.0)]
        max_age_min: f64,
    },
    /// Vault eröffnen mit KAS-Sicherheit
    OpenVault {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        kas: f64,
    },
    /// GHOST prägen
    Mint {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
        #[arg(long)]
        ghost: f64,
    },
    /// Schuld tilgen (mit eigenen GHOST)
    Repay {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
        /// Betrag; ohne Angabe die ganze Schuld
        #[arg(long)]
        ghost: Option<f64>,
    },
    /// KAS nachschießen
    Deposit {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
        #[arg(long)]
        kas: f64,
    },
    /// KAS herausnehmen (neue Sicherheit angeben)
    Withdraw {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
        /// verbleibende Sicherheit in KAS
        #[arg(long)]
        keep: f64,
    },
    /// Schuldenfreien Vault schließen
    Close {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
    },
    /// Schuldenfreien Vault, dessen Zins die ganze Sicherheit aufzehrt, zugunsten
    /// der Zinskasse auflösen (jeder darf; der Vault trägt 0,01 KAS der
    /// Netzgebühr, den Rest von etwa 0,05 KAS der Schlüssel --key)
    Sweep {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
    },
    /// Vault unter 150 % liquidieren
    Liquidate {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
        /// zu verbrennende GHOST (Teil-Liquidation); ohne Angabe die ganze Schuld
        #[arg(long)]
        ghost: Option<f64>,
    },
    /// Rücknahme: GHOST an einem Vault (ab 150 %) gegen KAS im Wert von 1 USD je
    /// GHOST abzüglich 1 % zurückgeben (mindestens 1 GHOST oder die ganze Schuld)
    Redeem {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        vault: usize,
        #[arg(long)]
        ghost: f64,
    },
    /// KAS an eine Adresse (kaspa:/kaspatest:) oder Schlüsseldatei senden
    Send {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        kas: f64,
        /// Nachricht (höchstens 100 Zeichen): steht im lokalen Verlauf und
        /// verschlüsselt an den Empfänger in der Transaktion (nur er kann sie lesen)
        #[arg(long)]
        message: Option<String>,
        /// Nachricht stattdessen öffentlich (Klartext, für alle sichtbar) in die Transaktion schreiben
        #[arg(long)]
        onchain_message: bool,
    },
    /// GHOST an eine Kaspa-Adresse, einen x-only-Pubkey (hex) oder eine Schlüsseldatei senden
    Transfer {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        ghost: f64,
        /// Nachricht (höchstens 100 Zeichen): steht im lokalen Verlauf und
        /// verschlüsselt an den Empfänger in der Transaktion (nur er kann sie lesen)
        #[arg(long)]
        message: Option<String>,
        /// Nachricht stattdessen öffentlich (Klartext, für alle sichtbar) in die Transaktion schreiben
        #[arg(long)]
        onchain_message: bool,
    },
    /// Daueraufträge: KAS oder GHOST in festen Abständen senden (deployments/<netz>-abos.json)
    Abo {
        #[command(subcommand)]
        cmd: AboCmd,
    },
    /// Browser-Wallet signiert (docs/wallet-probe.md, docs/wallet-aktionen.md):
    /// unsignierte Tx ausgeben und die Signaturen der Wallet übernehmen. Nie mit
    /// Schlüsseldatei. Probe (export-unsigned, attach-sigs, probe-pay) ohne
    /// deployments/; Nutzeraktionen (build, submit) lesen deployments/ und
    /// schreiben nur bei submit --send über das Journal
    Wallet {
        #[command(subcommand)]
        cmd: WalletCmd,
    },
    /// Daueraufträge mit Tresor: KAS liegen in einem Vertrag und werden zu
    /// festen Terminen gezahlt, auch wenn dieser Rechner aus ist
    /// (deployments/<netz>-tresore.json)
    Tresor {
        #[command(subcommand)]
        cmd: TresorCmd,
    },
    /// Eingegangene Nachrichten an die Adresse dieses Schlüssels (REST-API,
    /// sendet nichts): verschlüsselte werden entschlüsselt, öffentliche angezeigt
    Messages {
        #[arg(long)]
        key: PathBuf,
        /// so viele der neuesten Transaktionen der Adresse durchsehen
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Eingegangene GHOST mit genau diesem Betrag suchen und übernehmen (sendet nichts).
    /// Empfänger: Schlüsseldatei (--key) oder ohne Geheimnis eine Adresse bzw.
    /// ein x-only-Schlüssel (--owner, öffentliche Seite mit Browser-Wallet)
    Receive {
        #[arg(long, conflicts_with = "owner", required_unless_present = "owner")]
        key: Option<PathBuf>,
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        ghost: f64,
    },
    /// UTXOs an Adressen vom Node (nur lesend, für die .k-Namensprüfung der Seite):
    /// je UTXO Adresse, Covenant-ID, Outpoint und DAA-Score als JSON
    Utxos {
        /// höchstens 50 Adressen
        #[arg(long, required = true, num_args = 1..=50)]
        address: Vec<String>,
    },
    /// Tauschpool KAS/GHOST anlegen (einmalig je Netz). 1 KAS und GHOST zum
    /// gleichen Kurs bleiben als Mindestliquidität für immer im Pool; der Rest
    /// wird eingelegt und bringt Anteile.
    PoolOpen {
        #[arg(long)]
        key: PathBuf,
        /// KAS gesamt (mindestens 1)
        #[arg(long)]
        kas: f64,
        /// GHOST gesamt; das Verhältnis ist der Startkurs
        #[arg(long)]
        ghost: f64,
    },
    /// Liquidität einlegen (jeder): KAS und GHOST, dafür Pool-Anteile
    PoolAdd {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        kas: f64,
        #[arg(long)]
        ghost: f64,
        /// Mindestens so viele Anteile, sonst Abbruch (Schutz, falls sich der
        /// Pool zwischen Probelauf und Senden verschiebt; Standard: 1)
        #[arg(long)]
        min_shares: Option<i64>,
    },
    /// Liquidität abziehen: Prozent der eigenen Anteile (über 0 bis 100)
    PoolRemove {
        #[arg(long)]
        key: PathBuf,
        #[arg(long, default_value_t = 100.0)]
        percent: f64,
        /// Mindestens so viele KAS, sonst Abbruch (Standard: 0)
        #[arg(long)]
        min_kas: Option<f64>,
        /// Mindestens so viele GHOST, sonst Abbruch (Standard: 0)
        #[arg(long)]
        min_ghost: Option<f64>,
    },
    /// Tauschen: --kas X [--min-ghost M] oder --ghost Y [--min-kas M]
    Swap {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        kas: Option<f64>,
        #[arg(long)]
        ghost: Option<f64>,
        /// Mindestens so viele GHOST, sonst Abbruch (Standard: 1 % unter dem aktuellen Kurs)
        #[arg(long)]
        min_ghost: Option<f64>,
        /// Mindestens so viele KAS, sonst Abbruch (Standard: 1 % unter dem aktuellen Kurs)
        #[arg(long)]
        min_kas: Option<f64>,
    },
}

#[derive(Subcommand)]
enum AboCmd {
    /// Neuen Dauerauftrag anlegen (sendet nichts)
    Add {
        /// Schlüsseldatei des Absenders
        #[arg(long)]
        key: PathBuf,
        /// KAS oder GHOST
        #[arg(long)]
        asset: String,
        /// Adresse, Schlüsseldatei oder (nur GHOST) x-only-Pubkey
        #[arg(long)]
        to: String,
        /// Betrag je Termin, z. B. 12.5
        #[arg(long)]
        amount: String,
        /// daily | weekly | monthly | Anzahl Tage
        #[arg(long)]
        interval: String,
        /// erster Termin JJJJ-MM-TT (Standard: heute)
        #[arg(long)]
        start: Option<String>,
        /// letzter möglicher Termin JJJJ-MM-TT
        #[arg(long)]
        end: Option<String>,
        /// Anzahl der Termine insgesamt
        #[arg(long)]
        count: Option<u32>,
        /// Nachricht, z. B. „Miete“ (höchstens 100 Zeichen); bei jeder Zahlung
        /// neu verschlüsselt an den Empfänger in der Transaktion
        #[arg(long)]
        message: Option<String>,
        /// Nachricht bei jeder Zahlung stattdessen öffentlich (Klartext) in die Transaktion schreiben
        #[arg(long)]
        onchain_message: bool,
    },
    /// Alle Daueraufträge dieses Netzes
    List,
    /// Anhalten, bis `resume`
    Pause { id: String },
    /// Fortsetzen; Termine während der Pause werden nicht nachgeholt
    Resume { id: String },
    /// Beenden (ins Archiv, nichts wird gelöscht)
    Remove { id: String },
    /// Alle fälligen Aufträge ausführen (je Auftrag höchstens eine Zahlung)
    Run {
        /// anderes „heute“ (JJJJ-MM-TT), nur zusammen mit --dry-run
        #[arg(long)]
        today: Option<String>,
    },
}

#[derive(Subcommand, Clone)]
enum TresorCmd {
    /// Tresor anlegen: Startguthaben aus eigenen KAS in den Vertrag
    Open {
        /// Schlüsseldatei des Absenders
        #[arg(long)]
        key: PathBuf,
        /// Empfänger: Kaspa-Adresse (kaspa:q…), Schlüsseldatei oder x-only-Pubkey
        #[arg(long)]
        to: String,
        /// KAS je Zahlung (mindestens 1)
        #[arg(long)]
        amount: String,
        /// monthly | weekly | daily | Anzahl Tage
        #[arg(long)]
        interval: String,
        /// erster Termin JJJJ-MM-TT (UTC; Standard: heute)
        #[arg(long)]
        start: Option<String>,
        /// Uhrzeit der Termine HH:MM in UTC (Standard 00:00)
        #[arg(long)]
        time: Option<String>,
        /// Anzahl der Zahlungen (ohne Angabe: unbegrenzt)
        #[arg(long)]
        count: Option<u32>,
        /// Startguthaben in KAS (Standard mit --count: Anzahl × (Betrag + Höchstgebühr) + 1 KAS)
        #[arg(long)]
        fund: Option<String>,
        /// Höchstgebühr je Zahlung in KAS (Standard 0.01, mindestens 0.004, höchstens 0.1). So viel
        /// darf je Zahlung zusätzlich zum Betrag aus dem Tresor; den Teil, der nicht
        /// als Netzgebühr gebraucht wird, darf ein fremder Auslöser behalten
        #[arg(long)]
        max_fee: Option<String>,
        /// Beschreibung, z. B. „Miete“ (höchstens 100 Zeichen); steht im Tresor-Code
        #[arg(long)]
        message: Option<String>,
        /// Beschreibung bei jeder Zahlung öffentlich in die Transaktion schreiben
        #[arg(long)]
        onchain_message: bool,
    },
    /// Alle bekannten Tresore dieses Netzes (ohne Netz)
    List {
        /// Schlüssel des Empfängers: verschlüsselte Nachrichten der Tresore an
        /// ihn entschlüsseln und mit der Beschreibung vergleichen
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Tresor-Code für den Empfänger ausgeben (ohne Netz)
    Code { id: String },
    /// Über die Browser-Wallet angelegte Tresore eines Besitzers als JSON (ohne
    /// Netz, liest nur; ohne Pfade von Schlüsseldateien): „Meine Tresore“ der
    /// öffentlichen Seite
    Owned {
        /// Besitzer: Kaspa-Adresse (kaspa:q…) oder x-only-Pubkey (64 Hex)
        owner: String,
    },
    /// Tresor aus einem Tresor-Code übernehmen; am Node geprüft, sendet nichts.
    /// Liegt der Schlüssel des Empfängers in keys/ (oder mit --key), wird eine
    /// verschlüsselte Nachricht entschlüsselt und muss die Beschreibung ergeben
    Import {
        code: String,
        /// Schlüssel des Empfängers (Standard: passende Datei in keys/)
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// Alle Tresore mit dem Netz abgleichen (sendet nichts)
    Sync,
    /// Fällige Zahlung auslösen (darf jeder); ohne ID alle fälligen. Die
    /// Gebühr kommt aus dem Tresor (nur die nötige, der Rest bleibt darin).
    /// ghostctl zahlt nur, wenn danach mindestens 1 KAS im Tresor bleibt (der
    /// Vertrag selbst verlangt nur einen Rest über 0)
    Pay {
        id: Option<String>,
        /// eigener Schlüssel, falls mit der Gebühr aus dem Tresor keine 1 KAS
        /// darin blieben: dann zahlt dieser Schlüssel die Netzgebühr
        #[arg(long)]
        key: Option<PathBuf>,
    },
    /// KAS nachlegen (nur der Absender); Termine bleiben
    Topup {
        id: String,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        kas: String,
    },
    /// Kündigen: der Rest geht an den Absender, der Tresor endet (nur der Absender)
    Cancel {
        id: String,
        #[arg(long)]
        key: PathBuf,
    },
}

#[derive(Subcommand, Clone)]
enum WalletCmd {
    /// Unsignierte Tx für die Wallet bauen (sendet nichts). Stufen:
    /// dry-cancel (erfundener Tresor, nur lokal), tresor-open (Probe-Tresor
    /// anlegen), tresor-cancel (Probe-Tresor kündigen)
    ExportUnsigned {
        /// dry-cancel | tresor-open | tresor-cancel
        stage: String,
        /// Kaspa-Adresse des Wallet-Kontos (kaspa:q…)
        #[arg(long)]
        address: String,
        /// tresor-open: Startguthaben in KAS (Standard 1.5)
        #[arg(long)]
        fund: Option<f64>,
        /// tresor-open: erster (einziger) Termin in so vielen Minuten (Standard 60)
        #[arg(long)]
        due_minutes: Option<i64>,
        /// tresor-cancel: Probe-Datei (Ausgabe „probe“ von attach-sigs nach tresor-open)
        #[arg(long)]
        probe: Option<PathBuf>,
    },
    /// Signierte Tx der Wallet übernehmen: Signaturen prüfen, Signaturskripte
    /// bauen, Budgets/Masse setzen, lokal prüfen. Sendet nur mit --send
    AttachSigs {
        /// Plan (Ausgabe von export-unsigned)
        #[arg(long)]
        plan: PathBuf,
        /// Antwort der Wallet (Safe JSON, auch als JSON-Text)
        #[arg(long)]
        signed: PathBuf,
        /// fertige Tx senden (Mainnet: mit Rückfrage, außer --ja)
        #[arg(long)]
        send: bool,
        /// nach dem Senden den neuen Stand des Probe-Tresors hierhin schreiben
        #[arg(long)]
        save_probe: Option<PathBuf>,
    },
    /// Nutzeraktion für die Browser-Wallet bauen (sendet nichts, schreibt
    /// nichts, braucht keine Schlüsseldatei): Plan-JSON mit unsignierter Tx und
    /// Anfragen für KasWare (signPskt) und Kastle (signTx). Aktionen:
    /// open-vault, mint, repay, deposit, withdraw, close, redeem, liquidate,
    /// sweep, send, transfer, swap, pool-add, pool-remove; Tresore (Zustand
    /// ist die Tresor-Datei): tresor-open, tresor-topup, tresor-cancel
    Build {
        action: String,
        /// Kaspa-Adresse der Wallet (kaspa:q…); Gebühr und Einlagen kommen von dort
        #[arg(long)]
        address: String,
        #[arg(long, conflicts_with = "vault_id")]
        vault: Option<usize>,
        /// Vault über seine Covenant-ID (64 Hex) statt über die Nummer: die Nummer
        /// verschiebt sich, wenn ein Vault mit kleinerer Nummer endet (A11-O-15)
        #[arg(long)]
        vault_id: Option<String>,
        #[arg(long)]
        kas: Option<f64>,
        #[arg(long)]
        ghost: Option<f64>,
        /// withdraw: verbleibende Sicherheit in KAS
        #[arg(long)]
        keep: Option<f64>,
        /// send: Kaspa-Adresse; transfer: Schnorr-Adresse oder x-only-Pubkey (64 Hex)
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        onchain_message: bool,
        /// swap: Mindestbetrag (Standard 1 % unter dem aktuellen Kurs)
        #[arg(long)]
        min: Option<f64>,
        #[arg(long)]
        min_shares: Option<i64>,
        /// pool-remove: Prozent der eigenen Anteile (Standard 100)
        #[arg(long)]
        percent: Option<f64>,
        #[arg(long)]
        min_kas: Option<f64>,
        #[arg(long)]
        min_ghost: Option<f64>,
        /// tresor-open: KAS je Zahlung (mindestens 1)
        #[arg(long)]
        amount: Option<f64>,
        /// tresor-open: monthly | weekly | daily | Anzahl Tage
        #[arg(long)]
        interval: Option<String>,
        /// tresor-open: erster Termin JJJJ-MM-TT (UTC; Standard: heute)
        #[arg(long)]
        start: Option<String>,
        /// tresor-open: Uhrzeit der Termine HH:MM in UTC (Standard 00:00)
        #[arg(long)]
        time: Option<String>,
        /// tresor-open: Anzahl der Zahlungen (ohne Angabe: unbegrenzt)
        #[arg(long)]
        count: Option<u32>,
        /// tresor-open: Startguthaben in KAS (Standard mit --count: Anzahl × (Betrag + Höchstgebühr) + 1 KAS)
        #[arg(long)]
        fund: Option<f64>,
        /// tresor-open: Höchstgebühr je Zahlung in KAS (Standard 0.01)
        #[arg(long)]
        max_fee: Option<f64>,
        /// tresor-topup, tresor-cancel: volle Covenant-ID des Tresors (64 Hex)
        #[arg(long)]
        tresor: Option<String>,
    },
    /// Signierte Tx der Wallet zu einem Plan von `wallet build` übernehmen: Plan
    /// aus dem Zustand neu bauen und vergleichen, Signaturen prüfen, mit den
    /// Signaturen bauen, lokal wie der Konsens prüfen. Sendet nur mit --send
    /// (über das Journal; Zustand wird erst bei Annahme geschrieben)
    Submit {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        signed: PathBuf,
        #[arg(long)]
        send: bool,
    },
    /// Sicherheitsnetz: die eine Zahlung des Probe-Tresors nach dem Termin
    /// auslösen (keine Signatur nötig, 1 KAS an die Wallet). Sendet nur mit --send
    ProbePay {
        #[arg(long)]
        probe: PathBuf,
        #[arg(long)]
        send: bool,
        #[arg(long)]
        save_probe: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum SignerCmd {
    /// Aktueller Satz, Notfallsatz und offene Ankündigung
    Show,
    /// Neuen Satz ankündigen (gilt erst nach der Wartezeit, öffentlich sichtbar).
    /// Signieren muss die Austausch-Schwelle des aktuellen Satzes.
    Propose {
        /// zahlt die Gebühr und 1 KAS für das Ticket
        #[arg(long)]
        key: PathBuf,
        /// Schlüssel des aktuellen Satzes
        #[arg(long)]
        committee: PathBuf,
        /// neuer Satz: Komitee-Datei …
        #[arg(long)]
        new_committee: Option<PathBuf>,
        /// … oder x-only-Pubkeys (hex, durch Komma getrennt)
        #[arg(long)]
        new_keys: Option<String>,
        /// Preis-Schwelle des neuen Satzes (Standard: Mehrheit)
        #[arg(long)]
        threshold: Option<i64>,
        /// Austausch-Schwelle des neuen Satzes (Standard: = Preis-Schwelle)
        #[arg(long)]
        rotate_threshold: Option<i64>,
        /// Notfallsatz nach dem Austausch: x-only-Pubkeys (hex, Komma); ohne Angabe keiner
        #[arg(long)]
        fallback_keys: Option<String>,
        #[arg(long)]
        fallback_threshold: Option<i64>,
        /// Notfallweg: signiert vom Notfallsatz, erst nach langer Stille des Orakels
        #[arg(long)]
        emergency: bool,
    },
    /// Offene Ankündigung absagen (Preis-Schwelle des aktuellen Satzes)
    Cancel {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        committee: PathBuf,
    },
    /// Fällige Ankündigung in Kraft setzen (jeder darf, nach der Wartezeit)
    Activate {
        #[arg(long)]
        key: PathBuf,
    },
    /// Abgesagtes Ticket aufräumen (seine 1 KAS gehen an --key)
    Clear {
        #[arg(long)]
        key: PathBuf,
    },
}

/// Zwischenstand eines Deployments (Audit 4 F2). Version 4: fünf Schritte
/// (Register-Genesis, Orakel-Genesis, Register-init, Factory-Genesis, Factory-init)
#[derive(Clone, Serialize, Deserialize)]
struct DeployProgress {
    network: String,
    register_params: RegisterParams,
    register_genesis: RegisterState,
    signer_set: SignerSet,
    /// Orakel ohne reg_cov (folgt aus Schritt 1)
    max_rate: i64,
    rate_step: i64,
    rate_gap_daa: i64,
    freeze_after_daa: i64,
    oracle_state: OracleState,
    factory_params: FactoryParams,
    values: ops::CovValues,
    max_debt: i64,
    register: Option<ops::Tracked<RegisterState>>,
    oracle_params: Option<OracleParams>,
    oracle: Option<ops::Tracked<OracleState>>,
    register_ready: bool,
    factory: Option<ops::Tracked<FactoryState>>,
}

#[derive(Serialize, Deserialize)]
struct KeyFile {
    secret: String,
}

#[derive(Serialize, Deserialize)]
struct CommitteeFile {
    secrets: Vec<String>,
}

fn new_key() -> Keypair {
    Keypair::new(&Secp256k1::new(), &mut rand::thread_rng())
}

fn key_from_hex(h: &str) -> Result<Keypair, String> {
    let mut b = [0u8; 32];
    faster_hex::hex_decode(h.as_bytes(), &mut b).map_err(|e| e.to_string())?;
    let sk = SecretKey::from_slice(&b).map_err(|e| e.to_string())?;
    Ok(Keypair::from_secret_key(&Secp256k1::new(), &sk))
}

fn load_key(p: &Path) -> Result<Keypair, String> {
    let f: KeyFile = serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| e.to_string())?;
    key_from_hex(&f.secret)
}

fn load_committee(p: &Path) -> Result<Vec<Keypair>, String> {
    let f: CommitteeFile = serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?).map_err(|e| e.to_string())?;
    f.secrets.iter().map(|s| key_from_hex(s)).collect()
}

/// Legt eine Schlüsseldatei an: nie überschreiben, von Anfang an nur für den
/// Besitzer lesbar (Audit 4 F10: vorher kurz 0644 bis zum chmod).
fn write_new(p: &Path, content: &str) -> Result<(), String> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut f = opts.open(p).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!("{} existiert schon – wird nicht überschrieben", p.display())
        } else {
            format!("{}: {e}", p.display())
        }
    })?;
    f.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

struct Ctx {
    net: Net,
    network: String,
    state_path: PathBuf,
    mainnet: bool,
    yes: bool,
    dry_run: bool,
}

impl Ctx {
    /// Zustandsdatei lesen, ohne Netzabgleich.
    fn load(&self) -> Result<Deployment, String> {
        let s = std::fs::read_to_string(&self.state_path).map_err(|_| format!("keine Zustandsdatei {} – zuerst `deploy`", self.state_path.display()))?;
        let v: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
        // ältere Versionen klar benennen, statt an einem Feld zu scheitern
        if let Some(ver) = old_version(&v) {
            return Err(old_version_error(&self.state_path, ver));
        }
        let d: Deployment = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if d.network != self.network {
            return Err(format!("Zustandsdatei {} gehört zum Netz {}, gewählt ist {}", self.state_path.display(), d.network, self.network));
        }
        Ok(d)
    }
    /// Offene Tx klären, dann mit dem Netz abgleichen (Audit 4 F1/F4, Audit 3 O-1).
    async fn load_synced(&self) -> Result<Deployment, String> {
        if let Some(msg) = store::resolve_pending(&self.net, &self.state_path).await? {
            say!("Hinweis: {msg}");
        }
        let mut d = self.load()?;
        let mut notes = store::resync(&self.net, &mut d).await?;
        // Tauschpool: Dritte tauschen laufend; aktuelle UTXO über die feste Adresse
        if let Some(rec) = d.pool.clone() {
            match pool::resync(&self.net, &self.network, &rec).await {
                Ok(n) if n.pool.outpoint != rec.pool.outpoint => {
                    notes.push(format!("Pool nachgeladen: {:.4} KAS / {:.8} GHOST", n.pool.value as f64 / 1e8, n.reserve.state.amount as f64 / 1e8));
                    d.pool = Some(n);
                }
                Ok(_) => {}
                // nie löschen: ein offener Pool lässt sich nicht auflösen (A10-P-3)
                Err(e) => d.pool_unresolved = Some(e),
            }
        }
        for n in &notes {
            say!("Hinweis: {n}");
        }
        if !notes.is_empty() && !self.dry_run {
            self.save(&d)?;
        }
        Ok(d)
    }
    fn save(&self, d: &Deployment) -> Result<(), String> {
        store::atomic_write(&self.state_path, &serde_json::to_string_pretty(d).unwrap())
    }
    fn confirm(&self, what: &str, b: &Built) -> Result<(), String> {
        say!("→ {what}: Gebühr {:.4} KAS, {} Inputs, {} Outputs", b.fee as f64 / 1e8, b.tx.inputs.len(), b.tx.outputs.len());
        if self.mainnet && !self.yes {
            eprint!("  MAINNET – wirklich senden? [j/N] ");
            std::io::stderr().flush().ok();
            let mut a = String::new();
            std::io::stdin().read_line(&mut a).ok();
            if a.trim().to_lowercase() != "j" {
                return Err("abgebrochen".into());
            }
        }
        Ok(())
    }
    /// Senden, auf Bestätigung warten, Zustand speichern.
    async fn send(&self, what: &str, b: &Built, next: Option<&Deployment>) -> Result<(), String> {
        let target = next.map(|d| (self.state_path.clone(), serde_json::to_value(d).unwrap()));
        self.send_to(what, b, target).await
    }
    /// Senden mit Journal: vor dem Senden wird festgehalten, welche Datei bei
    /// Annahme welchen Inhalt bekommt. Bricht ghostctl danach ab, übernimmt der
    /// nächste Aufruf den Zustand (store::resolve_pending).
    async fn send_to(&self, what: &str, b: &Built, target: Option<(PathBuf, serde_json::Value)>) -> Result<(), String> {
        let mut rec = serde_json::json!({
            "action": what,
            "txid": b.tx.id().to_string(),
            "feeKas": b.fee as f64 / 1e8,
            "inputs": b.tx.inputs.len(),
            "outputs": b.tx.outputs.len(),
            "donatedKas": b.donated as f64 / 1e8,
            "sent": false,
            "confirmed": false,
        });
        if b.donated > 0 {
            say!("  Hinweis: {:.8} KAS Restbetrag sind zu klein für einen eigenen Ausgang und gehen als Gebühr an die Miner.", b.donated as f64 / 1e8);
        }
        if self.dry_run {
            say!("→ {what}: Gebühr {:.4} KAS – Probelauf, nicht gesendet", b.fee as f64 / 1e8);
            TXS.lock().unwrap().push(rec);
            return Ok(());
        }
        self.confirm(what, b)?;
        let (tpath, tval) = match target {
            Some((p, v)) => (Some(p), Some(v)),
            None => (None, None),
        };
        let spks: Vec<_> = b.entries.iter().map(|e| e.script_public_key.clone()).collect();
        store::write_pending(&self.state_path, what, &b.tx, &spks, b.change_index, tpath.clone(), tval.clone(), false)?;
        let id = match self.net.submit(b).await {
            Ok(id) => id,
            Err(e) => {
                // abgelehnt: Journal nur löschen, wenn die Tx sicher nicht im Netz ist
                if matches!(self.net.in_mempool(b.tx.id()).await, Ok(false)) {
                    store::clear_pending(&self.state_path);
                }
                return Err(e);
            }
        };
        rec["sent"] = true.into();
        rec["txid"] = id.to_string().into();
        TXS.lock().unwrap().push(rec.clone());
        say!("  gesendet: {id}");
        store::wait_accepted(&self.net, &b.tx, Duration::from_secs(120), Duration::from_secs(600)).await?;
        say!("  bestätigt");
        if let Some(r) = TXS.lock().unwrap().last_mut() {
            r["confirmed"] = true.into();
        }
        if let (Some(p), Some(v)) = (tpath, tval) {
            store::atomic_write(&p, &serde_json::to_string_pretty(&v).unwrap())?;
        }
        store::clear_pending(&self.state_path);
        Ok(())
    }
    /// Wallet-Tx senden (Audit 17 A17-6): Journal und Senden unter der
    /// Sperre `lock`, dann Sperre frei und OHNE Sperre höchstens
    /// WALLET_WAIT_MAX auf die Bestätigung warten. Der Agent wartet so nie
    /// länger als den Abgleich und das Senden selbst. Bestätigt: kurz neu
    /// sperren und das Journal auflösen (Zustand übernehmen); klappt das nicht,
    /// macht es der nächste Abgleich. Ok(true) = bestätigt, Ok(false) =
    /// gesendet, Bestätigung steht aus (das Journal klärt den Rest, A17-2).
    /// `next` = Folgezustand der Datei `self.state_path` (deployments/<netz>.json
    /// oder bei Tresor-Aktionen die Tresor-Datei).
    async fn send_wallet(&self, lock: store::Lock, what: &str, b: &Built, next: &impl serde::Serialize) -> Result<bool, String> {
        let mut rec = serde_json::json!({
            "action": what,
            "txid": b.tx.id().to_string(),
            "feeKas": b.fee as f64 / 1e8,
            "inputs": b.tx.inputs.len(),
            "outputs": b.tx.outputs.len(),
            "donatedKas": b.donated as f64 / 1e8,
            "sent": false,
            "confirmed": false,
        });
        self.confirm(what, b)?;
        let spks: Vec<_> = b.entries.iter().map(|e| e.script_public_key.clone()).collect();
        let target = Some(self.state_path.clone());
        store::write_pending(&self.state_path, what, &b.tx, &spks, b.change_index, target, Some(serde_json::to_value(next).unwrap()), true)?;
        let send = async {
            match self.net.submit(b).await {
                Ok(id) => Ok(id),
                Err(e) => {
                    // abgelehnt: Journal nur löschen, wenn die Tx sicher nicht im Netz ist
                    if matches!(self.net.in_mempool(b.tx.id()).await, Ok(false)) {
                        store::clear_pending(&self.state_path);
                    }
                    Err(e)
                }
            }
        };
        let wait = || store::wait_accepted(&self.net, &b.tx, WALLET_WAIT_BASE, WALLET_WAIT_MAX);
        let (id, waited) = match store::send_then_wait(lock, send, wait).await {
            Ok(x) => x,
            Err(e) => {
                if store::pending_path(&self.state_path).exists() {
                    // Journal steht noch: ob die Tx im Netz ist, ist unklar
                    rec["unclear"] = true.into();
                    TXS.lock().unwrap().push(rec);
                    return Err(format!("{e} – ob die Transaktion trotzdem im Netz ist, ist unklar; der nächste Abgleich klärt das"));
                }
                return Err(e);
            }
        };
        rec["sent"] = true.into();
        rec["txid"] = id.to_string().into();
        say!("  gesendet: {id}");
        if let Err(e) = waited {
            say!("  {e}");
            TXS.lock().unwrap().push(rec);
            return Ok(false);
        }
        say!("  bestätigt");
        rec["confirmed"] = true.into();
        TXS.lock().unwrap().push(rec);
        match store::lock(&self.state_path, WALLET_RELOCK_WAIT) {
            Ok(_l) => match store::resolve_pending(&self.net, &self.state_path).await {
                Ok(Some(m)) => say!("  {m}"),
                Ok(None) => {}
                Err(e) => say!("  Zustand übernimmt der nächste Abgleich ({e})"),
            },
            Err(_) => say!("  Zustand übernimmt der nächste Abgleich (Sperre belegt)"),
        }
        Ok(true)
    }
    async fn funds(&self, k: &Keypair) -> Result<Funds, String> {
        self.net.funds(k, 8).await
    }
    /// Prüft, ob die gespeicherten Covenant-UTXOs noch existieren.
    async fn check_fresh(&self, d: &Deployment) -> Result<(), String> {
        let oart = oracle(&d.oracle_params, &d.oracle.state);
        if !self.net.exists(&spk(&oart), &d.oracle.outpoint).await? {
            return Err("Orakel-UTXO nicht mehr vorhanden – jemand anderes hat es benutzt. Zustandsdatei ist veraltet.".into());
        }
        Ok(())
    }
}

fn to_units(x: f64) -> i64 {
    (x * 1e8).round() as i64
}

/// Betrag aus der Kommandozeile: endlich, > 0, höchstens 8 Nachkommastellen
/// sinnvoll, nicht absurd groß (Audit 4 F6: 0/negativ führte zu Überlauf/Panik).
fn amount(x: f64, name: &str) -> Result<i64, String> {
    if !x.is_finite() || x <= 0.0 {
        return Err(format!("{name} muss eine positive Zahl sein"));
    }
    if x > 1e10 {
        return Err(format!("{name} ist unrealistisch groß"));
    }
    let u = to_units(x);
    if u <= 0 {
        return Err(format!("{name} ist kleiner als die kleinste Einheit (1e-8)"));
    }
    Ok(u)
}

fn usable_vault(d: &Deployment, i: usize) -> Result<(), String> {
    let v = d.vaults.get(i).ok_or(format!("Vault {i} gibt es nicht (vorhanden: {})", d.vaults.len()))?;
    if v.stale {
        return Err(format!("Vault {i} wurde von Dritten verändert; sein Zustand ist unbekannt. Aktionen darauf sind gesperrt."));
    }
    Ok(())
}

/// Prägen, Tilgen, Einzahlen, Abheben und Schließen verlangen die Signatur
/// des Besitzers (Version 2.1). Früh und verständlich abweisen statt mit
/// einem Skriptfehler aus der Simulation.
fn owned_vault(d: &Deployment, i: usize, k: &Keypair) -> Result<(), String> {
    usable_vault(d, i)?;
    if d.vaults[i].owner != xonly(k) {
        return Err(format!("Vault {i} gehört nicht zu diesem Schlüssel; diese Aktion darf nur der Besitzer ausführen."));
    }
    Ok(())
}

/// Führt eigene GHOST-UTXOs per Selbstüberweisung zusammen, bis die zwei
/// größten zusammen `need` decken (eine GHOST-Gruppe fasst neben dem
/// Minter-Zweig nur 2 Token-Inputs; Audit 4 F12).
async fn consolidate(ctx: &Ctx, mut d: Deployment, k: &Keypair, need: i64, p: &kaspa_consensus_core::config::params::Params) -> Result<Deployment, String> {
    loop {
        let mut mine: Vec<usize> = d.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(k)).map(|(i, _)| i).collect();
        mine.sort_by_key(|&i| std::cmp::Reverse(d.tokens[i].state.amount));
        let total: i64 = mine.iter().map(|&i| d.tokens[i].state.amount).sum();
        let top2: i64 = mine.iter().take(2).map(|&i| d.tokens[i].state.amount).sum();
        if top2 >= need || mine.len() <= 2 {
            return Ok(d);
        }
        if total < need {
            return Err(format!("zu wenig GHOST: {:.8} vorhanden, {:.8} nötig", total as f64 / 1e8, need as f64 / 1e8));
        }
        if ctx.dry_run {
            return Err("Die GHOST liegen auf mehr als zwei UTXOs verteilt; vor dieser Aktion wird zuerst zusammengeführt. Das lässt sich nicht als Probelauf vorab zeigen – bitte echt ausführen oder vorher `transfer` an dich selbst.".into());
        }
        let three: Vec<usize> = mine.into_iter().take(3).collect();
        let sum: i64 = three.iter().map(|&i| d.tokens[i].state.amount).sum();
        say!("GHOST liegen auf mehreren UTXOs – führe {} zusammen …", three.len());
        let (b, nd) = ops::transfer(&d, k, &three, &xonly(k), sum, &ctx.funds(k).await?, p)?;
        ctx.send("GHOST zusammenführen", &b, Some(&nd)).await?;
        d = nd;
    }
}

/// Startzins: 0 … 20 % p. a. auf dem 0,5-Raster der Zinsregel (Audit 14 H3/M-2)
fn check_start_rate(rate: f64) -> Result<(), String> {
    if !rate.is_finite() || !(0.0..=math::RATE_MAX_PCT).contains(&rate) || (rate / math::RATE_STEP_PCT).fract() != 0.0 {
        return Err(format!("--rate: 0 bis {} % p. a. in Schritten von {} Punkten", math::RATE_MAX_PCT, math::RATE_STEP_PCT));
    }
    Ok(())
}

/// Version einer älteren Zustandsdatei (None = Version 4). Erkennung über
/// Felder, die nur bestimmte Versionen haben (Audit 15 G-7).
fn old_version(v: &serde_json::Value) -> Option<&'static str> {
    if v.get("register").is_some() {
        return None;
    }
    let vp = v.get("vault_params");
    Some(if vp.is_some_and(|p| p.get("treasury").is_some()) {
        "3"
    } else if vp.is_some_and(|p| p.is_object() && p.get("max_debt").is_none()) {
        "1"
    } else {
        "2"
    })
}

fn old_version_error(path: &Path, version: &str) -> String {
    format!(
        "{} stammt von Version {version}. Diese ghostctl-Version bedient nur Version 4 (ältere Versionen: ghostctl im Ordner kaspa-lending bzw. nach dem Umzug bin/ghostctl-v{version}); für Version 4 --state angeben",
        path.display()
    )
}

/// Register-Parameter v4. Ein einziges Orakel ist erlaubt (Nutzer,
/// 04.10.2026: zunächst Alleinbetrieb, weitere Unterzeichner später per
/// Austausch). Fristen: Austausch 14 Tage, Notfall nach 30 Tagen Stille plus
/// 14 Tage; Probe je 1 h.
fn deploy_register_params(deployer: &Keypair, probe: bool) -> RegisterParams {
    let (rot, after, delay) = if probe { (HOUR_DAA, HOUR_DAA, HOUR_DAA) } else { (14 * DAY_DAA, 30 * DAY_DAA, 14 * DAY_DAA) };
    RegisterParams { deployer: xonly(deployer), min_signers: 1, min_threshold: 1, rot_delay_daa: rot, emerg_after_daa: after, emerg_delay_daa: delay }
}

fn rate_from_apr(pct: f64) -> i64 {
    // Zins je DAA × 1e18 bei 10 DAA/s: apr / (10 · 31 536 000)
    (pct / 100.0 / 315_360_000.0 * 1e18).round() as i64
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    // status --json ist ebenfalls maschinenlesbar: Hinweise nach stderr (Fix-Review N-5)
    let json = cli.json;
    let pure_stdout = json || matches!(cli.cmd, Cmd::Status { json: true });
    JSON_MODE.store(pure_stdout, std::sync::atomic::Ordering::Relaxed);
    match run(cli).await {
        Ok(Some(v)) if json => println!("{}", serde_json::to_string(&v).unwrap()),
        Ok(_) => {}
        Err(e) => {
            if json {
                let txs = std::mem::take(&mut *TXS.lock().unwrap());
                println!("{}", serde_json::json!({ "ok": false, "error": e, "transactions": txs }));
            } else {
                eprintln!("Fehler: {e}");
            }
            std::process::exit(1);
        }
    }
}

async fn run(cli: Cli) -> Result<Option<serde_json::Value>, String> {
    use serde_json::json;
    // Befehle ohne Netz
    match &cli.cmd {
        Cmd::Keys { dir } => {
            // Schlüsseldateien liegen lokal. Sie erscheinen auch dann, wenn kein
            // Node antwortet, nur ohne KAS-Guthaben (sonst hing die Seite
            // minutenlang auf „lädt …" und zeigte danach gar kein Konto).
            let net = match tokio::time::timeout(Duration::from_secs(60), Net::connect(&cli.network, cli.rpc.as_deref())).await {
                Ok(Ok(n)) => Some(n),
                Ok(Err(e)) => {
                    say!("Hinweis: kein Node erreichbar ({e}); KAS-Guthaben unbekannt");
                    None
                }
                Err(_) => {
                    say!("Hinweis: kein Node erreichbar (Zeitlimit); KAS-Guthaben unbekannt");
                    None
                }
            };
            let sp = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
            let list = list_keys(net.as_ref(), &cli.network, &sp, dir).await?;
            for k in &list {
                say!("{}", k);
            }
            return Ok(Some(json!({ "ok": true, "keys": list, "offline": net.is_none(), "transactions": [] })));
        }
        Cmd::Messages { key, limit } => {
            // REST-API und lokale Zustandsdatei; der Node prüft nur nach, wenn erreichbar
            let k = load_key(key)?;
            let sp = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
            return messages(&cli.network, cli.rpc.as_deref(), &k, &sp, (*limit).clamp(1, 1000)).await.map(Some);
        }
        Cmd::Keygen { out } => {
            let k = new_key();
            write_new(out, &serde_json::to_string_pretty(&KeyFile { secret: faster_hex::hex_string(&k.secret_bytes()) }).unwrap())?;
            say!("Schlüssel in {} (Rechte 600). x-only-Pubkey: {}", out.display(), faster_hex::hex_string(&xonly(&k)));
            return Ok(Some(json!({ "ok": true, "file": out, "xonly": faster_hex::hex_string(&xonly(&k)) })));
        }
        Cmd::CommitteeKeygen { out, count } => {
            if !(1..=MAX_SIGNERS as usize).contains(count) {
                return Err(format!("--count: 1 bis {MAX_SIGNERS}"));
            }
            let ks: Vec<Keypair> = (0..*count).map(|_| new_key()).collect();
            let f = CommitteeFile { secrets: ks.iter().map(|k| faster_hex::hex_string(&k.secret_bytes())).collect() };
            write_new(out, &serde_json::to_string_pretty(&f).unwrap())?;
            let pubs: Vec<String> = ks.iter().map(|k| faster_hex::hex_string(&xonly(k))).collect();
            say!("{count} Unterzeichner-Schlüssel in {} (Rechte 600). Öffentlich: {}", out.display(), pubs.join(","));
            return Ok(Some(json!({ "ok": true, "file": out, "xonly": pubs })));
        }
        Cmd::Price => {
            let qs = tokio::task::spawn_blocking(price::fetch_all).await.map_err(|e| e.to_string())?;
            let ok: Vec<price::Quote> = qs.into_iter().filter_map(|q| q.map_err(|e| say!("  ✗ {e}")).ok()).collect();
            for q in &ok {
                say!("  {:<14} {:.6} USD", q.source, q.usd);
            }
            let median = price::median_price(&ok, 0.03)? as f64 / 1e8;
            say!("Median: {median:.6} USD");
            let quotes: Vec<_> = ok.iter().map(|q| json!({ "source": q.source, "usd": q.usd })).collect();
            return Ok(Some(json!({ "ok": true, "median": median, "quotes": quotes })));
        }
        Cmd::Abo { cmd } => {
            let sp = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
            if let Some(v) = abo_offline(&cli.network, &sp, cmd, cli.dry_run)? {
                return Ok(Some(v));
            }
        }
        Cmd::Wallet { cmd: cmd @ (WalletCmd::Build { .. } | WalletCmd::Submit { .. }) } => {
            // Nutzeraktionen: deployments/ lesen, schreiben nur bei --send über das Journal
            let sp = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
            return wallet_action_cmd(&cli.network, cli.rpc.as_deref(), &sp, cli.ja, cmd).await.map(Some);
        }
        Cmd::Wallet { cmd } => {
            // eigener Weg: kein deployments/, keine Sperre, kein Journal
            return wallet_cmd(&cli.network, cli.rpc.as_deref(), cli.ja, cmd).await.map(Some);
        }
        Cmd::Tresor { cmd } => {
            let sp = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
            if let Some(v) = tresor_offline(&cli.network, &tresor::path_for(&sp), cmd, cli.dry_run)? {
                return Ok(Some(v));
            }
        }
        _ => {}
    }

    let network_name = cli.network.clone();
    let mut extra = serde_json::Map::new();
    // Ohne Zustandsdatei gibt es nichts abzugleichen: sofort antworten, auch
    // wenn die Nodes gerade nicht erreichbar sind.
    if matches!(cli.cmd, Cmd::Status { json: true }) || (cli.json && matches!(cli.cmd, Cmd::Status { .. })) {
        let sp = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
        if !sp.exists() {
            println!("{}", json!({ "network": cli.network, "deployed": false }));
            return Ok(None);
        }
    }
    let state_path = cli.state.clone().unwrap_or_else(|| PathBuf::from(format!("deployments/{}.json", cli.network)));
    // Audit 14 N5: auch Befehle ohne GHOST-Zustand (send, abo, tresor, agent)
    // nicht auf der Zustandsdatei einer älteren Version laufen lassen
    if let Ok(t) = std::fs::read_to_string(&state_path) {
        if let Some(ver) = serde_json::from_str::<serde_json::Value>(&t).ok().as_ref().and_then(old_version) {
            return Err(old_version_error(&state_path, ver));
        }
    }
    let net = Net::connect(&cli.network, cli.rpc.as_deref()).await?;
    // Tresore: eigene Datei, eigene Sperre, eigenes Journal (unabhängig von GHOST)
    let state_path = if matches!(cli.cmd, Cmd::Tresor { .. }) { tresor::path_for(&state_path) } else { state_path };
    let ctx = Ctx { mainnet: cli.network == "mainnet", yes: cli.ja, dry_run: cli.dry_run, network: cli.network.clone(), net, state_path };
    let p = ctx.net.params.clone();
    let _lock = match cli.cmd {
        Cmd::OracleFeed { .. } | Cmd::Agent { .. } => None,
        // öffentlich erreichbar (.k-Namen, GHOST-Suche der Seite): nie die Hauptsperre
        // halten, auf die Orakel, Keeper und Wallet-Senden warten (Audit 19 A19-4);
        // receive sperrt nur kurz zum Speichern
        Cmd::Utxos { .. } | Cmd::Receive { owner: Some(_), .. } => None,
        _ => Some(store::lock(&ctx.state_path, Duration::from_secs(120))?),
    };

    match cli.cmd {
        Cmd::Keygen { .. } | Cmd::CommitteeKeygen { .. } | Cmd::Price | Cmd::Wallet { .. } => unreachable!(),
        Cmd::Balance { key } => {
            let k = load_key(&key)?;
            let addr = ctx.net.address_of_key(&k);
            let u = ctx.net.utxos(&addr).await?;
            let total: u64 = u.iter().filter(|(_, e)| e.covenant_id.is_none()).map(|(_, e)| e.amount).sum();
            say!("Adresse: {addr}\nGuthaben: {:.8} KAS in {} UTXOs", total as f64 / 1e8, u.len());
            extra.insert("address".into(), addr.to_string().into());
            extra.insert("kas".into(), (total as f64 / 1e8).into());
            if let Ok(d) = ctx.load() {
                let g: i64 = d.tokens.iter().filter(|t| t.state.owner == xonly(&k)).map(|t| t.state.amount).sum();
                say!("GHOST (laut Zustandsdatei): {:.8}", g as f64 / 1e8);
                extra.insert("ghost".into(), (g as f64 / 1e8).into());
            }
        }
        Cmd::Deploy { key, committee, rate, threshold, probe } => {
            let progress_file = ctx.state_path.with_extension("deploy.json");
            let taken_over = store::resolve_pending(&ctx.net, &ctx.state_path).await?;
            if let Some(msg) = &taken_over {
                say!("Hinweis: {msg}");
            }
            if ctx.state_path.exists() {
                if taken_over.is_some() && progress_file.exists() {
                    // Fix-Review 8 NEU-6: Der letzte Schritt war angenommen, nur das
                    // Speichern fehlte – das Deployment ist damit fertig.
                    let _ = std::fs::remove_file(&progress_file);
                    rate_restart(&RateFile::of(&ctx));
                    say!("Deployment ist abgeschlossen ({}).", ctx.state_path.display());
                    return Ok(None);
                }
                return Err(format!("{} existiert schon – in diesem Netz ist bereits angelegt", ctx.state_path.display()));
            }
            // Startzins im Rahmen des Orakels und auf dem Raster der Zinsregel (Audit 14 H3/M-2)
            check_start_rate(rate)?;
            let k = load_key(&key)?;
            let com = load_committee(&committee)?;
            // Schrittweise mit Fortschrittsdatei (Audit 4 F2): bricht der Lauf
            // ab, setzt der nächste Aufruf dort fort, statt einen zweiten Satz
            // Covenants anzulegen.
            let progress_path = ctx.state_path.with_extension("deploy.json");
            let mut prog: DeployProgress = match std::fs::read_to_string(&progress_path) {
                Ok(t) => {
                    say!("Setze angefangenes Deployment fort ({})", progress_path.display());
                    serde_json::from_str(&t).map_err(|e| e.to_string())?
                }
                Err(_) => {
                    let kas_usd = median_now().await?;
                    if !(1_000..=90_000_000_000).contains(&kas_usd) {
                        return Err(format!("Startpreis {:.8} USD außerhalb der Vertragsgrenzen", kas_usd as f64 / 1e8));
                    }
                    let daa = ctx.net.daa().await?;
                    let n = com.len() as i64;
                    let t = threshold.unwrap_or(n / 2 + 1);
                    let set = SignerSet { keys: com.iter().map(xonly).collect(), t, t_rot: t };
                    let rp = deploy_register_params(&k, probe);
                    set.check_bounds(&rp)?;
                    let me: [u8; 32] = xonly(&k).try_into().unwrap();
                    let reg = RegisterState::genesis(&set, None, 0, me, daa as i64);
                    // Zinsrahmen des Orakels = Rahmen der Zinsregel: 20 % p. a.,
                    // 0,5 Punkte je Schritt, höchstens stündlich (Audit 11 A11-V-9)
                    let os = OracleState {
                        kas_usd,
                        oracle_daa: daa as i64,
                        seq: 0,
                        stable_rate: rate_from_apr(rate),
                        stable_index: 1_000_000_000,
                        frozen: false,
                        last_rate_daa: daa as i64,
                    };
                    say!(
                        "Startpreis {:.6} USD, Zins {rate} % p. a., DAA {daa}, {n} Unterzeichner (Schwelle {t}){}",
                        kas_usd as f64 / 1e8,
                        if probe { " – PROBE: Fristen 1 h, höchstens 5 GHOST je Vault" } else { "" }
                    );
                    DeployProgress {
                        network: cli.network.clone(),
                        register_params: rp,
                        register_genesis: reg,
                        signer_set: set,
                        max_rate: rate_from_apr(math::RATE_MAX_PCT),
                        // +1: die Zinsregel rundet jeden Satz einzeln (rate_from_apr)
                        rate_step: rate_from_apr(math::RATE_STEP_PCT) + 1,
                        rate_gap_daa: HOUR_DAA,
                        freeze_after_daa: if probe { HOUR_DAA } else { 2 * HOUR_DAA },
                        oracle_state: os,
                        factory_params: FactoryParams { deployer: xonly(&k), ghost_tpl: ghost_template() },
                        // je 1 KAS auch im echten v4: die Mainnet-Probe (05.10.2026) lief damit
                        // durch, Speichermasse weit unter der Blockgrenze; 4 statt 40 KAS gebunden
                        values: ops::CovValues::small(),
                        max_debt: if probe { PROBE_MAX_DEBT } else { MAINNET_MAX_DEBT },
                        register: None,
                        oracle_params: None,
                        oracle: None,
                        register_ready: false,
                        factory: None,
                    }
                }
            };
            if prog.network != cli.network {
                return Err(format!("{} gehört zum Netz {}", progress_path.display(), prog.network));
            }
            // Fortsetzung nur mit denselben Schlüsseln (Fix-Review N-8); --rate und
            // Startpreis stammen aus dem ersten Aufruf
            if prog.signer_set.keys != com.iter().map(xonly).collect::<Vec<_>>() || prog.factory_params.deployer != xonly(&k) {
                return Err(format!("{} wurde mit anderen Schlüsseln begonnen – Fortsetzung abgelehnt", progress_path.display()));
            }
            // Audit 14 N4: geänderte Schalter nicht still übergehen
            let probe_was = prog.register_params.rot_delay_daa == HOUR_DAA;
            if probe_was != probe || threshold.is_some_and(|t| t != prog.signer_set.t) || rate_from_apr(rate) != prog.oracle_state.stable_rate {
                return Err(format!(
                    "{} wurde mit anderen Einstellungen begonnen ({}, Schwelle {}, Zins {:.2} %) – zum Fortsetzen dieselben angeben",
                    progress_path.display(),
                    if probe_was { "--probe" } else { "ohne --probe" },
                    prog.signer_set.t,
                    apr_of(prog.oracle_state.stable_rate)
                ));
            }
            // Audit 14/15 N4: solange das Orakel noch nicht angelegt ist, mit
            // aktuellem Preis starten statt mit dem des ersten Aufrufs
            if prog.oracle.is_none() {
                let kas_usd = median_now().await?;
                if !(1_000..=90_000_000_000).contains(&kas_usd) {
                    return Err(format!("Startpreis {:.8} USD außerhalb der Vertragsgrenzen", kas_usd as f64 / 1e8));
                }
                let daa = ctx.net.daa().await? as i64;
                prog.oracle_state = OracleState { kas_usd, oracle_daa: daa, last_rate_daa: daa, ..prog.oracle_state };
            }
            let save_prog = |next: &DeployProgress| Some((progress_path.clone(), serde_json::to_value(next).unwrap()));
            if prog.register.is_none() {
                let (b, r) = ops::deploy_register(&prog.register_params, prog.register_genesis.clone(), prog.values.register, &ctx.funds(&k).await?, &p)?;
                let mut next = prog.clone();
                next.register = Some(r);
                ctx.send_to("Register-Genesis", &b, save_prog(&next)).await?;
                prog = next;
            }
            if prog.oracle.is_none() {
                let reg_cov = prog.register.as_ref().unwrap().cov;
                let op = OracleParams { reg_cov, max_rate: prog.max_rate, rate_step: prog.rate_step, rate_gap_daa: prog.rate_gap_daa, freeze_after_daa: prog.freeze_after_daa };
                let (b, o) = ops::deploy_oracle(&op, prog.oracle_state, prog.values.oracle, &ctx.funds(&k).await?, &p)?;
                let mut next = prog.clone();
                next.oracle_params = Some(op);
                next.oracle = Some(o);
                ctx.send_to("Orakel-Genesis", &b, save_prog(&next)).await?;
                prog = next;
            }
            let op = prog.oracle_params.clone().unwrap();
            if !prog.register_ready {
                let (b, r) = ops::init_register(&prog.register_params, prog.register.as_ref().unwrap(), &prog.signer_set, &k, &op, prog.oracle.as_ref().unwrap(), &ctx.funds(&k).await?, &p)?;
                let mut next = prog.clone();
                next.register = Some(r);
                next.register_ready = true;
                ctx.send_to("Register-Init", &b, save_prog(&next)).await?;
                prog = next;
            }
            if prog.factory.is_none() {
                let (b, f) = ops::deploy_factory(&prog.factory_params, prog.values.factory, &ctx.funds(&k).await?, &p)?;
                let mut next = prog.clone();
                next.factory = Some(f);
                ctx.send_to("Factory-Genesis", &b, save_prog(&next)).await?;
                prog = next;
            }
            let (o, f) = (prog.oracle.clone().unwrap(), prog.factory.clone().unwrap());
            // Zinsziel: die eigene Adresse (Entscheidung des Nutzers, 04.10.2026)
            let interest = spk_bytes(&p2pk_spk(&xonly(&k)));
            let (b, f2, root, vp) =
                ops::init_factory(&op, &o, &prog.factory_params, &f, &k, (20_000, 15_000, 1_000, prog.max_debt), interest, prog.values.root, &ctx.funds(&k).await?, &p)?;
            let d = Deployment {
                network: cli.network.clone(),
                register_params: prog.register_params.clone(),
                register: prog.register.clone().unwrap(),
                signer_set: prog.signer_set.clone(),
                fallback_set: None,
                rotation: None,
                old_tickets: vec![],
                foreign_change: None,
                signers_unknown: false,
                oracle_params: op,
                oracle: o,
                factory_params: prog.factory_params.clone(),
                factory: f2,
                ghost_root: Some(root),
                vault_params: Some(vp),
                vaults: vec![],
                tokens: vec![],
                pool: None,
                pool_pending: None,
                lp_tokens: vec![],
                pool_unresolved: None,
            };
            ctx.send("Factory-Init + GHOST-Genesis", &b, Some(&d)).await?;
            if !ctx.dry_run {
                let _ = std::fs::remove_file(&progress_path);
            }
            rate_restart(&RateFile::of(&ctx));
            say!("Fertig. Register {}  Orakel {}  Factory {}  GHOST {}", d.register.cov, d.oracle.cov, d.factory.cov, d.vault_params.as_ref().unwrap().ghost_cov);
        }
        Cmd::OracleFreeze { key } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            if d.oracle.state.frozen {
                say!("Das Orakel ist schon eingefroren.");
            } else {
                let daa = ctx.net.daa().await?.saturating_sub(20);
                let (b, nd) = ops::oracle_freeze(&d, daa, &ctx.funds(&k).await?, &p)?;
                ctx.send("Orakel einfrieren", &b, Some(&nd)).await?;
            }
        }
        Cmd::Signers { cmd } => signers_cmd(&ctx, cmd, &mut extra).await?,
        Cmd::Sync => {
            let mut d = ctx.load_synced().await?;
            let notes = store::discover(&ctx.net, &mut d).await?;
            for n in &notes {
                say!("{n}");
            }
            if !notes.is_empty() && !ctx.dry_run {
                ctx.save(&d)?;
            }
            say!(
                "Abgleich fertig: Orakel {:.6} USD (Update Nr. {}), {} Vault(s), davon {} gesperrt",
                d.oracle.state.kas_usd as f64 / 1e8,
                d.oracle.state.seq,
                d.vaults.len(),
                d.vaults.iter().filter(|v| v.stale).count()
            );
            extra.insert("vaults".into(), d.vaults.len().into());
            extra.insert("found".into(), notes.iter().filter(|n| n.contains("gefunden (")).count().into());
        }
        Cmd::Status { json: true } => {
            if let Err(e) = store::resolve_pending(&ctx.net, &ctx.state_path).await {
                eprintln!("Hinweis: {e}");
            }
            println!("{}", serde_json::to_string_pretty(&status_json(&ctx, &cli.network).await).unwrap());
            return Ok(None);
        }
        Cmd::Keys { .. } | Cmd::Messages { .. } => unreachable!(),
        Cmd::Abo { cmd } => match cmd {
            AboCmd::Run { today } => {
                let reports = abo_run(&ctx, today.as_deref()).await?;
                extra.insert("reports".into(), serde_json::to_value(&reports).unwrap());
            }
            _ => unreachable!(),
        },
        Cmd::Tresor { cmd } => tresor_cmd(&ctx, cmd, &mut extra).await?,
        Cmd::Status { json: false } if cli.json => {
            println!("{}", serde_json::to_string_pretty(&status_json(&ctx, &cli.network).await).unwrap());
            return Ok(None);
        }
        Cmd::Status { json: false } => {
            let d = ctx.load_synced().await?;
            let s = d.oracle.state;
            let daa = ctx.net.daa().await?;
            let age_min = (daa as i64 - s.oracle_daa) as f64 / 10.0 / 60.0;
            say!("Netz {}  |  Orakel: {:.6} USD, Zins-Index {:.9}, seq {}, Alter {:.1} min", d.network, s.kas_usd as f64 / 1e8, s.stable_index as f64 / 1e9, s.seq, age_min);
            match ctx.check_fresh(&d).await {
                Ok(()) => say!("Orakel-UTXO aktuell"),
                Err(e) => say!("WARNUNG: {e}"),
            }
            let vp = d.vault_params.as_ref().ok_or("nicht initialisiert")?;
            for (i, v) in d.vaults.iter().enumerate() {
                let debt = v.vault.state.debt;
                let interest = math::accrued(&v.vault.state, s.stable_index);
                let owed = debt + interest;
                let value = math::value_of(v.vault.value as i64, s.kas_usd);
                let ratio = if owed > 0 { value as f64 / owed as f64 * 100.0 } else { f64::INFINITY };
                let liq_price = if owed > 0 { (owed as f64 * vp.liq_bps as f64 / 1e4) / (v.vault.value as f64 / 1e8) } else { 0.0 };
                say!(
                    "Vault {i}: Besitzer {}…  Sicherheit {:.2} KAS  Schuld {:.8} GHOST  Zins {:.8} USD  Quote {:.1} %  Liquidation unter {:.6} USD",
                    &faster_hex::hex_string(&v.owner)[..12],
                    v.vault.value as f64 / 1e8,
                    debt as f64 / 1e8,
                    interest as f64 / 1e8,
                    ratio,
                    liq_price / 1e8
                );
            }
            for t in &d.tokens {
                say!("GHOST {:.8} → {}…", t.state.amount as f64 / 1e8, &faster_hex::hex_string(&t.state.owner)[..12]);
            }
            if let Some(r) = &d.pool {
                match &d.pool_unresolved {
                    Some(e) => say!("Tauschpool: Reserve unbekannt ({e})"),
                    None => say!(
                        "Tauschpool: {:.4} KAS / {:.8} GHOST, 1 GHOST = {:.4} KAS, {} Anteile",
                        r.kas() as f64 / 1e8,
                        r.ghost() as f64 / 1e8,
                        r.kas() as f64 / r.ghost() as f64,
                        r.shares()
                    ),
                }
            }
        }
        Cmd::OracleUpdate { key, committee, usd, rate } => {
            oracle_update(&ctx, &key, &committee, usd, rate).await?;
        }
        Cmd::OracleFeed { key, committee, interval, min_change, max_age_min } => {
            // Öffentliche Nodes trennen ungenutzte WebSockets nach einigen
            // Minuten (Mainnet-Betrieb 28.09.2026: "WebSocket is not
            // connected"). Deshalb pro Runde eine frische Verbindung.
            let _ = ctx.net.client.disconnect().await;
            let (network, rpc, state_path) = (cli.network.clone(), cli.rpc.clone(), ctx.state_path.clone());
            // aufeinanderfolgende Runden mit großem Sprung in dieselbe Richtung
            let streak = std::sync::Mutex::new(Streak::default());
            loop {
                let now = chrono_now();
                // Zeitlimit je Runde: Mainnet-Betrieb 28.09.2026 zeigte Runden, die
                // hängen blieben (Node-Verbindung oder Kursquelle); die Schleife
                // darf daran nicht stehen bleiben.
                let round = async {
                    match Net::connect(&network, rpc.as_deref()).await {
                        Err(e) => eprintln!("[{now}] Keine Verbindung zum Node: {e}"),
                        Ok(net) => {
                            let round = Ctx { mainnet: network == "mainnet", yes: ctx.yes, dry_run: ctx.dry_run, network: network.clone(), net, state_path: state_path.clone() };
                            // Preis vor der Sperre holen: bis zu 6 Quellen à 8 s (A10-A-10)
                            let market = median_now().await;
                            let _lock = match store::lock(&round.state_path, Duration::from_secs(60)) {
                                Ok(l) => l,
                                Err(e) => {
                                    eprintln!("[{now}] {e}");
                                    let _ = round.net.client.disconnect().await;
                                    return;
                                }
                            };
                            match market {
                                Ok(m) => oracle_round(&round, &key, &committee, min_change, max_age_min, &streak, m, &now).await,
                                Err(e) => {
                                    *streak.lock().unwrap() = Streak::default();
                                    eprintln!("[{now}] kein Marktpreis: {e}");
                                }
                            }
                            let _ = round.net.client.disconnect().await;
                        }
                    }
                };
                if tokio::time::timeout(Duration::from_secs(180), round).await.is_err() {
                    eprintln!("[{now}] Runde nach 180 s abgebrochen (Zeitlimit), nächste Runde folgt");
                }
                tokio::time::sleep(Duration::from_secs(interval)).await;
            }
        }
        Cmd::Agent { key, committee, interval, min_change, max_age_min } => {
            // unbeaufsichtigt: eine Rückfrage über stdin würde unter der Sperre hängen (A10-A-11)
            if ctx.mainnet && !ctx.yes && !ctx.dry_run {
                return Err("agent im Mainnet nur mit --ja (Dauerbetrieb ohne Rückfragen) oder --dry-run".into());
            }
            let k = load_key(&key)?;
            if let Some(c) = &committee {
                load_committee(c)?;
            }
            let takt = Takt::agent(interval);
            say!(
                "GHOST-Agent gestartet: {}Liquidationen und Auflösen mit {} alle {interval} s, danach Daueraufträge und Tresore. Die beiden halten Orakel und Keeper höchstens je {} s Vorbereitung und {} s nach dem Senden auf (zwischen zwei Runden höchstens {} s). Beenden mit Ctrl+C.",
                if committee.is_some() { "Orakel, Zinsregel, " } else { "" },
                faster_hex::hex_string(&xonly(&k))[..12].to_string() + "…",
                takt.prep.as_secs(),
                takt.send.as_secs(),
                takt.max_gap().as_secs()
            );
            let _ = ctx.net.client.disconnect().await;
            let (network, rpc, state_path) = (cli.network.clone(), cli.rpc.clone(), ctx.state_path.clone());
            let streak = std::sync::Mutex::new(Streak::default());
            loop {
                let now = chrono_now();
                let round = async {
                    match Net::connect(&network, rpc.as_deref()).await {
                        Err(e) => eprintln!("[{now}] Keine Verbindung zum Node: {e}"),
                        Ok(net) => {
                            let round = Ctx { mainnet: network == "mainnet", yes: ctx.yes, dry_run: ctx.dry_run, network: network.clone(), net, state_path: state_path.clone() };
                            // Marktpreis einmal je Runde und vor der Sperre (A10-A-10)
                            let market = median_now().await;
                            match store::lock(&round.state_path, Duration::from_secs(60)) {
                                Err(e) => eprintln!("[{now}] {e}"),
                                Ok(_lock) => match market {
                                    Err(e) => {
                                        *streak.lock().unwrap() = Streak::default();
                                        eprintln!("[{now}] kein Marktpreis ({e}) – kein Orakel-Update, ohne Gegenprobe wird nicht liquidiert");
                                    }
                                    Ok(m) => {
                                        if let Some(c) = &committee {
                                            oracle_round(&round, &key, c, min_change, max_age_min, &streak, m, &now).await;
                                        }
                                        keeper_round(&round, &k, m, &now).await;
                                    }
                                },
                            }
                            let _ = round.net.client.disconnect().await;
                        }
                    }
                };
                // Daueraufträge und fällige Tresor-Zahlungen (eigene und importierte)
                // nach Orakel und Keeper, je mit eigenem Fehlerpfad; beim Senden
                // mit kurzem Zeitlimit (Takt, Audit 12 A12-5)
                let abo = abo_agent_step(&network, rpc.as_deref(), &state_path, ctx.yes, ctx.dry_run, &now);
                let tresor = tresor_agent_step(&network, rpc.as_deref(), &state_path, &key, ctx.yes, ctx.dry_run, &now);
                agent_cycle(takt, &now, sent_count, round, abo, tresor).await;
            }
        }
        Cmd::OpenVault { key, kas } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            let (b, nd) = ops::open_vault(&d, &xonly(&k), amount(kas, "--kas")? as u64, &ctx.funds(&k).await?, &p)?;
            ctx.send("Vault eröffnen", &b, Some(&nd)).await?;
            say!("Vault {} eröffnet", nd.vaults.len() - 1);
            extra.insert("vault".into(), (nd.vaults.len() - 1).into());
        }
        Cmd::Mint { key, vault, ghost } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            owned_vault(&d, vault, &k)?;
            let (b, nd) = ops::mint(&d, vault, &k, amount(ghost, "--ghost")?, &xonly(&k), &ctx.funds(&k).await?, &p)?;
            ctx.send("GHOST prägen", &b, Some(&nd)).await?;
        }
        Cmd::Repay { key, vault, ghost } => {
            let k = load_key(&key)?;
            let mut d = ctx.load_synced().await?;
            owned_vault(&d, vault, &k)?;
            let want = match ghost {
                Some(g) => amount(g, "--ghost")?,
                None => i64::MAX,
            };
            let need = want.min(d.vaults[vault].vault.state.debt);
            d = consolidate(&ctx, d, &k, need, &p).await?;
            let mine = own_tokens(&d, &k);
            let (b, nd) = ops::repay(&d, vault, &k, &mine, want, &ctx.funds(&k).await?, &p)?;
            ctx.send("Tilgen", &b, Some(&nd)).await?;
        }
        Cmd::Deposit { key, vault, kas } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            owned_vault(&d, vault, &k)?;
            let (b, nd) = ops::deposit(&d, vault, &k, amount(kas, "--kas")? as u64, &ctx.funds(&k).await?, &p)?;
            ctx.send("Einzahlen", &b, Some(&nd)).await?;
        }
        Cmd::Withdraw { key, vault, keep } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            owned_vault(&d, vault, &k)?;
            let keep_u = amount(keep, "--keep")? as u64;
            let have = d.vaults[vault].vault.value;
            if keep_u >= have {
                return Err(format!("--keep muss kleiner als die aktuelle Sicherheit ({:.8} KAS) sein; zum Nachschießen `deposit` verwenden", have as f64 / 1e8));
            }
            let (b, nd) = ops::withdraw(&d, vault, &k, keep_u, &p2pk_spk(&xonly(&k)), &ctx.funds(&k).await?, &p)?;
            ctx.send("Abheben", &b, Some(&nd)).await?;
        }
        Cmd::Sweep { key, vault } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            usable_vault(&d, vault)?;
            let (b, nd) = ops::sweep(&d, vault, &ctx.funds(&k).await?, &p)?;
            say!("Vault {vault} auflösen: {:.8} KAS an die Zinskasse", b.tx.outputs[1].value as f64 / 1e8);
            ctx.send("Auflösen", &b, Some(&nd)).await?;
        }
        Cmd::Close { key, vault } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            owned_vault(&d, vault, &k)?;
            // erst bauen: an der Rechengrenze des Vertrags meldet ops::close den
            // Grund, statt vorher die (gesättigte) ganze Sicherheit als Zins zu nennen
            let (b, nd) = ops::close(&d, vault, &k, &p2pk_spk(&xonly(&k)), &ctx.funds(&k).await?, &p)?;
            let fee = ops::close_fee(&d, vault);
            if fee > 0 {
                say!("Zins an die Zinskasse: {:.8} KAS", fee as f64 / 1e8);
            }
            extra.insert("interestKas".into(), (fee as f64 / 1e8).into());
            ctx.send("Schließen", &b, Some(&nd)).await?;
        }
        Cmd::Liquidate { key, vault, ghost } => {
            let k = load_key(&key)?;
            let mut d = ctx.load_synced().await?;
            usable_vault(&d, vault)?;
            let debt = d.vaults[vault].vault.state.debt;
            let burn = match ghost {
                Some(g) => amount(g, "--ghost")?.min(debt),
                None => debt,
            };
            d = consolidate(&ctx, d, &k, burn, &p).await?;
            let mine = own_tokens(&d, &k);
            let (b, nd) = ops::liquidate(&d, vault, &k, &mine, burn, &ctx.funds(&k).await?, &p)?;
            ctx.send("Liquidieren", &b, Some(&nd)).await?;
        }
        Cmd::Redeem { key, vault, ghost } => {
            let k = load_key(&key)?;
            let mut d = ctx.load_synced().await?;
            usable_vault(&d, vault)?;
            let a = amount(ghost, "--ghost")?;
            // erst prüfen, ob der Vault die Rücknahme annimmt – consolidate sendet
            // schon (Audit 11 A11-O-7, wie A10-A-9)
            let liq = d.vault_params.as_ref().map(|vp| vp.liq_bps).ok_or("keine Vault-Parameter")?;
            let v = &d.vaults[vault];
            if math::redemption(v.vault.value as i64, &v.vault.state, a, d.oracle.state.kas_usd, d.oracle.state.stable_index, liq).is_none() {
                return Err(format!(
                    "Vault {vault} nimmt diese Rücknahme nicht an: höchstens die Schuld ({:.8} GHOST), mindestens 1 GHOST oder die ganze Schuld, Vault ab {} %, danach ≥ 0,2 KAS im Vault",
                    v.vault.state.debt as f64 / 1e8,
                    liq / 100
                ));
            }
            d = consolidate(&ctx, d, &k, a, &p).await?;
            let mine = own_tokens(&d, &k);
            let paid = math::redeem_paid(a, d.oracle.state.kas_usd);
            say!("Rücknahme: {:.8} GHOST → {:.8} KAS (1 USD je GHOST abzüglich 1 %)", a as f64 / 1e8, paid as f64 / 1e8);
            extra.insert("kas".into(), (paid as f64 / 1e8).into());
            let (b, nd) = ops::redeem(&d, vault, &k, &mine, a, &ctx.funds(&k).await?, &p)?;
            ctx.send("Rücknahme", &b, Some(&nd)).await?;
        }
        Cmd::Send { key, to, kas, message, onchain_message } => {
            let k = load_key(&key)?;
            let spk = kas_target(ctx.net.prefix, &to)?;
            let (msg, payload) = message_args(message.as_deref(), onchain_message, message::recipient_of_spk(&spk).as_ref().map(|x| x.as_slice()))?;
            let units = amount(kas, "--kas")? as u64;
            let b = build_kas(&ctx, &k, spk, units, &payload).await?;
            ctx.send(&format!("{kas} KAS senden"), &b, None).await?;
            note_sent(&ctx, &b, json!({ "action": "send", "amount": abo::fmt_amount(units), "unit": "KAS", "to": to, "message": msg, "onchain": onchain_message, "encrypted": message::is_encrypted(&payload) }));
        }
        Cmd::Transfer { key, to, ghost, message, onchain_message } => {
            let k = load_key(&key)?;
            let g = amount(ghost, "--ghost")?;
            // Empfänger zuerst prüfen: sonst kostet ein Tippfehler die Gebühr des Zusammenführens (A10-A-9)
            let to_x = ghost_target(ctx.net.prefix, &ctx.network, &to)?;
            let (msg, payload) = message_args(message.as_deref(), onchain_message, Some(&to_x))?;
            let (b, nd) = build_ghost(&ctx, &k, &to_x, g, &payload, &p).await?;
            ctx.send("GHOST senden", &b, Some(&nd)).await?;
            note_sent(&ctx, &b, json!({ "action": "transfer", "amount": abo::fmt_amount(g as u64), "unit": "GHOST", "to": to, "message": msg, "onchain": onchain_message, "encrypted": message::is_encrypted(&payload) }));
        }
        Cmd::Receive { key, owner, ghost } => {
            let me = match (&key, &owner) {
                (Some(key), _) => xonly(&load_key(key)?),
                // nie als Dateipfad lesen: nur Adresse oder x-only (öffentlich erreichbar)
                (None, Some(o)) => {
                    let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(&ctx.network)?);
                    kaspa_lending_protocol::wallet_ops::ghost_target(prefix, o)?
                }
                (None, None) => return Err("--key oder --owner angeben".into()),
            };
            let public = owner.is_some();
            // öffentlich: ohne Netzabgleich lesen (A19-4), gespeichert wird unten unter kurzer Sperre
            let mut d = if public { ctx.load()? } else { ctx.load_synced().await? };
            let gcov = d.vault_params.as_ref().ok_or("nicht initialisiert")?.ghost_cov;
            let a = amount(ghost, "--ghost")?;
            let tok = GhostTok::to_pubkey(&me, a);
            let tspk = spk(&tok.artifact());
            let found: Vec<_> = ctx.net.utxos(&ctx.net.address_of_spk(&tspk)?).await?.into_iter().filter(|(_, e)| e.covenant_id == Some(gcov)).collect();
            let _save_lock = if public && !found.is_empty() && !ctx.dry_run {
                let l = store::lock(&ctx.state_path, Duration::from_secs(10))?;
                d = ctx.load()?;
                Some(l)
            } else {
                None
            };
            let (mut new, mut known) = (0usize, 0usize);
            for (op, e) in found {
                if d.tokens.iter().any(|t| t.outpoint == op) {
                    known += 1;
                } else {
                    d.tokens.push(ops::Tracked { outpoint: op, value: e.amount, cov: gcov, state: tok.clone() });
                    new += 1;
                }
            }
            if new > 0 && !ctx.dry_run {
                ctx.save(&d)?;
            }
            say!("{new} neue, {known} bekannte GHOST-UTXO(s) über {:.8} GHOST", a as f64 / 1e8);
            extra.insert("found".into(), new.into());
            extra.insert("known".into(), known.into());
        }
        Cmd::Utxos { address } => {
            let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(&ctx.network)?);
            let mut out = Vec::new();
            for a in &address {
                let addr = kaspa_addresses::Address::try_from(a.as_str()).map_err(|e| format!("--address: {e}"))?;
                if addr.prefix != prefix {
                    return Err(format!("Adresse gehört zu einem anderen Netz ({})", addr.prefix));
                }
                for (op, e) in ctx.net.utxos(&addr).await? {
                    out.push(json!({
                        "address": addr.to_string(),
                        "covenantId": e.covenant_id.map(|c| c.to_string()),
                        "transactionId": op.transaction_id.to_string(),
                        "index": op.index,
                        "daaScore": e.block_daa_score,
                    }));
                }
            }
            extra.insert("utxos".into(), out.into());
        }
        Cmd::PoolOpen { key, kas, ghost } => {
            let k = load_key(&key)?;
            let mut d = ctx.load_synced().await?;
            if let Some(old) = &d.pool {
                if old.params.band.is_some() {
                    return Err("In diesem Netz gibt es schon einen Pool. Einlegen mit pool-add.".into());
                }
                // Pool der Version 1 (ohne Kursband): erst die eigenen Anteile
                // abziehen, dann wird er durch einen Pool mit Kursband ersetzt
                let mine: i64 = d.lp_tokens.iter().filter(|t| t.cov == old.lp_cov && t.state.owner == xonly(&k)).map(|t| t.state.amount).sum();
                if mine > 0 {
                    return Err(format!(
                        "Der bisherige Pool hat noch kein Kursband. Zuerst deine {mine} Anteile abziehen (pool-remove --percent 100), dann neu anlegen."
                    ));
                }
                say!("Der bisherige Pool ohne Kursband wird nicht mehr genutzt ({:.4} KAS / {:.8} GHOST bleiben dort).", old.kas() as f64 / 1e8, old.ghost() as f64 / 1e8);
                let old_lp = old.lp_cov;
                d.lp_tokens.retain(|t| t.cov != old_lp);
                d.pool = None;
            }
            let gcov = d.vault_params.as_ref().ok_or("nicht initialisiert")?.ghost_cov;
            let (kas_u, ghost_u) = (amount(kas, "--kas")?, amount(ghost, "--ghost")?);
            let min = pool::POOL_MIN_KAS as i64;
            if kas_u < min {
                return Err("Der Pool braucht mindestens 1 KAS.".into());
            }
            // Mindestliquidität zum gleichen Kurs: 1 KAS und ghost·1/kas GHOST
            let ghost0 = ((ghost_u as i128 * min as i128) / kas_u as i128).max(1) as i64;
            // Schritt 1: Genesis (entfällt, wenn ein früherer Lauf sie schon angelegt hat)
            if d.pool_pending.is_none() {
                let band = pool::PoolBand::of(&d, pool::POOL_BAND_BPS);
                let t = pool::pool_create(gcov, band, &k, &ctx.funds(&k).await?, &p)?;
                let nd = pool::apply(&d, &t);
                ctx.send("Pool anlegen (1/3)", &t.built, Some(&nd)).await?;
                if ctx.dry_run {
                    say!("Probelauf: Schritt 2 und 3 bauen auf Schritt 1 auf und lassen sich erst danach prüfen.");
                    return Ok(Some(json!({ "ok": true, "dryRun": true, "transactions": std::mem::take(&mut *TXS.lock().unwrap()) })));
                }
                d = nd;
            } else {
                say!("Setze angefangenes Anlegen fort (Genesis besteht schon)");
            }
            // Schritt 2: init – Anteils-Token und gesperrte Mindestliquidität
            let pend = d.pool_pending.clone().ok_or("Pool-Genesis fehlt")?;
            d = consolidate(&ctx, d, &k, ghost0, &p).await?;
            let mine = pool::own(&d.tokens, &xonly(&k), 2);
            let t = pool::pool_init(&pend, Some(&pool::OracleRef::of(&d)), &k, &mine, ghost0, &ctx.funds(&k).await?, &p)?;
            let nd = pool::apply(&d, &t);
            ctx.send("Pool initialisieren (2/3)", &t.built, Some(&nd)).await?;
            d = nd;
            // Schritt 3: den Rest einlegen, dafür gibt es Anteile
            let (rest_kas, rest_ghost) = (kas_u - min, ghost_u - ghost0);
            if rest_kas > 0 && rest_ghost > 0 {
                let rec = d.pool.clone().ok_or("Pool fehlt nach init")?;
                d = consolidate(&ctx, d, &k, rest_ghost, &p).await?;
                let mine = pool::own(&d.tokens, &xonly(&k), 2);
                // Kursband gegen das gewählte Startverhältnis: wer den kleinen Pool
                // nach init verschiebt, bringt Schritt 3 nur zum Abbruch (A10-P-1)
                let (t, m, _) = pool::add(&rec, &k, &mine, rest_kas, rest_ghost, 1, pool::ADD_TOL_BPS, &ctx.funds(&k).await?, &p)
                    .map_err(|e| format!("{e} – Pool ist angelegt; den Rest später mit pool-add einlegen"))?;
                let nd = pool::apply(&d, &t);
                ctx.send("Liquidität einlegen (3/3)", &t.built, Some(&nd)).await?;
                say!("{m} Anteile erhalten");
            }
        }
        Cmd::PoolAdd { key, kas, ghost, min_shares } => {
            let k = load_key(&key)?;
            let mut d = ctx.load_synced().await?;
            if let Some(e) = &d.pool_unresolved {
                return Err(format!("Pool-Stand unbekannt: {e}"));
            }
            let rec = d.pool.clone().ok_or("In diesem Netz gibt es noch keinen Pool.")?;
            let g = amount(ghost, "--ghost")?;
            d = consolidate(&ctx, d, &k, g, &p).await?;
            let mine = pool::own(&d.tokens, &xonly(&k), 2);
            let min_m = min_shares.unwrap_or(1);
            if min_m < 1 {
                return Err("--min-shares muss mindestens 1 sein".into());
            }
            let (t, m, (dx, dy)) = pool::add(&rec, &k, &mine, amount(kas, "--kas")?, g, min_m, pool::ADD_TOL_BPS, &ctx.funds(&k).await?, &p)?;
            say!(
                "Einlage {:.8} KAS und {:.8} GHOST bringt {m} Anteile ({:.4} % des Pools)",
                dx as f64 / 1e8,
                dy as f64 / 1e8,
                m as f64 / (rec.shares() + m) as f64 * 100.0
            );
            extra.insert("shares".into(), m.into());
            extra.insert("kas".into(), (dx as f64 / 1e8).into());
            extra.insert("ghost".into(), (dy as f64 / 1e8).into());
            let nd = pool::apply(&d, &t);
            ctx.send("Liquidität einlegen", &t.built, Some(&nd)).await?;
        }
        Cmd::PoolRemove { key, percent, min_kas, min_ghost } => {
            let k = load_key(&key)?;
            let d = ctx.load_synced().await?;
            if let Some(e) = &d.pool_unresolved {
                return Err(format!("Pool-Stand unbekannt: {e}"));
            }
            let rec = d.pool.clone().ok_or("In diesem Netz gibt es keinen Pool.")?;
            if !(percent > 0.0 && percent <= 100.0) {
                return Err("--percent muss zwischen 0 und 100 liegen".into());
            }
            let mine = pool::own(&d.lp_tokens, &xonly(&k), 2);
            let have: i64 = mine.iter().map(|t| t.state.amount).sum();
            let all: i64 = d.lp_tokens.iter().filter(|t| t.state.owner == xonly(&k)).map(|t| t.state.amount).sum();
            if have == 0 {
                return Err("Dieser Schlüssel hat keine Pool-Anteile.".into());
            }
            if all > have {
                say!("Hinweis: Anteile liegen auf mehr als zwei UTXOs; abgezogen wird aus den zwei größten.");
            }
            let m = ((have as f64 * percent / 100.0).floor() as i64).clamp(1, have);
            let min_x = match min_kas {
                Some(v) if v > 0.0 => amount(v, "--min-kas")?,
                _ => 0,
            };
            let min_y = match min_ghost {
                Some(v) if v > 0.0 => amount(v, "--min-ghost")?,
                _ => 0,
            };
            let (t, (dx, dy)) = pool::remove(&rec, &k, &mine, m, min_x, min_y, &ctx.funds(&k).await?, &p)?;
            say!("{m} Anteile → {:.8} KAS und {:.8} GHOST", dx as f64 / 1e8, dy as f64 / 1e8);
            extra.insert("kas".into(), (dx as f64 / 1e8).into());
            extra.insert("ghost".into(), (dy as f64 / 1e8).into());
            let nd = pool::apply(&d, &t);
            ctx.send("Liquidität abziehen", &t.built, Some(&nd)).await?;
        }
        Cmd::Swap { key, kas, ghost, min_ghost, min_kas } => {
            let k = load_key(&key)?;
            // Bewegt ein Dritter den Pool zwischen Bau und Senden, ist die Tx
            // ungültig (exakte Pool-UTXO). Dann neu abgleichen und erneut bauen,
            // höchstens 3 Versuche und nur, wenn sicher nichts gesendet wurde
            // (Audit 9 P-3). Der Mindestbetrag gilt bei jedem Versuch.
            let mut fixed_min: Option<i64> = None;
            for attempt in 1..=3 {
                let mut d = ctx.load_synced().await?;
                if let Some(e) = &d.pool_unresolved {
                    return Err(format!("Pool-Reserve unbekannt, Tauschen gesperrt: {e}"));
                }
                let rec = d.pool.clone().ok_or("In diesem Netz gibt es noch keinen Pool.")?;
                let (x, y, f) = (rec.pool.value as i64, rec.reserve.state.amount, rec.params.fee_bps);
                let (s, mine) = match (kas, ghost) {
                    (Some(v), None) => {
                        let dx = amount(v, "--kas")?;
                        let min = match (min_ghost, fixed_min) {
                            (Some(m), _) => amount(m, "--min-ghost")?,
                            (None, Some(m)) => m,
                            (None, None) => pool::ghost_out(x, y, dx, f) * 99 / 100,
                        };
                        fixed_min = Some(min);
                        (pool::Swap::Buy { kas: dx, min_ghost: min }, vec![])
                    }
                    (None, Some(v)) => {
                        let dy = amount(v, "--ghost")?;
                        let min = match (min_kas, fixed_min) {
                            (Some(m), _) => amount(m, "--min-kas")?,
                            (None, Some(m)) => m,
                            (None, None) => pool::kas_out(x, y, dy, f) * 99 / 100,
                        };
                        fixed_min = Some(min);
                        d = consolidate(&ctx, d, &k, dy, &p).await?;
                        (pool::Swap::Sell { ghost: dy, min_kas: min }, pool::own(&d.tokens, &xonly(&k), 2))
                    }
                    _ => return Err("Tauschen: entweder --kas oder --ghost angeben".into()),
                };
                let (t, out) = pool::swap(&rec, Some(&pool::OracleRef::of(&d)), &k, &mine, s, &ctx.funds(&k).await?, &p)?;
                match s {
                    pool::Swap::Buy { kas, .. } => say!("Tausch: {:.8} KAS → {:.8} GHOST", kas as f64 / 1e8, out as f64 / 1e8),
                    pool::Swap::Sell { ghost, .. } => say!("Tausch: {:.8} GHOST → {:.8} KAS", ghost as f64 / 1e8, out as f64 / 1e8),
                }
                let nd = pool::apply(&d, &t);
                let sent_before = TXS.lock().unwrap().iter().filter(|r| r["sent"] == true).count();
                match ctx.send("Tauschen", &t.built, Some(&nd)).await {
                    Ok(()) => {
                        extra.insert("out".into(), (out as f64 / 1e8).into());
                        break;
                    }
                    Err(e) => {
                        let sent_now = TXS.lock().unwrap().iter().filter(|r| r["sent"] == true).count();
                        let moved = attempt < 3 && sent_now == sent_before && {
                            let d2 = ctx.load_synced().await?;
                            // Pool oder (Kursband) Orakel inzwischen von anderen bewegt
                            d2.pool.as_ref().map(|r| r.pool.outpoint) != Some(rec.pool.outpoint)
                                || (rec.params.band.is_some() && d2.oracle.outpoint != d.oracle.outpoint)
                        };
                        if !moved {
                            return Err(e);
                        }
                        say!("Der Pool wurde inzwischen bewegt – neuer Versuch ({}/3)", attempt + 1);
                    }
                }
            }
        }
    }
    let txs = std::mem::take(&mut *TXS.lock().unwrap());
    let mut out = serde_json::Map::new();
    out.insert("ok".into(), true.into());
    out.insert("network".into(), network_name.into());
    out.insert("dryRun".into(), ctx.dry_run.into());
    out.insert("transactions".into(), serde_json::Value::Array(txs));
    out.extend(extra);
    Ok(Some(serde_json::Value::Object(out)))
}

/// Schlüsseldateien mit Adresse und Guthaben. Gibt NIE Geheimnisse aus.
/// Ohne `net` (kein Node erreichbar) ist `kas` null.
async fn list_keys(net: Option<&Net>, network: &str, state_path: &Path, dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    use serde_json::json;
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    files.sort();
    let dep = std::fs::read_to_string(state_path).ok().and_then(|t| serde_json::from_str::<Deployment>(&t).ok()).filter(|d| d.network == network);
    let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(network)?);
    let mut out = vec![];
    for f in files {
        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if let Ok(c) = serde_json::from_str::<CommitteeFile>(&text) {
            out.push(json!({ "file": f, "type": "committee", "signers": c.secrets.len() }));
            continue;
        }
        let Ok(kf) = serde_json::from_str::<KeyFile>(&text) else { continue };
        let Ok(k) = key_from_hex(&kf.secret) else { continue };
        let addr = kaspa_addresses::Address::new(prefix, kaspa_addresses::Version::PubKey, &xonly(&k));
        let kas: Option<f64> = match net {
            // ein Abfragefehler kostet nur dieses Guthaben, nicht die ganze Liste (A10-A-13)
            Some(n) => n.utxos(&addr).await.ok().map(|u| u.iter().filter(|(_, e)| e.covenant_id.is_none()).map(|(_, e)| e.amount).sum::<u64>() as f64 / 1e8),
            None => None,
        };
        let x = xonly(&k);
        let ghost: i64 = dep.as_ref().map(|d| d.tokens.iter().filter(|t| t.state.owner == x).map(|t| t.state.amount).sum()).unwrap_or(0);
        let vaults: Vec<usize> = dep.as_ref().map(|d| d.vaults.iter().enumerate().filter(|(_, v)| v.owner == x).map(|(i, _)| i).collect()).unwrap_or_default();
        let lp: i64 = dep.as_ref().map(|d| d.lp_tokens.iter().filter(|t| t.state.owner == x).map(|t| t.state.amount).sum()).unwrap_or(0);
        out.push(json!({
            "file": f,
            "type": "key",
            "xonly": faster_hex::hex_string(&x),
            "address": addr.to_string(),
            "kas": kas,
            "ghost": ghost as f64 / 1e8,
            "lpShares": lp.to_string(),
            "vaults": vaults,
        }));
    }
    Ok(out)
}

/// Status für die Lending-Seite. Enthält keine Geheimnisse.
async fn status_json(ctx: &Ctx, network: &str) -> serde_json::Value {
    use serde_json::json;
    if !ctx.state_path.exists() {
        return json!({ "network": network, "deployed": false });
    }
    let (d, sync_error) = match ctx.load_synced().await {
        Ok(d) => (d, None),
        Err(e) => match ctx.load() {
            Ok(mut d) => {
                // Audit 9 P-4: ohne Abgleich ist auch der Pool-Stand ungeprüft
                if d.pool.is_some() {
                    d.pool_unresolved = Some(format!("Abgleich fehlgeschlagen: {e}"));
                }
                (d, Some(e))
            }
            Err(e2) => return json!({ "network": network, "deployed": false, "error": e2 }),
        },
    };
    let daa = ctx.net.daa().await.unwrap_or(0);
    let s = d.oracle.state;
    let fresh: Result<(), String> = match sync_error {
        Some(e) => Err(e),
        None => Ok(()),
    };
    let vp = d.vault_params.as_ref();
    let (mcr, liq, bonus, max_debt) = vp.map(|p| (p.mcr_bps, p.liq_bps, p.bonus_bps, p.max_debt)).unwrap_or((20_000, 15_000, 1_000, NO_DEBT_LIMIT));
    let vaults: Vec<_> = d
        .vaults
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let debt = v.vault.state.debt;
            let interest = math::accrued(&v.vault.state, s.stable_index);
            let owed = debt + interest;
            let value = math::value_of(v.vault.value as i64, s.kas_usd);
            json!({
                "index": i,
                "owner": faster_hex::hex_string(&v.owner),
                "covenantId": v.vault.cov.to_string(),
                "collateralKas": v.vault.value as f64 / 1e8,
                "debtGhost": debt as f64 / 1e8,
                // Zins in USD (Version 3): wird beim Schließen in KAS an die Zinskasse bezahlt
                "interestUsd": interest as f64 / 1e8,
                "ratioPct": if owed > 0 { Some(value as f64 / owed as f64 * 100.0) } else { None },
                "liquidationPriceUsd": if owed > 0 { Some(owed as f64 * liq as f64 / 1e4 / (v.vault.value as f64 / 1e8) / 1e8) } else { None },
                "maxMintGhost": math::max_mint(v.vault.value as i64, &v.vault.state, s.kas_usd, s.stable_index, mcr).min(math::cap_room(&v.vault.state, max_debt)) as f64 / 1e8,
                "stale": v.stale,
                // Schuld 0 und Zins ≥ Sicherheit: jeder darf ihn zugunsten der Zinskasse auflösen
                // (sweep), sofern der Kassen-Ausgang baubar ist (Audit 12 A12-2)
                "sweepable": math::sweepable(v.vault.value as i64, &v.vault.state, s.kas_usd, s.stable_index),
            })
        })
        .collect();
    let tokens: Vec<_> = d.tokens.iter().map(|t| json!({ "owner": faster_hex::hex_string(&t.state.owner), "amountGhost": t.state.amount as f64 / 1e8 })).collect();
    let pool_json = d.pool.as_ref().map(|r| {
        json!({
            "covenantId": r.pool.cov.to_string(),
            "lpCovenantId": r.lp_cov.to_string(),
            "kasSompi": r.pool.value.to_string(),
            "ghostUnits": r.reserve.state.amount.to_string(),
            "shares": r.shares().to_string(),
            "feeBps": r.params.fee_bps,
            "unresolved": d.pool_unresolved,
            // Kursband (null = Pool der Version 1 ohne Band)
            "bandBps": r.params.band.as_ref().map(|b| b.band_bps),
            "maxBuyKas": r.params.band.as_ref().map(|b| pool::band_max_in(r.kas(), r.ghost(), r.params.fee_bps, s.kas_usd, b.band_bps, true) as f64 / 1e8),
            "maxSellGhost": r.params.band.as_ref().map(|b| pool::band_max_in(r.kas(), r.ghost(), r.params.fee_bps, s.kas_usd, b.band_bps, false) as f64 / 1e8),
        })
    });
    let debt_total: i64 = d.vaults.iter().map(|v| v.vault.state.debt).sum();
    let coll_total: u64 = d.vaults.iter().map(|v| v.vault.value).sum();
    let interest_total: i64 = d.vaults.iter().map(|v| math::accrued(&v.vault.state, s.stable_index)).sum();
    json!({
        "network": network,
        "deployed": true,
        "daa": daa,
        "oracle": {
            "covenantId": d.oracle.cov.to_string(),
            "kasUsd": s.kas_usd as f64 / 1e8,
            "seq": s.seq,
            "ageMinutes": (daa as i64 - s.oracle_daa) as f64 / 600.0,
            "ratePctYear": s.stable_rate as f64 * 315_360_000.0 / 1e18 * 100.0,
            "index": s.stable_index as f64 / 1e9,
            "fresh": fresh.is_ok(),
            "freshError": fresh.err(),
            // Version 4: eingefroren = kein aktueller Preis; Prägen, Einlösen,
            // Liquidieren und Tausch sind gesperrt, bis wieder ein Preis kommt
            "frozen": s.frozen,
            "freezeInMinutes": (s.oracle_daa + d.oracle_params.freeze_after_daa - daa as i64) as f64 / 600.0,
        },
        "signers": signers_json(&d, daa),
        "params": {
            "mcrPct": mcr as f64 / 100.0,
            "liqPct": liq as f64 / 100.0,
            "bonusPct": bonus as f64 / 100.0,
            "maxDebtGhost": if max_debt >= NO_DEBT_LIMIT { None } else { Some(max_debt as f64 / 1e8) },
            "redeemFeePct": math::REDEEM_FEE_BPS as f64 / 100.0,
        },
        "factoryCovenantId": d.factory.cov.to_string(),
        "ghostCovenantId": vp.map(|p| p.ghost_cov.to_string()),
        "totals": { "vaults": d.vaults.len(), "collateralKas": coll_total as f64 / 1e8, "debtGhost": debt_total as f64 / 1e8, "interestUsd": interest_total as f64 / 1e8 },
        "vaults": vaults,
        "tokens": tokens,
        "pool": pool_json,
    })
}

fn chrono_now() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{:02}:{:02}:{:02} UTC", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

/// Ob ein Update fällig ist: Some((Grund, Preis)) oder None.
/// Sprünge über `max_jump` werden NICHT automatisch gesendet (Audit 3 O-6:
/// unter --ja gab es keine Plausibilitätsprüfung gegen den On-chain-Preis).
/// Ergebnis: Some((Grund, Preis, großer_Sprung)). Große Sprünge sendet die
/// Schleife erst, wenn sie über mehrere Runden bestehen (Fix-Review N-6: vorher
/// fror das Orakel bei einem echten Absturz > 20 % dauerhaft ein).
/// Eine Liquidationsrunde des Agenten. Wächter-Regeln:
/// - nur Vaults, die nach dem Orakelpreis UND nach dem aktuellen Marktmedian
///   unter der Liquidationsschwelle liegen (kein Liquidieren mit veraltetem Preis);
/// - höchstens so viele GHOST, dass die Sicherheit Anspruch + Bonus deckt
///   (nie ein Verlustgeschäft), und nicht mehr, als der Schlüssel hat;
/// - höchstens eine Liquidation je Runde (alle Aktionen teilen sich die Orakel-UTXO);
///   scheitert der Bau bei einem Vault, kommt der nächste dran (A10-A-1);
/// - ohne Liquidation höchstens ein auflösbarer Vault je Runde (sweep: Schuld 0,
///   Zins ≥ Sicherheit nach Orakel UND Marktpreis, Audit 11 A11-V-4). Die
///   Netzgebühr (≈ 0,055 KAS) trägt der Vault (SWEEP_FEE 0,1 KAS), den Keeper
///   kostet das nichts – deshalb macht es jeder Agent, nicht nur der Betreiber.
///   Vaults, deren Kassen-Ausgang zu klein wäre, gelten nicht als auflösbar;
///   scheitert der Bau bei einem, kommt der nächste dran (Audit 12 A12-2).
///
/// Nach dem Abgleich plant `keeper_plan` ohne Netz, `keeper_act` liquidiert
/// bzw. löst auf; beide laufen in den Tests ohne Netz (Audit 12, Restpunkt B-P3).
async fn keeper_round(round: &Ctx, k: &Keypair, market: i64, now: &str) {
    let mut d = match round.load_synced().await {
        Ok(d) => d,
        Err(e) => return eprintln!("[{now}] Keeper: Abgleich fehlgeschlagen: {e}"),
    };
    // Vaults anderer Besitzer, die diese Zustandsdatei noch nicht kennt (A10-A-3)
    match store::discover(&round.net, &mut d).await {
        Ok(notes) if !notes.is_empty() => {
            for n in &notes {
                say!("[{now}] {n}");
            }
            if !round.dry_run {
                if let Err(e) = round.save(&d) {
                    return eprintln!("[{now}] Keeper: Speichern fehlgeschlagen: {e}");
                }
            }
        }
        Ok(_) => {}
        Err(e) => eprintln!("[{now}] Keeper: Suche nach neuen Vaults fehlgeschlagen ({e}) – weiter mit den bekannten"),
    }
    // veraltetes Orakel einfrieren (jeder darf): kein Prägen, Einlösen,
    // Liquidieren oder Tauschen gegen einen alten Preis (Version 4)
    if let Some(nd) = freeze_if_stale(round, &d, k, now).await {
        d = nd;
    }
    let Some(plan) = keeper_plan(&d, &xonly(k), market, now) else { return };
    keeper_act(&mut KeeperNet { round, d: &d, k, now }, &plan, now).await;
}

/// Friert das Orakel ein, wenn seine Frist ohne Preis abgelaufen ist.
/// Some(neuer Zustand) nach einem gesendeten Einfrieren.
async fn freeze_if_stale(round: &Ctx, d: &Deployment, k: &Keypair, now: &str) -> Option<Deployment> {
    if d.oracle.state.frozen {
        return None;
    }
    let daa = round.net.daa().await.ok()?.saturating_sub(20);
    if (daa as i64) < d.oracle.state.oracle_daa + d.oracle_params.freeze_after_daa {
        return None;
    }
    say!("[{now}] Orakel-Preis ist älter als die Frist – friere ein");
    let r = async {
        let (b, nd) = ops::oracle_freeze(d, daa, &round.funds(k).await?, &round.net.params)?;
        round.send("Orakel einfrieren", &b, Some(&nd)).await?;
        Ok::<_, String>(nd)
    }
    .await;
    match r {
        Ok(nd) => Some(nd),
        Err(e) => {
            eprintln!("[{now}] Einfrieren fehlgeschlagen: {e}");
            None
        }
    }
}

/// Was eine Keeper-Runde vorhat, ohne Netz aus der abgeglichenen Zustandsdatei
#[derive(Debug, Default)]
struct KeeperPlan {
    /// liquidierbar nach Orakel- und Marktpreis (vor der Prüfung mit den eigenen GHOST)
    found: usize,
    /// eigene GHOST des Schlüssels
    mine: i64,
    /// (Vault, zu verbrennende GHOST): mit den eigenen GHOST möglich und mit Gewinn
    liquidate: Vec<(usize, i64)>,
    /// auflösbare Vaults in dieser Reihenfolge (ops::sweep_candidates)
    sweep: Vec<usize>,
}

/// Kandidaten der Runde. None: ohne Vault-Parameter (nicht initialisiert).
fn keeper_plan(d: &Deployment, me: &[u8], market: i64, now: &str) -> Option<KeeperPlan> {
    let vp = d.vault_params.as_ref()?;
    let (price, index) = (d.oracle.state.kas_usd, d.oracle.state.stable_index);
    let mine: i64 = d.tokens.iter().filter(|t| t.state.owner == me).map(|t| t.state.amount).sum();
    let mut plan = KeeperPlan { mine, ..Default::default() };
    for (i, v) in d.vaults.iter().enumerate() {
        if v.stale {
            continue;
        }
        let coll = v.vault.value as i64;
        match math::keeper_burn(coll, &v.vault.state, price, market, index, vp.liq_bps, vp.bonus_bps) {
            Some(want) => {
                plan.found += 1;
                if mine <= 0 {
                    continue;
                }
                let burn = want.min(mine);
                // weniger als gewollt (zu wenig GHOST): Annahme und Gewinn neu prüfen
                if !math::keeper_profitable(coll, &v.vault.state, burn, price, market, index, vp.liq_bps, vp.bonus_bps) {
                    eprintln!("[{now}] Keeper: Vault {i}: mit {:.8} GHOST nicht möglich oder ohne Gewinn – nächster Kandidat", burn as f64 / 1e8);
                    continue;
                }
                plan.liquidate.push((i, burn));
            }
            None if v.vault.state.debt > 0 && !math::healthy(coll, math::owed(&v.vault.state, index), price, vp.liq_bps) => {
                say!(
                    "[{now}] Vault {i} ist nach dem Orakel liquidierbar, aber nach dem Marktpreis ({:.6} USD) gesund oder ohne Gewinn – übersprungen",
                    market as f64 / 1e8
                );
            }
            None => {}
        }
    }
    // Vaults mit Schuld 0, deren Zins die Sicherheit aufzehrt: jeder darf sie
    // zugunsten der Zinskasse auflösen (Audit 11 A11-V-4). Geprüft zu Orakel-
    // UND Marktpreis, damit ein nachlaufendes Orakel keinen Vault auflöst, den
    // sein Besitzer noch mit Gewinn schließen könnte.
    plan.sweep = ops::sweep_candidates(d, market);
    Some(plan)
}

/// Senden der Keeper-Runde: im Agenten `KeeperNet`, in den Tests eine Attrappe
trait KeeperIo {
    /// Vault `i` mit `burn` GHOST liquidieren (samt Zusammenführen der GHOST)
    async fn liquidate(&mut self, i: usize, burn: i64) -> Result<(), String>;
    /// Vault `i` auflösen; `rest` = übrige Kandidaten (nur für die Meldung)
    async fn sweep(&mut self, i: usize, rest: usize) -> Result<(), String>;
    /// Anzahl gesendeter Tx dieses Prozesses
    fn sent(&self) -> usize;
    /// Journal der Zustandsdatei offen: eine gebaute Tx ist vielleicht
    /// unterwegs (Senden mehrdeutig gescheitert), der nächste Abgleich klärt sie
    fn journal_open(&self) -> bool;
}

struct KeeperNet<'a> {
    round: &'a Ctx,
    d: &'a Deployment,
    k: &'a Keypair,
    now: &'a str,
}

impl KeeperIo for KeeperNet<'_> {
    async fn liquidate(&mut self, i: usize, burn: i64) -> Result<(), String> {
        let (round, k) = (self.round, self.k);
        let p = round.net.params.clone();
        let d = consolidate(round, self.d.clone(), k, burn, &p).await?;
        let toks = own_tokens(&d, k);
        let (b, nd) = ops::liquidate(&d, i, k, &toks, burn, &round.funds(k).await?, &p)?;
        round.send("Liquidieren (Agent)", &b, Some(&nd)).await
    }

    async fn sweep(&mut self, i: usize, rest: usize) -> Result<(), String> {
        let (b, nd) = ops::sweep(self.d, i, &self.round.funds(self.k).await?, &self.round.net.params)?;
        say!(
            "[{}] Keeper: Vault {i} hat keine Schuld, aber der Zins zehrt die Sicherheit auf – löse ihn auf: {:.8} KAS an die Zinskasse ({rest} weitere Kandidaten)",
            self.now,
            b.tx.outputs[1].value as f64 / 1e8
        );
        self.round.send("Auflösen (Agent)", &b, Some(&nd)).await
    }

    fn sent(&self) -> usize {
        sent_count()
    }

    fn journal_open(&self) -> bool {
        store::pending_path(&self.round.state_path).exists()
    }
}

/// Ablauf der Keeper-Runde nach `plan`: höchstens eine Liquidation; ohne
/// gesendete Liquidation die auflösbaren Vaults der Reihe nach (die ganze
/// Liste). Wurde gesendet oder ist das Journal offen, endet die Runde.
async fn keeper_act(io: &mut impl KeeperIo, plan: &KeeperPlan, now: &str) {
    if plan.found == 0 && plan.sweep.is_empty() {
        return say!("[{now}] Keeper: kein Vault liquidierbar");
    }
    if plan.found > 0 && plan.mine <= 0 {
        eprintln!("[{now}] Keeper: {} Vault(s) liquidierbar, aber der Schlüssel hat keine GHOST", plan.found);
    }
    let n = plan.liquidate.len();
    for (pos, &(i, burn)) in plan.liquidate.iter().enumerate() {
        say!("[{now}] Keeper: liquidiere Vault {i} mit {:.8} GHOST ({} weitere Kandidaten)", burn as f64 / 1e8, n - pos - 1);
        let before = io.sent();
        match io.liquidate(i, burn).await {
            Ok(()) => return,
            Err(e) => {
                eprintln!("[{now}] Keeper: Liquidation von Vault {i} fehlgeschlagen: {e}");
                // wurde schon etwas gesendet (Zusammenführen) oder ist der Ausgang
                // unklar (Journal offen), ist der Stand neu – nächste Runde
                if io.sent() != before || io.journal_open() {
                    return;
                }
            }
        }
    }
    // keine Liquidation gesendet: dann darf die Runde noch auflösen
    sweep_in_turn(io, &plan.sweep, now).await;
}

/// Löst einen der Vaults `list` zugunsten der Zinskasse auf (höchstens einen je
/// Runde, alle teilen sich die Orakel-UTXO). Die Netzgebühr trägt der Vault
/// (bis SWEEP_FEE, 0,1 KAS; gemessen ≈ 0,055 KAS). Scheitert der Bau, kommt der
/// nächste Kandidat dran (Audit 12 A12-2: vorher blockierte ein nicht baubarer
/// Vault alle). Wurde gesendet, ist die Orakel-UTXO verbraucht und die Runde
/// endet. Ebenso, wenn das Senden mehrdeutig scheiterte und das Journal stehen
/// blieb: der nächste Versuch schriebe sonst ein neues Journal darüber
/// (Restpunkt B-P2). Rückgabe: der Vault, bei dem die Runde endete (None:
/// keiner gesendet).
async fn sweep_in_turn(io: &mut impl KeeperIo, list: &[usize], now: &str) -> Option<usize> {
    for (pos, &i) in list.iter().enumerate() {
        if io.journal_open() {
            eprintln!("[{now}] Keeper: eine Transaktion ist noch offen (Journal) – Auflösen erst in der nächsten Runde");
            return None;
        }
        let before = io.sent();
        match io.sweep(i, list.len() - pos - 1).await {
            Ok(()) => return Some(i),
            Err(e) => {
                eprintln!("[{now}] Keeper: Auflösen von Vault {i} fehlgeschlagen: {e}");
                if io.sent() != before || io.journal_open() {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Taktung der Agentenschleife (Audit 12 A12-5). Jeder Durchlauf beginnt mit
/// Orakel und Keeper (Zeitlimit `round`). Danach kommen Daueraufträge und
/// Tresore. Jeder dieser Schritte hat bis zu seiner ersten Sendung `prep`
/// (Verbindung, Sperre, Journal-Klärung, Abgleich), ab der ersten Sendung noch
/// `send`, dann geht es weiter; ein unterbrochener Schritt ist dank Journal
/// harmlos, der nächste Durchlauf klärt offene Zahlungen und zahlt die übrigen.
/// Die Schritte nutzen die Wartezeit: die Pause `interval` verkürzt sich um
/// ihre Dauer.
///
/// Vorher lagen zwischen zwei Orakel-/Keeper-Runden bis 240 + 700 + 700 + 120
/// = 1 760 s, vor allem wegen des Wartens auf Bestätigungen (bis 600 s je
/// Zahlung). Runde 2 begrenzte das Warten nach der ersten Sendung auf 90 s,
/// ließ der Vorbereitung aber 700 s je Schritt (Obergrenze 1 640 s). Die
/// Vorbereitung hat jetzt dasselbe Limit wie die Runde, die ebenfalls
/// verbindet, sperrt und abgleicht (Restpunkt B-P4). Siehe `usual_gap` und
/// `max_gap`.
#[derive(Clone, Copy, Debug)]
struct Takt {
    round: Duration,
    send: Duration,
    prep: Duration,
    interval: Duration,
}

impl Takt {
    fn agent(interval_secs: u64) -> Self {
        Takt { round: Duration::from_secs(240), send: Duration::from_secs(90), prep: Duration::from_secs(240), interval: Duration::from_secs(interval_secs) }
    }

    /// Abstand zwischen dem Beginn zweier Orakel-/Keeper-Runden, wenn beide
    /// Schritte senden und auf Bestätigungen warten; dazu kommt die Zeit für
    /// Verbindung und Abgleich (üblich: Sekunden). 420 s beim Standard
    /// (--interval 120), 540 s beim Startskript (--interval 300). Nur für
    /// Tests und Doku; das Programm nennt `max_gap`.
    #[cfg_attr(not(test), allow(dead_code))]
    fn usual_gap(&self) -> Duration {
        self.round + (self.send * 2).max(self.interval)
    }

    /// Obergrenze in jedem Fall: jeder Schritt braucht bis kurz vor `prep`
    /// für die Vorbereitung und wartet danach `send` auf Bestätigungen.
    /// 900 s bei `--interval` bis 660 s (vorher 1 640 s).
    fn max_gap(&self) -> Duration {
        self.round + ((self.prep + self.send) * 2).max(self.interval)
    }
}

/// Wie ein Nebenschritt (Daueraufträge, Tresore) endete
#[derive(Clone, Copy, Debug, PartialEq)]
enum StepEnd {
    Done,
    /// `send` nach der ersten Sendung unterbrochen
    SendLimit,
    /// `prep` erreicht, ohne gesendet zu haben
    PrepLimit,
}

/// Führt einen Nebenschritt nach `Takt` aus: bis zur ersten Sendung höchstens
/// `prep`, danach höchstens `send` (`sent` zählt die gesendeten Tx dieses
/// Prozesses, sekündlich nachgesehen). Eine Sperre, auf die store::lock
/// blockierend wartet (bis 60 s), kann keines der Limits unterbrechen; sie
/// verlängert den Schritt entsprechend.
async fn side_step(t: &Takt, sent: &impl Fn() -> usize, f: impl Future<Output = ()>) -> StepEnd {
    let before = sent();
    let prep = tokio::time::sleep(t.prep);
    let send = tokio::time::sleep(t.prep);
    let mut look = tokio::time::interval(Duration::from_secs(1));
    look.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut sending = false;
    tokio::pin!(f, prep, send);
    loop {
        tokio::select! {
            biased;
            () = &mut f => return StepEnd::Done,
            () = &mut prep, if !sending => {
                // gesendet, aber noch nicht nachgesehen: ab jetzt das Sendelimit
                if sent() == before {
                    return StepEnd::PrepLimit;
                }
                sending = true;
                send.as_mut().reset(tokio::time::Instant::now() + t.send);
            }
            () = &mut send, if sending => return StepEnd::SendLimit,
            _ = look.tick(), if !sending => {
                if sent() != before {
                    sending = true;
                    send.as_mut().reset(tokio::time::Instant::now() + t.send);
                }
            }
        }
    }
}

/// Ein Durchlauf des Agenten nach `Takt`: Orakel/Keeper, dann Daueraufträge und
/// Tresore, dann die restliche Pause. Rückgabe: ob die Runde im Zeitlimit
/// fertig wurde, und wie die beiden Nebenschritte endeten.
async fn agent_cycle(
    t: Takt,
    now: &str,
    sent: impl Fn() -> usize,
    round: impl Future<Output = ()>,
    abo: impl Future<Output = ()>,
    tresor: impl Future<Output = ()>,
) -> (bool, [StepEnd; 2]) {
    let r = tokio::time::timeout(t.round, round).await.is_ok();
    if !r {
        eprintln!("[{now}] Runde nach {} s abgebrochen (Zeitlimit), nächste Runde folgt", t.round.as_secs());
    }
    let side = tokio::time::Instant::now();
    let a = side_step(&t, &sent, abo).await;
    step_note(&t, now, "Daueraufträge", a);
    let b = side_step(&t, &sent, tresor).await;
    step_note(&t, now, "Tresore", b);
    tokio::time::sleep(t.interval.saturating_sub(side.elapsed())).await;
    (r, [a, b])
}

fn step_note(t: &Takt, now: &str, what: &str, end: StepEnd) {
    match end {
        StepEnd::Done => {}
        StepEnd::SendLimit => eprintln!(
            "[{now}] {what}: {} s nach der ersten Sendung unterbrochen, Orakel und Keeper gehen vor; der nächste Durchlauf klärt offene Zahlungen und zahlt die übrigen",
            t.send.as_secs()
        ),
        StepEnd::PrepLimit => eprintln!(
            "[{now}] {what}: Vorbereitung (Verbindung, Abgleich) nach {} s abgebrochen, Orakel und Keeper gehen vor; der nächste Durchlauf versucht es erneut. Kommt das in jedem Durchlauf vor, ist der Node zu langsam.",
            t.prep.as_secs()
        ),
    }
}

/// Sprungschutz des Orakel-Betriebs: Runden mit großem Sprung in dieselbe
/// Richtung, der erste gemessene Preis der Folge und der Median der Folge.
#[derive(Default)]
struct Streak {
    rounds: u32,
    dir: i64,
    prices: Vec<i64>,
}

/// Zinsregel im Probelauf: Messungen und Vormerkungen nur im Speicher, die
/// Datei deployments/<netz>-zins.json bleibt unberührt
static DRY_RATE_LOG: std::sync::Mutex<Option<rate::RateLog>> = std::sync::Mutex::new(None);

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn apr_of(rate: i64) -> f64 {
    rate as f64 * 315_360_000.0 / 1e18 * 100.0
}

/// Datei der Zinsregel zu einer Zustandsdatei (deployments/<netz>-zins.json)
struct RateFile<'a> {
    state_path: &'a Path,
    network: &'a str,
    dry_run: bool,
}

impl<'a> RateFile<'a> {
    fn of(ctx: &'a Ctx) -> Self {
        RateFile { state_path: &ctx.state_path, network: &ctx.network, dry_run: ctx.dry_run }
    }

    /// unter ihrer Sperre lesen, ändern, schreiben (Audit 11 A11-O-9: der Takt
    /// überlebt Neustarts und gilt für alle Prozesse); im Probelauf nur im Speicher
    fn update<R>(&self, f: impl FnOnce(&mut rate::RateLog) -> R) -> Result<R, String> {
        let path = rate::path_for(self.state_path);
        if self.dry_run {
            let mut g = DRY_RATE_LOG.lock().unwrap();
            if g.is_none() {
                *g = Some(rate::load(&path, self.network)?);
            }
            return Ok(f(g.as_mut().unwrap()));
        }
        rate::update(&path, self.network, f)
    }
}

/// deployments/<netz>-zins.json unter ihrer Sperre lesen, ändern, schreiben
fn rate_log<R>(round: &Ctx, f: impl FnOnce(&mut rate::RateLog) -> R) -> Result<R, String> {
    RateFile::of(round).update(f)
}

/// Zins von Hand (`oracle-update --rate`): Der Takt der Zinsregel beginnt
/// jetzt, vorgemerkt vor dem Senden und zurückgenommen, wenn sicher nichts
/// gesendet wurde (Audit 12 A12-17 c, Restpunkt C-O1: vorher durfte der Agent
/// den Zins gleich danach wieder ändern). Lässt sich die Datei nicht
/// schreiben, wird trotzdem gesendet, mit Hinweis.
async fn rate_by_hand(file: &RateFile<'_>, rate: Option<f64>, sent: impl Fn() -> usize, send: impl Future<Output = Result<(), String>>) -> Result<(), String> {
    if rate.is_none() {
        return send.await;
    }
    let t = unix_now();
    let mark = match file.update(|l| l.set_by_hand(t)) {
        Ok(prev) => Some(prev),
        Err(e) => {
            eprintln!("Hinweis: Takt der Zinsregel nicht gesetzt ({e}) – der Agent darf den Zins danach sofort wieder ändern");
            None
        }
    };
    let before = sent();
    let r = send.await;
    if let (Err(_), Some(prev)) = (&r, mark) {
        if sent() == before {
            let _ = file.update(|l| l.release(t, prev));
        }
    }
    r
}

/// Neues Deployment: Die Zinsregel beginnt neu, Messungen eines früheren Pools
/// gelten nicht (die Zinsdatei heißt nach dem Umzug gleich), der Takt beginnt
/// mit dem Startzins (Restpunkt C-O1)
fn rate_restart(file: &RateFile<'_>) {
    let t = unix_now();
    if let Err(e) = file.update(|l| l.restart(t)) {
        eprintln!("Hinweis: Zinsregel nicht zurückgesetzt ({e})");
    }
}

/// Zinsregel (Version 3): eine Messung des GHOST-Kurses je Runde aus dem frisch
/// abgeglichenen Pool `d` (nicht aus der Datei der Vorrunde, A11-O-8), Median
/// der Messungen der letzten Stunde, Änderung nur mit ≥ 6 Messungen über
/// ≥ 45 min (A12-12), ab Mindestliquidität und höchstens einmal je Stunde
/// (A11-O-1, A11-O-9).
/// Some((neuer Zins % p. a., Begründung)), wenn eine Änderung fällig ist.
fn rate_plan(round: &Ctx, d: &Deployment, market: i64, now: &str) -> Option<(f64, String)> {
    // Grundzins zuerst: liegt der Zins darunter, in Vertragsschritten anheben
    // (höchstens einmal je Stunde, unabhängig von Kurs und Pool-Liquidität)
    let cur = apr_of(d.oracle.state.stable_rate);
    if let Some(next) = math::rate_floor_step(cur) {
        return match rate_log(round, |l| l.wait_secs(unix_now())) {
            Ok(0) => Some((next, format!("Grundzins {:.1} % p. a.: Zins {cur:.2} % → {next:.2} % p. a.", math::RATE_MIN_PCT))),
            Ok(s) => {
                say!("[{now}] Zinsregel: Zins {cur:.2} % liegt unter dem Grundzins {:.1} % – nächster Schritt in {} min", math::RATE_MIN_PCT, s.div_ceil(60));
                None
            }
            Err(e) => {
                eprintln!("[{now}] Zinsregel: {e} – keine Zinsänderung");
                None
            }
        };
    }
    let pool = d.pool.as_ref().map(|p| (p.kas(), p.ghost()));
    let g = match rate::measure(pool, d.pool_unresolved.as_deref(), market) {
        Ok(g) => g,
        Err(why) => {
            say!("[{now}] Zinsregel: {why} – keine Messung, keine Zinsänderung");
            return None;
        }
    };
    let t = unix_now();
    let cur = apr_of(d.oracle.state.stable_rate);
    let dec = match rate_log(round, |l| {
        l.record(t, g);
        l.decide(t, cur)
    }) {
        Ok(dec) => dec,
        Err(e) => {
            eprintln!("[{now}] Zinsregel: {e} – keine Zinsänderung");
            return None;
        }
    };
    match dec {
        rate::Decision::Wait { secs_left } => {
            say!("[{now}] Zinsregel: GHOST {g:.4} USD gemessen, letzte Zinsänderung vor weniger als einer Stunde – nächste frühestens in {} min", secs_left.div_ceil(60));
            None
        }
        rate::Decision::TooFew { have } => {
            say!("[{now}] Zinsregel: GHOST {g:.4} USD gemessen, erst {have} von {} Messungen der letzten Stunde – noch keine Zinsänderung", rate::MIN_SAMPLES);
            None
        }
        rate::Decision::TooShort { have, span_secs } => {
            say!(
                "[{now}] Zinsregel: GHOST {g:.4} USD gemessen, {have} Messungen erst über {} min verteilt (nötig {} min) – noch keine Zinsänderung",
                span_secs / 60,
                rate::MIN_SPAN_SECS / 60
            );
            None
        }
        rate::Decision::Keep { median, have } => {
            say!("[{now}] Zinsregel: GHOST-Median {median:.4} USD aus {have} Messungen – Zins bleibt {cur:.2} % p. a.");
            None
        }
        rate::Decision::Change { median, have, next } => {
            Some((next, format!("GHOST-Median {median:.4} USD aus {have} Messungen der letzten Stunde → Zins {cur:.2} % → {next:.2} % p. a.")))
        }
    }
}

/// Zinsänderung vor dem Senden vormerken. None = ein anderer Prozess hat in
/// der letzten Stunde schon geändert (dann ohne Zinsänderung weiter).
fn rate_reserve(round: &Ctx, now: &str) -> Option<(u64, Option<u64>)> {
    let t = unix_now();
    match rate_log(round, |l| l.reserve(t)) {
        Ok(Some(prev)) => Some((t, prev)),
        Ok(None) => {
            say!("[{now}] Zinsregel: in der letzten Stunde schon geändert – diesmal ohne Zinsänderung");
            None
        }
        Err(e) => {
            eprintln!("[{now}] Zinsregel: {e} – keine Zinsänderung");
            None
        }
    }
}

/// Anzahl tatsächlich gesendeter Transaktionen dieses Prozesses
fn sent_count() -> usize {
    TXS.lock().unwrap().iter().filter(|r| r["sent"] == true).count()
}

/// Vormerkung zurücknehmen, wenn sicher nichts gesendet wurde
fn rate_release(round: &Ctx, reserved: (u64, Option<u64>), sent_before: usize) {
    if sent_count() == sent_before {
        let _ = rate_log(round, |l| l.release(reserved.0, reserved.1));
    }
}

/// Eine Runde des Orakel-Dauerbetriebs (oracle-feed und agent): Update, wenn
/// der Preis sich um `min_change` bewegt hat oder das Update zu alt ist. Große
/// Sprünge (> 20 %) erst nach 3 bestätigenden Runden und höchstens ×2/÷2.
/// Die Zinsregel (rate_plan) fährt beim Update mit, sonst mit eigenem Update.
async fn oracle_round(round: &Ctx, key: &Path, committee: &Path, min_change: f64, max_age_min: f64, streak: &std::sync::Mutex<Streak>, market: i64, now: &str) {
    // einmal frisch abgleichen: Orakel, Vaults und Pool (die Zinsregel misst daran)
    let d = match round.load_synced().await {
        Ok(d) => d,
        Err(e) => {
            // Fehlschlag unterbricht die Bestätigungsfolge (A10-A-7)
            *streak.lock().unwrap() = Streak::default();
            return eprintln!("[{now}] Prüfung fehlgeschlagen: {e}");
        }
    };
    let mut rate = rate_plan(round, &d, market, now);
    let old = d.oracle.state.kas_usd;
    match feed_due(round, &d, min_change, max_age_min, 0.20, market).await {
        Ok(Some((reason, mut price, big))) => {
            let mut send = true;
            if big {
                // Bestätigung: 3 Runden in dieselbe Richtung, alle innerhalb von 5 %
                // um den ersten Preis; gesendet wird der Median der drei (A10-A-7)
                let mut s = streak.lock().unwrap();
                let dir = if price > old { 1 } else { -1 };
                let close = s.prices.first().is_none_or(|&f| ((price - f) as f64 / f as f64).abs() <= 0.05);
                if s.dir == dir && close {
                    s.rounds += 1;
                    s.prices.push(price);
                } else {
                    *s = Streak { rounds: 1, dir, prices: vec![price] };
                }
                if s.rounds < 3 {
                    eprintln!("[{now}] {reason} – warte auf Bestätigung ({}/3 Runden)", s.rounds);
                    send = false;
                } else {
                    let mut ps = s.prices.clone();
                    ps.sort_unstable();
                    price = ps[ps.len() / 2];
                }
            } else {
                *streak.lock().unwrap() = Streak::default();
            }
            if send {
                say!("[{now}] Update fällig: {reason}");
                let reserved = if rate.is_some() { rate_reserve(round, now) } else { None };
                if reserved.is_none() {
                    rate = None;
                }
                if let Some((_, why)) = &rate {
                    say!("[{now}] Zinsregel: {why}");
                }
                // höchstens ×2/÷2 je Update (Vertragsgrenze)
                let step = price.clamp((old + 1) / 2, old * 2).clamp(1_000, 90_000_000_000);
                let before = sent_count();
                let r = oracle_update_with(round, key, committee, step, rate.as_ref().map(|r| r.0), true).await;
                *streak.lock().unwrap() = Streak::default();
                if let Err(e) = r {
                    eprintln!("[{now}] Orakel-Update fehlgeschlagen: {e}");
                    if let Some(res) = reserved {
                        rate_release(round, res, before);
                    }
                }
            }
        }
        Ok(None) => {
            *streak.lock().unwrap() = Streak::default();
            match rate {
                // Preis unverändert, aber der Zins soll dem GHOST-Kurs folgen
                // Pause des Vertrags (DAA) noch nicht um: keine Tx nur für den Preis (Audit 15 G-5)
                Some(_) if round.net.daa().await.is_ok_and(|daa| (daa.saturating_sub(20) as i64) < d.oracle.state.last_rate_daa + d.oracle_params.rate_gap_daa) => {
                    say!("[{now}] Zinsregel: Zinsänderung erst nach der Pause des Vertrags – diesmal keine");
                }
                Some((pct, why)) => {
                    if let Some(res) = rate_reserve(round, now) {
                        say!("[{now}] Zinsregel: {why}");
                        let step = market.clamp((old + 1) / 2, old * 2).clamp(1_000, 90_000_000_000);
                        let before = sent_count();
                        if let Err(e) = oracle_update_with(round, key, committee, step, Some(pct), true).await {
                            eprintln!("[{now}] Zins-Update fehlgeschlagen: {e}");
                            rate_release(round, res, before);
                        }
                    }
                }
                None => say!("[{now}] kein Update nötig"),
            }
        }
        Err(e) => {
            // Fehlschlag unterbricht die Bestätigungsfolge (A10-A-7)
            *streak.lock().unwrap() = Streak::default();
            eprintln!("[{now}] Prüfung fehlgeschlagen: {e}");
        }
    }
}

/// Höchstalter des Preises für den Orakel-Betrieb: --max-age-min, aber
/// höchstens die halbe Einfrier-Frist (Version 4, Audit 14 H2/H-1: mit 6 h
/// Standard fror der eigene Keeper das Orakel nach 2 h ein)
fn heartbeat_min(freeze_after_daa: i64, max_age_min: f64) -> f64 {
    max_age_min.min(freeze_after_daa as f64 / 600.0 / 2.0)
}

/// Zinssatz der Zinsregel so, wie der Vertrag ihn annimmt: höchstens
/// rateStep je Änderung, höchstens eine Änderung je rateGapDaa (gezählt in
/// DAA wie im Vertrag). Liegt der Zins neben dem 0,5-Raster, geht die Regel in
/// erlaubten Schritten dorthin (Audit 14 H3/M-1); ist die Pause noch nicht um,
/// bleibt der Zins und nur der Preis geht hinaus (Audit 14 N-2/N1).
fn fit_rate(p: &OracleParams, cur: &OracleState, want: i64, daa: i64) -> (i64, Option<String>) {
    if want == cur.stable_rate {
        return (want, None);
    }
    if daa < cur.last_rate_daa + p.rate_gap_daa {
        return (cur.stable_rate, Some("Zinsänderung erst nach der Pause des Vertrags – diesmal nur der Preis".into()));
    }
    let r = want.clamp(cur.stable_rate - p.rate_step, cur.stable_rate + p.rate_step).clamp(0, p.max_rate);
    (r, (r != want).then(|| format!("Zinsschritt auf {:.2} % p. a. begrenzt (Vertrag)", apr_of(r))))
}

async fn feed_due(ctx: &Ctx, d: &Deployment, min_change: f64, max_age_min: f64, max_jump: f64, p: i64) -> Result<Option<(String, i64, bool)>, String> {
    let old = d.oracle.state.kas_usd as f64;
    let change = (p as f64 - old) / old;
    if change.abs() > max_jump {
        return Ok(Some((format!("großer Preissprung {:+.1} % ({:.6} → {:.6} USD)", change * 100.0, old / 1e8, p as f64 / 1e8), p, true)));
    }
    let age_min = (ctx.net.daa().await? as i64 - d.oracle.state.oracle_daa) as f64 / 600.0;
    Ok(feed_reason(d, age_min, min_change, max_age_min, p).map(|r| (r, p, false)))
}

/// Grund für ein Update ohne großen Sprung, ohne Netz (Audit 15: testbar)
fn feed_reason(d: &Deployment, age_min: f64, min_change: f64, max_age_min: f64, p: i64) -> Option<String> {
    let old = d.oracle.state.kas_usd as f64;
    let change = (p as f64 - old) / old;
    if d.oracle.state.frozen {
        return Some("Orakel ist eingefroren – Preis taut es auf".into());
    }
    if age_min > heartbeat_min(d.oracle_params.freeze_after_daa, max_age_min) {
        return Some(format!("Alter {age_min:.0} min"));
    }
    (change.abs() >= min_change).then(|| format!("Preis {:.6} → {:.6} USD ({:+.2} %)", old / 1e8, p as f64 / 1e8, change * 100.0))
}

/// Eigene Token-UTXOs, größte zuerst, höchstens 2 (Grenze der GHOST-Gruppe
/// neben dem Minter-Zweig).
fn own_tokens(d: &Deployment, k: &Keypair) -> Vec<usize> {
    let mut v: Vec<usize> = d.tokens.iter().enumerate().filter(|(_, t)| t.state.owner == xonly(k)).map(|(i, _)| i).collect();
    v.sort_by_key(|&i| std::cmp::Reverse(d.tokens[i].state.amount));
    v.truncate(2);
    v
}

async fn median_now() -> Result<i64, String> {
    let qs = tokio::task::spawn_blocking(price::fetch_all).await.map_err(|e| e.to_string())?;
    price::median_price(&qs.into_iter().filter_map(Result::ok).collect::<Vec<_>>(), 0.03)
}

async fn oracle_update(ctx: &Ctx, key: &Path, committee: &Path, usd: Option<f64>, rate: Option<f64>) -> Result<(), String> {
    let kas_usd = match usd {
        Some(u) => amount(u, "--usd")?,
        None => median_now().await?,
    };
    rate_by_hand(&RateFile::of(ctx), rate, sent_count, oracle_update_price(ctx, key, committee, kas_usd, rate)).await
}

async fn oracle_update_price(ctx: &Ctx, key: &Path, committee: &Path, kas_usd: i64, rate: Option<f64>) -> Result<(), String> {
    oracle_update_with(ctx, key, committee, kas_usd, rate, false).await
}

/// `fit`: Zinsregel des Agenten – den Zins an die Vertragsgrenzen anpassen statt abzulehnen
async fn oracle_update_with(ctx: &Ctx, key: &Path, committee: &Path, kas_usd: i64, rate: Option<f64>, fit: bool) -> Result<(), String> {
    let k = load_key(key)?;
    let com = load_committee(committee)?;
    let d = ctx.load_synced().await?;
    if d.signers_unknown {
        return Err("Der Unterzeichner-Satz im Register ist dieser Zustandsdatei unbekannt (Austausch von einem anderen Rechner) – Preis-Updates erst mit dessen Datei".into());
    }
    let r = match rate {
        Some(pct) if pct.is_finite() && pct >= 0.0 => rate_from_apr(pct),
        Some(_) => return Err("--rate muss ≥ 0 sein".into()),
        None => d.oracle.state.stable_rate,
    };
    // DAA etwas zurück, damit die Locktime sicher erreicht ist
    let daa = ctx.net.daa().await?.saturating_sub(20);
    let r = if fit {
        let (r, note) = fit_rate(&d.oracle_params, &d.oracle.state, r, daa as i64);
        if let Some(n) = note {
            say!("Zinsregel: {n}");
        }
        r
    } else {
        r
    };
    // Grenzen des Vertrags vorab (Preis ×2/÷2, Abstand, Zinsrahmen)
    ops::check_update(&d.oracle_params, &d.oracle.state, kas_usd, daa as i64, r)?;
    let signers = ops::signers_of(&d.signer_set, &com);
    if (signers.len() as i64) < d.signer_set.t {
        return Err(format!("Komitee-Datei enthält {} von {} nötigen Schlüsseln des aktuellen Satzes", signers.len(), d.signer_set.t));
    }
    let (b, nd) = ops::oracle_update(&d, &signers, kas_usd, r, daa, &ctx.funds(&k).await?, &ctx.net.params)?;
    ctx.send(&format!("Orakel {:.6} USD", kas_usd as f64 / 1e8), &b, Some(&nd)).await?;
    Ok(())
}

// --------------------------------------------------------- Unterzeichner ----

fn hex32_list(s: &str, what: &str) -> Result<Vec<Vec<u8>>, String> {
    s.split(',')
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .map(|x| {
            let mut b = vec![0u8; 32];
            if x.len() != 64 {
                return Err(format!("{what}: x-only-Pubkey mit 64 Hex-Zeichen erwartet, nicht „{x}“"));
            }
            faster_hex::hex_decode(x.as_bytes(), &mut b).map_err(|e| format!("{what}: {e}"))?;
            Ok(b)
        })
        .collect()
}

fn set_json(s: &SignerSet) -> serde_json::Value {
    serde_json::json!({ "keys": s.keys.iter().map(|k| faster_hex::hex_string(k)).collect::<Vec<_>>(), "threshold": s.t, "rotateThreshold": s.t_rot })
}

fn signers_json(d: &Deployment, daa: u64) -> serde_json::Value {
    let rp = &d.register_params;
    serde_json::json!({
        "registerCovenantId": d.register.cov.to_string(),
        "set": set_json(&d.signer_set),
        "fallback": d.fallback_set.as_ref().map(set_json),
        "rotateDelayHours": rp.rot_delay_daa as f64 / HOUR_DAA as f64,
        "emergencyAfterDays": rp.emerg_after_daa as f64 / DAY_DAA as f64,
        "emergencyDelayHours": rp.emerg_delay_daa as f64 / HOUR_DAA as f64,
        "freezeAfterHours": d.oracle_params.freeze_after_daa as f64 / HOUR_DAA as f64,
        // Änderung von außen erkannt (fremde Ankündigung, Absage, Notfall)
        "foreignChange": d.foreign_change.is_some_and(|n| n == d.register.state.nonce),
        "emergencyOpen": d.register.state.emerg,
        "unknownSet": d.signers_unknown,
        "oldTickets": d.old_tickets.len(),
        "rotation": d.rotation.as_ref().map(|r| serde_json::json!({
            "set": set_json(&r.set),
            "fallback": r.fallback.as_ref().map(set_json),
            "emergency": r.emergency,
            "valid": r.ticket.state.nonce == d.register.state.nonce,
            // negativ = Wartezeit vorbei (Audit 14 H1: vorher bei 0 abgeschnitten)
            "readyInHours": (r.ready_daa as f64 - daa as f64) / HOUR_DAA as f64,
        })),
    })
}

async fn signers_cmd(ctx: &Ctx, cmd: SignerCmd, extra: &mut serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let p = ctx.net.params.clone();
    let d = ctx.load_synced().await?;
    let daa = ctx.net.daa().await?;
    match cmd {
        SignerCmd::Show => {}
        SignerCmd::Propose { key, committee, new_committee, new_keys, threshold, rotate_threshold, fallback_keys, fallback_threshold, emergency } => {
            let k = load_key(&key)?;
            let com = load_committee(&committee)?;
            let keys = match (new_committee, new_keys) {
                (Some(f), None) => load_committee(&f)?.iter().map(xonly).collect(),
                (None, Some(h)) => hex32_list(&h, "--new-keys")?,
                _ => return Err("genau eines von --new-committee oder --new-keys angeben".into()),
            };
            let t = threshold.unwrap_or(keys.len() as i64 / 2 + 1);
            let new_set = SignerSet { keys, t, t_rot: rotate_threshold.unwrap_or(t) };
            let fb = match fallback_keys {
                Some(h) => {
                    let keys = hex32_list(&h, "--fallback-keys")?;
                    let t = fallback_threshold.unwrap_or(keys.len() as i64 / 2 + 1);
                    Some(SignerSet { keys, t, t_rot: t })
                }
                None => None,
            };
            if d.rotation.as_ref().is_some_and(|r| r.ticket.state.nonce == d.register.state.nonce) {
                say!("Hinweis: Eine offene Ankündigung wird durch diese ersetzt.");
            }
            let auth = if emergency { d.fallback_set.as_ref().ok_or("kein Notfallsatz festgelegt")? } else { &d.signer_set };
            let signers = ops::signers_of(auth, &com);
            let (b, nd) = ops::propose(&d, &signers, &new_set, fb.as_ref(), emergency, daa.saturating_sub(20), &ctx.funds(&k).await?, &p)?;
            let wait = if emergency { d.register_params.emerg_delay_daa } else { d.register_params.rot_delay_daa };
            ctx.send("Austausch ankündigen", &b, Some(&nd)).await?;
            say!(
                "Angekündigt: {} Schlüssel, Schwelle {}. Aktivieren frühestens in {:.1} h (`ghostctl signers activate`); absagen mit `signers cancel`.",
                new_set.n(),
                new_set.t,
                wait as f64 / HOUR_DAA as f64
            );
        }
        SignerCmd::Cancel { key, committee } => {
            let k = load_key(&key)?;
            let com = load_committee(&committee)?;
            let (b, nd) = ops::cancel_rotation(&d, &ops::signers_of(&d.signer_set, &com), &ctx.funds(&k).await?, &p)?;
            ctx.send("Ankündigung absagen", &b, Some(&nd)).await?;
        }
        SignerCmd::Activate { key } => {
            let k = load_key(&key)?;
            let rot = d.rotation.as_ref().ok_or("keine Ankündigung offen")?;
            if daa < rot.ready_daa {
                say!("Hinweis: Die Wartezeit läuft laut Zustandsdatei noch {:.1} h; der Node lehnt eine zu frühe Aktivierung ab.", (rot.ready_daa - daa) as f64 / HOUR_DAA as f64);
            }
            let (b, nd) = ops::activate_rotation(&d, &ctx.funds(&k).await?, &p)?;
            ctx.send("Austausch aktivieren", &b, Some(&nd)).await?;
            say!("Neuer Satz aktiv: {} Schlüssel, Schwelle {}.", nd.signer_set.n(), nd.signer_set.t);
        }
        SignerCmd::Clear { key } => {
            let k = load_key(&key)?;
            if d.old_tickets.is_empty() {
                return Err("kein abgesagtes oder überholtes Ticket bekannt".into());
            }
            let mut cur = d.clone();
            for tk in d.old_tickets.clone() {
                let (b, nd) = ops::clear_ticket(&cur, &tk, &ctx.funds(&k).await?, &p)?;
                ctx.send("Ticket aufräumen", &b, Some(&nd)).await?;
                cur = nd;
            }
        }
    }
    let after = ctx.load().unwrap_or(d);
    let j = signers_json(&after, daa);
    say!("{}", serde_json::to_string_pretty(&j).unwrap());
    extra.insert("signers".into(), j);
    Ok(())
}

// ---------------------------------------------------- Senden mit Nachricht ----

/// --message/--onchain-message prüfen → (Nachricht, Payload). Ohne
/// --onchain-message wird eine Nachricht an `recipient` (x-only-Pubkey des
/// Empfängers) verschlüsselt; der Absender behält sie im lokalen Verlauf,
/// entschlüsseln kann er sie später nicht.
fn message_args(message: Option<&str>, onchain: bool, recipient: Option<&[u8]>) -> Result<(String, Vec<u8>), String> {
    let m = message.unwrap_or("").trim().to_string();
    abo::check_message(&m).map_err(|e| format!("--message: {e}"))?;
    if onchain && m.is_empty() {
        return Err("--onchain-message braucht eine --message".into());
    }
    let payload = message::payload_for(&m, onchain, recipient).map_err(|e| format!("--message: {e}"))?;
    Ok((m, payload))
}

/// Ein- und Ausgabe von `message::inbox`: öffentliche REST-API und Node. Der
/// Ablauf selbst (Tresore, GHOST, Abgleich am Node) steht in der Bibliothek und
/// läuft in den Tests gegen Attrappen.
struct InboxNet<'a> {
    network: &'a str,
    rpc: Option<&'a str>,
    net: Option<Net>,
}

impl message::InboxIo for InboxNet<'_> {
    fn address_txs(&mut self, address: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
        kaspa_lending_protocol::chain::address_txs_json(self.network, address, limit)
    }
    fn tx(&mut self, txid: &str) -> Result<serde_json::Value, String> {
        kaspa_lending_protocol::chain::tx_json(self.network, txid)
    }
    async fn connect(&mut self) -> bool {
        if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_secs(30), Net::connect(self.network, self.rpc)).await {
            self.net = Some(n);
        }
        self.net.is_some()
    }
    /// Tx aus einem der Blöcke, die die REST-API nennt, so wie der Node sie
    /// führt. Blockinhalte löscht ein Node nach 30 bis 42 Stunden (Pruning-
    /// Tiefe 30 h, der Pruning-Punkt rückt in 12-h-Schritten vor); dann
    /// liefert er nur noch den Kopf, ganz ohne Transaktionen.
    async fn node_tx(&mut self, rest: &serde_json::Value) -> message::NodeLookup {
        use kaspa_rpc_core::api::rpc::RpcApi;
        use message::NodeLookup;
        use std::str::FromStr;
        let Some(net) = &self.net else { return NodeLookup::Unavailable };
        let Some(txid) = rest["transaction_id"].as_str().and_then(|s| kaspa_consensus_core::Hash::from_str(s).ok()) else { return NodeLookup::Unavailable };
        let Some(hashes) = rest["block_hash"].as_array() else { return NodeLookup::Unavailable };
        let (mut body_without_tx, mut unknown, mut asked) = (false, 0usize, 0usize);
        for bh in hashes.iter().take(3) {
            let Some(h) = bh.as_str().and_then(|s| kaspa_consensus_core::Hash::from_str(s).ok()) else { continue };
            asked += 1;
            let block = match tokio::time::timeout(Duration::from_secs(10), net.client.get_block(h, true)).await {
                Ok(Ok(b)) => b,
                // unbekannter Block: der Node kennt nicht einmal den Kopf
                Ok(Err(e)) if e.to_string().contains("cannot find header") => {
                    unknown += 1;
                    continue;
                }
                _ => continue,
            };
            if block.transactions.is_empty() {
                continue; // nur der Kopf: Inhalt schon gelöscht
            }
            // „steht nicht darin“ nur, wenn jede Tx des Blocks bestimmbar war
            let mut all_known = true;
            for tx in &block.transactions {
                let id = match &tx.verbose_data {
                    Some(v) => v.transaction_id,
                    None => match kaspa_consensus_core::tx::Transaction::try_from(tx.clone()) {
                        Ok(t) => t.id(),
                        Err(_) => {
                            all_known = false;
                            continue;
                        }
                    },
                };
                if id == txid {
                    return NodeLookup::Found(message::NodeTx {
                        payload: tx.payload.clone(),
                        sigs: tx.inputs.iter().map(|i| i.signature_script.clone()).collect(),
                        outputs: tx.outputs.iter().map(|o| (o.value, o.script_public_key.script().to_vec())).collect(),
                    });
                }
            }
            body_without_tx |= all_known;
        }
        if body_without_tx {
            NodeLookup::NotInBlock
        } else if asked > 0 && unknown == asked {
            NodeLookup::UnknownBlock
        } else {
            NodeLookup::Unavailable
        }
    }
    async fn retention(&mut self) -> Option<message::Retention> {
        use kaspa_rpc_core::api::rpc::RpcApi;
        let net = self.net.as_ref()?;
        let info = tokio::time::timeout(Duration::from_secs(10), net.client.get_block_dag_info()).await.ok()?.ok()?;
        let pp = tokio::time::timeout(Duration::from_secs(10), net.client.get_block(info.pruning_point_hash, false)).await.ok()?.ok()?;
        Some(message::Retention { pruning_ms: pp.header.timestamp, pmt_ms: info.past_median_time })
    }
}

/// Wie `messages` einen gelesenen Payload zeigt
struct FoundView {
    /// angezeigter Text (nur öffentlich oder entschlüsselt und zulässig)
    text: Option<String>,
    /// encrypted | public | unreadable | invalid
    kind: &'static str,
    /// bei `invalid` der Grund für die Seite: length | chars | both
    invalid: Option<&'static str>,
    /// Zeile fürs Terminal
    line: String,
}

/// Anzeige eines gelesenen Payloads. Bei `invalid` mit dem richtigen Grund:
/// zu lang (etwa JSON einer anderen Anwendung) ist etwas anderes als
/// unzulässige Zeichen (Restpunkt zu A12-11).
fn found_view(f: &message::Found) -> FoundView {
    use message::{Found, Rejected};
    let max = abo::MAX_MESSAGE_CHARS;
    let (text, kind, invalid, line) = match f {
        Found::Private(t) => (Some(t.clone()), "encrypted", None, format!("„{t}“ (verschlüsselt)")),
        Found::Unreadable => (None, "unreadable", None, "verschlüsselt, nicht für diesen Schlüssel lesbar".to_string()),
        Found::Public(t) => (Some(t.clone()), "public", None, format!("„{t}“ (öffentlich)")),
        Found::Invalid(Rejected::TooLong) => (None, "invalid", Some("length"), format!("Nachricht zu lang (über {max} Zeichen) – nicht angezeigt")),
        Found::Invalid(Rejected::BadChars) => (None, "invalid", Some("chars"), "Nachricht mit unsichtbaren oder unzulässigen Zeichen – nicht angezeigt".to_string()),
        Found::Invalid(Rejected::TooLongAndBadChars) => {
            (None, "invalid", Some("both"), format!("Nachricht zu lang (über {max} Zeichen) und mit unsichtbaren oder unzulässigen Zeichen – nicht angezeigt"))
        }
    };
    FoundView { text, kind, invalid, line }
}

/// Eine eingegangene Nachricht für `messages`: Zeile fürs Terminal und JSON
/// für die Seite (Text, Art samt Grund bei `invalid`, Herkunft, Quelle)
fn inbox_item(f: &message::Inbox, prefix: kaspa_addresses::Prefix) -> (String, serde_json::Value) {
    let fmt_time = |ms: Option<u64>| {
        ms.and_then(|ms| chrono::DateTime::from_timestamp_millis(ms as i64)).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
    };
    let view = found_view(&f.found);
    // Herkunft: nie Absender-Echtheit, aber was sich über die Nachricht sagen
    // lässt. Den Besitzer eines Tresors nur bei hier übernommenen Tresoren,
    // und auch dann nur „laut Tresor-Code“ (anlegen kann jeder einen Tresor
    // mit beliebigem Besitzer, ohne dessen Signatur).
    let (origin, tresor_id, tresor_owner, origin_text) = match &f.origin {
        message::Origin::Direct => ("direct", None, None, String::new()),
        message::Origin::Contract => ("contract", None, None, " [über einen Vertrag – Herkunft der Nachricht nicht prüfbar]".to_string()),
        message::Origin::Tresor { id, owner, check } => {
            let owner = owner.map(|o| xonly_address(prefix, &o));
            let name = match (id, &owner) {
                (Some(i), Some(o)) => format!("Tresor {i}, Besitzer laut Tresor-Code {o}"),
                _ => "Tresor".into(),
            };
            match check {
                message::TresorCheck::AsStored => ("tresor-stored", id.clone(), owner, format!(" [{name}: vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)]")),
                message::TresorCheck::Bound => (
                    "tresor-bound",
                    id.clone(),
                    owner,
                    format!(" [{name}: vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen){}]", if id.is_some() { ", anders als die Beschreibung im Tresor-Code" } else { ", Besitzer nicht geprüft" }),
                ),
                message::TresorCheck::Inserted => ("tresor-inserted", id.clone(), owner, format!(" [{name}: passt NICHT zur im Vertrag gebundenen Nachricht – nicht vom Absender]")),
            }
        }
    };
    let source = match f.source {
        message::Source::Node => "node",
        message::Source::Rest => "rest",
    };
    let at = fmt_time(f.time_ms);
    let line = format!(
        "{}  {} {}  von {}  {}{}{}",
        at.as_deref().unwrap_or("–"),
        abo::fmt_amount(f.amount),
        f.unit,
        if f.from.is_empty() { "unbekannt".to_string() } else { f.from.join(", ") },
        view.line,
        origin_text,
        if f.source == message::Source::Node { "  (am Node geprüft)" } else { "  (laut REST-API)" }
    );
    let item = serde_json::json!({
        "txid": f.txid,
        "timeMs": f.time_ms,
        "at": at,
        "amount": abo::fmt_amount(f.amount),
        "unit": f.unit,
        "from": f.from,
        "text": view.text,
        "kind": view.kind,
        "invalid": view.invalid,
        "origin": origin,
        "tresor": tresor_id,
        "tresorOwner": tresor_owner,
        "source": source,
    });
    (line, item)
}

/// Eingegangene Nachrichten an den Schlüssel `k` (Ablauf: message::inbox)
async fn messages(network: &str, rpc: Option<&str>, k: &Keypair, state: &Path, limit: usize) -> Result<serde_json::Value, String> {
    use serde_json::json;
    let sk = SecretKey::from_keypair(k);
    let mut io = InboxNet { network, rpc, net: None };
    let res = message::inbox(&mut io, network, state, &sk, limit).await;
    if let Some(net) = &io.net {
        // auch das Trennen im Zeitrahmen (message::INBOX_BUDGET)
        let _ = tokio::time::timeout(Duration::from_secs(5), net.client.disconnect()).await;
    }
    let message::InboxResult { address: my_addr, messages: found, notes, checks, ghost_checked, node_checked: node_ok, hidden } = res?;
    let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(network)?);
    let mut list = vec![];
    for f in &found {
        let (line, item) = inbox_item(f, prefix);
        say!("{line}");
        list.push(item);
    }
    if list.is_empty() {
        say!("Keine eingegangenen Nachrichten an {my_addr} (letzte {limit} Transaktionen).");
    }
    for n in notes.iter().chain(&checks) {
        say!("Hinweis: {n}");
    }
    say!("Hinweis: Eine Nachricht beweist nicht, wer sie geschrieben hat – jeder kann an diese Adresse verschlüsseln; „von“ sind die Eingänge der Zahlung, bei einem Tresor ohne dessen Besitzer (den kann jeder beim Anlegen frei eintragen).");
    Ok(json!({
        "ok": true,
        "network": network,
        "address": my_addr,
        "limit": limit,
        "ghostChecked": ghost_checked,
        "messages": list,
        "notes": notes,
        "checks": checks,
        "nodeChecked": node_ok,
        "hidden": hidden,
    }))
}

/// KAS-Empfänger: Adresse des gewählten Netzes oder Schlüsseldatei
fn kas_target(prefix: kaspa_addresses::Prefix, to: &str) -> Result<kaspa_consensus_core::tx::ScriptPublicKey, String> {
    if Path::new(to).exists() {
        return Ok(p2pk_spk(&xonly(&load_key(Path::new(to))?)));
    }
    let addr = kaspa_addresses::Address::try_from(to).map_err(|e| format!("--to: {e}"))?;
    if addr.prefix != prefix {
        return Err(format!("Adresse gehört zu einem anderen Netz ({})", addr.prefix));
    }
    Ok(kaspa_txscript::pay_to_address_script(&addr))
}

/// GHOST-Empfänger → x-only-Pubkey (Schlüsseldatei, Schnorr-Adresse oder 64 Hex)
fn ghost_target(prefix: kaspa_addresses::Prefix, network: &str, to: &str) -> Result<Vec<u8>, String> {
    let to_x = if Path::new(to).exists() {
        xonly(&load_key(Path::new(to))?)
    } else if to.contains(':') {
        // Kaspa-Adresse: nur Schnorr-P2PK, deren Nutzlast der x-only-Pubkey ist
        let a = kaspa_addresses::Address::try_from(to).map_err(|e| format!("--to: ungültige Adresse ({e})"))?;
        if a.prefix != prefix {
            return Err(format!("--to: Adresse gehört nicht zum Netz {network}"));
        }
        if a.version != kaspa_addresses::Version::PubKey || a.payload.len() != 32 {
            return Err("--to: GHOST gehen nur an normale Schnorr-Adressen (kaspa:q…)".into());
        }
        a.payload.to_vec()
    } else {
        let mut b = vec![0u8; 32];
        faster_hex::hex_decode(to.as_bytes(), &mut b).map_err(|_| "--to: Kaspa-Adresse, Schlüsseldatei oder 64 Hex-Zeichen (x-only-Pubkey)")?;
        b
    };
    // Nutzlast muss ein Punkt auf secp256k1 sein, sonst sind die GHOST
    // für immer verloren – auch eine gültige kaspa:q…-Adresse kann das tragen (A10-A-8)
    secp256k1::XOnlyPublicKey::from_slice(&to_x).map_err(|_| "--to: kein gültiger Schnorr-Schlüssel (Tippfehler?) – nichts gesendet")?;
    Ok(to_x)
}

/// KAS-Überweisung bauen und prüfen (wie `send`), optional mit Payload
async fn build_kas(ctx: &Ctx, k: &Keypair, spk: kaspa_consensus_core::tx::ScriptPublicKey, units: u64, payload: &[u8]) -> Result<Built, String> {
    let fund = ctx.funds(k).await?;
    let out = kaspa_consensus_core::tx::TransactionOutput { value: units, script_public_key: spk, covenant: None };
    let inputs = fund
        .utxos
        .iter()
        .map(|(op, e)| kaspa_lending_protocol::txb::In { outpoint: *op, entry: e.clone(), unlock: kaspa_lending_protocol::txb::Unlock::P2pk { signer: k.into() } })
        .collect();
    kaspa_lending_protocol::txb::build_with_payload(
        kaspa_lending_protocol::txb::Draft { inputs, outputs: vec![out], change_spk: p2pk_spk(&xonly(k)), lock_time: 0 },
        payload,
        &ctx.net.params,
    )
}

/// GHOST-Überweisung bauen und prüfen (wie `transfer`), optional mit Payload.
/// Liegen die GHOST auf mehr als zwei UTXOs, wird vorher zusammengeführt.
async fn build_ghost(ctx: &Ctx, k: &Keypair, to_x: &[u8], units: i64, payload: &[u8], p: &kaspa_consensus_core::config::params::Params) -> Result<(Built, Deployment), String> {
    let d = ctx.load_synced().await?;
    let d = consolidate(ctx, d, k, units, p).await?;
    let mine = own_tokens(&d, k);
    ops::transfer_with_payload(&d, k, &mine, to_x, units, payload, &ctx.funds(k).await?, p)
}

/// Nachricht an die JSON-Ausgabe der letzten Tx hängen und die Sendung im
/// lokalen Verlauf (deployments/<netz>-txlog.jsonl) festhalten
fn note_sent(ctx: &Ctx, b: &Built, mut entry: serde_json::Value) {
    if let Some(r) = TXS.lock().unwrap().last_mut() {
        r["message"] = entry["message"].clone();
        r["onchain"] = entry["onchain"].clone();
        r["encrypted"] = entry["encrypted"].clone();
    }
    if ctx.dry_run {
        return;
    }
    entry["txid"] = b.tx.id().to_string().into();
    entry["network"] = ctx.network.clone().into();
    entry["at"] = abo_now().0.into();
    if let Err(e) = abo::log_tx(&ctx.state_path, &entry) {
        say!("Hinweis: lokaler Verlauf nicht geschrieben: {e}");
    }
}

// ---------------------------------------------------------- Daueraufträge ----

/// (lokale Zeit für den Verlauf, Unix-Sekunden, heutiges Datum)
fn abo_now() -> (String, u64, chrono::NaiveDate) {
    let now = chrono::Local::now();
    (now.format("%Y-%m-%d %H:%M").to_string(), now.timestamp().max(0) as u64, now.date_naive())
}

fn parse_date(s: &str, what: &str) -> Result<chrono::NaiveDate, String> {
    s.trim().parse::<chrono::NaiveDate>().map_err(|_| format!("{what}: Datum als JJJJ-MM-TT erwartet, nicht „{s}“"))
}

fn abo_json(a: &abo::Abo) -> serde_json::Value {
    let mut v = serde_json::to_value(a).unwrap();
    v["status"] = a.status().into();
    v["upcoming"] = serde_json::to_value(a.upcoming(3)).unwrap();
    v
}

/// abo-Befehle ohne Netz. Ok(None) = `run` mit Fälligem, braucht das Netz.
fn abo_offline(network: &str, state: &Path, cmd: &AboCmd, dry_run: bool) -> Result<Option<serde_json::Value>, String> {
    use serde_json::json;
    let path = abo::path_for(state);
    let (now, now_unix, today) = abo_now();
    let out = |a: &abo::Abo| Ok(Some(json!({ "ok": true, "network": network, "abo": abo_json(a) })));
    match cmd {
        AboCmd::Add { key, asset, to, amount, interval, start, end, count, message, onchain_message } => {
            load_key(key)?; // Schlüsseldatei muss lesbar sein; nichts davon wird gespeichert
            let asset = abo::Asset::parse(asset)?;
            let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(network)?);
            let recipient = match asset {
                abo::Asset::Kas => message::recipient_of_spk(&kas_target(prefix, to.trim())?).map(|x| x.to_vec()),
                abo::Asset::Ghost => Some(ghost_target(prefix, network, to.trim())?),
            };
            // verschlüsselt geht nur an Schnorr-Adressen: schon beim Anlegen prüfen
            message::payload_for(message.as_deref().unwrap_or(""), *onchain_message, recipient.as_deref()).map_err(|e| format!("--message: {e}"))?;
            let n = abo::NewAbo {
                key: key.to_string_lossy().to_string(),
                asset,
                to: to.clone(),
                amount: amount.clone(),
                message: message.clone().unwrap_or_default(),
                onchain: *onchain_message,
                interval: abo::Interval::parse(interval)?,
                start: match start {
                    Some(s) => parse_date(s, "--start")?,
                    None => today,
                },
                end: end.as_deref().map(|e| parse_date(e, "--end")).transpose()?,
                count: *count,
            };
            let a = abo::new_abo(n, today, &now, format!("{:08x}", rand::random::<u32>()))?;
            let what = format!("{} {} → {}, {}, erster Termin {}", a.amount, a.asset.unit(), a.to, a.interval, a.start);
            if dry_run {
                say!("Probelauf: Dauerauftrag {what} – nicht angelegt");
                return out(&a);
            }
            let a = abo::add(&path, network, a)?;
            say!("Dauerauftrag {} angelegt: {what}", a.id);
            out(&a)
        }
        AboCmd::List => {
            let f = abo::load(&path, network)?;
            for a in &f.abos {
                say!(
                    "{}  {:<13} {} {} → {}  {}  nächster Termin {}{}",
                    a.id,
                    a.status(),
                    a.amount,
                    a.asset.unit(),
                    a.to,
                    a.interval,
                    a.next_due.map(|d| d.to_string()).unwrap_or_else(|| "–".into()),
                    if a.message.is_empty() {
                        String::new()
                    } else {
                        format!("  „{}“ ({})", a.message, if a.onchain { "öffentlich" } else if a.encrypt { "verschlüsselt" } else { "nur lokal" })
                    }
                );
            }
            if f.abos.is_empty() {
                say!("Keine Daueraufträge in {}", path.display());
            }
            Ok(Some(json!({
                "ok": true,
                "network": network,
                "abos": f.abos.iter().map(abo_json).collect::<Vec<_>>(),
                "archive": f.archive.iter().map(abo_json).collect::<Vec<_>>(),
            })))
        }
        AboCmd::Pause { id } => {
            let a = abo::pause(&path, network, id)?;
            say!("Dauerauftrag {id} pausiert");
            out(&a)
        }
        AboCmd::Resume { id } => {
            let a = abo::resume(&path, network, id, today, &now)?;
            say!("Dauerauftrag {id} läuft wieder, nächster Termin {}", a.next_due.map(|d| d.to_string()).unwrap_or_else(|| "–".into()));
            out(&a)
        }
        AboCmd::Remove { id } => {
            let a = abo::remove(&path, network, id, &now)?;
            say!("Dauerauftrag {id} beendet (im Archiv)");
            out(&a)
        }
        AboCmd::Run { today: t } => {
            if t.is_some() && !dry_run {
                return Err("--today nur zusammen mit --dry-run".into());
            }
            let today = match t {
                Some(t) => parse_date(t, "--today")?,
                None => today,
            };
            // ohne Fälliges keine Verbindung zum Node (die Seite fragt jede Minute)
            if !abo::needs_run(&abo::load(&path, network)?, today, now_unix) {
                say!("Keine fälligen Daueraufträge.");
                return Ok(Some(json!({ "ok": true, "network": network, "dryRun": dry_run, "reports": [], "transactions": [] })));
            }
            Ok(None)
        }
    }
}

/// Alle fälligen Aufträge ausführen (Zustandssperre hält der Aufrufer)
async fn abo_run(ctx: &Ctx, today: Option<&str>) -> Result<Vec<abo::Report>, String> {
    let (now, now_unix, t) = abo_now();
    let today = match today {
        Some(s) => parse_date(s, "--today")?,
        None => t,
    };
    let o = abo::RunOpts { today, now, now_unix, dry_run: ctx.dry_run, lock_wait: Duration::from_secs(10) };
    let reports = abo::run(&abo::path_for(&ctx.state_path), &ctx.network, &o, &mut AboPayer { ctx }).await?;
    for r in &reports {
        say!("Dauerauftrag {}: {}", r.id, r.text);
    }
    Ok(reports)
}

/// Agent-Runde für Daueraufträge: eigene Verbindung, eigene Sperre, eigener
/// Fehlerpfad; ohne Fälliges keine Netzabfrage.
async fn abo_agent_step(network: &str, rpc: Option<&str>, state: &Path, yes: bool, dry_run: bool, now: &str) {
    let path = abo::path_for(state);
    let (_, now_unix, today) = abo_now();
    match abo::load(&path, network) {
        Ok(f) if abo::needs_run(&f, today, now_unix) => {}
        Ok(_) => return,
        Err(e) => return eprintln!("[{now}] Daueraufträge: {e}"),
    }
    let step = async {
        let net = match Net::connect(network, rpc).await {
            Ok(n) => n,
            Err(e) => return eprintln!("[{now}] Daueraufträge: keine Verbindung zum Node: {e}"),
        };
        let ctx = Ctx { mainnet: network == "mainnet", yes, dry_run, network: network.into(), net, state_path: state.to_path_buf() };
        match store::lock(&ctx.state_path, Duration::from_secs(60)) {
            Err(e) => eprintln!("[{now}] Daueraufträge: {e}"),
            Ok(_lock) => {
                if let Err(e) = abo_run(&ctx, None).await {
                    eprintln!("[{now}] Daueraufträge: {e}");
                }
            }
        }
        let _ = ctx.net.client.disconnect().await;
    };
    // Senden wartet bis zu 600 s auf die Bestätigung; ein Abbruch hier ist
    // dank Journal harmlos, der nächste Lauf klärt die Zahlung
    if tokio::time::timeout(Duration::from_secs(700), step).await.is_err() {
        eprintln!("[{now}] Daueraufträge: Runde nach 700 s abgebrochen, die nächste klärt offene Zahlungen");
    }
}

enum AboTx {
    Kas(Built),
    Ghost(Built, Box<Deployment>),
}

impl AboTx {
    fn built(&self) -> &Built {
        match self {
            AboTx::Kas(b) | AboTx::Ghost(b, _) => b,
        }
    }
}

/// Zahlungen der Daueraufträge über dieselben Wege wie `send` und `transfer`
struct AboPayer<'a> {
    ctx: &'a Ctx,
}

impl abo::Payer for AboPayer<'_> {
    type Tx = AboTx;

    fn info(tx: &AboTx) -> abo::TxInfo {
        let b = tx.built();
        abo::TxInfo {
            txid: b.tx.id().to_string(),
            fee: b.fee,
            outputs: b.tx.outputs.iter().enumerate().map(|(i, o)| (i as u32, o.script_public_key.clone())).collect(),
            inputs: b.tx.inputs.iter().zip(&b.entries).map(|(i, e)| (i.previous_outpoint, e.script_public_key.clone())).collect(),
        }
    }

    async fn prepare(&mut self, a: &abo::Abo, units: u64) -> Result<AboTx, String> {
        let ctx = self.ctx;
        let k = load_key(Path::new(&a.key))?;
        // je Ausführung frisch verschlüsselt (neuer Ephemeral-Schlüssel und Nonce)
        match a.asset {
            abo::Asset::Kas => {
                let spk = kas_target(ctx.net.prefix, &a.to)?;
                let payload = abo::payload(a, message::recipient_of_spk(&spk).as_ref().map(|x| x.as_slice()))?;
                Ok(AboTx::Kas(build_kas(ctx, &k, spk, units, &payload).await?))
            }
            abo::Asset::Ghost => {
                let to_x = ghost_target(ctx.net.prefix, &ctx.network, &a.to)?;
                let payload = abo::payload(a, Some(&to_x))?;
                let p = ctx.net.params.clone();
                let (b, nd) = build_ghost(ctx, &k, &to_x, units as i64, &payload, &p).await?;
                Ok(AboTx::Ghost(b, Box::new(nd)))
            }
        }
    }

    async fn submit(&mut self, a: &abo::Abo, tx: &AboTx) -> Result<(), abo::SubmitError> {
        let ctx = self.ctx;
        let what = format!("Dauerauftrag {}: {} {}", a.id, a.amount, a.asset.unit());
        let sent = || TXS.lock().unwrap().iter().filter(|r| r["sent"] == true).count();
        let before = sent();
        let r = match tx {
            AboTx::Kas(b) => ctx.send(&what, b, None).await,
            AboTx::Ghost(b, nd) => ctx.send(&what, b, Some(nd.as_ref())).await,
        };
        let b = tx.built();
        match r {
            Ok(()) => {
                note_sent(ctx, b, serde_json::json!({ "action": "abo", "abo": a.id, "amount": a.amount, "unit": a.asset.unit(), "to": a.to, "message": a.message, "onchain": a.onchain, "encrypted": a.encrypt && !a.onchain }));
                Ok(())
            }
            Err(e) => {
                // offen, wenn gesendet oder das Journal der Zustandsdatei die Tx noch führt
                let journal = std::fs::read_to_string(store::pending_path(&ctx.state_path))
                    .ok()
                    .and_then(|t| serde_json::from_str::<store::Pending>(&t).ok())
                    .is_some_and(|p| p.txid == b.tx.id().to_string());
                let maybe_sent = sent() != before || journal;
                Err(abo::SubmitError { declined: !maybe_sent && e == "abgebrochen", maybe_sent, msg: e })
            }
        }
    }

    async fn check(&mut self, _a: &abo::Abo, f: &abo::InFlight) -> abo::Check {
        use abo::Check;
        let net = &self.ctx.net;
        let Some(id) = &f.txid else { return Check::NotSent };
        let Ok(txid) = id.parse::<kaspa_consensus_core::Hash>() else { return Check::Unknown("TXID unlesbar".into()) };
        // Führt das Journal der Zustandsdatei diese Tx noch, klärt es store (und
        // übernimmt bei GHOST den neuen Zustand)
        let journal = std::fs::read_to_string(store::pending_path(&self.ctx.state_path)).ok().and_then(|t| serde_json::from_str::<store::Pending>(&t).ok());
        if journal.is_some_and(|p| &p.txid == id) {
            match store::resolve_pending(net, &self.ctx.state_path).await {
                Ok(Some(m)) if m.contains("war angenommen") => return Check::Accepted,
                Ok(Some(_)) => return Check::NotSent,
                Ok(None) => {}
                Err(e) if e.contains("noch unterwegs") => return Check::Pending,
                Err(_) => {}
            }
        }
        for (i, spk) in &f.outputs {
            match net.exists(spk, &kaspa_consensus_core::tx::TransactionOutpoint { transaction_id: txid, index: *i }).await {
                Ok(true) => return Check::Accepted,
                Ok(false) => {}
                Err(_) => return Check::Pending,
            }
        }
        let mempool = net.in_mempool(txid).await;
        if matches!(mempool, Ok(true)) {
            return Check::Pending;
        }
        // eine angenommene Tx verbraucht alle Eingänge
        for (op, spk) in &f.inputs {
            match net.exists(spk, op).await {
                Ok(true) => return if mempool.is_ok() { Check::NotSent } else { Check::Pending },
                Ok(false) => {}
                Err(_) => return Check::Pending,
            }
        }
        Check::Unknown("kein Ausgang sichtbar, alle Eingänge verbraucht".into())
    }
}

// ------------------------------------------------------------------ Tresore ----

// ------------------------------------------------- Browser-Wallet-Probe ----

fn read_text(p: &Path, what: &str) -> Result<String, String> {
    let meta = std::fs::metadata(p).map_err(|e| format!("{what} {}: {e}", p.display()))?;
    if meta.len() > 1_000_000 {
        return Err(format!("{what} {}: Datei zu groß", p.display()));
    }
    std::fs::read_to_string(p).map_err(|e| format!("{what} {}: {e}", p.display()))
}

fn wallet_confirm(mainnet: bool, yes: bool, what: &str, b: &Built) -> Result<(), String> {
    say!("→ {what}: Gebühr {:.8} KAS, {} Eingänge, {} Ausgänge", b.fee as f64 / 1e8, b.tx.inputs.len(), b.tx.outputs.len());
    if mainnet && !yes {
        eprint!("  MAINNET – wirklich senden? [j/N] ");
        std::io::stderr().flush().ok();
        let mut a = String::new();
        std::io::stdin().read_line(&mut a).ok();
        if a.trim().to_lowercase() != "j" {
            return Err("abgebrochen".into());
        }
    }
    Ok(())
}

/// Aktuelle UTXO eines Probe-Tresors am Node (lesend), auch nach einer Zahlung
async fn locate_probe(net: &Net, pr: &kaspa_lending_protocol::wallet::Probe) -> Result<(kaspa_lending_protocol::wallet::Probe, kaspa_consensus_core::tx::UtxoEntry), String> {
    let pmt = net.past_median_time().await?;
    for s in tresor::candidates(&pr.params, &pr.state, pmt) {
        let sp = spk(&standing_order(&pr.params, &s));
        let hits: Vec<_> = net.utxos(&net.address_of_spk(&sp)?).await?.into_iter().filter(|(_, e)| e.covenant_id == Some(pr.cov) && e.script_public_key == sp).collect();
        match hits.as_slice() {
            [] => continue,
            [(op, e)] => {
                let mut found = pr.clone();
                found.state = s;
                found.outpoint = tresor::outpoint_text(op);
                found.value = e.amount;
                return Ok((found, e.clone()));
            }
            _ => return Err("mehrere UTXOs derselben Tresor-Covenant".into()),
        }
    }
    Err("Probe-Tresor im Netz nicht gefunden (schon gekündigt, oder die Anlage-Tx ist noch nicht bestätigt?)".into())
}

async fn wallet_send(network: &str, rpc: Option<&str>, yes: bool, what: &str, b: &Built, out: &mut serde_json::Value) -> Result<(), String> {
    let net = Net::connect(network, rpc).await?;
    // alle Eingänge noch unverbraucht?
    for (i, (inp, e)) in b.tx.inputs.iter().zip(&b.entries).enumerate() {
        if !net.exists(&e.script_public_key, &inp.previous_outpoint).await? {
            return Err(format!("Eingang {i} ist im Netz nicht (mehr) vorhanden – nicht gesendet"));
        }
    }
    wallet_confirm(network == "mainnet", yes, what, b)?;
    let id = net.submit(b).await?;
    out["sent"] = true.into();
    out["txid"] = id.clone().into();
    say!("  gesendet: {id}");
    net.wait_accepted(b, 0, Duration::from_secs(600)).await?;
    out["confirmed"] = true.into();
    say!("  bestätigt");
    Ok(())
}

async fn wallet_cmd(network: &str, rpc: Option<&str>, yes: bool, cmd: &WalletCmd) -> Result<serde_json::Value, String> {
    use kaspa_lending_protocol::wallet as w;
    use serde_json::json;
    let nid = kaspa_lending_protocol::net::network_id(network)?;
    let prefix = kaspa_addresses::Prefix::from(nid);
    let params = kaspa_consensus_core::config::params::Params::from(nid);
    match cmd {
        WalletCmd::ExportUnsigned { stage, address, fund, due_minutes, probe } => {
            let owner = w::xonly_of_address(address, prefix)?;
            let plan = match stage.as_str() {
                "dry-cancel" => w::export_dry_cancel(&owner, network, prefix, now_ms(), &params)?,
                "tresor-open" => {
                    let fund = match fund {
                        Some(f) => amount(*f, "Startguthaben")? as u64,
                        None => w::PROBE_FUND,
                    };
                    let mins = due_minutes.unwrap_or(60);
                    if !(w::PROBE_DUE_MIN..=w::PROBE_DUE_MAX).contains(&mins) {
                        return Err(format!("Termin: {} bis {} Minuten nach jetzt", w::PROBE_DUE_MIN, w::PROBE_DUE_MAX));
                    }
                    let net = Net::connect(network, rpc).await?;
                    let addr = kaspa_addresses::Address::try_from(address.trim()).map_err(|_| "keine gültige Kaspa-Adresse".to_string())?;
                    let utxos = net.utxos(&addr).await?;
                    w::export_open(&owner, &utxos, fund, now_ms() + mins * 60_000, network, prefix, &net.params)?
                }
                "tresor-cancel" => {
                    let p = probe.as_ref().ok_or("tresor-cancel: --probe <Datei> fehlt")?;
                    let pr = w::Probe::decode(&read_text(p, "Probe-Datei")?, network)?;
                    if pr.params.owner != owner {
                        return Err("Der Probe-Tresor gehört nicht zu dieser Adresse".into());
                    }
                    let net = Net::connect(network, rpc).await?;
                    let (found, entry) = locate_probe(&net, &pr).await?;
                    w::export_cancel(&found, entry, network, prefix, &net.params)?
                }
                _ => return Err("Stufe: dry-cancel | tresor-open | tresor-cancel".into()),
            };
            say!("Unsignierte Tx ({}) für {}: Gebühr {:.8} KAS, {} Eingänge, {} Ausgänge – nichts gesendet", plan.stage, plan.address, plan.fee as f64 / 1e8, plan.tx.inputs.len(), plan.tx.outputs.len());
            Ok(json!({
                "ok": true,
                "stage": plan.stage,
                "network": network,
                "address": plan.address,
                "dryOnly": plan.dry_only,
                "feeSompi": plan.fee,
                "outputs": w::describe_outputs(&plan, prefix),
                "signInputs": plan.signers,
                "kastle": plan.kastle(),
                "kasware": plan.kasware(),
                "probe": plan.probe,
                "plan": plan,
            }))
        }
        WalletCmd::AttachSigs { plan, signed, send, save_probe } => {
            let plan = w::Plan::decode(&read_text(plan, "Plan")?)?;
            if plan.network != network {
                return Err(format!("Plan gehört zum Netz {}, gewählt ist {network}", plan.network));
            }
            let signed = w::parse_signed(&read_text(signed, "Wallet-Antwort")?)?;
            let (built, rep) = w::attach(&plan, &signed, &params)?;
            say!("Signatur {}", if rep.valid { "GÜLTIG – die Tx besteht die lokale Skriptprüfung" } else { "UNGÜLTIG" });
            for i in &rep.inputs {
                say!("  Eingang {} ({}): {}", i.index, i.kind, i.note);
            }
            if let Some(e) = &rep.error {
                say!("  Grund: {e}");
            }
            let mut out = json!({ "ok": true, "valid": rep.valid, "stage": plan.stage, "dryOnly": plan.dry_only, "report": rep, "sent": false });
            if let Some(b) = &built {
                out["txid"] = b.tx.id().to_string().into();
                out["feeSompi"] = b.fee.into();
                out["txJson"] = serde_json::to_string(&w::to_safe(&b.tx, &b.entries, prefix)).unwrap().into();
                if plan.stage == "tresor-open" {
                    out["probe"] = serde_json::to_value(w::probe_after(&plan, b)).unwrap();
                }
            }
            if *send {
                if plan.dry_only {
                    return Err("Trockenprobe: diese Tx gibt eine erfundene UTXO aus und wird nie gesendet".into());
                }
                let b = built.ok_or("Signatur ungültig – nicht gesendet")?;
                let what = if plan.stage == "tresor-open" { "Probe-Tresor anlegen (Wallet-Signatur)" } else { "Probe-Tresor kündigen (Wallet-Signatur)" };
                wallet_send(network, rpc, yes, what, &b, &mut out).await?;
                if let (Some(path), Some(p)) = (save_probe, out.get("probe")) {
                    store::atomic_write(path, &serde_json::to_string_pretty(p).unwrap())?;
                }
            }
            Ok(out)
        }
        WalletCmd::Build { .. } | WalletCmd::Submit { .. } => unreachable!("wallet_action_cmd"),
        WalletCmd::ProbePay { probe, send, save_probe } => {
            let pr = w::Probe::decode(&read_text(probe, "Probe-Datei")?, network)?;
            let net = Net::connect(network, rpc).await?;
            let (found, _) = locate_probe(&net, &pr).await?;
            let pmt = net.past_median_time().await?;
            if found.state.left == 0 {
                return Err("die Zahlung ist schon erfolgt – der Rest kommt nur mit Kündigen (Stufe 2) zurück".into());
            }
            if found.state.next_due >= pmt {
                return Err(format!("Termin {} ist im Netz noch nicht erreicht", tresor::fmt_time(found.state.next_due)));
            }
            let b = w::probe_pay(&found, &net.params)?;
            let next = w::probe_after_pay(&found, &b);
            say!("Zahlung {:.8} KAS an {} – Gebühr {:.8} KAS aus dem Tresor, Rest {:.8} KAS bleibt im Tresor", found.params.amount as f64 / 1e8, w::address_of_xonly(&found.params.recipient, prefix), b.fee as f64 / 1e8, next.value as f64 / 1e8);
            let mut out = json!({ "ok": true, "txid": b.tx.id().to_string(), "feeSompi": b.fee, "probe": next, "sent": false });
            if *send {
                wallet_send(network, rpc, yes, "Probe-Tresor: Zahlung auslösen", &b, &mut out).await?;
                if let Some(path) = save_probe {
                    store::atomic_write(path, &serde_json::to_string_pretty(&next).unwrap())?;
                }
            }
            Ok(out)
        }
    }
}

// ------------------------------------------------ Nutzeraktionen mit Wallet ----

/// Zustand lesen und mit dem Netz abgleichen, ohne etwas zu schreiben und ohne
/// Sperre (wallet build, wallet submit ohne --send). Ein offenes Journal wird
/// nicht aufgelöst; ist der Stand dadurch veraltet, scheitert submit am Vergleich.
async fn load_readonly(ctx: &Ctx) -> Result<Deployment, String> {
    let mut d = ctx.load()?;
    store::resync(&ctx.net, &mut d).await?;
    if let Some(rec) = d.pool.clone() {
        match pool::resync(&ctx.net, &ctx.network, &rec).await {
            Ok(n) => d.pool = Some(n),
            Err(e) => d.pool_unresolved = Some(e),
        }
    }
    Ok(d)
}

/// Betrag in Einheiten aus der Kommandozeile (Pflicht)
fn need_units(v: Option<f64>, name: &str) -> Result<i64, String> {
    amount(v.ok_or(format!("{name} fehlt"))?, name)
}

/// Kommandozeile → vollständig ausgerechnete Aktion (Vorgaben aus dem Zustand)
#[allow(clippy::too_many_arguments)]
fn wallet_action_of(d: &Deployment, prefix: kaspa_addresses::Prefix, owner: &[u8], cmd: &WalletCmd) -> Result<kaspa_lending_protocol::wallet_ops::Action, String> {
    use kaspa_lending_protocol::wallet_ops::{Action, ghost_target};
    let WalletCmd::Build { action, vault, vault_id, kas, ghost, keep, to, message, onchain_message, min, min_shares, percent, min_kas, min_ghost, .. } = cmd else {
        return Err("kein build".into());
    };
    let vault_n = || match vault_id {
        Some(id) => {
            let id = id.trim().to_lowercase();
            d.vaults.iter().position(|r| r.vault.cov.to_string() == id).ok_or_else(|| "Diesen Vault gibt es nicht (mehr) – Seite neu laden.".to_string())
        }
        None => vault.ok_or_else(|| "--vault fehlt".to_string()).and_then(|v| d.vaults.get(v).map(|_| v).ok_or(format!("Vault {v} gibt es nicht (vorhanden: {})", d.vaults.len()))),
    };
    let debt = |v: usize| d.vaults[v].vault.state.debt;
    Ok(match action.as_str() {
        "open-vault" => Action::OpenVault { kas: need_units(*kas, "--kas")? as u64 },
        "mint" => Action::Mint { vault: vault_n()?, ghost: need_units(*ghost, "--ghost")? },
        "repay" => {
            let v = vault_n()?;
            let g = match ghost {
                Some(g) => amount(*g, "--ghost")?.min(debt(v)),
                None => debt(v),
            };
            if g <= 0 {
                return Err("Keine GHOST-Schuld zu tilgen".into());
            }
            Action::Repay { vault: v, ghost: g }
        }
        "deposit" => Action::Deposit { vault: vault_n()?, kas: need_units(*kas, "--kas")? as u64 },
        "withdraw" => Action::Withdraw { vault: vault_n()?, keep: need_units(*keep, "--keep")? as u64 },
        "close" => Action::Close { vault: vault_n()? },
        "sweep" => Action::Sweep { vault: vault_n()? },
        "redeem" => Action::Redeem { vault: vault_n()?, ghost: need_units(*ghost, "--ghost")? },
        "liquidate" => {
            let v = vault_n()?;
            let g = match ghost {
                Some(g) => amount(*g, "--ghost")?.min(debt(v)),
                None => debt(v),
            };
            Action::Liquidate { vault: v, ghost: g }
        }
        "send" => {
            let t = to.as_deref().ok_or("--to fehlt")?.trim().to_string();
            // nur Adressen, nie Schlüsseldateien
            let a = kaspa_addresses::Address::try_from(t.as_str()).map_err(|_| "--to: keine gültige Kaspa-Adresse".to_string())?;
            if a.prefix != prefix {
                return Err(format!("--to: Adresse gehört zu einem anderen Netz ({})", a.prefix));
            }
            let spk = kaspa_txscript::pay_to_address_script(&a);
            let (_, payload) = message_args(message.as_deref(), *onchain_message, message::recipient_of_spk(&spk).as_ref().map(|x| x.as_slice()))?;
            Action::Send { to: t, kas: need_units(*kas, "--kas")? as u64, payload }
        }
        "transfer" => {
            let t = to.as_deref().ok_or("--to fehlt")?.trim().to_string();
            let x = ghost_target(prefix, &t)?;
            let (_, payload) = message_args(message.as_deref(), *onchain_message, Some(&x))?;
            Action::Transfer { to: t, ghost: need_units(*ghost, "--ghost")?, payload }
        }
        "swap" => {
            let rec = d.pool.as_ref().ok_or("In diesem Netz gibt es noch keinen Pool.")?;
            let (x, y, f) = (rec.pool.value as i64, rec.reserve.state.amount, rec.params.fee_bps);
            match (kas, ghost) {
                (Some(k), None) => {
                    let dx = amount(*k, "--kas")?;
                    let m = match min {
                        Some(m) => amount(*m, "--min")?,
                        None => (pool::ghost_out(x, y, dx, f) * 99 / 100).max(1),
                    };
                    Action::Swap { kas: Some(dx), ghost: None, min: m }
                }
                (None, Some(g)) => {
                    let dy = amount(*g, "--ghost")?;
                    let m = match min {
                        Some(m) => amount(*m, "--min")?,
                        None => (pool::kas_out(x, y, dy, f) * 99 / 100).max(1),
                    };
                    Action::Swap { kas: None, ghost: Some(dy), min: m }
                }
                _ => return Err("Tauschen: entweder --kas oder --ghost angeben".into()),
            }
        }
        "pool-add" => {
            let m = min_shares.unwrap_or(1);
            if m < 1 {
                return Err("--min-shares muss mindestens 1 sein".into());
            }
            Action::PoolAdd { kas: need_units(*kas, "--kas")?, ghost: need_units(*ghost, "--ghost")?, min_shares: m }
        }
        "pool-remove" => {
            let pct = percent.unwrap_or(100.0);
            if !(pct > 0.0 && pct <= 100.0) {
                return Err("--percent muss zwischen 0 und 100 liegen".into());
            }
            let have: i64 = pool::own(&d.lp_tokens, owner, 2).iter().map(|t| t.state.amount).sum();
            if have == 0 {
                return Err("Diese Adresse hat keine Pool-Anteile (laut Zustand).".into());
            }
            let shares = ((have as f64 * pct / 100.0).floor() as i64).clamp(1, have);
            let opt = |v: &Option<f64>, n: &str| -> Result<i64, String> {
                match v {
                    Some(x) if *x > 0.0 => amount(*x, n),
                    _ => Ok(0),
                }
            };
            Action::PoolRemove { shares, min_kas: opt(min_kas, "--min-kas")?, min_ghost: opt(min_ghost, "--min-ghost")? }
        }
        a => return Err(format!("unbekannte Aktion „{a}“ (open-vault, mint, repay, deposit, withdraw, close, redeem, liquidate, sweep, send, transfer, swap, pool-add, pool-remove)")),
    })
}

/// Eigene UTXOs des Plans müssen am Node unverändert vorhanden sein
async fn check_funding_at_node(net: &Net, plan: &kaspa_lending_protocol::wallet_ops::ActionPlan) -> Result<(), String> {
    let funding = kaspa_lending_protocol::wallet_ops::plan_funding(plan, net.prefix)?;
    let addr = kaspa_addresses::Address::try_from(plan.address.as_str()).map_err(|_| "Plan: ungültige Adresse".to_string())?;
    let have = net.utxos(&addr).await?;
    for (op, e) in &funding {
        let ok = have.iter().any(|(o, x)| o == op && x.amount == e.amount && x.script_public_key == e.script_public_key && x.covenant_id.is_none());
        if !ok {
            return Err(format!("Eigene UTXO {op} ist im Netz nicht (mehr) so vorhanden – bitte neu bauen"));
        }
    }
    Ok(())
}

/// Wallet-Senden (Audit 17 A17-4/A17-6): so lange wartet submit --send auf
/// die Sperre, und so lange danach OHNE Sperre auf die Bestätigung. Zusammen
/// mit Verbinden und zwei Abgleichen bleibt ein Aufruf unter dem Zeitlimit der
/// Seite (api.ts WALLET_SEND_TIMEOUT_MS, 170 s) und des Webservers (180 s).
const WALLET_LOCK_WAIT: Duration = Duration::from_secs(30);
const WALLET_WAIT_BASE: Duration = Duration::from_secs(20);
const WALLET_WAIT_MAX: Duration = Duration::from_secs(45);
const WALLET_RELOCK_WAIT: Duration = Duration::from_secs(5);

/// ghostctl wallet build | submit: Nutzeraktionen mit Browser-Wallet, ohne Schlüsseldatei
async fn wallet_action_cmd(network: &str, rpc: Option<&str>, state_path: &Path, yes: bool, cmd: &WalletCmd) -> Result<serde_json::Value, String> {
    use kaspa_lending_protocol::wallet::{parse_signed, xonly_of_address};
    use kaspa_lending_protocol::wallet_ops as wo;
    use serde_json::json;
    if let Ok(t) = std::fs::read_to_string(state_path) {
        if let Some(ver) = serde_json::from_str::<serde_json::Value>(&t).ok().as_ref().and_then(old_version) {
            return Err(old_version_error(state_path, ver));
        }
    }
    let sending = matches!(cmd, WalletCmd::Submit { send: true, .. });
    // A17-4: Plan und Signaturen zuerst prüfen – ohne Netz, ohne Sperre. Müll-
    // Signaturen und Pläne für fremde Adressen kosten so weder Node-Verbindung
    // noch Abgleich noch Sperre.
    let submitted = match cmd {
        WalletCmd::Submit { plan, signed, send } => {
            let plan = wo::ActionPlan::decode(&read_text(plan, "Plan")?)?;
            if plan.network != network {
                return Err(format!("Plan gehört zum Netz {}, gewählt ist {network}", plan.network));
            }
            let signed = parse_signed(&read_text(signed, "Wallet-Antwort")?)?;
            let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(network)?);
            let pre = wo::precheck(&plan, &signed, network, prefix)?;
            if let Some(e) = &pre.error {
                say!("Signatur UNGÜLTIG (Vorprüfung ohne Netz): {e}");
                if *send {
                    return Err(format!("Signatur ungültig – nicht gesendet ({e})"));
                }
                return Ok(json!({ "ok": true, "action": plan.action.name(), "valid": false, "report": pre, "info": null, "sent": false }));
            }
            Some((plan, signed))
        }
        _ => None,
    };
    // Tresor-Aktionen: Zustand, Sperre und Journal an der Tresor-Datei
    let tresor_action = match (cmd, &submitted) {
        (WalletCmd::Build { action, .. }, _) => action.starts_with("tresor-"),
        (_, Some((plan, _))) => plan.action.is_tresor(),
        _ => false,
    };
    let state_path = if tresor_action { tresor::path_for(state_path) } else { state_path.to_path_buf() };
    let net = Net::connect(network, rpc).await?;
    let prefix = net.prefix;
    let ctx = Ctx { mainnet: network == "mainnet", yes, dry_run: !sending, network: network.into(), net, state_path };
    let p = ctx.net.params.clone();
    match cmd {
        WalletCmd::Build { address, .. } => {
            if tresor_action {
                return wallet_tresor_build(&ctx, cmd).await;
            }
            let owner = xonly_of_address(address, prefix)?;
            let d = load_readonly(&ctx).await?;
            let a = wallet_action_of(&d, prefix, &owner, cmd)?;
            let addr = kaspa_addresses::Address::try_from(address.trim()).map_err(|_| "keine gültige Kaspa-Adresse".to_string())?;
            let utxos = ctx.net.utxos(&addr).await?;
            let (plan, info) = wo::build_plan(&d, &a, address, network, &utxos, prefix, &p)?;
            say!("Unsignierte Tx ({}) für {}: Gebühr {:.8} KAS, {} Eingänge ({} signiert die Wallet), {} Ausgänge – nichts gesendet", a.name(), plan.address, plan.fee as f64 / 1e8, plan.tx.inputs.len(), plan.signers.len(), plan.tx.outputs.len());
            Ok(json!({
                "ok": true,
                "action": a.name(),
                "network": network,
                "address": plan.address,
                "feeSompi": plan.fee,
                "outputs": plan.describe_outputs_in(Some(&d), prefix),
                "signInputs": plan.signers,
                "kastle": plan.kastle(),
                "kasware": plan.kasware(),
                "info": info,
                "plan": plan,
            }))
        }
        WalletCmd::Submit { send, .. } => {
            let (plan, signed) = submitted.ok_or("wallet submit ohne Plan")?;
            if tresor_action {
                return wallet_tresor_submit(&ctx, plan, signed, *send).await;
            }
            // Erst ohne Sperre: eigene UTXOs am Node, Neubau aus dem gelesenen
            // Stand, Wallet-Antwort einsetzen und wie der Konsens prüfen
            // Audit 18 G-3: zuerst die billige Prüfung am Node (eigene UTXOs noch
            // da?), erst dann der teure Abgleich – ein veralteter Plan endet hier
            check_funding_at_node(&ctx.net, &plan).await?;
            let d = load_readonly(&ctx).await?;
            let r = wo::submit(&d, &plan, &signed, network, prefix, &p)?;
            let rep = &r.report;
            say!("Signatur {}", if rep.valid { "GÜLTIG – die Tx besteht die lokale Prüfung" } else { "UNGÜLTIG" });
            for i in &rep.inputs {
                say!("  Eingang {} ({}): {}", i.index, i.kind, i.note);
            }
            if let Some(e) = &rep.error {
                say!("  Grund: {e}");
            }
            let mut out = json!({ "ok": true, "action": plan.action.name(), "valid": rep.valid, "report": rep, "info": r.info, "sent": false });
            if let Some(b) = &r.built {
                out["txid"] = b.tx.id().to_string().into();
                out["feeSompi"] = b.fee.into();
            }
            if *send {
                if r.built.is_none() {
                    return Err(format!("Signatur ungültig – nicht gesendet ({})", rep.error.clone().unwrap_or_default()));
                }
                // Senden: erst jetzt die Sperre (kurz), Journal-Abgleich und Neubau
                // unter Sperre wie jede andere Aktion; gewartet wird ohne Sperre
                let lock = store::lock(&ctx.state_path, WALLET_LOCK_WAIT)?;
                let d = ctx.load_synced().await?;
                check_funding_at_node(&ctx.net, &plan).await?;
                let r = wo::submit(&d, &plan, &signed, network, prefix, &p)?;
                let (Some(b), Some(next)) = (r.built.as_ref(), r.next.as_ref()) else {
                    return Err(format!("Signatur ungültig – nicht gesendet ({})", r.report.error.clone().unwrap_or_default()));
                };
                let confirmed = ctx.send_wallet(lock, plan.action.label(), b, next).await?;
                out["txid"] = b.tx.id().to_string().into();
                out["sent"] = true.into();
                out["confirmed"] = confirmed.into();
                if !confirmed {
                    out["pending"] = true.into();
                    out["note"] = "Gesendet; die Bestätigung steht noch aus. Bitte den Status prüfen und NICHT erneut senden.".into();
                }
                out["transactions"] = serde_json::Value::Array(std::mem::take(&mut *TXS.lock().unwrap()));
            }
            Ok(out)
        }
        _ => Err("wallet build | submit erwartet".into()),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Intervall + erster Tag + Uhrzeit (UTC) → (anchorDay, periodMs, erster Termin)
fn tresor_schedule(interval: &str, start: chrono::NaiveDate, time: Option<&str>) -> Result<(i64, i64, i64), String> {
    use chrono::Datelike;
    let t = match time {
        Some(s) => chrono::NaiveTime::parse_from_str(s.trim(), "%H:%M").map_err(|_| format!("--time: Uhrzeit HH:MM (UTC) erwartet, nicht „{s}“"))?,
        None => chrono::NaiveTime::MIN,
    };
    let first = start.and_time(t).and_utc().timestamp_millis();
    let day = kaspa_lending_protocol::standing::DAY_MS;
    let (anchor, period) = match abo::Interval::parse(interval)? {
        abo::Interval::Named(abo::Every::Monthly) => (start.day() as i64, 0),
        abo::Interval::Named(abo::Every::Weekly) => (0, 7 * day),
        abo::Interval::Named(abo::Every::Daily) => (0, day),
        abo::Interval::Days { days } => (0, days as i64 * day),
    };
    Ok((anchor, period, first))
}

fn tresor_interval_text(p: &TresorParams) -> String {
    tresor::interval_text(p)
}

fn xonly_address(prefix: kaspa_addresses::Prefix, x: &[u8]) -> String {
    kaspa_addresses::Address::new(prefix, kaspa_addresses::Version::PubKey, x).to_string()
}

fn tresor_json(network: &str, r: &TresorRec) -> serde_json::Value {
    tresor_json_with(network, r, None)
}

/// Wie `tresor_json`; `sk` = Schlüssel des Empfängers (`tresor list --key`),
/// damit wird eine verschlüsselte Nachricht gegen die Beschreibung geprüft
fn tresor_json_with(network: &str, r: &TresorRec, sk: Option<&SecretKey>) -> serde_json::Value {
    let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(network).expect("Netz geprüft"));
    let p = &r.params;
    let s = &r.utxo.state;
    let (shown, cleaned) = r.shown_message();
    serde_json::json!({
        "id": r.id,
        "covenantId": r.utxo.cov.to_string(),
        "owner": faster_hex::hex_string(&p.owner),
        "recipient": faster_hex::hex_string(&p.recipient),
        "ownerAddress": xonly_address(prefix, &p.owner),
        "recipientAddress": xonly_address(prefix, &p.recipient),
        "amount": abo::fmt_amount(p.amount as u64),
        "maxFee": abo::fmt_amount(p.max_fee as u64),
        "anchorDay": p.anchor_day,
        "periodMs": p.period_ms,
        "interval": tresor_interval_text(p),
        "nextDue": s.next_due,
        "nextDueText": tresor::fmt_time(s.next_due),
        "left": s.left,
        "value": abo::fmt_amount(r.utxo.value),
        "covered": tresor::payments_covered(p, r.utxo.value),
        "outpoint": tresor::outpoint_text(&r.utxo.outpoint),
        // bereinigt: ältere Tresore dürfen heute verbotene Zeichen in der
        // Beschreibung haben; gespeichert und im Code bleibt sie unverändert
        "message": shown,
        "messageCleaned": cleaned,
        "onchain": r.onchain,
        "encrypted": !r.onchain && !r.sealed.is_empty(),
        // none | bound | checked | unchecked | mismatch (A13-tresor-1): nur bei
        // bound und checked trägt jede Zahlung nachweislich diese Beschreibung
        "messageCheck": r.message_check(sk),
        "key": r.key,
        "created": r.created,
        "ended": r.ended,
        "missing": r.missing,
        "lastError": r.last_error,
        "due": r.active() && s.left != 0 && s.next_due + tresor::PMT_LAG_MS <= now_ms(),
        "history": r.history,
        "code": TresorCode::of(network, r).encode(),
    })
}

/// Tresor für die öffentliche Seite („Meine Tresore“, `tresor owned`): wie
/// `tresor_json`, aber ohne Pfad der Schlüsseldatei und ohne Fehlertexte des
/// Agenten; dazu, ob er über die Wallet angelegt wurde und ob der Tresor die
/// Netzgebühr der nächsten Zahlung selbst trägt
fn tresor_public_json(network: &str, r: &TresorRec) -> serde_json::Value {
    let mut v = tresor_json(network, r);
    if let Some(o) = v.as_object_mut() {
        o.remove("key");
        o.remove("lastError");
        o.insert("wallet".into(), r.wallet.into());
        o.insert("feeFromTresor".into(), tresor::fee_from_tresor(&r.params, r.utxo.value).into());
        o.insert("valueSompi".into(), r.utxo.value.into());
    }
    v
}

/// Schlüsseldatei in `dir`, deren x-only-Pubkey `x` ist (nur der Schlüssel,
/// nichts wird ausgegeben); None, wenn keine passt
fn recipient_key(dir: &Path, x: &[u8]) -> Option<SecretKey> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    files.iter().find_map(|f| {
        let kf: KeyFile = serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()?;
        let k = key_from_hex(&kf.secret).ok()?;
        (xonly(&k).as_slice() == x).then(|| SecretKey::from_keypair(&k))
    })
}

/// Offenes Journal einer Wallet-Tx an der Tresor-Datei `path`: Tx-ID,
/// Aktion und die Tresor-Datei, die bei Annahme gilt (nur lesend, A19-6)
fn pending_wallet_tresore(path: &Path) -> Option<(String, String, tresor::TresorFile)> {
    let p: store::Pending = serde_json::from_str(&std::fs::read_to_string(store::pending_path(path)).ok()?).ok()?;
    if !p.wallet || p.target.as_deref() != Some(path) {
        return None;
    }
    let next: tresor::TresorFile = serde_json::from_value(p.next?).ok()?;
    Some((p.txid, p.action, next))
}

/// Tresor-Befehle ohne Netz. Ok(None) = braucht das Netz.
fn tresor_offline(network: &str, path: &Path, cmd: &TresorCmd, dry_run: bool) -> Result<Option<serde_json::Value>, String> {
    use serde_json::json;
    match cmd {
        TresorCmd::List { key } => {
            let f = tresor::load(path, network)?;
            let sk = key.as_deref().map(load_key).transpose()?.map(|k| SecretKey::from_keypair(&k));
            for r in &f.tresore {
                let s = &r.utxo.state;
                say!(
                    "{}  {} KAS {} → {}…  Guthaben {} KAS, nächster Termin {}, {}{}",
                    r.id,
                    abo::fmt_amount(r.params.amount as u64),
                    tresor_interval_text(&r.params),
                    &faster_hex::hex_string(&r.params.recipient)[..12],
                    abo::fmt_amount(r.utxo.value),
                    tresor::fmt_time(s.next_due),
                    match s.left {
                        -1 => "unbegrenzt".to_string(),
                        0 => "alle Zahlungen erledigt".to_string(),
                        n => format!("noch {n} Zahlung(en)"),
                    },
                    if r.ended.is_some() { "  [gekündigt]" } else if r.missing.is_some() { "  [nicht auffindbar]" } else { "" }
                );
            }
            if f.tresore.is_empty() {
                say!("Keine Tresore in {}", path.display());
            }
            for r in &f.tresore {
                match r.message_check(sk.as_ref()) {
                    tresor::MessageCheck::Unchecked => say!("{}: Beschreibung laut Tresor-Code, nicht geprüft (verschlüsselt; prüfbar mit dem Schlüssel des Empfängers)", r.id),
                    tresor::MessageCheck::Mismatch => say!("{}: Achtung – die Zahlungen tragen nicht die Beschreibung „{}“", r.id, r.message),
                    _ => {}
                }
            }
            Ok(Some(json!({ "ok": true, "network": network, "tresore": f.tresore.iter().map(|r| tresor_json_with(network, r, sk.as_ref())).collect::<Vec<_>>() })))
        }
        TresorCmd::Code { id } => {
            let f = tresor::load(path, network)?;
            let r = &f.tresore[f.find(id)?];
            let code = TresorCode::of(network, r).encode();
            say!("{code}");
            Ok(Some(json!({ "ok": true, "network": network, "id": r.id, "code": code })))
        }
        TresorCmd::Owned { owner } => {
            let prefix = kaspa_addresses::Prefix::from(kaspa_lending_protocol::net::network_id(network)?);
            let x = kaspa_lending_protocol::wallet_ops::ghost_target(prefix, owner).map_err(|e| e.replace("Empfänger", "Besitzer"))?;
            let f = tresor::load(path, network)?;
            // nur über die Wallet angelegte: Tresore des Betreibers (Schlüsseldatei) bleiben privat
            let mut list: Vec<_> = f.of_owner(&x).filter(|r| r.wallet).map(|r| tresor_public_json(network, r)).collect();
            // A19-6: offenes Journal einer Wallet-Tx (gesendet, noch nicht
            // übernommen) mit anzeigen – sonst fehlt ein neuer Tresor in
            // „Meine Tresore“, bis der nächste Abgleich das Journal klärt
            if let Some((txid, action, next)) = pending_wallet_tresore(path) {
                let mark = |v: &mut serde_json::Value| v["pending"] = json!({ "txid": txid, "action": action });
                for (v, r) in list.iter_mut().zip(f.of_owner(&x).filter(|r| r.wallet)) {
                    if next.tresore.iter().any(|n| n.utxo.cov == r.utxo.cov && (n.utxo.outpoint != r.utxo.outpoint || n.ended != r.ended)) {
                        mark(v);
                    }
                }
                for n in next.of_owner(&x).filter(|n| n.wallet && !f.tresore.iter().any(|r| r.utxo.cov == n.utxo.cov)) {
                    let mut v = tresor_public_json(network, n);
                    mark(&mut v);
                    list.push(v);
                }
            }
            say!("{} Tresor(e) von {}", list.len(), xonly_address(prefix, &x));
            Ok(Some(json!({ "ok": true, "network": network, "owner": xonly_address(prefix, &x), "tresore": list })))
        }
        TresorCmd::Pay { id: None, key } => {
            // ohne Fälliges keine Verbindung zum Node (Seite und Agent fragen jede Minute)
            if !tresor::needs_run(&tresor::load(path, network)?, now_ms(), key.is_some()) {
                say!("Keine fälligen Tresor-Zahlungen.");
                return Ok(Some(json!({ "ok": true, "network": network, "dryRun": dry_run, "reports": [], "transactions": [] })));
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Ein- und Ausgabe der Tresor-Abläufe (tresor::follow, import, pay_round):
/// Node, Journal und Terminal. Die Abläufe selbst stehen in der Bibliothek und
/// laufen in den Tests gegen den Simulator.
struct TresorNet<'a> {
    ctx: &'a Ctx,
    key: Option<Keypair>,
}

impl tresor::TresorIo for TresorNet<'_> {
    async fn utxos(&mut self, spk: &kaspa_consensus_core::tx::ScriptPublicKey) -> Result<Vec<(kaspa_consensus_core::tx::TransactionOutpoint, kaspa_consensus_core::tx::UtxoEntry)>, String> {
        let addr = self.ctx.net.address_of_spk(spk)?;
        self.ctx.net.utxos(&addr).await
    }
    async fn funds(&mut self) -> Option<Funds> {
        match &self.key {
            Some(k) => self.ctx.funds(k).await.ok(),
            None => None,
        }
    }
    async fn send(&mut self, what: &str, b: &Built, next: &tresor::TresorFile) -> Result<(), String> {
        self.ctx.send_to(what, b, Some((self.ctx.state_path.clone(), serde_json::to_value(next).unwrap()))).await
    }
    fn save(&mut self, f: &tresor::TresorFile) -> Result<(), String> {
        if self.ctx.dry_run { Ok(()) } else { tresor::save(&self.ctx.state_path, f) }
    }
    fn journal_open(&self) -> bool {
        store::pending_path(&self.ctx.state_path).exists()
    }
    fn dry_run(&self) -> bool {
        self.ctx.dry_run
    }
    fn now_ms(&self) -> i64 {
        now_ms()
    }
    fn say(&mut self, line: &str) {
        say!("{line}");
    }
}

/// `tresor pay`: Der angegebene Schlüssel geht an die Ein-/Ausgabe (`io`
/// baut sie damit; sie zahlt mit ihm die Gebühr) und als `with_key` an die
/// Runde – beides aus derselben Angabe. Ohne Schlüssel löst die Runde also nur
/// Tresore aus, die die Gebühr selbst tragen, wie `payParams` auf der Seite
/// (A12-7; Restpunkt: die Übergabe war ungetestet).
async fn tresor_pay<Io: tresor::TresorIo>(
    io: impl FnOnce(Option<Keypair>) -> Io,
    key: Option<Keypair>,
    file: &mut tresor::TresorFile,
    id: Option<&str>,
    pmt: i64,
    now: &str,
    params: &kaspa_consensus_core::config::params::Params,
) -> Result<Vec<tresor::TresorReport>, String> {
    let with_key = key.is_some();
    tresor::pay_round(&mut io(key), file, id, with_key, pmt, now, params).await
}

/// Tresor-Befehle mit Netz. Sperre und Journal liegen an der Tresor-Datei (ctx.state_path).
async fn tresor_cmd(ctx: &Ctx, cmd: TresorCmd, extra: &mut serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let path = ctx.state_path.clone();
    if let Some(msg) = store::resolve_pending(&ctx.net, &path).await? {
        say!("Hinweis: {msg}");
    }
    let (now, _, _) = abo_now();
    let mut file = tresor::load(&path, &ctx.network)?;
    let pmt = ctx.net.past_median_time().await?;
    let params = ctx.net.params.clone();
    let target = |f: &tresor::TresorFile| Some((path.clone(), serde_json::to_value(f).unwrap()));
    let save = |f: &tresor::TresorFile| if ctx.dry_run { Ok(()) } else { tresor::save(&path, f) };
    match cmd {
        TresorCmd::List { .. } | TresorCmd::Code { .. } | TresorCmd::Owned { .. } => unreachable!(),
        TresorCmd::Open { key, to, amount, interval, start, time, count, fund, max_fee, message, onchain_message } => {
            let k = load_key(&key)?;
            let recipient = ghost_target(ctx.net.prefix, &ctx.network, to.trim()).map_err(|e| e.replace("GHOST gehen", "Tresor-Zahlungen gehen"))?;
            let amount = abo::parse_amount(&amount)? as i64;
            let max_fee = match &max_fee {
                Some(s) => abo::parse_amount(s).map_err(|e| format!("--max-fee: {e}"))? as i64,
                None => tresor::DEFAULT_MAX_FEE,
            };
            let start = match &start {
                Some(s) => parse_date(s, "--start")?,
                None => chrono::Utc::now().date_naive(),
            };
            let (anchor_day, period_ms, first) = tresor_schedule(&interval, start, time.as_deref())?;
            tresor::check_first_due(first, pmt)?;
            let left = match count {
                Some(0) => return Err("--count: mindestens 1".into()),
                Some(c) => c as i64,
                None => -1,
            };
            // öffentlich: Klartext in jeder Zahlung; sonst einmal an den Empfänger
            // verschlüsselt, dieselbe Fassung kommt in jede Zahlung – der
            // Vertrag bindet sie über ihren sha256 (payloadHash)
            let (message, payload) = message_args(message.as_deref(), onchain_message, Some(recipient.as_slice()))?;
            let mut p = TresorParams { owner: xonly(&k), recipient, amount, anchor_day, period_ms, max_fee, payload_hash: vec![] };
            let sealed = tresor::bind_message(&mut p, &message, onchain_message, &payload)?;
            let s0 = TresorState { next_due: first, left };
            tresor::check_params(&p, &s0)?;
            let fund = match &fund {
                Some(f) => abo::parse_amount(f).map_err(|e| format!("--fund: {e}"))?,
                None => tresor::suggested_fund(&p, left).ok_or("Unbegrenzter Tresor: Startguthaben mit --fund angeben")? as u64,
            };
            let (b, t) = tresor::open(&p, &s0, fund, &ctx.funds(&k).await?, &params)?;
            let mut rec = TresorRec::new(p, t, message, onchain_message, Some(key.to_string_lossy().to_string()), &now);
            rec.sealed = sealed;
            rec.push(tresor::TresorHist { at: now.clone(), action: "open".into(), txid: Some(b.tx.id().to_string()), due: None, note: None });
            let mut next = file.clone();
            next.upsert(rec.clone());
            say!(
                "Tresor {}: {} KAS {} ab {}, {} – Startguthaben {} KAS (reicht für {} Zahlung(en))",
                rec.id,
                abo::fmt_amount(amount as u64),
                tresor_interval_text(&rec.params),
                tresor::fmt_time(first),
                if left < 0 { "unbegrenzt".to_string() } else { format!("{left} Zahlung(en)") },
                abo::fmt_amount(fund),
                tresor::payments_covered(&rec.params, fund)
            );
            extra.insert("tresor".into(), tresor_json(&ctx.network, &rec));
            ctx.send_to("Tresor anlegen", &b, target(&next)).await?;
            if !ctx.dry_run {
                say!("Tresor-Code für den Empfänger (ghostctl tresor code {}):\n{}", rec.id, TresorCode::of(&ctx.network, &rec).encode());
            }
        }
        TresorCmd::Import { code, key } => {
            // Schlüssel des Empfängers: --key, sonst die passende Datei in keys/
            let sk = match &key {
                Some(k) => Some(SecretKey::from_keypair(&load_key(k)?)),
                None => TresorCode::decode(&code).ok().and_then(|c| recipient_key(Path::new("keys"), &c.params.recipient)),
            };
            let i = tresor::import(&mut TresorNet { ctx, key: None }, &mut file, &code, &ctx.network, pmt, &now, sk.as_ref()).await?;
            let r = &file.tresore[i];
            match r.message_check(None) {
                tresor::MessageCheck::Checked => say!("Beschreibung mit dem Schlüssel des Empfängers geprüft: jede Zahlung trägt „{}“", r.message),
                tresor::MessageCheck::Unchecked => say!(
                    "Hinweis: Beschreibung laut Tresor-Code, nicht geprüft – die Nachricht ist verschlüsselt und der Schlüssel des Empfängers liegt hier nicht vor (--key). Was die Zahlungen tragen, zeigt der Eingang."
                ),
                _ => {}
            }
            say!(
                "Tresor {} übernommen: {} KAS {} an {}…, Guthaben {} KAS, nächster Termin {}",
                r.id,
                abo::fmt_amount(r.params.amount as u64),
                tresor_interval_text(&r.params),
                &faster_hex::hex_string(&r.params.recipient)[..12],
                abo::fmt_amount(r.utxo.value),
                tresor::fmt_time(r.utxo.state.next_due)
            );
            extra.insert("tresor".into(), tresor_json(&ctx.network, r));
            save(&file)?;
        }
        TresorCmd::Sync => {
            let mut notes = vec![];
            let mut io = TresorNet { ctx, key: None };
            for r in file.tresore.iter_mut() {
                match tresor::follow(&mut io, r, pmt, &now, tresor::Search::Full).await {
                    Ok(Some(n)) => notes.push(n),
                    Ok(None) => {}
                    Err(e) => notes.push(format!("Tresor {}: {e}", r.id)),
                }
            }
            for n in &notes {
                say!("{n}");
            }
            say!("Abgleich fertig: {} Tresor(e)", file.tresore.len());
            extra.insert("notes".into(), notes.into());
            extra.insert("tresore".into(), file.tresore.iter().map(|r| tresor_json(&ctx.network, r)).collect::<Vec<_>>().into());
            save(&file)?;
        }
        TresorCmd::Pay { id, key } => {
            let k = key.as_deref().map(load_key).transpose()?;
            // Auswahl, Nachführen, Fehlerpfad: tresor::pay_round (getestet gegen den Simulator)
            let reports = tresor_pay(|key| TresorNet { ctx, key }, k, &mut file, id.as_deref(), pmt, &now, &params).await?;
            extra.insert("reports".into(), serde_json::to_value(&reports).unwrap());
        }
        TresorCmd::Topup { id, key, kas } => {
            let k = load_key(&key)?;
            let i = file.find(&id)?;
            if let Some(n) = tresor::follow(&mut TresorNet { ctx, key: None }, &mut file.tresore[i], pmt, &now, tresor::Search::Full).await? {
                say!("{n}");
            }
            let r = file.tresore[i].clone();
            if !r.active() {
                save(&file)?;
                return Err(format!("Tresor {} ist gekündigt oder nicht auffindbar", r.id));
            }
            let add = abo::parse_amount(&kas).map_err(|e| format!("--kas: {e}"))?;
            let (b, t) = tresor::topup(&r.params, &r.utxo, &k, add, &ctx.funds(&k).await?, &params)?;
            let mut next = file.clone();
            next.tresore[i].utxo = t.clone();
            next.tresore[i].retry_after = None;
            next.tresore[i].push(tresor::TresorHist { at: now.clone(), action: "topup".into(), txid: Some(b.tx.id().to_string()), due: None, note: None });
            say!("Tresor {}: {} KAS nachlegen, danach {} KAS (reicht für {} Zahlung(en))", r.id, abo::fmt_amount(add), abo::fmt_amount(t.value), tresor::payments_covered(&r.params, t.value));
            extra.insert("tresor".into(), tresor_json(&ctx.network, &next.tresore[i]));
            save(&file)?;
            ctx.send_to(&format!("Tresor {} auffüllen", r.id), &b, target(&next)).await?;
        }
        TresorCmd::Cancel { id, key } => {
            let k = load_key(&key)?;
            let i = file.find(&id)?;
            if let Some(n) = tresor::follow(&mut TresorNet { ctx, key: None }, &mut file.tresore[i], pmt, &now, tresor::Search::Full).await? {
                say!("{n}");
            }
            let r = file.tresore[i].clone();
            if !r.active() {
                save(&file)?;
                return Err(format!("Tresor {} ist schon gekündigt oder nicht auffindbar", r.id));
            }
            let b = tresor::cancel(&r.params, &r.utxo, &k, &params)?;
            let back = b.tx.outputs.iter().map(|o| o.value).sum::<u64>();
            let mut next = file.clone();
            next.tresore[i].ended = Some(now.clone());
            next.tresore[i].push(tresor::TresorHist { at: now.clone(), action: "cancel".into(), txid: Some(b.tx.id().to_string()), due: None, note: None });
            say!("Tresor {} kündigen: {} KAS zurück an den Absender", r.id, abo::fmt_amount(back));
            extra.insert("back".into(), abo::fmt_amount(back).into());
            save(&file)?;
            ctx.send_to(&format!("Tresor {} kündigen", r.id), &b, target(&next)).await?;
        }
    }
    Ok(())
}

/// Kommandozeile → Tresor-Aktion der Browser-Wallet (Termine wie `tresor open`)
fn wallet_tresor_action_of(prefix: kaspa_addresses::Prefix, pmt: i64, cmd: &WalletCmd) -> Result<kaspa_lending_protocol::wallet_ops::Action, String> {
    use kaspa_lending_protocol::wallet_ops::Action;
    let WalletCmd::Build { action, kas, to, message, onchain_message, amount: per, interval, start, time, count, fund, max_fee, tresor: id, .. } = cmd else {
        return Err("kein build".into());
    };
    let full_id = || -> Result<String, String> {
        let t = id.as_deref().ok_or("--tresor fehlt (volle Covenant-ID)")?.trim().to_lowercase();
        if t.len() != 64 || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("--tresor: volle Covenant-ID (64 Hex-Zeichen) erwartet".into());
        }
        Ok(t)
    };
    Ok(match action.as_str() {
        "tresor-open" => {
            let t = to.as_deref().ok_or("--to fehlt")?.trim().to_string();
            // nur Adressen bzw. x-only-Schlüssel, nie Schlüsseldateien
            let recipient = kaspa_lending_protocol::wallet_ops::ghost_target(prefix, &t)?;
            let amount = need_units(*per, "--amount")?;
            let max_fee = match max_fee {
                Some(m) => amount_units(*m, "--max-fee")?,
                None => tresor::DEFAULT_MAX_FEE,
            };
            let start = match start {
                Some(s) => parse_date(s, "--start")?,
                None => chrono::Utc::now().date_naive(),
            };
            let (anchor_day, period_ms, first) = tresor_schedule(interval.as_deref().ok_or("--interval fehlt")?, start, time.as_deref())?;
            // früh und verständlich; verbindlich prüft wallet_ops::run_tresor,
            // auch beim Neubau in submit (A19-1)
            tresor::check_wallet_first_due(first, pmt)?;
            let left = match count {
                Some(0) => return Err("--count: mindestens 1".into()),
                Some(c) => *c as i64,
                None => -1,
            };
            // Wallet: nur öffentliche Nachricht oder keine (wallet_ops, Modulkommentar)
            let m = message.as_deref().unwrap_or("").trim().to_string();
            if !m.is_empty() && !*onchain_message {
                return Err("Mit der Browser-Wallet nur öffentliche Nachricht (--onchain-message) oder keine".into());
            }
            let (m, _) = message_args(Some(&m), *onchain_message, Some(recipient.as_slice()))?;
            let probe = TresorParams { owner: vec![], recipient, amount, anchor_day, period_ms, max_fee, payload_hash: vec![] };
            let fund = match fund {
                Some(f) => amount_units(*f, "--fund")? as u64,
                None => tresor::suggested_fund(&probe, left).ok_or("Unbegrenzter Tresor: Startguthaben mit --fund angeben")? as u64,
            };
            Action::TresorOpen { to: t, amount, anchor_day, period_ms, first_due: first, count: left, fund, max_fee, message: m }
        }
        "tresor-topup" => Action::TresorTopup { tresor: full_id()?, kas: need_units(*kas, "--kas")? as u64 },
        "tresor-cancel" => Action::TresorCancel { tresor: full_id()? },
        a => return Err(format!("unbekannte Tresor-Aktion „{a}“ (tresor-open, tresor-topup, tresor-cancel)")),
    })
}

/// Betrag in Einheiten (Pflichtwert schon vorhanden)
fn amount_units(v: f64, name: &str) -> Result<i64, String> {
    amount(v, name)
}

/// Tresor-Datei lesen und – für Auffüllen und Kündigen – den betroffenen
/// Tresor am Node nachführen, erst nach der Besitzerprüfung und begrenzt
/// (wallet_ops::follow_for_wallet, A19-2; `owner` = x-only der Adresse bzw.
/// des Plans). Schreibt nichts; den Folgezustand schreibt nur das Journal bei
/// Annahme (Ctx::send_wallet).
async fn tresor_basis(ctx: &Ctx, a: &kaspa_lending_protocol::wallet_ops::Action, owner: &[u8], pmt: i64) -> Result<kaspa_lending_protocol::wallet_ops::TresorBasis, String> {
    let (now, _, _) = abo_now();
    let mut file = tresor::load(&ctx.state_path, &ctx.network)?;
    if let Some(n) = kaspa_lending_protocol::wallet_ops::follow_for_wallet(&mut TresorNet { ctx, key: None }, &mut file, a, owner, pmt, &now).await? {
        say!("{n}");
    }
    Ok(kaspa_lending_protocol::wallet_ops::TresorBasis { file, now, pmt })
}

/// `wallet build tresor-…`: Plan aus der Tresor-Datei (liest nur)
async fn wallet_tresor_build(ctx: &Ctx, cmd: &WalletCmd) -> Result<serde_json::Value, String> {
    use kaspa_lending_protocol::wallet_ops as wo;
    let WalletCmd::Build { address, .. } = cmd else { return Err("kein build".into()) };
    let prefix = ctx.net.prefix;
    let owner = kaspa_lending_protocol::wallet::xonly_of_address(address, prefix)?;
    let pmt = ctx.net.past_median_time().await?;
    let a = wallet_tresor_action_of(prefix, pmt, cmd)?;
    let basis = tresor_basis(ctx, &a, &owner, pmt).await?;
    let addr = kaspa_addresses::Address::try_from(address.trim()).map_err(|_| "keine gültige Kaspa-Adresse".to_string())?;
    let utxos = if a.needs_funding() { ctx.net.utxos(&addr).await? } else { vec![] };
    let (plan, info) = wo::build_plan(&basis, &a, address, &ctx.network, &utxos, prefix, &ctx.net.params)?;
    say!("Unsignierte Tx ({}) für {}: Gebühr {:.8} KAS, {} Eingänge ({} signiert die Wallet), {} Ausgänge – nichts gesendet", a.name(), plan.address, plan.fee as f64 / 1e8, plan.tx.inputs.len(), plan.signers.len(), plan.tx.outputs.len());
    Ok(serde_json::json!({
        "ok": true,
        "action": a.name(),
        "network": ctx.network,
        "address": plan.address,
        "feeSompi": plan.fee,
        "outputs": plan.describe_tresor_outputs(Some(&basis.file), prefix),
        "signInputs": plan.signers,
        "kastle": plan.kastle(),
        "kasware": plan.kasware(),
        "info": info,
        "plan": plan,
    }))
}

/// Meldung, wenn die Sperre der Tresor-Datei belegt ist (Audit 19 A19-7).
/// Meist hält sie der Tresor-Schritt des Agenten, der nach einer Zahlung auf
/// die Bestätigung wartet (höchstens Takt::agent `send` = 90 s nach der ersten
/// Sendung, dann bricht der Agent ab). Die Sperre wie bei A17-6 vor dem
/// Warten freizugeben, hülfe hier nicht: Das Journal der Zahlung bleibt bis
/// zur Bestätigung offen, und `resolve_pending` hielte den Wallet-Submit
/// ebenso auf. Also eine klare Meldung, die die Seite als „gleich erneut
/// versuchen“ erkennt (app/server/walletActions.ts `isRetryLater`): Plan nicht
/// sperren, Signatur behalten. Gesendet wurde zu diesem Zeitpunkt nichts.
const TRESOR_BUSY: &str = "Gerade läuft eine Zahlungsrunde für Tresore. Bitte in ein bis zwei Minuten erneut senden – es wurde nichts gesendet, deine Signatur bleibt gültig.";

fn tresor_lock_busy(e: String) -> String {
    if e.starts_with("Eine andere ghostctl-Instanz arbeitet gerade") { TRESOR_BUSY.into() } else { e }
}

/// `wallet submit` einer Tresor-Aktion: wie bei den GHOST-Aktionen erst ohne
/// Sperre prüfen (Vorprüfung ist schon gelaufen), erst zum Senden die Sperre
/// der Tresor-Datei, Journal klären, neu bauen und vergleichen. Der neue bzw.
/// geänderte Tresor kommt nur über das Journal in die Datei, also erst, wenn
/// die selbst geprüfte Tx angenommen ist.
async fn wallet_tresor_submit(ctx: &Ctx, plan: kaspa_lending_protocol::wallet_ops::ActionPlan, signed: kaspa_lending_protocol::wallet::SafeTx, send: bool) -> Result<serde_json::Value, String> {
    use kaspa_lending_protocol::wallet_ops as wo;
    use serde_json::json;
    let (prefix, p) = (ctx.net.prefix, ctx.net.params.clone());
    check_funding_at_node(&ctx.net, &plan).await?;
    let pmt = ctx.net.past_median_time().await?;
    let basis = tresor_basis(ctx, &plan.action, &plan.owner, pmt).await?;
    let r = wo::submit(&basis, &plan, &signed, &ctx.network, prefix, &p)?;
    let rep = &r.report;
    say!("Signatur {}", if rep.valid { "GÜLTIG – die Tx besteht die lokale Prüfung" } else { "UNGÜLTIG" });
    for i in &rep.inputs {
        say!("  Eingang {} ({}): {}", i.index, i.kind, i.note);
    }
    if let Some(e) = &rep.error {
        say!("  Grund: {e}");
    }
    let mut out = json!({ "ok": true, "action": plan.action.name(), "valid": rep.valid, "report": rep, "info": r.info, "sent": false });
    if let Some(b) = &r.built {
        out["txid"] = b.tx.id().to_string().into();
        out["feeSompi"] = b.fee.into();
    }
    if send {
        if r.built.is_none() {
            return Err(format!("Signatur ungültig – nicht gesendet ({})", rep.error.clone().unwrap_or_default()));
        }
        let lock = store::lock(&ctx.state_path, WALLET_LOCK_WAIT).map_err(tresor_lock_busy)?;
        if let Some(msg) = store::resolve_pending(&ctx.net, &ctx.state_path).await? {
            say!("Hinweis: {msg}");
        }
        check_funding_at_node(&ctx.net, &plan).await?;
        let pmt = ctx.net.past_median_time().await?;
        let basis = tresor_basis(ctx, &plan.action, &plan.owner, pmt).await?;
        let r = wo::submit(&basis, &plan, &signed, &ctx.network, prefix, &p)?;
        let (Some(b), Some(next)) = (r.built.as_ref(), r.next.as_ref()) else {
            return Err(format!("Signatur ungültig – nicht gesendet ({})", r.report.error.clone().unwrap_or_default()));
        };
        let confirmed = ctx.send_wallet(lock, plan.action.label(), b, &next.file).await?;
        out["txid"] = b.tx.id().to_string().into();
        out["sent"] = true.into();
        out["confirmed"] = confirmed.into();
        if !confirmed {
            out["pending"] = true.into();
            out["note"] = "Gesendet; die Bestätigung steht noch aus. Bitte den Status prüfen und NICHT erneut senden.".into();
        }
        out["transactions"] = serde_json::Value::Array(std::mem::take(&mut *TXS.lock().unwrap()));
    }
    Ok(out)
}

/// Agent-Runde für Tresore: eigene Verbindung, eigene Sperre (Tresor-Datei),
/// eigener Fehlerpfad; ohne (offline) Fälliges keine Netzabfrage. Doppelt
/// auslösen (Seite, Empfänger, anderer Agent) ist harmlos: die zweite Tx
/// scheitert an der verbrauchten UTXO.
async fn tresor_agent_step(network: &str, rpc: Option<&str>, state: &Path, key: &Path, yes: bool, dry_run: bool, now: &str) {
    let path = tresor::path_for(state);
    // A19-6: ein offenes Journal (z. B. Wallet-Tx, deren Übernahme ghostctl
    // nicht mehr erlebt hat) klärt der Agent in jeder Runde, auch wenn nichts
    // fällig ist – sonst fehlt ein neuer Tresor in „Meine Tresore“
    let journal = store::pending_path(&path).exists();
    let due = match tresor::load(&path, network) {
        Ok(f) => {
            if let Some(w) = tresor_room_warning(&f, now_ms()) {
                eprintln!("[{now}] Tresore: {w}");
            }
            tresor::needs_run(&f, now_ms(), true)
        }
        Err(e) => return eprintln!("[{now}] Tresore: {e}"),
    };
    if !due && !journal {
        return;
    }
    let step = async {
        let net = match Net::connect(network, rpc).await {
            Ok(n) => n,
            Err(e) => return eprintln!("[{now}] Tresore: keine Verbindung zum Node: {e}"),
        };
        let ctx = Ctx { mainnet: network == "mainnet", yes, dry_run, network: network.into(), net, state_path: path.clone() };
        match store::lock(&ctx.state_path, Duration::from_secs(60)) {
            Err(e) => eprintln!("[{now}] Tresore: {e}"),
            Ok(_lock) if !due => match store::resolve_pending(&ctx.net, &ctx.state_path).await {
                Ok(Some(m)) => eprintln!("[{now}] Tresore: {m}"),
                Ok(None) => {}
                Err(e) => eprintln!("[{now}] Tresore: {e}"),
            },
            Ok(_lock) => {
                let mut extra = serde_json::Map::new();
                if let Err(e) = tresor_cmd(&ctx, TresorCmd::Pay { id: None, key: Some(key.to_path_buf()) }, &mut extra).await {
                    eprintln!("[{now}] Tresore: {e}");
                }
            }
        }
        let _ = ctx.net.client.disconnect().await;
    };
    if tokio::time::timeout(Duration::from_secs(700), step).await.is_err() {
        eprintln!("[{now}] Tresore: Runde nach 700 s abgebrochen, die nächste klärt offene Zahlungen");
    }
}

/// Warnung an den Betreiber, wenn die Tresor-Datei sich ihren Grenzen nähert
/// (A19-3: ab 90 % der belegten Plätze bzw. aller Einträge)
fn tresor_room_warning(f: &tresor::TresorFile, now_ms: i64) -> Option<String> {
    let busy = f.tresore.iter().filter(|r| r.busy(now_ms)).count();
    let (all, wallet) = (f.tresore.len(), f.tresore.iter().filter(|r| r.wallet).count());
    (busy * 10 >= tresor::MAX_FILE_TRESORE * 9 || all * 10 >= tresor::MAX_FILE_ALL * 9).then(|| {
        format!(
            "Tresor-Datei fast voll: {busy} von {} Plätzen belegt, {all} von {} Einträgen ({wallet} über die Browser-Wallet) – bitte prüfen, ob jemand die Plätze absichtlich füllt",
            tresor::MAX_FILE_TRESORE,
            tresor::MAX_FILE_ALL
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::Instant;

    // Die Takt-Tests laufen mit angehaltener Uhr (tokio test-util): Zeitgeber
    // feuern, sobald nichts anderes zu tun ist, und die gemessenen Abstände
    // sind genau, auch unter Last. Ein Nebenschritt wird wie im Agenten in
    // jedem Durchlauf neu erzeugt.

    /// Nebenschritt zum Testen: `prep` für Verbindung, Sperre und Abgleich,
    /// dann eine Sendung, dann `confirm` warten (None: für immer)
    async fn fake_step(sent: &AtomicUsize, prep: Duration, confirm: Option<Duration>) {
        tokio::time::sleep(prep).await;
        sent.fetch_add(1, Ordering::SeqCst);
        match confirm {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending().await,
        }
    }

    /// Abstand auf die Sekunde genau (das Sendelimit sieht sekündlich nach,
    /// Zeitgeber runden auf Millisekunden)
    fn near(d: Duration, secs: u64) -> bool {
        (secs as f64 - 0.5..secs as f64 + 1.0).contains(&d.as_secs_f64())
    }

    /// Audit 12 A12-5: warten Dauerauftrags- und Tresor-Schritt nach einer
    /// Sendung auf die Bestätigung (bis 600 s je Zahlung), beginnt die nächste
    /// Orakel-/Keeper-Runde trotzdem nach 2 · 90 s; beide Schritte kommen in
    /// jedem Durchlauf dran. Beim Standard (--interval 120) verkürzen sie die
    /// Pause auf 0: 180 s statt 300 s (volle Pause) oder 1 400 s (ohne Limit).
    #[tokio::test(start_paused = true)]
    async fn a12_nebenschritte_hungern_orakel_und_keeper_nicht_aus() {
        let t = Takt::agent(120);
        let (rounds, sent) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let start = Instant::now();
        let mut starts = vec![];
        for _ in 0..4 {
            starts.push(start.elapsed());
            let round = async {
                rounds.fetch_add(1, Ordering::SeqCst);
            };
            let abo = fake_step(&sent, Duration::ZERO, None);
            let tresor = fake_step(&sent, Duration::ZERO, None);
            let r = agent_cycle(t, "test", || sent.load(Ordering::SeqCst), round, abo, tresor).await;
            assert_eq!(r, (true, [StepEnd::SendLimit, StepEnd::SendLimit]));
        }
        assert_eq!((rounds.load(Ordering::SeqCst), sent.load(Ordering::SeqCst)), (4, 8), "jeder Schritt sendet in jedem Durchlauf");
        for w in starts.windows(2) {
            assert!(near(w[1] - w[0], 180), "Abstand zweier Runden {:?}", w[1] - w[0]);
        }
    }

    /// Die Pause `interval` verkürzt sich um die Dauer der Nebenschritte.
    /// Startskript (--interval 300): höchstens 540 s zwischen zwei Runden,
    /// auch wenn die Runde selbst ihr Zeitlimit ausschöpft.
    #[tokio::test(start_paused = true)]
    async fn a12_pause_verkuerzt_sich_um_die_nebenschritte() {
        let t = Takt::agent(300);
        let sent = AtomicUsize::new(0);
        let count = || sent.load(Ordering::SeqCst);
        // Schritte senden und warten: 180 s, die Pause füllt auf 300 s auf (voll: 480 s)
        let t0 = Instant::now();
        let r = agent_cycle(t, "test", count, async {}, fake_step(&sent, Duration::ZERO, None), fake_step(&sent, Duration::ZERO, None)).await;
        assert_eq!(r.1, [StepEnd::SendLimit; 2]);
        assert!(near(t0.elapsed(), 300), "{:?}", t0.elapsed());
        // Schritte fertig nach je 50 s: 300 s (voll: 400 s)
        let t0 = Instant::now();
        let fifty = Some(Duration::from_secs(50));
        let r = agent_cycle(t, "test", count, async {}, fake_step(&sent, Duration::ZERO, fifty), fake_step(&sent, Duration::ZERO, fifty)).await;
        assert_eq!(r, (true, [StepEnd::Done; 2]));
        assert!(near(t0.elapsed(), 300), "{:?}", t0.elapsed());
        // die Runde hängt: 240 s, dann Schritte und Rest der Pause, 540 s (voll: 720 s)
        let t0 = Instant::now();
        let r = agent_cycle(t, "test", count, std::future::pending(), fake_step(&sent, Duration::ZERO, None), fake_step(&sent, Duration::ZERO, None)).await;
        assert_eq!(r, (false, [StepEnd::SendLimit; 2]));
        assert!(near(t0.elapsed(), 540) && near(t0.elapsed(), t.usual_gap().as_secs()), "{:?}", t0.elapsed());
    }

    /// Nachprüfung zu A12-5: Verbindung, Sperre und Abgleich vor der ersten
    /// Sendung zählen nicht zum Sendelimit. Ein Tresor-Schritt, der dafür 200 s
    /// braucht, zahlt im selben Durchlauf (mit festem 90-s-Limit nie: er brach
    /// in jedem Durchlauf an derselben Stelle ab).
    #[tokio::test(start_paused = true)]
    async fn a12_langsamer_abgleich_zahlt_trotzdem() {
        let t = Takt::agent(120);
        let sent = AtomicUsize::new(0);
        let count = || sent.load(Ordering::SeqCst);
        let t0 = Instant::now();
        let tresor = fake_step(&sent, Duration::from_secs(200), Some(Duration::from_secs(5)));
        let r = agent_cycle(t, "test", count, async {}, async {}, tresor).await;
        assert_eq!(r, (true, [StepEnd::Done; 2]));
        assert_eq!(sent.load(Ordering::SeqCst), 1, "gesendet");
        assert!(near(t0.elapsed(), 205), "{:?}", t0.elapsed());
        // wartet er danach auf die Bestätigung, zählen ab der Sendung 90 s
        let t0 = Instant::now();
        let tresor = fake_step(&sent, Duration::from_secs(200), None);
        let r = agent_cycle(t, "test", count, async {}, async {}, tresor).await;
        assert_eq!(r, (true, [StepEnd::Done, StepEnd::SendLimit]));
        assert!(near(t0.elapsed(), 290), "{:?}", t0.elapsed());
    }

    /// Restpunkt B-P4 (A12-5, Obergrenze): Kommt ein Schritt vor der ersten
    /// Sendung nicht weiter, endet er nach 240 s (Runde 2: 700 s). Die
    /// Obergrenze zwischen zwei Runden sinkt damit von 1 640 s auf 900 s, auch
    /// wenn beide Schritte kurz vor dem Limit senden und dann hängen.
    #[tokio::test(start_paused = true)]
    async fn a12_bp4_vorbereitung_hat_ein_eigenes_kurzes_limit() {
        let t = Takt::agent(120);
        assert_eq!(t.prep, Duration::from_secs(240));
        assert_eq!(t.max_gap(), Duration::from_secs(900));
        assert!(t.max_gap() < Duration::from_secs(240 + 700 + 700));
        let sent = AtomicUsize::new(0);
        let count = || sent.load(Ordering::SeqCst);
        // beide hängen schon vor der ersten Sendung: je 240 s, nichts gesendet
        let t0 = Instant::now();
        let r = agent_cycle(t, "test", count, async {}, std::future::pending(), std::future::pending()).await;
        assert_eq!(r, (true, [StepEnd::PrepLimit; 2]));
        assert!(near(t0.elapsed(), 480), "{:?}", t0.elapsed());
        // schlimmster Fall: die Runde schöpft ihr Limit aus, beide Schritte
        // senden kurz vor dem Vorbereitungslimit und warten dann für immer
        let t0 = Instant::now();
        let slow = || fake_step(&sent, Duration::from_millis(239_600), None);
        let r = agent_cycle(t, "test", count, std::future::pending(), slow(), slow()).await;
        assert_eq!(r, (false, [StepEnd::SendLimit; 2]));
        assert_eq!(sent.load(Ordering::SeqCst), 2, "beide gesendet");
        assert!(t0.elapsed().as_secs() <= t.max_gap().as_secs() && near(t0.elapsed(), 900), "{:?}", t0.elapsed());
        // eine Vorbereitung über dem Limit wird unterbrochen, der nächste
        // Durchlauf versucht es erneut (der Preis der Obergrenze)
        let r = agent_cycle(t, "test", count, async {}, async {}, fake_step(&sent, Duration::from_secs(300), None)).await;
        assert_eq!(r.1, [StepEnd::Done, StepEnd::PrepLimit]);
        assert_eq!(sent.load(Ordering::SeqCst), 2, "nicht gesendet");
    }

    /// Standardtakt 420 s (--interval 120), Startskript 540 s (--interval 300),
    /// jeweils wenn beide Schritte senden und warten; vorher bis 1 760 s.
    #[test]
    fn a12_standardtakt_und_startskript() {
        assert_eq!(Takt::agent(120).usual_gap(), Duration::from_secs(420));
        assert_eq!(Takt::agent(300).usual_gap(), Duration::from_secs(540));
        assert_eq!(Takt::agent(600).usual_gap(), Duration::from_secs(840), "lange Pause: die Schritte laufen darin");
    }

    /// Attrappe für KeeperIo: Ergebnis je Vault, Protokoll der Versuche
    #[derive(Default)]
    struct FakeKeeper {
        outcome: std::collections::HashMap<usize, Outcome>,
        tried: Vec<(&'static str, usize)>,
        sent: usize,
        journal: bool,
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        /// gesendet und bestätigt
        Ok,
        /// Bau scheitert, nichts gesendet
        Fail,
        /// gesendet, nicht bestätigt
        FailSent,
        /// submit mehrdeutig abgelehnt: nicht gesendet, Journal bleibt stehen
        FailJournal,
    }

    impl FakeKeeper {
        fn with(outcome: &[(usize, Outcome)]) -> Self {
            FakeKeeper { outcome: outcome.iter().copied().collect(), ..Default::default() }
        }
        fn run(&mut self, what: &'static str, i: usize) -> Result<(), String> {
            self.tried.push((what, i));
            match self.outcome.get(&i).copied().unwrap_or(Outcome::Fail) {
                Outcome::Ok => {
                    self.sent += 1;
                    Ok(())
                }
                Outcome::Fail => Err("zu groß für einen Block".into()),
                Outcome::FailSent => {
                    self.sent += 1;
                    Err("nicht bestätigt".into())
                }
                Outcome::FailJournal => {
                    self.journal = true;
                    Err("Node antwortet nicht".into())
                }
            }
        }
    }

    impl KeeperIo for FakeKeeper {
        async fn liquidate(&mut self, i: usize, _burn: i64) -> Result<(), String> {
            self.run("liq", i)
        }
        async fn sweep(&mut self, i: usize, _rest: usize) -> Result<(), String> {
            self.run("sweep", i)
        }
        fn sent(&self) -> usize {
            self.sent
        }
        fn journal_open(&self) -> bool {
            self.journal
        }
    }

    /// Audit 12 A12-2, Nachprüfung: sweep_in_turn versucht die Kandidaten der
    /// Reihe nach. Scheitert der Bau des ersten (z. B. zu schwerer Ausgang),
    /// kommt der nächste dran; wurde dabei schon gesendet, endet die Runde.
    #[tokio::test]
    async fn a12_keeper_versucht_alle_aufloesbaren_der_reihe_nach() {
        use Outcome::*;
        // erster scheitert ohne Sendung, zweiter geht
        let mut io = FakeKeeper::with(&[(3, Fail), (5, Ok)]);
        assert_eq!(sweep_in_turn(&mut io, &[3, 5, 7], "test").await, Some(5));
        assert_eq!(io.tried, vec![("sweep", 3), ("sweep", 5)]);
        // keiner geht: alle versucht
        let mut io = FakeKeeper::with(&[]);
        assert_eq!(sweep_in_turn(&mut io, &[3, 5, 7], "test").await, None);
        assert_eq!(io.tried.len(), 3);
        // gesendet, aber nicht bestätigt: Orakel-UTXO verbraucht, Runde endet
        let mut io = FakeKeeper::with(&[(3, FailSent)]);
        assert_eq!(sweep_in_turn(&mut io, &[3, 5, 7], "test").await, Some(3));
        assert_eq!(io.tried.len(), 1);
    }

    /// Restpunkt B-P2: Scheitert submit mehrdeutig, bleibt das Journal stehen,
    /// ohne dass der Zähler der gesendeten Tx steigt. Vorher galt der Kandidat
    /// als „nicht gesendet“, der nächste baute mit derselben Orakel-UTXO und
    /// überschrieb das Journal. Jetzt endet die Runde; liegt das Journal schon
    /// vorher, wird gar nicht erst gebaut.
    #[tokio::test]
    async fn a12_bp2_unklares_senden_beendet_das_aufloesen() {
        use Outcome::*;
        let mut io = FakeKeeper::with(&[(3, FailJournal), (5, Ok)]);
        assert_eq!(sweep_in_turn(&mut io, &[3, 5, 7], "test").await, Some(3));
        assert_eq!(io.tried, vec![("sweep", 3)], "kein zweiter Bau über dem offenen Journal");
        assert_eq!(io.sent, 0);
        let mut io = FakeKeeper { journal: true, ..FakeKeeper::with(&[(3, Ok)]) };
        assert_eq!(sweep_in_turn(&mut io, &[3, 5], "test").await, None);
        assert!(io.tried.is_empty());
        // gleiches Muster in der Liquidationsschleife: nach einer mehrdeutig
        // gescheiterten Liquidation weder die nächste noch ein Auflösen
        let plan = KeeperPlan { found: 2, mine: 100, liquidate: vec![(1, 10), (2, 10)], sweep: vec![3, 5] };
        let mut io = FakeKeeper::with(&[(1, FailJournal)]);
        keeper_act(&mut io, &plan, "test").await;
        assert_eq!(io.tried, vec![("liq", 1)]);
    }

    /// Restpunkt B-P3: Die Keeper-Runde gibt die GANZE Liste der auflösbaren
    /// Vaults an sweep_in_turn, auch nach gescheiterten Liquidationen und ohne
    /// eigene GHOST; nach einer gesendeten Liquidation löst sie nichts auf.
    #[tokio::test]
    async fn a12_bp3_keeper_runde_loest_mit_der_ganzen_liste_auf() {
        use Outcome::*;
        let sweeps = |io: &FakeKeeper| io.tried.iter().filter(|t| t.0 == "sweep").map(|t| t.1).collect::<Vec<_>>();
        // nichts liquidierbar
        let plan = KeeperPlan { sweep: vec![3, 5, 7], ..Default::default() };
        let mut io = FakeKeeper::with(&[]);
        keeper_act(&mut io, &plan, "test").await;
        assert_eq!(sweeps(&io), vec![3, 5, 7]);
        // liquidierbar, aber keine GHOST
        let plan = KeeperPlan { found: 2, mine: 0, liquidate: vec![], sweep: vec![3, 5, 7] };
        let mut io = FakeKeeper::with(&[(7, Ok)]);
        keeper_act(&mut io, &plan, "test").await;
        assert_eq!(io.tried, vec![("sweep", 3), ("sweep", 5), ("sweep", 7)]);
        // beide Liquidationen scheitern ohne Sendung: dann alle Auflösungen
        let plan = KeeperPlan { found: 2, mine: 100, liquidate: vec![(1, 10), (2, 10)], sweep: vec![3, 5, 7] };
        let mut io = FakeKeeper::with(&[]);
        keeper_act(&mut io, &plan, "test").await;
        assert_eq!(io.tried, vec![("liq", 1), ("liq", 2), ("sweep", 3), ("sweep", 5), ("sweep", 7)]);
        // Liquidation gesendet (auch unbestätigt): kein Auflösen
        for o in [Ok, FailSent] {
            let mut io = FakeKeeper::with(&[(1, o)]);
            keeper_act(&mut io, &plan, "test").await;
            assert_eq!(io.tried, vec![("liq", 1)]);
        }
    }

    /// Ausschnitt des Programms ohne dieses Testmodul, Leerraum entfernt
    fn code_between(from: &str, to: &str) -> String {
        let src = include_str!("ghostctl.rs");
        let code = &src[..src.find("#[cfg(test)]\nmod tests {").unwrap()];
        let a = code.find(from).unwrap_or_else(|| panic!("{from} fehlt"));
        let b = a + from.len() + code[a + from.len()..].find(to).unwrap_or_else(|| panic!("{to} fehlt"));
        code[a..b].split_whitespace().collect()
    }

    /// Restpunkt B-P3: Verdrahtung, die ohne Netz nicht läuft. Cmd::Agent
    /// überlässt jede Zeitsteuerung agent_cycle (Takt::agent), der Keeper
    /// läuft mit und ohne Komitee-Datei, keeper_round führt keeper_plan mit
    /// keeper_act aus, und der Plan enthält die ganze Liste aus
    /// ops::sweep_candidates. Ein Rückbau dieser Zeilen macht den Test rot.
    /// Wallet-Aktionen (wallet build | submit): nie eine Schlüsseldatei; build
    /// schreibt nichts; submit prüft zuerst ohne Netz und ohne Sperre die
    /// Signaturen (wallet_ops::precheck, A17-4), dann die eigenen UTXOs am
    /// Node, baut über wallet_ops::submit neu und sendet nur mit --send; die
    /// Sperre kommt erst danach, und Ctx::send_wallet gibt sie vor dem Warten
    /// frei (A17-6). Ein Rückbau dieser Zeilen macht den Test rot.
    #[test]
    fn wallet_aktionen_verdrahtung() {
        let f = code_between("async fn wallet_action_cmd(", "\nfn now_ms()");
        assert!(!f.contains("load_key") && !f.contains("keys/"), "keine Schlüsseldatei");
        let build = code_between("WalletCmd::Build { address, .. } => {", "WalletCmd::Submit { send, .. } => {");
        assert!(build.contains("letd=load_readonly(&ctx).await?;"), "{build}");
        assert!(!build.contains("save(") && !build.contains("atomic_write") && !build.contains("ctx.send("), "build schreibt nichts");
        let pre = code_between("let submitted = match cmd {", "let net = Net::connect(network, rpc).await?;");
        assert!(pre.contains("letpre=wo::precheck(&plan,&signed,network,prefix)?;ifletSome(e)=&pre.error{"), "{pre}");
        assert!(pre.contains("if*send{returnErr(format!(\"Signaturungültig–nichtgesendet({e})\"));}"), "{pre}");
        let submit = code_between("WalletCmd::Submit { send, .. } => {", "_ => Err(\"wallet build | submit erwartet\".into()),");
        let (unlocked, locked) = submit.split_once("letlock=store::lock(&ctx.state_path,WALLET_LOCK_WAIT)?;").expect("Sperre erst nach der Prüfung");
        assert!(unlocked.contains("check_funding_at_node(&ctx.net,&plan).await?;letd=load_readonly(&ctx).await?;letr=wo::submit(&d,&plan,&signed,network,prefix,&p)?;"), "{unlocked}");
        assert!(unlocked.contains("if*send{ifr.built.is_none(){returnErr("), "{unlocked}");
        assert!(!unlocked.contains("store::lock(") && !pre.contains("store::lock("), "keine Sperre vor der Prüfung: {unlocked}");
        assert!(locked.starts_with("letd=ctx.load_synced().await?;check_funding_at_node(&ctx.net,&plan).await?;letr=wo::submit(&d,&plan,&signed,network,prefix,&p)?;"), "{locked}");
        assert!(locked.contains("letconfirmed=ctx.send_wallet(lock,plan.action.label(),b,next).await?;"), "{locked}");
        let sw = code_between("async fn send_wallet(", "async fn funds(");
        assert!(sw.contains("store::send_then_wait(lock,send,wait).await"), "{sw}");
        assert!(sw.contains("store::wait_accepted(&self.net,&b.tx,WALLET_WAIT_BASE,WALLET_WAIT_MAX)"), "{sw}");
        assert!(sw.contains(",true)?;"), "Journal als Wallet-Tx: {sw}");
        let ro = code_between("async fn load_readonly(", "\n}\n");
        assert!(!ro.contains("save(") && !ro.contains("resolve_pending") && !ro.contains("atomic_write"), "{ro}");
        let chk = code_between("async fn check_funding_at_node(", "\n}\n");
        assert!(chk.contains("o==op&&x.amount==e.amount&&x.script_public_key==e.script_public_key&&x.covenant_id.is_none()"), "{chk}");
    }

    /// Tresor-Aktionen der Browser-Wallet (wallet build | submit tresor-…):
    /// Zustand, Sperre und Journal an der Tresor-Datei; nie eine
    /// Schlüsseldatei (auch nicht als Empfänger); build und die Prüfung ohne
    /// --send schreiben nichts; die Sperre kommt erst nach der Prüfung, unter
    /// ihr wird das Journal geklärt, neu gebaut und verglichen, und den neuen
    /// Stand der Tresor-Datei schreibt nur das Journal bei Annahme
    /// (send_wallet). Ein Rückbau dieser Zeilen macht den Test rot.
    #[test]
    fn wallet_tresor_verdrahtung() {
        let head = code_between("async fn wallet_action_cmd(", "let net = Net::connect(network, rpc).await?;");
        assert!(head.contains("letstate_path=iftresor_action{tresor::path_for(state_path)}else{state_path.to_path_buf()};"), "{head}");
        let of = code_between("fn wallet_tresor_action_of(", "\n}\n");
        assert!(!of.contains("load_key") && !of.contains("keys/") && !of.contains("Path::new"), "{of}");
        assert!(of.contains("letrecipient=kaspa_lending_protocol::wallet_ops::ghost_target(prefix,&t)?;"), "nur Adressen/x-only: {of}");
        assert!(of.contains("if!m.is_empty()&&!*onchain_message{returnErr("), "nur öffentliche Nachricht: {of}");
        let basis = code_between("async fn tresor_basis(", "\n}\n");
        assert!(!basis.contains("save(") && !basis.contains("atomic_write") && !basis.contains("resolve_pending"), "{basis}");
        // A19-2: Nachführen nur über follow_for_wallet (Besitzer zuerst, begrenzte Suche)
        assert!(basis.contains("kaspa_lending_protocol::wallet_ops::follow_for_wallet(&mutTresorNet{ctx,key:None},&mutfile,a,owner,pmt,&now).await?"), "{basis}");
        assert!(!basis.contains("tresor::follow(") && !basis.contains("Search::Full"), "{basis}");
        let build = code_between("async fn wallet_tresor_build(", "\n}\n");
        assert!(build.contains("letowner=kaspa_lending_protocol::wallet::xonly_of_address(address,prefix)?;") && build.contains("letbasis=tresor_basis(ctx,&a,&owner,pmt).await?;"), "{build}");
        let build = code_between("async fn wallet_tresor_build(", "\n}\n");
        assert!(!build.contains("save(") && !build.contains("atomic_write") && !build.contains("send_wallet") && !build.contains("load_key"), "{build}");
        let submit = code_between("async fn wallet_tresor_submit(", "\n}\n");
        assert!(!submit.contains("load_key") && !submit.contains("tresor::save") && !submit.contains("atomic_write"), "{submit}");
        let (unlocked, locked) = submit.split_once("letlock=store::lock(&ctx.state_path,WALLET_LOCK_WAIT).map_err(tresor_lock_busy)?;").expect("Sperre erst nach der Prüfung");
        assert!(unlocked.contains("check_funding_at_node(&ctx.net,&plan).await?;letpmt=ctx.net.past_median_time().await?;letbasis=tresor_basis(ctx,&plan.action,&plan.owner,pmt).await?;letr=wo::submit(&basis,&plan,&signed,&ctx.network,prefix,&p)?;"), "{unlocked}");
        assert!(unlocked.contains("ifsend{ifr.built.is_none(){returnErr("), "{unlocked}");
        assert!(!unlocked.contains("store::lock("), "keine Sperre vor der Prüfung: {unlocked}");
        assert!(locked.starts_with("ifletSome(msg)=store::resolve_pending(&ctx.net,&ctx.state_path).await?{"), "{locked}");
        assert!(locked.contains("check_funding_at_node(&ctx.net,&plan).await?;letpmt=ctx.net.past_median_time().await?;letbasis=tresor_basis(ctx,&plan.action,&plan.owner,pmt).await?;letr=wo::submit(&basis,&plan,&signed,&ctx.network,prefix,&p)?;"), "{locked}");
        assert!(locked.contains("letconfirmed=ctx.send_wallet(lock,plan.action.label(),b,&next.file).await?;"), "{locked}");
    }

    #[test]
    fn a12_bp3_verdrahtung_agent_und_keeper() {
        let agent = code_between("Cmd::Agent { key,", "Cmd::OpenVault {");
        assert!(agent.contains("lettakt=Takt::agent(interval);"), "{agent}");
        assert!(agent.contains("agent_cycle(takt,&now,sent_count,round,abo,tresor).await;"), "{agent}");
        assert!(!agent.contains("tokio::time::timeout") && !agent.contains("tokio::time::sleep"), "keine eigene Zeitsteuerung neben agent_cycle");
        assert!(agent.contains("ifletSome(c)=&committee{oracle_round(&round,&key,c,min_change,max_age_min,&streak,m,&now).await;}keeper_round(&round,&k,m,&now).await;"), "Keeper auch ohne Komitee-Datei");
        assert!(agent.contains("letabo=abo_agent_step(") && agent.contains("lettresor=tresor_agent_step("));
        let keeper = code_between("async fn keeper_round(", "\n}\n");
        assert!(keeper.contains("letSome(plan)=keeper_plan(&d,&xonly(k),market,now)else{return};keeper_act(&mutKeeperNet{round,d:&d,k,now},&plan,now).await;"), "{keeper}");
        let plan = code_between("fn keeper_plan(", "\n}\n");
        assert!(plan.contains("plan.sweep=ops::sweep_candidates(d,market);Some(plan)"), "{plan}");
        let act = code_between("async fn keeper_act(", "\n}\n");
        assert!(act.contains("sweep_in_turn(io,&plan.sweep,now).await;"), "{act}");
    }

    fn rate_state(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ghostctl-zins-{name}-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("mainnet.json")
    }

    /// Restpunkt C-O1 (A12-17 c): oracle-update --rate setzt den Takt der
    /// Zinsregel in derselben Datei wie der Agent, vor dem Senden; ohne
    /// Sendung wird die Vormerkung zurückgenommen, nach einer Sendung bleibt
    /// sie. Ohne --rate bleibt die Datei unberührt.
    #[tokio::test]
    async fn a12_co1_zins_von_hand_setzt_den_takt_der_zinsregel() {
        let state = rate_state("hand");
        let file = RateFile { state_path: &state, network: "mainnet", dry_run: false };
        let path = rate::path_for(&state);
        let old = unix_now() - 7_200;
        rate::update(&path, "mainnet", |l| l.last_change = Some(old)).unwrap();
        let last = || rate::load(&path, "mainnet").unwrap().last_change.unwrap();
        let sent = AtomicUsize::new(0);
        let count = || sent.load(Ordering::SeqCst);
        // ohne --rate: nichts
        rate_by_hand(&file, None, count, async { Ok(()) }).await.unwrap();
        assert_eq!(last(), old);
        // gesendet und bestätigt: Takt ab jetzt, die Zinsregel wartet eine Stunde
        let t0 = unix_now();
        rate_by_hand(&file, Some(4.0), count, async {
            sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
        assert!(last() >= t0);
        assert!(matches!(rate::load(&path, "mainnet").unwrap().decide(unix_now(), 4.0), rate::Decision::Wait { .. }));
        // abgelehnt, nichts gesendet: zurück auf den vorigen Stand
        rate::update(&path, "mainnet", |l| l.last_change = Some(old)).unwrap();
        let e = rate_by_hand(&file, Some(4.0), count, async { Err("Preissprung zu groß".to_string()) }).await;
        assert!(e.is_err());
        assert_eq!(last(), old);
        // gesendet, Bestätigung offen: der Takt bleibt gesetzt
        let e = rate_by_hand(&file, Some(4.0), count, async {
            sent.fetch_add(1, Ordering::SeqCst);
            Err("nicht bestätigt".to_string())
        })
        .await;
        assert!(e.is_err());
        assert!(last() >= t0);
        let _ = std::fs::remove_dir_all(state.parent().unwrap());
    }

    /// Restpunkt C-O1: deploy beginnt die Zinsregel neu (Messungen des alten
    /// Pools weg, Takt ab jetzt), und beide Befehle sind verdrahtet
    #[test]
    fn a12_co1_deploy_beginnt_die_zinsregel_neu_und_verdrahtung() {
        let state = rate_state("deploy");
        let path = rate::path_for(&state);
        rate::update(&path, "mainnet", |l| {
            for i in 0..6 {
                l.record(unix_now() - 3_000 + i * 500, 0.9);
            }
            l.last_change = Some(1);
        })
        .unwrap();
        let t0 = unix_now();
        rate_restart(&RateFile { state_path: &state, network: "mainnet", dry_run: false });
        let l = rate::load(&path, "mainnet").unwrap();
        assert!(l.samples.is_empty() && l.last_change.unwrap() >= t0, "{l:?}");
        let _ = std::fs::remove_dir_all(state.parent().unwrap());
        let deploy = code_between("Cmd::Deploy {", "Cmd::Sync =>");
        assert_eq!(deploy.matches("rate_restart(&RateFile::of(&ctx));").count(), 2, "nach dem letzten Schritt und bei „Deployment ist abgeschlossen“");
        let update = code_between("async fn oracle_update(", "\n}\n");
        assert!(update.contains("rate_by_hand(&RateFile::of(ctx),rate,sent_count,oracle_update_price(ctx,key,committee,kas_usd,rate)).await"), "{update}");
        assert!(code_between("Cmd::OracleUpdate {", "Cmd::OracleFeed {").contains("oracle_update(&ctx,&key,&committee,usd,rate).await?;"));
    }

    /// Restpunkt C-X1: Das Startskript nennt ohne Unterzeichner-Datei alle Aufgaben,
    /// die der Agent dann ausführt (Liquidationen, Auflösen, Daueraufträge,
    /// Tresore), statt „nur Liquidationen“; mit Komitee-Datei dazu Orakel und
    /// Zinsregel. Es startet mit --interval 300 (Takt-Tests: 540 s).
    #[test]
    fn a12_cx1_startskript_nennt_alle_aufgaben() {
        let skript = include_str!("../../../GHOST-Agent starten.command");
        assert!(skript.contains("--interval 300"));
        // erste Meldung: mit Komitee-Datei, zweite: ohne
        let start: Vec<&str> = skript.lines().filter(|l| l.trim_start().starts_with("echo \"GHOST-Agent ($NET):")).collect();
        assert_eq!(start.len(), 2, "{start:?}");
        let (mit, ohne) = (start[0], start[1]);
        assert!(!ohne.contains("nur Liquidationen"), "{ohne}");
        for w in ["Liquidationen", "Auflösen", "Daueraufträge", "Tresore", "ohne Unterzeichner-Datei"] {
            assert!(ohne.contains(w), "{w} fehlt: {ohne}");
        }
        for w in ["Orakel", "Zins", "Liquidationen", "Auflösen", "Daueraufträge", "Tresore"] {
            assert!(mit.contains(w), "{w} fehlt: {mit}");
        }
    }

    // --------------------------------------- Audit 12, Restpunkte (Gruppe a) ----

    use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint, UtxoEntry};
    use kaspa_lending_protocol::sim::Sim;

    const E8: u64 = 100_000_000;

    fn test_key(n: u8) -> Keypair {
        Keypair::from_secret_key(&Secp256k1::new(), &SecretKey::from_slice(&[n; 32]).unwrap())
    }

    /// Ein- und Ausgabe von `tresor_pay` gegen den Simulator (wie `SimIo` in
    /// tests/tresor_e2e_tests.rs; `TresorNet` braucht einen Node)
    struct SimIo<'a> {
        sim: &'a mut Sim,
        key: Option<Keypair>,
    }

    impl tresor::TresorIo for SimIo<'_> {
        async fn utxos(&mut self, s: &ScriptPublicKey) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
            Ok(self.sim.utxos.iter().filter(|(_, e)| e.script_public_key == *s).map(|(o, e)| (*o, e.clone())).collect())
        }
        async fn funds(&mut self) -> Option<Funds> {
            self.key.map(|k| self.sim.funds(&k))
        }
        async fn send(&mut self, _: &str, b: &Built, _: &tresor::TresorFile) -> Result<(), String> {
            self.sim.submit(b)
        }
        fn save(&mut self, _: &tresor::TresorFile) -> Result<(), String> {
            Ok(())
        }
        fn journal_open(&self) -> bool {
            false
        }
        fn dry_run(&self) -> bool {
            false
        }
        fn now_ms(&self) -> i64 {
            self.sim.now_ms as i64
        }
        fn say(&mut self, _: &str) {}
    }

    /// Fälliger Tresor im Simulator, der die Gebühr nicht mehr trägt: 10 KAS +
    /// 1 KAS + 0,005 KAS, nach Betrag und Höchstgebühr blieben < 1 KAS
    fn knapper_tresor(sim: &mut Sim) -> TresorRec {
        let p = TresorParams {
            owner: xonly(&test_key(1)),
            recipient: xonly(&test_key(2)),
            amount: 10 * E8 as i64,
            anchor_day: 1,
            period_ms: 0,
            max_fee: tresor::DEFAULT_MAX_FEE,
            payload_hash: payload_hash(&[]),
        };
        let s = TresorState { next_due: 1_801_468_800_000, left: 3 }; // 2027-02-01 08:00 UTC
        let (op, cov) = (TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([0xee; 32]), 0), kaspa_consensus_core::Hash::from_bytes([0xcc; 32]));
        let value = 11 * E8 + 500_000;
        sim.utxos.insert(op, UtxoEntry::new(value, tresor::TresorShape::of(&p).spk(&s), sim.daa, false, Some(cov)));
        sim.set_time(s.next_due as u64 + 10 * 60_000);
        TresorRec::new(p, ops::Tracked { outpoint: op, value, cov, state: s }, String::new(), false, None, "x")
    }

    /// Restpunkt zu A12-7: `tresor pay` gibt den Schlüssel an die Ein-/Ausgabe
    /// und als `with_key` an die Runde, beides aus derselben Angabe. Ohne
    /// Schlüssel bleibt ein Tresor, der die Gebühr nicht trägt, unberührt –
    /// kein Versuch, kein Fehler, keine Wartezeit (Rückbau `key.is_some()` →
    /// `true`: die Zahlung scheitert ohne Mittel, last_error und Wartezeit).
    /// Mit Schlüssel zahlt der Auslöser die Gebühr (Rückbau → `false`: nie).
    #[tokio::test]
    async fn a13_tresor_pay_uebergibt_den_schluessel_an_die_runde() {
        let mut sim = Sim::new();
        let rec = knapper_tresor(&mut sim);
        let (params, pmt) = (sim.params.clone(), sim.now_ms as i64 - tresor::PMT_LAG_MS);
        let mut f = tresor::TresorFile::empty("mainnet");
        f.upsert(rec.clone());
        // ohne Schlüssel: Runde und bestimmter Tresor
        let reps = tresor_pay(|key| SimIo { sim: &mut sim, key }, None, &mut f, None, pmt, "x", &params).await.unwrap();
        assert!(reps.is_empty(), "{reps:?}");
        let e = tresor_pay(|key| SimIo { sim: &mut sim, key }, None, &mut f, Some(&rec.id), pmt, "x", &params).await.unwrap_err();
        assert!(e.contains("auslösen nur mit eigenem Schlüssel"), "{e}");
        let t = &f.tresore[0];
        assert_eq!((t.last_error.clone(), t.retry_after, t.utxo.value), (None, None, rec.utxo.value), "unberührt");
        // mit Schlüssel: gezahlt, die Gebühr vom Schlüssel
        let helfer = test_key(3);
        sim.faucet(&helfer, 5 * E8);
        let reps = tresor_pay(|key| SimIo { sim: &mut sim, key }, Some(helfer), &mut f, None, pmt, "x", &params).await.unwrap();
        assert_eq!(reps.iter().map(|r| r.paid).collect::<Vec<_>>(), vec![true], "{reps:?}");
        assert_eq!(f.tresore[0].utxo.value, rec.utxo.value - 10 * E8, "der Tresor verliert nur den Betrag");
        assert!(sim.balance(&helfer) < 5 * E8, "Gebühr vom Schlüssel");
    }

    /// Über die Browser-Wallet angelegte Tresore (öffentliche Seite) tragen die
    /// Netzgebühr nur selbst: Der Agent zahlt sie auch mit Schlüssel nie dazu
    /// (TresorRec::wallet), weder in der Runde noch gezielt, und offline geht
    /// es dafür nicht einmal zum Node.
    #[tokio::test]
    async fn wallet_tresor_agent_zahlt_keine_gebuehr_dazu() {
        let mut sim = Sim::new();
        let mut rec = knapper_tresor(&mut sim);
        rec.wallet = true;
        let (params, pmt) = (sim.params.clone(), sim.now_ms as i64 - tresor::PMT_LAG_MS);
        let mut f = tresor::TresorFile::empty("mainnet");
        f.upsert(rec.clone());
        let helfer = test_key(3);
        sim.faucet(&helfer, 5 * E8);
        let reps = tresor_pay(|key| SimIo { sim: &mut sim, key }, Some(helfer), &mut f, None, pmt, "x", &params).await.unwrap();
        assert!(reps.is_empty(), "{reps:?}");
        let e = tresor_pay(|key| SimIo { sim: &mut sim, key }, Some(helfer), &mut f, Some(&rec.id), pmt, "x", &params).await.unwrap_err();
        assert!(e.contains("auslösen nur mit eigenem Schlüssel"), "{e}");
        assert_eq!(sim.balance(&helfer), 5 * E8, "keine Gebühr vom Agenten");
        assert_eq!(f.tresore[0].utxo.value, rec.utxo.value, "unberührt");
        assert!(!tresor::needs_run(&f, sim.now_ms as i64, true), "offline: nichts zu tun, auch mit Schlüssel");
    }

    /// `tresor owned`: nur die über die Wallet angelegten Tresore des Besitzers
    /// (Adresse oder x-only), ohne Pfad der Schlüsseldatei; andere Netze und
    /// Unsinn abgelehnt
    #[test]
    fn tresor_owned_nur_eigene_ohne_pfade() {
        let dir = std::env::temp_dir().join(format!("ghostctl-owned-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mainnet-tresore.json");
        let mut a = knapper_tresor(&mut Sim::new());
        a.wallet = true;
        a.key = Some("keys/geheim.json".into());
        let mut b = a.clone();
        b.params.owner = xonly(&test_key(4));
        b.utxo.cov = kaspa_consensus_core::Hash::from_bytes([0xdd; 32]);
        b.id = tresor::id_of(&b.utxo.cov);
        // Tresor desselben Besitzers mit Schlüsseldatei (Betreiber): bleibt privat
        let mut c = a.clone();
        c.wallet = false;
        c.utxo.cov = kaspa_consensus_core::Hash::from_bytes([0xde; 32]);
        c.id = tresor::id_of(&c.utxo.cov);
        let mut f = tresor::TresorFile::empty("mainnet");
        f.upsert(a.clone());
        f.upsert(b);
        f.upsert(c);
        tresor::save(&path, &f).unwrap();
        let owned = |o: &str| tresor_offline("mainnet", &path, &TresorCmd::Owned { owner: o.into() }, false);
        let addr = xonly_address(kaspa_addresses::Prefix::Mainnet, &a.params.owner);
        for o in [addr.clone(), faster_hex::hex_string(&a.params.owner)] {
            let v = owned(&o).unwrap().unwrap();
            let list = v["tresore"].as_array().unwrap();
            assert_eq!(list.len(), 1, "{v}");
            assert_eq!(list[0]["id"], a.id);
            assert_eq!(v["owner"], addr);
            assert!(list[0].get("key").is_none() && !v.to_string().contains("keys/"), "{v}");
            assert_eq!(list[0]["feeFromTresor"], false);
        }
        let other = xonly_address(kaspa_addresses::Prefix::Mainnet, &xonly(&test_key(5)));
        assert_eq!(owned(&other).unwrap().unwrap()["tresore"], serde_json::json!([]));
        assert!(owned("kaspatest:qqqq").is_err() && owned("keys/geheim.json").is_err() && owned("").is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    /// Audit 19 A19-6: Ein offenes Journal einer Wallet-Tx (gesendet, noch
    /// nicht übernommen) erscheint in `tresor owned` – der neue Tresor als
    /// Eintrag mit `pending`, ein geänderter mit Markierung. Fremde Tresore im
    /// Journal und Journale ohne Wallet-Kennung bleiben außen vor.
    #[test]
    fn a19_6_tresor_owned_zeigt_offenes_journal() {
        let dir = std::env::temp_dir().join(format!("ghostctl-a19-6-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mainnet-tresore.json");
        let mut a = knapper_tresor(&mut Sim::new());
        a.wallet = true;
        let mut f = tresor::TresorFile::empty("mainnet");
        f.upsert(a.clone());
        tresor::save(&path, &f).unwrap();
        // Journal: a aufgefüllt, b neu (gleicher Besitzer), c neu (fremd)
        let mut next = f.clone();
        next.tresore[0].utxo.outpoint = TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([0x11; 32]), 0);
        let mut b = a.clone();
        b.utxo.cov = kaspa_consensus_core::Hash::from_bytes([0xbb; 32]);
        b.id = tresor::id_of(&b.utxo.cov);
        let mut c = b.clone();
        c.params.owner = xonly(&test_key(5));
        c.utxo.cov = kaspa_consensus_core::Hash::from_bytes([0xcd; 32]);
        next.upsert(b.clone());
        next.upsert(c);
        let txid = "ab".repeat(32);
        let journal = |wallet: bool| {
            let op = TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([0x22; 32]), 0);
            let p = store::Pending {
                action: "Tresor anlegen (Wallet)".into(),
                txid: txid.clone(),
                outputs: vec![],
                first_input: (op, p2pk_spk(&a.params.owner)),
                inputs: vec![],
                change_output: None,
                wallet,
                values: vec![],
                target: Some(path.clone()),
                next: Some(serde_json::to_value(&next).unwrap()),
            };
            std::fs::write(store::pending_path(&path), serde_json::to_string(&p).unwrap()).unwrap();
        };
        let owner = xonly_address(kaspa_addresses::Prefix::Mainnet, &a.params.owner);
        let owned = || tresor_offline("mainnet", &path, &TresorCmd::Owned { owner: owner.clone() }, false).unwrap().unwrap();
        let v = owned();
        assert_eq!(v["tresore"].as_array().unwrap().len(), 1, "ohne Journal nur die Datei");
        assert!(v["tresore"][0].get("pending").is_none(), "{v}");
        journal(true);
        let v = owned();
        let list = v["tresore"].as_array().unwrap();
        assert_eq!(list.len(), 2, "der neue Tresor erscheint sofort: {v}");
        assert_eq!((list[0]["id"].clone(), list[0]["pending"]["txid"].clone()), (a.id.clone().into(), txid.clone().into()), "geänderter markiert");
        assert_eq!((list[1]["id"].clone(), list[1]["pending"]["txid"].clone()), (b.id.clone().into(), txid.clone().into()), "neuer markiert");
        assert!(!v.to_string().contains(&tresor::id_of(&kaspa_consensus_core::Hash::from_bytes([0xcd; 32]))), "fremder Tresor nicht");
        journal(false);
        assert_eq!(owned()["tresore"].as_array().unwrap().len(), 1, "kein Wallet-Journal: nichts dazu");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Audit 19 A19-6: Der Tresor-Schritt des Agenten klärt ein offenes
    /// Journal in jeder Runde, auch ohne Fälliges, und dann nur das (keine
    /// Runde über alle Tresore). A19-3: Warnung an den Betreiber, wenn die
    /// Datei sich den Grenzen nähert. Ein Rückbau macht den Test rot.
    #[test]
    fn a19_6_agent_klaert_journal_auch_ohne_faelliges() {
        let step = code_between("async fn tresor_agent_step(", "\n}\n");
        assert!(step.contains("letjournal=store::pending_path(&path).exists();"), "{step}");
        assert!(step.contains("if!due&&!journal{return;}"), "{step}");
        assert!(step.contains("Ok(_lock)if!due=>matchstore::resolve_pending(&ctx.net,&ctx.state_path).await{"), "{step}");
        assert!(step.contains("ifletSome(w)=tresor_room_warning(&f,now_ms()){"), "{step}");
        let mut f = tresor::TresorFile::empty("mainnet");
        let mut r = knapper_tresor(&mut Sim::new());
        r.utxo.value = 50 * E8;
        let now = r.utxo.state.next_due;
        assert!(r.busy(now));
        for n in 0..tresor::MAX_FILE_TRESORE * 9 / 10 - 1 {
            let mut x = r.clone();
            let mut h = [9u8; 32];
            h[..8].copy_from_slice(&(n as u64).to_le_bytes());
            x.utxo.cov = kaspa_consensus_core::Hash::from_bytes(h);
            f.tresore.push(x);
        }
        assert!(tresor_room_warning(&f, now).is_none());
        f.tresore.push(r);
        assert!(tresor_room_warning(&f, now).unwrap().contains("fast voll"));
    }

    /// Audit 19 A19-7: Ist die Sperre der Tresor-Datei belegt (meist wartet
    /// der Agent auf die Bestätigung einer Zahlung), bekommt der Wallet-Submit
    /// eine Meldung, die die Seite als „gleich erneut“ erkennt; andere Fehler
    /// bleiben unverändert.
    #[test]
    fn a19_7_belegte_tresor_sperre_heisst_gleich_erneut() {
        let dir = std::env::temp_dir().join(format!("ghostctl-a19-7-{}-{}", std::process::id(), rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mainnet-tresore.json");
        let _agent = store::lock(&path, Duration::ZERO).unwrap();
        let e = store::lock(&path, Duration::ZERO).map_err(tresor_lock_busy).err().unwrap();
        assert_eq!(e, TRESOR_BUSY);
        assert!(e.contains("Zahlungsrunde für Tresore") && e.contains("nichts gesendet") && e.contains("erneut senden"), "{e}");
        assert_eq!(tresor_lock_busy("Sperre x: Zugriff verweigert".into()), "Sperre x: Zugriff verweigert");
        drop(_agent);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Dieselbe Übergabe schon vor der Verbindung zum Node (`tresor pay` ohne
    /// ID, offline): mit Schlüssel geht es zum Node, ohne bleibt es bei
    /// „Keine fälligen Tresor-Zahlungen“.
    #[test]
    fn a13_tresor_pay_offline_mit_und_ohne_schluessel() {
        let dir = std::env::temp_dir().join(format!("ghostctl-a13-offline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mainnet-tresore.json");
        let mut rec = knapper_tresor(&mut Sim::new());
        rec.utxo.state.next_due = now_ms() - kaspa_lending_protocol::standing::DAY_MS;
        let mut f = tresor::TresorFile::empty("mainnet");
        f.upsert(rec);
        tresor::save(&path, &f).unwrap();
        let pay = |key: Option<PathBuf>| tresor_offline("mainnet", &path, &TresorCmd::Pay { id: None, key }, false).unwrap();
        assert!(pay(Some(dir.join("helfer.json"))).is_none(), "mit Schlüssel: zum Node");
        let r = pay(None).expect("ohne Schlüssel: offline fertig");
        assert_eq!(r["reports"], serde_json::json!([]));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    /// Restpunkt „Invalid-Text-zu-lang“: ghostctl messages nennt den
    /// richtigen Grund (Terminal und Feld `invalid` für die Seite). Vorher hieß
    /// es auch bei zu langen Texten „mit unzulässigen Zeichen“.
    #[test]
    fn a13_nicht_angezeigt_mit_dem_richtigen_grund() {
        use message::{Found, Rejected};
        let long = found_view(&Found::Invalid(Rejected::TooLong));
        assert_eq!((long.text.as_deref(), long.kind, long.invalid), (None, "invalid", Some("length")));
        assert_eq!(long.line, "Nachricht zu lang (über 100 Zeichen) – nicht angezeigt");
        let chars = found_view(&Found::Invalid(Rejected::BadChars));
        assert_eq!((chars.kind, chars.invalid), ("invalid", Some("chars")));
        assert!(chars.line.contains("unzulässigen Zeichen") && !chars.line.contains("zu lang"), "{}", chars.line);
        let both = found_view(&Found::Invalid(Rejected::TooLongAndBadChars));
        assert_eq!(both.invalid, Some("both"));
        assert!(both.line.contains("zu lang") && both.line.contains("unzulässigen Zeichen"), "{}", both.line);
        // vom Payload bis zur Anzeige: JSON einer anderen Anwendung ist zu lang, nicht unzulässig
        let sk = SecretKey::from_keypair(&test_key(4));
        let json = format!("{{\"app\":\"fremd\",\"daten\":\"{}\"}}", "a".repeat(120));
        assert_eq!(found_view(&message::read(&sk, json.as_bytes()).unwrap()).invalid, Some("length"));
        assert_eq!(found_view(&message::read(&sk, "a\u{202e}b".as_bytes()).unwrap()).invalid, Some("chars"));
        let ok = found_view(&Found::Public("Miete".into()));
        assert_eq!((ok.text.as_deref(), ok.kind, ok.invalid, ok.line.as_str()), (Some("Miete"), "public", None, "„Miete“ (öffentlich)"));
        // bis in die Ausgabe von `messages` (Zeile und JSON je Nachricht)
        let entry = |found: Found| message::Inbox {
            txid: "11".repeat(32),
            time_ms: None,
            amount: 5 * E8,
            unit: "KAS",
            from: vec![],
            found,
            origin: message::Origin::Direct,
            source: message::Source::Rest,
        };
        let prefix = kaspa_addresses::Prefix::Mainnet;
        for (why, code) in [(Rejected::TooLong, "length"), (Rejected::BadChars, "chars"), (Rejected::TooLongAndBadChars, "both")] {
            let (line, j) = inbox_item(&entry(Found::Invalid(why)), prefix);
            assert_eq!((j["kind"].as_str(), j["invalid"].as_str(), j["text"].is_null()), (Some("invalid"), Some(code), true), "{why:?}");
            assert!(line.contains(&found_view(&Found::Invalid(why)).line), "{line}");
        }
        let (line, j) = inbox_item(&entry(Found::Public("Miete".into())), prefix);
        assert_eq!((j["kind"].as_str(), j["text"].as_str(), j["invalid"].is_null(), j["source"].as_str()), (Some("public"), Some("Miete"), true, Some("rest")));
        assert!(line.contains("„Miete“ (öffentlich)") && line.ends_with("(laut REST-API)"), "{line}");
    }

    /// A12-1 im Vertrag: `messages` sagt bei Tresor-Zahlungen, dass die
    /// Nachricht beim Anlegen hinterlegt und vom Vertrag erzwungen ist – auch
    /// ohne übernommenen Tresor-Code (dann ohne Besitzer). „Passt nicht zum
    /// Vertrag“ nur, wenn der Payload nicht zum Hash passt (falsche REST-Daten).
    #[test]
    fn a13_eingang_nennt_die_gebundene_nachricht() {
        use message::{Found, Origin, TresorCheck};
        let owner: [u8; 32] = xonly(&test_key(1)).try_into().unwrap();
        let entry = |origin: Origin| message::Inbox {
            txid: "22".repeat(32),
            time_ms: None,
            amount: 10 * E8,
            unit: "KAS",
            from: vec![],
            found: Found::Private("Miete".into()),
            origin,
            source: message::Source::Node,
        };
        let prefix = kaspa_addresses::Prefix::Mainnet;
        let known = |check| Origin::Tresor { id: Some("abababab".into()), owner: Some(owner), check };
        let (line, j) = inbox_item(&entry(known(TresorCheck::AsStored)), prefix);
        assert_eq!((j["origin"].as_str(), j["tresor"].as_str()), (Some("tresor-stored"), Some("abababab")));
        assert!(line.contains("vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)"), "{line}");
        let (line, j) = inbox_item(&entry(Origin::Tresor { id: None, owner: None, check: TresorCheck::Bound }), prefix);
        assert_eq!((j["origin"].as_str(), j["tresor"].is_null(), j["tresorOwner"].is_null()), (Some("tresor-bound"), true, true));
        assert!(line.contains("vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)") && line.contains("Besitzer nicht geprüft"), "{line}");
        assert!(!line.contains("eingefügt") && !line.contains("nicht prüfbar"), "{line}");
        let (line, j) = inbox_item(&entry(known(TresorCheck::Bound)), prefix);
        assert_eq!(j["origin"].as_str(), Some("tresor-bound"));
        assert!(line.contains("anders als die Beschreibung im Tresor-Code"), "{line}");
        let (line, j) = inbox_item(&entry(known(TresorCheck::Inserted)), prefix);
        assert_eq!(j["origin"].as_str(), Some("tresor-inserted"));
        assert!(line.contains("passt NICHT zur im Vertrag gebundenen Nachricht"), "{line}");
    }

    /// Restpunkt „Import-alter-Tresor-Codes“: Die Liste zeigt die
    /// Beschreibung eines älteren Tresors bereinigt und gekennzeichnet; der
    /// Code bleibt der des Absenders (unverändert).
    #[test]
    fn a13_tresor_liste_zeigt_alte_beschreibung_bereinigt() {
        let mut rec = knapper_tresor(&mut Sim::new());
        rec.message = "Miete\u{2028}Mai 1\u{fe0f}\u{20e3}".into();
        // öffentlich: eine Beschreibung braucht eine Nachricht in den Zahlungen (A13-tresor-2)
        rec.onchain = true;
        rec.params.payload_hash = payload_hash(&rec.payload());
        let j = tresor_json("mainnet", &rec);
        assert_eq!(j["message"], "Miete Mai 1\u{20e3}");
        assert_eq!(j["messageCleaned"], true);
        assert_eq!(TresorCode::decode(j["code"].as_str().unwrap()).unwrap().message, rec.message, "Code unverändert");
        rec.message = "Miete Februar".into();
        let j = tresor_json("mainnet", &rec);
        assert_eq!((j["message"].as_str(), j["messageCleaned"].as_bool()), (Some("Miete Februar"), Some(false)));
    }

    /// A13-tresor-1: Die Liste sagt, ob jede Zahlung die Beschreibung
    /// nachweislich trägt (`messageCheck`); mit dem Schlüssel des Empfängers
    /// (`tresor list --key`) wird eine verschlüsselte Nachricht geprüft.
    /// `recipient_key` findet die Schlüsseldatei des Empfängers in keys/.
    #[test]
    fn a13_tresor_liste_prueft_verschluesselte_beschreibung() {
        let mut rec = knapper_tresor(&mut Sim::new());
        let (owner, recipient) = (test_key(1), test_key(2));
        let blob = message::encrypt(&xonly(&recipient), "Miete Februar").unwrap();
        (rec.message, rec.sealed) = ("Ab jetzt an kaspa:qneu".into(), faster_hex::hex_string(&blob));
        rec.params.payload_hash = payload_hash(&blob);
        let sk = SecretKey::from_keypair(&recipient);
        let check = |r: &TresorRec, k: Option<&SecretKey>| tresor_json_with("mainnet", r, k)["messageCheck"].as_str().unwrap().to_string();
        assert_eq!(check(&rec, None), "unchecked", "übernommen, nicht geprüft");
        assert_eq!(check(&rec, Some(&sk)), "mismatch", "Fälschung erkannt");
        assert_eq!(check(&rec, Some(&SecretKey::from_keypair(&owner))), "unchecked", "fremder Schlüssel prüft nichts");
        rec.message = "Miete Februar".into();
        assert_eq!(check(&rec, Some(&sk)), "checked");
        rec.key = Some("keys/owner.json".into());
        assert_eq!(check(&rec, None), "checked", "hier angelegt");
        // Schlüsseldatei des Empfängers in einem keys-Verzeichnis
        let dir = std::env::temp_dir().join(format!("ghost-a13-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, k) in [("a-owner.json", &owner), ("b-empfaenger.json", &recipient)] {
            let secret = faster_hex::hex_string(&k.secret_bytes());
            std::fs::write(dir.join(name), serde_json::to_string(&KeyFile { secret }).unwrap()).unwrap();
        }
        std::fs::write(dir.join("kaputt.json"), "{").unwrap();
        assert_eq!(recipient_key(&dir, &xonly(&recipient)), Some(sk));
        assert_eq!(recipient_key(&dir, &xonly(&test_key(3))), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Audit 14 H2/H-1: der Agent aktualisiert vor der Einfrier-Frist
    #[test]
    fn herzschlag_vor_der_einfrier_frist() {
        assert_eq!(heartbeat_min(2 * HOUR_DAA, 360.0), 60.0, "echtes v4: Frist 2 h → spätestens nach 60 min");
        assert_eq!(heartbeat_min(HOUR_DAA, 360.0), 30.0, "Probe: Frist 1 h → 30 min");
        assert_eq!(heartbeat_min(2 * HOUR_DAA, 20.0), 20.0, "kürzeres --max-age-min bleibt");
    }

    /// Audit 14 H3/M-1/N-2: die Zinsregel schlägt nur Schritte vor, die der Vertrag annimmt
    #[test]
    fn zinsschritt_passt_zum_vertrag() {
        let p = OracleParams {
            reg_cov: kaspa_consensus_core::Hash::from_bytes([0; 32]),
            max_rate: rate_from_apr(math::RATE_MAX_PCT),
            rate_step: rate_from_apr(math::RATE_STEP_PCT) + 1,
            rate_gap_daa: HOUR_DAA,
            freeze_after_daa: 2 * HOUR_DAA,
        };
        let cur = OracleState { kas_usd: 4_000_000, oracle_daa: 1_000_000, seq: 1, stable_rate: rate_from_apr(3.3), stable_index: 1_000_000_000, frozen: false, last_rate_daa: 1_000_000 };
        let ok = |r: i64, daa: i64| ops::check_update(&p, &cur, cur.kas_usd, daa, r).is_ok();
        // neben dem Raster: 3,3 → 4,0 wären 0,7 Punkte; begrenzt auf 0,5
        let later = cur.last_rate_daa + HOUR_DAA;
        let (r, note) = fit_rate(&p, &cur, rate_from_apr(4.0), later);
        assert!(note.is_some() && ok(r, later), "begrenzter Schritt nimmt der Vertrag an");
        assert!(!ok(rate_from_apr(4.0), later), "Gegenprobe: unbegrenzt lehnt er ab");
        // jeder Schritt der Zinsregel vom Raster aus passt
        for pct in [0.0, 0.5, 5.0, 19.5, 20.0] {
            let c = OracleState { stable_rate: rate_from_apr(pct), ..cur };
            for next in [pct - 0.5, pct + 0.5] {
                if (0.0..=20.0).contains(&next) {
                    let (r, note) = fit_rate(&p, &c, rate_from_apr(next), later);
                    assert!(note.is_none() && ops::check_update(&p, &c, c.kas_usd, later, r).is_ok(), "{pct} → {next}");
                }
            }
        }
        // Pause des Vertrags (DAA) noch nicht um: Zins bleibt, Preis geht
        let (r, note) = fit_rate(&p, &cur, rate_from_apr(3.8), later - 1);
        assert_eq!(r, cur.stable_rate);
        assert!(note.is_some() && ok(r, later - 1));
    }

    fn test_deployment() -> Deployment {
        let k = new_key();
        let set = SignerSet { keys: vec![xonly(&k)], t: 1, t_rot: 1 };
        let rp = deploy_register_params(&k, false);
        let reg = RegisterState::genesis(&set, None, 0, [0; 32], 1_000);
        let os = OracleState { kas_usd: 4_000_000, oracle_daa: 1_000, seq: 1, stable_rate: 0, stable_index: 1_000_000_000, frozen: false, last_rate_daa: 1_000 };
        let t = |s| ops::Tracked { outpoint: kaspa_consensus_core::tx::TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([1; 32]), 0), value: 1, cov: kaspa_consensus_core::Hash::from_bytes([2; 32]), state: s };
        Deployment {
            network: "mainnet".into(),
            register_params: rp,
            register: t(reg),
            signer_set: set,
            fallback_set: None,
            rotation: None,
            old_tickets: vec![],
            foreign_change: None,
            signers_unknown: false,
            oracle_params: OracleParams { reg_cov: kaspa_consensus_core::Hash::from_bytes([2; 32]), max_rate: 1, rate_step: 1, rate_gap_daa: HOUR_DAA, freeze_after_daa: 2 * HOUR_DAA },
            oracle: ops::Tracked { outpoint: kaspa_consensus_core::tx::TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([3; 32]), 0), value: 1, cov: kaspa_consensus_core::Hash::from_bytes([4; 32]), state: os },
            factory_params: FactoryParams { deployer: xonly(&k), ghost_tpl: ghost_template() },
            factory: ops::Tracked { outpoint: kaspa_consensus_core::tx::TransactionOutpoint::new(kaspa_consensus_core::Hash::from_bytes([5; 32]), 0), value: 1, cov: kaspa_consensus_core::Hash::from_bytes([6; 32]), state: FactoryState::uninitialized() },
            ghost_root: None,
            vault_params: None,
            vaults: vec![],
            tokens: vec![],
            pool: None,
            pool_pending: None,
            lp_tokens: vec![],
            pool_unresolved: None,
        }
    }

    /// Audit 15: der Herzschlag steckt in der Update-Entscheidung, nicht nur in einer Hilfsfunktion
    #[test]
    fn update_vor_der_einfrier_frist_und_bei_eingefrorenem_orakel() {
        let mut d = test_deployment();
        let p = d.oracle.state.kas_usd;
        assert!(feed_reason(&d, 59.0, 0.005, 360.0, p).is_none(), "ruhiger Kurs, 59 min");
        assert!(feed_reason(&d, 61.0, 0.005, 360.0, p).is_some(), "nach 60 min fällig, obwohl --max-age-min 360");
        assert!(feed_reason(&d, 1.0, 0.005, 360.0, p + p / 100).is_some(), "1 % Bewegung");
        d.oracle.state.frozen = true;
        assert!(feed_reason(&d, 1.0, 0.005, 360.0, p).unwrap().contains("eingefroren"), "eingefroren: sofort fällig");
    }

    #[test]
    fn startzins_im_rahmen_und_raster() {
        for ok in [0.0, 0.5, 5.0, 20.0] {
            assert!(check_start_rate(ok).is_ok(), "{ok}");
        }
        for bad in [-0.5, 3.3, 20.5, 25.0, f64::NAN] {
            assert!(check_start_rate(bad).is_err(), "{bad}");
        }
    }

    /// Audit 15 G-7: jede ältere Zustandsdatei wird mit ihrer Version abgewiesen
    #[test]
    fn aeltere_zustandsdateien_werden_erkannt() {
        let v4 = serde_json::to_value(test_deployment()).unwrap();
        assert_eq!(old_version(&v4), None);
        assert_eq!(old_version(&serde_json::json!({ "vault_params": { "treasury": "aa", "max_debt": 1 } })), Some("3"));
        assert_eq!(old_version(&serde_json::json!({ "vault_params": { "max_debt": 1 } })), Some("2"));
        assert_eq!(old_version(&serde_json::json!({ "vault_params": { "mcr_bps": 1 } })), Some("1"));
        assert!(old_version_error(Path::new("deployments/mainnet.json"), "3").contains("Version 3"));
    }

    /// Audit 14 H1: die Restwartezeit wird negativ, statt bei 0 zu enden
    #[test]
    fn restwartezeit_wird_negativ() {
        let mut d = test_deployment();
        let set = SignerSet { keys: vec![vec![9; 32]], t: 1, t_rot: 1 };
        d.rotation = Some(ops::Rotation { ticket: d.register.clone(), set, fallback: None, emergency: false, ready_daa: 100_000 });
        let j = signers_json(&d, 100_000 + HOUR_DAA as u64);
        assert_eq!(j["rotation"]["readyInHours"], -1.0);
        let j = signers_json(&d, 100_000 - HOUR_DAA as u64);
        assert_eq!(j["rotation"]["readyInHours"], 1.0);
    }
}
