# Audit-Übersicht GHOST (Stand 30.09.2026, Version 3 nach Audit 13)

Ein unabhängiges Audit mit 6 Fable-Prüfern hat am 28.09.2026 den Stand `v1-mainnet` (a738771) geprüft. Jeder Prüfer hatte einen eigenen Schwerpunkt, arbeitete ohne Schreibrecht am Projekt und musste jeden Befund belegen. Die Berichte liegen in `audit/`:

| Bericht | Schwerpunkt |
|---|---|
| `audit/1-vault-oekonomie.md` | Vault-Rechnung, Liquidation, Zins (mit ausführbaren Belegen) |
| `audit/2-covenant-integritaet.md` | GHOST-Menge über Vault, Factory und Token |
| `audit/3-orakel.md` | Orakelvertrag und Preisdienst |
| `audit/4-offchain-software.md` | Transaktionsbau, `ghostctl`, Netz, Zustandsdatei |
| `audit/5-testqualitaet.md` | Ob die Tests beweisen, was sie behaupten |
| `audit/7-fix-review.md` | Gegenprüfung der Behebungen von Version 2. Deren Befunde sind in Version 2.1 umgesetzt, siehe unten |
| `audit/8-fix-review-v2.1.md` | Gegenprüfung von Version 2.1: kein neuer Weg zu Geldverlust, 7 kleinere Befunde (NEU-1 bis NEU-7) |
| `audit/9-pool-audit.md` | Tauschpool und Wallet: Vertrag hält, 2 mittlere Befunde im Off-chain-Code (P-1, P-2). Damals noch der Pool des Besitzers |
| `audit/10-opus-pool.md` | Audit 10 (Opus, 29.09.2026): offener Tauschpool mit Anteils-Token |
| `audit/10-opus-agent-cap.md` | Audit 10 (Opus): GHOST-Agent (Orakel + Keeper), Obergrenze 50 GHOST je Vault, neue ghostctl-Pfade, Startskripte |
| `audit/10-opus-app.md` | Audit 10 (Opus): Web-Oberfläche und lokale API |
| `audit/11-opus-v3-vertrag.md` | Audit 11 Teil A (Opus, 29.09.2026): Vertrag Version 3 (Zins als eigener Posten, Zinskasse, Rücknahme), Mutationslücken |
| `audit/11-opus-v3-offchain.md` | Audit 11 Teil B (Opus, 29.09.2026): ghostctl, Zinsregel, Keeper, Umzugsskript und Seite zu Version 3 |
| `audit/12-opus-nachpruefung.md` | Audit 12 (Opus, 29./30.09.2026): Nachprüfung der Behebungen von Audit 11 sowie Tresor-Dauerauftrag, verschlüsselte Nachrichten, Daueraufträge; Stand je Befund in `audit/12-stand-a.md` (Tresor/Nachrichten), `-b.md` (Vault/Agent/Zinsregel), `-c.md` (Umzug) |
| `audit/13-stand-tresor.md`, `audit/13-stand-umzug.md` | Audit 13 (Opus, 30.09.2026): Abschlussprüfung der im Vertrag gebundenen Tresor-Nachricht und des Einmal-Bestätigen-Modus im Umzug; jeder Befund von einem zweiten Prüfer gegengeprüft |

Audit 1–9 liefen mit Fable, Audit 10 bis 13 mit Opus, weil das Fable-Guthaben aufgebraucht war.

**Wichtig:** Das ist ein KI-Audit und kein Audit durch eine Prüfgesellschaft. Es ersetzt keine professionelle Sicherheitsprüfung.

## Befunde und Stand

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| **V-01** | **kritisch** | Der GHOST-Token (unverändertes KCC20-Beispiel) prüft keine negativen Beträge. Aus 1 Einheit lassen sich beliebig viele GHOST erzeugen und damit Vaults liquidieren. | **Behoben (v2):** `contracts/ghost_token.sil` verlangt `amount >= 0`. Tests: `ghost_token_tests.rs`. Der Mainnet-Stand v1 wird zurückgebaut. |
| V-02 | hoch | Liquidation nur als Vollverbrennung, unter 100 % Deckung ein Verlustgeschäft | **Behoben:** Teil-Liquidation. Bei erschöpfter Sicherheit endet der Vault und die Restschuld wird ausgebucht. Die eigene Nachprüfung (Mutationstest v2) fand dabei einen neuen Fehler: Bei weniger als 0,2 KAS Sicherheit ließ sich mit 1 Einheit die ganze Schuld ausbuchen. **Ebenfalls behoben.** |
| O-2 | hoch | Orakelpreis ohne Grenzen (0 bis 1e18) | **Behoben:** 0,00001–900 USD, je Update höchstens ×2 bzw. ÷2 |
| O-1 | hoch | Jeder kann das Orakel für 0,003 KAS „lesen" und damit `ghostctl` aus dem Tritt bringen. Wiederholtes Lesen blockiert zudem andere Transaktionen. | **Teilweise:** `ghostctl` holt verschobene Orakel- und Vault-UTXOs jetzt selbst nach (`store.rs`). Die Blockade durch gezieltes Dauer-Lesen folgt aus dem Design und ist **offen**. |
| V-03 | mittel | Zinsen erhöhen die Schuld, werden aber nicht als GHOST geprägt → der letzte Schuldner kann nie voll tilgen | **Umgangen (v2):** Zins standardmäßig 0 %. **Behoben in Version 3:** Der Zins ist ein eigener Posten in USD und wird beim Schließen in KAS an die Zinskasse gezahlt. Die Schuld bleibt die geprägte Menge (`stable_vault.sil`, Tests `v3_*` in `vault_tests.rs`). Geprüft in Audit 11 (Opus): hält, siehe unten. |
| V-04 | mittel | `deposit` ohne Signatur: Fremde können Aktionen durch Konflikte stören | **Behoben:** nur mit Signatur des Besitzers |
| V-05 | niedrig | Mini-Tilgung ohne Anteilswirkung vernichtet GHOST | **Behoben** |
| F1 / O-3 | mittel | Transaktion gesendet, Zustand nicht gespeichert → Stillstand | **Behoben:** Journal mit Wiederaufnahme. Die erste Fassung war fehlerhaft (N-1), korrigiert in 2.1 |
| F2 | mittel | Deployment-Abbruch hinterlässt Covenants ohne Eintrag | **Behoben:** Fortschrittsdatei |
| F3 | mittel | Keine Dateisperre | **Behoben:** Die erste Fassung hatte eine Wettlaufsituation (N-2). Seit 2.1 sperrt das Betriebssystem die Datei (flock) |
| F4 | mittel | Änderungen Dritter werden nicht erkannt | **Behoben** für Lesen/Einzahlen (Nachladen). Von Dritten getilgte oder liquidierte Vaults werden als „veraltet" markiert und gesperrt. |
| O-4 | mittel | Ein signiertes Orakel-Update kann jeder einreichen, der es hat | **Offen**, ist Teil des Designs |
| O-5 | mittel | Keine Frischeprüfung des Preises im Vertrag | **Offen**, der Vertrag kennt nur Zeit-Untergrenzen. Nur der Dienst prüft das Alter. |
| O-6 | mittel | Der Preisdienst sendet unter `--ja` jeden Sprung | **Behoben:** Einen Sprung über 20 % sendet er erst, wenn er über 3 Runden besteht, und dann in Schritten von höchstens ×2/÷2 |
| F5–F13 | niedrig | JSON-Ausgabe, Eingabeprüfung, Blockgrenzen, verschenkte Reste, Dateirechte, Netzprüfung, Token-Zusammenführung, Startskripte | **Behoben** |
| Test-Audit | hoch | Einzelne Redundanz-Begründungen waren falsch (L145), ein Test war verloren gegangen (Hintertür mit gleicher Genesis-ID) | **Behoben:** Tests ergänzt. Jede neue Regel ist per Mutante belegt, siehe `protocol/mutation/mutate.sh`. |
| Doku-Audit | hoch | Die Seite versprach Dinge, die der Code nicht hält: „nur lesend", „5 unabhängige Stellen", garantierte Liquidation, Zinssteuerung | **Behoben:** Texte korrigiert, Risikobox auf der Vault-Seite |

