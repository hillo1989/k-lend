# GHOST v4: Verträge mit austauschbaren Unterzeichnern

Stand 04.10.2026, Branch `v4` (Basis 9dc741d). Gebaut sind die Verträge mit Engine-Tests und seit b0f6d86 die **Anbindung an ghostctl und die Seite** (Abschnitt 9). Es gibt noch kein Deployment. Die v3-Verträge sind unverändert, v3 läuft weiter.

Grundlage ist der Orakel-Entwurf vom 04.10.2026, Variante C: ein kleines Preis-Orakel, das keine Signaturen prüft, und eine eigene Register-Covenant mit dem Quorum.
**[B]** heißt belegt (Test oder Messung), **[V]** heißt vermutet oder nicht im Netz geprüft.

---

## 1. Kurzfassung

| Datei | Zweck | Redeem-Skript | Zustand |
|---|---|---|---|
| `contracts/price_oracle_v4.sil` | Preis, Zinsindex, `frozen`, `lastRateDaa`; `read`, `update`, `freeze` | **762 B** (v3: 852 B) | 56 B (v3: 45 B) |
| `contracts/signer_register_v4.sil` | Satz-Hash, Notfallsatz, Zinsziel; Preis-Quorum, Austausch über Ticket | 7 914 B | 209 B |
| `contracts/zinskasse_v4.sil` | sammelt Zins, leitet an das Zinsziel des Registers weiter | 851 B | – |
| `contracts/stable_vault_v4.sil` | Vault v3 + Frischeprüfung + Zinsziel als scriptPubKey | 25 199 B | wie v3 |
| `contracts/ghost_pool_v4.sil` | Pool v3 mit Orakel-v4-Layout, `stopWhenFrozen` | ≈ 23,3 KB (wie v3) | wie v3 |
| `contracts/vault_factory.sil` | **unverändert** nutzbar (bindet nur das Vault-Layout, das gleich bleibt) | – | – |

- Das Orakel-Skript fährt in jeder Vault- und Pool-Tx mit. Es ist **kleiner als in v3** [B], obwohl der Zustand zwei Felder mehr hat. Die Signaturprüfung steckt jetzt im Register, und das Register fährt nur bei Updates und beim Austausch mit.
- Eine Update-Tx kostet etwa **0,020 KAS** (v3: 0,0065 KAS) [B, Masse mit `txb::build` gegen Mainnet-Parameter]. Bei stündlichem Heartbeat sind das etwa 0,49 KAS am Tag. Der Hauptanteil ist das 7,9 KB große Register-Skript (Abschnitt 6).
- **Alleinbetrieb** geht: 1-von-1 oder 3-von-5 mit eigenen Schlüsseln. Später kommen unabhängige Unterzeichner über `propose → Wartezeit → activate` hinzu, ohne neues Deployment [B, Test `alleinbetrieb_eins_von_eins_mit_kurzen_fristen`].
- **Testzahlen:** 223 neue Engine-Tests in 6 Dateien. Davon sind 151 v3-Tests, die unverändert gegen Vault und Pool v4 laufen, und 72 neue v4-Tests. Alle 614 Tests des Projekts sind grün [B].
- **Mutationstest:** Orakel 15/15 rot. Zinskasse 7/7 rot. Register 31 von 38 rot, die 7 übrigen sind doppelt gesichert. Vault v4 52 von 65 rot, die 13 übrigen sind genau die in Audit 11 bestätigten v3-Lücken. Pool v4 39 von 67 rot, die übrigen 28 Lücken liegen in unverändertem v3-Code. Jede **neue** v4-Regel in Vault und Pool wird rot (Abschnitt 8).

---

## 2. Aufbau

```
 Register (Covenant REG)                 Orakel (Covenant ORA, regCovId als Konstante)
 ├─ Haupt-UTXO (Singleton)               ├─ read()    jeder; Zustand unverändert
 │   attestPrice ──────────────────────► ├─ update()  nur mit genau 1 Register-Eingang
 │   propose  → legt Ticket an           └─ freeze()  jeder, ab oracleDaa + freezeAfterDaa
 │   cancel / activate / witness / init
 └─ Tickets (isTicket = true)            Vault v4 / Pool v4: lesen ORA (Template-Hash +
     settle: Wartezeit per this.ageDaa   Covenant-ID), sperren bei frozen
                                         Zinskasse v4: liest REG (witness) → Zinsziel
```

**Reihenfolge beim Deployment** (Henne-Ei wie bei der Factory):
1. Register-Genesis mit Startsatz-Hash, Notfallsatz-Hash (oder 0), Zinsziel, `lastDaa` = Start-DAA, `initialized = false`. Die Genesis-Tx darf **genau einen** Ausgang mit der neuen Register-ID haben. Das Orakel prüft nur die Register-ID, nicht dessen Template. Ein zweiter Genesis-Ausgang mit fremdem Skript wäre eine Hintertür. Das muss ghostctl beim Deployment selbst sicherstellen und öffentlich prüfbar machen [V, noch nicht gebaut].
2. Orakel-Genesis mit `regCovId`, `frozen = false`, `lastRateDaa = initOracleDaa`.
3. `init` des Registers (Deployer-Signatur): Orakel-ID, Orakel-Template-Hash, Präfix-/Suffixlänge. Dabei werden der Startsatz gegen den Hash und die Grenzen geprüft. Ein Orakel-Eingang ist in dieser Tx verboten. Danach hat der Deployer keine Rechte mehr.
4. Adresse der Zinskasse berechnen (Register-ID + Register-Template). Dafür ist keine Tx nötig.
5. Factory (unveränderter Quelltext) und GHOST wie in v3, Vault-Template v4 mit `interestSpk` und `maxDebt`.
6. Pool v4.

