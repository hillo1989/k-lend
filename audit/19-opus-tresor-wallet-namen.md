# Audit 19: Tresor mit Browser-Wallet, öffentliche GHOST-Suche, .k-Namen

Prüfer: Claude Opus 5.5, unabhängig, nur lesend. Datum: 06.10.2026.
Stand: `main` bei 3618df1 (Merge von 1cd30c9), dazu 4161a8d (receive --owner) und 8967095 (.k-Namen).
Es wurde nichts gesendet und kein Server angefasst. keys/ wurde nicht gelesen. Der Code wurde nicht geändert. Einziger Eingriff war eine eigene Testdatei `protocol/tests/audit19_pruef.rs`. Sie wurde nach dem Lauf wieder entfernt, der Inhalt steht im Anhang.

Geprüft: `protocol/src/wallet_ops.rs` (Basis/TresorBasis, run_tresor, own_tresor, submit/finish), `tresor.rs` (topup/cancel mit Signer, build_exact_fee, key_may_pay, make_room_for_wallet, follow/locate_io, pay_round), `txb.rs` (wallet_fill_fee), `ghostctl.rs` (wallet_tresor_action_of, tresor_basis, wallet_tresor_build/submit, send_wallet, tresor owned, tresor_agent_step, Receive, Utxos, Sperre in `run`), `store.rs` (lock, resolve_pending, send_then_wait), `app/server/{api,walletActions,dotkNames}.ts`, `app/src/lib/{tresorWallet,kname}.ts`, `app/src/components/{WalletTresor,ActionForms,WalletSignFlow}.tsx`, `docs/wallet-aktionen.md`, `@dotk/sdk` 2.1.0 (node_modules, nur gelesen).

Testläufe: `cargo test --release --test wallet_ops_tests tresor` → 6/6 ok. `npx vitest run server/dotkNames.test.ts server/walletTresor.test.ts src/lib/tresorWallet.test.ts` → 24/24 ok. Eigener Prüftest `audit19_pruef` → 2/2 ok (Ausgaben unten).

## Kurzfassung

| # | Schwere | Befund |
|---|---|---|
| A19-1 | **hoch** | `wallet submit` übernimmt `first_due` aus dem Plan des Browsers ungeprüft. Die Regel „erster Termin nicht in der Vergangenheit“ steht nur im build-Pfad von ghostctl. Ein selbst gebauter Plan mit Termin im Jahr 2000 und täglichem Intervall wird angenommen. Folge: Rückstand von etwa 2 000 Terminen, der Agent zahlt in **jeder** Runde nach. Im Test belegt. |
| A19-2 | **mittel** | `tresor_basis` führt den Tresor am Node nach (`follow`, `Search::Full`), **bevor** der Besitzer geprüft wird. Jeder Besucher kann so für jede bekannte Tresor-ID bis zu 2 000 Node-Abfragen je Anfrage auslösen. Beim Agenten wiederholt sich die Suche jede Runde, wenn das Zeitlimit greift. Zahl der Abfragen im Test belegt, Laufzeit am Server vermutet. |
| A19-3 | **mittel** | Die 1 000 Plätze der Tresor-Datei lassen sich mit 50 Adressen für etwa 2 004 KAS (zurückholbar) plus etwa 2,2 KAS Gebühr belegen. Danach meldet die Seite allen anderen „kein Platz“. Belegt sind die Plätze durch Tresore, die nie fällig werden (Termin 2199). Kosten im Test gemessen. |
| A19-4 | **mittel** | `ghostctl utxos` (jede .k-Namensprüfung) und `receive` nehmen die **Haupt-Sperre** `deployments/mainnet.lock` (bis 120 s Wartezeit) und belegen den `run`-Pool, den auch status, keys und price nutzen. Namensanfragen konkurrieren so mit Orakel und Keeper (60 s) und mit Wallet-Senden (30 s). Im Code belegt. |
| A19-5 | niedrig (Funktion) | .k-Name im Empfängerfeld: Die Neuprüfung alle 60 s setzt die Parameter kurz auf `null`. WalletSignFlow verwirft dann Plan und geprüfte Signatur. Ein Ablauf von über 60 s (Plan → Wallet → Prüfen → Senden) bricht ab. Jede Prüfung zählt außerdem gegen das Wallet-Kontingent von 20 je Minute. |
| A19-6 | niedrig | Das Journal einer Wallet-Tresor-Tx klärt nur der nächste Wallet-Tresor-Submit oder eine Agent-Runde, wenn etwas fällig ist. Bis dahin zeigen `tresor owned` und build den alten Stand. Der neue Tresor fehlt in „Meine Tresore“, und es droht doppeltes Anlegen bzw. Auffüllen. |
| A19-7 | niedrig | Der Tresor-Schritt des Agenten hält die Sperre der Tresor-Datei, während er auf Bestätigungen wartet (`send_to`, bis 600 s je Zahlung). Wallet-Tresor-Sendungen warten nur 30 s und scheitern in dieser Zeit mit „Sperre“. A19-1 verlängert das. |
| A19-8 | niedrig (Text/Funktion) | „Zum Termin löst der Agent die Zahlung aus“ ist nicht gesichert (A19-1/A19-2). Für Nutzer westlich von UTC lehnt der Server abends das voreingestellte Startdatum „heute (Ortszeit)“ ab, die Seite hat es zuvor zugelassen. |
| A19-9 | Hinweis | Die Kurz-ID (8 Hex) lässt sich mit etwa 2³² Versuchen offline treffen. `TresorFile::find` meldet dann für Betreiber-Befehle mit Kurz-ID „mehrdeutig“. Kein Geldrisiko. |

