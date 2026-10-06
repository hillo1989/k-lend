# Audit 9 — Tauschpool KAS/GHOST und Wallet-Funktionen

Stand: 28.09.2026, Prüfstand **Commit `d595624`** („Tauschpool KAS/GHOST, Wallet-Seite, Fix-Review 8 umgesetzt“), Branch `main`.
Gegenstand: `contracts/ghost_pool.sil`, `protocol/src/pool.rs`, `protocol/src/bin/ghostctl.rs` (pool-open/-set/-close, swap, receive, transfer, `load_synced`, `status_json`), `protocol/src/store.rs` (`resolve_pending`), `app/src/pages/Swap.tsx`, `app/src/pages/Wallet.tsx`, `app/src/lib/poolMath.ts`, `app/server/actions.ts`, `app/server/api.ts`. Referenz für Konsens und Opcodes: rusty-kaspa `a41a333` (`crypto/txscript/src/covenants.rs`, `opcodes/mod.rs`), SilverScript v1.0.0 (`silverscript-abi/src/lib.rs`, `covenant_declarations.rs`).

**Methode.** Repository nur lesend. Arbeitskopie `git archive d595624` unter `…/scratchpad/poolaudit/`, `vendor/silverscript` als Symlink, `CARGO_TARGET_DIR` im Scratch-Ordner (2,3 GB, am Ende gelöscht). Vorher `df -h /`: 30 GB frei. Bestehende Suite: `pool_tests` 28 grün, `pool_e2e_tests` 1 grün, lib 4 grün; App: 126 Vitest grün (inkl. 2 eigener). Eigene Tests: `audit9_pool_tests.rs` (Engine, 10), `audit9_sim_tests.rs` (Simulator, 2), `audit9.poolMath.test.ts` (TS-Abgleich mit 400 aus Rust exportierten Fällen). Mutationstest `mutate.sh contracts/ghost_pool.sil pool_tests` in einer zweiten Kopie (Abschnitt 3). Netz: ein lesender GET auf `api.kaspa.org` (Feldnamen der REST-Antwort). `keys/` nicht gelesen, keine Transaktion gesendet.

Schweregrade: **H** Geldverlust, **M** Betriebsausfall/Zustandsverlust/gebundene Mittel, **N** Robustheit/Fehlbedienung, **I** Information. „UNVERIFIED“ = nur hergeleitet.

---

## 0. Kurzfazit

- **Der Vertrag `ghost_pool.sil` hält.** Kein Weg gefunden, den Pool leerzuräumen oder dem Besitzer Liquidität zu entziehen: geschenkte/gefälschte Reserven, Genesis-Tricks (A9-4), zwei pool-eigene Token, Minter, Gebührenrundung (A9-6), Obergrenzen und `mul()` (A9-5), KAS-Wert der Reserve-UTXO, mehrere Pool-Ausgänge, `manage`/`close` — alles wird abgelehnt oder ist harmlos. Tauschrechnung in `pool.rs`, `poolMath.ts` und Vertrag stimmen exakt überein (A9-7 + TS-Abgleich, 400 Fälle). Der Mindestbetrag ist wirksam, weil die Tx die exakte Pool-UTXO verbraucht und ihre Ausgänge festliegen. Mutationslauf 17/22 rot, die 5 Lücken sind durch KCC20 und Opcode-Fehler gedeckt (A9-12).
- **Die Off-chain-Seite hat zwei mittlere Befunde, beide im Auffinden der Reserve (`pool::resync`):**
  - **P-1 (M):** `amount_candidates` findet den neuen Reserve-Betrag **nicht**, sobald eine Pool-Tx zwei GHOST-Zustände hat — also bei **jedem Kauf**, jedem Verkauf mit Wechselgeld und jedem `manage` mit Rückfluss. Die ABI kodiert Struct-Arrays feldweise als einen Push aus 8-Byte-Zahlen, den der Parser nicht zerlegt. Nach dem ersten fremden Kauf ist der Pool für `ghostctl` und die Seite „unbestimmbar“, Tauschen und die Liquidität des Besitzers sind gesperrt, bis jemand den Zustand von Hand repariert. Belegt im Simulator (A9-10).
  - **P-2 (M):** Der Rate-Pfad (`(txid, 1)` mit altem Betrag) und die REST-Bestätigung prüfen die **Covenant-ID nicht**. Jeder Tauschende kann einen Köder-Ausgang ohne Covenant mit dem Reserve-Skript an Index 1 legen (der Vertrag lässt das zu, A9-1/A9-2). `ghostctl` übernimmt den Köder als Reserve, baut und prüft Tx lokal erfolgreich („Probelauf ok“), der Konsens lehnt ab (`WrongGenesisCovenantId`, A9-11). Kein Geldverlust, aber Stillstand inklusive `pool-close` des Besitzers, wiederholbar mit jedem Tausch des Angreifers.
