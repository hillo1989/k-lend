# Audit 11, Teil A: GHOST Version 3, Vertragsebene

**Datum:** 29.09.2026 · **Prüfer:** Claude Opus 5.5 (unabhängig) · **Stand:** Worktree `kaspa-lending-v3`, Branch `v3`, nur gelesen
**Gegenstand:** `contracts/stable_vault.sil` (v3), `contracts/vault_factory.sil`, Zusammenspiel mit `contracts/ghost_token.sil` und `contracts/risk_oracle.sil`, `protocol/src/math.rs`, `protocol/src/contracts.rs`, Tests `vault_tests.rs`, `vault_math_tests.rs`, `e2e_tests.rs`, `factory_tests.rs`.
**Methode:** Code-Lektüre Zeile für Zeile. Grenzwert- und Überlaufrechnung. Beweistests als ganze Transaktionen in der echten Skript-Engine, in einer Scratch-Kopie (Harness aus `vault_tests.rs`). Mutanten L165 und L268. Außerdem eine Kopie mit einem Compiler, dem der Laufzeit-Schleifenwächter fehlt, als Simulation einer künftigen Compiler-Version. Die Konsensregeln für Covenant-Bindungen habe ich in rusty-kaspa `a41a333` nachgelesen.

---

## Zusammenfassung

Der Kern von Version 3 hält. GHOST entsteht nur gegen gleich hohe Schuld, und die Summe aller `debt` bleibt immer ≤ der GHOST-Menge (V-03 ist behoben). Fremde KAS lassen sich nur nach den Regeln von Rücknahme und Liquidation entnehmen. Eine Rücknahme verschlechtert die Quote eines Vaults nie. Das ist in 499 976 Zufallsfällen belegt, eine Mindestquote nach der Rücknahme ist also nicht nötig. Der wichtigste neue Befund betrifft die Zinskasse. Schließt ein Besitzer mehrere Vaults in **einer** Transaktion, zeigen alle `treasuryIdx` auf denselben Ausgang, und die Kasse bekommt nur die größte Einzelgebühr (belegt, **mittel**). Wirtschaftlich ist die Rücknahme mit fest 0,5 % billiger als die Feed-Schwelle von 1 %. Läuft das Orakel dem Markt nach, lohnt sie sich deshalb auch bei GHOST = 1,00 USD, und zwar auf Kosten der Vault-Besitzer (**mittel**). Hinzu kommen zwei niedrige Befunde: Jeder kann für 2·10⁻⁸ GHOST fremde Vaults bewegen, und nach einer Liquidation bleiben Vaults mit Schuld 0 und Zins > Sicherheit zurück, die niemand auflöst. Von den Mutationslücken sind alle bis auf L165 tatsächlich doppelt gesichert. L165 ist im Vertrag die einzige Sperre gegen einen Orakelpreis ≤ 0 und lässt sich testen (Test liegt bei).

---

## Befundtabelle

