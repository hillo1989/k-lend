# Audit 16 – Umzug v3 → v4 (`GHOST-Umzug-v4.command`, Commit 7426f84)

Gegenstand: `GHOST-Umzug-v4.command` (1171 Zeilen) und `tests/umzug-v4/` (run.zsh, mock_ghostctl.py).
Abgeglichen mit dem Quelltext von ghostctl v3 (`kaspa-lending/protocol/src/bin/ghostctl.rs`, ops.rs, pool.rs, txb.rs, math.rs) und v4 (`kaspa-lending-v4/protocol/src/…`).
`bin/ghostctl-v3 --help` lief lokal ohne Netz; Befehle und Flags stimmen mit dem v3-Quelltext überein.
Nicht gelesen: `keys/`, die Inhalte unter `deployments/`. Nichts gesendet, ghostctl lief nicht gegen das Netz.

Testlauf in einer Scratchpad-Kopie: **242 bestanden, 0 fehlgeschlagen**. Die Kopie ist wieder entfernt.

## Ergebnis in Kürze

- **Kritisch: keiner. Hoch: keiner.** Jeder Schritt in Teil A holt Geld zurück. Gegen doppeltes Senden schützen das Journal von ghostctl, der Deploy-Fortschritt und der Vault-Merker. Ein Agent von Version 3 kann die Zustandsdatei von Version 4 nicht überschreiben.
- **Mittel: 4.** Zwei Zustände, aus denen ein erneuter Doppelklick nicht herausführt (M-1, M-2). Eine unvollständige Anzeige dessen, was in Version 3 gebunden bleibt (M-3). Eine falsche Meldung „Fertig“ nach einem halb angelegten Pool (M-4).
- **Niedrig: 6.**
- **Tests:** Alle 5 eigenen Rückbau-Proben überleben, die Tests bleiben also grün. Zur Attrappe fehlen Speichermasse, die Grenze von zwei Anteils-UTXOs, die drei Schritte von pool-open und ein eingefrorenes Orakel.

---

## Mittel

### M-1 Herausnehmen kleiner Beträge lässt sich nicht bauen: eine Fortsetzung kann bei Teil A hängen bleiben

- **Wo:** `GHOST-Umzug-v4.command:812-829`
  - Bei Restschuld wird herausgenommen, sobald `KEEP_S < COLL_S` gilt. Eine Mindestdifferenz gibt es nicht.
  - `KEEP` wird auf 0,01 KAS aufgerundet und folgt dem Orakelkurs.
  - Der Plan baut den Schritt nicht mit `--dry-run`, sondern trägt ihn nur ein (Z. 822-825, 826).
- **Was ghostctl v3 daraus macht:** `ops::withdraw` legt einen eigenen Ausgang über `v.vault.value - new_coll` an (`kaspa-lending/protocol/src/ops.rs:533-557`). `build` prüft die Blockgrenze der Speichermasse (`txb.rs:83`, `txb.rs:86-91`). Laut `math.rs:97-103` (gemessen in `tests/sweep_grenze_tests.rs`) lässt sich ein Ausgang unter etwa 0,02–0,025 KAS nicht bauen. Ein kleiner Ausgang wird nicht mit dem Wechselgeld zusammengelegt (`txb.rs:186-200`).
- **Ablauf:**
  1. Der erste Lauf senkt die Sicherheit des Rest-Vaults auf `KEEP`. Bei der Restschuld aus der Pool-Mindestliquidität sind das etwa 2,2 KAS.
  2. Teil B bricht ab, zum Beispiel weil der Node nicht erreichbar ist oder KAS fehlen.
  3. Bis zum nächsten Doppelklick steigt der KAS-Kurs um etwa 0,5–1 %.
  4. Der Plan zeigt jetzt „Sicherheit von 2,20 auf 2,18 KAS senken“. Nach dem j scheitert `withdraw` vor dem Senden mit „Transaktion zu groß für einen Block“.
  5. Jeder weitere Doppelklick läuft genauso. Teil B, also Deployment, Vault und Pool, wird nie erreicht, bis sich der Kurs weiter bewegt.
  6. Ist die Differenz etwas größer, etwa 0,03 KAS, wird gesendet. Die Gebühr von etwa 0,05 KAS ist dann höher als der Betrag, der zurückkommt.