- Weiteres: Leer-Tausch durch jeden für die Netzgebühr verdrängt jede gebaute `ghostctl`-Tx (P-3, N, Designgrenze), verlorenes `pool_unresolved` bei Gesamtfehler des Abgleichs (P-4, N), kleinere Anzeige- und Doku-Punkte (P-5 bis P-10). Journal-Umbau (alle Eingänge) ist korrekt, kein neues falsches Verwerfen. `receive` und `transfer` sind sauber (Covenant-Filter, Adresse = Hash des vollständigen Skripts; Netz und Adresstyp werden geprüft).

---

## 1. Befunde

### P-1 — REST-Kandidatensuche verfehlt den Reserve-Betrag bei zwei GHOST-Zuständen (M)

- **Ort:** `protocol/src/pool.rs` `pushes()` (335–370), `amount_candidates()` (389–398), `find_reserve_rest()` (436–460); Aufruf in `resync()` (492).
- **Hergang:** Der neue Reserve-Betrag steht in einer Pool-Tx nur in den Struct-Arrays `outStates` (Pool-Eintrag) und `newStates` (KCC20-Leader). `silverscript-abi` kodiert ein Struct-Array **feldweise** (`push_struct_array_fields`, lib.rs:967–1004): alle `amount`-Werte eines Arrays werden als *ein* Datenpush aus 8-Byte-LE-Zahlen abgelegt. Bei zwei Zuständen ist das ein 16-Byte-Push, z. B. `10 00286bee00000000 00aea68f02000000` (40 und 110 GHOST). `pushes()` zerlegt Pushes nur als Skript (0x00 = OP_0, 0x28 = „40 Byte“ → Abbruch); `script_num()` nimmt nur Pushes ≤ 8 Byte. Der Betrag fehlt daher in `cands`, `find_reserve_rest` liefert `Err("Reserve in Tx … nicht gefunden (N Kandidaten)")`, `load_synced` setzt `pool_unresolved`, und `swap`/`pool-set`/`pool-close` brechen mit „Pool-Reserve unbekannt“ ab; die Seite zeigt „Pool-Reserve unbekannt“ und sperrt das Tauschen.
- **Betroffen:** jeder Kauf (Zustände `[Reserve, Händler-Token]`), jeder Verkauf mit Wechselgeld-Token, jedes `manage` mit Rückfluss an den Besitzer, sogar die Genesis-Tx (`[Reserve, Rest]`). Nur Tx mit genau einem GHOST-Zustand (Leer-Tausch, Verkauf des ganzen Tokens, `manage` ohne Rest) werden gefunden. Der Rate-Pfad hilft nur, wenn sich der Betrag nicht geändert hat.
- **Beleg:** A9-10 (Simulator, echte Tx aus `pool.rs`): Kauf über 1,23456789 KAS → Reserve 3 995 082 596, 51 Kandidaten, Betrag nicht enthalten (die Liste enthält den *alten* Betrag 4 000 000 000 aus dem Redeem-Skript des Eingangs und die Konstante 1e16); Verkauf mit Rest: nicht gefunden; Verkauf ohne Rest: gefunden; `manage` mit Rückfluss: nicht gefunden. Der bestehende Live-Test (`rest_live_tests`) prüft eine *Präge*-Tx, in der der Betrag zusätzlich als nackter `int`-Parameter des Vault-Eintrags vorkommt — deshalb fiel es dort nicht auf.
- **Wirkung:** Nach dem ersten fremden Kauf ist der Pool für alle `ghostctl`-Nutzer unbestimmbar; der Besitzer kommt per Werkzeug nicht mehr an seine Liquidität (KAS und GHOST bleiben on-chain unversehrt). Erholung nur durch Handreparatur der Zustandsdatei oder zufällig durch eine spätere Ein-Zustand-Tx. Nicht UNVERIFIED: im Simulator mit den echten Bau-Funktionen belegt; die REST-API liefert `signature_script` und `covenant_id` (geprüft an Mainnet-Tx `368cc2d8…`), das ändert am Parser nichts.
- **Empfehlung:** In `amount_candidates` jeden Push mit `len % 8 == 0` zusätzlich als Folge von 8-Byte-LE-Zahlen deuten (Struct-Array-Felder), oder die Sigscripts mit der ABI dekodieren. Test mit einer Zwei-Zustands-Tx aus `pool::swap` ergänzen (A9-10 als Vorlage, dann mit umgekehrter Erwartung). Zusätzlich ein Notausgang: `pool-resync --ghost <Betrag>` (wie `receive`), das den Betrag vom Nutzer nimmt und am Node bestätigt.

