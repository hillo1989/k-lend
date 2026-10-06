# Audit 15: Gegenprüfung von Commit 134039d („Befunde aus Audit 14 behoben“)

Prüfer: Claude Opus 5.5, 05.10.2026. Gegenstand: 134039d auf b046a49, Befunde aus `audit/14-opus-v4-bibliothek.md` (H-1 … N-3) und `audit/14-opus-v4-ghostctl.md` (H1 … N9).

**Arbeitsweise**
- Eigene Arbeitskopie (`git worktree … 134039d --detach`) im Scratchpad. Nur dort habe ich geändert und getestet. Danach habe ich sie entfernt. Im v4-Ordner ist diese Datei die einzige Änderung. `keys/` und `deployments/` habe ich nicht gelesen, Transaktionen habe ich keine gesendet.
- Der Submodul-Inhalt `vendor/silverscript` und der Build-Cache sind als Kopie aus dem v4-Ordner übernommen, ohne Netz. `app/node_modules` ist ein Symlink.
- Grundlauf `cargo test --offline` (debug): **627 bestanden, 0 fehlgeschlagen, 2 ignoriert** (27 Testprogramme). `vitest run` in `app`: **442 bestanden** (24 Dateien).
- Rückbau-Probe: Ich habe die Kernänderung jeder Korrektur in der Kopie zurückgesetzt und die Tests erneut laufen lassen (Abschnitt 2).
- Probe-Skript: Das unveränderte Skript lief in einer Sandbox mit einer ghostctl-Attrappe (Python, Zustand in JSON, Fehler gezielt eingespeist). Die Eingaben kamen über `expect` mit Pseudo-TTY (Abschnitt 3).

Zeilenangaben beziehen sich auf den Stand 134039d. „Probe“ steht für `GHOST-v4-Probe.command`, „ghostctl“ für `protocol/src/bin/ghostctl.rs`.

---

## Kurzfassung

- **Vollständig behoben** sind die meisten Befunde: H1, H2/H-1, H3/M-1, M-2, M1 (mit Einschränkung), M2, M3, N-1, N2, N3, N-3, N8 und N9.
- **Kein Befund ist kritisch.** Neue Fehler gibt es vor allem im Probe-Skript (G-1 bis G-3) und bei der Ticket-Liste (G-4).
- **Die Tests belegen die Korrekturen nur zum kleinen Teil.** Von 18 zurückgesetzten Kernänderungen machen nur 5 einen Test rot. **12 Korrekturen** lassen sich gemeinsam zurücksetzen, und alle 627 Rust-Tests bleiben grün. Darunter sind:
  - die Verdrahtung des Herzschlags im Agenten (H2/H-1);
  - der Weg vom Agenten zu `fit_rate`;
  - der gesamte neue Register-Abschnitt von `store::resync` (H-2, M-3/M5);
  - `readyInHours` (H1), die Prüfung von `deploy --rate` (M-2) sowie N4, N5 und N9.
  Weder für die Seite noch für das Probe-Skript gibt es einen Test, der eine Korrektur absichert.

| Nr | Schwere | Kurz |
|---|---|---|
| G-1 | **mittel** | Probe Stufe 2: Läuft die Konsens-Probe nach Ablauf der Wartezeit, gibt es einen Fehlalarm „ANGENOMMEN“. Der Austausch ist dann wirklich aktiviert, und das Skript hängt dauerhaft in Stufe 2b fest (mit Attrappe belegt) |
| G-2 | **mittel** | Probe Stufe 2: Als „Node lehnt ab“ zählt jeder Fehler, auch Netz-, Sperr- und Vorprüfungsfehler (mit Attrappe belegt) |
| G-3 | mittel | Probe: `must` innerhalb von `[ "$(must …)" … ]` bricht das Skript nicht ab. Nach „✗ Status nicht lesbar“ läuft es weiter, bis „✓ Probe abgeschlossen“ (mit Attrappe belegt) |
| G-4 | niedrig–mittel | `old_tickets` wird nie bereinigt. Räumt ein Dritter ein Ticket auf, und dafür bekommt er 1 KAS, scheitert `signers clear` danach dauerhaft |
| G-5 | niedrig | `fit_rate` (N-2/N1): In der Zinsrunde geht eine überflüssige Tx nur mit dem Preis hinaus, und die Zinsregel verliert eine Stunde |
| G-6 | niedrig | `resync` übernimmt einen Satz nur anhand des Hashs des Satzes. Der Notfallsatz (`fb_hash`) wird nicht verglichen. Ein ersetztes eigenes Ticket fällt dabei aus der Datei |
| G-7 | niedrig | N5 macht die neue v1/v2/v3-Meldung aus N7 unerreichbar |
| G-8 | niedrig | Probe Stufe 3 ist nicht idempotent: Jeder Neustart sendet ein weiteres Preis-Update mit Unterzeichner 2 |
| G-9 | niedrig | Teilweise offen: Seitentext „6 Stunden“ (H2), „Zinskasse“ (N6), M4 (alter Unterzeichner braucht Marktpreis, Vertragssperre beim Prägen ungeprüft), N4 (Startpreis) |
| G-10 | Hinweis | `status --json` verschluckt Fehler beim Abgleich. `must` erkennt nur einen völlig fehlenden Status |

