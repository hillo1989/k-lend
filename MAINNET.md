# Mainnet-Probelauf mit Kleinstbeträgen

> **Echtes Geld.** Die Verträge sind nicht auditiert. Setze nur ein, was du verlieren kannst.
> Jede Mainnet-Transaktion zeigt vorher die Gebühr an und fragt `MAINNET – wirklich senden? [j/N]`. **Ausnahmen:**
> - Der Agent (`GHOST-Agent starten.command`) sendet Orakel-Updates und Liquidationen mit `--ja` ohne Rückfrage.
>   Er nimmt `keys/mainnet-keeper.json`, wenn es die Datei gibt, sonst den Besitzer-Schlüssel, und darf dann dessen GHOST für Liquidationen verbrennen. Er liquidiert nur, wenn zum Marktpreis mindestens 2 % Gewinn bleiben. Als reiner Liquidator (ohne Komitee-Datei) genügt auf einem anderen Rechner einmal eine Kopie von `deployments/mainnet.json`: Fremde Orakel-Updates, Vault-Änderungen und neue Vaults führt ghostctl selbst von der Kette nach (`ghostctl sync`, im Agenten jede Runde).
> - Die Lending-Seite sendet erst nach einer erfolgreichen Prüfung, wenn du danach auf Senden klickst.
> Private Schlüssel erzeugst und hältst **du**. Sie liegen in `keys/`, sind nur für dich lesbar (Rechte 600) und stehen in `.gitignore`.

## Version 1 und Version 2

Das erste Mainnet-Deployment vom 28.09.2026 war **Version 1**. Ein unabhängiges Audit hat darin einen kritischen Fehler im GHOST-Token gefunden: Über negative Beträge ließen sich beliebig viele GHOST erzeugen. Details stehen in `AUDIT.md`. Version 2 behebt das, braucht aber ein **neues Deployment**.

- **Alter Vault:** am 28.09.2026 zurückgebaut, die 150 KAS sind zurück. Version 1 ist damit erledigt; `bin/ghostctl-v1` und `deployments/mainnet-v1.json` bleiben nur als Beleg.
- **Version 2 anlegen:** wie unten beschrieben, mit `GHOST-Mainnet-Test.command`. Deine Schlüssel bleiben dieselben. Das neue Deployment bindet wieder etwa 33 KAS.

## Version 3 (ab 29.09.2026) und Umzug

Version 3 hält GHOST bei etwa 1 USD: Zins als eigener Posten mit Zinskasse, Rücknahme zu 1 USD, automatische Zinsregel im Agenten, Pool mit Kursband. Details stehen in `ARCHITEKTUR.md`, Abschnitt „Version 3". Weil sich Vault und GHOST ändern, braucht Version 3 ein **neues Deployment**. Die GHOST von Version 2 gelten dort nicht.

