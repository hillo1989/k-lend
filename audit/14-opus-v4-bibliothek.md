# Audit 14: Off-chain-Bibliothek v4 (Commit b0f6d86)

Stand 05.10.2026, Worktree `kaspa-lending-v4`, Branch `v4`. Ich habe nur gelesen und nichts committet. Dateien unter `keys/` und `deployments/` habe ich nicht gelesen, Transaktionen habe ich keine gesendet.

**Gegenstand:** `protocol/src/{ops,contracts,chain,store,txb,sim,pool}.rs` und die dazu nötigen Stellen in `bin/ghostctl.rs`. Verglichen habe ich sie mit `contracts/{signer_register_v4,price_oracle_v4,stable_vault_v4,ghost_pool_v4}.sil` und mit rusty-kaspa a41a333.

**Methode:**
- Ich habe den Code gelesen.
- Ich habe eine temporäre Testdatei `protocol/tests/audit14_probe.rs` mit 8 Tests geschrieben (A–H). Alle 8 liefen grün und belegen die Befunde. Danach habe ich die Datei wieder gelöscht. Die wichtigsten Stellen sind unten zitiert.
- Die ganze Suite läuft mit `cargo test --release --offline` vollständig grün, kein Test schlägt fehl.
- `git status` war danach sauber. Die Ausnahme ist `audit/14-opus-v4-ghostctl.md`, die nicht von mir stammt.

Kennzeichnung: **[B]** heißt belegt durch einen Test oder eine Quellstelle, **[V]** heißt Vermutung.

---

## Zusammenfassung nach Schwere

| # | Schwere | Befund |
|---|---|---|
| H-1 | **hoch** | Der Agent friert sein eigenes Orakel ein. Das Heartbeat steht standardmäßig auf 6 h, die Frist zum Einfrieren liegt bei 2 h. |
| H-2 | **hoch** | Ist ein Austausch aktiviert, sperrt `resync` ghostctl für alle Nutzer der Zustandsdatei. Das gilt auch, wenn ein Dritter das eigene Ticket aktiviert, und dafür bekommt er rund 0,97 KAS. |
| M-1 | mittel | Ein Zins außerhalb des 0,5-Rasters (von Hand oder per `deploy --rate`) blockiert dauerhaft jedes Preis-Update, solange die Zinsregel ändern will. Folge: Das Orakel friert ein. |
| M-2 | mittel | `deploy --rate` ist nach oben nicht begrenzt. Ein Startzins über maxRate + rateStep sperrt das Orakel für immer, und das Deployment ist verloren. |
| M-3 | mittel | Eine fremde reguläre Ankündigung wird nicht gemeldet. Gleich nach der Ankündigung beginnt die Frist, in der man widersprechen kann, aber niemand erfährt davon. |
| N-1 | niedrig | Tickets fallen aus der Zustandsdatei: bei einer zweiten Ankündigung und bei einem Preis-Update mit offenem Notfall-Ticket. Je Ticket ist 1 KAS nur noch von Hand rückholbar oder geht an Dritte. |
| N-2 | niedrig | Zinsschritt: Der Takt der Zinsregel misst Uhrzeit, der Vertrag misst DAA. Ein knapper Takt lässt das ganze Update der Runde scheitern, auch den Preis. |
| N-3 | niedrig | `spk_bytes` und `spk_from_bytes` kodieren die Version als little-endian, der Konsens als big-endian. Das ist heute folgenlos, weil die Version immer 0 ist. |
| Hinweise | – | MAX_STEPS beim Nachführen; Melde-Text bei Notfall neben eigener Ankündigung; Bestätigung der Genesis nur lokal; v3-Dateien laden nicht. |

**Ohne Befund, also korrekt [B]:**
- Alle acht ops-Funktionen erfüllen die Vertragsanforderungen: Argumente, Ausgangsindizes, `authorizing_input`, Werte, Payload, Locktime und Sequenz.
- Die Zustandsbytes und `parse_register_state` stimmen mit dem Compiler überein, auch bei Grenzwerten.
- `oracle_next` und `register_next` sind eindeutig.
- Eine falsche REST-Antwort führt nicht zu einem falschen Zustand.
- Der Simulator prüft die relative Sperre bit- und grenzgenau wie `check_sequence_lock`.
- `check_update` deckt sich 1:1 mit `price_oracle_v4.sil update()`.

