# Audit 12 (Opus): Nachprüfung der Behebungen aus Audit 11 und Prüfung der Neuerungen

Stand: v3, Commit `8b6ee75` (per `git archive` in eine Arbeitskopie, Datum 29.09.2026).
Geprüft:
- Behebungen zu Audit 11: Commits `bea9b56`, `a8165ea`, `1483cc8`, `cdc7710`, `597d9c8`.
- Neu dazugekommen:
  - Dauerauftrag mit Tresor: `contracts/standing_order.sil`, `protocol/src/{standing,tresor,sim}.rs`, `ghostctl tresor …`, Agent-Schritt, Seite.
  - Verschlüsselte Nachrichten: `protocol/src/message.rs`, `ghostctl send/transfer/abo/messages`, `IncomingMessages.tsx`.
  - Wechselwirkungen mit `abo.rs`.

Ausgeführt:
- Ausgangslage: `cargo test --release --offline` mit 290 Tests grün, 2 ignoriert (Netz). `npm ci --offline` und `npx vitest run` mit 224 Tests grün.
- Mit den Tests dieses Audits:
  - Rust: 17 neue Tests in 6 Dateien, alle grün:
    - 5 in `vault_tests.rs`
    - 3 in `standing_order_tests.rs`
    - 2 in `tresor_e2e_tests.rs`
    - 7 in `review12_{rate,sweep,tresor}.rs`
  - Seite: 245 Tests grün (21 neue).
- 34 gezielte Mutanten (Abschnitt 5).

Keine echten Transaktionen. `keys/` und `deployments/` wurden nicht gelesen.

---

## 1. Zusammenfassung

**Kein Befund der Stufe kritisch oder hoch.** Die Behebungen aus Audit 11 sind im Vertrag korrekt und fast vollständig.
- **Kassen-Ausgang:** Er steht fest an `activeInputIndex + 1`. Das gilt auch, wenn der Vault nicht an Eingang 0 steht (neuer Test). Zwei Vaults können sich keinen Kassen-Ausgang mehr teilen.
- **`sweep`:** Nur Vaults mit Schuld 0 lassen sich auflösen, und nur wenn der Zins die ganze Sicherheit aufzehrt. Der Besitzer bekäme beim Schließen dann ohnehin nichts. Weder über Rundung noch über Rücknahme lässt sich ein Vault vorzeitig in diesen Zustand bringen (Begründung in 3.2).
- **Überläufe:** Innerhalb von `MAX_DEBT` + `MAX_INTEREST` gibt es keine. Nur die Umrechnung in KAS (`interestFee`) hat bei Tiefstpreisen eine Grenze. Sie liegt bei etwa 922 000 USD Zins, im Mainnet mit 50 GHOST Obergrenze unerreichbar (A12-14).

**Tresor-Vertrag (`standing_order.sil`):** Er hält, was er verspricht.
- Kein Auslöser entnimmt mehr als `amount + maxFee`. Er kann nichts umleiten, keinen Termin vorziehen oder überspringen, und nur der Absender kann kündigen oder auffüllen.
- Zwei Tresore können sich keinen Ausgang teilen.
- Alle 19 Mutanten am Tresor sind rot.

**Zwei Dinge bindet der Vertrag aber nicht:**
- **Den Payload.** Ein beliebiger Auslöser kann in eine echte Tresor-Zahlung eine eigene, an den Empfänger verschlüsselte Nachricht legen. Im Eingang ist sie von der echten versiegelten Nachricht nicht zu unterscheiden. Die Seite sagt nirgends, dass es keine Absender-Echtheit gibt (**A12-1, mittel**).
- **Den Rest der Höchstgebühr.** Den darf der Auslöser behalten (A12-8).

**Kryptografie in `message.rs`:** korrekt. x-only-ECDH ist mit beiden Vorzeichen richtig. Der Schlüssel ist an E_x und P_x gebunden. Die AEAD-Prüfung hat Magic und E_x als AAD. Ungültige Punkte und Fremddaten werden abgelehnt, und Klartext kann nie mit dem Magic beginnen.

**Off-chain:**
- **Umzugsskript:** Der f-String-Fehler in Schritt 2 ist behoben. Neu: Der eigene Schlüssel wird unscharf gesucht, das kann bei jedem Doppelklick einen weiteren Vault eröffnen (A12-4).
- **Agent:** Ein einziger Zombie-Vault unter 0,1 KAS blockiert das automatische Auflösen aller anderen (A12-2).
- **Zinsregel:** Der Zinstakt friert ein, wenn die Uhr einmal vorging (A12-3).
- **Seite:** mehrere niedrige Punkte (A12-6 bis A12-11).

**Tests:**
- Die Regel `noGhost()` in `sweep` war ohne Test. Der Kommentar in `vault_tests.rs` behauptet das Gegenteil. Ohne die Regel könnte jeder beim Auflösen unbegrenzt GHOST prägen. Die Regel steht im Vertrag, jetzt mit Test (A12-15).
- Zwei weitere Mutanten überleben (nOut-Grenze, Grenze des Auflösens auf 1 sompi genau). Beide sind unkritisch.

---

## 2. Befundtabelle

