# Audit 3 — Preis- und Zinsorakel, Ende zu Ende

Stand 28.09.2026, 11:50 UTC. Unabhängige Prüfung, nur lesend. Keine Repository-Datei wurde verändert, keine Transaktion gesendet, `keys/` nicht gelesen.

**Umfang:** `contracts/risk_oracle.sil`, `protocol/src/price.rs`, `protocol/src/bin/ghostctl.rs` (`oracle-update`, `oracle-feed`, `feed_due`), `protocol/src/ops.rs` (`oracle_update`, `oracle_read_input`), `contracts/stable_vault.sil` (`readOracle`), `GHOST-Orakel starten.command`, `deployments/mainnet.json` (nur öffentliche Werte). Dazu SilverScript v1.0.0 (Compiler-Absenkung von `tx.daa`, `validateOutputState`) und rusty-kaspa a41a333 (CLTV, Finalität, Mempool-RBF, Covenant-Kontext).

**Ausführbare Beweise:** `audit/3-orakel-beweise.rs` (Kopie von `…/scratchpad/audit-oracle/protocol/tests/audit_oracle.rs`), Lauf in `audit/3-orakel-beweise-lauf.txt`. Sie laufen gegen die echte Skript-Engine und die lokale Kette (`sim.rs`) mit den Mainnet-Parametern (3-von-5, maxRate 1e9, 5 % p. a., MCR 200 %, Liquidation 150 %, Bonus 10 %). Alle 5 Beweise grün.

**Netzstand beim Prüfen (api.kaspa.org, lesend):** Orakel-UTXO liegt unverändert auf `368cc2d8…5151:1` (seq 0, 10 KAS), Block-DAA 551 728 764; virtueller DAA 551 759 018, also ≈ 30 000 DAA ≈ 50 min alt. On-chain 0,047740 USD, Markt 0,04757 USD (−0,35 %, innerhalb der 1 %-Schwelle des Feeds). Ob der Feed läuft, lässt sich daraus nicht ablesen.

---

## Übersicht

