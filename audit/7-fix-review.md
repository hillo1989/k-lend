# Audit 7 — Fix-Review Version 2 (Behebungen aus Audit 1, 3, 4, 5)

Stand: 28.09.2026, Prüfstand **HEAD `0bb9e19`** („Mutationstest v2 ausgewertet"), Vergleichsbasis Tag
`v1-mainnet`. Geprüft: `git diff v1-mainnet HEAD -- contracts protocol` (18 Dateien, +1431/−140),
`app/server/api.ts`, `app/server/actions.ts`, `app/src/lib/vaultMath.ts` nur zur Aufrufform und
Rechenparität; rusty-kaspa `a41a333` als Konsens-Referenz. Ein früherer Stand dieses Berichts
(Commits bis `506b03b`) ist durch `0bb9e19` teilweise überholt; dieser Bericht ersetzt ihn.

**Methode.** Repository nur lesend. Alle Läufe in Kopien unter `…/scratchpad/fixreview2/`
(`repo/` für Szenario-Tests, `mutA/` für Mutanten; `vendor/silverscript` als Symlink;
`CARGO_TARGET_DIR` im Scratch-Ordner, vorhandene Build-Verzeichnisse wiederverwendet, kein
zusätzlicher Plattenverbrauch). Bestehende Suite: **105 Tests grün** (e2e 1, factory 20, token 9,
oracle 25, math 5, vault 44, lib 1). Eigene Tests: `fixreview2_tests.rs` (Simulator/ops),
`fixreview2_vault_tests.rs` und `fixreview2_oracle_tests.rs` (Engine, Kopien der Suite-Harnische
plus Anhang), dazu die Szenarien des Vorlaufs (`fixreview_tests.rs`, `fixreview_value_tests.rs`)
erneut gegen HEAD. Keine Transaktion gesendet, `keys/` nicht gelesen, keine Netzverbindung
(öffentliche Nodes liefern 502).

Schweregrade wie Audit 4: **H** (Geldverlust), **M** (Betriebsausfall / Zustandsverlust / gebundene
Mittel), **N** (Fehlbedienung, Robustheit), **I** (Information). „UNVERIFIED" = nur hergeleitet.

---

## 0. Kurzfazit

- **Verträge:** V-01, V-02, V-04 (deposit), V-05 und O-2 sind geschlossen; ich habe keinen
  Umgehungsweg gefunden. Der im Vorlauf gemeldete DUST-Ausbuchungsfehler (N-3) ist mit `0bb9e19`
  behoben — gegen die Engine nachgeprüft (Abschnitt 2, N-3 → geschlossen). Teil-Liquidation
  gewinnt durch Rundung höchstens 1 sompi + 1 Einheit je Tx (gemessen: 37 sompi je Tx im
  ungünstigsten Fall gegen 4,8 Mio. sompi Gebühr).
- **Off-chain bleiben zwei ernste Fehler offen** (unverändert seit `c26f51c`): das Journal wertet
  „erster Input verbraucht" als „angenommen" und schreibt im Konfliktfall einen falschen Zustand,
  der jeden weiteren Aufruf inkl. Feed blockiert (**N-1, M**); die Dateisperre lässt sich bei
  verwaister Sperre doppelt nehmen (**N-2, M**, 5 von 12 Läufen an HEAD).
- **V-04 nur halb:** Fremd-`repay` mit 2 Einheiten (2·10⁻⁸ GHOST) bleibt gültig und setzt den
  Vault des Opfers zusätzlich auf `stale` (Sperre aller Aktionen bis zur Handreparatur) — **N-4, M**.
- **Vorbestehend, in Audit 1–6 nicht benannt:** kein Vertrag prüft den KAS-Wert der
  Minter-Fortsetzungen. Ein Dritter zieht 9,8 KAS aus dem Wurzel-Minter (`openVault`) und 2,8 KAS
  je Vault-Zweig (`repay` mit 2 Einheiten) — **N-11, M**, beides an HEAD im Simulator/Engine belegt.
- Die ×2/÷2-Regel ist **keine** Bremse gegen ein kompromittiertes Komitee: 0,04 → 900 USD in 15
  aufeinanderfolgenden Updates (Sim: 169 DAA ≈ 17 s). Die Doku-Aussage „kann nicht alles auf einmal
  kippen" gilt nur je Transaktion (**I-3**).

---

## 1. Prüfung der behaupteten Behebungen

| Befund | Behebung (Ort) | Urteil | Beleg |
|---|---|---|---|
| **V-01** negative Token-Beträge | `contracts/ghost_token.sil:55-59,72` `checkNonNegative(newStates)` im Leader-`transfer` | **geschlossen.** Jeder GHOST-Ausgang entsteht nur über einen Leader-`transfer` (Genesis nur über Factory-`init`, dort `amount: 0`). Alle Zustände ≥ 0, `ghostDelta` summiert nie Negatives. Im Vault zusätzlich `o.amount > 0` / `== 0` für Ausgänge (L170/L173). | `ghost_token_tests::negativer_ausgang_wird_abgelehnt`; Mutante L57 rot |
| **V-02** Teil-Liquidation, Bad-Debt | `stable_vault.sil:254-289`: `burn` als Argument, `burn ≤ debt`, `claim = burn·1,1`; Vault endet **nur** bei `seize == coll` (Wert ≤ Anspruch) oder bei `rest < DUST` **und** `burn == debt` | **geschlossen.** `seize == coll` erfordert `value ≤ 1,1·burn ≤ 1,1·debt`, also Deckung ≤ 110 % (+ ≤ 1 Einheit Rundung) — Ausbuchung nur bei tatsächlich erschöpfter Sicherheit. Über 110 % verbessert jede Teil-Liquidation die Quote ((V−1,1b)/(D−b) steigt, wenn V > 1,1D); unter 110 % verschlechtert sie sie, der `seize == coll`-Abschluss bleibt aber jederzeit möglich (`burn = ⌈value/1,1⌉ ≤ debt`). Kein Zustand, in dem keine gültige Liquidation existiert (`burn = debt` geht immer). | Tests A, S2, S3, E1 (Anhang); Mutanten L260/L261/L272/L277/L283 rot, L278 Lücke (N-12) |
| **V-04** deposit ohne Signatur | `stable_vault.sil:187-188` `deposit(sig s)`; `ops::deposit` `sig_at = (0, owner)` | **teilweise.** ABI-Position korrekt (Test S5: Fremder scheitert beim Bauen, Besitzer baut). `repay` bleibt signaturfrei → N-4. | `v2_einzahlen_nur_durch_besitzer`; Mutante L188 rot; Test S5 |
| **V-05** Tilgung ohne Anteilswirkung | `:235-239` `require(cut > 0)` | **geschlossen.** `cut < debtShares` ist garantiert (burned ≤ debt−1 < shares·index/1e9). | `v2_tilgung_ohne_anteilswirkung_scheitert`; Mutante L238 rot |
| **O-2** Preisgrenzen | `risk_oracle.sil:33-34,77-82`; `ghostctl::check_price_step` spiegelt | **geschlossen für den Vertrag.** Keine Überläufe (`newKasUsd·2 ≤ 1,8·10¹¹`; `OpMul` prüft i64). Grenzen konsistent mit den Vault-Rechnungen (`(claim % kasUsd)·1e8 ≤ 9·10¹⁸`). Halbieren an der Obergrenze, ×2 an 2³², Gleichstand: alle korrekt (Test E3). **Aber:** `initKasUsd` wird nicht geprüft (I-2), und die Regel bremst nur je Tx (I-3). Halbieren eines ungeraden Preises per Abrunden wird abgelehnt (`2·⌊p/2⌋ < p`); `check_price_step` verhält sich identisch, also kein Betriebsfehler. | `oracle_tests::v2_*`; Mutanten L77–82 rot; Tests S1, E3 |
| **O-1/F4** Resync nach fremdem `read()` | `store::resync/follow` | **geschlossen für den Normalfall** (gleiches Skript + Covenant-ID → Outpoint nachgeführt; Vault-Änderung → `stale`). Schwächen: N-1 (falscher Zustand → harter Fehler), N-4 (`stale` als Waffe), N-9. | Code |
| **O-3/F1** Journal | `store::write_pending/resolve_pending`, `Ctx::send_to` | **Kernfall abgedeckt**, aber **N-1**: Konfliktfall wird als Annahme gewertet. | Herleitung |
| **O-6** Plausibilitätsband | `feed_due(max_jump = 0,20)` | **umgesetzt**, ohne Stufenmodus (N-6). | Code |
| **F2** deploy wiederaufnehmbar | `DeployProgress`, `…deploy.json` | **umgesetzt**; N-8 (geänderte Parameter still ignoriert), N-10 (Reihenfolge Journal/Existenzprüfung). | Code |
| **F3** Dateisperre | `store::lock` | **umgesetzt, fehlerhaft** (N-2). | Test C an HEAD |
| **F5** JSON-Reinheit | `say!` → stderr bei `JSON_MODE`, Fehler als JSON | **teilweise.** `JSON_MODE` folgt nur dem globalen `--json`; die Seite ruft `status --json` (Unterkommando) → N-5. | `ghostctl.rs:21,453,602-607`, `api.ts:126` |
| **F6** Bereichsprüfung | `amount()`, `withdraw --keep` | **geschlossen.** | Code |
| **F7** Blocklimits | `txb::check_block_limits` 500 000/500 000/1 000 000 g, in `build` und `Sim::submit` | **korrekt** (`params.rs` `prior_block_mass_limits`, `new_transient_mass_limit`). | rusty-kaspa |
| **F8** Wechselgeld-Spende | `Built.donated = paid − fee`, Hinweis vor dem Senden | **umgesetzt**; `paid ≥ fee` ist durch `assemble` garantiert. | Code |
| **F9/F10** Netzprüfung, Schlüsseldatei 0600 | `Ctx::load`, `write_new` | **geschlossen.** | Code |
| **F11** Annahmeprüfung | `store::wait_accepted`: irgendein Ausgang sichtbar; Mempool-Nachfrage bis 600 s | **verbessert.** Restrisiko N-7 (`in_mempool` mappt RPC-Fehler auf `false`). | Code |
| **F12** > 2 Token-UTXOs | `consolidate()` | **geschlossen.** Schleife terminiert (je Runde −2 UTXOs); Teilausfall hinterlässt gespeicherten Zwischenstand (jeder Schritt ist eine eigene Sendung mit Journal). | Code |
| net.rs Wiederholungen | 3 Versuche, 2/6/10 s Pause | **umgesetzt** (worst case ≈ 63 s bis zum Fehler, unter dem 180-s-Rundenlimit). | Code |
| sim.rs Locktime, 0-Ausgänge | `lock_time != 0 && lock_time >= daa` → Fehler; 0-sompi-Ausgang → Fehler | **korrekt** (`check_tx_is_finalized`: `lock_time < daa`). | rusty-kaspa `tx_validation_in_header_context.rs` |
| Orakel-Nachricht `script_num8` | `contracts.rs:107-114` Betrag LE + Vorzeichenbit | **korrekt:** `OpNum2Bin` → `serialize_i64(num, Some(8))` (Betrag, `0x80` im letzten Byte bei negativ) — identisch. Für die Praxis irrelevant (Vertrag verlangt `rate ≥ 0`), aber der Test L86 prüft jetzt wirklich die Regel. | `data_stack.rs:153-185` |
| **V-03** Zins 0 | `deploy --rate` Standard 0; Skript `--rate 0` | **Umgehung, so dokumentiert.** Mit Index 1,0 ist die V-05-Mindestwirkung 1 Einheit (N-4 noch billiger). `MAINNET.md:74` zeigt weiterhin `--rate 5` (I-1). | Doku |

---

## 2. Befunde an HEAD

### N-1 (M) — Journal: „erster Input verbraucht" ≠ „angenommen"; Konfliktfall schreibt falschen Zustand und blockiert alles

**Ort:** `protocol/src/store.rs:125-136` (`resolve_pending`), `:166-172` (`resync` → harter Fehler),
`ghostctl.rs:350` (`first_input = entries[0]`: bei `oracle_update` das Orakel, bei allen Vault-Aktionen
der Vault, bei `deploy` eine P2PK-UTXO). Unverändert seit `c26f51c`.

**Logik:** Ist kein Ausgang der Journal-Tx sichtbar und der erste Input nicht mehr vorhanden, gilt die Tx
als angenommen und `next` wird geschrieben. Der Input kann aber von einer **fremden** Tx verbraucht sein —
und das ist der typische Grund, warum die eigene Tx nicht angenommen wurde.

**Wann das Journal liegen bleibt:** nicht beim direkten Node-Reject (`send_to` löscht es, wenn die Tx nicht im
Mempool ist), sondern wenn `submit` gelang und die Tx danach verdrängt wurde (Konkurrenz-Tx an einem
anderen öffentlichen Node zuerst in einem Block) → `wait_accepted` gibt nach 120 s auf (nicht mehr im Mempool)
oder das 180-s-Rundenlimit bricht die Runde ab. Zweiter Weg: N-7 (RPC-Fehler in `in_mempool`).

1. **Orakel-Feed** baut `update` auf O₁; gleichzeitig gibt eine Vault-Aktion eines anderen Nutzers O₁ per
   `read()` aus (Seite und Feed nutzen verschiedene Nodes des Resolvers). Nächste Runde: Ausgänge unsichtbar,
   O₁ verbraucht → „war angenommen – Zustand übernommen": Datei enthält Preis, `seq+1` und Outpoint einer nie
   akzeptierten Tx. `resync` sucht das Orakel unter dem Skript dieses Fantasiezustands → **„Orakel-UTXO nicht
   auffindbar … Bitte Zustandsdatei prüfen"** bei jedem weiteren Aufruf (`load_synced`), also `status`, `mint`,
   `repay`, `liquidate` und jede Feed-Runde. Kein Selbstheilen, das Journal ist gelöscht. `status --json` fällt
   auf `load()` zurück und zeigt `freshError`, alle Aktionen sind blockiert.
2. **Vault-Aktion gegen fremde Tilgung/Liquidation:** Auflösung markiert den Vault `stale` (richtig), entfernt
   aber bei `repay`/`liquidate` die eigenen Token-UTXOs aus der Datei, obwohl sie auf der Kette liegen; `resync`
   holt nur bekannte Einträge nach → GHOST unsichtbar bis zur Handreparatur.
3. **`deploy`:** wird die Funding-UTXO anderweitig ausgegeben (zweite Instanz, Wallet), gilt eine abgelehnte
   Genesis als angenommen; das Deployment liefe mit einer nicht existierenden Covenant-ID weiter.
4. Verschärfung durch O-4 (offen): nach „verworfen" signiert die nächste Runde dieselbe `seq` neu; ein Dritter
   kann das alte Sigscript einreichen (Audit 3, Beweis E) → Fall 1.

**Beleg:** Herleitung aus dem Code (`Net` nicht attrappierbar, Nodes 502). Die Entscheidung steht wörtlich in
`store.rs:134`: `// Input verbraucht … → angenommen`. UNVERIFIED im Netz, im Code eindeutig.

**Empfehlung:** „angenommen" nur bei sichtbarem Ausgang oder Beleg über
`get_virtual_chain_from_block(include_accepted_transaction_ids)`. Sonst: irgendein Input unverbraucht →
verworfen; alle Inputs verbraucht und kein Ausgang sichtbar → **nicht entscheiden**, Journal behalten, Meldung mit
TXID. `resync` sollte bei „Orakel nicht auffindbar" die Adresse aller bekannten Zustände (letzter bestätigter
und Journal-`next`) absuchen statt abzubrechen.

