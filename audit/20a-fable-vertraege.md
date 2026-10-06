# Audit 20, Teil a: Verträge der Version 4 (Fable, 06.10.2026)

Unabhängige Prüfung der Verträge in `contracts/`, die das Mainnet-Deployment Version 4 benutzt: `signer_register_v4.sil`, `price_oracle_v4.sil`, `stable_vault_v4.sil`, `ghost_pool_v4.sil`, `ghost_token.sil`, `vault_factory.sil`, `standing_order.sil`, `zinskasse_v4.sil`. Dazu die Rust-Gegenstücke `protocol/src/contracts.rs`, `ops.rs`, `sim.rs` und die Konsensstellen in rusty-kaspa a41a333 (`crypto/txscript/src/covenants.rs`, `opcodes/mod.rs`, `tx_validation_in_utxo_context.rs`) sowie der SilverScript-Compiler (`compile/statement.rs`, `docs/DECL.md`).

Arbeitsweise: Verträge Zeile für Zeile gelesen, jeden Verdacht mit einem eigenen Engine-Test gegen die echte Skript-Engine geprüft (temporäre Datei `protocol/tests/audit20a_tests.rs`, 7 Tests, alle grün, nach dem Lauf entfernt; Kern im Anhang). Keine Transaktion gesendet, keine Datei unter `keys/` gelesen, Produktivcode und Verträge unverändert. Live-Parameter aus `deployments/mainnet.json` (nur Parameter- und Zustandsfelder): Register 1-von-1 (`min_signers = min_threshold = 1`, Fristen 14 Tage / 30 Tage / 14 Tage), **kein Notfallsatz** (`fb_hash = 0`), Orakel `freeze_after_daa = 72 000`, `max_rate` 20 %/J, Vault `max_debt` = 50 GHOST, MCR 200 %, Liquidation 150 %, Bonus 10 %, `interest_spk` = P2PK des Betreibers (keine Zinskasse), Zins derzeit 0.

**[B]** = belegt mit Test, **[V]** = aus dem Code hergeleitet, nicht ausgeführt.

Frühere Befunde (AUDIT.md, `audit/`) sind nicht wiederholt; wo ich Behebungen geprüft habe, steht es unter „geprüft und sauber“.

---

## Befundliste

| ID | Schwere | Kurz | Status |
|---|---|---|---|
| A20a-1 | **mittel** (Vertrag); im Live-Deployment ohne Notfallsatz ohne Wirkung | Replay der letzten Preis-Signatur mit Orakel-`read` entwertet ein offenes Notfall-Ticket, ohne Schlüssel | [B] |
| A20a-2 | niedrig (Designgrenze) | Gegen einen aktiven Angreifer mit t Schlüsseln gibt es keinen Wiederherstellungsweg | [V] |
| A20a-3 | niedrig | `sweep` bei Sicherheit unter `SWEEP_FEE`: Kassen-Ausgang darf 1 sompi sein, der Rest geht an den Auslöser | [B] |
| A20a-4 | Hinweis | Vorhandener Test `preis_mit_orakel_read_statt_update_scheitert` belegt nicht, was er behauptet | [B] |
| A20a-5 | Hinweis | Tresor: `maxFee` ist keine Netzgebühr-Grenze, sondern eine mögliche Prämie für den Auslöser | [B] |
| A20a-6 | Hinweis | Bei eingefrorenem Orakel rechnen `close` und `withdraw` (ohne Schuld) den Zins zum veralteten Preis | [B] |
| A20a-7 | Hinweis | Live-Deployment 1-von-1 ohne Notfallsatz: Schlüsselverlust friert das System dauerhaft ein, Schlüsselkompromittierung erlaubt in ≈ 15 min jeden Preis | [V], bekannt aus AUDIT.md, jetzt mit einem Schlüssel |

Kein Befund der Stufe kritisch oder hoch. Ich habe keinen Weg gefunden, Geld zu stehlen, GHOST ungedeckt zu prägen, fremde Vaults, Tresore oder Pool-Anteile zu bewegen, das Orakel oder das Register ohne die Schlüssel zu übernehmen, Zins oder Index außerhalb des Vertragsrahmens zu verändern, die Kovenant-Kontinuität zu brechen oder Folgezustände zu fälschen.

---