| ID | Schwere | Befund | Beleg | Empfehlung |
|---|---|---|---|---|
| **A11-V-1** | **mittel** | **Zinskasse per Sammel-Schließung prellbar.** `close` prüft nur `outputs[treasuryIdx]` ≥ eigene Gebühr (`stable_vault.sil:257-260`). Mehrere Vaults in einer Tx dürfen auf denselben Kassen-Ausgang zeigen. Die Kasse bekommt dann max(Gebühren) statt der Summe. Bei 50 GHOST je Vault (Mainnet) haben größere Schuldner zwangsläufig viele Vaults. | **belegt:** `a11_zwei_vaults_teilen_sich_eine_zinszahlung`: 2 Vaults mit je 50 KAS Zins, 1 Ausgang mit 50,00000025 KAS → `[Ok, Ok, Ok]`. Verlust 50 KAS. Gegenprobe 1 sompi weniger → Vault 2 lehnt ab. | Kassen-Ausgang injektiv an den eigenen Input binden, z. B. `require(treasuryIdx == this.activeInputIndex + 1)` (passt zur Ausgangsreihenfolge in `ops::close`) oder `== this.activeInputIndex`. Mit `== activeInputIndex` geprüft (`a11_fix_kassen_ausgang_je_input`): ehrlich ok, geteilt abgelehnt. |
| **A11-V-2** | **mittel** | **Rücknahme gegen nachlaufendes Orakel.** Ausgezahlt wird zum Orakelpreis mit 0,5 % Abschlag. Der Feed aktualisiert erst ab 1 % Änderung (Heartbeat 6 h, `ghostctl.rs:117-122`). Liegt der Markt mehr als ≈ 0,5 % über dem Orakel, bringt die Rücknahme auch bei GHOST = 1,00 USD Gewinn. Den trägt der Vault-Besitzer: Er verkauft KAS unter Markt. Das Ziel ist frei wählbar, auch Vaults mit 300 %. | **belegt (Rechnung + Tx):** `a11_ruecknahme_gegen_nachlaufendes_orakel`: 50 GHOST ⇒ 1 243,75 KAS; Markt +1 % ⇒ +0,2475 USD, +2 % ⇒ +0,745 USD, +5 % ⇒ +2,24 USD. Die Rücknahme des ganzen Vaults (50 GHOST, 150 %) geht durch. | Rücknahmegebühr ≥ Feed-Schwelle + Marge (z. B. 1,5 %) oder dynamisch (Liquity-`baseRate`). Alternativ Feed-Schwelle unter die Gebühr senken. Doku: Rücknahme trifft beliebige Vaults, nicht die riskantesten. |
| **A11-V-3** | niedrig | **Rücknahme ohne Mindestmenge: Fremde bewegen gesunde Vaults fast kostenlos.** Mit 2 Einheiten (2·10⁻⁸ GHOST) ändern sich Outpoint und Zustand jedes Vaults ≥ 150 %. Das ist die Klasse von N-4 („Fremdtilgung sperrt den Besitzer aus“), jetzt über `redeem` wieder offen. Betroffen ist auch `deposit` des Besitzers (Rennen um die UTXO). | **belegt:** `a11_ruecknahme_mit_zwei_einheiten_bewegt_fremden_vault`: 2 Einheiten ⇒ 25 sompi, `all_ok`. Kleinste Menge: 2 Einheiten bis 1 USD/KAS, 905 Einheiten bei 900 USD/KAS. | `require(amount >= MIN_REDEEM \|\| amount == debt)`, z. B. MIN_REDEEM = 5 GHOST. Dann lässt sich ein Vault mit 50 GHOST höchstens 10-mal anfassen, danach ist die Schuld 0. |
| **A11-V-4** | niedrig | **Zombie-Vaults nach Liquidation.** Bei Rest ≥ DUST bleibt der Zins stehen (`:341`). Nach voller Tilgung ist `debt = 0`, und `liquidate` ist gesperrt (`burn ≤ debt`). Ist der Zins ≥ Wert des Rests, bekommt der Besitzer beim `close` nichts und hat keinen Anreiz. Die Kasse erhält nichts, und die KAS (plus 3 KAS Minter-Zweig) liegen fest. GHOST-Deckung ist nicht betroffen. | **belegt:** `a11_zombie_vault_nach_liquidation`: 100 GHOST + 150 USD Zins, 200 USD Sicherheit → Rest 2 250 KAS (90 USD), Zins 150 USD. Weitere Liquidation und Abheben scheitern, `close` nur mit 100 % an die Kasse, `deposit` geht. | Eintrag „jeder darf einen Vault mit `debt == 0` und Zinsgebühr ≥ Sicherheit an die Zinskasse schließen“, oder bei `burn == debt` die Kasse im selben Zweig bedienen. Mindestens dokumentieren. |
| **A11-V-5** | Info | **Zins hängt von der Abrechnungshäufigkeit ab.** Der Index verzinst sich je Orakel-Update, der verbuchte `interest` wächst nicht mit. Häufiges Abrechnen (Besitzer über `withdraw`/`repay` mit 1 Einheit, oder jeder Rücknehmer) senkt den Zins. | **belegt:** `a11_zins_haengt_von_der_abrechnungshaeufigkeit_ab`: 50 GHOST, 20 % p. a., stündliche Updates: einmal abgerechnet 11,0697 USD, stündlich 10,0000 USD (−9,7 %). | Dokumentieren, oder pfadunabhängig rechnen: `interest + accrual(debt + interest, indexAt, index)` (überlauffrei bis 2e17, siehe Details). |
| **A11-V-6** | Info | **L165 (`kasUsd > 0`) ist im Vertrag die einzige Sperre gegen Preis ≤ 0 und lässt sich testen.** Ohne sie gilt jeder Vault als ungesund, und `seize = coll` (`:325` ist bei Wert 0 falsch). Ein Liquidator nimmt mit 1 Einheit GHOST die ganze Sicherheit. Gesichert ist das sonst nur außerhalb des Vaults: durch die Update-Regel des Orakels (≥ 1 000) und die Startpreis-Prüfung in `ghostctl.rs:789-791` (off-chain). | **belegt:** `a11_preis_null_nur_durch_l165_gesperrt`: Original `[Err, Ok, Ok, Ok]`, Mutante L165 `[Ok, Ok, Ok, Ok]` (Test rot). | Test übernehmen. Damit ist die Lücke geschlossen. |
| **A11-V-7** | Info | **Die Zahl der GHOST-Ausgänge hat im Vault keine ausdrückliche Obergrenze** (`ghostDelta`, `:180-184`; nur `mint` hat L268). Heute schützt doppelt: (a) der Laufzeit-Schleifenwächter des Compilers (`for.rs:136-137`, im Quelltext mit „TODO: Consider moving check to debug-mode compilation“), (b) das ausdrückliche `out_count ≤ to` des KCC20-Leaders (`covenant_declarations.rs:745`). | **belegt:** Mit einem Compiler ohne Wächter nimmt der **Vault** eine Rücknahme mit drittem Ausgang „1 Mio GHOST an Dieb“ an (`[Ok, Ok, Err, Ok]`). Nur KCC20 (Input 2) lehnt ab. Mit dem heutigen Compiler lehnen beide ab. | `require(nOut <= MAX_GHOST_OUTS)` in `ghostDelta` ergänzen (Symmetrie zu L182). Beim Compiler-Update Schleifenwächter und `GHOST_MAX_OUTS == MAX_GHOST_OUTS` prüfen. |
| **A11-V-8** | Info | **DUST-Erlass ist fest in KAS.** Erlassen werden bei 0,04 USD/KAS 0,008 USD, bei 1 USD 0,20 USD, bei 50 USD 10 USD und bei 900 USD 180 USD je Vault. Aufteilen auf viele Vaults lohnt nicht, weil jeder Vault 3 KAS Minter-Zweig dauerhaft bindet (`ops.rs:30`). | **belegt (Rechnung):** Ausgabe von `a11_zins_haengt_…`. | Nur dokumentieren. Bei hohem KAS-Kurs DUST neu bewerten. |
| **A11-V-9** | Info | **maxRate 1e9 ≙ 31,54 % p. a.** (`ghostctl.rs:794`) liegt über der Agent-Grenze von 20 % (`math.rs:145`). Ein unbegründeter Liquidationssprung über den Satz ist nicht möglich: Von 200 % auf 150 % braucht es ≥ 0,91 Jahre bei Höchstsatz. Das Komitee kann über den Preis (×2/÷2 je Minute) ohnehin mehr. | **belegt (Rechnung)** | maxRate auf die Agent-Grenze (≈ 6,34e8 für 20 %) setzen, damit auch ein fehlerhaftes Komitee-Update nicht darüber geht. |
| **A11-V-10** | Info | **Doku und Tests.** (a) `math.rs:95` sagt „bei Teil-Liquidation sinkt der Zins anteilig“, der Code lässt ihn stehen. (b) `v3_schliessen_rechnet_den_zins_bis_zum_orakelindex` kann nichts Zinsiges prüfen: `close` verlangt `debt == 0`, also ist `accrued(index) == interest`. (c) Die „Gegenprobe“ `mint_mit_echtem_orakel_und_gleichem_betrag_als_gegenprobe` führt keine Tx aus. | **belegt:** Code. Tx-Gegenprobe `a11_gegenprobe_falsches_orakel_als_tx` grün (echtes Orakel, 100-facher Preis, 1 000 GHOST). | Kommentar korrigieren, Test umbenennen, Tx-Gegenprobe übernehmen. |

---

## Details je Befund

### A11-V-1 (mittel, belegt): Zinskasse per Sammel-Schließung prellbar

**Ort:** `contracts/stable_vault.sil:247-262`, besonders `:257-260`

```
if (fee >= DUST) {
    byte[36] kasse = new ScriptPubKeyP2PK(pubkey(treasury));
    require(tx.outputs[treasuryIdx].scriptPubKey == byte[](kasse));
    require(tx.outputs[treasuryIdx].value >= fee);
}
```

Jeder Vault-Input prüft für sich, ob **ein** Ausgang an die Kasse mindestens seine Gebühr trägt. Nichts verhindert, dass ein zweiter Vault-Input in derselben Tx denselben `treasuryIdx` angibt. Drei Dinge machen das praktisch:

- `close` ist ein `noGhost`-Eintrag. Mehrere Vaults ohne GHOST-Gruppe dürfen in einer Tx stehen, denn nur Einträge mit GHOST-Gruppe schließen sich gegenseitig aus, weil Eingang 0 der Gruppe der eigene Minter sein muss.
- Alle Vaults lesen dasselbe Orakel-Input (`oracleIdx` gleich), und `read()` erzeugt es einmal neu.
- Auf dem Mainnet gilt `maxDebt` = 50 GHOST je Vault. Wer mehr leihen will, braucht mehrere Vaults und schließt sie später gern zusammen.