---

## 1. Bauen die ops-Funktionen die verlangten Transaktionen?

Ich habe jede Funktion Zeile für Zeile mit dem Vertrag verglichen. Das Ergebnis ist **in allen acht Fällen korrekt** [B, Quellvergleich; die Simulator-Tests in `v4_ops_tests.rs` sowie Test C, D, E und H laufen damit durch].

| Funktion | Prüfung |
|---|---|
| `deploy_register` (ops.rs:188–195) | Genau ein Ausgang mit Index 0. `authorizing_input` ist 0, die Covenant-ID kommt aus dem ersten Funding-Outpoint. `check_register_genesis` zählt die Ausgänge mit dieser ID (ops.rs:198–204). Locktime ist 0. |
| `init_register` (ops.rs:219–256) | Die Argumente sind n, t, tRot, keys, oCov, oTpl, oPre, oSuf und die Signatur an Index **8** (ops.rs:250). Das passt zu `init(...)` in signer_register_v4.sil:143. Die Tx hat keinen Orakel-Eingang (L149). Ausgang 0 hat auth 0 und denselben Wert (`cont()`, L86–90). Der Folgezustand ändert nur die Orakelfelder und `initialized` (L151–155). |
| `oracle_update` (ops.rs:429–482) | Eingang 0 ist Register `attestPrice`. Dessen Argumente stehen in der Reihenfolge von L164f. (Preis, DAA, seq, rate, index, rateDaa, Satz, sigs, idx). Eingang 1 ist Orakel `update(kas, daa, rate)`. Ausgang 0 ist das Register (auth 0), Ausgang 1 das Orakel (auth 1, `OpCovOutputIdx(oracleCov,0)`, L175). Der Digest `oracle_digest` (contracts.rs:153) entspricht L168–170. Der Registerfolgezustand ist nonce+1 nur bei `emerg`, dazu `emerg=false` und `lastDaa=newDaa` (L182–192). Locktime ist `new_daa`, weil `tx.daa ≥ newOracleDaa` als CLTV gilt. |
| `oracle_freeze` (ops.rs:511–526) | Ein Eingang `freeze`, Ausgang 0 mit gleichem Wert. Locktime ist `daa ≥ oracleDaa + freezeAfterDaa`. |
| `propose` (ops.rs:533–582) | Die Argumente stehen in der Reihenfolge von L203f. Der Payload ist genau `newSet‖newFb‖kind‖to` mit 97 B (ops.rs:564, 576). Der Digest enthält Tag, nonce+1 und die Ankündigung (contracts.rs:340). Es gibt zwei Ausgänge mit auth 0 (`OpAuthOutputCount==2`, L224). Das Ticket bekommt `TICKET_VALUE`. Die Locktime ist nur im Notfall gesetzt, und zwar `daa`. |
| `cancel_rotation` (ops.rs:585–598) | Das Quorum ist t, der Digest `cancel_digest(nonce+1)` (L248). Folgezustand: nonce+1 und `emerg=false`. |
| `activate_rotation` (ops.rs:602–635) | Eingang 0 ist Haupt `activate(ticketIdx=1)`, Eingang 1 ist Ticket `settle(mainIdx=0)`. Es gibt genau einen Ausgang mit auth 0. Das Ticket hat `OpAuthOutputCount==0` (L301). Die Sequenz ist `[0, delay]` (ops.rs:628), dabei `delay=emergDelay` beim Notfall-Ticket. Der Folgezustand stimmt mit `activate` (L270–275) und mit `settle` (L311–316, `lastDaa = mn.lastDaa`) überein. |
| `clear_ticket` (ops.rs:639–657) | Haupt `witness`, Ticket `settle(0)` mit abweichender nonce (freier Zweig). Der Ausgang ist der unveränderte Hauptzustand. Die 1 KAS des Tickets gehen an `fund` als Wechselgeld. |

Massen aus Test H mit Mainnet-Parametern des Simulators, bei einem 9er-Zielsatz:

| Tx | compute | transient | storage | Gebühr |
|---|---|---|---|---|
| `propose` | 16 330 | 37 320 | 43 920 | 0,0196 KAS |
| `activate` | 19 031 | 65 204 | – | 0,0342 KAS |

