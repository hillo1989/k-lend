# Audit 12, Stand Gruppe C: Umzugsskript (A12-4, A12-17)

Bearbeiter: Claude Opus 5.5, 29.09.2026.
Branch `fix12c`, Basis `8b6ee75`. Grundlage: Bericht zu Audit 12, Abschnitte 2, 3.5 und 4 (A11-O-4, A11-O-5).

Geändert:
- `GHOST-Umzug-v3.command`
- `GHOST-Agent starten.command`, nur Kommentar und eine Hinweiszeile (A12-17, Kleinigkeiten). Die Datei liegt außerhalb der Liste dieser Gruppe. Der Text steht nur dort, der Bericht nennt die Stelle ausdrücklich (`GHOST-Agent starten.command:23`).

Nicht geändert: Verträge, Rust, Seite. Eine Vertragsänderung ist für keinen der beiden Befunde nötig, beide liegen allein im Skript.

Es wurde nichts gesendet. `keys/` und `deployments/` wurden nicht gelesen. Alle Läufe arbeiten mit Attrappen von `ghostctl`, `bin/ghostctl-v2` und `pgrep` in einem eigenen Ordner.

## Übersicht

| ID | Stand | Kurz |
|---|---|---|
| A12-4 | **behoben** | Eigener Schlüssel genau über die Besitzer-Datei (Ordner gelistet, voller Pfad bzw. dieselbe Datei), genau ein Treffer oder Abbruch. Schritt 5 eröffnet nur, wenn es sicher keinen eigenen v3-Vault gibt, gesperrte zählen mit. Merker gegen einen zweiten Vault. |
| A12-17 (a) Journal-Ziel | **behoben** | Vor jedem Aufruf von Version 2 wird `mainnet-v2.pending.json` geprüft. Zeigt es noch auf `mainnet.json`, wird es umgerichtet, im Probelauf wird abgebrochen. Jedes andere Ziel führt zum Abbruch. |
| A12-17 (b) Probelauf | **behoben** | Die Vorschau zieht die für vorige Vaults verplanten GHOST ab und rechnet Restschuld und Sicherheit wie der echte Lauf. |
| A12-17 (c) Text „Keeper zahlt 0,045 KAS“ | **behoben** (Agent-Skript) | Die Netzgebühr trägt der Vault über 0,1 KAS, den Rest bekommt der Keeper. Auflösen läuft auch ohne Komitee-Datei. |
| A12-17 (c) Keeper ohne KAS | **dokumentiert** | Kommentar und Startmeldung des Agenten sagen jetzt, dass der Keeper eine eigene KAS-UTXO braucht. Das Verhalten bleibt (Rust, Gruppe Agent). |
| A12-17 (c) manuelles `--rate` | **behoben** (Restpunkte, Branch `fix13b`) | Betrifft `ghostctl oracle-update --rate` bzw. `deploy --rate` (Rust), nicht das Umzugsskript. Beide setzen jetzt den Takt der Zinsregel, siehe „Restpunkte“. |

## A12-4: Schlüsselwahl und Schritt 5

**Ursache (nachvollzogen):**
- `MYKEY` suchte in `keys --json` (ohne `--dir`, also immer `./keys`) den ersten Eintrag, dessen `file` auf den Basisnamen der Besitzer-Datei **endet**.
- `keys/alt-mainnet-owner.json` sortiert vor `keys/mainnet-owner.json` und passte deshalb.
- Mit `KEYS` in einem anderen Ordner wurde die Besitzer-Datei gar nicht gelistet. Eine gleichnamige Datei in `keys/` gewann.
- Mit dem falschen x-only-Schlüssel fand Schritt 5 keinen eigenen Vault und eröffnete einen. Der neue Vault gehörte dem echten Schlüssel, wurde also wieder nicht gefunden, und der Lauf brach ab. Der nächste Doppelklick eröffnete den nächsten 50-KAS-Vault.
- Teil A ließ mit dem falschen Schlüssel die eigenen v2-Vaults still liegen.

**Änderung in `GHOST-Umzug-v3.command`:**
- **`MYKEY`:**
  - Aufruf `keys --dir "$(dirname "$OWNER")"`.
  - Ein Eintrag zählt nur, wenn er vom Typ `key` ist und `realpath(file) == realpath($OWNER)` gilt oder `os.path.samefile` greift. Das ist ein exakter Vergleich, nie per Endung.
  - Verlangt sind genau ein Treffer und ein x-only aus 64 Hex-Zeichen. Sonst gibt es keine Ausgabe und Rückgabe 1.
  - Jeder Aufrufer bricht dann ab: `xonly`, `lpShares`, `ghost` und `kas` in Teil A und B. Vorher wurde ein leeres Ergebnis teils als 0 gelesen.