**Beleg:** `a11_zwei_vaults_teilen_sich_eine_zinszahlung`. Zwei Vaults desselben Besitzers (Covenant-IDs `0x0c…`, `0x0e…`) haben je ≈ 2 USD Zins, also 50 KAS Gebühr bei 0,04 USD/KAS. In der Tx gibt es **einen** P2PK-Ausgang an die Kasse über 5 000 000 025 sompi, und beide Vaults geben `treasuryIdx = 1` an. Ergebnis `[Ok(()), Ok(()), Ok(())]`: Die Kasse erhält 50 KAS statt 100 KAS. Gegenproben: Mit zwei ehrlichen Ausgängen geht die Tx durch. Mit 1 sompi unter der größten Einzelgebühr lehnt Vault 2 ab.

**Wirkung:** Bei N gemeinsam geschlossenen Vaults zahlt der Besitzer nur die größte Einzelgebühr, der Rest (N−1 Gebühren) fehlt der Zinskasse. Die Signatur jedes Besitzers deckt die ganze Tx (SIG_HASH_ALL). Fremde Besitzer können also nur kooperativ bündeln, etwa über einen „Sammel-Schließ-Dienst“. Der eigene Besitzer kann es jederzeit. Nutzergelder sind nicht betroffen, nur die Einnahmen des Protokolls.

**Empfehlung:** Jeder Vault-Input bekommt einen eigenen Kassen-Ausgang über eine injektive Abbildung Input → Ausgang:

- `require(treasuryIdx == this.activeInputIndex + 1);` Das passt ohne Umbau zu `ops::close` (Vault = Input 0, Kasse = Ausgang 1, Orakel-Fortsetzung = Ausgang 0) und ist für Sammel-Schließungen weiter injektiv.
- Oder `require(treasuryIdx == this.activeInputIndex);` Diese Variante habe ich geprüft: `a11_fix_kassen_ausgang_je_input` ist mit dem Fix grün. Ehrliche Doppel-Schließung `[Ok, Ok, Ok]`, geteilter Ausgang `[Ok, Err, Ok]`, Einzel-Schließung `[Ok, Ok]`. `ops::close` müsste dafür die Ausgänge umordnen.

### A11-V-2 (mittel, belegt): Rücknahme gegen nachlaufendes Orakel

**Ort:** `stable_vault.sil:294-307` (`paid = ⌊⌊amount·0,995⌋·1e8/kasUsd⌋`), Feed-Voreinstellungen `protocol/src/bin/ghostctl.rs:117-122` (`min_change` 1 %, `max_age_min` 360, Intervall 120–300 s).

Der Rücknehmer bekommt KAS zum **Orakel**preis. Steigt KAS am Markt, bleibt das Orakel bis knapp 1 % darunter stehen, bis zu 6 Stunden lang, bei schnellen Bewegungen zusätzlich eine Abfrageperiode. Das Dauer-`read()` aus O-1 (offen) kann das Update zusätzlich verzögern. Der Gewinn des Rücknehmers bei GHOST = 1,00 USD ist `0,995·p_Markt/p_Orakel − 1`:

| Markt über Orakel | 50 GHOST ⇒ KAS | Marktwert | Gewinn Rücknehmer = Verlust Besitzer |
|---|---|---|---|
| 0 % | 1 243,75 | 49,75 USD | −0,25 USD |
| +0,5 % | 1 243,75 | 49,9988 USD | −0,0013 USD |
| +1 % | 1 243,75 | 50,2475 USD | **+0,2475 USD** |
| +2 % | 1 243,75 | 50,745 USD | **+0,745 USD** |
| +5 % | 1 243,75 | 52,2375 USD | **+2,2375 USD** |

**Wirkung:** Oberhalb von ≈ 0,5 % Nachlauf ist die Rücknahme ein risikoloses Geschäft gegen jeden Vault ≥ 150 %. Der Kurs von GHOST spielt dabei keine Rolle. Die Untergrenze für den GHOST-Kurs funktioniert zwar, aber die Besitzer zahlen zusätzlich jede Orakel-Verzögerung nach oben. Ziel ist jeder beliebige Vault, auch einer mit 300 %. Die riskanten Vaults zwischen 150 und 200 % bleiben dagegen stehen. Anders als bei Liquity gibt es keine Reihenfolge nach Quote, und die ist im UTXO-Modell auch nicht erzwingbar.

**Empfehlung:** Gebühr über der Feed-Schwelle (z. B. 1,5 %) oder dynamisch nach Liquity-`baseRate`, die nach jeder Rücknahme steigt und mit der Zeit abklingt. Alternativ die Feed-Schwelle deutlich unter die Rücknahmegebühr senken, was mehr Updates kostet. In der Doku für Vault-Besitzer offen aussprechen: „Rücknahme kann jederzeit jeden Vault ab 150 % treffen, zum Orakelpreis.“

### A11-V-3 (niedrig, belegt): Rücknahme ohne Mindestmenge

**Ort:** `stable_vault.sil:294-305`. Die einzige Untergrenze ist `paid > 0` (L304).

Vor v3 konnten Dritte einen gesunden Vault nicht mehr bewegen: V-04 (deposit) und N-4 (repay) wurden gerade deshalb behoben, N-4 wurde als **mittel** bewertet. `redeem` öffnet diese Klasse bewusst wieder, und zwar ohne Mindestmenge. Mit 2 Einheiten (2·10⁻⁸ GHOST) fließen 25 sompi an den Rücknehmer. Outpoint und Zustand des Vaults ändern sich, der Zins wird abgerechnet, und alle vorbereiteten Transaktionen des Besitzers werden ungültig. Das betrifft auch `deposit`, das kein Orakel braucht und deshalb von O-1 nicht erfasst ist. Die Kosten je Anstoß sind im Wesentlichen die Tx-Gebühr.

Seit dem O-1-Fix synchronisiert `ghostctl` fremde Vault-Änderungen nach. Es bleibt ein reines Rennen um die UTXO, deshalb nur **niedrig**. Liquidationen lassen sich damit nicht blockieren, weil Rücknahme (≥ 150 %) und Liquidation (< 150 %) disjunkt sind.

**Beleg:** `a11_ruecknahme_mit_zwei_einheiten_bewegt_fremden_vault` (`all_ok`). Die kleinste wirksame Menge liegt bei 2 Einheiten für Preise von 0,00001 bis 1 USD/KAS und bei 905 Einheiten bei 900 USD/KAS.

**Empfehlung:** `require(amount >= MIN_REDEEM || amount == debt);` Mit MIN_REDEEM = 5 GHOST lässt sich ein Vault mit 50 GHOST höchstens zehnmal anfassen, danach ist die Schuld 0 und nichts mehr rücknehmbar. Die Belästigung ist damit begrenzt, die Funktion als Untergrenze für den GHOST-Kurs bleibt.

