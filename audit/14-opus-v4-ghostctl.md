# Audit 14: ghostctl v4, Mainnet-Probe und Seite (Commits b0f6d86, b046a49)

Prüfer: Claude Opus 5.5, 05.10.2026. Nur lesend gearbeitet: nichts committet, keine Transaktion gesendet, ghostctl nicht gegen das Netz gestartet, `keys/` und `deployments/` nicht gelesen (nur Dateinamen aus `ls`).
Ausgeführt: `cargo test --release --offline --bin ghostctl` (18 bestanden), `node_modules/.bin/vitest run` in `app` (24 Dateien, 442 bestanden).

Zeilenangaben beziehen sich auf den Stand b046a49. „Probe" steht für `GHOST-v4-Probe.command`, „ghostctl" für `protocol/src/bin/ghostctl.rs`.

## Kurzfassung

- **Kann Version 3 gestört werden?** Nicht nach dem, was ich geprüft habe. Alle v4-Aufrufe im Netz laufen mit `--state deployments/mainnet-v4probe.json`. Sperre, Journal, Deploy-Fortschritt, Zinsdatei und Sendeverlauf leiten sich alle von diesem Pfad ab. Das v4-Programm wird in einen eigenen Build-Ordner gebaut. Version 3 wird nur an einer Stelle berührt: Es gibt eine einzige Überweisung vom Owner-Schlüssel, und die geht über ghostctl v3 und dessen eigene Sperre.
- **Das Probe-Skript ist nicht robust:**
  - Stufe 3 kann nie starten (H1).
  - Bei einem Statusfehler meldet Stufe 1 trotzdem Erfolg (M1).
  - Die Rücküberweisung kann still ausfallen (M2).
  - `DRY=1` kann die Stufendatei weiterschalten (M3).
  - Zwei „✓"-Meldungen beruhen nur auf lokalen Vorprüfungen von ghostctl, nicht auf dem Vertrag (M4).
- **Agent:** Zwei Fehler können dazu führen, dass der Agent sein eigenes Orakel einfriert:
  - Er aktualisiert in ruhigen Phasen nur alle 6 h, friert aber nach 2 h ein (H2).
  - Ein Zinssatz außerhalb des 0,5-Raster lässt jedes Preis-Update scheitern (H3).

  Gebühren verbrennt der Agent dabei nicht, weil die Fehler vor dem Senden auftreten.
- **Kritisch:** kein Befund.

| Nr | Schwere | Kurz |
|---|---|---|
| H1 | hoch | Stufe 3 startet nie (`readyInHours` ist ab 0 abgeschnitten) und ist nach `activate` nicht fortsetzbar |
| H2 | hoch | Agent aktualisiert bei ruhigem Kurs erst nach 6 h, friert aber nach 2 h ein, das eigene Orakel friert regelmäßig ein |
| H3 | hoch | Zinsregel schlägt bei Zinssatz außerhalb des Rasters Schritte bis 0,75 Punkte vor; der Vertrag lehnt ab, das Preis-Update gleich mit; danach friert der eigene Keeper ein. `deploy --rate` > 20,5 % macht das Orakel dauerhaft unbedienbar |
| M1 | mittel | Stufe 1 meldet Erfolg und schaltet weiter, obwohl Vault/Prägen übersprungen wurden (Status nicht lesbar) |
| M2 | mittel | Stufe 3: Rücküberweisung fällt still aus, wenn das Guthaben nicht lesbar ist; trotzdem „Probe abgeschlossen" |
| M3 | mittel | `DRY=1` nach echtem Deployment schreibt `stufe = 2` |
| M4 | mittel | „Prägen gesperrt" und „alter Unterzeichner abgewiesen" prüfen nur ghostctl selbst, nicht Vertrag/Node; frühe Aktivierung wird im Mainnet nie probiert |
| M5 | mittel | Ankündigungen, die nicht von dieser Zustandsdatei stammen, werden weder erkannt noch auf der Seite gezeigt |
| N1 | niedrig | Zinstakt in Unix-Sekunden, Vertrag in DAA: Ein Update genau an der Stundengrenze scheitert ganz |
| N2 | niedrig | Doppelte Finanzierung (25 KAS) möglich, wenn das Guthaben nicht lesbar ist |
| N3 | niedrig | Stufe 2 nach gesendeter Ankündigung erneut: neues Ticket, altes 1-KAS-Ticket bleibt liegen |
| N4 | niedrig | Deploy-Fortsetzung ignoriert geänderte `--probe/--threshold/--rate` still; Startpreis/-DAA bleiben vom ersten Aufruf |
| N5 | niedrig | ghostctl v4 ohne `--state` arbeitet auf `deployments/mainnet.json`; nur die GHOST-Befehle sind durch `Ctx::load` geschützt |
| N6 | niedrig | Seite spricht in v4 weiter von „Zinskasse", der Zins geht aber an die Betreiberadresse |
| N7 | niedrig | Fehlermeldung nennt `bin/ghostctl-v3`, das es nicht gibt; v1-Dateien heißen „Version 3" |
| N8 | niedrig | Vault-Liste zeigt Liquidieren/Auflösen auch bei eingefrorenem Orakel an (erst das Formular sperrt) |
| N9 | niedrig | `readyInHours` rechnet ab Bau-DAA, nicht ab Ticket-Aufnahme |
| Hinweise | – | siehe Abschnitt 6 |