**Keine** Befunde gibt es bei:
- Gebühren des Agenten bei Wallet-Tresoren (`key_may_pay`)
- Fremde Tresore kündigen, auffüllen oder umleiten
- Gefälschte Einträge in der Tresor-Datei (nur über Neubau und Journal)
- Messkopie bei Tresoren
- `wallet_fill_fee` beim Kündigen
- Schädliche Einträge über `receive` (nur echte Covenant-UTXOs)
- Argument-Injektion über SDK-Adressen
- Beweis der .k-Adresse (Urkunde mit Register-Covenant-ID, aktiver Zustand)
- Unternamen, Covenant-Besitzer, Groß-/Kleinschreibung, Unicode

---

## Frage 1: Tresor per Browser-Wallet

### A19-1 (hoch): `first_due` aus dem Browser wird in submit nicht geprüft

**Ort:** `protocol/src/wallet_ops.rs:1029–1035` (Neubau aus `plan.action`), `run_tresor` (wallet_ops.rs:532ff., prüft nur `Action::check` und `tresor::check_params`), `ghostctl.rs:4551` (Prüfung `first < pmt - DAY_MS` nur in `wallet_tresor_action_of`, also nur bei `wallet build`), `wallet_tresor_submit` (ghostctl.rs:4633ff.).

**Szenario:**
1. Der Angreifer baut sich den Plan selbst. Er braucht dazu `build_plan` aus dem Quellcode oder einen Nachbau, weil die Tx bei TresorOpen nur von Aktion und Funding-UTXOs abhängt.
2. Der Plan enthält `TresorOpen { first_due: 2000-01-01, period_ms: 1 Tag, count: -1, amount: 1 KAS, max_fee: 0,004 }`.
3. Er signiert mit der eigenen Wallet und schickt ihn an `POST /api/wallet/submit` mit `send`.

`check_params` erlaubt Termine ab 1985 (tresor.rs:149). `MAX_IMPORT_BACKLOG` (400) gilt nur für den Import (tresor.rs:1369). Der Neubau ist bitgleich, weil beide Seiten dieselbe Aktion benutzen. Die Doku sagt: „Nichts aus dem Plan des Browsers geht in die Datei ein außer über den bitgleichen Neubau“. Das stimmt, aber die Aktion selbst ist Browser-Eingabe, und die build-Regeln fehlen dort.

**Folgen:**
- `pay_round` zahlt je Tresor und Runde einen Termin (tresor.rs:1445ff.). Ein solcher Tresor ist bei Agent-Intervall 300 s **in jeder Runde fällig**, also etwa 288 Zahlungen am Tag. Die Gebühr kommt aus dem Tresor des Angreifers (etwa 0,002 KAS je Zahlung, `wallet = true`). Der Betrag geht an seine zweite Adresse. Ihn kostet das fast nichts.
- Der Tresor-Schritt wird 90 s nach der ersten Sendung abgebrochen (`Takt::agent`, ghostctl.rs:2459, `side_step`). `pay_round` geht nach Dateireihenfolge vor, und neue Einträge kommen ans Ende (`upsert`). Ein paar Dutzend solcher Tresore verbrauchen deshalb die Sendezeit jeder Runde. **Tresore, die später angelegt werden, kommen nicht mehr an die Reihe.** Das ist aus dem Code abgeleitet und hängt von der Bestätigungszeit ab. Orakel und Keeper sind nicht betroffen, sie gehen vor (A12-5).
- Grundlage für A19-2: Jede Suche nach einem solchen Tresor kostet bis zu MAX_FOLLOW = 2 000 Kandidaten.

**Beleg (Test `a19_1_vergangener_termin_ueber_submit`):**
```
A19-1 Rückstand laut tresor::backlog: 1999 (MAX_IMPORT_BACKLOG = 400)
A19-1 Zahlungen in 5 Agent-Runden: 5, Termin jetzt 2000-01-06 00:00 UTC
```
`wo::submit` meldet `report.valid`, und der Simulator nimmt Anlegen und alle 5 Rückstandszahlungen an.

**Vorschlag:** Die build-Regeln in `run_tresor` bzw. `Action::check` übernehmen. `TresorBasis` sollte dafür die Past Median Time bekommen (`now_ms`/`pmt`), wie `now` für den Verlauf. Dann `first_due >= pmt - DAY_MS` und eine Obergrenze für den ersten Termin prüfen, zum Beispiel höchstens 1–2 Jahre voraus (siehe A19-3). Grundsätzlich alle Regeln des build-Pfads (ghostctl `wallet_tresor_action_of`) in die Bibliothek verlegen, damit submit sie mitprüft. Dazu ein Regressionstest wie im Anhang: submit eines Plans mit vergangenem Termin muss scheitern.

### A19-2 (mittel): Nachführen vor der Besitzerprüfung, ungebremste Suche

