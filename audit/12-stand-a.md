# Audit 12 – Stand der Behebungen, Gruppe a (Tresor und Nachrichten)

Branch `fix12a`, Basis `8b6ee75`, 29.09.2026. Grundlage ist der Bericht von Audit 12 (Opus).
Befunde dieser Gruppe: A12-1, A12-6, A12-7, A12-8, A12-9, A12-10, A12-11, A12-16, A12-19 und die Tresor-Teile von A12-18.

Die erste Behebung (Commits `2f061a7`, `1f193ff`, `5aea635`) hat ein unabhängiger Prüfer beanstandet. Die Punkte und ihr Stand stehen im Abschnitt **Nachprüfung**; die Einzelheiten weiter unten sind auf den neuen Stand gebracht.

Den Stand danach (`d5fab53`) hat ein Prüfer erneut beanstandet: zwei ungetestete Kernstellen (Rückbauten T6b und R20 überlebten) und fünf kleine Punkte. Stand dazu im Abschnitt **Zweite Nachprüfung** am Ende.

Die unabhängigen Prüfer der Behebungen (Stand `a70fbbd`) haben sechs Restpunkte gemeldet. Stand dazu im Abschnitt **Restpunkte** am Ende (Branch `fix13a`).

- Verträge (`contracts/*.sil`) sind unverändert.
- Keine echten Transaktionen. `keys/` und `deployments/` wurden nicht gelesen.
- Einmal lesend gegen das Mainnet geprüft: eine Tx über die REST-API und ihr Block über `get_block` am Node. Der Abgleich aus A12-19 ergab für die echte Tx „gleich“ und für einen um ein Byte veränderten Payload „ungleich“.
- Übernommen aus der Prüfung wurden die passenden Belegtests:
  - `tresor_e2e_tests.rs`: `a12_untergeschobene_nachricht_…` und `a12_fremder_ausloeser_…`. Beide sind auf die Behebung umgeschrieben.
  - `standing_order_tests.rs`: die drei `a12_*`-Tests. Sie beschreiben Vertragsverhalten und sind unverändert.
  - `app/src/**/audit12*.test.ts`: umgeschrieben zu `audit12a-*.test.ts`.
- Kontrolle: Die neuen Seiten-Tests liefen gegen die Quelldateien von `8b6ee75`. 61 Tests schlugen fehl. Grün blieben nur die Gegenproben, etwa „normaler Text bleibt erlaubt“ und „React escaped“.
- Die neuen Rust-Tests nutzen die neuen Schnittstellen (`inbox_entry_with`, `Origin`, `due_now(…, with_key)`, `parse_script` …) und lassen sich gegen den alten Stand nicht einmal übersetzen. Der Filtertest scheitert inhaltlich, siehe A12-11.

## Übersicht

| ID | Stand | Kern der Änderung |
|---|---|---|
| A12-1 | behoben, seit 30.09.2026 im Vertrag (siehe **A12-1 im Vertrag**) | Der Vertrag bindet die Nachricht über `payloadHash`; wer auslöst, kann sie nicht ändern. Vorher (off-chain): Der Eingang erkennt Tresor-Zahlungen am Zweig `pay` samt Ausgang an mich und vergleicht die Nachricht mit der hinterlegten Fassung übernommener Tresore. Den Besitzer nennt er nur bei übernommenen Tresoren, als Angabe des Codes. Seite und FAQ sagen, dass es keine Absender-Echtheit gibt. |
| A12-6 | behoben | Der Probelauf gilt nur im selben Netz. Die Bestätigung nennt Empfänger, Betrag, Intervall, Anzahl, ersten Termin (Ortszeit und UTC), Höchstgebühr und Netz. |
| A12-7 | behoben | „Fällig“ nur, wenn `pay` wirklich zahlt (1 KAS Rest). Rust (`due_now`, `payable`) und Seite (`feeSource`) sind gleich. „Aufgebraucht“ nur, wenn auch der Vertrag nicht mehr zahlen lässt, sonst „knapp“. |
| A12-8 | dokumentiert (Texte korrigiert) | Hinweise, FAQ, Tresor-Liste und `--max-fee`-Hilfe sagen: jeder darf auslösen, und ein fremder Auslöser darf den Rest der Höchstgebühr behalten. Die 1 KAS Rest sind als Regel von ghostctl benannt, nicht als Vertragsgarantie. |
| A12-9 | behoben | Termine erscheinen je Termin mit Datum und Uhrzeit in Ortszeit und zusätzlich in UTC. Hinweis auf die Zeitumstellung. |
| A12-10 | behoben | „Abholen“ nur mit Rückfrage (seit der zweiten Nachprüfung mit Test am Knopf). Der eigene Schlüssel wird nur mitgegeben, wenn der Tresor die Gebühr nicht mehr trägt, und das steht in der Rückfrage. Die Tresor-Nachricht bleibt dann im eigenen Eingang. |
| A12-11 | behoben | Gleicher Filter in Rust, auf der Seite und im Server: Cc, Cf, Default_Ignorable, Zl/Zp, Private Use, Nichtzeichen. VS16 ist nur hinter Bildzeichen erlaubt. Gemeinsame Fallsammlung. Ältere Daueraufträge senden bereinigt weiter; Zahlungen mit solchen Zeichen bleiben im Eingang (Text ausgeblendet). |
| A12-16 | behoben | Der Import prüft wie das Anlegen und lehnt einen Rückstand über 400 Termine ab. Nach einem Sendefehler geht die Runde weiter, wenn kein Journal offen ist. „Nicht auffindbar“ wird eine Woche lang stündlich erneut gesucht, dabei nur über 32 Zustände. Die Runde steht in der Bibliothek und ist gegen den Simulator getestet. |
| A12-18 (Tresor-Teile) | behoben | Die Höchstgebühr wird angezeigt. Beim Auffüllen geht der geprüfte Betrag an ghostctl, nicht der Rohtext. Die Nachricht aus einem Code ist als „laut Tresor-Code“ gekennzeichnet. |
| A12-19 | behoben, soweit der Node es erlaubt | Jede Nachricht wird im Block am Node gegengeprüft, solange der Node den Block noch hat. Sonst steht „laut REST-API“ daneben. Abweichungen werden ausgeblendet und gemeldet, ebenso ein genannter Block ohne die Tx und ein unbekannter Block im Aufbewahrungsfenster. Ein- und Ausgänge werden nach dem Feld `index` verglichen; der ganze Ablauf bleibt unter der Wartezeit der Seite. |

## Einzelheiten

### A12-1 (mittel): untergeschobene Nachricht in echter Tresor-Zahlung

**Stand:** behoben, off-chain.

**Dateien:**
- `protocol/src/message.rs`: `inbox_entry_with`, `Origin`, `TresorCheck`, `tresor_payment`, `output_at`, `pays_me`, `check_tresor`, `inbox` (Ablauf von `ghostctl messages`). `inbox_entry` bleibt als Hülle, damit `rest_live_tests.rs` unverändert bleibt.
- `protocol/src/tresor.rs`: `redeem_script`, `parse_script`, `pay_input`.
- `protocol/src/bin/ghostctl.rs`: `messages` (nur noch Ein-/Ausgabe über `InboxNet` und die Ausgabe).
- `app/src/components/IncomingMessages.tsx`, `TresorList.tsx`.
- `app/src/lib/tresor.ts`: Hinweis beim Anlegen.
- `app/src/lib/api.ts`: nur Typen.
- `app/src/pages/Faq.tsx`.

**Erkennung:**
- Der letzte Push jedes Signaturskripts ist das Redeem-Skript.
- `parse_script` liest daraus die Parameter:
  - Befehl für Befehl gegen eine Referenzübersetzung.
  - Die Parameter stecken mit variabler Länge im Rumpf. Einige Befehle hängen von der Skriptlänge ab (Sprungweiten).
  - Danach wird die Form neu übersetzt und byte-genau verglichen. Ein fremdes Skript mit ähnlichem Aufbau gilt damit nie als Tresor.
- Nennt die REST-API die Adresse des ausgegebenen Outpoints, muss die P2SH-Adresse zum Skript passen.
- Das funktioniert auch für Tresore, die dieser Rechner nicht kennt.
- Tresor-Zahlung ist nur der Zweig `pay` (Nachprüfung A12-1-neu-a):
  - `tresor::pay_input`: Das Signaturskript ist genau der Selektor von `pay` und das Redeem-Skript, so wie `tresor::pay` es baut. `cancel` und `topUp` (Signatur des Besitzers, beliebige Ausgänge) laufen durch dasselbe Redeem-Skript, zählen aber nicht.
  - Am Index des Tresor-Eingangs steht der Ausgang an mich mit genau dem Betrag (`pay` verlangt das). Geprüft werden Skript und Adresse, soweit die API sie nennt, und das Feld `index` der Ausgänge.
  - Sonst: gewöhnliche Zahlung mit dem tatsächlichen Betrag an mich, bei Skript-Eingängen „über einen Vertrag“.

**Anzeige einer Tresor-Zahlung an mich:**
- Betrag laut Vertrag (= Ausgang an mich am Index des Tresors; ein eigenes Wechselgeld zählt nicht mit).
- „Von“ sind die Schlüssel-Adressen unter den Eingängen, ohne die eigene: bei Gebühr aus dem Tresor keine, sonst der Auslöser.
- Den Besitzer (Parameter `owner`) nennt der Eingang nur bei Tresoren, deren Code hier übernommen oder die hier angelegt wurden, und die Seite schreibt „Besitzer laut Tresor-Code“. `owner` ist nur ein Parameter des Skripts: Anlegen und Auslösen braucht keine Signatur des Besitzers (Nachprüfung A12-1-neu-b).
- Die Nachricht wird verglichen mit `TresorRec::payload()` aller Tresore in `deployments/<netz>-tresore.json` mit genau diesen Parametern, also mit `sealed` bzw. dem öffentlichen Klartext. „Wie hinterlegt“ zusätzlich nur, wenn der Text für diesen Schlüssel die Beschreibung `message` des Tresors ergibt.
- Ergebnis:
  - gleich: „wie im Tresor … hinterlegt“
  - abweichend: **„nicht vom Absender – beim Auslösen der Tresor-Zahlung eingefügt“** (Warnetikett)
  - Tresor nicht übernommen: „Tresor (Besitzer nicht geprüft)“ und „Tresor-Code hier nicht übernommen – Herkunft der Nachricht nicht prüfbar“
- Andere Skript-Eingänge bei KAS, auch Kündigen und Auffüllen eines Tresors: „über einen Vertrag – Herkunft der Nachricht nicht prüfbar“.

**Replay:** Wer die versiegelte Fassung in eine normale Sendung kopiert, bekommt kein „wie hinterlegt“. Ohne Tresor-Eingang gilt die Zahlung als direkt, Absender ist der Sender (Test).

**Allgemeine Aussage:**
- Eingang, Tresor-Liste, Anlegen-Hinweis, FAQ und `ghostctl messages` sagen, dass eine Nachricht nicht beweist, wer sie geschrieben hat, und dass den Besitzer eines Tresors jeder frei eintragen kann.
- Dass der Abgleich nur bei übernommenen Tresor-Codes greift, steht im Eingang, im Hinweis beim Anlegen, in der Tresor-Liste und in der FAQ.
- Die Spalte heißt jetzt „Von“ statt „Absender“.

**Tests (Rust):**
- `tresor_e2e_tests.rs`:
  - `a12_untergeschobene_nachricht_im_eingang_erkannt` (übernommen; vorher „ununterscheidbar“)
  - `a12_fremder_ausloeser_setzt_eigene_nachricht_und_behaelt_den_gebuehrenrest` (übernommen, jetzt mit Eingang)
  - `a12_oeffentliche_tresor_nachricht_geprueft`
- `tresor.rs`: `a12_tresor_am_skript_erkannt`. Drei Parametersätze. Einzelne Bytes verändert, gekürzt, GHOST-Token und Minter ergeben `None`.
- `message.rs`: `a12_herkunft_vertrag_und_direkt`.