## Fix-Review (Audit 7) und Version 2.1

Ein weiterer Fable-Prüfer hat die Behebungen von Version 2 gegengeprüft und dabei neue Fehler gefunden. Alle Vertragsänderungen sind per Mutante belegt: Wird die neue Regel durch `require(true)` ersetzt, schlägt mindestens ein Test fehl.

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| N-1 | mittel | Journal: „erster Input verbraucht" wurde als „angenommen" gewertet. Im Konfliktfall entstand ein falscher Zustand | **Behoben:** Die Entscheidung fällt über den eigenen Wechselgeld-Ausgang |
| N-2 | mittel | Dateisperre: Zwei Aufrufer konnten eine verwaiste Sperre gleichzeitig übernehmen | **Behoben:** flock, mit Test |
| N-4 | mittel | Jeder konnte mit 2 Einheiten „tilgen" und den Vault des Besitzers sperren | **Behoben (Vertrag):** `repay` nur mit Signatur des Besitzers |
| N-11 | mittel | Der KAS-Wert der Minter-UTXOs war ungeprüft. Fremde konnten je Vault etwa 2,8 KAS und aus der Wurzel etwa 9,8 KAS entnehmen | **Behoben (Vertrag):** Vault und Factory verlangen für die Minter-Fortsetzung denselben Wert wie am Eingang |
| N-12 | niedrig | Testlücke `stable_vault.sil:278` | **Behoben:** gezielter Test |
| N-5 | niedrig | `status --json` gab Hinweise auf stdout aus, und die Seite konnte das JSON nicht lesen | **Behoben:** Hinweise gehen nach stderr |
| N-6 | niedrig | Nach einem Sprung über 20 % stand der Feed still | **Behoben:** Nach 3 bestätigenden Runden nähert sich der Feed in Schritten (×2/÷2) an |
| N-7 | niedrig | RPC-Fehler galt als „nicht im Mempool" | **Behoben:** Bei einem Fehler bleibt das Journal erhalten |
| N-8 | niedrig | `deploy`-Fortsetzung verwarf geänderte Parameter ohne Hinweis | **Behoben:** Fortsetzung nur mit denselben Schlüsseln, sonst Hinweis |
| N-9 | Info | `resync` bei zwei identischen Token-UTXOs | **Offen:** Tritt nur auf, wenn Token außerhalb der Zustandsdatei bewegt werden |
| N-10 | niedrig | `deploy` löste das Journal zu spät auf | **Behoben:** Reihenfolge getauscht |
| I-1 | Info | `MAINNET.md` zeigte `--rate 5` | **Behoben:** `--rate 0` |
| I-2 | Info | Startpreis wurde beim Deploy nicht gegen die Grenzen geprüft | **Behoben** |
| I-3 | Info | ×2/÷2 je Update bremste nicht zeitlich: 0,04 → 900 USD in Sekunden | **Nur teilweise (Vertrag):** Jedes Update braucht eine um mindestens 600 DAA höhere Orakel-DAA als das vorige. Die Orakel-DAA wählt aber das Komitee, und sie darf hinter der Kette liegen. Lag das letzte Update Stunden zurück, sind in einem Block viele Updates hintereinander möglich (Fix-Review 8, NEU-1, belegt). Eine echte Zeitbremse ist das nur bei häufigen Updates. |

Die Seite und `ghostctl` weisen Aktionen, die nur der Besitzer ausführen darf (Prägen, Tilgen, Einzahlen, Abheben, Schließen), vorab mit einer verständlichen Meldung ab.

## Fix-Review 8 (Version 2.1)