**Umzug per Doppelklick: `GHOST-Umzug-v3.command`. Du bestätigst einmal.** Vor dem ersten Senden rechnet das Skript den ganzen Plan. Teil A baut es mit `--dry-run` wie der Probelauf, Teil B rechnet es aus dem Stand. Danach zeigt es eine Zusammenfassung. Im Kopf stehen Netz, Schlüsseldatei (mit vollem Pfad), Adresse und x-only des Schlüssels, mit dem gesendet wird. Wer `KEYS=…` setzt, sieht hier vor dem `j`, welche Datei gilt (Audit 13 A13-umzug-5). Für jeden Schritt stehen dort die Aktion mit Beträgen (KAS/GHOST), was zurückkommt, was gebunden bleibt und die Gebühr (gebaut oder geschätzt). Darunter stehen Gebühren und Endstand: KAS jetzt, vor Teil B und am Ende sowie GHOST am Ende. Dann kommt **eine** Frage `Alles so ausführen? [j/N]`. Nur bei `j` laufen alle etwa zwölf Transaktionen mit `--ja` durch. Jede andere Antwort bricht ab. Dann ist nichts gesendet und auch nichts umbenannt.
- **Was die Zusammenfassung außerdem nennt (Audit 13):** Beim Pool-Abzug die Mindestbeträge, die zurückkommen müssen (Plan minus 1 %, siehe unten). Beim Pool-Anlegen die Toleranz: Der KAS-Betrag folgt dem Orakelkurs beim Senden, bis ± 1 % geht ohne erneute Frage hinaus (A13-umzug-6). Bei Tilgen und Pool-Anlegen eine mögliche Zusatz-Transaktion „GHOST zusammenführen“: ghostctl sendet sie mit `--ja` ohne Rückfrage, wenn die eigenen GHOST auf mehr als zwei UTXOs liegen. Sie ist eine Selbstüberweisung und kostet nur Gebühr; die Gebührenzeile zählt sie als „ggf. bis zu …“ (A13-umzug-3).
- **Abbruch unterwegs:** Das Skript stoppt sofort. Es nennt, was in diesem Lauf schon gesendet ist und was laut Plan noch fehlt. Das gilt auch für Ctrl+C und `kill` (Signal): Dann nennt es zusätzlich den Schritt, der gerade lief. Ob dessen Transaktion angenommen wurde, klärt ghostctl beim nächsten Aufruf über das Journal (A13-umzug-4). Ein erneuter Doppelklick überspringt die erledigten Schritte, zeigt die Zusammenfassung der **restlichen** Schritte und fragt wieder einmal.
- **Teil A wird vor jedem Senden mit dem Plan verglichen:** Tilgen, Schließen und Herausnehmen hängen an dem, was der Pool wirklich zurückgibt, und am Orakel von Version 2. Bevor Teil A einen Schritt sendet, vergleicht das Skript Befehl, Vault und Betrag mit dem bestätigten Plan. Wenn sie abweichen, etwa weil weniger GHOST aus dem Pool kamen und nur noch teilweise getilgt werden kann, sendet es diesen Schritt nicht. Es zeigt dann „geplant“ und „jetzt“, rechnet Teil A aus dem jetzigen Stand neu und fragt erneut (`Teil A so ausführen? [j/N]`). Fällt ein geplanter Schritt weg, fragt es vor Teil B (`Ohne diese Schritte mit Teil B weitermachen? [j/N]`). Ist nichts mehr zu senden, endet es ohne Frage.
- **Vault-Nummer und Covenant-ID (Audit 13 A13-umzug-1):** `bin/ghostctl-v2` kennt einen Vault nur als Nummer (`--vault`). Verschwindet ein Vault mit kleinerer Nummer (ein fremder wird geschlossen oder liquidiert), rücken die Nummern nach, und dieselbe Nummer träfe einen anderen eigenen Vault. Deshalb liest das Skript unmittelbar vor jedem Tilgen, Schließen und Herausnehmen den Status neu. Trägt die Nummer nicht mehr die bestätigte Covenant-ID, hält es an, ohne zu senden („Vault-Nummer verschoben“). Getilgt wird immer mit `--ghost <bestätigter Betrag>`, auch beim vollen Tilgen; mehr als bestätigt geht so nie hinaus. Nach jedem Senden prüft es, dass sich genau dieser Vault wie erwartet geändert hat (Schuld, Sicherheit bzw. geschlossen) und die übrigen eigenen Vaults nicht. Sonst hält es an und zeigt Soll und Ist. Ein erneuter Doppelklick rechnet mit den neuen Nummern und fragt erneut.
- **Pool-Abzug mit Mindestbeträgen (Audit 13 A13-umzug-2):** `pool-remove` geht mit `--min-kas` und `--min-ghost` hinaus. Die Werte sind der Rückfluss aus dem bestätigten Plan (gerechnet wie ghostctl: Anteil der Reserven, abgerundet, KAS-Seite höchstens bis auf 1 KAS Mindestreserve) minus 1 % (`TOL_ABZUG_BPS=100` im Skript). Verschiebt sich der Pool bis zum Senden, rechnet das Skript neu und fragt. Verschiebt er sich während des Sendens stärker, sendet ghostctl nicht („Der Pool hat sich verschoben“), und das Skript bricht vor dem Tilgen ab.
- **Rechnen in Sompi (Audit 13 A13-umzug-7):** Tilgbetrag, Grenzen (voll getilgt, schuldenfrei, weniger als die Sicherheit) und `--keep` rechnet das Skript in ganzen Sompi bzw. GHOST-Einheiten, `--keep` exakt mit Brüchen. Float-Rauschen hob `--keep` vorher um 0,01 KAS an (etwa 10,01 statt 10,00 KAS).
- **Vor Teil B wird neu gerechnet:** Nach Teil A haben sich die Guthaben geändert (Rückflüsse aus Pool und Vaults). Deshalb rechnet das Skript Teil B (Deployment, Vault, Prägen, Pool) aus dem echten Stand neu. Es hält an und fragt erneut (`Teil B so ausführen? [j/N]`), wenn der Stand abweicht: andere Schritte, mehr als 0,5 KAS weniger als geplant oder ein Pool-Betrag, der um mehr als 1 % abweicht. Ebenso fragt es vor dem Pool noch einmal, wenn der Orakelkurs nach dem Deployment den KAS-Betrag des Pools um mehr als 1 % verschiebt. Andere Beträge sendet es nie still.
- **Wie früher, je Transaktion fragen:** `EINZELN=1 ./GHOST-Umzug-v3.command`. Die Zusammenfassung erscheint trotzdem, aber ohne Sammelfrage.
- **Probelauf ohne Senden:** `DRY=1 ./GHOST-Umzug-v3.command`. Nur `DRY=1` ist ein Probelauf, `DRY=0` oder nichts sendet echt. Der Probelauf fragt nichts.
0. **Vorher den GHOST-Agenten und einen etwa laufenden `oracle-feed` beenden.** Das Skript prüft das vor Teil A und noch einmal vor Teil B (`pgrep`) und bricht sonst ab: Ein noch laufender Agent der Version 2 würde die neue Zustandsdatei im alten Format zurückschreiben, danach hinge der Umzug (Audit 11 A11-O-3). Ein zweiter, gleichzeitig gestarteter Umzug bricht an der Sperre `deployments/.umzug-v3.lock` ab.
1. Nach dem `j` wird `deployments/mainnet.json` (Version 2) in `deployments/mainnet-v2.json` umbenannt (vor der Frage prüft das Skript nur, ob das geht), zusammen mit dem Journal (`mainnet.pending.json` → `mainnet-v2.pending.json`, dessen Zieldatei auf den neuen Namen umgeschrieben wird), dem Deploy-Fortschritt (`mainnet.deploy.json`) und der Sperre (`mainnet.lock`). Das geschieht unter der Dateisperre von ghostctl, damit die Seite nicht dazwischen ein Journal klärt. Es wird nichts gelöscht; gibt es ein Ziel schon, bricht das Skript ab, ohne etwas umzubenennen.
2. Mit `bin/ghostctl-v2`: Pool-Anteile abziehen, Vaults mit den eigenen GHOST tilgen (kleinste Schuld zuerst) und schuldenfreie Vaults schließen. Beim letzten Vault bleiben etwa 0,045 GHOST Restschuld, weil die Mindestliquidität des alten Pools für immer gebunden ist. Dieser Vault behält die nötige Sicherheit plus 10 % Puffer (etwa 2–3 KAS), der Rest kommt zurück.
3. Mit `./ghostctl` (Version 3): Deployment (30 KAS bleiben gebunden), eigener Vault mit `VAULT_KAS` (Standard 50) KAS, `MINT_GHOST` (Standard 0,5) GHOST prägen, Pool mit Kursband und `POOL_GHOST` (Standard 0,25) GHOST. Gezählt werden nur **eigene** Vaults: Gibt es schon einen eigenen Vault ohne Schuld (etwa weil das Prägen beim letzten Lauf scheiterte), wird nur geprägt. Den Pool legt das Skript nur an, wenn der Schlüssel genug GHOST und KAS hat (Pool-Betrag plus etwa 4 KAS für Minter, Token-UTXOs und Gebühren); sonst bricht es vorher mit einer klaren Meldung ab. Die Beträge lassen sich beim Aufruf ändern, zum Beispiel `VAULT_KAS=80 ./GHOST-Umzug-v3.command`.
4. Danach den Agenten neu starten. Er betreibt Orakel, Zinsregel, Liquidationen und das Auflösen von Zins-Vaults auf Version 3.

