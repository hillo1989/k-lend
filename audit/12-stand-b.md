# Audit 12, Stand Gruppe B: Vault, Agent und Zinsregel

Stand 29.09.2026, Branch `fix12b` auf Basis `8b6ee75`. Grundlage ist der Bericht von Audit 12 (Opus). Zweite Runde nach der Nachprüfung der Behebungen: Abschnitt „Nachprüfung, Runde 2“ am Ende. Keine Transaktionen gesendet, `keys/` und `deployments/` nicht gelesen, Verträge (`contracts/*.sil`) unverändert.

Befunde dieser Gruppe: A12-2, A12-3, A12-5, A12-12, A12-13 (dokumentiert), A12-14, A12-15 und der Vault-Teil von A12-18.

Ausführen:

```
cd protocol && CARGO_TARGET_DIR=../target-fix12b cargo test --release --offline
cd app && npx vitest run && npx tsc --noEmit -p . && npm run build
```

## Übersicht

| ID | Stand | Wo | Tests (scheitern ohne die Behebung) |
|---|---|---|---|
| A12-2 | **behoben** | `math.rs` (`SWEEP_MIN_TREASURY`, `sweep_allowed`, `sweepable`), `ops.rs` (`sweep`, `sweep_candidates`), `ghostctl.rs` (`keeper_round`, `keeper_sweep`, `sweep_in_turn`, `sweep`, `status_json`), `vaultMath.ts`, `precheck.ts` | `sweep_grenze_tests.rs`: `a12_messung_kleinste_aufloesbare_sicherheit`, `a12_kleiner_zombie_blockiert_das_aufloesen_nicht`; `ghostctl` (bin): `a12_keeper_versucht_alle_aufloesbaren_der_reihe_nach`; `audit12-vault.test.ts` (5 Tests, A12-2) |
| A12-3 | **behoben** | `rate.rs` (`prune`, `reserve`) | `rate::tests::a12_uhr_einmal_vorgestellt_friert_den_takt_nicht_ein` |
| A12-5 | **behoben** | `ghostctl.rs` (`Takt`, `side_step`, `agent_cycle`, Agentenschleife) | `ghostctl` (bin, angehaltene Uhr): `a12_nebenschritte_hungern_orakel_und_keeper_nicht_aus`, `a12_pause_verkuerzt_sich_um_die_nebenschritte`, `a12_langsamer_abgleich_zahlt_trotzdem`, `a12_haengende_vorbereitung_endet_nach_dem_alten_limit`, `a12_standardtakt_und_startskript` |
| A12-12 | **behoben** | `rate.rs` (`MIN_SPAN_SECS`, `Decision::TooShort`, Modulkopf), `ghostctl.rs` (`rate_plan`) | `rate::tests::a12_sechs_messungen_in_zwanzig_minuten_reichen_nicht`, `a12_zwanzig_minuten_manipulation_bewegen_den_zins_nicht` |
| A12-13 | **dokumentiert** | hier, Test in `vault_tests.rs` | `a12_tresor_zahlung_an_die_kasse_deckt_zugleich_den_zins` (hält den Befund fest, Schließen und Auflösen) |
| A12-14 | **behoben** (Rust, Seite); Vertragsgrenze **dokumentiert** | `math.rs` (`interest_fee`, `interest_fee_checked`), `ops.rs` (`close`), `ghostctl.rs` (`close`: Meldung erst nach dem Bau), `precheck.ts` | `vault_tests.rs`: `a12_zinsgebuehr_rechengrenze_bei_tiefstpreis`; `sweep_grenze_tests.rs`: `a12_schliessen_an_der_rechengrenze_meldet_den_grund`; `audit12-vault.test.ts` (Vorprüfung; der zweite Test sichert `vaultMath`, das schon vorher wie der Vertrag abbrach) |
| A12-15 | **behoben** | `vault_tests.rs` | `a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault` (M04), `a12_aufloesen_grenze_auf_ein_sompi_genau` (M08), `a11_drei_ghost_ausgaenge_bei_ruecknahme_zweitsicherung` (umbenannt) |
| A12-18 (Vault) | **behoben** (Seite) | `precheck.ts` | `audit12-vault.test.ts` (2 Tests, A12-18); `e2e_tests.rs`: `a12_ruecknahme_mit_winziger_auszahlung_baut` |

## A12-2: Zombie-Vault unter 0,1 KAS blockiert das Auflösen

**Vorher.** `ops::sweep` rechnete `value − SWEEP_FEE` in u64. Unter 0,1 KAS lief das über; im Release-Build kam ein riesiger Ausgang heraus, den die Skript-Engine mit `NumberTooBig` ablehnte. `keeper_round` nahm nur den ersten auflösbaren Vault (`position`), also scheiterte jede Runde am selben Vault. Die Seite zeigte „−0,05 KAS gehen an die Zinskasse“.

**Gemessen** (`a12_messung_kleinste_aufloesbare_sicherheit`, Bisektion auf 1 000 sompi, nur Bau ohne Einreichen). Der Vertrag lässt jede Sicherheit zu, deren Zins sie aufzehrt; Grenze ist allein die Speichermasse des Kassen-Ausgangs (Blockgrenze 500 000 g):

| Geldbörse des Auslösers | baubar ab Sicherheit | davon an die Kasse | Speichermasse bei 0,125 KAS |
|---|---|---|---|
| keine KAS | 0,11954 KAS | 0,01954 KAS | 388 200 g |
| eine UTXO mit 1 000 KAS | 0,12015 KAS | 0,02015 KAS | 403 764 g |
| 8 UTXOs à 0,3 KAS | 0,11648 KAS | 0,01648 KAS | 293 132 g |
| eine UTXO mit 0,155 KAS (Wechselgeld knapp 0,2 KAS, ungünstigster Fall) | 0,12126 KAS | 0,02126 KAS | 429 551 g |

Bis einschließlich `SWEEP_FEE` gibt es gar keinen Kassen-Ausgang.

