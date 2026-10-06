# Nutzeraktionen mit Browser-Wallet (ohne Schlüssel auf dem Server)

Jeder Besucher signiert mit seiner eigenen Browser-Wallet (KasWare `signPskt`, Kastle `signTx`). Der Server hat keine Nutzer-Schlüssel: Er baut die Transaktion, die Wallet signiert, der Server prüft und sendet. Grundlage ist die Wallet-Probe (docs/wallet-probe.md). Stufe 0 der Probe ist mit KasWare im Mainnet bestanden (05.10.2026): Covenant-Eingang gültig signiert, Hashtype 0x01, geplante Skript-Einheiten = gemessen. Kastle ist nicht getestet.

## Aktionen

open-vault, mint, repay, deposit, withdraw, close, redeem, liquidate, sweep, send (KAS), transfer (GHOST), swap, pool-add, pool-remove; Tresore (Daueraufträge): tresor-open, tresor-topup, tresor-cancel (Abschnitt „Tresore“ unten).

Nicht über die Wallet: Orakel-Update, Unterzeichner-Austausch, Deployment, pool-open (Betreiber, Schlüsseldatei).

Was die Wallet signiert:

| Eingang | Art im Plan | Beispiel |
|---|---|---|
| eigene KAS (Gebühr, Einlage) | `p2pk` | jede Aktion |
| Covenant mit Besitzersignatur (`sig_at`) | `entry` | Vault mint (Pos. 3), repay (2), deposit (0), withdraw (2), close (1) |
| eigene GHOST bzw. Pool-Anteile | `leader` / `delegate` | transfer, repay, redeem, liquidate, swap (Verkauf), pool-add, pool-remove |
| Tresor mit Besitzersignatur | `entry` | tresor-topup (`topUp`, Pos. 0), tresor-cancel (`cancel`, Pos. 0) |

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

## Tresore (Daueraufträge) mit der Wallet

Vertrag `contracts/standing_order.sil` (unverändert), Rust `src/tresor.rs`, Bau `src/wallet_ops.rs` (`TresorBasis`).

```
ghostctl --json wallet build tresor-open --address kaspa:q… --to kaspa:q… --amount 10 --interval monthly \
         --start 2027-02-01 [--count 12 | --fund 100] [--max-fee 0.01] [--message=Miete --onchain-message]
ghostctl --json wallet build tresor-topup  --address kaspa:q… --tresor <Covenant-ID, 64 Hex> --kas 5
ghostctl --json wallet build tresor-cancel --address kaspa:q… --tresor <Covenant-ID, 64 Hex>
ghostctl --json tresor owned kaspa:q…        # „Meine Tresore“: nur über die Wallet angelegte, ohne Pfade
```

Seite: dieselben Routen `POST /api/wallet/build|submit`, dazu `GET /api/wallet/tresore?network=…&owner=kaspa:q…` (nur lesend, gleiche Ratenbegrenzung). Wallet-Seite im öffentlichen Modus: Abschnitt „Daueraufträge (Tresor)“ (components/WalletTresor.tsx).

