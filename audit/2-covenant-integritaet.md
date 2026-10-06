# Audit 2 — Mengenintegrität des Covenant-Systems (StableVault + VaultFactory + KCC20/GHOST)

Stand: 2026-09-28. Auditor (unabhängig). Umfang laut Auftrag: Supply-Integrität
über StableVault, VaultFactory und den KCC20-Token GHOST. Kein Netz-Versand,
keine Repo-Änderung, keine keys/. Alle Belege gegen die echte Skript-Engine aus
rusty-kaspa `a41a333` und den SilverScript-Compiler v1.0.0 (3ed9733).

Untersuchte Dateien: `contracts/stable_vault.sil`, `contracts/vault_factory.sil`,
`contracts/risk_oracle.sil`, KCC20 (`vendor/.../examples/kcc20.sil`),
`protocol/src/{ops,txb,contracts,sim}.rs`, Compiler-Lowering
(`vendor/silverscript/silverscript-lang/src/compiler/{for.rs,compile/state.rs,
compile/validate_output_state.rs,read_input_state.rs}`), Opcodes und
`covenants.rs` (from_tx) aus rusty-kaspa.

Ausführbare Belege: `audit/audit_supply.rs` (Kopie; ausgeführt aus dem
Scratch-Build `…/scratchpad/audit-covenant/protocol`, `CARGO_TARGET_DIR` =
`vendor/silverscript/target`). Alle drei Tests grün, Ausgabe unten je Befund.

## Gesamturteil

Für den geprüften Umfang wurden **keine kritischen oder hohen** Lücken gefunden,
über die sich GHOST ohne Schuldbuchung prägen, ein fremder Minter-Zweig kapern
oder KAS aus Covenant-UTXOs abziehen ließe. Die zentralen Schutzregeln habe ich
nicht nur gelesen, sondern gegen die Engine nachgestellt bzw. aus dem
Compiler-Lowering hergeleitet. Es bleiben zwei **niedrig/informativ** eingestufte
Punkte (verwaiste Minter-Zweige = dauerhaft gebundene KAS; nicht-minimale
Zahlencodierung als Annahme) sowie mehrere architektonische Hinweise.

---

## Verifizierte Schutzmechanismen (Beleg, keine Lücke)

### V1 — for-Schleifen erzwingen `require(end - start <= max)` (Kern der Mengenerhaltung)

Der zentrale Verdacht war: Da der KCC20-**Minter**-Leader (`isMinter`) die
Mengenerhaltung überspringt (`checkAmounts`/`checkMintingTransfer` nur für
Nicht-Minter) und `MAX_GHOST_OUTS = 2` gilt, ließe sich ein **dritter**
GHOST-Covenant-Ausgang an der Vault-Buchhaltung *und* am KCC20-Leader
vorbeischmuggeln, wenn die Schleife `for(j,0,nOut,2)` bei `nOut > 2` still nur
zweimal liefe.

Befund: `compiler/for.rs:lower_for_statement` erzeugt vor der (auf `max`
abgerollten) Schleife den Laufzeit-Guard
`require(end_name - start_name <= max_iterations)`. `ghostDelta`
(`stable_vault.sil:164`) läuft damit effektiv auf `require(nOut <= 2)`, der
KCC20-Leader (`to = maxCovOuts = 2`) analog. Ein dritter Ausgang wird also nicht
ignoriert, sondern hart abgelehnt — von **beiden** Seiten.