Alle Vertragsbehebungen von 2.1 greifen. Die Prüfung fand keinen neuen Weg zu Geldverlust, aber diese Punkte:

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| NEU-1 | niedrig | Der Orakel-Abstand von 600 DAA ist keine Zeitbremse: Die Orakel-DAA wählt das Komitee, und sie darf hinter der Kette liegen | **Doku korrigiert.** Eine echte Bremse wäre ein relatives Zeitschloss, aber jedes `read()` setzt das Alter der Orakel-UTXO zurück. Damit könnte jeder Updates blockieren. **Offen, Designfrage** |
| NEU-2 | Info | Die Seite prüfte „unter 1 Minute“, ghostctl verlangt 620 DAA; bei fehlgeschlagener DAA-Abfrage kam eine falsche Warnung | **Behoben** |
| NEU-3 | Info | Die Doku zum Feed bei Sprüngen über 20 % war veraltet | **Behoben** |
| NEU-4 | niedrig | Eine verdrängte Tx ohne Wechselgeld hielt jeden Aufruf an, bis ein Mensch das Journal löschte | **Behoben:** Das Journal merkt sich alle Eingänge. Ist einer unverbraucht, war die Tx nicht angenommen |
| NEU-5 | niedrig | „Nicht im Mempool“ wurde am Fehlertext erkannt; ein anderer Wortlaut führte zum Stillstand | **Behoben:** Entscheidungen, die vom Mempool nicht abhängen, fallen trotzdem |
| NEU-6 | Info | Irreführende Meldung nach übernommenem Deploy-Schritt | **Behoben** |
| NEU-7 | Info | Sperr- und Temp-Dateien nicht in `.gitignore` | **Behoben** |

## Tauschpool KAS/GHOST (`contracts/ghost_pool.sil`, Audit 9)

**Vertrag:** Die Prüfung fand keinen Weg, den Pool leerzuräumen oder dem Besitzer Liquidität zu entziehen. Geprüft wurden geschenkte und gefälschte Reserven, Genesis-Tricks, zwei pool-eigene Token, Minter, Gebührenrundung, `mul()` an der Obergrenze, der KAS-Wert der Reserve, mehrere Pool-Ausgänge sowie `manage` und `close`. Die Tauschrechnung in `pool.rs`, in `poolMath.ts` und im Vertrag ist in 400 Zufallsfällen identisch. Mutationstest: 17 von 22 Regeln rot, die 5 übrigen deckt KCC20 ab (per Test belegt).

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| P-1 | mittel | Die Kandidatensuche fand den neuen Reserve-Betrag nicht, sobald eine Tx zwei GHOST-Zustände hat. Das betrifft jeden Kauf, denn die ABI legt die Beträge eines Struct-Arrays als einen Push aus 8-Byte-Zahlen ab. Folge: Nach dem ersten fremden Kauf wäre der Pool für ghostctl gesperrt gewesen | **Behoben:** Pushes werden zusätzlich im 8-Byte-Raster gelesen. Der Simulator-Test prüft nach jeder Pool-Tx, dass der Betrag gefunden wird. Gegenprobe: Ohne die Korrektur scheitert er schon bei der Anlage |
| P-2 | mittel | Der Rate-Pfad und die Bestätigung prüften die Covenant-ID nicht. Ein Köder-Ausgang ohne Covenant konnte alle ghostctl-Instanzen lahmlegen | **Behoben:** Die Reserve muss aus der Pool-Tx stammen und die GHOST-Covenant tragen. KAS-Wert und Outpoint kommen vom Node, und die Reserve wird bei jedem Abgleich neu bestätigt |
| P-3 | niedrig | Jeder kann den Pool für die Netzgebühr „anstupsen“ und damit gebaute Tx ungültig machen | **Abgemildert:** `swap` gleicht neu ab und versucht es bis zu 3-mal, nur wenn sicher nichts gesendet wurde. Das Anstupsen selbst gehört zum UTXO-Design |
| P-4 | niedrig | Scheiterte der Gesamtabgleich, zeigte die Seite alte Reserven als aktuell | **Behoben:** Der Pool ist dann als „unbestimmt“ markiert, und die Seite warnt |
| P-5 | Info | Die Seite verschwieg 1 KAS für den neuen Token und zeigte den erhaltenen Betrag nicht. Die Guthabenprüfung war ungenau, und `status` im Textmodus zeigte den Pool nicht | **Behoben** |
| P-6 | Info | `poolMath.ts` ohne Obergrenzen | **Behoben** |
| P-7 | Info | Adresstexte ungenau (nur `kaspa:q…` kann GHOST empfangen) | **Behoben** |
| P-8 | Info | Geschenkte pool-eigene Token bleiben beim Auflösen liegen | **Offen**, harmlos |
| P-9 | Info | Journal: Restfälle bei Mempool-Fehlertexten | **Offen**, siehe NEU-5 |
| P-10 | Info | Die REST-Abfrage blockierte einen Tokio-Worker | **Behoben** (`spawn_blocking`) |

Bekannte Grenzen (Stand Audit 9): Kleiner Pool heißt leicht verschiebbarer Kurs. Ohne REST-API kann ghostctl die Reserve nach fremden Tauschen nicht bestimmen, dann ist Tauschen gesperrt. Seit dem 28.09.2026 ist der Pool **offen** (Anteils-Token, jeder legt ein und zieht ab); geprüft in Audit 10.

## Pool Version 2: Kursband (29.09.2026, nach Audit 10)

Auf Wunsch des Nutzers hält der Pool GHOST bei 1 USD ± 3 % (`checkBand` in `contracts/ghost_pool.sil`): Er liest beim Tauschen das Orakel in derselben Tx (Covenant-ID und Vorlage geprüft wie im Vault) und lässt nur Tausche zu, nach denen der Preis im Band liegt oder sich darauf zubewegt. `init` verlangt einen Startkurs im Band. Einlegen und Abziehen bleiben frei. Pools der Version 1 laufen mit `contracts/ghost_pool_v1.sil` weiter, damit Einleger abziehen können.

Tests: Die 32 Pool-Tests und 14 Engine-Tests laufen auf dem neuen Vertrag (mit weitem Band), dazu 6 Band-Tests mit Gegenprobe am Mutanten (obere und untere Grenze exakt, außerhalb nur Richtung Band, fremdes Orakel, Startkurs, Rust-Regel gleich Vertrag in 24 Zufallsfällen) und 2 Simulator-Tests (Band mit voller Konsensprüfung, Angriff A10-P-1 gesperrt, Orakel danach für Vaults nutzbar; Pool v1 bleibt bedienbar). **Nicht unabhängig geprüft.** Das Band verlagert Vertrauen auf das Orakel: Wer das Orakel setzt, bestimmt, welche Tausche erlaubt sind.