| ID | Schwere | Status | Befund | Ort |
|---|---|---|---|---|
| A12-1 | mittel | belegt | Tresor-Zahlung: Jeder Auslöser setzt den Payload frei. Eine untergeschobene verschlüsselte Nachricht erscheint im Eingang genau wie die echte (gleicher Betrag, „Absender unbekannt“, Etikett „verschlüsselt“). Kein Hinweis auf fehlende Absender-Echtheit, kein Abgleich mit `sealed` | `standing_order.sil:110–129`, `message.rs:249–254`, `ghostctl.rs:2140–2151`, `IncomingMessages.tsx:52–57, 76, 87–106` |
| A12-2 | niedrig | belegt | Ein Zombie-Vault unter 0,1 KAS: `ops::sweep` läuft über (`value − SWEEP_FEE` in u64), der Agent versucht nur den ersten Kandidaten. Alle anderen Zombies löst er nie auf. Die Seite zeigt eine negative Kassenzahlung | `ops.rs:614`, `ghostctl.rs:1718–1732, 1767–1789`, `vaultMath.ts:289`, `precheck.ts:220–230` |
| A12-3 | niedrig | belegt | Zinstakt: Steht `last_change` in der Zukunft (Uhr war vorgestellt), wartet die Zinsregel für immer | `rate.rs:140–146` |
| A12-4 | niedrig | belegt | Umzug: Der eigene Schlüssel wird über die Endung des Dateinamens in `./keys` gesucht. Ein falscher Treffer eröffnet bei jedem Doppelklick einen weiteren 50-KAS-Vault | `GHOST-Umzug-v3.command:163–167, 259–271` |
| A12-5 | niedrig | per Code | Die Agentenschleife arbeitet der Reihe nach: Orakel/Keeper bis 240 s, Daueraufträge bis 700 s, Tresore bis 700 s. Bis zu ≈ 30 min zwischen zwei Orakel- und Liquidationsrunden, das Fenster aus A11-O-2 wird länger | `ghostctl.rs:1139–1146, 2416, 2992` |
| A12-6 | niedrig | belegt | Seite: Der Probelauf des Tresors ist nicht ans Netz gebunden. Die Bestätigung nennt weder Empfänger noch Betrag je Zahlung, Anzahl oder Intervall | `StandingOrders.tsx:44, 86–89, 125–146, 498–505` |
| A12-7 | niedrig | belegt | Seite zeigt „Zahlung fällig“ samt Knopf, ghostctl verlangt aber 1 KAS Rest (`MIN_KEEP`) und zahlt nicht | `tresor.ts:203` gegen `tresor.rs:290, 312` |
| A12-8 | niedrig/Info | belegt | Den Rest der Höchstgebühr darf der Auslöser behalten, bis ≈ `maxFee` je Zahlung (0,006 KAS beim Standard, 0,096 KAS bei 0,1 KAS). Die Seite sagt „Empfänger oder Agent“ und „Netzgebühr aus dem Tresor“ | `standing_order.sil:117–121`, `tresor.ts:162–168` |
| A12-9 | niedrig | belegt | Termine liegen auf 00:00 UTC, die Seite zeigt Datum und Uhrzeit lokal: in New York fällig am Vortag 19:00, Sommerzeit verschiebt die Stunde | `StandingOrders.tsx:18, 413–420, 508` |
| A12-10 | niedrig | per Code | „Fällige Zahlung abholen“ ohne Rückfrage, auch im Mainnet. Reicht der Tresor nicht, zahlt der eigene Schlüssel still die Gebühr. Die Tresor-Nachricht verschwindet dann aus dem eigenen Eingang | `TresorList.tsx:56–57, 170`, `message.rs:235` |
| A12-11 | niedrig | belegt/per Code | Der Nachrichtenfilter lässt unsichtbare Zeichen durch (U+2028/2029, Tag-Zeichen U+E00xx, U+3164, U+180E, U+034F) | `abo.rs:330–341`, `abo.ts:61`, `actions.ts:66` |
| A12-12 | Info | belegt | Zinsregel: 6 Messungen können in 20 min zusammenkommen, 4 manipulierte genügen. Der Modulkopf („über die Hälfte der Stunde“) stimmt so nicht | `rate.rs` (`SAMPLE_GAP_SECS`, `MIN_SAMPLES`) |
| A12-13 | Info | belegt | Feste Ausgangsindizes verschiedener Verträge können zusammenfallen. Schließt ein Vault an Eingang i und zahlt ein Tresor an Eingang i+1 an die Zinskasse, erfüllt EIN Ausgang beide | `stable_vault.sil:162–167`, `standing_order.sil:113–116` |
| A12-14 | Info | belegt | `interestFee`: `(accrued / kasUsd)·1e8` läuft bei `MIN_KAS_USD` ab ≈ 922 000 USD Zins über, `close`/`sweep` scheitern dann. `math::interest_fee` schneidet beim `as i64` ab und wird negativ | `stable_vault.sil:151–157`, `math.rs:98–100` |
| A12-15 | Info | belegt | Testlücken: `noGhost()` in `sweep` war ungetestet, obwohl der Kommentar Gegenteiliges sagt. Test zu A11-V-7 beweist die neue Regel nicht (dreifach gesichert). Grenze `interestFee == coll` nicht auf 1 sompi genau getestet | `vault_tests.rs:1927` (Stand 8b6ee75), Mutanten M03, M04, M08 |
| A12-16 | Info | belegt/per Code | Tresor-Automatik: Ein Sendefehler beendet den Lauf für alle folgenden Tresore (`break`). „missing“ bleibt für die Automatik hängen. Der Import prüft schwächer als das Anlegen (ein präparierter Code kostet 2000 Node-Abfragen je Abgleich) | `ghostctl.rs:2820, 2904`, `tresor.rs:474–475, 582` |
| A12-17 | Info | belegt | Umzug: Nach einem abgebrochenen Umbenennen zeigt das Journal weiter auf `mainnet.json`. Die DRY-Vorschau zieht die GHOST nicht ab. Kleinigkeiten (Text „Keeper zahlt 0,045 KAS“, Keeper ohne KAS kann nicht auflösen, manuelles `--rate` setzt den Takt nicht) | `GHOST-Umzug-v3.command:129–151, 203–204`, `GHOST-Agent starten.command:23` |
| A12-18 | Info | belegt | Seite: `maxFee` eines Tresors wird nirgends angezeigt, die Nachricht aus einem importierten Code erscheint ungeprüft als „verschlüsselt“. Auffüllen sendet den Rohtext statt des geprüften Werts („1.000,5“ wird abgelehnt). Keine Warnung vor winzigem Ausgang bei der Rücknahme | `TresorList.tsx:121–128, 185, 199`, `precheck.ts:276–340` |
| A12-19 | Info | per Code | Der Eingang (`ghostctl messages`) glaubt der REST-API Absender, Betrag und Payload ohne Abgleich mit dem Node | `ghostctl.rs:2093–2094`, `message.rs:223–256` |

