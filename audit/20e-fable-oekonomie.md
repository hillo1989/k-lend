# Audit 20e: Ökonomie, Orakel und Systemangriffe (GHOST v4, Mainnet)

Prüfer: Claude Fable 5.1, unabhängig, nur lesend. Datum: 06.10.2026. Stand `main` = fd4e28e (Grundzins 2 %).
Nicht angefasst: `keys/`, Server, Produktivcode. Eine lesende Abfrage `GET https://k-lend.com/api/status?network=mainnet` (Live-Parameter, Abschnitt 0). Keine Transaktion gesendet.
Eigene Belege: zwei temporäre Simulator-Tests (`protocol/tests/a20e_tmp_tests.rs`, gegen die echte Skript-Engine mit Mainnet-Parametern, nach dem Audit entfernt; Ausgabe unten zitiert) und Python-Rechnungen (Scratchpad, Zahlen im Text).

**[B]** = belegt (Test, Messung, Live-Status oder nachgerechnete Formel). **[V]** = vermutet / nicht gemessen.

---

## 0. Live-Lage (Status 06.10.2026, DAA 558 648 539) [B]

| Größe | Wert |
|---|---|
| Orakel | 0,04335083 USD/KAS, seq 21, Alter 37,6 min, Zins 0,5 % p. a., nicht eingefroren, Einfrieren in 82 min |
| Unterzeichner | **1 Schlüssel, Schwelle 1, Rotationsschwelle 1, kein Notfallsatz** (`fallback: null`), Rotation 336 h, Notfall ab 30 Tagen (ohne Notfallsatz wirkungslos) |
| Vaults | 3: Betreiber 50 KAS / 0,5 GHOST (433 %); 110 KAS / 0,1 GHOST (4 769 %); 2 KAS / 0 GHOST |
| Summen | 162 KAS Sicherheit = 7,02 USD, 0,6 GHOST Schuld, Deckung 1 170 % |
| Pool | 5,88 KAS / 0,2444 GHOST, Band ±3 %, Gebühr 0,3 %. **GHOST-Kurs im Pool 1,043 USD – außerhalb des Bands**, Kauf gesperrt (`maxBuyKas: 0`), verkaufbar 0,009 GHOST |
| Token | 0,25 GHOST (Betreiber), 0,1 + 0,0056 GHOST (zweiter Nutzer) |
| Parameter | MCR 200 %, Liquidation 150 %, Bonus 10 %, Rücknahme 1 %, 50 GHOST je Vault |
| Agent (Startskript) | Takt **300 s**, `--min-change 0.005`, `--max-age-min 60`, Keeper-Schlüssel falls vorhanden |

Gemessene Gebühren im Simulator mit Mainnet-Parametern (stimmen mit der Mainnet-Probe 05.10. überein): Vault eröffnen 0,0634 KAS, Prägen 0,0608, **Liquidation 0,0647**, Orakel-Update 0,0168 [B].

---

## 1. Kopplung an 1 USD und Liquidation

### Was die Kopplung heute trägt [B]

- **Untergrenze:** Rücknahme zu 0,99 USD an Vaults ≥ 150 %. Sie wirkt, solange GHOST irgendwo unter 0,99 zu kaufen ist. Im Pool verhindert das Band das Unterschreiten von 0,97 ohnehin.
- **Obergrenze:** nur das Prägen. Wer GHOST über 1 USD verkaufen will, muss einen Vault eröffnen (3 KAS Minter-Zweig + 1 KAS Token dauerhaft bzw. gebunden, 200 % Sicherheit, 0,12 KAS Gebühren). Bei 0,009 GHOST verkaufbarem Volumen (Status) lohnt das niemandem: Gewinn 0,009 × 4 % = 0,0004 USD.
- **Folge heute:** GHOST steht im Pool bei 1,043 USD, Kauf ist gesperrt. **Nachfrager können GHOST nicht beschaffen**, Liquidatoren und Rücknehmer ebenso wenig (A20e-4). Die Zinsregel ist inaktiv (Pool < 10 GHOST), der Zins steigt nur wegen des Grundzinses auf 2 %.
- Das Band ist am Orakel verankert, der Pool nicht: Jede KAS-Bewegung > 3 % seit dem letzten Tausch sperrt eine Richtung, bis jemand gegen Gebühr zurücktauscht. Bei 0,25 USD Tiefe passiert das nicht. Die Aussage „Pool hält GHOST bei 1 USD ± 3 %“ bedeutet in der Praxis: **außerhalb des Bands wird nicht gehandelt**, nicht: „es gibt Liquidität bei 1 USD“.