**Tests (Seite):**
- `app/src/components/audit12a-messages.test.ts`, Block A12-1.
- `app/src/lib/audit12a-tresor.test.ts`, Hinweis „Vertrag bindet die Nachricht nicht“.
- `app/src/components/audit12a-tresor.test.ts`, „laut Tresor-Code“.

**Vertrag:** Für die Behebung ist keine Änderung nötig. Zum Bericht eine Ergänzung:
- Der Bericht sagt, der Payload ließe sich nur über eine Signatur des Absenders binden. Das stimmt nicht ganz.
- Die Skript-Engine hat `OpTxPayloadLen` (0xc4) und `OpTxPayloadSubstr` (0xb8), siehe rusty-kaspa `crypto/txscript/src/opcodes/mod.rs`.
- SilverScript übersetzt beide: `vendor/silverscript/silverscript-lang/tests/compiler_tests.rs`, Tests mit `require(OpTxPayloadSubstr(1, 3) == byte[]("ok"))`.
- Ein Parameter `byte[32] payloadHash` und in `pay()` ein `require(blake2b(OpTxPayloadSubstr(0, OpTxPayloadLen())) == payloadHash)` würde die Nachricht binden, ohne dass jemand signieren muss. Die längste Nachricht hat 464 Byte, also unter 520 Byte je Stapelelement.
- Das wäre eine Vertragsänderung für künftige Tresore, hier nicht gemacht. Die off-chain-Prüfung wäre dann nur noch Zweitsicherung.

### A12-1 im Vertrag (30.09.2026)

**Stand:** behoben im Vertrag. Branch `fix15a`, Basis `d21e9a8`.

Der Vertrag bekam im Commit `c046461` den Parameter `byte[32] payloadHash` (nach `maxFee`, vor `initNextDue`); `pay()` verlangt `sha256(OpTxPayloadSubstr(0, OpTxPayloadLen())) == payloadHash` (Engine-Tests dort: `nachricht_ist_an_jede_zahlung_gebunden`, `ohne_nachricht_bleibt_der_payload_leer`, `laengste_nachricht_passt`). Diese Runde bringt Rust-Seite, Eingang, Seite und Doku in Einklang. Der Vertrag selbst ist hier unverändert.

**Rust (`protocol/src/contracts.rs`, `tresor.rs`, `message.rs`, `bin/ghostctl.rs`):**
- `TresorParams::payload_hash` (32 Byte, serde Hex `payloadHash`); `contracts::payload_hash` = sha256 wie im Vertrag; `standing_order()` übergibt ihn an der richtigen Stelle.
- Anlegen: `tresor::bind_message` setzt den Hash genau auf den Payload jeder Zahlung. Öffentlich ist das der Klartext, verschlüsselt die einmal erzeugte `sealed`-Fassung, ohne Nachricht der leere Payload. `ghostctl tresor open` nutzt es.
- `check_params` und der Code-Import verlangen 32 Byte. Der Import prüft außerdem mit `check_bound`, dass der Hash zu `message`/`onchain` bzw. `sealed` passt; sonst wird der Code abgelehnt, bevor der Node gefragt wird.
- `TresorShape`, `layout`/`parse_script` kennen den neuen 32-Byte-Parameter (Slots: 3 Byte-Parameter, 4 Zahlen, längenabhängige Stellen). Die Gegenprobe über die Neuübersetzung bleibt.
- `tresor::pay` lehnt jeden Payload ab, dessen Hash nicht passt. `pay_round` nimmt `TresorRec::payload()` (= `bound_payload`). Ein Auslöser über ghostctl kann also keinen anderen Payload setzen, der Vertrag lehnt ihn ohnehin ab.
- Tresor-Code Version 2 (`ghost-tresor:2:`). Codes `ghost-tresor:1:` werden mit eigener Meldung abgelehnt (alter Vertrag, im Mainnet gab es noch keine Tresore). Tresor-Dateien mit Tresoren ohne `payloadHash` lehnt `tresor::load` klar ab; eine leere alte Datei geht (Version 2).
- Eingang: `TresorCheck::Bound` = der Payload passt zum Hash im Vertrag. Anzeige: „vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)“, auch ohne übernommenen Tresor-Code (dann „Besitzer nicht geprüft“). `AsStored` bleibt für übernommene Tresore, deren Beschreibung der Text ist. Beschreibt der Code die Nachricht anders, heißt es `Bound` mit dem Zusatz „anders als die Beschreibung im Tresor-Code“. `Inserted` („passt NICHT zur im Vertrag gebundenen Nachricht“) gibt es nur noch, wenn der Payload nicht zum Hash passt. Das kann nur aus falschen REST-Daten kommen; der Node-Abgleich blendet solche ohnehin aus. `Unknown` entfällt.

**Seite:** Hinweis beim Anlegen: die Nachricht ist fest gebunden, niemand ändert sie, auch der Absender nachträglich nicht (nur neuer Tresor); ohne Nachricht bleibt jede Zahlung ohne Text. Die Bestätigung nennt die gebundene Nachricht. Eingang mit `tresor-bound` statt `tresor-unknown`, neue Texte. Tresor-Liste für Empfänger und Absender. FAQ: Antwort zu Nachrichten umgeschrieben, neue Frage „Kann ich die Nachricht eines Tresors später ändern?“. Tresor-Code v2 in Seite (`codeProblem`) und Server (`tresor-import`), alte Codes mit eigener Meldung.

**Doku:** `ARCHITEKTUR.md` und `MAINNET.md`, Abschnitt „Dauerauftrag mit Tresor“.

**Tests (Rust):**
- `tresor_e2e_tests.rs`:
  - `a12_untergeschobene_nachricht_im_eingang_erkannt` (umgeschrieben): Eine fremde Nachricht lehnen `tresor::pay` und der Vertrag ab; von Hand gebaut scheitern auch der leere Payload und ein Byte mehr. Die hinterlegte Fassung geht, auch von Hand gebaut. Ohne übernommenen Code: `Bound`. Eine falsche REST-Antwort ergibt `Inserted`.
  - `a12_fremder_ausloeser_kann_keine_eigene_nachricht_setzen_behaelt_aber_den_gebuehrenrest` (vorher `…_setzt_eigene_nachricht_…`): Ein fremder Payload wird abgelehnt. Mit dem gebundenen Payload zahlt der Auslöser und behält den Gebührenrest (A12-8 bleibt).
  - `a12_ohne_nachricht_bleibt_jede_zahlung_ohne_text`: Hash des leeren Payloads, der Payload „Hallo“ wird abgelehnt, die Zahlung ist ohne Payload, kein Eintrag im Eingang.
  - `verschluesselte_nachricht_in_jeder_zahlung_nur_fuer_den_empfaenger`: der Empfänger liest, der Absender nicht, zweite Zahlung gleiche Fassung.
  - `a13_tresor_code_v2_hin_und_zurueck`: öffentlich, verschlüsselt, ohne Nachricht; Import und Zahlung am Simulator.
  - `a13_alter_tresor_code_abgelehnt`, `a13_import_mit_falschem_hash_abgelehnt` (andere verschlüsselte Fassung desselben Textes, Fassung entfernt, als öffentlich ausgegeben, Hash verändert, andere öffentliche Beschreibung), `a13_alte_tresor_datei_abgelehnt`.
  - `skript_ohne_kompilieren_stimmt` mit `payloadHash` (öffentlich, verschlüsselt; anderer Hash ergibt ein anderes Skript; `parse_script` liest ihn zurück). `anlegen_prueft_die_eingaben`: Hash mit 31 und 0 Byte abgelehnt.
  - Übrige Tests auf Tresore mit gebundener Nachricht umgestellt (`World::public`, `World::sealed`). `a12n_fremder_besitzer_nicht_als_absender`: Die Nachricht des Dritten ist die beim Anlegen gebundene (`Bound`), Alice wird nicht genannt. `a12n_hinterlegt_heisst_auch_derselbe_text`: Der Code passt zum Vertrag, nur die Beschreibung lügt, daher `Bound` statt „wie im Code“.
- `tresor.rs`: `a13_nachricht_beim_anlegen_gebunden` (neu); `code_ablehnen` und `a13_alte_codes_…` setzen den passenden Hash, damit sie an der Beschreibung scheitern und nicht am Hash; `a12n_laengenabhaengige_stellen_…` auf die neue Slot-Nummer.
- `message.rs`: `a12p_altdaten_…` (andere Nachricht nur über falsche REST-Daten, abweichende Beschreibung ergibt `Bound`).
- `ghostctl.rs`: `a13_eingang_nennt_die_gebundene_nachricht` (Terminalzeile und JSON `tresor-stored`/`tresor-bound`/`tresor-inserted`).

**Tests (Seite):** `audit12a-messages.test.ts` (gebundene Nachricht ohne Code, abweichende Beschreibung, allgemeiner Hinweis), `lib/audit12a-tresor.test.ts` (Hinweise mit und ohne Nachricht), `components/audit12a-tresor.test.ts` (Bestätigung nennt die Nachricht, Liste, Code-Feld v2), `lib/tresor.test.ts` (alter Code), `server/actions.test.ts` (alter Code), `pages/audit12a-faq.test.ts` (Antwort und neue Frage).

**Rückbauproben (Rust; je zwei Rückbauten in einem Lauf, danach die Dateien zurückgesetzt, `git diff` leer):**

| Rückbau | rot |
|---|---|
| Hash-Prüfung in `tresor::pay` entfernt | `tresor::a13_nachricht_beim_anlegen_gebunden` |
| `Bound` immer wahr in `check_tresor` | `message::a12p_altdaten_…` |
| `check_bound` beim Import entfernt | `tresor_e2e::a13_import_mit_falschem_hash_abgelehnt` |
| Ablehnung alter Codes entfernt | `tresor_e2e::a13_alter_tresor_code_abgelehnt` |

**Nebenbefund:** `vault_tests::a12_tresor_zahlung_an_die_kasse_deckt_zugleich_den_zins` übersetzt den Tresor-Vertrag selbst und übergab noch 8 Argumente. An der Basis `d21e9a8` war er deshalb rot („constructor argument count mismatch: expected 9, got 8“). Jetzt übergibt er den Hash des leeren Payloads, weil die Tx dort keinen Payload hat.

**Testzahlen (A12-1 im Vertrag):**
- Rust: `cargo test --release --offline` jetzt 376 grün, 2 ignoriert, 0 rot. Neu sind 7 Tests: 1 in `tresor.rs`, 1 in `ghostctl.rs`, 5 in `tresor_e2e_tests.rs`. An der Basis waren es 369, davon einer rot (siehe Nebenbefund).
  - Aufteilung: lib 79, ghostctl 17, standing_order 21, tresor_e2e 32, vault 86, übrige unverändert.
- Seite: `npx vitest run` 416 grün in 21 Dateien (vorher 411). Neu sind 5: +1 `audit12a-messages`, +3 `components/audit12a-tresor`, +1 `audit12a-faq`. Umgeschrieben wurden die Tests zu Hinweis, Code v2 und Server.
- `npx tsc --noEmit -p .` fehlerfrei; `npm run build` fehlerfrei, nur die bekannte Warnung zur Chunk-Größe.

### A12-6: Probelauf nicht ans Netz gebunden, Bestätigung unvollständig

**Stand:** behoben.

**Datei:** `app/src/components/StandingOrders.tsx`.

**Änderung:**
- Der Schlüssel der Prüfung ist `JSON.stringify({ network, params })`.
- Beim Wechsel von Netz oder Schlüssel wird die Prüfung verworfen.
- Die Bestätigung nimmt die Beschreibung aus dem Probelauf von ghostctl (`r.tresor`). Sie nennt:
  - das Netz im Titel, im Mainnet als Warnung
  - Startguthaben und Netzgebühr
  - Empfängeradresse
  - Betrag je Zahlung
  - Intervall
  - Anzahl („12-mal“ bzw. „ohne Ende“)
  - ersten Termin in Ortszeit und UTC
  - Höchstgebühr je Zahlung

