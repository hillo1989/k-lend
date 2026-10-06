# Audit 11, Teil B: Off-chain-Software von GHOST Version 3

Prüfer: Claude Opus 5.5, 29.09.2026.
Gegenstand: Worktree `kaspa-lending-v3`, Branch `v3`, Commit `1ff078e`.
Nicht geprüft wurden die zum Prüfzeitpunkt uncommitteten Dateien `contracts/standing_order.sil`, `protocol/src/standing.rs` und `protocol/tests/standing_order_tests.rs` sowie die Zeile `pub mod standing` in `lib.rs`. Sie stammen offenbar aus einer parallelen Sitzung. In der Prüfkopie wurden sie auf den Stand von HEAD zurückgesetzt.

Geprüfter Umfang:
- `protocol/src/ops.rs`, `chain.rs`, `store.rs` und `math.rs`
- `protocol/src/bin/ghostctl.rs` (redeem, close, status-JSON, v2-Erkennung, Zinsregel, Keeper, Wechselwirkung mit Daueraufträgen)
- `GHOST-Umzug-v3.command`
- In `app/`: `vaultMath.ts`, `precheck.ts`, `commands.ts`, `server/actions.ts`, `Vault.tsx`, `HowItWorks.tsx` und `Faq.tsx`
- Die Frage, ob v3 frühere Behebungen aus Audit 10 wieder aufreißt

Beweistests liefen nur in einer Kopie:
- `…/scratchpad/audit11b/protocol/tests/audit11b_tests.rs` (8 Tests)
- `…/scratchpad/audit11b-v2/protocol/tests/a11_v2_liest_v3.rs` (Gegenprobe mit dem v2-Code aus `main`)

Die Ausgaben stehen im Anhang. Es wurde nichts gesendet. `keys/` und `deployments/` wurden nicht gelesen.

## Zusammenfassung

Der Kern hält. `ops.rs`, `math.rs` und `vaultMath.ts` folgen dem Vertrag Version 3 Eintrag für Eintrag. Das gilt für die Zinsabrechnung bis zum Orakelindex, für den Zins, der bei Teil-Liquidation stehen bleibt, für die Rücknahme mit 0,5 % Abschlag und DUST-Rest und für das Schließen mit Zinskasse (Ausgang 1, erst ab 0,2 KAS). `chain.rs` rekonstruiert alle v3-Einträge exakt. Das ist mit einem neuen Test belegt: Abheben mit Zins, Rücknahme durch Dritte, Teil- und Voll-Liquidation, Tilgen und Schließen mit Zinskasse. Status-JSON und Seite passen zusammen. Eine Transaktion, die der Vertrag ablehnt, wird nie gesendet, weil `txb` vorher alle Skripte lokal ausführt. Die gesamte Rust-Suite (235 Tests einschließlich meiner 8) und alle Vitest-Tests (196) laufen grün.

Die Schwächen liegen in drei Bereichen.

1. **Zinsregel (A11-O-1, mittel).** Der Zins hängt an einer einzelnen Messung des Poolkurses. Es gibt weder Mittelung noch Mindestliquidität. Das Kursband von ±3 % ist sechsmal so breit wie die Totzone von ±0,5 %. Ein Zinsschritt lässt sich für rund 0,006 USD erzwingen. Ohne Arbitrage folgt der Zins sogar nur der KAS-Kursbewegung.
2. **Rücknahme zum veralteten Orakelpreis (A11-O-2, mittel).** Der Sprungschutz des Agenten hält große Preissprünge drei Runden lang zurück. In dieser Zeit lösen GHOST-Inhaber zum alten Preis ein. Belegt ist das mit +24 % je GHOST bei einem Sprung von +25 %, und die Differenz verlieren gesunde Vault-Besitzer.
3. **Umzug (A11-O-3 bis A11-O-6).** Direkten Geldverlust habe ich nicht gefunden: Die Beträge stimmen, und im normalen, nacheinander ausgeführten Ablauf wird nichts doppelt gesendet. Das Skript stoppt aber einen laufenden Agenten nicht. Belegt ist: Ein noch laufender v2-Prozess liest die frische v3-Datei und speichert sie ohne `treasury` zurück. Danach ist v3 gesperrt, und der Umzug hängt. Dazu kommen ein zurückbleibendes Journal, ein übersprungenes Prägen (A10-A-12(e) ist wieder da) und kein Schutz gegen einen Doppelstart.