### Wann die Kopplung bricht (Rechnung)

| Szenario | Mechanik | Ergebnis |
|---|---|---|
| KAS-Crash > 25 % in < 1 Runde | Vault bei 200 % → < 150 %. Orakel folgt mit ≤ 1 Runde (300 s) Verzug, bei > 20 % mit 3 Runden (10–15 min) und dann höchstens ÷2 je Update | Liquidation möglich, sobald jemand GHOST hat (A20e-4). Kein Kaskadeneffekt im System: Liquidation verbrennt GHOST, verkauft nichts |
| KAS −45 % ohne Liquidation (z. B. Orakel eingefroren, Betreiber offline) | Vault 200 % → 110 %; darunter nimmt der Liquidator alles, Restschuld wird ausgebucht | **Ungedeckte GHOST**; nichts verteilt die Lücke, letzte Inhaber können nicht einlösen (A20e-11) |
| Eingefrorenes Orakel (2 h ohne Update) | mint/redeem/liquidate/sweep/withdraw-mit-Schuld und Pool-Swaps gesperrt; repay/close/deposit/add/remove frei | Schuldner kommen raus, GHOST-Inhaber ohne Vault nicht. Preisrisiko läuft weiter, Liquidation steht still |
| Kein Arbitrageur | Pool driftet mit KAS aus dem Band, eine Richtung sperrt (live der Fall) | GHOST nicht beschaffbar bzw. nicht verkaufbar; Zinsregel misst Drift statt Nachfrage (A20e-6) |
| Illiquider Pool | 0,244 GHOST Reserve | Liquidation von Vault 0 (0,5 GHOST) nur mit selbst geprägten GHOST |

### Ist Liquidation für Dritte profitabel? [B]

Netzgebühr 0,0647 KAS = 0,0028 USD je Liquidations-Tx. Bonus 10 % des Burns:

| Burn | Bonus | netto | `keeper_burn` sagt „ja“ |
|---|---|---|---|
| 0,001 GHOST | 0,0001 USD | **−0,060 KAS** | ja |
| 0,01 GHOST | 0,001 USD | **−0,039 KAS** | ja |
| 0,03 GHOST | 0,003 USD | +0,007 KAS | ja |
| 0,5 GHOST (Vault 0) | 0,05 USD | +1,09 KAS | ja |
| 50 GHOST (voller Vault) | 5 USD | +115 KAS | ja |

Break-even bei **0,027 GHOST** Burn. Ab normalen Vault-Größen ist Liquidation sehr profitabel; das Problem ist nicht der Anreiz, sondern die **GHOST-Beschaffung** (A20e-4) und die Gebührenblindheit des Keepers (A20e-3). Außerdem bindet jede Liquidation 1 KAS im Wechsel-Token des Liquidators (kommt zurück).

---

## 2. Orakel

### A20e-1 — hoch — Gestohlener Signer-Schlüssel: gesamter Bestand in Minuten weg, keine Gegenwehr [B]

**Szenario.** Der eine Unterzeichner-Schlüssel (`keys/mainnet-committee.json`, auf dem Rechner des Agenten, 1 von 1) wird kopiert. Der Vertrag begrenzt je Update auf ×2/÷2 und 600 DAA Abstand der *Orakel-DAA*; die Orakel-DAA liegt aber hinter der Kette (live 22 566 DAA = 37 Updates ohne Wartezeit, NEU-1).