Beide liegen unter den Blockgrenzen. Mempool-Standardregeln prüft der Simulator nicht [V: keine Prüfung gegen einen echten Node].

## 2. Zustandsbytes und Nachführen

**Layout [B, Test A]:** Für zufällige und extreme Zustände gilt `Shape::of(...).code(register_state_bytes(s)) == bytecode(register(p, s))`. Getestet habe ich:
- `pay_kind` mit 0, 1, 2, 0x7f, 0x80 und 0xff;
- nonce bis `i64::MAX/3`;
- `oracle_suf` mit 2^33;
- `ticket`, `emerg` und `initialized` in allen Kombinationen.

`parse_register_state(register_state_bytes(s)) == s` gilt ebenfalls. Beim Orakel stimmen 56 B gegen den Compiler, mit `frozen` = true/false und `stable_index` bis `i64::MAX/2`. Die Länge ist 209 B (chain.rs:132–148) bzw. 56 B (chain.rs:121–130) und passt zu `docs/v4-entwurf.md` Abschnitt 6.

**Eindeutigkeit:**
- `oracle_next` (chain.rs:333–361) prüft `read`, also unverändert, und `freeze`, wobei nur `frozen` gesetzt wird. Danach geht es über `update` mit Kandidaten aus dem Signaturskript des Orakel-Eingangs. Der Folgezustand ist `oracle_next_state`, genau wie im Vertrag samt `lastRateDaa`.
- `register_next` (chain.rs:368–394) deckt `witness`, `cancel`/regulär `propose` (dasselbe Ergebnis, siehe M-3), Notfall-`propose`, `attestPrice` mit oder ohne offenen Notfall und `activate` ab. `activate` liest den Ticket-Zustand aus dem Redeem-Skript des Ticket-Eingangs. Alle Kandidaten haben `ticket=false` und können deshalb nicht auf den Ticket-Ausgang passen.
- Die Zustände sind paarweise verschieden (nonce, lastDaa). Ein Treffer ist darum eindeutig.
- `init` fehlt als Übergang. Das ist folgenlos, denn die Zustandsdatei entsteht erst nach `init` (ghostctl.rs:1173–1180, 1193ff.).
- Test E [B]: Das Nachführen gelingt über Ankündigung → Absage → `witness` und `settle` durch einen Dritten → neue Ankündigung → Preis → Aktivierung durch einen Dritten.

**Falsche REST-Antwort:** Sie kann zu keinem falschen Zustand führen [B, Quellstelle].
- `chain_register` (store.rs:243–260) übernimmt den Zustand nur, wenn `confirm` (store.rs:208–219) am Node eine **unverbrauchte** UTXO genau dieses Outpoints findet, mit dieser Covenant-ID und genau diesem P2SH-Skript.
- Das Skript enthält den Zustand als Hash-Urbild.
- Die Haupt-UTXO ist Singleton: Tickets haben `isTicket=true` und damit ein anderes Skript.
- Eine falsche Antwort endet also in einem Fehler. Dieser Fehler bricht allerdings `resync` und damit jeden Befehl ab (store.rs:361). Das gilt wie in v3 für das Orakel.

## 3. Relative Sperre im Simulator

`sim.rs:107–116` entspricht `check_sequence_lock` (tx_validation_in_utxo_context.rs:136–155) [B]:
- Der Test ist `sequence & SEQUENCE_LOCK_TIME_DISABLED == 0`. Das entspricht `!= DISABLED`, weil es ein einzelnes Bit ist.
- Die Maske ist `0xffffffff`.
- Gesperrt ist, solange `block_daa_score + rel − 1 ≥ DAA` gilt.

Test C prüft das genau an der Grenze: Alter = WAIT − 1 wird abgelehnt, Alter = WAIT angenommen.

Unterschiede ohne Wirkung:
- Der Konsens nimmt `pov_daa_score`, der Simulator `self.daa`.
- Die UTXO bekommt im Simulator den DAA vor dem `+10` je Tx.
- Die Sequenz ist vor dem Signieren gesetzt (txb.rs:206–210) und damit im Sighash enthalten.

## 4. `store::resync` und Register