---

## 1. Kann die Probe oder ghostctl v4 Version 3 stören?

### Geprüft und getrennt

- **Zustandsdatei:**
  - Jeder Netz-Aufruf von v4 im Skript geht über `g4`, `g4check`, `status_field` oder `kas_of`. Alle vier setzen `--state "$STATE"` (Probe:61–64).
  - Ohne `--state` laufen nur `keygen` und `committee-keygen` (Probe:77–78). Beide brauchen kein Netz und lesen keine Zustandsdatei (ghostctl:977–989).
- **Sperre:** `store::lock` → `state.with_extension("lock")` (store.rs:32–33) → `deployments/mainnet-v4probe.lock`. Version 3 nutzt `mainnet.lock`.
- **Journal:**
  - `pending_path` → `mainnet-v4probe.pending.json` (store.rs:74–75).
  - `write_pending` schreibt nur dorthin (ghostctl:806). Die Zieldatei im Journal ist ebenfalls die Probe-Datei bzw. `mainnet-v4probe.deploy.json` (ghostctl:1089, 1154 `save_prog`).
- **Weitere abgeleitete Dateien:**
  - Zinsregel: `mainnet-v4probe-zins.json` (rate.rs:89–91).
  - Sendeverlauf: `mainnet-v4probe-txlog.jsonl` (abo.rs:673).
  - Daueraufträge und Tresore ebenfalls mit Präfix `mainnet-v4probe-` (abo.rs:558–560, tresor.rs:1177–1179).
  - Zwischen v3 und v4 kollidiert keine dieser Dateien.
- **Programm:**
  - `.cargo/config.toml` setzt `target-dir = "vendor/silverscript/target"`, relativ zum jeweiligen Worktree. v4 baut nach `kaspa-lending-v4/vendor/silverscript/target/release/ghostctl`, das ist `BIN` in Probe:39.
  - Der Wrapper `kaspa-lending/ghostctl` von v3 nutzt `target/debug` im v3-Ordner.
  - `abo.rs`, `tresor.rs`, `rate.rs` und `net.rs` sind zwischen v3 (HEAD 9dc741d) und v4 unverändert. Nur `store.rs` ist um das Nachführen des Registers ergänzt (`git diff 9dc741d HEAD --stat`). Das Journal-Format ist also gleich.
- **UTXO-Konflikte:**
  - Die Probe gibt nur UTXOs von `keys/v4probe-owner.json` aus.
  - Den Owner-Schlüssel berührt sie einmal: Probe:93 `./ghostctl --ja send` mit dem v3-Programm und der v3-Standarddatei. Das ist eine gewöhnliche v3-Überweisung. Sie wartet bis zu 120 s auf `mainnet.lock` (ghostctl:1043–1045, gleicher Code in v3). Orakel-, Keeper- und Abo-Schritte des v3-Agenten halten dieselbe Sperre (ghostctl:1393, 3215).
  - Die Rücküberweisung (Probe:171) erzeugt nur einen neuen Owner-UTXO.
- **Spuren:** Im v3-Ordner liegt bereits `deployments/mainnet-v4probe.lock`. Das passt zu einem Probelauf am 04.10. um 23:58; die Datei habe ich nicht gelesen.

### Restpunkte zu Frage 1

- **N5 (niedrig), Standardpfade:** Ohne `--state` nimmt v4 `deployments/mainnet.json` (ghostctl:1038). Im v3-Ordner aufgerufen gilt Folgendes:
  - **Geschützt:** Alle GHOST-Befehle scheitern sauber an der Versionsprüfung (ghostctl:716–722). `deploy` bricht mit „existiert schon" ab (ghostctl:1070–1079).
  - **Nicht geschützt:** `send`, `balance`, `status`, `abo run`, `tresor …` und die Abo-/Tresor-Schritte von `agent` (ghostctl:1415–1416). Sie arbeiten auf den v3-Dateien, nehmen `mainnet.lock` und schreiben `mainnet.pending.json`. `status --json` ruft vorher `resolve_pending` auf die v3-Datei auf (ghostctl:1252).
  - **Einordnung:** Wegen des gleichen Codes ist das kooperativ und nicht zerstörerisch. Ein versehentlich gestarteter `ghostctl-v4 agent` würde aber v3-Daueraufträge und -Tresore mitbedienen. Doppelzahlungen sollten die gemeinsame Sperre und Laufdatei verhindern; das habe ich nicht im Einzelnen geprüft.
  - **Empfehlung:** v4 ohne `--state` auf `deployments/<netz>-v4.json` legen, oder den Start verweigern, wenn die Datei fehlt und keine v4-Datei ist.