- **Zustand** ist die Tresor-Datei `deployments/<netz>-tresore.json` (`tresor::path_for`), nicht deployments/<netz>.json. Sperre und Journal liegen an dieser Datei, wie bei `ghostctl tresor …`. build und submit ohne `--send` lesen nur; für Auffüllen und Kündigen wird der Tresor vorher am Node nachgeführt (`wallet_ops::follow_for_wallet`): erst nach der Besitzerprüfung und höchstens über `tresor::PUBLIC_FOLLOW` (64) Zustände; fremde, gekündigte und als fehlend markierte Tresore kosten keine Node-Abfrage (A19-2).
- **Besitzer** eines über die Wallet angelegten Tresors ist der x-only-Schlüssel der Wallet. In der Messkopie wird er wie Vaults und Token durch den Ersatzschlüssel ersetzt; Tresore, die schon dem Ersatzschlüssel gehören, bekommen einen neutralen Besitzer (A17-1).
- **Eintrag in der Tresor-Datei nur nach Erfolg:** Der Folgezustand (neuer Eintrag, nachgeführter Betrag, `ended`) kommt aus dem Neubau unter Sperre und geht über das Journal (`Ctx::send_wallet`, Merkmal `wallet`) in die Datei, also erst, wenn die selbst gebaute und geprüfte Tx angenommen ist. Nichts aus dem Plan des Browsers geht in die Datei ein außer über den bitgleichen Neubau. Die Aktion selbst stammt dort aus dem Plan; darum prüft `run_tresor` (build UND Neubau in submit) alle Regeln, auch den ersten Termin gegen die Past Median Time (`TresorBasis::pmt`): höchstens einen Tag zurück, höchstens ein Jahr voraus (`tresor::check_wallet_first_due`, A19-1).
- **Offenes Journal:** `tresor owned` zeigt eine gesendete, noch nicht übernommene Wallet-Tx mit an (`pending`); der Agent klärt ein offenes Journal der Tresor-Datei in jeder Runde, auch ohne fällige Zahlung (A19-6).
- **Grenzen (A19-3):** höchstens 10 laufende Tresore je Besitzer (`tresor::MAX_WALLET_PER_OWNER`); höchstens 1 000 *belegte* Plätze (`MAX_FILE_TRESORE`, `TresorRec::busy`: laufend, zahlbar, binnen 32 Tagen fällig) und 3 000 Einträge insgesamt (`MAX_FILE_ALL`). Ab 1 000 Einträgen fallen gekündigte und seit über einer Woche fehlende Wallet-Tresore heraus, laufende mit Guthaben nie. Jeder Tresor bindet mindestens Betrag + Höchstgebühr + 1 KAS. Ab 90 % warnt der Agent im Log.
- **Sperre belegt (A19-7):** Wartet der Agent gerade auf die Bestätigung einer Tresor-Zahlung, meldet submit „Gerade läuft eine Zahlungsrunde für Tresore …“; die Seite sperrt den Plan dann nicht und lässt erneut senden.
- **Auffüllen/Kündigen** nur der Besitzer (früh in `wallet_ops`, endgültig im Vertrag). Kündigen braucht keine eigenen KAS: die Gebühr kommt aus dem Tresor; der Plan hat dann nur den Tresor-Eingang. Mit Wallet wird `tresor::cancel` genau einmal gebaut, mit derselben Gebühr im Ausgang wie die Messkopie (`txb::wallet_fill_fee`).
- **Zahlen** (`pay`) bleibt beim Agenten (`tresor_agent_step`, `tresor::pay_round`); es braucht keine Signatur des Besitzers. Je Tresor und Runde höchstens ein Termin, fällige Tresore reihum (am längsten nicht bediente zuerst, `last_paid_ms`, A19-1). Bei Wallet-Tresoren (`TresorRec::wallet`) zahlt der Agent die Netzgebühr **nie** mit eigenem Schlüssel dazu, sie kommt nur aus dem Tresor (Höchstgebühr). Sonst ließe sich sein Guthaben über fremde Tresore mit knappem Rest aufbrauchen. Reicht das Guthaben nicht für Betrag, Höchstgebühr und 1 KAS Rest, zahlt er nicht; die Seite zeigt „Guthaben knapp“.
- **Nachricht** nur öffentlich (Klartext, im Vertrag als `payloadHash` gebunden) oder keine. Verschlüsseln ginge ohne Geheimschlüssel des Absenders (message.rs nimmt einen Einmalschlüssel), aber die verschlüsselte Fassung ist zufällig und müsste beim Neubau aus dem Plan des Browsers übernommen werden; ob sie zur Beschreibung passt, könnte der Server ohne Schlüssel des Empfängers nicht prüfen.
- **Liste** (`tresor owned`) zeigt nur über die Wallet angelegte Tresore der Adresse (keine Tresore des Betreibers, keine Pfade von Schlüsseldateien, keine Fehlertexte des Agenten). Sie ist öffentlich wie die Adresse: Wer eine Adresse kennt, sieht deren Wallet-Tresore samt Empfänger und Betrag (nach der ersten Zahlung steht das ohnehin im Redeem-Skript auf der Kette).
- Tests: tests/wallet_ops_tests.rs (Abschnitt „Tresore“), ghostctl `wallet_tresor_verdrahtung`, `wallet_tresor_agent_zahlt_keine_gebuehr_dazu`, `tresor_owned_nur_eigene_ohne_pfade`, Audit 19 (`a19_*` in tests/wallet_ops_tests.rs und ghostctl); Rückbau-Probe: sechs Tresor-Mutanten in tests/wallet_ops_mutation.sh.

## Grenzen

- GHOST und Pool-Anteile kennt der Server nur aus deployments/<netz>.json (wie `ghostctl receive`). Fremd empfangene Token muss der Betreiber erst übernehmen.
- Liegen die GHOST auf mehr als zwei UTXOs, meldet die Seite „zusammenführen“. Abhilfe: `transfer` an die eigene Adresse (bis zu 3 Eingänge).
- Höchstens 8 eigene KAS-UTXOs je Tx (die größten).
- Verschlüsselte Nachrichten werden beim Bauen erzeugt und stehen fertig im Plan.