**Behebung.**
- `math::SWEEP_MIN_TREASURY` = 0,025 KAS (Abstand zur gemessenen Grenze, gleich `SMALL_OUTPUT` der Seite).
- `math::sweep_allowed` ist die reine Vertragsregel. `math::sweepable` verlangt zusätzlich Sicherheit ≥ `SWEEP_FEE + SWEEP_MIN_TREASURY` (0,125 KAS). `status_json` und der Keeper nutzen `sweepable`.
- `ops::sweep` prüft `sweep_allowed` und rechnet mit `checked_sub`: bis `SWEEP_FEE` Fehler „zu klein zum Auflösen“, knapp darüber meldet der Bau „zu groß für einen Block“. `ghostctl sweep` baut also alles, was sich wirklich bauen lässt; nur der Agent und die Seite halten den Abstand ein.
- `ops::sweep_candidates` liefert alle Kandidaten (nicht veraltet, auflösbar zu Orakel- und Marktpreis). `keeper_sweep` versucht sie der Reihe nach (`sweep_in_turn`), bis einer gesendet ist; nach einem Senden (auch ohne Bestätigung) endet die Runde, weil die Orakel-UTXO verbraucht ist.
- `ghostctl sweep` und die Keeper-Meldung nennen den Kassenbetrag aus der gebauten Tx statt `value − SWEEP_FEE`.
- Seite: `vaultMath.ts` spiegelt `SWEEP_MIN_TREASURY`, `sweepAllowed` und `sweepable`; `simSweep` meldet einen Fehler statt eines negativen Betrags. `vaultSweepable` schließt zu kleine Vaults aus, auch wenn ein älteres ghostctl `sweepable: true` liefert; die Vorprüfung sagt „zu klein zum Auflösen“. Kommentar zu `SWEEP_FEE` (stand 0,01 KAS) korrigiert.
- Text (Runde 2): Die 0,1 KAS heißen nicht mehr „Netzgebühr“. Es ist der Anteil, den der Vault für das Auflösen trägt: Die Netzgebühr (≈ 0,055 KAS) geht davon ab, den Rest bekommt, wer auflöst.

**Tests.** `a12_kleiner_zombie_blockiert_das_aufloesen_nicht` baut fünf Zombie-Vaults im Simulator (0,09 / 0,11 / 0,125 / 0,2 / 0,3 KAS). Die beiden kleinen gelten als nicht auflösbar und liefern saubere Fehler. `sweep_candidates` enthält die drei übrigen in Reihenfolge. Alle drei werden nacheinander aufgelöst und angenommen (Netzgebühr je 0,0545 KAS, Speichermasse 403 764 / 103 764 / 53 764 g), die Kasse bekommt jeweils Sicherheit − 0,1 KAS. Ohne die Behebung meldet `math::sweepable` den 0,09-KAS-Vault als auflösbar (erste Prüfung scheitert). Die Seite prüft das in `audit12-vault.test.ts`, einschließlich der Grenze auf den sompi genau.

**Schleife des Keepers (Runde 2).** Die Reihenfolge steckt jetzt in `sweep_in_turn`; `keeper_sweep` gibt nur Bau und Senden hinein. `a12_keeper_versucht_alle_aufloesbaren_der_reihe_nach` prüft drei Fälle: Der erste scheitert ohne Sendung, dann kommt der zweite dran. Scheitern alle, wurden alle versucht. Wurde gesendet, aber nicht bestätigt, endet die Runde. Mit nur dem ersten Kandidaten (`.take(1)`) ist der Test rot.

## A12-3: Zinstakt friert nach vorgestellter Uhr ein

**Vorher.** `wait_secs` wartete bei `last_change > now` immer eine volle Stunde, ohne `last_change` je zu ändern. Nach einem Lauf mit Uhr +10 Jahre wartete die Zinsregel also bis in zehn Jahren.

**Behebung.** `RateLog::prune` (läuft bei jeder Messung) kappt eine letzte Zinsänderung aus der Zukunft auf `now`. `reserve` ruft `prune` ebenfalls auf. Danach wartet die Zinsregel noch eine Stunde ab dem Moment, in dem die Uhr wieder stimmt. `decide` bleibt ohne Seiteneffekt und wartet bei Zeiten aus der Zukunft weiter vorsichtig.

**Test.** `a12_uhr_einmal_vorgestellt_friert_den_takt_nicht_ein`: Vormerkung bei +10 Jahren, dann Korrektur. Die erste Messung kappt auf jetzt (`Wait { 3600 }`), nach einer Stunde regelmäßiger Messungen folgt `Change`. Ohne Kappung bleibt `last_change` in der Zukunft (erste Prüfung scheitert).

## A12-12: sechs Messungen in 20 Minuten

**Vorher.** Mit `SAMPLE_GAP_SECS` = 240 s lagen 6 Messungen in 20 min, 4 manipulierte genügten. Der Modulkopf behauptete „über die Hälfte der Stunde“.

**Behebung.** Neu `MIN_SPAN_SECS` = 2 700 s: Älteste und jüngste Messung im Fenster müssen mindestens 45 min auseinanderliegen, sonst `Decision::TooShort { have, span_secs }`. `rate_plan` meldet das („erst über N min verteilt, nötig 45 min“). Modulkopf richtiggestellt.

**Rechnung.** Bei regelmäßigen Messungen alle 240 s liegen bei der ersten zulässigen Entscheidung mindestens 13 Messungen im Fenster (48 min). Für den Median braucht ein Manipulator mehr als die Hälfte, also mindestens 7 Messungen, und muss den Kurs dafür mindestens 24 min halten.

**Tests.**
- `a12_sechs_messungen_in_zwanzig_minuten_reichen_nicht`: der Fall aus dem Bericht ergibt `TooShort { have: 6, span_secs: 1200 }`.
- `a12_zwanzig_minuten_manipulation_bewegen_den_zins_nicht`: zwei Stunden Messungen alle 240 s, 20 min Manipulation an jeder Startminute der ersten Stunde, nie `Change`. Gegenprobe: 30 min gehalten ändern den Zins.
- Die bestehenden Tests (`mindestens_sechs_messungen_im_fenster`, `median_statt_einzelmessung`, `takt_hoechstens_einmal_je_stunde`) messen jetzt im Abstand 9 min, damit sechs Messungen 45 min abdecken.

**Rest (Info).** Fallen Messungen aus (Agent aus, Pool unter 10 GHOST), zählen nur die vorhandenen, Lücken gewichtet der Median nicht. Bei sehr lückenhaften Messungen genügen daher weiter weniger als 24 min. Dagegen hülfe ein zeitgewichteter Median; das ist hier nicht umgesetzt.

