# Nutzeraktionen mit Browser-Wallet (ohne Schlüssel auf dem Server)

Jeder Besucher signiert mit seiner eigenen Browser-Wallet (KasWare `signPskt`, Kastle `signTx`). Der Server hat keine Nutzer-Schlüssel: Er baut die Transaktion, die Wallet signiert, der Server prüft und sendet. Grundlage ist die Wallet-Probe (docs/wallet-probe.md). Stufe 0 der Probe ist mit KasWare im Mainnet bestanden (05.10.2026): Covenant-Eingang gültig signiert, Hashtype 0x01, geplante Skript-Einheiten = gemessen. Kastle ist nicht getestet.

## Aktionen

open-vault, mint, repay, deposit, withdraw, close, redeem, liquidate, sweep, send (KAS), transfer (GHOST), swap, pool-add, pool-remove.

Nicht über die Wallet: Orakel-Update, Unterzeichner-Austausch, Deployment, pool-open (Betreiber, Schlüsseldatei).

Was die Wallet signiert:

| Eingang | Art im Plan | Beispiel |
|---|---|---|
| eigene KAS (Gebühr, Einlage) | `p2pk` | jede Aktion |
| Covenant mit Besitzersignatur (`sig_at`) | `entry` | Vault mint (Pos. 3), repay (2), deposit (0), withdraw (2), close (1) |
| eigene GHOST bzw. Pool-Anteile | `leader` / `delegate` | transfer, repay, redeem, liquidate, swap (Verkauf), pool-add, pool-remove |

## Ablauf

```
ghostctl --json wallet build <aktion> --address kaspa:q… [--vault N] [--kas X] [--ghost Y] [--keep K]
         [--to ADRESSE] [--message TEXT] [--onchain-message] [--min M] [--min-shares S]
         [--percent P] [--min-kas X] [--min-ghost Y]                       > plan.json
#   Browser: kasware.signPskt(p.kasware)  bzw.  kastle.signTx(p.kastle.networkId, p.kastle.txJson, p.kastle.scripts)
ghostctl --json wallet submit --plan plan.json --signed signed.json         # nur prüfen
ghostctl --json --ja wallet submit --plan plan.json --signed signed.json --send
```

Seite: `POST /api/wallet/build` `{network, action, address, params}` und `POST /api/wallet/submit` `{network, plan, signed, send?, confirmMainnet?}`. In den Formularen (Vault, Tauschen, Wallet/Senden, Liquidität) gibt es „Signieren mit: Schlüsseldatei | Browser-Wallet“; ohne Schlüsseldateien (öffentlicher Modus) nur die Wallet. Schritte: 1. Prüfen (Plan holen) → 2. In der Wallet signieren (danach prüft der Server) → 3. Senden.

## Was technisch passiert (protocol/src/wallet_ops.rs)

