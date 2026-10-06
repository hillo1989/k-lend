# Audit 1 — StableVault: Ökonomie und Arithmetik

Datum: 28.09.2026 · Prüfer: unabhängiger Auditor (Fable 5.1) · Stand: Mainnet-Deployment `deployments/mainnet.json`
(1 Vault, 150 KAS, 1 GHOST Schuld; Orakel seq 0, Index 1e9; MCR 200 %, Liquidation 150 %, Bonus 10 %).

Geprüft: `contracts/stable_vault.sil` (Rechnung, mint/repay/withdraw/deposit/close/liquidate), Rust-Referenz
`protocol/src/math.rs` und `ops.rs`, die vorhandenen Tests, sowie der GHOST-Token (`vendor/silverscript/…/kcc20.sil`,
unverändert übernommen, `contracts.rs:17`), weil die Vault-Ökonomie an dessen Mengenerhaltung hängt.

**Beweisführung:** Alle mit „getestet“ markierten Befunde wurden mit eigenen Transaktionstests gegen die echte
Skript-Engine (rusty-kaspa a41a333, jeder Input ausgeführt) nachgestellt. Testquelle: `audit/1-vault-oekonomie-tests.rs`
(Kopie der Helfer aus `vault_tests.rs` plus Fälle A1–A8), Protokoll: `audit/1-vault-oekonomie-testlog.txt`.
Reproduktion: Datei nach `protocol/tests/audit_vault.rs` kopieren,
`cd protocol && cargo test --test audit_vault -- --nocapture`. Es wurde nichts im Repository verändert und nichts gesendet.

## Übersicht

| ID | Schwere | Kurzfassung | Beleg |
|---|---|---|---|
| V-01 | **kritisch** | GHOST (KCC20) erlaubt negative Token-Beträge: aus 1 Einheit werden beliebig viele GHOST; damit kostenlose Liquidation fremder Sicherheiten | getestet (A1, A1b, A1c) |
| V-02 | hoch | Liquidation nur als Vollverbrennung; bei Quote < 100 % Verlustgeschäft für den Liquidator, kein Bad-Debt-Pfad, ungedeckte GHOST bleiben dauerhaft | getestet (A2) + Herleitung |
| V-03 | mittel | Zins ohne Gegenwert: Summe der Schulden > GHOST-Umlauf; letzte Schuldner können nie schließen, ≥ 2× Restschuld bleibt gebunden | getestet (A3) + Herleitung |
| V-04 | mittel | `deposit` und `repay` ohne Signatur: Fremde geben die Vault-UTXO aus (Konflikt-Griefing gegen Besitzer und Liquidatoren, Liquidations-DoS durch den Besitzer) | getestet (A4) |
| V-05 | niedrig | Teiltilgung verliert bis zu ⌈Index/1e9⌉−1 Einheiten je Tx (sharesFor rundet auf 0), Überzahlungsschutz greift nur beim Vollausgleich | getestet (A4) |
| V-06 | info | DUST-Regel: Rest < 0,2 KAS geht an den Liquidator (dokumentiert, nicht steuerbar) | getestet (A5) |
| V-07 | info | Minter-Zweig (3 KAS) nach close/Totalliquidation dauerhaft unbrauchbar (dokumentiert) | getestet (A6) |
| V-08 | info | Überlaufgrenze von `debtOf` bei MAX_DEBT_SHARES ist Index 92×, nicht 9 200× wie dokumentiert; Abbruch ist fail-safe | getestet (A8) |
| V-09 | info | Kein Preisalter im Vault (Designgrenze B1); Zinsindex rundet je Update ab | Herleitung, UNVERIFIED als Test |
| V-10 | info | Rust-Referenz `math.rs`/`ops.rs` stimmt mit dem Vertrag überein; keine Abweichung gefunden | Herleitung + vorhandene `vault_math_tests` |

Was **nicht** gefunden wurde (geprüft, kein Befund): Erzeugen von GHOST über den Vault ohne Schuld, Schuld senken ohne
Verbrennen, Rundung zugunsten des Nutzers (A7), Abziehen von KAS über `deposit`/`continueWith`, Übernahme des
Minter-Zweigs, Nutzung eines fremden Orakels, Überlauf mit falschem Ergebnis (Engine bricht mit `NumberTooBig` ab).

---

## V-01 (kritisch) — GHOST ist beliebig vermehrbar: KCC20 prüft kein Vorzeichen

