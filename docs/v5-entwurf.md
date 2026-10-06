# GHOST v5: Verträge nach Audit 20

Stand 06.10.2026, Zweig `worktree-agent-af57067bb26b4b013` (Basis `db18e56`, Audit 20). Entscheidung des Betreibers: „Version 5 jetzt planen und bauen“. Gebaut sind die Verträge mit Engine- und Simulator-Tests. **Keine Anbindung** an ghostctl, Agent, Seite (kommt nach Freigabe dieses Entwurfs). v4 läuft unverändert weiter, kein v4-Vertrag und keine v4-Logik ist geändert.

**[B]** = belegt (Test oder Messung), **[V]** = hergeleitet, nicht im Netz geprüft. Messwerte in Abschnitt 8 stammen aus den Tests `protocol/tests/v5_*`.

---

## 1. Ziele

| Befund (Audit 20) | Ziel v5 |
|---|---|
| **A20e-1** (hoch) gestohlener Signer: Preis in 13 Updates < 15 min beliebig, gesunde Vaults leer | Ein gestohlener Signer allein bewegt in einem festen Fenster kein Geld; ein Wächter kann in diesem Fenster sperren, ohne dass der Dieb es verhindern kann; 2-von-3 bleibt möglich |
| **A20e-2** (hoch) verlorener Signer: Orakel für immer eingefroren, GHOST-Inhaber ohne Ausweg | Austausch über Wächter + Notfallsatz ohne 30 Tage Stille; als letzter Ausweg Abwicklung zum letzten Preis |
| **A20a-1** (mittel) Replay alter Preis-Signaturen mit Orakel-`read` tötet Notfall-Ticket | `attestPrice` liest den Orakel-Eingang, `newSeq == cur.seq + 1` |
| **A20a-2** (niedrig) Patt bei t < tRot | `cancel` mit tRot; gegen einen Dieb mit allen Hauptschlüsseln: Wächter + Notfallsatz |
| **A20a-3** (niedrig) `sweep` unter `SWEEP_FEE` | `require(coll > SWEEP_FEE)` |
| **A20e-11** (niedrig) Ausbuchung ohne Verteilung | bewertet (Abschnitt 6), nicht im Vertrag |
| A20e-7/-8/-9 | Agent/Off-chain; v5 ändert nichts daran, macht aber die Wächter-Sperre unabhängig vom Orakel-UTXO (Lese-Spam kann sie nicht aufhalten) |
| Rahmen | ≤ 15 statische Signaturprüfungen je Eingang, Speichermasse/Blockgrenzen, KIP-9 |

## 2. Entscheidungen mit Begründung

### 2.1 Mindestabstand in Kettenzeit über das Alter der Register-UTXO

**Problem in v4:** Der Abstand 600 DAA wird an der *Orakel-DAA* gemessen, die der Signer selbst wählt (≤ Kettenstand). Liegt das Orakel 37 Updates hinter der Kette, gehen 37 Updates sofort (NEU-1, A20e-1). Ein Vertrag kann `tx.daa` nicht nach oben begrenzen (B1).

**Entscheidung:** `attestPrice`, `propose` und `cancel` verlangen `this.ageDaa >= minGapDaa` – die relative Sperre der Register-Haupt-UTXO, im Konsens durchgesetzt (in der Mainnet-Probe 05.10. bestätigt für das Ticket). Damit ist die Zahl der Preis-Updates je Kettenzeit hart begrenzt, unabhängig von der Orakel-DAA.

Damit das Alter eine verlässliche Uhr ist, darf niemand die Haupt-UTXO ohne Schlüssel ausgeben: **`witness` entfällt** (und mit ihm Zinsziel `payKind/payTo` und die Zinskasse, die nie in Gebrauch war; Zins geht wie heute über `interestSpk` des Vaults an eine Adresse oder eine unspendbare P2SH-Adresse). Ohne Schlüssel bleiben nur `activate` (braucht ein reifes Ticket) und `settle` (Tickets).

Verworfen:
- *Alter der Orakel-UTXO:* jedes `read` einer Vault-Tx setzt es zurück → jeder Nutzer (und jeder Spammer) blockiert Updates.
- *Abstand nur über `oracleDaa`:* wie v4, durch Rückdatieren aushebelbar.

