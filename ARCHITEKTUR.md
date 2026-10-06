# kaspa-lending — Architektur (Arbeitsstand)

Ziel: Stablecoin nach GHO-Vorbild plus Lending-Pool, **direkt auf Kaspa L1**
(Covenants seit Toccata, SilverScript 1.0), mit Orakeln und KI-Agenten als Keeper.
Bezugsgröße zunächst **USD**. Gold kommt später als zusätzlicher Preisfeed dazu.

Name Stablecoin: **GHOST** (Entscheidung des Nutzers 28.09.2026; Anspielung auf GHOSTDAG und GHO).

Werkzeug: `vendor/silverscript` = kaspanet/silverscript **v1.0.0** (3ed9733, 2026-09-09),
rusty-kaspa rev a41a333 (vom SilverScript-Workspace gepinnt).

---

## 1. Befunde aus dem Code, die das Design bestimmen

Alle Befunde vom 28.09.2026, belegt an den genannten Stellen.

| # | Befund | Beleg | Folge |
|---|---|---|---|
| B1 | Zeitprüfungen gibt es nur als **Untergrenze**: `tx.time >= X`, `tx.daa >= X`, `this.ageDaa >= X` (CLTV/CSV). | `docs/TUTORIAL.md:517-567`, `:1119` | Ein Skript kann nicht prüfen, ob ein signierter Preis frisch ist. |
| B2 | `OpChainblockSeqCommit(block)` schlägt fehl, wenn der Block tiefer als `finality_depth` liegt. | rusty-kaspa `opcodes/mod.rs:1581`, `utxo_validation.rs:388` (Schwelle = `finality_depth`), `config/bps.rs:92` (`finality_depth`) = 432 000 Blöcke ≈ 12 h bei 10 BPS (Korrektur nach Audit: `params.rs:907` war ein Test-Fixture) | Die einzige Obergrenze fürs Alter liegt bei ≈ 12 h. Für Preise **zu grob**, taugt nur als Notbremse. |
| B3 | Eine UTXO lässt sich nicht lesen, ohne sie auszugeben. | KIP-10 (laut kaspanet/kips#46) | Das Orakel muss als UTXO **ausgegeben und unverändert neu erzeugt** werden. |
| B4 | Signaturen über beliebige Nachrichten sind prüfbar: `checkMsgSig(datasig, sha256(msg), pk)`. | `docs/TUTORIAL.md:990` | Orakel-Komitee m-von-n und **signierte Nutzer-Aufträge** sind möglich. |
| B5 | KCC20-Minter-Zweige dürfen sich **aufteilen**. Ein Zweig kann einer anderen Covenant-ID gehören. | `docs/kcc20-book`, Test `kcc20_minter_can_split_then_mint_then_burn` | Jeder Vault kann einen **eigenen Minter-Zweig** besitzen. Dann gibt es keinen globalen Engpass beim Prägen. |
| B6 | Schleifen haben eine feste Obergrenze, die beim Kompilieren feststeht (Unrolling). | `docs/TUTORIAL.md:569` | Sammeltransaktionen mit N Aufträgen gehen, N ist fest. Skriptgröße wächst mit N, **Grenze messen**. |
| B7 | `validateOutputState` prüft nur den Zustand, **nicht den KAS-Betrag** der Ausgabe. | `docs/DECL.md` | Jeder Vertrag muss Beträge selbst prüfen, sonst kann jemand die KAS abziehen. |
| B8 | `int` ist 64 Bit mit Vorzeichen. | `docs/TUTORIAL.md:235` | Bei der Zinsrechnung **Überlauf** beachten und testen. |

## 2. Bausteine

```
            Orakel-Komitee (3-von-5, deterministischer Median)
                         │  signiert (preis, daa, seq, zinssatz)
                         ▼
   ┌──────────── RiskOracle (Singleton-UTXO, nur die Spitze zählt) ────────────┐
   │ state: kasUsd, oracleDaa, seq, stableRate, stableIndex                     │
   │ update(): m-von-n Signaturen, seq+1, tx.daa >= oracleDaa, Index fortführen │
   │ read():   jeder darf ausgeben, wenn Zustand + Betrag identisch neu erzeugt │
   └────────────────────────────────────────────────────────────────────────────┘
          ▲ gelesen in derselben Tx                      ▲
          │                                              │
   StableVault (je Nutzer)                         LendingPool (Singleton)
   KAS-Sicherheit, debtShares,                     KAS-Einlagen gegen Anteils-Tokens,
   besitzt eigenen GHOST-Minter-Zweig               Kredit in KAS gegen GHOST-Sicherheit
          ▲                                              ▲
          └──── Keeper (KI-Agenten) bündeln signierte Nutzer-Aufträge ────┘
```

### 2.1 RiskOracle — gegen veraltete Preise

- **Nur die Spitze zählt.** Wegen B1 kann ein Vertrag das Alter eines Preises nicht prüfen. Das Orakel ist deshalb eine Singleton-Covenant-Kette. Wer einen Preis nutzt, muss die aktuelle Orakel-UTXO ausgeben und sie unverändert neu erzeugen. Einen alten Preis kann man so nicht verwenden, weil dessen UTXO schon ausgegeben ist.
- **Restrisiko:** Das Komitee hört auf zu aktualisieren. Dann gilt der letzte Preis. Als Notbremse kann das Orakel einen aktuellen Chain-Block referenzieren, den `OpChainblockSeqCommit` höchstens ≈ 12 h alt zulässt (B2). Zusätzlich erkennen die Keeper-Agenten, wenn das Orakel ausbleibt, und schlagen Alarm.
- **Stablecoin-Zins (GHO-Prinzip):** `stableRate` setzt das Komitee (Governance). Den Index `stableIndex` fortzuschreiben ist Aufgabe des **Vertrags**: `index_neu = index_alt · (1 + rate · Δdaa)`. Das Komitee kann die Schulden also nicht beliebig aufblähen, sondern nur den Satz ändern. Dass `tx.daa >= oracleDaa` gilt, verhindert Zeitstempel aus der Zukunft.
- **Engpass:** Alle Aktionen, die einen Preis brauchen, geben dieselbe Orakel-UTXO aus. Keeper bündeln deshalb viele Aktionen in **einer** Transaktion (siehe 2.4). Wie viele passen, müssen wir **messen**.

### 2.2 StableVault — Prägen nach GHO-Art

- Die KAS-Sicherheit ist der **Betrag der UTXO** selbst. Der Zustand speichert `owner`, `debtShares` und `nonce`.
- Die Schuld errechnet sich als `debtShares · stableIndex / SCALE`.
- Jeder Vault besitzt einen eigenen GHOST-Minter-Zweig (B5). Prägen und Verbrennen laufen darüber, ohne den Pool anzufassen.
- Aktionen:
  - `deposit`
  - `withdraw` (Quote prüfen)
  - `mint` (Quote prüfen)
  - `repay` (GHOST verbrennen)
  - `liquidate`: Jeder darf, wenn die Quote unter der Schwelle liegt. Er verbrennt GHOST und erhält KAS plus Bonus.
- Die Vault-Genesis läuft über einen Factory- bzw. Root-Minter. Nur Vaults mit geprüftem Template-Hash bekommen einen Minter-Zweig.

### 2.3 LendingPool — KAS verleihen

- **Zustand:** `totalShares`, `totalBorrowShares`, `borrowIndex`, `lastDaa`.
- **Einlagen:** Wer KAS einzahlt, bekommt Anteils-Tokens (KCC20). Deren Gegenwert steigt mit dem Index.
- **Kredit:** KAS leihen gegen GHOST als Sicherheit. Der Zins hängt von der **Auslastung** ab, wie bei Aave.
- **Engpass:** Die Pool-UTXO ist geteilter Zustand, deshalb laufen Aktionen nur gebündelt über Keeper.

### 2.4 Aufträge und Keeper

- Nutzer signieren einen **Auftrag** außerhalb der Chain (`checkMsgSig`, B4). Er enthält: Aktion, Betrag, Empfänger, Vault-ID bzw. Pool, Nonce und Ablauf-DAA.
- Keeper (KI-Agenten) bündeln die Aufträge zusammen mit Orakel und Pool in eine Transaktion und bekommen dafür eine Gebühr.
- Der Vertrag erzwingt das Ergebnis jedes Auftrags. Ein Keeper kann Aufträge nur ausführen, nichts stehlen oder umleiten.
- Pool-Einzahlungen brauchen die KAS des Nutzers als Input. Dafür gibt es Auftrags-UTXOs, die der Nutzer jederzeit zurückziehen kann. Details folgen später.

### 2.5 Rolle der KI

- **Nicht** als Preisquelle: nicht deterministisch und manipulierbar.
- **Keeper:** bündeln, liquidieren, Orakel-Updates einreichen.
- **Wächter:** Ausreißer zwischen Börsen erkennen, ausbleibendes Orakel melden, Pausenvorschlag an das Komitee.
- **MCP-Server:** Agenten der Nutzer verwalten eigene Vaults, zum Beispiel automatisch tilgen, bevor die Quote unter X % fällt.

## 3. Reihenfolge

1. **RiskOracle** mit Tests: Update m-von-n, Lesen mit Neuerzeugung, Angriffe wie Replay, falscher Signierer, Betrag abgezogen, seq zurück, Index-Überlauf.
2. **StableVault** mit KCC20-Minter-Zweig: mint, repay, withdraw, liquidate.
3. **Messung:** Skriptgröße, Masse und Gebühr je Aktion. Wie viele Vault-Aktionen passen in eine Sammeltransaktion?
4. **LendingPool** mit Aufträgen.
5. Danach: Orakel-Knoten, Keeper-Agent, MCP-Server, Oberfläche, Testnetz.

## Stand 28.09.2026

**Schritt 1 (RiskOracle) erledigt:** `contracts/risk_oracle.sil`, 21 Tests in `protocol/tests/oracle_tests.rs`.

- Aufruf: `cd protocol && cargo test --test oracle_tests`
- Die Tests laufen gegen die echte Skript-Engine aus rusty-kaspa a41a333.
- **Mutationstest:** Jede Schutzregel einzeln entfernt, der zugehörige Test wird rot. Die beiden Längenprüfungen sichern sich gegenseitig ab und sind deshalb nur gemeinsam geprüft.
- **Überlauf:** Die Engine bricht mit `NumberTooBig` ab und rechnet nicht mit einem umgebrochenen Wert weiter. Die erste Rechenreihenfolge (`index·rate`) wäre bei einem etwa 9-fachen Index übergelaufen, das Orakel hätte sich damit dauerhaft blockiert. Umgestellt auf `rate·Δ` zuerst.
- **Größen:** Redeem-Skript 820 B, Sigscript für read 828 B, für update 1 060 B.
- **Gebühr:** grob 0,002–0,005 KAS pro Transaktion. Das ist **nur eine Schätzung** über die Formel `100 sompi · max(compute, 2·bytes)`, die Masse ist noch nicht berechnet.
- **Pragma:** muss `^0.1.0` sein. Der Compiler v1.0.0 meldet die Sprachversion 0.1.0 (`compiler/mod.rs:55`), `^1.0.0` wird abgelehnt.

**Schritt 2 (StableVault) erledigt:** `contracts/stable_vault.sil` mit deposit, withdraw, close, mint, repay und liquidate.

- **Tests:** 34 Transaktionstests in `protocol/tests/vault_tests.rs` mit Vault, Orakel und GHOST (KCC20 aus SilverScript v1.0.0). Jeder Input der Transaktion wird ausgeführt.
- **Rechenfunktionen:** `mulDivDown/Up`, `debtOf`, `sharesFor` ohne Überlauf. `vault_math_tests.rs` prüft sie in der Engine gegen exakte u128-Rechnung (Grenzfälle plus 600 Zufallsfälle, jeweils auch „Wert + 1 wird abgelehnt").
- **Gültigkeitsgrenzen:** Preis ≤ 920 USD/KAS, Sicherheit ≤ 1e8 KAS je Vault, Index bis 9 200-fach. Darüber bricht die Engine mit `NumberTooBig` ab, statt falsch zu rechnen.
- **Sicherheitskern:** KCC20 prüft bei einem Minter-Zweig mit Covenant-Besitzer nur, ob der Vault in der Transaktion vorkommt. Ist der Zweig Leader, entfällt außerdem die Mengenerhaltung. Deshalb prüft jede Vault-Funktion die gesamte GHOST-Seite selbst. Der Test `deposit_mit_heimlichem_praegen_scheitert_am_vault` zeigt, dass KCC20 allein das Prägen zulassen würde.
- **Mutationstest:** Jede `require`-Zeile wurde einzeln durch `require(true)` ersetzt. Im ersten Durchlauf fielen 29 von 43 Mutanten durch keinen Test auf, darunter 4 kritische:
  1. `close` ohne Signatur,
  2. `repay` mit negativem Betrag (fremde Schuld hochtreiben),
  3. Fortsetzung der Covenant-ID bei `close` und bei Totalliquidation (Minter-Zweig danach frei nutzbar),
  4. Minter-Fortsetzung an einen Angreifer.

  Die Regeln standen im Code, nur die Tests fehlten. Für jede Regel gibt es jetzt einen Test, der **nur sie** trifft, und der Mutationstest wurde danach wiederholt.
- **Nicht einzeln getestet, weil anderweitig abgesichert** (Begründung, kein Test):
  - L116 `newShares >= 0`: unerreichbar, denn beim Teiltilgen gilt burned < debt.
  - L127 `kasUsd > 0`: Das Orakel erzwingt > 0 bei jedem Update.
  - L143, L144, L146: Array-Grenzen scheitern ohnehin in der Engine, zusätzliche `outStates` werden ignoriert. **Korrektur nach Test-Audit 5:** Die frühere Begründung für L145 (`nOut >= 1`) war falsch. Ohne L145 könnte jeder per `repay` mit leerer GHOST-Gruppe den Minter-Zweig eines fremden Vaults vernichten. L145 hat jetzt einen eigenen Test (`repay_ohne_ghost_ausgang_vernichtet_keinen_minter_zweig`).
  - L154–156, L158: Ist Input 0 kein eigener Minter, erzwingt KCC20 die Mengenerhaltung, und mint/repay/liquidate verlangen eine Differenz ungleich 0. Fremde Minter-Zweige brauchen ihren eigenen Vault in der Transaktion.
  - L173: GHOST-Ausgänge mit 0 sind harmlos.
  - L195: Ohne die Regel wäre withdraw nur ein Einzahlen durch den Besitzer.
  - L213: Mehr als 2 Ausgänge scheitern an der Schleifengrenze, 1 Ausgang an `amount > 0`.
- **Größen:** Vault-Skript 16 877 B, KCC20 1 786 B, Orakel 820 B.
  - Seit Toccata gelten 1 MB als Skriptgrenze (rusty-kaspa `txscript/src/lib.rs:78`). Die Standard-Massengrenze von 100 000 g gilt nur vor Toccata (`check_transaction_standard.rs:26,43`).
  - **Gebühr:** Diese erste Schätzung (rund 0,04 KAS) ist inzwischen gemessen, siehe Schritt 3 und `TESTNET_LOG.md`.
  - **Durchsatz:** Die transiente Masse liegt bei 4 g/Byte, das Blocklimit bei 1 000 000. Damit passen etwa 11 Vault-Transaktionen in einen Block. **Optimierung nötig**: Die Schleifen und die Rechnung sind derzeit voll ausgerollt.
- **Einschränkung Prototyp:** Pro Transaktion ist nur ein Vault möglich, weil die GHOST-Gruppe nur einen Leader hat. Bündeln durch Keeper folgt später.

**Schritt 3 (Deployment-Werkzeuge), Stand 28.09.2026:**

- **VaultFactory** (`contracts/vault_factory.sil`): Genesis, einmaliges `init` (legt die GHOST-Genesis an und das Vault-Template fest) und `openVault`.
- **Kernbefund:** Genesis-Ausgaben zählen nicht in `OpAuthOutputCount`/`OpCovOutputCount` (rusty-kaspa `covenants.rs`, `from_tx`). Die Factory prüft deshalb selbst per `freshGenesis()`, dass kein Input die neue ID trägt und dass keine zweite Ausgabe dieselbe ID hat. Sonst wäre eine Hintertür-UTXO neben dem Vault möglich.
- **Tests:**
  - 19 Angriffstests in `factory_tests.rs`, jeweils mit ehrlicher Gegenprobe auf demselben Deployment.
  - Mutationstest: Im ersten Lauf fielen 21 von 25 Regeln durch keinen Test auf. Kritisch waren darunter die Fortsetzung eines Angreifer-Covenants als „Vault", ein Minter-Zweig mit Vorab-Guthaben, der Wurzel-Minter an Fremde und ein zweites `init` durch den Deployer.
  - Jetzt hat jede dieser Regeln ihren eigenen Test.
  - Nur doppelt abgesichert und deshalb ohne eigenen Test:
    - L58: Die Schleife L64 greift, sobald eine Ausgabe ohne Covenant existiert.
    - L61: Der Compiler prüft die Schleifengrenze zur Laufzeit.
    - L88: Vor `init` ist die GHOST-ID 0.
    - L98 und L101–104: KCC20 verbietet Minter-Ausgänge bei einem Nicht-Minter-Leader, und ein fremder Vault-Zweig braucht seinen eigenen Vault in der Transaktion.
- **Bibliothek** (`protocol/src`):
  - `txb.rs`: Signaturen, Compute-Budget je Input (Probelauf und `ComputeBudget::checked_covering_script_units`), Commit der Speichermasse, Mindestgebühr `100 · max(compute, transient/2)`.
  - `ops.rs`: alle Aktionen.
  - `sim.rs`: lokale Kette.
- **`e2e_tests.rs`:** kompletter Lebenszyklus, jede Transaktion mit echten Budgets und Gebühren.
  - **Gebühren:** Vault-Aktionen kosten 0,04–0,05 KAS, ein Orakel-Update 0,0065 KAS.
  - **Speichermasse:** Jede neue Covenant-Ausgabe kostet etwa 4·10¹² / Wert Gramm. Bei 0,3 KAS pro Token-UTXO kam eine Vault-Transaktion auf 307 000 g, bei einem Blocklimit von 500 000. Deshalb gelten jetzt: Token 1 KAS, Minter-Zweig 3 KAS, Orakel und Factory 10 KAS. Das ergibt 21 000–57 000 g pro Vault-Transaktion.

## Version 2 (28.09.2026, nach dem Fable-Audit)

Die Befundberichte liegen in `audit/`, die Übersicht mit Status in `AUDIT.md`. Version 1 bleibt als Tag `v1-mainnet` erhalten und läuft mit `bin/ghostctl-v1`.

| Befund | Schwere | Behebung in Version 2 |
|---|---|---|
| V-01 GHOST-Token prüft keine negativen Beträge → aus 1 Einheit beliebig viele GHOST | kritisch | `contracts/ghost_token.sil`: KCC20 plus `amount >= 0` für jeden neuen Zustand |
| O-2 Orakelpreis ohne Grenzen | hoch | Preis 0,00001–900 USD, je Update höchstens ×2 bzw. ÷2 |
| V-02 Liquidation nur als Vollverbrennung, unter 100 % Deckung ein Verlustgeschäft | hoch | Teil-Liquidation. Ist die Sicherheit erschöpft, endet der Vault und die Restschuld wird ausgebucht. |
| O-1 / F4 Fremdes `read()` oder `deposit` bringen `ghostctl` aus dem Tritt | hoch / mittel | `store.rs` holt verschobene Covenant-UTXOs nach (gleiche Adresse und Covenant-ID); von Dritten veränderte Vaults werden als `stale` markiert |
| F1 / O-3 Transaktion gesendet, Zustand nicht gespeichert | mittel | Journal der offenen Transaktion; der nächste Aufruf klärt, ob sie angenommen wurde |
| F2 Deployment-Abbruch hinterlässt Covenants ohne Eintrag | mittel | Fortschrittsdatei; ein erneuter Aufruf setzt fort |
| F3 Parallele Aufrufe überschreiben sich | mittel | Dateisperre, der Orakel-Dauerbetrieb sperrt je Runde |
| V-03 Zinsen erhöhen die Schuld, werden aber nicht als GHOST geprägt → der letzte Schuldner kann nie voll tilgen | mittel | **Zins standardmäßig 0 %**. Die eigentliche Lösung (Zins-Prägung an eine Treasury) steht noch aus. |
| V-04 `deposit` ohne Signatur (Störung durch Konflikte) | mittel | nur mit Signatur des Besitzers |
| V-05 Mini-Tilgung ohne Anteilswirkung | niedrig | abgelehnt |
| O-6 Orakel sendet unter `--ja` jeden Sprung | mittel | Sprünge über 20 % erst nach 3 bestätigenden Runden und in Schritten von höchstens ×2/÷2 |
| F5–F13 | niedrig | JSON-Ausgabe, Eingabeprüfung, Blockgrenzen, verschenkte Reste ausgewiesen, Schlüsseldateien ab Anlage 0600, Netzprüfung, Zusammenführen von Token-UTXOs, Kurzaufruf baut bei geänderten Quellen neu |

**Bewusst offen, im Protokoll-Design begründet:**
- Das Orakel wird durch Ausgeben und Neuerzeugen gelesen, also kann jeder durch viele `read()` stören (O-1).
- Der Vertrag prüft das Alter eines Preises nicht (O-5).
- Signierte Orakel-Updates kann jeder einreichen, der sie hat (O-4).
- Minter-Zweige geschlossener Vaults binden je 3 KAS dauerhaft.

### Version 2.1 (nach dem Fix-Review, `audit/7-fix-review.md`)

Vertragsänderungen, jede per Mutante belegt:
- `stable_vault.sil`: `repay` nur mit Signatur des Besitzers (N-4). `ghostDelta` verlangt für die Fortsetzung des Minter-Zweigs denselben KAS-Wert wie am Eingang (N-11).
- `vault_factory.sil`: Die Fortsetzung des Wurzel-Minters behält ihren KAS-Wert (N-11).
- `risk_oracle.sil`: Die Orakel-DAA muss je Update um mindestens 600 steigen (I-3). Das ist keine Bremse gegen ein Komitee, das die DAA-Werte nachholt (Fix-Review 8, NEU-1).

Off-chain: Journal-Entscheidung über das eigene Wechselgeld (N-1), flock-Sperre (N-2), `in_mempool` mit Fehlerrückgabe (N-7), `status --json` bleibt rein (N-5), Stufenmodus des Feeds (N-6), Deploy-Reihenfolge und Startpreis-Prüfung (N-8, N-10, I-2). Seite und `ghostctl` weisen Aktionen, die nur der Besitzer darf, vorab ab.

Mit v2.1 sind die Minter-Zweige vor Entnahme geschützt. Die 3 KAS je Vault bleiben aber nach dem Schließen gebunden.

## Obergrenze je Vault (28.09.2026)

`stable_vault.sil` hat den Parameter `maxDebt`. Beim Prägen muss die neue Schuld `≤ maxDebt` sein. Fürs Mainnet sind es 50 GHOST (`MAINNET_MAX_DEBT`), die Zahl der Vaults ist nicht begrenzt (Entscheidung des Nutzers). Ein globaler Zähler hätte jede Präge-Tx über eine gemeinsame UTXO geführt. Die Grenze gilt nur fürs Prägen, Tilgen und Liquidieren bleiben frei. Seit Version 3 zählt sie nur die geprägten GHOST (`debt`), der Zins ist ein eigener Posten. Belegt durch `mint_bis_zur_obergrenze_je_vault` und eine gezielte Mutante.

## Offener Tauschpool KAS/GHOST (`contracts/ghost_pool.sil`, 28.09.2026)

- **Regel:** Konstantes Produkt nach Uniswap v2, 0,3 % Gebühr auf den Zufluss. Die Gebühr bleibt im Pool und gehört den Einlegern. Alle Produkte vergleicht `productGe` exakt über zwei 62-Bit-Hälften (`mul`).
- **Offen:** Jeder darf tauschen (`swap`), einlegen (`add`) und mit seinen Anteilen abziehen (`remove`). Einlegen bringt höchstens `min(S·Δx/x, S·Δy/y)` Anteile, Abziehen höchstens den Anteil `m/S` beider Reserven. Gerundet wird immer zugunsten der übrigen Einleger. Einen Besitzer gibt es nicht.
- **Anteils-Token:** Er ist ein eigener KCC20-Token mit derselben Vorlage wie GHOST, aber einer eigenen Covenant-ID. Sein Minter gehört dem Pool, und dessen Betrag ist die Gesamtzahl der Anteile S. Weil ein Minter-Leader die Mengenerhaltung von KCC20 aufhebt, prüft der Pool sie selbst: Änderung von S = Änderung der Inhaber-Anteile.
- **Henne-Ei bei der Anlage:** Der Pool muss die ID seines Anteils-Tokens kennen, der Token gehört aber dem Pool. Deshalb gibt es zwei Schritte wie bei der Factory: Genesis ohne Anteile, dann ein einmaliges `init` des Gründers. Es legt den Anteils-Minter als frische Genesis an (`freshGenesis`) und speichert die ID. Danach ändert sich der Pool-Zustand nie mehr.
- **Mindestliquidität:** Bei `init` entstehen S0 = Start-KAS (1 KAS) Anteile, die niemandem gehören. Sie bleiben mit dem GHOST zum Startkurs für immer im Pool, wie `MINIMUM_LIQUIDITY` bei Uniswap. Das verhindert, dass ein erster Einleger den Anteilspreis aufbläht.
- **Auffinden:** Pool, GHOST-Reserve und Anteils-Minter entstehen in jeder Pool-Tx neu. Den Pool findet `ghostctl` über seine feste Adresse. Reserve und Minter erkennt der Vertrag an derselben Herkunfts-Tx (`outpointTxId`). Ihre Beträge liest `pool::resync` aus den Signaturskripten dieser Tx (REST-API, auch im 8-Byte-Raster der ABI). Jeden Treffer bestätigt es am Node mit Covenant-Prüfung (Audit 9, P-1/P-2).
- **Gebühren gemessen (Simulator):** 0,054–0,059 KAS je Pool-Tx. Ein Tausch bindet zusätzlich 1 KAS im neuen Token des Empfängers.

## GHOST-Agent (`ghostctl agent`, 28.09.2026)

Er arbeitet in Runden, im Startskript alle 5 Minuten. Jede Runde nutzt eine frische Node-Verbindung, eine Dateisperre und ein Zeitlimit.
- **Orakel** (nur mit Komitee-Datei): wie `oracle-feed`, gemeinsame Funktion `oracle_round`.
- **Keeper:** `math::keeper_burn` entscheidet. Liquidiert wird nur, wenn der Vault nach dem Orakelpreis UND nach dem aktuellen Marktmedian unter 150 % liegt. Das ist die Wächter-Rolle: kein Liquidieren mit veraltetem Orakel. Verbrannt werden höchstens Sicherheit/(1+Bonus), also nie ein Verlustgeschäft, und nur eigene GHOST. Je Runde gibt es höchstens eine Liquidation, weil alle Aktionen die Orakel-UTXO teilen.
- **Auflösen (Version 3, Audit 11 A11-V-4):** Ohne Liquidation in der Runde löst der Agent höchstens einen Vault per `ops::sweep` auf, der Schuld 0 hat und dessen Zinsgebühr nach Orakel- UND Marktpreis die ganze Sicherheit erreicht (`math::sweepable`). Die Kasse bekommt Sicherheit − `SWEEP_FEE`. Die Netzgebühr der sweep-Tx (gemessen ≈ 0,055 KAS) trägt der Vault über `SWEEP_FEE` (0,1 KAS); den Keeper kostet das nichts (`e2e_tests::keeper_loest_zombie_vault_zugunsten_der_zinskasse_auf`).
- **Nicht dabei:** GHOST für Liquidationen im Pool nachkaufen, signierte Aufträge bündeln, MCP-Schnittstelle.

## Wallet (Seite „Wallet“)

Schlüsseldatei = Wallet. KAS und GHOST gehen an dieselbe Adresse (GHOST an den x-only-Schlüssel der Schnorr-Adresse). Der Node kennt keine Abfrage nach Covenant-ID, deshalb findet `ghostctl receive --ghost <Betrag>` eingegangene GHOST über den Betrag: Es rechnet die Token-Adresse aus und fragt sie beim Node ab.

## Version 3 (29.09.2026): Zins als eigener Posten, Zinskasse, Rücknahme, Zinsregel

Ziel: GHOST bleibt bei etwa 1 USD. Dafür gibt es drei Hebel, einen je Richtung und einen, der den Kurs zurückholt.

- **Schuld und Zins getrennt (`stable_vault.sil`).** Der Vault-Zustand ist `debt` (genau die geprägten, nicht getilgten GHOST), `interest` (aufgelaufener Zins in USD × 1e8) und `indexAt` (Orakelindex, bis zu dem abgerechnet ist). Version 2 buchte den Zins als GHOST-Schuld, für die nie GHOST entstanden. Der letzte Schuldner hätte nie tilgen können (Audit 1, V-03). In Version 3 gibt es nie mehr Schuld als GHOST.
- **Zinsrechnung.** Jede Aktion mit Orakel rechnet ab: `interest += ⌈(debt + interest) · growth(indexAt, index) / 1e9⌉` (Zinseszins seit Audit 11 A11-V-5, vorher nur `debt`), `growth = ⌈(index − indexAt) · 1e9 / indexAt⌉`, gedeckelt auf Index ×10 je Abrechnung. Weil der offene Zins mitwächst, hängt das Ergebnis nicht mehr davon ab, wie oft abgerechnet wird: Vorher senkte stündliches Abrechnen den Jahreszins bei 20 % um 9,7 %. Offener Zins wächst auch nach vollem Tilgen weiter (Designentscheidung). Überlauffrei bis `debt + interest ≤ 2e17`. So bleibt ein lange unberührter Vault ohne Überlauf bedienbar. Die Rechnung in Teilprodukten ist exakt bis `indexAt ≤ 9,2e12`. `vault_math_tests.rs` prüft sie gegen u128 in der Skript-Engine, auch mit Zufallswerten.
- **Quoten.** Für die Mindestquote (200 %, Prägen/Auszahlen), die Liquidationsschwelle (150 %) und die Rücknahme zählt `debt + Zins`.
- **Zinskasse.** Der Zins wird beim Schließen in KAS bezahlt: `⌈Zins · 1e8 / kasUsd⌉` sompi an die P2PK-Adresse `treasury` (der Deployer). Höchstens ist das die ganze Sicherheit. Der Kassen-Ausgang steht fest an Ausgang `activeInputIndex + 1`, direkt hinter dem eigenen Vault-Eingang; `close(oracleIdx, sig)` hat kein `treasuryIdx` mehr. Vorher konnten mehrere Vaults in einer Tx auf denselben Kassen-Ausgang zeigen, und die Kasse bekam nur die größte Einzelgebühr (Audit 11 A11-V-1). Unter 0,2 KAS wird der Zins erlassen, weil kleinere Ausgänge KIP-9 praktisch verbietet. Tilgen lässt den Zins stehen. Bei einer Liquidation mit Rest-Vault bleibt er ebenfalls stehen. Endet der Vault bei einer Liquidation, ist der Zins ausgebucht.
- **Rücknahme (`redeem`, jeder).** Wer GHOST an einen Vault zurückgibt, der mindestens bei 150 % steht, bekommt KAS im Wert von 1 USD je GHOST abzüglich 1 % (`REDEEM_FEE_BPS = 100`, vorher 0,5 %). Das 1 % bleibt als Ausgleich im Vault. Die Schuld sinkt um die zurückgegebenen GHOST, und im Vault bleiben mindestens 0,2 KAS. Zurückgegeben wird mindestens 1 GHOST (`MIN_REDEEM`) oder die ganze Schuld; vorher bewegten 2 Einheiten jeden fremden Vault (Audit 11 A11-V-3). Die Gebühr liegt über der Nachführschwelle des Orakels (0,5 %, Agent `--min-change 0.005`), damit sich eine Rücknahme gegen ein leicht nachlaufendes Orakel nicht lohnt (A11-V-2). Handelt GHOST unter etwa 0,99 USD, lohnt es sich, GHOST billig zu kaufen und zurückzugeben. Das ist die Untergrenze. Unter 150 % ist Rücknahme gesperrt, dort sind die Liquidatoren dran.
- **Auflösen (`sweep`, jeder).** Hat ein Vault Schuld 0 und erreicht die Zinsgebühr die ganze Sicherheit (bleibt nach einer Liquidation mit hohem Zins übrig), darf jeder ihn auflösen: alles bis auf `SWEEP_FEE` (0,1 KAS) an die Kasse, am selben festen Ausgang wie beim Schließen. Sonst läge er für immer, weil der Besitzer beim Schließen nichts mehr bekäme (A11-V-4). `ghostctl sweep --key --vault`, Seite: „Auflösen (Zinskasse)“. Die sweep-Tx kostet gemessen ≈ 0,055 KAS (der Vault-Eingang trägt das ganze Vertragsskript, 103 700 g transiente Masse); deshalb `SWEEP_FEE` = 0,1 KAS (die erste Fassung mit 0,01 KAS war nicht baubar ohne eigene KAS des Aufrufers).
- **Zinsregel (GHOST-Agent, mit Komitee-Datei; `protocol/src/rate.rs`).** Je Orakel-Runde eine Messung des GHOST-Kurses (Poolkurs × KAS-Marktmedian) aus dem **frisch** abgeglichenen Pool (`load_synced` in derselben Runde, nicht der Pool-Stand der Vorrunde aus der Datei; ist die Reserve unbestimmt, `pool_unresolved`, wird nicht gemessen – A11-O-8). Entschieden wird über den **Median der Messungen der letzten Stunde**: unter 0,97 USD +0,5 Prozentpunkte, über 1,03 USD −0,5 Punkte, dazwischen nichts (bis Audit 20: 0,995/1,005), und nur, wenn der Pool im Fenster gehandelt wurde (siehe Abschnitt „Audit 20“); Rahmen 2 % (Grundzins) bis 20 % p. a. (`math::rate_next`, rastet auf das 0,5-Raster ein). Geändert wird nur mit mindestens 6 Messungen (Abstand ≥ 4 min, damit Neustarts das Fenster nicht füllen) und nur, wenn der Pool mindestens `MIN_POOL_GHOST` = 10 GHOST hält; sonst steht der Zins, und das Protokoll sagt warum. Höchstens eine Änderung je Stunde: Messungen und Zeitpunkt der letzten Änderung liegen in `deployments/<netz>-zins.json` unter eigener Sperre (flock), die Änderung wird vor dem Senden vorgemerkt und nur zurückgenommen, wenn sicher nichts gesendet wurde. Das gilt über Neustarts und für ein paralleles `oracle-feed` (A11-O-9; vorher lag der Takt nur im Speicher). Der Satz geht mit dem nächsten Orakel-Update in den Zinsindex, ist der Preis unverändert, gibt es ein eigenes Update. Steigender Zins macht Schulden teurer: Vault-Besitzer kaufen GHOST zurück und tilgen, das hebt den Kurs. Sinkender Zins macht Prägen billiger und drückt den Kurs.
  - **Warum Median und 10 GHOST (A11-O-1):** Eine Einzelmessung ließ sich mit einem Kauf von 2,55 KAS (Kosten 0,006 USD) über die Totzone schieben. Den Median der Stunde bewegt ein Angreifer nur, wenn er den Kurs über mehr als die Hälfte der Stunde hält; dann zahlt er Gebühren je Hin und Rück und trägt das Risiko, dass andere zurücktauschen, und jede Stunde bringt höchstens 0,5 Punkte (von 0 auf 20 % also mindestens 40 Stunden). Die Mindestliquidität ist **keine** Kostenhürde: Auch bei 10 GHOST kostet eine Verschiebung um 0,5 % nur Cent-Beträge. Sie schließt Pools aus, deren Kurs nichts aussagt – den Startpool des Umzugs mit 0,25 GHOST, in dem ohne Handel nur die KAS-Bewegung ankommt. 10 GHOST sind ein Fünftel eines vollen Mainnet-Vaults. Unbehoben bleibt: In einem wenig gehandelten Pool über 10 GHOST folgt `x/y · KAS-Markt` weiter der KAS-Bewegung, solange niemand arbitriert; ein Tag fallender KAS kann den Zins über den Median hinweg Schritt für Schritt heben.
- **Kursband im Pool** (seit dem Pool v2): Tausche sind nur zulässig, solange GHOST im Pool bei 1 USD ± 3 % liegt oder sich darauf zubewegt.
- **Obergrenze:** 50 GHOST je Vault für `debt`, keine Gesamtobergrenze.
- **Off-chain.** `math.rs` spiegelt den Vertrag (`accrued`, `healthy`, `max_mint`, `redemption`, `liquidation`, `rate_next`). `chain.rs` rekonstruiert fremde Vault-Änderungen: Kandidaten sind die neue Schuld ± verbrannte oder geprägte Mengen, der Zins ist bis zum Orakelindex der Tx abgerechnet. Jeder Kandidat wird am Skript-Hash bestätigt. `ghostctl redeem` führt die Rücknahme aus, `close` zeigt die Zinsgebühr.
- **Umzug:** Version 3 ist ein neues Deployment mit neuem GHOST. Version 2 wird mit `GHOST-Umzug-v3.command` abgebaut (`bin/ghostctl-v2`, `deployments/mainnet-v2.json`). Das Skript bricht ab, solange ein Agent oder `oracle-feed` läuft (A11-O-3), benennt Journal, Deploy-Fortschritt und Sperre mit um und schreibt das Ziel des Journals um (A11-O-4), zählt in Schritt 5 nur eigene Vaults und prägt an einem eigenen ohne Schuld nach (A11-O-5), prüft vor `pool-open` GHOST und KAS, sperrt sich gegen einen Doppelstart (A11-O-6), und nur `DRY=1` ist ein Probelauf (A11-O-14). Beim Durchspielen mit Attrappen fiel ein älterer Fehler auf: Schritt 2 brach mit einem Python-Syntaxfehler ab (`v[\"index\"]` im f-String) und übersprang dadurch still jeden Vault. Behoben.

### Version 3 nach Audit 11: was bleibt

- **Rücknahme zum alten Orakelpreis (A11-O-2, Rest-Risiko, mittel).** `redeem` zahlt zum Orakelpreis. Der Agent hält Sprünge über 20 % absichtlich drei Runden zurück (bei 5 Minuten Takt etwa 10–15 Minuten, Schutz gegen Ausreißer, A10-A-7), und läuft kein Agent, bleibt das Orakel beliebig alt (O-5: keine Frischeprüfung im Vertrag). Steigt KAS in dieser Zeit, bekommt jeder GHOST-Inhaber mehr KAS, als der Markt hergibt: bei +25 % etwa +24 % je GHOST (Simulator, Audit 11 Teil B). Das zahlen **gesunde** Vault-Besitzer über 150 %, ohne etwas falsch gemacht zu haben. Die Gebühr von 1 % schützt nur gegen kleine Nachläufe unter 1 %; die Feed-Schwelle 0,5 % sorgt dafür, dass im Normalbetrieb kein größerer entsteht. Aus dem Nichts ausbeutbar ist es nicht (das Kursband des Pools hängt am selben Orakel, Kauf im Pool plus Rücknahme verliert), es trifft aber alle, die GHOST schon halten. Mögliche Abhilfen, alle nicht umgesetzt: bei ausstehendem Aufwärtssprung sofort einen gedeckelten Zwischenschritt senden (z. B. +20 %), im Vertrag eine Frischeprüfung oder eine dynamische Rücknahmegebühr (Liquity-`baseRate`). Die Seite nennt das Risiko in „So funktioniert’s“ und in den FAQ.
- **Mutationslücken, die bleiben (Mutationstest v3 vom 29.09.2026; Zeilen damals → heute nach bea9b56).** Alle von Audit 11 Teil A bestätigt als doppelt gesichert oder unerreichbar:
  - L152 → L181 `newDebt >= 0` und L285 → L324 `burned <= debt`, L296 → L335 `amount <= debt`: sichern sich gegenseitig. Jede Minderung ist auf die Schuld begrenzt, `mint` addiert nur positive Beträge, die Factory startet bei 0. Fällt eine Regel weg, greift die andere (Überzahlung getestet: `repay_vollstaendig_und_ueberzahlung`).
  - L154 → L183 `newInterest >= 0`: unerreichbar. `accrual ≥ 0` (nur bei positiver Basis, `to > from > 0`, `growth > 0`), die Engine bricht bei Überlauf ab statt umzuschlagen, die Factory startet mit 0.
  - L171 → L200 `OpCovOutputCount(ghost) == 0`: Konsens. Ein Ausgang zählt nur für die ID seines autorisierenden Eingangs; ohne GHOST-Eingang (L170) wäre er eine Genesis mit neuer ID.
  - L181 → L210 `nIn >= 1`, L183 → L212 `nOut >= 1`: Ohne Eingang bzw. Ausgang bricht `OpCovInputIdx`/`OpCovOutputIdx(…, 0)` ab.
  - L182 → L211 `nIn <= 3` und L184 → L214 `outStates.length == nOut`: Zweitsicherung durch den KCC20-Leader (`in_count ≤ from`, `out_count ≤ to`) und den Schleifenwächter des Compilers. Seit bea9b56 steht zusätzlich `nOut <= MAX_GHOST_OUTS` (L213) ausdrücklich im Vault (A11-V-7), damit die Grenze nicht an einem Compiler-Detail hängt.
  - L268 → L307 `OpCovOutputCount(ghost) == 2` (mint): nOut = 1 ergibt kein positives Delta; nOut = 3 lehnen KCC20 und jetzt auch L213 ab.
  - L295 → L334 `amount > 0` und L304 → L344 `paid > 0` (redeem): sichern sich gegenseitig gegen negative Beträge (`ruecknahme_mit_negativem_betrag_praegt_nicht`); `paid > 0` allein weggelassen erlaubte nur eine Rücknahme ohne Auszahlung, die den Rücknehmer selbst schädigt.
  - L165 → L194 `kasUsd > 0` ist **keine** Lücke mehr: `a11_preis_null_nur_durch_l165_gesperrt` tötet die Mutante.
- **Zinsindex rundet ab (A11-O-11).** „Gerundet wird zugunsten der Zinskasse“ gilt nur für die Abrechnung im Vault (`accrual`, aufgerundet). Das Orakel schreibt den Index je Update **abgerundet** fort (`risk_oracle.sil`, zugunsten der Schuldner). Bei 0,5 % p. a. fehlen bei 600 DAA Abstand 5,4 %, bei 3 000 DAA 1,2 %, bei 36 000 DAA (1 h) 0,1 % des Zinses. Der Agent aktualisiert meist nur bei 0,5 % Preisänderung oder nach 6 Stunden, der Verlust liegt also meist unter 0,1 %. Die Seite sagt das jetzt so.
- **Rücknahme und Liquidation adressieren den Vault über die Nummer (A11-O-15).** `--vault` ist der Index in der Zustandsdatei, nicht die Covenant-ID, und `redeem` hat kein `--min-kas`. Endet zwischen Probelauf und Senden ein Vault mit kleinerer Nummer, trifft die Aktion einen anderen Vault. Wirtschaftlich neutral (gleicher Kurs, gleiche Regeln), aber nicht der gewählte. Offen; Abhilfe wäre `--vault` wahlweise als Covenant-ID und `--min-kas` für `redeem`.
- **Winzige Ausgänge (A11-O-12).** Ein Ausgang unter etwa 0,02 KAS ist zu schwer (Speichermasse ≈ 10¹² / Betrag in sompi Gramm, Blockgrenze 500 000 g); Schließen oder Abheben mit so einem Rest lässt sich nicht bauen. Kein Verlust, Einzahlen hilft. Die Seite warnt beim Schließen und Abheben unter 0,025 KAS.

## Audit 20 (06.10.2026): Zinsregel „Totzone + nur bei Handel“, Notfallsatz, Agent

**Zinsregel neu (Entscheidung des Betreibers, Befund A20e-6).** Die Messung `Poolkurs × KAS-Markt` ändert sich ohne Tausch nur mit KAS. Mit der alten Totzone ±0,5 % und dem Grundzins als Untergrenze trieb reine KAS-Drift den Zins einseitig nach oben (Simulation in Audit 20e: Ø ≈ 11 % nach 30 Tagen, 45 % der Läufe bei 20 %). Jetzt gilt (`math::rate_next`, `rate::RateLog::decide`):

| | Wert | Code |
|---|---|---|
| Messung | je Orakel-Runde GHOST in USD = KAS-Reserve ÷ GHOST-Reserve × KAS-Marktmedian, frühestens alle 4 min, dazu das Tauschverhältnis KAS-Reserve ÷ GHOST-Reserve | `rate::measure`, `rate::pool_ratio`, `Sample.pool_ratio` |
| Pool messbar | mindestens 10 GHOST Reserve | `MIN_POOL_GHOST` |
| Entscheidung | Median der Messungen der letzten Stunde, mindestens 6 Messungen über mindestens 45 min | `WINDOW_SECS`, `MIN_SAMPLES`, `MIN_SPAN_SECS` |
| Totzone | 0,97 bis 1,03 USD (einschließlich) = Kursband des Pools ±3 % | `math::RATE_ZONE` = `pool::POOL_BAND_BPS` |
| unter 0,97 USD | +0,5 Prozentpunkte | `RATE_STEP_PCT` |
| über 1,03 USD | −0,5 Prozentpunkte, nie unter den Grundzins | `RATE_MIN_PCT` |
| Handel | Tauschverhältnis hat sich im Messfenster zwischen aufeinanderfolgenden Messungen zusammen um mindestens 2 % bewegt (Σ \|ln(rᵢ/rᵢ₋₁)\|) – sonst bleibt der Zins | `MIN_TRADE_MOVE` |
| Grundzins | 2 % p. a. Untergrenze; liegt der Satz darunter, +0,5 Punkte je Stunde bis 2 %, unabhängig von Kurs und Handel (wie bisher) | `rate_floor_step` |
| Rahmen, Takt | 0–20 % p. a., höchstens eine Änderung je Stunde (Zinsdatei, Vertrag `rateGapDaa`) | unverändert |

*Warum „Handel“ so:* Das Tauschverhältnis ändert sich nur durch Tausch; Einlegen und Abziehen halten es (bis auf Rundung, ein schiefes Einlegen verschenkt den Überschuss). 2 % Verhältnis entsprechen bei einem Produktpool etwa 1 % der Reserve als Umsatz, im kleinsten messbaren Pool (10 GHOST) ≈ 0,1 GHOST. Ein Tausch hin und zurück zwischen zwei Messungen ist unsichtbar und zählt nicht. Außerhalb des Bands lässt der Pool nur Tausche Richtung Band zu; wer Umsatz vortäuschen will, schiebt den Kurs damit selbst Richtung Totzone. Weil die Totzone gleich dem Band ist, ändert die Regel den Zins praktisch nur noch, wenn der Pool trotz Handel am Rand oder außerhalb des Bands steht (einseitiger Druck), nicht wegen KAS-Drift. Messungen ohne Tauschverhältnis (ältere Zinsdatei) zählen nicht als Handel. Die Messung läuft jetzt auch während der Grundzins-Anhebung (A20b-6), eine Zukunftszeit in der Zinsdatei wird auch im Grundzins-Pfad gekappt (A20b-2), und vorgemerkt wird eine Änderung erst, wenn die Pause des Vertrags um ist (A20b-6).

**Notfallsatz (A20e-2/A20a-7, Entscheidung: ja).** `GHOST-Notfallsatz.command` erzeugt einen Notfall-Schlüssel (`committee-keygen --count 1`) auf einem USB-Stick und kündigt im Register „gleicher Hauptsatz + neuer Notfallsatz (1 von 1)“ an (`signers propose --same-set --fallback-keys …`, signiert von `keys/mainnet-signer.json`); nach 14 Tagen `signers activate`. Der Abgleich übernimmt eine solche Aktivierung jetzt auch, wenn der Satz gleich bleibt und ein Dritter aktiviert (vorher galt sie dann als abgesagt). Rechner ohne die Ankündigung (Server) setzen weiter Preise; sie kennen den Notfallsatz nur als Hash. Simulator: `v4_ops_tests::a20_notfallsatz_nachtragen_und_notfallweg`.

**Notfallweg nur bei eingefrorenem Orakel (A20a-1).** `attestPrice` prüft nur den Orakel-Ausgang; die öffentliche Signatur des letzten Updates mit einem Orakel-`read` wiederholt, gilt als Lebenszeichen und entwertet ein Notfall-Ticket. Bei eingefrorenem Orakel scheitert das (Register verlangt `frozen = false`). `ops::propose(emergency)` und `ghostctl signers propose --emergency` verlangen deshalb ein eingefrorenes Orakel und sagen, ob `oracle-freeze` schon geht. Im Vertrag bleibt die Lücke bis v5 (`newSeq == alt.seq + 1`).

**Agent (A20b-1, A20e-3, A20e-7, A20e-9).** Die Orakel-Runde gleicht nur Orakel, Register, Factory, Wurzel und Pool ab und baut das Update auf genau diesem Stand (`SyncScope::core`); Vaults und Token gleicht danach der Keeper ab – viele Vaults oder Token verzögern das Orakel nicht mehr. Vaults und Token werden gebündelt abgefragt (`store::snapshot`, eine Node-Abfrage je 100 Adressen statt ein bis zwei je UTXO). Der Keeper prüft je Runde die eigenen Token und 50 fremde reihum; ein Fehler beim Token-Abgleich wird zum Hinweis, die Runde läuft weiter. Die Token-Liste hat Obergrenzen (`ops::MAX_TOKENS` 1 000 für fremde Empfänger, je Empfänger 16, je eigenem Besitzer 64); darüber entsteht der Token auf der Kette trotzdem, geführt wird er erst nach `receive` (GHOST-Suche). Der Keeper zieht die Netzgebühr (0,1 KAS zum Marktpreis, `KEEPER_FEE_SOMPI`) vom Erlös ab, bevor er die 2 % Mindestgewinn prüft. Ist das Orakel eingefroren oder älter als der Herzschlag, sendet der Agent bei einem großen Sprung sofort einen Zwischenschritt um höchstens 20 % statt auf drei ruhige Runden zu warten (`feed_decision`).

## Dauerauftrag mit Tresor (29.09.2026)

Wunsch des Nutzers: Daueraufträge sollen auch zahlen, wenn der Rechner des Absenders aus ist. Lösung: Das Geld liegt vorab in einem Covenant, und zum Termin darf **jeder** die Zahlung auslösen – aber nur so, wie der Vertrag sie vorschreibt.

- **Vertrag (`contracts/standing_order.sil`).** Parameter `owner`, `recipient` (x-only), `amount`, `anchorDay` (1–31 monatlich, 0 = festes Intervall `periodMs`), `maxFee`, `payloadHash` (sha256 des Payloads, den jede Zahlung tragen muss); Zustand `nextDue` (Unix-ms, UTC) und `left` (−1 unbegrenzt). `pay()` verlangt `tx.time ≥ nextDue`, genau `amount` als P2PK an `recipient` am Ausgang mit dem Index des Tresor-Eingangs (zwei Tresore können sich so keinen Ausgang teilen), genau eine Fortsetzung mit ≥ Wert − amount − maxFee und dem nächsten Termin. `topUp`/`cancel` nur mit Signatur des Absenders. Engine-Tests: `protocol/tests/standing_order_tests.rs` (21 Tests, Mutationstest ohne Lücke: 15 von 15 Regeln einschließlich der Nachrichtenbindung), Terminrechnung gegen chrono (`standing.rs`).
- **Nachricht fest gebunden (30.09.2026, Audit 12, A12-1 im Vertrag).** `pay()` verlangt `sha256(Payload der Tx) == payloadHash`. Beim Anlegen bindet `tresor::bind_message` genau den Payload, den jede Zahlung trägt: öffentlich den Klartext, sonst die einmal an den Empfänger verschlüsselte Fassung (`sealed`), ohne Nachricht den leeren Payload. Wer auslöst, kann die Nachricht weder weglassen noch ersetzen, und auch der Absender kann sie nachträglich nicht ändern – nur kündigen und einen neuen Tresor anlegen. `tresor::pay` lehnt jeden anderen Payload schon vor dem Bauen ab. Der Eingang (`message.rs`) prüft den Payload gegen den Hash aus dem Skript, auch bei hier nicht übernommenen Tresoren: „vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)“. „Passt nicht zur im Vertrag gebundenen Nachricht“ kann nur noch aus falschen Daten der REST-API stammen. Der Besitzer bleibt ein bloßer Parameter (anlegen kann jeder, ohne dessen Signatur); Absender-Echtheit gibt es weiterhin nicht.
- **Zeit-Locktime (gemessen im Code von rusty-kaspa a41a333).** `tx.time` übersetzt der Compiler in `OP_CHECKLOCKTIMEVERIFY` mit Prüfung ≥ `LOCK_TIME_THRESHOLD` (`vendor/silverscript/silverscript-lang/src/compiler/compile/statement.rs:390–402`). CLTV verlangt Zeit-Locktime auf beiden Seiten, `nextDue ≤ lock_time` und einen nicht finalen eigenen Eingang, `sequence ≠ u64::MAX` (`crypto/txscript/src/opcodes/mod.rs:1014–1064`, Sequence-Prüfung Z. 1057). Der Konsens nimmt eine Tx mit `lock_time ≥ 5·10^11` erst, wenn `lock_time < Past Median Time` (`consensus/src/processes/transaction_validator/tx_validation_in_header_context.rs:56–93`; Mempool `virtual_processor/processor.rs:1226–1230`, Block `body_processor/body_validation_in_context.rs:31–39`). Die PMT ist der Mittelwert der 11 mittleren von 27 Zeitstempeln im Abstand von 10 s (`past_median_time.rs:18–38`, `constants.rs:23–30`), in ms, und hinkt der Uhr etwa 2¼ min nach. `txb.rs` setzt `sequence = 0` für jeden Eingang (Z. 208) und `lock_time` aus dem Entwurf (Z. 212); `tresor::pay` nimmt `lock_time = nextDue` – das früheste gültige.
- **Simulator (`sim.rs`).** Neu `now_ms` (simulierte PMT, Start 2027-01-01) mit `advance_time`/`set_time`; jede angenommene Tx rückt sie um 1 s vor (10 Blöcke bei 10 BPS). Zeit-Locktimes prüft er wie der Konsens (strikt kleiner, Ausnahme alle Eingänge final); DAA-Locktimes unverändert.
- **Transaktionen (`protocol/src/tresor.rs`).** `open` (Genesis wie beim Orakel: Covenant-ID aus dem ersten Funding-Outpoint), `pay` (Tresor = Eingang 0, Empfänger = Ausgang 0, Fortsetzung = Ausgang 1; Gebühr aus dem Tresor, zweimal gebaut, damit genau die Mindestgebühr + 5 % abgeht; reicht der Tresor nicht, zahlt der Auslöser mit eigenen Eingängen und Wechselgeld), `topup`, `cancel` (alles abzüglich Gebühr an den Absender).
- **Mindestwerte (gemessen, `tests/tresor_e2e_tests.rs`).** Der Simulator nimmt Zahlungen ab ≈ 0,0205 KAS (bei 100 KAS Fortsetzung) und Fortsetzungen ab ≈ 0,076 KAS (bei 1 KAS Betrag); darunter sprengt die Speichermasse das Blocklimit. ghostctl erzwingt 1 KAS je Zahlung und 1 KAS Fortsetzung (Speichermasse dann 29 802 g, 6 % des Blocklimits). Gebühr einer Zahlung 0,0025 KAS, mit 400 Byte Payload 0,0033 KAS (transiente Masse bestimmt sie); Standard-Höchstgebühr 0,01 KAS.
- **Nachführen ohne REST.** Der Zustand nach einer Zahlung ist vollständig bestimmt. `tresor::candidates` rechnet vom bekannten Zustand alle Nachfolger, deren Vorgänger bis zur PMT (+3 min) fällig war; für jeden fragt ghostctl den Node nach der Adresse und nimmt die UTXO mit dieser Covenant-ID und genau diesem Skript. Keine UTXO mehr = gekündigt (als „nicht auffindbar“ markiert, beim nächsten Abgleich erneut geprüft). Skripte ohne Kompilieren über die Template-Form (`TresorShape`, Test gegen den Compiler, auch mit `payloadHash`).
- **Tresor-Code statt Import über die Covenant-ID.** Eine P2SH-Adresse verrät das Skript erst beim Ausgeben; bei einem frischen Tresor stünden die Parameter also nirgends auf der Kette, und die REST-API wäre eine zweite, oft gestörte Quelle. Der Code (`ghost-tresor:2:` + base64url-JSON mit Netz, Covenant-ID, Parametern samt `payloadHash`, Zustand, Outpoint, Beschreibung, verschlüsselter Fassung) reicht allein, und der Node bestätigt ihn (Skript-Hash + Covenant-ID). Beim Import muss der Hash zur Beschreibung (öffentlich) bzw. zur verschlüsselten Fassung passen, sonst wird der Code abgelehnt. Codes `ghost-tresor:1:` gehören zum alten Vertrag ohne gebundene Nachricht und werden abgelehnt, ebenso Tresor-Dateien mit solchen Tresoren (im Mainnet gab es noch keine). Ein veralteter Code schadet nicht: die aktuelle UTXO findet das Nachführen.
- **Zustand.** `deployments/<netz>-tresore.json` (eigene Sperre und eigenes Journal über `store::write_pending`/`resolve_pending`), ID = erste 8 Hex-Zeichen der Covenant-ID, auf allen Rechnern gleich. Keine Schlüssel, nur der Pfad der Schlüsseldatei des Absenders.
- **Auslösen.** `ghostctl tresor pay` (einzeln oder alle fälligen), der Agent in jeder Runde (`tresor_agent_step`, eigene Verbindung, Sperre, 700 s Zeitlimit, eigener Fehlerpfad) und der lokale Server jede Minute. Offline-Vorprüfung mit der Uhr des Rechners (Termin + 3 min); nach einem Fehlschlag 15 min Pause. Doppelt auslösen ist harmlos: die zweite Tx findet die UTXO nicht mehr.
- **Seite.** Karte „Daueraufträge“: Wahl „vom Rechner | mit Tresor“, Startguthaben mit Vorschlag, Probelauf mit Gebühr vor dem Anlegen, Tresor-Code zum Kopieren, Liste mit Guthaben, nächstem Termin, verbleibenden Zahlungen, Auffüllen/Kündigen; für Empfänger „Tresor-Code einfügen“ und „Fällige Zahlung abholen“. Server-Aktionen `tresor-open|pay|topup|cancel|import|sync` mit fester Parameterliste (`server/actions.ts`).
- **Offen:** Durchlauf im Testnetz (echte PMT, echte Mempool-Regeln); Gebühren bei vollem Mempool (Höchstgebühr fest im Vertrag – wird sie zu klein, muss der Auslöser mit `--key` zahlen, dann bleibt die Fortsetzung bei Wert − Betrag).

## 4. Offen / nicht verifiziert

- Gebühren sind gemessen (Schritt 3, `TESTNET_LOG.md`): Vault-Aktionen kosten 0,036–0,051 KAS. Sie enthalten 5 % Aufschlag auf die Mindestgebühr (`FEE_MARGIN_PERMILLE`, `txb.rs`).
- Ob Kaspa `SigHashAnyOneCanPay` für Auftrags-UTXOs so unterstützt, wie wir es brauchen: nicht geprüft.
- Konkurrenz: Kaskad (Igra, L2), 1kUSD (Forschung, Ziel Toccata). Stand vom 28.09.2026.