**Beleg (Simulator, Mainnet-Parameter, 1-von-1-Register):**
```
Preis 0,04335 → 0,00001 USD in 13 Updates; davon 13 ohne Wartezeit (Rückstand)
nötiger Burn: 0,00909091 GHOST für 1000 KAS
Angreifer: +999,9353 KAS netto für 0,00909091 GHOST
Preis → 900 USD in 27 Updates; 50 GHOST gegen 0,12 KAS geprägt (Obergrenze je Vault)
Gebühren aller 40 Orakel-Updates: 0,6738 KAS
```
Ein gesunder Vault (1 000 KAS, 20 GHOST, 217 %) ging für 0,009 GHOST (0,009 USD) komplett an den Angreifer. Die Beschränkung „höchstens Sicherheit/(1+Bonus)“ schützt nicht, weil der Preis gefälscht ist. Danach bei 900 USD: 50 GHOST je Vault gegen 0,12 KAS, beliebig viele Vaults (keine Gesamtobergrenze), verkaufbar in den Pool (Band folgt demselben Orakel) bis auf 1 KAS Rest.

**Quantifiziert auf heute:** 162 KAS Vaults + 4,9 KAS Pool = **167 KAS = 7,2 USD, in < 5 min**, Gebühren ≈ 0,7 KAS. Skaliert 1:1 mit dem TVL. Der Betreiber-Keeper verhindert nichts (er liquidiert nur bei Marktbestätigung, der Angreifer braucht ihn nicht). Der Betreiber **kann nicht gegensteuern**: `cancel` braucht t = 1 Signatur des Hauptsatzes (hat der Angreifer auch), `propose` → 14 Tage Wartezeit, der Angreifer kann jede Ankündigung absagen, es gibt keinen Notfallsatz. Einziger Ausweg: neues Deployment.

**Abhilfen im Vertrag (Vorschlag, in Reihenfolge des Nutzens):**
1. **Notfall-Einfrieren durch einen getrennten Wächterschlüssel** (Cold Key, oder m-von-n): `freezeNow(sig)` im Orakel, Auftauen nur über `activate` eines Austauschs. Dann kostet ein Diebstahl des Signer-Schlüssels höchstens das Fenster bis zum Einfrieren. (Heute: `freeze` erst nach 2 h Stille, ein Angreifer hält das Orakel frisch.)
2. **Zwei-Preis-Regel im Vault:** Das Orakel führt `prevKasUsd`/`prevDaa` mit (Preis, der mindestens 1 h = 36 000 DAA alt ist). `liquidate` verlangt Unterdeckung zu *beiden* Preisen, `mint` Deckung zu beiden. Ein gefälschter Preis muss dann ≥ 1 h stehen, bevor er Geld bewegt – Zeit für 1. Der Vault wird um einen Vergleich teurer, das Orakel um 16 B Zustand. Kein oberes Zeitlimit nötig (B1 bleibt gewahrt).
3. **Mehrere unabhängige Unterzeichner (2 von 3)** auf getrennten Rechnern – das Register kann es, der Betreiber hat sich für 1 von 1 entschieden. Mit 2 von 3 reicht ein Diebstahl nicht.
4. Nur als Ergänzung: kleinerer Schritt je Update (z. B. ±25 %) verlängert den Angriff von 13 auf 37 Updates, hilft ohne 1./2. nicht, weil der Rückstand der Orakel-DAA beliebig viele Updates sofort erlaubt. Ein Preiskorridor als Template-Konstante (z. B. 0,005–0,5 USD) senkt die Beute nur auf ≈ 90 % (Rechnung: bei Floor ÷8,7 braucht der Angreifer 10,5 % des Werts in GHOST).

Die Seite sagt das Risiko offen („Wer diesen Schlüssel hält, kann jeden beliebigen Preis setzen“). Die Schwere bleibt hoch, weil der Verlust total und die Abwehr null ist.

### A20e-2 — hoch — Schlüsselverlust = Protokoll dauerhaft eingefroren [B, aus Status + Vertrag]

Ohne Notfallsatz und mit tRot = 1 gibt es nach Verlust von `mainnet-committee.json` keinen Weg zu einem neuen Unterzeichner. Nach 2 h friert das Orakel (jeder darf), und bleibt es: Prägen, Rücknahme, Liquidation, Abheben mit Schuld und Pool-Swaps für immer gesperrt. Schuldner können tilgen und schließen; **GHOST-Inhaber ohne eigenen Vault kommen nur heraus, wenn ein Schuldner ihnen GHOST abkauft** (Pool gesperrt). Vorschlag: sofort per `propose → activate` (14 Tage) einen Notfallsatz mit einem kalt gelagerten Schlüssel setzen (geht ohne neues Deployment), Schlüssel sichern.