**Ort:** `ghostctl.rs:4586ff.` (`tresor_basis`): Für TresorTopup und TresorCancel läuft `tresor::follow(…, Search::Full)` für **jeden** Eintrag mit dieser Covenant-ID. Erst danach prüft `own_tresor` (wallet_ops.rs:511) den Besitzer. Das geschieht bei `wallet build` ohne Sperre und bei `wallet submit` zweimal, einmal davon unter der Sperre der Tresor-Datei. `Search::Full` ignoriert `missing` und sucht immer bis MAX_FOLLOW (tresor.rs:1327). Gespeichert wird nichts, jede Anfrage beginnt also von vorn.

**Szenario:**
1. Der Angreifer legt einen Tresor nach A19-1 an.
2. Er kündigt ihn am Server vorbei. Das erlaubt der Vertrag dem Besitzer, er bekommt sein Geld zurück.
3. Danach schickt er wiederholt `POST /api/wallet/build {action: "tresor-cancel", params: {tresor: <ID>}}` mit irgendeiner gültigen Adresse.

Jede Anfrage startet ghostctl mit 2 000 `getUtxosByAddresses` nacheinander und belegt dabei einen der 2 `runWallet`-Plätze. Ohne A19-1 ist die Suche durch den tatsächlichen Rückstand begrenzt, bei einem alten, leer gelaufenen Tresor sind das aber auch Hunderte. Die ID ist öffentlich (`tresor owned`, Kette).

**Agent:** In `pay_round` (Search::Auto, `missing` noch leer) sucht er beim ersten Mal ebenfalls bis zu 2 000 Kandidaten je verschwundenem Tresor. Ergebnisse speichert `pay_round` nur vor einer Sendung oder am Ende (tresor.rs:1508, 1545). Bricht das Zeitlimit den Schritt vorher ab (prep 240 s, send 90 s, außen 700 s), geht die Markierung `missing` verloren, und die nächste Runde sucht wieder alles ab. Bei z. B. 3 ms je Abfrage reichen dafür rechnerisch etwa 40 solcher Tresore. Sie kosten nur Gebühren, weil das Guthaben nach dem Kündigen zurück ist. Ob das am Server tatsächlich so lange dauert, ist **vermutet**, Latenz nicht gemessen.

**Beleg (Test, Zählung der Node-Abfragen):**
```
A19-1 Node-Abfragen einer Suche (Full): 2000
A19-1 zweite Suche (Full, missing gesetzt): 2000
```

**Vorschlag:**
- In `tresor_basis` zuerst den Besitzer gegen die Adresse aus Plan bzw. Kommando prüfen (`own_tresor` vorziehen), erst dann nachführen.
- Für den öffentlichen Weg die Suche begrenzen (z. B. `MISSING_RECHECK_FOLLOW`) oder einen schon als `missing` markierten Tresor gar nicht erst durchsuchen.
- In `pay_round` nach jeder teuren Suche (`note.is_some()`) speichern.
- Mit A19-1 entfällt der große Rückstand ohnehin.

### A19-3 (mittel): Datei-Plätze billig dauerhaft belegbar

**Ort:** `tresor.rs:1229ff.` (`make_room_for_wallet`). Verdrängt werden nur **gekündigte** Wallet-Tresore. Laufende, auch nie fällige oder unbezahlbare, zählen weiter.

**Szenario:** Der Angreifer legt je Adresse 20 Tresore an, jeweils mit Mindestguthaben (Betrag 1 + Höchstgebühr 0,004 + 1 KAS Rest) und erstem Termin 2199. Das ist schon über `wallet build` möglich, weil `--start` nach oben nicht begrenzt ist. 50 Adressen ergeben 1 000 Einträge. Danach bekommt jeder neue Nutzer „Auf diesem Server ist gerade kein Platz für weitere Tresore“. Der Agent hat keine Kosten, weil `looks_due` falsch ist. Der Angreifer holt das Geld später per Kündigen zurück.

**Beleg (Test `a19_2_platz_belegen_kosten`):**
```
A19-2 Platz: Startguthaben 2.00400 KAS + Gebühr 0.00219 KAS; 1000 Plätze ≈ 2004 KAS gebunden (zurückholbar)
```

**Vorschlag:** Den ersten Termin nach oben begrenzen (A19-1). Bei voller Datei auch Wallet-Tresore verdrängen, die `missing` sind, leer gelaufen sind (`!payable`, älter als N Tage) oder `left == 0` haben. Die Grenze nicht nur je Besitzer setzen, sondern zusätzlich als Mindestguthaben bzw. Mindestanteil fälliger Zahlungen. Oder eine eigene, größere Obergrenze für Wallet-Einträge mit Warnung an den Betreiber.

### Ohne Befund (Tresor)

