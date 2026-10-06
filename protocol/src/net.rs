//! Anbindung an einen Kaspa-Node per wRPC (Borsh). Ohne `--rpc` wird über den
//! öffentlichen Resolver ein Node gewählt.

use crate::ops::{Funds, p2pk_spk, xonly};
use crate::txb::Built;
use kaspa_addresses::{Address, Prefix, Version};
use kaspa_consensus_core::config::params::Params;
use kaspa_consensus_core::network::{NetworkId, NetworkType};
use kaspa_consensus_core::tx::{ScriptPublicKey, TransactionOutpoint, UtxoEntry};
use kaspa_rpc_core::RpcTransaction;
use kaspa_rpc_core::api::rpc::RpcApi;
use kaspa_txscript::standard::extract_script_pub_key_address;
use kaspa_wrpc_client::client::{ConnectOptions, ConnectStrategy};
use kaspa_wrpc_client::{KaspaRpcClient, Resolver, WrpcEncoding};
use secp256k1::Keypair;
use std::time::{Duration, Instant};

pub struct Net {
    pub client: KaspaRpcClient,
    pub network: NetworkId,
    pub prefix: Prefix,
    pub params: Params,
}

/// Feste Ausweich-Nodes (nur Testnetz; im Mainnet gibt es keinen verlässlich
/// erreichbaren festen Node, geprüft 29.09.2026). Unverschlüsselt (ws://): Ein
/// Node kann falsche Daten liefern, aber nichts signieren. Jede Transaktion
/// prüft ghostctl vorher selbst gegen die Verträge.
pub fn fallback_nodes(name: &str) -> &'static [&'static str] {
    match name {
        "testnet-10" => &["ws://n-testnet-10.kaspa.ws:17210"],
        _ => &[],
    }
}

/// DNS-Seeder der Kaspa-Entwickler. Sie liefern bei JEDER Abfrage andere,
/// zufällige Node-Adressen – viele davon bieten wRPC gar nicht öffentlich an.
/// Der Name als Node-Adresse war daher Glückssache (29.09.2026: abwechselnd
/// erreichbar und „Connection timeout“). Deshalb: alle gelieferten Adressen
/// sammeln und nur die versuchen, deren wRPC-Port wirklich antwortet.
pub fn seeders(name: &str) -> (&'static [&'static str], u16) {
    match name {
        "mainnet" => (&["seeder1.kaspad.net", "seeder2.kaspad.net", "seeder3.kaspad.net", "seeder4.kaspad.net"], 17110),
        "testnet-10" => (&["seeder1-tn.kaspad.net"], 17210),
        _ => (&[], 0),
    }
}

/// Adressen aller Seeder, deren wRPC-Port antwortet (IPv4; im lokalen Netz war
/// IPv6 gestört). DNS mehrfach, weil jede Antwort andere Nodes enthält.
async fn seeder_candidates(name: &str) -> Vec<String> {
    let (hosts, port) = seeders(name);
    let mut addrs: Vec<std::net::SocketAddr> = vec![];
    for _ in 0..2 {
        for h in hosts {
            if let Ok(Ok(found)) = tokio::time::timeout(Duration::from_secs(4), tokio::net::lookup_host((*h, port))).await {
                for a in found {
                    if a.is_ipv4() && !addrs.contains(&a) {
                        addrs.push(a);
                    }
                }
            }
        }
    }
    let mut probes = tokio::task::JoinSet::new();
    for a in addrs {
        probes.spawn(async move {
            let ok = matches!(tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(a)).await, Ok(Ok(_)));
            ok.then_some(a)
        });
    }
    let mut open = vec![];
    while let Some(r) = probes.join_next().await {
        if let Ok(Some(a)) = r {
            open.push(format!("ws://{a}"));
        }
    }
    open
}

pub fn network_id(name: &str) -> Result<NetworkId, String> {
    match name {
        "mainnet" => Ok(NetworkId::new(NetworkType::Mainnet)),
        "testnet-10" => Ok(NetworkId::with_suffix(NetworkType::Testnet, 10)),
        _ => Err(format!("unbekanntes Netz {name} (mainnet | testnet-10)")),
    }
}