Keinen Befund der Stufe **kritisch** oder **hoch**.

## Befundtabelle

| ID | Schwere | Status | Befund | Beleg | Empfehlung |
|---|---|---|---|---|---|
| A11-O-1 | mittel | belegt | Zinsregel: eine einzelne Messung des Poolkurses aus der Zustandsdatei, keine Mittelung, keine Mindestliquidität. Das Band ±3 % ist viel breiter als die Totzone ±0,5 %. Ein Schritt kostet bei einem Pool von 1 000 KAS etwa 0,14 KAS. Ohne Handel folgt der Zins der KAS-Kursbewegung (±0,6 % reichen). | `ghostctl.rs:1626–1636`, `math.rs:146–160`, `pool.rs:109–111`; Test `a11_zinsregel_ist_mit_kleinem_tausch_verschiebbar` | Poolkurs je Runde sammeln, Median über ≥ 1 h verwenden. Regel nur ab Mindestreserve. Pool vor der Messung frisch abgleichen. Totzone größer als Gebühr plus Rauschen. |
| A11-O-2 | mittel | belegt (Simulator) | Rücknahme zahlt zum Orakelpreis. Solange der Agent einen Sprung > 20 % bestätigt (3 Runden, A10-A-7) oder nicht läuft, lösen GHOST-Inhaber zum alten Preis ein. Bei +25 % bekommt man 24,9 USD für 20 GHOST (+24,4 %), der Vault-Besitzer verliert die Differenz. | `stable_vault.sil:294–307`, `ghostctl.rs:1646–1667`; Test `a11_ruecknahme_zum_veralteten_orakelpreis` | Bei ausstehendem Aufwärtssprung sofort einen gedeckelten Zwischenschritt senden (z. B. +20 %). Vertragsseitig Frischeprüfung oder dynamischen Rücknahmeabschlag erwägen (O-5). Risiko in FAQ/So funktioniert’s nennen. |
| A11-O-3 | mittel | belegt (v2-Code) | Das Umzugsskript stoppt bzw. prüft einen laufenden Agenten (v2-Prozess) nicht. Nach `deploy` lädt v2 die v3-Datei ohne Fehler, findet die geänderte Factory nicht und speichert die Datei im v2-Format. `treasury` ist dann weg, v3-`ghostctl` meldet „stammt von Version 2“, der Umzug hängt („von Hand klären“). Wer die Datei dann löscht, legt ein zweites Deployment an (30 KAS). | `GHOST-Umzug-v3.command:41–52, 137–150`; `store.rs` (v2) `resync` + `ghostctl.rs` (v2) `load_synced` speichert bei Hinweisen; Test `a11_v2_laedt_v3_datei_und_speichert_ohne_treasury` | Vor Schritt 0 prüfen, ob ein `ghostctl agent`/`oracle-feed` läuft (`pgrep`), und abbrechen. In MAINNET.md „Agent vorher beenden“ an Schritt 0 setzen. |
| A11-O-4 | niedrig | Code-Lektüre | Beim Umbenennen bleibt das Journal `deployments/mainnet.pending.json` (v2) liegen. Sein `target` ist `deployments/mainnet.json`. v3-`deploy` (bzw. `status --json` von Seite/Agent) übernimmt es und schreibt bei „angenommen“ v2-Inhalt nach `mainnet.json`. Danach Abbruch „existiert schon“, der Umzug hängt. v2 (mit `--state …-v2.json`) sieht das Journal nicht, die letzte v2-Tx fehlt dort. | `GHOST-Umzug-v3.command:48–49`; `store.rs:74–76, 179–185`; `ghostctl.rs:760–772` | Vor dem Umbenennen abbrechen, wenn `${S3:r}.pending.json` existiert („erst `bin/ghostctl-v2 status` laufen lassen“), oder es mit `bin/ghostctl-v2 --state $S3 status` vorher klären. |
| A11-O-5 | niedrig | Code-Lektüre | Wiederaufnahme: Schritt 5 zählt **alle** Vaults. Scheiterte das Prägen nach dem Eröffnen, überspringt ein neuer Lauf das Prägen ohne Hinweis (A10-A-12(e) wieder da). Schritt 6 sendet dann die Pool-Genesis (1 KAS gebunden) und scheitert erst danach an fehlenden GHOST. | `GHOST-Umzug-v3.command:145–160`; `ghostctl.rs:1186–1206`, `pool.rs:292–294` | Eigene Vaults mit Schuld 0 erkennen und Prägen nachholen. In `pool-open` GHOST vor der Genesis prüfen. |
| A11-O-6 | niedrig | Code-Lektüre | Kein Schutz gegen gleichzeitigen Doppelstart. Beide Läufe sehen „0 Vaults“, die Sperre serialisiert nur einzelne Befehle. Folge: zwei Vaults (3 KAS Minter-Zweig nicht zurückholbar) und zweimal Prägen an Vault 0. | `GHOST-Umzug-v3.command:145–150` | Skriptweite Sperre (`mkdir`-Lock oder `flock`) am Anfang. |
| A11-O-7 | niedrig | Code-Lektüre | `redeem` führt Token zusammen und **sendet** dabei, bevor geprüft ist, ob die Rücknahme überhaupt geht (Vault unter 150 %, Betrag > Schuld, DUST). Gleiches Muster wie A10-A-9 (dort für `transfer` behoben). | `ghostctl.rs:1102–1113` (consolidate Z. 1107, Prüfung erst in `ops::redeem` Z. 1112) | `math::redemption` vor `consolidate` aufrufen, ebenso bei `liquidate` (`math::liquidation`). |
| A11-O-8 | niedrig | belegt | Der Schutz `pool_unresolved.is_none()` in `rate_plan` greift nie: Das Feld ist `#[serde(skip)]`, und `rate_plan` liest mit `Ctx::load()`. Die Zinsregel misst mit dem Pool-Stand der Datei aus der Vorrunde, bei REST-Ausfall mit beliebig altem. | `ops.rs:74–76`, `ghostctl.rs:1627–1628, 1643`; Test `a11_pool_unresolved_ueberlebt_speichern_nicht` | Pool in `rate_plan` selbst abgleichen (`pool::resync`) und bei Fehler keine Änderung. |
| A11-O-9 | niedrig | Code-Lektüre | `RATE_CLOCK` liegt nur im Speicher. Nach jedem Neustart (Startskript startet nach 30 s neu) und bei parallelem `oracle-feed` plus `agent --committee` gibt es mehr als einen Schritt je Stunde. Die Texte sagen „höchstens einmal pro Stunde“. | `ghostctl.rs:1617–1618, 1682, 1697`; `Faq.tsx:37`, `HowItWorks.tsx:129` | Zeitpunkt der letzten Zinsänderung aus der Kette ableiten (Orakel-Verlauf) oder in der Zustandsdatei speichern. |
| A11-O-10 | Info | belegt | `rate_next` rundet auf das 0,5-Raster: Ein Satz außerhalb davon macht Schritte von +0,27 bzw. −0,73. Über 20 % (von Hand gesetzt) **senkt** ein „Zins rauf“-Signal auf 20 %. | `math.rs:150–159`; Test `a11_zinsregel_raster_und_obergrenze` | Erst auf das Raster setzen, dann Schritt; über dem Rahmen nichts tun. |
| A11-O-11 | Info | belegt | „Gerundet wird zugunsten der Zinskasse“ gilt nur für `accrual`. Die Index-Fortschreibung rundet ab: Bei 0,5 % p. a. fehlen bei 600 DAA Abstand 5,4 %, bei 3 000 DAA 1,2 % des Zinses. | `risk_oracle.sil:107–109`, `HowItWorks.tsx:171`; Test `a11_index_rundung` | Text präzisieren. Agent-Updates liegen meist ≥ 1 h auseinander (0,1 %). |
| A11-O-12 | Info | belegt | Schließen (bzw. Abheben) mit Rest-Ausgang unter etwa 0,02 KAS lässt sich nicht bauen (Speichermasse > 500 000 g). Kein Verlust, Einzahlen hilft. Die Seite kündigt „an dich gehen etwa 0,01 KAS“ ohne Warnung an. | `ops.rs:585–588`; Test `a11_schliessen_mit_winzigem_rest_baut_nicht` | Kleinen Rest in den Wechselgeld-Ausgang legen (`to` ist bei ghostctl ohnehin der eigene Schlüssel). |
| A11-O-13 | Info | Code-Lektüre | Veraltete Kommentare: „bei Teil-Liquidation sinkt der Zins anteilig“ bzw. „Zins dann anteilig“. Der Code (richtig) lässt ihn stehen. | `math.rs:95`, `chain.rs:282` | Kommentare anpassen. |
| A11-O-14 | Info | Code-Lektüre | Kleinigkeiten im Umzug, einzeln aufgeführt in den Details. | `GHOST-Umzug-v3.command` | siehe Details |
| A11-O-15 | Info | Code-Lektüre | Rücknahme und Liquidation adressieren den Vault über den Index, nicht über die Covenant-ID, und haben keinen Mindest-KAS-Schutz. Endet dazwischen ein Vault mit kleinerem Index, trifft die Aktion einen anderen Vault. Wirtschaftlich neutral (gleicher Kurs), aber nicht das Gewählte. | `actions.ts:268–271`, `ghostctl.rs:1102–1113` | `--vault` optional als Covenant-ID; `--min-kas` für `redeem`. |