- **Gegenprobe:** Liegt aus Teil A der v2-Schlüssel vor, muss der v3-Schlüssel derselbe sein, sonst Abbruch.
- **`own_vaults`:**
  - Ohne lesbare Vault-Liste gibt es Rückgabe 1 und damit einen Abbruch. Vorher gab es einen Python-Fehler und ein leeres Ergebnis.
  - Gezählt werden alle Vaults mit `owner == ME3`, auch gesperrte (`stale`). Geprägt wird nur an einem nicht gesperrten ohne Schuld.
  - Vorher fielen gesperrte eigene Vaults aus der Zählung. Ein gesperrter eigener Vault führte so zu einem zweiten Vault.
- **Merker `deployments/.umzug-v3-vault.lock`** (von `.gitignore` erfasst):
  - Er wird nach einem angenommenen `open-vault` geschrieben und entfernt, sobald der Vault als eigener gefunden ist.
  - Liegt er und gibt es keinen eigenen Vault, bricht jeder weitere Lauf vor Schritt 5 mit einem Hinweis ab.
  - So führt auch ein unvorhergesehener Fehltreffer höchstens zu einem Vault.
  - Scheitert `open-vault` selbst, entsteht kein Merker: Eine gesendete, aber unbestätigte Tx klärt das Journal von ghostctl beim nächsten Status.

**Tests** (`tests/umzug/run.zsh`, bis zu den Restpunkten `…/scratchpad/umzug12c/run.zsh`):

| Fall | Prüft | Ohne Behebung (8b6ee75) |
|---|---|---|
| N1 | `keys/alt-mainnet-owner.json` vor `keys/mainnet-owner.json`, zwei Doppelklicks: genau ein `open-vault`, v2-Vault getilgt und geschlossen, Prägen am eigenen Vault | rot: zwei `open-vault`, v2-Vault nie getilgt |
| N2 | `KEYS=schluessel/mainnet`, fremde gleichnamige Datei in `keys/` | rot: v2 übersprungen, `open-vault`, dann „Neuer Vault nicht gefunden“ |
| N3 | Besitzer-Datei nicht als Schlüssel lesbar, `keys/x-mainnet-owner.json` vorhanden: Abbruch vor jedem Senden | rot: `open-vault` gesendet |
| N4 | Besitzer-Datei doppelt gelistet: Abbruch | rot |
| N5 | eigener v3-Vault gesperrt: kein zweiter Vault | rot: `open-vault` + `mint` |
| N6 | neuer Vault erscheint mit fremdem Besitzer: Merker, zweiter Lauf eröffnet nichts | rot: zweiter `open-vault` |
| N7 | Merker liegt, eigener Vault da: Merker weg, Prägen | rot (Merker unbekannt) |
| N8 | Status ohne Vault-Liste: nichts eröffnen | grün (Regression) |
| P-sg | Szenario der Prüfer (`STUB_ALTKEY=1`, deren `stub.py`), zwei Läufe | rot: zwei `open-vault` |
| R4 | `keys --dir keys` in v2 und v3 | rot |

## A12-17 (a): Journal nach unterbrochenem Umbenennen

**Ursache (nachvollzogen):**
- Schritt 0 benennt der Reihe nach um: Journal, Deploy-Fortschritt, `mainnet.json`, danach wird das Journal-Ziel umgeschrieben, zuletzt die Sperre.
- Bricht der Lauf nach dem Umbenennen des Journals ab, zeigt `mainnet-v2.pending.json` weiter auf `deployments/mainnet.json`. Das passiert bei geschlossenem Fenster oder Absturz.
- Beim nächsten Lauf fehlt entweder `mainnet.json`, dann wird Schritt 0 übersprungen. Oder es gibt kein `mainnet.pending.json` mehr, dann wird kein Ziel umgeschrieben.
- Der erste Aufruf `bin/ghostctl-v2 --state deployments/mainnet-v2.json status` übernimmt das Journal (`store::resolve_pending` in `load_synced`, auch bei `--dry-run`). Er schreibt den v2-Stand nach `mainnet.json`, also genau dorthin, wo Version 3 hinkommt. Der Umzug hängt dann mit „ist nicht Version 3“.

**Änderung:**
- Nach Schritt 0 und vor jedem Aufruf von Version 2 wird `mainnet-v2.pending.json` geprüft. Die Prüfung läuft unter `flock` auf `mainnet-v2.lock`, wie ghostctl es macht.
- Ziel leer oder `mainnet-v2.json`: nichts zu tun.
- Ziel `mainnet.json`: Das Ziel wird atomar auf `mainnet-v2.json` umgeschrieben, relativ oder absolut wie vorher, mit Meldung.
- Jedes andere Ziel oder ein unlesbares Journal: Abbruch, nichts verändert.
- Probelauf: Er ändert nichts. Zeigt das Journal auf `mainnet.json`, bricht er ab, bevor Version 2 aufgerufen wird. Vorher schrieb schon der Probelauf den v2-Stand nach `mainnet.json` (Fall J4).

