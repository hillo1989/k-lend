# Audit 20, Teil d – Oberfläche, Signierablauf und Aussagen (k-lend.com)

Prüfer: unabhängig (Fable 5.1), 06.10.2026. Umfang: `app/src/` (Komponenten, lib, wallet, pages), zum Abgleich `app/server/walletActions.ts`, `app/server/api.ts`, `protocol/src/wallet_ops.rs`, `protocol/src/math.rs`, `protocol/src/contracts.rs`, `contracts/*.sil`, `ARCHITEKTUR.md`, `docs/wallet-aktionen.md`. Live-Seite nur lesend per `curl` (Kopfzeilen, `/api/status`). Keine Transaktionen, keine Wallet-Aktionen, kein Produktivcode geändert; die eigenen Belegtests (`app/src/wallet/audit20d.test.ts`, 8 Tests, alle grün) wurden nach dem Lauf entfernt.

Schwere: **Hoch** = Nutzer kann Geld an die falsche Stelle signieren oder doppelt senden · **Mittel** = irreführende Anzeige/Aussage mit Geldbezug oder rechtlich heikel · **Niedrig** = Unschärfe, Kosmetik, Härtung.

Kennzeichnung: **belegt** = im Code/Test nachgewiesen · **vermutet** = plausibel, nicht ausgeführt.

---

## Befunde

### A20d-1 · Mittel · Wallet-Ablauf ohne „Unklar-Sperre“ – Doppelsenden ist möglich, wenn der Nutzer den Rat ignoriert
**Ort:** `components/WalletSignFlow.tsx` (`sendNow`, Zeilen 171–195), `components/ActionForms.tsx` (`unclearLock`, Zeile 235, nur im Schlüsselmodus gesetzt), `components/WalletTresor.tsx` und `pages/Swap.tsx` (`blocked={false}` bzw. `blocked={nodeDown}`).
**Szenario:** Senden mit der Wallet endet „unklar“ (502/504, Netzabbruch, Zeitlimit). Die Seite verwirft nur die Signatur (`setChecked(null)`), der Plan bleibt bis zu 2 Minuten gültig. Der Nutzer kann sofort erneut „2. Signieren“ und „3. Senden“ drücken; es gibt keine Sperre bis zum nächsten Status wie im Schlüsselmodus (A10-W-2, `unclearLock`). Nach Ablauf des Plans kann er „1. Prüfen“ neu drücken und einen zweiten, neuen Plan senden.
**Beleg (belegt):** `onDone(true)` löst nur `refreshStatus()` aus; `unclearAt` wird in `ActionForms` ausschließlich in `exec()` (Schlüsselmodus) gesetzt. `blocked` für `WalletSignFlow` enthält daher nie eine Unklar-Sperre.
**Abmilderung (belegt):** Derselbe Plan referenziert dieselben UTXOs – ein zweiter Versuch mit demselben Plan scheitert am Node (Doppelausgabe) bzw. am Server („Plan passt nicht zum aktuellen Stand“). Solange das Wallet-Journal offen ist (bis 180 s), meldet `submit` „noch unterwegs … kurz warten“ (`isRetryLater`). Ein echter Doppelversand braucht also: neuer Plan nach Bestätigung der ersten Tx – genau das, wovor der Text nur warnt.
**Vorschlag:** In `WalletSignFlow` nach `unclear`/`timeout` denselben Mechanismus wie im Schlüsselmodus: `blocked`, bis `updatedAt` nach dem Vorfall liegt, und der Plan-Knopf „1. Prüfen“ gesperrt mit Hinweis „Status wird neu geladen“.