| Fall | Verhalten [B] |
|---|---|
| `witness` durch Dritte | `follow` findet die UTXO mit gleichem Skript und gleicher Covenant (store.rs:192–204). Sonst greift das Nachführen über die Kette (Test E). Richtig. |
| Fremde Absage | Nachgeführt. Bei eigener `rotation` kommt der Hinweis „abgesagt oder überholt“ (store.rs:369–372). Richtig. |
| Fremdes Notfall-Ticket | `emerg=true` wird nachgeführt. Ist keine eigene `rotation` offen, kommt der Hinweis „ACHTUNG“ (store.rs:373–374). Das nächste eigene Preis-Update entwertet das Ticket (ops.rs:456). Richtig. Bei offener eigener Ankündigung fehlt der deutliche Notfall-Hinweis (Hinweis H-b). |
| Fremde **reguläre** Ankündigung | Kein Hinweis → **M-3** |
| Austausch aktiviert (fremder Rechner **oder Dritter**) | `resync` bricht hart ab → **H-2** |

---

## Befunde

### H-1 (hoch): Der Agent friert sein eigenes Orakel ein

**Was passiert [B, Quellstellen]:**
- Ein Preis-Update sendet der Agent nur bei mindestens 0,5 % Preisänderung oder bei einem Alter über `--max-age-min`. Der Standardwert ist **360 min** (ghostctl.rs:155, 174; Prüfung ghostctl.rs:2576–2578). `GHOST-Agent starten.command:51` setzt ebenfalls `--max-age-min 360`.
- Die Frist zum Einfrieren ist im echten v4 `2*HOUR_DAA`, also 2 h, in der Probe 1 h (ghostctl.rs:1133).
- Dieselbe Agent-Runde ruft nach `oracle_round` den `keeper_round` auf (ghostctl.rs:1401–1404). Der friert über `freeze_if_stale` jedes Orakel ein, das älter als die Frist ist (ghostctl.rs:1978–1980, 1989–2010).

**Folge:** Bei ruhigem Kurs (< 0,5 % Bewegung) ist das Orakel ab Stunde 2 eingefroren, und zwar durch den Betreiber selbst. Bis Stunde 6 gesperrt sind:
- `mint`, `redeem`, `liquidate`, `sweep` und `withdraw` bei offener Schuld;
- `swap` im Pool.

Das wiederholt sich jeden Zyklus. Liquidationen sind dabei ebenfalls blockiert, dazu kommt je Zyklus eine Gebühr fürs Einfrieren. `docs/v4-entwurf.md` Abschnitt 3 verlangt „Heartbeat ≤ 60 min“, umgesetzt ist das nicht.

**Vorschlag:**
- Den Standardwert von `max_age_min` aus `freeze_after_daa` ableiten, z. B. Frist/2.
- In `oracle_round` ein Update erzwingen, sobald `oracleDaa + freeze_after − Puffer` erreicht ist.
- `freeze_if_stale` erst nach einem gescheiterten eigenen Update auslösen, wenn eine Komitee-Datei vorhanden ist.

### H-2 (hoch): Nach jeder Aktivierung blockiert `resync` jeden ghostctl-Befehl

**Was passiert:** `resync` gibt `Err` zurück, sobald sich `set_hash` geändert hat (store.rs:364–368). Die Meldung verlangt, die Datei „des Rechners, der den Austausch aktiviert hat“ zu übernehmen. `load_synced` reicht den Fehler durch (ghostctl.rs:736). Damit scheitern **alle** Befehle, die abgleichen:
- auch `signers show`;
- auch `oracle-update` und der Agent;
- auch Vault-Aktionen gewöhnlicher Nutzer mit einer Kopie der Zustandsdatei.

**Warum das ein Problem ist:**
- `activate` ist erlaubnisfrei (signer_register_v4.sil:262ff.). Ein Dritter, der das fällige Ticket aktiviert, bekommt dessen 1 KAS abzüglich etwa 0,034 KAS Gebühr (Test H). Er hat also einen Anreiz, dem Betreiber zuvorzukommen.
- Die Datei des Betreibers kennt den neuen Satz bereits: `rotation.set` und `rotation.fallback`.
- Test E [B]: Nach der Aktivierung durch einen Dritten liefert `follow_register` genau `set_hash == mine.rotation.set.hash()`. `resync` würde trotzdem abbrechen.