## A20a-1 (mittel): Replay der letzten Preis-Signatur mit `read` tötet das Notfall-Ticket

**Ort:** `contracts/signer_register_v4.sil:175–204` (`attestPrice`), `contracts/price_oracle_v4.sil:60–66` (`read`).

**Was fehlt:** `attestPrice` prüft, dass der Orakel-**Ausgang** genau den signierten Zustand `(kasUsd, oracleDaa, seq, rate, frozen = false)` trägt. Es prüft aber nicht, dass das Orakel in dieser Tx `update` ausführt, also dass `seq` gegenüber dem Orakel-**Eingang** steigt. Die Doku (`price_oracle_v4.sil:13–16`, `docs/v4-entwurf.md` 2.2) begründet das Weglassen damit, dass „attestPrice seq + 1 verlangt“. Das stimmt nur, wenn die Signatur über `seq + 1` lautet. Die Signaturen jedes echten Updates stehen aber öffentlich im Sigscript auf der Kette, und nach dem Update trägt das Orakel genau den signierten Zustand. Wer diese Bytes wiederholt und das Orakel `read` ausführen lässt (Zustand unverändert), erfüllt beide Prüfungen.

**Wirkung:** Der Register-Ausgang ist dann `lastDaa = newDaa` (unverändert) und bei `emerg = true` **`nonce + 1`, `emerg = false`** (`:193–196`). Ein offenes Notfall-Ticket ist damit tot. Das Update sollte beweisen, „dass der Hauptsatz lebt“ (`:173–174`); der Replay fälscht diesen Beweis, kostet ≈ 0,02 KAS und braucht keinen Schlüssel. Der Notfallweg ist der einzige Weg, einen verstummten Hauptsatz zu ersetzen (bei Schlüsselverlust). Ein Angreifer kann jede Notfall-Ankündigung sofort entwerten, solange das Orakel nicht eingefroren ist. Ohne Notfall-Ticket ist der Replay ein No-op (nonce bleibt, belegt).

**Beleg [B]:** Test `a20a_1`: ehrliches Update mit 4 von 7 Signaturen; danach Welt mit `emerg = true, nonce = 6`; dieselben Signatur-Bytes mit Orakel-`read` → alle Eingänge gültig, Register-Ausgang `nonce = 7, emerg = false`. Gegenproben: ohne Notfall-Ticket darf die nonce nicht steigen (abgelehnt); mit eingefrorenem Orakel scheitert der Replay (Register verlangt `frozen = false`, `read` reproduziert `true`); mit `freeze` als Orakel-Eintrag scheitert er ebenfalls. Test `a20a_2`: nach einem Satzwechsel scheitert der Replay an `hashSet`.

**Abhilfe:**
1. Im Vertrag (empfohlen, wenige Bytes): in `attestPrice` den Orakel-Eingang lesen und `newSeq == alt.seq + 1` verlangen, z. B. `OracleState cur = readInputStateWithTemplate(oIn, oraclePrefixLen, oracleSuffixLen, oracleTpl); require(newSeq == cur.seq + 1);`. Dann ist `read` neben `attestPrice` unmöglich. Das Register fährt nur bei Updates mit, der Mehrpreis je Update ist gering.
2. Off-chain, solange der Vertrag steht: Der Notfallsatz muss das Orakel **vor** `propose --emergency` einfrieren (jeder darf das nach 2 h; nach 30 Tagen Stille ist das immer möglich), und `ops::propose(emergency = true)` sollte `frozen == true` verlangen (`ops.rs:566–573` prüft das heute nicht). Ist das Orakel eingefroren, kann es nur ein echtes Update mit Schlüsseln des Hauptsatzes auftauen; der Replay bleibt für die 14 Tage Wartezeit gesperrt.

Für das Live-Deployment ohne Notfallsatz ist der Befund ohne Wirkung (es gibt nie ein Notfall-Ticket). Er betrifft den Vertrag, sobald ein Notfallsatz eingetragen wird – das ist nur über `propose → activate` möglich und wäre dann der Zeitpunkt, den Vertrag nicht zu ändern, sondern die Regel 2 strikt einzuhalten.

## A20a-2 (niedrig, Designgrenze): kein Wiederherstellungsweg gegen einen aktiven Angreifer mit t Schlüsseln