- **(a) Geld bzw. Gebühren des Agenten:** `TresorRec::wallet` wird nur in `run_tresor` (TresorOpen) gesetzt und kommt ausschließlich über den Neubau und das Journal in die Datei. In `pay_round` gilt `let with_key = r.key_may_pay(with_key)` (tresor.rs:1464), und auch die Offline-Schätzung `looks_due` beachtet es. Der Test `wallet_tresor_agent_zahlt_keine_gebuehr_dazu` läuft grün: Der Agent zahlt bei Wallet-Tresoren nie aus dem eigenen Schlüssel. Der Text „Gebühr nur aus dem Tresor (höchstens Höchstgebühr)“ stimmt.
- **(b) Fremde Tresore:** `own_tresor` prüft den Besitzer früh. Endgültig verlangt der Vertrag die Besitzersignatur an Position 0 (`topUp`/`cancel`), und `check_scripts` läuft vor dem Senden. Der Plan für eine fremde Adresse scheitert schon in der Vorprüfung, weil die Signatur nicht vom Plan-Schlüssel stammt. Test `fremder_kann_tresor_weder_kuendigen_noch_auffuellen` ist grün. Umleiten ist unmöglich, denn der Empfänger steht im Skript.
- **(c) Gefälschte Einträge:** Der Folgezustand stammt aus dem Neubau unter Sperre. Das Journal schreibt ihn erst bei Annahme (`send_wallet` → `write_pending(..., wallet = true)`, ghostctl.rs:946). Alle Schreiber der Tresor-Datei (Agent `tresor_cmd`, Wallet-Submit) lösen das Journal vor dem Schreiben unter Sperre auf. Die Offline-Befehle (`list`, `code`, `owned`, `pay` ohne Fälliges) schreiben nicht. Der Schnappschuss im Journal ist damit beim Übernehmen nicht veraltet. Die Grenzen dazu stehen in A19-6.
- **Bitgleicher Neubau:** `plan = fresh.plan` (wallet_ops.rs:1035). Gebühr, `paid` (für Kündigen), Budgets, Eingänge und Signierer kommen alle vom Server, aus dem Browser kommt nur die **Aktion**. Die Lücke darin ist A19-1. `finish` verwendet `paid: plan.fee`, also die Neubau-Gebühr, nicht die des Browsers.
- **Messkopie:** `TresorBasis::mirror` ersetzt den Besitzer des Nutzers und neutralisiert Vorbesitz des Ersatzschlüssels. Der Ersatzschlüssel ist inzwischen ohnehin je Aufruf zufällig (wallet.rs:313). Das Kündigen baut mit Wallet genau einmal mit der Gebühr der Messkopie (`wallet_fill_fee`). Andere Aufrufer von `build_exact_fee` (wallet.rs:543/1007) laufen nicht in einem `with_wallet_fill`. Tests `tresor_messkopie_…` und `tresor_anlegen_…` sind grün.
- **Kündigen ohne eigene KAS:** `needs_funding` ist false. Ein Plan mit Funding wird abgelehnt (wallet_ops.rs:888–890).
- **Endlosschleifen:** Keine. `pay_round` zahlt höchstens einen Termin je Tresor und Runde, `candidates_max` ist auf 2 000 begrenzt. Die Kosten beschreiben A19-1 und A19-2.
- **Oberflächentexte:** „Nur du kannst auffüllen oder kündigen“, „Rest zurück an deine Adresse“, „Gebühr aus dem Tresor, höchstens 0,01 KAS“ und „1 KAS Rest“ stimmen mit dem Code überein. Die Seite schickt keine `maxFee`, also gilt die Voreinstellung 0,01. Einschränkungen in A19-8.

### A19-6 (niedrig): Journal der Tresor-Datei bleibt liegen, Liste veraltet

**Ort:** Den Agenten startet `tresor_agent_step` (ghostctl.rs:4686) nur, wenn `tresor::needs_run` offline etwas Fälliges sieht. `tresor owned` und `wallet build` (`tresor_basis`) lesen die Datei, ohne das Journal zu klären.

**Szenario:** Die Wallet-Tx wird gesendet, aber vor der Übernahme ins Journal endet ghostctl, etwa durch das Zeitlimit von 170 s, einen Neustart oder `MemoryMax`, oder `wait_accepted` (45 s) läuft ab. Die Seite meldet dann „Bestätigung steht aus“. Der Eintrag kommt erst mit dem nächsten Wallet-Tresor-Submit oder mit der nächsten fälligen Zahlung irgendeines Tresors in die Datei. Bis dahin fehlt der neue Tresor in „Meine Tresore“ (Abfrage jede Minute), und der Nutzer legt womöglich einen zweiten an. Das Geld bleibt dabei im eigenen Tresor, es ist also kein Verlust, aber gebunden. Ein angelegter, aber nie übernommener Tresor wäre über die Seite weder sichtbar noch kündbar.

**Vorschlag:** `tresor owned` gibt ein offenes Journal mit aus (`pending: true`, Aktion, txid), und die Seite zeigt es an. Der Agent klärt ein offenes Journal der Tresor-Datei in jeder Runde, auch wenn nichts fällig ist (Journal vorhanden → verbinden).

### A19-7 (niedrig): Sperre der Tresor-Datei beim Warten auf Bestätigung

**Ort:** `tresor_agent_step` hält `_lock` während `tresor_cmd` → `pay_round` → `TresorNet::send` → `send_to`, darin `wait_accepted` 120–600 s (ghostctl.rs:912). Beim GHOST-Zustand gibt A17-6 die Sperre vor dem Warten frei, hier nicht. Die Wallet-Tresor-Sendung wartet 30 s (`WALLET_LOCK_WAIT`) und scheitert dann mit „Eine andere ghostctl-Instanz arbeitet gerade“.

**Vorschlag:** Wie bei A17-6 nach dem Senden die Sperre freigeben (`send_then_wait`), oder dem Nutzer die Meldung verständlich geben („Zahlungsrunde läuft, gleich erneut“).

### A19-8 (niedrig): Texte und Zeitzonen