Belegt heißt: mit einem ausführbaren Test und dessen Ausgabe (Anhang). Per Code heißt: aus dem Quelltext abgeleitet, ohne eigenen Test.

---

## 3. Details

### 3.1 A12-1 (mittel): Untergeschobene Nachricht in echter Tresor-Zahlung

**Was der Vertrag bindet**
- `pay()` (`standing_order.sil:110–129`) prüft Empfänger, Betrag, Fortsetzung und Termin.
- Den Payload der Tx prüft er nicht. Es gibt auch keine Signatur, die ihn bindet: Auslösen darf jeder.

**Wer auslösen kann**
- Die Parameter stehen spätestens nach der ersten Zahlung im Klartext auf der Kette (P2SH-Skript im Eingang).
- Von da an kann jeder, der als Erster nach dem Termin sendet, zwei Dinge tun:
  - die versiegelte Nachricht des Absenders weglassen,
  - oder sie durch einen eigenen, an den x-only-Schlüssel des Empfängers verschlüsselten Text ersetzen.

**Wie der Empfänger es sieht**
- `inbox_entry` zeigt als Absender nur Eingänge mit Schlüssel-Adresse (`message.rs:249–254`).
- Zahlt der Auslöser die Gebühr aus dem Tresor (wie der Agent), steht dort „unbekannt“, genau wie bei der echten Zahlung.

Test `a12_untergeschobene_nachricht_im_eingang_ununterscheidbar` (`tresor_e2e_tests.rs`):

```
echt: Private("Miete Februar") von []
fremd: Private("Miete ab Maerz bitte an neue Adresse melden") von []
```

**Mit eigenem Eingang**
- Legt der Auslöser einen eigenen Eingang dazu, erscheint seine Adresse als „Absender“ der vollen Zahlung.
- Zugleich behält er den Rest der Höchstgebühr (A12-8). Test `a12_fremder_ausloeser_setzt_eigene_nachricht_und_behaelt_den_gebuehrenrest`.

**Was die Seite sagt**
- Sie beschreibt den Eingang als „Nachrichten, die andere … an diese Adresse geschickt haben“ (`IncomingMessages.tsx:52–57`).
- Sie hat eine Spalte „Absender“ und zeigt den Text mit dem Etikett „verschlüsselt“.
- Kein Hinweis darauf, dass:
  - jeder an jede Adresse verschlüsseln kann,
  - die Absender-Spalte nur Eingangsadressen der Tx zeigt,
  - bei Tresor-Zahlungen der Auslöser den Text bestimmt.
- Weder ghostctl noch die Seite gleichen den Payload mit `sealed` aus der Tresor-Datei ab (`tresor.rs:560–568`).

**Nebenpunkte**
- `sealed` ist in jeder Zahlung derselbe Chiffretext. Ein Dritter kann ihn in eigene Sendungen kopieren (Replay).
- Kryptografisch ist die Wiederholung unkritisch: gleicher Schlüssel, gleicher Nonce, gleicher Klartext ergeben dieselben Bytes, es gibt keine zwei Klartexte unter einem Nonce.
- Die Beschreibung im Tresor-Code wird beim Import ungeprüft übernommen und in der Tresor-Liste als „verschlüsselt“ gezeigt.

**Schwere**
- Kein direkter Geldverlust.
- Aber eine echte, pünktliche Zahlung in erwarteter Höhe verleiht jedem Text Glaubwürdigkeit („neue Adresse“, „Überzahlung, bitte zurück“). Deshalb mittel.

**Abhilfe**
- Im Eingang deutlich sagen, dass Nachrichten keine Absender-Echtheit haben.
- Tresor-Zahlungen als solche kennzeichnen (Eingang = bekannter Tresor).
- Den Payload gegen `sealed` bzw. die bekannte öffentliche Nachricht prüfen; bei Abweichung warnen („Nachricht stammt vom Auslöser, nicht vom Absender“).
- Im Vertrag ließe sich der Payload nur über eine Signatur des Absenders binden. Dann könnte nicht mehr jeder auslösen, das ist nicht empfohlen.

### 3.2 `sweep` und die übrigen Behebungen im Vertrag