## Audit 10 (Opus, 29.09.2026): offener Tauschpool

**Vertrag hält:** Die Prüfung fand keinen Weg, Reserven oder Anteile zu stehlen, Anteile aus dem Nichts zu erzeugen, einen zweiten Minter oder eine zweite Reserve einzuschleusen oder den Minter-Status zu kippen. Einlegen und Abziehen runden exakt zugunsten der übrigen Einleger, geprüft im Zufallstest gegen die Engine, auch bei S nahe 2^60. Die Grenzen halten auch alle zugleich am Maximum. Beim Spende-/Inflationsangriff verliert der Angreifer fast die ganze Spende, das Opfer höchstens einen Anteil. Die Mutationslücke bei `require(o.isMinter == poolIsMinter)` ist mit einem Test geschlossen: KCC20 lässt den Anteils-Minter seinen Status abgeben, nur der Pool verhindert das. Gegenprobe mit dem Mutanten: rot.

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| P-1 | mittel | `pool-add` und Schritt 3 von `pool-open` legten beide Beträge voll ein und verlangten nur 1 Anteil. Nach einer Kursverschiebung war der Überschuss verschenkt (Simulator: Opfer −24,8 %, Angreifer +482 KAS) | **Behoben:** Genommen wird nur der passende Teil. Ein Kursband von 1 % gegen das eingegebene Verhältnis bricht sonst ab, dazu Mindestanteile aus dem Probelauf. Die Seite warnt, wenn der Poolkurs mehr als 5 % vom Orakel abweicht (Restrisiko: wer zu einem verschobenen Kurs *passend* einlegt, verliert beim Zurückholen) |
| P-2 | niedrig | Anteile über 1e16 fand der Abgleich nicht | **Behoben** (Kandidaten bis `MAX_SHARES`, Test) |
| P-3 | mittel | Eine leere Node-Antwort löschte den Pool aus der Zustandsdatei | **Behoben:** Das gilt jetzt als Node-Fehler, und der Pool wird als „unbestimmt“ markiert, nie gelöscht |
| P-4, P-5 | niedrig | Der letzte Einleger kam nach Kursbewegungen nicht ganz heraus, und ein Mini-Abzug verbrannte Anteile für nichts | **Behoben:** KAS-Seite auf die 1-KAS-Grenze gekappt, Auszahlung (0, 0) wird abgelehnt (Test) |
| P-6 | Info | Token im Besitz der Covenant-IDs des Pools kann jeder in einer Pool-Tx ausgeben | **Offen**, dokumentiert. `ghostctl` kann solche Token nicht erzeugen |
| P-7 | Info | Die Echtheit eines fremden Pools hängt an seiner Genesis-Kette | **Offen**, dokumentiert. `ghostctl` nutzt nur selbst angelegte Pools. Eine spätere „Pool übernehmen“-Funktion muss Genesis und `init` prüfen |
| P-8 | Info | Anteils-UTXOs werden nicht nachgeführt, und abgezogen wird aus den zwei größten | **Offen**, niedrig |
| P-9 | Info | Keine Wiederholung bei „Pool bewegt“ für `pool-add` und `pool-remove`; Schritt 3 von `pool-open` ist nicht fortsetzbar | **Teilweise:** Schritt 3 meldet jetzt, dass der Rest per `pool-add` eingelegt werden kann |
| P-10 | niedrig | Mutationslauf: 28 von 60 Regeln rot, 32 ohne roten Test. Fast alle deckt KCC20 oder der Konsens ab. L214 (`OpCovInputCount(id) == 0` in `freshGenesis`) ist aber tragend: Ohne sie nimmt `init` einen bestehenden Token des Gründers als Anteils-Token | **Behoben:** Die Engine-Tests des Prüfers sind übernommen (`tests/audit10_pool_engine.rs`, 10 Tests, darunter L214 mit Mutanten-Gegenprobe). Dazu 4 Tests für L249/L250 (init ohne Minter- und pool-eigene GHOST) und `MAX_GHOST` (L168, L254), jeweils mit Gegenprobe am Mutanten: KCC20 allein ließe es zu, ohne die Regel geht es durch |
| – | Werkzeug | `mutation/mutate.sh` lief bei Abbruch nach dem Trap weiter und konnte den Vertrag leeren | **Behoben:** Der Trap beendet das Skript. Gegenprobe: Abbruch mitten im Lauf, Datei danach mit gleicher Prüfsumme |

## Audit 10 (Opus, 29.09.2026): Agent und Obergrenze