Nebenwirkung, gewollt: Zwischen zwei Einträgen des Hauptsatzes liegen immer `minGapDaa`, in denen **nur** der Wächter (`guardLock`, ohne Mindestalter) die Haupt-UTXO ausgeben kann. Ein Dieb kann die Sperre nicht durch eine Kette eigener Register-Tx verdrängen.

### 2.2 Zwei-Preis-Regel (Referenzpreis)

Das Orakel v5 führt neben `kasUsd` einen **Referenzpreis** `refKasUsd`: einen Preis, der seit mindestens `refAfter` Updates öffentlich ist (Kandidat `candKasUsd`/`candSeq`, Fortschreibung im Vertrag, siehe `price_oracle_v5.sil`). Wegen 2.1 ist er mindestens **W = refAfter · minGapDaa Kettenzeit** alt, gleich wer die Updates gesendet hat.

| Aktion | v4 | v5 |
|---|---|---|
| `mint`, `withdraw` mit Schuld | MCR zum Preis | MCR zu **beiden** Preisen |
| `redeem` | ≥ Liq.-Schwelle, Auszahlung zum Preis | ≥ Schwelle zu beiden, Auszahlung zum **höheren** Preis |
| `liquidate` | Unterdeckung zum Preis | Unterdeckung zu **beiden**, Abrechnung zum aktuellen Preis |
| `sweep` | Zinsgebühr zum Preis | zum höheren Preis (kleinere Gebühr), `coll > SWEEP_FEE` |
| Pool `swap` | Bandregel zum Preis | Bandregel zu **beiden** Preisen |
| `close`, `repay`, `deposit` | – | unverändert |

Die Regel ist symmetrisch: Geld bewegt sich nur, wenn **beide** Preise es erlauben. Ein gefälschter aktueller Preis bei ehrlicher Referenz bringt nichts, eine gefälschte Referenz bei ehrlichem aktuellen Preis ebenfalls nichts (z. B. nach der Wiederherstellung). Ein Dieb braucht also mehr als `refAfter` eigene Updates, also mindestens W Kettenzeit – das ist das Fenster des Wächters.

Warum Liquidation zum *aktuellen* Preis abrechnet: Zum höheren Preis bekäme der Liquidator in einem echten Crash weniger als die Schuld wert ist (bei −10 % in W schon Verlust), Liquidationen stockten genau dann, wenn sie nötig sind. Restrisiko im Fenster: Vaults, die schon zur Referenz unter 150 % liegen, kann ein Dieb mit gefälschtem tiefem Preis bis zur ganzen Sicherheit statt Schuld + 10 % liquidieren. Das betrifft nur Vaults, die ohnehin liquidierbar sind (heute keiner, alle > 400 %).

Preis der Regel (ehrliche Lage): Liquidationen setzen erst ein, wenn auch die Referenz die Unterdeckung zeigt, also W bis 2W nach dem Kurssturz. Mit den Vorschlagswerten (5 min, 6 Updates) 30–60 min bei Crash-Takt. Vorbild: MakerDAO OSM (1 h Verzögerung). Bei MCR 200 % / Liquidation 150 % braucht es danach weitere −33 % bis zur Unterdeckung.

Verworfen:
- *Nur Sprunggrenze:* ×1,25 je 5 min sind in 30 min ×3,8 – reicht, um alles zu leeren. Hilft nur zusammen mit der Zwei-Preis-Regel.
- *Rotation nach Orakel-DAA (≥ 1 h):* rückdatierbar (2.1), Fenster wäre gefälscht. Rotation nach Update-Zahl ist sicher, weil jedes Update `minGapDaa` Kettenzeit kostet.
- *Nur Referenz (OSM) statt beider Preise:* Liquidationen und Band liefen dauernd W hinterher; mit beiden Preisen läuft alles normal, solange beide nah beieinander liegen.

### 2.3 Sprunggrenze je Update

`jumpBps` als Template-Konstante: neuer Preis in [kasUsd/(1+j), kasUsd·(1+j)]. v4 entspricht `jumpBps = 10 000` (×2/÷2). Vorschlag 2 500 (×1,25). Ehrlicher Crash −50 %: 4 Updates = 15 min bei 5 min Abstand. Die Grenze verlängert vor allem die Zeit **nach** dem Fenster (Abschnitt 5).

### 2.4 Wächter (Sperre)