---

## 1. Befund für Befund

### Audit 14, Bibliothek

| Befund | Urteil | Beleg |
|---|---|---|
| **H-1** Agent friert eigenes Orakel ein | **behoben** (Code). Rückbau bleibt grün | `heartbeat_min` ghostctl:2596–2598 begrenzt auf Frist/2 (2 h → 60 min, Probe → 30 min). Angewandt in `feed_due` ghostctl:2626. Bei `frozen` ist ein Update sofort fällig (ghostctl:2623–2625). Der Test `herzschlag_vor_der_einfrier_frist` (ghostctl:4669) prüft nur die Hilfsfunktion, nicht ihre Verwendung (R1 und R2 grün). Der vorgeschlagene Schritt, `freeze_if_stale` erst nach einem gescheiterten eigenen Update auszulösen, ist nicht umgesetzt. Mit 60 min Herzschlag ist das vertretbar. |
| **H-2** `resync` sperrt nach einer Aktivierung alles | **behoben**. Rückbau bleibt grün | store.rs:367–385: Der eigene Satz wird übernommen. Ein unbekannter Satz setzt nur `signers_unknown`, und `oracle_update_with` lehnt dann ab (ghostctl:2666). Vaults und Pool laufen weiter. Kein Test, siehe R7. Zur Restlücke siehe G-6. |
| **M-1** Zins neben dem Raster blockiert Updates | **behoben** für den Agenten | `fit_rate` ghostctl:2605–2614 begrenzt auf ±`rate_step` und `[0, max_rate]`. Der Agent ruft es über `oracle_update_with(…, true)` auf (ghostctl:2557, 2576). Der Test `zinsschritt_passt_zum_vertrag` wird beim Rückbau der Begrenzung rot (R4). Der Rückbau der Verdrahtung bleibt dagegen grün (R3). Ein manuelles `oracle-update --rate 3.3` ist weiter erlaubt, wird aber danach vom Agenten verkraftet. |
| **M-2** `deploy --rate` ohne Obergrenze | **behoben**. Rückbau bleibt grün | ghostctl:1090: Erlaubt ist 0 … `RATE_MAX_PCT`, auf dem 0,5-Raster. Kein Test (R6). |
| **M-3** Fremde reguläre Ankündigung bleibt stumm | **behoben**. Rückbau bleibt grün | store.rs:387–403: Jede fremde nonce-Änderung erzeugt einmal je nonce eine Meldung und `foreign_change`. Status: `foreignChange` (ghostctl:2728). Seite: OracleCard.tsx:36–53. `signers cancel` funktioniert auch ohne eigene `rotation` (ops.rs:600–616). Den vorgeschlagenen Ticket-Outpoint und Satz-Hash aus der Kette liest der Code nicht, er warnt nur. Kein Test (R7). |
| **N-1** Tickets fallen aus der Datei | **behoben**, mit neuem Folgefehler G-4 | `retire_rotation` ops.rs:680–684, aufgerufen in propose (591), cancel (612) und oracle_update bei `emerg` (491). In resync store.rs:392. `signers clear` räumt alle auf (ghostctl:2804–2808). R8 und R9 rot, R10 (oracle_update) und R17 (Schleife) grün. |
| **N-2** Zinstakt Uhr gegen DAA | **behoben**, mit Nebenwirkung G-5 | ghostctl:2609–2611: Ist die DAA-Pause nicht um, geht nur der Preis hinaus. Test rot bei Rückbau (R5). |
| **N-3** Byte-Reihenfolge der spk-Version | **behoben** | contracts.rs:474, 484 auf BE. Der Test `zinsziel_bytes_wie_der_konsens` (v4_ops_tests.rs:378) wird beim Rückbau rot (R11). `spk_from_bytes` begrenzt die Version nicht auf 0 (Vorschlag aus Audit 14). Das ist folgenlos. |
| Hinweis b (Notfall neben eigener Ankündigung) | **behoben** | store.rs:387–401: Die eigene Ankündigung wandert zu den alten Tickets, und die Meldung trägt „, Notfallweg“. |
| Fehlende Tests 1–8 | **großteils offen** | Neu sind nur Test 8 (redeem, liquidate und sweep bei eingefrorenem Orakel, v4_ops_tests.rs:150–159) und Test 6 (`ersetzte_ankuendigung_bleibt_aufraeumbar`). Für Test 3 (`resync`/`store` mit Mock) und Test 7 (Agent-Logik) gibt es weiterhin nichts. |