### A20d-2 · Mittel · Vault-Auswahl hängt an der verschiebbaren Nummer; die Zusammenfassung nennt im Wallet-Modus keinen Vault
**Ort:** `components/ActionForms.tsx` (`vault: number | null`, Zeilen 91, 138–145, 254; `summary`), `components/VaultList.tsx` (`onAction(q.action, v.index)`), `lib/status.ts` `vaultLabel`.
**Szenario:** Die Auswahl (`<select>`) und die Vorbelegung aus der Vault-Liste speichern den Zustandsdatei-Index. Laut A11-O-15 verschiebt sich dieser Index, wenn ein Vault mit kleinerer Nummer endet (Liquidation durch Dritte, Schließen). Beim nächsten Statusabruf (alle 30 s) zeigt derselbe Index einen anderen Vault; ist er ebenfalls ein eigener, bleibt die Auswahl still darauf stehen (`sel !== undefined`, nicht `foreign`). Zusätzlich springt die Anzeige „Vault 2“ → „Vault 1“ (Test: belegt).
**Was schützt (belegt):** Der Wallet-Plan verwendet die Covenant-ID (`toWalletParams`), und die Signatur `sig` enthält sie. Ändert sich der Vault hinter dem Index, verfallen Plan und Signatur (`plan.sig !== sig`). Der Nutzer kann also nichts anderes signieren als das, was der zuletzt geholte Plan zeigte.
**Lücke (belegt):** Im Wallet-Modus ist `built.params.vault` ein String (Covenant-ID), die Zusammenfassung prüft `typeof === "number"` und lässt den Vault weg. „Plan fertig – Prägen · 10 GHOST“ nennt keinen Vault; bei „Liquidieren · ganze Schuld“ sieht der Nutzer nur die Restzahlung an eine fremde Adresse, nicht, welchen Vault er trifft. Nach einem stillen Indexsprung prüft er den neuen Plan und sieht wieder keinen Vault-Namen.
**Vorschlag:** `vault`-Zustand als Covenant-ID führen (Index nur zur Anzeige); in `summary` `vaultLabel(...)` plus die ersten 8 Zeichen der Covenant-ID ausgeben; in `WalletSignFlow` den Vault aus `b.plan` anzeigen.

### A20d-3 · Mittel · Orakel „frisch“ und Pille „live“ bis 6,5 h, obwohl der Vertrag nach 2 h einfrierbar ist
**Ort:** `config.ts` `ORACLE_STALE_MINUTES = 390` (Kommentar: „Dauerbetrieb aktualisiert spätestens nach 6 h“), `lib/status.ts` `oracleStale`, `components/Network.tsx` (Pille, StatusNotices), `components/OracleCard.tsx` (Tag „frisch“).
**Szenario:** Kommt 2 h kein Preis, darf jeder einfrieren (Live-Status: `freezeAfterHours: 2`, `freezeInMinutes: 78.8` bei 41 min Alter). Solange niemand einfriert (`frozen: false`), zeigt die Seite zwischen 120 und 390 Minuten „frisch“/„live“, obwohl Prägen/Einlösen/Liquidieren jederzeit gesperrt werden können und der Preis 2–6 h alt ist. Die Seite „Orakel“ sagt zugleich „Agent aktualisiert nach 60 Minuten (halbe Einfrier-Frist)“ – Konstante und Kommentar in `config.ts` stammen aus Version 3.
**Beleg (belegt):** `ghostctl.rs` `heartbeat_min` = halbe Einfrier-Frist (60 min); Test `ORACLE_STALE_MINUTES > 120` grün.
**Vorschlag:** Grenze aus `status.signers.freezeAfterHours` ableiten (z. B. „veraltet“ ab 90 min, „einfrierbar“ ab 120 min) statt fester 390.

### A20d-4 · Mittel · Aussage „höchstens 9 Unterzeichner“ widerspricht dem Vertrag (7)
**Ort:** `pages/Faq.tsx` („höchstens 9 Schlüssel“), `pages/HowItWorks.tsx` (Abschnitt Orakel), `pages/Oracle.tsx` („n Schlüssel (höchstens 9)“).
**Beleg (belegt):** `contracts/signer_register_v4.sil` Zeile 80 `MAX_SIGNERS = 7`, Kommentar: „Mit 9 lag das Register bei 28 Sigops“; `protocol/src/contracts.rs` Zeile 330 `MAX_SIGNERS = 7`.
**Vorschlag:** Zahl aus einer Konstante ziehen oder auf 7 korrigieren.