impl Net {
    /// Ohne `url`: erst der öffentliche Resolver, dann feste Ausweich-Nodes.
    /// Am 28.09.2026 lieferten alle Nodes hinter dem Resolver (kaspa.stream/
    /// .red/.green/.blue) 502, während die Seeder der Kaspa-Entwickler liefen.
    pub async fn connect(name: &str, url: Option<&str>) -> Result<Self, String> {
        if url.is_some() {
            return Self::connect_one(name, url, 3).await;
        }
        let mut errs = vec![];
        // Ein gesunder Resolver-Node verbindet in 1–2 s; ein toter hielte sonst
        // jeden Aufruf fast eine Minute auf.
        // Bis zu 3 Runden: Ausfälle öffentlicher Nodes sind oft nach Sekunden
        // vorbei, und jede Seeder-Abfrage liefert neue Kandidaten.
        for round in 0..3u64 {
            if round > 0 {
                tokio::time::sleep(Duration::from_secs(3 * round)).await;
            }
            match tokio::time::timeout(Duration::from_secs(8), Self::connect_one(name, None, 1)).await {
                Ok(Ok(n)) => return Ok(n),
                Ok(Err(e)) => errs.push(format!("Resolver: {e}")),
                Err(_) => errs.push("Resolver: Zeitlimit 8 s".into()),
            }
            let mut cands: Vec<String> = fallback_nodes(name).iter().map(|s| s.to_string()).collect();
            let seeded = seeder_candidates(name).await;
            if seeded.is_empty() {
                errs.push("Seeder: kein Node mit offenem wRPC-Port".into());
            }
            cands.extend(seeded);
            // höchstens 6 je Runde, jeder mit eigenem Zeitlimit
            for u in cands.iter().take(6) {
                match tokio::time::timeout(Duration::from_secs(20), Self::connect_one(name, Some(u), 1)).await {
                    Ok(Ok(n)) => return Ok(n),
                    Ok(Err(e)) => errs.push(format!("{u}: {e}")),
                    Err(_) => errs.push(format!("{u}: Zeitlimit 20 s")),
                }
            }
        }
        errs.dedup();
        Err(format!("kein Node erreichbar ({})", errs.iter().rev().take(6).cloned().collect::<Vec<_>>().join("; ")))
    }

    async fn connect_one(name: &str, url: Option<&str>, attempts: u64) -> Result<Self, String> {
        let network = network_id(name)?;
        let resolver = if url.is_none() { Some(Resolver::default()) } else { None };
        let client = KaspaRpcClient::new(WrpcEncoding::Borsh, url, resolver, Some(network), None).map_err(|e| e.to_string())?;
        let opts = ConnectOptions {
            block_async_connect: true,
            connect_timeout: Some(Duration::from_secs(15)),
            strategy: ConnectStrategy::Fallback,
            ..Default::default()
        };
        // Öffentliche Nodes brechen gelegentlich ab oder drosseln; drei Versuche
        // mit wachsender Pause (28.09.2026: wiederholte "Connection timeout").
        let mut last = String::new();
        let mut connected = false;
        for attempt in 0..attempts {
            match client.connect(Some(opts.clone())).await {
                Ok(_) => {
                    connected = true;
                    break;
                }
                Err(e) => {
                    last = e.to_string();
                    if attempt + 1 < attempts {
                        tokio::time::sleep(Duration::from_secs(2 + attempt * 4)).await;
                    }
                }
            }
        }
        if !connected {
            let _ = client.disconnect().await;
            return Err(format!("Verbindung fehlgeschlagen ({attempts} Versuch(e)): {last}"));
        }
        let info = client.get_server_info().await.map_err(|e| e.to_string())?;
        if !info.is_synced {
            return Err("Node ist nicht synchronisiert".into());
        }
        if !info.has_utxo_index {
            return Err("Node hat keinen UTXO-Index (--utxoindex)".into());
        }
        if info.network_id != network {
            return Err(format!("Node ist im Netz {}, erwartet {network}", info.network_id));
        }
        let prefix = Prefix::from(network);
        Ok(Self { client, network, prefix, params: Params::from(network) })
    }

    pub fn address_of_key(&self, k: &Keypair) -> Address {
        Address::new(self.prefix, Version::PubKey, &xonly(k))
    }

    pub fn address_of_spk(&self, spk: &ScriptPublicKey) -> Result<Address, String> {
        extract_script_pub_key_address(spk, self.prefix).map_err(|e| e.to_string())
    }