## Details

### A11-O-1 Zinsregel: verschiebbar und KAS-getrieben (mittel, belegt)

`rate_plan` (`ghostctl.rs:1626–1636`) nimmt `pool::ghost_usd(x, y, market) = x/y · market` aus der Zustandsdatei und wendet `rate_next` an: unter 0,995 +0,5 Punkte, über 1,005 −0,5 Punkte. Es gibt nur diese eine Messung, ohne Mittelung und ohne Mindestgröße des Pools. Das Kursband des Pools (±3 %, gemessen am Orakel) begrenzt das Schieben nicht, weil die Totzone ganz im Band liegt.

Messung im Simulator mit voller Konsensprüfung, Pool 1 000 KAS / 40 GHOST bei 0,04 USD:
- Ein Kauf von 2,55 KAS hebt GHOST von 1,00000 auf 1,00510 USD. `rate_next(3,0)` liefert 2,5.
- Kauf und Rückverkauf kosten zusammen 0,1407 KAS (≈ 0,0056 USD), fast nur Tx-Gebühren.

Der Umzug legt einen noch kleineren Pool an (0,25 GHOST, ≈ 5,6 KAS). Das Schieben wird dadurch noch billiger.

Ohne jeden Handel reicht eine KAS-Bewegung von ±0,6 % am Markt, um den Zins zu verschieben (belegt im selben Test). In einem wenig gehandelten Pool bildet die Regel also die KAS-Volatilität ab, nicht den GHOST-Kurs: Fällt KAS einen Tag lang, steigt der Zins in 40 Stunden auf 20 %.