**Tests:** `app/src/components/audit12a-tresor.test.ts`, Block A12-6. Eine im Testnetz geprüfte Anfrage führt im Mainnet wieder zu „Tresor prüfen“, mit altem und neuem Schlüssel. Im selben Netz stehen alle Angaben in der Bestätigung.

### A12-7: „Zahlung fällig“, obwohl ghostctl ohne 1 KAS Rest nicht zahlt

**Stand:** behoben.

**Dateien:**
- `protocol/src/tresor.rs`: `fee_from_tresor`, `payable`, `due_now(…, with_key)`, `looks_due`.
- `protocol/src/bin/ghostctl.rs`: `due_now` mit `k.is_some()`.
- `app/src/lib/tresor.ts`: `feeSource`, `tresorStatus`, `TRESOR_MIN_KEEP`.
- `app/src/components/TresorList.tsx`.

**Änderung:**
- Eine gemeinsame Regel wie `pay`:
  - Ohne eigenen Schlüssel nur bei `Wert − Betrag − Höchstgebühr ≥ 1 KAS`.
  - Mit eigenem Schlüssel bei `Wert − Betrag ≥ 1 KAS`.
- `due_now` sagte vorher schon bei `Wert − Betrag − Höchstgebühr > 0` „fällig“.
- Auf der Seite:
  - „Guthaben aufgebraucht“ nur, wenn auch der Vertrag nicht mehr zahlen lässt (`Wert − Betrag − Höchstgebühr ≤ 0`, `contractPayable`).
  - „Guthaben knapp“, wenn ghostctl nicht mehr zahlt, ein fremder Auslöser aber noch könnte (Nachprüfung A12-8).
  - „fällig“ mit dem Zusatz „nur mit eigener Gebühr“, wenn ghostctl nur mit eigenem Schlüssel zahlt. Für den Absender „bitte auffüllen“.

**Tests:**
- `tresor.rs`: `a12_faellig_nur_mit_einem_kas_rest`.
- `tresor_e2e_tests.rs`: `a12_gebuehr_vom_eigenen_schluessel_…` prüft `due_now` mit und ohne Schlüssel.
- `app/src/lib/audit12a-tresor.test.ts`, Block A12-7.
- `app/src/components/audit12a-tresor.test.ts`, Block A12-7: 10,5 KAS für 10 KAS ergeben keinen Knopf.
- Der bestehende Test in `tresor.test.ts` hielt das alte Verhalten fest (10,01000001 KAS galt als „fällig“) und ist angepasst; seit der Nachprüfung erwartet er für 10,99999999 KAS „knapp“ statt „aufgebraucht“.

### A12-8: Rest der Höchstgebühr darf der Auslöser behalten

**Stand:** dokumentiert. Die Texte stimmen jetzt mit dem Verhalten überein. Eine Vertragsänderung ist nicht sinnvoll, siehe unten.

**Dateien:**
- `app/src/lib/tresor.ts` (`tresorHints`)
- `app/src/components/TresorList.tsx` (neue Zeile „Höchstgebühr je Zahlung“)
- `app/src/pages/Faq.tsx`
- `protocol/src/tresor.rs` (Modulkopf, `DEFAULT_MAX_FEE`)
- `protocol/src/bin/ghostctl.rs` (Hilfe zu `--max-fee` und `tresor pay`)

**Texte:**
- Vorher: „Empfänger oder Agent“ und „Netzgebühr (höchstens 0,01 KAS) kommt aus dem Tresor“.
- Jetzt:
  - „Zum Termin darf jeder die Zahlung auslösen …“
  - „Je Zahlung gehen der Betrag und höchstens 0,01 KAS Höchstgebühr aus dem Tresor. Was davon nicht als Netzgebühr gebraucht wird, darf ein fremder Auslöser behalten; der GHOST-Agent und diese Seite lassen es im Tresor.“
  - Nachprüfung: Die 1 KAS Rest sind eine Regel von ghostctl und der Seite. Der Vertrag verlangt nach Betrag und Höchstgebühr nur einen Rest über 0; ein fremder Auslöser kann also auch mit weniger Rest zahlen (10,5 KAS für 10 KAS: 0,49 KAS bleiben). FAQ, Tresor-Liste, `MIN_KEEP` in `tresor.rs` und die Hilfe zu `tresor pay` sagen das jetzt so.

**Tests:**
- `app/src/lib/audit12a-tresor.test.ts`, Block A12-8.
- `standing_order_tests.rs`: `a12_ausloeser_darf_den_rest_der_hoechstgebuehr_behalten` (Vertragsverhalten, übernommen).
- `tresor_e2e_tests.rs`: `a12_fremder_ausloeser_…` misst den Gewinn: 0,00624 KAS bei 0,01 KAS Höchstgebühr.

**Warum keine Vertragsänderung:**
- Der Vertrag kann die tatsächliche Netzgebühr nicht kennen, denn die Masse ist nicht einsehbar.
- Man könnte genau zwei Ausgänge und keinen fremden Eingang verlangen. Dann ginge der Rest an die Miner statt an den Auslöser. Das hilft dem Absender nicht und verhindert zugleich das Auslösen mit eigener Gebühr.
- Der Schaden ist auf `maxFee` je Zahlung begrenzt (Standard 0,01 KAS, höchstens 0,1 KAS). Der Vorschlag für das Startguthaben rechnet ihn bereits ein.

### A12-9: Termine 00:00 UTC, Anzeige lokal ohne Datum bzw. mit falscher Uhrzeit

**Stand:** behoben.

**Dateien:**
- `app/src/lib/tresor.ts`: `dueMs`, `localDateTime`, `utcDateTime`. `dueLocalTime` ist entfallen.
- `app/src/components/StandingOrders.tsx`.
- `app/src/components/TresorList.tsx`.

**Änderung:**
- Beim Anlegen:
  - Die Vorschau „Nächste Termine (deine Ortszeit)“ zeigt je Termin Datum und Uhrzeit, in New York also „30.01.2031, 19:00 · … · 30.03.2031, 20:00“.
  - Der Hinweis nennt 00:00 UTC, den ersten Termin in Ortszeit mit Datum und die Zeitumstellung.
- In der Tresor-Liste steht beim nächsten Termin Ortszeit und UTC.
- Die Erfolgsmeldung nennt den ersten Termin in beiden Zeiten.

**Tests:**
- `app/src/components/audit12a-tresor.test.ts`, Block A12-9: New York, Berlin Winter und Sommer, Liste. Setzt `TZ` zur Laufzeit.
- `app/src/lib/audit12a-tresor.test.ts`, Block A12-9.

### A12-10: „Fällige Zahlung abholen“ ohne Rückfrage, eigene Gebühr still

**Stand:** behoben.

**Dateien:**
- `app/src/components/TresorList.tsx`
- `app/src/lib/tresor.ts` (`payParams`)
- `protocol/src/message.rs` (Eingang)

**Änderung:**
- Abholen geht jetzt zweistufig, in jedem Netz. Die Rückfrage nennt Betrag, Empfänger und wer die Gebühr zahlt: „trägt der Tresor“ bzw. „zahlt dein Schlüssel …“.
- Der eigene Schlüssel geht nur an `tresor pay`, wenn der Tresor die Gebühr nicht mehr trägt.
  - Vorher ging er immer mit. Bei einer Gebührenspitze zahlte ghostctl dann still vom eigenen Schlüssel.
  - Ohne Schlüsseldatei erscheint in diesem Fall kein Knopf, sondern ein Hinweis.
- Eingang: Eine Tresor-Zahlung an mich mit eigenem Eingang (ich habe die Gebühr gezahlt) zählt nicht mehr als eigene Sendung. Betrag ist der Vertragsbetrag, ohne mein Wechselgeld.

**Tests:**
- `tresor_e2e_tests.rs`: `a12_gebuehr_vom_eigenen_schluessel_nachricht_bleibt_im_eingang`. Vorher ergab dieser Fall `None`.
- `app/src/lib/audit12a-tresor.test.ts`, Block A12-10 (`payParams`).
- `app/src/components/audit12a-tresor.test.ts`, Block A12-10.

### A12-11: Nachrichtenfilter lässt unsichtbare Zeichen durch

**Stand:** behoben.

**Dateien:**
- `protocol/src/abo.rs`: `BAD_CHARS`, `is_bad_char`, `check_message`.
- `app/src/lib/abo.ts`: `hasBadChar`, `messageProblem`.
- `app/server/actions.ts`: `hasBadMessageChar`, `checkMessage`.
- Neu: `protocol/tests/data/nachrichtenfilter.json`, die gemeinsame Fallsammlung.

**Auswahl und Begründung:**

Abgelehnt werden:
- **Cc**, die Steuerzeichen, wie bisher.
- **Cf** (Formatzeichen) vollständig:
  - Richtungswechsel und Isolates, Null-Breite einschließlich ZWJ/ZWNJ (in einer einzeiligen Zahlungsnachricht entbehrlich), Soft Hyphen, Word Joiner, Invisible Operators.
  - Tag-Zeichen U+E0001 und U+E0020–E007F: unsichtbarer ASCII-Text.
  - U+180E, U+FFF9–FFFB, arabische und ägyptische Formatzeichen u. a.
- **Default_Ignorable_Code_Point**, von Unicode selbst als „standardmäßig unsichtbar“ markiert:
  - U+034F (CGJ)
  - Hangul-Füller U+115F/1160/3164/FFA0
  - Khmer U+17B4/17B5
  - alle Variation Selectors (U+180B–180F, U+FE00–FE0F, U+E0100–E01EF)
  - der reservierte Bereich U+E0000–E0FFF
- **Zl/Zp**: U+2028, U+2029.
- **Co** (Private Use): U+E000–F8FF und die Ebenen 15/16. Diese Zeichen haben keine festgelegte Gestalt, je nach Schrift unsichtbar oder ein beliebiges Bild (z. B. U+F8FF).
- **Nichtzeichen**: U+FDD0–FDEF und U+xFFFE/xFFFF.
- Auf der Seite zusätzlich einzelne Surrogate. In Rust-Text gibt es sie nicht.

Erlaubt bleibt, was normale Texte brauchen:
- Buchstaben aller Schriften mit kombinierenden Akzenten, auch Arabisch und Hebräisch als Buchstaben.
- Leerzeichen einschließlich geschütztem Leerzeichen, Satzzeichen, Emoji mit Hautfarbe.
- VS16/VS15 (U+FE0F/FE0E) **direkt hinter einem Bildzeichen**, sonst gingen ❤️, ☺️, ✔️ nicht.
  - Hinter Buchstaben oder Ziffern, am Anfang oder doppelt wäre der Selector unsichtbar und bleibt verboten.
  - Nebenwirkung: Tastenkappen wie 1️⃣ und ZWJ-Emoji wie Familien gehen nicht.

**Gleichlauf:**
- Rust, Seite und Server haben dieselbe Tabelle.
- `nachrichtenfilter.json` enthält 59 Fälle und die abgelehnten Bereiche als Liste.
- Rust prüft alle 1 114 112 Codepunkte gegen diese Liste. Die Seite prüft Ebene 0, 1, 14, alle Grenzen und Stichproben.
- Die Seite prüft die Liste zusätzlich gegen die Unicode-Eigenschaften ihrer Engine: Node 25 mit Unicode 17.0, genau gleich.
- Kommt mit einer neuen Unicode-Version ein neues Formatzeichen hinzu, schlägt dieser Test an.

**Folgen für Bestehendes (Nachprüfung):**
- Gespeicherte Daueraufträge wurden mit dem schwächeren Filter angelegt. Vorher scheiterte jede Ausführung – die Zahlung selbst – an einem Tastenkappen-Emoji, U+2028, Private Use oder VS16 hinter einem Nicht-Bildzeichen.
  - Jetzt sendet `abo::payload` die Nachricht nach heutigem Filter (`abo::sendable_message`): verbotene Zeichen fallen weg, verbotener Leerraum wird zum Leerzeichen. Bleibt nichts übrig, geht die Zahlung ohne Nachricht.
  - Die Seite zeigt beim Auftrag, was gesendet wird (`sendableMessage` in `abo.ts`, gleiche Regeln).
