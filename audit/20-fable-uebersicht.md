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