Auswirkung:
- Schuldner können den Zins billig auf 0 drücken, zulasten der Zinskasse.
- Umgekehrt kann jemand ihn auf 20 % treiben. Das verteuert alle Schulden und rückt Vaults langsam an die Liquidation.

Gemessen wird der Pool-Stand der Vorrunde (siehe A11-O-8). Ein Angreifer muss den Kurs also nur über eine Rundengrenze halten. Einen „Flash-Tausch“ in einer einzigen Tx gibt es auf Kaspa nicht, er ist aber auch nicht nötig.

### A11-O-2 Rücknahme zum alten Orakelpreis (mittel, belegt)

`redeem` zahlt `amount·0,995/kasUsd` zum Orakelpreis (`stable_vault.sil:302–303`). Der Agent sendet Sprünge über 20 % absichtlich erst nach 3 bestätigenden Runden (`ghostctl.rs:1648–1667`, A10-A-7). Bei `--interval 300` sind das rund 10 Minuten. Fällt der Agent aus, bleibt das Orakel beliebig alt (O-5).

Steigt KAS in dieser Zeit, bekommt jeder GHOST-Inhaber mehr als 1 USD je GHOST. Im Test (Orakel 0,04, Markt 0,05) ergeben 20 GHOST 497,5 KAS, das sind 24,875 USD, also +24,4 %. Der Simulator nimmt die Tx an. Bezahlt wird das von **gesunden** Vaults über 150 %, deren Besitzer nichts falsch gemacht haben.