### P-2 — Köder-Ausgang vergiftet den Rate-Pfad von `pool::resync` (M)

- **Ort:** `protocol/src/pool.rs` `resync()` 488–497: `guess = (txid, 1)`, Suche nur nach Outpoint unter der Adresse des alten Reserve-Skripts, ohne `covenant_id`; Bestätigung `net.exists(spk, r_op)` (`net.rs:152`) ebenfalls ohne Covenant-ID. `load_synced` (`ghostctl.rs:341–353`) speichert das Ergebnis.
- **Hergang:** Der Vertrag beschränkt nur GHOST-Covenant-Ausgänge und die eigene Fortsetzung; beliebige weitere Ausgänge ohne Covenant-Bindung sind erlaubt (A9-1), ebenso Genesis-Ausgänge fremder Covenants (A9-2). Ein Tauschender legt an **Index 1** einen P2SH-Ausgang mit exakt dem Skript `reserve(poolCov, alterBetrag)` ohne Covenant und die echte Reserve an Index 2. Alle `ghostctl`-Instanzen, deren letzter Stand den alten Betrag kennt (das ist nach jedem regulären Abgleich der Fall), finden beim nächsten `load_synced` die Pool-UTXO an `(txid, 0)`, raten `(txid, 1)`, finden dort den Köder und übernehmen ihn als Reserve. Da der Pool-Outpoint danach unverändert ist, gibt `resync` bei jedem weiteren Aufruf `rec.clone()` zurück — der Stand bleibt vergiftet, bis der Pool bewegt wird; der Angreifer kann den Köder in jeden seiner Tausche legen. Bei einem Leer-Tausch (Betrag unverändert) genügt sogar ein Köder mit identischem Skript.
- **Beleg:** A9-1/A9-2 (Engine: Köder-Tx ist vertragsgültig), A9-11 (Simulator): Köder-Tx angenommen; mit dem vergifteten `PoolRec` baut `pool::swap` die Tx und `txb::build` prüft sie lokal erfolgreich (Probelauf „ok“ auf der Seite); Simulator lehnt ab („UTXO weicht vom erwarteten Zustand ab“), Konsens-Sicht mit echten UTXO-Einträgen: `Covenant-Kontext: WrongGenesisCovenantId(1, …)` (Ausgang „Reserve“ bindet an Eingang 1 ohne Covenant → Genesis-Prüfung). `pool::close` des Besitzers mit dem vergifteten Stand scheitert ebenso; mit dem echten Stand (Index 2) geht es.
- **Wirkung:** Stillstand aller Pool-Funktionen in `ghostctl`/Seite für jeden, der den Köder übernommen hat, inklusive `pool-close` des Besitzers; Anzeige alter Reserven/Kurse (bei Betragswechsel). Kein Geldverlust: der Konsens lehnt jede Tx mit dem Köder ab, `send_to` löscht das Journal (`in_mempool == Ok(false)`). Kosten für den Angreifer: eine Netzgebühr je Tausch. Dieselbe Lücke macht auch eine manipulierte REST-Antwort (Index, `amount`-Wert) zur „Buchung“ in der Zustandsdatei — on-chain bleibt es bei Störung (ARCHITEKTUR.md:198 „kann nur stören, nicht falsch buchen“ ist damit halb richtig: falsch gebucht wird lokal, verloren geht nichts).
- **Empfehlung:** Im Rate-Pfad `e.covenant_id == Some(rec.params.ghost_cov)` verlangen; die Bestätigung des REST-Treffers über `net.utxos` mit Covenant-Filter und mit dem `amount`-Wert des Nodes (nicht der REST) führen; `resync` auch bei unverändertem Pool-Outpoint prüfen, ob die gespeicherte Reserve-UTXO noch existiert und die Covenant trägt, sonst neu bestimmen. Damit ist der Köder wirkungslos (der REST-Pfad filtert bereits nach `covenant_id` der API).