    pub async fn utxos(&self, addr: &Address) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        let r = self.client.get_utxos_by_addresses(vec![addr.clone()]).await.map_err(|e| e.to_string())?;
        Ok(r.into_iter().map(|e| (TransactionOutpoint::from(e.outpoint), UtxoEntry::from(e.utxo_entry))).collect())
    }

    /// UTXOs mehrerer Adressen mit EINER Abfrage (store::snapshot bündelt so
    /// den Abgleich von Vaults und Token, Audit 20 A20b-1/A20e-7)
    pub async fn utxos_many(&self, addrs: &[Address]) -> Result<Vec<(TransactionOutpoint, UtxoEntry)>, String> {
        if addrs.is_empty() {
            return Ok(vec![]);
        }
        let r = self.client.get_utxos_by_addresses(addrs.to_vec()).await.map_err(|e| e.to_string())?;
        Ok(r.into_iter().map(|e| (TransactionOutpoint::from(e.outpoint), UtxoEntry::from(e.utxo_entry))).collect())
    }

    /// P2PK-Guthaben eines Schlüssels; höchstens `max` größte UTXOs.
    pub async fn funds(&self, k: &Keypair, max: usize) -> Result<Funds, String> {
        let spk = p2pk_spk(&xonly(k));
        let mut u: Vec<_> = self.utxos(&self.address_of_key(k)).await?.into_iter().filter(|(_, e)| e.script_public_key == spk && e.covenant_id.is_none()).collect();
        u.sort_by_key(|(_, e)| std::cmp::Reverse(e.amount));
        u.truncate(max);
        if u.is_empty() {
            return Err(format!("keine KAS auf {}", self.address_of_key(k)));
        }
        Ok(Funds::new(k, u))
    }

    pub async fn daa(&self) -> Result<u64, String> {
        Ok(self.client.get_block_dag_info().await.map_err(|e| e.to_string())?.virtual_daa_score)
    }

    /// Past Median Time (Unix-ms): Zeit-Locktimes sind gültig, wenn sie
    /// darunter liegen (tx_validation_in_header_context.rs:79)
    pub async fn past_median_time(&self) -> Result<i64, String> {
        Ok(self.client.get_block_dag_info().await.map_err(|e| e.to_string())?.past_median_time as i64)
    }

    pub async fn submit(&self, b: &Built) -> Result<String, String> {
        let rtx = RpcTransaction::from(&b.tx);
        let id = self.client.submit_transaction(rtx, false).await.map_err(|e| format!("Node lehnt ab: {e}"))?;
        Ok(id.to_string())
    }

    /// Existiert die UTXO (noch)?
    pub async fn exists(&self, spk: &ScriptPublicKey, op: &TransactionOutpoint) -> Result<bool, String> {
        Ok(self.utxos(&self.address_of_spk(spk)?).await?.iter().any(|(o, _)| o == op))
    }

    /// Liegt die Tx im Mempool?
    /// Ok(true)/Ok(false) = im Mempool bzw. sicher nicht; Err = Node nicht
    /// befragbar (Fix-Review N-7: vorher galt jeder Fehler als "nicht im Mempool").
    pub async fn in_mempool(&self, txid: kaspa_consensus_core::Hash) -> Result<bool, String> {
        match self.client.get_mempool_entry(txid, true, false).await {
            Ok(_) => Ok(true),
            Err(e) => {
                let m = e.to_string().to_lowercase();
                if m.contains("not found") || m.contains("notfound") || m.contains("was not found") {
                    Ok(false)
                } else {
                    Err(format!("Mempool-Abfrage fehlgeschlagen: {e}"))
                }
            }
        }
    }

    /// Wartet, bis Ausgang `index` der Tx als UTXO sichtbar ist (= akzeptiert).
    pub async fn wait_accepted(&self, b: &Built, index: usize, timeout: Duration) -> Result<(), String> {
        let op = TransactionOutpoint { transaction_id: b.tx.id(), index: index as u32 };
        let spk = &b.tx.outputs[index].script_public_key;
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.exists(spk, &op).await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        Err(format!("Tx {} nach {:?} nicht bestätigt", b.tx.id(), timeout))
    }
}