### A11-V-4 (niedrig, belegt): Zombie-Vaults nach Liquidation

**Ort:** `stable_vault.sil:337-342` (Rest ≥ DUST → `continueWith(rest, debt − burn, owedInterest, index)`), `:319-320` (`burn > 0`, `burn ≤ debt`), `:240` (withdraw braucht 200 % auf den Zins), `:254-256` (Gebühr gedeckelt auf die Sicherheit).

Ist ein Vault wegen Schuld und Zins ungesund und verbrennt der Liquidator die ganze Schuld, bleibt der Zins im Rest-Vault stehen. Danach gilt `debt = 0`, und eine weitere Liquidation ist unmöglich. Ist der Zins (in KAS) ≥ Rest, bekommt der Besitzer beim `close` nichts, also schließt er nie. Die Zinskasse erhält nichts, weil niemand außer dem Besitzer schließen darf. Die KAS des Rests und die 3 KAS des Minter-Zweigs liegen dauerhaft fest.

**Beleg:** `a11_zombie_vault_nach_liquidation`. 100 GHOST Schuld, 150 USD Zins, 5 000 KAS à 0,04 USD = 200 USD (80 %). Die volle Liquidation geht durch, übrig bleiben 2 250 KAS = 90 USD bei 150 USD Zins. Danach scheitert `liquidate` mit 1 Einheit am Vault, und `withdraw` scheitert ebenfalls. `close` geht nur mit 100 % an die Kasse (`close_fee == rest`), ein Abzweig an den Besitzer wird abgelehnt. `deposit` geht, die Hülle bleibt also bestehen.

**Realismus:** Dafür muss der Rest-Wert (V − 1,1·D) unter dem Zins I liegen. Beispiel: I = 0,2·D (ein Jahr bei 20 %) und eine Deckung unter ≈ 108 % von D + I, also ein Crash bei alten Vaults. Die GHOST-Deckung ist nicht betroffen, weil `debt` 0 ist und kein GHOST ohne Schuld umläuft. Es geht nur um Einnahmen der Kasse und festliegende KAS.

**Folgen für `keeper_burn`:** `math.rs:130` gibt bei `debt == 0` `None` zurück. Der Agent versucht diese Vaults nicht. Das ist richtig.

**Empfehlung:** Entweder (a) einen Eintrag `collect(oracleIdx, treasuryIdx)` für jeden, wenn `debt == 0` und `⌈Zins·1e8/kasUsd⌉ ≥ collateral()`: alles an die Kasse, Vault endet. Oder (b) im Liquidationszweig mit `burn == debt` die Kasse gleich mitbedienen und den Rest per P2PK-Prüfung an den Besitzer geben. Oder (c) als bewusste Grenze dokumentieren.

### A11-V-5 (Info, belegt): Zins hängt von der Abrechnungshäufigkeit ab

**Ort:** `stable_vault.sil:121-136` (`accrual(debt, indexAt, index)`), `risk_oracle.sil:107-109` (Index verzinst sich je Update).

Der Index wächst multiplikativ je Orakel-Update. Der verbuchte `interest` wächst aber nicht mit, es gibt keinen Zins auf den Zins. Damit ist der Zins pfadabhängig:

- Selten abgerechnet: D·(I_n/I_0 − 1), mit Zinseszins aus dem Index.
- Oft abgerechnet: Σ D·(I_{k+1}/I_k − 1), einfach verzinst.

Beleg: 50 GHOST bei 20 % p. a. und stündlichen Updates über ein Jahr (Index 1,2214) ergeben 11,0697 USD bei einmaliger und 10,0000 USD bei stündlicher Abrechnung. Das sind −9,7 % Kasseneinnahmen. Abrechnen kann der Besitzer über `withdraw` um 1 sompi oder `repay` um 1 Einheit, und jeder Rücknehmer (A11-V-3) tut es nebenbei. Bei 50 GHOST lohnt das wegen der Tx-Gebühren nicht, bei höheren Grenzen schon eher.

**Empfehlung:** Dokumentieren, oder pfadunabhängig rechnen: `interest + accrual(debt + interest, indexAt, index)`. Die Überlaufgrenzen reichen (debt + interest ≤ 2e17 ⇒ ⌊d/1e9⌋·g ≤ 1,8e18, (d mod 1e9)·g < 9e18). Dann wächst der Zins allerdings auch bei `debt == 0` weiter. Das ist eine Designentscheidung.

### A11-V-6 (Info, belegt): L165 ist die einzige Sperre gegen Preis ≤ 0 im Vault

**Ort:** `stable_vault.sil:165`. Folgen ohne die Zeile: `healthy` liefert bei `kasUsd == 0` Wert 0 und damit ungesund (`:139-143`). `mulDivDown(coll, 0, …) > claim` ist falsch, also `seize = coll` (`:324-327`), ohne Division durch 0 auf diesem Pfad.

**Beleg:** `a11_preis_null_nur_durch_l165_gesperrt`: Ein gesunder Vault (10 GHOST gegen 400 USD) wird mit 1 Einheit GHOST liquidiert, und die ganze Sicherheit geht an den Liquidator. Original: `[Err("VerifyError"), Ok, Ok, Ok]`. Mutante L165: `[Ok, Ok, Ok, Ok]`, der Test wird rot.

**Einordnung:** Im Betrieb ist das unerreichbar, solange das Orakel `update` nur mit ≥ 1 000 annimmt (`risk_oracle.sil:78`) und der Startzustand korrekt ist. Den Startzustand prüft **nur** Off-chain-Code (`ghostctl.rs:789-791`). Die Einschätzung „doppelt gesichert“ stimmt also nur, wenn man das Orakel und das Deploy-Werkzeug mitzählt. Im Vault selbst ist L165 allein. Der Test schließt die Mutationslücke.

### A11-V-7 (Info, belegt): Keine ausdrückliche Obergrenze für GHOST-Ausgänge im Vault

**Ort:** `stable_vault.sil:178-220`. Für Eingänge gibt es `nIn <= MAX_GHOST_INS` (L182), für Ausgänge fehlt das Gegenstück. `mint` hat L268 (`== 2`), `repay`, `redeem` und `liquidate` haben nichts. Die Schleife `for(j, 0, nOut, MAX_GHOST_OUTS)` bricht heute nur ab, weil der Compiler einen Laufzeitwächter einsetzt (`vendor/silverscript/silverscript-lang/src/compiler/for.rs:136-157`). Der Quelltext kommentiert ihn mit „TODO: Consider moving check to debug-mode compilation“, und `TUTORIAL.md` warnt ausdrücklich davor, sich darauf zu verlassen.

**Beleg:** In einer Kopie mit gepatchtem Compiler (Wächter-Grenze auf 1 000 000, `for.rs:150`) ergibt sich:

- `a11_drei_ghost_ausgaenge_bei_ruecknahme`: Rücknahme von 10 GHOST plus dritter Ausgang „1 000 000 GHOST an Dieb“ ergibt `[Ok, Ok, Err, Ok]`. Der **Vault nimmt an**, nur der KCC20-Leader (Input 2) lehnt ab.
- `a11_drei_ghost_ausgaenge_beim_praegen` mit Mutante L268: `[Ok, Ok, Err]`, ebenfalls nur KCC20.
- Mit dem heutigen Compiler: `[Err, Ok, Err, Ok]` bzw. `[Err, Ok, Err]`. Es lehnen beide ab.

Die zweite Sicherung ist das ausdrücklich erzeugte `require(__cov_out_count <= to)` im KCC20-Leader (`covenant_declarations.rs:745`), dazu `in_count ≤ from` (`:728`). Weil `maxCovOuts = 2` Teil des GHOST-Templates ist, dessen Hash der Vault festhält, gilt das auch künftig. Die Einschätzung „doppelt gesichert“ hält also, eine der beiden Sicherungen liegt aber außerhalb des Vaults.

**Empfehlung:** `require(nOut <= MAX_GHOST_OUTS);` in `ghostDelta` ergänzen. Das kostet wenige Bytes und macht den Vault unabhängig von Compiler-Interna.

### A11-V-8 bis A11-V-10 (Info)

- **V-8:** DUST = 0,2 KAS ist fest. Erlassen werden je Vault 0,008 / 0,20 / 10 / 180 USD bei 0,04 / 1 / 50 / 900 USD je KAS. „DUST-Farming“ über viele kleine Vaults lohnt nicht: Jeder Vault bindet 3 KAS Minter-Zweig (`ops.rs:30`), die nach `close` für immer unter der untergegangenen Vault-ID liegen. Das ist mehr als 15-mal der mögliche Erlass.
- **V-9:** `risk_oracle.sil:90-91` begrenzt den Satz auf `maxRate` = 1e9 ≙ 31,54 % p. a. bei 10 DAA/s. Der Agent geht höchstens auf 20 %. Für den abgelaufenen Zeitraum gilt der alte Satz (`:107-109`), rückwirkend lässt sich also nichts ändern. Von 200 % auf 150 % braucht es Faktor 4/3 auf die Schuld, also ≥ 0,91 Jahre bei Höchstsatz. Ein Komitee mit 3 von 5 Schlüsseln kann über den Preis (÷2 je 600 DAA) in Minuten liquidieren, der Zinspfad ist dagegen nachrangig.
- **V-10:** `math.rs:93-95` beschreibt das v3-Zwischenstadium („Zins sinkt anteilig“), der Code darunter (`:108-109`) lässt ihn stehen, wie der Vertrag. Der Test `v3_schliessen_rechnet_den_zins_bis_zum_orakelindex` würde auch ohne `accrued` in `close` bestehen: `close` verlangt `debt == 0` (`:250`), `accrual` ist dann 0. Das ist harmlos, der Name verspricht aber mehr. `mint_mit_echtem_orakel_und_gleichem_betrag_als_gegenprobe` (`vault_tests.rs:539`) rechnet nur mit `healthy`. Die echte Tx-Gegenprobe `a11_gegenprobe_falsches_orakel_als_tx` ist grün, der Falsch-Orakel-Test scheitert also wirklich an der Herkunft des Orakels.

---

## Bewertung der Mutationslücken (Mutationstest v3, 29.09.2026)

| Zeile | Regel | Einschätzung des Autors | Prüfergebnis |
|---|---|---|---|
| L152 | `newDebt >= 0` | doppelt | **bestätigt.** Jede Minderung ist durch `burned ≤ debt` (L285), `amount ≤ debt` (L296) bzw. `burn ≤ debt` (L320) begrenzt, `mint` addiert `amount > 0`, und die Factory startet bei 0. L152 und L285/L296 sichern sich gegenseitig: Jede Einzelmutante überlebt, fällt aber die andere Regel weg, greift L152. Überzahlung ist getestet (`repay_vollstaendig_und_ueberzahlung`). |
| L154 | `newInterest >= 0` | unerreichbar | **bestätigt.** `accrual ≥ 0`: nur bei d > 0, to > from > 0, und `growth > 0`. Die Engine bricht bei i64-Überlauf ab, statt umzuschlagen. Die Factory startet mit 0. |
| L165 | `kasUsd > 0` | unerreichbar | **Nur im Betrieb richtig, im Vault nicht doppelt gesichert.** Die Mutante ist mit `a11_preis_null_nur_durch_l165_gesperrt` tötbar (A11-V-6). Test übernehmen. |
| L171 | `OpCovOutputCount(ghost) == 0` | doppelt | **bestätigt (Konsens).** Ein Ausgang zählt in `OpCovOutputCount(id)` nur, wenn sein autorisierender Input dieselbe ID trägt (rusty-kaspa `a41a333` `crypto/txscript/src/covenants.rs:126`). Sonst ist er eine Genesis mit nachgerechneter neuer ID (`:138`, `:158`), und die kann nicht `ghostCovId` sein. Aus L170 (keine GHOST-Eingänge) folgt also L171. |
| L181 | `nIn >= 1` | doppelt | **bestätigt.** Bei nIn = 0 bricht `OpCovInputIdx(ghostCovId, 0)` in `:186` ab. |
| L182 | `nIn <= 3` | doppelt | **bestätigt.** Zweitsicherung: KCC20-Leader `require(__cov_in_count <= from)` (`covenant_declarations.rs:728`), dazu der Schleifenwächter. |
| L183 | `nOut >= 1` | doppelt | **bestätigt.** Bei nOut = 0 bricht `OpCovOutputIdx(ghostCovId, 0)` in `:189` ab. |
| L184 | `outStates.length == nOut` | doppelt | **bestätigt.** Zu lang: Überzählige Einträge werden nie gelesen, das ist harmlos. Zu kurz: Indexfehler. Die Grenze nOut ≤ 2 kommt nur von KCC20 (`:745`) und dem Schleifenwächter (A11-V-7). |
| L268 | `OpCovOutputCount(ghost) == 2` (mint) | doppelt | **bestätigt, mit Einschränkung.** nOut = 1 ergibt delta ≤ 0 ≠ amount. nOut = 3 wird heute vom Schleifenwächter im Vault und von KCC20 `:745` abgewiesen. Ohne Wächter nimmt der Vault an (Mutante + gepatchter Compiler: `[Ok, Ok, Err]`), dann bleibt nur KCC20. |
| L285 | `burned <= debt` | doppelt | **bestätigt** (L152). |
| L295 | `amount > 0` (redeem) | doppelt | **bestätigt.** amount ≤ 0 ergibt `paid ≤ 0`, dann greift L304. Test `ruecknahme_mit_negativem_betrag_praegt_nicht`. |
| L296 | `amount <= debt` | doppelt | **bestätigt** (L152). |
| L304 | `paid > 0` | doppelt | **bestätigt.** Sichert mit L295 negative Beträge. Allein weggelassen bliebe nur eine Rücknahme ohne Auszahlung, die den Rücknehmer selbst schädigt. |