### Preisquellen, Verzögerung, Sprungsperre [B]

- Median aus 6 Quellen, Ausreißer > 3 % fallen weg, ≥ 3 nötig. Drei kolludierende Quellen (api.kaspa.org und CoinGecko teilen Upstreams) verschieben den Median um höchstens ≈ 3 %, darüber greift der Filter oder die Prüfung scheitert. Gegen den Signer-Rechner selbst hilft das nicht (A20e-1).
- Verzug: ≤ 0,5 % oder ≤ 60 min im Normalfall; bei Bewegung > 20 % drei Runden = **10–15 min** (Takt 300 s), dann ÷2/×2-Schritte.
- **A20e-9 — niedrig/mittel — Sprungsperre schlägt Heartbeat und Auftauen.** `feed_due` prüft den Sprung *vor* dem Alter: Liegt der Markt > 20 % vom Orakel entfernt und streuen drei aufeinanderfolgende Runden um > 5 %, wird weder der Heartbeat noch das Auftauen gesendet. In einem volatilen Crash (> 20 % Bewegung, > 5 % Streuung über 15 min) bleibt der alte Preis stehen, nach 2 h friert der Keeper das eigene Orakel ein, und es bleibt eingefroren, bis drei ruhige Runden kommen. Vorschlag (wie A11-O-2): Ist das Alter über dem Heartbeat oder das Orakel eingefroren, einen gedeckelten Zwischenschritt (±20 %) sofort senden.
- **A20e-10 — niedrig — Rücknahme gegen nachlaufendes Orakel, quantifiziert mit 300-s-Takt.** Gewinn je GHOST = Sprung − 1 %; Fenster ≤ 300 s (Sprung ≤ 20 %) bzw. 10–15 min (> 20 %). Schaden ≤ Umlauf × (Sprung − 1 %): heute bei +25 % ≈ 0,14 USD, bei +50 % ≈ 0,29 USD; linear im Umlauf. Nicht aus dem Nichts ausbeutbar (Kauf im Pool + Rücknahme verliert ≈ 4 %, da das Band am selben Orakel hängt). Bekannt (A11-O-2), hier nur die Zahlen für den heutigen Takt.

---

## 3. Zinsregel mit Grundzins 2 %

### A20e-6 — mittel — Ratsche: in einem nicht arbitrierten Pool treibt KAS-Drift den Zins auf 20 % [Simulation]

Messung = Pool-Verhältnis × KAS-Markt. Das Verhältnis ändert sich nur durch Tausch; das Band ändert es nicht. Ohne Arbitrage ist die Messung also KAS/KAS₀ und verlässt ±0,5 % in Stunden. Unter 0,995: +0,5/h bis 20 %; über 1,005: −0,5/h, **aber nur bis 2 %**. Monte-Carlo (400 Läufe, stündlich, Start 2 %, 30 Tage):

| Stunden-Volatilität | Zins nach 30 Tagen (Mittel / Median) | Anteil der Läufe bei 20 % |
|---|---|---|
| 0,8 % | 10,8 % / 8,8 % | 45 % |
| 1,3 % (≈ 6 % Tagesvol) | 10,9 % / 10,0 % | 46 % |
| 2,0 % | 11,2 % / 13,5 % | 47 % |

Vor dem Grundzins war die Ratsche symmetrisch (0–20 %); der Grundzins macht den Erwartungswert einseitig. Heute inaktiv (Pool < 10 GHOST), aktiv ab der ersten 10-GHOST-Einlage. Vorschlag: Zins nur ändern, wenn im Fenster **gehandelt** wurde (Reserveänderung ≠ 0), oder Totzone auf ±3 % (= Band) weiten, oder die Messung auf den Kurs des *letzten Tauschs* beziehen.

### Manipulation über den kleinen Pool [B, Rechnung]