**Tests:**

| Fall | Prüft | Ohne Behebung |
|---|---|---|
| J1 | `mainnet.json` und Journal umbenannt, Ziel noch alt: Journal landet in `mainnet-v2.json`, `mainnet.json` ist am Ende Version 3 | rot: `mainnet.json` bekommt den v2-Stand, Abbruch „ist nicht Version 3“ |
| J2 | nur das Journal umbenannt, `mainnet.json` (v2) noch da | rot, wie J1 |
| J3 | wie J1 mit absolutem Ziel | rot |
| J4 | Probelauf mit Journal aus J1: Abbruch, kein v2-Aufruf, nichts geschrieben | rot: der Probelauf schreibt `mainnet.json` |
| J5 | Ziel `deployments/anders.json`: Abbruch vor jedem v2-Aufruf | rot: `anders.json` geschrieben |
| J6 | Ziel schon richtig: normaler Lauf | grün (Regression) |
| P-sc | Szenario `sc` der Prüfer: Journal zeigt danach auf `mainnet-v2.json` | rot |

Die Attrappe bildet `resolve_pending` im Fall „angenommen“ nach (Ziel bekommt `next`, Journal wird gelöscht), in J1 bis J6 eingeschaltet.

## A12-17 (b): Probelauf zieht die verplanten GHOST ab

**Ursache:**
- Im Probelauf wird nicht getilgt. `HAVE` wird für jeden Vault neu aus der Zustandsdatei gelesen und war deshalb immer das volle Guthaben.
- Die Vorschau zeigte jeden Vault als getilgt. Restschuld und Rest-Sicherheit des letzten Vaults stimmten nicht.
- Beispiel der Prüfer (`dry1`): 1,0 GHOST, Schulden 0,1 / 0,3 / 2,0. Die Vorschau tilgte am letzten Vault 1,0 statt 0,6 und senkte die Sicherheit auf 44,00 statt 61,60 KAS.

**Änderung:**
- Im Probelauf zählt `USED` die in der Vorschau verplanten GHOST. `HAVE` ist dort Guthaben − `USED`, mindestens 0.
- Die Zeile nennt dann „(nach den vorigen Tilgungen)“.
- Der echte Lauf ist unverändert, er liest nach jedem Tilgen den wirklichen Stand.

**Tests:**

| Fall | Prüft | Ohne Behebung |
|---|---|---|
| D1 | Probelauf: 0,1 und 0,3 ganz, dann `--ghost 0.60000000`, Restschuld 1,4, Sicherheit 61,60 KAS | rot: `--ghost 1.00000000`, 44,00 KAS |
| D2 | echter Lauf mit denselben Zahlen tilgt genauso, `withdraw --keep 61.60` | grün (Vorschau = echter Lauf) |
| P-dry1 | Szenario `dry1` der Prüfer mit deren `stub.py` | rot |

## A12-17 (c): Kleinigkeiten

- **Text „Keeper zahlt 0,045 KAS“:**
  - Falsch waren zwei Aussagen. Der Keeper zahlt nichts drauf: `ops::sweep` gibt der Kasse Sicherheit − `SWEEP_FEE`, das Wechselgeld geht an `fund.change()`, also an den Keeper. Und das Auflösen läuft nicht nur mit Komitee-Datei: `keeper_round` wird in der Agentenschleife immer aufgerufen (`ghostctl.rs:1131`).
  - Der Kommentar in `GHOST-Agent starten.command` ist korrigiert, gleichlautend mit ARCHITEKTUR.md und MAINNET.md: 0,1 KAS tragen etwa 0,055 KAS Netzgebühr, etwa 0,045 KAS bekommt der Keeper.
- **Keeper ohne KAS kann nicht auflösen:**
  - `ops::sweep` nimmt `fund.inputs()`, und `Net::funds` bricht ohne eigene UTXO mit „keine KAS auf …“ ab (`net.rs:195–197`).
  - Nebenbei gibt der eigene Eingang der Tx einen Wechselgeld-Ausgang des Keepers, an dem das Journal eine verdrängte Tx erkennt (`store::resolve_pending`, Fall 4). Ob das Auflösen auch ohne eigene KAS gehen soll, ist eine Frage an `ops::sweep`/`keeper_sweep` (Rust, Gruppe Agent/A12-2).
  - Kommentar und Startmeldung des Agenten sagen es jetzt. Das Verhalten ist unverändert.