### N-2 (M) — Dateisperre: verwaiste Sperre wird von zwei Aufrufern gleichzeitig übernommen

**Ort:** `store.rs:52-58`. Erkennung `pid_alive` (Spawn von `kill -0`) → `remove_file` → `continue` →
`create_new` ist nicht atomar. Zwei Wartende lesen denselben toten PID, A löscht und legt seine Sperre an, B
löscht A's Sperre und legt seine an; A's `Drop` löscht anschließend B's Datei.

**Beleg an HEAD (Test C):** `Doppelte Sperre in 5 von 12 Läufen`, beide Aufrufer nach ≈ 650 ms mit `Ok`.
Gegenprobe `lock_lebende_sperre_blockiert` grün. Weitere Schwächen: `kill -0` liefert bei Prozessen anderer
Nutzer EPERM → „tot"; leere Datei im Fenster zwischen `create_new` und `write` gilt nach 500 ms als verwaist;
`Drop` prüft nicht, ob die Datei noch die eigene ist.

**Empfehlung:** `flock(2)` auf eine dauerhafte Datei, keine PID-Heuristik; oder verwaiste Sperre nur per
`rename` auf eindeutigen Namen „stehlen" und dann `create_new`.

### N-3 — DUST-Pfad mit Teilverbrennung: **geschlossen mit `0bb9e19`**