- `walletTresorBasics` verspricht: „Zum Termin löst der K.Lend-Agent die Zahlung aus.“ Nach A19-1/A19-2 kann sich das beliebig verzögern. Ehrlicher wäre „in der Regel innerhalb weniger Minuten nach dem Termin; zahlen darf auch jeder andere, z. B. der Empfänger“. Dafür bräuchte es einen öffentlichen Weg, wenigstens den Tresor-Code. Im öffentlichen Modus gibt es keine Minutentakt-Auslösung der Seite (`aboTimer`, api.ts:785: `|| isPublic`), nur den Agenten.
- Startdatum: Die Seite vergleicht mit dem Datum in Ortszeit (`localToday`), der Server setzt `start` als UTC-Datum 00:00 und lehnt `first < pmt − 1 Tag` ab. In UTC−5 bis −10 ist abends (UTC schon nächster Tag) „heute“ bis zu 34 h alt. Die Seite lässt das zu, der Server lehnt mit „liegt in der Vergangenheit“ ab. Östlich von UTC ist der erste Termin bei „heute“ schon vorbei, und die erste Zahlung kommt sofort. Das ist so gewollt, sollte aber dastehen. Aus dem Code abgeleitet, nicht im Browser getestet.

### A19-9 (Hinweis): Kurz-ID ergrindbar

`id_of` nimmt die ersten 8 Hex der Covenant-ID (tresor.rs:1155). Die Covenant-ID hängt vom ersten Funding-Outpoint ab, mit etwa 2³² Hashes lässt sich offline eine passende finden. `TresorFile::find` (Kurz-ID, Betreiber-Befehle `tresor pay|cancel|code <id>`) meldet dann „mehrdeutig“. Der öffentliche Weg nutzt die volle ID, und `upsert` arbeitet über die volle ID. Kein Geldrisiko, nur Bedienung.

## Frage 2: Öffentliche GHOST-Suche (`receive --owner`)

**Kein schädlicher Eintrag möglich.** `--owner` geht nur über `ghost_target` (Schnorr-Adresse oder 64 Hex, nie ein Pfad). Übernommen werden nur UTXOs unter genau dem Skript `GhostTok::to_pubkey(owner, betrag)` **und** mit `covenant_id == ghost_cov` (ghostctl.rs:1791). Covenant-IDs setzt der Konsens durch (KIP-20). Solche UTXOs kann also nur der GHOST-Vertrag erzeugen, und sie sind echte Token. Bekannte Outpoints werden nicht doppelt eingetragen. Geschrieben wird unter der Sperre nach `load_synced`, das vorher das Journal auflöst. Der zufällige Ersatzschlüssel macht A17-1 hier gegenstandslos.

Was bleibt, steht in A19-4: Sperre, Pool und vollständiger Abgleich je Aufruf. `busy` serialisiert receive global. Bei zwei gleichzeitigen Nutzern bekommt einer 409, im öffentlichen Modus sonst ohne Wirkung.

### A19-4 (mittel): Namensprüfung und Suche unter der Haupt-Sperre, im gemeinsamen Pool

**Ort:**
- `ghostctl.rs:1247–1250`: Außer `OracleFeed` und `Agent` nimmt **jeder** Befehl dieses Pfads `store::lock(deployments/<netz>.json, 120 s)`, auch `Utxos` (rein lesend) und `Receive`.
- `api.ts:568ff.` / `588ff.`: Beide nutzen `run`, also 2 Plätze und eine Warteschlange von 20. Diesen Pool teilen sie mit `statusCache`, `keysCache`, `priceCache` und den Nachrichten (api.ts:311ff.).
- `receive` macht dazu `load_synced`, also Journal plus `resync` aller Vaults und Token, und das bei jedem Aufruf.

**Szenario:** Mehrere Adressen, mit IPv6 je /64 billig (A17-3), fragen im Kontingent (20/min) registrierte .k-Namen ab. Jede Anfrage startet ghostctl, das die Haupt-Sperre nimmt. Die Folgen:
- Der Agent wartet 60 s auf dieselbe Sperre (ghostctl.rs:1567/1622). Das Abfragen alle 300 ms ohne Reihenfolge macht seine Chance zufällig. Ein dauerhafter Ausfall ist unwahrscheinlich, Verzögerungen sind wahrscheinlich.
- Wallet-Sendungen warten nur 30 s und scheitern eher.
- Ist der `run`-Pool voll, bekommt status `BUSY_RESULT`. Der Fehler wird wie ein Erfolg zwischengespeichert.

Zusätzlich geht je Namensprüfung eine Anfrage an api.dotk.name, ohne Zwischenspeicher.

**Beleg:** Codestellen oben. Laufzeitverhalten nicht gemessen (Server nicht angefasst).

**Vorschlag:**
- `Cmd::Utxos` (und `Price`, `Status --json` ohne Schreiben) ohne Sperre ausführen.
- `receive` nur zum Schreiben sperren: erst ohne Sperre suchen, dann kurz sperren und eintragen.
- Namens- und Suchrouten auf `runWallet` oder einen eigenen Pool legen.
- Eigene, engere Ratenbegrenzung und einen kurzen Zwischenspeicher je Name (z. B. 30 s) einführen.

## Frage 3: .k-Namen (dotk.name)