- **Manuelles `--rate` setzt den Takt nicht:**
  - `oracle-update --rate` und `deploy --rate` schreiben `last_change` in `<netz>-zins.json` nicht. Die Zinsregel des Agenten darf also sofort wieder ändern, sobald sie 6 Messungen und einen Pool ab 10 GHOST hat.
  - **Offen:** Das ist eine Änderung in `ghostctl.rs` bzw. `rate.rs`, bei der Gruppe Zinsregel (A12-3). Vorschlag: Nach einem gesendeten Satz mit `--rate` unter `rate::update` `last_change = now` setzen.
  - Für den Umzug ohne Wirkung: Er legt den Pool mit `POOL_GHOST` = 0,25 GHOST an, unter der Mindestliquidität der Zinsregel (10 GHOST). Das Umzugsskript schreibt die Zinsdatei bewusst nicht selbst.

## Testzahlen

Gelaufen mit `…/scratchpad/umzug12c/run.zsh` (Attrappen `mock_ghostctl.py`, `mockbin/pgrep`; die Prüfer-Szenarien laufen mit deren `stub.py` aus `umzug12/`). Die Tests liegen jetzt im Projekt unter `tests/umzug/`, Zahlen dort siehe „Restpunkte“:

- Stand fix12c: **89 bestanden, 0 fehlgeschlagen** (`ergebnis-neu.txt`).
- Stand 8b6ee75 (`SCRIPT=alt/GHOST-Umzug-v3.command ./run.zsh`): **53 bestanden, 36 fehlgeschlagen** (`ergebnis-alt-8b6ee75.txt`).
  - Rot sind genau die neuen Prüfungen zu A12-4 und A12-17.
  - Die Regressionsfälle R1 bis R10 aus `umzug-test/run.zsh` (Audit 11: Agent, Sperre, `DRY`, Journal, Kollision, unlesbare Datei, Wiederaufnahme, Restschuld, Doppelstart) sind in beiden Ständen grün. Ausnahme ist die neue Prüfung `keys --dir` in R4.
  - Die Szenarien `sa`, `sb`, `sd`, `se`, `sf` und `real1` der Prüfer sind ebenfalls in beiden Ständen grün. `real1` hat mit Behebung dieselbe Sendefolge wie im Bericht.

`zsh -n` für beide `.command`-Dateien: ohne Fehler. Rust und Seite sind nicht betroffen und wurden nicht gebaut.

Die Attrappen lagen zunächst außerhalb der Arbeitskopie, wie bei Audit 12 (`umzug12/`). Seit den Restpunkten liegen `run.zsh`, `mock_ghostctl.py`, `mockbin/pgrep` und `pruefer/stub.py` im Projekt unter `tests/umzug/` (Restpunkt C-T1).

## Restpunkte (Runde 3)

Stand 30.09.2026, Branch `fix13b` auf Basis `a70fbbd`. Grundlage sind die Restpunkte der Prüfer (Schlüssel „c“). Nichts gesendet, `keys/` und `deployments/` nicht gelesen.

| Punkt | Stand | Was geändert ist | Test |
|---|---|---|---|
| C-T1 (Testablage) | **behoben** | Die Szenario-Tests liegen jetzt in `tests/umzug/`: `run.zsh`, `mock_ghostctl.py`, `mockbin/pgrep`, `pruefer/stub.py`, `pruefer/real1-calls.log`. Pfade relativ zum Ordner des Skripts; geprüft wird `../../GHOST-Umzug-v3.command`, die Fälle laufen in einem Temp-Ordner (nach Erfolg entfernt, bei Fehlern stehen gelassen). Die Vorlage der Prüfer (`pruefer/tpl`) wird im Lauf erzeugt, nicht kopiert, siehe unten. | `tests/umzug/run.zsh` |
| C-T2 (ungetestete Schutzprüfungen) | **behoben** | Keine Änderung am Schutz; zwei neue Fälle. | C1 (Gegenprobe ME = ME3), C2/C2b (x-only aus 64 Hex-Zeichen) |
| C-R1 (Symlink/Hardlink) | **behoben** | `MYKEY` fasst Treffer zusammen, die dieselbe Datei wie die Besitzer-Datei sind (`realpath` bzw. `samefile`) und denselben Schlüssel nennen. Jeder Abbruch nennt jetzt den Grund auf einer Zeile „Grund: …“: nicht gelistet, mehrfach mit verschiedenen Schlüsseln, kein x-only aus 64 Hex-Zeichen. | C3 (Symlink), C4 (Hardlink), C5 (Kopie), N4b (doppelt gelistet, derselbe Schlüssel); N4 jetzt mit verschiedenem Schlüssel und Grund |
| C-O1 (manuelles `--rate`) | **behoben** (Rust) | `oracle-update --rate` setzt den Takt der Zinsregel in derselben Datei wie der Agent (`<netz>-zins.json`, `RateLog::set_by_hand` über `rate::update`): vorgemerkt vor dem Senden, zurückgenommen, wenn sicher nichts gesendet wurde. Die Hand geht vor, auch innerhalb der Stunde nach einer Änderung der Zinsregel. `deploy` beginnt die Zinsregel neu (`RateLog::restart`): Messungen eines früheren Pools weg, Takt ab dem Startzins. Das betrifft auch den Umzug, denn die Zinsdatei heißt nach dem Umbenennen gleich. Lässt sich die Datei nicht schreiben, wird trotzdem gesendet, mit Hinweis. | `rate::tests::a12_co1_zins_von_hand_setzt_den_takt`, `a12_co1_deployment_beginnt_die_zinsregel_neu`; `ghostctl` (bin): `a12_co1_zins_von_hand_setzt_den_takt_der_zinsregel`, `a12_co1_deploy_beginnt_die_zinsregel_neu_und_verdrahtung` |
| C-X1 (Startmeldung Agent) | **behoben** | `GHOST-Agent starten.command`: ohne Komitee-Datei „Liquidationen, Auflösen, Daueraufträge und Tresore – ohne Komitee-Datei kein Orakel und keine Zinsregel“, mit Komitee-Datei alle sechs Aufgaben. Kopf um die Tresore ergänzt. Die Startmeldung von `ghostctl agent` nennt ebenfalls Auflösen, Daueraufträge und Tresore. | `ghostctl` (bin): `a12_cx1_startskript_nennt_alle_aufgaben` |