- **Folge:** Kein Geldverlust. Der Umzug hängt aber, und der Doppelklick hilft nicht. Die Grenze ist aus dem Quelltext abgeleitet und nicht gegen einen Node gemessen.
- **Vorschlag:** Herausnehmen nur, wenn `COLL_S - KEEP_S` mindestens 0,5 KAS beträgt (sonst als „bleibt“ zeigen). Zusätzlich im Plan `withdraw` mit `--dry-run` proben, sofern kein vorheriges Tilgen nötig ist.

### M-2 Pool-Anteile auf mehr als zwei UTXOs: jeder Doppelklick scheitert an der Probe

- **Wo:** Das Skript nimmt `SHARES` aus `keys --json` („lpShares“, Z. 686) und rechnet damit den Rückfluss und `--min-kas`/`--min-ghost` (Z. 693-702).
- **Was ghostctl v3 tut:**
  - `lpShares` summiert **alle** eigenen Anteils-UTXOs (`ghostctl.rs:1622`, `ghostctl.rs:1630`).
  - `pool-remove` zieht nur aus den **zwei größten** ab (`ghostctl.rs:1487-1496`, `pool.rs:637-642`), mit dem Hinweis „abgezogen wird aus den zwei größten“.
  - Jedes `pool-add` legt einen neuen Anteils-UTXO an und führt nie zusammen (`pool.rs:587`: `my_lp = &[]`).
- **Ablauf:** Hat der Schlüssel drei oder mehr Anteils-UTXOs (pool-open plus mindestens zwei pool-add), kommt weniger zurück als gerechnet. Liegt die Abweichung über 1 %, lehnt ghostctl ab (`pool.rs:611-613`). Schon die Probe im Plan scheitert (Z. 707, `probe_fail`). Jeder Doppelklick endet vor der Frage. Nichts wird gesendet, aber der Umzug kommt nicht voran.
- **Offen:** Ob das im Mainnet zutrifft, ist nicht geprüft, weil `deployments/` nicht gelesen wurde.
- **Vorschlag:** Die Anteile der zwei größten UTXOs zählen, oder `pool-remove` wiederholen, bis keine Anteile mehr da sind. Dazu einen Fall in der Attrappe; sie zieht heute alles auf einmal ab (`mock_ghostctl.py:264-272`).

### M-3 Was in Version 3 gebunden bleibt, zeigt das Skript nur teilweise

Gezeigt werden der Rest-Vault mit Sicherheit und Restschuld (`REST_V3`, Z. 825/832) und die übrigen v3-GHOST (`G3_REST`, Z. 477). **Nicht gezeigt** werden:

- **Pool-Mindestliquidität von Version 3** (1 KAS und GHOST im Wert von 1 KAS, für immer). Beim Schritt „Pool-Anteile abziehen“ ist das Feld „gebunden“ leer (Z. 709-712). `endstand` nennt nur Vaults und GHOST (Z. 476-478). Das Wort „Mindestliquidität“ kommt nur im Kommentar (Z. 32) und im v4-Pool (Z. 957, 963) vor.
- **Warum Restschuld bleibt:** Die GHOST der Mindestliquidität fehlen zum Tilgen. Das steht nur im Kommentar Z. 32-33. Der Plan sagt lediglich „Restschuld … bleibt“.
- **Covenants der Version 3:**
  - Orakel, Factory und GHOST-Wurzel binden je 10 KAS (`kaspa-lending/protocol/src/ops.rs:27-29`, `135`, `144`, `164`).
  - Jeder v3-Vault bindet 3 KAS im Minter-Zweig (`ops.rs:30`, `242`). `close` gibt den Zweig nicht frei (`ops.rs:568-600`).
  - Der Plan für Version 4 nennt beides („4 KAS dauerhaft“, „3 KAS Minter-Zweig dauerhaft“, Z. 925, 935). Für Version 3 steht nichts davon da.
- **Rest-Vault ist praktisch verloren:** Der Zins läuft weiter. Mit 220 % Sicherheit und 150 % Liquidationsschwelle kann der Vault bei fallendem Kurs oder nach langer Zeit liquidiert werden. Tilgen ginge nur mit v3-GHOST, die fast nur noch in der Pool-Mindestliquidität liegen. Für den Nutzer ist „gebunden“ hier also praktisch „verloren“. Der Plan sagt das nicht.