**Kein Befund beim Beweis:**
- `resolveName` gibt eine Adresse nur heraus, wenn `proven === true` ist. Das heißt: Die SDK leitet die Urkunden-Adresse selbst aus Name, Besitzer-Art, Besitzer und **aktivem** Urkunden-Zustand ab (`encodeActiveDeedState`), und der eigene Node meldet dort eine UTXO mit der Covenant-ID des Registers (`attribute`, node.js).
- Eine lügende oder veraltete API liefert keine Adresse (RefutedError, Test grün). Unternamen sind abgelehnt. Covenant-Besitzer (`ownerType 4`) liefern keine Adresse. Die Adresse ist die des Urkunden-Besitzers, nicht ein Eintrag der Karte (Records).
- Schreibweise: Die SDK normalisiert mit ASCII-Kleinbuchstaben (`asciiLowercase`), erlaubt sind nur `[a-z0-9-]`. `isKName` lässt nur ASCII durch. Unicode-Homoglyphen sind damit ausgeschlossen. Verwechselbare ASCII-Namen (`rn`/`m`, `l`/`1`) gibt es grundsätzlich. Die Seite zeigt deshalb Kanon-Name und Adresse und bittet um Vergleich. Das ist angemessen.
- Besitzer mit P2SH- oder ECDSA-Adresse liefern `kaspa:p…` bzw. `kaspa:qyp…`. Für KAS-Senden ist das richtig. Bei GHOST, Tresor oder x-only lehnt `ghost_target` bzw. die Seite das ab. Das ist sicher, aber die Meldung ist eher technisch.

**Keine Argument-Injektion:** Die Adressen an `ghostctl utxos` stammen aus `encodeAddress` der SDK, also bech32 mit Netzpräfix. Name und Besitzer werden vorher geprüft (Zeichensatz bzw. hex32). Sie können nicht mit `-` beginnen. Je Wert gibt es ein eigenes `--address`, höchstens 50, `execFile` ohne Shell. ghostctl prüft Netz und Format noch einmal (ghostctl.rs:1812ff.).

**Race zwischen Auflösung und Bau:** Der Bau verwendet `kname.address` zum Zeitpunkt des Klicks. Der Plan zeigt die Empfängeradresse, die beim Signieren gilt. Wechselt der Name danach den Besitzer, zahlt man an den bisherigen, bewiesenen Besitzer. Das Fenster ist kürzer als 60 s (siehe A19-5) bzw. die Plan-Gültigkeit. Das ist systembedingt und vertretbar.

### A19-5 (niedrig, Funktion): Neuprüfung verwirft den laufenden Ablauf

**Ort:** `kname.ts` (`setState({...s, loading: true})` bei jeder 60-s-Neuprüfung), `ActionForms.tsx:183` (`kname.loading` → `params: null`), `WalletSignFlow.tsx:110–113` (`sig` ändert sich → `setPlan(null)`, `setChecked(null)`).

**Szenario:** Ein Nutzer gibt `alice.k` ein, holt den Plan, signiert in KasWare, prüft und will senden. Nach spätestens 60 s ab der letzten Auflösung sind Plan und geprüfte Signatur weg, auch wenn die Adresse gleich bleibt. Endet der Wallet-Dialog nach dem Zurücksetzen, ist die Signatur verloren, und der Nutzer muss neu beginnen.

Dazu kommen: Tippen mit 400 ms Entprellung, die Neuprüfung je Minute und die Abfrage „Meine Tresore“ je Minute. Alle zählen gegen dasselbe Wallet-Kontingent von 20 je Minute wie build und submit. Beim Eintippen einer Adresse ohne Präfix ist der Anfang (z. B. `kaspa`) ein gültiger Name und löst eine Suche aus.

**Beleg:** Codepfad gelesen. Nicht im Browser nachgestellt.

**Vorschlag:** Während einer Neuprüfung die alte Adresse behalten (`loading` nur anzeigen, `params` nicht auf `null` setzen). Nur bei **geänderter** Adresse den Plan verwerfen. Die Namensroute vom Wallet-Kontingent trennen.

---

## Anhang: Prüftest (entfernt)

`protocol/tests/audit19_pruef.rs` (Helfer wie `wallet_ops_tests.rs`: `key`, `addr`, `kasware_sign`, `AgentIo` mit Zähler in `utxos`). Kern:

```rust
let first_due: i64 = 946_684_800_000; // 2000-01-01 00:00 UTC
let a = Action::TresorOpen { to: addr(&empf), amount: E8, anchor_day: 0, period_ms: DAY_MS, first_due,
    count: -1, fund: 100 * E8 as u64, max_fee: tresor::MIN_MAX_FEE, message: String::new() };
let (plan, _) = wo::build_plan(&basis, &a, &addr(&u), NET, &sim.funds(&u).utxos, P, &sim.params)?;
let s = wo::submit(&basis, &plan, &w::parse_signed(&kasware_sign(&plan, &u))?, NET, P, &sim.params)?;
assert!(s.report.valid);                       // angenommen
// 5 × tresor::pay_round(..., with_key = true) → 5 Zahlungen
// tresor::cancel am Server vorbei, dann tresor::follow(Search::Full) → 2000 Abfragen (zweimal)
```
Lauf: `cargo test --release --test audit19_pruef -- --nocapture`, 2 passed (Ausgaben in A19-1 bis A19-3).

---

## Behebung

Stand 06.10.2026. A19-4 und A19-5 in 850b8ac, die übrigen im Commit „Audit 19: Behebung A19-1/2/3/6/7/8/9“ (danach). Nichts gesendet, kein Server angefasst, Verträge unverändert.

