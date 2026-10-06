# Audit 13 – Stand der Gruppe „Umzug“ (Branch fix16b)

Geprüft wurde `GHOST-Umzug-v3.command` mit den Szenario-Tests in `tests/umzug/run.zsh`.
Die Tests laufen gegen die Attrappe `tests/umzug/mock_ghostctl.py`, also ohne Netz und ohne echte Schlüssel.
Die neuen Fälle stehen im Abschnitt **U** (U1–U7). Jeder Fall enthält Prüfungen, die mit dem Skript von Basis 1148270 scheitern (`SCRIPT=<alt> tests/umzug/run.zsh`: 31 Prüfungen rot, davon 22 in U; die übrigen 9 sind die angepassten Erwartungen „repay mit --ghost“ und E9); mit dieser Fassung bestehen alle.

Ergebnis: `tests/umzug/run.zsh` 179 bestanden, 0 fehlgeschlagen (vorher 151). `zsh -n` ist sauber für Skript und Tests.

Grenze: `bin/ghostctl-v2` fehlt in diesem Worktree.
Laut Auftrag kennt es `--vault <Nummer>`, `pool-remove --min-kas/--min-ghost` und `repay --ghost`.
Das Skript verlässt sich darauf. Gegen das echte Programm ist das nicht gemessen.

| Befund | Stand | Datei | Test |
|---|---|---|---|
| A13-umzug-1 | behoben | GHOST-Umzug-v3.command: `txa`, `vault_vor`, `vault_nach`, `v2_abdruck`, `teil_a` | U1, U2, U3; angepasst R3, R4, P dry1/real1, N1, D1, E3, E4, E8 |
| A13-umzug-2 | behoben | `teil_a` (Pool-Anteile), `TOL_ABZUG_BPS` | U4 (zwei Fälle); E9 an die Toleranz angepasst |
| A13-umzug-3 | dokumentiert (Anzeige) | `H_ZUS`, `plan_zeigen`, `endstand` | U6 |
| A13-umzug-4 | behoben | `sig_ende`, `tx` (LAUFEND) | U5 |
| A13-umzug-5 | behoben | `kopf_zeigen` | U6 (auch mit KEYS=alt/x) |
| A13-umzug-6 | behoben (Anzeige) | `plan_b` (Pool-Hinweis) | U4 |
| A13-umzug-7 | behoben | `SU`/`sompi`/`ks`, `teil_a`, Pool-Grenzen in Teil B | U7 |

## A13-umzug-1 – Senden über die Nummer statt über die Covenant-ID

- **Tilgen mit festem Betrag:** `repay` geht immer mit `--ghost <bestätigter Betrag>` hinaus, auch beim vollen Tilgen. Trifft die Nummer doch einen anderen Vault, geht höchstens der bestätigte Betrag weg.
- **Prüfung vor dem Senden:** `txa` bekommt über `VC` die Art (repay/close/withdraw), die Covenant-ID und die Nummer. Unmittelbar vor dem Senden liest `vault_vor` den Status neu, also auch nach einer möglichen Zwischenfrage. Trägt die Nummer nicht mehr die Covenant-ID, hält das Skript mit „Vault-Nummer verschoben …“ an und sendet nichts.
- **Prüfung nach dem Senden:** `vault_nach` prüft, dass genau dieser Vault wie erwartet aussieht:
  - repay: Schuld minus Betrag, Sicherheit gleich
  - close: der Vault ist weg
  - withdraw: Sicherheit = `--keep`, Schuld gleich

  Außerdem müssen die übrigen eigenen Vaults unverändert sein (verglichen über die Covenant-ID). Sonst hält das Skript an und zeigt Soll und Ist. Im Stand ist der gesendete Schritt dann als „gesendet, aber die Wirkung weicht ab“ markiert.

Tests:
- **U1:** Beweisfall X1 der Prüfer. Der fremde Vault verschwindet während des Tilgens. Gesendet wird nur 0,5 GHOST an c2, danach wird angehalten: kein close, c2 bleibt offen.
- **U2:** Die Nummern verschieben sich zwischen Tilgen und Schließen. close wird nicht gesendet, c2 bleibt unberührt.
- **U3:** Die Nummern verschieben sich zwischen dem Lesen des Status und dem Senden. Es wird gar nichts gesendet. Ein erneuter Doppelklick rechnet mit den neuen Nummern und läuft durch.

Die Attrappe kennt dafür `move_after` (nach einem Sendebefehl oder nach dem N-ten Status).

## A13-umzug-2 – pool-remove ohne Mindestbeträge