- Eingehende Nachrichten: Text, den der Filter ablehnt (öffentlich oder entschlüsselt), ergibt `Found::Invalid`. Die Zahlung bleibt im Eingang, samt Herkunft („eingefügt“ bei übernommenen Tresoren), nur der Text wird nicht gezeigt. Vorher verschwand sie über `read → None` ganz.
- Die Beschreibung im Tresor-Code prüft der Import weiterhin streng; ein alter Code mit solchen Zeichen wird abgelehnt. (Überholt: seit den Restpunkten übernimmt der Import solche Codes, siehe dort.)

**Tests:**
- `payload_tests.rs`: `a12_nachrichtenfilter_gleiche_faelle_wie_die_seite` und `a12_nachrichtenfilter_alle_codepunkte`. Gegen das alte `abo.rs` sind beide rot: U+2028 wurde angenommen, abgelehnt wurden nur 8 Bereiche.
- `app/src/lib/audit12a-nachrichten.test.ts` (63 Tests). Gegen den alten Stand scheitern 29 der 59 Fälle, dazu der Fehlertext und die Codepunkt-Prüfungen.

### A12-16: Tresor-Automatik und Import

**Stand:** behoben.

**Dateien:**
- `protocol/src/tresor.rs`: `TresorCode::decode`, `MISSING_RECHECK_MS`, `MISSING_RECHECK_FOR_MS`, `MISSING_RECHECK_FOLLOW`, `MAX_IMPORT_BACKLOG`, `missing_ms`, `recheck_missing`, `looks_due`, `continue_after_send_error`, `candidates_max`, `locate_io`, `backlog`, `check_backlog`. Neu seit der Nachprüfung die Abläufe `follow`, `import`, `pay_round` über die Schnittstelle `TresorIo`.
- `protocol/src/bin/ghostctl.rs`: `TresorNet` (Ein-/Ausgabe über Node, Journal, Terminal), `TresorCmd::Import`, `Sync`, `Pay`, `Topup`, `Cancel` rufen die Abläufe auf. `tresor_follow` und `tresor::locate_at_node` sind entfallen.
- `app/server/actions.ts`: `tresorNeedsRun`.

**Import:**
- `decode` prüft wie `check_params`. Nur der Zustand darf fortgeschritten sein (Anzahl 0, Termin nicht mehr am Starttag). Verlangt werden:
  - Betrag ≥ 1 KAS
  - Höchstgebühr zwischen 1 sompi und 0,1 KAS
  - Intervall monatlich 1–31 oder 1 bis 3650 Tage
  - Absender ≠ Empfänger
  - Termin bis 2200
- Ein präparierter Code mit 1 ms Intervall kostete vorher bis zu 2000 Node-Abfragen je Abgleich. Jetzt höchstens eine je erreichtem Tag.
- Nachprüfung: `decode` prüft den Termin nur nach unten (1985). Ein echter Tresor mit Termin 1985 und täglichem Intervall ging durch. `tresor::import` lehnt nach dem Auffinden einen Rückstand über `MAX_IMPORT_BACKLOG` = 400 erreichte, nicht gezahlte Termine ab (gut ein Jahr täglicher Zahlungen). Ein regulär bedienter Tresor hat so einen Rückstand nicht.

**Sendefehler:**
- Vorher beendete `break` die ganze Runde.
- Jetzt geht es mit dem nächsten Tresor weiter, außer das Journal ist noch offen (die Tx ist vielleicht im Netz) oder der Nutzer hat abgebrochen.
- Der häufige Fall, dass ein anderer Auslöser schneller war, räumt das Journal selbst ab: `send_to` löscht es, wenn die Tx sicher nicht im Mempool ist.

**„missing“:**
- Die Automatik sucht einen nicht auffindbaren Tresor eine Woche lang stündlich erneut: `retry_after`, neues Feld `missing_ms`.
- Nachprüfung: Dabei sucht sie nur über `MISSING_RECHECK_FOLLOW` = 32 Zustände ab dem letzten bekannten (`Search::Auto`). Die volle Suche (2000) läuft beim ersten Fehlen und beim Abgleich von Hand (`Search::Full`). Eine Woche erneutes Nachsehen kostet so höchstens 168 × 32 = 5 376 statt 336 000 Abfragen.
- Gemeldet wird nur beim ersten Mal.
- Wird der Tresor wieder gefunden, fallen Markierung und Wartezeit weg.
- Ältere Dateien ohne `missing_ms` werden einmal nachgesehen, damit beginnt das Fenster.
- `tresorNeedsRun` auf dem Server folgt derselben Regel.

**Tests:**
- `tresor.rs`: `a12_import_prueft_wie_das_anlegen`. Ersetzt den Belegtest `import_grenzen_und_nachfuehr_aufwand` der Prüfung, der noch `decode(1 ms …) == Ok` festhielt.
- `tresor.rs`: `a12_nicht_auffindbar_wird_wieder_gesucht` und `a12_weiter_nach_sendefehler`.
- `app/server/actions.test.ts`: Test zu `tresorNeedsRun` angepasst. Er hielt vorher „missing = nie“ fest.
- `faellig_offline` in `tresor.rs` ist ebenso angepasst.

**Schleife:** Seit der Nachprüfung steht die Runde (`pay_round`) samt Auswahl, Nachführen und Fehlerpfad in der Bibliothek und läuft in `tresor_e2e_tests.rs` gegen den Simulator (`SimIo`). Ungetestet bleibt nur die Ein-/Ausgabe in ghostctl (`TresorNet`), siehe Nachprüfung.

### A12-18 (Tresor-Teile)

**Stand:** behoben.

**Dateien:** `app/src/components/TresorList.tsx` und `app/src/lib/tresor.ts` (`topupKas`, `payParams`).

**Änderung:**
- Neue Zeile „Höchstgebühr je Zahlung“ mit dem Hinweis auf den Rest (A12-8).
- Beim Auffüllen gehen `cliDecimal(parseUnits(text))` an ghostctl und in die Rückfrage, nicht `add.trim().replace(",", ".")`. Aus „1.000,5“ wird so 1000,5 KAS; vorher lehnte der Server die Eingabe ab.
- Die Nachricht eines übernommenen Tresors (ohne eigene Schlüsseldatei) trägt das Etikett „laut Tresor-Code“. Dazu der Hinweis, dass der Auslöser die Nachricht bestimmt.

**Tests:**
- `app/src/lib/audit12a-tresor.test.ts`, Block A12-18. Seite und Server lesen denselben Betrag.
- `app/src/components/audit12a-tresor.test.ts`, Block A12-18.

**Nicht in dieser Gruppe:** die Warnung vor einem winzigen Ausgang bei der Rücknahme (Vault).

### A12-19: Eingang glaubt der REST-API

**Stand:** behoben, soweit ein Node es erlaubt. Der Rest ist gekennzeichnet.

**Dateien:**
- `protocol/src/message.rs`: `NodeTx`, `same_as_node`, `Source`.
- `protocol/src/bin/ghostctl.rs`: `node_tx`, `messages`, neue JSON-Felder `source`, `nodeChecked`, `hidden`, `checks`.
- `app/src/components/IncomingMessages.tsx`.
- `app/src/lib/api.ts`: nur Typen.

**Änderung:**
- Der Ablauf steht seit der Nachprüfung in `message::inbox` (Schnittstelle `InboxIo`); ghostctl liefert REST-API und Node (`InboxNet`).
- `ghostctl messages` verbindet sich, wenn möglich, mit einem Node. Zeitlimit 30 s.
- Für die neuesten 30 Nachrichten, zusammen höchstens 60 s (seit der zweiten Nachprüfung samt Verbinden und Aufbewahrungsfenster, siehe dort):
  - Den Block, den die REST-API nennt, mit `get_block` holen.
  - Die Tx darin suchen.
  - Payload, alle Signaturskripte (daraus die Tresor-Erkennung) sowie Beträge und Skripte aller Ausgänge vergleichen.
- Ergebnis (`message::node_verdict`):
  - gleich: „am Node geprüft“
  - abweichend: Nachricht ausgeblendet, die Seite meldet „… ausgeblendet“
  - Nachprüfung: ein genannter Block liegt vollständig am Node, die Tx steht aber nicht darin: ausgeblendet
  - Nachprüfung: der Node kennt keinen der genannten Blöcke („cannot find header“), obwohl die Blockzeit laut REST-API mehr als 1 h nach seinem Pruning-Punkt und mehr als 1 h vor seiner Past Median Time liegt: ausgeblendet
  - Block nicht (mehr) am Node (Inhalt gelöscht, nur der Kopf), älter oder zu frisch, Fehler, oder kein Node erreichbar: „laut REST-API“, dazu ein Hinweis unter der Tabelle

**Grenzen, auf der Seite genannt:**
- Kaspa-Nodes führen keinen Tx-Index. Eine Tx ist nur über ihren Block abrufbar.
- Blockinhalte behält ein Node 30 bis 42 Stunden (Pruning-Tiefe 1 080 000 Blöcke = 30 h, der Pruning-Punkt rückt in Schritten der Finalitätstiefe von 12 h vor; rusty-kaspa `a41a333`, `consensus/src/processes/pruning.rs`). Gemessen an api.kaspa.org: 1 332 369 Blöcke ≈ 37 h bei 10 BPS.
- Ob die Tx angenommen wurde (`is_accepted`) und welche Adressen die Eingänge haben, sagt der Block nicht. Das bleibt Angabe der REST-API.
- Eine Prüfung der Annahme wäre nur über die Akzeptanzdaten möglich (`get_virtual_chain_from_block` bzw. `get_utxo_return_address` im Aufbewahrungsfenster). Das ist deutlich mehr Aufwand und hier nicht gemacht.

**Live geprüft (lesend, Mainnet):** Tx `97b14d0d…` über die REST-API, Block `a54b728d…` am Node: gleich. Mit einem Byte mehr im Payload: ungleich.

**Tests:**
- `message.rs`: `a12_abgleich_mit_dem_node`.
- `app/src/components/audit12a-messages.test.ts`, Block A12-19.

## Nachprüfung (Prüfer 12a)

Der Prüfer hat die erste Behebung beanstandet (Stand `5aea635`). Seine Belegtests aus `scratchpad/pruef12a/zz_pruefer12a.rs` sind übernommen und auf die Behebung umgeschrieben. Jede Behebung wurde zurückgebaut und der zugehörige Test dabei rot gesehen (Rückbauproben unten).

| ID | Stufe | Stand |
|---|---|---|
| A12-1-neu-a | blockierend | behoben |
| A12-1-neu-b | blockierend | behoben |
| A12-16-rest | wichtig | behoben |
| Testlücke Seite | wichtig | behoben |
| Testlücke ghostctl | wichtig | behoben, bis auf die reine Ein-/Ausgabe (unten) |
| A12-8-Texte | wichtig | behoben |
| A12-1-Hinweise | klein | behoben |
| A12-11-Folgen | klein | behoben |
| A12-19-Grenze | klein | behoben, soweit der Node es erkennen lässt |

### A12-1-neu-a: falscher Betrag bei Kündigen und Auffüllen

**Befund zutreffend.** Jede Tx mit standing_order-Eingang und Empfänger = ich galt als Tresor-Zahlung über `amount`. `cancel` und `topUp` laufen mit der Signatur des Besitzers durch dasselbe Redeem-Skript und dürfen beliebige Ausgänge haben: Überzahlungs-Masche mit 0,3 KAS, die als 10 000 KAS „wie hinterlegt“ erschienen.

**Behebung** (`message.rs` `tresor_payment`, `tresor.rs` `pay_input`):
- Tresor-Zahlung nur, wenn das Signaturskript genau Selektor von `pay` + Redeem-Skript ist.
- Und am Index des Tresor-Eingangs steht der Ausgang an mich mit genau `amount` (Skript und Adresse, soweit die API sie nennt; Feld `index`).
- Sonst gewöhnlicher Pfad: tatsächliche Summe an mich, „über einen Vertrag – Herkunft nicht prüfbar“.