**Ort:** `vendor/silverscript/silverscript-lang/tests/examples/kcc20.sil:24-38` (`checkAmounts`: nur `totalIn == totalOut`),
verwendet unverändert als GHOST (`protocol/src/contracts.rs:17`). Mainnet: `factory_params.ghost_tpl.hash =
7c57e2a9a335…eeeb` ist exakt der Template-Hash dieses Skripts (nachgerechnet, `audit_tpl`-Lauf). Im Vault selbst:
`contracts/stable_vault.sil:160` summiert Eingangs-Token ohne Vorzeichenprüfung (Ausgänge werden in Zeile 173 auf > 0 geprüft).

**Szenario:** Ein Angreifer besitzt eine beliebig kleine GHOST-Menge (1 Einheit = 1e-8 GHOST genügt). Er baut einen
gewöhnlichen KCC20-Transfer (Leader = sein eigener Token, kein Vault, kein Minter): Eingang 1 Einheit, Ausgänge
`[+D, −(D−1)]`. Die Mengenerhaltung `1 == D − (D−1)` hält. Der Ausgang `+D` ist ein byte-identischer, normaler GHOST-Token;
den Ausgang `−(D−1)` lässt er einfach liegen. Damit:
1. liquidiert er jeden Vault unter 150 % ohne eigene GHOST und erhält die Sicherheit (Diebstahl von KAS im Wert der Schuld
   plus Bonus, bei Unterdeckung die gesamte Sicherheit);
2. kann er den Umlauf beliebig aufblähen und GHOST verkaufen — der Peg ist wertlos;
3. kann er jede Schuld „tilgen“ (für sich harmlos, aber die Bilanz wird sinnlos).

**Beleg (A1, A1c, Testlog):**
```
A1 KCC20-Transfer 10 -> [+15, -5]: [Ok(())]
A1c: aus 1 Einheit GHOST wurden 15750000000 Einheiten; damit Liquidation gültig,
     Angreifer erhält 1000000000000 sompi Sicherheit
```
In A1c wird geprüft, dass `script_public_key` von Ausgang 0 der Split-Tx genau dem Token-Input der Liquidations-Tx
entspricht; alle vier Inputs (Vault `liquidate`, Orakel `read`, Minter-Leader, Token-Delegate) laufen durch.
A1b zeigt zusätzlich, dass der Vault ein negatives Token auch als Input akzeptiert (`[+30, −20]` als 10 verbucht) —
das ist für sich kein Verlust für den Vault, aber es fehlt die zweite Verteidigungslinie.

**Warum die Vault-Tests das nicht sehen:** `vault_tests.rs` erzeugt Token-Inputs direkt per Konstruktor; es gibt
keinen Test für den reinen Token-Transfer mit negativem Ausgang. `ARCHITEKTUR.md` behandelt KCC20 als gegeben.

**Einordnung Mainnet heute:** Der einzige umlaufende Token (1 GHOST) liegt beim Deployer. Sobald irgendjemand
GHOST erhält (Überweisung, zweiter Vault), ist der Angriff ausführbar. Für einen öffentlichen Betrieb ist das ein
Totalausfall der Ökonomie.

**Empfehlung:** GHOST auf ein angepasstes KCC20 umstellen: in `transfer` für jeden Ausgang `require(newStates[i].amount >= 0)`
(und sinnvoll `> 0` für Nicht-Minter), im Vault `ghostDelta` zusätzlich `require(s.amount >= 0)` für Inputs
(`stable_vault.sil:157-160`). Da der GHOST-Template-Hash in Factory und Vault fest einkompiliert ist, bedeutet das ein
neues Deployment (Token, Factory, Vaults). Bis dahin keine GHOST an Dritte geben.

---

## V-02 (hoch) — Liquidation ist nur als Vollverbrennung möglich; keine Behandlung von Bad Debt

**Ort:** `contracts/stable_vault.sil:240-259`, insbesondere Zeile 245 (`0 − ghostDelta == debt`) und 248-251
(`seize = coll`, wenn `coll·p ≤ claim`).