Hinweise zum Umzug (Audit 11 A11-O-14):
- Abgebaut werden nur Vaults, GHOST und Pool-Anteile des Besitzer-Schlüssels. GHOST auf `keys/mainnet-keeper.json` werden nicht zum Tilgen genommen; wer sie mitnutzen will, schickt sie vorher an den Besitzer-Schlüssel.
- Der Rest-Vault der Version 2 bleibt mit 220 % zum eingefrorenen v2-Orakel stehen. Solange niemand das v2-Orakel bewegt, ist er nicht liquidierbar.
- Daueraufträge (`deployments/mainnet-abos.json`) bleiben bei `mainnet.json`: GHOST-Aufträge zahlen nach dem Umzug **GHOST der Version 3**.

`GHOST-Pool-Kursband.command` ist mit dem Umzug überholt: Version 3 legt den Pool gleich mit Kursband an. Führst du es vorher noch aus, bleibt im alten Pool ein weiteres Mal Mindestliquidität gebunden.

**Was sich für dich ändert:**
- **Schuld = geprägte GHOST.** 1 GHOST tilgt genau 1 GHOST Schuld, auch bei Zins über 0 %.
- **Zins** läuft in USD auf, steht in `status` als „Zins" und zählt für Quote und Liquidation. Bezahlt wird er beim Schließen in KAS an die Zinskasse (die Deployer-Adresse). Unter 0,2 KAS wird er erlassen.
- **Zinseszins:** Offener Zins verzinst sich mit, solange er nicht bezahlt ist (auch nach vollem Tilgen). So hängt der Betrag nicht davon ab, wie oft abgerechnet wird (Audit 11 A11-V-5).
- **Rücknahme:** `./ghostctl redeem --key <Schlüssel> --vault <Nr> --ghost <Betrag>` gibt GHOST an einen Vault ab 150 % zurück. Dafür gibt es KAS im Wert von 1 USD je GHOST **minus 1 %**. Zurückgegeben wird mindestens 1 GHOST oder die ganze Schuld des Vaults. Gerechnet wird zum Orakelpreis; das Orakel folgt dem Markt ab 0,5 % Abweichung (Agent `--min-change 0.005`), also unter der Gebühr von 1 %. Bei Sprüngen über 20 % wartet der Agent drei Runden: In dieser Zeit ist eine Rücknahme günstiger als der Markt, die Differenz trägt der Vault-Besitzer (Audit 11 A11-O-2, siehe `ARCHITEKTUR.md`).
- **Zins-Vault auflösen:** `./ghostctl sweep --key <Schlüssel> --vault <Nr>` löst einen Vault ohne Schuld auf, dessen Zins die ganze Sicherheit aufzehrt (bleibt manchmal nach einer Liquidation übrig). Jeder darf das. Die Sicherheit geht bis auf 0,1 KAS an die Zinskasse. Diese 0,1 KAS decken die Netzgebühr (etwa 0,055 KAS), den Überschuss bekommt der Auslöser `--key`. Der GHOST-Agent macht das automatisch, höchstens einen Vault je Runde. Auf der Seite: Knopf „Auflösen (Zinskasse)“ bei solchen Vaults.
- **Zinsregel (Stand Audit 20, 06.10.2026):** Der Agent misst je Runde den GHOST-Kurs im frisch abgeglichenen Pool und ändert den Zins nach dem Median der letzten Stunde, frühestens ab 6 Messungen über 45 min, nur wenn der Pool mindestens 10 GHOST hält, und höchstens einmal pro Stunde. Unter 0,97 USD steigt er um 0,5 Punkte, über 1,03 USD sinkt er um 0,5 Punkte (nie unter den Grundzins 2 % p. a.), dazwischen bleibt er. Geändert wird nur, wenn der Pool in dieser Stunde gehandelt wurde: Sein Tauschverhältnis (KAS ÷ GHOST in der Reserve) muss sich zwischen den Messungen zusammen um mindestens 2 % bewegt haben. Reine KAS-Bewegung ohne Tausch ändert den Zins nicht mehr. Liegt der Zins unter 2 %, hebt der Agent ihn stündlich um 0,5 Punkte auf 2 % an. Messungen und letzte Änderung liegen in `deployments/mainnet-zins.json` (mit Sperre `mainnet-zins.lock`), das gilt also auch über Neustarts und für ein parallel laufendes `oracle-feed`. Der Startpool des Umzugs (0,25 GHOST) liegt unter der Mindestliquidität: Bis jemand mehr einlegt, bleibt der Zins stehen, und der Agent schreibt das in jeder Runde ins Protokoll.

