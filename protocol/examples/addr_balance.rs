//! Guthaben beliebiger Adressen abfragen (nur lesend).
use kaspa_lending_protocol::net::Net;
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let net = Net::connect(&args[0], None).await.expect("Verbindung");
    for a in &args[1..] {
        let addr = kaspa_addresses::Address::try_from(a.as_str()).expect("Adresse");
        let u = net.utxos(&addr).await.expect("UTXOs");
        let total: u64 = u.iter().map(|(_, e)| e.amount).sum();
        println!("{a}: {:.8} KAS in {} UTXOs", total as f64 / 1e8, u.len());
    }
}