**Vorlage der Prüfer.** Die Kopie `pruefer/tpl` im Scratchpad enthält einen Ordner `keys/`, den ich nicht lese. Außerdem ignoriert `.gitignore` jeden Ordner `keys/`. Deshalb erzeugt `psetup` die Vorlage im Lauf:
- `ghostctl` und `bin/ghostctl-v2` verweisen auf `pruefer/stub.py`.
- Die Schlüsseldateien tragen nur einen Attrappen-Text. `stub.py` liest sie nicht.
- Den Zustand von Version 2 (Modell `_m`) habe ich aus den Ausgaben der früheren Läufe nachgebaut, also aus der Anzeige des Probelaufs und `real1-calls.log`. Eigene Vaults 0 / 2 / 3 mit 0,3 / 0,1 / 2,0 GHOST Schuld und 10 / 5 / 60 KAS, Vault 1 fremd, 1,0 GHOST, Orakel 0,05 USD. Dazu ein offenes Journal auf `deployments/mainnet.json`.
- Bestätigt ist der Nachbau durch die Szenarien selbst: `real1` hat genau die Sendefolge aus `real1-calls.log`, `dry1` die Texte der Prüfer, und alle P-Fälle sind grün wie im Scratchpad (89/0 vor den neuen Fällen).

**Neue Fälle (C):**

| Fall | Prüft | Rückbau |
|---|---|---|
| C1 | Version 3 nennt für die Besitzer-Datei ein anderes x-only als Version 2: Abbruch mit beiden Schlüsseln, kein `open-vault`, kein `mint` | Gegenprobe entfernt: rot (2 Prüfungen) |
| C2 | x-only `abc` in beiden Versionen: Abbruch mit Grund, kein Sendebefehl | Hex-Prüfung aus: rot |
| C2b | nur Version 3, x-only `A`: Abbruch vor `open-vault` | Hex-Prüfung aus: rot |
| C3 | `keys/zz-kopie.json` → `mainnet-owner.json` (Symlink): normaler Lauf, genau ein `open-vault` | alte Regel „genau ein Eintrag“: rot |
| C4 | Hardlink `keys/zz-hart.json`: ebenso | rot |
| C5 | Kopie mit gleichem Inhalt: ist nicht die Besitzer-Datei, normaler Lauf | – (Regression, grün in allen Ständen) |
| N4 | doppelt gelistet, die Kopie mit anderem x-only: Abbruch mit Grund | Stand a70fbbd: Grund fehlt (rot) |
| N4b | doppelt gelistet mit demselben Schlüssel: zählt einmal | alte Regel: rot |

N4 hieß vorher „Besitzer-Datei doppelt gelistet → Abbruch“. Derselbe Eintrag zweimal ist jetzt dieselbe Datei und zählt einmal (wie Symlink/Hardlink). Abgebrochen wird, sobald dieselbe Datei verschiedene Schlüssel nennt.

**Zahlen** (`tests/umzug/run.zsh`, Protokolle unter `…/scratchpad/fix13b-rueckbau/`):

| Stand | bestanden | fehlgeschlagen |
|---|---|---|
| `fix13b` | **102** | **0** |
| Basis `a70fbbd` (fix12c) | 95 | 7 (N4-Grund, N4b, C2-Grund, C2b-Grund, C3 ×2, C4) |
| 8b6ee75 (vor Audit 12) | 60 | 42 (die 36 aus fix12c und sechs der neuen) |
| Rückbau Gegenprobe ME = ME3 | 100 | 2 (C1) |
| Rückbau Hex-Prüfung | 99 | 3 (C2, C2b) |
| Rückbau Zusammenfassen | 98 | 4 (N4b, C3 ×2, C4) |