### A20d-5 · Mittel · Auflösen (sweep): Seite nennt 0,01 KAS und „Netzgebühr zahlst du selbst“, Vertrag/ghostctl: 0,1 KAS aus dem Vault
**Ort:** `lib/commands.ts` `sweep.help` („bis auf 0,01 KAS … Netzgebühr von etwa 0,05 KAS zahlst du … selbst“), `pages/HowItWorks.tsx` („bis auf 0,01 KAS an die Zinsadresse“); dagegen `components/VaultList.tsx`/`lib/precheck.ts` („nach den 0,1 KAS für das Auflösen“, `SWEEP_FEE = 10_000_000`).
**Beleg (belegt):** `ARCHITEKTUR.md` Version 3: „`SWEEP_FEE` = 0,1 KAS (die erste Fassung mit 0,01 KAS war nicht baubar)“, „den Keeper kostet das nichts“. Die Seite widerspricht sich selbst.
**Vorschlag:** Hilfetext und HowItWorks auf 0,1 KAS und „Gebühr trägt der Vault“ bringen.

### A20d-6 · Mittel · Rechtstexte: „Rechtliches & Risiken“ und „Datenschutz“ sind Platzhalter auf einer Live-Seite mit Geldbezug
**Ort:** `pages/Legal.tsx` (`Placeholder`, „Inhalte folgen“), `Datenschutz()` ohne Inhalt.
**Was verarbeitet wird (belegt):** Server-seitige Ratenbegrenzung je IP (`clientKey`, `X-Forwarded-For`), Wallet-Adressen und Pläne in `/api/wallet/*`, .k-Namen als GET-Parameter an `/api/wallet/name` (Weiterleitung an dotk.name-API und eigenen Node), Explorer-Links zu Dritten, LocalStorage (Verlauf inkl. Klartext verschlüsselter Nachrichten und Empfängeradressen, `ghost.txlog.v1`). Eine Datenschutzerklärung (Art. 13 DSGVO) fehlt; Impressum ist befüllt.
**Vorschlag:** Mindestens eine kurze Datenschutzerklärung (Server-Logs/IP, dotk.name, Explorer, LocalStorage) und ein Risikohinweis ohne „Platzhalter“-Rahmen. Rechtsrat einholen (kein Rechtsrat hier).

### A20d-7 · Niedrig · Aussagen, die im öffentlichen Modus falsch oder widersprüchlich sind
**Ort/Beleg (belegt):**
- `pages/Landing.tsx` Schritt 3: „Danach signiert und sendet der lokale Server sie mit ghostctl“ – nicht `pub`-abhängig; im öffentlichen Modus signiert die Wallet. Widerspricht „Schlüssel verlassen die Wallet nie“ auf derselben Seite (Schritt 1).
- `pages/Faq.tsx` „Wer löst die Zahlungen eines Tresors aus“: „zahlen sie nur noch mit Gebühr vom eigenen Schlüssel … Ein laufender GHOST-Agent zahlt die Gebühr ohne Rückfrage vom Schlüssel seines Betreibers“ – für Wallet-Tresore (öffentlicher Modus) gilt laut `docs/wallet-aktionen.md` und `tresor.rs` Z. 1193: Agent zahlt **nie** dazu; die Seite selbst sagt das in `walletTresorBasics()` richtig. Die FAQ ist nicht `pub`-abhängig.
- `pages/HowItWorks.tsx` Vertragsrisiko: „Version 2.1 wird direkt im Mainnet … erprobt“ vs. `pages/Legal.tsx`: „Version 4 läuft im Mainnet“.
- `components/WalletButton.tsx`: „Die Seite liest nur Adresse, Netz und Guthaben“ – sie liest auch den öffentlichen Schlüssel (`publicKey()`; `WalletPanel` sagt es richtig).
- `config.ts` Kommentar zu `ORACLE_STALE_MINUTES` („spätestens nach 6 h“) vs. Orakel-Seite („60 Minuten“) – siehe A20d-3.
- Grundzins 2 %: Seite (Vault-Rechner, FAQ, HowItWorks) stimmt mit `math.rs` (`RATE_MIN_PCT = 2.0`, `rate_floor_step`) überein; Live-Satz ist 0,5 % p. a. (Status `ratePctYear`), d. h. die Anhebung läuft noch. `ARCHITEKTUR.md` Z. 230 sagt noch „Rahmen 0–20 %“ – Doku veraltet, Seite korrekt.
**Vorschlag:** Texte `pub`-abhängig machen, Versionsangaben zentral aus einer Konstante.