### Audit 14, ghostctl, Probe und Seite

| Befund | Urteil | Beleg |
|---|---|---|
| **H1** Stufe 3 startet nie | **behoben** (Code und Skript), aber ohne Test | `readyInHours` darf negativ sein (ghostctl:2738). Probe:174–190 behandelt `rotation == null` als „schon aktiviert“. Mit Attrappe läuft der Normalablauf 1→2→3→done durch. Rückbau R13 bleibt grün. Die Seite klemmt die Anzeige korrekt auf ≥ 0 (OracleCard.tsx:57). |
| **H2** siehe H-1 | **behoben** im Code | Die Seitentexte sind unverändert (G-9). |
| **H3** siehe M-1/M-2 | **behoben** | wie oben |
| **M1** Stufe 1 meldet Erfolg trotz Statusfehler | **behoben**, mit Einschränkung | `must` (Probe:65) und die Abschlussprüfung Probe:117. Belegt mit Attrappe: Ein Statusfehler bei `seq` bricht sauber ab. Scheitert der Status bei Probe:117, kommen zwei Fehlermeldungen, und die zweite ist falsch („Vault hat nicht die erwartete Schuld“), siehe G-3. Zusätzlich G-10. |
| **M2** Rücküberweisung fällt still aus | **behoben** | Probe:202–203. Belegt: Ist das Guthaben nicht lesbar, endet der Lauf mit „✗ … erneut starten“, und die Stufendatei bleibt 3. Ein Neustart überweist. Ein erneutes Abfragen des Guthabens nach dem Senden fehlt weiter. |
| **M3** `DRY=1` schaltet die Stufe weiter | **behoben** | Probe:118, 152, 166, 209 nur ohne `DRY_RUN`. Belegt: Ein Probelauf mit vorhandener Zustandsdatei schreibt nichts. Er endet aber mit der irreführenden Meldung „Status nicht lesbar (Node erreichbar?)“, weil `d["vaults"][0]` ohne Vault einen IndexError wirft. |
| **M4** „✓“-Prüfungen testen nur ghostctl | **teilweise** | Die Texte sind jetzt ehrlich (Probe:136, 183). Die neue Konsens-Probe für zu frühes Aktivieren steht in Probe:159–165, ist aber fehlerhaft (G-1, G-2). Offen bleibt die Vertragssperre beim Prägen. Ebenfalls offen: Der Test „alter Unterzeichner“ (Probe:191) braucht weiter einen Marktpreis, kein `--usd`, und gibt sonst Fehlalarm. |
| **M5** fremde Ankündigungen unsichtbar | **behoben** | wie M-3. Ohne Test. |
| **N1** | wie N-2 | |
| **N2** doppelte Finanzierung | **behoben** | Probe:81–82: Ein nicht lesbares Guthaben führt zum Abbruch. `FUND` wird weiter nicht geprüft (Probe:36). |
| **N3** Stufe 2 nach Ankündigung nicht idempotent | **behoben** | Probe:127, 155–157: Bei vorhandener `rotation` wird nicht neu angekündigt. Zum neuen Folgefehler siehe G-1. |
| **N4** Deploy-Fortsetzung ignoriert Schalter | **teilweise** | ghostctl:1164–1173 lehnt abweichende `--probe`, `--threshold` und `--rate` ab. Startpreis und Start-DAA vom ersten Aufruf bleiben, und das ist nicht behandelt. Wird `--threshold` beim Fortsetzen weggelassen, fällt das nicht auf. Ohne Test (R15). |
| **N5** v4 ohne `--state` auf v3-Datei | **behoben**. Rückbau bleibt grün | ghostctl:1039–1045. Folge für N7 siehe G-7. |
| **N6** „Zinskasse“ | **nicht behoben** | Das Wort steht noch in VaultList.tsx, Vault.tsx, HowItWorks.tsx, Faq.tsx, commands.ts, precheck.ts, vaultMath.ts und status.ts. |
| **N7** falscher Binary-Name, v1 als „3“ | **behoben, aber unerreichbar** | ghostctl:718–721, G-7 |
| **N8** Vault-Liste bei Einfrieren | **behoben** | VaultList.tsx:63–69, 131, 145. Rückbau auf b046a49: Alle 442 App-Tests bleiben grün. |
| **N9** `readyInHours` ab Bau-DAA | **behoben** (Puffer 600 DAA) | ops.rs:595. Rückbau bleibt grün (R12). Wird das Ticket erst nach mehr als ~1 min aufgenommen, scheitert Stufe 3 einmal mit „zu früh?“. Das ist vertretbar. |