## Am einfachsten: Doppelklick

**`GHOST-Mainnet-Test.command`** im Projektordner doppelklicken. Das Skript macht Folgendes:
1. Es legt deine Schlüssel an (`keys/mainnet-owner.json`, `keys/mainnet-committee.json`) und überschreibt dabei nie einen vorhandenen Schlüssel.
2. Es zeigt deine Mainnet-Adresse und wartet, bis mindestens 155 KAS angekommen sind.
3. Es legt Orakel, Factory und GHOST an, eröffnet einen Vault mit 100 KAS und prägt 1 GHOST. **Jede Transaktion fragt vorher nach `j`.**
4. Es legt den offenen Tauschpool an: 0,5 GHOST und KAS im gleichen Dollarwert zum aktuellen Kurs (bei 0,045 USD etwa 11 KAS). Das Verhältnis ist der Startkurs von GHOST. Das sind drei Transaktionen: Pool anlegen, initialisieren, Rest einlegen. 1 KAS und etwa 0,045 GHOST bleiben als Mindestliquidität für immer im Pool, für den Rest bekommst du Anteile.

Das Skript wurde am 28.09.2026 im Testnetz komplett durchgespielt. Verbraucht wurden 184 von 250 tKAS: 30 KAS fest im Deployment, 150 KAS im Vault, 4 KAS für Minter-Zweig und Token, dazu 0,1 KAS Gebühren.

`ghostctl` läuft standardmäßig im **Mainnet**. Fürs Testnetz hängst du `--network testnet-10` an.

## Einzelne Befehle

Alle Befehle im Terminal im Projektordner ausführen:

```bash
cd ~/Desktop/Claude/kaspa-lending
```

## Was es kostet (gemessen im Testnetz am 28.09.2026)

| Posten | KAS | Zurückholbar? |
|---|---|---|
| Orakel-UTXO | 10 | nein, bleibt dauerhaft im Orakel-Covenant |
| Factory-UTXO | 10 | nein |
| GHOST-Wurzel-Minter | 10 | nein |
| Minter-Zweig je Vault | 3 | nein, bleibt nach dem Schließen verwaist |
| GHOST-Token-UTXO | 1 je Stück | ja, beim Tilgen oder Überweisen kommt es als Wechselgeld zurück |
| Gebühren | Deploy ≈ 0,014; Vault-Aktion ≈ 0,04–0,05; Orakel-Update ≈ 0,0065 | nein |
| Sicherheit im Vault | frei wählbar | ja, beim Abheben oder Schließen |
| Tauschpool: Mindestliquidität | 1 KAS + GHOST zum Startkurs (im Test ≈ 0,045) + je 1 KAS in Reserve- und Anteils-UTXO | nein, bleibt für immer im Pool |
| Tauschpool: deine Einlage | frei wählbar | ja, mit `pool-remove` samt anteiliger Gebühren |
| Gebühr je Pool-Tx | ≈ 0,06 (Netz); beim Tausch zusätzlich 0,3 % des Zuflusses (geht an die Einleger) | nein |

**Empfehlung für den Probelauf:** etwa **160 KAS** auf die Deployer-Adresse. 30 KAS gehen fest ins Deployment, 3 KAS in den Minter-Zweig, 1 KAS in den Token, 100 KAS dienen als Sicherheit, etwa 12 KAS gehen in den Tauschpool.

## Ablauf

**1. Schlüssel erzeugen.** Die Datei wird nie überschrieben.

```bash
./ghostctl keygen keys/mainnet-owner.json
```

```bash
./ghostctl committee-keygen keys/mainnet-committee.json
```

