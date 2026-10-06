# Audit 4 — Off-chain-Software und Betriebssicherheit (ghostctl, protocol/src)

Stand: 28.09.2026. Unabhängige Prüfung, nur lesend. Geprüft gegen rusty-kaspa a41a333
(`~/.cargo/git/checkouts/rusty-kaspa-410e06d1fde91a92/a41a333`), Zeilenangaben beziehen sich darauf.

## 0. Umfang und Methode

- Gelesen: `ARCHITEKTUR.md`, `MAINNET.md`, `TESTNET_LOG.md`, `protocol/src/{txb,ops,net,sim,contracts,price,math}.rs`,
  `protocol/src/bin/ghostctl.rs`, die drei `.command`-Starter, `./ghostctl`, `app/server/{api,actions}.ts`,
  die drei Verträge, KCC20 aus SilverScript v1.0.0.
- Verglichen mit: `mining/src/mempool/check_transaction_standard.rs`, `check_transaction_limits.rs`,
  `consensus/core/src/mass/{mod,units}.rs`, `consensus/core/src/hashing/{sighash,tx,covenant_id}.rs`,
  `consensus/src/processes/transaction_validator/*`, `crypto/txscript/src/{lib,covenants}.rs`,
  `consensus/src/pipeline/virtual_processor/utxo_validation.rs`.
- Ausgeführt (nur lesend, nie gesendet): `./ghostctl --network testnet-10 status`, `--json keys`,
  `--json status`, `--json --dry-run mint`, `--json --dry-run withdraw --keep 5000`,
  `--network testnet-10 --state deployments/mainnet.json status`.
- Beweisprogramme in einer Kopie des Crates im Scratch-Verzeichnis
  (`…/scratchpad/audit-offchain/proto/examples/audit_probe.rs`, `audit_tokens.rs`), gegen die
  Simulationskette `sim.rs` und die Mempool-Funktionen aus rusty-kaspa. Ausgaben stehen bei den Befunden.
- Nicht gemacht: keine Mainnet-Aktion außer `status` (hier gar nicht nötig), kein Lesen von `keys/*.json`,
  keine Änderung an Repository-Dateien. Hinweis: Weil `.cargo/config.toml` das Zielverzeichnis mit dem
  SilverScript-Workspace teilt, hat der Probelauf `target/debug/ghostctl` neu gebaut; ein anschließendes
  `cargo build --bin ghostctl` im Projekt bestätigte, dass die Binärdatei zu den Projektquellen passt.

Schweregrade: **H** hoch (Geldverlust möglich), **M** mittel (Betriebsausfall, gebundene Mittel, Zustandsverlust),
**N** niedrig (Fehlbedienung, Robustheit), **I** Information.

## 1. Was stimmt (verifiziert)