**Kann ein Vault vorzeitig als aufzehrend gelten?**
- `sweep` verlangt `debt == 0` und `interestFee(index, kasUsd) == coll` (`stable_vault.sil:295–303`).
- Den Zustand eines fremden Vaults ändern nur zwei Wege.
- **`redeem`:** Auch die ganze Schuld darf man zurückgeben. Vorher war der Vault gesund zur Liquidationsquote `liq` (≥ 1), also Wert ≥ (Schuld + Zins)·liq. Danach bleibt Wert ≥ Schuld·(liq − 0,99) + Zins·liq > Zins. `sweep` greift also nicht sofort.
- **`liquidate`:** Nach voller Tilgung darf ein Rest unter dem Zins bleiben. Genau das ist der beabsichtigte Zombie-Fall.
- Die Aufrundung in `mulDivUp` macht höchstens 1 sompi aus.
- Den Preis wählt der Auslöser nicht: Gelesen wird die einzige, aktuelle Orakel-UTXO.
- Ein Preiseinbruch macht einen tilgungsfreien Vault mit offenem Zins auflösbar. Der Besitzer bekäme beim Schließen dann ebenfalls nichts. Er verliert nur die Möglichkeit, auf Erholung zu warten. Das folgt aus dem USD-Zins und ist kein Fehler.

**Wer den Rest von `SWEEP_FEE` bekommt**
- Der Auslöser darf ihn nach der Netzgebühr als Wechselgeld nehmen (≈ 0,045 KAS). Das ist gewollt und dient als Anreiz.

**Kassen-Ausgang an `activeInputIndex + 1`**
- Test `a12_kasse_direkt_hinter_dem_vault_auch_an_eingang_1`: Orakel an Eingang 0, Vault an 1, `close` und `sweep` verlangen die Kasse an Ausgang 2. An Ausgang 1 wird sie abgelehnt.
- Mutante M02 (fester Index 1) ist rot.

**A12-13, feste Ausgangsindizes über Vertragsgrenzen**
- Der Tresor verlangt seine Zahlung an Ausgang = eigener Eingang, der Vault seine Kasse an Ausgang = eigener Eingang + 1.
- Zahlt ein Tresor an die Zinskasse, erfüllt ein einziger Ausgang beide Verträge. Test `a12_tresor_zahlung_an_die_kasse_deckt_zugleich_den_zins`:

  ```
  Vault-close + Tresor-pay mit geteiltem Ausgang 1: [Ok(()), Ok(()), Ok(())]
  ```

- Der Vault-Besitzer nimmt die ganze Sicherheit, der Tresor eines Dritten zahlt seinen Zins.
- Das setzt einen Tresor mit der Zinskasse als Empfänger voraus. Deshalb nur Info.
- Allgemein gilt: Muster mit festem Index schützen nur gegen Duplikate desselben Vertrags.

**A12-14, Rechengrenze von `interestFee`**
- `accrued` bleibt bis 1,9e18 überlauffrei.
- `mulDivUp(accrued, 1e8, kasUsd)` rechnet `(accrued / kasUsd)·1e8` und läuft bei `kasUsd` = 1000 (0,00001 USD) ab ≈ 9,2e13 über, also ≈ 922 000 USD Zins.
- Test `a12_zinsgebuehr_rechengrenze_bei_tiefstpreis`:

  ```
  math::interest_fee (u128 → i64): -8446744073709551616
  sweep bei 1 Mio USD Zins, Preis 0,00001 USD: [Err("NumberTooBig(\"Product exceeds 64-bit signed integer range\")"), Ok(())]
  ```

- Mit `maxDebt` = 50 GHOST unerreichbar.
- Zu dokumentieren ist, dass die Grenze nicht aus `MAX_DEBT`/`MAX_INTEREST` folgt, sondern vom Preis abhängt.
- In Rust `i64::try_from` statt `as i64` verwenden.

**Übrige Behebungen**
- **MIN_REDEEM, 1 % Rücknahmegebühr, Zinseszins:** korrekt. Die Mutanten M10, M11 und M12 sind rot.
- **Überläufe in `accrual`:** keine, da `debt + interest ≤ 2e17`. Es gilt `(d/1e9)·g ≤ 1,8e18` und `(d % 1e9)·g < 9e18`.
- **`healthy`:** bis `MAX_COLLATERAL` und 900 USD sicher.
- **`nOut ≤ MAX_GHOST_OUTS`:** steht im Vertrag, ist aber redundant (siehe A12-15).
- **`max_rate` 20 %:** `ghostctl.rs:912` setzt `rate_from_apr(RATE_MAX_PCT = 20)`. Das ist dieselbe Funktion, mit der der Agent Sätze erzeugt. Kein Satz des Agenten kann also über `maxRate` liegen.

### 3.3 `standing_order.sil`