**Herleitung:** Mit Quote `CR = coll·p / debt` erhält der Liquidator `min(coll, 1,1·debt/p)·p − debt = (min(CR, 1,1) − 1)·debt`.
- CR ≥ 110 %: Gewinn 10 % der Schuld, Rest an den Besitzer.
- 100 % < CR < 110 %: Gewinn (CR−1)·debt, Besitzer verliert **alles** (A2b: Quote 104,8 %, Gewinn 7,50 USD statt 15,75 USD; das ist
  formelgemäß, aber der Anreiz schrumpft, je nötiger die Liquidation wird).
- CR ≤ 100 %: **Verlust** für den Liquidator. Niemand liquidiert. Die Schuld bleibt, die GHOST sind ungedeckt, und es gibt
  keinen Pfad, der das auflöst: keine Teil-Liquidation, keine Umverteilung, kein Sicherheitsfonds, kein Debt-Ceiling.
  Der Besitzer hat ebenfalls keinen Anreiz zu tilgen (Schuld > Sicherheit).

Zusätzlich braucht ein Liquidator die **gesamte** Schuld in höchstens zwei Token-UTXOs (`MAX_GHOST_INS = 3`, Zeile 55/144).
Bei großen Vaults und knappem GHOST (siehe V-03) existiert dieser Liquidator womöglich nicht.

Vom Liquidationsschwellwert 150 % bis zur Unterdeckung sind es 33 % Preisrückgang. Das Orakel läuft laut `MAINNET.md`
nicht dauerhaft (`oracle-feed` nur solange der Rechner läuft; Mainnet-Orakel steht bei seq 0), das Vault-Skript kennt
kein Preisalter (V-09). Ein Preisrutsch zwischen zwei Updates landet also direkt in dieser Zone.

**Beleg (A2, Testlog):**
```
A2: Liquidator verbrennt 15750000000 Einheiten = 157.50 USD, erhält 1000000000000 sompi = 150.00 USD
    -> Verlust 7.50 USD; Teilverbrennung abgelehnt
```
Geprüft: `burn = debt − 1` → Vault-Input `Err`; Fortsetzung mit Rest 1 KAS → `Err`; `burn = debt` ohne Vault-Ausgang → alle Inputs `Ok`.

**Empfehlung:** Teil-Liquidation (`burned ≤ debt`, Sicherheit anteilig `burned·(1+bonus)/p`, Schuld anteilig senken),
Anreiz auch bei CR < 110 % erhalten (z. B. Bonus aus einem Protokollpuffer), und ein Verfahren für Restschuld nach
Totalverlust (Umlage auf andere Vaults oder Stabilitätsreserve). Mindestens: Debt-Ceiling und konservativere Schwelle,
solange das Orakel nicht dauerhaft läuft.

---

## V-03 (mittel) — Zins ohne Gegenwert: die Summe der Schulden übersteigt den GHOST-Umlauf

**Ort:** `stable_vault.sil:217` (Anteile = amount·1e9/index, aufgerundet), `:227` (Schuld = shares·index/1e9, aufgerundet),
`:233` (`burned == debt` beim Vollausgleich), `risk_oracle.sil:92-94` (Index wächst mit `stableRate`, Mainnet 5 %/Jahr).

**Herleitung:** Jede Prägung `m` bei Index `i₀` bucht `m·1e9/i₀` Anteile; bei Index `i` ist die Schuld `m·i/i₀ ≥ m`.
Umlauf = Σ geprägt − Σ verbrannt, Schulden = Σ m·i/i₀. Sobald der Index gewachsen ist, gilt Schulden > Umlauf, und die
Differenz (der „Zins“) wird nirgends geprägt. Der Zins fließt niemandem zu (kein Treasury wie bei GHO). Er ist reine
Deadweight-Schuld, die nur beglichen werden kann, indem jemand GHOST aus **fremden** Prägungen verbrennt. Der letzte
Schuldner kann nie schließen; wegen MCR 200 % bleiben mindestens 2× der Restschuld an Sicherheit gebunden.
`MAINNET.md` beschreibt den Effekt je Vault („1,00000003 statt 1 GHOST“), nicht die Gesamtbilanz.

**Beleg (A3, Testlog):** 100 GHOST bei Index 1,0 geprägt, Index 1,05:
```
A3: Schuld 10500000000 Einheiten; nach Verbrennen aller 100 GHOST bleiben 476190477 Anteile
    = 500000001 Einheiten; close scheitert; mindestens 25000000050 sompi (250.00 KAS) bleiben gebunden
```
Geprüft: repay mit 100 GHOST → Teiltilgung `Ok`; repay auf 0 mit 100 GHOST → `Err`; `close` mit Restanteilen → `Err`;
withdraw bis exakt zur Mindestquote → `Ok` (250 KAS bei 0,04 USD bleiben im Vault).