**Außerhalb meiner Dateien, nicht geändert:** `HowItWorks.tsx` und `Faq.tsx` sagen „ab 6 Messungen“. Das stimmt weiter, nennt aber die 45 min nicht. Ebenso der Kopf von `GHOST-Agent starten.command`.

## A12-5: Taktung der Agentenschleife

**Vorher.** Der Ablauf war streng nacheinander: Orakel und Keeper bis 240 s, Daueraufträge bis 700 s, Tresore bis 700 s, dann `interval`. Zwischen zwei Orakel- und Liquidationsrunden lagen so bis zu 1 760 s. Die 700 s waren für das Warten auf Bestätigungen bemessen (`store::wait_accepted`: bis 600 s je Zahlung, solange die Tx im Mempool liegt).

**Neue Taktung** (`Takt`, `side_step`, `agent_cycle`; Runde 2 nach der Nachprüfung):
1. Jeder Durchlauf beginnt mit Orakel und Keeper, Zeitlimit 240 s (wie bisher).
2. Danach Daueraufträge, dann Tresore. **Ab seiner ersten Sendung hat jeder Schritt noch 90 s**, dann geht es weiter. Die Sendung erkennt `side_step` am Zähler der gesendeten Tx (`sent_count`, sekündlich nachgesehen).
3. **Verbindung, Sperre, Journal-Klärung und Abgleich vor der ersten Sendung zählen nicht zu den 90 s.** Für sie gilt wie vor Audit 12 höchstens 700 s je Schritt.
4. Beide Schritte kommen in jedem Durchlauf dran; hängt der erste, läuft der zweite trotzdem.
5. Die Schritte nutzen die Wartezeit: Die Pause bis zum nächsten Durchlauf ist `interval` minus ihre Dauer (mindestens 0).

**Abstände zwischen dem Beginn zweier Orakel-/Keeper-Runden:**

| Lage | `--interval 120` (Standard von ghostctl) | `--interval 300` (Startskript `GHOST-Agent starten.command`) |
|---|---|---|
| nichts fällig | Runde + 120 s | Runde + 300 s |
| beide Schritte senden und warten auf Bestätigungen | 240 + max(2 · 90, 120) = **420 s** | 240 + max(2 · 90, 300) = **540 s** |
| dazu langsame Verbindung oder langer Abgleich | zuzüglich dieser Zeit | ebenso |
| Obergrenze (Runde 2: beide Schritte kommen schon vor der ersten Sendung 700 s nicht weiter) | 240 + 1 400 = 1 640 s | 1 640 s |
| **Obergrenze (Restpunkte, Runde 3): Vorbereitung bis 240 s, danach 90 s** | 240 + 2 · (240 + 90) = **900 s** | 900 s |
| vor Audit 12 | bis 1 760 s | bis 1 940 s |

Die Vorbereitung hat seit Runde 3 ein eigenes Limit von 240 s (Restpunkt B-P4, siehe „Restpunkte“ am Ende); die Punkte 3 und die Tests unten beschreiben Runde 2.

In der ersten Runde stand hier „420 s Standard“. Das gilt nur für `ghostctl agent` ohne `--interval`. Das Startskript startet mit `--interval 300`, dort sind es 540 s. Die Commit-Nachricht von `4b99193` nennt noch 420 s.

Die Obergrenze ist praktisch kaum erreichbar. `Net::connect` gibt nach höchstens drei Runden mit eigenen Zeitlimits auf (Resolver 8 s, bis zu 6 Nodes à 20 s). Jeder RPC-Aufruf hat 60 s (workflow-rpc, `timeout_duration`). Schlägt der erste Aufruf fehl, endet der Schritt mit einem Fehler.

**Warum nicht mehr das feste 90-s-Limit aus Runde 1** (Nachprüfung, „A12-5-Zeitbudget“): Es umfasste auch Verbindung, Sperre, `resolve_pending` und den Tresor-Abgleich. Dauerte das länger als 90 s, brach der Schritt in jedem Durchlauf an derselben Stelle ab. Zahlungen dahinter kamen dann nie dran. Jetzt bleibt für die Vorbereitung das alte Limit, und nur das Warten nach dem Senden ist kurz. Das Warten war der eigentliche Grund der langen Abstände.

**Ein unterbrochener Schritt ist harmlos**, wie schon beim alten 700-s-Limit:
- Das Journal (`store::write_pending` vor dem Senden, Abo-Journal mit `inflight`) hält die gesendete Tx fest.
- Der nächste Durchlauf klärt sie (`resolve_pending` im Abo- und im Tresor-Pfad). Liegt sie noch im Mempool, heißt es „noch unterwegs“, und es wird nicht neu gesendet. Dieser Durchlauf ist dann schnell vorbei, das kurze Limit greift also nicht wiederholt.
- Die übrigen Zahlungen folgen im nächsten Durchlauf.
- Doppelt gesendet wird nicht (Journal bzw. Covenant-UTXO der Tresore; so auch die Nachprüfung).

**Bekannte Grenze.** `store::lock` wartet blockierend (bis 60 s, `std::thread::sleep`). Während dieser Wartezeit kann keines der Zeitlimits greifen, der Schritt dauert entsprechend länger. Das gilt auch für die Runde, und es war schon vor Audit 12 so. `store.rs` gehört nicht zu dieser Gruppe. Abhilfe wäre das Warten in `spawn_blocking`.

**Nicht umgesetzt: eine gemeinsame Verbindung je Durchlauf.** Das würde die Signaturen von `abo_agent_step` und `tresor_agent_step` ändern. Nötig ist es nicht mehr, weil die Verbindung nicht mehr zum kurzen Limit zählt. Die Schritte verbinden sich nur, wenn offline etwas fällig ist.

Die inneren 700-s-Limits in `abo_agent_step` und `tresor_agent_step` sind unverändert. Ich habe beide Funktionen nicht angefasst, um die Tresor-Abschnitte der anderen Gruppe nicht zu berühren.