| Frage | Ergebnis |
|---|---|
| Mehr als `amount + maxFee` entnehmen? | Nein. Die Zahlung ist exakt `amount` (`==`), die Fortsetzung `≥ Wert − amount − maxFee`, genau eine Fortsetzung (M19–M24 rot). **Aber:** Wohin die bis zu `maxFee` fließen, bestimmt der Auslöser (A12-8). Test `a12_ausloeser_darf_den_rest_der_hoechstgebuehr_behalten`, im Simulator mit echter Gebühr: Gewinn 0,00624 KAS (Standard) bzw. 0,09625 KAS (maxFee 0,1 KAS). |
| Termine überspringen oder vorziehen? | Nein. Die CLTV-Schwelle `nextDue` gehört zum Zeitbereich, dynamisch geprüft (`≥ LOCK_TIME_THRESHOLD`). Final erst bei `lock_time < PMT`. Jede Zahlung schiebt genau eine Periode weiter, `due > nextDue` (M16, M25 rot). Einen Termin unter der Zeitschwelle kann niemand zahlen, nur kündigen (`a12_termin_unter_der_zeitschwelle_zahlt_nie`). Rückstände sind einzeln nachzahlbar, das ist gewollt. |
| Umleiten an Dritte? | Nein, `scriptPubKey == P2PK(recipient)` (M18 rot). |
| Blockieren oder zerstören? | Ohne Signatur des Absenders nicht. `keep > 0` und genau eine Fortsetzung, `cancel`/`topUp` nur mit Signatur (M28–M32 rot). Reicht das Guthaben nicht (`keep ≤ 0`) oder ist `left = 0` oder das Jahr 2200 erreicht, bleibt der Tresor bis zur Kündigung liegen. Ist der Schlüssel des Absenders verloren, liegen die KAS für immer fest (Design). |
| Zwei Tresore gegeneinander? | Nein, die Zahlung steht an Ausgang = eigener Eingang. Die Fortsetzung ist ein Covenant-Ausgang mit eigenem Skript und kann nie der P2PK-Zahlungsausgang sein (M17 rot). Mit anderen Verträgen aber möglich, siehe A12-13. |
| Tresor nicht an Eingang 0? | Funktioniert symmetrisch: Zahlung an Ausgang i (Test `zwei_tresore_teilen_sich_keinen_ausgang`, Tresor 2 an Eingang 1). ghostctl baut immer Eingang 0. |
| Kalender | `civil`/`days`/`monthDays` stimmen mit chrono überein (Grenzfälle und 120 Zufallswerte, M33 rot). Der Anker bleibt erhalten (31.01. → 28.02. → 31.03.). Anker > 31 wirkt wie Monatsletzter, negativer Anker wie Intervall; ghostctl lässt beides nicht zu (`a12_anker_ausserhalb_1_bis_31`). `MAX_TIME` gilt für den neuen Termin (M26 rot). |
| Zeit-Locktime gegen DAA | `tx.time` bedeutet ms gegen Past Median Time, `sim.rs` bildet den Konsens nach (strikt kleiner, Ausnahme: alle Eingänge final). Miner können die PMT nur mit Mehrheit und um Minuten verschieben. Unkritisch. |

### 3.4 `message.rs`, Kryptografie

- **x-only-ECDH:**
  - Der Sender rechnet `x(e·lift_x(P))`, der Empfänger `x(sk·lift_x(E_x))`.
  - `lift_x(P) = ±sk·G`, und ±Q haben dieselbe x-Koordinate. Das Geheimnis stimmt also für beide Vorzeichen.
  - Getestet mit beiden Paritäten und unabhängig über `mul_tweak` nachgerechnet.
  - Die Negation von `e` ist für die Korrektheit nicht nötig, schadet aber nicht.
- **KDF:** BLAKE2b-256(Domäne ‖ shared_x ‖ E_x ‖ P_x). Das bindet an genau dieses Paar.
- **AEAD:** ChaCha20-Poly1305, AAD = Magic ‖ E_x. Der Nonce geht in die AEAD-Prüfung ein. Jedes gekippte Byte, jede Kürzung und jede Verlängerung ergibt `None` (Test).
- **Ungültiges E_x:** Kein Punkt oder ≥ p wird von `XOnlyPublicKey::from_slice` abgelehnt. Kofaktor 1, keine Kleingruppen-Angriffe.
- **Fremddaten:**
  - Binärer Payload wird ignoriert, UTF-8 nur nach `check_message`.
  - Auch entschlüsselter Text muss `check_message` bestehen.
  - Klartext kann nicht mit dem Magic beginnen (`\x01` ist ein Steuerzeichen).
  - Unsichtbare Zeichen außerhalb der Liste: A12-11.
- **Nicht gegeben, und so auch nicht behauptet:** Absender-Echtheit und Key-Commitment. Ein Payload könnte für zwei Empfänger zu verschiedenen Texten entschlüsseln. Bei einem Payload je Tx ist das ohne praktische Bedeutung. Die fehlende Absender-Echtheit wird auf der Seite nicht gesagt: A12-1.

### 3.5 Off-chain

**Umzugsskript, Stand der Punkte aus Audit 11**
- **Schritt 2:** Er formatiert mit `%` statt f-String, der Syntaxfehler ist behoben. Szenario `real1`: Die eigenen Vaults werden nach Schuld sortiert abgearbeitet. Der fremde Vault bleibt unberührt, nach dem Schließen wird über die Covenant-ID neu indiziert.
- **mkdir-Sperre mit `trap`, `pgrep`, Journal unter flock:**
  - Szenario `sd`: Ein laufender Agent führt zum Abbruch, die Sperre ist danach weg.
  - Szenario `se`: Bei einer fremden Sperre bricht das Skript ab.
  - Szenario `sf`: Bei einem Fehler räumt der `trap` auf.
  - Szenario `sa`: Ein fremdes Ziel im Journal führt zum Abbruch ohne Umbenennen.
- **DRY:** Nur `DRY=1` ist ein Probelauf.
- **Neu:** A12-4 (Schlüsselwahl) und A12-17 (Randfall Journal, DRY-Vorschau).
- **Wo die Attrappen liegen:** `…/scratchpad/umzug12/`, nicht in der Arbeitskopie. Dazu gehören `stub.py` und die Szenarien `dry1`, `real1` sowie `sa` bis `sg`.