Beleg `drei_ghost_ausgaenge_werden_am_schleifen_guard_abgelehnt`:
```
drei_ausgaenge => [Err("VerifyError"), Ok(()), Err("VerifyError"), Ok(())]
```
Input 0 (Vault) und Input 2 (Minter-Leader) lehnen ab; Orakel und Zahler-Token
(reine Token-Seite) wären für sich gültig. Damit ist „mehr GHOST ausgeben als
verbucht" über zusätzliche Covenant-Ausgänge ausgeschlossen. Dies bestätigt die
Behauptung in ARCHITEKTUR.md („Mehr als 2 Ausgänge scheitern an der
Schleifengrenze") — jetzt gemessen, nicht nur behauptet.

### V2 — Fremder Minter-Zweig kann nicht gekapert werden

`ghostDelta` verlangt für GHOST-Gruppen-Input 0: `isMinter`,
`identifierType == COVENANT_ID`, `ownerIdentifier == OpInputCovenantId(this.activeInputIndex)`
(`stable_vault.sil:153-156`). Ein Minter-Zweig, der einem anderen Vault gehört,
wird abgelehnt.

Beleg `fremder_minter_zweig_wird_abgelehnt`:
```
fremder_minter => [Err("VerifyError"), Ok(()), Err("VerifyError"), Ok(())]
```
(Gegenprobe `ehrliches_repay_geht_durch` → `[Ok, Ok, Ok, Ok]`.)

### V3 — Positions- und Reihenfolge-Annahmen sind abgesichert

`OpCovInputIdx/OpCovOutputIdx` liefern die Indizes in Transaktions-Reihenfolge
(rusty-kaspa `covenants.rs`, `shared_ctxs…input_indices/output_indices` per
`push` beim Enumerieren). Der Vault fixiert Gruppen-Input 0 = Minter (V2) und
Gruppen-Output 0 = unveränderte Minter-Fortsetzung mit `amount == 0`
(`stable_vault.sil:166-170`). Wird ein Nicht-Minter-Token auf Gruppenposition 0
gelegt, ist zugleich *es* der KCC20-Leader und erzwingt Mengenerhaltung
(kein Prägen), während der Vault am `isMinter`-Check scheitert. Beide Wege
schützen sich gegenseitig. Da **alle** GHOST-Tokens dieselbe Covenant-ID tragen
(`ops.rs` `open_vault`: Minter-Zweig als Output unter `root.cov`), zählen
`OpCovInputCount/OpCovOutputCount(ghostCovId)` über die **ganze** Tx — die
Bündelung mehrerer Vaults im selben GHOST-Vorgang ist dadurch (und durch V1)
blockiert, nicht nur konventionell vermieden.

### V4 — Jede GHOST-berührende Aktion ist vollständig verbucht

`mint`, `repay`, `liquidate` gehen ausnahmslos durch `ghostDelta`
(vollständige In-/Out-Summe der GHOST-Gruppe). `deposit`, `withdraw`, `close`
erzwingen `noGhost()` (`OpCovInputCount == 0 && OpCovOutputCount == 0`,
`stable_vault.sil:131-134`). Es gibt keinen Pfad, der GHOST anfasst, ohne die
Differenz gegen `debtShares` zu prüfen. `mint` verlangt zusätzlich Besitzer-
Signatur; `repay`/`liquidate` können nur verbrennen (`burned > 0` bzw. volle
Schuld), nie prägen.

### V5 — KAS-Abfluss aus Covenant-UTXOs ist gesperrt

`validateOutputState` prüft den Betrag nicht (ARCHITEKTUR.md B7). Alle Verträge
gleichen das selbst aus: `continueWith`/`continuation` erzwingen
`tx.outputs[outIdx].value == value` (Vault `:120`, Orakel `:36`, Factory `:50`)
mit `value` = Eingangswert bzw. exakt errechnetem Rest. Die Liquidations-
Auszahlung ist über die erzwungene Fortsetzung `rest = coll - seize` gedeckelt;
mehr als `seize` kann der Liquidator dem Vault nicht entnehmen.

### V6 — readInputStateWithTemplate / …WithInputTemplate authentifizieren echt

`compile_read_input_state_with_template_validation` (state.rs) rekonstruiert das
Redeem-Skript aus dem **eigenen** Sigscript des referenzierten Inputs, prüft
`blake3(template) == expected` **und** `P2SH(redeem) == input.scriptPubKey`
(`OpEqualVerify`). Der Zustand (mittlerer Bereich) ist damit an die real
committete UTXO gebunden; gefälschte Preise/Beträge/Minter-Flags eines Inputs
sind ausgeschlossen. Der Vault fixiert zusätzlich die Herkunft über
`OpInputCovenantId(oracleIdx) == oracleCovId` (`:125`) bzw. die feste
`ghostCovId`. Ein Fremd-Orakel wird abgelehnt (bestehender Test
`mint_mit_falschem_orakel_scheitert`).

### V7 — Genesis-Semantik der Factory ist korrekt behandelt

`covenants.rs::from_tx` fügt Genesis-Ausgaben **nicht** in die Skript-Kontexte
ein (`input_ctxs`/`shared_ctxs`), validiert aber deren ID per
`covenant_id(outpoint, outputs)`. `freshGenesis` (`vault_factory.sil:56`)
schließt die daraus folgende Hintertür (zweite Ausgabe mit gleicher neuer ID)
explizit aus: `id != 0`, `OpCovInputCount(id) == 0`, Schleife über alle Ausgaben
(`n <= 8`) mit `OpOutputCovenantId(i) != id` für `i != idx`. Bestehende Tests
(`factory_tests.rs`) decken die Angriffe (fremder Covenant als „Vault",
Vorab-Guthaben, Wurzel-Minter an Fremde, zweites `init`) ab; Gegenproben
vorhanden.

---

## Befunde

### F1 — Verwaiste Minter-Zweige binden KAS dauerhaft (Niedrig; hergeleitet)

- **Ort:** `contracts/stable_vault.sil` `close` (`:202`) und Total-Liquidation
  (`:253-255`); `ops.rs` `open_vault`/`BRANCH_VALUE` (3 KAS je Zweig).
- **Szenario:** Nach `close` (Schuld 0, keine Vault-Fortsetzung) bzw.
  Total-Liquidation (`rest < DUST`, keine Fortsetzung) existiert die Vault-
  Covenant-ID als UTXO nicht mehr. Der zugehörige GHOST-Minter-Zweig lebt aber
  als eigener GHOST-UTXO mit `ownerIdentifier = vaultCovId`,
  `identifierType = COVENANT_ID` weiter (bei Liquidation wird er als
  Gruppen-Output 0 sogar frisch fortgesetzt).
- **Herleitung/Beleg:** KCC20 erlaubt das Ausgeben eines
  Covenant-ID-eigenen Zweigs nur, wenn die Tx einen Input mit passender
  Covenant-ID enthält (`kcc20.sil:18` `require(OpCovInputCount(ownerIdentifier) > 0)`
  bzw. Array-Variante `OpInputCovenantId(witnesses[i]) == ownerIdentifier`). Da
  keine UTXO mehr die tote `vaultCovId` trägt und eine Genesis diese ID (Hash aus
  einem konkreten, bereits ausgegebenen Outpoint) nicht reproduzieren kann, ist
  der Zweig **dauerhaft unspendbar**.
- **Wirkung:** Kein Diebstahl und **kein** späteres Prägen (die GHOST-Schuld
  des Vaults ist vor `close` vollständig getilgt bzw. bei Liquidation in
  derselben Tx verbrannt). Es sind lediglich die 3 KAS (`BRANCH_VALUE`) je
  geschlossenem/liquidiertem Vault für immer gebunden. Selbst verursacht,
  in `MAINNET.md` als „bleibt nach dem Schließen verwaist" bereits benannt.
- **Empfehlung:** In der Doku/UI klar als nicht rückholbar ausweisen; optional
  einen `close`-Pfad ergänzen, der den Minter-Zweig kontrolliert einzieht
  (Menge 0, keine Fortsetzung), um die KAS freizugeben. Reine Verbesserung,
  keine Sicherheitslücke.

### F2 — Nicht-minimale Zahlencodierung bei aktivierten Covenants (Info; nicht ausnutzbar)

- **Ort:** rusty-kaspa `crypto/txscript/src/data_stack.rs:pop_items` →
  `deserialize(!self.covenants_enabled)`. Bei aktivierten Covenants wird die
  **Minimal-Codierung nicht erzwungen**; 8-Byte-Werte werden ohne
  Minimalitätsprüfung als `i64` gelesen.
- **Analyse:** Im GHOST-System wird jeder Zustand vom jeweiligen Vertrag
  kanonisch geschrieben (`compile_encoded_state_fields`: `OpNum2Bin` fester
  Breite) und die Ausgabe-SPK gegen die Rekonstruktion geprüft
  (`validateOutputState*`). Eine UTXO mit nicht-minimal codiertem Zustand kann
  also gar nicht entstehen, weil ihr Ersteller den Ausgang kanonisch
  rekonstruiert und per `OpEqualVerify` erzwingt. Ein Angreifer kann somit keine
  „krumme" Menge/kein manipuliertes `isMinter`-Flag einschleusen.
- **Status:** **UNVERIFIED als Lücke** — kein ausnutzbarer Pfad gefunden.
  Als Annahme dokumentiert: Die Sicherheit hängt daran, dass **alle**
  zustandstragenden UTXOs ausschließlich über die templatebasierten
  Builtins erzeugt/gelesen werden (im Code erfüllt).

### F3 — Liveness-/Obergrenzen als kontrollierter Abbruch (Info)

- Jenseits der dokumentierten Grenzen (Preis ≤ 920 USD/KAS, Sicherheit
  ≤ 1e8 KAS, Index bis ~9200×) bricht die Engine mit `NumberTooBig` ab statt
  falsch zu rechnen — Fail-safe, kein Supply-Risiko, aber eine
  Verfügbarkeitsgrenze. Zusätzlich: **ein Vault je GHOST-berührender Tx**
  (einziger Leader, V3). Beides ist Design/Doku, keine Lücke.

---

## Nicht abschließend geprüft / Empfehlung für weitere Runde

- Vollständige Kombinatorik „mehrere Vaults + Orakel-Update + Factory in einer
  Tx" nur analytisch (V3) ausgeschlossen, nicht für alle Permutationen
  ausgeführt. Da GHOST **eine** Covenant-ID ist und V1/V3 greifen, sehe ich kein
  offenes Risiko, aber ein erschöpfender fuzzing-artiger Test wäre die saubere
  Absicherung.
- Massen-/Gebührenverhalten unter Blocklimits ist Verfügbarkeit, nicht Umfang
  dieses Audits.

## Reproduktion

```
# Scratch-Build (Repo unverändert):
cd …/scratchpad/audit-covenant/protocol
CARGO_TARGET_DIR=…/kaspa-lending/vendor/silverscript/target \
  cargo test --test audit_supply -- --nocapture
# 3 passed: drei_ghost_ausgaenge…, fremder_minter…, ehrliches_repay…
```
Testquelle liegt als `audit/audit_supply.rs` bei.