Über den Pool lässt sich das nicht aus dem Nichts ausbeuten. Das Band hängt am selben alten Orakel, ein Kauf im Pool mit anschließender Rücknahme verliert also etwa 0,8 %. Es trifft aber alle, die GHOST schon halten.

Bei kleinen Abweichungen unter 1 % (`min_change`) bleiben netto höchstens rund 0,5 %. Das ist unerheblich.

### A11-O-3 Umzug bei laufendem v2-Agenten (mittel, belegt)

MAINNET.md sagt nur „Danach den Agenten neu starten“. Das Skript prüft nicht, ob einer läuft. Das Startskript startet `./ghostctl` in einer Schleife, der **laufende** Prozess ist also noch das v2-Binary. So läuft es ab:

1. Nach dem Umbenennen findet der v2-Agent keine Datei. Das ist harmlos.
2. Nach `G3 deploy` liest er die neue `mainnet.json`. Belegt mit dem v2-Code aus `main`: `serde` akzeptiert die v3-Datei ohne Vaults (unbekanntes Feld `treasury` wird ignoriert).
3. `resync` baut das Factory-Skript aus dem v2-`vault_factory.sil`, das sich vom v3-Skript unterscheidet (belegt: „Factory-Skript v2 = v3? false“). Daraus entsteht der Hinweis „Factory-UTXO nicht auffindbar.“, und `load_synced` speichert.
4. Die gespeicherte Datei enthält kein `treasury` mehr (belegt). v3-`Ctx::load` (`ghostctl.rs:443–448`) lehnt sie als „Version 2“ ab.
5. `G3 open-vault` scheitert. Ein neuer Lauf trifft Z. 42–43: „$S3 ist Version 2, aber $S2 gibt es schon – bitte von Hand klären“.

Direkter Geldverlust entsteht dabei nicht. Das v3-Deployment ist aber bis zur Handreparatur (Feld `treasury` = Deployer ergänzen) unbenutzbar. Löscht jemand „die kaputte Datei“, legt der nächste Lauf ein zweites Deployment an: noch einmal 30 KAS dauerhaft gebunden, das erste ist verwaist. Zusätzlich aktualisiert der v2-Agent in der Zwischenzeit das v3-Orakel ohne Zinsregel. Das Orakel ist unverändert, seine Updates sind also gültig.

### A11-O-4 Journal bleibt beim Umbenennen liegen (niedrig)

Das Skript verschiebt nur `mainnet.json` und `mainnet.lock`. Ein offenes v2-Journal `mainnet.pending.json`, etwa nach einem Zeitlimit des Agenten oder eines Dauerauftrags, bleibt liegen. Die Folgen:
- `bin/ghostctl-v2 --state mainnet-v2.json` sucht `mainnet-v2.pending.json` und klärt die Tx nie. Der v2-Stand kennt deren Ergebnis nicht, zum Beispiel ein Wechselgeld-Token. Der Umzug tilgt dann eventuell weniger als möglich.
- v3 klärt das Journal beim ersten Aufruf (`deploy` in `ghostctl.rs:760`, `status --json` der Seite). Bei „angenommen“ schreibt `store.rs:180–182` den v2-Inhalt nach `target = deployments/mainnet.json`. `deploy` bricht dann mit „existiert schon“ ab, und der Umzug hängt wie in A11-O-3.

Das Journal nur mitzuverschieben wäre **falsch**, weil `target` auf `mainnet.json` zeigt. Besser ist es, vorher zu klären oder abzubrechen.

### A11-O-5 Wiederaufnahme überspringt Prägen (niedrig)

`ME3` ist die Zahl **aller** Vaults (Z. 146). Scheitert `mint` nach `open-vault`, zum Beispiel mit „j“ verneint oder durch ein Zeitlimit, dann gilt beim nächsten Lauf „es gibt schon einen Vault – übersprungen“. Schritt 6 ruft daraufhin `pool-open` auf. Dort wird die Genesis (1 KAS) gesendet (`ghostctl.rs:1188–1197`), bevor `pool_init` an „zu wenig GHOST“ scheitert (`pool.rs:293`). Das lässt sich fortsetzen, bindet aber KAS und verwirrt. Dasselbe Muster hat Audit 10 im Testskript gefunden (A10-A-12(e)).