**Tests:**
- `tresor_e2e_tests.rs`:
  - `a12n_kuendigen_und_auffuellen_sind_keine_tresor_zahlung`: `cancel` und `topUp` mit 0,5 KAS an den Empfänger, dazu `cancel` mit genau dem Vertragsbetrag an Ausgang 0 (dort greift nur die Selektorprüfung).
  - `a12n_ueberzahlungs_masche_per_kuendigung`: Beleg des Prüfers, 0,3 KAS statt 10 000 KAS.
  - `a12n_tresor_zahlung_nur_mit_ausgang_an_mich`: REST-Antwort mit anderem Ausgang oder Betrag; Ausgänge mit Feld `index` in anderer Reihenfolge.
- `tresor.rs`: `a12n_nur_der_zweig_pay`.

### A12-1-neu-b: „Tresor von <Adresse>“ fälschbar

**Befund zutreffend.** `owner` ist nur ein Parameter des Skripts; einen Tresor mit Besitzer = Alice kann jeder anlegen und auslösen.

**Behebung:**
- `Origin::Tresor.owner` ist jetzt `Option`: nur bei Tresoren aus `<netz>-tresore.json` (Code übernommen oder hier angelegt).
- `from` sind wieder nur die Schlüssel-Adressen unter den Eingängen (ohne die eigene), nie der Besitzer.
- `ghostctl messages` liefert `tresorOwner` nur bei bekannten Tresoren.
- Die Seite schreibt:
  - bei übernommenem Tresor „Tresor <id>“ und „Besitzer laut Tresor-Code <Adresse>“,
  - sonst „Tresor (Besitzer nicht geprüft)“,
  - eigene Eingänge des Auslösers unter „weitere Eingänge“.
- Die Tresor-Liste setzt beim Besitzer eines übernommenen Tresors „(laut Tresor-Code)“ dazu und sagt, dass den Besitzer jeder frei eintragen kann.
- Die Texte „„Von“ zeigt nur, von welchen Adressen die Zahlung kam“ (Seite), FAQ und ghostctl-Hinweis stimmen damit wieder.
- Der ghostctl-Hinweis nennt jetzt den Besitzer ausdrücklich.

**Tests:**
- `tresor_e2e_tests.rs`: `a12n_fremder_besitzer_nicht_als_absender` (Beleg des Prüfers). Die A12-Tests aus der ersten Runde hielten „von = Besitzer“ fest und sind angepasst.
- `audit12a-messages.test.ts`: „Besitzer nur bei übernommenem Tresor …“, auch wenn ein älteres ghostctl `tresorOwner` mitschickt.
- `audit12a-tresor.test.ts`: „Empfänger sieht den Besitzer mit Herkunft der Angabe“.

### A12-16-rest: 2000 Node-Abfragen je erneutem Nachsehen

**Befund zutreffend.** `decode` prüft den Termin nur nach unten.

**Behebung, beides:**
- Die Automatik sucht einen fehlenden Tresor nur über `MISSING_RECHECK_FOLLOW` = 32 Zustände (`tresor::follow` mit `Search::Auto`).
- `tresor::import` lehnt einen Rückstand über `MAX_IMPORT_BACKLOG` = 400 Termine ab.
- Der Abgleich von Hand sucht weiter vollständig. Nur so findet er einen Tresor wieder, der während einer Abwesenheit viele Zahlungen weiter ist.

**Tests (`tresor.rs`):**
- `a12n_fehlender_tresor_automatik_sucht_begrenzt`: Die Abfragen werden gezählt, erst 2000, dann 32, von Hand 2000; wiedergefunden nach 1.
- `a12n_import_mit_altem_termin_abgelehnt`: Beleg des Prüfers (Termin 1985, täglich); Grenze 400/401; regulärer Import, anderes Netz.

**Nicht geändert:** `tresor::locate` selbst (volle Suche) – der Beleg des Prüfers ruft es direkt auf und bleibt dort bei 2000; begrenzt sind die beiden Wege, auf denen ein fremder Code zu wiederholten Suchen führt.

### Testlücke Seite: Verdrahtung der Knöpfe in `TresorList.tsx`

**Befund zutreffend.**

**Behebung:** neuer Test `app/src/components/audit12a-tresor-aktionen.test.ts`:
- Er ruft die Komponente ohne DOM als Funktion auf; die Hooks sind ersetzt.
- Dann sucht er im Elementbaum den Knopf, führt dessen `onClick` aus und prüft, was an `runAction` geht:
  - „Wirklich abholen“ → `{ id }` bzw. mit Schlüssel nur bei eigener Gebühr
  - „Wirklich 1000,5 KAS nachlegen“ → `kas: "1000.5"`

Beide Rückbauten des Prüfers machen ihn rot.

### Testlücke ghostctl

**Befund zutreffend.** Die Logik lag in ghostctl, wo kein Test sie erreichte.

**Behebung:** Die Abläufe stehen jetzt in der Bibliothek, ghostctl liefert nur noch Ein- und Ausgabe:
- `tresor::follow`, `tresor::import`, `tresor::pay_round` über `TresorIo`: in ghostctl `TresorNet` (Node, Journal, Terminal), im Test `SimIo` (Simulator) bzw. eine Attrappe.
- `message::inbox` über `InboxIo`: in ghostctl `InboxNet` (REST-API, Node), im Test Attrappen. `inbox` liest die Tresor-Datei selbst; ein Aufruf mit leerer Liste ist nicht mehr möglich.
- `parse_script`: Test `a12n_laengenabhaengige_stellen_nur_ueber_die_neuuebersetzung`. Er verändert jede längenabhängige Stelle (Slot 6) so, dass die Befehlsfolge gleich bleibt. Nur der byte-genaue Vergleich mit der Neuübersetzung lehnt das ab.

**Tests zu den Rückbauten des Prüfers:**
- `break` statt `continue_after_send_error`: `a12n_runde_geht_nach_sendefehler_weiter` (erster Tresor scheitert beim Senden, der zweite wird gezahlt; mit offenem Journal endet die Runde).
- `recheck_missing` aus der Auswahl: `a12n_runde_sieht_fehlende_wieder_nach`.
- `known → &[]`: `a12n_eingang_prueft_gegen_die_tresor_datei` (Tresor-Datei im Temp-Verzeichnis).
- Node-Abgleich abgeschaltet: `message::a12n_ablauf_eingang_mit_node` und `a12n_eingang_prueft_gegen_die_tresor_datei`.

**Weiter ohne Test (braucht Node bzw. REST-API):** die reine Ein-/Ausgabe in ghostctl.
- `TresorNet`: Adresse → `utxos`, `send_to` mit Journal, `pending_path`.
- `InboxNet`: REST-Abruf, `get_block`, Deutung „nur Kopf“ und „cannot find header“, `get_block_dag_info`.
- Die Ausgabe von `messages`.
- Die Wahl von `Search::Full` bei `sync`, `topup`, `cancel`.

### A12-8-Texte: Regeln von ghostctl als Vertragsgarantie

**Befund zutreffend.**

**Behebung:**
- FAQ: „Nach Betrag und Höchstgebühr muss laut Vertrag nur etwas übrig bleiben. ghostctl und diese Seite zahlen nur, wenn danach mindestens 1 KAS im Tresor bleibt … Ein fremder Auslöser kann dagegen zahlen, solange Betrag und Höchstgebühr gedeckt sind, und weniger Rest lassen.“
- Tresor-Liste:
  - neuer Zustand „Guthaben knapp“ (`tresorStatus` = `low`): ghostctl zahlt nicht mehr, der Vertrag ließe es noch zu;
  - „Guthaben aufgebraucht“ nur bei `Wert − Betrag − Höchstgebühr ≤ 0`;
  - der Absender-Text bei Gebühr nur mit eigenem Schlüssel sagt, dass ein fremder Auslöser sie weiter aus dem Tresor nehmen darf.
- `MIN_KEEP` in `tresor.rs`, `TRESOR_MIN_KEEP` in `tresor.ts` und die Hilfe zu `tresor pay` nennen die 1 KAS als Regel von ghostctl.

**Tests:**
- `app/src/pages/audit12a-faq.test.ts`
- `audit12a-tresor.test.ts` (Komponente, Beispiel 10,5 KAS aus der Prüfung, 10,01 KAS „aufgebraucht“)
- `lib/audit12a-tresor.test.ts` (`contractPayable`)

Zwei bestehende Tests hielten „aufgebraucht“ fest und sind angepasst.

### A12-1-Hinweise: Abgleich nur bei übernommenem Code; `sealed` ungeprüft

**Befund zutreffend.**

**Behebung:**
- FAQ, Hinweis beim Anlegen, Eingang und Tresor-Liste sagen, dass der Abgleich nur bei Tresoren mit hier übernommenem Code greift, sonst „nicht prüfbar“.
- „Wie hinterlegt“ verlangt zusätzlich, dass der Payload für diesen Schlüssel den Text `message` des Tresors ergibt (`check_tresor`). Ein Code mit `sealed` ≠ `message` ergibt „eingefügt“.

**Tests:**
- `a12n_hinterlegt_heisst_auch_derselbe_text`
- `lib/audit12a-tresor.test.ts` (Hinweis beim Anlegen)
- `audit12a-faq.test.ts`

### A12-11-Folgen: ältere Daueraufträge, verschwindende Zahlungen

**Befund zutreffend.** Behebung siehe A12-11, „Folgen für Bestehendes“.

**Tests:**
- `payload_tests.rs`: `a12n_aeltere_dauerauftraege_zahlen_weiter`. Alle Fälle der gemeinsamen Sammlung bestehen bereinigt den Filter, erlaubte bleiben unverändert; `abo::payload` für öffentliche und verschlüsselte Aufträge.
- `message.rs`: `a12n_unzulaessige_zeichen_zahlung_bleibt_sichtbar`.
- Seite:
  - `audit12a-nachrichten.test.ts` (`sendableMessage`, dieselben Fälle und Beispiele wie Rust)
  - `audit12a-tresor.test.ts` (Hinweis beim Auftrag)
  - `audit12a-messages.test.ts` (Zeile „nicht angezeigt“ samt Herkunft)
- Zwei Aussagen in `klartext_nie_mit_magic` hielten `None`/`Unreadable` fest und erwarten jetzt `Invalid`.

### A12-19-Grenze: erfundener Blockhash

**Befund zutreffend.**

**Behebung** (`message::NodeLookup`, `node_verdict`, `Retention`; in ghostctl `InboxNet::node_tx` und `retention`):
- Liegt ein genannter Block vollständig am Node (Transaktionen vorhanden) und die Tx steht nicht darin: ausgeblendet. Das gilt nur, wenn sich jede Tx des Blocks bestimmen ließ.
- Kennt der Node keinen der genannten Blöcke („cannot find header“), obwohl die Blockzeit laut REST-API mehr als 1 h nach seinem Pruning-Punkt und mehr als 1 h vor seiner Past Median Time liegt: ausgeblendet.
- Ein Node liefert für Blöcke mit gelöschtem Inhalt nur den Kopf ohne Transaktionen (`get_block_even_if_header_only`). Das bleibt „laut REST-API“, ebenso Fehler und Zeitüberschreitungen.

**Grenzen:**
- Nennt die REST-API zusätzlich eine alte Blockzeit, bleibt die Nachricht „laut REST-API“ – mit dieser alten Zeit in der Anzeige.
- Den Fehlertext „cannot find header“ habe ich aus dem Quelltext von rusty-kaspa `a41a333` abgeleitet (`ConsensusError::HeaderNotFound`, über `RpcError::ConsensusError` transparent weitergereicht), nicht live gemessen.
- Trifft der Text nicht zu, fällt die Prüfung auf den alten Stand zurück („laut REST-API“), sie blendet nie fälschlich aus.

**Tests:** `message.rs`: `a12n_urteil_am_node`, `a12n_ablauf_eingang_mit_node`.

