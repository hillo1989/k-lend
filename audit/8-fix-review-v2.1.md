# Audit 8 — Fix-Review Version 2.1 (Behebungen aus Audit 7)

Stand: 28.09.2026, Prüfstand **HEAD `29a8cd8`** („Version 2.1: Befunde des Fix-Reviews behoben"),
Vergleichsbasis `0bb9e19`. Geprüft: `git diff 0bb9e19 29a8cd8 -- contracts protocol app/src app/server`
(23 Dateien, +328/−95), dazu `app/server/api.ts`, `app/server/actions.ts` und `app/src/lib/status.ts`
zur Aufrufform; rusty-kaspa `a41a333` und workflow-rpc 0.18.0 als Referenz für Konsens und RPC-Fehlertexte.

**Methode.** Repository nur lesend. Arbeitskopie `git archive 29a8cd8` unter
`…/scratchpad/fixreview2/`, `vendor/silverscript` als Symlink, `CARGO_TARGET_DIR` im Scratch-Ordner
(2,6 GB, am Ende gelöscht). Vorher `df -h /`: 32 GB frei. Bestehende Suite: **111 Tests grün**
(lib 2 inkl. Sperrtest, e2e 1, factory 21, token 9, oracle 26, math 5, vault 47); App: **116 Vitest grün**.
Eigene Tests: `fixreview3_oracle_tests.rs` (Engine, 4), `fixreview3_vault_tests.rs` (Engine, 7),
`fixreview3_sim_tests.rs` (Simulator/ops, 4) — alle 15 grün, Ausgaben im Anhang. Mutationstest mit
`protocol/mutation/mutate.sh` für alle drei geänderten Verträge (Abschnitt 3). Keine Transaktion
gesendet, `keys/` nicht gelesen, keine Netzverbindung.

**Hinweis zum Prüfstand.** Während der Prüfung lief im Repository weitere Entwicklung (Commits nach `29a8cd8`, uncommittete Änderungen in `app/`, neue Dateien `contracts/ghost_pool.sil`, `protocol/tests/pool_tests.rs`). Dieser Bericht bezieht sich ausschließlich auf `29a8cd8`; die Arbeitskopie stammt aus `git archive 29a8cd8` und wurde durch die spätere Entwicklung nicht berührt.

Schweregrade: **H** (Geldverlust), **M** (Betriebsausfall / Zustandsverlust / gebundene Mittel),
**N** (Robustheit, Fehlbedienung), **I** (Information). „UNVERIFIED" = nur hergeleitet.

---

## 0. Kurzfazit

- **Alle Vertragsbehebungen (N-4, N-11, N-12, I-3) greifen wie beschrieben.** `repay` verlangt die
  Besitzersignatur; Teil- und Volltilgung, Liquidation durch Dritte und `close` funktionieren
  unverändert (Engine und Simulator, Anhang V1–V7, S1–S4). Die Wertprüfung der Minter-Fortsetzungen
  bindet Ein- und Ausgang exakt (Gleichheit, nicht ≥); in keinem Pfad (`mint`, `repay`, `liquidate`,
  `openVault`) ist Gleichheit unerwünscht, und ein anderer Covenant-Ausgang an Position 0 der
  GHOST-Gruppe wird durch die anschließende Zustandsprüfung ausgeschlossen. Kein neuer Weg zu
  Geldverlust gefunden.
- **Die 600-DAA-Regel ist keine Zeitbremse gegen ein kompromittiertes Komitee (NEU-1, N).**
  Der Abstand wird nur zum *gespeicherten* `oracleDaa` gemessen, und `newOracleDaa` darf beliebig
  weit hinter der Kette liegen. Der „Slack" (Kettenzeit seit dem letzten Update — beim Feed mit
  Standardwerten regelmäßig Stunden) lässt sich in einem Rutsch verbrauchen: 0,04 → 900 USD in
  15 Updates, alle mit derselben Locktime (Engine) bzw. in 150 Ketten-DAA (Simulator). Die Sätze in
  `AUDIT.md` („höchstens ein Update pro Minute", „dauert mindestens 15 Minuten") sind falsch.
