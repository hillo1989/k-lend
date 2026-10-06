# Audit 13 – Stand der Behebungen, Gruppe Tresor-Code (A13-tresor-1 bis 4)

Branch `fix16a`, Basis `1148270`, 30.09.2026. Grundlage sind die bestätigten Befunde von Audit 13 (Bereich „tresor“) und die Beweistests der Prüfer (`r13_*` in `protocol/tests/tresor_e2e_tests.rs` der Prüfkopie).

Leitlinie: Die Beschreibung im Tresor-Code darf nie etwas behaupten, das der Vertrag nicht erzwingt.

- Verträge (`contracts/*.sil`) sind unverändert. Die Bindung des Payloads im Vertrag hält laut Prüfung; geändert ist nur, was ghostctl und die Seite über die Beschreibung sagen.
- Keine echten Transaktionen. `keys/` und `deployments/` wurden nicht gelesen. Der neue Test für die Schlüsselsuche legt eigene Testschlüssel in einem temporären Ordner an und löscht ihn danach.
- Übernommen aus der Prüfung und auf die Behebung umgeschrieben: `r13_code_beschreibung_bei_verschluesselt_frei`, `r13_code_erfundene_beschreibung_ohne_nachricht`, `r13_code_onchain_ohne_text_mit_chiffrat` und `r13_hoechstgebuehr_unter_mindestgebuehr`. Die übrigen `r13_*`-Tests belegen Eigenschaften, die schon halten (Payload-Grenzen, Form/Parse, Payload über 520 Byte), und wurden nicht übernommen.

Ausführen:

```
cd protocol && CARGO_TARGET_DIR=../target-fix16a cargo test --release --offline
cd app && npx vitest run && npx tsc --noEmit -p . && npm run build
```

## Übersicht

| ID | Stand | Wo | Tests (scheitern ohne die Behebung) |
|---|---|---|---|
| A13-tresor-1 | **behoben** | `tresor.rs` (`import(…, recipient)`, `sealed_matches`, `TresorRec::checked`, `message_check`, `MessageCheck`), `ghostctl.rs` (`tresor import [--key]`, `recipient_key`, `tresor list --key`, `tresor_json_with` → `messageCheck`), `lib/tresor.ts` (`messageSure`), `TresorList.tsx`, `Faq.tsx` | `tresor_e2e_tests.rs`: `a13f_import_prueft_verschluesselte_beschreibung`; `ghostctl` (bin): `a13_tresor_liste_prueft_verschluesselte_beschreibung`; `audit13-tresor-beschreibung.test.ts` (6 Tests A13-tresor-1); angepasst: `audit12a-tresor.test.ts` („Nachricht fest im Vertrag …“) |
| A13-tresor-2 | **behoben** | `tresor.rs` (`check_bound`: Form; `TresorFile::upsert`) | `a13f_code_beschreibung_nur_mit_passender_nachricht`, `a13f_upsert_nur_gebundene_beschreibung` |
| A13-tresor-3 | **behoben** | `tresor.rs` (`check_bound`: Form) | `a13f_code_beschreibung_nur_mit_passender_nachricht` |
| A13-tresor-4 | **behoben** (Anlegen); fremde Codes **dokumentiert** | `tresor.rs` (`MIN_MAX_FEE`, `check_params`), `ghostctl.rs` (`--max-fee`-Hilfe), `lib/tresor.ts` (`TRESOR_MIN_MAX_FEE`, `maxFeeTooLow`), `TresorList.tsx`, `Faq.tsx` | `a13f_mindest_hoechstgebuehr_ueber_der_gemessenen_gebuehr` (Messung), `a13f_hoechstgebuehr_unter_mindestgebuehr_abgelehnt`; `audit13-tresor-beschreibung.test.ts` (2 Tests A13-tresor-4) |

Gegenprobe: Jede Behebung wurde einzeln zurückgebaut und die neuen Tests liefen gegen den Rückbau.

| Rückbau | scheitert |
|---|---|
| Form-Regeln in `check_bound` aus | `a13f_code_beschreibung_nur_mit_passender_nachricht` |
| `import` prüft `sealed` nicht | `a13f_import_prueft_verschluesselte_beschreibung` |
| `upsert` wie vorher (übernimmt bei leerer alter Beschreibung) | `a13f_upsert_nur_gebundene_beschreibung` |
| `check_params` wieder ab 1 sompi | `a13f_hoechstgebuehr_unter_mindestgebuehr_abgelehnt` |
| `message_check` ignoriert den Schlüssel | `a13_tresor_liste_prueft_…` (bin), `a13f_import_prueft_…` |
| Seite: „genau sie“ bei jeder Beschreibung (`sure = !!t.message`) | 5 Seiten-Tests |
| Seite: `maxFeeTooLow` immer false | 2 Seiten-Tests |