### A20d-8 · Niedrig · GHOST-Transfer: Empfänger ist aus der Ausgangsliste nicht ablesbar; Vertrauen liegt beim Server
**Ort:** `components/WalletSignFlow.tsx` (`payees` nur `what === "andere Adresse"`), `protocol/src/wallet_ops.rs` `describe_with` (Token-Ausgang = „GHOST-Token – … stecken darin“, Adresse = Covenant-Skriptadresse).
**Szenario (belegt, Test):** Bei `transfer` ist der Empfänger Besitzer im Token-Covenant; die Liste „Zahlungen an andere Adressen“ bleibt leer, auch „Alle Ausgänge“ zeigt nur die Covenant-Adresse. Der Nutzer sieht den Empfänger nur in der `summary` (`→ kaspa:q…`), die aus seiner Eingabe stammt, nicht aus dem Plan. Ein kompromittierter Server könnte einen anderen Token-Besitzer eintragen; weder Seite noch Wallet-Dialog könnten das zeigen. Das ist das dokumentierte Vertrauensmodell (docs/wallet-aktionen.md: „Der Plan kommt aus dem Browser und gilt als nicht vertrauenswürdig“ – umgekehrt gilt der Server für den Browser als vertrauenswürdig).
**Vorschlag:** `wallet build` könnte je Token-Ausgang den Besitzer (x-only bzw. Adresse) im `what` oder in einem eigenen Feld mitgeben; die Seite vergleicht ihn mit `params.to` und bricht bei Abweichung ab. Gleiches für Tresor-Empfänger (`describe_tresor_outputs` liefert ihn bereits im Text, die Seite vergleicht nicht).

### A20d-9 · Niedrig · Ausgangsbezeichner nur deutsch, auch in der englischen Oberfläche
**Ort:** `wallet_ops.rs` `what`-Strings („Wechselgeld an die Wallet“, „andere Adresse“, „Minter-Zweig …“), angezeigt in `WalletSignFlow` ohne Übersetzung; die Seite filtert zudem auf den deutschen Literal `"andere Adresse"` – eine spätere Übersetzung im Server würde die Empfängerliste leeren.
**Vorschlag:** Server liefert einen stabilen Code (`kind: "other" | "change" | "self" | "covenant"`) plus Text; Seite filtert auf den Code.

### A20d-10 · Niedrig · .k-Namensauflösung sendet jedes Tippfragment an Server und dotk.name
**Ort:** `lib/kname.ts` (`isKName`, 400 ms Debounce), `server/api.ts` `GET /api/wallet/name`.
**Beleg (belegt, Test):** `isKName("k")`, `isKName("kaspa")` → `true`. Wer eine Adresse langsam eintippt, löst bis zum Doppelpunkt Anfragen aus (Name als GET-Parameter → Caddy-Access-Log, dotk.name-API, Node-UTXO-Abfrage über ghostctl) und verbraucht Kontingent der Wallet-Ratenbegrenzung (20/min, gemeinsam mit build/submit). Datenschutz: Eingabefragmente verlassen den Browser, ohne dass der Nutzer einen Namen meinte.
**Vorschlag:** Erst auflösen, wenn die Eingabe auf `.k` endet oder einen Punkt enthält, und/oder Debounce auf 800 ms; „kaspa“/„kaspatest“ als Präfix ausschließen.