### P-3 — „Anstupsen“: Leer-Tausch durch jeden verdrängt gebaute Transaktionen (N, Design)

- **Ort:** `ghost_pool.sil` `swap` (x' = x, y' = y erfüllt `productGe`), keine eigene GHOST nötig (GHOST-Eingang 0 ist die Reserve selbst).
- **Beleg:** A9-3 (Engine).
- **Wirkung:** Wer die Pool-UTXO für eine Netzgebühr (≈ 0,03 KAS) bewegt, lässt jede gerade gebaute `ghostctl`-Tx scheitern (exakter Outpoint) — auch `pool-close`. Bei Dauerfeuer ist der Pool über `ghostctl` praktisch unbenutzbar; on-chain unschädlich. Inhärent für UTXO-AMMs; AUDIT.md nennt „gleichzeitige Tausche kollidieren“, nicht die gezielte Variante.
- **Empfehlung:** In `ghostctl swap` nach Ablehnung wegen verbrauchter Pool-UTXO automatisch neu abgleichen und (unter Beachtung des Mindestbetrags) erneut bauen, mit Obergrenze; auf der Seite erklären.

### P-4 — `pool_unresolved` geht verloren, Pool-Aktionen hängen am Gesamtabgleich (N)

- **Ort:** `ghostctl.rs` `status_json` 1115–1121 (Fallback `ctx.load()` bei Fehler von `load_synced`; `pool_unresolved` ist `#[serde(skip)]` → `None`), `load_synced` 339 (`store::resync(...)?` vor dem Pool-Abgleich).
- **Wirkung:** Scheitert der Abgleich insgesamt (z. B. „Orakel-UTXO nicht auffindbar“), liefert die Statusausgabe den Pool mit alten Reserven und `unresolved: null`; die Tausch-Seite wertet nur `pool.unresolved` aus, nicht `oracle.fresh`, und zeigt Kurs/Beträge als aktuell. Ein Tausch scheitert dann erst in `ghostctl`. Umgekehrt blockiert ein Orakel-/Vault-Problem alle Pool-Aktionen, obwohl der Pool davon unabhängig ist.
- **Empfehlung:** `unresolved` im Fehlerfall mit dem Sync-Fehler füllen; Swap-Seite bei `!oracle.fresh` sperren oder warnen; Pool-Abgleich vom Rest entkoppeln.

### P-5 — Anzeige und Texte der Tausch-Seite (I)

- „Du gibst X KAS“: tatsächlich gehen X KAS + 1 KAS (`TOKEN_VALUE` der neuen Token-UTXO, kommt beim späteren Ausgeben zurück) + Netzgebühr; der Probelauf nennt nur die Gebühr (`Swap.tsx:194`).
- Nach dem Tausch wird der erhaltene Betrag nicht gezeigt; `ghostctl` liefert ihn als `out` im JSON (`ghostctl.rs:1049`), die Seite listet nur TXIDs.
- Guthabenprüfung `Number(amountStr.replace(",", "."))` (`Swap.tsx:66`) versagt bei Tausenderpunkt („1.000“ → NaN → keine Warnung); harmlos, `ghostctl` lehnt ab.
- „Einen schlechteren Kurs als den Mindestbetrag bekommst du nie“ ist richtig (exakte UTXO, feste Ausgänge, Prüfung beim Bau gegen frischen Abgleich). „Du bekommst etwa“ stammt aus dem bis zu 20 s alten Status-Cache; der echte Betrag ist der von `ghostctl` gerechnete (≥ Mindestbetrag).
- `ghostctl status` (Textmodus) zeigt den Pool und `pool_unresolved` gar nicht (nur `--json`).

### P-6 — `poolMath.ts` ohne Obergrenzen (I)

`swapOk` prüft `MIN_KAS` und `y' > 0`, nicht `MAX_KAS`/`MAX_GHOST` (1e16). Unterhalb von 1e8 KAS Reserve ohne Wirkung; in 400 Zufallsfällen bis 1e16 sind `ghostOut`/`kasOut` identisch mit `pool.rs`, das die Grenzen (als 0-Ergebnis) enthält. Vollständigkeitshalber ergänzen.

### P-7 — Transfer-Adressen: nur Schnorr-P2PK, Texte sagen „Kaspa-Adresse“ (I)