---

## 2. Rückbau-Probe

Jede Zeile setzt die Kernänderung in der Arbeitskopie zurück. Rot heißt, ein Test schlägt fehl.

| R | Zurückgesetzt | Datei:Zeile | Ergebnis |
|---|---|---|---|
| R4 | Begrenzung in `fit_rate` (`let r = want`) | ghostctl:2612 | **rot**: `zinsschritt_passt_zum_vertrag` (ghostctl:4690) |
| R5 | DAA-Pause in `fit_rate` | ghostctl:2609 | **rot**: dieselbe Funktion (ghostctl:4704) |
| R8 | `retire_rotation` in `propose` | ops.rs:591 | **rot**: `ersetzte_ankuendigung_bleibt_aufraeumbar` (v4_ops_tests.rs:365) |
| R9 | `retire_rotation` in `cancel_rotation` | ops.rs:612 | **rot**: `absage_entwertet_das_ticket` (v4_ops_tests.rs:229) |
| R11 | spk-Version wieder LE | contracts.rs:474, 484 | **rot**: `zinsziel_bytes_wie_der_konsens` (v4_ops_tests.rs:381) |
| R1 | `feed_due` wieder `age_min > max_age_min` | ghostctl:2626 | **grün** |
| R2 | `frozen` → Update fällig entfernt | ghostctl:2623–2625 | **grün** |
| R3 | Agent ruft `oracle_update_with(…, false)` auf | ghostctl:2557, 2576 | **grün** |
| R6 | `deploy --rate` wieder nur ≥ 0 | ghostctl:1090 | **grün** |
| R7 | ganzes `store.rs` auf b046a49 (H-2, M-3/M5) | store.rs:363–403 | **grün** |
| R10 | `oracle_update` bei `emerg` wieder `rotation = None` | ops.rs:491 | **grün** |
| R12 | Puffer +600 DAA entfernt | ops.rs:595 | **grün** |
| R13 | `readyInHours` wieder `.max(0.0)` | ghostctl:2738 | **grün** |
| R14 | N5-Prüfung in `run` aus | ghostctl:1042 | **grün** |
| R15 | N4-Prüfung beim Fortsetzen aus | ghostctl:1165 | **grün** |
| R16 | Sperre bei `signers_unknown` aus | ghostctl:2666 | **grün** |
| R17 | `signers clear` räumt nur das erste Ticket | ghostctl:2804 | **grün** |
| – | VaultList.tsx auf b046a49 (N8) | VaultList.tsx | **grün** (vitest 442/442) |

R1–R3, R6, R7, R10 und R12–R17 liefen **gemeinsam** in einem vollständigen `cargo test --offline`. Ergebnis: 627 bestanden, 0 fehlgeschlagen. **Diese Tests beweisen für die betroffenen Korrekturen nichts.** Am schwersten wiegt das für:
- R7: Die Register-Logik von `resync` ist ganz ohne Test. Für den Test braucht `store` eine Attrappe für `Net`, wie schon in Audit 14 Abschnitt 6 Punkt 3 gefordert.
- R1 und R2: Der Herzschlag ist nur als Hilfsfunktion getestet. Die Verwendung in `feed_due` prüft kein Test.
- R13: Kein Test liest `signers_json`, obwohl genau dieser Wert H1 auslöste.