**Ort:** `signer_register_v4.sil:254–268` (`cancel` mit `t` Signaturen), `:214–250` (`propose` mit `tRot ≥ t`), `:223` (Notfall nur bei Stille `lastDaa + emergAfterDaa`), `:175–204` (`attestPrice` setzt `lastDaa`).

**Szenario [V]:** Ein Angreifer hält `t` Schlüssel (Preis-Quorum), die ehrliche Mehrheit `tRot`. Die ehrlichen Unterzeichner kündigen einen neuen Satz an; der Angreifer sagt mit `t` Signaturen ab (`cancel`), beliebig oft. Zugleich hält er den Preis mit `attestPrice` aktuell, `lastDaa` wandert mit, der Notfallweg wird nie frei (er setzt Stille voraus). Ergebnis: Patt, der Angreifer bestimmt den Preis innerhalb der Grenzen dauerhaft. Mit den Vorschlagswerten (7 Schlüssel, `t = tRot = 4`) ist `t = tRot`, dann ist der Punkt gegenstandslos; bei `t < tRot` (Doku-Beispiel 7 / 4 / 5) nicht. Im Live-Deployment (1/1/1) ist jede Kompromittierung ohnehin vollständig (A20a-7).

**Vorschlag:** `cancel` mit `tRot` statt `t` signieren lassen, oder festhalten, dass `t = tRot` zu wählen ist. Eine Notfallklausel ohne Stille-Bedingung wäre falsch (dann könnte der Notfallsatz den Hauptsatz jederzeit verdrängen).

## A20a-3 (niedrig): `sweep` unter `SWEEP_FEE`

**Ort:** `stable_vault_v4.sil:313–321`, `:179–183` (`payTreasury`).

`sweep` verlangt `payTreasury(coll − SWEEP_FEE)`. Ist die Sicherheit kleiner als `SWEEP_FEE` (0,1 KAS), ist die Forderung negativ, und `tx.outputs[idx].value >= fee` ist mit jedem Betrag erfüllt. Der Auslöser legt 1 sompi an die Kasse und nimmt den Rest. **Beleg [B]:** Test `a20a_3`: Sicherheit 0,05 KAS, Zins 0,003 USD (> Wert 0,002 USD), Kassen-Ausgang 1 sompi, Rest an den Auslöser → Vault akzeptiert. Gegenprobe bei 0,5 KAS: Kasse muss genau `coll − SWEEP_FEE` bekommen (1 sompi weniger wird abgelehnt).

Wirkung: unter 0,1 KAS je Zombie-Vault, abzüglich ≈ 0,055 KAS Netzgebühr – wirtschaftlich belanglos, aber ein vorzeichenloses `fee`, das die Doku („alles bis auf SWEEP_FEE an die Kasse“) nicht hält. `ops::sweep` baut solche Tx nicht (A12-2). **Vorschlag:** `require(coll > SWEEP_FEE)` in `sweep`, oder die Kasse bekommt `max(0, coll − SWEEP_FEE)` ausdrücklich; im nächsten Deployment.

## A20a-4 (Hinweis): Test belegt nicht, was er behauptet

**Ort:** `protocol/tests/register_v4_tests.rs:311–321`.

`preis_mit_orakel_read_statt_update_scheitert` setzt `arg_seq = seq`, signiert aber über `seq + 1` (`Attest::run`, `signed_seq.unwrap_or(new_seq)`). Die Tx scheitert deshalb an `checkQuorum`, nicht an der Ausgangsprüfung – der Fall, den der Kommentar beschreibt („Register verlangt seq + 1 im Orakel-Ausgang“), ist ungetestet. Genau diese Lücke verdeckt A20a-1. **Vorschlag:** `signed_seq = Some(w.oracle.seq)` dazu, und der Test wird rot; dann die Abhilfe aus A20a-1 einbauen, damit er wieder grün wird.

## A20a-5 (Hinweis): Tresor-`maxFee` als Prämie

**Ort:** `standing_order.sil:116–137`, Doku `ARCHITEKTUR.md` („höchstens maxFee aus dem Tresor für die Netzgebühr“).