**Tests** (Unit-Tests im Binary `ghostctl`, mit angehaltener Uhr: `#[tokio::test(start_paused = true)]`, dafür `tokio` mit Feature `test-util` als Dev-Abhängigkeit). Die Abstände sind damit auf die Sekunde genau und hängen nicht von der Last ab. Vorher prüfte ein Test mit echten Zeitgebern und 100 ms Spielraum.
- `a12_nebenschritte_hungern_orakel_und_keeper_nicht_aus`: Beide Schritte senden und warten dann für immer. Vier Durchläufe mit `--interval 120`, jede Runde beginnt nach genau 180 s, beide Schritte senden in jedem Durchlauf.
- `a12_pause_verkuerzt_sich_um_die_nebenschritte`: `--interval 300`. Schritte hängen nach dem Senden: 300 s (volle Pause: 480 s). Schritte fertig nach je 50 s: 300 s (voll: 400 s). Runde hängt: 540 s (voll: 720 s), gleich `usual_gap`.
- `a12_langsamer_abgleich_zahlt_trotzdem`: 300 s Vorbereitung, dann Sendung und 5 s Bestätigung. Der Schritt wird fertig und zahlt im selben Durchlauf. Wartet er danach für immer, endet er 90 s nach der Sendung (bei 390 s).
- `a12_haengende_vorbereitung_endet_nach_dem_alten_limit`: ohne Sendung endet jeder Schritt nach 700 s; `max_gap` = 1 640 s.
- `a12_standardtakt_und_startskript` (vorher `a12_standardtakt_hoechstens_sieben_minuten`): 420 s, 540 s, 840 s bei `--interval 600`.

## A12-13: feste Ausgangsindizes über Vertragsgrenzen (dokumentiert)

**Befund.**
- Der Vault verlangt die Kasse an Ausgang `eigener Eingang + 1` (`payTreasury`), der Tresor seine Zahlung an Ausgang `eigener Eingang` (`pay`).
- Steht der Vault an Eingang i und ein Tresor, der an die Zinskasse zahlt, an Eingang i + 1, erfüllt EIN Ausgang beide Verträge.

**Wer verliert.** Die Zinskasse. Der Tresor zahlt wie vereinbart (Betrag, Termin, Empfänger stimmen). Der Zins des Vaults fehlt aber, bis zur Höhe der Tresor-Zahlung.

**Wer es kann.**
- **Schließen** (`close`): nur der Vault-Besitzer (Signatur). Er nimmt die ganze Sicherheit.
- **Auflösen** (`sweep`): **jeder**, ohne Signatur. Ein Fremder nimmt die Sicherheit eines Zombie-Vaults, die sonst an die Kasse ginge.
- Der Test `a12_tresor_zahlung_an_die_kasse_deckt_zugleich_den_zins` zeigt beide Fälle; alle drei Eingänge werden angenommen.

**Wann relevant.** Nur wenn
- ein Tresor an die Zinskasse zahlt,
- dieser Tresor gerade fällig ist (Auslösen darf jeder, aber erst ab dem Termin),
- und seine Zahlung mindestens die Zinsgebühr (Schließen) bzw. Sicherheit − 0,1 KAS (Auflösen) erreicht.

Heute ist die Zinskasse ein Schlüssel des Betreibers, an den niemand per Tresor zahlt. Deshalb nur Info.

**Nur Tresore, keine Daueraufträge** (berichtigt nach der Nachprüfung, Restpunkt B-P6). Ein Dauerauftrag ist eine gewöhnliche, vom Auftraggeber signierte Tx. Ein Dritter kann sie nicht mit einem Vault-Eingang zu einer Tx kombinieren. Ein Tresor dagegen darf jeder auslösen, sobald er fällig ist. Einen eigenen Tresor an die Kasse anzulegen, bringt einem Angreifer nichts: Er zahlt die Kasse dann selbst.

**Empfehlung.**
- Keine Tresore an die Adresse der Zinskasse. (Die erste Fassung nannte hier auch Daueraufträge; das war überflüssig, siehe oben.)
- `ghostctl tresor open` könnte die Zinskasse als Empfänger ablehnen. Das liegt im Tresor-Abschnitt der anderen Gruppe und ist hier nicht umgesetzt.

**Vertragsänderung nötig?** Nein, solange die Empfehlung gilt. Eine Behebung im Vertrag müsste den Kassen-Ausgang einem Vertrag eindeutig zuordnen. Ein anderer Vertrag mit festem Index ließe sich damit nicht ausschließen. Möglich wäre etwa ein Kassen-Ausgang, der die Covenant-ID des Vaults trägt; das wäre ein neues Format der Zinskasse. Für einen Info-Befund ist das unverhältnismäßig.

## A12-14: Rechengrenze der Zinsgebühr

**Vorher.** `math::interest_fee` rechnete in u128 und schnitt mit `as i64` ab: Bei 1 Mio USD Zins zum Tiefstpreis kam −8 446 744 073 709 551 616 heraus.

**Behebung.**
- `math::interest_fee` rechnet sättigend (höchstens die Sicherheit, nie negativ).
- Neu `math::interest_fee_checked`: gibt `None`, genau wenn der Vertrag überläuft (⌈Zins·1e8/Preis⌉ > i64::MAX).
- `sweep_allowed` (und damit `sweepable`) nutzt diese Prüfung, folgt also dem Vertrag.
- `ops::close` meldet die Grenze verständlich, statt eine Tx zu bauen, die an `NumberTooBig` scheitert. Test (Runde 2): `a12_schliessen_an_der_rechengrenze_meldet_den_grund` baut `close` beim Tiefstpreis. Mit 1 Mio USD Zins und genau eine Einheit über der Grenze kommt die Meldung. Genau an der Grenze baut Schließen, und die ganze Sicherheit geht an die Kasse.
- `ghostctl close` nennt „Zins an die Zinskasse“ erst nach dem Bau. Vorher stand dort an der Grenze die (gesättigte) ganze Sicherheit, bevor die Fehlermeldung kam.
- Seite: `vaultMath.interestFee` bricht schon wie der Vertrag ab (`OverflowError`), negative Werte gab es dort nie. Neu sagt die Vorprüfung bei Schließen und Auflösen „zu groß für die Rechnung des Vertrags“. Vorher schwieg sie beim Schließen, und beim Auflösen nannte sie den falschen Grund („Zins kleiner als die Sicherheit“).

**Vertragsgrenze (nur dokumentiert).**
- `interestFee` = `mulDivUp(accrued, 1e8, kasUsd)` bricht ab, sobald ⌈accrued·1e8/kasUsd⌉ > i64::MAX.
- Die Grenze folgt nicht aus `MAX_DEBT`/`MAX_INTEREST`, sondern hängt vom Preis ab: beim Tiefstpreis 0,00001 USD (kasUsd = 1 000) ab 92 233 720 368 548 Einheiten, also ≈ 922 337 USD Zins. Bei 0,05 USD liegt sie bei ≈ 4,6 Mrd. USD.
- Dann scheitern `close` und `sweep`, bis der Preis steigt. Mit 50 GHOST Höchstschuld je Vault im Mainnet ist das unerreichbar.
- Keine Vertragsänderung nötig.