| # | Stand | Änderung |
|---|---|---|
| A19-1 | **behoben** | Terminregel in der Bibliothek, im gemeinsamen Pfad von build und submit; Agent bedient reihum |
| A19-2 | **behoben** | Besitzer vor der Suche, Suche der Seite höchstens 64 Zustände, Agent sichert sofort |
| A19-3 | **behoben** (Restrisiko unten) | nur bald fällige, zahlbare Tresore belegen Plätze; erster Termin ≤ 1 Jahr; 10 je Besitzer; Gesamtgrenze 3 000 |
| A19-4 | behoben in 850b8ac | – |
| A19-5 | behoben in 850b8ac | – |
| A19-6 | **behoben** | `tresor owned` zeigt offenes Journal, Agent klärt es jede Runde, Hinweis auf der Seite |
| A19-7 | **behoben** (über Meldung) | klare Meldung „gleich erneut“, Plan wird nicht gesperrt, Signatur bleibt |
| A19-8 | **behoben** | Startdatum in UTC auf Seite und Server, gekennzeichnet; Text ehrlich |
| A19-9 | Hinweis umgesetzt | „mehrdeutig“ nennt die vollen Covenant-IDs |

**A19-1.** `TresorBasis` hat jetzt `pmt` (Past Median Time). `wallet_ops::run_tresor` prüft bei TresorOpen nach `check_params` zusätzlich `tresor::check_wallet_first_due(first_due, pmt)`: höchstens einen Tag zurück, höchstens `MAX_FIRST_DUE_AHEAD_MS` (366 Tage) voraus. `run_tresor` läuft in build **und** im Neubau von submit, die Aktion aus dem Plan des Browsers geht also nicht mehr ungeprüft durch. ghostctl `wallet_tresor_action_of` und `tresor open` nutzen dieselben Funktionen (`check_first_due`). Agent: `pay_round` sortiert die Runde. Offline fällige Tresore kommen zuerst, und zwar der am längsten nicht bediente zuerst (neues Feld `TresorRec::last_paid_ms`, wird mit der Zahlung über das Journal gesetzt), danach die übrigen in Dateireihenfolge. Je Tresor und Runde bleibt es bei höchstens einem Termin. Das gilt für Schlüsseldatei-Tresore genauso, sie zahlen ihren Rückstand weiter nach, aber reihum.
Tests: `a19_1_vergangener_termin_im_submit_abgelehnt` (genau der Audit-Fall: Plan mit Termin 2000-01-01, täglich, gebaut mit einer Uhr im Jahr 2000, signiert, submit gegen die echte Basis → „liegt in der Vergangenheit“; dazu Grenzen ±1 ms und Termin 2199 im Neubau), `a19_1_agent_bedient_faellige_tresore_reihum` (eine Sendung je Runde: A, B, A, B statt immer A). Rückbau-Proben in tests/wallet_ops_mutation.sh.

**A19-2.** Neue Bibliotheksfunktion `wallet_ops::follow_for_wallet`: erst `own_tresor` (Besitzer aus Adresse bzw. `plan.owner`, nicht gekündigt, nicht als fehlend markiert), dann `tresor::follow` mit dem neuen `Search::Public`, höchstens `tresor::PUBLIC_FOLLOW` = 64 Zustände. ghostctl `tresor_basis` ruft nur noch diese Funktion auf. Fremde, unbekannte, gekündigte und fehlende Tresore kosten keine Node-Abfrage. `pay_round` speichert sofort, wenn das Nachführen etwas ergibt (`note`), die teure erste Suche wiederholt sich nach einem Abbruch also nicht.
Tests: `a19_2_besitzer_vor_der_suche_und_begrenzt` (fremd/unbekannt/Anlegen: 0 Abfragen; am Server vorbei gekündigt mit 2 500 Tagen Rückstand: 64 statt 2 000 Abfragen; danach 0), `a19_2_fehlender_tresor_bleibt_nach_abbruch_markiert` (Runde hängt am zweiten Tresor, Markierung des ersten ist schon gesichert), Verdrahtung in ghostctl `wallet_tresor_verdrahtung`.

**A19-3.** `make_room_for_wallet(owner, now_ms)`:
- Höchstens `MAX_WALLET_PER_OWNER` = 10 (vorher 20) laufende je Besitzer.
- Ab 1 000 Einträgen fallen gekündigte und seit über einer Woche fehlende Wallet-Tresore heraus (`evictable`). Laufende mit Guthaben fallen nie heraus, sonst könnte der Besitzer über die Seite nicht mehr kündigen.
- Abgelehnt wird, wenn 1 000 Tresore *belegt* sind (`TresorRec::busy`: laufend, Zahlungen übrig, zahlbar, nächster Termin binnen `BUSY_HORIZON_MS` = 32 Tage), oder bei `MAX_FILE_ALL` = 3 000 Einträgen insgesamt.
- Dazu der erste Termin ≤ 1 Jahr (A19-1).
- Der Agent warnt im Log ab 90 % (`tresor_room_warning`).