`pay` verlangt nur, dass die Fortsetzung ≥ `Wert − amount − maxFee` behält; weitere Ausgänge sind frei. Wer auslöst, kann `maxFee − Netzgebühr` als eigenen Ausgang nehmen. **Beleg [B]:** Test `a20a_7`: Zahlung 10 KAS, `maxFee` 0,01 KAS, dritter Ausgang 0,00999999 KAS an den Auslöser → gültig. Mit dem Standard 0,01 KAS und ≈ 0,0025 KAS Gebühr sind das ≈ 0,0075 KAS je Zahlung, nur einmal je Termin. Ein Vertrag kann die Gebühr nicht von einem Ausgang unterscheiden; die Doku sollte „Prämie bis maxFee an den, der auslöst“ sagen. Das ist zugleich der Anreiz, fremde Tresore fristgerecht auszulösen.

## A20a-6 (Hinweis): Zins zum veralteten Preis bei eingefrorenem Orakel

**Ort:** `stable_vault_v4.sil:297–307` (`close`, `fresh = false`), `:284–292` (`withdraw`, `fresh = debt > 0`).

Belegt [B] in `a20a_4` und `a20a_6`: Bei `frozen = true` geht `close` durch, die Zinsgebühr wird zum eingefrorenen Preis in KAS gerechnet; `withdraw` ohne Schuld geht durch und muss nur den Zins zum alten Preis mit MCR decken; mit Schuld ist `withdraw` gesperrt, nach dem Auftauen wieder frei. Das ist die dokumentierte Absicht („Besitzer kommen raus“). Konsequenz: Fällt KAS während eines Stillstands, bekommt die Kasse weniger als den Zins wert ist; der Zins ist klein, das Risiko trägt der Betreiber. Kein Handlungsbedarf, nur festhalten.

## A20a-7 (Hinweis): Live-Deployment 1-von-1 ohne Notfallsatz

Aus `deployments/mainnet.json`: ein Unterzeichner, `fb_hash = 0`. Folgen [V]:
- **Schlüsselverlust:** Nach 2 h darf jeder einfrieren; kein Eintrag des Registers oder Orakels kommt ohne diesen Schlüssel zurück (Notfallweg ohne Urbild unmöglich, `propose` braucht `tRot` des Hauptsatzes). Dauerhaft gesperrt: `mint`, `redeem`, `liquidate`, `sweep`, `withdraw` mit Schuld, Pool-`swap`/`init` (`stopWhenFrozen`). Frei bleiben `deposit`, `repay`, `close`, `withdraw` ohne Schuld, Pool-`add`/`remove`. Nutzer kommen also heraus, aber Unterdeckung wächst ohne Liquidation.
- **Schlüsselkompromittierung:** Preis je Update ×2 oder ÷2, Mindestabstand 600 DAA bei `tx.daa ≥ newOracleDaa` – von 0,043 auf 0,00001 USD in 13 Updates, also ≈ 13 min, wenn das Orakel am Kettenstand liegt. Danach kauft 1 GHOST bei `liquidate` jede Sicherheit. Das ist in AUDIT.md („Bekannte Grenzen“) für 3 von 5 beschrieben; jetzt genügt ein Schlüssel. Der Zinsrahmen (`rateStep`, `rateGapDaa`) bremst nur den Zins, nicht den Preis.

Empfehlung: Den Notfallsatz per `propose → activate` nachtragen (geht ohne neues Deployment) und dabei A20a-1 beachten (vor einem Notfall einfrieren).

---

## Geprüft und sauber

**Relative Sperre / `this.ageDaa` (Ticket-Wartezeit).** Der Compiler (`compile/statement.rs:360–372`) prüft den Vergleichswert mit `OpWithin [0, 2^32)` und ruft `OpCheckSequenceVerify`. Das Opcode (rusty-kaspa `opcodes/mod.rs:1066–1104`) lehnt eine Eingangssequenz mit gesetztem Disable-Bit ab und vergleicht maskiert; der Konsens (`tx_validation_in_utxo_context.rs:136–161`) erzwingt `UTXO-DAA + Sequenz − 1 < Block-DAA`. Ein Umgehen über das Disable-Bit (Konsens überspringt dann die Sperre) ist ausgeschlossen, weil das Opcode genau das verbietet. Mainnet-Probe 05.10. bestätigt die Ablehnung einer zu frühen Aktivierung.