- **Hinweis:** Tresor-Schritte von v3 sperren eine andere Datei (`…-tresore.lock`, ghostctl:3925–3926) und können mit dem Owner-Schlüssel zahlen. Gleichzeitig mit Probe:93 ist dann ein UTXO-Konflikt denkbar. Folge: Eine der beiden Transaktionen lehnt der Node ab, Verlust gibt es keinen. Ob Tresor-Zahlungen Owner-UTXOs verbrauchen, habe ich nicht abschließend geprüft.
- **Hinweis:** Die Probe legt neue Dateien in `kaspa-lending/keys/` und `kaspa-lending/deployments/` an. Dazu gehören `mainnet-v4probe.json`, `.stufe`, `.deploy.json` und `.pending.json`; die `.gitignore` von v3 deckt sie nicht ab. `keys/` ist ignoriert. Getrackt sind in v3 nur `deployments/mainnet-v1.json` und `deployments/testnet-10-v1.json` (`git ls-files`).

---

## 2. Robustheit des Probe-Skripts

### H1 (hoch): Stufe 3 kann nie starten und ist nach `activate` nicht fortsetzbar

- ghostctl:2666: `"readyInHours": (r.ready_daa as f64 - daa as f64).max(0.0) / HOUR_DAA` liefert nie einen negativen Wert.
- Probe:152 verlangt aber `float(left) <= -0.05` (gedacht als 3 Minuten Puffer).
- Folge: Jede Stufe 3 endet mit „Noch zu früh: Aktivieren geht in etwa 3 Minuten" (Probe:153, `max(1, round(0*60+3))`), auch Tage später.
- Behebt man nur das: Nach einem erfolgreichen `signers activate` (Probe:162) ist `rotation` gleich `null` (ops.rs:633). Probe:149 setzt dann `readyInHours = 99`. Scheitert danach Tilgen, Schließen oder Rücküberweisen, meldet jeder Neustart „zu früh: ~5943 Minuten".
- `signers activate` ist außerdem nicht wiederholbar („keine Ankündigung offen", ghostctl:2718, ops.rs:604).
- **Empfehlung:**
  - `readyInHours` ohne `max(0.0)` ausgeben, oder `readyDaa` und `daa` getrennt.
  - Stufe 3 in Unterschritte teilen, die man überspringen kann: aktiv ⇔ `signers.set.keys` enthält Signer 2; getilgt ⇔ `debtGhost == 0`; geschlossen ⇔ `len(vaults) == 0`.

### M1 (mittel): Stufe 1 meldet Erfolg, obwohl Vault und Prägen fehlen

- Probe:107 und 110 vergleichen die Ausgabe von `status_field` mit `0` bzw. `"0.0"`.
- Scheitert der Aufruf, ist die Ausgabe leer. Das passiert bei `Net::connect` (ghostctl:1037) oder wenn die Sperre nach 120 s nicht frei wird. Die Python-Zeile scheitert dann am JSON und gibt nichts aus.
- Beide Bedingungen sind dann falsch. Das Skript überspringt Eröffnen und Prägen, schreibt `2` in die Stufendatei (Probe:113) und meldet „✓ Stufe 1 fertig".
- Geprüft wird nur `seq` auf Leere (Probe:101).
- Folgefehler: Stufe 2 meldet bei fehlendem Vault „Prägen war NICHT gesperrt" (falscher Alarm). Bei vorhandenem Vault ohne Schuld läuft Stufe 2 durch, und erst Stufe 3 scheitert beim Tilgen.
- **Empfehlung:** Jede `status_field`-Ausgabe auf Leere prüfen. Vor dem Weiterschalten der Stufe nachprüfen: `len(vaults)==1` und `debtGhost>0`.

### M2 (mittel): Rücküberweisung fällt still aus

- Probe:168–172: Ist `kas_of` leer, gilt `rest=0`, also `back=0`. Die Überweisung wird übersprungen.
- Trotzdem folgt `echo done` und „✓ Probe abgeschlossen" (Probe:173–175).
- Rund 10 KAS aus dem geschlossenen Vault bleiben dann unbemerkt auf dem Probe-Schlüssel.
- Auch im Normalfall bleiben absichtlich 0,2 KAS liegen (Probe:169). Die Kopfzeile sagt aber „Rest-KAS zurück" (Probe:19–20).
- **Empfehlung:** Leeres oder nicht numerisches `rest` als Fehler behandeln. Nach dem Senden das Guthaben erneut abfragen und anzeigen.

### M3 (mittel): `DRY=1` kann die Stufendatei weiterschalten

- Probe:98 beendet den Probelauf nur innerhalb von `if [ ! -f "$STATE" ]`.
- Existiert die Zustandsdatei schon (echtes Deployment, aber Stufe 1 noch nicht fertig), läuft `DRY=1` weiter:
  - Update, Eröffnen und Prägen nur als Probelauf (Probe:105–111);
  - danach `echo 2 > "$STAGE_FILE"` (Probe:113).
- Ein späterer echter Lauf beginnt dann in Stufe 2 ohne Vault.
- **Empfehlung:** Im Probelauf nie die Stufendatei schreiben (`(( DRY_RUN )) || echo …`).

### M4 (mittel): Zwei „✓"-Prüfungen testen nicht den Vertrag

- **Stufe 2, „Prägen gesperrt" (Probe:135–136):**
  - `g4check mint` scheitert schon an `ops::not_frozen` (ops.rs:660–665, Aufruf ops.rs:689), bevor eine Transaktion gebaut wird.
  - Damit ist nur die Vorprüfung von ghostctl getestet. Ob Vault v4 im Mainnet `mint` bei `frozen` ablehnt (stable_vault_v4.sil:214, `readOracle(…, true)`), prüft die Probe nicht.
- **Stufe 3, „alter Unterzeichner abgewiesen" (Probe:163–164):**
  - Sucht nach „nötigen Schlüsseln", also der lokalen Prüfung ghostctl:2621–2623 gegen den Satz in der Zustandsdatei. Das Register wird nicht gefragt.
  - Vorher braucht `oracle-update` ohne `--usd` einen Marktpreis (ghostctl:2604–2606). Scheitern die Kursquellen, meldet das Skript „Alter Unterzeichner wurde NICHT abgewiesen" (falscher Alarm).
- **Stufe 3, Wartezeit:** Die Probe aktiviert erst nach Ablauf. Dass der Node eine **zu frühe** Aktivierung ablehnt, prüft sie im Mainnet nie. Abschnitt 9 Punkt 8 des Entwurfs verlangt aber gerade, dass `settle` mit der Sequenzsperre echt ausgeführt wird.
- **Empfehlung:**
  - Die Texte ehrlich halten („ghostctl lehnt ab").
  - Die Ablehnung durch den Node gezielt prüfen. Das kostet keine Gebühr, denn eine abgelehnte Transaktion zahlt nichts: in Stufe 2 direkt nach der Ankündigung `signers activate` senden und „Node lehnt ab" erwarten.
  - Für die Vertragssperre beim Prägen braucht es einen Weg, der die Vorprüfung umgeht (z. B. ein verstecktes `--skip-precheck`).

### N2 (niedrig): Doppelte Finanzierung

- Ist das Guthaben nicht lesbar (`have` leer, Probe:79), wertet Probe:89 es als 0 und sendet erneut `FUND` KAS vom Owner (Probe:93).
- Das passiert auch, wenn ein abgebrochener Lauf den Probe-Schlüssel schon finanziert hat und das Deployment noch nicht fertig ist (die Zustandsdatei fehlt dann noch).
- Kein Verlust: Stufe 3 überweist den Rest zurück, sofern M2 nicht greift.
- `FUND` wird nicht geprüft (Probe:36).

### N3 (niedrig): Stufe 2 ist nach gesendeter Ankündigung nicht idempotent

- Scheitert nach `signers propose` das Warten auf Bestätigung, wird Stufe 3 nicht eingetragen (Probe:140–141).
- Ein Neustart wartet erneut 1 h und friert ein. Dann taut er auf und kündigt neu an („wird ersetzt", ghostctl:2696).
- Das alte Ticket (1 KAS) wird nicht aufgeräumt. `signers clear` kennt nur das jeweils letzte Ticket aus `d.rotation` (ghostctl:2728).
- **Empfehlung:** Vor der Ankündigung prüfen, ob `signers.rotation.valid` gesetzt ist, und dann direkt Stufe 3 eintragen.

### Weitere Beobachtungen zum Skript

- **Abbruch/Fortsetzung in Stufe 1:**
  - Grundsätzlich richtig: Die Finanzierung wird bei vorhandener Zustandsdatei übersprungen, `deploy` setzt fort, `seq==0` steuert das Update.
  - Eine Ausnahme ist die Kombination aus N2 und M1.
- **Stufe 2 ohne Statuswerte:** `freezeInMinutes` ist bei unlesbarem DAA wegen `unwrap_or(0)` (ghostctl:1842) groß und positiv. Das ist sicher („zu früh").
- **zsh:**
  - Leere Arrays `"${DRYF[@]}"` werden in zsh korrekt weggelassen.
  - `read -r "a?…"` und `read -k 1` sind zsh-Syntax, das passt zum Shebang.
  - Werte aus ghostctl werden ungeprüft in Python-Code eingesetzt (Probe:89, 124, 152, 169). Die Werte stammen aus dem eigenen Programm, das Risiko ist daher gering. Robuster ist es, sie über `sys.argv` zu übergeben.
- **Paralleler Start:** Doppelklickt man zweimal, laufen zwei Instanzen. Die ghostctl-Sperre ordnet zwar jeden einzelnen Befehl, aber nicht die Abfolge im Skript. Zweimal Stufe 1 hieße zweimal 25 KAS vom Owner. Empfehlung: eine Skript-Sperre (`mkdir …lock`).
- **Rücküberweisung:**
  - `--to "$OWNER"` lädt den geheimen Owner-Schlüssel in den v4-Prozess, nur um die Adresse zu bilden (ghostctl:2982–2983). Eine Adresse statt der Datei reicht.
- **Beträge:**
  - 25 KAS reichen für:
    - 4 Covenants je 1 KAS (`CovValues::small`);
    - Vault 10 KAS + 3 KAS Minter-Zweig;
    - Ticket 1 KAS (kommt bei `activate` als Wechselgeld zurück, ops.rs:623–628);
    - Gebühren.
  - Ab 20 KAS bleiben nach rund 18 KAS Bindung etwa 2 KAS für Gebühren. Das ist knapp, aber ausreichend.

---

## 3. Fortsetzbarkeit des Deployments (DeployProgress)

Ablauf in ghostctl:1064–1219:

- Jeder Schritt sendet mit `send_to(…, save_prog(next))`.
- Vor dem Senden schreibt `write_pending` das Journal, mit der Fortschrittsdatei als Ziel und dem vollständigen nächsten Stand.
- Erst nach der Bestätigung wird die Fortschrittsdatei geschrieben (ghostctl:826–828).
- Jeder neue `deploy`-Aufruf ruft zuerst `resolve_pending` auf (ghostctl:1066).

| Abbruch | Verhalten beim nächsten Aufruf | Bewertung |
|---|---|---|
| vor dem Senden von Schritt 1 | keine Fortschrittsdatei → neuer Startpreis/-DAA, Neubeginn | in Ordnung |
| Schritt 1 (Register-Genesis) gesendet, unbestätigt | Journal: angenommen → Fortschritt mit `register` geschrieben → weiter mit Schritt 2. Nicht angenommen, Eingänge frei → Journal verworfen, Neubeginn. Noch im Mempool → Fehler „noch unterwegs" | in Ordnung |
| nach Schritt 1 bzw. 2 (Orakel-Genesis) | weiter mit dem nächsten fehlenden Schritt (`register`/`oracle` = None) | in Ordnung |
| nach Schritt 3 (Register-Init) | `register_ready = true` steht in derselben Fortschrittsdatei wie das neue `register` → kein zweites Init | in Ordnung |
| nach Schritt 4 (Factory-Genesis) | weiter mit Schritt 5 | in Ordnung |
| Schritt 5 gesendet, unbestätigt | Journal-Ziel ist die Zustandsdatei; `resolve_pending` schreibt sie; Zustandsdatei + Fortschritt + übernommen → Fortschritt löschen, „abgeschlossen" (ghostctl:1070–1077) | in Ordnung |
| Schritt 5 bestätigt, Absturz vor `remove_file` | „existiert schon" (ghostctl:1079); die Probe überspringt `deploy`, weil die Zustandsdatei existiert (Probe:96). Die Fortschrittsdatei bleibt liegen | harmlos |
| Orakel-UTXO zwischen den Schritten von Dritten bewegt (`read`, oder `freeze` nach 1 h/2 h) | Schritt 3 und 5 lesen nur ID und Template, sie geben das Orakel nicht aus (ops.rs:219–232, init_factory ops.rs:286–290); die gespeicherte Position wird später nachgeführt | in Ordnung |

**N4 (niedrig), geänderte Parameter:**

- Geprüft werden nur Netz, Unterzeichner-Schlüssel und Deployer (ghostctl:1146–1153).
- `--probe`, `--threshold` und `--rate` eines Folgeaufrufs werden ohne Hinweis ignoriert. Es gelten die Werte aus der Fortschrittsdatei.
- Startpreis und `oracle_daa` stammen aus dem ersten Aufruf. Wird Tage später fortgesetzt, entsteht ein Orakel mit altem Preis: sofort einfrierbar, und das erste Update ist nur ×2/÷2 vom alten Preis erlaubt.
- Ändert sich der Code zwischen zwei Aufrufen (Templates), wird das nicht erkannt.
- **Empfehlung:** Abweichende Schalter ablehnen oder melden. Den Startpreis vor Schritt 2 aktualisieren, solange das Orakel noch nicht angelegt ist.

---

## 4. Agent: Einfrieren und Zinsregel

### H2 (hoch): Der Agent friert sein eigenes Orakel in ruhigen Phasen ein

- **Aktualisierung:**
  - `agent` und `oracle-feed` haben `max_age_min = 360` als Standard (ghostctl:155, 174). Das Startskript übergibt `--max-age-min 360` (`GHOST-Agent starten.command`:51).
  - `feed_due` sendet ein Update nur bei einer Kursbewegung ≥ 0,5 % oder einem Alter über 360 min (ghostctl:2568–2583).
- **Einfrieren:** Die Frist beträgt `freeze_after_daa = 2 h` (ghostctl:1133). `freeze_if_stale` im selben Agenten friert danach ein (ghostctl:1980, 1989–2011).
- **Folge:**
  - Bewegt sich KAS 2 h lang um weniger als 0,5 %, friert der Betreiber-Agent sein eigenes Orakel ein.
  - Aufgetaut wird erst bei 0,5 % Bewegung oder nach 6 h. `feed_due` beachtet `frozen` nicht.
  - In dieser Zeit sind Prägen, Rücknahme, Liquidieren, Auflösen und Tausch gesperrt. Jedes Einfrieren kostet eine Gebühr.
- **Widerspruch auf der Seite:** Oracle.tsx:120 („6 Stunden alt") steht neben Oracle.tsx:126, Faq.tsx:58 und Landing.tsx:38 („hält das Orakel frisch", „Kommt 2 Stunden lang kein Preis, friert er ein").
- **Empfehlung:**
  - `max_age_min` auf höchstens die Hälfte der Frist begrenzen (aus `freeze_after_daa` ableiten).
  - Bei `frozen` immer sofort ein Update senden.
  - Test: `feed_due` mit einem Alter knapp unter `freeze_after_daa` muss fällig sein.

### H3 (hoch): Die Zinsregel kann einen Schritt vorschlagen, den der Vertrag ablehnt

- **Raster gegen Ist-Satz:**
  - `math::rate_next` rundet den aktuellen Satz auf das 0,5-Raster und geht von dort einen Schritt (math.rs:194–210). Der Abstand zum **tatsächlichen** Satz ist dann bis zu 0,75 Punkte.
  - Der Vertrag erlaubt höchstens `rateStep = rate_from_apr(0.5)+1` (price_oracle_v4.sil:86–87). `ops::check_update` prüft das ebenso (ops.rs:502).
- **Nachgerechnet** (Python, gleiche Formeln):

  | Ist-Satz | GHOST-Kurs | Vorschlag | Schritt | Ergebnis |
  |---|---|---|---|---|
  | 3,3 % | 0,99 | 4,0 % | 0,70 Punkte | Vertrag lehnt ab |
  | 3,2 % | 1,01 | 2,5 % | 0,70 Punkte | Vertrag lehnt ab |
  | 1,75 % | 0,99 | 2,5 % | 0,75 Punkte | Vertrag lehnt ab |
  | Werte auf dem Raster 0…20 % | beide Richtungen | | höchstens `rateStep` (die +1 deckt die Rundung) | in Ordnung |
- **Wie ein Satz neben das Raster kommt:**
  - `deploy --rate x` mit beliebigem x ≥ 0 (ghostctl:1081, 1114);
  - `oracle-update --rate x` von Hand mit einem Schritt ≤ 0,5 (z. B. 3,0 → 3,3).
- **Folge:**
  - Im Zweig „Update fällig" fährt der Zins im **selben** Update mit (ghostctl:2519–2535). `check_update` scheitert (ghostctl:2620), also unterbleibt auch das Preis-Update.
  - `rate_release` gibt den Takt frei. In der nächsten Runde wiederholt sich das, solange GHOST außerhalb des Bands liegt, also genau dann, wenn die Regel wirken soll.
  - Nach 2 h friert der eigene Keeper das Orakel ein (siehe H2). Das Auftauen scheitert am selben Fehler.
  - Gebühren werden nicht verbrannt, weil vor dem Senden abgebrochen wird. Das System bleibt aber gesperrt, bis jemand den Zins von Hand setzt.
- **Verschärfung beim Deployment:**
  - `deploy --rate` prüft die Obergrenze nicht (ghostctl:1081). Bei `--rate` > 20,5 liegt `stable_rate` über `maxRate + rateStep`.
  - Kein Update kann dann je gültig sein, weder mit gleichem Satz (`newStableRate <= maxRate`) noch mit einem Schritt. Das Orakel ist dauerhaft tot, ein neues Deployment wird nötig.
  - Zwischen 20 und 20,5 % ist genau ein Schritt nach unten möglich.
- **Empfehlung:**
  - `rate_next` vom **Ist-Satz** aus begrenzen: `next = clamp(next, cur−0.5, cur+0.5)`.
  - Scheitert nur der Zinsteil von `check_update`, das Preis-Update ohne Zinsänderung senden.
  - `deploy --rate` auf 0 … `RATE_MAX_PCT` begrenzen und am besten aufs Raster zwingen.

### N1 (niedrig): Zinstakt in Sekunden, Vertrag in DAA

- `rate::decide` und `reserve` messen eine Stunde in Unix-Sekunden (rate.rs:160–200). Der Vertrag misst sie in DAA ab dem letzten `newOracleDaa` (price_oracle_v4.sil:85).
- `newOracleDaa` ist `daa − 20` zum Bauzeitpunkt (ghostctl:2618).
- Läuft die Kette etwas langsamer als 10 DAA/s, oder dauert der Abgleich vor dem Bau kürzer als beim letzten Mal, scheitert die erste Runde nach Ablauf der Stunde an „Zins darf sich erst ab DAA …". Mit ihr scheitert auch das Preis-Update dieser Runde.
- Die nächste Runde gelingt. Gebühren entstehen keine.
- **Empfehlung:** wie bei H3 nur den Zinsteil fallen lassen, oder den Takt aus `last_rate_daa` statt aus der Uhr ableiten.

### `freeze_if_stale` sonst

- Friert nur ein, wenn nicht schon eingefroren und die Frist abgelaufen ist (ghostctl:1990–1994). Nach einem gesendeten Einfrieren ist der Zustand gespeichert. Ein wiederholtes Einfrieren derselben Phase findet nicht statt.
- Abgelehnte Transaktionen kosten nichts. Ein Wettlauf mit einem fremden Update endet mit einer abgelehnten Transaktion; das Journal verwirft sie über die Wechselgeld-Regel (store.rs:161–167).
- Ohne Marktpreis läuft die Keeper-Runde gar nicht und friert daher auch nicht ein (ghostctl:1395–1405). Das ist richtig, aber erwähnenswert.
- Blockieren kann `freeze_if_stale` Updates nur über H2/H3. Der Vertrag lässt das Update eines eingefrorenen Orakels zu (price_oracle_v4.sil:70–98).

---

## 5. Stimmen die Texte der Seite?

| Text | Code/Vertrag | Urteil |
|---|---|---|
| Einfrieren sperrt Prägen, Rücknahme, Liquidieren, Auflösen, Abheben mit Schuld, Tausch; Einzahlen, Tilgen, Schließen, Abheben ohne Schuld, Liquidität gehen weiter (Faq, HowItWorks, Oracle, `frozenText` precheck.ts) | stable_vault_v4.sil: `readOracle(…, true)` bei sweep/mint/redeem/liquidate, `debt > 0` bei withdraw, `false` bei close/repay; Pool `stopWhenFrozen` nur bei swap (ghost_pool_v4.sil:149); `frozenBlocks` (precheck.ts) passt dazu | stimmt |
| „0,5 Prozentpunkte je Stunde, höchstens 20 %, erzwingt der Vertrag" (HowItWorks:171) | price_oracle_v4.sil:79–89, `rate_step`/`rate_gap` ghostctl:1129–1131 | stimmt (siehe aber H3) |
| Notfallsatz nach 30 Tagen, jedes Preis-Update macht die Ankündigung ungültig | `attestPrice`: `emerg` → `nonce+1` (signer_register_v4.sil:182–184) | stimmt |
| „aktivieren darf dann jeder" | `activate`/`settle` ohne Signatur (signer_register_v4.sil:262–329) | stimmt für den Vertrag; ghostctl kann nur Ankündigungen aktivieren, die es selbst gesendet hat (M5) |
| „Der GHOST-Agent hält das Orakel frisch … Kommt 2 Stunden lang kein Preis, friert er ein" (Landing:38, Faq:58, Oracle:126) neben „Update … wenn 6 Stunden alt" (Oracle:120, HowItWorks:319) | H2 | **widersprüchlich**; im Ergebnis friert der Agent sein eigenes Orakel ein |
| Ankündigung „öffentlich", Karte „Angekündigt: …" (OracleCard:19, 39, 98) | M5 | Die Karte zeigt nur Ankündigungen dieser Zustandsdatei |
| „Gültig frühestens in …" (OracleCard:39) | N9 | leicht zu früh |
| Zins „zugunsten der Zinskasse" / „an die Zinskasse" (HowItWorks:171, commands.ts:90–99, precheck.ts:258/298, VaultList.tsx:117–150, Vault.tsx:180, vaultMath.ts) | v4: `interestSpk` = P2PK des Betreibers (ghostctl:1189–1190), so steht es auch in HowItWorks:177 | **N6**: veraltet bzw. widersprüchlich |
| Fehlermeldung „Version 3 bedient bin/ghostctl-v3" (ghostctl:719) | `kaspa-lending/bin` enthält nur `ghostctl-v1` und `ghostctl-v2`, v3 ist `./ghostctl` | **N7** |

### M5 (mittel): Fremde Ankündigungen bleiben unsichtbar

- `d.rotation` wird nur von `ops::propose` gesetzt (ops.rs:580, einzige Stelle mit `Some(Rotation`).
- `resync` meldet eine fremde Ankündigung nur, wenn es eine **Notfall**-Ankündigung ist (store.rs:368–374). Eine reguläre Ankündigung von einem anderen Rechner oder mit gestohlenem Unterzeichner-Schlüssel erhöht zwar `nonce`, erzeugt aber keine Meldung, keinen Status und keine Anzeige.
- Erst nach der Aktivierung bricht `resync` ab (store.rs:362–366). Dann ist die Absagefrist von 14 Tagen vorbei.
- Die Wartezeit soll gerade das Absagen und Reagieren ermöglichen. Werkzeug und Seite sehen die Ankündigung aber nicht.
- **Empfehlung:**
  - Jede `nonce`-Änderung ohne eigene `rotation` als „ACHTUNG: Austausch angekündigt" melden und im Status als `rotation.foreign = true` ausgeben.
  - Den Agenten laut warnen lassen. Optional automatisch absagen, sofern er den Satz hält.

### N8, N9 (niedrig)

- **N8:**
  - VaultList.tsx:143–150 zeigt bei eingefrorenem Orakel weiter Buttons für „liquidieren" und „Auflösen". Das Formular sperrt erst danach über `precheck` (ActionForms.tsx:226, 581).
  - Abschnitt 9 Punkt 7 des Entwurfs verlangt „ausgrauen".
- **N9:** `ready_daa = daa − 20 + delay` beim Bau (ops.rs:580). Die Sperre zählt aber ab der Aufnahme des Tickets. Die Karte und `signers activate` (ghostctl:2719) nennen deshalb einen etwas zu frühen Zeitpunkt.

---

## 6. Fehlende Tests

**Ergebnisse:**

- ghostctl: 18 Tests bestanden. Keiner davon berührt den neuen Code.
- `grep` im Testmodul (ab ghostctl:3942) findet weder `freeze`, `signers_json`, `DeployProgress`, „stammt von Version", `readyInHours` noch `check_update`.
- app: 442 Tests bestanden. Neu ist nur `precheck.test.ts` („v4: eingefrorenes Orakel sperrt …"), und der deckt `frozenBlocks` gut ab.

**Es fehlen Tests (mit dem Befund, den sie gefunden hätten):**

1. `signers_json`: `readyInHours` nach Ablauf negativ bzw. so, dass die Probe den Wert auswerten kann → **H1**.
2. `feed_due`/Agent: Bei `age < max_age` und `age ≥ freeze_after` muss ein Update fällig sein; bei `frozen` immer → **H2**.
3. `rate_plan` → `check_update` für Sätze außerhalb des Rasters (3,3 / 3,2 / 1,75 %) und für `deploy --rate 25` → **H3**.
4. `resync` mit fremder regulärer Ankündigung (nonce+1, `emerg = false`, keine eigene `rotation`) muss eine Meldung erzeugen → **M5**.
5. `Ctx::load`: v2-, v3- und v1-Datei liefern den richtigen Text, eine v4-Datei wird angenommen → **N7**.
6. `DeployProgress`: Abbruch nach jedem der fünf Schritte (Simulator wie `tests/v4_ops_tests.rs`), Fortsetzung mit geändertem `--probe`/`--threshold` → **N4**.
7. `oracle_update_price`: Unterzeichner-Datei mit zu wenigen Schlüsseln des aktuellen Satzes (ghostctl:2621–2623).
8. `freeze_if_stale`: Frist nicht erreicht → kein Bau; schon eingefroren → nichts; Fehler beim Bau → `None` ohne Senden.
9. `status_json`: Felder `frozen`, `freezeInMinutes` und `signers` sind vorhanden (Vertrag zur Seite, status.ts).
10. Probe-Skript, wie `a12_cx1_startskript_nennt_alle_aufgaben` für das Startskript:
    - jeder Netzaufruf von `$BIN` trägt `--state`;
    - `STAGE_FILE` wird im Probelauf nie geschrieben;
    - leere `status_field`-Werte führen zu `fail`.
    - **Befunde:** M1, M3.
11. Seite: OracleCard und Swap bei `frozen` und bei `rotation` (Darstellung; heute ist nur die Logik in `precheck` getestet).

### Hinweise

- Die Probe übt `cancel`, `clear`, den Notfallweg, `redeem`, `liquidate`, `sweep` und den Pool v4 nicht im Mainnet. Abschnitt 9 Punkt 8 des Entwurfs nennt „alle Einträge einmal echt ausführen".
- Die Seitentexte nennen feste Fristen (2 h / 14 Tage / 30 Tage). Die Probe nutzt 1 h. Die Orakel-Karte liest die Fristen richtig aus dem Status, die FAQ nicht. Das ist unkritisch, solange die Seite nie auf die Probe-Datei zeigt.
- `ghostctl send` von v3 und v4 klärt kein offenes Journal (kein `resolve_pending` im Sendeweg, ghostctl:1528–1536). Das gab es schon vor v4 und ist keine Folge dieser Commits.