**Obergrenze `maxDebt` hält:** Mehrere Prägungen werden zusammengezählt. Nur `mint` erhöht die Schuld. Der Wert steckt im Template-Hash und ist nicht fälschbar. Zins kann die Schuld über die Grenze heben, dann ist nur Prägen gesperrt, Tilgen und Liquidieren gehen weiter. Die Grenze gilt **je Vault**: Wer zwei Vaults hat, kann 100 GHOST prägen (A-6). Das ist so gewollt; eine Obergrenze für alle GHOST zusammen soll es nicht geben (Entscheidung vom 29.09.2026).

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| A-1 | **hoch** | `keeper_burn` rundete ab. Bei einer Deckung zwischen 100 und 110 % lehnte der Vertrag in 9 % der Fälle ab, weil ein Rest unter 0,2 KAS blieb. Das wiederholte sich in jeder Runde, und weil nur der erste Kandidat versucht wurde, blockierte ein Vault alle weiteren Liquidationen | **Behoben:** Aufrunden. Test über 34 000 Preise im Band: nie abgelehnt, nie Verlust. Gegenprobe mit Abrunden: rot. Scheitert ein Kandidat, kommt der nächste dran |
| A-2 | mittel | „Nie mit Verlust“ galt nur zum Orakelpreis | **Behoben:** Liquidiert wird nur, wenn zum Marktpreis mindestens 2 % Gewinn bleiben (Test) |
| A-3 | mittel | Ein Liquidator ohne dieselbe Zustandsdatei findet das Orakel nach einem fremden Update nicht mehr | **Behoben** (`src/chain.rs`): Der Abgleich verfolgt Orakel und Vaults über den Adressverlauf der REST-API. Den neuen Zustand rekonstruiert er aus den Vertragsargumenten; jeder Kandidat muss den Skript-Hash exakt treffen, und der Node bestätigt Covenant-ID und Skript. Neue Vaults findet er über die Factory. Tests: Simulator mit 4 Updates, Prägen, Tilgen, Einzahlen, Teil-Liquidation, Schließen, gefälschte Ausgänge (Gegenprobe rot). Live im Mainnet: ein alter Stand ohne Vaults wird zu einem Ergebnis nachgeführt, das Feld für Feld mit der echten Zustandsdatei übereinstimmt |
| A-4 | mittel | `cap_room` ohne Grenze lief über (Endlosschleife unter der Dateisperre) | **Behoben** (Test) |
| A-5 | niedrig | Alte Zustandsdateien ohne `max_debt` passen nicht zu den Vaults | **Offen**, betrifft nur Dateien vor der Obergrenze. Im Mainnet gibt es keine (v1 ist zurückgebaut) |
| A-7 | niedrig | Sprungschutz prüfte nur die Richtung | **Behoben:** 3 Runden innerhalb von 5 %, gesendet wird der Median, Fehlschläge setzen zurück |
| A-8, A-9 | niedrig | `transfer` nahm ungültige Schlüssel an und führte Token vor der Prüfung zusammen | **Behoben** |
| A-10 | niedrig | Der Agent holte den Preis zweimal und unter der Sperre | **Behoben:** einmal je Runde, vor der Sperre |
| A-11 | niedrig | `agent` im Mainnet ohne `--ja` hätte unter der Sperre nachgefragt | **Behoben:** Abbruch mit Meldung |
| A-12 | niedrig | Startskripte: Besitzer-Schlüssel als Keeper, kein Neustart, KEYS/STATE uneinheitlich, Python-Einfügung | **Behoben:** `keys/<netz>-keeper.json` wird bevorzugt, Neustart nach 30 s. Alle 5 Komitee-Schlüssel neben dem Agenten bleiben bekannt (O-2) |
| A-13 | Info | Ein Abfragefehler leerte die ganze Schlüsselliste | **Behoben** |
| A-14, A-15 | Info | Preisquellen teilen Upstreams; Prägen gegen veraltetes Orakel | **Offen**, bekannt aus Audit 3 |

## Audit 10 (Opus, 29.09.2026): Web-Oberfläche und lokale API

Ohne Befund geprüft: Befehlsinjektion, Pfad-Traversal, CSRF und DNS-Rebinding, serverseitige Mainnet-Bestätigung, parallele Aktionen, die Kopplung Probelauf → Senden und die Poolrechnung.

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| W-1 | **hoch** | Die Betragseingabe las „1.500“ als 1500 (DE) und „0,001“ als 1 (EN), also bis zu 1000-fach zu viel | **Behoben:** strenger Parser je Sprache. Mehrdeutiges wird abgelehnt, und unter dem Feld steht der gelesene Wert (Tests DE und EN) |
| W-2 | mittel | Bei unklarem Ausgang (Zeitüberschreitung) stand „Nicht gesendet“ da | **Behoben:** Die Meldung lautet „Ergebnis unklar“, Senden bleibt bis zum neuen Status gesperrt |
| W-3 | mittel | „Als Befehl“ ließ bei Teil-Liquidation `--ghost` weg | **Behoben** (Test koppelt an die Positivliste des Servers) |
| W-5 | mittel | Probelauf und Bestätigung nannten weder Betrag noch Empfänger | **Behoben:** Die Bestätigung gilt nur für genau diese Werte |
| W-6 | mittel | Pool einlegen/abziehen ohne Mindestwerte | **Behoben:** Mindestwerte aus dem Probelauf (1 % Spielraum), `ghostctl --min-shares/--min-kas/--min-ghost` |
| W-4, W-7 bis W-12 | niedrig | Sprachwechsel während des Sendens, Cache-Race, GET fremder Seiten, zu breite Node-Fehlererkennung, ungefilterte Ausgabe, Probelauf ohne Verfall, Verlaufs-Labels | **Behoben.** Bei W-10 zeigt der Code: ghostctl gibt Schlüssel nur in Dateien aus; der Filter ist eine zweite Sicherung |
| W-13 bis W-18 | niedrig/Info | Texte (Orakel-Sprünge, Pool, Obergrenze, Statistik), x-only ohne Prüfung | **Behoben** |
| W-19 | Info | Lesende und sendende Aufrufe parallel | **Kein Befund:** ghostctl nimmt für jeden Befehl außer `agent`/`oracle-feed` die Dateisperre, der Agent je Runde |

## Audit 11 (Opus, 29.09.2026): Version 3

Kein Befund der Stufe kritisch oder hoch. Der Kern hält: GHOST entsteht nur gegen gleich hohe Schuld (V-03 behoben), eine Rücknahme verschlechtert die Quote nie (499 976 Zufallsfälle), `ops.rs`, `math.rs`, `chain.rs` und `vaultMath.ts` folgen dem Vertrag. Teil A ist im Vertrag behoben (Commit `bea9b56`), Teil B off-chain (Commit `a8165ea`). Tests und Belege je Befund:

**Teil A – Vertrag (`audit/11-opus-v3-vertrag.md`)**

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| A11-V-1 | mittel | Mehrere Vaults in einer Tx konnten auf denselben Kassen-Ausgang zeigen; die Kasse bekam nur die größte Einzelgebühr | **Behoben (bea9b56):** `close(oracleIdx, sig)`, Kassen-Ausgang fest an `activeInputIndex + 1`. Test `a11_zwei_vaults_teilen_sich_keine_zinszahlung`, `v3_schliessen_zahlt_den_zins_an_die_zinskasse` (Ausgang an anderer Stelle abgelehnt) |
| A11-V-2 | mittel | Rücknahme mit 0,5 % billiger als die Feed-Schwelle 1 %: lohnte gegen ein nachlaufendes Orakel, auf Kosten der Vault-Besitzer | **Behoben:** Rücknahmegebühr 1 % (bea9b56), Feed-Schwelle 0,5 % im Agenten, in `oracle-feed` und im Startskript (a8165ea). Tests `ruecknahme_zahlt_einen_dollar_je_ghost` (Rust), `vaultMath.test.ts`/`precheck.test.ts` (Seite). Größere Nachläufe bei Sprüngen über 20 %: siehe A11-O-2 |
| A11-V-3 | niedrig | 2 Einheiten Rücknahme bewegten jeden fremden Vault | **Behoben (bea9b56):** mindestens 1 GHOST oder die ganze Schuld. Test `v3_ruecknahme_mindestens_ein_ghost_oder_die_ganze_schuld`; Seite: `simRedeem`, Vorprüfung |
| A11-V-4 | niedrig | Vaults mit Schuld 0 und Zins > Sicherheit blieben nach einer Liquidation für immer liegen | **Behoben:** Eintrag `sweep` (bea9b56, Tests `a11_zombie_vault_wird_zugunsten_der_kasse_aufgeloest`, `a11_auflösen_nur_ohne_schuld_…`), `ghostctl sweep`, Agent löst automatisch auf, Knopf „Auflösen (Zinskasse)“ (a8165ea, Simulator-Test `keeper_loest_zombie_vault_zugunsten_der_zinskasse_auf`). **Neuer Befund dabei:** `SWEEP_FEE` (0,01 KAS) deckte die Netzgebühr der sweep-Tx (gemessen 0,054 KAS) nicht. **Behoben:** `SWEEP_FEE` = 0,1 KAS im Vertrag, der Auslöser legt nichts drauf (e2e-Test) |
| A11-V-5 | Info | Zins hing von der Abrechnungshäufigkeit ab (−9,7 % bei stündlich) | **Behoben (bea9b56):** `accrued = interest + accrual(debt + interest, …)`. Test `a11_zins_unabhaengig_von_der_abrechnungshaeufigkeit`; Seite gleich (`vaultMath.test.ts`: Referenzwerte 3,65e8 und 2,1e8, Pfadunabhängigkeit, 150 Zufallsfälle) |
| A11-V-6 | Info | L165 (`kasUsd > 0`) war im Vault die einzige Sperre gegen Preis 0 | **Behoben:** Test `a11_preis_null_nur_durch_l165_gesperrt` tötet die Mutante |
| A11-V-7 | Info | Keine ausdrückliche Obergrenze der GHOST-Ausgänge im Vault | **Behoben (bea9b56):** `require(nOut <= MAX_GHOST_OUTS)`. Test `a11_drei_ghost_ausgaenge_bei_ruecknahme` |
| A11-V-8 | Info | DUST-Erlass fest in KAS | **Dokumentiert** (`ARCHITEKTUR.md`); bei hohem KAS-Kurs neu bewerten |
| A11-V-9 | Info | Orakel-`maxRate` 31,5 % über der Agent-Grenze 20 % | **Behoben (bea9b56):** Deploy setzt `max_rate` = 20 % p. a. |
| A11-V-10 | Info | Veralteter Kommentar, Test ohne Aussage, Gegenprobe ohne Tx | **Behoben (bea9b56):** Kommentar `math.rs`; `v3_schliessen_rechnet_den_zins_bis_zum_orakelindex` prüft mit Zinseszins jetzt echten Zuwachs; Tx-Gegenprobe `a11_gegenprobe_falsches_orakel_als_tx` |

Mutationslücken: 12 von 13 Einschätzungen bestätigt, L165 ist mit Test geschlossen. Die verbleibenden (L152/L154/L171/L181–L184/L268/L285/L295/L296/L304) sind doppelt gesichert oder unerreichbar; Begründung je Zeile in `ARCHITEKTUR.md`, Abschnitt „Version 3 nach Audit 11“.