### 2.1 Register (`signer_register_v4.sil`)

Satz-Hash: `sha256(n ‖ t ‖ tRot ‖ k0 ‖ … ‖ k(n−1))`, Zahlen als 8 Byte wie `x as byte[8]`. Die Schlüssel kommen bei jedem Eintrag als Argument mit. `fbHash = 0…0` heißt „kein Notfallsatz“, denn dazu gibt es kein Urbild.

| Eintrag | wer | Regeln |
|---|---|---|
| `init` | Deployer, einmal | nicht initialisiert, kein Ticket, Startsatz in den Grenzen und passend zum Hash, kein Orakel-Eingang |
| `attestPrice` | jeder Einreicher mit t Signaturen | t Signaturen des Hauptsatzes (Indizes streng aufsteigend, < n) über `sha256(oracleCov ‖ 0x01 ‖ kasUsd ‖ oracleDaa ‖ seq ‖ rate)`. Der Orakel-Ausgang muss genau diesen Zustand tragen (`validateOutputStateWithInputTemplate`). `lastDaa` wird neu gesetzt. Ein offenes **Notfall**-Ticket verfällt (nonce + 1). |
| `propose` | tRot des Hauptsatzes oder, als Notfall, tRot des Notfallsatzes | Grenzen des neuen Satzes. Der Payload ist genau `newSet ‖ newFb ‖ newKind ‖ newTo`. Signiert wird `sha256(regCov ‖ Tag ‖ nonce+1 ‖ Ankündigung)`. Es entstehen genau zwei Ausgänge: die Haupt-UTXO (nonce + 1, Satz unverändert) und ein Ticket. Ein Notfall ist erst ab `lastDaa + emergAfterDaa` möglich, und der Notfallsatz wird beim Einsatz auf die Grenzen geprüft. |
| `cancel` | t des Hauptsatzes | nonce + 1 entwertet jedes offene Ticket, auch ein Notfall-Ticket |
| `activate` | jeder | lebendes Ticket derselben Covenant mit `nonce == nonce`. Satz, Notfallsatz und Zinsziel werden übernommen, nonce + 1. |
| `witness` | jeder | Haupt-UTXO unverändert, damit die Zinskasse das Zinsziel lesen kann |
| `settle` (Ticket) | jeder | Das Ticket erzeugt keine Ausgänge der Register-Covenant. Die Haupt-UTXO muss in der Tx stehen. Ein **lebendes** Ticket gilt erst nach `this.ageDaa ≥ rotDelayDaa` (Notfall: `emergDelayDaa`) und nur, wenn die Haupt-UTXO genau den aktivierten Zustand erzeugt. Ein **abgelaufenes** Ticket darf jeder aufräumen. |

Alle Einträge außer `attestPrice` verbieten einen Orakel-Eingang (`noOracle`).

### 2.2 Orakel v4 (`price_oracle_v4.sil`)

- `update`: genau ein Register-Eingang. Dazu gelten alle v3-Grenzen: 0,00001–900 USD, ×2/÷2, mindestens 600 DAA Abstand, keine Zukunft, 0 ≤ Zins ≤ `maxRate`.
- **Neu ist der Zinsrahmen**: höchstens `rateStep` je Änderung und höchstens eine Änderung je `rateGapDaa`. `lastRateDaa` wird gesetzt.
- Der Index wird wie in v3 fortgeschrieben. `frozen` geht auf `false`.
- `freeze`: jeder darf das, sobald `tx.daa ≥ oracleDaa + freezeAfterDaa` gilt. Danach ist `frozen = true`, sonst ändert sich nichts.
- `read` und `freeze` prüfen das Register nicht. Kein Register-Eintrag erlaubt einen unveränderten oder eingefrorenen Orakel-Ausgang: `attestPrice` verlangt `seq + 1` und `frozen = false`, alle anderen Einträge verbieten das Orakel. Das spart Bytes in jeder Nutzer-Tx.

### 2.3 Vault v4, Pool v4, Factory

- **Vault v4:** Ist das Orakel `frozen`, sind gesperrt: `mint`, `redeem`, `liquidate`, `sweep` sowie `withdraw` bei Schuld > 0. Erlaubt bleiben `deposit`, `repay`, `close` und `withdraw` ohne Schuld.
- **Zinsziel des Vaults:** `byte[] interestSpk` ist ein beliebiges scriptPubKey (Version + Skript), und zwar am Ausgang direkt hinter dem eigenen Eingang, wie in v3.
- **Pool v4:** Das Orakel-Layout ist erweitert. `stopWhenFrozen` (Konstruktor) blockt bei eingefrorenem Orakel `swap` und `init`. `add` und `remove` lesen das Orakel nicht und bleiben frei.
- **Factory:** Der Quelltext bleibt unverändert. Der Test `v4_factory_bleibt_unveraendert_nutzbar` zeigt, dass die Zustandsbytes von Vault v3 und v4 für jeden Zustand gleich sind [B]. Die Factory braucht deshalb kein v4.