Bei 10 GHOST / 230 KAS kostet ein Kurs-Schub um 0,6 %, 30 min gehalten: Einsatz 0,69 KAS, 0,3 % Gebühr 0,002 KAS, 2 Netz-Tx 0,12 KAS, 1 KAS Token gebunden → **≈ 0,12 KAS je 0,5-Punkte-Schritt, ≈ 4,4 KAS (0,19 USD) für 0 → 20 % in 36 h**. Der Median schützt nur gegen Einzelmessungen, nicht gegen einen Angreifer mit 0,7 KAS Kapital. Wer profitiert? **Der Zinsempfänger = der Betreiber** (A20e-13); Schuldner verlieren. Bei 0,6 GHOST Schuld sind 20 % = 0,12 USD/Jahr – heute belanglos, bei 10 000 GHOST 2 000 USD/Jahr.

### Wechselwirkung mit der Kopplung [B]

GHOST steht über 1 USD; die Regel dürfte senken, kann aber nicht unter 2 %. Ein höherer Zins drückt Prägung, also Angebot → verstärkt den Überhang. Umgekehrt (unter 0,995) steigt der Zins, aber der Zins wirkt nur, wenn Schuldner reagieren; bei 0,5 GHOST Schuld sind 0,5 Punkte = 0,0025 USD/Jahr. Die Regel ist bei dieser Größe wirkungslos, aber harmlos. Kleiner Nebenbefund **A20e-15 — Info:** Während der Grundzins-Anhebung (jetzt 3 h) nimmt `rate_plan` keine Messung auf (kehrt vor `record` zurück); nach Erreichen der 2 % fehlen mindestens 45 min Messungen.

---

## 4. Pool

### A20e-12 — niedrig — Sandwich/MEV [Rechnung]

Es gibt keinen Sandwich im Uniswap-Sinn: Die Pool-UTXO lässt nur eine Tx je Zustand zu; wer zuerst ausgibt, macht die andere ungültig (P-3). Ein Vorläufer-Angriff bringt dem Angreifer höchstens die Slippage-Toleranz des Opfers (1 %, ghostctl setzt `min_*` aus dem Plan) minus 2 × 0,3 % Poolgebühr minus 2 × 0,06 KAS Netz: **positiv erst ab ≈ 30 KAS Opfer-Trade**, bei 100 KAS +0,28 KAS. Das Band ±3 % deckelt jede Verschiebung, und bei 5,9 KAS Tiefe ist ein 30-KAS-Trade unmöglich. Tragfähig.

### Mindestbeträge, Rundung [B, Vertragstext + pool.rs]

- `shares = min(⌊S·Δx/x⌋, ⌊S·Δy/y⌋)`, `payout = ⌊x·m/S⌋` – immer zugunsten der übrigen Einleger; S₀ = 10⁸ je Start-KAS, Auflösung 1 Sompi. Rundungsverlust ≤ 1 Sompi je Vorgang. Mini-Abzug (0, 0) wird abgelehnt.
- Kauf mit Kleinstbetrag liefert 0 GHOST → abgelehnt. Jeder Tausch bindet 1 KAS im Token-Ausgang. Pool-Tx 0,06 KAS. Die 1 KAS Mindestreserve und die Startreserve bleiben für immer.
- Bei eingefrorenem Orakel sind nur Swaps gesperrt; `add`/`remove` lesen das Orakel nicht → Einleger kommen immer raus. Tragfähig.

---

## 5. Griefing und wirtschaftliche DoS

### A20e-3 — mittel — Keeper rechnet die Netzgebühr nicht ein [B]

`keeper_profitable` prüft nur `Wert(got) ≥ burn × 1,02` zum Marktpreis; die Tx-Gebühr (0,0647 KAS) fehlt. Simulator:
```
Mini-Vault: burn 0,01 GHOST (0,01 USD), erhält 0,3903 KAS = 0,011 USD, Bonus +0,001 USD,
Netzgebühr 0,0647 KAS = 0,00182 USD → netto −0,00082 USD = −0,0292 KAS
```
Ein Angreifer kann das kaum ausnutzen (je Mini-Vault 3 KAS Minter-Zweig + 1 KAS Token + Sicherheit, und er braucht einen Crash), aber in jedem echten Crash verbrennt der Keeper für alle Vaults mit Burn < 0,027 GHOST Geld, eine Liquidation je Runde (300 s). Vorschlag: in `keeper_profitable` die geschätzte Gebühr (≈ 0,07 KAS) zum Marktpreis abziehen, Mindest-Burn ≈ 0,05 GHOST.