**Vorschlag:** Im Endstand ein Block „Version 3 bleibt für immer“ mit Pool-Mindestliquidität, Covenant-KAS, Minter-Zweigen und dem Hinweis, dass die Restschuld daraus folgt.

### M-4 Pool halb angelegt: der nächste Doppelklick meldet „Fertig“

- **Wo:** `pool-open` in v4 läuft in drei Schritten: Genesis, Init mit Mindestliquidität, dann Einlegen des Rests (`ghostctl.rs:1657-1692`). Scheitert Schritt 3 („Pool ist angelegt; den Rest später mit pool-add einlegen“, Z. 1687-1688), besteht der Pool schon.
- **Was das Skript tut:** Es prüft nur `status.pool` (Z. 1136). Der nächste Doppelklick zeigt „Tauschpool existiert schon – übersprungen“ (Z. 1158) und „=== Fertig: Version 4 läuft ===“.
- **Widerspruch:** Die Abbruchmeldung verspricht „erneuter Doppelklick setzt fort“ (Z. 1156). Der Pool hat dann aber nur die Mindestliquidität von 1 KAS.
- **Folge:** Nichts geht verloren, die KAS und GHOST bleiben auf dem Schlüssel. Die Meldung ist aber falsch, und mit dem Kursband ist der Mini-Pool kaum nutzbar.
- **Vorschlag:** Eigene Pool-Anteile prüfen. Hat der Schlüssel einen Pool, aber keine Anteile, `pool-add` mit dem Rest anbieten oder klar melden.

---

## Niedrig

- **N-1 Signal vor der Definition des Handlers:** `trap sig_ende …` steht in Z. 167, `sig_ende()` wird erst in Z. 524 definiert.
  - Kommt Ctrl+C dazwischen, meldet zsh „command not found“ und läuft weiter. Nachgeprüft mit `zsh -c 'trap f INT; kill -INT $$; echo weiter'`.
  - In diesem Abschnitt wird nichts gesendet, und vor dem „j“ wird nichts umbenannt. Harmlos.
  - Aus der Vorlage geerbt (`GHOST-Umzug-v3.command:130/464`).
- **N-2 Zu wenig KAS ist nur eine Warnung:**
  - `plan_b` gibt fehlende KAS nur als ⚠ aus (Z. 953-955, 970). Der Nutzer kann trotzdem „j“ sagen. Dann läuft Teil A, und Teil B bricht mitten im Deployment ab. Es lässt sich fortsetzen, sobald KAS nachgeschossen sind.
  - Nicht geprüft wird außerdem, ob 50 KAS für 0,5 GHOST bei 200 % reichen. Das braucht einen Kurs von mindestens etwa 0,02 USD (`ops.rs` v4 `mint`, Z. 716-723). Darunter scheitert `mint`, und ohne GHOST gibt es auch keinen Pool.
  - Vorschlag: Bei fehlenden KAS oder zu niedrigem Kurs vor der Frage abbrechen.
- **N-3 Eingefrorenes Orakel bei später Fortsetzung:**
  - Version 4 friert nach 2 h ohne Preis ein (`ghostctl.rs:1165`). Danach darf jeder `oracle-freeze` senden.
  - Eingefroren sperrt `mint` (`ops.rs:688-693`, `717`) und den Pool mit Kursband (`pool.rs:96-101`, `318`).
  - Das Skript verbietet einen laufenden Agenten (Z. 169, 1051). Liegt eine Fortsetzung mehr als 2 h nach dem Deployment und hat jemand eingefroren, hilft der Doppelklick nicht. Der Nutzer muss von Hand `oracle-update` senden.
  - Das Skript weist darauf nicht hin.
- **N-4 Irreführende oder unvollständige Meldungen:**
  - ghostctl v3 auf einer v4-`mainnet.json` meldet „stammt von Version 2“ (`kaspa-lending/…/ghostctl.rs:622-627`), weil v4 `interest_spk` statt `treasury` hat (`contracts.rs` v4 Z. 407-421). Das ist sicher, aber verwirrend.
  - Die Abbruchmeldung zur Zinsdatei (Z. 918, 1091) rät „nach mainnet-v3-zins.json legen“, auch wenn es diese Datei schon gibt.
  - Der Wrapper `./ghostctl` nennt sich im Kopf noch „Version 3“ (`ghostctl:2`).
  - Bei „$S4 ist Version 3, aber $S3 gibt es schon“ (Z. 248) erfährt der Nutzer nicht, welche Datei die neuere ist.