**Zinsregel (`rate.rs`):**
- Median, ≥ 6 Messungen, ≥ 240 s Abstand, Pool ≥ 10 GHOST und unbekannter Pool wirken wie beschrieben.
- Die Sperre ist prozessübergreifend: Test gegen Python-flock, Rust scheitert nach 0,6 s, `rate::update` wartet.
- Eine beschädigte Datei sperrt die Zinsregel auf der sicheren Seite.
- Neu: A12-3 und A12-12.

**Agent:**
- Liquidationskandidaten: Nach einem Fehler kommt der nächste dran.
- Eine Doppelsendung scheitert an der verbrauchten UTXO, das Journal klärt das.
- `sweepable` wird zu Orakel- UND Marktpreis geprüft.
- Neu: A12-2 und A12-5.
- Tresor-Schritt:
  - Er nimmt immer `r.payload()`.
  - `upsert` überschreibt keine vorhandene Nachricht.
  - Tresor und Dauerauftrag vom Rechner sind getrennte Wege. Doppelt gezahlt wird nur, wenn der Nutzer beides anlegt.
  - `<netz>-abos.json` ist rechnerlokal: Zwei Agenten mit kopiertem `deployments/` zahlen Daueraufträge doppelt, Tresore nicht.

---

## 4. Stand der Audit-11-Befunde

| ID | Stand in Audit 12 | Begründung |
|---|---|---|
| A11-V-1 | **hält** | Kasse an `activeInputIndex + 1`, auch bei Vault an Eingang 1 (neuer Test). M00, M01, M02 rot. Nebenwirkung mit anderen Verträgen: A12-13 (Info) |
| A11-V-2 | **hält** | `REDEEM_FEE_BPS` = 100. M12 (50) rot (`zins_ruecknahme_und_zinskasse`). Feed-Schwelle 0,5 % laut Teil B |
| A11-V-3 | **hält** | M10 rot (`v3_ruecknahme_mindestens_ein_ghost_oder_die_ganze_schuld`) |
| A11-V-4 | **hält, mit Einschränkungen** | Der Vertrag ist dicht (M04 bis M07, M09, M13 rot). `noGhost()` in `sweep` war aber ungetestet, jetzt mit Test (A12-15). Agent: A12-2. Seite: negative Anzeige unter 0,1 KAS |
| A11-V-5 | **hält** | M11 rot (`kompletter_lebenszyklus`). Überlauffrei im Rahmen, Grenze der Umrechnung: A12-14 |
| A11-V-6 | übernommen | Nicht erneut mutiert, Test vorhanden |
| A11-V-7 | **hält als Zweitsicherung** | M03 überlebt. Drei GHOST-Ausgänge lehnt der Vault auch ohne die Regel ab, und KCC20 lehnt sie ab (Diagnose unten). Der Test beweist also nicht die neue Regel |
| A11-V-8 | dokumentiert | – |
| A11-V-9 | **hält** | `ghostctl.rs:912` |
| A11-V-10 | **hält** | – |
| A11-O-1 | **hält, Rest** | Median, 6 Messungen, Mindestliquidität. Rest: A12-12 |
| A11-O-2 | dokumentiert | Das Fenster wird durch A12-5 länger |
| A11-O-3 | **hält** | Szenario `sd` |
| A11-O-4 | **hält, Randfall** | Szenarien `sa`, `sb`, `real1`. Abbruch mitten im Umbenennen: A12-17 |
| A11-O-5 | **hält, neuer Fehler daneben** | Nur eigene Vaults, aber die Schlüsselwahl ist unscharf (A12-4) |
| A11-O-6 | **hält** | Szenarien `se`, `sf` |
| A11-O-7 | **hält** | `ghostctl.rs:1236–1247`: `redemption` vor `consolidate` |
| A11-O-8 | **hält** | `rate::measure` am frisch abgeglichenen Stand, `pool_unresolved` wirkt |
| A11-O-9 | **hält, Randfall** | Takt in Datei unter Sperre. Uhr vorgestellt: A12-3 |
| A11-O-10 | **hält** | – |
| A11-O-11 | dokumentiert | – |
| A11-O-12 | **hält** für Schließen und Abheben | Rücknahme ohne Warnung: A12-18 |
| A11-O-13 | **hält** | – |
| A11-O-14 | **hält** | f-String in Schritt 2 behoben, `real1` |
| A11-O-15 | offen, dokumentiert | – |

---

## 5. Mutantenergebnis

**Wie gerechnet wurde**
- Skript `…/scratchpad/mut12.py`: je Mutante genau eine Zeile ändern, bauen, testen, wiederherstellen.
- Tests je Vertrag:
  - `stable_vault.sil`: `vault_tests`, `vault_math_tests`, `e2e_tests`.
  - `standing_order.sil`: `standing_order_tests`, `tresor_e2e_tests`.
- Die neuen Tests dieses Audits waren dabei schon enthalten.
- `cargo test` bricht nach der ersten roten Test-Datei ab. Die Liste der roten Tests ist bei Mutanten, die schon `e2e_tests` tötet, deshalb unvollständig. „Überlebt“ ist dagegen belastbar.
- Nach dem Lauf sind beide Verträge byte-gleich mit `8b6ee75`.

**Ergebnis:** 32 von 34 rot, 2 überleben (M03, M08).