### 2.4 Zinsziel: drei Varianten

| Variante | `interestSpk` im Vault | Wechsel ohne neues Deployment |
|---|---|---|
| Adresse wie heute | P2PK des Schlüssels | nein (im Vault-Template fest) |
| Verbrennen | P2SH von `OP_FALSE` (unspendbar) | nein |
| **Zinskasse v4** | P2SH der Zinskasse | **ja**: Das Ziel steht im Register (`payKind`/`payTo`) und ändert sich nur über `propose` → Wartezeit → `activate`. Mögliche Ziele: P2PK (0), P2SH (1, z. B. späterer Topf oder unspendbar = verbrennen), jeder andere Wert = Kasse hält fest. |

Warum der Wechsel nicht direkt im Vault geht: Das Zinsziel steckt im Vault-Template, und die Factory bindet den Template-Hash unveränderlich. Ein änderbares Ziel braucht deshalb eine Umleitung. Die Zinskasse ist diese Umleitung.

**Sicherheit der Zinskasse:**
- `forward` ist für jeden erlaubt. Der ganze Betrag geht an den Ausgang am **Index des eigenen Eingangs**. Zwei Kassen-Eingänge können sich keinen Ausgang teilen (Lehre aus A11-V-1).
- Ziel ist nie die Kasse selbst. Sonst könnte ein Vault-Schließen den Ausgang einer Weiterleitung als seine Zinszahlung mitbenutzen. Das ist ein geteilter Ausgang, getestet in `nie_an_die_kasse_selbst`.
- Das Ziel stammt nur aus der echten Haupt-UTXO: Register-ID, Template-Hash und `isTicket = false` werden geprüft. Ein angekündigtes Ziel in einem Ticket zählt nicht.
- Die Netzgebühr trägt, wer weiterleitet. Eine `witness`-Tx kostet etwa 0,017 KAS, weil das Register-Skript mitfährt.

---

## 3. Parameter (alle einstellbar)

| Parameter | Vertrag | Vorschlag | Alleinbetrieb / Mainnet-Probe | Hinweis |
|---|---|---|---|---|
| `minSigners` | Register | 5 | 1 | Grenze für jeden **neuen** Satz und den Startsatz; Template-Konstante |
| `minThreshold` | Register | 3 | 1 | dazu immer `2t > n`, `t ≤ tRot ≤ n`, `n ≤ 9` (Schleifengrenze) |
| Startsatz n / t / tRot | Register-Genesis | 7 / 4 / 5 | 1/1/1 oder 5/3/4 mit eigenen Schlüsseln | später per Austausch |
| Notfallsatz | Register-Genesis, bei jedem Austausch neu | 5 / 3 / 4, unabhängig | `0…0` (keiner) oder eigener Schlüssel | optional |
| `rotDelayDaa` | Register | 14 Tage = 12 096 000 | 1 h = 36 000 | Ticket-Alter (hart, `this.ageDaa`) |
| `emergAfterDaa` | Register | 30 Tage = 25 920 000 | 1 h | ab `lastDaa` (letzter Preis) |
| `emergDelayDaa` | Register | 14 Tage = 12 096 000 | 1 h | Ticket-Alter |
| `freezeAfterDaa` | Orakel | 2 h = 72 000 | 1 h oder kürzer | Heartbeat ≤ 60 min |
| `maxRate` | Orakel | 20 %/Jahr = 634 195 839 | gleich | Zins je DAA × 1e18 |
| `rateStep` | Orakel | 0,5 Punkte = 15 854 896 | gleich | |
| `rateGapDaa` | Orakel | 1 h = 36 000 | gleich | |
| `interestSpk` | Vault | P2SH der Zinskasse | P2PK eigene Adresse oder Zinskasse | Abschnitt 2.4 |
| Zinsziel `payKind`/`payTo` | Register-Genesis | Entscheidung offen | eigene Adresse (0) | nur über Austausch änderbar |
| `maxDebt` | Vault | – | **≤ 5 GHOST** = 500 000 000 | |
| `stopWhenFrozen` | Pool | true | true | |

Umrechnung: 10 BPS, 1 h = 36 000 DAA, 1 Tag = 864 000 DAA.

**Achtung bei der Probe:** Alle Grenzen und Fristen sind Template-Konstanten. Ein Probe-Deployment mit `minSigners = 1` und 1 h Wartezeit lässt sich später nicht verschärfen. Für das echte v4 braucht es ein eigenes Deployment mit den Vorschlagswerten. Der Austausch der Schlüssel geht dagegen in beiden ohne neues Deployment.

---

## 4. Sicherheitsregeln und ihre Tests

