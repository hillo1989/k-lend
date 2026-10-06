# Audit 20b: Rust-Protokoll und Agent (Version 4, Mainnet)

Prüfer: Claude Fable 5.1, unabhängig, nur lesend. Datum: 06.10.2026.
Stand: Arbeitskopie `main` (nach Audit 19 und der Behebung A19-1…9, mit Grundzins `rate_floor_step`).
Es wurde nichts gesendet, kein Server angefasst, `keys/` nicht gelesen, aus `deployments/` nur Dateinamen. Produktivcode unverändert. Einziger Eingriff: die Testdatei `protocol/tests/audit20b_pruef.rs`, nach dem Lauf entfernt; der Kern steht im Anhang.

Geprüft: `protocol/src/{wallet_ops,wallet,txb,ops,tresor,store,chain,net,rate,math,pool,message,price,sim}.rs`, `protocol/src/bin/ghostctl.rs` (wallet build/submit, agent, oracle_round, rate_plan mit `rate_floor_step`, keeper, tresor_agent_step, receive --owner, utxos), dazu `contracts/stable_vault_v4.sil` und `price_oracle_v4.sil` an den Rechenstellen. Hintergrund: ARCHITEKTUR.md, docs/wallet-aktionen.md, AUDIT.md, audit/17–19.

Testläufe: `cargo test --release --test audit20b_pruef` 5/5 ok (Ausgaben unten); Gegenprobe der früheren Behebungen: `wallet_ops_tests a1*` 7/7, `store::` 8/8, `rate::` 12/12, ghostctl `a19_*` 3/3 ok.

## Kurzfassung

| # | Schwere | Befund | Beleg |
|---|---|---|---|
| A20b-1 | **mittel** (Verfügbarkeit) | Jede Wallet-Überweisung an eine neue Adresse hängt einen Eintrag an die serverweite Token-Liste `d.tokens`; jeder Abgleich (`store::resync`) fragt je Eintrag einmal den Node. Ein Besucher wächst die Liste für ≈ 0,006 KAS Gebühr je Eintrag (1 KAS je Eintrag nur gebunden, holt er zurück). Jede Orakel-/Keeper-Runde, jeder Status, jeder `wallet build/submit` wird linear langsamer – unter der Haupt-Sperre. | Test (Liste +1 je Überweisung, Kosten gemessen); Zahl der Node-Abfragen aus dem Code; Laufzeit am Server vermutet |
| A20b-2 | niedrig (Regression A12-3) | Der neue Grundzins-Pfad in `rate_plan` fragt nur `wait_secs`. Eine `last_change` aus der Zukunft (Uhr lief einmal vor) kappt nur `prune`, das dort nie läuft. Liegt der Zins unter 2 %, wartet die Zinsregel dann für immer, und es werden auch keine Messungen aufgezeichnet. | Test an `RateLog` + Code |
| A20b-3 | niedrig | Die Frist eines Wallet-Journals hängt an der mtime der Journaldatei. Liegt die mtime in der Zukunft (Uhr nach dem Schreiben zurückgestellt, NTP-Korrektur), wird `elapsed()` zum Fehler, das Alter `Duration::MAX`, und das Journal ist sofort verworfen, obwohl die Tx angenommen wird. Folge wie G-2/G-6: GHOST-/LP-Einträge fehlen dauerhaft im Zustand. | Test (Journal mit mtime +5 min wird sofort verworfen) |
| A20b-4 | niedrig (CPU) | `precheck` rechnet je Signer-Eintrag des Browser-Plans einen vollen Sighash und eine Schnorr-Prüfung, ohne vorher Zahl und Eindeutigkeit der Einträge zu prüfen. Ein 714-kB-Plan (unter dem 768-kB-Limit) kostet 0,8 s CPU statt 0,17 ms – vor jeder weiteren Prüfung, je Aufruf. | Test (gemessen) |
| A20b-5 | niedrig (Buchführung) | `receive --owner` (öffentlich) schreibt die Zustandsdatei unter Sperre, ohne das Journal aufzulösen. Ein gleichzeitig offenes Wallet-Journal überschreibt beim Übernehmen den Folgezustand und verliert die gerade eingetragenen Token (erneutes `receive` holt sie wieder). | Code |
| A20b-6 | Hinweis | Solange der Zins unter dem Grundzins liegt, misst die Zinsregel den GHOST-Kurs nicht; nach Erreichen von 2 % braucht sie erst wieder 6 Messungen über 45 min. Fällt der Grundzins-Schritt in eine Runde, in der der Vertrag die Pause (DAA) noch nicht freigibt, geht die Vormerkung der Zinsdatei für eine Stunde verloren. | Code |