`ghostctl.rs:929–938`: Präfix gegen `ctx.net.prefix` (mainnet `kaspa`, testnet-10 `kaspatest`), `Version::PubKey` und 32-Byte-Nutzlast verlangt; ECDSA- (`PubKeyECDSA`, 33 Byte) und P2SH-Adressen (`ScriptHash`) werden mit klarer Meldung abgelehnt — korrekt, weil KCC20-Besitz ein x-only-Schlüssel ist. Die Seite (`actions.ts:100–108`) prüft Muster und Präfix, lässt ECDSA/P2SH aber bis zu `ghostctl` durch; `commands.ts:80` und `Wallet.tsx:110` sagen „dieselbe Adresse, an die man auch KAS schickt“. Hinweis „nur `kaspa:q…`-Adressen mit Schnorr-Schlüssel“ ergänzen. `send` (KAS) nutzt `pay_to_address_script` und unterstützt alle drei Typen; Präfix wird geprüft.

### P-8 — Geschenkte pool-eigene Token bleiben mit `ghostctl` verwaist (I)

Token, die jemand an die Pool-Covenant-ID schickt (aus fremder Tx), sind in `swap`/`manage` unzulässig (`require(!mine)`), können aber vom Besitzer in `close` mitgenommen werden, weil `close` keine GHOST prüft (A9-8). `pool::close` in `ghostctl` baut das nicht; solche Token bleiben liegen. Optional: `pool-close` sammelt bekannte pool-eigene Token ein.

### P-9 — Journal-Umbau: korrekt, Restfälle unverändert (I)

`resolve_pending` (`store.rs:113–186`): „irgendein Eingang unverbraucht ⇒ nicht angenommen“ ist logisch zwingend (eine angenommene Tx verbraucht alle Eingänge); kein Fall gefunden, in dem eine noch laufende Tx verworfen wird — bei `mempool == Ok(true)` wird gewartet, bei `Err` und unverbrauchten Eingängen ebenfalls. Für Pool-Tx: Pool-Eingang durch Dritte verbraucht + eigene Gebühren-Eingänge unverbraucht → korrekt verworfen; angenommen → Ausgang 0 (Pool-Fortsetzung) sichtbar. Unverändert bestehen: das Ausbreitungsfenster nach Prozessabbruch direkt nach `submit` (Audit 8) und die Abhängigkeit von `in_mempool` vom Fehlertext (`net.rs:164`): bei abweichendem Wortlaut und unverbrauchten Eingängen endet Schritt 3 dauerhaft in „Unklar … später erneut“ — NEU-5 entkoppelt nur Schritt 4.

### P-10 — Kleinigkeiten (I)

- `find_reserve_rest` blockiert mit `ureq` (bis 15 s) den Tokio-Worker innerhalb `async fn resync` (`pool.rs:438`); `spawn_blocking` verwenden.
- `amount()` in `ghostctl` rechnet über `f64` (`to_units`); bis 1e10 KAS/GHOST kann `x*1e8` oberhalb 2^53 um Einheiten abweichen — praktisch ohne Bedeutung, vorbestehend.
- `MAINNET.md:136` / `ARCHITEKTUR.md:198`: „bestätigt ihn über den Node“ meint nur die Existenz einer UTXO mit dem Skript (siehe P-2).