### A20d-11 · Niedrig · Handy: lange Adressen in der Empfängerliste ohne Umbruchregel (vermutet)
**Ort:** `WalletSignFlow.tsx` `<ul className="tx-list">` mit `… KAS → {o.address}`; `styles.css` `.tx-list` ohne `overflow-wrap`/`word-break` (nur `.txid code`, `.addr-break` haben es).
**Szenario (vermutet, nicht im Browser geprüft):** Eine 69-Zeichen-Adresse ohne Umbruchstelle sprengt bei 360 px die Ergebnisbox; die Adresse ist dann nur per Querscrollen vollständig sichtbar – genau die Angabe, die der Nutzer vor dem Signieren vergleichen soll.
**Vorschlag:** `.tx-list li { overflow-wrap: anywhere; }` oder Adresse in `<code className="addr-break">`.

---

## Geprüft und sauber (belegt)

**Signierablauf (Schwerpunkt 1)**
- Plan und Signatur sind an `sig = [network, action, address, params]` gebunden; `params` enthält Covenant-ID (Vault), aufgelöste .k-Adresse und Beträge. Ändert sich etwas – Eingabe, Wallet-Konto (`accountsChanged`), .k-Neuauflösung (60 s), Pool-Stand (Swap `min`) – verfallen Plan und Signatur (`useEffect` auf `sig`). Ein während des Wallet-Dialogs gewechseltes Konto führt zu einer Signatur, die der Server ablehnt (Plan-Schlüssel ≠ Signatur), und Senden bleibt gesperrt (`!planFresh`).
- Plan-TTL 2 min (`PLAN_TTL_MS`), Neuzeichnen per Timer, Senden danach gesperrt.
- Netzwechsel in der Wallet: `networkProblem`/`addressProblem` sperren alle drei Knöpfe; `planProblem` prüft Netz, Adresse, Plan-Art und dass Signier-Anfragen vorhanden sind. Server prüft zusätzlich Signaturen gegen die Plan-Adresse und baut den Plan bitgleich neu (`wallet_ops::precheck`, docs/wallet-aktionen.md, Rückbau-Probe).
- Doppelklick: alle Knöpfe `disabled` bei `phase !== "idle"`; `markBusy()` sperrt Sprachwechsel und fragt bei `beforeunload`; nach Erfolg `setPlan(null)`, nach Senden `setChecked(null)` außer bei `busy` (A19-7).
- Mainnet-Senden nur mit `send: true` + `confirmMainnet: true` (`submitBody`, serverseitig erzwungen). Die Seite ruft nie `sendKaspa`/`signAndBroadcastTx`/`pushTx` auf (grep über `src/`), nur `signPskt`/`signTx`.
- Zahlungen an fremde P2PK-Adressen (KAS-Senden, Liquidationsrest, Rücknahme, Tresor-Empfänger bei Zahlung) stehen immer offen über der eingeklappten Liste (Test „Liquidation“ grün). Einschränkung für Token/Tresor-Covenants: A20d-8.
- Tresor: Kündigen/Auffüllen nur wenn `!pending` (A19-6); Empfänger ≠ eigene Adresse, Netzpräfix, 1 KAS Minimum, Datum UTC (A19-8) – Seite und Server prüfen gleich.
- Unklar-Meldung beim Senden: Netzfehler, Nicht-JSON, 5xx → „unklar“, nie „nicht gesendet“ (A17-7, `callWalletApi`).