| # | Ort | Mutante | Ergebnis |
|---|---|---|---|
| M00 | vault:165 | Kassen-Skript → true | rot |
| M01 | vault:166 | Kassen-Betrag → true | rot |
| M02 | vault:163 | Index fest 1 | rot |
| **M03** | vault:215 | `nOut <= MAX_GHOST_OUTS` → true | **überlebt** (redundant, s. u.) |
| M04 | vault:296 | `noGhost()` in sweep entfernt | rot, **nur** durch den neuen `a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault` |
| M05 | vault:297 | sweep `debt == 0` | rot |
| M06 | vault:298 | sweep keine Fortsetzung | rot |
| M07 | vault:301 | sweep `interestFee == coll` → true | rot |
| **M08** | vault:301 | `interestFee >= coll − 1` | **überlebt** (1 sompi, unkritisch) |
| M09 | vault:302 | 1 sompi weniger an die Kasse | rot |
| M10 | vault:338 | MIN_REDEEM | rot |
| M11 | vault:146 | ohne Zinseszins | rot (e2e) |
| M12 | vault:77 | Rücknahmegebühr 0,5 % | rot (e2e) |
| M13 | vault:84 | SWEEP_FEE 0,01 KAS | rot (e2e) |
| M14 | vault:286 | Staubgrenze `>` | rot |
| M15–M33 | Tresor | alle `require`, Zahlungsindex fest 0, `keep − 1`, `left − 1`, Monatsende, `>=` statt `==` beim Betrag | **alle rot** |

**Befunde aus dem Lauf**
- **M04:** Ohne `noGhost()` in `sweep` könnte jeder den Minter-Zweig mitnehmen und beliebig GHOST prägen. Das KCC20-Ergebnis im neuen Test: `[Err("VerifyError"), Ok(()), Ok(())]`, also lehnt nur der Vault ab. Die Regel steht im Vertrag; vor diesem Audit hat kein Test sie gesichert. Der Kommentar in `vault_tests.rs:1927` („heimliches Prägen nebenbei scheitert an noGhost“) steht über einer reinen Gegenprobe ohne Prägen.
- **M03, Diagnose** (`a12_diagnose_drei_ghost_ausgaenge`): Mit und ohne Regel ergibt sich `[Err("VerifyError"), Ok(()), Err("VerifyError"), Ok(())]`. Vault und KCC20-Leader lehnen beide ab, dahinter steht der Schleifenwächter. Die Regel ist eine Zweitsicherung, der Test in AUDIT.md belegt sie nicht.

---

## 6. Was hält

- **Kern v3 nach den Behebungen:**
  - Die Kasse lässt sich nicht teilen, auch mit dem Vault an Eingang 1.
  - `sweep` lässt sich nicht vorzeitig auslösen, und beim Auflösen lässt sich nichts prägen.
  - Rücknahmegebühr 1 % und MIN_REDEEM wirken.
  - Der Zinseszins ist pfadunabhängig (Test aus Audit 11) und läuft innerhalb der Obergrenzen nicht über.
  - `max_rate` 20 % im Deploy.
- **Tresor-Vertrag:** alle Regeln durch Tests gesichert, 19 von 19 Mutanten rot.
  - Kein Abfluss über `amount + maxFee`, kein Vorziehen, kein Umleiten.
  - Kündigen und Auffüllen nur mit Signatur; keine zweite Fortsetzung.
  - Kalender gegen chrono geprüft, CLTV im Zeitbereich.
- **`message.rs`:** ECDH mit beiden Vorzeichen, KDF-Bindung, AEAD mit AAD, frischer Ephemeral-Schlüssel und Nonce je Nachricht. Fremd- und Binärdaten werden abgelehnt, Klartext und Magic sind nicht verwechselbar. Tresor-Wiederholung desselben Chiffretexts ist kryptografisch unkritisch.
- **Seite:** kein XSS (React escaped, kein `dangerouslySetInnerHTML`).
  - Server-Positivliste, `execFile` ohne Shell, Herkunftsprüfung.
  - Beträge und Startguthaben gleich wie in Rust; Monatsende gleich wie im Vertrag.
  - `vaultMath.ts` stimmt wörtlich mit dem Vertrag überein, der Auflösen-Knopf erscheint nur bei auflösbarem Vault.
- **Off-chain:**
  - f-String-Fehler behoben.
  - Sperren und Journal im Umzug, Zinstakt prozessübergreifend.
  - Keeper-Kandidaten nacheinander, Doppelsendung durch das UTXO-Modell ausgeschlossen.
  - Der Tresor-Agent zahlt mit Gebühr ≤ maxFee und lässt den Rest im Tresor.

---

## 7. Empfehlungen (Reihenfolge)

1. **A12-1:**
   - Eingang und Tresor-Liste sagen ausdrücklich, dass Nachrichten keine Absender-Echtheit haben.
   - Tresor-Zahlungen kennzeichnen und gegen `sealed` bzw. die bekannte Nachricht abgleichen.
   - `from` für P2SH-Eingänge als „Vertrag/Tresor“ benennen statt „unbekannt“.
2. **A12-2:**
   - `checked_sub` in `ops::sweep`.
   - Kandidaten unter `SWEEP_FEE` plus Mindestausgang ausschließen.
   - Nach einem Fehlschlag den nächsten Kandidaten nehmen.
   - Die Seite zeigt nie einen negativen Betrag.