| Regel | Test (Datei) |
|---|---|
| Orakel-Update ohne Register scheitert | `update_ohne_register_scheitert` (oracle_v4) |
| **Kein Austausch-Eintrag neben einem Orakel-Update**: `witness`, `propose`, Notfall, `cancel` und `init` lehnen selbst ab, obwohl das Orakel allein zustimmen würde. `activate` scheitert schon am Orakel (zwei Register-Eingänge). | `kein_austausch_eintrag_neben_einem_orakel_update`, `init_mit_orakel_update_scheitert` (register_v4) |
| Quorum signiert P, Orakel setzt P′: das Register lehnt ab | `orakel_ausgang_muss_dem_signierten_preis_entsprechen` |
| Zu wenige, doppelte, unsortierte, fremde Signaturen; Index ≥ n; falsche Schwelle; Notfallsatz setzt Preise | `preis_mit_*`, `ankuendigung_braucht_austausch_schwelle` |
| Signatur bindet Preis, seq und Orakel-ID bzw. Ankündigung, nonce und Tag (keine Wiederholung) | `signatur_bindet_*`, `absage_signatur_gilt_nur_fuer_diese_nonce` |
| **Wartezeit hart**: Das Ticket prüft `this.ageDaa`. Die relative Sperre zählt ab Aufnahme des Tickets und lässt sich weder zurückdatieren (NEU-1) noch durch Updates zurücksetzen. Eine DAA zu früh scheitert. | `austausch_erst_nach_der_wartezeit`, `notfall_ticket_wartet_emerg_delay_und_verfaellt_durch_update` |
| Reifes Ticket lässt sich nicht zerstören (z. B. per `witness`); gefälschtes Ticket (fremde Covenant), Ticket gegen ein zweites Ticket statt gegen die Haupt-UTXO, Ticket legt zweite Haupt-UTXO an | `lebendes_ticket_laesst_sich_nicht_zerstoeren`, `ticket_muss_von_der_register_covenant_sein`, `ticket_braucht_die_haupt_utxo`, `ticket_erzeugt_keine_register_ausgaenge` |
| Geteilte Ausgänge: Register und Orakel setzen sich über `OpAuthOutputIdx` des eigenen Eingangs fort. Die Zinskasse zahlt am eigenen Index und nie an sich selbst. Der Vault zahlt den Zins wie in v3 an `idx + 1`. | `zahlung_am_index_des_eigenen_eingangs`, `nie_an_die_kasse_selbst`, v3-Tests in vault_v4 |
| Ankündigung öffentlich (Payload genau = Ankündigung) | `ankuendigung_muss_im_payload_stehen` |
| Grenzen des neuen Satzes und des Notfallsatzes | `ankuendigung_prueft_die_grenzen_des_neuen_satzes`, `notfallsatz_wird_bei_benutzung_auf_grenzen_geprueft` |
| Notfall erst nach Stille, nur durch den Notfallsatz, verfällt durch jedes Preis-Update | `notfall_*`, `notfall_ankuendigung_verfaellt_durch_preis_update` |
| Einfrieren erst nach der Frist, durch jeden; Update taut auf und trägt den Zins nach | `einfrieren_*` (oracle_v4) |
| Vault-Sperren bei `frozen` (jede einzeln mit Gegenprobe), erlaubte Aktionen bleiben | `v4_eingefroren_*` (vault_v4) |
| Pool bei `frozen` | `v4_eingefroren_*` (pool_v4) |
| Zinsziel P2PK / P2SH / Verbrennen | `v4_zinsziel_p2sh_zinskasse_und_verbrennen`, `weiterleiten_*` |

---

## 5. Grenzen und Restrisiken

- **Einfrieren wirkt erst, wenn es jemand auslöst.** Zwischen Fristablauf und `freeze` kann jeder den veralteten Preis per `read` nutzen. Ein Vertrag kann keine Obergrenze für `tx.daa` prüfen. Der Agent (oder jeder Nutzer) muss `freeze` sofort nach Fristablauf senden.
- **NEU-1 bleibt für Preise:** `oracleDaa` wählen die Unterzeichner (≤ Kettenstand). Ein Rückdatieren macht den Preis nur früher einfrierbar und den Notfallweg früher möglich. Beides schadet dem Angreifer.
- **`lastDaa` für den Notfall** setzen die Unterzeichner. Ein böswilliger Hauptsatz könnte den Notfallweg nur verfrühen, nicht verhindern.
- **`witness` ist für jeden offen.** Wer ständig `witness` sendet, kann Updates verzögern: Das Register muss neu gebaut werden, die Signaturen bleiben aber gültig, weil sie das Register-Outpoint nicht enthalten. Das ist dasselbe Muster wie O-1 beim Orakel-`read`.
- **Register-Genesis:** siehe Abschnitt 2, Schritt 1.
- **Doppelte Schlüssel** in einem Satz prüft der Vertrag nicht (Entwurf 1.3). Die Unterzeichner müssen das off-chain ablehnen.
- **Sequenzsperren für Covenant-Eingänge im echten Netz** sind nur in der Engine geprüft (`OpCheckSequenceVerify` gegen die Eingangssequenz) [V]. Die Konsensprüfung der relativen Sperre (UTXO-DAA + Sequenz ≤ Block-DAA) ist erst in der Mainnet-Probe nachzuweisen.
- **Gebühr je Update** (0,02 KAS) wird vom Register-Skript bestimmt. Wenn das zu teuer ist, ließe sich `attestPrice` in eine eigene kleine Covenant auslagern (etwa 1,9 KB, gemessen als Teilübersetzung). Das bedeutet mehr Verträge und mehr Angriffsfläche, deshalb ist es bewusst nicht umgesetzt.