- `guardHash = sha256(n ‖ g0 ‖ … ‖ g(n−1))`, n ≤ 3, im Register-Zustand, über Austausch änderbar. **Jeder einzelne Wächter** darf sperren (1-von-n): mehrere unabhängige Wachhunde, jeder Diebstahl eines Wächterschlüssels ist nur DoS.
- `guardLock`: signiert `sha256(regCov ‖ 0x05 ‖ nonce+1)`, kein Mindestalter, **kein Orakel in der Tx** (Lese-Spam O-1/A20e-8 kann die Sperre nicht blockieren). Setzt `locked = true`, nonce + 1 (entwertet jedes offene Ticket, auch eine Ankündigung des Diebs), nur wenn nicht schon gesperrt.
- Unter Sperre gesperrt: `attestPrice`, `propose` des Hauptsatzes, `cancel`. Das Orakel friert nach `freezeAfterDaa` (2 h) über den bestehenden `freeze` ein; bis dahin schützt die Zwei-Preis-Regel (ohne Updates rotiert die Referenz nicht, sie bleibt ehrlich).
- **Aufheben nur über `activate`** eines Tickets, das der **Notfallsatz** während der Sperre angekündigt hat (ohne Stille-Bedingung, Wartezeit `emergDelayDaa`). Der Hauptsatz kann es nicht absagen. Danach `locked = false`, das Orakel taut mit dem ersten Update des neuen Satzes auf.
- Signaturprüfungen: `init` 1 + `attestPrice`/`propose`/`cancel` je 4 + `guardLock` 1 = **14 ≤ 15** [B].

Verworfen:
- *Sperre im Orakel* (Feld `locked`): Orakel-UTXO wäre in der Sperr-Tx → durch Lese-Spam blockierbar, und jede Vault-Tx trüge die Zusatzbytes.
- *Schnelles Entsperren durch Hauptsatz + Wächter:* braucht eine weitere Quorum-Stelle (4 Signaturprüfungen → 18 > 15) und erlaubt bei gestohlenem Wächter einen Sperr-/Entsperr-Krieg.
- *Wächter m-von-n:* Sperren ist harmlos; 1-von-n macht die Reaktion schneller und robuster.

### 2.5 Austausch und Patt (A20a-2)

| Wer | v4 | v5 |
|---|---|---|
| `propose` regulär | tRot Hauptsatz | tRot Hauptsatz, nicht unter Sperre, Mindestalter |
| `propose` Notfall | tRot Notfallsatz, nach 30 Tagen Stille | tRot Notfallsatz, nach Stille **oder unter Sperre** |
| `cancel` | **t** Hauptsatz | **tRot** Hauptsatz, nicht unter Sperre, Mindestalter |
| `guardLock` | – | 1 Wächter |
| `activate` | jeder, reifes Ticket | jeder, reifes Ticket; hebt Sperre auf |

Ausgang der Szenarien:
- Dieb mit t < tRot Schlüsseln: kann nicht mehr absagen → ehrliche Mehrheit tauscht aus (14 Tage).
- Dieb mit allen Hauptschlüsseln (1-von-1 heute): Wächter sperrt, Notfallsatz kündigt neuen Satz an, Dieb kann nichts absagen → nach `emergDelayDaa` neuer Satz.
- Dieb mit Wächter: DoS bis zum Notfall-Austausch (`emergDelayDaa` + 2 h).
- Dieb mit Wächter **und** Notfallsatz: Übernahme nach `emergDelayDaa` (öffentlich sichtbar). Deshalb liegen Wächter und Notfallsatz getrennt (Wächter online auf eigenem Rechner, Notfallsatz kalt).
- Notfallsatz verloren, Hauptsatz verloren: Abwicklung (2.6).

### 2.6 Abwicklung (A20e-2)

Abwicklungszweig im Vault-Eintrag `redeem` (ein eigener Eintrag hätte `ghostDelta` ein weiteres Mal ausgerollt, +5,5 KB je Vault-Tx): Orakel **eingefroren** und `tx.daa ≥ oracleDaa + settleAfterDaa` (Vorschlag 60 Tage). Dann darf **jeder** GHOST an einem Vault mit Schuld zurückgeben, ohne Quotenprüfung:
- Preis p = höherer von aktuellem und Referenzpreis (ein gefälschter tiefer Schlusspreis zahlt nicht mehr aus).
- Deckt die Sicherheit die Schuld zu p (≥ 100 %): `amount·(1 − settleFeeBps)` USD in KAS, Abschlag (Vorschlag 5 %) bleibt beim Besitzer, Rest ≥ 0,2 KAS.
- Sonst: nur die ganze Schuld gegen die ganze Sicherheit (Vault endet). Eine anteilige Auszahlung `coll·amount/debt` läuft bei 1e16 × 5e9 über 64 Bit.
- Mindestens 0,01 GHOST oder die ganze Schuld; der Zins bleibt am Vault (GHOST-Inhaber vor der Zinskasse).