**Teil B – Off-chain (`audit/11-opus-v3-offchain.md`)**

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| A11-O-1 | mittel | Zinsregel hing an einer einzelnen Poolmessung ohne Mindestliquidität; für 0,006 USD verschiebbar | **Behoben (a8165ea):** `protocol/src/rate.rs`: Median der Messungen der letzten Stunde, ≥ 6 Messungen, Pool ≥ 10 GHOST, sonst keine Änderung mit Log-Zeile. Tests `rate::tests::median_statt_einzelmessung`, `mindestens_sechs_messungen_im_fenster`, `messung_nur_aus_bekanntem_pool_mit_mindestliquiditaet`. Rest: In einem wenig gehandelten Pool folgt der Kurs weiter der KAS-Bewegung (dokumentiert) |
| A11-O-2 | mittel | Rücknahme zum alten Orakelpreis, solange der Agent einen Sprung > 20 % bestätigt | **Dokumentiert als Rest-Risiko** (`ARCHITEKTUR.md`, Seite „So funktioniert’s“ und FAQ). Nicht behoben: Zwischenschritt beim Aufwärtssprung, Frischeprüfung oder dynamische Gebühr im Vertrag |
| A11-O-3 | mittel | Umzug stoppte bzw. prüfte einen laufenden Agenten nicht | **Behoben (a8165ea):** `pgrep -fl "ghostctl.*(agent|oracle-feed)"` vor Teil A und vor Teil B, Abbruch mit Meldung. Attrappen-Szenarien 1 und 9 |
| A11-O-4 | niedrig | Journal blieb beim Umbenennen liegen und hätte v2-Inhalt nach `mainnet.json` geschrieben | **Behoben:** Journal, Deploy-Fortschritt und Sperre werden unter der flock-Sperre mitbenannt, das Ziel im Journal auf `mainnet-v2.json` umgeschrieben; fremdes Ziel oder vorhandene Zieldatei → Abbruch ohne Umbenennen. Szenarien 4, 6, 7 |
| A11-O-5 | niedrig | Wiederaufnahme zählte alle Vaults, übersprang das Prägen; `pool-open` sendete die Genesis vor der GHOST-Prüfung | **Behoben:** nur eigene Vaults; eigener Vault ohne Schuld → nur prägen, am richtigen Index; `pool-open` nur mit genug GHOST und KAS. Szenarien 4 und 5 |
| A11-O-6 | niedrig | Kein Schutz gegen gleichzeitigen Doppelstart | **Behoben:** `mkdir`-Sperre `deployments/.umzug-v3.lock` mit `trap`. Szenarien 2 und 10 |
| A11-O-7 | niedrig | `redeem` führte Token vor der Prüfung zusammen | **Behoben (bea9b56):** `math::redemption` vor `consolidate` |
| A11-O-8 | niedrig | `pool_unresolved` griff nie, gemessen wurde der Pool der Vorrunde | **Behoben (a8165ea):** Die Zinsregel misst am frisch abgeglichenen Stand derselben Runde (`oracle_round` → `load_synced`), `pool_unresolved` verhindert die Messung (`rate::measure`, Test) |
| A11-O-9 | niedrig | `RATE_CLOCK` nur im Speicher: Neustart oder paralleles `oracle-feed` → mehrere Schritte je Stunde | **Behoben:** Takt in `deployments/<netz>-zins.json` unter eigener Sperre, Vormerkung vor dem Senden. Tests `takt_hoechstens_einmal_je_stunde`, `datei_ueberlebt_neustart_und_sperrt_parallele_prozesse` (4 Threads, genau einer darf) |
| A11-O-10 | Info | `rate_next` außerhalb des Rasters | **Behoben (bea9b56):** erst einrasten, dann Schritt (Test `zinsregel_folgt_dem_ghost_kurs`) |
| A11-O-11 | Info | Index-Fortschreibung rundet ab, entgegen „zugunsten der Zinskasse“ | **Dokumentiert:** Text auf der Seite präzisiert, Zahlen in `ARCHITEKTUR.md` |
| A11-O-12 | Info | Schließen/Abheben mit Rest unter ≈ 0,02 KAS nicht baubar, Seite ohne Warnung | **Behoben (Seite):** Vorprüfung warnt beim Schließen und Abheben unter 0,025 KAS (Test in `precheck.test.ts`). ghostctl selbst unverändert |
| A11-O-13 | Info | Veraltete Kommentare „Zins anteilig“ | **Behoben (bea9b56)** in `math.rs` und `chain.rs` |
| A11-O-14 | Info | Umzug: `DRY=0` galt als Probelauf und weitere Kleinigkeiten | **Behoben:** nur `DRY=1` ist Probelauf, unbekannte Werte brechen ab; unlesbare `mainnet.json` wird nicht mehr als v2 umbenannt. Keeper-GHOST, eingefrorener Rest-Vault und Daueraufträge in `MAINNET.md` erklärt. **Dazu gefunden:** Schritt 2 übersprang wegen eines Python-Syntaxfehlers jeden Vault – behoben, Szenarien 4 und 10 |
| A11-O-15 | Info | Rücknahme/Liquidation adressieren den Vault über die Nummer, `redeem` ohne `--min-kas` | **Offen, dokumentiert** (`ARCHITEKTUR.md`) |

Testzahlen nach a8165ea: Rust `cargo test --release` 259 Tests grün (lib 32 davon 7 `rate::tests`, e2e 4, vault 81, übrige Suiten unverändert; `rest_live_tests` 1 ignoriert, braucht Netz), Seite `vitest` 204 Tests grün, `tsc` und `npm run build` sauber.

Umzugsskript: `zsh -n` sauber, 10 Szenarien mit Attrappen von `ghostctl`, `bin/ghostctl-v2` und `pgrep` (feste JSON-Antworten, kein Netz, keine Schlüssel), 39 Prüfungen bestanden.

## Audit 12 (Opus, 29./30.09.2026): Nachprüfung und neue Teile

Kein Befund der Stufe kritisch oder hoch. Die Behebungen von Audit 11 halten im Vertrag (V-1, -2, -3, -5, -9, -10; V-4 mit A12-2/A12-15, V-7 als Zweitsicherung). Behoben in drei Zweigen, jeder von einem unabhängigen Prüfer mit Rückbau-Proben (Kernänderung zurücksetzen → ein Test wird rot) gegengeprüft und bis zur Freigabe nachgebessert; die Restpunkte der Prüfer in einer weiteren Runde. Einzelheiten, Tests und Rückbau-Proben je Befund in `audit/12-stand-a.md`, `-b.md`, `-c.md`.

| ID | Schwere | Befund | Stand |
|---|---|---|---|
| A12-1 | mittel | Tresor-Vertrag band den Payload nicht: wer auslöst, konnte eine eigene (auch verschlüsselte) Nachricht in die echte Zahlung legen | **Behoben im Vertrag:** Parameter `payloadHash`, `pay()` verlangt `sha256(Payload) == payloadHash` (c046461; Tests `nachricht_ist_an_jede_zahlung_gebunden`, `ohne_nachricht_bleibt_der_payload_leer`, `laengste_nachricht_passt`; Mutationstest 15/15). Rust/Seite: Code v2, Eingang „vom Vertrag erzwungen“ |
| A12-2 | niedrig | Zombie-Vault unter 0,1 KAS ließ `ops::sweep` überlaufen, Agent versuchte immer denselben | **Behoben:** `sweepable` nur bei baubarer Auflösung, Keeper versucht der Reihe nach |
| A12-3 | niedrig | Zinstakt fror nach vorgestellter Uhr ein | **Behoben** (`rate.rs`) |
| A12-4 | niedrig | Umzug suchte den eigenen Schlüssel über die Dateiendung; Fehltreffer → weiterer 50-KAS-Vault je Doppelklick | **Behoben:** genau ein Treffer über die Besitzer-Datei, sonst Abbruch; Schritt 5 nur ohne eigenen v3-Vault |
| A12-5 | niedrig | Agentenschleife der Reihe nach, bis ≈ 30 min zwischen Orakel-/Keeper-Runden | **Behoben:** Nebenschritte mit Sendelimit nach Orakel/Keeper, Tests mit angehaltener Uhr |
| A12-6 … A12-10 | niedrig | Tresor-Seite: Probelauf nicht ans Netz gebunden, „fällig“ ohne Zahlbarkeit, Texte zur Höchstgebühr, Ortszeit/UTC, Abholen ohne Rückfrage | **Behoben** bzw. Texte korrigiert (A12-8) |
| A12-11 | niedrig | Nachrichtenfilter ließ unsichtbare Zeichen durch | **Behoben:** gleicher Filter in Rust, Seite und Server, gemeinsame Fallsammlung |
| A12-12 | Info | Zinsregel: 6 Messungen in 20 min reichten | **Behoben:** Messungen müssen ≥ 45 min umspannen |
| A12-13 | Info | Vault-`close` und ein Tresor an die Zinskasse teilen sich einen festen Ausgang | **Dokumentiert** (nur Tresore an die Kasse, Verlust trägt die Kasse selbst), Test hält es fest |
| A12-14 | Info | `interestFee` Rechengrenze bei Tiefstpreis | **Behoben** (sättigend in Rust/Seite), Vertragsgrenze dokumentiert (mit 50 GHOST unerreichbar) |
| A12-15 | Info | Testlücken (u. a. `noGhost()` in `sweep` ungetestet) | **Behoben:** Tests für M04 und M08 |
| A12-16 … A12-19 | Info/niedrig | Tresor-Automatik, Umzug-Kleinigkeiten, Seite (maxFee, Auffüllen, winzige Rücknahme), Eingang glaubte der REST-API | **Behoben**; Eingang prüft am Node, sonst „laut REST-API“ |