### A11-O-6 Doppelstart (niedrig)

`store::lock` serialisiert nur einzelne Befehle. Zwei gleichzeitig gestartete Umzüge sehen beide `ME3 = 0` und rufen beide `open-vault` auf, was zwei Vaults ergibt. Danach prägen beide an Vault 0. `deploy` ist dagegen geschützt: Der zweite Lauf bricht mit „existiert schon“ ab. Dasselbe gilt für Teil A, weil ein veralteter Stand dort zu Fehlermeldungen führt, nicht zu Doppelsendungen: Tilgen scheitert an „zu wenig GHOST“, Abheben an `keep ≥ have`.

### A11-O-7 `redeem`: erst zusammenführen, dann prüfen (niedrig)

`ghostctl.rs:1107` ruft `consolidate` auf, und das **sendet**, wenn die GHOST auf mehr als zwei UTXOs liegen. Erst Z. 1112 (`ops::redeem` → `math::redemption`) prüft Schwelle, Schuld und DUST. Die Seite fängt die meisten Fälle mit dem Probelauf ab. Der Probelauf bricht bei verteilten GHOST aber schon in `consolidate` ab (Z. 624–626) und kann deshalb nichts vorab prüfen.

### A11-O-8 / A11-O-9 Zinsregel-Betrieb (niedrig)

- `pool_unresolved` ist `#[serde(skip)]` (`ops.rs:74–76`). Nach `Ctx::load()` ist es immer `None`, belegt mit dem Test. Der Filter in Z. 1628 wirkt also nie.
- `rate_plan` läuft **vor** `feed_due` → `load_synced` (Z. 1643 gegenüber 1645). Gemessen wird damit der Pool-Stand der letzten gespeicherten Synchronisation.
- `RATE_CLOCK` ist ein `static` im Prozess. Ein Neustart des Agenten setzt ihn zurück, ebenso ein zweiter Prozess (`oracle-feed`). Die Zusage „höchstens stündlich“ gilt also nur je Prozess und Laufzeit.

### A11-O-14 Umzug: Kleinigkeiten (Info)

- `DRY=0` gilt als Probelauf, weil `-n "$DRY"` greift. Das ist die sichere Richtung.
- `is_v3` wertet eine unlesbare `mainnet.json` als „v2“ und benennt sie um. Solange `$S2` fehlt, geht dabei nichts verloren.
- Es werden nur die Vaults, GHOST und Anteile des Besitzer-Schlüssels abgebaut. GHOST auf `keys/mainnet-keeper.json` (die der Agent für Liquidationen nutzt) werden nicht zum Tilgen verwendet, die Restschuld fällt dann höher aus.
- Der Rest-Vault v2 bleibt mit 220 % zum **eingefrorenen** v2-Orakel stehen, denn der Agent betreibt danach v3. Jeder neue Lauf senkt die Sicherheit zum aktuellen v2-Orakelpreis wieder auf 220 %. Das ist unkritisch, solange niemand das v2-Orakel bewegt.
- Daueraufträge (`mainnet-abos.json`) bleiben bei `mainnet.json`. GHOST-Aufträge zahlen nach dem Umzug automatisch **v3**-GHOST. Das ist vermutlich gewollt, aber nirgends gesagt.
- Beträge: Die Python-Floats werden mit `:.8f` formatiert, bei Schulden unter 9·10⁷ GHOST ist das exakt. `KEEP` wird auf 0,01 KAS aufgerundet, im Fall von 2,2000000000000006 bleibt dadurch 0,01 KAS mehr im Vault. `POOL_KAS` entspricht genau 1 USD zum Orakel. Zahlen gehen per `argv` an Python, A10-A-12(d) ist hier behoben.

## Was hält (mit Begründung)