- **N-5 Eigene gesperrte (stale) v3-Vaults übergeht Teil A ohne Hinweis:** Der Filter steht in Z. 734. Diese Vaults erscheinen nicht unter „Version 3 bleibt“.
- **N-6 Schließen mit sehr kleinem Rest:** Ist nach dem Zins nur noch ein Rest unter etwa 0,02 KAS übrig, lässt sich `close` nicht bauen (`ops.rs:594-596`). Das ist dieselbe Ursache wie M-1. Es ist ein Randfall und tritt nur bei Vaults auf, die das Skript schon auf 0,3 KAS gesenkt hat und die danach viel Zins angesammelt haben.

---

## Antworten auf die Prüffragen

### 1. Geldverlust, Zurückgelassenes, doppelt Senden, Hängenbleiben

- **Geldverlust:** keiner gefunden.
  - Teil A holt nur zurück: Pool-Anteile mit Mindestbeträgen (Z. 716), Tilgen immer mit `--ghost`, Prüfung vorher und nachher über die Covenant-ID (Z. 562-564, 598-624).
  - Teil B legt nur neu an.
  - Unnötig sind nur kleine Gebühren für Kleinst-Herausnahmen (M-1).
- **Doppelt senden:** Dagegen schützen drei Dinge:
  - das Journal von ghostctl (`store.rs:78-96`, `126-196`: `resolve_pending` vor jeder Aktion; bei „noch unterwegs“ bricht ghostctl ab),
  - der Deploy-Fortschritt (`ghostctl.rs:1121-1242` v4, Fortsetzung nur mit denselben Schlüsseln und Einstellungen, Z. 1185-1198),
  - der Vault-Merker (Z. 1113-1126).
- **Zurückgelassenes:** Rest-Vault, v3-GHOST und Mindestliquidität (siehe M-3).
- **Hängenbleiben:** M-1, M-2 und M-4 (falsches „Fertig“), dazu N-3.

### 2. Aufrufe Zeile für Zeile gegen den Quelltext

Globale Flags `--network`, `--state`, `--ja`, `--dry-run` und `--json` stehen vor dem Unterbefehl. Das stimmt in v3 (`ghostctl.rs:39-60`) und v4 (`ghostctl.rs:39-60`). Alle Aufrufe passen:

| Skript | Quelltext v3 | Quelltext v4 |
|---|---|---|
| `--json keys --dir <ordner>` (Z. 328, 864); Felder `keys[].type/file/xonly/address/kas/ghost/lpShares`, Unterzeichner mit `type:"committee"`, `signers` | Cmd 76-79; Felder 1608, 1625-1630 | Cmd 82-85; Felder 1852, 1868-1875. `keys` läuft vor der Versionsprüfung (980-997 vor 1063-1070), geht also auch auf der v3-Datei |
| `status --json` (Z. 354, 586, 681, 730, 737, 912, 1135); `vaults[].index/owner/covenantId/debtGhost/collateralKas/interestUsd/stale`, `oracle.kasUsd`, `pool.shares/kasSompi/ghostUnits` | 96-100; 1674-1698, 1716 | 111-115; 1918-1928, 1938-1942, 1960 |
| `--dry-run --json <befehl>` → `transactions[].feeKas` (Z. 661-668) | 690 | 780 |
| `pool-remove --key --percent 100 --min-kas --min-ghost` (Z. 707, 716) | 325-336, 1477-1511 | – |
| `repay --key --vault --ghost` (Z. 779, 792) | 167-175; ops 462-470 | – |
| `close --key --vault` (Z. 805); Zins wie `close_fee` (Z. 750-755) | 196-201; ops 559-564; math 108-111; DUST 92 | – |
| `withdraw --key --vault --keep` (Z. 829); 200 % von Schuld + Zins | 186-194, 1270-1279; ops 533-541 | – |
| `committee-keygen <datei> --count 1` (Z. 1094) | – | 69-73, 1010-1021 (vor der Versionsprüfung, ohne Netz) |
| `deploy --key --committee`, ohne `--probe`/`--rate`/`--threshold` (Z. 1103) | – | 88-106, 1097-1277: Schwelle n/2+1 = 1 (1134), `CovValues::small` = 4 × 1 KAS (1170; ops.rs 55-57), `rate_restart` legt `mainnet-zins.json` an (1275) |
| `open-vault --key --kas`, `mint --key --vault --ghost` (Z. 1120, 1127, 1130) | – | 178-192, 1486-1492 |
| `pool-open --key --kas --ghost` (Z. 1156); Kursband automatisch | – | 328-337, 1629-1693; `POOL_BAND_BPS = 300` (pool.rs 28) |
| `--json price` → `median` (Z. 930) | – | 1022-1031 |
| Versionserkennung `state_version` (Z. 124-134) | v3 lehnt eine Datei ohne `treasury` ab (622-627) | wie `old_version` (916-928) |
| Dateinamen: `<stem>.lock`, `.pending.json`, `.deploy.json`, `-zins.json`, `-zins.lock` | store.rs 33, 74-75; ghostctl 957/981; rate.rs 89-91, 243-246 | gleich |