Fazit: 12 von 13 Einschätzungen halten. L165 lässt sich testen und sollte getestet werden. Für L184/L268 empfehle ich die ausdrückliche Obergrenze aus A11-V-7, damit die Sicherheit nicht an einem Compiler-Detail hängt.

---

## Was hält (geprüft)

**Frage 1: GHOST ohne Schuld, fremde KAS, Zinskasse, Zins löschen, Sperren**

- **GHOST nur gegen Schuld.** Neue GHOST entstehen nur, wenn ein Minter-Zweig KCC20-Leader ist. Der Minter-Zweig eines Vaults verlangt den Vault in der Tx (`ghost_token.sil:31-32`), und jeder Vault-Eintrag prüft die ganze GHOST-Gruppe: entweder `noGhost` oder `ghostDelta` mit Eingang 0 = eigener Minter. `mint` bucht `debt += delta`. `repay`, `redeem` und `liquidate` verlangen delta < 0 und senken `debt` um genau |delta|. Zwei GHOST-Einträge verschiedener Vaults schließen sich in einer Tx aus, weil es nur einen Eingang 0 gibt. Der Wurzel-Minter der Factory gibt nur Zweige mit Menge 0 aus (`vault_factory.sil:100-124`). Invariante: Σ`debt` ≤ umlaufende GHOST. Nur das Ausbuchen bei Unterdeckung macht die Differenz größer, wie gewollt. **V-03 ist behoben.**
- **Fremde KAS:** `withdraw`, `close`, `deposit`, `mint` und `repay` verlangen die Besitzer-Signatur. Dritte kommen nur über `redeem` (≥ 150 %, 0,995 USD je GHOST, beide Rundungen zugunsten des Vaults, Rest ≥ DUST) und `liquidate` (< 150 %) an KAS. Den Vermögensschaden über Orakel-Nachlauf behandelt A11-V-2.
- **Zins löschen:** Der Zins verschwindet nur beim bezahlten `close` oder beim Ende eines Vaults durch Liquidation. Das Ende tritt nur ein, wenn der Wert ≤ 1,1·burn ≤ 1,1·debt ist oder der Rest < 0,2 KAS bei voller Tilgung. Dann ist die Sicherheit ohnehin aufgebraucht. Alle anderen Pfade schreiben `accrued(index)` fort, und Gegenproben dafür gibt es in `v3_*`.
- **Zins vermeiden (Frage 3):** Nach voller Tilgung friert der Zins ein (`debt = 0` ⇒ kein Zuwachs), das ist richtig. `withdraw` verlangt danach 200 % **auf den Zins** (`:240`). Knapp über 0 abheben geht also nicht, und die Deckelung der Gebühr auf die Sicherheit greift nur beim Zombie (A11-V-4). Selbst-Liquidation beendet den Vault nur bei ≤ 110 % Wert/Schuld. Das kann der Besitzer nicht herbeiführen, weil `mint` und `withdraw` 200 % verlangen. Liegt er dort, ist der Zins ohnehin uneinbringlich. Selbst-Rücknahme senkt die Schuld, nicht den Zins. Der Besitzer kann so zwar bei 150–200 % KAS herausholen, die Quote steigt dabei aber (siehe unten). DUST-Farming lohnt nicht (V-8). Die offene Umgehung ist die Sammel-Schließung (A11-V-1).
- **Treasury-Skriptvergleich:** `byte[36]` = 2 Byte Version + `0x20` + 32 Byte Schlüssel + `0xac`. Verglichen wird exakt, einschließlich Version. `treasury` ist ein Konstruktor-Parameter und damit Teil des Template-Hashs, den die Factory prüft (`vault_factory.sil:96`). Ein Nutzer kann also keinen Vault mit eigener Kasse eröffnen. Falscher Empfänger, zu wenig, keine Zahlung und ein Index auf die Orakel-Fortsetzung werden abgelehnt (`v3_schliessen_zahlt_den_zins_an_die_zinskasse`). Ein Index außerhalb der Ausgänge bricht ab.
- **Überläufe (MAX_*)**, gerechnet bis MAX_COLLATERAL 1e16, MAX_DEBT = MAX_INTEREST = 1e17 und `kasUsd` 1 000 … 9e10:
  - `value` ≤ 9,0000001e18.
  - `need = ⌈owed·bps/1e4⌉` mit owed ≤ 1,1e18 ist unkritisch.
  - `accrual`: ⌊d/1e9⌋·g ≤ 9e17, (d mod 1e9)·g < 9e18.
  - `seize = ⌈claim·1e8/p⌉` wird nur gerechnet, wenn claim < Wert ist, also seize < coll.
  - Beim `redeem` bricht `paid` nur dann über, wenn es ohnehin > coll wäre. Ablehnung ist dort das richtige Ergebnis.
  - `close` bricht bei Zins > ≈ 9,2e10·kasUsd über, also > 920 000 USD bei Mindestpreis. Das ist mit `maxDebt` = 50 GHOST unerreichbar und ohnehin nur eine vorübergehende Sperre bis zu einem höheren Preis.
  - Die Tests `mul_div_grenzfaelle`, `zins_grenzfaelle`, `zufallswerte_gegen_u128`, `schuld_ueber_der_obergrenze_scheitert` und `v3_zins_ueber_der_rechengrenze_scheitert` sind grün.
- **Index kleiner als indexAt:** `accrual` liefert 0 (`to > from`). Der Orakelindex fällt nie: `stableRate ≥ 0` (`risk_oracle.sil:90`), Δ ≥ 600, und der Startindex ist 1e9 (`ghostctl.rs:795`). Ein fallender Index wäre nur mit einem fehlerhaften Genesis-Satz < 0 denkbar. Dann würde `indexAt` zurückgesetzt und Zins später doppelt berechnet, betroffen wären nur die Deploy-Parameter.
- **Sehr alter Vault:** Das Wachstum ist je Abrechnung auf Index ×10 gedeckelt (`MAX_GROWTH`), das begünstigt den Schuldner, und nichts läuft über. Exakt rechnet der Vault bis `indexAt ≤ 9,2e12`. Darüber bricht `growth` ab (`t2·1e6`), und Vaults mit `debt > 0` könnten nur noch einzahlen. Bei 31,5 % p. a. ist das in ≈ 29 Jahren erreicht, bei 20 % in ≈ 46 Jahren. Das ist dokumentiert, ein Befund ist es nicht.
- **Preisgrenzen des Orakels:** 1 000 … 9e10. Die Rechnung ist bis 920 USD/KAS geprüft (`mul_div_grenzfaelle`).
- **Dauerhaft sperren:** Ich habe keinen Weg gefunden, einen Vault ohne das Zutun des Besitzers dauerhaft zu sperren. Ausnahmen sind die jahrzehntelange Indexgrenze, der Zombie aus A11-V-4 (der nur den Besitzer betrifft und auch nur, wenn er ohnehin nichts mehr bekäme) und die UTXO-Rennen aus O-1 und A11-V-3.