- **Off-chain: N-1, N-2, N-5, N-7, N-8, N-10, I-2 sind geschlossen**, N-6 umgesetzt. Der neue
  Journal-Entscheid über das eigene Wechselgeld ist korrekt, hat aber einen Stopp-Fall ohne
  Selbstheilung (NEU-4, N). Die `in_mempool`-Erkennung hängt am Fehlertext des Nodes (NEU-5, N,
  für a41a333 verifiziert).
- **Keine neue Regression in den reparierten Stellen gefunden.** Die Lücken, die bleiben, sind
  Doku-Widersprüche (NEU-2, NEU-3, NEU-6) und Restrisiken, die schon Audit 7 als Design benannte.

---

## 1. Prüfung je Befund

| Befund | Behebung (Ort) | Urteil | Beleg |
|---|---|---|---|
| **N-1** Journal wertet „erster Input verbraucht" als angenommen | `store.rs:109-166`: Reihenfolge 1) Ausgang sichtbar → angenommen, 2) im Mempool → `Err` warten, 3) erster Input unverbraucht → verworfen, 4) sonst Entscheid über `change_output` (`Some` → verworfen, `None` → `Err` „Unklar"); `txb.rs:196-203,243` liefert `change_index` | **geschlossen.** Der Konfliktfall aus Audit 7 (Orakel-`read()` eines Dritten verdrängt das Feed-Update) landet in Schritt 4 mit `Some` → verworfen; danach findet `resync::follow` die `read()`-Fortsetzung unter demselben Skript. Das Wechselgeld geht an P2PK des Gebührenzahlers; jeder Aufruf, der es ausgeben könnte, löst zuerst das Journal auf (Sperre + `load_synced`). Der Deploy-Pfad ruft `resolve_pending` jetzt vor der Existenzprüfung (N-10). Rest: NEU-4 (kein Wechselgeld → Stopp), Ausbreitungsfenster bei Prozessabbruch direkt nach `submit` (vorbestehend, siehe „ohne Befund"). | Simulator-Test S1–S4 prüft je Tx, dass `change_index` auf den letzten Ausgang mit P2PK des Zahlers zeigt und kein Covenant ist; Code |
| **N-2** Dateisperre doppelt übernehmbar | `store.rs:23-50` `File::try_lock` (flock), keine PID-Heuristik, Datei bleibt bestehen | **geschlossen.** flock ist an die offene Dateibeschreibung gebunden und endet mit dem Prozess; es gibt nichts mehr aufzuräumen. `Lock` hält die Datei bis zum Drop; im Feed wird der Rundenfuture nach 180 s verworfen → Sperre frei. | `store::tests::sperre_ist_exklusiv_und_wird_freigegeben` grün (zweite Beschreibung im selben Prozess blockiert, nach Drop frei) |
| **N-4** Fremd-`repay` mit 2 Einheiten sperrt den Vault | `stable_vault.sil:233-234` `repay(int, GhostState[], sig s)` + `checkSig(s, pubkey(owner))`; `ops.rs:390-392` `sig_at = (2, payer)`; `ghostctl.rs:420-426` `owned_vault`; `precheck.ts:79` | **geschlossen.** ABI-Position 2 (nach `oracleIdx`, `outStates`) stimmt; die Signatur wird über Input 0 (Vault) gerechnet, die Token-Delegates über Input 3/4 — keine Kollision. Fremdsignatur scheitert **nur** am Vault-Input, alle übrigen Inputs bleiben gültig (V3). Besitzer kann mit 2 Token-Inputs und Wechselgeld-Token teilweise (V1) und voll (V2) tilgen; Überzahlung um 1 Einheit wird weiter abgelehnt. Liquidation durch Dritte ohne Signatur geht weiter (V6, S2), `close` danach (S1). Kooperativ erlaubt: Besitzer signiert, ein Dritter liefert die GHOST (V4) — kein Fehler. `ghostctl` weist Fremdschlüssel schon vor der Simulation ab; Simulator-Bau scheitert mit `Input 0: VerifyError` (S1). | V1–V4, V6, S1, S2; Suite `v21_repay_nur_durch_besitzer`; Mutante L234 rot |
| **N-11** KAS-Wert der Minter-Fortsetzungen | `stable_vault.sil:151` in `ghostDelta`: `tx.outputs[OpCovOutputIdx(ghostCovId,0)].value == tx.inputs[templateIdx].value`; `vault_factory.sil:120` analog mit `rootIn` | **geschlossen.** `templateIdx = OpCovInputIdx(ghostCovId,0)` ist der erste GHOST-Input der Tx; die Schleife verlangt für `i == 0` `isMinter`, `identifierType == COV`, `ownerIdentifier == myCov` → das ist der eigene Zweig. `OpCovOutputIdx(ghostCovId,0)` ist der erste GHOST-Ausgang; `j == 0` verlangt Minter-Zustand mit `amount == 0` und `validateOutputStateWithInputTemplate` → ein Angreifer kann dort keinen anderen Ausgang platzieren (der Ausgang müsste zugleich `outStates[0]` entsprechen). Pfade: `mint`/`repay`/`liquidate` bauen `cov_out(branch, v.branch.value, …)` (`ops.rs:343,408`), `openVault` `cov_out(rart, root.value, …)` (`ops.rs:224`) → Gleichheit immer erfüllbar; KIP-9-neutral, da Ein- und Ausgang gleich groß. Beim Vault-Ende (`seize == coll`) bleibt der Zweig ebenso gebunden (S2 „Rest-Liquidation"). Gleichheit ist strikt: +1 sompi wird ebenfalls abgelehnt (V5) — gewollt, sonst könnte niemand mehr abschöpfen, aber auch niemand „auffüllen"; kein Betriebsfall braucht das. Nicht umgesetzt: Mindestwert für den *neuen* Zweig in `openVault` (Audit-7-Empfehlung) — ohne Auswirkung, weil `ghostctl` nur selbst eröffnete Vaults verfolgt und der Zweig nur mit Vault-Signatur bewegbar ist. | V5, V7 (mint 999/1001 sompi), S3/S4 (Wurzel- und Zweigwert nach Tx unverändert); Suite `v21_minter_zweig_behaelt_seine_kas`, `wurzel_minter_behaelt_seine_kas`; Mutanten L151/L120 (Abschnitt 3) |
| **N-12** Testlücke L278 (DUST-Zweig ohne Fortsetzung) | `vault_tests.rs` `v21_kleiner_rest_ohne_weiterleben_der_covenant_id` | **geschlossen** — Mutante L285 (neue Zeilennummer) rot, siehe Abschnitt 3. | Mutationslauf |
| **N-5** `status --json` nicht rein | `ghostctl.rs:465-466` `pure_stdout = json \|\| Status{json:true}` → `JSON_MODE` | **geschlossen für Hinweise.** Alle `say!` gehen nach stderr. Fehler *vor* dem JSON (z. B. `Net::connect`) gehen weiter nur nach stderr (`main` prüft das globale Flag) — `api.ts:118-124` fängt das ab (`parseJson` → null → `errorBody(stderr-Tail)`, `isNodeError` → `nodeDown`). Kein Betriebsproblem. | Code |
| **N-6** Feed friert bei Sprung > 20 % | `ghostctl.rs:699-728,976-992`: `feed_due` liefert `big`, Zähler `streak` (3 gleichgerichtete Runden), dann Schritt `price.clamp(⌈old/2⌉, 2·old)` | **umgesetzt.** Rechnung: `(old+1)/2` = ⌈old/2⌉ erfüllt `new·2 ≥ old`; `clamp` kann nicht panicken (min ≤ max für old ≥ 0). Zeitverhalten: ein Sturz um 75 % braucht 2 Schritte à 3 Runden = 6 Runden ≈ 30 min bei `--interval 300`; solange gilt der alte Preis (O-5-Wirkung, Design). Streak wird nach erfolgreichem Senden und bei „kein Update" zurückgesetzt, nach „zu frisch"-Fehler nicht (korrekt: nächste Runde sendet). | Code; NEU-3 (Doku) |
| **N-7** `in_mempool` wertet RPC-Fehler als „nicht im Mempool" | `net.rs:120-132` → `Result<bool>`; `ghostctl.rs:355` löscht Journal nur bei `Ok(false)`; `store.rs:246` wartet bei `Err` weiter; `store.rs:125` `?` | **geschlossen für rusty-kaspa a41a333.** Server: `RpcError::TransactionNotFound` → Display `"Transaction {id} not found"` (`rpc/core/src/error.rs:63`, `rpc/service/src/service.rs:602-604`); Transport: `ServerError::Text(e.to_string())` (`rpc/macros/src/wrpc/server.rs:58`) → Client `RpcError::RpcSubsystem(String)` (`rpc/macros/src/wrpc/client.rs:74`), Text bleibt erhalten → `contains("not found")` trifft. Restrisiko NEU-5. | rusty-kaspa/workflow-rpc Quellen |
| **N-8** `deploy`-Fortsetzung mit anderen Parametern | `ghostctl.rs:587-589` Komitee und Deployer verglichen | **geschlossen** für Schlüssel; `--rate` wird still aus der Fortschrittsdatei genommen (kommentiert). | Code |
| **N-10** Journal-Auflösung erst nach Existenzprüfung | `ghostctl.rs:550-555` getauscht | **geschlossen.** Nach Übernahme eines angenommenen „Factory-Init" folgt jetzt „existiert schon – Deployment läuft bereits" (Text irreführend, Zustand richtig; `…deploy.json` bleibt liegen) — NEU-6. | Code |
| **I-1** `MAINNET.md --rate 5` | Doku | geschlossen (kein `--rate 5` mehr gefunden). | grep |
| **I-2** Startpreis ungeprüft | `ghostctl.rs:572-574` `1_000..=90_000_000_000` | **geschlossen.** | Code |
| **I-3** ×2/÷2 keine Zeitbremse | `risk_oracle.sil:33,87` `newOracleDaa >= oracleDaa + 600`; `ghostctl.rs:1043` spiegelt; `precheck.ts:180-184`; `e2e_tests.rs:92` `advance(700)`; `sim.rs:46` | **nur teilweise — NEU-1.** Die Regel bremst genau dann, wenn `oracleDaa` eng an der Kette liegt. `tx.daa >= newOracleDaa` (CLTV, `TUTORIAL.md:522`) verbietet nur die Zukunft; die Vergangenheit ist frei. Feed nicht blockiert: `oracle_update_price` setzt `newOracleDaa = Ketten-DAA − 20` und verlangt `≥ oracleDaa + 600` (O1, O4: nach 599 DAA rot, nach 600 grün; Sim S3 „599 DAA: Input 0: VerifyError"). `read()` verbraucht keinen Abstand (O3). | O1–O4, S3 |
| **N-9** | offen (so dokumentiert) | unverändert. | — |

**Geprüft und ohne Befund**
- **Indizes in `burn_op`:** Inputs [Vault 0, Orakel 1, Zweig 2, Token 3…, Gebühren]; `sig_at` ist eine ABI-Position im Argument-Array, kein Input-Index. Simulator S1 bestätigt `entries[0..3]` = Vault-, Orakel-, Zweig-Covenant.
- **Compute-Budget:** die zusätzliche `checkSig` (1000 g Sig-Op) wird von `txb::build` in der Budgetrunde erfasst; Simulator nimmt alle Tilgungen an.
- **`ghostDelta` Reihenfolge:** die Wertprüfung steht vor der Schleife, `nOut ≥ 1` ist vorher gesichert (`OpCovOutputIdx` außerhalb des Bereichs scheitert ohnehin).
- **Zweigwert durch Dritte veränderbar?** Nein: jede Tx, die den Zweig ausgibt, braucht den Vault als Input (`ghost_token.sil:32`), und alle Vault-Einträge außer `liquidate` verlangen die Signatur; `liquidate` erzwingt Gleichheit.
- **flock im Feed:** `store::lock` innerhalb des 180-s-Futures; `std::thread::sleep(300 ms)` blockiert den Worker kurz (vorbestehend, harmlos).
- **Journal-Ausbreitungsfenster (vorbestehend):** wird `ghostctl` direkt nach `submit` beendet und der nächste Aufruf trifft binnen Sekunden einen Node ohne die Tx, entscheidet Schritt 3 „nicht angenommen" — F1-Fall. Nur bei Prozessabbruch, da `wait_accepted` sonst ≥ 120 s wartet. I.
- **Oracle-`update` mit gleichem Preis:** Abstand gilt ebenso (O4).
- **`txb::assemble` `change_index`** zeigt stets auf den zuletzt angehängten Ausgang; bei `rest < MIN_CHANGE` `None` (S1–S4 protokollieren keinen solchen Fall).

---

## 2. Neue Befunde

### NEU-1 (N) — 600-DAA-Mindestabstand bremst nur bei eng nachgeführtem `oracleDaa`; „Slack" erlaubt Update-Salven

**Ort:** `risk_oracle.sil:87-89` (`newOracleDaa >= oracleDaa + 600`, `tx.daa >= newOracleDaa`), `AUDIT.md:58,64`, `ARCHITEKTUR.md:188`.

**Logik:** Der Vertrag kennt keine Kettenzeit, nur Locktime-Untergrenzen. `newOracleDaa` muss ≤ Locktime < Block-DAA sein, darf aber beliebig weit **hinter** der Kette liegen. Der Mindestabstand bezieht sich auf das zuletzt *gespeicherte* `oracleDaa`. Ist die Kette seit dem letzten Update um S DAA weitergelaufen, sind ⌊S/600⌋ Updates sofort hintereinander gültig — alle mit derselben Locktime, also im selben Block. Beim Feed mit Standardwerten (`--min-change 0.01`, `--max-age-min 360`) entsteht bei ruhigem Markt Slack von Stunden.

**Beleg:**
- O1 (Engine): `START.oracle_daa + 100 000` als Locktime, 15 Updates mit `newOracleDaa += 600`: `0,04 → 900 USD in 15 Updates, alle mit Locktime 1100000 (derselbe Block möglich); verbrauchter Slack 9000 von 100000 DAA`.
- S3 (Simulator, echte Tx mit Gebühr, Massen, Locktime-Regel): `Rallye 0,041 → 900 USD: 15 Updates, Kette rückte dabei nur 150 DAA vor (Sim: 10 je Tx); Orakel-DAA liegt 92590 DAA hinter der Kette`.
- Gegenprobe O2: liegt `newOracleDaa` über der Locktime, scheitert CLTV — die Regel wirkt, sobald kein Slack da ist.

**Auswirkung:** Gegen ein Komitee mit 3 Schlüsseln (die genannte Bedrohung von I-3) ist die Bremse nur so stark wie die Frequenz des ehrlichen Feeds. Die Aussagen „höchstens ein Update pro Minute" und „0,04 → 900 USD dauert mindestens 15 Minuten" in `AUDIT.md` sind falsch; sie geben Betreibern ein falsches Bild. Kein Geldverlust über den bekannten Fall (kompromittiertes Komitee) hinaus.

**Empfehlung:** (a) Doku korrigieren: „mindestens 600 DAA *Orakelzeit* je Update; die Kettenzeit zwischen Updates ist nur begrenzt, soweit `oracleDaa` der Kette folgt". (b) Wenn eine echte Zeitbremse gewollt ist: Feed als Heartbeat mit `--max-age-min` ≈ 5–10 (Slack ≤ Intervall, Kosten ≈ 0,05 KAS je Update), oder im Vertrag ein relativer Lock `this.ageDaa >= 600` für `update` — Achtung: `read()` erzeugt die UTXO neu und setzt das Alter zurück, damit könnten Dauer-Reads den Feed blockieren (verschärft O-1). Design-Entscheidung.

### NEU-2 (I) — Vorprüfung „1 Minute" ≠ `ghostctl` „620 DAA"

**Ort:** `precheck.ts:180` (`ageMinutes < 1`), `ghostctl.rs:1042-1043` (`daa − 20 ≥ oracle_daa + 600`, also Alter ≥ 620 DAA ≈ 1,03 min), `ghostctl.rs:911` (`daa … unwrap_or(0)`).

Zwischen 600 und 620 DAA Alter meldet die Seite nichts, `ghostctl` lehnt mit „zu frisch" ab (klare Meldung, selbstheilend nach 2 s). Scheitert die DAA-Abfrage in `status_json`, ist `daa = 0`, `ageMinutes` stark negativ und die Seite meldet fälschlich „mindestens 600 DAA Abstand". Kosmetik.

### NEU-3 (I) — Doku-Widersprüche zum Feed

`AUDIT.md:34` (O-6: „Sprünge über 20 % sendet er nicht automatisch") und `ARCHITEKTUR.md:174` stehen gegen `AUDIT.md:51` (N-6: nach 3 Runden schrittweise) und den Code. `precheck.ts:197` sagt weiterhin „Der Dauerbetrieb sendet Sprünge über 20 % nicht automatisch".

### NEU-4 (N) — Journal ohne Wechselgeld-Ausgang hält alles an, bis jemand die Datei löscht

**Ort:** `store.rs:148-155` (Schritt 4, `change_output == None` → `Err`), aufgerufen aus `load_synced` (jeder Befehl, jede Feed-Runde, `status --json`).

Tritt ein, wenn eine Tx **ohne** Wechselgeld (`rest < MIN_CHANGE`, `txb.rs:197-203`; typisch `send` mit fast dem ganzen Guthaben, oder ein knapp finanzierter Schlüssel) verdrängt wurde und kein Ausgang sichtbar ist. Dann liefert jeder Aufruf „Unklar, ob … angenommen … Bitte im Explorer prüfen und danach … löschen" — auch der Feed steht („Prüfung fehlgeschlagen"), bis ein Mensch eingreift. Die Entscheidung ist sicher (kein falscher Zustand), aber ohne Selbstheilung. Für Journale aus Version 2.0 (Feld fehlt, `#[serde(default)]`) gilt dasselbe.

**Empfehlung:** In diesem Fall die Kette selbst befragen (`get_virtual_chain_from_block(include_accepted_transaction_ids)` ab dem DAA der Sendung), oder als Fallback nach z. B. 30 min ohne Mempool-Eintrag „verworfen" annehmen und beim nächsten `resync` prüfen; mindestens im Feed die Runde nicht dauerhaft blockieren lassen.

### NEU-5 (N, teils UNVERIFIED) — `in_mempool` erkennt „nicht vorhanden" nur am Fehlertext

**Ort:** `net.rs:124-129`. Der Client bekommt nur `RpcError::RpcSubsystem(String)`; die Unterscheidung „nicht gefunden" / „Node-Fehler" läuft über `contains("not found")`. Für rusty-kaspa a41a333 verifiziert (siehe N-7). Liefert ein Node (andere Version, anderer Wortlaut, Übersetzung, Proxy) einen abweichenden Text, wird jede fehlende Tx zum `Err`: `submit`-Fehler lassen das Journal stehen (`ghostctl.rs:355`), und `resolve_pending` scheitert danach bei jedem Aufruf in Schritt 2 (`store.rs:125` `?`) — Stillstand wie NEU-4, bis ein Node „not found" sagt. Über den öffentlichen Resolver ist die Node-Version nicht wählbar. UNVERIFIED für andere Versionen; im Code eindeutig.

**Empfehlung:** Bei `Err` in Schritt 2 nicht hart abbrechen, sondern mit Schritt 3/4 weiterentscheiden (erster Input unverbraucht → verworfen ist auch dann sicher); nur bei „Input verbraucht" den Fehler melden.

### NEU-6 (I) — Deploy nach Journal-Übernahme: irreführende Meldung, Fortschrittsdatei bleibt

`ghostctl.rs:550-555`: nach „Letzte Transaktion (Factory-Init …) war angenommen – Zustand übernommen" folgt „… existiert schon – Deployment läuft bereits" und `…deploy.json` wird nie gelöscht. Zustand korrekt.

### NEU-7 (I) — Sperrdatei nicht in `.gitignore`

`deployments/<netz>.lock` bleibt dauerhaft liegen (gewollt), `.gitignore` kennt nur `keys/`, `*.json.tmp`, `deployments/*skripttest*`. Die Datei ist leer, aber sie erscheint bei `git status`. Außerdem: `atomic_write` schreibt `<name>.tmp`, nicht `.json.tmp` — das Muster in `.gitignore` passt nicht.

### Vorbestehend, in v2.1 nicht adressiert (zur Vollständigkeit)

- **Audit-7-Empfehlung „`resync` liest `debtShares` aus der Kette"** nicht umgesetzt: nach einer legitimen Fremd-Liquidation bleibt der Vault `stale` und gesperrt bis zur Handreparatur (F4-Design, N).
- **Mindestwert des neuen Zweigs in `openVault`** nicht umgesetzt (ohne Auswirkung, s. o.).
- **N-9** offen.

---

## 3. Mutationslauf an HEAD (Kopie, `protocol/mutation/mutate.sh`)

Jede `require`-Zeile einzeln durch `require(true)` ersetzt; Aufrufe wie im Skript dokumentiert
(`stable_vault.sil vault_tests vault_math_tests`, `risk_oracle.sil oracle_tests`,
`vault_factory.sil factory_tests e2e_tests`). Vollständige Ausgabe in `…/scratchpad/fixreview2/mut-v21.txt`.
Verträge danach byte-identisch mit `29a8cd8` (Diff-Kontrolle am Ende des Laufs).

**Ergebnis:** `stable_vault.sil` 50 Mutanten → **34 rot / 16 Lücken**; `risk_oracle.sil` 15 → **12 rot / 3 Lücken**;
`vault_factory.sil` 26 → **16 rot / 10 Lücken**. Kein Mutant mit Build-Fehler. Laufzeit 51 min (≈ 45 s je Mutante).

**Jede v2.1-Regel hat einen Test, der beim Rückbau rot wird:**

| Fix | Zeile (HEAD) | rot durch |
|---|---|---|
| N-11 Vault | `stable_vault.sil:151` Zweigwert Ein = Aus | `v21_minter_zweig_behaelt_seine_kas` |
| N-4 | `:234` `checkSig` in `repay` | `v21_repay_nur_durch_besitzer` |
| N-12 | `:285` DUST-Zweig ohne Fortsetzung | `v21_kleiner_rest_ohne_weiterleben_der_covenant_id` (Audit 7: LUECKE) |
| N-11 Factory | `vault_factory.sil:120` Wurzelwert Ein = Aus | `wurzel_minter_behaelt_seine_kas` |
| I-3 | `risk_oracle.sil:87` Mindestabstand | `daa_muss_steigen`, `v21_mindestabstand_zwischen_updates` |

**Lücken (alle vorbestehend, keine neue):**
- Vault (16): L116, L127, L133, L143–L146, L157–L161, L176, L202, L220, L236, L266 — identisch mit der Liste aus Audit 7 (Zeilennummern +7 durch die neue Regel), dort als „Regel korrekt, ungetestet" begründet.
- Orakel (3): L48, L49, L56 — wie Audit 7.
- Factory (10): L58 (Genesis-ID ≠ 0), L61 (`n ≤ MAX_OUTPUTS`), L88 (`initialized`), L98 (`OpCovInputCount(ghostCovId) == 1`), L101–L104 (Wurzel-Eingang: Minter, Typ, Besitzer, Betrag 0), L106/L107 (Ausgangsanzahl). Die Factory war in Audit 7 nicht im Mutationslauf. Die Eingangsprüfungen L98/L101–L104 sind durch den Token-Vertrag gedeckt (ein Nicht-Minter-Leader hält die Menge und darf keine Minter-Ausgänge erzeugen, `ghost_token.sil:39-51,61-67`; ein fremder Minter kann nur mit *seinem* Vault als Input bewegt werden), L88 durch `ghostCovId = 0` (kein Input trägt diese ID). L58/L61/L106/L107 sind Schleifen- und Grenzprüfungen. Empfehlung: Tests nachziehen, analog zu Audit 7 (N). Kein Sicherheitsgewicht wie N-12 erkennbar.

---

## 4. Empfehlungen (priorisiert)

1. **NEU-1**: `AUDIT.md`/`ARCHITEKTUR.md` korrigieren; entscheiden, ob ein Heartbeat-Feed (kleines `--max-age-min`) oder ein relativer Lock die gewünschte Bremse ist.
2. **NEU-5**: `resolve_pending` bei Mempool-Fehler nicht hart abbrechen; Fehlertext-Abhängigkeit dokumentieren.
3. **NEU-4**: Stopp-Fall ohne Wechselgeld mit Kettenabfrage oder Zeitfallback lösen; Feed darf nicht dauerhaft stehen.
4. **NEU-2, NEU-3, NEU-6, NEU-7**: Texte und Kleinigkeiten.
5. Vorbestehend: `resync` mit Vault-Zustand aus der Kette (Audit 7), Testnetz-Lauf für 2.1 (die Seite sagt selbst, dass er aussteht).

---

## Anhang — eigene Tests (nur in der Kopie, nicht im Repository)

Quelle: `…/scratchpad/fixreview2/protocol/tests/fixreview3_{oracle,vault,sim}_tests.rs`
(Orakel- und Vault-Datei = Kopie der Suite-Harnische plus Anhang). Ausgaben wörtlich.

**Orakel (Engine)**
- O1 `fr3_slack_erlaubt_preisrallye_ohne_wartezeit`: `0,04 → 900 USD in 15 Updates, alle mit Locktime 1100000 (derselbe Block möglich); verbrauchter Slack 9000 von 100000 DAA` — ok
- O2 `fr3_ohne_slack_greift_der_mindestabstand`: Locktime 599 → Err, 600 → Ok — ok
- O3 `fr3_read_verbraucht_keinen_abstand` — ok
- O4 `fr3_gap_gilt_auch_bei_gleichem_preis`: +1 DAA Err, +600 Ok — ok

**Vault (Engine)**
- V1 `fr3_repay_teilweise_zwei_tokens_mit_wechselgeld` (30 aus 20+15, Besitzer) — alle Inputs Ok
- V2 `fr3_repay_voll_zwei_tokens` (105 aus 100+5,00000001; +1 Einheit → Vault Err) — ok
- V3 `fr3_repay_fremde_signatur_scheitert_nur_am_vault` — Input 0 Err, Inputs 1–4 Ok
- V4 `fr3_repay_besitzer_signiert_fremde_token_zulaessig` — alle Ok
- V5 `fr3_minter_zweig_darf_auch_nicht_mehr_kas_bekommen` (300 000 001 / 299 999 999 sompi) — Vault Err
- V6 `fr3_liquidate_durch_dritten_ohne_signatur_weiterhin_moeglich` (voll und Teil 10 GHOST) — alle Ok
- V7 `fr3_mint_mit_veraendertem_zweigwert_scheitert` (999/1001 sompi) — Vault Err

**Simulator (ops/txb/sim, vollständige Tx mit Gebühr und Massen)**
- S1 `fr3_sim_repay_nur_besitzer_voll_und_teilweise_close_danach`: `Fremd-repay: Input 0: VerifyError`, Teiltilgung 30, `Restschuld 70.00000000 GHOST`, Volltilgung, `close`; `change_index` je Tx geprüft — ok
- S2 `fr3_sim_liquidation_durch_dritten_bleibt_moeglich`: zwei ÷2-Updates (je +700 DAA), Teil-Liquidation 50 GHOST, Rest-Liquidation; Zweigwert 3 KAS unverändert — ok
- S3 `fr3_sim_orakel_mindestabstand_und_slack`: `599 DAA: Input 0: VerifyError`; `Rallye 0,041 → 900 USD: 15 Updates, Kette rückte dabei nur 150 DAA vor (Sim: 10 je Tx); Orakel-DAA liegt 92590 DAA hinter der Kette` — ok
- S4 `fr3_sim_open_vault_wurzel_wert_bleibt` — ok

**Bestehende Suite:** `cargo test` 111 grün; `npx vitest run` 116 grün (6 Dateien).