- **`vaultMath.ts` folgt dem Vertrag exakt.** `mulDivDown/Up`, `growth`, `accrual`, `accrued`, `healthy`, `closeFee`, `redeemPayout` und `oracleIndexAfter` sind Zeile für Zeile gleich (`stable_vault.sil:83–143, 247–307`, `risk_oracle.sil:107–109`). Die `sim*`-Funktionen prüfen in derselben Logik, einschließlich DUST, `seize == coll` und „Rest < DUST nur bei voller Tilgung“. Vitest: 11 Dateien, 196 Tests grün.
- **`ops.rs` baut, was der Vertrag verlangt:**
  - `redeem`/`liquidate` mit `(oracleIdx, burn, outStates)` ohne Signatur, `repay` mit Besitzer-Signatur.
  - `close` mit `treasuryIdx = 1` nur bei Gebühr ≥ DUST, Ausgang an `vp.treasury` (Deployer).
  - Neuer Zustand jeweils `settled(index)`, bei Teil-Liquidation mit stehendem Zins.
  - Gleiche Grenzen `max_debt` (nur Schuld) und `healthy(debt + Zins)`.
  - Belegt durch `vault_tests` (73 Tests) und meinen Kettentest: Die Zinskasse bekommt 236,52 KAS an Ausgang 1.
- **`chain.rs` ist für v3 vollständig.** Kandidaten sind Schuld ± Betrag, der Betrag kommt aus den Argumenten bzw. aus GHOST-Eingang minus GHOST-Ausgang. Der Zins wird bis zum Index des mitgelesenen Orakels berechnet. Übernommen wird ein Zustand nur bei exaktem Skript-Treffer, am Ende bestätigt der Node. Belegt mit `a11_chain_rekonstruiert_…`: Abheben mit Zins, Rücknahme durch Dritte, Teil-Liquidation mit Rest (Zwischenstand exakt), Voll-Liquidation (Ende), Tilgen und Schließen mit Zinskasse (Ende), 3 Orakel-Updates.
- **Keine vom Vertrag abgelehnte Tx wird gesendet.** `txb::build_with_payload` führt alle Skripte und Blockgrenzen vorher lokal aus. Gebühren gehen nur bei tatsächlich angenommenen, aber nutzlosen Tx verloren (A11-O-5, A11-O-7).
- **Status-JSON und Seite passen zusammen.** Die Felder `interestUsd` (je Vault und in `totals`), `params.redeemFeePct`, `params.maxDebtGhost` und `ratioPct` (auf Schuld + Zins) werden in `status.ts`, `precheck.ts`, `VaultList`, `ActionForms` und `Statistics` passend gelesen. `maxMintGhost` entspricht exakt `mint` (Mindestquote mit Zins, Grenze nur für die Schuld).
- **Web-API:** `ALLOWED.redeem = [key, vault, ghost]`, Betrag Pflicht, `FLAG_ORDER` gleich (A10-W-3 bleibt behoben). Die Tests in `actions.test.ts` und `commands.test.ts` sind grün.
- **Keeper mit Zins:** `keeper_burn` rechnet mit Schuld + Zins, liquidiert nur Schuld > 0 und behält A10-A-1/A-2 bei (Tests in `math.rs`).
- **Erkennung von v2-Dateien:** `Ctx::load` lehnt `vault_params` ohne `treasury` mit einem klaren Hinweis ab. `status --json` meldet dann `deployed: false` samt Fehler. A10-A-5 wird damit in v3 sauber gelöst.
- **Umzug im normalen Ablauf:**
  - Jeder Schritt prüft den Stand neu.
  - Vaults werden über die Covenant-ID verfolgt.
  - `deploy` hat eine Fortschrittsdatei.
  - Tilgen/Schließen/Abheben laufen auf dem Journal von `S2`.
  - Im Probelauf wird in Teil A mit `--dry-run` gerechnet und Teil B nicht ausgeführt.
  - Einen Pfad, der nacheinander ausgeführt doppelt sendet oder falsche Beträge sendet, habe ich nicht gefunden.
- **Texte** zu Zins (Posten in USD, Zahlung beim Schließen, unter 0,2 KAS erlassen), Rücknahme (ab 150 %, −0,5 % beim Besitzer, 0,2 KAS Rest), Kursband ±3 % am Orakel und Obergrenze 50 GHOST je Vault ohne Gesamtgrenze stimmen mit dem Code überein. Ausnahmen sind A11-O-9 („höchstens stündlich“) und A11-O-11 (Rundung).
- **Audit-10-Behebungen:** A10-A-1/2/3/7/8/10/11 und A10-W-3 sind intakt. Wieder aufgetaucht sind nur zwei Muster, jeweils in neuem Code: A10-A-9 (A11-O-7) und A10-A-12(e) (A11-O-5).