**Empfehlung:** Zins beim Fortschreiben oder beim Tilgen als GHOST an ein Treasury prägen (GHO-Modell), oder
`stableRate = 0` bis ein Empfänger existiert, oder Vollausgleich alternativ gegen KAS im Gegenwert der Restschuld erlauben.

---

## V-04 (mittel) — `deposit` und `repay` ohne Signatur: Fremde geben die Vault-UTXO aus

**Ort:** `stable_vault.sil:184-189` (deposit, `newColl > collateral()`, also +1 sompi genügt), `:223-236` (repay, `burned > 0`,
also 1 Einheit genügt; siehe V-05: sie tilgt 0 Anteile).

**Szenario:**
1. *Konflikt-Griefing:* Jede Vault-Aktion des Besitzers (und jede Liquidation) gibt die aktuelle Vault-UTXO aus.
   Ein Dritter, der parallel `deposit(+1 sompi)` einreicht, erzeugt eine konkurrierende Tx; wird sie zuerst
   angenommen, sind die signierte Owner-Tx und die Liquidations-Tx ungültig. Kosten: ~0,04 KAS Gebühr je Versuch.
   `ghostctl` hält den Zustand lokal („Zustandsdatei weiß davon nichts“, `MAINNET.md`), läuft also aus dem Tritt.
2. *Liquidations-DoS durch den Besitzer:* Ein unterdeckter Besitzer kann seine eigene Vault-UTXO fortlaufend mit
   +1-sompi-Einzahlungen (ohne Orakel, billig) weiterreichen. Jede Liquidation muss die jeweils neueste UTXO treffen;
   im DAG entscheidet die Reihenfolge, der Besitzer gewinnt das Rennen mit Gebühr statt mit Sicherheit.
   Zusammen mit V-02 (keine Teil-Liquidation, schmaler Anreiz) verlängert das die Zeit in der Bad-Debt-Zone.

**Beleg (A4, Testlog):** repay durch zufälligen Zahler mit 1 Einheit → alle Inputs `Ok`, Anteile unverändert;
deposit +1 sompi ohne Signatur → `Ok`.

**Empfehlung:** `deposit` und Fremd-`repay` an eine Signatur des Besitzers binden oder an eine Mindestmenge (z. B.
≥ 1 % der Sicherheit bzw. ≥ 1 Anteil getilgt). Alternativ Fremd-Tilgung nur, wenn `sharesFor(...) ≥ 1`.

---

## V-05 (niedrig) — Teiltilgung verfällt unterhalb einer Anteils-Einheit

**Ort:** `stable_vault.sil:231` (`sharesFor(burned, index, false)`), Schutz in Zeile 233 nur für den Vollausgleich.

**Herleitung:** `floor(burned·1e9/index) = 0` für `burned < index/1e9`. Bei Index 1,05 verfällt 1 Einheit je
Teiltilgung (A4: „bis 1 Einheiten je Teiltilgung verfallen“), bei Index 2,0 eine Einheit, bei Index 92× bis zu 91
Einheiten. Wirtschaftlich ≤ 1e-6 GHOST je Tx, aber der Vertrag verspricht in Zeile 233 gerade, dass keine GHOST
verloren gehen.

**Empfehlung:** `require(sharesFor(burned, index, false) >= 1)` in der Teilzweig-Verzweigung.

---

## V-06 (info) — DUST: Rest unter 0,2 KAS geht an den Liquidator

**Ort:** `stable_vault.sil:253-255`. Der Rest ergibt sich allein aus Preis und Schuld und ist von keiner Partei
steuerbar; Verlust für den Besitzer ≤ 0,2 KAS. Dokumentiert (KIP-9-Begründung). **Beleg (A5):** Rest 0,1 KAS →
Fortsetzung `Err`, Vollübernahme `Ok`. Kein Handlungsbedarf.

## V-07 (info) — Minter-Zweig nach close/Totalliquidation dauerhaft unbrauchbar