### Vertrag

Keine Änderung nötig. Die Anmerkung zu `OpTxPayloadSubstr`/`OpTxPayloadLen` (A12-1) gilt weiter.

### Rückbauproben

Je Probe genau eine Kernänderung zurückgebaut, Tests laufen lassen, Datei wiederhergestellt (Skripte unter `scratchpad/fix12a-n/`). Rust: `--lib`, `tresor_e2e_tests`, `payload_tests`, `standing_order_tests` (115 Tests); Seite: volle Suite (387).

**Rust**

| Rückbau | rot |
|---|---|
| Selektor von `pay` nicht geprüft | `a12n_kuendigen_und_auffuellen_…`, `tresor::a12n_nur_der_zweig_pay` |
| Ausgang an mich nicht geprüft | `a12n_tresor_zahlung_nur_mit_ausgang_an_mich` |
| jede Tx mit Tresor-Eingang als Zahlung (Stand `5aea635`) | `a12n_kuendigen_und_auffuellen_…`, `a12n_ueberzahlungs_masche_…`, `a12n_tresor_zahlung_nur_mit_ausgang_…` |
| Besitzer auch bei unbekanntem Tresor | `a12n_fremder_besitzer_…`, `a12n_eingang_prueft_…`, `a12_untergeschobene_…` |
| „von“ = Besitzer (Stand `5aea635`) | `a12n_fremder_besitzer_…`, drei `a12_*` in `tresor_e2e_tests.rs` |
| „wie hinterlegt“ ohne Textvergleich | `a12n_hinterlegt_heisst_auch_derselbe_text` |
| Automatik sucht fehlende voll | `tresor::a12n_fehlender_tresor_automatik_sucht_begrenzt` |
| Import ohne Rückstandsgrenze | `tresor::a12n_import_mit_altem_termin_abgelehnt` |
| `break` statt `continue_after_send_error` | `a12n_runde_geht_nach_sendefehler_weiter` |
| `recheck_missing` nicht in der Auswahl | `a12n_runde_sieht_fehlende_wieder_nach` |
| `known` → `&[]` im Eingang | `a12n_eingang_prueft_gegen_die_tresor_datei` |
| Node-Abgleich abgeschaltet | `message::a12n_ablauf_eingang_mit_node`, `a12n_eingang_prueft_…` |
| Block ohne die Tx nicht ausgeblendet | `message::a12n_urteil_am_node`, `message::a12n_ablauf_…` |
| unbekannter Block nicht ausgeblendet | `message::a12n_urteil_am_node`, `message::a12n_ablauf_…` |
| `parse_script` ohne byte-genauen Vergleich | `tresor::a12n_laengenabhaengige_stellen_…` |
| `read` wie vorher (verschlüsselt bzw. öffentlich) | `message::a12n_unzulaessige_zeichen_…`, `message::klartext_nie_mit_magic` |
| Daueraufträge ohne Bereinigung | `a12n_aeltere_dauerauftraege_zahlen_weiter` |

**Seite**

| Rückbau | rot |
|---|---|
| „Wirklich abholen“ sendet wieder `{id, key}` | `audit12a-tresor-aktionen`: „Tresor trägt die Gebühr: ohne Schlüssel“ |
| Auffüllen sendet den Rohtext | `audit12a-tresor-aktionen`: „„1.000,5“ → 1000.5“ |
| Besitzer auch ohne übernommenen Tresor | `audit12a-messages`: „Besitzer nur bei übernommenem Tresor …“ |
| wieder „Tresor von“ | dieselbe |
| „knapp“ wieder „aufgebraucht“ | 4 Tests (Komponente, `lib/audit12a-tresor` 2×, `tresor.test`) |
| alter Absender-Text (Gebühr nur mit Schlüssel) | `audit12a-tresor`: Nachprüfung A12-8 |
| alte FAQ zu 1 KAS Rest | `audit12a-faq`: A12-8 |
| alte FAQ zum Abgleich | `audit12a-faq`: A12-1 |
| alter Hinweis beim Anlegen | `lib/audit12a-tresor`: A12-1 |
| `invalid` ohne Anzeige | `audit12a-messages`: A12-11 |
| `sendableMessage` ohne Bereinigung | 41 Tests |
| Hinweis bei Aufträgen entfernt | `audit12a-tresor`: A12-11 |
| „(laut Tresor-Code)“ beim Besitzer entfernt | `audit12a-tresor`: A12-1 |

## Zweite Nachprüfung (Prüfer 12a, Stand `d5fab53`)

Der Prüfer hat jede Behebung einzeln zurückgebaut. Überlebt haben T6b (Abholen ohne Rückfrage) und R20 (stündlicher Takt) – beides richtig im Code, aber ohne Test –, dazu R21 (P2SH-Adresse) und T1b (folgenlos, siehe unten). Dazu fünf kleine Punkte.

| ID | Stufe | Stand |
|---|---|---|
| A12-10-Rueckfrage-ungetestet | wichtig | behoben (Test) |
| A12-16-Takt-ungetestet | wichtig | behoben (Tests) |
| FAQ-Agent-fragt-nicht | klein | behoben |
| Altdaten-falsche-Warnung | klein | behoben |
| same_as_node-Reihenfolge | klein | behoben, auch in der Tresor-Erkennung |
| Texte-und-Zeitrahmen | klein | behoben (drei Punkte) |
| R21-P2SH-Pruefung-ungetestet | klein | behoben (Test) |

### A12-10-Rueckfrage-ungetestet: erster Knopf ohne Test

**Befund zutreffend.** „Erster Klick fragt nur nach“ prüfte nur das Markup, der Aktionen-Test klickte nur „Wirklich abholen“.

**Behebung (nur Test):** `app/src/components/audit12a-tresor-aktionen.test.ts`
- Die Hook-Attrappe merkt sich jetzt, was `useState` setzt.
- „Fällige Zahlung abholen“ wird angeklickt, für einen Tresor, der die Gebühr trägt, und einen, bei dem nur die eigene Gebühr geht. Erwartet: kein Aufruf von `runAction`, gesetzt wird nur die Rückfrage `pay-<id>`.
- Ohne Rückfrage gibt es keinen Knopf „Wirklich abholen“; in der Rückfrage steht der erste Knopf nicht mehr da.

### A12-16-Takt-ungetestet: stündliches Nachsehen

**Befund zutreffend.** `a12n_fehlender_tresor_automatik_sucht_begrenzt` prüfte `recheck_missing(NOW + 1 h)`, das auch ohne Wartezeit gilt.

**Behebung (nur Tests):**
- `tresor.rs`: `a12p_fehlender_tresor_hoechstens_stuendlich`. Nach dem Fehlen ist `retry_after = jetzt + 1 h`. 1 ms, 1 min und 1 h − 1 ms danach sind weder `recheck_missing` noch `needs_run` (Agent) wahr, nach 1 h beides. Nach dem erneuten Nachsehen wieder 1 h Pause.
- `tresor_e2e_tests.rs`: `a12p_runde_sucht_fehlende_hoechstens_stuendlich`. Die Runde (`pay_round`) läuft gegen den Simulator nach einer Kündigung zu fünf Zeitpunkten. Node-Abfragen: `[2, 0, 0, 0, 2]` bei 0 s, 1 min, 30 min, 1 h − 1 ms und 1 h.
- Damit ist auch die Zusage „168 × 32 = 5 376 Abfragen je Woche“ abgesichert. Der Server (`tresorNeedsRun`) liest dasselbe Feld `retryAfter`; sein Test aus der ersten Runde prüft die Regel mit gesetztem Feld.

### FAQ-Agent-fragt-nicht

**Befund zutreffend.** `tresor_agent_step` ruft `tresor pay` mit dem Agentenschlüssel und `--ja` auf; `tresor::pay` nimmt die Gebühr dann ohne Rückfrage vom Schlüssel, sobald der Tresor sie nicht trägt (zu wenig Rest oder Gebühr über `maxFee`).

**Behebung:** `app/src/pages/Faq.tsx`. Neu: „… zahlen sie nur noch mit Gebühr vom eigenen Schlüssel. Diese Seite fragt dann vorher nach, ghostctl im Mainnet ebenso. Ein laufender GHOST-Agent zahlt die Gebühr dagegen ohne Rückfrage vom Schlüssel seines Betreibers, sobald der Tresor sie nicht trägt, auch wenn die Netzgebühr über der Höchstgebühr liegt.“
- Die Seite selbst zahlt bei einer Gebührenspitze nicht vom eigenen Schlüssel (`payParams` gibt den Schlüssel nur bei zu wenig Rest mit). Deshalb steht die Gebührenspitze nur beim Agenten.
- ghostctl fragt im Mainnet ohne `--ja` nach (`Ctx::confirm`).

**Test:** `app/src/pages/audit12a-faq.test.ts`, „Zweite Nachprüfung: der GHOST-Agent fragt nicht nach“ (deutsch und englisch).

### Altdaten-falsche-Warnung

**Befund zutreffend.** Die Beschreibung eines vor der Verschärfung übernommenen Tresors darf heute verbotene Zeichen enthalten. Seine echte Zahlung ergab `Found::Invalid`, `found.text()` war `None`, also „NICHT vom Absender – eingefügt“.

**Behebung** (`message.rs`): Bei `Found::Invalid` vergleicht `check_tresor` mit dem gelesenen, nicht angezeigten Text (`raw_text`: entschlüsselt bzw. UTF-8, ohne `check_message`).
- „Wie hinterlegt“ verlangt weiter beides: den hinterlegten Payload und denselben Text wie die Beschreibung.
- Die Seite zeigt dann „Nachricht mit unsichtbaren oder unzulässigen Zeichen – nicht angezeigt“ und „wie im Tresor … hinterlegt“. Der Text selbst erscheint nie.

**Test:** `message.rs`, `a12p_altdaten_echte_zahlung_nicht_eingefuegt`.
- Tastenkappe und ZWJ-Emoji verschlüsselt, U+2028 öffentlich, jeweils in einer echten `tresor::pay`-Zahlung: „wie hinterlegt“.
- Gegenproben bleiben „eingefügt“: eine andere Nachricht mit solchen Zeichen, und eine hinterlegte Fassung, deren Text nicht die Beschreibung ist.

### same_as_node-Reihenfolge

**Befund zutreffend.** Lesend nachgeprüft an api.kaspa.org (drei Tx aus aktuellen Blöcken): Auch die Eingänge tragen das Feld `index`, neben `previous_outpoint_address`, `signature_script` u. a.

Dieselbe Abhängigkeit steckte auch in `tresor_payment`. Dort galt die Listenstelle des Eingangs als Index. Zahlt der Empfänger die Gebühr selbst (zwei Eingänge) und kommen die Eingänge in anderer Reihenfolge, wäre die Tresor-Zahlung als eigene Sendung aus dem Eingang verschwunden.

**Behebung** (`message.rs`): Neue Funktion `in_index_order`. Sie ordnet nach `index` bzw. lässt ohne das Feld die Listenreihenfolge. Indizes, die nicht genau 0..n sind (doppelt, Lücke, nur teilweise), ergeben `None`: Abweichung bzw. keine Tresor-Zahlung. Genutzt in `same_as_node` (Eingänge und Ausgänge) und `tresor_payment`.

**Tests** (`message.rs`):
- `a12p_node_abgleich_nach_index`: vertauscht mit `index` gleich, ohne `index` abweichend; doppelt, Lücke, teilweise abweichend.
- `a12p_tresor_erkennung_nach_index`: Tresor-Zahlung mit eigener Gebühr; Eingänge vertauscht, bleibt „wie hinterlegt“ mit dem Vertragsbetrag.

### Texte-und-Zeitrahmen