**Kovenant-Kontinuität.** `cont()` (Register), `continuation()` (Orakel, Factory), `continueWith()` (Vault), `continuePool()` (Pool) verlangen `OpAuthOutputCount == 1` und gleichen Betrag bzw. geprüften Betrag. Nach `covenants.rs:101–146` zählen nur Ausgänge mit derselben Covenant-ID wie der autorisierende Eingang als Fortsetzung; Genesis-Ausgänge (andere ID) liegen außerhalb der Skript-Kontexte und werden per ID-Rekonstruktion geprüft. `freshGenesis` in Factory und Pool deckt den Genesis-Fall ab. Die Haupt-UTXO des Registers ist unsterblich (jeder Eintrag außer `settle` setzt sie fort, `settle` verlangt `isTicket`), ebenso das Orakel.

**Register.** `nonce` ist streng monoton → Signaturen für `propose`/`cancel` (binden `regCov ‖ Tag ‖ nonce + 1 [‖ Ankündigung]`) sind nicht wiederholbar; nur die Preis-Signatur ist es (A20a-1). `activate` liest das Ticket als Eingang, der Ticket-Eingang kann nur `settle` ausführen (alle anderen Einträge verlangen `!isTicket`, `init` auch), `settle` prüft bei gleicher nonce das Alter und den aktivierten Ausgang der Haupt-UTXO. Ein Ticket kann nie Haupt-UTXO werden, ein abgelaufenes Ticket darf jeder aufräumen (KAS des Tickets gehen frei). `settle` und `init` haben kein `noOracle`, aber `settle` braucht zwei Register-Eingänge (Orakel verlangt genau einen), `init` verlangt `OpCovInputCount(oCov) == 0`. `checkQuorum`: Indizes streng aufsteigend ab −1, `< n`, `keys.length == n` im Hash. Emergency-`propose` prüft den Notfallsatz auf die Grenzen und `tx.daa ≥ lastDaa + emergAfterDaa`. Mutationslog: Lücken L107/L115/L126/L127/L132/L173/L300 wie in `docs/v4-entwurf.md` begründet, nachvollzogen.

**Orakel.** `update` nur mit genau einem Register-Eingang; Grenzen 0,00001–900 USD, ×2/÷2 **genau** an der Grenze (Test `a20a_5`: ×2 geht, ×2 + 1 sompi nicht; ÷2 geht, ÷2 − 1 nicht), Abstand ≥ 600 DAA, `tx.daa ≥ newOracleDaa` (keine Zukunft), Zinsrahmen `rateStep`/`rateGapDaa`, `frozen → false`. Index: `stableRate · Δ / 1e9`, `index · growth / 1e9` – bei `maxRate` und 1 Jahr Pause (3,15e8 DAA) ≈ +20 %, kein Überlauf (Test). `freeze` nur ab `oracleDaa + freezeAfterDaa`, nur `frozen` ändert sich; `read` unverändert. Ein `update` oder `freeze` neben einer Vault-Aktion in derselben Tx: der Vault liest den Eingangszustand, also den bis dahin gültigen Preis – kein Vorteil für Dritte.

**Vault v4.** Jeder Eintrag prüft die ganze GHOST-Seite (`noGhost` oder `ghostDelta`: Minter des eigenen Vaults am Kovenant-Index 0, Fortsetzung mit Betrag 0 und gleichem KAS-Wert, weitere Ausgänge keine Minter mit Betrag > 0, Eingänge keine Minter). Der Minter-Zweig ist nur mit dem Vault ausgebbar (`OpCovInputCount(vault) > 0` im Token) und dann nur über `ghostDelta`. Arithmetik an den Grenzen: `healthy` bei 1e8 KAS × 900 USD = 9,0e18 < 9,22e18; `growth` exakt bis Index 9,2e12 (`t2·1e6 < 9,2e18`), darüber Deckel ×10; `accrual` (d/1e9)·g ≤ 1,8e18, (d%1e9)·g < 9e18; `interestFee`/`redeem`/`liquidate` bei `maxDebt` 50 GHOST weit unter den Grenzen (A12-14 bestätigt). Rundung: `accrual`, `interestFee`, `need`, `claim`, `seize` aufgerundet (zugunsten Kasse/Liquidator), `value`, `usd`, `paid` abgerundet (zugunsten Vault) – wie dokumentiert. `payTreasury` fest an `idx + 1` (A11-V-1 hält). Sperren bei `frozen` für `mint`, `redeem`, `liquidate`, `sweep`, `withdraw` mit Schuld (Test `a20a_6` für `withdraw`, v4-Tests für die übrigen). `maxDebt` nur beim Prägen, `MAX_GHOST_OUTS` ausdrücklich. `indexAt = 0` aus der Factory: `accrual` schützt mit `from > 0`, erste Abrechnung setzt `indexAt`. Liquidation: Teilablösung, Rest < DUST nur bei voller Tilgung, Vollverlust schreibt Restschuld ab (V-02-Design).