---

## 6. Messwerte [B]

| Messung | Wert |
|---|---|
| Orakel v4 Redeem-Skript / Zustand | 762 B / 56 B (Präfix 1 B, Suffix 705 B); v3: 852 B / 45 B |
| Register v4 Redeem-Skript / Zustand | 7 914 B / 209 B |
| Zinskasse v4 | 851 B |
| Vault v4 | 25 199 B (v3 + 2 Regeln) |
| Pool v4 | ≈ 23,3 KB (silverc mit Platzhalter-Argumenten; v3 mit gleichen Argumenten ≈ 23,6 KB) |
| Teilübersetzung nur `attestPrice` | ≈ 1 923 B (zum Vergleich für eine Auslagerung) |

Update-Tx (Register `attestPrice` + Orakel `update` + Gebührenzahler), `txb::build` mit `MAINNET_PARAMS` (`register_v4_tests::groessen_und_massen`):

| Satz | Sigscripts | compute | transient | storage | Gebühr |
|---|---|---|---|---|---|
| 1 Schlüssel, 1 Sig. | 8 835 B | 11 554 g | 37 136 g | 671 g | 0,0195 KAS |
| 5 Schlüssel, 3 Sig. | 9 109 B | 13 928 g | 38 232 g | 672 g | 0,0201 KAS |
| 7 Schlüssel, 4 Sig. | 9 246 B | 15 065 g | 38 780 g | 672 g | 0,0204 KAS |
| 9 Schlüssel, 5 Sig. | 9 383 B | 16 302 g | 39 328 g | 672 g | 0,0206 KAS |

- Die Gebühr wird von `transient/2` bestimmt, also von der Tx-Größe. Signaturen fallen kaum ins Gewicht.
- Ankündigung (7er-Satz, 5 Signaturen, Payload 97 B, Ticket 1 KAS): 0,0195 KAS, storage 40 613 g (Ticket-Ausgang).
- `witness` (für die Zinskasse): 0,0173 KAS.
- Eine Vault- oder Pool-Tx wird gegenüber v3 um etwa 90 B **kleiner**, weil das Orakel 762 B statt 852 B hat.

---

## 7. Tests

| Datei | Tests | Inhalt |
|---|---|---|
| `protocol/tests/oracle_v4_tests.rs` | 14 | Preisgrenzen, Zinsrahmen, Index, `read`, `freeze`, Auftauen, Größe |
| `protocol/tests/register_v4_tests.rs` | 50 | Quorum, init, propose/cancel/activate/settle/witness, Notfall, Umgehungsangriffe, Alleinbetrieb, Messung |
| `protocol/tests/zinskasse_v4_tests.rs` | 8 | Weiterleiten P2PK/P2SH/Verbrennen, festhalten, geteilte Ausgänge, falsche Quelle |
| `protocol/tests/vault_v4_tests.rs` | 95 | alle 86 v3-Vault-Tests gegen v4 + 9 v4-Tests (Einfrieren je Eintrag, Zinsziel, Factory-Layout) |
| `protocol/tests/pool_v4_tests.rs` | 36 | alle 32 v3-Pool-Tests gegen v4 + 4 v4-Tests (Einfrieren, `stopWhenFrozen`) |
| `protocol/tests/audit10_pool_v4_tests.rs` | 20 | Audit-10-Engine-Tests gegen Pool v4 |
| `protocol/tests/v4common/mod.rs` | – | gemeinsame Bausteine (Digests, Satz-Hash, Tx mit Payload/Locktime/Sequenz) |

Alle bestehenden Suiten bleiben grün (`cargo test --release --offline`, 614 Tests).

---

## 8. Mutationstest

Verfahren: `protocol/mutation/mutate.sh` (jede `require`-Zeile einzeln durch `require(true)` ersetzt), `CARGO_FLAGS="--release --offline"`.

Logs: `audit/v4-mutation-*.log`.

| Vertrag | Testsuiten | rot | Lücken |
|---|---|---|---|
| Orakel v4 | oracle_v4, register_v4 | 15 / 15 | 0 |
| Register v4 | register_v4, oracle_v4, zinskasse_v4 | 31 / 38 | 7, alle doppelt gesichert |
| Zinskasse v4 | zinskasse_v4 | 7 / 7 | 0 |
| Vault v4 | vault_v4 | 52 / 65 | 13 = die v3-Lücken aus Audit 11 |
| Pool v4 | pool_v4, audit10_pool_v4 | 39 / 67 | 28, alle in unverändertem v3-Code |