**Aussagen (Schwerpunkt 2)** – übereinstimmend mit Vertrag/Protokoll: Mindestquote 200 %, Liquidation < 150 %, Bonus 10 %, Ausbuchen unter ≈110 %, 0,2-KAS-Rest (`DUST`), Rücknahme 1 % / mind. 1 GHOST oder ganze Schuld / ≥ 150 % (`stable_vault_v4.sil`), 50 GHOST je Vault (`MAINNET_MAX_DEBT`, Live-Status `maxDebtGhost: 50`), 3 KAS Minter-Zweig dauerhaft (`ops::BRANCH_VALUE`), Zins erlassen < 0,2 KAS, Zinsregel ±0,5 pp/h, 2–20 %, Median der Stunde, ≥ 6 Messungen, ≥ 10 GHOST im Pool (`rate.rs`, `math.rs`), Pool-Gebühr 0,3 % und Band ±3 % (Live `feeBps: 30`, `bandBps: 300`), Einfrieren nach 2 h, Austausch 14 Tage, Notfall nach 30 Tagen (Live `signers`), 6 Preisquellen (`price.rs`). Keine Formulierung „sicher“/„garantiert“ im Sinne eines Versprechens; „nicht garantiert“, „experimentell“, „nicht professionell geprüft“, „Totalverlust“ stehen an allen relevanten Stellen (Landing, HowItWorks, FAQ, Legal). „Schlüssel verlassen die Wallet nie“ ist für `signPskt`/`signTx` zutreffend.

**XSS/Injection/Datenschutz (Schwerpunkt 3)**
- Kein `dangerouslySetInnerHTML`/`innerHTML` in `src/`; alle Daten (Fehlertexte, `what`, Adressen, Nachrichten, `kname.display`) werden als React-Text gerendert.
- Externe Links nur zu festen Hosts (`explorer.kaspa.org`, `explorer-tn10.kaspa.org`, `kasware.xyz`, `kastle.cc`) mit `rel="noopener noreferrer"`; TXID/Adresse nur als Pfadsegment; keine `javascript:`-URLs möglich (Präfix fest).
- CSP live: `default-src 'self'; script-src 'self' chrome-extension: moz-extension:; connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, dazu HSTS, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`.
- Nachrichten: `messageProblem` sperrt Steuer-/unsichtbare Zeichen (gleiche Tabelle wie ghostctl); Server `--message=<text>` verhindert Options-Injektion.
- LocalStorage: `kl-demo-wallet` (nur Wallet-Art), `gh-lang`, `gh-network`, `klend-install-hint-weg`, `ghost.txlog.v1` (Verlauf; Hinweis „Steht auch im Verlauf auf diesem Rechner“ steht am Feld; Verschlüsselte Nachricht im Klartext nur im Browser des Absenders – dokumentiert in `txlog.ts`). Keine Schlüssel, keine Signaturen gespeichert.
- Server: Fehlerausgaben ohne Pfade (A17-8); Ratenbegrenzung IPv6 je /64 (A17-3).

**Zustand/Handy (Schwerpunkt 4)**
- Statusabruf sequenziell (nächster erst nach Abschluss, 30 s), Abbruch per `AbortController`; Netzwechsel verwirft alte Zahlen.
- `UpdateHint` erkennt neue Builds (Hash im Skriptnamen) alle 5 min und bei Rückkehr; `InstallHint` nur Handy, nicht im Standalone-Modus.
- `NoWallet` erkennt iOS/Android und erklärt den Weg über den Wallet-App-Browser; Links `noopener`.
- Zahleneingabe: mehrdeutige „1.500“ wird abgelehnt und erklärt (A10-W-1), Rücklese-Zeile zeigt den gelesenen Wert.

## Nicht geprüft
- Tatsächliche Wallet-Dialoge (KasWare/Kastle) und Handy-Darstellung im Browser (A20d-11 daher „vermutet“).
- Ob der Live-Build (`main-DbnbI1WG.js`) exakt dem geprüften Quellstand entspricht.