### A20e-4 — mittel — Keine GHOST-Quelle für Liquidation und Rücknahme [B]

Pool-Reserve 0,244 GHOST, Kauf gesperrt; Keeper hält 0,25 GHOST (wenn er den Besitzer-Schlüssel nutzt) → Vault 0 (0,5 GHOST) nur zur Hälfte liquidierbar; ein Dritter muss zuerst selbst prägen (Vault eröffnen: 3 KAS + 1 KAS dauerhaft/gebunden, 200 % Sicherheit, 0,12 KAS Gebühren) – in einem fallenden Markt. Folge: Liquidationen verzögern sich, der Weg unter 110 % (Ausbuchung, A20e-11) wird wahrscheinlicher. Vorschlag: Keeper-Schlüssel mit GHOST-Reserve ≥ größte Einzelschuld ausstatten; Pool-Liquidität mindestens in Höhe der größten Schuld; mittelfristig ein Stabilitäts-Topf (GHOST-Einlagen, die bei Liquidation verbrannt werden und KAS + Bonus erhalten).

### A20e-7 — mittel [V] — Massen-Vaults verlangsamen die Agentenrunde bis zum Einfrieren

`store::resync` macht je Vault **2 Node-Abfragen** nacheinander (Vault- und Zweig-UTXO), `discover` liest die Factory-Historie über REST (höchstens 2 000 Tx). Die Orakel-Runde hat 240 s Zeitlimit; wird es gerissen, gibt es kein Update, nach 2 h friert das Orakel. Kosten je Dust-Vault: 3 KAS Minter-Zweig (verloren) + ≥ 0,02 KAS Sicherheit + 0,063 KAS Gebühr ≈ **3,1 KAS**:

| Vaults | Kosten Angreifer | Abfragen je Runde | Zeit bei 30 ms / 100 ms je Abfrage |
|---|---|---|---|
| 1 000 | 3 100 KAS (134 USD) | 2 000 | 60 s / 200 s |
| 4 000 | 12 400 KAS (536 USD) | 8 000 | 240 s / 800 s |

Die RPC-Latenz ist nicht gemessen [V]; bei einem entfernten Node (100 ms) reichen ≈ 1 200 Vaults (≈ 160 USD) für eine dauerhaft gerissene Runde. Zusätzlich zeigt `status` alle Vaults (JSON-Größe, Seite). Vorschlag: Orakel-Update **vor** dem Vault-Abgleich senden (das Orakel braucht nur die eigene UTXO und das Register), Vault-Abgleich parallelisieren oder nur bei Bedarf (Keeper) und mit eigenem Zeitlimit; Vaults mit Sicherheit < 1 KAS nicht abgleichen.

### A20e-8 — mittel [V] — Lese-Spam auf die Orakel-UTXO (O-1) kostet 12–120 USD/Tag

Jeder `read()` erzeugt die Orakel-UTXO neu (≈ 0,003 KAS). Hält ein Angreifer ständig ein unbestätigtes Kind der Spitze im Mempool (vorsignierte Kette), scheitern alle anderen Orakel-Tx (Update, Vault, Pool) am Konflikt (first-seen, vermutet). Bei 1 read/s ≈ 268 KAS/Tag (12 USD), bei 10/s ≈ 2 700 KAS/Tag (116 USD). Nach 2 h friert das Orakel, und es bleibt eingefroren, solange der Spam läuft (auch das Auftauen braucht die Spitze). Bekannt als Designgrenze; die Zahl zeigt, dass ein kompletter Stillstand für < 100 USD/Tag zu haben ist. Abhilfe nur im Design (z. B. mehrere Orakel-Leser-UTXOs, oder Updates, die jede Spitze derselben seq akzeptieren).

### Gebühren-Wallet des Betreibers [B, Rechnung]

