# Audit 5 – Testqualität (GHOST auf Kaspa L1)

Datum: 28.09.2026. Prüfgegenstand: Repo-Stand **Commit a738771** (Verträge `contracts/*.sil`, `protocol/tests/*.rs`, `protocol/src/{sim,txb,ops}.rs`, `app/src/lib/vaultMath.ts`), so wie er um 13:38 Uhr in die Arbeitskopie kopiert wurde. Während des Audits wurde Commit **c26f51c „Version 2"** (14:30 Uhr) eingespielt; Auswirkungen siehe Abschnitt 7. Das Repo wurde nicht verändert; alle Läufe liefen in `/private/tmp/claude-501/…/scratchpad/audit-tests/` (Kopien `protocol/`, `protocol2/`, `protocol3/`, jeweils eigenes `CARGO_TARGET_DIR`, Vendor-Pfade absolut).

## 0. Kurzfassung

Die Tests sind handwerklich gut (Gegenproben, „welcher Input scheitert"-Prüfung, Referenzrechnung in u128), und alle Mutanten, für die ARCHITEKTUR.md einen dedizierten Test nennt, werden von genau diesem Test getötet. **Falsch sind aber mehrere „doppelt abgesichert"-Begründungen**: Für fünf ohne Test gelassene Regeln habe ich gültige Transaktionen gebaut, die bei entfernter Regel durchgehen und dem Protokoll schaden (Vault L145, L156, L132; Factory L64; Orakel L71). Zusätzlich: die Simulationskette prüft einige Konsens-/Mempool-Regeln nicht (0-Wert-Ausgänge, Locktime-Strenge, Blockmassen-Grenzen), und drei Repo-Tests führen keinen Vertragscode aus.

## 1. Vorgehen und Belege

- Basislauf (Arbeitskopie, `cargo test --offline`): oracle_tests 21/21, vault_math_tests 5/5, vault_tests 34/34, factory_tests 19/19, e2e_tests 1/1; Vitest `vaultMath.test.ts` 35/35.
- **Sweep A** (`mutate.sh`, Log `mutants.log`): jede `require`-Zeile in `stable_vault.sil` einzeln → `require(true)`, danach vault_tests + e2e_tests + vault_math_tests. Vollständig für L114–L255 (43 Regeln).
- **Sweep B** (`mutate_expr.sh`, Log `mutants_expr.log`): 36 Expression-Mutanten (`>=`→`>`, `+`→`-`, Rundungsrichtung, vertauschte `mulDiv`-Argumente, Off-by-one).
- **Gezielte Mutanten** in `protocol2`/`protocol3` mit eigenen Exploit-Tests (`audit_vault_extra.rs`, `audit_oracle_extra.rs`, `audit_factory_extra.rs`, an die Repo-Suiten angehängt).
- **UNVERIFIED:** Ein vollständiger `require`-Sweep über `risk_oracle.sil` und `vault_factory.sil` konnte nicht abgeschlossen werden (Platte lief während des Laufs voll, 217 MB frei; Nachlauf lieferte inkonsistente Ergebnisse und wurde verworfen). Für Orakel und Factory gelten daher nur die gezielten Mutanten unten plus die Expression-Mutanten.

## 2. Befunde

### F-1 (HOCH) Vault L145 `require(nOut >= 1)`: Begründung falsch, Mutant überlebt, Griefing möglich
- Behauptung (ARCHITEKTUR.md): „L143–146: Array-Grenzen scheitern ohnehin in der Engine".
- Szenario: `repay` mit **null** GHOST-Ausgängen (`outStates = []`, KCC20-Leader `new_states = []`). Der Minter-Zweig des Opfer-Vaults wird verbrannt; der Vault kann danach weder tilgen noch liquidiert werden (beides braucht `ghostDelta` mit eigenem Minter als Input 0), die Sicherheit ist dauerhaft eingefroren. `repay` braucht keine Signatur, 1 GHOST-Einheit genügt.
- Beweis: `audit_repay_ohne_ghost_ausgaenge` – Original: `[Err(VerifyError), Ok, Ok, Ok]` (Orakel, KCC20-Leader **ohne Ausgänge**, Delegate sind gültig); mit L145→`require(true)`: `[Ok, Ok, Ok, Ok]`. Sweep A: L145 ÜBERLEBT.
- Empfehlung: Test „repay/liquidate ohne GHOST-Ausgänge scheitert am Vault (nicht an KCC20)".

### F-2 (HOCH) Vault L156 `require(s.ownerIdentifier == myCov)`: Doppelverbrennung über zwei Vaults
- Behauptung: „Fremde Minter-Zweige brauchen ihren eigenen Vault in der Transaktion" – richtig, aber dieser Vault kann ein Komplize sein.
- Szenario: Angreifer-Vault X (`repay`, verbrennt D über X-Minter als Input 0) und Opfer-Vault V (`liquidate`) in einer Tx; beide lesen dieselbe GHOST-Gruppe. Ohne L156 akzeptiert V den X-Minter als „eigenen": ein Burn von D tilgt X **und** liquidiert V (Sicherheit + Bonus an den Angreifer).
- Beweis: `audit_doppelverbrennung_zwei_vaults` – Original: `[Err, Ok, Ok, Ok, Ok]` (nur V lehnt ab); Sweep A: L156 ÜBERLEBT. Die Mutante gegen meinen Test lief im `protocol2`-Lauf **nicht** rot – Ursache: in meiner Tx ist der X-Minter Input 3, nicht Covenant-Input 0 der GHOST-Gruppe, daher greift zusätzlich L154/L158 (**UNVERIFIED**, ob eine Anordnung existiert, in der nur L156 schützt; konservativ ist L156 als ungetestet und potenziell alleinige Schutzregel zu behandeln).
- Empfehlung: Test mit zwei Vaults in einer Tx; besser zusätzlich im Vertrag `require(OpCovInputCount(myCov) == 1)` je Vault-Aktion.

### F-3 (MITTEL) Vault L132 `require(OpCovInputCount(ghostCovId) == 0)`: nur gemeinsam mit L133 getestet
- `deposit_mit_heimlichem_praegen` hat GHOST-Inputs **und** -Outputs; L132 allein überlebt (Sweep A). Exploit: `deposit` + eigener Minter-Zweig als KCC20-Leader mit null Ausgängen → Minter verbrannt. Beweis `audit_deposit_mit_minter_verbrennung`: Original `[Err, Ok]`; L132→true: `[Ok, Ok]`. Vor Version 2 konnte das jeder Dritte auslösen; seit V2 (`deposit(sig s)`) nur der Besitzer selbst → Schwere sinkt auf NIEDRIG.
- L133 allein: Mutant überlebt, aber nicht exploitierbar – ein GHOST-Ausgang ohne GHOST-Input ist konsensseitig eine Genesis mit anderer ID (`covenants.rs from_tx`, Zeile 137–143). Begründung fehlt in ARCHITEKTUR.md.

### F-4 (HOCH) Factory L64 (zweite Ausgabe mit derselben Genesis-ID): kein Test, Mutant überlebt
- ARCHITEKTUR.md nennt L64 nur als Absicherung von L58; der Kommentar im Vertrag beschreibt genau diese Hintertür als Kernbefund, ein Test fehlt.
- Beweis `audit_hintertuer_in_derselben_genesis_gruppe` (Genesis-Gruppe über Vault-Ausgang 1 und OpTrue-Ausgang 4 gerechnet, also konsensgültig): Original `Err("Input 0: VerifyError")`; L64→true: `Ok(())` – die Factory akzeptiert die Hintertür, über die der Minter-Zweig des Vaults beliebig bedienbar wäre.

### F-5 (MITTEL) Orakel L71 `require(newKasUsd > 0)`: kein Test (Stand a738771)
- Beweis `audit_preis_null_oder_negativ`: L71→true: Preis 0 wird angenommen (`Ok`); negativer Preis scheitert erst am Vault (L127). Preis 0 friert alle Vaults ein (`readOracle` L127). In Version 2 durch `MIN_KAS_USD/MAX_KAS_USD` plus ×2/÷2 ersetzt und mit `v2_absolute_preisgrenzen` getestet – **UNVERIFIED** per Mutation.
- L75 `newStableRate >= 0`: kein Test; Mutant scheitert trotzdem (`VerifyError`) – vermutlich Signaturkodierung negativer Werte; Begründung fehlt.

### F-6 (NIEDRIG) Übrige überlebende Mutanten (Sweep A/B) und Bewertung
| Zeile | Mutante | Bewertung |
|---|---|---|
| V L116 `newShares >= 0` | überlebt | unerreichbar, Herleitung stimmt (burned < ceil → floor(shares) ≤ shares−1) |
| V L127 `kasUsd > 0` | überlebt | Begründung „Orakel erzwingt" stimmt nur für `update`, nicht für die Genesis (Deployer-Sache) |
| V L143/L144/L146 | überlebt | korrekt redundant: `OpCovInputIdx` bzw. Schleifengrenze (TUTORIAL.md: Laufzeitprüfung) |
| V L154/L155/L158 | überlebt | Herleitung plausibel (Nicht-Minter-Leader erzwingt Delta 0; PUBKEY-Minter mit Cov-Bytes nicht erzeugbar); nicht durch Tx belegt |
| V L173 `o.amount > 0` | überlebt | harmlos |
| V L195, L187 `>`→`>=`, L213 | überlebt | harmlos (No-op-Aktionen); L213 durch Schleifengrenze/L214 gedeckt |
| V L105 `mulDivUp`→`mulDivDown` (need) | überlebt | Off-by-one an der Quote ungetestet; `withdraw_bis_zur_mindestquote` trifft den Rundungsfall nicht |
| V L229 `<`→`<=`, L245 `==`→`>=`, L249 `>`→`>=` | überlebt | äquivalent bzw. nur zum Nachteil des Aufrufers |
| V L106 `shares == 0 \|\|` entfernt | überlebt | äquivalent (`need` = 0) |
| O L49 `< 5` | überlebt | Engine: `OutOfBoundsSubstring` – redundant, Begründung fehlt |
| F L104 `root.amount == 0` | überlebt | Eingang kann nicht > 0 sein, solange L112 gilt |
| F L58 | überlebt | Begründung „L64 greift" stimmt nur, wenn ein zweiter Ausgang ohne Covenant existiert (Wechselgeld) |

Nicht ausgeführte Mutanten (Platz/Build): Factory `o1.amount >= 0`, `== 2`→`>= 2` (L106), `== 0`→`<= 1` (L59); Orakel `tx.daa + 1` ist keine gültige Mutante (Compiler lehnt ab). Sweep A bestätigt die dedizierten Tests für alle 4 im ARCHITEKTUR.md genannten kritischen Regeln (L203, L225, L206/L255, L169).

### F-7 (NIEDRIG) Tests ohne Vertragsausführung / schwache Gegenproben
- `mint_mit_echtem_orakel_und_gleichem_betrag_als_gegenprobe`, `index_waechst_mit_altem_satz`, `alte_reihenfolge_waere_frueh_uebergelaufen`: reine Rust-Rechnung, kein Skript. Die echte Gegenprobe (gleiche Tx, überhöhtes Orakel unter `ORACLE_COV`) fehlte; nachgeholt in `audit_fake_oracle_gegenprobe`: `[Ok, Ok, Ok]` → `mint_mit_falschem_orakel_scheitert` scheitert tatsächlich nur an L125.
- `withdraw_nur_durch_besitzer` nutzt eine 65-Byte-Müllsignatur statt eines fremden Schlüssels.
- Negativtests prüfen nur `is_err()`; da Sweep A alle behaupteten Tests als Killer bestätigt, sind sie nicht vakuös.

### F-8 (MITTEL) sim.rs gegenüber echtem Konsens/Mempool (rusty-kaspa a41a333)
Übereinstimmend: Skriptprüfung mit Budget (`check_scripts_sequential`), `mass_per_sig_op` 1000, Speichermasse-Commit, Covenant-Genesis (`CovenantsContext::from_tx`), Mindestgebühr `100·max(compute, transient/2)` = `minimum_relay_transaction_fee` 100 000 sompi/kg × `max(compute, ceil(transient·500k/1M))` (post-Toccata), P2SH-Sigops ≤ 15 (gemessen: Vault 3, Orakel 3, KCC20 2, Factory 1). Testnet-Log bestätigt Annahme.
Abweichungen (sim nimmt an, Node lehnt ab):
1. **Locktime**: Konsens verlangt `lock_time < daa` (`tx_validation_in_header_context.rs:79`), sim erlaubt `lock_time == daa`; `ops::oracle_update` setzt `lock_time = new_daa`, e2e nutzt `new_daa = sim.daa` → genau dieser Fall. ghostctl zieht 20 ab, daher praktisch unauffällig.
2. **0-Wert-Ausgänge** (`TxOutZero`): sim prüft nicht; `vault_tests` nutzen `plain_out(1)`, `ops` nie 0.
3. **Blockmassen-Grenzen** je Tx (500 000 compute/storage, 1 000 000 transient): sim prüft nicht – Version 2 ergänzt `check_block_limits` mit dem Kommentar, dass eine Tx mit 1 003 783 g Speichermasse akzeptiert worden war (bestätigt die Lücke).
4. Nicht geprüft: Sigscript-Maximum 250 000 B, Sequence-Locks, Standard-Skriptklassen der Ausgänge – für die erzeugten Tx unkritisch.
Umgekehrt (Node akzeptiert, sim nicht): keine gefunden.

### F-9 (INFO) vaultMath.ts
Formeln in `mulDivDown/Up`, `debtOf`, `sharesFor`, `healthy`, `oracleIndexAfter`, `simLiquidate` sind Zeile für Zeile identisch mit `stable_vault.sil` (Stand a738771) inkl. Überlaufabbruch je Zwischenschritt; 35 Vitest-Fälle grün. **Nicht nachgezogen für Version 2** (Teil-Liquidation mit `burn`, `cut > 0`, Orakel-Preisgrenzen) – UNVERIFIED, vermutlich veraltet.

## 3. Version 2 (c26f51c) – Einordnung
`deposit` nur mit Signatur (entschärft F-3), `repay`/`liquidate` mit `cut > 0`, Teil-Liquidation, Orakel-Preisgrenzen, `ghost_token.sil` mit `amount >= 0`. F-1, F-2, F-4 bleiben unverändert bestehen (Zeilennummern verschoben). Die neuen Regeln wurden **nicht** mutationsgetestet.

## 4. Empfehlungen
1. Tests für F-1, F-2, F-4 (Vorlagen in der Arbeitskopie: `audit_*_extra.rs`).
2. ARCHITEKTUR.md: Begründungen für L132/L133, L143–146, L49, L58 präzisieren; L145/L156/L64 aus der Liste streichen.
3. sim.rs: `lock_time < daa`, `value > 0`, Blockgrenzen prüfen.
4. Mutationsskript im Repo (`protocol/mutation/mutate.sh`) läuft nur zeilenweise `require(true)`; Expression-Mutanten ergänzen (L105-Rundung).