Das Komitee aus 5 Orakel-Schlüsseln liegt beim Probelauf komplett bei dir. Damit bist du allein das Orakel. Für einen öffentlichen Betrieb müssen die 5 Schlüssel auf verschiedene Personen verteilt werden.

**2. Adresse anzeigen und KAS dorthin senden** (aus deiner eigenen Wallet):

```bash
./ghostctl --network mainnet balance --key keys/mainnet-owner.json
```

**3. Deployment** anlegen: Orakel mit dem aktuellen Median aus 6 Quellen, Factory und GHOST. Es folgen drei Rückfragen.

```bash
./ghostctl --network mainnet deploy --key keys/mainnet-owner.json --committee keys/mainnet-committee.json --rate 0
```

**4. Vault eröffnen** mit 100 KAS:

```bash
./ghostctl --network mainnet open-vault --key keys/mainnet-owner.json --kas 100
```

**5. GHOST prägen.** Bei 100 KAS und 0,046 USD sind das höchstens etwa 2,3 GHOST. Für den Anfang reicht 1 GHOST. Unabhängig von der Sicherheit gilt: höchstens 50 GHOST Schuld je Vault, die Zahl der Vaults ist nicht begrenzt.

```bash
./ghostctl --network mainnet mint --key keys/mainnet-owner.json --vault 0 --ghost 1
```

**6. Überblick** über Quote und Liquidationspreis:

```bash
./ghostctl --network mainnet status
```

**7. Orakel aktualisieren** (mindestens vor jeder Aktion, bei der der Preis zählt):

```bash
./ghostctl --network mainnet oracle-update --key keys/mainnet-owner.json --committee keys/mainnet-committee.json
```

**8. Tauschen** (optional; ohne `--min-…` gilt 1 % unter dem aktuellen Kurs als Mindestbetrag):

```bash
./ghostctl --network mainnet swap --key keys/mainnet-owner.json --kas 2
```

```bash
./ghostctl --network mainnet swap --key keys/mainnet-owner.json --ghost 0.05
```

**9. Zurückholen:** zuerst deine Pool-Anteile abziehen (die GHOST daraus brauchst du zum Tilgen).

```bash
./ghostctl --network mainnet pool-remove --key keys/mainnet-owner.json --percent 100
```

**Wichtig:** Die etwa 0,045 GHOST der Mindestliquidität bleiben im Pool. Du hast danach also etwas weniger GHOST, als du geprägt hast, und kannst den Vault nicht ganz tilgen. Tilge, was du hast (`repay --ghost <Betrag>`), und hol die Sicherheit bis auf wenige KAS mit `withdraw --keep` heraus. Bei 0,045 GHOST Restschuld reichen etwa 3 KAS. Voll tilgen und schließen geht erst, wenn du die fehlenden GHOST irgendwo bekommst, zum Beispiel im Pool gegen KAS.


```bash
./ghostctl --network mainnet repay --key keys/mainnet-owner.json --vault 0
```

```bash
./ghostctl --network mainnet close --key keys/mainnet-owner.json --vault 0
```

**Volles Tilgen in Version 2** (in Version 3 entfällt das, siehe oben): Der Zins steht auf 0 %, der Schuldindex bleibt bei 1,0. Die Schuld ist deshalb exakt die geprägte Menge, und 1 GHOST tilgt 1 GHOST Schuld. Das Folgende gilt nur, wenn ein Zins über 0 % gesetzt wird: Der Vertrag rundet die Schuld zugunsten des Protokolls auf, und der Zins läuft weiter. Die Schuld ist deshalb immer etwas höher als die geprägte Menge, zum Beispiel 1,00000003 statt 1 GHOST. Zum vollen Tilgen brauchst du also etwas mehr GHOST, als du geprägt hast. Für den Probelauf gibt es zwei Wege:
- **Zweiter Vault:** eröffnen und dort ein wenig mehr prägen, zum Beispiel 0,1 GHOST, dann zusammen tilgen.
- **Vault stehen lassen:** zum Beispiel mit 0,01 GHOST Restschuld. `withdraw --keep` gibt dir den größten Teil der KAS zurück.

## Was dabei nicht abgedeckt ist

- **Orakel:** Es gibt keinen dauerhaft laufenden Orakel-Dienst. Ohne Updates bleibt der letzte Preis stehen. `oracle-feed` kann das übernehmen, solange dein Rechner läuft.
- **Handlungen Dritter:** Liquidiert jemand anderes einen Vault, weiß die Zustandsdatei davon nichts. `status` warnt dann, ein Nachladen gibt es noch nicht. Tauschen andere im Pool, lädt `ghostctl` den Pool selbst nach; den GHOST-Betrag der Reserve liest es dafür über die öffentliche REST-API (api.kaspa.org) und prüft ihn am Node.
- **GHOST von anderen:** Schickt dir jemand GHOST von einem anderen Rechner, siehst du sie erst, wenn du auf der Seite „Wallet“ (oder mit `receive --ghost <Betrag>`) genau diesen Betrag suchst.
- **Wallets:** KasWare und Kastle zeigen GHOST (KCC20-Covenant-Token) nicht an. Den Bestand zeigt die Seite „Wallet“ bzw. `status`.
- **Lending-Seite:** Sie zeigt die Live-Daten dieses Deployments. Aktionen führt der lokale Server über `ghostctl` mit deinen Schlüsseldateien aus, siehe `app/README.md`.