**Tests.** `a12_zinsgebuehr_rechengrenze_bei_tiefstpreis` prüft Rust und Vertrag auf die Einheit genau an der Grenze: 92 233 720 368 547 geht in beiden, eine Einheit mehr in keinem. Dazu kommen 1 Mio USD (Vertrag lehnt ab, Rust sättigt auf die Sicherheit) und 900 000 USD (geht). Ohne die Behebung ist `interest_fee` negativ (erste Prüfung scheitert). Seite: `audit12-vault.test.ts`.

## A12-15: Testlücken

- **`noGhost()` in `sweep` (M04):** Test `a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault` aus der Nachprüfung übernommen. KCC20 allein ließe das Prägen zu (`r[2]` ok), der Vault lehnt ab.
- **Grenze auf 1 sompi (M08):** neuer Test `a12_aufloesen_grenze_auf_ein_sompi_genau`. Zehrt der Zins die Sicherheit genau auf, geht es. Bei 1 sompi mehr Sicherheit (Zinsgebühr = Sicherheit − 1) lehnt der Vault ab. Der Rust-Spiegel zieht dieselbe Grenze.
- **A11-V-7 ehrlich benannt:**
  - `a11_drei_ghost_ausgaenge_bei_ruecknahme` heißt jetzt `a11_drei_ghost_ausgaenge_bei_ruecknahme_zweitsicherung`.
  - Der Kommentar sagt: Drei GHOST-Ausgänge lehnt der Vault auch ohne `require(nOut <= MAX_GHOST_OUTS)` ab (Schleifenwächter), ebenso der KCC20-Leader (jetzt mitgeprüft). Die Regel ist eine Zweitsicherung, M03 überlebt.
  - **AUDIT.md nennt noch den alten Namen**; beim nächsten Nachtrag dort bitte ändern und „Zweitsicherung“ vermerken.
- **Irreführender Kommentar** in `a11_auflösen_nur_ohne_schuld_und_wenn_der_zins_alles_aufzehrt` („heimliches Prägen … scheitert an noGhost“ über einer Gegenprobe ohne Prägen) richtiggestellt.
- Aus der Nachprüfung zusätzlich übernommen: `a12_kasse_direkt_hinter_dem_vault_auch_an_eingang_1` (A11-V-1 mit dem Vault an Eingang 1) und `execute_lt` (Locktime für Tresor-Eingänge).

**Gegenproben.** Skript `…/scratchpad/fix12b-mut.py`: je Probe genau eine Stelle ändern, die Tests des Befunds laufen lassen, danach byte-gleich wiederherstellen. Die Vertragsmutanten liefen nur in einer Kopie unter `…/scratchpad/fix12b-mut/`; der Vertrag im Worktree ist unverändert.

| Probe | Änderung | Ergebnis |
|---|---|---|
| M03 | `require(nOut <= MAX_GHOST_OUTS)` → `true` | überlebt (Zweitsicherung, siehe oben) |
| M04 | `noGhost()` in `sweep` entfernt | **rot**: `a12_aufloesen_mit_heimlichem_praegen_scheitert_am_vault` |
| M08 | `interestFee == coll` → `>= coll − 1` | **rot**: `a12_aufloesen_grenze_auf_ein_sompi_genau` |
| R1 (A12-3) | Kappung in `prune` entfernt | **rot**: `a12_uhr_einmal_vorgestellt_friert_den_takt_nicht_ein` |
| R2 (A12-12) | `MIN_SPAN_SECS` = 0 | **rot**: `a12_sechs_messungen…`, `a12_zwanzig_minuten…` (und `a12_uhr…`) |
| R3 (A12-14) | `interest_fee` wieder mit `as i64` | **rot**: `a12_zinsgebuehr_rechengrenze_bei_tiefstpreis` |
| R4 (A12-2) | `sweepable` ohne Mindestbetrag | **rot**: `a12_kleiner_zombie_blockiert_das_aufloesen_nicht` |
| R5 (A12-2) | `ops::sweep` mit ungeprüfter u64-Subtraktion | **rot**: `a12_messung_kleinste_aufloesbare_sicherheit` |
| R6 (A12-5) | Schritt-Zeitlimit wieder 700 s | **rot**: `a12_standardtakt_hoechstens_sieben_minuten` |

Seite, gegen die alten Fassungen von `precheck.ts` bzw. `vaultMath.ts` (ohne Mindestbetrag) gelaufen: 5 der 8 Tests in `audit12-vault.test.ts` rot. Grün blieben die Konstanten, die Gegenprobe bei 198 KAS und der A12-14-Rechentest; `vaultMath` brach schon vorher wie der Vertrag ab.

**Gegenproben, Runde 2** (Skript `…/scratchpad/fix12b-r2/rueckbau.py` bzw. `rueckbau_app.py`, Protokolle daneben; jede Datei danach byte-gleich wiederhergestellt):

| Probe | Änderung | Ergebnis |
|---|---|---|
| R7 (A12-5) | ohne Sendelimit, nur 700 s je Schritt | **rot**: `a12_nebenschritte…`, `a12_pause…`, `a12_langsamer…` |
| R8 (A12-5) | volle Pause `sleep(interval)` statt `interval` − Dauer der Schritte | **rot**: `a12_nebenschritte…`, `a12_pause…`, `a12_langsamer…`, `a12_haengende…` |
| R9 (A12-5) | festes 90-s-Limit ab Beginn des Schritts (Stand Runde 1) | **rot**: u. a. `a12_langsamer_abgleich_zahlt_trotzdem` (nichts gesendet) |
| R10 (A12-2) | `sweep_in_turn` nur erster Kandidat (`.take(1)`) | **rot**: `a12_keeper_versucht_alle_aufloesbaren_der_reihe_nach` |
| R11 (A12-14) | `ops::close` ohne Grenzprüfung | **rot**: `a12_schliessen_an_der_rechengrenze_meldet_den_grund` |
| R12 (Text) | Seite wieder „Nach der Netzgebühr von 0,1 KAS“ | **rot**: `audit12-vault.test.ts`, „0,1 KAS heißen nicht Netzgebühr …“ |