Das Probe-Skript und die OracleCard haben keinen Test. Ein Test wie `a12_cx1_startskript_nennt_alle_aufgaben` für das Probe-Skript (Audit 14 Fehlende Tests Punkt 10) fehlt weiterhin.

---

## 3. Probe-Skript

Mit der Attrappe geprüft. Der Normalablauf Stufe 1 → 2 → 3 → `done` läuft durch, ein vierter Start meldet „schon abgeschlossen“.

### G-1 (mittel): Konsens-Probe nach Ablauf der Wartezeit führt zu Fehlalarm und Stillstand

**Was passiert:**
- Probe:159–163 aktiviert in Stufe 2 bedingungslos, sobald eine `rotation` existiert.
- Eine Zeitprüfung gibt es nicht. `ghostctl signers activate` gibt bei zu früher Aktivierung nur einen Hinweis aus und sendet trotzdem (ghostctl:2791–2793).

**Wie man hineingerät:** Wird Stufe 2 erst nach Ablauf der Wartezeit (Probe: 1 h) erneut gestartet, passiert Folgendes:
1. Der Node nimmt die Aktivierung **an**, der Austausch ist wirksam.
2. Das Skript meldet „Der Node hat die zu frühe Aktivierung ANGENOMMEN – Wartezeit wird nicht erzwungen! Bitte sofort Claude Bescheid geben“. Das ist ein Fehlalarm.
3. Die Stufendatei bleibt `2b`.
4. Jeder weitere Start sieht `rotation == null` und ruft erneut `signers propose` mit `--committee SIGNER1` auf (Probe:154). Das scheitert, weil der Satz jetzt Unterzeichner 2 ist („nötigen Schlüsseln“). **Das Skript kommt nie mehr in Stufe 3.**

Dahin kommt man auf natürlichem Weg:
- `signers propose` wurde gesendet, aber das Warten auf Bestätigung lief ab. Das ist genau der Fall N3.
- Man hat bei „Alles so ausführen?“ abgebrochen.
- Die Probe scheiterte am Netz, und man startet erst später neu.

**Beleg (Attrappe, Szenario S2):** Ankündigung gesendet, Bestätigung gemeldet als gescheitert, Neustart 2 h später. Ausgabe: „✗ Der Node hat die zu frühe Aktivierung ANGENOMMEN“, danach `signer = signer2`, Stufe `2b`. Der dritte Start endet mit „✗ Ankündigung fehlgeschlagen“, und so geht es bei jedem weiteren Start weiter.

**Vorschlag:**
- Vor der Probe `readyInHours` lesen. Ist er ≤ 0, die Probe überspringen und mit „Wartezeit schon um, Probe nicht mehr möglich“ Stufe 3 eintragen.
- In Stufe 2b mit `rotation == null` prüfen, ob `signers.set.keys` schon Unterzeichner 2 enthält. Dann direkt Stufe 3 eintragen.

### G-2 (mittel): Erfolgsbedingung der Konsens-Probe ist zu weit

**Was passiert:** Probe:160–165 wertet **jeden** Exit-Code ≠ 0 als „✓ zu frühe Aktivierung abgelehnt“ und schaltet auf Stufe 3. Darunter fallen auch:
- „keine Ankündigung offen“;
- `Net::connect` scheitert;
- die Sperre ist nach 120 s noch belegt;
- `funds()` scheitert;
- Fehler in `ops::activate_rotation` vor dem Senden, etwa „Zustandsdatei passt nicht zum Ticket“ (ops.rs:626–628).

Die Prüfung der Sequenzsperre im Mainnet, um die es in M4 ging, ist dann nicht erbracht. Das Skript bittet nur, die letzten drei Zeilen weiterzugeben, „falls sie nicht nach Sperre/Sequenz aussieht“.

**Beleg (S7):** Ein eingespeister Fehler „FEHLER (injiziert: activate)“ führt zu „✓ zu frühe Aktivierung abgelehnt“ und Stufe 3.