## Tauschpool mit Kursband (ab 29.09.2026)

Neue Pools halten GHOST bei **1 USD ± 3 %**: Der Pool liest beim Tauschen das Orakel und lässt nur Tausche zu, nach denen GHOST im Band liegt oder sich darauf zubewegt. Reicht die Liquidität nicht, ist eine Richtung gesperrt, bis jemand einlegt oder zurücktauscht. Der Startkurs muss im Band liegen.

Umstieg vom ersten Pool (ohne Band):

```
./ghostctl --network mainnet pool-remove --key keys/mainnet-owner.json --percent 100
./ghostctl --network mainnet pool-open --key keys/mainnet-owner.json --kas <KAS> --ghost <GHOST>
```

Zu `<KAS>` passen `<KAS> × Orakelpreis` GHOST. 1 KAS und die dazu passenden GHOST des alten Pools bleiben dort für immer (gesperrte Mindestliquidität).

## Nachricht und Daueraufträge (ab 29.09.2026)

**Nachricht beim Senden.** `send` und `transfer` nehmen `--message "Miete Oktober"` (höchstens 100 Zeichen, eine Zeile).
Die Nachricht steht im lokalen Verlauf `deployments/<netz>-txlog.jsonl` und auf der Seite im Verlauf der Wallet. Mit
`--onchain-message` steht sie zusätzlich öffentlich und dauerhaft im Payload der Transaktion (UTF-8). Das ist gültig:
rusty-kaspa a41a333 kennt für normale Transaktionen keine Payload-Regel, zählt aber jedes Byte in die Masse (1 g Compute,
4 g transient); ghostctl rechnet das in die Gebühr ein. Gemessen im Simulator: 38 Byte kosten rund 4 000 sompi mehr
(`protocol/tests/payload_tests.rs`). Die Signatur deckt den Payload, niemand kann die Nachricht nachträglich ändern.

**Verschlüsselte Nachricht (Standard ohne `--onchain-message`, ab 29.09.2026).** Ohne `--onchain-message` schreibt
ghostctl die Nachricht verschlüsselt an den Empfänger in den Payload (`protocol/src/message.rs`):

```
"GHM\x01" (4) || E_x (32) || Nonce (12) || Ciphertext || Tag (16)   → 64 Byte Zusatz
```

- Schlüssel des Empfängers ist der x-only-Pubkey seiner Adresse `kaspa:q…` (bzw. der Schlüsseldatei). An P2SH- und
  ECDSA-Adressen geht das nicht; ghostctl bricht dann ab: „verschlüsselt nur an normale Kaspa-Adressen – Nachricht
  öffentlich oder weglassen“.
- Je Nachricht ein frischer Einmalschlüssel e; ECDH nur über die x-Koordinate (x(e·P) = x(sk·E), unabhängig vom
  Vorzeichen des Empfängerschlüssels), Schlüssel = BLAKE2b-256("GHOST-Nachricht v1" ‖ shared_x ‖ E_x ‖ P_x),
  ChaCha20-Poly1305 mit zufälligem Nonce und Magic ‖ E_x als Associated Data.
- **Verschlüsselt** ist nur der Text. **Öffentlich bleibt:** dass es eine Nachricht gibt, ihre Länge (Bytes des Textes
  + 64), Absender, Empfänger, Betrag und Zeit der Transaktion.
- Der Absender kann seine verschlüsselte Nachricht später nicht entschlüsseln (der Einmalschlüssel wird verworfen); er
  sieht sie nur im lokalen Verlauf `deployments/<netz>-txlog.jsonl` bzw. im Browser-Verlauf.
- Wer den geheimen Schlüssel des Empfängers erfährt, kann **alle alten Nachrichten** an ihn lesen (keine
  Vorwärtssicherheit). Schlüsseldateien also wie Geld behandeln.
- Kosten: 64 Byte mehr als öffentlich, gemessen 25 Zeichen: 223 545 statt 216 825 sompi; die längste Nachricht
  (100 × 4 Byte UTF-8 = 464 Byte Payload, `txb::MAX_PAYLOAD`) kostet 262 710 sompi, compute 2 502 g, transient 3 128 g.

**Empfangen:** `ghostctl --network mainnet messages --key keys/<name>.json` liest die letzten 200 Transaktionen der
Adresse über api.kaspa.org (Feld `payload`, Hex), entschlüsselt, was an diesen Schlüssel ging, und zeigt öffentliche
Nachrichten als „öffentlich“. Eigene Sendungen zählen nicht. Kein Node nötig, es wird nichts gesendet. GHOST-Eingänge:
Ein GHOST-Token liegt unter einer Skript-Adresse, die Adresse des Empfängers steht nicht in der Tx. Gefunden werden
daher nur Nachrichten zu GHOST-UTXOs, die die Zustandsdatei für diesen Schlüssel kennt (nach `receive` bzw. eigenem
`sync`); bereits weitergegebene GHOST tauchen dort nicht mehr auf.