**1. „etwa 30 Stunden“: zutreffend.** 30 h ist die Untergrenze. Der Pruning-Punkt ist die jüngste Stichprobe (alle 432 000 Blöcke = 12 h) mit mindestens 1 080 000 Blöcken (30 h) Tiefe (rusty-kaspa `a41a333`, `pruning.rs`, `expected_header_pruning_point`). Blockinhalte liegen also 30 bis 42 h am Node; gemessen ≈ 37 h. Neuer Text im Eingang: „Blöcke behält ein Node nur etwa 30 bis 42 Stunden, ältere Zahlungen stehen „laut REST-API“ da“. Ebenso im Kommentar von `InboxNet::node_tx`.

**2. Warnhinweis „ausgeblendet“: zutreffend.** Neu: „… meldete sie anders, als der Node sie kennt – mit anderem Inhalt als im Block, in einem Block ohne diese Transaktion oder in einem Block, den der Node nicht kennt, obwohl er ihn noch haben müsste.“ Die Gründe je Tx stehen weiter in `checks` (Tooltip).

**3. Zeitrahmen: zutreffend.** Neu `message::InboxBudget`/`INBOX_BUDGET`, ab dem Start von `inbox` gerechnet:
- Der Verlauf (200 Tx) braucht höchstens 4 REST-Abfragen zu je höchstens 20 s (`chain::http`), also ≤ 80 s.
- GHOST-Tx: neue Abrufe nur in den ersten 60 s (`ghost_receipts`); der Rest wird gemeldet („… GHOST-Tx aus Zeitgründen nicht geprüft“, auf der Seite der bestehende Hinweis zu GHOST-Eingängen).
- Node: Verbinden, Aufbewahrungsfenster und jede Abfrage laufen unter `tokio::time::timeout` mit der Restzeit. Das sind höchstens 60 s ab Beginn des Abgleichs und nie über 150 s ab Start. Hängt ein Schritt, bleiben die Nachrichten „laut REST-API“; in `checks` steht „Zeitrahmen … erschöpft“.
- In ghostctl ist das Trennen auf 5 s begrenzt.
- Zusammen ≤ 80 s + 60 s + 5 s = 145 s, unter den 180 s der Seite (`READ_TIMEOUT_MS`).
- `NODE_BUDGET` ist in `INBOX_BUDGET.node` aufgegangen. Tests setzen über `InboxIo::budget` kürzere Rahmen.

**Tests:**
- Seite: `audit12a-messages.test.ts`, „Aufbewahrung am Node und alle Gründe fürs Ausblenden genannt“; der Test „ohne Node …“ prüft den neuen Aufbewahrungstext.
- Rust (`message.rs`):
  - `a12p_eingang_im_zeitrahmen`: Verbinden, Aufbewahrungsfenster bzw. Abfrage hängen je 30 s; mit 300 ms Rahmen ist `inbox` nach unter 2 s fertig, beide Nachrichten „laut REST-API“. Dauert schon der Verlauf länger als der Gesamtrahmen, fragt der Node nichts mehr. Gegenprobe ohne Hänger: beide am Node geprüft.
  - `a12p_ghost_abrufe_im_zeitrahmen`: fünf GHOST-Tx zu je 100 ms mit 250 ms Rahmen, 2 bis 3 Abrufe, der Rest gemeldet.

### R21-P2SH-Pruefung-ungetestet

**Befund zutreffend.**

**Behebung (nur Test):** `message.rs`, `a12p_tresor_eingang_mit_fremder_p2sh_adresse`.
- Eine echte `tresor::pay`-Zahlung ergibt „wie hinterlegt“.
- Mit einer anderen P2SH-Adresse am Tresor-Eingang ist sie keine Tresor-Zahlung („über einen Vertrag“).
- Ohne Adressangabe zählt nur das Skript.

### T1b (Prüfer: folgenlos)

Nicht geändert. Der Schlüssel der Prüfung enthält das Netz (`JSON.stringify({ network, params })`). Eine im Testnetz geprüfte Anfrage passt im Mainnet also nie, auch wenn das Verwerfen im `useEffect` fehlte. Das deckt der Test aus der ersten Runde ab („im Mainnet wieder „Tresor prüfen““).

### Rückbauproben (zweite Nachprüfung)

Je Probe genau eine Änderung zurückgenommen, Tests laufen lassen, Inhalt zurückgeschrieben (Skripte `scratchpad/fix12a-p/mut-rust.py` und `mut-ts.py`). Rust: `--lib` und `tresor_e2e_tests` mit `--no-fail-fast`. Seite: volle Suite (392).

**Rust**

| Rückbau | rot |
|---|---|
| R20: kein `retry_after` beim Fehlen | `tresor::a12p_fehlender_tresor_hoechstens_stuendlich`, `a12p_runde_sucht_fehlende_hoechstens_stuendlich` |
| R21: P2SH-Adresse nicht geprüft | `message::a12p_tresor_eingang_mit_fremder_p2sh_adresse` |
| Altdaten: bei `Invalid` kein Text | `message::a12p_altdaten_echte_zahlung_nicht_eingefuegt` |
| `same_as_node` in Listenreihenfolge | `message::a12p_node_abgleich_nach_index` |
| `tresor_payment` in Listenreihenfolge | `message::a12p_tresor_erkennung_nach_index` |
| Abfrage am Node ohne Zeitgrenze | `message::a12p_eingang_im_zeitrahmen` |
| Verbinden ohne Zeitgrenze | dieselbe |
| Aufbewahrungsfenster ohne Zeitgrenze | dieselbe |
| Node-Rahmen ohne Gesamtgrenze | dieselbe |
| GHOST-Abrufe ohne Zeitgrenze | `message::a12p_ghost_abrufe_im_zeitrahmen` |

**Seite**

| Rückbau | rot |
|---|---|
| T6b: „Fällige Zahlung abholen“ sendet direkt | `audit12a-tresor-aktionen`: „… erster Klick sendet nichts“ (2 Tests) |
| „Wirklich abholen“ auch ohne Rückfrage sichtbar | dieselben 2 und `audit12a-tresor`: „erster Klick fragt nur nach“ |
| FAQ: alter Satz „fragen vorher nach“ | `audit12a-faq`: „der GHOST-Agent fragt nicht nach“ |
| Eingang: „etwa 30 Stunden“ | `audit12a-messages`: 2 Tests |
| Eingang: alter Hinweis „ausgeblendet“ | `audit12a-messages`: „Aufbewahrung am Node und alle Gründe …“ |

Alle 15 Proben rot. Danach sind die Dateien byte-gleich mit dem Stand vor den Proben (Diff verglichen).

## Dateien außerhalb der Liste

`app/src/lib/api.ts`: nur die Typen `InboxMessage` (`origin`, `tresor`, `source`, seit der Nachprüfung `tresorOwner` und `kind: "invalid"`) und `InboxResult` (`checks`, `nodeChecked`, `hidden`). Das war nötig, weil `ghostctl messages` neue Felder liefert.

## Testzahlen

- Rust: `cargo test --release --offline` vorher 290 grün und 2 ignoriert.
  - Nach der ersten Runde 306 grün, 2 ignoriert. 16 neue Tests: 5 in `tresor.rs`, 2 in `message.rs`, 2 in `payload_tests.rs`, 3 in `standing_order_tests.rs`, 4 in `tresor_e2e_tests.rs`.
  - Nach der Nachprüfung 322 grün, 2 ignoriert. 16 weitere Tests:
    - 4 in `tresor.rs`
    - 3 in `message.rs`
    - 1 in `payload_tests.rs`
    - 8 in `tresor_e2e_tests.rs`
  - Nach der zweiten Nachprüfung 330 grün, 2 ignoriert. 8 weitere Tests: 7 `a12p_*` in der Bibliothek (6 in `message.rs`, 1 in `tresor.rs`), 1 in `tresor_e2e_tests.rs`.
  - Aufteilung: lib 70, audit10_pool_engine 20, audit10_pool_regress 4, chain 3, e2e 4, factory 21, ghost_token 9, oracle 26, payload 10, pool_e2e 3, pool 32, standing_order 18, tresor_e2e 25, vault_math 4, vault 81.
- Seite:
  - `npx vitest run` vorher 224, nach der ersten Runde 314, nach der Nachprüfung 387, jetzt 392 grün in 18 Dateien.
  - Neu seit der Nachprüfung: `audit12a-tresor-aktionen.test.ts` (4) und `pages/audit12a-faq.test.ts` (2), dazu Ergänzungen in den vier `audit12a-*.test.ts`.
  - Zweite Nachprüfung: +3 in `audit12a-tresor-aktionen.test.ts`, +1 in `audit12a-faq.test.ts`, +1 in `audit12a-messages.test.ts`.
  - `npx tsc --noEmit -p .` fehlerfrei.
  - `npm run build` fehlerfrei, nur die bekannte Warnung zur Chunk-Größe.

## Restpunkte (Prüfer der Behebungen, Stand `a70fbbd`)

Branch `fix13a`, Basis `a70fbbd`, 30.09.2026. Die unabhängigen Prüfer der Behebungen haben für diese Gruppe sechs Restpunkte gemeldet: eine Testlücke „wichtig“ und fünf kleine Punkte. Verträge (`contracts/*.sil`) sind unverändert; keine echten Transaktionen, `keys/` und `deployments/` nicht gelesen.

| ID | Stufe | Stand |
|---|---|---|
| A12-19-Zeilenkennzeichnung-ungetestet | wichtig | behoben (Test) |
| A12-7-pay_round-with_key-ungetestet | klein | behoben (Tests, ghostctl über `tresor_pay`) |
| A12-16-Meldung-nur-einmal-ungetestet | klein | behoben (Test) |
| Invalid-Text-zu-lang | klein | behoben |
| Import-alter-Tresor-Codes | klein | behoben (übernommen und gekennzeichnet) |
| upsert-retry_after | klein | behoben |

### A12-19-Zeilenkennzeichnung-ungetestet

**Befund zutreffend.** Der Fußtext unter der Tabelle enthält beide Etiketten, der Test prüfte das ganze Markup. Die Rückbauten T28 (`m.source === "node" ?` → `false ?`) und T28b (→ `true ?`) überlebten.

**Behebung (nur Test):** `app/src/components/audit12a-messages.test.ts`, „je Zahlung: am Node geprüft oder nur laut REST-API“.
- Neue Hilfe `rowList`: die einzelnen `<tr>` im `<tbody>`.
- Drei Zahlungen (node, rest, node): je Zeile genau das eigene Etikett. Eine einzelne REST-Zeile trägt nie „am Node geprüft“. Dasselbe englisch.
- T28 und T28b sind jetzt rot (siehe Rückbauproben).

### A12-7-pay_round-with_key-ungetestet

**Befund zutreffend.** Keine Runde lief ohne Schlüssel mit einem Tresor, der die Gebühr nicht trägt; die Übergabe `k.is_some()` in ghostctl war ungetestet.

**Behebung:**
- `ghostctl.rs`: neue Hilfsfunktion `tresor_pay`. Sie bekommt den Schlüssel einmal und gibt ihn an die Ein-/Ausgabe (die damit die Gebühr zahlt) und als `with_key = key.is_some()` an `tresor::pay_round`. `TresorCmd::Pay` ruft nur noch sie auf (`|key| TresorNet { ctx, key }`).
- Tests:
  - `tresor_e2e_tests.rs`: `a13_runde_ohne_schluessel_nur_wenn_der_tresor_die_gebuehr_traegt`. Tresor mit 11,005 KAS für 10 KAS im Simulator. Runde ohne Schlüssel: kein Bericht, nichts gesendet, weder `last_error` noch `retry_after`. Bestimmter Tresor ohne Schlüssel: Fehler „trägt die Netzgebühr nicht mehr … eigenem Schlüssel“, Tresor unberührt. Mit Schlüssel: gezahlt, der Tresor verliert nur den Betrag, Verlauf „Gebühr vom Auslöser“.
  - `ghostctl.rs`: `a13_tresor_pay_uebergibt_den_schluessel_an_die_runde` fährt `tresor_pay` mit einer Simulator-Ein-/Ausgabe (`SimIo` im Testmodul), ohne und mit Schlüssel.
  - `ghostctl.rs`: `a13_tresor_pay_offline_mit_und_ohne_schluessel`. Dieselbe Übergabe schon in der Vorprüfung ohne Netz (`tresor_offline`, `needs_run(…, key.is_some())`): mit Schlüssel geht es zum Node, ohne bleibt es bei „Keine fälligen Tresor-Zahlungen“.