**Vorschlag:** Erfolg nur, wenn die Ausgabe `Node lehnt ab` (net.rs:213) **und** `sequence lock` enthält. Der Text des Konsens lautet „one of the transaction sequence locks conditions was not met“ (rusty-kaspa a41a333 `consensus/core/src/errors/tx.rs:75`). Der Mempool prüft die Sperre ebenfalls (`tx_validation_in_utxo_context.rs:53`). Bei jedem anderen Fehler mit `fail` abbrechen, ohne Stufe 3 einzutragen.

### G-3 (mittel): `must` in Bedingungen bricht nicht ab

**Was passiert:**
- `must` ruft `fail` in der Subshell der Kommandosubstitution auf. `exit 1` beendet nur diese Subshell.
- Bei Zuweisungen mit `|| exit 1` (Probe:103, 109, 113, 127, 129, 130, 174, 176, 193) ist das korrekt. Dort gibt es genau eine Meldung und eine Taste, dann ist Schluss.
- Bei `[ "$(must …)" … ]` läuft das Skript dagegen mit leerem Wert weiter. Das betrifft Probe:117, 146, 150, 195, 196 und 197.
- Probe:196/197: Ist der Wert leer, gilt `"" != 0` und `"" != "0.0"`. Das Skript ruft dann `repay` und `close` auf, ohne den Status zu kennen.
- Probe:117, 146 und 150: Es kommt eine zweite, falsche Fehlermeldung samt zweiter Tastenabfrage.

**Beleg:**
- S3: Der Status scheitert einmal bei Probe:196. Ausgabe: „✗ Status nicht lesbar (Node erreichbar?) – einfach erneut starten“. Danach läuft das Skript weiter: Tilgen, Schließen, Rücküberweisung, „✓ Probe abgeschlossen“, Stufe `done`.
- S4b: Bei Probe:117 erscheinen „✗ Status nicht lesbar“ und gleich danach „✗ Vault hat nicht die erwartete Schuld von 0.1 GHOST – bitte Claude fragen“.

**Folge:** Geld geht keines verloren, denn ghostctl prüft vor dem Senden. Die Meldungen widersprechen sich aber, und ein „✓ abgeschlossen“ folgt auf ein „✗“.

**Vorschlag:** Überall `x=$(must …) || exit 1` und danach vergleichen.

### Weitere Beobachtungen zum Skript

- **Fortsetzung in Stufe 2:**
  - Abbruch nach dem Einfrieren und vor dem Auftauen: richtig. `frozen = True` überspringt Zeitprüfung und Einfrieren.
  - Abbruch nach dem Auftauen, aber bevor `2b` geschrieben ist (etwa wenn `must` in Probe:150 scheitert): Der nächste Start meldet „Noch zu früh: Einfrieren geht erst in … Minuten“. Nach einer weiteren Stunde friert er erneut ein. Das kostet nur Zeit und Gebühren.
- **Stufe 3 nicht idempotent (G-8, niedrig):**
  - Probe:193–195 sendet bei jedem Neustart ein neues Preis-Update mit Unterzeichner 2. In S6 stieg `seq` von 3 auf 4.
  - Ein Neustart innerhalb einer Minute scheitert am Mindestabstand (ops.rs:504) mit „Preis-Update mit Unterzeichner 2 fehlgeschlagen“.
  - Vorschlag: In der Stufendatei `3b` nach dem Update eintragen.
- **Stufe 3, „alter Unterzeichner abgewiesen“ (Probe:191):** `oracle-update` ohne `--usd` braucht weiter einen Marktpreis. Ohne Kursquelle gibt es Fehlalarm (M4 Rest). Vorschlag: `--usd` mit dem Status-Preis übergeben.
- **DRY=1:** Nach dem Deployment endet der Probelauf mit „Status nicht lesbar (Node erreichbar?)“ (IndexError bei `vaults[0]`). Das ist irreführend, aber harmlos.
- **zsh:**
  - Die Prompts von `read "?…"` erscheinen mit TTY; geprüft mit Pseudo-TTY.
  - `out=$(…); if [ $? -eq 0 ]` (Probe:160–161) liefert korrekt den Status des Befehls.
  - Werte aus dem Status werden weiter ungeprüft in Python-Code eingesetzt (Probe:92, 131, 177, 204). Das Risiko ist gering.