## A12-18, Vault-Teil: winzige Auszahlung bei der Rücknahme

**Befund nachgeprüft.**
- Anders als beim Schließen und Abheben (A11-O-12) gibt es bei der Rücknahme **keinen eigenen Ausgang** für den Rücknehmer. `burn_op` legt die KAS in sein Wechselgeld.
- Eine winzige Auszahlung macht die Tx deshalb nicht zu schwer.
- `a12_ruecknahme_mit_winziger_auszahlung_baut`: 0,001 GHOST, die ganze Schuld, ergeben 0,02475 KAS. Die Tx baut und wird angenommen (Speichermasse 12 120 g), kein Ausgang in dieser Höhe. Die Netzgebühr (0,0634 KAS im Simulator) ist aber größer als die Auszahlung.

**Behebung (Seite).** Die Vorprüfung warnt, wenn die Auszahlung unter der Gebührenreserve (0,06 KAS) liegt: „Die Rücknahme zahlt nur … KAS aus, die Netzgebühr dafür liegt bei etwa 0,06 KAS. Unterm Strich zahlst du womöglich drauf.“ Eine Warnung „zu schwer“ wäre hier falsch.

**Tests.** `audit12-vault.test.ts`: Warnung bei 0,0198 KAS, keine bei 198 KAS. Ohne die Behebung fehlt die Warnung.

## Nicht übernommen aus der Nachprüfung

- `review12_rate.rs`:
  - `beschaedigte_oder_leere_datei_sperrt_dauerhaft` und `sperre_prozessuebergreifend_mit_python_flock` beschreiben bestehendes, gewolltes Verhalten und sind kein Befund dieser Gruppe. Der zweite braucht außerdem `python3`.
  - `uhr_einmal_vorgestellt…` und `sechs_messungen_in_zwanzig_minuten_reichen` gingen umgekehrt in die `a12_*`-Tests von `rate.rs` ein.
- `review12_sweep.rs`: ersetzt durch `sweep_grenze_tests.rs` (gleiche Zombie-Erzeugung, mit Prüfungen statt Ausgabe).
- `review12_tresor.rs`, `standing_order_tests.rs`, `tresor_e2e_tests.rs` und die übrigen Seiten-Tests (`audit12-messages`, `audit12-tresor`, A12-W-2 bis A12-W-8 in `audit12.test.ts`): Befunde anderer Gruppen.

## Dateien außerhalb der Liste

- `protocol/Cargo.toml` (Runde 2): Abschnitt `[dev-dependencies]` mit `tokio` und dem Feature `test-util`. Damit laufen die Takt-Tests mit angehaltener Uhr. Es ist dieselbe Crate aus dem Lockfile, keine neue Abhängigkeit, und das Programm selbst ändert sich nicht. Die Alternative wäre eine eigene Uhr-Abstraktion in `agent_cycle` nur für Tests.
- `ghostctl.rs`, `Cmd::Close` (Runde 2): Die Meldung zum Zins kommt erst nach dem Bau. Das ist eine Folge von A12-14 (`close_fee` sättigt).

Sonst geändert: `protocol/src/{math,ops,rate}.rs`, `protocol/src/bin/ghostctl.rs` (nur Agentenschleife, `sweep`, `status_json`, `keeper_round`, `keeper_sweep`, `rate_plan`, neue `Takt`/`side_step`/`agent_cycle`/`sweep_in_turn` und Tests am Dateiende; dazu `Cmd::Close`, siehe oben), `protocol/tests/{vault_tests,e2e_tests}.rs`. Neu: `protocol/tests/sweep_grenze_tests.rs`. Seite: `app/src/lib/{vaultMath,precheck}.ts`, neu `app/src/lib/audit12-vault.test.ts`. `VaultList.tsx` und `Vault.tsx` brauchten keine Änderung: Sie fragen `vaultSweepable`, das zu kleine Vaults jetzt ausschließt.

## Testzahlen

Rust, `cargo test --release --offline`: **308 grün**, 2 ignoriert (`rest_live_tests`, braucht Netz). Basis `8b6ee75`: 290. Neu sind 18 Tests (13 aus Runde 1, 5 aus Runde 2):

| Suite | vorher | jetzt | neu |
|---|---|---|---|
| lib (`rate::tests`) | 49 | 52 | `a12_uhr…`, `a12_sechs…`, `a12_zwanzig…` |
| bin `ghostctl` | 0 | 6 | `a12_nebenschritte…`, `a12_pause…`, `a12_langsamer…`, `a12_haengende…`, `a12_standardtakt_und_startskript`, `a12_keeper_versucht…` |
| `e2e_tests` | 4 | 5 | `a12_ruecknahme_mit_winziger_auszahlung_baut` |
| `sweep_grenze_tests` (neu) | – | 3 | `a12_messung…`, `a12_kleiner_zombie…`, `a12_schliessen_an_der_rechengrenze…` |
| `vault_tests` | 81 | 86 | fünf `a12_*`; `a11_drei_ghost…` umbenannt |

Die übrigen Suiten sind unverändert grün: audit10_pool_engine 20, audit10_pool_regress 4, chain 3, factory 21, ghost_token 9, oracle 26, payload 7, pool_e2e 3, pool 32, standing_order 15, tresor_e2e 12, vault_math 4.

Seite: `npx vitest run` **233 grün** (Basis 224, neu 9 in `audit12-vault.test.ts`), `npx tsc --noEmit -p .` sauber, `npm run build` sauber (nur die übliche Warnung zur Chunk-Größe).

## Nachprüfung, Runde 2

Ein unabhängiger Prüfer hat die erste Runde beanstandet. Stand je Punkt:

| Punkt | Schwere | Stand | Was geändert ist | Test |
|---|---|---|---|---|
| A12-5-Zeitbudget | wichtig | **behoben** | Das 90-s-Limit gilt nur noch ab der ersten Sendung eines Schritts. Verbindung, Sperre, Journal-Klärung und Abgleich davor haben wie vor Audit 12 bis 700 s. Ein langsamer Abgleich bricht nicht mehr in jedem Durchlauf an derselben Stelle ab. Die blockierende Sperre ist als Grenze dokumentiert (siehe A12-5). | `a12_langsamer_abgleich_zahlt_trotzdem`, `a12_haengende_vorbereitung_endet_nach_dem_alten_limit` |
| A12-5-Pause-ungetestet | wichtig | **behoben** | Keine Code-Änderung nötig. Die Tests messen jetzt genau, und jede Obergrenze liegt unter der Summe aus Schritten und voller Pause. | `a12_nebenschritte…` (180 s statt 300 s), `a12_pause_verkuerzt_sich_um_die_nebenschritte` (300/300/540 s statt 480/400/720 s); Rückbau R8 rot |
| A12-2-Keeper-Schleife | klein | **behoben** | Die Schleife steckt in `sweep_in_turn`, `keeper_sweep` nutzt sie. | `a12_keeper_versucht_alle_aufloesbaren_der_reihe_nach`; Rückbau R10 rot |
| A12-14-close-Guard | klein | **behoben** | Test für die Prüfung in `ops::close`. `ghostctl close` nennt den Zins erst nach dem Bau. | `a12_schliessen_an_der_rechengrenze_meldet_den_grund`; Rückbau R11 rot |
| A12-5-Zeitmessungstest | klein | **behoben** | Angehaltene Uhr (`tokio::test(start_paused = true)`) statt echter Zeitgeber, unabhängig von der Last | alle Takt-Tests |
| Texte (a) | klein | **behoben** (hier) | 420 s gelten für `--interval 120`, das Startskript (`--interval 300`) hat 540 s. Die Tabelle unter A12-5 nennt beides. Die Commit-Nachricht von `4b99193` bleibt unverändert (kein Umschreiben der Historie). | `a12_standardtakt_und_startskript` |
| Texte (b) | klein | **behoben** | Seite und `vaultMath`: Die 0,1 KAS sind der Anteil, den der Vault für das Auflösen trägt. Davon geht die Netzgebühr ab (≈ 0,055 KAS), den Rest bekommt, wer auflöst. | `audit12-vault.test.ts`, „0,1 KAS heißen nicht Netzgebühr …“; Rückbau R12 rot |
| Texte (c) | klein | **offen, dokumentiert** | `HowItWorks.tsx`, `Faq.tsx` und der Kopf von `GHOST-Agent starten.command` nennen die 45-min-Spanne (A12-12) nicht. Sie sind unvollständig, aber nicht falsch. Die Dateien gehören nicht zu dieser Gruppe, `Faq.tsx` bearbeitet Gruppe A. | – |
| Texte (d) | klein | **offen, dokumentiert** | AUDIT.md nennt für A11-V-7 noch `a11_drei_ghost_ausgaenge_bei_ruecknahme`. Neu heißt der Test `a11_drei_ghost_ausgaenge_bei_ruecknahme_zweitsicherung`. AUDIT.md darf ich nicht bearbeiten; beim nächsten Nachtrag dort ändern. | – |

## Restpunkte (Runde 3)

Stand 30.09.2026, Branch `fix13b` auf Basis `a70fbbd` (v3 mit fix12a–c zusammengeführt). Grundlage sind die Restpunkte der unabhängigen Prüfer der Behebungen (Schlüssel „b“). Keine Transaktionen, `keys/` und `deployments/` nicht gelesen, Verträge unverändert.

| Punkt | Stand | Was geändert ist | Test (scheitert ohne die Behebung) |
|---|---|---|---|
| B-P1 (A12-2, Text) | **behoben** | `ops::sweep` nennt SWEEP_FEE nicht mehr „die Netzgebühr des Auflösens“: Vorab gehen 0,1 KAS für das Auflösen ab, davon die Netzgebühr (≈ 0,055 KAS), den Rest bekommt, wer auflöst; für die Zinskasse bliebe nichts. Gleicher Wortlaut wie die Seite. | `sweep_grenze_tests.rs`: `a12_bp1_zu_klein_meldung_nennt_sweep_fee_nicht_netzgebuehr` |
| B-P2 (A12-2, `sweep_in_turn`) | **behoben** | `sweep_in_turn` fragt nach jedem Fehlschlag auch das Journal der Zustandsdatei (`KeeperIo::journal_open`). Ist es offen (submit mehrdeutig gescheitert), endet die Runde wie nach einer Sendung. Liegt schon vor dem Bau ein Journal, wird nicht gebaut. Dasselbe Muster galt in der Liquidationsschleife; dort gilt jetzt dieselbe Prüfung. | `ghostctl` (bin): `a12_bp2_unklares_senden_beendet_das_aufloesen` |
| B-P3 (Verdrahtung) | **behoben** | `keeper_round` ist geteilt: `keeper_plan` (ohne Netz: Liquidationen mit Gewinn, `sweep` = ganze Liste aus `ops::sweep_candidates`) und `keeper_act` (Ablauf über die Schnittstelle `KeeperIo`; im Agenten `KeeperNet`, im Test eine Attrappe). Die Zeilen, die ohne Netz nicht laufen (Cmd::Agent → `agent_cycle`, Keeper auch ohne Komitee-Datei, `keeper_round` → `keeper_plan`/`keeper_act`, Plan → ganze Liste), prüft ein Quelltext-Test. | `a12_bp3_keeper_runde_loest_mit_der_ganzen_liste_auf` (Ablauf), `a12_bp3_verdrahtung_agent_und_keeper` (Quelltext); `a12_keeper_versucht_alle_aufloesbaren_der_reihe_nach` auf `KeeperIo` umgestellt |
| B-P4 (A12-5, Obergrenze) | **behoben** | `Takt.prep` = 240 s ersetzt `step_max` = 700 s: bis zur ersten Sendung höchstens 240 s (wie die Runde, die ebenfalls verbindet, sperrt und abgleicht), danach 90 s. Obergrenze zwischen zwei Runden **900 s** (Runde 2: 1 640 s, vor Audit 12: 1 760 s). Die Startmeldung nennt 240 s, 90 s und die Obergrenze. | `a12_bp4_vorbereitung_hat_ein_eigenes_kurzes_limit` (480 s ohne Sendung, 900 s im schlimmsten Fall, `max_gap` = 900 s); `a12_langsamer_abgleich_zahlt_trotzdem` jetzt mit 200 s Vorbereitung |
| B-P5 (A12-2, Seite) | **behoben** | Neu `vaultInterestEatsCollateral` (`precheck.ts`): Schuld 0 und Zins ≥ Sicherheit, auch unter 0,125 KAS. `VaultList.tsx` zeigt „Zins zehrt Sicherheit auf“ dann auch bei zu kleinen Zombie-Vaults, ohne Auflöse-Knopf, mit Hinweis. | `app/src/components/audit12-vaultlist.test.ts` (5 Tests) |
| B-P6 (A12-13, Doku) | **behoben** (Doku) | Abschnitt A12-13 berichtigt: betroffen sind nur Tresore, nicht Daueraufträge. | – (reiner Text in dieser Datei) |