- Orakel: Heartbeat 60 min → ≥ 24 Updates/Tag = 0,40 KAS/Tag; bei dauernder 0,5-%-Bewegung je 300-s-Runde höchstens 288/Tag = 4,8 KAS/Tag (0,21 USD). Ein Dritter kann Updates nicht erzwingen: Auslöser ist der Marktmedian, nicht die Kette. Einfrieren einmalig 0,0031 KAS.
- Keeper: Liquidation 0,0647 KAS, nur bei Kandidaten; Verluste bei Dust (A20e-3). `sweep` zahlt der Vault (0,1 KAS), Tresor-Zahlungen zahlt der Tresor (≤ 0,01 KAS).
- Minter-Zweige: 3 KAS je Vault dauerhaft – trägt der Eröffner, nicht der Betreiber.

### A20e-11 — niedrig — Ausgebuchte Restschuld wird nicht verteilt [B, Vertragstext]

Unter ≈ 110 % nimmt der Liquidator alles, die Restschuld „ist ausgebucht“: GHOST bleiben im Umlauf ohne Deckung, und `redeem` braucht Vault-Schuld – die letzten Inhaber finden keine. Nötig dafür: KAS −45 % ohne Liquidation (etwa bei eingefrorenem Orakel oder fehlenden GHOST, A20e-4). Die Seite sagt das. Es gibt weder Stabilitäts-Topf noch Umlage noch Protokollschuld. Vorschlag: mindestens einen Zähler „ungedeckte GHOST“ im Status führen; mittelfristig Stabilitäts-Topf (siehe A20e-4).

---

## 6. Konzentration und Betreiberrisiko [B, Status]

- Betreiber hält Vault 0 = **31 % der Sicherheit, 83 % der Schuld**, nach Lage der Dinge alle 5,75 × 10⁸ handelbaren Pool-Anteile [V, Status nennt keinen Inhaber], den einzigen Signer, den Keeper, die Zinsadresse und die Seite. Ausfall des Rechners → nach 2 h Freeze (siehe Tabelle in 1.), Schlüsselverlust → A20e-2, Diebstahl → A20e-1.
- **A20e-13 — niedrig — Interessenkonflikt Zins.** Der Zins (2–20 % p. a.) geht an die Betreiberadresse; der Betreiber steuert die Regel, kann den Satz von Hand setzen (`oracle-update --rate`, 0,5 Punkte/h) und könnte die Messung für ≈ 0,12 KAS/h schieben (Abschnitt 3). On-chain ist jede Änderung sichtbar (`lastRateDaa`), aber nicht ihre Begründung. Vorschlag: Zinskasse v4 mit Verbrennen oder neutralem Ziel, oder Zinsänderungen samt Median-Messung öffentlich protokollieren (Datei `mainnet-zins.json` veröffentlichen).
- Haftung: nicht Gegenstand dieser Prüfung (RECHT_PRUEFUNG.md); wirtschaftlich ist der Betreiber heute größter Schuldner und größter Verlierer jedes Fehlers im eigenen Orakel.

---

## Befundliste