**Ort:** `stable_vault.sil:206` und `:255` (keine Fortsetzung der Vault-ID), `kcc20.sil:18` (`OpCovInputCount(owner) > 0`).
Die 3 KAS je Zweig (`ops.rs:30`) sind danach unerreichbar. In `MAINNET.md` als „verwaist“ dokumentiert.
**Beleg (A6):** Leader-Transfer des Zweigs ohne Vault-Input → `VerifyError`. Verbesserung: `close` könnte den Zweig in
derselben Tx auf einen Nicht-Minter-Token mit Menge 0 an den Besitzer überführen (KAS zurück).

## V-08 (info) — Überlaufgrenzen: `debtOf` bei MAX_DEBT_SHARES kippt ab Index 93×

**Ort:** `stable_vault.sil:84` (`q * index`, q ≤ 1e8). `ARCHITEKTUR.md` nennt „Index bis 9 200-fach“; das gilt nur für
den Restterm (Zeile 74-76), nicht für `q·index`. Bei 1e9 GHOST Schuld in einem Vault überläuft die Rechnung ab Index
93× (≈ 90 Jahre bei 5 %). Abbruch mit `NumberTooBig` ist fail-safe, würde diesen Vault aber einfrieren (repay und
liquidate rufen beide `debtOf`). Preisgrenze 920 USD/KAS gilt nur bei 1e8 KAS Sicherheit; bei 1e6 KAS liegt sie bei 92 000 USD.
**Beleg (A8):** `debtOf(1e17, 92e9)` → `Ok`, `debtOf(1e17, 93e9)` → `NumberTooBig`. Doku anpassen; praktisch irrelevant.

## V-09 (info, UNVERIFIED als Test) — Kein Preisalter im Vault; Zinsindex rundet ab

- `stable_vault.sil` enthält keinerlei `tx.daa`/`tx.time`-Prüfung; `healthy()` nutzt den Preis der Orakel-UTXO, wie alt
  er auch ist (Designgrenze B1 in `ARCHITEKTUR.md`). Mainnet-Orakel: seq 0, Preis vom Deployment. Ein Besitzer kann bei
  veraltetem hohem Preis prägen/abheben; die Vault-Ökonomie steht und fällt mit einem dauerhaft laufenden Feed.
  Gehört in das Orakel-Audit; hier nur der ökonomische Bezug.
- `risk_oracle.sil:93-94`: `growth = rate·Δ/1e9` und `index·growth/1e9` runden ab. Beim Standard-Intervall 300 s
  (Δ = 3000 DAA, Rate 158 548 960) ist growth = 475 statt 475,65 → 0,14 % des Periodenzinses gehen verloren; bei Δ < 7 DAA
  fällt gar kein Zins an. Zins ist je Update linear, über Updates hinweg verzinst (5,0 % → ≈ 5,1 % effektiv). Unkritisch.

## V-10 (info) — Rust-Referenz stimmt mit dem Vertrag überein

`math.rs`: `debt_of` (ceil), `shares_for` (ceil/floor), `value_of` (floor), `healthy` (need ceil, `shares == 0` gesund),
`seize` (claim ceil, `value > claim` strikt, sonst ganze Sicherheit), `DUST` — alle identisch zu `stable_vault.sil:77-107, 247-251`.
`ops::repay` (`amount ≥ debt` → Vollausgleich) und `ops::liquidate` (`rest < DUST` → kein Vault-Ausgang) verzweigen wie der
Vertrag; `oracle_next_index` rechnet wie `risk_oracle.sil:92-94` (checked statt Wrap, Engine bricht ebenfalls ab).
`max_mint` (Binärsuche) ist korrekt, weil `healthy` in den Anteilen monoton fällt. `vault_math_tests.rs` belegt die
Engine-Gleichheit für Index bis 90× (Zufall) — konsistent mit V-08. Einziger Unterschied: `ghostctl.rs:465` rechnet den
Liquidationspreis in f64 (nur Anzeige). Kein Befund.

---

## Rangfolge der Maßnahmen

1. **V-01 sofort:** keine GHOST an Dritte; neues Deployment mit vorzeichengeprüftem Token und Input-Prüfung im Vault.
2. **V-02/V-03 vor öffentlichem Betrieb:** Teil-Liquidation, Bad-Debt-Verfahren, Zins-Empfänger oder Rate 0, Debt-Ceiling.
3. **V-04/V-05:** deposit/repay absichern (Signatur oder Mindestwirkung).
4. Doku: V-07 (verwaiste 3 KAS), V-08 (reale Grenzen).