- **Rücküberweisung (Probe:202–208):** Sie ist richtig. Bei weniger als 0,3 KAS über dem Rest von 0,2 KAS wird nichts gesendet, und der Lauf meldet trotzdem „✓ abgeschlossen“. Das ist gewollt. `--to "$OWNER"` lädt weiter den geheimen Owner-Schlüssel in den v4-Prozess (Hinweis aus Audit 14, offen).
- **Offen aus Audit 14:**
  - keine Skript-Sperre gegen einen Doppelstart;
  - `FUND` ungeprüft.

---

## 4. `store::resync`, Register-Abschnitt (store.rs:359–403)

| Fall | Verhalten | Urteil |
|---|---|---|
| **Austausch durch Dritte aktiviert** (eigene Ankündigung) | `set_hash` geändert. Der eigene Satz passt (store.rs:369), also werden `signer_set`, `fallback_set` und `rotation = None` übernommen, und eine Meldung erscheint. `adopted` überspringt den nonce-Block. | **richtig**, mit Lücke G-6 |
| **Austausch von fremdem Rechner** (unbekannter Satz) | `known = false`: Meldung einmalig, `signers_unknown = true` (store.rs:379–385). Preis-Updates sind gesperrt (ghostctl:2666), alles andere läuft. Eine eigene offene Ankündigung kommt zu `old_tickets` (store.rs:388–393). | **richtig** |
| **Fremde Ankündigung** (regulär) | nonce+1, `known`, `foreign_change = nonce`, Meldung einmal je nonce (store.rs:395–402). Status und Seite zeigen sie, solange die nonce gleich bleibt. `signers cancel` geht ohne eigene `rotation`. | **richtig**. Das fremde Ticket wird nicht verfolgt, siehe Vorschlag M-3 |
| **Eigene Ankündigung von Dritten abgesagt** | nonce+1, Ticket-nonce ≠ Register-nonce: Meldung „abgesagt oder überholt“, das Ticket kommt zu `old_tickets`, dazu die Meldung „von außen angekündigt oder abgesagt“. | **richtig** |
| **Notfall** (fremd) | wie fremde Ankündigung, Text mit „, Notfallweg“. Die eigene offene Ankündigung wird stillgelegt. Das nächste eigene Preis-Update entwertet den Notfall, und `retire_rotation` in ops.rs:491 greift. OracleCard zeigt `emergencyOpen`. | **richtig**. Den früheren deutlichen Hinweis „Ein Preis-Update macht ihn ungültig“ gibt es in der CLI nicht mehr, nur noch auf der Seite. |
| **nonce unverändert** | kein Eintrag. `signers_unknown` wird nur neu berechnet. | **richtig** |
| Preis-Update eines zweiten Betreiber-Rechners bei `emerg` | nonce+1 durch `attestPrice` wird als „Austausch angekündigt oder abgesagt“ gemeldet | **Fehlalarm** (niedrig) |

### G-4 (niedrig–mittel): `old_tickets` wird nie bereinigt

**Was passiert:**
- `old_tickets` wächst in ops.rs:682 und store.rs:392.
- Es schrumpft nur durch das eigene `clear_ticket` (ops.rs:675).
- `clear_ticket` ist erlaubnisfrei. Wer aufräumt, bekommt 1 KAS (Audit 14 Hinweis e), es ist also zu erwarten, dass Dritte das tun.
- Hat ein Dritter ein Ticket schon aufgeräumt, baut `signers clear` (ghostctl:2804–2808) eine Tx mit verbrauchtem Eingang, und der Node lehnt ab. Die Schleife bricht beim ersten solchen Eintrag ab.
- Der Eintrag bleibt für immer in der Liste. Alle späteren Tickets dahinter lassen sich mit `signers clear` nie mehr aufräumen.
- `oldTickets` im Status zählt verbrauchte Tickets mit.

**Vorschlag:**
- In `resync` jeden Eintrag von `old_tickets` mit `net.exists` prüfen und verbrauchte entfernen.
- In der Schleife einzelne Fehler überspringen statt abzubrechen.

### G-6 (niedrig): Übernahme nur über den Satz-Hash

**Was passiert:**
- store.rs:369 vergleicht nur `rot.set.hash()` mit `set_hash`. `fb_hash` wird nicht verglichen.
- Angenommen, die eigene Ankündigung wurde ohne zwischenzeitlichen Abgleich durch eine fremde ersetzt, mit **gleichem** Satz, aber anderem Notfallsatz, und dann aktiviert. Dann übernimmt `resync` den eigenen `fallback`, der zum Register nicht passt. Es gibt keine Meldung, denn `known` ist wahr.
- Das alte eigene Ticket wird dabei nicht in `old_tickets` gelegt.
- Voraussetzung sind tRot fremde Schlüssel. Deshalb ist die Schwere niedrig.