Aufruf eines anderen Stands: `git show 8b6ee75:GHOST-Umzug-v3.command > /tmp/alt.command; SCRIPT=/tmp/alt.command tests/umzug/run.zsh`. Das Skript macht die Kopie selbst ausführbar. `zsh -n` für `GHOST-Umzug-v3.command`, `GHOST-Agent starten.command` und `tests/umzug/run.zsh`: ohne Fehler.

**Gegenproben Rust zu C-O1 und C-X1** (Protokoll `…/scratchpad/fix13b-rueckbau/rust.txt`):
- `set_by_hand` ohne Wirkung: `rate::tests::a12_co1_zins_von_hand_setzt_den_takt` rot.
- `oracle-update` ohne `rate_by_hand`: `a12_co1_deploy_…_und_verdrahtung` rot.
- Vormerkung wird nie zurückgenommen: `a12_co1_zins_von_hand_setzt_den_takt_der_zinsregel` rot.
- `deploy` ohne `rate_restart`: `a12_co1_deploy_…` rot.
- Alte Startmeldung: `a12_cx1_startskript_nennt_alle_aufgaben` rot.

Testzahlen aller Teile (Rust 357, Seite 409, Umzug 102) stehen in `12-stand-b.md`, Abschnitt „Restpunkte“.

## Umzug: einmal bestätigen (Runde 4)

Stand 30.09.2026, Branch `fix15b` auf Basis `d21e9a8`. Wunsch des Nutzers: einmal bestätigen statt vor jeder der etwa zwölf Transaktionen `j` zu tippen. Nichts gesendet, `keys/` und `deployments/` nicht gelesen, Verträge, Rust und Seite unverändert.

**Modus (Standard):**
- **Plan vor dem ersten Senden.** Teil A läuft mit derselben Logik wie der Probelauf. Er verrechnet die schon verplanten GHOST und zusätzlich den Rückfluss aus dem Pool (eigener Anteil an `kasSompi`/`ghostUnits`). Gebaut wird mit `bin/ghostctl-v2 --dry-run --json`, die Gebühr kommt aus `transactions[].feeKas`. Schließen, Herausnehmen und Tilgungen, die erst die GHOST aus dem Pool decken, lassen sich erst nach dem echten Vorschritt bauen. Sie stehen mit geschätzter Gebühr im Plan (MAINNET.md, „Was es kostet“). Teil B wird aus dem Stand gerechnet: Deployment, Vault oder nur Prägen, Pool zum Kurs (`ghostctl price`, sonst Orakel von Version 2, bei schon angelegter Version 3 deren Orakel).
- **Zusammenfassung:** je Schritt Aktion und Beträge, „zurück“, „gebunden“ (30 KAS Deployment, 3 KAS Minter-Zweig, Pool-Mindestliquidität, Rest-Vault der Version 2) und Gebühr, dazu Gebühren gesamt und Endstand. Der Endstand nennt KAS jetzt, vor Teil B und am Ende, GHOST der Version 3 und Version 2 sowie Warnungen, etwa „fehlen voraussichtlich GHOST/KAS“.
- **Eine Frage** `Alles so ausführen? [j/N]`. Jede andere Antwort, auch keine Eingabe, bricht ab. Bis zur Antwort wird nichts gesendet und **nichts umbenannt**. Schritt 0 prüft vorher nur, ob das Umbenennen geht (`rename_v2 1`: Sperre, Ziele, Journal-Ziel), und benennt erst nach `j` um. Das Umrichten eines Journals nach unterbrochenem Umbenennen (A12-17) läuft wie bisher vor dem ersten Aufruf von Version 2 auf `mainnet-v2.json`, bei Version 2 unter altem Namen erst nach dem Umbenennen.
- Nach `j` gehen alle Sendebefehle mit `--ja` (Flag von ghostctl, `confirm` in `ghostctl.rs`).
- **Abbruch:** Jeder Abbruch nach der Bestätigung zeigt „In diesem Lauf schon gesendet“ und „Noch offen (laut Plan)“, ab dem abgebrochenen Schritt. Er weist darauf hin, dass ghostctl eine vielleicht doch angenommene Transaktion beim nächsten Aufruf über das Journal klärt. Die Wiederaufnahme ist die vorhandene: Der nächste Lauf rechnet den Plan aus dem Stand und zeigt deshalb nur die restlichen Schritte, mit einer Frage.
- **Vor Teil B neu rechnen:** Nach Teil A wird Teil B aus dem echten KAS- und GHOST-Stand neu gerechnet. Angehalten und erneut gefragt (`Teil B so ausführen? [j/N]`) wird bei anderen Schritten oder festen Beträgen, bei mehr als 0,5 KAS weniger als geplant (Spielraum für Gebühren) und bei einem Pool-Betrag, der um mehr als 1 % abweicht. Mehr KAS als geplant ist keine Abweichung. Vor dem Pool wird noch einmal verglichen: Weicht der KAS-Betrag zum Orakelkurs von Version 3 um mehr als 1 % vom bestätigten ab oder war der Kurs im Plan unbekannt, wird vor `pool-open` gefragt.
- **`EINZELN=1`:** der bisherige Modus. Ohne `--ja` fragt ghostctl je Transaktion. Die Zusammenfassung erscheint, eine Sammelfrage und die Zusatzfragen nicht. Werte außer 0/1 werden abgelehnt (wie `DRY`).
- **`DRY=1`:** unverändert ein reiner Probelauf, ohne Plan und ohne Frage.
- Früher als bisher bricht der Plan ab, bevor etwas gesendet ist: Komitee-Datei fehlt (vorher erst nach Teil A), Merker `.umzug-v3-vault.lock` ohne eigenen Vault und bei schon angelegter Version 3 die Gegenprobe ME = ME3.