## A13-tresor-1: verschlüsselte Beschreibung nicht gebunden

**Vorher.** Bei `onchain=false` bindet der Vertrag über `payloadHash` nur die verschlüsselte Fassung (`sealed`), nicht den lesbaren Text im Code. `import` übernahm jede Beschreibung, die Liste zeigte dazu „Jede Zahlung trägt genau sie“.

**Jetzt.**
- `tresor::import` bekommt den Schlüssel des Empfängers als Argument (`Option<&SecretKey>`); die Funktion sucht nicht selbst. ghostctl reicht `--key` durch oder, ohne Angabe, die Datei in `keys/`, deren x-only-Pubkey der Empfänger des Codes ist (`recipient_key`, gibt nichts aus).
- Mit passendem Schlüssel wird `sealed` entschlüsselt und mit der Beschreibung verglichen (getrimmt, wie im Eingang). Abweichung oder nicht lesbar: Der Code wird abgelehnt, bevor der Node gefragt wird („Die Beschreibung weicht von der verschlüsselten Nachricht ab …“). Gleich: `TresorRec.checked = true` (gespeichert, `serde(default)`, ältere Dateien bleiben lesbar).
- Ein Schlüssel, der nicht der des Empfängers ist, prüft nichts; die Beschreibung bleibt ungeprüft.
- `TresorRec::message_check(sk)` liefert `none | bound | checked | unchecked | mismatch`: öffentlich = `bound`; verschlüsselt und hier angelegt (`key`, der Absender hat selbst verschlüsselt) oder beim Import geprüft = `checked`; sonst `unchecked`. Mit `sk` des Empfängers (`tresor list --key`) wird bei der Anzeige geprüft; Abweichung = `mismatch`.
- `tresor_json` gibt `messageCheck` aus. `tresor import` meldet „geprüft“ oder „Beschreibung laut Tresor-Code, nicht geprüft“; `tresor list` nennt ungeprüfte und abweichende Tresore.
- Seite (`TresorList.tsx`): „Die Nachricht ist im Vertrag fest gebunden: Jede Zahlung trägt genau sie …“ nur bei `bound`/`checked` (`messageSure`). Ohne `messageCheck` (älteres ghostctl) nur bei öffentlicher Nachricht oder hier angelegtem Tresor. Sonst das Kennzeichen „laut Tresor-Code, nicht geprüft“ und der Satz, dass der Vertrag nur die verschlüsselte Fassung bindet und der Eingang zeigt, was die Zahlungen tragen. Bei `mismatch` eine Warnung („der Tresor-Code wurde verändert“). Für den Absender, der hier angelegt hat, bleibt „genau sie“.
- FAQ („Kann ich die Nachricht eines Tresors später ändern?“) ergänzt.

**Test.** `a13f_import_prueft_verschluesselte_beschreibung`: gefälschter Code mit Empfängerschlüssel abgelehnt; ohne oder mit fremdem Schlüssel übernommen als `unchecked`, bei der Anzeige mit Empfängerschlüssel `mismatch`; echter Code mit Schlüssel `checked`; ein ohne Schlüssel übernommener Eintrag wird durch erneuten Import mit Schlüssel `checked`.

## A13-tresor-2: Tresor ohne Nachricht, erfundene Beschreibung

**Vorher.** Mit `onchain=false` und leerem `sealed` ist der Payload leer, jede Beschreibung passte zum Hash. `upsert` übernahm sie nachträglich, wenn die alte Beschreibung leer war.

**Jetzt.**
- `check_bound` (und damit `decode`, `bind_message`) verlangt die Form: bei `onchain=false` gibt es eine Beschreibung genau dann, wenn es eine verschlüsselte Fassung gibt. Beschreibung ohne `sealed` → „Beschreibung ohne Nachricht in den Zahlungen“; `sealed` ohne Beschreibung → abgelehnt.
- `upsert` übernimmt eine Beschreibung nur, wenn sie gebunden ist (`message_sure`: öffentlich oder geprüft) und zu den Parametern des vorhandenen Eintrags passt (`check_bound`), und nur, wenn die bisherige nicht gebunden war. Eine ungeprüfte ersetzt nie etwas; eine geprüfte ersetzt eine ungeprüfte. Gleiche Beschreibung, jetzt geprüft: `checked` wird nachgetragen.

