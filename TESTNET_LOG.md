# Testnetz-Lauf TN10 — 28.09.2026

- **Netz:** Kaspa Testnet-10, Toccata aktiv.
- **Node:** öffentlicher Resolver (wRPC/Borsh), synchron, mit UTXO-Index.
- **Werkzeug:** `ghostctl` (protocol/src/bin), Zustand in `deployments/testnet-10.json`.
- **Bestätigung:** „bestätigt" heißt, dass Ausgang 0 der Transaktion am Node als UTXO sichtbar war (`Net::wait_accepted`).
- **Explorer:** Die Links sind nicht geprüft. `explorer-tn10.kaspa.org` antwortete mit 402, `api-tn10.kaspa.org` kannte die Transaktion nicht.

**Start-Guthaben:** 10 000 tKAS vom Faucet an den Deployer (TXID 63e30503…8bcd).

| # | Aktion | Werte | Gebühr (tKAS) | TXID |
|---|---|---|---|---|
| 1 | 4 000 tKAS an Test-Nutzer | | 0,0021 | 4fc0b6f319d32facf91871bda7fb7907fec084ef4478f5217eac16ecda588133 |
| 2 | Orakel-Genesis | Median 0,047355 USD, 5 % p. a. | 0,0022 | e19cbf5429af0ff0363b055ae7ded4fb91ce4b3a1bca4c976bbcb12e84c3ac30 |
| 3 | Factory-Genesis | | 0,0022 | c25b7c7adf8f34cfc7d9ad6d0d25b1e3197a594144925e7c138958b1b6335c7e |
| 4 | Factory-Init + GHOST-Genesis | | 0,0094 | 58a0d8c7bb766b07475956f65b7e374e49d31edff04abebb8eb44ffe2926ebb2 |
| 5 | Vault 0 eröffnen (Nutzer) | 2 000 tKAS | 0,0457 | 4f68067446f848f13fb2762ebf030950f3f475988d7d1f570dc656c5c74ec736 |
| 6 | 40 GHOST prägen | Quote 236,8 % | 0,0433 | 6ea335f377573baa6cbf911eab7e4086fa77fce671d03fae2439860cd1aaba02 |
| 7 | Orakel-Update (3 von 5) | 0,047494 USD, seq 1 | 0,0065 | b5e59a48c7c323b601dc63faa542f946a51bb4916e8403213acbe7d78df80d69 |
| 8 | 15 GHOST tilgen | Schuld → 25,00001177 | 0,0471 | 31349af67d0c3f8532d430fbfa32558b3de95992501207b3f11a94b2b144c4f8 |
| 9 | Vault 1 eröffnen (Deployer) | 4 000 tKAS | 0,0457 | 994233d6ea1405c35db1ee09e1dacb75b421e06e96a827bcfc3940f87af7022b |
| 10 | 60 GHOST prägen | | 0,0433 | 571d600a5023f6fee9412e390ef1b65f3ca4649c4d3c3db85f9bb4c0d92c9764 |
| 11 | Orakel: Preissturz simuliert | 0,015 USD, seq 2 | 0,0065 | 635fd81e5ac044e9b79066c39ce03fd17a09cda06d8262083fbc8507112b56d1 |
| 12 | **Liquidation Vault 0** | 25,00001869 GHOST verbrannt, Liquidator **+1 833,29 tKAS** (erwartet 1 833,33 − Gebühr), 166,67 tKAS bleiben im Vault | 0,0471 | 2b5ca643cef6e5be8fd7b150281e8552de3e2501f4a07d02477e67d86ea51bd0 |
| 13 | Vault 0 schließen (schuldenfrei) | 166,67 tKAS zurück | 0,0364 | ee2f2934b75f24c87c83d2e0915e613908040e35c801c568a6866e0bb5bcaf1b |
| 14 | Orakel-Update | 0,047431 USD, seq 3 | 0,0065 | 91e54c84ab597e38318685124db9b6c2731fd2e598d9d28e578eb591acc19e9b |
| 15 | 25 GHOST überweisen | Nutzer → Deployer | 0,0066 | 5a6ea7e46828e8a27dece6a98561441b88820e1d82ee99d34904cc84c99a6226 |
| 16 | 59 GHOST tilgen | Schuld → 1,00002228 | 0,0512 | fcc32065d1f68e194bea46770793b27ddaa21a43b94935c15d75457e51efd89e |
| 17 | 100 tKAS einzahlen | | 0,0363 | 589f44bc35e2a1500a9c1482a6eb41f469eed2eab67ac3187b6586904e2a7448 |
| 18 | Abheben auf 3 000 tKAS | 1 100 tKAS frei | 0,0386 | 689913ae691c84b3974af439f5ade6d632e9e19df02f07da1564d613d332245f |

**Angriff gegen den echten Node** (`examples/testnet_attack.rs`):
- **Aufbau:** Ein „deposit" auf Vault 0, das 100 tKAS aus dem Vault an den Angreifer umleitet. Alle Signaturen sind gültig, das Rechenbudget ist fest vorgegeben, die lokale Prüfung wurde umgangen.
- **Antwort des Nodes:** `Rejected transaction e975b2d1…08f8: failed to verify the signature script: script ran, but verification failed`.
- **Ergebnis:** Das Netz selbst setzt die Vault-Regel durch, nicht nur `ghostctl`.

**Endstand nach Schritt 18** (danach folgten weitere Testläufe, der aktuelle Stand steht bei `./ghostctl --network testnet-10 status`): Orakel seq 3. Vault 0 gehört dem Deployer, mit 3 000 tKAS Sicherheit und 1,00002228 GHOST Schuld. Im Umlauf sind 0,99998131 GHOST.

**Beobachtungen:**
- **Gebühren:** Sie stimmen exakt mit den Werten der Simulationskette überein. Der Node hat keine einzige Transaktion wegen der Gebühr, der Masse oder der Standardregeln abgelehnt.
- **Große Transaktionen:** Die Vault-Transaktionen mit rund 21 KB, darin 17 KB Redeem-Skript, gehen im Testnetz problemlos durch.
- **Fremde Aktionen:** Liquidiert ein Dritter, veraltet die Zustandsdatei. `status` erkennt das beim Orakel. Ein automatisches Nachladen fremder Transaktionen fehlt noch.


## Mainnet-Probelauf — 28.09.2026 (vom Nutzer per `GHOST-Mainnet-Test.command` ausgeführt)

| Baustein | Covenant-ID |
|---|---|
| Orakel | 88d4852ef2ebeb84bcb9f7bccd638e04e60d2a55cd3d54514ec89a188ead24f7 |
| Factory | bd7a0d9e5286c0703b9aa8f6f672d3466823e7453da7af4ed2aa0058ccd923b9 |
| GHOST | 3b5e236f623c642780936144f825733c86957079ac043129181a90f4267f11d6 |
| Vault 0 | d7dba7c24b9c88ec3525022ef25d0a4221e2b85c65c67672af15b2444cb17c39 |

**Stand beim Nachprüfen (lesend, 28.09.2026):**
- **Orakel:** 0,047740 USD, seq 0.
- **Vault 0:** 150 KAS Sicherheit, 1 GHOST Schuld, Quote 716 %.
- **Umlauf:** 1 GHOST.
- **Owner-Adresse:** 15,90 KAS frei.