### A12-16-Meldung-nur-einmal-ungetestet

**Befund zutreffend.** „Gemeldet wird nur beim ersten Mal“ war nur für `follow` getestet.

**Behebung (nur Test):** `tresor_e2e_tests.rs`, `a13_runde_meldet_fehlende_nur_beim_ersten_mal`. Nach einer Kündigung läuft die Runde bei 0 h, 1 h, 2 h und 24 h. Jede Runde fragt den Node, die Meldung „nicht auffindbar oder gekündigt“ steht nur in der ersten: `[1, 0, 0, 0]`. Rückbau R30 ist rot.

### Invalid-Text-zu-lang

**Befund zutreffend.** `Found::Invalid` entsteht auch bei Texten über 100 Zeichen, etwa JSON einer anderen Anwendung. ghostctl schrieb dazu „mit unzulässigen Zeichen“, die Seite „mit unsichtbaren oder unzulässigen Zeichen“.

**Behebung:**
- `abo.rs`: `has_bad_char` (Zeichenprüfung von `check_message` ohne die Länge). `check_message` nutzt sie, das Verhalten ist gleich.
- `message.rs`: `Found::Invalid(Rejected)` mit `Rejected::{TooLong, BadChars, TooLongAndBadChars}`, dazu `rejected(text)`. `read` setzt den Grund, öffentlich wie entschlüsselt.
- `ghostctl.rs`: `found_view` liefert Text, Art, Grund und Terminalzeile; `inbox_item` baut daraus Zeile und JSON je Nachricht (vorher in der Schleife von `messages`). Neues JSON-Feld `invalid`: `length`, `chars` oder `both`. Terminal: „Nachricht zu lang (über 100 Zeichen) – nicht angezeigt“, „… mit unsichtbaren oder unzulässigen Zeichen …“ bzw. beides.
- Seite: `IncomingMessages.tsx` (`hiddenReason`) zeigt je Zeile den Grund. Fehlt das Feld (älteres ghostctl), steht „zu lang oder mit unzulässigen Zeichen“. `api.ts`: nur der Typ `invalid`.

**Tests:**
- `message.rs`: `a13_nicht_angezeigt_mit_dem_richtigen_grund`. Zu lang (ASCII, JSON, Emoji), unzulässig (Richtungszeichen, JSON mit Zeilenumbruch), beides; Grenze 100 Emoji. Je Fall `read` öffentlich und verschlüsselt sowie `inbox_entry`. `rejected` stimmt mit `check_message` überein, auch über alle Fälle von `nachrichtenfilter.json`.
- `ghostctl.rs`: `a13_nicht_angezeigt_mit_dem_richtigen_grund` (Terminalzeile und Feld, vom Payload bis zum JSON von `inbox_item`).
- `audit12a-messages.test.ts`: „je Zeile der richtige Grund: zu lang, unzulässige Zeichen oder beides“, dazu der Fall ohne Feld und englisch.
- Die bestehenden Tests mit `Found::Invalid` erwarten jetzt `Invalid(Rejected::BadChars)`; ihre Texte sind kurz und enthalten verbotene Zeichen.

### Import-alter-Tresor-Codes

**Befund zutreffend, mit einer Einschränkung.** Tastenkappe, VS16 hinter Buchstaben, U+2028 oder Private Use konnten vor der Verschärfung in einer Tresor-Beschreibung stehen, der Code ließ sich dann nicht mehr übernehmen. Ein ZWJ-Emoji dagegen nicht: U+200D stand schon im alten Filter (`is_format_char`, U+200B–200F), seit es Tresore gibt (`0572e0a`).

**Entscheidung: übernehmen und kennzeichnen, nicht bereinigt speichern.**
- Die gespeicherte Beschreibung ist der Vergleichstext des Eingangs (`check_tresor`: „wie hinterlegt“ nur, wenn der Payload den Text der Beschreibung ergibt). Bei öffentlichen Tresoren ist sie zugleich der Payload jeder Zahlung (`TresorRec::payload`). Eine bereinigte Fassung würde die echten Zahlungen des Absenders als „eingefügt“ melden, und eigene Auslösungen sendeten eine andere Nachricht als der Absender.
- Deshalb bleibt sie unverändert gespeichert und steht so im Code. Nur die Anzeige ist bereinigt.

**Behebung:**
- `tresor.rs`:
  - `check_code_message` statt `check_message` in `TresorCode::decode`. Abgelehnt wird weiter, was ghostctl nie angenommen hat (Steuerzeichen, Richtungszeichen, Null-Breite einschließlich ZWJ, U+FEFF, U+061C, U+00AD), und über 100 Zeichen.
  - Vertragsdaten und verschlüsselte Fassung sind unverändert streng geprüft.
  - `TresorRec::shown_message`: Beschreibung nach heutigem Filter (`abo::sendable_message`, wie bei Daueraufträgen) und ob dabei etwas wegfiel.
  - `tresor::import` meldet eine bereinigte Beschreibung („… angezeigt wird sie bereinigt: „…““).
- `ghostctl.rs` (`tresor_json`): `message` ist die bereinigte Fassung, neu `messageCleaned`; `code` bleibt der Code des Absenders.
- Seite: `TresorList.tsx` zeigt das Kennzeichen „bereinigt: enthielt unsichtbare oder heute unzulässige Zeichen“, auch wenn nichts übrig blieb. `lib/tresor.ts`: nur der Typ.

**Tests:**
- `tresor.rs`: `a13_alte_codes_mit_heute_verbotenen_zeichen_uebernommen`. Fünf alte Beschreibungen, je öffentlich und verschlüsselt:
  - Code lesbar und importiert (Attrappe des Nodes)
  - gespeichert wie im Code, gleicher Payload wie beim Absender
  - Anzeige bereinigt und gekennzeichnet, Hinweis gemeldet
  - Gegenproben: zulässige Beschreibung ohne Kennzeichen und Hinweis; abgelehnt bleiben Steuer-, Richtungs-, Null-Breite-Zeichen, ZWJ-Emoji und 101 Zeichen; Betrag, Höchstgebühr und `sealed` bleiben auch mit alter Beschreibung streng.
- `ghostctl.rs`: `a13_tresor_liste_zeigt_alte_beschreibung_bereinigt` (`tresor_json`).
- `audit12a-tresor.test.ts`: „die Beschreibung erscheint bereinigt und gekennzeichnet“.
- Dass die echte Zahlung eines solchen Tresors „wie hinterlegt“ ergibt, deckt weiter `message::a12p_altdaten_echte_zahlung_nicht_eingefuegt` ab. Der neue Test sichert zu, dass Beschreibung und Payload nach dem Import genau diese Eingaben sind.
- `code_ablehnen` bleibt unverändert: „a\nb“ und U+202E sind weiter abgelehnt.

### upsert-retry_after

**Befund zutreffend.**

**Behebung** (`tresor.rs`, `TresorFile::upsert`): War der Tresor als fehlend markiert, fällt beim erneuten Übernehmen auch `retry_after` weg, wie in `follow`. Die Wartezeit nach einem Fehlschlag beim Auslösen (Tresor nicht verschwunden) bleibt.

**Test:** `tresor.rs`, `a13_erneut_uebernommen_ohne_wartezeit`.
- Ablauf: Fehlen beim Abgleich (Wartezeit 1 h, `needs_run` falsch), dann erneuter Import. Danach sind `missing`, `missing_ms` und `retry_after` leer, `looks_due` und `needs_run` sofort wahr.
- Dasselbe direkt über `upsert`.
- Gegenprobe: Die Wartezeit nach einem Fehlschlag bleibt.

### Rückbauproben (Restpunkte)

Je Probe genau eine Änderung zurückgenommen, Tests laufen lassen, Inhalt zurückgeschrieben (Skripte `scratchpad/fix13a/mut-rust.py` und `mut-ts.py`). Rust: `--lib`, `--bin ghostctl`, `tresor_e2e_tests`, `payload_tests` mit `--no-fail-fast`. Seite: volle Suite (406).

**Rust**

| Rückbau | rot |
|---|---|
| R15: `due_now(…, with_key)` in `pay_round` → `true` | `a13_runde_ohne_schluessel_…`, `ghostctl::a13_tresor_pay_uebergibt_…` |
| R30: `if note.is_some()` → `if true` | `a13_runde_meldet_fehlende_nur_beim_ersten_mal` |
| `tresor_pay`: `key.is_some()` → `true` | `ghostctl::a13_tresor_pay_uebergibt_…` |
| `tresor_pay`: `key.is_some()` → `false` | dieselbe |
| `tresor_offline`: `key.is_some()` → `true` | `ghostctl::a13_tresor_pay_offline_…` |
| `tresor_offline`: `key.is_some()` → `false` | dieselbe |
| zu lang als `BadChars` | `message::a13_nicht_angezeigt_…`, `ghostctl::a13_nicht_angezeigt_…` |
| Terminal: alter Text bei zu lang | `ghostctl::a13_nicht_angezeigt_…` |
| JSON: `invalid` immer `chars` | `ghostctl::a13_nicht_angezeigt_…` (erst nach Auslagern von `inbox_item`, siehe unten) |
| Import wieder mit `check_message` | `tresor::a13_alte_codes_…`, `ghostctl::a13_tresor_liste_…` |
| Import speichert bereinigt | `tresor::a13_alte_codes_…` |
| Liste zeigt den Rohtext | `ghostctl::a13_tresor_liste_…` |
| kein Hinweis beim Import | `tresor::a13_alte_codes_…` |
| `upsert` ohne Zurücksetzen von `retry_after` | `tresor::a13_erneut_uebernommen_ohne_wartezeit` |

**Seite**

| Rückbau | rot |
|---|---|
| T28: `m.source === "node"` → `false` | `audit12a-messages`: „je Zahlung: am Node geprüft oder nur laut REST-API“ |
| T28b: → `true` | dieselbe |
| `length` zeigt den Text für Zeichen | `audit12a-messages`: „je Zeile der richtige Grund …“ |
| alter Text für jedes `invalid` | dieselbe |
| kein Kennzeichen „bereinigt“ | `audit12a-tresor`: „die Beschreibung erscheint bereinigt und gekennzeichnet“ |
| Bedingung wieder nur `t.message` | dieselbe |

Die Probe „JSON: `invalid` immer `chars`“ überlebte zuerst: Das JSON von `messages` entstand in der Schleife, die ohne Netz kein Test erreicht. Die Ausgabe je Nachricht (Terminalzeile und JSON) steht jetzt in `inbox_item`; der ghostctl-Test prüft sie für alle drei Gründe, danach ist die Probe rot. Alle 20 Proben rot. Danach sind die Dateien byte-gleich mit dem Stand vor den Proben (`git diff` verglichen).

### Testzahlen (Restpunkte)

- Rust: `cargo test --release --offline` vorher (`a70fbbd`) 348 grün, 2 ignoriert; jetzt 357 grün, 2 ignoriert. 9 neue Tests:
  - 1 in `message.rs`, 2 in `tresor.rs`
  - 4 in `ghostctl.rs`
  - 2 in `tresor_e2e_tests.rs`
  - Aufteilung: lib 76, ghostctl 10, audit10_pool_engine 20, audit10_pool_regress 4, chain 3, e2e 5, factory 21, ghost_token 9, oracle 26, payload 10, pool_e2e 3, pool 32, standing_order 18, sweep_grenze 3, tresor_e2e 27, vault_math 4, vault 86.
- Seite: `npx vitest run` vorher 404, jetzt 406 grün in 20 Dateien (+1 in `audit12a-messages.test.ts`, +1 in `audit12a-tresor.test.ts`; der A12-19-Test ist umgebaut).
- `npx tsc --noEmit -p .` fehlerfrei; `npm run build` fehlerfrei, nur die bekannte Warnung zur Chunk-Größe.