**Pool v4.** `checkBand` exakt über `productGe` (Faktoren < 2^60 nachgerechnet: `k ≤ 9e14`, `x2, y2 ≤ 1e16`, Refs ≤ 1,01e14); `rises`/`falls` an den Reserven abgelesen; außerhalb des Bands nur Richtung Band, nie darüber hinaus; `stopWhenFrozen` blockt `swap` und `init`, `add`/`remove` lesen kein Orakel. Reserve und Anteils-Minter müssen pool-eigen **und** aus derselben Tx sein (`sameTx`), GHOST-Eingänge keine Minter (KCC20-Erhaltung bleibt), Anteile: Erhaltung im Vertrag (`newTotal − total == holdersOut − holdersIn`). `add`/`remove` runden zugunsten der übrigen Einleger, `MIN_KAS` hält die Pool-UTXO ≥ 1 KAS. KCC20-Wrapper liest `prev_states` aus dem Tx-Kontext (`DECL.md:179`), nicht aus Argumenten.

**Factory.** `init` einmalig mit Deployer-Signatur, GHOST-Genesis mit Menge 0 als Factory-eigener Minter; `openVault`: frische Genesis, Vault-Template-Hash fest (damit `interestSpk`, `maxDebt`, Quoten für alle Vaults gleich), Startzustand (owner, 0, 0, 0), Wurzel-Minter mit gleichem KAS-Wert (N-11 hält), Minter-Zweig für die neue ID.

**Token (KCC20-Abwandlung).** `amount ≥ 0` für jeden neuen Zustand (V-01 hält); Besitz per Pubkey (Signatur), Script-Hash (Eingang mit dem P2SH) oder Covenant-ID (Covenant in der Tx). Token im Besitz einer Covenant-ID oder eines Script-Hashs, der in Vault-/Pool-/Orakel-Tx vorkommt, kann jeder dort ausgeben – bekannt (A10-P-6), nur Selbstschädigung.

**Zinskasse v4.** Zahlt am Index des eigenen Eingangs, nie an sich selbst, nur an P2PK/P2SH des Ziels aus der Haupt-UTXO (`!isTicket`). Im Live-Deployment nicht in Gebrauch.

**Tresor.** Zahlung nur ab `tx.time ≥ nextDue` (Zeit-Locktime, PMT), genau `amount` als P2PK an `recipient` am eigenen Index, Fortsetzung ≥ `Wert − amount − maxFee`, `due > nextDue` (Intervall 0 blockiert, von ghostctl abgefangen), `≤ 2200`, Payload fest gebunden (A12-1 hält), `cancel`/`topUp` nur mit Besitzersignatur.

**Signaturprüfungen je Eingang (Mempool-Regel 15).** Register 3·4 + 1 = 13, Vault 5, Token 2, Tresor 2, Factory 1, Pool 1, Orakel 0 (`tests/standard_sigops_tests.rs`, Simulator prüft mit).

**Griefing/Dust/Speichermasse.** Bekannt und dokumentiert: `read`-/`witness`-Spam (O-1, v4-entwurf 5) verzögert Updates; die Signaturen bleiben gültig. Winzige Ausgänge scheitern an der Speichermasse, nicht am Vertrag (A11-O-12). Ein Ticket mit beliebig kleinem Betrag schadet nur dem Vorschlagenden. Kein Weg gefunden, einen Covenant dauerhaft zu blockieren, außer über den Verlust der Unterzeichner-Schlüssel (A20a-7).