| Thema | Befund | Beleg |
|---|---|---|
| Mindestgebühr | `min_fee = 100 · max(compute, ceil(transient/2))` entspricht exakt der Mempool-Regel: `fee_mass = max(compute, ceil(transient · 0,5))`, `minimum_fee = mass · 100 000 / 1000` (Cofaktor 500 000 / 1 000 000). Dazu 5 % Aufschlag. | `check_transaction_standard.rs:139-152,165-175`, `config.rs:23`, `params.rs:698-699`; Probe: gezahlte Gebühr = Eingänge − Ausgänge in allen 10 Aktionen |
| Compute-Budget | Probelauf mit Limit `u64::MAX`, dann `checked_covering_script_units`, danach Kontrolle mit dem echten Limit `allowed_script_units()` — genau wie `check_scripts_sequential`. Sigop-Kosten werden **vor** dem Signatur-Cache verbucht, das Budget ist also deterministisch. | `txb.rs:83-96,193-213`, `tx_validation_in_utxo_context.rs:214-222`, `crypto/txscript/src/lib.rs:894-912` |
| Speichermasse | `masses()` benutzt `MassCalculator::new_with_consensus_params`; Commit über `set_storage_mass`. Der Mempool setzt das Commit ohnehin selbst (`utxo_validation.rs:460`, `SkipMassCheck`), Konsens prüft es im Block. | `txb.rs:98-104,164-166` |
| Signierreihenfolge | Signaturen entstehen nach Budget- und Massen-Setzung. Für v1-Transaktionen enthält der Sighash weder Budget noch Signaturskripte; die Tx-ID (`id_v1`) ebenfalls nicht. Reihenfolge ist damit unkritisch, und `wait_accepted` findet die Tx auch dann, wenn ein Relay das Budget oder Dummy-Signaturen verändert. | `sighash.rs:245-283`, `hashing/tx.rs:207-236` |
| Genesis-ID | `genesis_id()` hasht Outpoint des autorisierenden Inputs + (Index, Wert, SPK-Version, SPK). Das Covenant-Feld des Ausgangs geht nicht ein, also ist der Trick „erst ID rechnen, dann Binding setzen" korrekt. | `ops.rs:97-108`, `covenant_id.rs:16-30`, `covenants.rs:147-160` |
| Ein-/Ausgabe-Layouts | Alle Argumentlisten passen zu den Vertragsköpfen: `init(hash, 1, prefix, suffix, sig@4)`, `openVault(owner, 1, prefix, suffix, states)`, `mint(amount, 1, states, sig@3)`, `repay/liquidate(1, states)`, `withdraw(newColl, 1, sig@2)`, `close(sig@0)`, `update(price, daa, rate, sigs, idx)`. Orakel ist überall Input 1 und Ausgang mit `auth=1`, Minter-Zweig Input 2 / `auth=2`, Vault Input 0 / `auth=0`. Zustandsfortschreibung (`o_idx`, `b_idx`, `c_idx`, Token-Entfernung absteigend) ist konsistent. | `ops.rs` gesamt, `stable_vault.sil`, `vault_factory.sil`, `risk_oracle.sil` |
| Mempool-Standardregeln | P2SH-Sigop-Zähler je Input höchstens 3 (Grenze 15), alle Ausgänge Standardklasse, Sigscript ≤ 19 113 B (Grenze 250 000). | Probe `audit_probe.rs` |
| Locktime | `oracle_update` setzt `lock_time = DAA − 20`; Konsens verlangt `lock_time < DAA`. | `ghostctl.rs:758`, `tx_validation_in_header_context.rs:80` |
| Mainnet-Rückfrage | Ohne TTY liefert `read_line` leer → „abgebrochen". `--ja` nur bewusst. | `ghostctl.rs:265-277` |

## 2. Befunde

### F1 (M) — Gesendet, aber nicht gespeichert: kein Wiederanlauf nach Timeout oder Abbruch

**Ort:** `ghostctl.rs:279-309` (`Ctx::send`), `net.rs:96-110` (`wait_accepted`), `ghostctl.rs:493-514` (Feed-Runde).

**Szenario:** `send` speichert die Zustandsdatei erst **nach** `wait_accepted` (120 s). Drei reale Wege, bei denen die
Tx im Netz landet, die Datei aber alt bleibt:
1. Mempool-Stau: Gebühr ist Minimum + 5 %, Blockvorlagen sortieren nach Feerate. Dauert die Aufnahme > 120 s, kommt
   `Err("… nicht bestätigt")`, `save` entfällt.
2. Orakel-Feed: `tokio::time::timeout(180 s, round)` **droppt** die Runde mitten in `send` — nach `submit`, vor
   `save`. Verbindungsaufbau + 6 Kursabfragen (je bis 8 s) + Bau + Wartezeit reichen dafür aus; der Kommentar
   in Z. 490-492 bestätigt hängende Runden im Mainnet-Betrieb.
3. `submit` meldet einen Fehler (WebSocket weg), obwohl der Node die Tx schon hat.