Missbrauch abgewogen:
- Nur nach 60 Tagen Stille. Der Wiederherstellungsweg (Sperre + Notfall-Austausch) braucht ≤ `emergDelayDaa` (7 Tage), der Stille-Weg 30 + 7 Tage, beide deutlich kürzer.
- Wer das Orakel absichtlich 60 Tage still hält (Lese-Spam A20e-8: 12–120 USD/Tag → 720–7 200 USD), bekommt eine Rücknahme zum alten Preis −5 %. Gewinn nur, wenn KAS in der Zeit um > 5 % gestiegen ist, und nur in der Höhe der eigenen GHOST; Vault-Besitzer verlieren dann den Kursgewinn auf den zurückgenommenen Teil. Rückdatieren der `oracleDaa` verkürzt die 60 Tage um höchstens den Rückstand (≈ 1 h).
- Taut das Orakel auf, ist die Abwicklung sofort wieder zu.

### 2.7 A20a-1, A20a-3

- `attestPrice` liest den Orakel-Eingang mit Template-Prüfung und verlangt `newSeq == cur.seq + 1`. Ein `read` mit wiederholten Signatur-Bytes scheitert; ein Notfall-Ticket verfällt nur noch durch ein echtes Update [B].
- `sweep` verlangt `coll > SWEEP_FEE` [B].

## 3. Vertragsschnittstellen

### 3.1 `contracts/price_oracle_v5.sil`
Konstruktor: `regCovId, maxRate, rateStep, rateGapDaa, freezeAfterDaa, jumpBps, refAfter`, Zustand `kasUsd, oracleDaa, seq, stableRate, stableIndex, frozen, lastRateDaa, refKasUsd, candKasUsd, candSeq` (Genesis: ref = cand = Startpreis, candSeq = Start-seq).
- `read()` unverändert, `freeze()` wie v4.
- `update(newKasUsd, newOracleDaa, newStableRate)`: wie v4, Sprunggrenze `jumpBps`, Referenz-Fortschreibung.

### 3.2 `contracts/signer_register_v5.sil`
Konstruktor: `deployer, minSigners, minThreshold, rotDelayDaa, emergAfterDaa, emergDelayDaa, minGapDaa`, Zustand `isTicket, setHash, fbHash, guardHash, nonce, emerg, locked, lastDaa, oracleCov, oracleTpl, oraclePrefixLen, oracleSuffixLen, initialized`.

| Eintrag | Argumente | Signiert |
|---|---|---|
| `init` | n, t, tRot, keys, oCov, oTpl, oPre, oSuf, sig | Tx-Signatur des Deployers |
| `attestPrice` | kas, daa, seq, rate, index, rateDaa, ref, cand, candSeq, n, t, tRot, keys, sigs, idx | `sha256(oracleCov ‖ 01 ‖ kas ‖ daa ‖ seq ‖ rate)` (wie v4) |
| `propose` | newN, newT, newTRot, newKeys, newFb, newGuard, isEmerg, n, t, tRot, keys, sigs, idx | `sha256(regCov ‖ 02/04 ‖ nonce+1 ‖ newSet ‖ newFb ‖ newGuard)`; Payload = `newSet ‖ newFb ‖ newGuard` (96 B) |
| `cancel` | n, t, tRot, keys, sigs, idx | `sha256(regCov ‖ 03 ‖ nonce+1)`, **tRot** Signaturen |
| `guardLock` | gn, gkeys, gi, gs | `sha256(regCov ‖ 05 ‖ nonce+1)` |
| `activate` | ticketIdx | – |
| `settle` (Ticket) | mainIdx | – |

Aufräumen abgelaufener Tickets: neben `propose`, `cancel`, `guardLock` oder `activate` (da `witness` entfällt). Neben einem Preis-Update geht es nicht, weil das Orakel genau einen Register-Eingang verlangt [B, `abgelaufenes_ticket_wird_neben_einem_register_eintrag_aufgeraeumt`]. Die 1 KAS eines abgelaufenen Tickets liegen also bis zum nächsten Austausch-Eintrag.