**Keine Befunde** bei: Umleiten von Ausgängen, Gebühr oder Wechselgeld in `submit`; Fälschen von Einträgen in deployments/*.json, Tresor- oder Zinsdatei; Verlust-Transaktionen oder Gebührenverbrauch des Agenten durch Außenstehende; Endlosschleifen; Rundung/Überlauf in math.rs gegenüber dem Vertrag; Grundzins-Schritte gegenüber dem Vertragsrahmen; Manipulierbarkeit des Preis-Medians. Einzelheiten unter „Geprüft und sauber“.

---

## Frage 1: Manipulierter Plan oder manipulierte Wallet-Antwort in `submit`

**Kein Weg gefunden, etwas anderes zu senden als geprüft.** Ablauf (`wallet_ops.rs:1054–1109`, `finish` 1113–1170; `ghostctl.rs:4112–4161`):

- Aus dem Browser-Plan werden nur drei Dinge verwendet: `action`, `address`/`owner` und die eigenen P2PK-Eingänge (`plan_funding`, 986–998: nur Eingänge mit dem P2PK-Skript der Adresse, höchstens 8). Alles andere (Covenant-Eingänge, Beträge, Ausgänge, Gebühr, Budgets, Signer-Liste) kommt aus dem Neubau `plan_with_funding` (908–965) aus dem Serverzustand. Der Neubau muss als JSON bitgleich sein (1065–1068), weiter geht es nur mit `fresh.plan` (1069).
- Die Beträge der eigenen UTXOs stammen aus dem Plan, werden aber vor beiden Abgleichen am Node geprüft: `check_funding_at_node` (`ghostctl.rs:4019–4030`, Betrag, Skript, keine Covenant) läuft vor `load_readonly` (4121) und erneut unter der Sperre (4145). `is_coinbase` lehnt der Neubau ab (929), `block_daa_score` geht weder in Sighash noch Masse ein (nur in relative Sperren, die ghostctl nicht nutzt). Ein falscher Wert kann also höchstens die eigene Tx am Node scheitern lassen.
- Signaturen werden gegen den Sighash der **neu gebauten** unsignierten Tx geprüft (`read_sigs`, `wallet.rs:814–844`), Hashtype ALL erzwungen. `finish` baut mit den Signaturen ein drittes Mal, misst Budgets nach (nicht im Sighash), prüft `check_scripts`, Sigops, Blockgrenzen und Mindestgebühr (1145–1164). Eine Signatur über eine andere Tx scheitert dort. Compute-Budget und Speichermasse der Wallet-Antwort werden ignoriert und neu gesetzt (`diff`, 761–762/804).
- Gebühr und Wechselgeld: `WalletFill { fee: fee_req, paid: plan.fee }` sind in Plan und `finish` dieselben Werte (`plan_with_funding` 933 ↔ `finish` 1126); `same_shape` (695–716) verlangt in der Messkopie gleiche Beträge, Massen, Gebühr und Wechselgeld-Index. Das Wechselgeld geht immer an `p2pk_spk(&me)` (Funds::change, `ops.rs:169–171`), der Rest unter 0,2 KAS an die Miner – bei beiden Bauten gleich.
- Zustand: `next` stammt aus dem Neubau unter Sperre und geht nur über das Journal (`send_wallet`, `ghostctl.rs:936–996`) in die Datei. Für Tresore ebenso (`wallet_tresor_submit` 4734–4745, Journal-Ziel = Tresor-Datei). Die Zinsdatei berührt kein Wallet-Pfad.
- Tresor-Aktion aus dem Plan: `run_tresor` prüft `Action::check` (Beträge, Nachricht über `abo::check_message`, Tresor-ID 64 Hex), `check_params`, `check_wallet_first_due` gegen die PMT und `make_room_for_wallet` – in build und Neubau (A19-1 hält, Test grün). `TresorCancel` mit eigenen KAS wird abgelehnt (922–924).

Restpunkt A20b-4 (CPU in `precheck`) betrifft nur Rechenzeit, nicht die Prüfung.

## Frage 2: Kann ein Außenstehender den Agenten schädigen?

- **Verlust-Transaktionen:** Liquidiert wird nur, wenn der Vault nach Orakel- UND Marktpreis unterdeckt ist und zum Marktpreis ≥ 2 % Gewinn bleibt (`math::keeper_burn`, `keeper_plan` `ghostctl.rs:2301–2340`); höchstens eine je Runde. Auflösen (`sweep`) trägt der Vault (SWEEP_FEE 0,1 KAS, Netzgebühr ≈ 0,055), nur bei Orakel UND Markt `sweepable`. Wallet-Tresore zahlen die Gebühr nie aus dem Agenten-Schlüssel (`key_may_pay`, `tresor.rs:1194–1198`; `pay_round` 1601). Einfrieren kostet den Agenten eine Tx, aber nur, wenn das eigene Orakel ausfällt.
- **Falsche Orakelpreise:** Median aus 6 Quellen, mindestens 3, Ausreißer > 3 % fallen heraus (`price.rs:54–69`); je Update höchstens ×2/÷2 (`oracle_round` 2822, Vertrag Z. 74–75); Sprünge > 20 % erst nach 3 Runden in dieselbe Richtung binnen 5 %, gesendet wird der Median der drei (2789–2808). Ein Außenstehender müsste mindestens 3 der 6 Quellen gleichzeitig bewegen. Bekannt bleibt A-14/A-15 (gemeinsame Upstreams der Aggregatoren).
- **Falsche Zinsänderungen:** Der Grundzins (`math::rate_floor_step`, `rate_plan` 2677–2693) hat keinen Eingang von außen. Die Kursregel bleibt die von A12-12 (Median über 1 h, ≥ 6 Messungen über ≥ 45 min, Pool ≥ 10 GHOST, Band ±3 %); nach unten jetzt bei 2 % begrenzt (`rate_next`). Bei einem von Hand gesetzten Satz neben dem Raster (z. B. 2,2 %) senkt die Regel nie und hebt in 0,5er-Schritten, `fit_rate` (2876–2885) hält jeden Schritt im Vertragsrahmen.
- **Endlosschleifen:** alle Schleifen begrenzt (`build_ext` 6 Runden, `finish` 2, `consolidate` endet bei ≤ 2 UTXOs, `follow_*` MAX_STEPS 5 000, `candidates_max`, `wait_accepted` max, `Net::connect` 3 Runden). Die Takt-Grenzen der Nebenschritte (A12-5) gelten weiter.
- **Sperren-Blockade:** Die öffentlichen Wege nehmen die Haupt-Sperre nur zum Senden (`precheck` davor, A17-4; `receive --owner` nur zum Speichern, `utxos` nie, A19-4). Was bleibt, ist die **Dauer** eines Abgleichs unter Sperre – und die lässt sich über die Token-Liste strecken (A20b-1).
- **Gebührenverbrauch:** kein Weg gefunden. Oracle-Updates folgen dem Marktpreis (nicht beeinflussbar), Grundzins höchstens 4 zusätzliche Updates insgesamt.

### A20b-1 (mittel): Token-Liste wächst mit jeder Überweisung, jeder Abgleich fragt je Eintrag den Node

**Ort:** `ops.rs:1056–1102` (`transfer_with_payload`: Empfänger- und Wechselgeld-Token werden an `d.tokens` angehängt, 1098–1100), `store.rs:702–711` (`resync`: `follow` je Token, 407–419: `net.exists` = `get_utxos_by_addresses` je Eintrag), Aufrufer `load_synced` (`ghostctl.rs:826–851`: Orakel-Runde 2776, `oracle_update_with` 2940, Keeper 2233, Status 2103, Wallet-Senden 4144) und `load_readonly` (3890–3900: jeder `wallet build` 4092 und jeder `submit` 4122). Entfernt wird ein Eintrag erst, wenn seine UTXO verschwunden ist (`resync` 705–709).

**Szenario:** Ein Besucher prägt einige GHOST und überweist über die Seite je 1 Einheit an frische eigene Adressen (`transfer`). Jede Tx hängt einen Eintrag an die Liste und bindet 1 KAS je Token-Ausgang (TOKEN_VALUE), die er später durch Zusammenführen zurückbekommt. Die Liste hat keine Obergrenze und wird nie nach Besitzern zusammengefasst. Mit 2 000 Einträgen macht jeder Abgleich 2 000 zusätzliche wRPC-Abfragen hintereinander; bei 20–50 ms je Abfrage sind das 40–100 s **je Abgleich**. Die Orakel-/Keeper-Runde gleicht bis zu dreimal ab (Runde, Update, Keeper) und hat ein Limit von 240 s; `wallet submit --send` gleicht zweimal ab, davon einmal unter der Haupt-Sperre, mit 170 s Seitenlimit; `status` alle 20 s. Der Agent wartet 60 s auf die Sperre. Ab einigen tausend Einträgen fallen Runden aus, der Preis veraltet (Einfrieren nach 2 h), und Wallet-Sendungen scheitern am Zeitlimit.

**Beleg** (Test `a20b_token_liste_waechst_je_ueberweisung`, Simulator):
```
A20b Token-Liste: 1 → 26 Einträge nach 25 Überweisungen; KAS des Angreifers weniger: 25.1532
     (davon 25 KAS in Token-UTXOs gebunden, Gebühren 0.1532 KAS)
```
Also ≈ 0,006 KAS Gebühr je Eintrag; 2 000 Einträge ≈ 12 KAS Gebühr plus 2 000 KAS vorübergehend gebunden. Die Zahl der Node-Abfragen je Abgleich (eine je Token) folgt aus dem Code; die Latenz am Server ist **vermutet**, nicht gemessen. Über die Seite begrenzt das Kontingent (20/min je Absender, IPv6 je /64) nur das Tempo.

**Vorschlag:** Token in `resync` nach Adresse bündeln (`get_utxos_by_addresses` nimmt eine Liste; je Besitzer eine Abfrage statt je UTXO) und Vaults genauso (Vault und Zweig zusammen). Dazu eine Obergrenze je Besitzer und insgesamt, über der fremde Token nur noch auf Anfrage (`receive`) geführt werden, und Einträge fremder Besitzer, die lange unverändert sind, nur jede n-te Runde prüfen. Mittelfristig gehört der Abgleich fremder Token nicht in jede Orakel-Runde.

## Frage 3: Journal, Wiederanlauf, Nebenläufigkeit, Uhr

**Sauber:** Alle Schreiber der Zustandsdatei und der Tresor-Datei lösen vor dem Schreiben unter Sperre das Journal auf (`load_synced`; `tresor_cmd` 4425; `wallet_tresor_submit` 4735). Ein offenes Wallet-Journal sperrt andere Schreiber so lange, bis es übernommen oder verworfen ist (höchstens 180 s, Mempool-Fall G-4 bleibt). `send_then_wait` gibt die Sperre nach dem Senden frei (A17-6 hält, Test `senden_haelt_die_sperre_warten_nicht`). Zwei gleichzeitige `submit --send` desselben Plans: der zweite trifft unter Sperre auf das Journal („noch unterwegs“ oder „nicht geklärt“) und sendet nicht. Nach `write_pending` und Node-Ablehnung wird das Journal nur bei `in_mempool == Ok(false)` gelöscht. `take_over` ist idempotent (Datei schreiben, dann Journal löschen). Sperre per `flock`, vom Betriebssystem freigegeben; Reihenfolge Zustandssperre → Zinssperre überall gleich, kein Verklemmen. Zeitlimits der Agentenschritte lassen die Sperre mit dem abgebrochenen Future fallen.

### A20b-2 (niedrig): Grundzins-Pfad kappt eine Zukunftszeit nicht (Regression A12-3)

**Ort:** `ghostctl.rs:2677–2693` (`rate_plan`: `if let Some(next) = math::rate_floor_step(cur) { … rate_log(round, |l| l.wait_secs(unix_now())) … }` und Rückkehr **vor** `rate::measure`/`record`), `rate.rs:160–166` (`wait_secs`: `Some(t) if t > now => EVERY_SECS`), Kappen nur in `prune` (131–136), das über `record`, `reserve`, `set_by_hand` läuft.

**Szenario:** Die Uhr des Servers lief einmal vor, der Agent hat eine Zinsänderung vorgemerkt (`last_change` in der Zukunft; genau der Fall A12-3). Nach der Korrektur liegt der Zins unter 2 % (z. B. nach einem Umzug mit `--rate 0`, oder nach einem Zins von Hand). Jede Runde meldet „nächster Schritt in 60 min“, für immer; weil der Pfad vor der Messung zurückkehrt, füllt sich auch das Messfenster nicht. Nur `oracle-update --rate` von Hand (`set_by_hand` → `prune`) löst es.

**Beleg** (Test `a20b_grundzins_pfad_kappt_zukunftszeit_nicht`): `wait_secs` bleibt über 5 Stunden bei 3 600 s; `rate_floor_step(0.0)` ist `Some`; erst `prune` kappt. Nicht von außen auslösbar, aber die A12-3-Behebung verlässt sich auf `record`/`reserve`, die hier nicht laufen.

**Vorschlag:** Im Grundzins-Pfad `l.prune(now)` vor `wait_secs` aufrufen (oder `wait_secs` kappend machen), und die Messung `rate::measure`/`record` auch dann aufnehmen, wenn der Grundzins-Schritt zurückgegeben wird (A20b-6).

### A20b-3 (niedrig): Journal-Frist über mtime, Zukunfts-mtime verwirft sofort

**Ort:** `store.rs:210` und `366`: `metadata(path).modified().elapsed().ok().unwrap_or(Duration::MAX)`. `SystemTime::elapsed` liefert `Err`, wenn die mtime nach „jetzt“ liegt. Dann ist `age = MAX`, und sowohl die 60-s-Frist bei sichtbaren Eingängen (A18 G-2, 215–219) als auch die 180-s-Frist in `resolve_wallet` (367–376) sind sofort verstrichen.

**Szenario:** Das Journal wird geschrieben, danach stellt NTP die Uhr um ein paar Sekunden zurück (oder ein Dateisystem mit vorlaufender Zeit). Der nächste Abgleich – Status alle 20 s, Agent – sieht den Eingang noch (UTXO-Index hinkt), REST kennt die Tx noch nicht, und verwirft das Journal. Die Tx wird trotzdem angenommen; ihre GHOST-/LP-Ausgänge fehlen im Zustand (`resync` entdeckt keine Token neu, G-6), der Besucher muss `receive` nutzen, der Empfänger einer Überweisung sieht seine GHOST auf der Seite nicht.

**Beleg** (Test `a20b_journal_mit_zukuenftiger_mtime_wird_sofort_verworfen`): innerhalb der Frist `Err` (warten); nach `set_modified(now + 300 s)`:
```
Ok(Some("Letzte Wallet-Transaktion (GHOST senden (Wallet)) wurde nicht angenommen – verworfen."))
```
Nicht von außen auslösbar; Uhrannahme.

**Vorschlag:** Bei `elapsed() == Err` (Zukunft) das Alter als 0 werten, nicht als MAX; besser den Schreibzeitpunkt (Unix-Sekunden) ins Journal schreiben und mit `unix_now()` vergleichen, dabei Zukunft als 0 behandeln.

### A20b-5 (niedrig): `receive --owner` löst das Journal nicht auf

**Ort:** `ghostctl.rs:1784–1824`: öffentlich (`owner` gesetzt) wird `ctx.load()` ohne Abgleich gelesen (1796), bei Treffern `store::lock(10 s)` genommen, erneut `ctx.load()` (1803–1804), Token angehängt und gespeichert (1818–1819) – ohne `store::resolve_pending`. Alle anderen Schreiber gehen über `load_synced`.

**Szenario:** Ein Wallet-Journal ist offen (Tx gesendet, Bestätigung läuft, Sperre frei). `receive --owner` trägt Token ein und speichert. Danach wird das Journal übernommen: `take_over` schreibt den **Folgezustand von vor dem receive** – die eingetragenen Token sind wieder weg. Umgekehrt kennt ein eingetragener Token das Journal nicht; sein Eintrag kann unter `next` doppelt vorkommen (gleicher Outpoint, einmal aus dem Journal), was `resync` beim nächsten Verschwinden nur einmal entfernt. Kein Geldverlust, nur Buchführung; ein erneutes `receive` heilt. Im Code belegt, nicht im Netz nachgestellt.

**Vorschlag:** Unter der kurzen Sperre zuerst `resolve_pending`; ist das Journal „unklar“, das Eintragen mit einer verständlichen Meldung abbrechen („bitte gleich erneut“).

### Uhr und DAA

Orakel-Alter, Einfrieren und Zinspause rechnen in DAA mit Rückstand 20 (`feed_due`, `freeze_if_stale`, `fit_rate`); die Zinsdatei kappt Zukunftszeiten in `prune` (außer A20b-2); Tresor-Termine gegen die Past Median Time des Nodes (`check_wallet_first_due`, `pay` mit `lock_time = next_due`); `make_room_for_wallet` mit PMT, `needs_run` offline mit Rechneruhr (nur Vorfilter). Kein weiterer Befund.

## Frage 4: Rundung und Überlauf in math.rs gegenüber dem Vertrag

Verglichen mit `stable_vault_v4.sil`: `growth` (Vertrag Z. 127–144, dreistufig mit Rest) = ⌈Δ·1e9/from⌉ wie `math::growth`; `accrual`/`accrued` (146–165) = interest + ⌈base·growth/1e9⌉ wie `math::accrued` (u128, Deckel MAX_GROWTH); `interestFee` (168–174) ⌈accrued·1e8/price⌉, höchstens die Sicherheit, wie `interest_fee`/`interest_fee_checked`; `healthy` (186–190) ⌊coll·price/1e8⌋ ≥ ⌈owed·bps/1e4⌉ wie `math::healthy`; `redeem` (353–372) ⌊⌊amount·9900/1e4⌋·1e8/price⌋, `paid > 0`, `coll − paid ≥ DUST` wie `redeem_paid`/`redemption`; `liquidate` (374–396) claim ⌈burn·(1e4+bonus)/1e4⌉, seize ⌈claim·1e8/price⌉ bzw. coll, Endezweige wie `math::seize`/`liquidation`. `max_mint` ⌊v·1e4/m⌋ ist das größte o mit ⌈o·m/1e4⌉ ≤ v. Die vorhandenen Tests `vault_math_tests.rs` (Zufallswerte gegen u128) und `vault_v4_tests.rs` decken das ab.

Überlauf: `value_of` rechnet in u128 und schneidet mit `as i64`; mit MAX_COLLATERAL 1e16 sompi und MAX_KAS_USD 9e10 ist das Maximum 9e18 < i64::MAX, also sicher. `keeper_burn` rechnet in i128. `cap_room` ist gegen MAX gesichert (A-4). `pool.rs` rechnet in i128 und prüft MAX_RESERVE/MAX_SHARES. Die Beträge aus einem Plan sind auf 10^18 begrenzt (`MAX_UNITS`), `v.vault.value + add` und `t.value + add` bleiben unter u64::MAX, Überschüsse scheitern am Bau („Zu wenig KAS“). Division durch 0 nur bei `price == 0` oder `index_at == 0` – beides schließt der Vertrag aus, und `accrued` prüft `index_at > 0`.

Grundzins gegen den Vertrag (Test `a20b_grundzins_schritte_erreichen_den_grundzins_im_vertragsrahmen`): von 0 %, 0,3 %, 0,7 %, 1,3 %, 1,8 % und 1,9999 % aus erreicht die Regel mit `fit_rate`-Begrenzung in 1–4 Schritten genau 2,0 %, jeder Schritt nimmt `ops::check_update` (rateStep, rateGapDaa) an.

## Frage 5: Preisquellen des Agenten

Siehe Frage 2. Ergänzend: `Streak` lebt nur im Prozess (Neustart setzt zurück), große Sprünge brauchen 3 Runden à 5 min; ein Fehlschlag der Runde setzt zurück (A10-A-7). Die Kursquelle der Zinsregel ist der Pool (Band ±3 % um das Orakel), nicht die Preisquellen. Kein neuer Weg, den Median oder den Pool-Median zu verschieben, ohne Geld zu binden (A12-12, A11-O-1 halten; Tests `rate::` grün).

### A20b-4 (niedrig): CPU-Last der Vorprüfung mit aufgeblasenem Plan

**Ort:** `wallet_ops.rs:1018–1049` (`precheck`): `plan_funding` prüft nur die eigenen Eingänge (≤ 8); dann `from_safe` beider Tx, `diff` und `read_sigs` (`wallet.rs:814–844`) über **alle** `plan.signers` – je Eintrag `sighash_all` (656–660, klont die Tx und hasht sie samt allen Ausgängen neu) und eine Schnorr-Prüfung. Weder Zahl noch Eindeutigkeit der Signer-Einträge noch die Zahl der Ausgänge werden vorher begrenzt.

**Beleg** (Test `a20b_precheck_cpu_mit_aufgeblasenem_plan`, gleiche Signatur-Bytes an jedem Eingang, p2pk-Signer 5 000-mal auf denselben Eingang, 700 zusätzliche Ausgänge):
```
A20b precheck: normal 172 µs; aufgeblasen (700 Ausgänge, 5000 Signer, Body 714 kB): 814 ms
```
Unter dem Körperlimit der Seite (768 kB). Der Aufruf wird abgelehnt, kostet aber 0,8 s CPU je Anfrage auf einem der zwei Wallet-Plätze; bei 20 je Minute und mehreren Absendern (IPv6 je /64) ist das ein Dauerbrenner, aber kein Ausfall. **Vorschlag:** Vor `read_sigs` verlangen: `signers.len() ≤ tx.inputs.len()`, Indizes eindeutig und aufsteigend, Zahl der Ausgänge plausibel (die Aktionen von ghostctl haben ≤ 6); `SigHashReusedValues` einmal je Tx wiederverwenden.

### A20b-6 (Hinweis): Grundzins und Messfenster, Grundzins und Vertragspause

`rate_plan` kehrt im Grundzins-Pfad vor `rate::measure`/`record` zurück (2680–2693). Bis der Zins 2 % erreicht (vom Start bei 0 % mindestens 4 Stunden), sammelt die Zinsregel keine Messungen; danach dauert es weitere ≥ 45 min, bis sie reagieren darf. Außerdem: Fällt der Grundzins-Schritt in eine Runde mit Preis-Update, merkt `rate_reserve` die Stunde vor (2814), `fit_rate` liefert aber „diesmal nur der Preis“, wenn die DAA-Pause des Vertrags noch läuft (2880–2882); weil gesendet wurde, wird nicht freigegeben, und der Schritt verschiebt sich um eine Stunde. Beides nur Verzögerung, keine falsche Änderung. **Vorschlag:** Messung immer aufnehmen; die Vormerkung erst nach `fit_rate` setzen, wenn der Zins tatsächlich mitgeht.

---

## Geprüft und sauber

- `submit`: Neubau bitgleich, nur Neubau weiter verwendet; Signaturen gegen den Neubau; dritter Bau mit Signaturen und Konsensprüfung; Gebühr/Wechselgeld in Messkopie und echter Tx gleich; eigene UTXOs am Node geprüft (zweimal); Coinbase abgelehnt; Payload nur Länge (eine Zeichenprüfung gibt es nur im build-Pfad – unschädlich, der Payload liegt nur auf der Kette).
- Zustände: Folgezustände nur aus dem Neubau unter Sperre, Übernahme nur über das Journal bei Annahme; Tresor-Datei und Zinsdatei von keinem öffentlichen Pfad direkt beschreibbar; `receive --owner` übernimmt nur UTXOs mit GHOST-Covenant-ID und exaktem Skript (A19 hält).
- Frühere Behebungen halten: A17-1 (zufälliger Ersatzschlüssel, Neutralisierung), A17-4 (`precheck` vor Sperre), A17-6 (`send_then_wait`), A18 G-2 (60-s-Frist, bis auf A20b-3), A19-1/2/3/6/7 (Tests grün, Verdrahtung gelesen).
- Agent: nie mit Verlust, nie Gebühr für fremde Wallet-Tresore, Sprungschutz, ×2/÷2, Grundzins-Schritte im Vertragsrahmen, Takt-Grenzen, alle Schleifen begrenzt, Sperrenreihenfolge, Zeitlimits lassen Sperren fallen.
- Nebenläufigkeit: zwei Prozesse auf derselben Datei (Agent + Terminal/Seite) sind über `flock` und Journal konsistent; Zinsdatei mit eigener Sperre, „einmal je Stunde“ prozessübergreifend; `oracle-feed` neben `agent` erzeugt höchstens abgelehnte Doppel-Updates.
- Rechenkern: siehe Frage 4.
- Preis: siehe Frage 5.

## Grenzen

Kein Netz, kein Node, keine Messung am Server: Laufzeiten (A20b-1) sind aus dem Code und üblichen Latenzen abgeleitet. `store::resync` und `Net` sind nicht attrappierbar; die Zahl der Abfragen je Token ist aus `follow`/`exists` abgelesen. Web-Schicht (Teil a) nicht Gegenstand; dort gilt das Kontingent von 20/min je Absender als einzige Bremse für A20b-1 und A20b-4.

## Anhang: Prüftests (entfernt)

`protocol/tests/audit20b_pruef.rs`, Helfer wie `wallet_ops_tests.rs` (`World`, `kasware_sign`). Kern:

```rust
// A20b-1: Liste wächst je Überweisung
let before = w.dep.tokens.len(); let bal0 = w.sim.balance(&u);
for _ in 0..25 { w.by_wallet("transfer", &u, Action::Transfer { to: addr(&key()), ghost: 1, payload: vec![] }); }
assert_eq!(w.dep.tokens.len(), before + 25);   // Gebühren 0,1532 KAS, 25 KAS gebunden

// A20b-2: Grundzins-Pfad fragt nur wait_secs
let mut l = rate::RateLog::new("mainnet"); l.last_change = Some(now + 10 * 365 * 86_400);
for i in 0..5 { assert_eq!(l.wait_secs(now + i * 3600), rate::EVERY_SECS); }
assert!(math::rate_floor_step(0.0).is_some()); l.prune(now); assert_eq!(l.wait_secs(now + EVERY_SECS), 0);

// A20b-3: Zukunfts-mtime
store::write_pending(&state, "GHOST senden (Wallet)", &tx, &spks, None, Some(state.clone()), Some(next), true)?;
assert!(resolve_pending_with(&net_mit_sichtbarem_eingang, &state, 3600 s).await.is_err()); // warten
File::open(pending).set_modified(SystemTime::now() + 300 s)?;
assert!(matches!(resolve_pending_with(..).await, Ok(Some(m)) if m.contains("nicht angenommen")));

// A20b-4: precheck mit 5 000 p2pk-Signer-Einträgen auf Eingang 0, 700 Ausgängen, 65-Byte-Push je Eingang
let t = Instant::now(); let rep = wo::precheck(&big, &signed_tx, NET, P)?; // 814 ms, Body 714 kB

// sauber: Grundzins-Schritte vs. Vertrag
while let Some(next) = math::rate_floor_step(apr_of(cur.stable_rate)) {
    let r = rate_from_apr(next).clamp(cur.stable_rate - p.rate_step, cur.stable_rate + p.rate_step);
    ops::check_update(&p, &cur, cur.kas_usd, cur.last_rate_daa + p.rate_gap_daa, r)?; …
}
```
Lauf: `cargo test --release --test audit20b_pruef -- --nocapture`, 5 passed.