**Vorschlag:** Bei der Übernahme auch `fb_hash` vergleichen. Bei Abweichung `signers_unknown` setzen und `rotation.ticket` zu `old_tickets` legen.

### G-5 (niedrig): Nebenwirkung von `fit_rate`

**Was passiert:**
- In der reinen Zinsrunde (Zweig `Ok(None)`, ghostctl:2567–2580) liefert `fit_rate` bei noch laufender DAA-Pause den alten Zins (ghostctl:2609–2611).
- Dann geht ein Preis-Update **ohne** Preisänderung und ohne Zinsänderung hinaus. Das ist eine überflüssige Tx mit Gebühr.
- Weil gesendet wurde, nimmt `rate_release` die Vormerkung nicht zurück (ghostctl:2497–2500). Die Zinsregel wartet also eine weitere Stunde, obwohl sich der Zins nicht geändert hat.
- Im Zweig „Update fällig“ verliert die Regel ebenfalls die Stunde.

**Vorschlag:** In der reinen Zinsrunde nicht senden, wenn `fit_rate` den Zins unverändert lässt. In beiden Zweigen die Vormerkung zurücknehmen, wenn sich der Zins nicht geändert hat.

### G-7 (niedrig): Die neue v1/v2/v3-Meldung ist unerreichbar

**Was passiert:**
- Die N5-Prüfung in `run` (ghostctl:1039–1045) greift für **jede** Datei ohne `register`, und zwar vor jedem Befehl. Sie gibt nur „stammt von einer älteren Version … (--state angeben)“ aus.
- `Ctx::load` liest denselben Pfad. Die Unterscheidung in ghostctl:716–724, die mit N7 korrigiert wurde, kommt deshalb nicht mehr zum Zug.

**Vorschlag:** Die Versionserkennung in eine Funktion ziehen und in `run` dieselbe Meldung ausgeben. Die v1-Erkennung (`oracle_params.signers` und kein `vault_params.max_debt`) passt zur Historie: `max_debt` kam mit e3c6f36, das liegt nach dem Tag `v1-mainnet`.

### G-9 (niedrig): Offene Teile

- **Seite H2:** Oracle.tsx:120 und HowItWorks.tsx:319 nennen weiter „6 Stunden“. Tatsächlich sind es jetzt höchstens 60 min.
- **N6:** „Zinskasse“ steht in 8 Dateien unter `app/src`, siehe Tabelle oben.
- **N4:** Der Startpreis aus dem ersten Aufruf bleibt.
- **M4:** Die Vertragssperre beim Prägen wird nicht geprüft.

### G-10 (Hinweis): `status --json` verschluckt Fehler beim Abgleich

- `status_json` (ghostctl:1847–1865) liefert bei einem `resync`-Fehler den Stand aus der Datei, mit `fresh: false`.
- Kann der DAA nicht gelesen werden, gilt `daa = 0` (ghostctl:1865). Dann werden `freezeInMinutes` und `readyInHours` riesig.
- `must` prüft nur, ob die Ausgabe leer ist. Das Skript entscheidet dann auf veralteten Daten. Dank der Vorprüfungen in ghostctl ist das ungefährlich, aber die Meldungen führen in die Irre („Noch zu früh: ~N Minuten“).
- Vorschlag: In `must` zusätzlich `d["oracle"]["fresh"]` und `d["daa"] > 0` verlangen.

---

## 5. Empfohlene Reihenfolge

1. **G-1 und G-2** im Probe-Skript beheben, bevor Stufe 2 im Mainnet läuft.
2. **G-3:** alle `[ "$(must …)" … ]` umstellen.
3. **G-4:** `old_tickets` mit der Kette abgleichen.
4. **Tests nachziehen:**
   - eine Attrappe für `Net` mit Tests der sechs `resync`-Fälle aus Abschnitt 4 (R7);
   - `feed_due` bzw. `oracle_round` mit Herzschlag und `frozen` (R1–R3);
   - `signers_json.readyInHours` negativ (R13);
   - ein Skript-Test für das Probe-Skript (G-1 bis G-3, M3).
5. **Texte** der Seite für H2 und N6 anpassen.