1. **Messkopie.** Die Aktion wird mit denselben ops/pool-Funktionen gebaut wie mit Schlüsseldatei, nur ist der Schlüssel des Nutzers überall (Vault-Besitz, GHOST, Anteile, eigene KAS, Empfänger = man selbst) durch den Ersatzschlüssel ersetzt, der lokal echt signiert. Der Ersatzschlüssel ist bei jedem Bau neu und zufällig; was ihm vorher schon gehörte (Token, Anteile, Vaults), bekommt in der Messkopie einen neutralen Besitzer, und eigene KAS der Messkopie sind nur die umgeschriebenen UTXOs des Nutzers (Audit 17 A17-1: GHOST an den früher festen Schlüssel 0x42 sperrten sonst alle GHOST-Aktionen). Tests `ersatzschluessel_ist_je_bau_zufaellig`, `a17_1_staub_an_den_ersatzschluessel_sperrt_niemanden`, `messkopie_neutralisiert_vorbesitz_des_ersatzschluessels`. Daraus kommen Skript-Einheiten, Compute-Budgets und Gebühr. Test `messkopie_entspricht_dem_bau_mit_schluessel`: gleiche Budgets, Gebühr, Massen und Ausgänge wie der Bau mit Schlüsseldatei.
2. **Echte Tx.** Signierer `txb::Signer::Wallet(x)`; an jeder Signaturstelle ein Platzhalter, Budgets und Gebühr aus der Messkopie (`txb::with_wallet_fill`). Beide Bauten müssen in Beträgen, Größen und Massen übereinstimmen (`same_shape`, Test `messkopie_gleich_echte_tx`). Die Wallet bekommt diese Tx mit leeren Signaturskripten.
3. **submit.** Zuerst, ohne Netz, ohne Abgleich und ohne Sperre: Passt die Wallet-Antwort zur unsignierten Tx des Plans, und sind alle Signaturen gültige Schnorr-Signaturen des Plan-Schlüssels mit Hashtype ALL (`wallet_ops::precheck`, A17-4)? Müll-Signaturen und Pläne für fremde Adressen enden hier. Der Plan kommt aus dem Browser und gilt als nicht vertrauenswürdig. ghostctl baut ihn aus deployments/<netz>.json und den im Plan genannten eigenen UTXOs neu. Das Ergebnis muss bitgleich sein, sonst „Plan passt nicht zum aktuellen Stand“. Weiter geht es nur mit dem Neubau. Die eigenen UTXOs müssen am Node unverändert vorhanden sein.
4. Die Wallet-Antwort muss bis auf Signaturskripte, Compute-Budgets und Speichermasse gleich sein, denn diese drei deckt der v1-Sighash nicht ab. Je Eingang wird der erste 65-Byte-Push gelesen; verlangt sind Hashtype 0x01 und eine gültige Schnorr-Signatur zur Adresse über den ALL-Sighash.
5. Dann wird die Aktion mit den echten Signaturen gebaut. Die Budgets werden nachgemessen (bei Bedarf erhöht, solange die Gebühr reicht, und neu gebaut, damit Tx-ID und Folgezustand passen). Danach wird lokal wie der Konsens geprüft: Skripte, Standard-Sigops, Blockgrenzen, Mindestgebühr.
6. Mit `--send` wird erst jetzt die Sperre genommen (höchstens 30 s Wartezeit), dann unter der Sperre abgeglichen (`load_synced`), neu gebaut und geprüft, das Journal geschrieben (`store::write_pending`, Merkmal `wallet`) und gesendet. Danach wird die Sperre freigegeben und OHNE Sperre höchstens 45 s auf die Bestätigung gewartet (`store::send_then_wait`, A17-6); der Agent wartet so nie auf eine Wallet-Tx. Bestätigt: kurz neu sperren und das Journal auflösen, also deployments/<netz>.json mit dem Folgezustand aus dem Neubau schreiben. Nicht bestätigt: Antwort `sent: true, confirmed: false, pending: true`; den Rest klärt der nächste Abgleich über das Journal.
7. **Journal einer Wallet-Tx** (`store::resolve_pending`, A17-2): Eingänge und Wechselgeld gehören dem Besucher, der sie jederzeit selbst ausgeben kann. Sind alle Eingänge verbraucht und kein Ausgang sichtbar, entscheidet daher nicht das Wechselgeld, sondern die Tx-ID: angenommen laut REST-API (`is_accepted`) → Folgezustand übernehmen; eine gleichwertige Tx mit anderer Tx-ID (gleiche Ein- und Ausgänge) angenommen → übernehmen mit deren Tx-ID; eine andere Tx gab einen Eingang aus → verwerfen; keine klare Auskunft → nach 180 s verwerfen. Der Zustand der Covenants wird danach über `resync` von der Kette nachgeführt. Ein Wallet-Journal sperrt Orakel, Keeper und Status also höchstens 180 s (bzw. solange die Tx im Mempool liegt), nie dauerhaft.

## Annahmen über die Wallets

Aus wallet.rs übernommen (für KasWare durch Stufe 0 bestätigt):
- Safe JSON von kaspa-wasm hin und zurück; Antwort als Text oder Objekt.
- Signaturskript beginnt mit `0x41 ‖ sig64 ‖ hashtype`; gelesen wird nur dieser erste Push.
- Compute-Budget und Speichermasse darf die Wallet ändern; sie werden überschrieben.

Neu für die Aktionen, im Browser noch NICHT bestätigt:
- Die Wallet signiert mehrere Eingänge einer Tx: Covenant (z. B. Vault), KCC20-Token (Leader/Delegate) und eigene P2PK-Eingänge. Die Probe hatte immer nur einen Eingang.
- Eingänge, die sie nicht signieren soll (Orakel, Factory, Pool, Reserve), lässt sie leer oder unverändert. Füllt sie sie trotzdem, wird das nur vermerkt und überschrieben, denn der Sighash deckt Signaturskripte nicht ab.
- Kastle: `scripts` enthält je Covenant-Eingang das Redeem-Skript, bei Token-Eingängen das Token-Skript (mehrere Einträge je Tx). Danach signiert `signTransaction` die eigenen P2PK-Eingänge. Nicht belegt ist, dass Kastle mehrere Einträge und große Skripte annimmt.
- Die Wallet zeigt im Dialog nur Ein- und Ausgänge. Die Seite listet vorher jeden Ausgang mit Zweck (Vertrag, Wechselgeld, andere Adresse).