**Erster Lauf des Registers: 12 Lücken.** Fünf davon waren Testlücken mit echtem Angriff dahinter. Sie sind jetzt jeweils mit einem eigenen Test geschlossen:

| Zeile | Regel | Angriff | Test |
|---|---|---|---|
| L87 | `OpAuthOutputCount == 1` in `cont()` | zweite Haupt-UTXO mit eigenem Satz neben der Fortsetzung | `keine_zweite_haupt_utxo` |
| L94 | `!isTicket` in `live()` | Ticket führt `witness` aus und setzt sich als Haupt-UTXO fort: zweite Haupt-UTXO mit dem angekündigten Satz, ohne Wartezeit | `ticket_wird_nicht_zur_haupt_utxo` |
| L95 | `initialized` in `live()` | Genesis per `witness` als „initialisiert“ fortsetzen, ohne Deployer und ohne Satzprüfung | `witness_initialisiert_nicht` |
| L144 | `!isTicket` in `init` | Ticket-Genesis wird per init zur Haupt-UTXO | `init_nicht_aus_einem_ticket` |
| L267 | `tk.isTicket` in `activate` | `activate` zeigt auf die Haupt-UTXO selbst: nonce + 1 ohne Signatur, also eine freie Absage jeder Ankündigung | `activate_ohne_ticket_ist_keine_freie_absage` |

Dazu kam bei der Zinskasse L53 (`payKind == 1`): Eine Zahlung an P2SH(payTo) bei payKind 2 wurde nicht geprüft. Neu ist der Test `unbekannte_art_auch_nicht_als_p2sh`.

**Register, verbleibende 7 Lücken, alle doppelt gesichert:**
- **L107** `n ≤ 9`: Die Schleife in `hashSet` hat die Grenze 9, der Compiler prüft sie zur Laufzeit. Der Fall (10, 6, 6) scheitert auch ohne diese Regel.
- **L115** `keys.length == n` und **L132** `j < n`: sichern sich gegenseitig. Bei mehr Schlüsseln als n zählt nur der Hash über die ersten n, und `j < n` verbietet die übrigen. Bei weniger Schlüsseln bricht der Zugriff `keys[i]` ab. **Nur gemeinsam geprüft:** Fehlten beide, könnte jemand ungehashte Zusatzschlüssel anhängen und damit signieren.
- **L126** `sigs.length == need` und **L127** `idx.length == need`: Die Schleife läuft genau `need`-mal. Fehlende Einträge brechen ab, überzählige werden nicht gelesen (wie im v3-Orakel).
- **L173** `OpCovOutputCount(oracleCov) == 1`: Ausgänge mit der Orakel-ID kann nur der Orakel-Eingang erzeugen, und dessen `continuation()` erlaubt genau einen.
- **L300** `isTicket` in `settle`: Führt die Haupt-UTXO `settle` aus, kann sie nur auf sich selbst zeigen. Wegen gleicher nonce greift dann der Zweig für lebende Tickets, und der verlangt eine Fortsetzung, die bei null Ausgängen fehlt. Test `haupt_utxo_laesst_sich_nicht_per_settle_vernichten`.

**Vault v4:** Die 13 Lücken sind L199 `newDebt ≥ 0`, L201 `newInterest ≥ 0`, L220, L230–L234, L327, L344, L354, L355 und L364. Es sind dieselben Regeln wie in `ARCHITEKTUR.md` „Version 3 nach Audit 11“, um 18 Zeilen verschoben. Neu in v4 sind L181 (Zinsziel) und L214 (Frischeprüfung), beide rot.

**Pool v4:** Neu ist nur L149 (`stopWhenFrozen && frozen`), sie wird rot. Die 28 Lücken liegen alle in Zeilen, die aus v3 unverändert übernommen sind:
- Ein-/Ausgangszahlen und `outs.length` (KCC20 oder `OpCov*Idx`);
- `mine`/`isMinter` weiterer Anteils-Eingänge;
- Reserve > 0, S′ > 0, m > 0, Richtung der Reserven;
- `initialized` in add/remove;
- Genesis-ID ≠ 0, ≤ 8 Ausgänge, `lid ≠ ghostCovId`, `bandBps`-Grenzen in init;
- L148 `kasUsd > 0`: unerreichbar, das Orakel v4 hat mindestens 1 000.

Die Begründungen stehen in `audit/10-opus-pool.md`, Abschnitt 5. Ein Vergleichslauf derselben Methode auf `ghost_pool.sil` (v3) wurde **nicht** gemacht, um den v3-Vertrag im Arbeitsbaum nicht anzufassen. Die Zuordnung ist hergeleitet.

---

## 9. Offene Punkte für ghostctl, Seite und Deployment