**Off-chain-Gegenstücke (nur gegengelesen).** `oracle_digest`, `rotate_digest`, `cancel_digest`, `SignerSet::hash` in `contracts.rs` entsprechen den Vertragsnachrichten Byte für Byte (`script_num8` wie `as byte[8]`); `oracle_next_state` = `update()`; `ops::oracle_update`, `propose`, `activate_rotation`, `close`, `sweep` legen Eingänge und Ausgänge so, wie die Verträge sie verlangen (Audit 14 bestätigt). `sim.rs` prüft Locktime und relative Sperre wie der Konsens.

---

## Anhang: Kern der Prüftests (entfernt)

Datei `protocol/tests/audit20a_tests.rs` (mit `mod v4common;`), Lauf `cargo test --release --offline --test audit20a_tests`: 7 Tests, 7 grün.

```rust
// attestPrice + Orakel mit FESTEN Signatur-Bytes `quorum` (sigs, idx)
fn attest_with_sigs(w: &World, kas, daa, seq, rate, index, rate_daa, quorum: &[ArtifactValue],
                    oracle_entry: &'static str, oracle_out: OSt, reg_out: RSt) -> Vec<Result<(), String>> {
    let mut reg_args = vec![i(kas), i(daa), i(seq), i(rate), i(index), i(rate_daa)];
    reg_args.extend(w.set.args());
    reg_args.extend(quorum.iter().cloned());
    let oargs = if oracle_entry == "update" { vec![i(kas), i(daa), i(rate)] } else { vec![] };
    execute(vec![w.reg_in(&w.reg, "attestPrice", reg_args), w.oracle_in(w.oracle, oracle_entry, oargs)],
            vec![w.reg_out(&reg_out, 0), w.oracle_out(oracle_out, 1)], daa as u64, vec![])
}

// A20a-1
let w = World::standard();                       // 7 Schlüssel, 4/4, Notfallsatz 5 Schlüssel
let (kas, daa, rate) = (w.oracle.kas_usd, w.oracle.daa + 600, w.oracle.rate);
let seq = w.oracle.seq + 1;
let quorum = w.set.quorum(price_digest(ORACLE_COV, kas, daa, seq, rate), &Satz::first(w.set.t));
let next = oracle_next(w.oracle, kas, daa, rate);
assert!(all_ok(&attest_with_sigs(&w, kas, daa, seq, rate, next.index, next.last_rate_daa, &quorum,
                                 "update", next, w.reg_after_price(daa))));   // ehrlich
let mut w2 = w.clone(); w2.oracle = next; w2.reg = w.reg_after_price(daa);
w2.reg.emerg = true; w2.reg.nonce = 6;           // Notfall-Ticket offen
let killed = RSt { nonce: 7, emerg: false, ..w2.reg.clone() };
assert!(all_ok(&attest_with_sigs(&w2, kas, daa, seq, rate, next.index, next.last_rate_daa, &quorum,
                                 "read", next, killed.clone())));             // Replay geht durch
let mut w4 = w2.clone(); w4.oracle.frozen = true;                              // Abhilfe: einfrieren
assert!(attest_with_sigs(&w4, kas, daa, seq, rate, next.index, next.last_rate_daa, &quorum,
                         "read", w4.oracle, killed)[0].is_err());

// A20a-3: sweep(1) mit Vault-Eingang 0 (coll 0,05 KAS, Zins 300 000 = 0,003 USD), Orakel read
// Ausgänge: [0] Orakel, [1] P2PK(Kasse) 1 sompi, [2] Rest an den Auslöser  → alle Eingänge ok
// Gegenprobe coll 0,5 KAS: Kasse coll − SWEEP_FEE − 1 → Vault lehnt ab

// A20a-5: Tresor pay (amount 10 KAS, maxFee 0,01 KAS, anchor 0, period 7 d, Payload leer)
// Ausgänge: [0] P2PK(Empfänger) 10 KAS, [1] Fortsetzung = Wert − 10 KAS − maxFee, [2] maxFee − 1 an den Auslöser → ok

// A20a-6: withdraw bei frozen: Schuld 0, Zins 1 USD, Preis 0,04 → 50 KAS müssen bleiben (49,99999999 abgelehnt);
//         Schuld > 0 bei frozen abgelehnt, nach dem Auftauen erlaubt. close bei frozen: Zins 1 USD → 25 KAS an die Kasse (24,99999999 abgelehnt).

// Orakel: Pause 1 Jahr bei maxRate → Index ≈ 1,2e9, Update ok; ×2 / ÷2 genau an der Grenze.
```