**Frage 2: Rücknahme**

- **Die Quote wird nie schlechter.** Sie ist nach der Rücknahme ≥ vorher ⇔ coll·amount ≥ paid·owed. Mit paid ≤ 0,995·amount/p und coll·p ≥ 1,5·owed gilt das immer. Belegt in `a11_ruecknahme_verbessert_die_quote`: 499 976 zulässige Rücknahmen über Preise 0,00001–900 USD, Zins 0–200 % der Schuld und Quoten 150–400 %. Ergebnis: 0 Verschlechterungen, und nach jeder Rücknahme ≥ 150 %. **Eine Mindestquote nach der Rücknahme ist nicht nötig.** Die Frage „entnimmt bei 150–200 % KAS zu 1 USD je GHOST“ beantwortet sich so: Entnommen werden 0,995 USD Sicherheit je 1 USD Schuld, und bei jeder Quote > 99,5 % hebt das die Quote. Das gilt ausdrücklich auch mit Zins, obwohl der Zins stehen bleibt.
- **Rundung:** Beide Abrundungen gehen zugunsten des Vaults (`usd`, `paid`). Bei Kleinstmengen behält der Vault bis zu 50 % statt 0,5 % (amount 2 ⇒ usd 1).
- **Rest ≥ DUST:** `coll − paid ≥ DUST` (L305), getestet mit Gegenprobe.
- **Wechselwirkung mit der Liquidation:** Rücknahme (`healthy` bei liqBps) und Liquidation (`!healthy` bei liqBps) sind für owed > 0 exakt disjunkt. Ein kranker Vault lässt sich also nicht billiger per Rücknahme ausräumen (`v3_ruecknahme_nur_ab_der_liquidationsschwelle`).
- **Wechselwirkung mit dem Zins:** Die Rücknahme rechnet ab, der Zins bleibt stehen (`v3_ruecknahme_verbucht_den_zins`). Nach voller Rücknahme (`debt = 0`) wird der Zins beim `close` fällig. Die Quote auf den Zins liegt dann weiter ≥ 150 %.
- **Offene wirtschaftliche Punkte:** Orakel-Nachlauf (A11-V-2) und Mindestmenge (A11-V-3).

**Frage 4:** Liquidation mit stehenbleibendem Zins erzeugt Zombie-Vaults, siehe A11-V-4. Die GHOST-Seite ist davon nicht betroffen. Teil-Liquidationen verschlechtern die Quote nur bei < 110 % Wert/Schuld, wie in v2. Bei ≤ 110 % nimmt der Liquidator dann alles, und der Vault endet.

**Frage 5: math.rs und Vertrag**

- `growth`, `accrued`, `healthy`, `max_mint` (⌈o·m/1e4⌉ ≤ v ⇔ o ≤ ⌊v·1e4/m⌋), `redeem_paid`, `seize` und die drei Zweige von `liquidation` stimmen mit dem Vertrag überein, einschließlich der Rundungsrichtung.
- Die Teilprodukt-Rechnung von `growth` ist exakt gleich ⌈Δ·1e9/from⌉ für from ≤ 9,2e12. Belegt durch `vault_math_tests` (herausgelöster Vertragscode gegen u128, mit Gegenprobe „+1 wird abgelehnt“).
- `redemption` prüft MAX_* nicht, das ist für die Off-chain-Nutzung unerheblich.
- Einzige Abweichung ist der Kommentar `math.rs:95` (V-10).
- Die Tests prüfen meist mit echten Gegenproben, also derselben Tx mit einem Wert Abstand. Ausnahmen stehen in V-10.

**Frage 6: Zinsindex und Satz**

- Das Komitee kann den Satz bis `maxRate` setzen, für den abgelaufenen Zeitraum gilt der alte Satz.
- Es gibt keinen Pfad zu einem unbegründeten, plötzlichen Liquidationsschub über den Zins (V-9).
- Der Index läuft bei Index·Wachstum > 9,2e18 über. Bei stündlichen Updates liegt das weit jenseits der Vault-Grenze 9,2e12.
- Die Abrundung des Index (≤ 1e-9 je Update) begünstigt die Schuldner. Bei minütlichen Updates und 0,5 % p. a. fehlen so ≈ 5 % des Zinses (⌊9,51⌋ = 9), bei stündlichen ≈ 0,14 %. Das habe ich nur gerechnet, nicht als Test ausgeführt.

**Factory**

- Der Vault-Zustand bei der Eröffnung ist genau (owner, 0, 0, 0) (`vault_factory.sil:96`). `indexAt = 0` bedeutet „noch kein Zins“, und die erste Aktion mit Orakel setzt den Index (`accrual`: `from > 0`).
- Das Template bindet mcr, liq, bonus, maxDebt, treasury und die Orakel-/GHOST-IDs.
- `freshGenesis` schließt Doppel-IDs aus.

---

## Anhang: Testausgaben

Alle Beweistests liegen in der Scratch-Kopie `/private/tmp/claude-501/-Users-peterpan-Desktop-Claude/5377b9f0-55c7-4ff1-9290-e92b15a1ae48/scratchpad/audit11a/`:

- `protocol/tests/audit11a_vault.rs`: Kopie von `vault_tests.rs` plus 10 Tests `a11_*`.
- `audit11a_m165.rs` / `audit11a_m268.rs` / `audit11a_fix1.rs`: dieselbe Datei gegen `contracts/stable_vault_m165.sil`, `_m268.sil` und `_fix1.sil`.
- `…/scratchpad/audit11a_ng/`: gleiche Kopie mit `vendor/silverscript/silverscript-lang/src/compiler/for.rs:150` = `Expr::int(1_000_000)` (Schleifenwächter praktisch aus).

Aufruf: `CARGO_TARGET_DIR=…/scratchpad/target-audit11a cargo test --release --test audit11a_vault -- --nocapture --test-threads=1`

**1. Basis (unveränderter Stand, alle Suiten des Gegenstands):**
```
cargo test --release --no-fail-fast --test vault_tests --test vault_math_tests --test e2e_tests --test factory_tests --lib
lib (math.rs u. a.)   test result: ok. 24 passed; 0 failed
e2e_tests             test result: ok. 3 passed; 0 failed
factory_tests         test result: ok. 21 passed; 0 failed
vault_math_tests      test result: ok. 4 passed; 0 failed
vault_tests           test result: ok. 73 passed; 0 failed
```