1. **`contracts.rs`:** Kompilier-Helfer und Parameter-Structs für Register, Orakel v4, Zinskasse, Vault v4 und Pool v4. Dazu die Digests (`price`, `rotate`, `cancel`) und `set_hash`. Vorlage ist `protocol/tests/v4common/mod.rs`.
2. **`txb.rs`:** setzt heute für jeden Eingang Sequenz 0 (`assemble`). Für `settle` ist eine Sequenz je Eingang nötig (Ticket-Alter). Sonst scheitert jede Aktivierung.
3. **`chain.rs`:** Folger für die Register-Haupt-UTXO und für Tickets. Die Ankündigung steht im Payload, die Schlüssel im Sigscript. Der Orakel-Folger braucht `frozen`/`lastRateDaa`.
4. **Deployment** in der Reihenfolge aus Abschnitt 2. Dazu gehört eine Prüfung, dass die Register-Genesis genau einen Ausgang mit der neuen ID hat. Beim Start wird gegengeprüft: Orakel-Template neu kompiliert = Hash im Register, Register-Template = Hash in der Zinskasse.
5. **Signer und Relay** (Entwurf 2.1–2.3): Signaturen über `price_digest`, Austausch-Signaturen von Hand. Die deterministische Zinsregel muss den Zinsrahmen des Orakels einhalten (Schritt, Takt).
6. **Agent:**
   - `freeze` nach Fristablauf sofort senden;
   - `activate` + `settle` bei fälligem Ticket;
   - abgelaufene Tickets aufräumen;
   - Zinskasse weiterleiten (`witness` + `forward`).
7. **Seite:**
   - Anzeige „eingefroren“;
   - angekündigter Austausch mit Wirksamkeits-DAA (Ticket-Aufnahme + Wartezeit) und den neuen Schlüsseln;
   - gesperrte Aktionen ausgrauen.
8. **Mainnet-Probe:**
   - Parameter wie Spalte „Alleinbetrieb“, `maxDebt ≤ 5 GHOST`;
   - alle Einträge einmal echt ausführen, besonders `settle` mit Sequenzsperre;
   - Gebühren gegen Abschnitt 6 vergleichen.

## 10. Fragen an den Nutzer

1. **Zinsziel beim Start:** eigene Adresse (P2PK, wie heute), Zinskasse v4 (Ziel später änderbar) oder Verbrennen? Empfehlung: Zinskasse v4 mit eigener Adresse als erstem Ziel. Ein Wechsel ist dann ohne neues Deployment möglich.
2. **Update-Gebühr** von etwa 0,02 KAS (rund 0,49 KAS/Tag bei stündlichem Heartbeat) annehmen, oder soll `attestPrice` in eine kleinere Covenant ausgelagert werden?
3. **Pool bei eingefrorenem Orakel sperren** (`stopWhenFrozen = true`, Vorschlag)?
4. **Probe-Parameter:** 1-von-1 mit `minSigners = minThreshold = 1` und Fristen von 1 h? Oder schon 3-von-5 mit eigenen Schlüsseln?
5. **Notfallsatz** in der Probe: keiner (`0…0`) oder ein zweiter eigener Schlüssel, um den Notfallweg zu üben?
6. **Fristen für das echte v4:** 14 Tage / 2 h / 30 Tage übernehmen?

---

## 9. ghostctl und Seite (Stand b0f6d86)

Entscheidungen des Nutzers (04.10.2026): Update-Gebühr ≈ 0,02 KAS bleibt (keine Auslagerung von `attestPrice`), Pool mit `stopWhenFrozen = true`, Zins direkt an die eigene Adresse (`interestSpk` = P2PK, keine Zinskasse), **ein einziges Orakel** (Register mit `minSigners = minThreshold = 1` auch im echten v4), Fristen echt 14 Tage / 2 h / 30 Tage + 14 Tage, Probe 1 h, 1 KAS je Covenant, höchstens 5 GHOST je Vault, kein Notfallsatz.

| Befehl | Was |
|---|---|
| `committee-keygen <datei> --count n` | Unterzeichner-Schlüssel (Standard 1) |
| `deploy --key … --committee … [--probe] [--threshold t]` | Register-Genesis, Orakel-Genesis, Register-Init, Factory-Genesis, Factory-Init (fortsetzbar) |
| `oracle-update`, `oracle-feed`, `agent --committee` | Preis über `attestPrice` + `update`; Signierer = Schlüssel der Datei, die zum aktuellen Satz gehören |
| `oracle-freeze --key …` | jeder, ab Frist; der Agent (auch ohne `--committee`) friert ein veraltetes Orakel selbst ein |
| `signers show / propose / cancel / activate / clear` | Austausch: `activate` setzt die Sequenz des Ticket-Eingangs auf die Wartezeit |

- Bibliothek: `ops::{deploy_register, init_register, oracle_update, oracle_freeze, propose, cancel_rotation, activate_rotation, clear_ticket, check_update}`, `chain::{register_next, follow_register}`, `txb::build_ext` (Sequenz je Eingang), Simulator prüft die relative Sperre wie `check_sequence_lock`.
- Tests: alle bisherigen Suiten laufen gegen das v4-Deployment; neu `tests/v4_ops_tests.rs` (9 Tests: Quorum, Zinsrahmen, Einfrieren/Auftauen mit Sperren, Austausch mit zu früher Aktivierung, Absage, Notfallweg, Nachführen über die Kette, Genesis-Prüfung, Satzgrenzen).
- Probelauf gegen das Mainnet (04.10.2026, nichts gesendet): Deployment baut, Gebühren 0,0022 / 0,0022 / 0,0178 / 0,0022 / 0,0097 KAS.
- Mainnet-Probe: `GHOST-v4-Probe.command` (drei Stufen, eigene Zustandsdatei `deployments/mainnet-v4probe.json`, eigener Schlüssel `keys/v4probe-owner.json`).