| ID | Schwere | Status | Kurz |
|---|---|---|---|
| A20e-1 | **hoch** | belegt (Test) | Gestohlener 1-von-1-Schlüssel: 13 Updates ohne Wartezeit, 1 000 KAS für 0,009 GHOST, 50 GHOST gegen 0,12 KAS; heute 167 KAS, keine Gegenwehr (kein Notfallsatz, Rotation 14 Tage) |
| A20e-2 | **hoch** | belegt (Status) | Schlüsselverlust = dauerhaftes Einfrieren; GHOST-Inhaber ohne Vault ohne Ausweg |
| A20e-3 | mittel | belegt (Test) | Keeper ignoriert Netzgebühr: Burn < 0,027 GHOST ist Verlust (Mini-Vault −0,029 KAS) |
| A20e-4 | mittel | belegt (Status) | Keine GHOST-Quelle: Pool 0,244 GHOST, Kauf gesperrt, Keeper 0,25 GHOST; Vault 0 nur halb liquidierbar |
| A20e-6 | mittel | Simulation | Grundzins + unarbitrierter Pool = Ratsche: 30 Tage → Ø 11 %, 45 % der Läufe bei 20 % |
| A20e-7 | mittel | vermutet (Rechnung) | Massen-Vaults (3,1 KAS je Stück) reißen das 240-s-Limit der Runde → Freeze; ab ≈ 1 200–4 000 Vaults |
| A20e-8 | mittel | vermutet (Rechnung) | Lese-Spam O-1: kompletter Stillstand für 12–120 USD/Tag |
| A20e-9 | niedrig–mittel | belegt (Code) | Sprungsperre vor Heartbeat/Auftauen: volatiler Crash > 20 % hält das Orakel alt und dann eingefroren |
| A20e-10 | niedrig | Rechnung | Rücknahme gegen Nachlauf: Fenster 300 s bzw. 10–15 min, Schaden ≤ Umlauf × (Sprung − 1 %) |
| A20e-11 | niedrig | belegt (Vertrag) | Ausbuchung unter 110 % ohne Verteilung → ungedeckte GHOST, letzte Inhaber ohne Rücknahme |
| A20e-12 | niedrig | Rechnung | Vorläufer-Trade erst ab ≈ 30 KAS Opfer-Trade lohnend, Band deckelt – tragfähig |
| A20e-13 | niedrig | belegt (Code) | Zins an den Betreiber, Regel und Hand-Satz beim Betreiber, Messung für 0,12 KAS/h verschiebbar |
| A20e-15 | Info | belegt (Code) | Während der Grundzins-Anhebung keine Messungen, danach ≥ 45 min Wartezeit |

## Geprüft und tragfähig

- **Liquidationsanreiz** ab 0,03 GHOST Burn positiv, bei 50 GHOST +115 KAS je Tx; Keeper-Band 100–110 % (bestehender Test über 34 000 Preise) und Teil-Liquidation funktionieren [B].
- **Rücknahme-Arbitrage aus dem Nichts** unmöglich: Kauf im Pool (≤ 1,03 zum Orakel) + Rücknahme (0,99) verliert ≈ 4 %, Band und Rücknahme hängen am selben Preis [B, Rechnung].
- **Zinsrahmen im Vertrag** (0,5 Punkte je Stunde, 0–20 %): auch mit gestohlenem Schlüssel dauert 0 → 20 % mindestens 36 h; `fit_rate` hält den Agenten in den Grenzen [B, Code + Audit 15].
- **Pool-Rundung und Mindestbeträge** zugunsten der Einleger, 1 Sompi Auflösung, (0, 0)-Abzug abgelehnt, Einleger kommen auch bei eingefrorenem Orakel heraus [B].
- **Mindest-Rücknahme 1 GHOST** und Mindestrest 0,2 KAS verhindern Dust-Griefing fremder Vaults; `sweep` trägt seine Gebühr selbst [B, Vertrag].
- **Tresor-Zahlungen**: Betrag, Empfänger und Nachricht fest, Gebühr ≤ 0,01 KAS aus dem Tresor, ≥ 1 KAS bleibt – kein Abfluss durch fremde Auslöser (nachvollzogen anhand ARCHITEKTUR.md und Audit 12/13; der Vertrag wurde in diesem Teil nicht erneut gelesen).
- **Preis- und Mengengrenzen** (0,00001–900 USD, 1e8 KAS, 1e9 GHOST, Index ×10 je Abrechnung) sind überlauffrei; sie begrenzen aber keinen Angreifer mit Schlüssel (A20e-1).
- **Betriebskosten** des Orakels sind von Dritten nicht erzwingbar (Auslöser ist der Marktmedian): 0,4 KAS/Tag normal, ≤ 4,8 KAS/Tag bei Dauerbewegung [B].

## Empfohlene Reihenfolge

1. Notfallsatz mit kaltem Schlüssel setzen (A20e-2, sofort, ohne Deployment) und Signer-Schlüssel sichern.
2. Für das nächste Deployment: Wächter-Einfrieren und Zwei-Preis-Regel (A20e-1), 2 von 3 Unterzeichner.
3. Keeper: Gebühr in die Gewinnprüfung (A20e-3), GHOST-Reserve (A20e-4), Orakel-Update vor dem Vault-Abgleich (A20e-7), Zwischenschritt bei altem Orakel (A20e-9).
4. Zinsregel: nur bei gehandeltem Pool ändern oder Totzone = Band (A20e-6).