**Folge:**
- Der Betreiber-Agent stoppt die Preis-Updates, das Orakel friert ein.
- Alle Nutzer können ghostctl nicht mehr benutzen, bis sie die Datei von Hand bearbeiten oder ersetzen. Einen „Rechner, der aktiviert hat“ gibt es dabei womöglich gar nicht.

**Vorschlag:**
- In `resync` den neuen `set_hash` mit `rotation.set.hash()` vergleichen (Notfallsatz analog). Bei Gleichheit `signer_set` und `fallback_set` übernehmen und `rotation` leeren.
- Für Nutzer ohne Komitee sollte ein unbekannter Satz nur eine Warnung sein, kein Fehler. Vault-Aktionen brauchen den Satz nicht.

### M-1 (mittel): Ein Zins außerhalb des Rasters blockiert die Preis-Updates

**Was passiert:**
- `math::rate_next` rundet den aktuellen Satz erst auf das 0,5-Raster und geht dann einen Schritt (math.rs:201–206).
- Der Vertrag verlangt `|Δ| ≤ rateStep` (price_oracle_v4.sil:86–87). Dabei gilt `rate_step = rate_from_apr(0,5)+1` (ghostctl.rs:1131).
- Liegt der Satz nicht auf dem Raster, ist ein Schritt größer als 0,5:
  - 3,3 % mit GHOST < 0,995 ergibt das Ziel 4,0 %, also Δ 0,7.
  - 3,2 % mit GHOST > 1,005 ergibt 2,5 %, ebenfalls Δ 0,7.
- `check_update` lehnt das ab (ops.rs:502–504). In `oracle_round` fährt der Zins beim Preis-Update mit (ghostctl.rs:2532–2540). Damit scheitert **auch der Preis**.
- Die Vormerkung wird zurückgenommen (`rate_release`), und in der nächsten Runde wiederholt sich alles. Das geht so, bis der GHOST-Kurs ins Band zurückkehrt.

**Wie es entsteht:** Ein solcher Satz entsteht legal:
- `oracle-update --rate 3.3` von 3,0 aus ist erlaubt, Test B prüft das mit `check_update(...).expect(...)`.
- `deploy --rate 3.3` ist ebenfalls möglich.

**Folge:** Keine Preis-Updates, nach 2 h ist das Orakel eingefroren (jeder darf das).

**Beleg:** Test B [B]. Gegenprobe: Auf dem Raster 0 bis 19,5 % ist jeder Schritt der Regel zulässig.

**Vorschlag:**
- `rate_next` begrenzt den Schritt auf `rate_step`.
- Oder `oracle_round` sendet bei Ablehnung des Zinses den Preis mit dem alten Zins.
- Zusätzlich `--rate` auf das Raster prüfen.

### M-2 (mittel): `deploy --rate` ohne Obergrenze sperrt das Orakel unwiderruflich

**Was passiert:**
- `deploy` prüft nur `rate ≥ 0` (ghostctl.rs:1081–1083), dann `stable_rate = rate_from_apr(rate)` (ghostctl.rs:1114). Die Genesis prüft den Zustand nicht.
- Bei einem Startzins über `maxRate + rateStep`, z. B. 25 %, ist kein `update` mehr möglich. Der Vertrag verlangt `newRate ≤ maxRate` **und** `|newRate − rate| ≤ rateStep`, beides zusammen ist unerfüllbar.

**Folge:**
- Das Orakel bekommt nie wieder einen Preis. Nach der Frist ist es dauerhaft eingefroren.
- Register- und Orakel-KAS sind gebunden.
- Es braucht ein neues Deployment.

**Beleg:** Test G [B]: Mit der Genesis bei 25 % scheitern alle Zinsen {0, max−step, max, 25 %}.

**Vorschlag:** In `deploy` die Grenze `0 ≤ rate ≤ RATE_MAX_PCT` setzen und den Startzins auf das Raster legen (siehe M-1).

### M-3 (mittel): Eine fremde reguläre Ankündigung bleibt stumm

**Was passiert:**
- Eine reguläre `propose` ergibt im Hauptzustand dasselbe wie `cancel`: nonce+1, `emerg=false` (Test F [B]).
- `resync` meldet nur den Notfallfall (store.rs:373). Das Ticket selbst verfolgt niemand: `resync` sucht keine Tickets, und `rotation` entsteht nur bei eigener Ankündigung (ops.rs:580).