**Daueraufträge.** KAS oder GHOST in festen Abständen senden, z. B. die Miete:

```bash
./ghostctl --network mainnet abo add --key keys/mainnet-owner.json --asset kas --to kaspa:q… \
  --amount 25 --interval monthly --start 2026-10-01 --count 12 --message "Miete"
./ghostctl --network mainnet abo list
./ghostctl --network mainnet abo pause <id>      # resume <id>, remove <id>
./ghostctl --network mainnet --dry-run abo run   # was wäre heute fällig?
```

- Nachricht wie beim Senden: ohne `--onchain-message` bei jeder Zahlung neu verschlüsselt (neuer Einmalschlüssel und
  Nonce), mit öffentlich. Aufträge von vor dem 29.09.2026 ohne Häkchen behalten ihr Verhalten (Nachricht nur lokal);
  `abo list` zeigt „verschlüsselt“, „öffentlich“ oder „nur lokal“.

- Intervall `daily`, `weekly`, `monthly` oder Anzahl Tage. Monatlich heißt gleicher Kalendertag; in kürzeren Monaten
  der letzte Tag (31.1. → 28.2. → 31.3.). Ende mit `--end JJJJ-MM-TT` oder `--count N`.
- Die Aufträge liegen in `deployments/<netz>-abos.json`, ohne Schlüssel (nur der Pfad der Schlüsseldatei). `remove`
  verschiebt ins Archiv der Datei, gelöscht wird nichts.
- Ausgeführt wird von `abo run`: vom GHOST-Agenten in jeder Runde und von der lokalen Seite jede Minute, solange eines
  davon auf diesem Rechner läuft. `abo run` sendet im Mainnet ohne Rückfrage nur mit `--ja`.
- **Nicht doppelt zahlen:** Ein Lauf sperrt die Datei, und der Termin wird vor dem Senden fortgeschrieben; die TXID steht
  vor dem Senden im Journal des Auftrags. Bricht ein Lauf ab, klärt der nächste am Node, ob die Zahlung angekommen ist,
  und sendet im Zweifel nicht erneut (Eintrag „Ausgang unklar“ im Verlauf).
- **Verpasste Termine** (Rechner war aus) werden genau einmal nachgeholt; die übrigen stehen als „übersprungen“ im Verlauf.
- **Fehler** ohne Senden (z. B. zu wenig KAS): neuer Versuch nach 10 bzw. 20 Minuten, nach 3 Versuchen wird der Auftrag
  pausiert. Fortsetzen holt Termine aus der Pause nicht nach.


## Dauerauftrag mit Tresor (ab 29.09.2026)

Ein Dauerauftrag „vom Rechner“ zahlt nur, solange hier der GHOST-Agent oder die Seite läuft. Mit Tresor liegt das Geld
vorab in einem Vertrag (`contracts/standing_order.sil`) und wird auch gezahlt, wenn dieser Rechner aus ist: Zum Termin
löst der Empfänger oder irgendein laufender GHOST-Agent die Zahlung aus. Der Vertrag lässt nur genau den Betrag an genau
den Empfänger zu, frühestens am Termin und höchstens einmal je Termin.

```bash
# anlegen: 25 KAS monatlich am 1., 12 Zahlungen; Startguthaben-Vorschlag 12 × (25 + 0,01) + 1 = 301,12 KAS
./ghostctl --network mainnet tresor open --key keys/mainnet-owner.json --to kaspa:q… \
  --amount 25 --interval monthly --start 2026-11-01 --count 12 --message "Miete"
./ghostctl --network mainnet tresor list                  # ohne Netz
./ghostctl --network mainnet tresor code <id>             # Tresor-Code für den Empfänger
./ghostctl --network mainnet tresor import 'ghost-tresor:2:…'   # beim Empfänger; sendet nichts
./ghostctl --network mainnet tresor pay [<id>]            # fällige Zahlung auslösen (jeder darf)
./ghostctl --network mainnet tresor topup <id> --key … --kas 50
./ghostctl --network mainnet tresor cancel <id> --key …   # Rest zurück, Tresor endet
./ghostctl --network mainnet tresor sync                  # Stand am Node abgleichen
```

- **Termine in UTC:** fällig um 00:00 UTC des gewählten Tages (`--time HH:MM` für eine andere Uhrzeit), monatlich am
  selben Kalendertag, in kürzeren Monaten am Monatsletzten. Auszahlen lässt der Konsens erst, wenn die Past Median Time
  den Termin überschritten hat – sie läuft der Uhr etwa 2¼ Minuten hinterher.
- **Beträge:** mindestens 1 KAS je Zahlung, nach jeder Zahlung bleibt mindestens 1 KAS im Tresor (Speichermasse,
  KIP-9). Höchstgebühr je Zahlung 0,01 KAS aus dem Tresor; gemessen kostet eine Zahlung 0,0025 KAS, mit 100 Zeichen
  öffentlicher Nachricht 0,0033 KAS. Anlegen kostet rund 0,002 KAS, Kündigen 0,003 KAS, Auffüllen 0,004 KAS.