### 3.3 `contracts/stable_vault_v5.sil`
Konstruktor wie v4 plus `settleAfterDaa, settleFeeBps` (vor dem Zustand). Zustand unverändert (Factory unverändert nutzbar). Kein neuer Eintrag: `redeem(oracleIdx, amount, outStates)` hat bei eingefrorenem, `settleAfterDaa` altem Orakel den Abwicklungszweig (2.6), sonst die Regeln aus 2.2.

### 3.4 `contracts/ghost_pool_v5.sil`
Konstruktor wie v4; Orakel-Layout v5; Bandregel zu beiden Preisen.

### 3.5 Rust (`protocol/src/contracts.rs`, Abschnitt „Version 5“; `protocol/src/v5.rs`)
`OracleV5Params/State`, `RegisterV5Params/State`, `GuardSet`, `VaultV5Params`, Kompilier-Helfer, Digests (`lock_digest`, `announcement_v5`), Folgezustand `oracle_v5_next_state`. `v5.rs`: Bau der v5-Transaktionen für den Simulator (Deployment Register/Orakel, Update, Sperre, Ankündigung, Absage, Aktivierung, Einfrieren). Keine Änderung an `ops.rs`, `wallet_ops.rs`, `ghostctl.rs`.

## 4. Parameter-Vorschläge

| Parameter | Vertrag | Vorschlag | Begründung |
|---|---|---|---|
| `minGapDaa` | Register | 5 min = 3 000 | Takt des Agenten 300 s |
| `refAfter` | Orakel | 6 | Fenster W ≥ 30 min |
| `jumpBps` | Orakel | 2 500 (×1,25) | −50 % in 15 min ehrlich nachführbar |
| `freezeAfterDaa` | Orakel | 2 h | wie v4 |
| Hauptsatz | Register | **2 von 3** (t = tRot = 2) auf getrennten Rechnern; Übergang 1-von-1 möglich | ein gestohlener Schlüssel wirkungslos |
| `minSigners`/`minThreshold` | Register | 1/1 erlauben (Start), Ziel 3/2 | Template-Konstanten, nicht nachschärfbar |
| Wächter | Register | 1–3 Schlüssel, je auf eigenem Rechner mit Wachhund (Kettenpreis gegen Börsenmedian) | Sperre automatisch in Sekunden |
| Notfallsatz | Register | 1-von-1 kalt (Papier/Hardware), getrennt vom Wächter | |
| `rotDelayDaa` | Register | 14 Tage | wie v4 |
| `emergAfterDaa` | Register | 30 Tage | wie v4 |
| `emergDelayDaa` | Register | 7 Tage | Dauer einer Fehlalarm-Sperre |
| `settleAfterDaa` | Vault | 60 Tage | > 30 + 7 + Reserve |
| `settleFeeBps` | Vault | 500 (5 %) | Ausgleich für veralteten Preis |
| übrige | | wie v4 (MCR 200 %, Liq. 150 %, Bonus 10 %, 50 GHOST je Vault) | |

## 5. Angriffsrechnung vorher/nachher (gestohlener Signer)

Annahmen: Vorschlagswerte aus 4, ehrlicher Stand wie Audit 20e (167 KAS im System, alle Vaults > 400 %), Vault bei 200 % als ungünstiger Fall.

| Lage | v4 (live) | v5 |
|---|---|---|
| 1 Schlüssel von 1 gestohlen, Wächter reagiert innerhalb W (30 min) | alles in < 15 min (13 Updates ohne Wartezeit), heute 167 KAS | **≈ 0**: Referenz bleibt ehrlich; mint/redeem/withdraw/Pool bringen nichts. Rest: Vaults, die zur Referenz schon < 150 % sind, bis zur ganzen Sicherheit statt Schuld + 10 % (heute 0 KAS) |
| … Wächter reagiert nicht | 13 min | Referenz frühestens nach W = 30 min gefälscht (Faktor 1,25), nach 2W = 60 min Faktor 1,25⁷ ≈ 4,8: Vaults bei 200 % liquidierbar ab Faktor > 1,33 → **gesamter Bestand nach ≈ 60 min** (Prägen zu ×4,8 ebenso) |
| 1 Schlüssel von 2-von-3 gestohlen | (1-von-1 live) | **0** – braucht 2 Schlüssel |
| Dieb versucht, die Sperre zu verhindern | – | geht nicht: Hauptsatz-Einträge brauchen 5 min Mindestalter, Sperre nicht; Sperre ohne Orakel (kein Lese-Spam) |
| Gegenmaßnahme | neues Deployment | Sperre (Sekunden), Notfall-Austausch 7 Tage, Hauptsatz kann nicht absagen |
| Schlüsselverlust | System für immer eingefroren | Sperre + Notfall-Austausch (7 Tage); ohne Wächter Stille-Weg 30 + 7 Tage; ohne alles Abwicklung nach 60 Tagen |