## Anhang

### Testläufe (Kopie, `cargo test --release`)

Alle Suiten ohne `rest_live_tests`:
- lib: 24
- audit10_pool_engine: 20
- audit10_pool_regress: 4
- audit11b_tests: 8
- chain_tests: 3
- e2e_tests: 3
- factory_tests: 21
- ghost_token_tests: 9
- oracle_tests: 26
- payload_tests: 5
- pool_e2e_tests: 3
- pool_tests: 32
- vault_math_tests: 4
- vault_tests: 73

Alle grün. App-Kopie: `npm ci && npx vitest run` ergibt 11 Dateien und 196 Tests, alle grün.

Ausgabe `audit11b_tests`:

```
test a11_chain_rekonstruiert_redeem_withdraw_liquidation_close_mit_zins ... Index nach 1 Jahr: 1.315360050; Zins Vault 0 12.61440200 USD
Schließen Vault 1: 236.5200 KAS Zins an die Zinskasse
ok
test a11_index_rundung ... 0,5 % p. a., Abstand 600 DAA: Index +9 statt +9.513 (5.4 % Zins fehlt)
0,5 % p. a., Abstand 3000 DAA: Index +47 statt +47.565 (1.2 % Zins fehlt)
0,5 % p. a., Abstand 36000 DAA: Index +570 statt +570.776 (0.1 % Zins fehlt)
ok
test a11_pool_unresolved_ueberlebt_speichern_nicht ... ok
test a11_ruecknahme_zum_veralteten_orakelpreis ... Orakel 0.0400, Markt 0.0500: 20 GHOST → 497.5000 KAS = 24.8750 USD zum Marktpreis (+24.4 %); Nettozufluss beim Inhaber 498.4386 KAS
ok
test a11_schliessen_mit_winzigem_rest_baut_nicht ... Rest 0.04996761 KAS (Gebühr 99.95003239): baut, storage 203995 g, Sim: Ok(())
Rest 0.00998441 KAS (Gebühr 99.99001559): baut NICHT: Transaktion zu groß für einen Block (compute 29077 g, storage 1005427 g, transient 100508 g). …
Rest 0.00094811 KAS …: baut NICHT … storage 10551165 g …
Rest 0.00010252 KAS …: baut NICHT … storage 97545809 g …
ok
test a11_v3_deployment_als_json_fuer_v2_gegenprobe ... ok
test a11_zinsregel_ist_mit_kleinem_tausch_verschiebbar ... Pool 1000.00 KAS / 40.0000 GHOST: GHOST 1.00000 USD → nach Kauf von 2.5506 KAS 1.00510 USD → Zins 3,0 % → Some(2.5)
Kosten des Hin-und-Rück-Tauschs inkl. Tx-Gebühren: 0.140656 KAS = 0.0056 USD (Token-UTXO-KAS zurück)
ok
test a11_zinsregel_raster_und_obergrenze ... ok
test result: ok. 8 passed; 0 failed
```

Hinweis zu A11-O-12: Meine erste Vermutung war, schon 0,05 KAS Rest scheitern. Die Messung widerlegt das (204 000 g, angenommen). Die Grenze liegt bei etwa 0,02 KAS.

Gegenprobe v2 (`main`, `audit11b-v2/protocol/tests/a11_v2_liest_v3.rs`, gleiche Submodul-Revision `3ed9733`):

```
Factory-Skript v2 = v3? false
nach Speichern durch v2: treasury vorhanden? false
test a11_v2_laedt_v3_datei_und_speichert_ohne_treasury ... ok
```

### Nicht geprüft / Grenzen

- `bin/ghostctl-v2` selbst (nicht im Worktree). Für A11-O-3 wurde der v2-Code aus `main` verwendet. Ob das Binary genau diesem Stand entspricht, ist nicht geprüft.
- Mempool-Standardregeln des Mainnets (der Simulator prüft nur Konsens und Blockgrenzen).
- Kein Lauf gegen das Netz. Die REST- und Node-Pfade (`History`, `confirm`) wurden nur gelesen.