Mutationstest Vault nach den Audit-11-Behebungen: 51 von 64 `require`-Zeilen einzeln abgedeckt; die 13 übrigen sind die doppelt gesicherten bzw. unerreichbaren Zeilen (Begründung `ARCHITEKTUR.md`), `noGhost()` in `sweep` (kein `require`) durch Test A12-15 abgedeckt.

## Audit 13 (Opus, 30.09.2026): Abschlussprüfung

Zwei Prüfer (Tresor-Nachrichtenbindung; Einmal-Bestätigen-Modus des Umzugs), jeder Befund von einem zweiten Prüfer adversarial gegengeprüft (alle bestätigt, teils herabgestuft). **Die Nachrichtenbindung im Vertrag hält vollständig:** Payload 0 bis 10 000 Byte über die Engine, jede Byte-Änderung abgelehnt; Skript ohne Kompilieren und `parse_script` byte-genau in 2 113 Varianten; 15 von 16 Mutanten erkannt (der eine ist gleichwertig). Behoben in zwei Zweigen, je vom Prüfer im ersten Anlauf freigegeben (`audit/13-stand-tresor.md`, `audit/13-stand-umzug.md`).

| ID | Schwere (bestätigt) | Befund | Stand |
|---|---|---|---|
| A13-umzug-1 | mittel | Nach einer Bestätigung sprach das Skript v2-Vaults über die Nummer an; bei verschobener Nummer hätte es einen anderen eigenen Vault getilgt | **Behoben:** vor jedem Senden Nummer ↔ bestätigte Covenant-ID prüfen, sonst Halt; `repay --ghost <bestätigter Betrag>`; nach jedem Senden Wirkung prüfen |
| A13-umzug-2 | mittel | `pool-remove` ohne Mindestbeträge | **Behoben:** `--min-kas/--min-ghost` aus dem bestätigten Plan minus genannter Toleranz |
| A13-umzug-3 … -7 | niedrig/Info | mögliche Zusammenführungs-Tx nicht genannt, Signal-Abbruch ohne Stand, Schlüssel/Adresse nicht in der Zusammenfassung, Pool-Toleranz, Float-Rundung | **Behoben** (-3 in der Anzeige dokumentiert); Beträge in Sompi |
| A13-tresor-1 | niedrig | Bei verschlüsselter Nachricht war die Beschreibung im Tresor-Code nicht gebunden | **Behoben:** Import entschlüsselt mit dem Empfängerschlüssel und vergleicht; ohne Schlüssel „laut Tresor-Code, nicht geprüft“ |
| A13-tresor-2, -3 | niedrig/Info | Erfundene Beschreibung zu Tresor ohne Nachricht; öffentlich ohne Text mit Chiffrat | **Behoben:** Formregeln in `check_bound`, `upsert` übernimmt nur gebundene Beschreibungen |
| A13-tresor-4 | niedrig | Höchstgebühr unter der Mindestgebühr erlaubt | **Behoben:** `MIN_MAX_FEE` 0,004 KAS (gemessen höchstens 0,00359 KAS mit längster Nachricht) |

Testzahlen nach Audit 13: Rust `cargo test --release --offline` 382 grün, 2 ignoriert (brauchen Netz); Seite `vitest` 424 grün, `tsc` und `npm run build` sauber; Umzugs-Szenarien `tests/umzug/run.zsh` 179 bestanden, 0 fehlgeschlagen.

## Bekannte Grenzen (nicht behoben)

- **Orakel:** Im Mainnet-Probelauf hält der Betreiber alle 5 Schlüssel. Wer 3 davon hat, bestimmt den Preis innerhalb von 0,00001–900 USD, und zwar praktisch sofort: Die Grenze ×2/÷2 je Update und der Abstand von 600 DAA bremsen nur, wenn das letzte Update kurz zurückliegt (Fix-Review 8, NEU-1).
- **Liquidation:** Ein Keeper-Agent ist gebaut (`GHOST-Agent starten.command`), ob einer läuft, ist nicht garantiert. Ein Liquidator auf einem anderen Rechner braucht einmal eine Kopie der Zustandsdatei und die REST-API (api.kaspa.org), um fremde Änderungen nachzuführen.
- **Minter-Zweige:** Nach dem Schließen eines Vaults bleiben je 3 KAS dauerhaft gebunden.
- **Durchsatz:** Nur ein Vault pro Transaktion. Alle preisabhängigen Aktionen konkurrieren um die Orakel-UTXO.
- **Prüfumfang:** Die Verträge sind weder formal verifiziert noch von einer unabhängigen Prüfgesellschaft auditiert.