**Folge:** Orakel-Outpoint und `seq`/`stable_index` in der Datei sind falsch. Jede weitere Aktion scheitert an
`check_fresh` („jemand anderes hat es benutzt"), der Feed meldet bei jeder Runde „Prüfung fehlgeschlagen" und
aktualisiert nie wieder — mit `--ja` im Mainnet unbemerkt, und der letzte Preis bleibt auf der Kette stehen.
Es gibt weder ein `resync` noch wird die ausstehende TXID gespeichert; die Reparatur verlangt, den neuen
Orakelzustand (Preis, DAA, seq, Index) von Hand aus der Kette zu rekonstruieren.

**Empfehlung:** vor `submit` die Tx mitsamt Folgezustand als „pending" in die Datei schreiben; beim Start prüfen,
ob Ausgang 0 der pending Tx existiert oder die Eingänge noch unverbraucht sind; Feed-Timeout nur um den Kursabruf
legen, nicht um `send`.

### F2 (M) — `deploy` ist nicht wiederaufnehmbar; 20 KAS können ohne Aufzeichnung gebunden werden

**Ort:** `ghostctl.rs:405-438`, `GHOST-Mainnet-Test.command` Z. 38-43.

**Szenario:** Drei Transaktionen, Zustand wird erst mit der dritten gespeichert (`Some(&d)` nur beim Init). Scheitert
Tx 2 oder 3 (Node-Ablehnung, F1-Timeout, Abbruch mit `N`), existieren Orakel (10 KAS) und Factory (10 KAS) auf der
Kette ohne jede Aufzeichnung — die Covenant-IDs kennt niemand mehr. Ein erneutes `deploy` legt ein zweites Set an.
Das Doppelklick-Skript prüft nur, ob die Zustandsdatei existiert.

**Folge:** bis 20 KAS dauerhaft gebunden (Orakel und Factory sind ohnehin nicht rückholbar, aber hier auch nutzlos),
plus Gebühren. Beim Mainnet-Lauf ist das nicht passiert; der Pfad ist aber ungeschützt.

**Empfehlung:** nach jeder der drei Transaktionen speichern (`Deployment` mit `ghost_root: None`, `vault_params: None`
ist dafür schon vorgesehen) und `deploy` bei vorhandener Teildatei fortsetzen.

### F3 (M) — Verlorene Aktualisierung zwischen Orakel-Feed und Nutzeraktionen (keine Sperre auf der Zustandsdatei)

**Ort:** `ghostctl.rs:253-264` (`load`/`save`), `GHOST-Orakel starten.command` (eigener Prozess, `--ja`), `app/server/api.ts`
(`busy` sperrt nur innerhalb des Seitenservers).

**Szenario (Read-Modify-Write ohne Lock):** Feed lädt Zustand S0 in `oracle_update()` (Z. 747), baut, sendet, wartet
bis 120 s. In dieser Zeit führt der Nutzer `transfer` oder `mint` aus (kein Orakelkonflikt bei `transfer`): lädt S0,
sendet, speichert S0' (Token-Liste geändert). Dann bestätigt der Feed und speichert S1 = S0 + neues Orakel — und
**überschreibt** S0'. Die Datei führt danach eine bereits ausgegebene Token-UTXO und kennt die neue nicht.

**Folge:** spätere `repay`/`transfer` wählen die alte UTXO → „Node lehnt ab"; die neue GHOST-UTXO ist für das
Werkzeug unsichtbar (Guthaben liegt weiter auf der Kette, kein Verlust, aber ohne Kettenscan nicht auffindbar).
Bei Orakel-Konflikten (`mint` gegen Feed) entscheidet die Kette und der Verlierer speichert nicht — das ist
unkritisch; das Problem sind die orakelfreien Aktionen `transfer`, `deposit`, `close`, `send`.

**Empfehlung:** Dateisperre (`flock`) über Laden→Senden→Speichern, oder `save` als Merge (nur die eigenen
geänderten Felder ersetzen), mindestens aber: Feed und manuelle Aktionen nie parallel laufen lassen (dokumentieren).

### F4 (M) — `check_fresh` deckt nur das Orakel ab; Aktionen Dritter machen die Datei unbemerkt falsch

**Ort:** `ghostctl.rs:313-320`; Verträge: `deposit`, `repay`, `liquidate` und Orakel-`read` sind für jeden offen.

**Szenario:**
- Ein Dritter ruft `read()` auf das Orakel (Kosten ≈ 0,0065 KAS). Zustand bleibt gleich, der Outpoint wechselt.
  Ab da scheitert jede Aktion des Betreibers an `check_fresh`; Feed steht. Ein Resync wäre trivial (gleiche P2SH-Adresse,
  da Zustand unverändert), existiert aber nicht. → billiges Griefing gegen den Betrieb.
- Ein Dritter zahlt in Vault 0 ein oder tilgt/liquidiert ihn: `status` rechnet mit alter Sicherheit und Schuld
  (Quote, Liquidationspreis, `maxMintGhost` falsch), `mint`/`withdraw` bauen gegen stale Outpoints → Ablehnung.
  Vault, Minter-Zweig, Wurzel-Minter, Factory und Token-UTXOs werden nie geprüft.

**Folge:** kein Geldverlust, aber Betriebsausfall und falsche Anzeigen; `MAINNET.md` nennt das Problem, ohne Ausweg.

**Empfehlung:** `check_fresh` auf alle getrackten Outpoints ausdehnen (eine `get_utxos_by_addresses`-Abfrage pro
Adresse) und ein `resync`, das für jede bekannte Covenant-Adresse die aktuelle UTXO liest und den Zustand aus dem
Skript (State-Span) zurückgewinnt.

### F5 (N) — JSON-Schnittstelle: gesendete Transaktionen gehen im Fehlerfall verloren, stdout nicht rein

**Ort:** `ghostctl.rs:337-348` (`main`), `265-277` (`confirm`), `439-442`/`450-478` (`Status`).

- Bei `Err` druckt `main` nur `{"ok":false,"error":…}`; `ctx.txs` mit `sent:true` wird verworfen. Die Lending-Seite
  (`api.ts`, 422-Pfad) kann nicht erkennen, dass Geld unterwegs ist (F1-Fälle, oder `deploy` nach Tx 1/2).
- `confirm()` schreibt die Rückfrage mit `print!` nach **stdout** und blockiert auf stdin — bei `--json` ohne `--ja`
  im Mainnet ist stdout kein JSON mehr (die App sendet immer `--ja`, ein Skript-Aufrufer nicht zwingend).
- `ghostctl --json status` (globales Flag) liefert `{"dryRun":false,"network":…,"ok":true,"transactions":[]}` ohne
  Statusdaten; nur `status --json` (Unterkommando-Flag) liefert sie. **Ausgeführt und bestätigt.**
- Panics (siehe F6) liefern gar kein JSON, Exit 101.

### F6 (N) — Eingaben ohne Bereichsprüfung: Panik statt Fehlermeldung

**Ort:** `ops.rs:468` (`withdraw`), `ghostctl.rs:323-325` (`to_units`), `mass/mod.rs:465` (Division durch `amount`).

- `withdraw --keep 5000` bei 3 001 KAS Sicherheit: **ausgeführt (dry-run, Testnetz)** →
  `panicked at src/ops.rs:468:36: attempt to subtract with overflow`. Nur weil der Wrapper den Debug-Build nutzt;
  im Release-Build würde der Wert umbrechen (~1,8·10¹⁹ sompi) und in `assemble` an „Zu wenig KAS" scheitern.
- `--kas 0`, `--keep` = Sicherheit, `send --kas 0`: Ausgang mit Wert 0 → `calc_storage_mass` teilt durch `amount`
  (UNVERIFIED, aus dem Code abgeleitet; der Kommentar dort verlangt „All output values are non-zero").
- Negative Beträge (`--ghost -1`): `shares_for` mit `m as u128` überläuft/paniert im Debug-Build.

Kein Geldverlust möglich, aber im JSON-Modus keine Fehlermeldung. Empfehlung: Beträge in `to_units` und in `ops`
auf `> 0` und `≤ Bestand` prüfen.

### F7 (N) — `sim.rs`/`txb::build` prüfen keine Blocklimits und keine Mempool-Regeln

**Ort:** `sim.rs:47-83`, `txb.rs:193-213`; Node: `check_transaction_limits.rs:14-52`, `check_transaction_standard.rs`.

**Bewiesen** (`audit_probe.rs`): `withdraw`, das 0,01 KAS auszahlt, ergibt Speichermasse **1 003 783 g**; `sim.submit`
nimmt an, der Mempool lehnt ab (Limit 500 000 g, `RejectStorageMass`). `ghostctl` würde die Tx bauen, die
Mainnet-Rückfrage stellen und erst dann „Node lehnt ab" melden. Weitere Lücken der Simulation, alle aus dem Code:
- Coinbase-Reife (`coinbase_maturity`) fehlt; `Net::funds` filtert `is_coinbase` auch nicht.
- P2SH-Sigop-Grenze 15 fehlt (derzeit ≤ 3, unkritisch, aber bei größeren Verträgen relevant).
- Locktime: Sim erlaubt `lock_time == daa`, Konsens verlangt `<`.
- Ausgangs-Skriptklasse, `max_tx_inputs/outputs`, Compute-/Transient-Limits (500 000 / 1 000 000 g).
`TESTNET_LOG.md` schreibt „Gebühren stimmen exakt mit der Simulationskette überein" — das gilt für die Gebühr,
nicht für Ablehnungsgründe. Empfehlung: in `build` die drei Blocklimits prüfen; Auszahlungs-/Wechselgeld-Ausgänge
unter ≈ 0,05 KAS ablehnen oder aufrunden.

### F8 (N) — Wechselgeld-Spende bis 0,2 KAS ohne Warnung

**Ort:** `txb.rs:79,150-157` (`MIN_CHANGE = 20 000 000`).

**Bewiesen:** `send` mit Rest = MIN_CHANGE + 100 sompi vor Gebühr → 1 Ausgang, gezahlte Gebühr 20 000 100 sompi bei
Mindestgebühr 162 600 → **0,198 KAS Überzahlung**. Sichtbar nur in der Gebührenzeile; mit `--ja` (Feed, App) oder im
Testnetz gibt es keine Warnung. Ein 0,2-KAS-P2PK-Ausgang kostet 50 000 g Speichermasse, wäre also erlaubt; eine
Schwelle um 0,05 KAS würde die Spende auf ein Viertel senken.

### F9 (N) — Netz und Zustandsdatei werden nicht abgeglichen; Standardnetz ist Mainnet

**Ort:** `ghostctl.rs:34-35,253-256,385`.

**Bewiesen:** `./ghostctl --network testnet-10 --state deployments/mainnet.json status` läuft durch und zeigt
„Netz mainnet" aus der Datei, verbunden ist Testnet. `Deployment.network` wird nie gegen `--network` geprüft. Ein
vertauschtes `--state` baut Transaktionen gegen fremde Outpoints (Ablehnung, kein Verlust). Umgekehrt: `--network`
vergessen + `--ja` = Mainnet ohne Rückfrage. Empfehlung: `d.network != cli.network` → Abbruch.

### F10 (N) — Schlüsseldateien

**Ort:** `ghostctl.rs:226-240` (`write_new`), `keys/`.

- `std::fs::write` legt die Datei mit umask an (hier 022 → `0644`), erst danach `set_permissions(0600)`. Kurzes
  Fenster mit lesbarem Geheimnis; `exists()`-Vorprüfung statt `create_new`. Besser:
  `OpenOptions::new().write(true).create_new(true).mode(0o600)`.
- Die Komitee-Datei enthält alle 5 Signierer-Geheimnisse im Klartext neben dem Owner-Schlüssel; das 3-von-5 ist
  damit im Betrieb ein 1-von-1 (in `MAINNET.md` offen benannt).
- `keys` parst jede `*.json` im Verzeichnis als Schlüssel; Schlüssel sind nicht netzgebunden (`mainnet-owner.json`
  erscheint unter `--network testnet-10` mit `kaspatest:`-Adresse — bestätigt). Verwechslungsgefahr, kein Verlust.

### F11 (N) — `wait_accepted` = „Ausgang 0 als UTXO sichtbar" ist keine Annahmeprüfung

**Ort:** `net.rs:96-110`, `ghostctl.rs:300`.

- Wird Ausgang 0 innerhalb des 700-ms-Rasters von einem Dritten ausgegeben (Orakel-Ausgang nach `oracle-update`:
  `read()` darf jeder), sieht die Schleife ihn nie → Timeout → F1, obwohl die Tx angenommen ist.
- Keine Bestätigungstiefe: eine Reorganisation des virtuellen Zustands nach dem `save` bliebe unbemerkt
  (Häufigkeit auf Kaspa: UNVERIFIED; die Tx-ID bleibt gleich, bei Wiederaufnahme heilt es sich selbst).
- Vertrauen in den Resolver-Node: der Node liefert UTXO-Einträge und „Annahme". Falsche Beträge/SPKs machen die
  Signatur ungültig (kein Verlust), aber ein Node kann Annahme vortäuschen oder verschweigen → Zustandsdrift.
Empfehlung: Annahme über `get_virtual_chain_from_block(include_accepted_transaction_ids)` oder Mempool-Eintrag +
UTXO prüfen, ein paar Blöcke Tiefe abwarten, `--rpc` auf eigenen Node empfehlen.

### F12 (N) — `own_tokens` nimmt höchstens 2 UTXOs: Volltilgung scheitert trotz ausreichendem Guthaben

**Ort:** `ghostctl.rs:735-742`, `ops.rs:326` (`GHOST_MAX_INS − 1 = 2`).

**Bewiesen** (`audit_tokens.rs`): dreimal 0,3 GHOST geprägt, Schuld 0,9, Guthaben 0,9 in 3 UTXOs →
`repay` ohne `--ghost`: „zu wenig GHOST: 60000000 vorhanden, 90000000 nötig"; mit 3 Indizes: „1 bis 2 Token-UTXOs
erlaubt". Ausweg: Selbst-`transfer` zum Zusammenlegen (2 → 1), danach klappt es — nirgends dokumentiert, in der
App nicht erreichbar. Gleiches gilt für `liquidate`. Empfehlung: automatisch zusammenlegen oder klare Meldung.

### F13 (N) — Starter und Wrapper

- `GHOST-Mainnet-Test.command` Z. 46: `if ! G status | grep -q "^Vault 0:"` — jeder Fehlschlag von `status`
  (Netz, Node) gilt als „kein Vault" und führt zu `open-vault` (weitere 150 + 3 KAS). Die Mainnet-Rückfrage ist die
  einzige Bremse. `balance()` unterdrückt stderr, ein Netzfehler wird zu „0 KAS".
- `./ghostctl` baut nur, wenn die Binärdatei fehlt; nach Quelländerungen läuft die alte Version. Es läuft der
  Debug-Build (Überlaufprüfungen aktiv, siehe F6 — hier eher ein Vorteil, aber langsam und unbeabsichtigt).
- `GHOST-Orakel starten.command`: `--ja` im Mainnet mit allen 5 Komitee-Schlüsseln auf dem Rechner; ohne
  Überwachung, ob Runden wegen F1/F4 dauerhaft scheitern (nur stderr im Fenster).
- `app/server/actions.ts` setzt immer `--json --ja`; der Schutz ist `confirmMainnet === true` aus dem Browser plus
  Origin-/Host-/Header-Prüfung. Das ist konsistent, verlagert aber die Mainnet-Rückfrage vollständig in die Seite.

### I1 — Compute-Budget ist weder in Tx-ID noch Sighash (v1)

`hashing/tx.rs:111-114` (nur ohne `EXCLUDE_MASS_COMMIT`), `sighash.rs:258-268` (nur `version < 1`). Ein Relay könnte
Budgets verändern; mit größerem Budget steigt die Compute-Masse, die Gebühr könnte unter das Minimum fallen, die Tx
bliebe liegen. Kein Angriff auf Mittel, aber ein möglicher Grund für F1-Timeouts. Betrifft das Protokoll, nicht
diese Software.

### I2 — Storage-Mass-Commit wird vom Mempool überschrieben

`utxo_validation.rs:460` setzt die Speichermasse selbst und validiert mit `SkipMassCheck`; ein falsches lokales
Commit fiele erst bei der Blockvalidierung auf. Da `masses()` korrekt rechnet, ohne Folge.

## 3. Messwerte aus der Probe (Simulation, Mainnet-Parameter)

```
Aktion                 ins outs  compute transient storage   Gebühr   P2SH (idx,sigops,B)
Orakel-Genesis          1   2     2083     1412     4000    218715   –
Factory-Init+GHOST      2   3     7890    17960     7920    942900   (0,1,3996)
Vault eröffnen 150      3   5    27125    87140    21044   4574850   (0,1,19113) (1,2,1950)
Mint 1 GHOST            4   5    26139    82396    56449   4325790   (0,3,17045) (1,3,828) (2,2,1950)
Orakel-Update 3/5       2   2     6197     5868     3920    650685   (0,3,1060)
Repay 0,5 GHOST         5   5    28184    89776    55668   4713240   (0,3,16974) (1,3,828) (2,2,1950) (3,2,1862)
Withdraw auf 100        3   4    22846    73544     4025   3861060
Deposit 10              3   2    21142    69648        0   3656520
Transfer 0,2 GHOST      2   3     5744     9776    79723    603120
Withdraw, 0,01 KAS raus 3   4    22846    73544  1003783   3861060   ← Sim nimmt an, Mempool lehnt ab
```
Gebühr = Eingänge − Ausgänge in jedem Fall; Budgets z. B. Mint `[25, 0, 2, 10]`.

## 4. Priorisierte Empfehlungen

1. Pending-Tx in der Zustandsdatei + Wiederanlauf/`resync` (F1, F4) — das ist die eine Sache, die den
   Mainnet-Betrieb heute stoppen kann.
2. `deploy` schrittweise speichern (F2).
3. Dateisperre oder Merge beim Speichern (F3).
4. Blocklimits und Betragsbereiche in `build`/`ops` prüfen (F6, F7); Netz/Zustand abgleichen (F9).
5. `keygen` mit `create_new` + Modus 0600 (F10); JSON-Fehlerpfad um `transactions` ergänzen, Rückfrage nach stderr (F5).