**Attrappe** (`tests/umzug/mock_ghostctl.py`), jetzt näher an ghostctl:
- Ohne `--ja` und ohne `--dry-run` fragt jeder Sendebefehl wie `confirm` und liest eine Antwort. Jede Rückfrage steht als `# Rückfrage` in `calls.log`.
- `--dry-run --json` nennt eine Gebühr, `price` einen Kurs.
- Rückflüsse aus Version 2 und Ausgaben in Version 3 gehen auf dasselbe KAS-Guthaben. Mit `kas_back` < 1 kommt weniger zurück.

**Anpassungen an bestehenden Fällen** (keine Prüfung gelockert):
- Echte Läufe bekommen `j` auf stdin (`$JA`) statt `/dev/null`.
- `real1` vergleicht die Sendefolge ohne Proben und ohne `--ja`.
- R9 prüft „kein deploy“ jetzt als ` deploy ` statt `.*deploy`. Das alte Muster traf auch `--state deployments/…` des neuen Guthaben-Abrufs vor Teil A.

**Neue Fälle (E):**

| Fall | Prüft |
|---|---|
| E1 | Plan mit allen sechs Schritten, zurück/gebunden/Gebühr (Probe und Schätzung), Endstand; Plan und Frage vor dem ersten Schritt; genau eine Frage; alle sechs Sendebefehle mit `--ja`, keine Rückfrage von ghostctl; Teil A auf `mainnet.json` geprobt |
| E2 | `n` und keine Eingabe: Abbruch, kein Sendebefehl, nichts umbenannt, keine `mainnet-v2.lock`, Sperre frei |
| E3 | `close` scheitert nach dem ersten Tilgen: gesendet/offen genannt; zweiter Lauf zeigt nur die sieben restlichen Schritte, eine Frage, tilgt nur Vault 2 |
| E4 | `kas_back` 0,5: vor Teil B „geplant etwa 259,91, jetzt 230,00“, Teil B neu gerechnet, zweite Frage; `n` → Teil A gesendet, von Teil B nichts; erneuter Doppelklick nur Teil B ohne Abweichung; `j`,`j` läuft durch |
| E5 | Börsenkurs 0,05, Orakel 0,04: Frage vor dem Pool (6,25 statt 5 KAS), `n` → kein `pool-open`; 0,5 % Abweichung → keine zweite Frage |
| E6 | `EINZELN=1`: keine Sammelfrage, sechs Rückfragen, kein `--ja`; `n` auf die erste Rückfrage bricht ab; `EINZELN=ja` abgelehnt |
| E7 | `DRY=1`: kein Plan, keine Frage, kein `--ja` |
| E8 | Pool-Anteile: Plan nennt 10 KAS und 0,25 GHOST zurück, tilgt damit ganz und schließt wie der echte Lauf, ohne zweite Frage |

**Zahlen** (`tests/umzug/run.zsh`, Protokolle unter `…/scratchpad/fix15b/`):

| Stand | bestanden | fehlgeschlagen |
|---|---|---|
| `fix15b` | **138** | **0** |
| Basis `d21e9a8` (mit den neuen Fällen) | 108 | 30, alle in E. R, P, N, J, D und C bleiben grün, die Anpassungen der Testumgebung hängen also nicht am neuen Skript. |
| Rückbau „vor Teil B neu fragen“ | 30 von 36 in E | 6 (E4) |
| Rückbau „vor dem Pool fragen“ | 34 von 36 in E | 2 (E5) |
| Rückbau `--ja` | 27 von 36 in E | 9 (E1, E4, E5: ghostctl fragt je Transaktion und verbraucht die Antworten) |