**Geprüft und ohne Befund (Vertrag).**
- *Reserve-Herkunft:* GHOST-Eingang 0 muss pool-eigen (`identifierType == COV`, Besitzer = `OpInputCovenantId`) und aus derselben Tx wie die Pool-UTXO sein; alle weiteren GHOST-Ein- und -Ausgänge dürfen nicht pool-eigen sein; kein Minter beteiligt. Geschenke, fremde Tx, Händler-Token vor der Reserve (A9-9) scheitern.
- *Genesis-Trick (A9-4):* Ein vom Pool-Eingang autorisierter Ausgang mit neuer Covenant-ID steht laut `CovenantsContext::from_tx` (covenants.rs:129–147) **nicht** im Auth-Kontext; `OpAuthOutputCount == 1` verlangt die echte Fortsetzung. Ein Genesis-„Zweitpool“ neben der Fortsetzung ist erlaubt, hat aber eine andere ID — `resync` filtert nach `covenant_id`. Die Reserve muss die GHOST-Covenant tragen, die nur über KCC20 fortgesetzt wird.
- *Zahlen:* Reserven ≤ 1e16 < 2^54; `mul()` exakt bis 2^60 (bestehende Tests) und der ganze `swap`-Pfad an der Obergrenze (A9-5: Maximum geht, +1 Einheit und `MAX_KAS + 1` scheitern). `mulDivUp` ohne Überlauf (a ≤ 1e16, b = 30). Gebühr rundet auf: 1 sompi/1 Einheit Zufluss sind ganz Gebühr (A9-6); die Rundung geht stets zugunsten des Pools; kein Splitting-Vorteil, da jede Tx einzeln `x·y` halten muss.
- *Rechnung vs. Vertrag:* Referenz (u128) = `pool.rs` = `poolMath.ts` in 400 Zufallsfällen (A9-7 + TS-Test); Referenz = Vertrag in den bestehenden Zufallstests (Maximum geht, +1 scheitert). `ghost_out`/`kas_out` sind monoton, die Binärsuche korrekt; `swap_ok` enthält dieselben Grenzen wie der Vertrag.
- *KAS-Wert der Reserve-UTXO* gleich Eingang; *Pool ≥ 10 KAS*, ≤ 1e8 KAS; *genau eine Fortsetzung* (Genesis zählt nicht); *`close`* ohne Fortsetzung, nur mit Besitzersignatur (SIGHASH_ALL über die ganze Tx → nicht durch Dritte formbar); *`manage`* prüft Reserve-Regeln, aber bewusst nicht `productGe`.
- *Leader/Delegate:* Unabhängig davon, welcher GHOST-Eingang KCC20-Leader ist, prüft der Pool alle GHOST-Ein-/Ausgänge selbst; Mengenerhaltung kommt vom Nicht-Minter-Leader (`totalIn == totalOut`, `amount >= 0`).
- *Slippage:* wirksam (siehe P-5); Standard in `ghostctl` (1 % unter dem eigenen Angebot) ist rechnerisch immer erfüllt, was wegen der exakten UTXO unschädlich ist.
- *Tx-ID:* Signaturskripte gehen nicht in die ID ein (`hashing/tx.rs`, `EXCLUDE_SIGNATURE_SCRIPT`); Journal-TXID ist gegen Sigscript-Formbarkeit stabil.

**Geprüft und ohne Befund (Off-chain).**
- *`receive`:* Filter `covenant_id == ghost_cov`; die Adresse ist der Hash des vollständigen Token-Skripts (Besitzer, Typ, Betrag, Minter-Flag) — ein „nur gleich aussehendes“ Skript oder eine fremde Covenant fällt heraus; Doppelbuchung über Outpoint ausgeschlossen. `/api/receive` mit Origin/Host/Header-Schutz und `busy`-Sperre.
- *`pool_open`/`pool_tx`/`close` (Bau):* Genesis-ID aus erstem Funding-Eingang; Autorisierungen (Pool ← Eingang 0, GHOST ← Leader) korrekt; `sig_at`-Positionen (manage 1, close 0) passen zur ABI; `apply` führt Token und Pool nach. Simulator-Lebenszyklus grün.
- *`manage`/`pool-set`:* Bilanz `y + have = new + mine`, Fehlbestand wird abgelehnt; nur Besitzer.
- *Seite/API:* `swap` verlangt genau eine Seite und einen Mindestbetrag > 0; Mainnet nur mit `confirmMainnet`; `checkAmount` normalisiert; Argumente ohne Shell.

---

## 2. Schwerpunkte des Auftrags — Kurzantworten

| Frage | Antwort |
|---|---|
| Pool leerräumen / Liquidität stehlen? | Nein. Alle genannten Wege scheitern am Vertrag (Abschnitt 1, „ohne Befund“). |
| Rechnung `pool.rs`/`poolMath.ts` = Vertrag? | Ja, exakt (A9-7, TS-Abgleich, bestehende Zufallstests). Einziger Unterschied: TS ohne 1e16-Grenzen (P-6). |
| Mindestbetrag wirksam? | Ja: exakte UTXO, feste Ausgänge, Prüfung beim Bau nach frischem Abgleich. Ein Tausch kann nicht schlechter ausfallen als `ghostctl`s `out`. Der erhaltene Betrag wird der Seite nicht gezeigt (P-5). |
| REST-Auffinden: Fehlbuchung oder Störung? | On-chain nur Störung (Konsens lehnt ab). Lokal wird der REST-Treffer (Index, KAS-Wert) ohne Covenant-Prüfung gebucht (P-2). Mehrere Kandidaten passen nie (nur die echte Reserve ist pool-eigen und GHOST-gebunden) — aber der richtige Kandidat **fehlt** bei zwei Zuständen (P-1). |
| `receive`: falsche Token unterschieben? | Nein (Covenant-Filter, Skript-Hash). |
| `transfer`: Netz und Adresstyp? | Ja, in `ghostctl` und `actions.ts`; ECDSA/P2SH abgelehnt (P-7 nur Text). |
| Journal: falsches Verwerfen? | Nein (P-9). |
| Seite: irreführende Kurse/Beträge? | Kurs/Marktwert korrekt (x/y, × Orakelpreis). Kleinere Punkte P-4/P-5. |

