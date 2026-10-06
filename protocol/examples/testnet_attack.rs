//! Angriff gegen den echten Testnet-10-Node: ein "deposit", das 100 KAS aus
//! dem Vault abzieht. Signaturen sind gültig, nur die Vault-Regel ist verletzt.
//! Erwartung: der Node lehnt mit einem Skriptfehler ab.
//! Aufruf: cargo run --example testnet_attack -- deployments/testnet-10.json keys/tn10-user.json 0
use kaspa_consensus_core::tx::TransactionOutput;
use kaspa_lending_protocol::contracts::*;
use kaspa_lending_protocol::net::Net;
use kaspa_lending_protocol::ops::{Deployment, p2pk_spk, xonly};
use kaspa_lending_protocol::txb::{Draft, In, Unlock, build_unverified, check_scripts};
use kaspa_consensus_core::tx::{CovenantBinding, UtxoEntry};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dep: Deployment = serde_json::from_str(&std::fs::read_to_string(&a[0]).unwrap()).unwrap();
    let kf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&a[1]).unwrap()).unwrap();
    let mut sk = [0u8; 32];
    faster_hex::hex_decode(kf["secret"].as_str().unwrap().as_bytes(), &mut sk).unwrap();
    let attacker = secp256k1::Keypair::from_seckey_slice(&secp256k1::Secp256k1::new(), &sk).unwrap();
    let i: usize = a[2].parse().unwrap();

    let net = Net::connect("testnet-10", None).await.unwrap();
    let v = &dep.vaults[i];
    let art = vault(dep.vault_params.as_ref().unwrap(), &v.owner, &v.vault.state);
    let fund = net.funds(&attacker, 4).await.unwrap();
    let mut inputs = vec![In {
        outpoint: v.vault.outpoint,
        entry: UtxoEntry::new(v.vault.value, spk(&art), 0, false, Some(v.vault.cov)),
        unlock: Unlock::Entry { art: art.clone(), entry: "deposit", args: vec![], sig_at: None },
    }];
    for (op, e) in &fund.utxos {
        inputs.push(In { outpoint: *op, entry: e.clone(), unlock: Unlock::P2pk { signer: (attacker).into() } });
    }
    let steal = 100 * 100_000_000u64;
    let outputs = vec![
        TransactionOutput { value: v.vault.value - steal, script_public_key: spk(&art), covenant: Some(CovenantBinding { authorizing_input: 0, covenant_id: v.vault.cov }) },
        TransactionOutput { value: steal, script_public_key: p2pk_spk(&xonly(&attacker)), covenant: None },
    ];
    let mut budgets = vec![20u16];
    budgets.extend(std::iter::repeat(10u16).take(fund.utxos.len()));
    let b = build_unverified(Draft { inputs, outputs, change_spk: p2pk_spk(&xonly(&attacker)), lock_time: 0 }, &budgets, 5_000_000, &net.params).unwrap();
    println!("Lokale Prüfung (zur Kontrolle): {:?}", check_scripts(&b.tx, &b.entries).err());
    match net.submit(&b).await {
        Ok(id) => println!("!!! Node hat die Angriffs-Tx angenommen: {id}"),
        Err(e) => println!("Node lehnt ab (erwartet): {e}"),
    }
}