| Nr. | Schwere | Befund | Beleg |
|---|---|---|---|
| O-1 | **Hoch** | Jeder kann die Orakel-UTXO für 0,0031 KAS per `read()` ausgeben. `ghostctl`/Feed haben keine Resynchronisation: danach steht der Feed dauerhaft („Orakel-UTXO nicht mehr vorhanden"), der Preis friert ein. Fortlaufendes `read()`-Spam blockiert außerdem alle Vault-Aktionen inkl. Liquidationen. | Beweis A, Code |
| O-2 | **Hoch** (öffentl. Betrieb) / Mittel (jetzt) | Komitee kann jeden Preis > 0 setzen, keine Sprung- oder Bereichsgrenze. Mit 3 Schlüsseln: alle Vaults zum Fantasiepreis liquidieren oder unbegrenzt prägen. Alle 5 Schlüssel liegen auf einem Rechner, der Daemon sendet mit `--ja`. Keine Schlüsselrotation, keine Pause möglich. | Beweis C, Code |
| O-3 | Mittel | Feed-Zustand wird erst nach Bestätigung gespeichert; Zeitlimit 180 s je Runde, `wait_accepted` 120 s, Preis wird doppelt abgerufen (2 × bis 48 s). Ein Abbruch nach dem Senden führt zum selben Stillstand wie O-1. Kein Alarm außer stderr im Terminalfenster. | Code, Rechnung |
| O-4 | Mittel | Ein signiertes Update ist bis zum nächsten seq ein Inhaberpapier: Wer das Sigscript sieht (Mempool), kann es mit eigenen Gebühren einreichen. Signiert der Feed nach Timeout dieselbe seq neu, entscheidet ein Dritter, welcher Preis landet; das gewollte Update scheitert, Feed steht. | Beweis E |
| O-5 | Mittel | Keine Frischeprüfung on-chain (bekannt, B1). Feed-Heartbeat 6 h; die in ARCHITEKTUR 2.1 genannte `OpChainblockSeqCommit`-Notbremse ist nicht umgesetzt. Vault prägt zu beliebig altem Preis. | Code |
| O-6 | Mittel | Preisfeed: 2 korrelierte Aggregatoren + 4 Börsen (Last Trade, USDT, 24-h-Umsatz ≈ 3 M USD je Börse), eine Stichprobe je Runde, kein TWAP. 3 manipulierte Quellen verschieben den Median um bis zu ≈ 1,45 %. USDT≈USD ungeprüft. Kein Plausibilitätsband gegen den letzten On-chain-Preis trotz `--ja`. | Rechnung, API-Abfrage; Manipulationskosten UNVERIFIZIERT |
| O-7 | Niedrig | Zinsrechnung korrekt (5,0000 % bei Jahres-Update). Effektiver Satz hängt von der Update-Frequenz ab (5-min-Takt: 5,11 %). Überlauf bricht sauber ab, wäre aber ohne Ausweg (alter Satz zählt) — praktisch erst nach Jahrzehnten. | Beweis D, bestehende Tests |
| O-8 | Niedrig | Feed nutzt bis zu 8 Owner-UTXOs als Gebühren-Inputs; parallele Vault-Aktionen des Owners kollidieren. Vertrauen in öffentliche Resolver-Nodes betrifft nur Verfügbarkeit. | Code |
| — | kein Befund | Domänentrennung (covId in der Nachricht), Replay über seq, Signierer-Dedup, Wertbindung, Parameterbindung über `validateOutputState`, Locktime-Domäne (Compiler erzwingt DAA < 5e11), `tx.daa`-Semantik. | Beweis B, Code, bestehende Tests |

---

## O-1 (Hoch) — `read()` durch Dritte: Stillstand des Feeds und Blockade aller Vault-Aktionen

**Ort:** `contracts/risk_oracle.sil:56-65` (`read()` ohne jede Bedingung außer Werterhalt), `protocol/src/bin/ghostctl.rs:314-321` (`check_fresh`), `:744-748` (`oracle_update` bricht bei fremdem Outpoint ab), `:492-516` (Feed-Schleife ohne Resynchronisation), `MAINNET.md` („ein Nachladen gibt es noch nicht").

**Szenario 1 — Feed-Stillstand (ein einziges `read()`):**
1. Ein Dritter gibt die Orakel-UTXO per `read()` aus und erzeugt sie unverändert neu. Das Skript verlangt nur `OpAuthOutputCount == 1` und Werterhalt. Gebühr: **311 325 sompi = 0,0031 KAS** (Beweis A; compute 2 965 g, transient 4 940 g, storage 5 850 g).
2. `deployments/mainnet.json` zeigt weiter auf den alten Outpoint. Jede Runde von `oracle-feed` ruft `check_fresh` → `Err("Orakel-UTXO nicht mehr vorhanden …")`. Der Daemon läuft weiter, schreibt alle 5 Minuten eine Fehlerzeile auf stderr und sendet nie wieder ein Update.
3. Der On-chain-Preis friert ein. Fällt der Markt, prägen Vault-Besitzer weiter zum eingefrorenen Preis (Vault prüft kein Alter, O-5); steigt er, sind Vaults nicht liquidierbar, obwohl sie es sein müssten.
4. Auch `mint`, `withdraw`, `repay`, `liquidate` in `ghostctl` bauen auf demselben Outpoint und scheitern („Zustandsdatei ist veraltet"). Ein Dritter, der mit eigenem Werkzeug liquidiert oder einfach nur liest, löst dasselbe aus — das ist kein Angriff, das passiert im Normalbetrieb, sobald es einen zweiten Teilnehmer gibt.

**Beweis A** (`a_fremdes_read_macht_zustandsdatei_und_update_unbrauchbar`): Dritter mit 1 KAS führt `read()` aus (angenommen), danach baut das Komitee-Update auf der alten Zustandsdatei lokal noch, wird aber abgelehnt: `Input 0: UTXO nicht vorhanden (schon ausgegeben?)`. Der echte Node antwortet entsprechend mit „missing outpoint".

**Szenario 2 — Dauerblockade (Griefing):**
- Normale `submit_transaction` läuft im Node mit `RbfPolicy::Forbidden` (`protocol/flows/src/flow_context.rs:690`): Ein zweiter Spender derselben UTXO wird als Double-Spend abgelehnt. Wer die frische Orakel-UTXO zuerst im Mempool hat, gewinnt.
- Ein Angreifer beobachtet den Mempool oder spendet einfach nach jedem Block die jeweils aktuelle Orakel-UTXO erneut. Jede Vault-Aktion (0,04–0,05 KAS, ≈ 21 KB, Bauzeit mit Skript-Probelauf im Sekundenbereich) verliert das Rennen gegen ein 1 KB-`read()` für 0,003 KAS. Bei 10 Blöcken/s kostet die Vollblockade ≈ 0,031 KAS/s ≈ 2 700 KAS/Tag (≈ 130 USD); gezieltes Kontern einzelner Opfer-Transaktionen kostet 0,003 KAS je Versuch, das Opfer 15-mal mehr.
- Ein Vault-Besitzer kann seine eigene Liquidation so verzögern, bis er getilgt hat (oder bis der Preis wieder steigt). Liquidatoren tragen das Risiko, Prozess-Bad-Debt entsteht.
- Auch Komitee-Updates sind Opfer: Sie referenzieren denselben Outpoint.

**Bewertung:** Der Entwurf „Spend-to-read" macht die Orakel-UTXO zur globalen, unautorisierten Ressource. Kombiniert mit dem fehlenden Nachladen ist die Verfügbarkeit des gesamten Protokolls von jedem beliebigen Dritten für 0,003 KAS abschaltbar. Für den jetzigen Ein-Personen-Betrieb bedeutet das: Sobald irgendjemand die Verträge findet und einmal liest, steht das Mainnet-Orakel still, bis die Zustandsdatei von Hand repariert wird — und dafür gibt es kein Werkzeug.

**Empfehlung:**
1. `ghostctl` muss den Orakel-Zustand aus der Kette laden können (UTXO der P2SH-Adresse suchen, Zustand aus dem Redeem-Skript des letzten Spenders bzw. per `OpTxOutputSpk`-Vergleich der bekannten Zustände; kurzfristig: Kandidaten `seq`, `kasUsd` etc. aus dem Sigscript der ausgebenden Tx rekonstruieren). Der Feed muss bei `check_fresh`-Fehler neu synchronisieren statt aufzugeben.
2. Gegen die Blockade: `read()` beschränken (z. B. nur zusammen mit einem Vault-Input derselben Tx, also `OpCovInputCount(vaultTemplate…) ≥ 1` ist im Orakel nicht prüfbar, aber „Output-Anzahl ≥ 2" oder eine Mindestgebühr/ein Pfand im Orakel sind möglich), oder das Orakel replizieren (mehrere gleichwertige Orakel-UTXOs, der Vault akzeptiert jede mit gleichem `seq`), oder Vaults die Orakel-UTXO nicht ausgeben lassen, sondern nur einen *Beweis* (signierte Nachricht mit `tx.daa`-Untergrenze und `OpChainblockSeqCommit`-Obergrenze, O-5). Das ist eine Designentscheidung, keine Kleinigkeit.
3. Bis dahin: Feed-Alarm (E-Mail/Push), wenn `check_fresh` zweimal hintereinander scheitert.

## O-2 (Hoch bei öffentlichem Betrieb, Mittel im jetzigen Probelauf) — Komitee ohne Preisgrenze, Schlüssel auf einem Rechner

**Ort:** `risk_oracle.sil:70-76` (nur `newKasUsd > 0`, keine Ober-/Untergrenze, kein Sprunglimit), `ghostctl.rs:361-366` (`committee-keygen` legt alle 5 Schlüssel in eine Datei), `GHOST-Orakel starten.command` (`--ja`, Komitee-Datei auf demselben Mac wie der Owner-Schlüssel), `MAINNET.md` („Damit bist du allein das Orakel").

**Szenario:** Wer 3 der 5 Komitee-Schlüssel hat — im jetzigen Aufbau: wer `keys/mainnet-committee.json` liest (Malware, Backup, Time Machine, Sync-Ordner) — kann
- den Preis auf 1 sompi-USD setzen und jeden Vault mit Schuld liquidieren: **Beweis C**: Vault mit 150 KAS (≈ 7,16 USD zum echten Kurs) geht für ≈ 1 GHOST an den Liquidator (+149,95 KAS). Das lässt sich in einer einzigen Transaktion mit dem Update kombinieren (Vault liest den Input-Zustand, O-10), also ohne Reaktionsfenster für Besitzer.
- den Preis auf 1e18 setzen und unbegrenzt GHOST prägen (Sicherheitenwert 10^10 USD/KAS). Ab ≈ 9,2e10 (920 USD/KAS) laufen die Vault-Rechnungen über: `withdraw` → `NumberTooBig`, `liquidate` → „gesund" (Beweis C). `repay` und `close` funktionieren weiter (brauchen den Preis nicht), der Ausstieg bleibt also möglich. Eine `mint`-Grenze existiert nur durch MAX_DEBT_SHARES.
- Zins: durch `maxRate` (1e9 = 31,5 % p. a.) begrenzt, der Index wächst je Update höchstens um `rate·Δ` — die Aussage in ARCHITEKTUR 2.1 („kann die Schulden nicht beliebig aufblähen") stimmt für den Zins, **nicht für den Preis**.

**Weitere Punkte:**
- Es gibt keine Möglichkeit, Komitee-Schlüssel zu tauschen: `signer0..4`, `threshold`, `maxRate` sind Konstruktor-Parameter und stehen im Template-Prefix; der Vault prüft `oracleTemplateHash`. Ein neues Komitee heißt neues Orakel, neue Factory, neue Vaults.
- Es gibt keinen Pausen-/Notschalter, weder im Orakel noch im Vault.
- Die drei Signierer sind fest `[0,1,2]` (`ghostctl.rs:762`); Schlüssel 3 und 4 sind ungenutzt. Das ist harmlos, zeigt aber, dass 3-von-5 derzeit Kosmetik ist.

**Empfehlung:** Im Vertrag ein Sprunglimit je Update (z. B. ±20 % relativ zum alten Preis, plus absolute Grenzen 1e4 ≤ kasUsd ≤ 9e10) — ein kompromittiertes Komitee braucht dann viele Updates über Zeit, was Wächtern ein Fenster gibt. Schlüssel physisch trennen (mindestens 2 der 3 aktiven Signierer auf anderen Geräten, Signatur über eine kleine Signier-API statt Schlüsseldatei im Daemon). Für den Probelauf: Komitee-Datei nicht im Projektordner, nicht in Backups/Cloud-Sync.

## O-3 (Mittel) — Feed-Zustand: Abbruch nach dem Senden wedgt den Feed

**Ort:** `ghostctl.rs:279-308` (`send`: submit → `wait_accepted` 120 s → erst dann `save`), `:512-514` (Runde nach 180 s abgebrochen), `:744-767` (`oracle_update` ruft die Preise erneut ab, obwohl `feed_due` sie eben schon geholt hat), `net.rs:100-111`.

**Ablauf einer Runde (Worst Case):** `connect` ≤ 15 s + `get_server_info` + `feed_due` (DAA + `fetch_all` 6 × 8 s = 48 s) + `oracle_update` (`check_fresh`, **nochmals** `fetch_all` 48 s, DAA, `funds`, Bau, submit) + `wait_accepted` ≤ 120 s. Das sind > 230 s bei einem Limit von 180 s. Das Zeitlimit ist also im Störungsfall erreichbar, und genau dann tritt der schlimmste Fall ein:
- Tx wurde gesendet, das Future wird vor `save` abgebrochen (oder `wait_accepted` gibt nach 120 s auf, oder der RPC-Aufruf `submit_transaction` bricht nach erfolgreichem Broadcast mit Fehler ab) → Tx wird trotzdem bestätigt → Zustandsdatei veraltet → alle Folgerunden `check_fresh`-Fehler → Stillstand wie O-1.
- Wird die Tx nicht bestätigt (z. B. anderer Node in der nächsten Runde, Mempool-Abweichung), signiert die nächste Runde dieselbe `seq` neu → O-4.

Zusätzlich: Der zweite Preisabruf kann einen anderen Median liefern als der, der das Update ausgelöst hat (harmlos, aber verwirrend im Log), verdoppelt aber die Fehlerquote. Fehler landen nur auf stderr eines Terminalfensters; ein `caffeinate` verhindert zwar den Ruhezustand, aber nicht Netzwerkverlust, Node-Wechsel oder ein zugeklapptes MacBook.

**Empfehlung:** Zustand vor dem Senden als „pending" (mit TXID) schreiben; nach Neustart/nächster Runde zuerst prüfen, ob Pending-Tx bestätigt ist (UTXO der neuen Outpoint sichtbar) und dann übernehmen. Preise nur einmal je Runde abrufen. Zeitlimit so wählen, dass es größer als die Summe der inneren Timeouts ist, oder die inneren Timeouts kürzen. Alarm bei N Fehlrunden.

## O-4 (Mittel) — Signiertes Update ist bis zum nächsten `seq` ein Inhaberpapier

**Ort:** `risk_oracle.sil:78-85` (Nachricht = covId ‖ kasUsd ‖ daa ‖ seq ‖ rate, keine Bindung an Tx oder Einreicher), `ops.rs:251-297` (`sig_at: None`, keine Tx-Signatur am Orakel-Input).

**Beweis E** (`e_signiertes_update_ist_inhaberpapier_bis_seq_steigt`): Das Komitee signiert P1 = 0,04 USD (seq 1), die Tx bleibt liegen. Es signiert P2 = 0,03 USD (seq 1). Ein Dritter nimmt das Sigscript von Tx 1 (nur Datasigs, keine Tx-Signatur) als `Unlock::Raw` in eine eigene Tx mit eigenen Gebühren-UTXOs — angenommen. P2 scheitert danach („UTXO nicht vorhanden"), im echten Netz zusätzlich am `seq`.

**Folgen:**
- In Kombination mit O-3: Der Feed signiert nach Abbruch dieselbe `seq` neu; ein Beobachter (oder schlicht die Mempool-Propagation) entscheidet, ob der ältere oder neuere Preis landet. Landet der ältere, steht der Feed (Zustandsdatei erwartet P2).
- Das ist per se kein Diebstahl, weil die Signatur nur diesen einen Zustand erlaubt. Aber ein Vault-Besitzer, der eine Preissenkung im Mempool sieht, kann sie durch ein `read()` (O-1) verzögern und derweil zum alten Preis prägen/abheben — Front-Running des Orakels. Das Fenster ist die Feed-Latenz: bis 5 min Takt, 1 %-Schwelle, 6 h Heartbeat; in einem Crash können das ≥ 5–10 % sein.

**Empfehlung:** Nachricht um eine Gültigkeitsobergrenze ergänzen (`expiryDaa`, geprüft per `tx.daa`… geht nur als Untergrenze — also stattdessen `OpChainblockSeqCommit` auf einen Block ≤ N tief, siehe O-5) oder den Feed so bauen, dass eine `seq` nie zweimal signiert wird (Pending-Datei aus O-3).

## O-5 (Mittel) — Keine Frischeprüfung; Notbremse aus dem Entwurf fehlt

**Ort:** `stable_vault.sil:124-129` (`readOracle` prüft nur Covenant-ID, Template, `kasUsd > 0`), `risk_oracle.sil` (kein `OpChainblockSeqCommit`), ARCHITEKTUR 2.1 („kann das Orakel einen aktuellen Chain-Block referenzieren … ≈ 12 h").

Bekannte Einschränkung (B1), aber die im Entwurf angekündigte Notbremse ist nicht umgesetzt, und der Betrieb hängt an einem Terminalfenster (O-1, O-3). Stand des Mainnet-Orakels beim Prüfen: seq 0, ≈ 50 min alt — noch innerhalb der Regeln, aber die Regeln erlauben 6 h ohne Update und beliebig lang bei Stillstand.

**Empfehlung:** Im Vault `require(tx.daa >= o.oracleDaa)` bringt nichts (Untergrenze). Möglich ist: `update()` speichert zusätzlich einen `chainBlockHash`, und der **Vault** verlangt `OpChainblockSeqCommit(o.chainBlockHash)` — schlägt fehl, wenn der Block tiefer als `finality_depth` (≈ 12 h) liegt. Das ist grob, verhindert aber Prägen zu Tage alten Preisen. Feed-Heartbeat auf ≤ 1 h.

## O-6 (Mittel) — Preisfeed: Korrelation, dünne Märkte, kein TWAP, kein Plausibilitätsband

**Ort:** `price.rs:20-27` (Quellen), `:44-59` (Median, 3 %-Filter), `ghostctl.rs:719-735` (`feed_due`).

**Abfrage der sechs Quellen am 28.09.2026 (lesend):** api.kaspa.org 0,047573; CoinGecko 0,047566; MEXC 0,047639; Gate 0,04761 (24-h-Umsatz 3,5 M USDT); KuCoin 0,04763; Bybit 0,04759 (24-h-Umsatz 3,1 M USDT, liefert zusätzlich `usdIndexPrice` 0,0475806). Alle erreichbar.

**Befunde:**
1. **Korrelation:** api.kaspa.org und CoinGecko sind Aggregatoren, die aus denselben Börsen ableiten; effektiv gibt es 4 unabhängige Quellen plus 2 Ableitungen. Fällt eine Börse aus oder wird geo-gesperrt (Bybit, KuCoin), bleiben 5; bei zwei Ausfällen plus einem Ausreißer gibt es kein Update (fail-safe, aber Verfügbarkeit).
2. **Manipulationsspielraum (Rechnung):** Median aus 6 = Mittel aus 3. und 4. Wert. Kontrolliert ein Angreifer 3 Quellen (etwa durch gleichzeitige Trades auf 3 dünnen Börsen im Abfragemoment) und schiebt sie um +2,9 %, liegt der Median bei +1,45 %, und alle Werte bleiben innerhalb der 3 %-Toleranz. Ab ≈ +3 % fallen entweder die ehrlichen oder die manipulierten Werte heraus, und `median_price` liefert einen Fehler (kein Update). Der Schaden ist also auf ≈ 1,5 % je Update begrenzt — bei MCR 200 % irrelevant für Prägen, aber relevant für knapp gesunde Vaults (Liquidationsschwelle 150 %). Kosten der Manipulation: UNVERIFIZIERT.
3. **Stichprobe statt TWAP:** Ein Sample alle 5 min, „Last Trade" statt Mid/VWAP. Ein Docht auf drei Börsen zur selben Sekunde reicht (siehe 2).
4. **USDT ≈ USD:** 4 von 6 Quellen sind USDT-Paare, CoinGecko rechnet um. Bei einem USDT-Depeg um x % ist der Sicherheitenwert um x % falsch; die Richtung hängt vom Vorzeichen ab. Bybits `usdIndexPrice` wäre eine kostenlose USD-Referenz.
5. **Kein Plausibilitätsband gegen den On-chain-Preis:** `feed_due` löst ab 1 % Änderung aus, hat aber keine Obergrenze. Ein grober Fehler, der 3 Quellen gleichzeitig trifft (API-Formatwechsel mit falscher Einheit, gemeinsamer Upstream), wird mit `--ja` ohne Rückfrage on-chain geschrieben. `num()` akzeptiert Strings und Zahlen, prüft aber nur `> 0` und `finite`.
6. Positiv: HTTPS mit Zertifikatprüfung (ureq 2 Standard-Features), 8-s-Timeout je Quelle, HTTP-Fehlercodes werden als Fehler behandelt, `sort_by(partial_cmp).unwrap()` ist durch den `is_finite`-Filter abgesichert.

**Empfehlung:** Plausibilitätsband (z. B. ±25 % gegenüber On-chain, darüber nur mit manueller Bestätigung oder nach zwei aufeinanderfolgenden Runden), 2–3 Stichproben je Runde mit Median, Bybit-USD-Index und einen echten USD-Markt (z. B. Kraken KAS/USD, falls vorhanden — UNVERIFIZIERT) aufnehmen, Aggregatoren nur als eine Stimme zählen.

## O-7 (Niedrig) — Zinsrechnung

**Beweis D:** Mit den Mainnet-Konstanten (`stable_rate` 158 548 960) ergibt ein Jahres-Update genau 5,0000 %; im 5-Minuten-Takt 5,1146 % (Zinseszins durch die Update-Frequenz, nicht Rundung). Der effektive Satz hängt also von der Frequenz des Komitees ab (Obergrenze kontinuierlich e^0,05 − 1 = 5,13 %). Δ < 7 DAA ergibt 0 Zuwachs — nur relevant, wenn das Komitee absichtlich sekündlich aktualisiert (dann zinsfrei).

**Überlauf:** `rate·Δ` und `index·growth` brechen mit `NumberTooBig` ab (bestehender Test). Da `update()` immer den **alten** Satz für Δ anwendet und Δ nur wachsen kann, wäre das Orakel nach einem Überlauf **dauerhaft unbenutzbar** (kein Update kann den Satz senken). Bei Index ≈ 1e9 und maxRate erst nach ≈ 29 Jahren Pause, bei Index 1e10 nach ≈ 2,9 Jahren; praktisch irrelevant, aber ein `Δ = min(Δ, CAP)` (z. B. 30 Tage) wäre ein billiger Schutz.

## O-8 (Niedrig) — Betrieb

- `Net::funds(k, 8)` hängt bis zu 8 Owner-UTXOs als Gebühren-Inputs an das Orakel-Update. Gleichzeitige Vault-Aktionen des Owners (derselbe Schlüssel laut MAINNET.md) kollidieren im Mempool.
- Der Feed vertraut dem öffentlichen Resolver-Node für DAA, UTXO-Sicht und Broadcast. Ein bösartiger Node kann nur die Verfügbarkeit stören (falscher DAA → Locktime in der Zukunft → Tx nicht final → O-3/O-4), nicht Preise oder Signaturen.
- `daa − 20` als Locktime: Konsens verlangt `lock_time < Block-DAA` (`tx_validation_in_header_context.rs:79`); Blöcke mit > 20 DAA Rückstand zum virtuellen Score sind selten, die Tx bliebe dann nur kurz im Mempool. Sequence 0 erfüllt die CLTV-Bedingung (`opcodes/mod.rs:1052`). Kein Befund.

---

## Geprüft ohne Befund

- **Nachrichtenformat / Domänentrennung:** `covId(32) ‖ kasUsd(8) ‖ daa(8) ‖ seq(8) ‖ rate(8)`, alle Felder feste Länge, LE (`oracle_digest` in `contracts.rs:105`). Die Covenant-ID ist aus dem Genesis-Outpoint abgeleitet, also je Deployment und Netz einmalig: Testnet-Signaturen gelten nicht im Mainnet und umgekehrt, auch bei gleichen Schlüsseln (Test `signatur_fuer_anderes_orakel_gilt_nicht`). Ein Schnorr-Datasig über sha256(msg) kollidiert nicht mit Tx-Sighashes (blake2b-Domäne).
- **Quorum:** `sigs.length == threshold`, `signerIdx` streng aufsteigend, `idx < 5`; kein doppeltes Zählen, Schleife fest auf 5 begrenzt. `threshold = 0` würde alles freischalten — `ghostctl` setzt fest 3.
- **Replay:** `seq+1` in der Nachricht; ein bereits verbrauchtes Update ist ungültig (Test `signatur_fuer_alte_seq_ist_kein_replay`). Offen bleibt nur O-4.
- **Werterhalt und Parameterbindung:** `continuation()` verlangt genau eine autorisierte Ausgabe mit identischem Betrag; `validateOutputState` rekonstruiert `prefix ‖ state ‖ suffix` aus dem eigenen Redeem-Skript (`compile/state.rs:285-330`) — ein `read()` kann also weder Komitee noch Schwellen austauschen. Die Covenant-ID-Kontinuität erzwingt der Konsens (`covenants.rs:124-136`).
- **Locktime-Domäne (Beweis B):** Der Compiler senkt `require(tx.daa >= x)` als `OpWithin(0, 5e11) OpVerify OpCheckLockTimeVerify` ab (`compile/statement.rs:381-391`). Ein Wechsel in die Millisekunden-Domäne (der die Verzinsung um ×100 beschleunigt hätte) scheitert mit `VerifyError`. Ein Komitee kann `newOracleDaa` bis 5e11 in die Zukunft signieren; die Tx wird dann erst bei diesem DAA final — kein Schaden.
- **`tx.daa >= newOracleDaa` mit Locktime = newOracleDaa:** `stack ≤ lock_time` und `lock_time < Block-DAA` zusammen verhindern Zeitstempel aus der Zukunft; Δ ist damit an echte Zeit gebunden.
- **Vault-Lesepfad (O-10, aus Code abgeleitet, nicht ausgeführt):** `readInputStateWithTemplate` liest den **Input**-Zustand. Wird das Orakel in derselben Tx per `update()` ausgegeben, sieht der Vault den alten Preis — das ist dasselbe Ergebnis wie ein `read()` unmittelbar vor dem Update, kein zusätzlicher Schaden. Der Vault prüft Covenant-ID und Template-Hash; ein fremdes „Orakel" mit gleicher Struktur wird abgelehnt (bestehende Vault-Tests).
- **Index-Arithmetik:** Reihenfolge `rate·Δ` vor `index·growth` verhindert den frühen Überlauf; Abrundung ≤ 1e-9 je Update zugunsten der Schuldner.

---

## Priorisierte Empfehlungen

1. **Sofort (Betrieb):** Komitee-Datei vom Daemon trennen oder zumindest aus Backups/Sync fernhalten; Alarmierung bei wiederholten Feed-Fehlern; Pending-Zustand vor dem Senden schreiben (O-3).
2. **Vor jedem Betrieb mit fremden Nutzern:** Resynchronisation des Orakel-Zustands aus der Kette (O-1) — ohne sie ist das Protokoll für 0,003 KAS abschaltbar.
3. **Vertrag (nächste Version, erfordert Neudeployment):** Sprung- und Bereichsgrenze für den Preis (O-2), Δ-Kappung (O-7), Chain-Block-Referenz + `OpChainblockSeqCommit` im Vault (O-5), und eine Antwort auf das Spend-to-read-Griefing (O-1) — z. B. replizierte Orakel-UTXOs oder Beweis statt Ausgabe.
4. **Feed:** ein Preisabruf je Runde, Plausibilitätsband, TWAP aus mehreren Stichproben, USD-Referenz (O-6).