- **Rückfluss:** Der Plan rechnet ihn ganzzahlig wie `pool::remove`: Anteil m/S, abgerundet, die KAS-Seite gekappt auf Reserve − 1 KAS.
- **Mindestbeträge:** `--min-kas` und `--min-ghost` sind dieser Rückfluss minus `TOL_ABZUG_BPS` = 100 (1 %).
- **Kennung:** Die Mindestbeträge stehen in der Kennung (`pool-remove <Anteile> <min-kas> <min-ghost>`). Verschiebt sich der Pool bis zum Senden, rechnet das Skript neu und fragt. Verschiebt er sich beim Senden, sendet ghostctl nicht.
- **Anzeige:** Die Zusammenfassung nennt „mindestens … KAS und … GHOST (Toleranz 1 %)“.
- **Pool nicht lesbar:** Sind die Reserven nicht lesbar, zieht das Skript nicht ab (Abbruch ohne Senden).

Tests:
- **U4/u4-min:** Probe und Senden laufen mit `--min-kas 9.90000000 --min-ghost 0.24750000`.
- **U4/u4-verschoben:** Beweisfall X2. Die Attrappe weist wie `pool.rs` ab, das Skript bricht vor dem Tilgen ab, Anteile und GHOST sind unverändert.
- **E9:** Der Fall bewegt sich jetzt innerhalb der Toleranz (0,248 statt 0,25 GHOST).

## A13-umzug-3 – mögliche Zusammenführungs-Tx

Nicht vermeidbar, weil ghostctl sie intern sendet.
Die Zusammenfassung nennt sie jetzt beim Tilgen und beim Pool-Anlegen: „dazu ggf. 1 Transaktion „GHOST zusammenführen“ …“.
Die Gebührenzeile zählt sie als „dazu ggf. bis zu n × …“.

Test: U6.

## A13-umzug-4 – Signal-Abbruch ohne Stand

`trap sig_ende INT TERM HUP`. Nach der Bestätigung nennt der Abbruch den Schritt, der gerade lief (`LAUFEND` aus `tx`), und ruft `stand_zeigen` auf. Das Ende ist weiterhin 130, die Sperre wird frei.

Test U5: TERM während des Prägens. Der Abbruch nennt „Unterbrochen während: 0.5 GHOST geprägt (Vault 0)“, als gesendet „Vault mit 50 KAS eröffnet“ und als offen Prägen und Pool.

## A13-umzug-5 – Schlüssel und Adresse nicht in der Zusammenfassung

`kopf_zeigen` gibt über dem Plan aus:
- Netz
- Schlüsseldatei (relativ und mit vollem Pfad)
- Adresse (Feld `address` aus `keys --json`)
- x-only

Test U6: Standardfall, dazu `KEYS=alt/x` (zeigt `alt/x-owner.json`).

## A13-umzug-6 – Pool-Toleranz nicht genannt

Der Pool-Schritt nennt jetzt: „der KAS-Betrag folgt dem Orakelkurs beim Senden: bis ± 1 % Abweichung geht ohne erneute Frage hinaus, darüber wird erneut gefragt“. Der Wert wird aus `TOL_POOL` gerechnet.

Test: U4 (u4-min).

## A13-umzug-7 – Float-Rundung bei --keep und Grenzen

Teil A rechnet Tilgbetrag, Restschuld, eigene GHOST, verplante GHOST und Rückfluss aus dem Pool in ganzen Sompi. Dazu kommen die Grenzen: voll getilgt, schuldenfrei, `--keep` kleiner als die Sicherheit. Hilfsfunktionen sind `SU`, `sompi` und `ks`.

`--keep` wird exakt mit `fractions.Fraction` gerechnet: 220 % der Restschuld zum Kurs aus dem Status, auf 0,01 KAS aufgerundet, mindestens 0,3 KAS.

Die GHOST- und KAS-Grenzen vor dem Pool in Teil B vergleichen ebenfalls Sompi. Die Schätzungen im Plan (Endstand, Warnungen) bleiben Float, sie senden nichts.

Test U7:
- 0,1 GHOST bei 0,022 USD ergibt `--keep 10.00` (vorher 10.01).
- 0,3 GHOST ergibt `--keep 30.00`.
- Die Prüfung nach dem Herausnehmen besteht.

## Weitere Änderungen

- `tests/umzug/mock_ghostctl.py`:
  - `move_after` sowie `sleep` und `sleep_after` (aus der Kopie der Prüfer)
  - `pool-remove` mit `--min-kas/--min-ghost` und ganzzahligem Rückfluss wie `pool.rs`
  - `keys` nennt eine Adresse
- `tests/umzug/pruefer/real1-calls.log`: Die Vorlage der Prüfer von Audit 12 hat für die zwei vollen Tilgungen jetzt `--ghost`.
- `tests/umzug/run.zsh`: `setup` warnt bei ungültiger `mock.json`.
- `MAINNET.md`, Abschnitt „Version 3 und Umzug“: Kopf der Zusammenfassung, Mindestbeträge, Toleranzen, Zusammenführung, Signal-Abbruch, Prüfung von Nummer und Covenant-ID, Rechnen in Sompi.