Keine Abweichung gefunden.

### 3. Umbenennen, Abbruch dazwischen, alter v3-Agent

**Reihenfolge** (`rename_v3`, Z. 185-241):

1. Unter flock auf `mainnet.lock` und `mainnet-zins.lock` (Z. 202-207).
2. Umbenennen in der Reihenfolge Journal → Deploy-Fortschritt → Zinsdatei → Zustand (Z. 189-190, 226-228).
3. Journal-Ziel umschreiben (Z. 229-237).
4. Sperren umbenennen (Z. 238-240).

Umbenannt wird erst nach dem „j“ (Z. 1034-1038). Vorher wird nur geprüft (Z. 253).

**Abbruch an jeder Stelle:**

| Abbruch nach | Nächster Doppelklick |
|---|---|
| Journal | Zustand noch v3 unter `mainnet.json`. Er benennt den Rest um, `journal_v3` richtet das Journal danach aus (Z. 1037). Test J2. |
| Zinsdatei | `mainnet.json` (v3) liegt noch da. Der Rest wird nachgeholt. Hat ein v3-Aufruf inzwischen eine neue `mainnet-zins.json` angelegt, folgt Abbruch „Gibt es schon“ und Klären von Hand. Sicher. |
| Zustand, aber vor dem Journal-Ziel | `journal_v3` richtet um, bevor v3 aufgerufen wird (Z. 313). Tests J1, J3. |
| Journal-Ziel, aber vor den Sperren | Die Sperren bleiben unter dem alten Namen. Harmlos: ghostctl legt sie selbst an, v4 nutzt `mainnet.lock` und `mainnet-zins.lock` weiter. |

**Alter v3-Agent danach:**

- **Mit Standard-`--state` (`mainnet.json`):**
  - Solange die Datei fehlt, scheitert jede Runde an `load`, bevor die Zinsdatei berührt wird (`ghostctl.rs:2264-2272`). Es entsteht nur `mainnet.lock` (store.rs 32-37).
  - Liegt dort v4, lehnt `Ctx::load` ab (`ghostctl.rs:622-627`). Der v3-Agent kann den v4-Stand also nicht überschreiben.
- **Mit `--state mainnet-v3.json`:** Er arbeitet weiter auf Version 3 mit `mainnet-v3-zins.json`. Das ist stimmig.
- **Einziger Weg zu einer v3-Datei unter `mainnet.json`:** Das Umbenennen wurde zwischen Zustand und Journal-Ziel unterbrochen, und vor dem nächsten Doppelklick läuft ein v3-Aufruf mit `--state mainnet-v3.json`. Dann hält das Skript an (Z. 248). Sicher, aber nur von Hand lösbar (N-4).
- **Agent-Prüfung:** Sie läuft nur an zwei Stellen (Z. 169 und 1051). Ein v3-Agent, der dazwischen startet, ist nach dem oben Gesagten harmlos.

### 4. Teil B

