# Audit 20 (Fable, 06.10.2026) – Übersicht

Fünf unabhängige Teilprüfungen, Berichte: 20a Verträge, 20b Protokoll/Agent,
20c Server/Web, 20d Oberfläche/Aussagen, 20e Ökonomie/Orakel.

**Kein kritischer Befund.** Kein Weg zu Diebstahl, ungedecktem Prägen oder
Bewegen fremder Vaults/Tresore/Anteile ohne Schlüssel (20a, 20b).

## Hoch
- A20e-1 Gestohlener 1-von-1-Signer: Preis in ≈13 Updates (< 15 min) beliebig, gesunde Vaults leer liquidierbar / ungedecktes Prägen. Im Vertrag keine Sprunggrenze, kein Notfallsatz.
- A20e-2 (= A20a-7) Verlorener Signer: Orakel dauerhaft eingefroren, GHOST-Inhaber ohne Vault kommen nicht heraus.

## Mittel
- A20a-1 Notfall-Ticket per Replay alter Preis-Signaturen blockierbar (Vertrag; heute ohne Notfallsatz wirkungslos).
- A20b-1 Token-Liste wächst je Wallet-Überweisung unbegrenzt, Abgleich je Runde → Agent/Senden fallen aus (≈12 KAS für 2 000 Einträge).
- A20c-1 Wallet-Build belegt 11–13 s einen von zwei Plätzen vor der Adressprüfung → ein Absender blockiert Wallet-Aktionen.
- A20c-2 Supply-Chain: setup.sh als root im ghost-eigenen Ordner, npm-Skripte aktiv.
- A20d-1 Keine „Unklar-Sperre“ im Wallet-Ablauf.
- A20d-2 Vault-Auswahl hält Index; Zusammenfassung ohne Vault im Wallet-Modus.
- A20d-3 Orakel bis 6,5 h als „frisch“ angezeigt (Einfrieren ab 2 h).
- A20d-4/5 Falsche Zahlen: 9 statt 7 Unterzeichner; Auflösen 0,01 statt 0,1 KAS.
- A20d-6 Datenschutz/Rechtliches Platzhalter (Kanzlei).
- A20e-3 Keeper rechnet Netzgebühr nicht ein. A20e-4 keine GHOST-Quelle für Liquidationen.
- A20e-6 Grundzins + kaum gehandelter Pool = Zins-Ratsche (Simulation Ø ≈ 11 % nach 30 Tagen).
- A20e-7/8 Massen-Vaults bzw. Lese-Spam auf Orakel-UTXO bremsen die Orakelrunde (vermutet).

## Niedrig / Hinweise
Siehe Einzelberichte (A20a-2…6, A20b-2…6, A20c-3…7, A20d-7…11, A20e-9…15).

## Behebung
Stand je Befund wird hier nachgetragen.

### Zweig Protokoll/Agent/Skripte (protocol/, *.command, Doku; 06.10.2026)

| Befund | Stand | Änderung | Tests |
|---|---|---|---|
| A20a-1 (ghostctl-Seite) | behoben off-chain; Vertrag bleibt für v5 | `ops::propose(emergency)` und `ghostctl signers propose --emergency` (`check_emergency_allowed`) nur bei eingefrorenem Orakel, Meldung sagt, ob `oracle-freeze` schon geht; Doku MAINNET.md/ARCHITEKTUR.md | `v4_ops_tests::a20_notfallsatz_nachtragen_und_notfallweg`, `notfallweg_nur_nach_stille_…` (angepasst), ghostctl `a20a_1_…` |
| A20e-2/A20a-7 Notfallsatz | vorbereitet, nichts gesendet | `GHOST-Notfallsatz.command`: Schlüssel auf USB-Stick, Ankündigung `--same-set` + Notfallsatz (Probelauf, Senden nur nach „ja“), Aktivieren nach 14 Tagen; neu `signers propose --same-set`; `reconcile_register` übernimmt eine Aktivierung auch bei gleichem Satz | Simulator-Weg (s. o.), ghostctl `notfallsatz_skript_passt_zu_ghostctl` |
| A20e-6 Zinsregel | umgesetzt (Entscheidung „Totzone + nur bei Handel“) | Totzone 0,97–1,03 USD (= Band), Änderung nur bei Handel (Tauschverhältnis im Fenster Σ ≥ 2 %) | `rate::a20e_6_*`, `math::zinsregel_folgt_dem_ghost_kurs`, `totzone_ist_das_kursband_des_pools` |
| A20b-1 Token-Liste | behoben | Obergrenzen (`ops::MAX_TOKENS` 1 000 fremd, 16 je Empfänger, 64 je eigenem Besitzer), gebündelte Node-Abfragen (`store::snapshot`, 100 Adressen je Abfrage, auch für Vaults), Keeper prüft eigene Token jede Runde und 50 fremde reihum, Token-Fehler nur Hinweis | `store::a20b_1_*`, ghostctl `a20b_1_token_liste_hat_obergrenzen` |
| A20b-2 | behoben | `rate_step`: `prune` vor dem Grundzins-Pfad | ghostctl `a20b_2_…` |
| A20b-3 | behoben | `store::journal_age`: Zukunfts-mtime = Alter 0, mtime auf jetzt | `store::a20b_3_…` |
| A20b-4 | behoben | `wallet_ops::check_plan_shape` vor jeder Signaturprüfung; Wallet-Antwort mit anderer Form wird ohne Sighash abgelehnt | `wallet_ops_tests::a20b_4_…` |
| A20b-5 | behoben | `receive --owner` klärt unter der Sperre zuerst das Journal (`receive_resolve`) | ghostctl `a20b_5_…` |
| A20b-6 / A20e-15 | behoben | Messung auch während der Grundzins-Anhebung; Vormerkung erst, wenn die Vertragspause um ist (`rate_gap_over`) | ghostctl `a20b_6_…` (2 Tests) |
| A20c-1 (ghostctl-Teil) | behoben | `wallet build`: alle Eingaben vor `Net::connect` (`wallet_build_precheck`); `utxos`, `receive --owner` (`public_precheck`); `submit` prüfte schon vorher (`precheck`); `tresor owned` ist ohne Netz | ghostctl `a20c_1_…` (2 Tests, einer mit unerreichbarem Node < 1 s) |
| A20c-4 | nichts in protocol nötig | Routen gehören in den Web-Zweig | – |
| A20e-3 | behoben | `math::KEEPER_FEE_SOMPI` (0,1 KAS) geht zum Marktpreis vom Erlös ab | `math::a20e_3_…` |
| A20e-7 | behoben | Orakel-Runde: Abgleich nur Kern + Pool (`SyncScope::core`), Update auf diesem Stand vor dem Keeper; Vaults gebündelt | ghostctl `a20e_7_…` |
| A20e-9 | behoben | `feed_decision`: bei altem oder eingefrorenem Orakel sofort Zwischenschritt ±20 % | ghostctl `a20e_9_…` |

Rückbau-Probe (je Fix eine Zeile zurück, gezielter Test): 11 von 11 rot, dazu `reconcile_register` (Notfallsatz bei gleichem Satz) und die Einfrier-Pflicht in `ops::propose` je rot.