---

## 3. Mutationslauf

`protocol/mutation/mutate.sh contracts/ghost_pool.sil pool_tests` (zweite Kopie, eigener `CARGO_TARGET_DIR`; Vertrag danach unverändert). Ergebnis **17 rot, 5 LUECKE, 0 BUILD** — wie im Commit angegeben.

| Zeile | require | Ergebnis | Deckung der Lücke |
|---|---|---|---|
| L95 | `nIn >= 1` | LUECKE | `OpCovInputIdx(ghostCovId, 0)` schlägt bei 0 Eingängen fehl (`InvalidCovInIndex`). |
| L96 | `nIn <= MAX_GHOST_INS` | LUECKE | KCC20-Leader: `__cov_in_count <= from` (from = 3, `covenant_declarations.rs:729`). A9-12: 4 Eingänge → Pool **und** Leader `VerifyError`; 3 Eingänge gehen. |
| L97 | `nOut >= 1` | LUECKE | `OpCovOutputIdx(ghostCovId, 0)` schlägt fehl; zudem `newReserve > 0` (L132) bleibt 0. |
| L98 | `nOut <= MAX_GHOST_OUTS` | LUECKE | KCC20-Leader: `__cov_out_count <= to` (to = 2). |
| L99 | `outStates.length == nOut` | LUECKE | Kürzer → Index außerhalb (Fehler); länger → überzählige Zustände ohne Wirkung, alle echten Ausgänge werden geprüft. KCC20 prüft `out_count == newStates.length`. |
| L102–L178 (17) | Wert, Minter, Herkunft, Besitz, Grenzen, Fortsetzung, `productGe`, Signaturen | rot | siehe `9-mutation.log` |

Empfehlung: die fünf Grenzfälle mit eigenen Tests decken (A9-12 als Vorlage für L96; analog 3 Ausgänge für L98, leere Zustandsliste für L99), damit der Schutz nicht allein an KCC20 hängt.

---

## 4. Empfehlungen (Reihenfolge)

1. **P-1:** `amount_candidates` um das 8-Byte-Raster (Struct-Array-Felder) erweitern oder ABI-Dekodierung nutzen; Test mit `pool::swap`-Tx (zwei Zustände) als Regression; Notbefehl `pool-resync --ghost`.
2. **P-2:** Covenant-ID in Rate-Pfad und Bestätigung prüfen, `amount`-Wert vom Node nehmen, gespeicherte Reserve bei jedem Abgleich verifizieren. Engine-Test A9-1 als Regression (Köder ist gültig — die Abwehr muss off-chain sitzen).
3. **P-3/P-4:** Wiederholung nach verbrauchter Pool-UTXO; `unresolved`/`fresh` auf der Tausch-Seite auswerten; Pool-Abgleich vom Orakel-Abgleich entkoppeln.
4. **P-5–P-8, P-10:** Texte (1 KAS Token-UTXO, nur Schnorr-Adressen), `out` anzeigen, `status`-Textmodus, `poolMath` Grenzen, Geschenke in `pool-close`, `spawn_blocking`.
5. Mutationslücken L95–L99 mit Tests decken.

---

## Anhang — eigene Tests

Dateien (Arbeitskopie, nicht ins Repository geschrieben):
- `…/scratchpad/poolaudit/protocol/tests/audit9_pool_tests.rs` — Engine (Hilfsfunktionen aus `pool_tests.rs`), 10 Tests.
- `…/scratchpad/poolaudit/protocol/tests/audit9_sim_tests.rs` — Simulator, 2 Tests.
- `…/scratchpad/poolaudit/app/src/lib/audit9.poolMath.test.ts` — TS-Abgleich, liest `…/scratchpad/poolaudit/a9_cases.json` (400 Fälle, von A9-7 exportiert).
- Protokolle: `…/scratchpad/poolaudit/a9_testlog.txt`, `…/scratchpad/poolaudit/9-mutation.log`.