`zsh -n` für `GHOST-Umzug-v3.command` und `tests/umzug/run.zsh`: ohne Fehler. Rust und Seite sind nicht betroffen.

### Nachprüfung: Teil A mit dem Plan vergleichen

Ein Prüfer hat mit einem Pool, der sich zwischen Plan und Senden bewegt, Folgendes gezeigt: Nach dem einen `j` rechnete Teil A Tilgen, Schließen und Herausnehmen aus dem jeweils aktuellen Stand neu und sendete ohne Frage `repay --ghost 0.9` und `withdraw --keep 5.51` statt des angezeigten vollen Tilgens mit Schließen. Die Aussage oben, andere Beträge gingen „nie still“ hinaus, galt also nur für Teil B. Behoben:
- Jeder Planeintrag von Teil A trägt eine Kennung: Befehl, Covenant-ID und Betrag (`pool-remove <Anteile>`, `repay <CID> <PAY> <voll>`, `close <CID> <KAS>`, `withdraw <CID> <KAS> <KEEP>`). Der echte Lauf sendet in Teil A über `txa`. `txa` vergleicht die Kennung vor dem Senden mit dem nächsten Planeintrag. Weicht sie ab oder fehlt ein Eintrag, wird der Schritt nicht gesendet. `a_neu_planen` rechnet Teil A dann aus dem jetzigen Stand neu (in einer Subshell mit `--dry-run`, wie der Plan) und zeigt das Ergebnis. Es ersetzt die offenen Einträge von Teil A im Plan und rechnet „KAS vor Teil B“ neu. Danach wird gefragt (`Teil A so ausführen? [j/N]`). Nach dreimal neu Rechnen bricht der Lauf ab. Bei `EINZELN=1` wird die Abweichung gezeigt, und ghostctl fragt wie dort je Transaktion.
- Fallen geplante Schritte nach dem jetzigen Stand weg (Vault nicht mehr da), fragt der Lauf vor Teil B (`Ohne diese Schritte mit Teil B weitermachen?`).
- „Noch offen (laut Plan)“ nennt nach einem Abbruch die neu gerechneten Schritte. Die gesendeten Einträge bleiben im Plan stehen, deshalb trifft die Zählung über `#GESENDET − PL_BASIS` weiter.
- Leerer Plan („Nichts mehr zu senden“): Das Skript endet dort mit 0. Es gibt keinen Lauf von Teil A/B mit `--ja` ohne bestätigten Schritt.
- Attrappe: `"move": {…}` wird beim ersten echten Sendebefehl einer Version in die Welt übernommen (Pool, `kasUsd`, Vaults), bevor der Befehl wirkt.

| Fall | Prüft |
|---|---|
| E9 | Pool gibt 0,2 statt 0,5 GHOST: vor dem Tilgen „geplant 1,0 / jetzt 0,9“, Teil A neu (teilweise tilgen, herausnehmen), zweite Frage; `n` → nur `pool-remove` gesendet, „Noch offen“ nennt die neuen Schritte; `j` → gesendet wird, was die zweite Frage zeigte, Teil B ohne dritte Frage; `EINZELN=1` → Abweichung gezeigt, keine Sammelfrage, kein `--ja` |
| E10 | Pool gibt mehr GHOST: kein stilles volles Tilgen mit Schließen, erst die Frage; `n` → kein `repay`/`close`/`withdraw` |
| E11 | Orakel von Version 2 fällt beim Pool-Abzug von 0,04 auf 0,02: Tilgen wie geplant, Herausnehmen „27,50 → 55,00 KAS“ fragt; `n` → nicht herausgenommen |
| E12 | Vault verschwindet: entfallende Schritte genannt, Frage vor Teil B; `n` → nichts von Teil B |
| E13 | leerer Plan: Exitcode 0, keine Frage, kein Teil A/B, nichts gesendet |

Zahlen: `fix15b` **151 bestanden, 0 fehlgeschlagen**. Das Skript von `296a5f5` fällt mit denselben Tests in 12 von 151 Prüfungen durch (E9–E13 und N1 „2. Lauf“). Rückbau des Planvergleichs in `txa`: 8 rot (E9, E10, E11). Rückbau der Frage bei entfallenden Schritten: 1 rot (E12). Rückbau von EINZELN in `txa`: 1 rot (E9 EINZELN).

Offen, außerhalb dieser Gruppe: Der Kasten oben in `MAINNET.md` nennt als Ausnahmen von „jede Mainnet-Transaktion fragt“ nur Agent und Seite. Das Umzugsskript sendet nach der einen Bestätigung mit `--ja` und gehört dort ergänzt. Geändert ist hier nur der Abschnitt zum Umzug.