**B-P4, der Preis der Obergrenze.** Eine Vorbereitung, die länger als 240 s braucht, wird unterbrochen. Kommt das in jedem Durchlauf vor, zahlt der Schritt nie; Runde 2 hatte genau das beim festen 90-s-Limit beanstandet. 240 s sind aber das Zweieinhalbfache des früheren Limits und so viel, wie Orakel und Keeper für dieselbe Art Arbeit haben. Üblich sind Sekunden: Verbindung, Sperre, einige RPC-Aufrufe je Tresor. Die Meldung sagt dann ausdrücklich „Kommt das in jedem Durchlauf vor, ist der Node zu langsam“. Ein Schritt, der kurz vor dem Limit sendet, wird nicht abgeschnitten: Er bekommt ab dem Limit noch die 90 s zum Warten.

**B-P5, eine Richtigstellung.** Die Nachprüfung schreibt, beim Schließen sage die Vorprüfung „an dich geht nichts“. Für Vaults unter 0,2 KAS stimmt das nicht: `stable_vault.sil` (`close`) zahlt eine Zinsgebühr erst ab `DUST` = 0,2 KAS, darunter wird sie erlassen, und die Vorprüfung sagt „Alle … KAS gehen an dich zurück“. Der Besitzer eines zu kleinen Zombie-Vaults bekommt beim Schließen also seine Sicherheit zurück. Der Hinweis in der Liste sagt das: „Sein Besitzer kann ihn schließen; ein Zins unter 0,2 KAS wird dabei erlassen.“

**B-P3, Quelltext-Test.** Die Agentenschleife und `keeper_round` brauchen einen Node; ein Ablauftest ohne Netz ginge nur über einen Umbau von `Ctx`. Der Test liest deshalb `ghostctl.rs` ohne das Testmodul, entfernt Leerraum und verlangt die Aufrufe wörtlich. Ein Umformatieren bleibt grün, eine andere Verdrahtung wird rot. Das Verhalten dahinter prüfen die Ablauftests (`keeper_act`, `sweep_in_turn`, `agent_cycle`).

Geändert: `protocol/src/ops.rs`, `protocol/src/rate.rs` (für C-O1 der Gruppe c), `protocol/src/bin/ghostctl.rs` (Keeper, Takt, Startmeldung, Zinsdatei, `deploy`/`oracle-update`, Tests), `protocol/tests/sweep_grenze_tests.rs`, `app/src/lib/precheck.ts`, `app/src/components/VaultList.tsx`, neu `app/src/components/audit12-vaultlist.test.ts`. Die Punkte der Gruppe c (Umzug, `--rate`, Startskript) stehen in `audit/12-stand-c.md`.

**Gegenproben, Runde 3** (Skript `…/scratchpad/fix13b-rueckbau/rueckbau.py`, Protokoll `rust.txt`; je Probe eine Stelle geändert, die passenden Tests gelaufen, die Datei danach byte-gleich wiederhergestellt):

| Probe | Änderung | Ergebnis |
|---|---|---|
| RB1 (B-P1) | alter Text „die Netzgebühr des Auflösens (SWEEP_FEE) ist …“ | **rot**: `a12_bp1_…` |
| RB2 (B-P2) | nach dem Fehlschlag nur `sent()` statt `sent() \|\| journal_open()` | **rot**: `a12_bp2_…` |
| RB2b (B-P2) | ohne Prüfung des Journals vor dem Bau | **rot**: `a12_bp2_…` |
| RB2c (B-P2) | Liquidationsschleife ohne Journal-Prüfung | **rot**: `a12_bp2_…` |
| RB3a (B-P3) | `keeper_act` gibt nur den ersten Kandidaten weiter | **rot**: `a12_bp3_keeper_runde…`, `a12_bp3_verdrahtung…` |
| RB3b (B-P3) | Cmd::Agent wieder ohne `agent_cycle` (Runde, Schritte, volle Pause nacheinander) | **rot**: `a12_bp3_verdrahtung…` |
| RB3c (B-P3) | Keeper nur mit Komitee-Datei | **rot**: `a12_bp3_verdrahtung…` |
| RB3d (B-P3) | `keeper_plan` kürzt die Liste auf einen | **rot**: `a12_bp3_verdrahtung…` |
| RB4 (B-P4) | `prep` wieder 700 s | **rot**: `a12_bp4_…` |
| RB10a–d (C-O1, Gruppe c) | `set_by_hand` ohne Wirkung / `oracle-update` ohne `rate_by_hand` / Vormerkung nie zurück / `deploy` ohne `rate_restart` | je **rot** (siehe `12-stand-c.md`) |
| RB11 (C-X1, Gruppe c) | alte Startmeldung „nur Liquidationen“ | **rot**: `a12_cx1_…` |
| RA5a (B-P5) | `VaultList`: Etikett nur bei `vaultSweepable` | **rot**: 2 von 5 in `audit12-vaultlist.test.ts` |
| RA5b (B-P5) | `vaultInterestEatsCollateral` rechnet nicht aus den Anzeigewerten | **rot**: 3 von 5 |

**Testzahlen, Runde 3.**
- Rust (`cargo test --release --offline`): **357 grün**, 2 ignoriert (`rest_live_tests`). Basis `a70fbbd`: 348, gezählt aus den Testattributen. Neu sind 9: lib `rate::tests` +2 (75), bin `ghostctl` +6 (12; `a12_haengende_vorbereitung…` ging in `a12_bp4_…` auf), `sweep_grenze_tests` +1 (4).
- Seite: `npx vitest run` **409 grün** in 21 Dateien (Basis 404, neu 5 in `audit12-vaultlist.test.ts`). `npx tsc --noEmit -p .` ohne Fehler, `npm run build` ohne Fehler (nur die übliche Warnung zur Chunk-Größe).
- Umzug: `tests/umzug/run.zsh` **102/0**, siehe `12-stand-c.md`.