Formel ohne Wächter: Referenz nach k Fenstern um höchstens (1+j)^((k−1)·refAfter+1) verschoben; Vault mit Quote r fällt, sobald dieser Faktor r/1,5 übersteigt. Mit `refAfter = 12` verdoppelt sich die Zeit (≈ 2 h), um den Preis von Liquidationen, die 1–2 h hinter einem echten Sturz liegen.

## 6. A20e-11: Ausbuchung ohne Verteilung (bewertet)

Varianten:
1. *Globaler Zähler ungedeckter GHOST* in einer Singleton-UTXO: jede Liquidation mit Ausbuchung müsste sie fortschreiben → zweiter Engpass neben dem Orakel, mehr Angriffsfläche (Lese-Spam), Nutzen nur informativ.
2. *Stabilitäts-Topf* (GHOST-Einlagen tragen Verluste, bekommen Sicherheit + Bonus): löst zugleich A20e-4, ist aber ein eigener großer Vertrag mit Anteilsrechnung wie der Pool. Eigenes Projekt (v6).
3. *Umlage auf alle Vaults* (Schuldindex): Vault müsste einen globalen Verlustindex aus dem Orakel lesen → Signer setzt Verluste, neues Vertrauensproblem.

Entscheidung v5: **nicht im Vertrag**. Off-chain: Status zählt ausgebuchte Schuld (Liquidation mit `seize == coll`: `debt − burn` des Vaults) und zeigt „ungedeckte GHOST“; die Zwei-Preis-Regel verzögert Liquidationen leicht (Abschnitt 2.2), daher Keeper mit GHOST-Reserve (A20e-4) wichtiger. Die Abwicklung (2.6) verteilt nicht anteilig: die ersten Rückgeber nehmen gedeckte Vaults zu 95 %, unterdeckte Vaults nur als Ganzes.

## 7. Zustände

Register (Haupt-UTXO):
```
 offen ──propose(S)──► offen + Ticket ──activate (rotDelay)──► offen (neuer Satz)
   │  ◄──cancel(S, tRot)──┘
   │──propose(F, Stille)─► offen + Notfall-Ticket ──activate (emergDelay)──► offen
   │  ◄──attestPrice / cancel──┘
   └──guardLock(G)──► gesperrt ──propose(F)──► gesperrt + Notfall-Ticket ──activate──► offen (neuer Satz)
                         (attestPrice, propose(S), cancel gesperrt; Orakel friert nach 2 h ein)
```
Orakel: `frisch ──(2 h ohne Update) freeze──► eingefroren ──update──► frisch`; eingefroren und 60 Tage alt → Vault-Abwicklung offen.

## 8. Tests und Messwerte

Siehe Abschnitt 11.

## 9. Umzug v4 → v5 (Skizze)

Wie v3 → v4 (`GHOST-Umzug-v4.command`):
1. Deployment v5 parallel (Register, Orakel, Factory, GHOST v5, Pool v5), Probe mit `--probe`-Fristen (1 h) im Mainnet wie 05.10.: Sperre, Notfall-Austausch unter Sperre, zu frühes Update (Sequenzsperre!), Abwicklung mit kurzer `settleAfterDaa` nur in der Probe.
2. Agent v5 parallel zum Agent v4 (eigene Zustandsdatei), Wachhund auf zweitem Rechner.
3. Nutzer: v4-Vault tilgen und schließen, v5-Vault öffnen und prägen. GHOST v4 und v5 sind verschiedene Token (eigene Covenant-ID); kein automatischer Tausch. Der Betreiber hält ≥ 83 % der Schuld selbst (Audit 20e) – er zieht zuerst um. v4-Pool: Einleger ziehen ab (add/remove frei).
4. v4 einfrieren lassen (Agent v4 aus), nach 2 h friert das Orakel v4; Restnutzer können tilgen/schließen. v4 hat keine Abwicklung: verbleibende v4-GHOST-Inhaber ohne Vault brauchen einen Schuldner (heute: Betreiber + 1 Nutzer, 0,6 GHOST). Vorschlag: Betreiber kauft Rest-GHOST v4 zurück, bevor der Agent v4 stoppt.