**Folge:** Wer tRot Schlüssel hat, etwa durch einen kompromittierten Teil des Satzes, kann einen Austausch ankündigen. Der Betreiber sieht weder in `status` noch in `signers show` noch auf der Seite etwas davon. Die 14 Tage Wartezeit sollen gerade für eine Absage (`cancel`, t Schlüssel) reichen, laufen so aber unbemerkt ab. Nach der Aktivierung folgt H-2.

**Vorschlag:**
- Bei nonce+1 ohne eigene `rotation` die Register-Tx aus der Kette lesen. Payload = Ankündigung, Ausgang 1 = Ticket.
- Daraus `rotation` (Satz-Hash, Fallback-Hash) samt Ticket-Outpoint übernehmen und laut warnen.

### N-1 (niedrig): Tickets gehen aus der Zustandsdatei verloren

**Was passiert:**
- `propose` überschreibt `d.rotation` (ops.rs:580). ghostctl gibt nur einen Hinweis aus, wenn das alte Ticket noch gültig ist (ghostctl.rs:2694–2696). Danach steht der Outpoint des alten Tickets nirgends mehr in der Datei.
- `oracle_update` setzt `d.rotation = None`, sobald das Register `emerg` trägt (ops.rs:477–480). Das trifft auch eine eigene Notfall-Ankündigung.
- `signers clear` nimmt nur `d.rotation` (ghostctl.rs:2727–2729).

**Folge:** Die 1 KAS des Tickets (`TICKET_VALUE`, ops.rs:34) holt sich der erste Dritte per `clear_ticket`, denn jeder darf aufräumen. Der Betreiber könnte sie nur mit selbst gebautem Code zurückholen.

**Beleg:** Test D [B]: Ticket A liegt nach Ankündigung B noch auf der Kette, und sein Outpoint steht nicht im JSON. Aufräumen gelingt nur mit dem außerhalb gemerkten `Tracked`.

**Vorschlag:** Eine Liste `old_tickets` führen und mit `signers clear` alle abgelaufenen aufräumen.

### N-2 (niedrig): Zinstakt nach Uhrzeit, Vertrag nach DAA

- Die Zinsregel wartet `EVERY_SECS = 3600` Sekunden (rate.rs:44, 160–166).
- Der Vertrag verlangt `newOracleDaa ≥ lastRateDaa + rateGapDaa` mit 36 000 DAA (price_oracle_v4.sil:85), dazu `daa = Kettenstand − 20` (ghostctl.rs:2618).

Läuft die Kette in der Stunde etwas langsamer als 10 DAA/s, lehnt `check_update` ab (ops.rs:499–501). Mit dem Zins scheitert auch der Preis dieser Runde. Gemessen ist das nicht [V], ich erwarte Verzögerungen von Sekunden bis wenigen Minuten. Abhilfe: im Zinsrahmen nach `last_rate_daa` statt nach der Uhr entscheiden, oder ohne Zins weitersenden (wie M-1).

### N-3 (niedrig): Byte-Reihenfolge der scriptPubKey-Version

- `spk_bytes` und `spk_from_bytes` nutzen `to_le_bytes`/`from_le_bytes` (contracts.rs:471–483, Kommentar contracts.rs:415 „2 Byte LE“).
- `OpTxOutputSpk` liefert `version.to_be_bytes()` + Skript (rusty-kaspa `crypto/txscript/src/lib.rs:950–951`, opcodes/mod.rs:1332).

Für Version 0, die alle heutigen Ziele haben, sind beide gleich, deshalb ohne Wirkung [B, Quellstelle]. Bei Version ≠ 0 würde der Vault (stable_vault_v4.sil:181) jede `close`/`sweep`-Tx ablehnen. Vorschlag: auf `to_be_bytes` umstellen und die Version in `spk_from_bytes` auf 0 begrenzen.

### Hinweise