- **Startguthaben:** Vorschlag Anzahl × (Betrag + 0,01) + 1 KAS; bei „unbegrenzt“ frei (`--fund`). Das Guthaben ist im
  Tresor gebunden. Nach der letzten Zahlung bleibt der Rest stehen, bis du kündigst.
- **Auslöser braucht keine eigenen KAS:** Die Gebühr kommt aus dem Tresor. Nur wenn danach weniger als 1 KAS übrig
  bliebe, zahlt der Auslöser die Gebühr mit `--key` selbst. Verpasste Termine lassen sich einzeln nachzahlen (bei Miete ist
  jeder Monat geschuldet).
- **Automatisch:** Der GHOST-Agent löst in jeder Runde alle fälligen Zahlungen der bekannten Tresore aus (eigene und
  importierte), die Seite jede Minute. Doppelt auslösen ist harmlos: die zweite Tx findet die UTXO nicht mehr.
- **Nachricht fest im Vertrag:** Die Nachricht (`--message`, verschlüsselt an den Empfänger oder mit
  `--onchain-message` öffentlich) wird beim Anlegen über ihren Hash im Vertrag gebunden. Jede Zahlung trägt genau sie;
  wer auslöst, kann sie weder weglassen noch ändern. Auch nachträglich lässt sie sich nicht ändern – dafür kündigen und
  einen neuen Tresor anlegen. Ohne Nachricht tragen alle Zahlungen keinen Text.
- **Tresor-Code:** Der Empfänger braucht ihn einmal (Seite: „Tresor-Code kopieren“ bzw. „Tresor-Code einfügen“).
  ghostctl prüft ihn am Node (Skript-Hash und Covenant-ID); ein falscher Code findet keine UTXO. Der Code enthält keine
  Schlüssel, aber die Beschreibung – nur an den Empfänger weitergeben. Codes beginnen mit `ghost-tresor:2:`; alte Codes
  `ghost-tresor:1:` (Vertrag ohne gebundene Nachricht) nimmt ghostctl nicht mehr an.
- Die Tresore liegen in `deployments/<netz>-tresore.json` (Sperre `…-tresore.lock`, Journal `…-tresore.pending.json`),
  unabhängig vom GHOST-Deployment.

## Notfallsatz (ab 06.10.2026, nach Audit 20)

Live gibt es einen einzigen Unterzeichner (`keys/mainnet-signer.json`) und bis jetzt keinen Notfallsatz. Geht dieser Schlüssel verloren, friert das Orakel nach 2 h ein und bleibt es: Prägen, Rücknahme, Liquidation und Pool-Tausch wären für immer gesperrt (A20e-2). Abhilfe ohne neues Deployment: ein zweiter, kalt gelagerter Schlüssel als Notfallsatz.

**Doppelklick `GHOST-Notfallsatz.command`** (auf dem Mac, nicht auf dem Server). Es erkennt selbst, was dran ist:
1. Notfall-Schlüssel erzeugen (`ghostctl committee-keygen <Ordner>/ghost-notfall-<Datum>.json --count 1`). Standard ist ein Ordner `GHOST-Notfall` auf dem ersten eingesteckten USB-Stick; ein Ordner im Projekt wird abgelehnt. Danach steht die Anleitung zur Offline-Sicherung im Fenster (zweiter Stick, getrennt lagern, optional auf Papier, nie Cloud). Nur der öffentliche Schlüssel bleibt auf dem Mac (`deployments/mainnet-notfall.pub`).
2. Ankündigung: gleicher Hauptsatz (bisheriger Unterzeichner, unverändert) + neuer Notfallsatz 1 von 1. Signiert mit `keys/mainnet-signer.json`, Gebühr und 1 KAS Ticket zahlt `keys/mainnet-owner.json`. Erst ein Probelauf, gesendet wird nur nach Eingabe von `ja`.
3. Nach 14 Tagen erneut doppelklicken: Das Skript bietet `signers activate` an (Probelauf, dann `ja`). Die 1 KAS des Tickets kommt zurück.

Der Agent auf dem Server kennt die Ankündigung nicht und meldet sie bis zur Aktivierung als „von außen angekündigt“. Das ist erwartet. Seine Preis-Updates laufen weiter. Probelauf ohne Senden: `DRY=1 ./GHOST-Notfallsatz.command`.

**Wann der Notfallweg greift:** nur nach 30 Tagen ohne Preis des Hauptsatzes **und** bei eingefrorenem Orakel. Ablauf im Ernstfall: `./ghostctl oracle-freeze --key <Zahler>` (darf jeder, sobald der Preis älter als 2 h ist), dann `./ghostctl signers propose --emergency --key <Zahler> --committee <Notfall-Datei vom Stick> --new-keys <neuer Unterzeichner>`, nach weiteren 14 Tagen `signers activate`. `ghostctl` lehnt `--emergency` ab, solange das Orakel nicht eingefroren ist: Sonst könnte jeder die öffentliche Signatur des letzten Preis-Updates wiederholen und die Notfall-Ankündigung damit entwerten (A20a-1). Ist das Orakel eingefroren, taut es nur ein echtes Update des Hauptsatzes auf – dann lebt der Hauptsatz, und der Notfall ist zu Recht vorbei.