Begründung der Zahlen steht an den Konstanten in tresor.rs. Das Muster aus dem Audit (1 000 Tresore mit Termin 2199) geht nicht mehr. Ein Termin 2199 wird abgelehnt, und Tresore mit fernem Termin, leere und erledigte ruhen und belegen keinen Platz.
**Restrisiko:** Wer die 1 000 belegten Plätze sperren will, braucht 100 Adressen und 1 000 Tresore mit je ≥ 2 KAS, die mindestens monatlich zahlen und danach wieder aufgefüllt werden. Jede Zahlung kostet ihn Netzgebühr, und das Geld ist gebunden. Wer die 3 000 Gesamtplätze mit ruhenden Tresoren füllen will, bindet ≥ 6 000 KAS über 300 Adressen. Möglich bleibt das, aber nicht mehr billig und nicht mehr dauerhaft ohne Zutun. Gegen einen entschlossenen Angreifer hilft nur die Warnung an den Betreiber.
Test: `a19_3_ruhende_tresore_belegen_keine_plaetze` (Termin in 200 Tagen, leer, erledigt und gekündigt belegen nichts; Gesamtgrenze; Verdrängen gekündigter und eine Woche fehlender Wallet-Tresore; Betreiber-Tresore bleiben), `a19_6_agent_klaert_journal_auch_ohne_faelliges` (Warnung). Der bisherige Test `tresor_grenzen_und_eingaben` läuft unverändert.

**A19-6.** `tresor owned` liest ein offenes Wallet-Journal der Tresor-Datei (nur lesend, `pending_wallet_tresore`). Ein neuer Tresor des Besitzers erscheint sofort mit `pending: {txid, action}`, ein geänderter bekommt die Markierung. `tresor_agent_step` verbindet sich auch ohne Fälliges, wenn ein Journal offen ist, und klärt dann nur das Journal (`resolve_pending`), ohne Runde über alle Tresore. Die Seite zeigt „noch nicht bestätigt“ mit Tx-Link und blendet Auffüllen und Kündigen dafür aus (`walletTresorActionable`). Nach dem Anlegen steht der Hinweis „erscheint unter „Meine Tresore“, sobald die Transaktion bestätigt ist … nicht ein zweites Mal anlegen“.
Tests: ghostctl `a19_6_tresor_owned_zeigt_offenes_journal`, `a19_6_agent_klaert_journal_auch_ohne_faelliges`; app `src/lib/audit19-tresor.test.ts`.

**A19-7.** Die Sperre im Agenten während des Wartens freizugeben (wie A17-6) hilft hier nicht. Das Journal der Zahlung bleibt bis zur Bestätigung offen, und `resolve_pending` hielte den Wallet-Submit ebenso auf („noch unterwegs“). Darum:
- `wallet_tresor_submit` übersetzt eine belegte Sperre in `TRESOR_BUSY` („Gerade läuft eine Zahlungsrunde für Tresore. Bitte in ein bis zwei Minuten erneut senden – es wurde nichts gesendet, deine Signatur bleibt gültig.“).
- Der Server erkennt diese und die „Bitte kurz warten“-Meldungen (`isRetryLater`), sperrt den Plan nicht (G-3) und antwortet mit `busy: true`.
- `WalletSignFlow` behält dann die geprüfte Signatur (`keepSigned`), „Senden“ geht gleich wieder.

Wie lange der Agent die Sperre hält, begrenzt weiter `Takt::agent` (90 s nach der ersten Sendung).
Tests: ghostctl `a19_7_belegte_tresor_sperre_heisst_gleich_erneut`, `server/walletActions.test.ts` (Audit 19), `src/lib/audit19-tresor.test.ts`.

**A19-8.** Die Seite nimmt als „heute“ das UTC-Datum (`utcToday`), wie die Termine (00:00 UTC) und ghostctl. Das Feld heißt „Erster Termin (Datum in UTC)“ und ist auf höchstens 365 Tage voraus begrenzt (`lastFirstDue`; ghostctl: 366 Tage ab Past Median Time). Liegt der Termin schon zurück, sagt die Seite, dass die erste Zahlung kurz nach dem Anlegen kommt. Der Text lautet jetzt: „Ausgelöst wird die Zahlung vom K.Lend-Agenten, in der Regel innerhalb weniger Minuten nach dem Termin – solange der Agent läuft und das Guthaben reicht. Laut Vertrag darf auch jeder andere auslösen …“.
Test: `src/lib/audit19-tresor.test.ts` (Abend in New York: Vortag abgelehnt wie am Server).

**A19-9.** `TresorFile::find` nennt bei mehrdeutiger Kurz-ID alle vollen Covenant-IDs. Test `a19_9_mehrdeutige_kurz_id_nennt_die_vollen`.

**Testläufe:** `cargo test --release` 678 bestanden, 0 fehlgeschlagen, 2 ignoriert (29 Testläufe); app: tsc (beide Projekte) sauber, `vitest run` 537/537 in 32 Dateien. Rückbau-Probe `zsh tests/wallet_ops_mutation.sh A19`: 6 von 6 Mutanten erkannt (Terminregel im Neubau, reihum, Besitzer vor der Suche, PUBLIC_FOLLOW, sofort sichern, busy).

**Offen:**
- Ein Tresor, der schon vor dieser Version mit altem Termin über submit angelegt wurde, bleibt in der Datei. Er zahlt jetzt reihum und nimmt anderen keine Sendezeit mehr. Löschen müsste der Betreiber von Hand. Ob es auf dem Server solche gibt, ist nicht geprüft (Server nicht angefasst).
- Die Oberfläche ist per Test und Typprüfung abgedeckt, nicht im Browser angesehen (Wallet nötig).