## 10. Offene Fragen an den Betreiber

1. **Parameter:** `minGapDaa` 5 min, `refAfter` 6 (Fenster 30 min, Liquidationsverzug 30–60 min) oder 12 (1 h / 1–2 h)? `jumpBps` 2 500?
2. **Wer hält den Wächter?** Vorschlag: Wachhund auf einem zweiten Rechner (anderer Ort, z. B. Hetzner-Server getrennt vom Agenten oder ein Raspberry zu Hause), vergleicht Kettenpreis mit Börsenmedian und sperrt bei > 15 % Abweichung. Zweiter Wächterschlüssel beim Betreiber auf dem Handy/Hardware?
3. **Wer hält den Notfallsatz?** Vorschlag: kalter Schlüssel (Papier/Hardware), nicht auf Agent- oder Wächter-Rechner. Wird der Notfallsatz vom Wächter getrennt gelagert?
4. **2 von 3 Unterzeichner** ab Start (drei Rechner) oder 1-von-1 mit späterem Austausch?
5. **emergDelay 7 Tage** (Fehlalarm kostet 7 Tage Stillstand) oder kürzer (3 Tage)?
6. **Abwicklung**: 60 Tage, Abschlag 5 % – einverstanden? Oder ganz weglassen (dann bleibt A20e-2 für den Fall „alles verloren“ offen)?
7. **Zinskasse entfällt** – einverstanden (Zins an eigene Adresse oder Verbrennen über `interestSpk`)?
8. Umzug: Rest-GHOST v4 zurückkaufen?

## 11. Ergebnisse [B]

### Größen (Redeem-Skript, `v5_sigops_tests`)

| Vertrag | v4 | v5 | Signaturprüfungen (statisch, ≤ 15) |
|---|---|---|---|
| Register | 6 539 B | 7 610 B | **14** (3 · 4 + init + guardLock) |
| Orakel (fährt in jeder Vault-/Pool-Tx mit) | 744 B | 939 B (+195 B) | 0 |
| Vault | 25 191 B | 26 454 B (+1 263 B) | 5 |
| Pool | ≈ 23,3 KB | 23 466 B | 1 |

Vault- und Pool-Tx werden dadurch um ≈ 1,5 KB bzw. 0,2 KB größer, [V] ≈ +0,003 KAS Gebühr je Vault-Tx.

### Gebühren und Massen (Simulator mit Mainnet-Parametern, `v5_sim_tests`, je 1 KAS in Register/Orakel/Ticket)

| Tx | Gebühr | compute / transient / storage |
|---|---|---|
| Preis-Update 1 Schlüssel | 0,0203 KAS (v4 0,0195) | 16 349 / 38 596 / 79 850 g |
| Preis-Update 2 von 3 | 0,0206 KAS | 17 487 / 39 148 / 79 850 g |
| Preis-Update 4 von 7 | 0,0211 KAS | 19 860 / 40 240 / 79 850 g |
| Wächter-Sperre | 0,0172 KAS | 11 298 / 32 672 / 39 981 g |
| Ankündigung (Payload 96 B) | 0,0185 KAS | 15 692 / 35 168 / 79 918 g |
| Notfall-Ankündigung unter Sperre | 0,0178 KAS | 12 397 / 33 988 / 79 981 g |
| Absage | 0,0171 KAS | 11 276 / 32 584 / 39 981 g |
| Aktivierung | 0,0330 KAS | 18 251 / 62 884 / 39 942 g |

Alle weit unter den Blockgrenzen (500 000 g). Stündlicher Heartbeat: ≈ 0,49 KAS/Tag wie v4.

### Zeitachse eines Diebs mit 1-von-1-Schlüssel (`a20e_1_zeitachse_des_angreifers`)

Ungünstigster Fall (Referenz rotiert beim ersten gefälschten Update), Orakel 1 h hinter der Kette, je Update −25 %, so schnell der Konsens es zulässt:

| Zeit | aktueller Preis | Referenz | Folge |
|---|---|---|---|
| 0 min | 0,0347 | 0,0434 (ehrlich) | nichts bewegt sich (Zwei-Preis-Regel) |
| 25 min | 0,0114 | 0,0434 | nichts |
| 30 min | 0,0091 | 0,0347 (÷1,25) | Vaults < 187 % zur ehrlichen Quote liquidierbar |
| 60 min | 0,0024 | 0,0091 (÷4,8) | alle Vaults bei 200 % liquidierbar, Prägen ×4,8 |

v4 zum Vergleich: 13 Updates ohne Wartezeit, alles in < 15 min. Mit Wächter: Sperre jederzeit in der nächsten Sekunde möglich (`absage_ohne_mindestalter_lehnt_der_konsens_ab`, `waechter_sperrt_dieb_kommt_nicht_mehr_durch_notfallsatz_tauscht_aus`), Referenz bleibt dann ehrlich.

### Tests

| Datei | Tests | Inhalt |
|---|---|---|
| `v5_sigops_tests.rs` | 3 | Signaturprüfungen je Vertrag (genau gezählt), Größen v4/v5 |
| `v5_register_tests.rs` | 20 | A20a-1 Replay, A20a-4, Mindestabstand (Update, Ankündigung, Absage), Wächter (jeder/fremd/Index/Liste/alte nonce/Ausgang/neben Orakel-Update/zweite Sperre), Sperre entwertet Tickets, Notfallsatz unter Sperre, A20a-2 (Dieb mit allen Schlüsseln; Absage mit tRot), Aktivierung hebt Sperre auf, Aufräumen |
| `v5_oracle_tests.rs` | 6 | Sprunggrenze exakt, v4-Äquivalenz (10 000), Referenz-Rotation, Orakel rechnet Referenz selbst, read/freeze |
| `v5_vault_tests.rs` | 11 | Zwei-Preis-Regel für mint/withdraw/liquidate/redeem/sweep (Beleg A20e-1 aus dem Audit: 1 000 KAS / 20 GHOST bei ÷4 gefälscht), A20a-3, Abwicklung (Frist, ohne Quote, Mindestbetrag, Fälschungspreis, unterdeckt nur im Ganzen) |
| `v5_vault_regress_tests.rs` | 95 | alle v4-Vault-Tests gegen Vault v5 |
| `v5_pool_tests.rs` | 38 | alle 36 v4-Pool-Tests gegen Pool v5 + 2 Tests Kursband zu beiden Preisen |
| `v5_sim_tests.rs` | 5 | Mindestabstand im Konsens trotz Rückstand, Zeitachse, Sperre + Notfall-Austausch + Aktivierung mit Sequenzsperre, Massen |

Gesamtlauf `cargo test --release --offline`: **856 bestanden, 0 fehlgeschlagen**, 2 ignoriert (wie vorher), davon 178 v5-Tests.

Nicht gemacht: Mutationstest der v5-Verträge (`protocol/mutation/mutate.sh`) – vor dem Audit nachholen.

## 12. Aufwand bis zum Umzug (Schätzung)

| Schritt | Aufwand |
|---|---|
| Mutationstest v5, Lücken schließen | 0,5 Tag |
| Anbindung: Deployment-Struktur v5, `ops`/`ghostctl` (deploy, oracle-update mit Sequenz, guard-lock, signers propose/cancel/activate v5, Abwicklung als `redeem`), `chain`/`store`-Folger für Register v5 und Orakel v5, Vault-/Pool-Aktionen gegen v5 | 2–3 Tage |
| Agent: Sequenz für Updates, Referenzpreis im Keeper (Liquidation erst bei Unterdeckung zu beiden), Wachhund als eigenes Programm (Kettenpreis gegen Börsenmedian, sperrt) | 1–2 Tage |
| Seite: Referenzpreis, Sperre, Notfall-Ticket, Abwicklung, Hinweise | 1 Tag |
| Testnet/Mainnet-Probe mit kurzen Fristen (Sperre, Notfall-Austausch, zu frühes Update, Abwicklung) | 1 Tag + Wartezeiten |
| Unabhängiges Audit (Verträge + Anbindung) | 1–2 Tage |
| Umzug v4 → v5 (Skript wie `GHOST-Umzug-v4.command`, Rest-GHOST v4) | 1 Tag |
| **Summe** | **≈ 8–11 Arbeitstage** |