- **Reihenfolge:** Unterzeichner-Datei → Deployment → Vault → Prägen → Pool. Sie ist richtig: Der Pool braucht die GHOST aus dem Prägen.
- **Unterzeichner-Datei:**
  - Sie muss genau 1 Schlüssel als „committee“ enthalten (Z. 864-881).
  - Fehlt sie bei einem angefangenen Deployment, bricht das Skript ab (Z. 891-892). Das passt zu ghostctl, das eine Fortsetzung mit anderen Schlüsseln ablehnt (v4 Z. 1185-1187).
- **Deployment:**
  - Es lässt sich fortsetzen und läuft ohne `--probe`. ghostctl lehnt geänderte Schalter ab (v4 Z. 1189-1198).
  - Die Zinsdatei von Version 3 darf nicht im Weg liegen (Z. 918, 1091).
- **Vault:** Gezählt werden eigene Vaults, ein gesperrter zählt mit (Z. 354-362). Dazu kommt der Merker (Z. 1113-1126).
- **Pool:**
  - Die KAS kommen aus dem Orakelkurs, damit beginnt der Pool genau bei 1 USD in der Mitte des Bands (pool.rs 315-325).
  - Vor dem Senden werden GHOST und KAS geprüft (Z. 1143-1148), weil pool-open die Genesis vor der GHOST-Prüfung sendet.
- **Reicht „genug KAS“?** Nicht ganz:
  - Der Kurs muss für 0,5 GHOST bei 200 % reichen (N-2).
  - Das Orakel darf nicht eingefroren sein (N-3).
  - pool-open kann nach Schritt 2 hängen bleiben (M-4).
  - Fehlende KAS sind im Plan nur eine Warnung (N-2).

### 5. Was in Version 3 gebunden bleibt

Siehe M-3. Die Pool-Mindestliquidität wird **nicht** klar gezeigt. Gezeigt werden nur der Rest-Vault und die übrigen v3-GHOST.

### 6. Tests

Basis: 242/242 grün.

**Eigene Rückbau-Proben** (vom Autor nicht genannt). Jede lief mit dem vollen `tests/umzug-v4/run.zsh` über `SCRIPT=<mutant>`:

| # | Rückbau | Stelle | Ergebnis |
|---|---|---|---|
| R1 | Kappung auf die 1-KAS-Reserve beim Pool-Rückfluss entfernt (`dx = x*m//s`) | Z. 698 | **überlebt** (242/242). Kein Fall, in dem die Kappung greift, obwohl ghostctl sie anwendet (pool.rs 603) |
| R2 | Pool-Vergleich vor Teil B entfernt | Z. 1070-1072 | **überlebt**. E5 prüft nur die Frage direkt vor dem Pool. Bei schon vorhandenem v4, also mit Orakelkurs im Plan, gibt es keinen Fall |
| R3 | Zinsdatei erst nach der Zustandsdatei umbenennen | Z. 189-190 | **überlebt**. Kein Fall für einen Abbruch zwischen Zinsdatei und Zustand. Die im Kommentar Z. 179-183 begründete Reihenfolge ist ungetestet |
| R4 | Prüfung der Zinsdatei direkt vor dem Deployment entfernt | Z. 1091 | **überlebt**. V2 trifft nur die gleiche Prüfung im Plan (Z. 918), nicht eine Zinsdatei, die nach der Frage entsteht |
| R5 | `--keep` abrunden statt aufrunden | Z. 816 | **überlebt**. Alle Fälle (R10, D1, U7, V7, E9, E11) haben glatte Werte |

**Was die Attrappe nicht kann** (daher ohne Fall):

- Speichermasse (M-1, N-6)
- Abziehen aus nur zwei Anteils-UTXOs (M-2)
- die drei Schritte von pool-open (M-4)
- eingefrorenes Orakel (N-3)
- das Signal-Fenster vor Z. 524 (N-1)

Die Attrappe schreibt in den v4-Zustand außerdem `vault_params.treasury` (`mock_ghostctl.py:225`, `run.zsh:66`). Das echte v4 hat stattdessen `interest_spk`. Für die Versionserkennung ist das ohne Folgen, weil `register` zuerst geprüft wird.

Bewertung: Die Tests decken die genannten Regeln gründlich ab. Mit Agent, Journal, Vault-Nummer, Fortsetzung an jedem Schritt und Ctrl+C beweisen sie das, was sie nachbilden können. Die Lücken liegen dort, wo die Attrappe ghostctl vereinfacht, und an den fünf Stellen oben.