Nächster Schritt in der Praxis: eine kleine Aktion mit echter Wallet im Testnet oder mit Kleinstbetrag (z. B. `send` 1 KAS an sich selbst, dann `deposit` 1 KAS in einen eigenen Vault).

## Rückbau-Probe (protocol/tests/wallet_ops_mutation.sh)

Jede Prüfung wird einzeln abgeschaltet, dann laufen die Gegenproben: alle Tests aus tests/wallet_ops_tests.rs (seit Audit 17 auch die großen), die Journal- und Sperr-Tests in store.rs und der Verdrahtungstest in ghostctl.rs. Stand 06.10.2026:

| Prüfung | Ergebnis |
|---|---|
| Plan-Vergleich (Neubau ≠ Plan) | erkannt |
| veränderte Felder der Wallet-Antwort | erkannt |
| Signatur gültig (`sig_valid`) | erkannt |
| Schnorr-Prüfung gegen die Adresse | erkannt |
| Hashtype 0x01 | überlebt: die Schnorr-Prüfung über den ALL-Sighash fängt es (zweite Linie) |
| Skriptprüfung nach dem Einsetzen | überlebt: die Signaturprüfung fängt falsche Schlüssel vorher (zweite Linie) |
| RM2 Besitzer-Filter der eigenen GHOST | erkannt (4 Tests) |
| RM3 Messkopie = echte Tx (`same_shape`) | erkannt (`messkopie_gleich_echte_tx`) |
| A17-1 Ersatzschlüssel je Bau zufällig | erkannt |
| A17-1 Vorbesitz des Ersatzschlüssels neutralisieren | erkannt (Angriff aus dem Bericht nachgestellt) |
| A17-2 Wallet-Journal über die Tx-ID / Frist / Zwilling | erkannt |
| A17-4 Vorprüfung ohne Netz, Verdrahtung, Sperre erst danach | erkannt |
| A17-6 Sperre vor dem Warten frei, Verdrahtung | erkannt |
| A17-8 Dateiname statt Pfad | erkannt |

Die Korrekturen in der Seite (A17-3, -4, -5, -7, -8, -9) sind auf dieselbe Weise geprüft (Vitest, server/walletActions.test.ts, server/prod.test.ts, src/wallet/actions.test.ts): alle 13 Rückbauten erkannt.

## Öffentliche Seite (server/api.ts, server/prod.ts)

- Ratenbegrenzung je Absender (20 je Minute). Hinter einem Webserver zählt der letzte Eintrag von X-Forwarded-For, aber nur, wenn die Verbindung von einem vertrauten Proxy kommt: Loopback und das /16 jeder Adresse aus `GHOST_HOST` (Docker-Netz) oder `GHOST_TRUSTED_PROXY`. IPv6 zählt je /64 (A17-3).
- Eigene 2 ghostctl-Plätze für die Wallet-Routen (Status und Preis behalten ihre 2), höchstens 2 submit gleichzeitig, davon 1 mit Senden; Überzählige bekommen sofort 503 „ausgelastet“ (A17-4).
- Zeitlimits: Senden 170 s in der Seite, der Webserver davor muss länger warten (nginx/Apache 180 s, Caddy ohne Limit). Zeitüberschreitung, 5xx, Netzfehler oder ein Fehler nach dem Senden heißen auf der Seite „unklar – Status prüfen, nicht erneut senden“, nie „nicht gesendet“ (A17-7).
- Größen: Plan + Antwort messen bis etwa 130 kB, die Seite nimmt bis 768 kB an, der Webserver muss mindestens das durchlassen (Vorlagen: 1 MB, A17-5).
- Fehlermeldungen ohne absolute Server-Pfade (A17-8); Content-Security-Policy in prod.ts (A17-9).

## Grenzen

- GHOST und Pool-Anteile kennt der Server nur aus deployments/<netz>.json (wie `ghostctl receive`). Fremd empfangene Token muss der Betreiber erst übernehmen.
- Liegen die GHOST auf mehr als zwei UTXOs, meldet die Seite „zusammenführen“. Abhilfe: `transfer` an die eigene Adresse (bis zu 3 Eingänge).
- Höchstens 8 eigene KAS-UTXOs je Tx (die größten).
- Verschlüsselte Nachrichten werden beim Bauen erzeugt und stehen fertig im Plan.