Vorlauf-Befund: kleiner Vault (0,5 KAS) bei 140 % Deckung, Teilverbrennung so gewählt, dass `rest < DUST`,
Vault endet, 23,6 % der Schuld ausgebucht. HEAD verlangt im DUST-Zweig `burn == debt`
(`stable_vault.sil:277`). **Beleg:** Test B des Vorlaufs schlägt an HEAD schon in `ops::liquidate` fehl
(„Der Rest (0.19999957 KAS) wäre kleiner als 0,2 KAS – dann muss die ganze Schuld … verbrannt werden");
Test E1 gegen die Engine: Vault-Ende **und** Fortsetzung mit `rest < DUST` werden abgelehnt, volle Tilgung
geht durch. Mutante L277 rot (`kleiner_rest_nur_bei_voller_tilgung`). `vaultMath.ts:220-247` und
`liquidationPreview` rechnen identisch.

### N-4 (M) — V-04 nur halb: Fremd-`repay` mit 2 Einheiten bleibt gültig und sperrt den Vault des Opfers

**Ort:** `stable_vault.sil:227-244`, `store.rs:190-201` (`stale`), `ghostctl.rs:409-415` (`usable_vault`).

**Beleg an HEAD (Test D, Simulator):** Fremder tilgt 2 Einheiten (bei Index 1,05; bei Zins 0 reicht 1 Einheit):
alle Inputs gültig, Anteile 14 285 714 286 → 14 285 714 285, Gebühr 4 795 980 sompi. Kosten je Konflikt-Tx
≈ 0,048 KAS — wie das `+1 sompi`-`deposit` aus Audit 1 (0,037 KAS). Zusätzlich neu in v2: `resync` findet den
Vault unter dem alten Skript nicht mehr → `stale` → **alle** Besitzeraktionen (`mint`, `repay`, `withdraw`,
`close`) sind gesperrt, bis jemand `debtShares` von Hand in die Zustandsdatei schreibt (`resync` liest keinen
Vault-Zustand aus der Kette). Ein Angreifer sperrt damit für 0,05 KAS je Vault die Bedienung aller Vaults,
die ihm bekannt sind (Covenant-IDs stehen in `status --json`).

**Empfehlung:** Fremd-`repay` an eine gebührendominierende Mindestwirkung binden (z. B. `burned ≥ 0,01 GHOST`)
oder signieren; unabhängig davon `resync` befähigen, `debtShares` aus dem Redeem-Skript der gefundenen
Vault-UTXO zu lesen (Skript = Template + Zustand; Adresse folgt aus Owner + Kandidaten-Shares nur durch Suche —
praktikabel über die Vault-Ausgänge der Tx, die den alten Outpoint verbraucht hat).

### N-5 (N) — `status --json` (Aufruf der Lending-Seite) ist nicht rein

**Ort:** `ghostctl.rs:21,26-30,453` (`JSON_MODE` nur aus dem globalen `--json`), `:602-607`
(`Cmd::Status { json: true }` → `status_json` → `load_synced` → `say!("Hinweis: …")` → **stdout**),
`app/server/api.ts:126` (`["--network", n, "status", "--json"]`; `keys` dagegen mit globalem `--json`).

Sobald `resolve_pending` etwas übernimmt oder `resync` etwas nachführt (fremdes `read()`, Fremdtilgung,
entfernte Token), steht vor dem JSON eine Hinweiszeile. `parseJson` (`api.ts:64-80`) scheitert am
Gesamttext, der Fallback nimmt die letzte mit `{` beginnende Zeile — bei Pretty-JSON eine innere Zeile →
`null` → „ghostctl status fehlgeschlagen", 20 s gecacht. Selbstheilend beim nächsten Aufruf, aber genau nach
jeder Fremdaktion sichtbar. Ebenso ungleich: bei einem Fehler vor dem JSON (`Net::connect`) liefert
`status --json` gar kein JSON (Fehlerpfad in `main` prüft nur das globale Flag).

**Empfehlung:** `JSON_MODE` auch bei `Status { json: true }` setzen (vor `run`), oder die Seite `--json status`
aufrufen lassen.

### N-6 (N) — Feed friert bei Preissprung > 20 % ein; kein Stufenmodus

**Ort:** `ghostctl.rs:929-942`. Die 20-%-Grenze liegt **vor** der Altersprüfung und liefert `Err`: keine Runde
sendet, solange der Median mehr als 20 % vom On-chain-Preis abweicht. Nach einem Absturz von 25 % zwischen
zwei Runden (Feed über Nacht offline, Node-Störung) steht der Preis, bis ein Mensch `oracle-update --usd`
fährt; unterdessen wird zum alten Preis geprägt und nicht liquidiert (O-5-Wirkung). Die Vertragsgrenze
×2/÷2 wäre kein Hindernis.

**Empfehlung:** Preis stufenweise annähern (je Runde ±20 % Richtung Median) oder nach zwei bestätigenden
Runden senden; Alarm statt stiller Fehlerzeile.

### N-7 (N) — `in_mempool` wertet RPC-Fehler als „nicht im Mempool"

**Ort:** `net.rs:118-120`; Nutzer `store.rs:128,224`, `ghostctl.rs:355`. Bei Verbindungsabbruch liefert
`get_mempool_entry` einen Fehler → `false` → `send_to` löscht das Journal nach einem `submit`-Fehler, obwohl
die Tx unterwegs sein kann; `wait_accepted` gibt nach 120 s auf; `resolve_pending` verwirft eine laufende Tx.
Der F1-Fall (angenommen, nicht gespeichert) bleibt über diesen Weg erreichbar; `resync` heilt Orakel-Reads,
nicht eigene Zustandsänderungen (`mint` → Vault `stale`, geprägter Token unbekannt).

**Empfehlung:** `in_mempool` → `Result<bool>`; bei Fehler Journal behalten.

### N-8 (N) — `deploy`-Fortsetzung ignoriert geänderte Parameter still

**Ort:** `ghostctl.rs:552-568`. Beim Fortsetzen stammen Komitee, Startpreis, Zins und Deployer aus
`…deploy.json`; anderes `--committee`/`--rate` wird ohne Hinweis verworfen. Ein anderer `--key` scheitert
erst in `init_factory` am `checkSig(deployer)` mit Engine-Fehler.

### N-9 (I) — `resync::follow` bei mehreren identischen Token-UTXOs

**Ort:** `store.rs:155-161`. Zwei Token-UTXOs mit gleichem Besitzer, Betrag und Covenant-ID haben dasselbe
Skript; verschwindet der verfolgte Outpoint, findet `follow` zwei Kandidaten → `false` → Token wird
„entfernt", obwohl er existiert. Nur nach Bewegung außerhalb der Datei (zweite Instanz, N-1 Fall 2).

### N-10 (N) — `deploy`: Journal-Auflösung erst nach der Existenzprüfung

**Ort:** `ghostctl.rs:537-551`. Bricht `deploy` nach dem Senden von „Factory-Init + GHOST-Genesis" ab
(Journal-Ziel = Zustandsdatei), prüft der nächste `deploy` zuerst `state_path.exists()` (nein), löst dann das
Journal auf (schreibt die Zustandsdatei) und baut **trotzdem** `init_factory` erneut auf den verbrauchten
Factory-Outpoint → Node-Reject als Fehlermeldung, Fortschrittsdatei bleibt liegen. Zustand ist korrekt, die
Meldung irreführend („Node lehnt ab"). Reihenfolge tauschen: erst `resolve_pending`, dann Existenzprüfung.

### N-11 (M, vorbestehend, in Audit 1–6 nicht benannt) — KAS-Wert der Minter-Fortsetzungen ungeprüft

**Ort:** `stable_vault.sil:140-179` (`ghostDelta`: `validateOutputStateWithInputTemplate` prüft Skript und
Zustand, nie `tx.outputs[..].value`), `vault_factory.sil:118-119` (Wurzel-Fortsetzung ebenso),
`ghost_token.sil` (KCC20 prüft keine Werte). Orakel, Factory und Vault schützen ihren eigenen Betrag
(`continuation`/`continueWith`), die GHOST-Minter-UTXOs schützt niemand.

**Beleg an HEAD:**
- Test E (Simulator, vollständige Tx mit Massen und Gebühr): `openVault` durch einen Dritten mit
  Wurzel-Minter-Fortsetzung 0,2 KAS — `Wurzel-Minter 10.00 → 0.20 KAS; Dritter erhält netto 9.8000 KAS`.
- Test E2 (Engine): Fremd-`repay`, Fortsetzung des Vault-Minter-Zweigs mit halbem Wert — alle vier Inputs
  `Ok`. Im Mainnet-Maßstab: 3 KAS → 0,2 KAS, 2,8 KAS je Vault an den Angreifer (kombinierbar mit N-4:
  dieselbe 2-Einheiten-Tx).

Folgen: kein Einfluss auf Schuld oder Umlauf; Vault bleibt bedienbar (Ein- und Ausgang gleich klein, KIP-9
neutral). Die in `MAINNET.md` als „dauerhaft gebunden" beschriebenen 30 KAS + 3 KAS je Vault sind für
Fremde entnehmbar. Untergrenze ≈ 0,02 KAS (Speichermasse-Limit), darunter baut die Angreifer-Tx nicht.

**Empfehlung:** In `ghostDelta` `require(tx.outputs[OpCovOutputIdx(ghostCovId,0)].value >=
tx.inputs[OpCovInputIdx(ghostCovId,0)].value)`, in `openVault` analog für die Wurzel-Fortsetzung und ein
Mindestwert für den neuen Zweig. Template-Änderung → vor dem Neu-Deployment.

### I-1 — Doku

`MAINNET.md:74` zeigt `deploy … --rate 5`, das Skript `GHOST-Mainnet-Test.command` und `ARCHITEKTUR.md`
sagen 0 %. `AUDIT.md` verweist auf diesen Bericht als „Gegenprüfung", die Tabelle nennt F3 und F1/O-3 als
„behoben" — nach N-1/N-2 nur teilweise.

### I-2 — `deploy` prüft den Startpreis nicht gegen die Vertragsgrenzen

`initKasUsd` ist Konstruktorparameter; `update` verlangt später `MIN ≤ newKasUsd ≤ MAX` und ×2/÷2. Ein
Startwert außerhalb 0,00001–900 USD (Quellenfehler, falsche Einheit) ergäbe ein Orakel ohne gültiges erstes
Update. `median_price` mit 3-%-Band macht das unwahrscheinlich; `check_price_step` sollte trotzdem beim
Deploy laufen.

### I-3 — ×2/÷2 je Update ist keine Zeitbremse

`risk_oracle.sil:79-82` bindet nur an das vorherige Update; zwischen zwei Updates muss nur `newOracleDaa`
steigen (1 DAA ≈ 0,1 s) und die Tx final sein. **Beleg (Test S1):** 0,04 → 900 USD in 15 Updates,
DAA-Spanne 169 im Simulator; zurück auf 0,00001 USD in 27 Updates. Ein Komitee mit 3 Schlüsseln setzt also
in Sekunden jeden Preis in [0,00001; 900]. Die wirksame Schranke ist allein MAX/MIN (schützt vor
`NumberTooBig`, nicht vor Manipulation). Die Formulierung in `risk_oracle.sil:79-80` und `AUDIT.md`
(„innerhalb der Grenzen ×2/÷2 je Update") sollte das sagen; eine echte Bremse bräuchte
`newOracleDaa ≥ oracleDaa + MIN_DELTA` (Kosten: Feed-Reaktionszeit) — Design-Entscheidung.

### Geprüft und ohne Befund

- **Rundung Teil-Liquidation:** Test A (25 × 0,01 GHOST) `extra 0 sompi`; Test S2 (ungünstigster Fall
  `burn = 2 Einheiten`, `claim = ⌈2,2⌉ = 3`): 10 Tx → 370 sompi Rundungsgewinn gegen 47,96 Mio. sompi
  Gebühr; Schuldabbau ≤ burn (floor zugunsten des Protokolls).
- **Unter 110 % Deckung** (Test S3, 100 %): Teil-Liquidation 20 GHOST → Deckung 98,88 %; Abschluss mit
  `seize == coll`; insgesamt verbrannt 18 090 909 090 Einheiten bei Wert/1,1 = 18 090 909 091 — der
  Liquidator zahlt nicht weniger als Wert/1,1; Ausbuchung = debt − Wert/1,1 (Design).
- **`seize == coll` bei Wert > Anspruch** durch Aufrunden von `seize` (nur bei Preis > 1 USD möglich):
  Ausbuchung höchstens (p/1e8 + 1)/1,1 Einheiten ≈ 8·10⁻⁶ GHOST bei 900 USD — vernachlässigbar.
- **Selbstliquidation des Besitzers** über 110 % gewinnt nur aus der eigenen Sicherheit, die Quote steigt;
  unter 110 % ist der Bonus für jeden Liquidator gleich (Design des Write-off).
- **Schleifengrenze `nOut`** (kein `require(nOut ≤ 2)` in `ghostDelta`): Audit 2 belegt, dass
  `for(j,0,nOut,2)` bei `nOut > 2` scheitert; `mint` verlangt zusätzlich `== 2`.
- **ghost_token `amount >= 0` und Minter-Zweige:** Faktory erzwingt `amount == 0` für Wurzel und Zweig,
  Vault für seine Fortsetzung; ein negativer Eingang kann nicht entstehen.
- **`deposit`-ABI:** einziges Argument `sig`, Position 0 wie `close`; Test S5.
- **`consolidate()`:** je Runde 3 → 1 UTXO, Abbruch bei `len ≤ 2` oder `top2 ≥ need`; `total < need` →
  klare Meldung; Dry-Run bricht erklärend ab.
- **`check_block_limits`, `donated`, 0-Ausgänge, Locktime-Regel:** korrekt (Tabelle).
- **`script_num8` vs. `as byte[8]`:** identisch (Tabelle).

---

## 3. Mutationslauf an HEAD (Kopie `mutA/`, `protocol/mutation/mutate.sh`)

Jede `require`-Zeile einzeln durch `require(true)` ersetzt; Aufrufe wie im Skript dokumentiert
(`stable_vault.sil vault_tests vault_math_tests`, `risk_oracle.sil oracle_tests`,
`ghost_token.sil ghost_token_tests e2e_tests`). Vollständige Ausgabe in
`…/scratchpad/fixreview2/mut-{vault,oracle,token}.txt`; Kopie danach byte-identisch mit dem Original.

**Ergebnis:** `stable_vault.sil` 48 Mutanten → **32 rot / 16 Lücken**; `risk_oracle.sil` 15 → **12 rot /
3 Lücken**; `ghost_token.sil` 7 → **7 rot / 0 Lücken**. Kein Mutant mit Build-Fehler. Gegenüber dem Vorlauf
(30/16, 10/5, 4/3) sind die in `0bb9e19` angekündigten Tests wirksam: L27, L30, L64 (Token), L40, L86
(Orakel), L132, L145, L261, L277 (Vault) sind jetzt rot.

**Jede neue v2-Regel hat einen Test, der beim Rückbau rot wird:**

| Fix | Zeile (HEAD) | rot durch |
|---|---|---|
| V-01 | `ghost_token.sil:57` `amount >= 0` | `negativer_ausgang_wird_abgelehnt` |
| V-02 | `stable_vault.sil:260` `burn <= debt` | `v2_liquidation_bei_unterdeckung_lohnt_sich_und_bucht_aus` |
| V-02 | `:261` `−ghostDelta == burn` | `liquidation_mit_falschem_burn_argument_scheitert` |
| V-02 | `:272` Vault-Ende ohne Fortsetzung (`seize == coll`) | `totalliquidation_darf_die_covenant_id_nicht_weiterleben_lassen` |
| V-02 | `:277` `burn == debt` im DUST-Zweig | `kleiner_rest_nur_bei_voller_tilgung` |
| V-02 | `:283` `cut > 0` (Teil-Liquidation) | `v2_teil_liquidation_ohne_schuldabbau_scheitert` |
| V-04 | `:188` `checkSig` in `deposit` | `v2_einzahlen_nur_durch_besitzer` |
| V-05 | `:238` `cut > 0` (repay) | `v2_tilgung_ohne_anteilswirkung_scheitert` |
| O-2 | `risk_oracle.sil:77,78` Absolutgrenzen | `v2_absolute_preisgrenzen` |
| O-2 | `:81,82` ×2/÷2 | `v2_preis_hoechstens_verdoppeln_oder_halbieren` |

### N-12 (N) — Testlücke mit Sicherheitsgewicht: `stable_vault.sil:278`

`require(OpAuthOutputCount(this.activeInputIndex) == 0)` im **DUST-Zweig** ist ungetestet (LUECKE), während
dieselbe Regel im `seize == coll`-Zweig (L272) einen Test hat. Ohne L278 könnte ein Liquidator bei
`burn == debt` und `rest < DUST` eine Fortsetzung mit der Vault-Covenant-ID und **beliebigem Skript** anhängen
(`validateOutputState` wird dort nicht aufgerufen). Diese Fortsetzung autorisiert danach den Minter-Zweig
(`ghost_token.sil:32` prüft nur `OpCovInputCount(vaultId) > 0`), ohne dass ein Vault-Skript die
GHOST-Gruppe prüft — das ist die „Hintertür" aus Test-Audit 5, hier über einen zweiten Pfad. Die Regel ist
korrekt, der Test fehlt: `totalliquidation_darf_die_covenant_id_nicht_weiterleben_lassen` für den DUST-Fall
duplizieren (kleiner Vault, `burn == debt`, Rest < 0,2 KAS, Fortsetzung mit OpTrue-Skript → muss rot sein).

**Übrige Lücken (vorbestehend, Regeln korrekt, ungetestet):** `stable_vault.sil` L116 (`newShares >= 0`,
rechnerisch ausgeschlossen), L127 (`kasUsd > 0`, durch O-2 gedeckt), L133 (`noGhost`-Ausgangsseite),
L143/L144/L146 (Gruppengrößen), L154–L158 (Eingangsprüfungen der GHOST-Gruppe; Audit 2 belegt L154–156 in
`audit/audit_supply.rs`, nicht in der Suite), L173 (`o.amount > 0`), L199 (`withdraw` mit `newColl ≥ coll`),
L217 (durch `ghostDelta` gedeckt), L229/L259 (`burned/burn > 0`, durch `cut > 0` indirekt gedeckt).
`risk_oracle.sil` L47/L48/L55 (Quorum-Längen, teils durch Schleifengrenze gedeckt).

---

## 4. Priorisierte Empfehlungen

1. **N-1** Journal-Auflösung: kein „angenommen" ohne Beleg; `resync` bei fehlendem Orakel nicht hart
   abbrechen. Der Punkt stoppt den v2-Betrieb, sobald ein zweiter Teilnehmer das Orakel liest, während der
   Feed sendet.
2. **N-2** Sperre auf `flock` umstellen.
3. **N-4** Fremd-`repay` mit Mindestwirkung oder Signatur; `resync` soll Vault-Zustände aus der Kette lesen.
4. **N-11** Werte der Minter-Fortsetzungen im Vertrag prüfen — vor dem Neu-Deployment (Template-Änderung).
5. **N-12** Test für L278 (DUST-Zweig ohne Fortsetzung) ergänzen — billig, sichert eine kritische Regel.
6. **N-5, N-6, N-7, N-8, N-10, I-1, I-2, I-3** Betriebs- und Dokupunkte.

---

## Anhang — eigene Tests (nur in der Kopie, nicht im Repository)

Quelle: `…/scratchpad/fixreview2/repo/protocol/tests/fixreview2_tests.rs` (S1–S5, Simulator),
`fixreview2_vault_tests.rs` (E1, E2; Kopie von `vault_tests.rs` + Anhang), `fixreview2_oracle_tests.rs`
(E3), dazu `fixreview_tests.rs` (A–D) und `fixreview_value_tests.rs` (E) aus dem Vorlauf, `price()` an die
neue Simulator-Locktime-Regel angepasst (`daa − 1`). Ausgaben wörtlich:

**S1 — `orakel_x2_kette_erreicht_die_grenzen_in_wenigen_updates`** (I-3)
```
0,04 → 900 USD in 15 Updates, DAA-Spanne 169 (Sim: 10 DAA je Tx ≈ 1 s)
900 → 0,00001 USD in 27 weiteren Updates
```
(Gegenprobe im Test: 900 USD + 1 Einheit wird abgelehnt.)

**S2 — `rundungsgewinn_bei_zwei_einheiten_je_tx`** (V-02 Rundung)
```
10 × burn 2 Einheiten: erhalten 1370 sompi, fair 1000 sompi, Rundungsgewinn 370 sompi gesamt;
Gebühren 47961900 sompi; Anteile getilgt 10 (exakt 19)
```

**S3 — `teil_liquidation_unter_110_prozent_dann_ausbuchung`**
```
Deckung 100.00 %
nach Teil-Liquidation: Deckung 98.88 % (Schuld 17900000002, Wert 17699999999)
verbrannt 18090909090 Einheiten (Wert/1,1 = 18090909091), erhalten 1000000000000 sompi
(Sicherheit 1000000000000), ausgebucht 1809090911
```

**S4 — `dust_pfad_mit_teilverbrennung_wird_jetzt_abgelehnt`** (N-3 geschlossen, ops-Ebene)
```
ops::liquidate → Some("Der Rest (0.19999957 KAS) wäre kleiner als 0,2 KAS – dann muss die ganze
Schuld (0.00999999 GHOST) verbrannt werden")
```
Vorlauf-Test B (`dust_pfad_bucht_schuld_aus_obwohl_deckung_ueber_110_prozent`) schlägt an HEAD mit
derselben Meldung fehl — erwartet.

**S5 — `deposit_durch_fremden_scheitert`**: Fremder → `Err` beim Bauen, Besitzer → `Ok`.

**E1 — `audit7_dust_pfad_teilverbrennung_bei_140_prozent_abgelehnt`** (Engine)
```
140 %: burn 761729 < debt 997500, rest 19999928 < DUST → beide Varianten abgelehnt; voll: ok
```

**E2 — `audit7_minter_zweig_wert_bei_repay_ungeprueft`** (N-11, Engine)
```
Minter-Zweig 1000 → 500 sompi bei Fremd-repay: [Ok(()), Ok(()), Ok(()), Ok(())]
```

**E3 — `audit7_halbieren_an_der_obergrenze`** (O-2 Randwerte)
```
9e10 → 9e10: Ok   9e10 → 4.5e10: Ok   9e10 → 4.5e10+1: Ok   6e10 → 3e10: Ok
2^32 → 2^31: Ok   2^32 → 2^31+1: Ok   1e9 → 5e8: Ok   1e9 → 2e9: Ok   1e9 → 2e9+1: Err(VerifyError)
```

**A — `teil_liquidation_schleife_rundungsgewinn_begrenzt`** (Vorlauf, an HEAD)
```
seized 1250000000 sompi, ideal 1250000000, extra 0 sompi über 25 Tx (Schranke je Tx 47);
Anteile getilgt 23809500, exakt 23809523
```

**C — `lock_verwaiste_sperre_zwei_aufrufer`** (N-2, an HEAD)
```
Lauf 0: [(true, 663.341944ms), (true, 657.796148ms)] → gleichzeitig gehalten: true
…
Doppelte Sperre in 5 von 12 Läufen
```

**D — `fremdes_repay_mit_zwei_einheiten_geht_durch`** (N-4, an HEAD)
```
kleinste wirksame Tilgung bei Index 1050000000: 2 Einheiten
Fremd-Repay 2 Einheiten: Gebühr 4795980 sompi, Anteile 14285714286 → 14285714285
```

**E — `dritter_kann_kas_aus_dem_wurzel_minter_abziehen`** (N-11, an HEAD)
```
Wurzel-Minter 10.00 → 0.20 KAS; Dritter erhält netto 9.8000 KAS aus dem Minter (Gebühr 0.0463 KAS,
storage 219356 g)
```