- **a) Grenze beim Nachführen:** `MAX_STEPS = 5 000` (chain.rs:679) gilt jetzt auch fürs Register. Bei stündlichen Updates reicht das für rund 7 Monate ohne Abgleich, danach ist die Datei dauerhaft „zu viele Schritte“. Für das Orakel war das in v3 schon genauso.
- **b) Notfall neben eigener Ankündigung:** Ist eine eigene `rotation` offen und kommt ein fremdes Notfall-Ticket hinzu, meldet `resync` nur „abgesagt oder überholt“ (store.rs:369–372), nicht „ACHTUNG Notfall“.
- **c) Genesis-Prüfung:** `check_register_genesis` prüft nur die selbst gebaute Tx (ops.rs:192). Die Gegenprüfung beim Start (Entwurf Abschnitt 9 Punkt 4: Orakel-Template = Hash im Register) ist nicht gebaut. Das ist für Dritte relevant, die einem Deployment vertrauen sollen.
- **d) v3-Zustandsdateien:** Sie laden mit dem v4-Binary nicht, weil `register_params` ohne serde-Default ist (ops.rs:98). `pool.rs` kompiliert jetzt immer `ghost_pool_v4.sil` (pool.rs:24). Ein v3-Binary und ein v4-Binary dürfen nicht dieselbe Datei teilen. Das ist so gewollt, sollte aber dokumentiert sein.
- **e) Ticket-Rückgabe an Dritte:** Dass der Ticket-Betrag an den geht, der aktiviert oder aufräumt (ops.rs:33), ist Vertragsdesign. Zusammen mit H-2 entsteht dadurch aber ein Anreiz, einen Fehlerzustand auszulösen.
- **f) Pool:** `frozen_stop` (pool.rs:96–101) spiegelt `stopWhenFrozen && frozen` (ghost_pool_v4.sil:149). `add` und `remove` bleiben frei. Ohne Befund.

## 6. Fehlende Tests

`v4_ops_tests.rs` deckt die Hauptwege ab. Es fehlen:

1. das Layout der Zustandsbytes gegen den Compiler mit Grenzwerten (jetzt Test A, vorher nur indirekt);
2. der Zins der Zinsregel gegen `check_update` mit Zins außerhalb des Rasters (M-1) und der Startzins über dem Rahmen (M-2);
3. das Verhalten von `resync`/`store` für das Register, also fremde Aktivierung, fremde Ankündigung und Notfall. Dafür gibt es gar keinen Test, weil `store` `Net` braucht; ein Mock fehlt.
4. Nachführen über `witness`/`clear_ticket` durch Dritte und über eine fremde Absage (jetzt Test E);
5. die relative Sperre genau an der Grenze (Test C; vorher nur ±20 DAA);
6. Mehrfach-Ankündigung und der Verbleib alter Tickets (Test D);
7. die Agent-Logik: Heartbeat gegen Einfrieren (H-1). `oracle_round` und `keeper_round` sind im Binary und nicht testbar.
8. `liquidate`, `redeem` und `sweep` bei eingefrorenem Orakel über ops. Nur `mint` und `withdraw` sind in `einfrieren_sperrt_und_preis_taut_auf` geprüft.

## Anhang: Testdatei (gelöscht)

`protocol/tests/audit14_probe.rs` enthielt die Tests a–h. Ergebnis: `test result: ok. 8 passed` mit `cargo test --release --offline --test audit14_probe`. Kernzeilen:

```rust
// B (M-1)
let s1 = OracleState { stable_rate: rate_from_apr(3.3), last_rate_daa: 1_040_000, oracle_daa: 1_040_000, ..s0 };
let next = math::rate_next(apr_of(s1.stable_rate), 0.98).unwrap();          // 4.0
let e = ops::check_update(&p, &s1, 4_000_000, 1_080_000, rate_from_apr(next)).err().unwrap();
assert!(e.contains("Zinsschritt zu groß"));
// E (H-2)
let (_, _, s) = chain::follow_register(&mut h, &sh, &mine.register).unwrap();
assert_ne!(s.set_hash, mine.register.state.set_hash);                       // resync: Err
assert_eq!(s.set_hash, mine.rotation.as_ref().unwrap().set.hash().to_vec()); // Datei kennt den Satz
// G (M-2): deploy_feed_with(..., rate_from_apr(25.0), (max, step, 36_000)) → jedes oracle_update Err
```

Die vollständige Kopie liegt im Scratchpad der Sitzung, nicht im Repository.