---

## 10. Mainnet-Probe 05.10.2026: Signaturprüfungen je Eingang

Die erste Probe scheiterte bei Register-Init: Der Node lehnte ab mit „transaction input #0 has 28 signature operations which is more than the allowed max amount of 15“. Das ist eine Mempool-Standardregel (rusty-kaspa `MAX_STANDARD_P2SH_SIG_OPS = 15`, `post_toccata_p2sh_sig_scanner`), keine Konsensregel. Gezählt wird **statisch** jede `OpCheckSig*`/`OpCheckSigFromStack*` im ganzen Redeem-Skript, auch in ausgerollten Schleifen. Das Register enthielt `checkQuorum` dreimal mit je 9 ausgerollten Prüfungen plus `checkSig` in `init`: 3 · 9 + 1 = 28.

- **Vertrag:** `MAX_QUORUM = 4` (Schleife in `checkQuorum`, ausdrücklich `need ≤ 4`), `MAX_SIGNERS = 7` (aus 2t > n), `checkBounds` verlangt `tRot ≤ 4`. Jetzt 3 · 4 + 1 = **13**. Größter Satz: 7 Schlüssel, Schwellen bis 4 (z. B. 4 von 7).
- **Off-chain:** `txb::check_standard_sig_ops` in jedem Bau und im Simulator; `tests/standard_sigops_tests.rs` zählt jedes Vertragsskript (Register 13, Vault v4 5, Token 2, Tresor 2, Factory 1, Pool v4 1, Orakel 0).
- **Rückbau-Proben:** `tRot ≤ 4` → Test rot. `need ≤ 4` und `n ≤ 7` in `hashSet` sind doppelt gesichert (Laufzeitprüfung der Schleife des Compilers, `checkBounds`), daher kein Test rot.
- **Kosten der gescheiterten Probe:** Register- und Orakel-Genesis (je 1 KAS) sind mit dem alten Template gebunden und nicht nutzbar, dazu etwa 0,005 KAS Gebühren. Fortschritt beiseitegelegt als `deployments/mainnet-v4probe.deploy-verworfen-sigops.json`.
- Register-Init kostet jetzt 0,0150 KAS (vorher 0,0178).

## 11. Ergebnis der Mainnet-Probe (05.10.2026, 13:36–17:35)

Alle drei Stufen sind im Mainnet durchgelaufen (Register e26ea1eb…, Orakel b15af2be…, Factory 4a9323f6…, GHOST 6ae7d53f…).

| Schritt | Ergebnis | Gebühr |
|---|---|---|
| Register-Genesis, Orakel-Genesis, Register-Init, Factory-Genesis, Factory-Init | angenommen | 0,0022 / 0,0022 / 0,0150 / 0,0022 / 0,0097 KAS |
| Preis-Update über das Register (attestPrice + update) | angenommen | 0,0168 KAS |
| Vault eröffnen (10 KAS), 0,1 GHOST prägen | angenommen | 0,0634 / 0,0608 KAS |
| Orakel einfrieren (jeder, nach der Frist) | angenommen | 0,0031 KAS |
| Prägen bei eingefrorenem Orakel | gesperrt (Vorprüfung ghostctl; die Vertragssperre ist Engine-getestet) | – |
| Preis-Update taut auf | angenommen | 0,0168 KAS |
| Austausch ankündigen (Payload, Ticket 1 KAS) | angenommen (WebSocket riss danach ab, Journal übernahm den Zustand) | 0,0155 KAS |
| **Zu frühe Aktivierung** | **vom Node abgelehnt: „one of the transaction sequence locks conditions was not met“** – die relative Sperre setzt der Konsens durch | – |
| Aktivierung nach der Wartezeit | angenommen, neuer Satz aktiv | 0,0285 KAS |
| Alter Unterzeichner | abgewiesen (Vorprüfung ghostctl) | – |
| Preis-Update mit dem neuen Unterzeichner | angenommen | 0,0168 KAS |
| Tilgen, Schließen, Rest zurück (15,42 KAS) | angenommen | 0,0646 / 0,0558 KAS |

Vorfälle und Korrekturen:
- **Mempool-Regel 15 Signaturprüfungen je Eingang** (1. Versuch): Register verkleinert (Abschnitt 10).
- **Status zehn Minuten nicht lesbar** (13:49–14:00): Das Skript hielt nach drei Versuchen an; jetzt zählt das als Störung (bis 1 h).
- **WebSocket-Abbruch nach dem Senden**: Das Journal (`store::resolve_pending`) übernahm die angenommene Ankündigung, nichts wurde doppelt gesendet.
- **Rücküberweisung**: 0,2 KAS Rest gingen als Gebühr an die Miner (unter MIN_CHANGE); jetzt bleiben nur 0,005 KAS stehen.