**2. `audit11a_vault` (Original-Vertrag), 83 Tests (73 aus vault_tests + 10 neue):**
```
test a11_drei_ghost_ausgaenge_bei_ruecknahme ... Rücknahme + 1 Mio GHOST Beute im 3. Ausgang: [Err("VerifyError"), Ok(()), Err("VerifyError"), Ok(())]
test a11_drei_ghost_ausgaenge_beim_praegen ... Prägen 10 GHOST + 1 Mio GHOST Beute im 3. Ausgang: [Err("VerifyError"), Ok(()), Err("VerifyError")]
test a11_gegenprobe_falsches_orakel_als_tx ... echtes Orakel, 100-facher Preis, 1000 GHOST: [Ok(()), Ok(()), Ok(())]
test a11_preis_null_nur_durch_l165_gesperrt ... Preis 0, Liquidation mit 1 Einheit, ganze Sicherheit: [Err("VerifyError"), Ok(()), Ok(()), Ok(())]
test a11_ruecknahme_gegen_nachlaufendes_orakel ...
  Markt +0.00 % über Orakel: 50 GHOST ⇒ 1243.7500 KAS, Marktwert 49.7500 USD, Gewinn Rücknehmer -0.2500 USD
  Markt +0.50 % über Orakel: 50 GHOST ⇒ 1243.7500 KAS, Marktwert 49.9988 USD, Gewinn Rücknehmer -0.0013 USD
  Markt +1.00 % über Orakel: 50 GHOST ⇒ 1243.7500 KAS, Marktwert 50.2475 USD, Gewinn Rücknehmer +0.2475 USD
  Markt +2.00 % über Orakel: 50 GHOST ⇒ 1243.7500 KAS, Marktwert 50.7450 USD, Gewinn Rücknehmer +0.7450 USD
  Markt +5.00 % über Orakel: 50 GHOST ⇒ 1243.7500 KAS, Marktwert 52.2375 USD, Gewinn Rücknehmer +2.2375 USD
test a11_ruecknahme_mit_zwei_einheiten_bewegt_fremden_vault ... 2 Einheiten GHOST ⇒ 25 sompi an den Rücknehmer
  Preis 0.00001000 USD/KAS: kleinste Rücknahme 2 Einheiten = 0.00000002 GHOST
  Preis 0.04000000 USD/KAS: kleinste Rücknahme 2 Einheiten = 0.00000002 GHOST
  Preis 1.00000000 USD/KAS: kleinste Rücknahme 2 Einheiten = 0.00000002 GHOST
  Preis 900.00000000 USD/KAS: kleinste Rücknahme 905 Einheiten = 0.00000905 GHOST
test a11_ruecknahme_verbessert_die_quote ... 499976 zulässige Rücknahmen geprüft, Quote verschlechtert: 0
test a11_zins_haengt_von_der_abrechnungshaeufigkeit_ab ...
  Index nach 1 Jahr: 1221394587
  50 GHOST, 20 % p. a.: einmal abgerechnet 11.0697 USD, stündlich abgerechnet 10.0000 USD
  maxRate 1e9 ≙ 31.54 % p. a. (Agent-Obergrenze 20 %)
  Preis   0.0400 USD/KAS: Zins unter 0.0080 USD wird beim Schließen erlassen
  Preis   1.0000 USD/KAS: Zins unter 0.2000 USD wird beim Schließen erlassen
  Preis  50.0000 USD/KAS: Zins unter 10.0000 USD wird beim Schließen erlassen
  Preis 900.0000 USD/KAS: Zins unter 180.0000 USD wird beim Schließen erlassen
test a11_zombie_vault_nach_liquidation ... Rest nach voller Liquidation: 2250 KAS = 90 USD, Zins 150 USD
test a11_zwei_vaults_teilen_sich_eine_zinszahlung ...
  Gebühr Vault 1 = 5000000000 sompi, Vault 2 = 5000000025 sompi, geschuldet zusammen 10000000025
  geteilter Kassen-Ausgang (5000000025 sompi für beide): [Ok(()), Ok(()), Ok(())]
  Zinskasse erhält 5000000025 statt 10000000025 sompi (Verlust 5000000000 sompi = 50 KAS)
test result: ok. 83 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 92.85s
```

**3. Mutante L165 (`audit11a_m165`):**
```
test a11_preis_null_nur_durch_l165_gesperrt ... Preis 0, Liquidation mit 1 Einheit, ganze Sicherheit: [Ok(()), Ok(()), Ok(()), Ok(())]
thread 'a11_preis_null_nur_durch_l165_gesperrt' panicked at tests/audit11a_m165.rs:1919:5
test result: FAILED. 2 passed; 1 failed
```

**4. Mutante L268 (`audit11a_m268`), heutiger Compiler:**
```
test a11_drei_ghost_ausgaenge_bei_ruecknahme ... [Err("VerifyError"), Ok(()), Err("VerifyError"), Ok(())]
test a11_drei_ghost_ausgaenge_beim_praegen ... [Err("VerifyError"), Ok(()), Err("VerifyError")]
test result: ok. 2 passed
```

**5. Compiler ohne Schleifenwächter (`audit11a_ng`):**
```
audit11a_m268 (Mutante L268):
test a11_drei_ghost_ausgaenge_bei_ruecknahme ... [Ok(()), Ok(()), Err("VerifyError"), Ok(())]
test a11_drei_ghost_ausgaenge_beim_praegen ... [Ok(()), Ok(()), Err("VerifyError")]
audit11a_vault (Original-Vertrag):
test a11_drei_ghost_ausgaenge_bei_ruecknahme ... [Ok(()), Ok(()), Err("VerifyError"), Ok(())]
test a11_drei_ghost_ausgaenge_beim_praegen ... [Err("VerifyError"), Ok(()), Err("VerifyError")]
```
Input 0 = Vault, Input 2 = KCC20-Leader. Ohne Wächter lehnt allein KCC20 ab (`covenant_declarations.rs:745`).

**6. Fix-Probe A11-V-1 (`audit11a_fix1`, `require(treasuryIdx == this.activeInputIndex)` nach `stable_vault.sil:258`):**
```
mit Fix, ehrlich: [Ok(()), Ok(()), Ok(())]
mit Fix, geteilter Ausgang: [Ok(()), Err("VerifyError"), Ok(())]
mit Fix, einzeln: [Ok(()), Ok(())]
test a11_fix_kassen_ausgang_je_input ... ok
```

**Kern des Belegs A11-V-1** (zum Nachbauen im Harness von `vault_tests.rs`):
```rust
// Input 0: Vault A (close, oracleIdx 2, treasuryIdx 1), Input 1: Vault B (close, oracleIdx 2, treasuryIdx 1),
// Input 2: Orakel read(); Ausgänge: [0] Orakel-Fortsetzung, [1] P2PK(treasury) = max(fA, fB), [2] Rest
let shared = close_two(&e, s1, s2, 1, 1, vec![p2pk_out(&e.treasury, f2), plain_out(2 * COLL - f2)]);
assert!(all_ok(&shared)); // Kasse erhält f2 statt f1 + f2
```