**Tests.** `a13f_code_beschreibung_nur_mit_passender_nachricht` (erfundene Beschreibung zum leeren Payload abgelehnt, echte Codes aller drei Arten gültig); `a13f_upsert_nur_gebundene_beschreibung` (Eintrag an `decode` vorbei, etwa aus einer älteren Datei: nicht übernommen, auch nicht mit gesetztem `checked`, weil er nicht zum leeren Payload passt).

## A13-tresor-3: öffentlich ohne Text mit Chiffrat

**Jetzt.** `check_bound`: `onchain=true` verlangt eine nicht leere Beschreibung und ein leeres `sealed` („öffentliche Nachricht ohne Text oder mit verschlüsselter Fassung“). `bound_payload` bleibt unverändert; der Rückfall auf `sealed` bei öffentlicher leerer Nachricht ist über `check_bound` nicht mehr erreichbar.

**Test.** `a13f_code_beschreibung_nur_mit_passender_nachricht` (öffentlich ohne Text mit Chiffrat, öffentlich mit Text und Chiffrat, verschlüsselt mit leerer Beschreibung: alle abgelehnt). Der bestehende Test `a13_import_mit_falschem_hash_abgelehnt` nimmt die neuen Meldungen für zwei seiner Fälle an (sie scheitern jetzt schon an der Form statt erst am Hash).

## A13-tresor-4: Höchstgebühr unter der Mindestgebühr

**Messung** (`a13f_mindest_hoechstgebuehr_ueber_der_gemessenen_gebuehr`, Simulator): Zahlung mit dem längsten Payload (`MAX_PAYLOAD` = 464 Byte) über `tresor::pay` ohne Schlüssel, für 36 Kombinationen aus Betrag (1 KAS, 1000 KAS, 2^55 sompi), Anzahl (1, unbegrenzt, 2^40), Intervall (Monatstag 31, 3650 Tage) und Fortsetzung (1 KAS, 1.000.000 KAS), Höchstgebühr `MAX_MAX_FEE` (längster Push). Ungünstigster Fall: **359.100 sompi**. Zum Vergleich die Prüfer: 356.160 sompi bei 464 Byte, 258.720 bei 0 Byte.

**Jetzt.** `MIN_MAX_FEE = 400.000 sompi` (0,004 KAS), gut 10 % über dem gemessenen Höchstwert; der Test verlangt mindestens 10 % Aufschlag und `MIN_MAX_FEE ≤ DEFAULT_MAX_FEE`. `check_params` lehnt darunter ab („Höchstgebühr: 0.004 bis 0.1 KAS …“), damit auch `ghostctl tresor open --max-fee`. Mit genau `MIN_MAX_FEE` und der längsten Nachricht zahlt der Tresor die Gebühr selbst (im Test angenommen). Seite: FAQ nennt die Untergrenze; die Tresor-Liste warnt bei einem Tresor mit kleinerer Höchstgebühr („unter der Mindestgebühr einer Zahlung – auslösen meist nur mit eigenem Schlüssel“). Die Seite bietet `--max-fee` beim Anlegen weiterhin nicht an.

**Dokumentiert, nicht geändert.** `TresorCode::decode` nimmt weiter Höchstgebühren ab 1 sompi an: Ein fremder oder älterer Tresor mit kleiner Höchstgebühr steht bereits in der Kette und bleibt mit eigenem Schlüssel zahlbar; ihn abzulehnen, nähme dem Empfänger nur die Anzeige. `due_now`/`looks_due` prüfen die Höchstgebühr weiterhin nicht gegen die tatsächliche Gebühr. Bei einem solchen Tresor versucht die Automatik ohne Schlüssel also weiter alle 15 Minuten. Neu anlegen lässt sich so ein Tresor mit ghostctl nicht mehr.

## Offene Hinweise

- Tresore, die vor diesem Stand ohne Empfängerschlüssel übernommen wurden, haben `checked = false` und erscheinen bis zu einem erneuten Import mit Schlüssel (oder `tresor list --key`) als „nicht geprüft“. Das ist beabsichtigt: Geprüft wurde bei ihnen nichts.
- Die Seite übergibt beim Übernehmen keinen Schlüssel. ghostctl findet den Empfängerschlüssel selbst in `keys/`, sofern die Seite ghostctl im Projektordner startet (wie `keys` ohne `--dir`).

## Testzahlen

- Rust (`cargo test --release --offline`): 382 bestanden, 0 fehlgeschlagen, 2 ignoriert (live). Darunter `--lib` 79, `ghostctl` (bin) 18 (vorher 17), `tresor_e2e_tests` 37 (vorher 32).
- Seite: vitest 424/424 (vorher 416), `tsc --noEmit` ohne Fehler, `npm run build` erfolgreich.