3. **A12-4:** `keys --json --dir "$(dirname "$OWNER")"` und den vollen Pfad vergleichen.
4. **A12-3:** `last_change > now` auf `now` kappen.
5. **A12-5:** Daueraufträge und Tresore als eigene Tasks, oder mit kürzerem Zeitlimit.
6. **A12-6 bis A12-10:** Probelauf ans Netz binden, Bestätigung mit Empfänger/Betrag/Anzahl, Status „fällig“ mit `MIN_KEEP`, Texte zu Auslöser und `maxFee`, Termine mit Zeitzone, Rückfrage beim Abholen.
7. **A12-11:** Filter auf Unicode-Kategorien Cf/Zl/Zp/Co und Tag-Zeichen erweitern (Rust und TS gleich).
8. **Tests:** Die neuen Tests übernehmen. Einen Test auf die Grenze `interestFee == coll` genau am Rand (M08) ergänzen. In AUDIT.md bei A11-V-7 die Zweitsicherung benennen.

---

## Anhang A: Testausgaben

**Ausgangslage (`cargo test --release --offline`, 8b6ee75):**
- lib 49
- audit10_pool_engine 20, audit10_pool_regress 4
- chain 3, e2e 4, factory 21, ghost_token 9, oracle 26, payload 7
- pool_e2e 3, pool 32
- standing_order 15, tresor_e2e 12
- vault_math 4, vault 81
- rest_live 2 ignoriert
- Summe 290 grün

**Nach Audit 12:**

```
standing_order_tests: test result: ok. 18 passed; 0 failed
tresor_e2e_tests:     test result: ok. 14 passed; 0 failed
vault_tests:          test result: ok. 86 passed; 0 failed
review12_rate:        test result: ok. 4 passed
review12_sweep:       test result: ok. 1 passed
review12_tresor:      test result: ok. 2 passed
app (vitest):         Tests  245 passed (245)
```

**Ausgewählte Ausgaben:**

```
sweep + Prägen: [Err("VerifyError"), Ok(()), Ok(())]
Vault-close + Tresor-pay mit geteiltem Ausgang 1: [Ok(()), Ok(()), Ok(())]
math::interest_fee (u128 → i64): -8446744073709551616
sweep bei 1 Mio USD Zins, Preis 0,00001 USD: [Err("NumberTooBig(\"Product exceeds 64-bit signed integer range\")"), Ok(())]
drei GHOST-Ausgänge (mit und ohne L215): [Err("VerifyError"), Ok(()), Err("VerifyError"), Ok(())]

Auslöser: Netzgebühr 375690 sompi, Gewinn 624310 sompi (0.00624 KAS) aus maxFee 1000000
echt: Private("Miete Februar") von []
fremd: Private("Miete ab Maerz bitte an neue Adresse melden") von []
Auslöser: Netzgebühr 375165 sompi, Gewinn 9624835 sompi (0.09625 KAS); Tresor verliert 10000000 statt 375165 sompi
Empfänger liest: Some(Private("Überzahlung – bitte 50 KAS zurück an kaspa:qfremd"))

Vault auf 0.09 KAS gesenkt (storage 448310 g)
keeper_round wählt Vault Some(0)
Vault 0: Sicherheit 0.0900 KAS, auflösbar true
   mit Keeper-KAS: Err("Input 0: NumberTooBig(\"out of range integral type conversion attempted\")")
Vault 1: Sicherheit 0.2000 KAS, auflösbar true  →  Einreichen: angenommen

nach 1200 s (20 min), davon 720 s manipuliert: Change { median: 1.006, have: 6, next: 2.5 }
    1 h nach Korrektur: Wait { secs_left: 3600 }, reserve = None
 8760 h nach Korrektur: Wait { secs_left: 3600 }, reserve = None
decode(period 1 ms, amount 1 sompi, maxFee 100 KAS): Ok((1, 1, 10000000000))
locate: Ok(None), Node-Abfragen je Abgleich: 2000
```

## Anhang B: Neue und geänderte Dateien in der Arbeitskopie

**`protocol/tests/vault_tests.rs`**
- `execute` ruft jetzt `execute_lt(…, lock_time)` auf.
- Neue Tests:
  - `a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault`
  - `a12_kasse_direkt_hinter_dem_vault_auch_an_eingang_1`
  - `a12_tresor_zahlung_an_die_kasse_deckt_zugleich_den_zins`
  - `a12_zinsgebuehr_rechengrenze_bei_tiefstpreis`
  - `a12_diagnose_drei_ghost_ausgaenge`

**`protocol/tests/standing_order_tests.rs`**
- `a12_ausloeser_darf_den_rest_der_hoechstgebuehr_behalten`
- `a12_anker_ausserhalb_1_bis_31`
- `a12_termin_unter_der_zeitschwelle_zahlt_nie`

**`protocol/tests/tresor_e2e_tests.rs`**
- `a12_fremder_ausloeser_setzt_eigene_nachricht_und_behaelt_den_gebuehrenrest`
- `a12_untergeschobene_nachricht_im_eingang_ununterscheidbar`

**`protocol/tests/review12_rate.rs`, `review12_sweep.rs`, `review12_tresor.rs`:** Belegtests Off-chain (A12-2, A12-3, A12-12, A12-8, A12-16).

**`app/src/components/audit12-messages.test.ts`, `app/src/components/audit12-tresor.test.ts`, `app/src/lib/audit12.test.ts`:**
- Die Befund-Tests der Seite (21).
- Sie halten den heutigen Stand fest und müssen nach einer Behebung umgeschrieben werden.

**Außerhalb der Arbeitskopie (`…/scratchpad/`):**
- `mut12.py`, `mut12.log` (Mutanten)
- `umzug12/` (Attrappen und Szenarien Umzug)