Ausführen (Kopie mit `vendor/silverscript`-Symlink): `cd protocol && AUDIT9_EXPORT=../a9_cases.json cargo test --test audit9_pool_tests --test audit9_sim_tests -- --nocapture`; `cd app && AUDIT9_CASES=../a9_cases.json npx vitest run`.

| Test | Inhalt | Ergebnis |
|---|---|---|
| A9-1 `koeder_mit_reserve_skript_ohne_covenant_wird_akzeptiert` | Tausch mit Köder-P2SH (Reserve-Skript, alter bzw. neuer Betrag, keine Covenant) an Index 1, echte Reserve an Index 2 | alle Eingänge ok (Vertrag lässt es zu) |
| A9-2 `koeder_als_fremde_genesis_wird_akzeptiert` | Köder als Genesis-Ausgang eines fremden Covenants (autorisiert vom Händler-Eingang) | alle ok |
| A9-3 `leerer_tausch_bewegt_den_pool_fuer_jeden` | x' = x, y' = y, keine eigenen GHOST | alle ok |
| A9-4 `genesis_ersetzt_keine_fortsetzung` | Pool-Skript als Genesis-Ausgang (neue ID) vom Pool-Eingang: allein → Pool `Err`, Leader ok; neben echter Fortsetzung → alle ok | wie erwartet |
| A9-5 `tausch_an_der_obergrenze` | x = 1e16 − 100 KAS, y = 1e16: Maximum ok, +1 Einheit Err, `MAX_KAS + 1` Err | wie erwartet |
| A9-6 `kleinstbetraege_bringen_nichts` | `ghost_out(X,Y,1) = 0`, `(…,20) = 0`, `(…,1000) = 45`; Engine: 20 sompi gegen 1 Einheit → Err | wie erwartet |
| A9-7 `rust_rechnung_gleich_referenz_und_export` | 400 Zufallsfälle bis 1e16: `pool::ghost_out`/`kas_out` = u128-Referenz, Maximum ok / +1 nicht; Export | ok |
| A9-8 `geschenke_holt_der_besitzer_beim_aufloesen` | `close` mit geschenktem pool-eigenem Token aus fremder Tx als zusätzlichem Eingang | alle ok |
| A9-9 `haendler_token_vor_der_reserve_scheitert` | Händler-Token als GHOST-Eingang 0 | Pool Err |
| A9-12 `vier_ghost_eingaenge` | Reserve + 3 Händler-Token | Pool `VerifyError`, Leader `VerifyError`; 3 Eingänge ok |
| A9-10 `rest_kandidaten_verfehlen_die_reserve_bei_zwei_zustaenden` (Sim) | Genesis, Kauf, Teilverkauf, `manage −`: Betrag fehlt in den Kandidaten; Verkauf ohne Rest: gefunden | Befund P-1 |
| A9-11 `vergiftete_reserve_baut_lokal_und_scheitert_am_konsens` (Sim) | Köder-Tx angenommen; vergifteter `PoolRec`: `pool::swap` + `txb::build` ok, Simulator lehnt ab, `check_scripts` mit echten Einträgen: `WrongGenesisCovenantId(1, …)`; `close` vergiftet Err, echt ok | Befund P-2 |
| TS `audit9: poolMath = pool.rs` | 400 Fälle identisch; `swapOk` ohne 1e16-Grenze dokumentiert | ok (P-6) |

Auszug `a9_testlog.txt`:
```
Kauf: Reserve neu = 3995082596, 51 Kandidaten: [1, 2, …, 1880, 9441, 10000, 1000000000, 1054508240, 1143226963, 1847395877, 2147483648, 4000000000, 10000000000000000]
Simulator: Input 1: UTXO weicht vom erwarteten Zustand ab
Konsens: Covenant-Kontext: WrongGenesisCovenantId(1, 680e8a15…)
vier GHOST-Eingänge: Pool Err("VerifyError"), Leader Err("VerifyError")
test result: ok. 10 passed (audit9_pool_tests) · ok. 2 passed (audit9_sim_tests) · ok. 4 passed (lib)
```

Bestehende Suiten auf `d595624`: `pool_tests` 28 ok (24 s), `pool_e2e_tests` 1 ok, lib 4 ok; App 126 Vitest ok.
