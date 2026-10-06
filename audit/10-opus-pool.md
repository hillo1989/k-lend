# Audit 10 — Offener Tauschpool KAS/GHOST mit Anteils-Token (Opus)

Stand: 29.09.2026, Arbeitskopie `…/scratchpad/audit10/kaspa-lending` (das Original wurde weder gelesen noch geändert).
Gegenstand: `contracts/ghost_pool.sil` (init/swap/add/remove), `contracts/ghost_token.sil` (KCC20 v2), `protocol/src/pool.rs`, `protocol/src/bin/ghostctl.rs` (pool-open, pool-add, pool-remove, swap, `load_synced`, `status_json`), `protocol/src/ops.rs` (Pool-Felder), `protocol/src/store.rs`, `app/src/lib/poolMath.ts`, `app/src/lib/precheck.ts`, `app/server/actions.ts`. Referenz für Konsens: rusty-kaspa `a41a333` (`consensus/core/src/hashing/covenant_id.rs`), SilverScript aus `vendor/silverscript` (`covenant_declarations.rs`, `compile/statement.rs`, `silverscript-abi`).

**Methode.** Keine Transaktion gesendet, kein Netzzugriff auf Nodes/REST, keine Schlüsseldateien. Build mit eigenem `CARGO_TARGET_DIR=<kopie>/target`, `--release`. Bestehende Suiten: `pool_tests` 32/32 grün, `pool_e2e_tests` 1/1 grün. Eigene Tests: `protocol/tests/audit10_pool.rs` (Skript-Engine, 10 Tests inkl. Mutanten-Gegenproben), `protocol/tests/audit10_pool_sim.rs` (Simulator mit den echten Bau-Funktionen aus `pool.rs`, 4 Tests), `audit/a10_poolmath_check.mjs` (Abgleich `poolMath.ts` ↔ `pool.rs`, 500 Fälle). Mutationslauf über alle `require` des Vertrags (Abschnitt 5). Schweregrade: **kritisch** (Diebstahl ohne Mitwirkung des Opfers), **hoch** (Geldverlust mit geringer Hürde), **mittel** (Geldverlust unter Bedingungen, Zustandsverlust, Sperre), **niedrig** (Grenzfälle, Bedienung), **Info**. Was nur hergeleitet und nicht per Test belegt ist, steht als „hergeleitet“ dabei.

---

## 0. Kurzfazit

- **Der Vertrag hält.** Kein Weg gefunden, Reserven oder Anteile zu stehlen, Anteile aus dem Nichts zu erzeugen, einen zweiten Minter oder eine zweite Reserve einzuschleusen oder den Minter-Status zu kippen. Einlegen und Abziehen runden exakt zugunsten der übrigen Einleger (Zufallstest gegen die Engine, auch bei S nahe 2^60). Die Grenzen `MAX_KAS`, `MAX_GHOST`, `MAX_SHARES` und `mul()` halten auch alle zugleich am Maximum. Der Spende-/Inflationsangriff kostet den Angreifer fast die ganze Spende, das Opfer verliert höchstens einen Anteil (9 sompi im Beispiel).
- **Die Schwachstellen liegen off-chain, zwei davon mittel:**
  - **A10-P-1 (mittel):** `pool-add` (und Schritt 3 von `pool-open`) legt beide Beträge **voll** ein und verlangt nur **1 Anteil** — ohne Kursband, ohne Mindestanteile. Liegt der Poolkurs beim Bau anders als beim Eintippen, ist der Überschuss geschenkt. Im Simulator verliert ein Einleger so **24,8 %** (495 KAS von 1 998 KAS Wert), ein Angreifer, der den 1-KAS-Pool vorher mit 9 KAS verschoben und selbst eingelegt hat, **gewinnt 482 KAS**.
  - **A10-P-3 (mittel):** Liefert der Node für die Pool-Adresse keine UTXO, schreibt `ghostctl` „Der Pool wurde aufgelöst“ und **löscht den Pool dauerhaft aus der Zustandsdatei**. Der offene Pool kann aber gar nicht mehr aufgelöst werden (kein `close` mehr). Ein falscher oder unvollständiger Node genügt; danach sind die eigenen Anteile per `ghostctl` nicht mehr abziehbar, und `pool-open` legt einen zweiten Pool an.
- Weitere Punkte: Die Mutationsprüfung zeigt eine tragende Vertragsregel ohne Test (L214, A10-P-10). Anteile über 1e16 werden beim Nachladen nicht gefunden (A10-P-2, niedrig), der letzte Einleger kommt nicht ganz heraus, und ein Mini-Abzug verbrennt Anteile für nichts (A10-P-4/-5), Token im Besitz der Anteils-Covenant-ID sind für jeden frei (A10-P-6), die Echtheit eines Pools hängt an seiner Genesis-Kette (A10-P-7), Kleinigkeiten (A10-P-8/-9).

---

## 1. Befundtabelle

| ID | Schwere | Befund | Beleg / Test | Empfehlung (Kurzform, Code in Abschnitt 2) |
|---|---|---|---|---|
| **A10-P-1** | **mittel** | `pool::add` nimmt `kas` und `ghost` vollständig, obwohl nur `min(S·Δx/x, S·Δy/y)` Anteile entstehen; `ghostctl pool-add` und `pool-open` Schritt 3 übergeben `min_shares = 1`, die Seite übergibt gar nichts. Verschiebt sich der Kurs zwischen Anzeige und Bau (Fremdtausch, gezieltes Verschieben des nach `init` nur 1 KAS großen Pools), wird der Überschuss verschenkt – an die Inhaber, also auch an einen Angreifer. | `audit10_pool_sim.rs::a10_einlage_nach_kursverschiebung_wird_verschenkt`: Opfer −494,74 KAS (24,8 %), Angreifer +481,69 KAS, Poolkurs danach 49,5 statt 25 KAS/GHOST; `a10_passende_einlage_zum_verschobenen_kurs`: passende Einlage zum verschobenen Kurs −80,1 % | Nur die zu `m` passenden Beträge nehmen (`⌈m·x/S⌉`, `⌈m·y/S⌉`) **und** ein Kursband je Seite wie `amountAMin/amountBMin` (Standard 0,5 %); `--min-shares`; Schritt 3 von `pool-open` nach Abgleich mit Band; Seite: Poolkurs gegen Orakel warnen |
| A10-P-2 | niedrig | Vertrag erlaubt S bis 2^60−1, `pool.rs::script_num` verwirft aber Kandidaten > 1e16 (`MAX_RESERVE`). Ab S > 1e16 findet `find_amounts_rest` den Minter nicht: Pool für `ghostctl`/Seite „unbestimmt“, Tauschen/Einlegen/Abziehen gesperrt. Erreichbar nur bei kleinem Startverhältnis `y0/x0` (S² ≈ x·y·x0/y0), z. B. 1 GHOST-Einheit auf 1 KAS: dann ab ≈ 2 000 GHOST + 50 000 KAS im Pool. | `audit10_pool_sim.rs::a10_anteile_ueber_1e16_findet_resync_nicht` (gültige, lokal skriptgeprüfte Tx bei S = 2e16; Betrag fehlt in den Kandidaten; Gegenprobe 1e16 gefunden); `audit10_pool.rs::a10_grenzen_max_shares_und_max_reserven` (S = 2^60−1 ist gültig) | Obergrenze in `script_num` auf `MAX_SHARES` anheben (Treffer werden ohnehin am Skript und Node bestätigt) |
| **A10-P-3** | **mittel** | `pool::resync` gibt bei leerer Node-Antwort `Ok(None)` zurück, `load_synced` setzt `d.pool = None` und **speichert**. Seit dem offenen Pool gibt es kein Auflösen mehr, der Fall ist also immer ein Node-Fehler. Folge: Pool-Datensatz (Parameter, `lp_cov`) weg, `pool-remove` meldet „keinen Pool“, `pool-open` legt einen zweiten Pool an. Läuft auch bei jedem Status-Abruf der Seite. | Codestellen `pool.rs:610`, `ghostctl.rs:371–382`; **hergeleitet**, nicht per Test (kein Node-Zugriff erlaubt, `Net` ist nicht mockbar) | Leere Antwort als Fehler behandeln (`pool_unresolved`), `d.pool` nie automatisch löschen |
| A10-P-4 | niedrig | Ist ein Anteil weniger als 1 sompi wert (KAS ist seit `init` gestiegen oder GHOST wurde in den Pool verkauft), lehnt `pool::remove` 100 % ab („mindestens 1 KAS bleibt“), statt die KAS-Auszahlung auf `x − 1 KAS` zu kappen. Der Vertrag erlaubt die gekappte Auszahlung. | `audit10_pool_sim.rs::a10_abziehen_nach_kursverschiebung` (x/S = 0,286; 100 % abgelehnt); `audit10_pool.rs::a10_letzter_einleger_und_1_kas_rest` (gekappte Auszahlung gültig, ohne Kappung säßen 0,714 KAS fest) | In `pool::remove` `dx` auf `x − POOL_MIN_KAS` kappen und den Rest anzeigen |
| A10-P-5 | niedrig | Mini-Abzug: `pool::remove` baut auch bei Auszahlung (0, 0); der Anteil wird verbrannt, es kommt nichts zurück. `ghostctl` setzt `m ≥ 1` (`clamp(1, have)`), `min_kas`/`min_ghost` sind fest 0. | `a10_abziehen_nach_kursverschiebung`: 1 Anteil → (0, 0), angenommen, Anteile −1 | `if dx == 0 && dy == 0 { Err(..) }`; Vorschau-Werte als Mindestauszahlung übergeben |
| A10-P-6 | Info | KCC20 prüft bei Covenant-Besitz nur, dass ein Eingang der Besitzer-ID in der Tx ist. Jede Pool-Tx enthält Eingänge mit `lpCovId` und `ghostCovId`. Token, die jemand an eine dieser IDs „schickt“, kann jeder in einer Pool-Tx ausgeben (hier: Anteile verbrennen und die Auszahlung nehmen). `ghostctl transfer` kann das nicht erzeugen (nur Schnorr-Adressen). | `audit10_pool.rs::a10_token_im_besitz_der_anteils_id_ist_fuer_jeden_frei` | Dokumentieren; Integratoren warnen; ggf. im Pool `require(!(o.identifierType == COV && (o.ownerIdentifier == lpCovId \|\| o.ownerIdentifier == ghostCovId)))` für Ausgänge |
| A10-P-7 | Info | Der Vertrag setzt voraus, dass die Inhaber zusammen S − S0 Anteile halten. Das folgt nur aus der Herkunft: Pool-Genesis mit `initialized = false` und `init` über den Vertrag. Eine Genesis direkt mit `initialized = true` und `lpCovId` = einem **vorher** angelegten, eigenen KCC20-Token ist bildbar (kein Hash-Zyklus) und gibt dem Ersteller Anteile ohne Deckung. `ghostctl` nutzt nur selbst angelegte Pools, daher derzeit ohne Wirkung. | Teilbeleg `audit10_pool.rs::a10_vertrag_vertraut_der_herkunft_der_anteile` (Inhaber mit mehr als S zieht 99,9 % ab); Bildbarkeit **hergeleitet** aus `covenant_id.rs` | Vor dem Nutzen eines fremden Pools die Genesis- und init-Tx prüfen (Abschnitt 2); Doku |
| A10-P-8 | Info | Anteils-UTXOs werden in `store::resync` nicht nachgeführt (nur `d.tokens`); mehr als zwei Anteils-UTXOs lassen sich nicht zusammenführen (kein `consolidate`); die Vorschau der Seite rechnet mit allen Anteilen, `ghostctl` zieht nur aus den zwei größten ab. | Code `store.rs:242`, `ghostctl.rs:1091–1099`, `precheck.ts:223–241` | Anteils-UTXOs wie GHOST nachführen; in `pool-remove` vorher zusammenführen; Vorschau auf die zwei größten beschränken oder Hinweis |
| A10-P-9 | Info | Nachlauf von Audit 9 P-3: Die Wiederholung nach „Pool inzwischen bewegt“ gibt es nur für `swap`, nicht für `pool-add`/`pool-remove`/`pool-open` Schritt 3. Ein gescheiterter Schritt 3 lässt sich nicht fortsetzen (`pool-open` meldet dann „schon einen Pool“). Kommentar `Swap.tsx:290` („Liquidität des Besitzers“) ist veraltet. | Code `ghostctl.rs:1018–1107` | Wiederholung wie bei `swap` (mit Band aus A10-P-1); `pool-open` fortsetzbar machen |
| A10-P-10 | niedrig | Mutationslauf: 32 von 60 `require` ohne roten Test. Fast alle sind durch KCC20/Konsens gedeckt. **L214** (`OpCovInputCount(id) == 0` in `freshGenesis`) ist aber tragend: Ohne die Regel übernimmt `init` einen bestehenden Token des Gründers als Anteils-Token. | `a10_gegenprobe_l214_init_mit_bestehendem_token` (Mutante: alle Eingänge ok; Original: Pool lehnt ab); Abschnitt 5 | Gegenprobe als Regressionstest übernehmen; Tests für L249/L250 und `MAX_GHOST` |

---

## 2. Befunde im Detail

### A10-P-1 — Einlegen ohne Kursband verschenkt den Überschuss (mittel)

**Ort.** `protocol/src/pool.rs:364–385` (`add`: `pool_tx(…, x + kas, y + ghost, …, sh + m, m, …)` in Zeile 383), `ghostctl.rs:1059` (`pool-open` Schritt 3) und `ghostctl.rs:1075` (`pool-add`), beide mit `min_shares = 1`; `app/server/actions.ts:200–203` übergibt nur `--kas`/`--ghost`. Die Hilfe der Seite (`commands.ts:122–129`) sagt korrekt „was darüber hinausgeht, ist geschenkt“, die Warnung in `precheck.ts:211–219` beruht aber auf dem bis zu 20 s alten Status. `ghostctl` gleicht zwar vor dem Bau ab, prüft das Ergebnis aber gegen nichts.

**Hergang (Test).** Pool wie im Mainnet-Plan: `init` mit 1 KAS + 0,04 GHOST (1 GHOST = 25 KAS). (1) Angreifer kauft für 9 KAS GHOST – der Pool steht jetzt bei ≈ 2 440 KAS/GHOST. (2) Angreifer legt 990 KAS + 0,4 GHOST zum verschobenen Kurs ein. (3) Opfer ruft `pool::add` mit 999 KAS + 39,96 GHOST und `min_shares = 1` auf (genau Schritt 3 von `pool-open` bzw. ein `pool-add` mit den geplanten Zahlen). Die Tx baut, besteht die lokale Skriptprüfung und wird im Simulator angenommen:

```
Opfer:     eingelegt 1998.00 KAS-Wert, Anspruch 1503.26 → Verlust 494.74 KAS (24.8 %)
Angreifer: eingesetzt 1008.03 KAS-Wert, Anspruch 1489.72 → Gewinn 481.69 KAS
Poolkurs danach: 1 GHOST = 49.53 KAS (fair 25)
```

Danach steht der Pool 98 % über dem fairen Kurs; die Arbitrage zurück gehört ebenfalls dem, der schnell ist.

**Einordnung.** Gezieltes „Sandwichen“ einer bestimmten Tx geht im UTXO-Modell nicht (die Tx bindet die exakte Pool-UTXO, der Bau folgt dem Abgleich). Der Angriff braucht also, dass das Opfer nach der Verschiebung mit alten Zahlen einlegt. Am wahrscheinlichsten ist das bei `pool-open`: Der Pool ist nach `init` nur 1 KAS groß und lässt sich für wenige KAS beliebig verschieben. Schritt 3 wird gegen die init-Ausgabe gebaut und scheitert, wenn jemand dazwischen tauscht. `pool-open` ist danach nicht fortsetzbar („schon einen Pool … Einlegen mit pool-add“), der Betreiber legt dann mit `pool-add` und den geplanten Zahlen nach (MAINNET.md, Schritt 4). Auch ohne Angreifer genügt ein Fremdtausch zwischen Vorschau und Senden.

**Wichtig für die Behebung.** Nur den Überschuss zurückzugeben reicht **nicht**. Wer zu einem verschobenen Kurs *passend* einlegt, verliert anschließend über die Arbitrage, und zwar noch mehr. Test `a10_passende_einlage_zum_verschobenen_kurs`: Das Opfer legt nach der 9-KAS-Verschiebung 999 KAS + die passenden 0,4 GHOST ein, der Angreifer verkauft GHOST zurück auf 25 KAS/GHOST. Ergebnis: Opfer −80,1 % (1 009 → 200 KAS-Wert), Angreifer netto +808,67 KAS. Nötig ist ein Kursband gegen das Verhältnis, das der Nutzer erwartet – wie `amountAMin/amountBMin` im Uniswap-Router.

**Code-Vorschlag** (`pool.rs`):

```rust
/// Einlegen: höchstens `kas`/`ghost`; genommen wird nur, was zu m passt.
/// `tol_bps`: je Seite darf höchstens so viel weniger genommen werden als
/// eingegeben (Kursband, wie amountAMin/amountBMin). Rückgabe: Tx, m, (dx, dy).
pub fn add(rec: &PoolRec, who: &Keypair, my_tokens: &[Tracked<GhostTok>], kas: i64, ghost: i64,
           min_shares: i64, tol_bps: i64, fund: &Funds, net: &Params) -> Result<(PoolTx, i64, (i64, i64)), String> {
    let (x, y, sh) = (rec.kas(), rec.ghost(), rec.shares());
    // … bisherige Prüfungen (kas/ghost > 0, have >= ghost) …
    let m = shares_for_deposit(sh, x, y, kas, ghost);
    if m <= 0 { return Err("Einlage zu klein für einen Anteil".into()); }
    if m < min_shares.max(1) { return Err("Der Pool hat sich verschoben: weniger Anteile als erwartet. Bitte neu prüfen.".into()); }
    if sh as i128 + m as i128 > MAX_SHARES as i128 { return Err("Anteils-Obergrenze des Vertrags erreicht".into()); }
    // Vertrag: m·x ≤ S·Δx und m·y ≤ S·Δy → kleinste passende Beträge (≤ Eingabe, da m ≤ S·kas/x)
    let up = |a: i64| ((m as i128 * a as i128 + sh as i128 - 1) / sh as i128) as i64;
    let (dx, dy) = (up(x), up(y));
    let low = |got: i64, want: i64| (got as i128) * 10_000 < want as i128 * (10_000 - tol_bps) as i128;
    if low(dx, kas) || low(dy, ghost) {
        return Err(format!(
            "Der Kurs des Pools weicht von deinem Verhältnis ab: passend wären {:.8} KAS zu {:.8} GHOST. Bitte neu prüfen.",
            dx as f64 / 1e8, dy as f64 / 1e8
        ));
    }
    let t = pool_tx(rec, "add", who, my_tokens, &[], x + dx, y + dy, have - dy, sh + m, m, fund, net)?;
    Ok((t, m, (dx, dy)))
}
```

`ghostctl`: `PoolAdd { …, #[arg(long, default_value_t = 50)] max_abweichung_bps: i64, #[arg(long)] min_shares: Option<i64> }`; `pool-open` Schritt 3 vor dem Bau `pool::resync` und `add(…, tol_bps = 50)` mit dem vom Nutzer gewählten Startverhältnis; Seite: `--min-shares` aus der Vorschau mit Slippage übergeben und in `precheck.ts` warnen, wenn der Poolkurs mehr als einige Prozent vom Orakelkurs abweicht. Die Vertragsregel selbst ist richtig und bleibt. Optional (Vertragsänderung): `init` die erste Einlage des Gründers in derselben Tx mitgeben lassen, damit es das Fenster „Pool mit 1 KAS“ nicht gibt.

### A10-P-2 — Anteile über 1e16 findet `resync` nicht (niedrig)

**Ort.** `pool.rs:30` (`MAX_RESERVE = 1e16`), `pool.rs:489` (`script_num`: `v > MAX_RESERVE` → verworfen), `find_amounts_rest` (575: „Anteils-Minter … nicht gefunden“). Vertrag: `MAX_SHARES = 2^60−1` (`ghost_pool.sil:51, 204`).

**Erreichbarkeit (hergeleitet, Invariante per Rundungsrichtung belegt).** Kein Vorgang senkt `x/S · y/S`: `add` rundet `m` ab (`(x+Δx)/(S+m) ≥ x/S`), `remove` rundet die Auszahlung ab, `swap` hält `x·y` mindestens und lässt S gleich. Mit S0 = x0 folgt `S² ≈ x·y·x0/y0` (Gleichheit bis auf Gebühren und Rundung). Beim Mainnet-Plan (0,045 GHOST je KAS, also y0/x0 = 0,045 Einheiten je sompi) und fairem Kurs von 25 KAS/GHOST ist S ≈ 23,6·y; S > 1e16 bräuchte 4,2 Mio. GHOST und mehr als `MAX_KAS` – unerreichbar. Legt jemand den Pool aber mit einem sehr kleinen GHOST-Anteil an (z. B. `pool-open --kas 1000 --ghost 0.00001` → 1 Einheit auf 1 KAS gesperrt, y0/x0 = 1e-8), ist S ≈ 5·10^4·y; schon bei rund 2 000 GHOST und 50 000 KAS im Pool ist S > 1e16, und der Pool ist für jeden `ghostctl`-Nutzer gesperrt, bis Anteile abgezogen werden. `MAX_SHARES` selbst ist nur bei so einem Fehlstart und sehr großen Reserven erreichbar. `pool-open` prüft das Startverhältnis nicht (die Seite zeigt nur den Startkurs an).

**Code-Vorschlag.**

```rust
const MAX_CANDIDATE: u64 = (1u64 << 60) - 1; // Anteile bis MAX_SHARES; GHOST ≤ 1e16 liegt darunter
fn script_num(b: &[u8]) -> Option<i64> {
    // … wie bisher …
    if b[b.len() - 1] & 0x80 != 0 || v == 0 || v > MAX_CANDIDATE { return None; }
    Some(v as i64)
}
```

Die zusätzlichen Kandidaten sind unschädlich, weil jeder Treffer gegen das Ausgangsskript und am Node (Covenant-ID, Tx) bestätigt wird. Regressionstest: `a10_anteile_ueber_1e16_findet_resync_nicht` mit umgekehrter Erwartung.

### A10-P-3 — Leere Node-Antwort löscht den Pool aus der Zustandsdatei (mittel)

**Ort.** `pool.rs:608–610`: Pool-UTXOs unter der festen Adresse mit passender Covenant-ID; `[] => return Ok(None)`. `ghostctl.rs:371–374`: `Ok(None) => { notes.push("Der Pool wurde aufgelöst."); d.pool = None; }`, Zeile 382 speichert, weil `notes` nicht leer ist. `status_json` ruft `load_synced` bei jedem Status-Abruf auf.

**Warum falsch.** Im offenen Pool gibt es kein `close`: Eine initialisierte Pool-UTXO wird in jeder Pool-Tx genau einmal fortgesetzt (`OpAuthOutputCount == 1`) und kann nie verschwinden. `Ok(None)` ist also immer ein Node-Fehler (nicht synchron, UTXO-Index unvollständig, manipulierter Node; `ghostctl` verbindet sich unverschlüsselt über `ws://` zu öffentlichen Nodes, siehe `net.rs:25–28`). Die Folge ist dauerhaft: `PoolRec` mit `params` und `lp_cov` ist weg, `pool-remove` meldet „In diesem Netz gibt es keinen Pool“, die Anteile in `lp_tokens` sind über `ghostctl` nicht mehr abziehbar, und `pool-open` legt einen zweiten Pool an (die Sperre prüft nur `d.pool.is_some()`). Wiederherstellung nur mit einer Sicherung der Zustandsdatei. On-chain geht nichts verloren. Das Muster „leere Antwort ⇒ lokal löschen“ steckt auch in `store::resync` für GHOST-UTXOs (`store.rs:242–249`), dort aber vor diesem Audit und außerhalb des Umfangs.

**Beleg.** Hergeleitet aus dem Code; ein Test ohne Node ist nicht möglich (`Net` ist ein echter RPC-Client).

**Code-Vorschlag.**

```rust
// pool.rs, resync
let (op, entry) = match cands.as_slice() {
    // Offener Pool: keine Auflösung möglich – leere Antwort ist ein Node-Fehler
    [] => return Err("Pool-UTXO am Node nicht gefunden (Node nicht synchron?) – Stand bleibt unverändert".into()),
    [one] => one.clone(),
    _ => return Err("mehrere Pool-UTXOs gefunden – der Pool ist nicht eindeutig".into()),
};
// Rückgabetyp dann Result<PoolRec, String>; in ghostctl::load_synced den Zweig Ok(None) streichen.
```

### A10-P-4 / A10-P-5 — Abziehen an den Rändern (niedrig)

**Ort.** `pool.rs:389–407`, `ghostctl.rs:1097–1101`.

- *Letzter Einleger:* Die gesperrten S0 Anteile sind bei init 1 KAS + ghost0 wert. Steigt KAS gegenüber GHOST (oder wird GHOST in den Pool verkauft), fällt `x/S` unter 1 sompi je Anteil; dann ist der KAS-Anteil der gesperrten Anteile kleiner als die 1 KAS, die der Vertrag im Pool verlangt. `pool::remove` lehnt 100 % ab. Der Vertrag erlaubt eine Auszahlung mit weniger KAS (jede Seite wird einzeln begrenzt, `ghost_pool.sil:310–311`). Im Test säßen ohne Kappung 0,714 KAS fest; mit Kappung kommt alles bis auf den Rest zur 1-KAS-Grenze heraus.
- *Mini-Abzug:* `m = 1` bei `x/S < 1` ergibt (0, 0); die Tx verbrennt den Anteil und zahlt nichts aus.

```rust
let (dx0, dy) = payout_for(sh, x, y, m);
let dx = dx0.min(x - POOL_MIN_KAS as i64).max(0); // Vertrag: x' ≥ MIN_KAS, Seiten einzeln begrenzt
if dx == 0 && dy == 0 { return Err("Zu wenige Anteile für eine Auszahlung".into()); }
if dy >= y { return Err("Die GHOST-Reserve darf nicht leer werden".into()); }
// dx0 − dx dem Nutzer als „bleibt im Pool (1-KAS-Grenze)“ anzeigen
```

In `ghostctl pool-remove` die Vorschau als `min_kas`/`min_ghost` übergeben (heute fest 0, 0). Das ist unkritisch, weil `remove` immer anteilig auszahlt und die Tx die exakte Pool-UTXO bindet, zeigt aber den Betrag, der tatsächlich kommt.

### A10-P-6 — Token im Besitz der Covenant-IDs des Pools (Info)

`ghost_token.sil:31–33`: Für `IDENTIFIER_COVENANT_ID` gilt nur `OpCovInputCount(owner) > 0`. In jeder Pool-Tx sind `ghostCovId` und `lpCovId` als Eingänge vorhanden. Der Pool schließt nur Token mit Besitzer = Pool-Covenant aus (`mine`). Test: Ein Anteils-Token mit Besitzer `(COV, lpCovId)` wird von einem beliebigen Unterzeichner verbrannt, die Auszahlung geht an ihn. Heute entsteht so ein Token nur durch einen Bedienfehler außerhalb von `ghostctl`. Wer Token „an den Pool“ schicken will, verliert sie so ohnehin (Geschenke bleiben seit dem Wegfall von `close` für immer liegen, vgl. Audit 9 P-8).

### A10-P-7 — Echtheit eines Pools hängt an seiner Genesis-Kette (Info)

Die Covenant-ID bindet Wert und Skript der Genesis-Ausgänge (`covenant_id.rs:16–30`). Der „Henne-Ei“-Zyklus aus `ARCHITEKTUR.md:203` verhindert zwar, dass Pool und *neuer* Anteils-Token sich gegenseitig in einer Genesis kennen. Er greift aber nicht, wenn `lpCovId` auf einen **bereits bestehenden** KCC20-Token T mit derselben Vorlage zeigt, dessen Minter der Ersteller hält. Dann enthält *eine* Tx: die Pool-Genesis mit Zustand `(T, initialized = true)`, den T-Minter mit Besitzer = Pool und S Anteilen, beliebig viele T-Anteile für den Ersteller (Minter-Leader, keine Mengenerhaltung) und eine GHOST-Reserve an den Pool. Alles stammt aus derselben Tx, `swap`/`add`/`remove` laufen normal – nur hält der Ersteller Anteile ohne Deckung und zieht spätere Einlagen ab. Teilbeleg: `a10_vertrag_vertraut_der_herkunft_der_anteile` (Inhaber mit 10^14 Anteilen bei S = 2·10^8 zieht 99,9 % beider Reserven ab). Die Bildbarkeit der Genesis ist hergeleitet und nicht im Simulator gebaut.

Wer einen Pool nutzt, den er nicht selbst angelegt hat, muss prüfen: (1) Die Genesis-Tx der Pool-Covenant-ID hat genau eine Pool-Ausgabe mit `initialized = false`, `lpCovId = 0`. (2) Die folgende Tx verbraucht sie über `init` und erzeugt `lpCovId` dort als Genesis, autorisiert vom Pool-Eingang, als einzige Ausgabe mit dieser ID. `ghostctl` kennt nur selbst angelegte Pools, daher heute ohne Wirkung. Für eine spätere „Pool übernehmen“-Funktion ist die Prüfung Pflicht.

### A10-P-8 / A10-P-9 — Kleinigkeiten (Info)

Siehe Tabelle. Zusätzlich: `amount()` begrenzt auf 1e10 KAS/GHOST, `pool::add` prüft `x + kas ≤ MAX_KAS` nicht selbst. Das ist harmlos, weil `txb::build` die Skripte lokal ausführt und die Tx vor dem Senden ablehnt; die Meldung ist dann aber nur „VerifyError“.

---

## 3. Geprüft und ohne Befund

**Vertrag `ghost_pool.sil`**

| Frage | Ergebnis | Beleg |
|---|---|---|
| Anteile aus dem Nichts / Verwässerung | Nein. `ΔS == ΔInhaber` (Zeile 205); nur Ausgang 0 ist Minter und pool-eigen, alle anderen weder noch (`checkOuts`); Inhaber-Eingänge brauchen die KCC20-Signatur. `x/S` und `y/S` sinken durch `add`/`remove` nie, `x·y/S²` sinkt durch keinen Vorgang. | `pool_tests` (Mengenerhaltung, zweiter Minter, Anteile an den Pool); `a10_rechnung_pool_rs_gleich_vertrag_bei_add_und_remove` (Einlegen und sofort Abziehen bringt nie mehr als eingelegt, Grenze m / m+1 in der Engine bei S bis 2^58) |
| Erste Einlage / Inflationsangriff / Spende | Wirkungslos: Die Spende fließt überwiegend an die gesperrten S0 = 1e8 Anteile. Das Opfer verliert höchstens ⌈x/S⌉. | `a10_spendeangriff_lohnt_nicht`: Opfer −9 sompi, Angreifer −999,99 KAS |
| Minter-Status verlieren/gewinnen, zweiter Minter | Nicht möglich: Pool-Minter muss Minter bleiben (Zeile 129), weitere Ausgänge nie Minter; Nicht-Minter-Leader darf keine Minter erzeugen (KCC20). Leader ist per Compiler immer `OpCovInputIdx(cov, 0)` (`covenant_declarations.rs:723–724`), Delegates dürfen nicht an Index 0 stehen (819–823). | `minter_status_der_pool_ausgaenge_ist_fest`, `einlegen_mengenerhaltung_der_anteile` |
| Zweite Reserve / geschenkte Token / fremde Tx | Nur der pool-eigene Token aus derselben Tx wie die Pool-UTXO zählt (`sameTx`); in jeder Pool-Tx gibt es genau einen pool-eigenen GHOST- und Anteils-Ausgang. | `geschenkter_token_als_reserve_scheitert`, `minter_aus_fremder_tx_oder_fehlend_scheitert`, `zweiter_pool_token_rein_oder_raus_scheitert` |
| Fremde Token mit gleicher Vorlage | Zählen nicht: Erkennung über `OpCovInputCount(ghostCovId/lpCovId)`; Genesis-Ausgänge erhalten eine andere ID. | Konsens `covenant_id.rs`; A9-4 |
| Überläufe (`mul`, `productGe`, `mulDivUp`) | Alle Argumente < 2^60: Reserven ≤ 1e16, S ≤ 2^60−1, m < 2^60; `mulDivUp` mit a ≤ 1e16, b = 30. Tausch, Einlegen und Abziehen bei `x, y ≈ 1e16` und `S = 2^60−1` gleichzeitig exakt (Maximum geht, +1 nicht). | `a10_grenzen_max_shares_und_max_reserven`; `mul_ist_exakt_…` |
| Sperren des Pools on-chain | Kein Zustand ohne gültige Folge-Tx: Leer-Tausch und Abzug ohne KAS gehen immer. `MAX_KAS`/`MAX_SHARES` sperren nur weitere Zuflüsse; Abziehen und Tausch in Gegenrichtung bleiben möglich. `MIN_KAS` verhindert das Leerräumen. Das Anstupsen (Audit 9 P-3) bleibt bestehen. | `pool_behaelt_1_kas_und_etwas_ghost`, `abziehen_haelt_1_kas` |
| `init` / Genesis des Anteils-Tokens | Nur Gründer, nur einmal; `freshGenesis`: ID ≠ 0, kein Eingang mit dieser ID, einzige Ausgabe mit ihr; Minter-Zustand (Pool, COV, S0 = x2, Minter) per Vorlage geprüft; GHOST-Eingänge weder Minter noch pool-eigen. Der Gründer kann sich bei `init` keine Anteile geben. | `init_*`-Tests in `pool_tests` |
| KAS-Wert von Reserve und Minter | Gleich dem Eingang. | `kas_der_reserve_und_des_minters_bleiben` |
| Gebühr und Rundung beim Tausch | Aufgerundet zugunsten des Pools, nur auf den Zufluss. | Audit 9 A9-6, `ohne_gebuehr_zu_tauschen_scheitert` |
| Parser-DoS durch andere Kodierung | Nicht möglich: Der Compiler verlangt für dynamische Arrays `OP_SIZE % 8 == 0` (`statement.rs:190–199`); die Beträge stehen also immer im 8-Byte-Raster, das `amount_candidates` liest. | Code |

**Off-chain**

- *Rechnung pool.rs = poolMath.ts = Vertrag:* 500 Zufallsfälle (`ghostOut`, `kasOut`, `sharesForDeposit`, `payoutFor`, davon 100 mit Werten bis 1e16/2^60) ohne Abweichung (`a10_poolmath_check.mjs`). `pool.rs` ist an der Grenze gegen die Engine geprüft (`a10_rechnung_…`). `sharesForDeposit` in TS kennt `MAX_SHARES` nicht, `ghostctl` prüft sie aber.
- *Köder und REST (Audit 9 P-1/P-2):* weiter behoben. `find_tok` verlangt Tx-ID **und** Covenant-ID, Wert und Outpoint kommen vom Node. Im Vertrag kann es je Pool-Tx nur einen pool-eigenen Ausgang je Token geben, Köder ohne Covenant und fremde Genesis fallen heraus. Beide Beträge (Reserve und Anteile) werden aus dem 8-Byte-Raster gelesen (e2e-Test prüft das je Pool-Tx).
- *Tausch:* Mindestbetrag wirksam (exakte UTXO, Prüfung beim Bau). Die Seite übergibt ihn immer (`actions.ts:210–217`); die Wiederholung in `ghostctl swap` hält den ersten Mindestbetrag.
- *Journal:* Pool-Tx laufen über `send_to` mit Ziel-Zustand. Nach Annahme schreibt `resolve_pending` den Zustand, `resync` holt spätere Fremd-Tx nach. Kein neuer Fehler.

---

## 4. Was sich durch „offen“ geändert hat (gegenüber Audit 9)

| Vorher (besitzergebunden) | Jetzt (offen) | Folge |
|---|---|---|
| Liquidität nur vom Besitzer, `manage`/`close` mit Signatur | `add`/`remove` für jeden, Anteils-Token mit Minter im Pool | Neue Angriffsfläche Anteilsrechnung: hält (Abschnitt 3). Der Schutz des Einlegers liegt jetzt off-chain und fehlt (A10-P-1). |
| `close` konnte den Pool auflösen | kein `close`, Pool dauerhaft | `resync`-Zweig „aufgelöst“ ist jetzt immer ein Fehlerfall und zerstört Zustand (A10-P-3). Geschenkte pool-eigene Token bleiben für immer liegen (P-8 wird endgültig). Die 1 KAS Pool-Genesis bleibt ohne `init` für immer gebunden. |
| ein Betrag (Reserve) zu finden | zwei Beträge (Reserve, S) | S hat eine andere Obergrenze als die Reserven (A10-P-2). |
| Mindestliquidität = Besitzerkapital | S0 = Start-KAS gesperrt, niemandem gehörend | Schützt gegen Inflationsangriffe (belegt). Nebenwirkung: Der Wert eines Anteils hängt am Startverhältnis (A10-P-2), und der letzte Einleger stößt an die 1-KAS-Grenze (A10-P-4). |
| Gründer = Besitzer mit Rechten | Gründer nur für `init` | Nach `init` keine Sonderrechte (geprüft). Die Echtheit eines fremden Pools ist nur über die Genesis-Kette prüfbar (A10-P-7). |

---

## 5. Mutationslauf

`audit/a10_mutate.sh`: jede der 60 `require`-Zeilen einzeln durch `require(true)` ersetzt, **nur in einer Temp-Datei**; die Testkopien `m10_pool_tests`/`m10_audit10_pool` lesen den Vertrag aus `$A10_POOL_SIL`. Geprüft wurde gegen `pool_tests` (32) + `audit10_pool`. Der Vertrag ist danach unverändert (SHA-1 `12a2d170…`). Ergebnis: **28 rot, 32 Lücken** (Log: `audit/a10-mutation.log`).

*Hinweis zur Methode:* Ein erster Versuch mit `protocol/mutation/mutate.sh`-Logik (Vertrag an Ort und Stelle ändern) wurde abgebrochen, weil jede Mutante die ganze Bibliothek neu baut. Beim Abbruch lief die Schleife nach der Trap-Wiederherstellung weiter und leerte den Vertrag in der **Kopie**. Er wurde sofort aus der vorher gezogenen Sicherung wiederhergestellt (Prüfsumme gleich). Das eigene Skript des Projekts hat dieselbe Schwäche: Bei einem Abbruch (INT) läuft die `for`-Schleife nach dem Trap weiter, und `awk` schreibt aus der gelöschten Sicherung eine leere Datei. Empfehlung: `trap '…; exit 1' INT TERM`.

**Lücken nach Art der Deckung**

| Zeilen | Regel | Deckung / Bewertung |
|---|---|---|
| **L214** | `OpCovInputCount(id) == 0` (freshGenesis) | **Tragend, ohne Test.** Gegenprobe `a10_gegenprobe_l214_init_mit_bestehendem_token`: Mit der Mutante nimmt `init` die Fortsetzung eines **bestehenden** KCC20-Tokens des Gründers als „Genesis“ des Anteils-Tokens an (alle Eingänge ok), das Original lehnt nur am Pool ab. Ohne die Regel könnte der Gründer vorab Anteile ohne Deckung halten (A10-P-7 über den regulären `init`). → **A10-P-10** |
| L118–L120, L147/L148, L180/L181, L245/L246 | Anzahl Ein-/Ausgänge, `outs.length` | Wie in Audit 9 von KCC20 gedeckt (Leader: `in ≤ from`, `out ≤ to`, `out == newStates.length`) bzw. von fehlschlagenden `OpCov*Idx(…, 0)`. |
| L192/L193 | Minter pool-eigen / Minter-Flag | Gegenprobe `a10_gegenprobe_l192_l193_inhaber_als_minter`: Ein Inhaber-Token aus der letzten Pool-Tx als „Minter“ scheitert am Pool (Ausgang 0 muss Minter sein) und an KCC20 (Signatur, Nicht-Minter darf keinen Minter ausgeben). |
| L197/L198 | weitere Anteils-Eingänge nicht pool-eigen / kein Minter | Es gibt je Pool-Tx nur einen pool-eigenen Anteils-Ausgang (den Minter, `checkOuts`), und der steht an Index 0. Ein zweiter kann nicht existieren. Hergeleitet. |
| L167/L168, L253/L254, L203 | Reserve > 0, ≤ MAX_GHOST; S' > 0 | `> 0`: durch `productGe` (swap), anteilige Grenze mit m < S (remove) und `init`-Genesis gedeckt. `≤ MAX_GHOST`: nur mit mehr als 1e16 GHOST verletzbar; ohne sie wäre `mul()` nicht mehr exakt. Test fehlt (wie `einlegen_ueber_der_kas_obergrenze_scheitert` für KAS). |
| L213, L216, L234 | Genesis-ID ≠ 0, ≤ 8 Ausgänge, `lid ≠ ghostCovId` | Konsens bzw. `checkOuts` (ein GHOST-Ausgang kann den verlangten Minter-Zustand mit Pool-Besitz nicht tragen). |
| L249/L250 | init: GHOST-Eingänge weder Minter noch pool-eigen | Ohne Test. Ein Minter-Eingang würde KCC20-Mengenerhaltung aufheben (GHOST aus dem Nichts, aber nur für den Gründer selbst, der den Minter ohnehin besitzen müsste); pool-eigene GHOST gibt es vor `init` nur als Geschenk. Test empfohlen. |
| L283, L301 | `require(initialized)` in add/remove | Vor `init` gibt es weder Reserve noch Minter aus derselben Tx (`sameTx`, `lpCovId = 0`). Hergeleitet. |
| L289, L306 | `m > 0` | m = 0: Spende bzw. leerer Abzug, harmlos; m < 0 ist durch die Mengenerhaltung (hIn = 0 bei add) bzw. `productGe` gedeckt. |
| L291/L292, L308/L309 | Richtung der Reserven | Gegenprobe `a10_gegenprobe_l291_add_mit_kas_entnahme`: `productGe` mit negativem Faktor lehnt ab. Bei remove wäre eine Zuzahlung eine Spende. |

**Empfehlung:** Test für L214 (die Gegenprobe mit umgekehrter Erwartung übernehmen), L249/L250 und `MAX_GHOST` ergänzen, damit der Schutz nicht nur an Konsens- und KCC20-Details hängt.

---

## 6. Tests und Ausführung

Dateien (nur in der Arbeitskopie):
- `protocol/tests/audit10_pool.rs` — Engine; Harness aus `pool_tests.rs` übernommen (einzige Änderung: `Env.src` für Mutanten-Gegenproben). Tests: `a10_rechnung_pool_rs_gleich_vertrag_bei_add_und_remove`, `a10_grenzen_max_shares_und_max_reserven`, `a10_spendeangriff_lohnt_nicht`, `a10_letzter_einleger_und_1_kas_rest`, `a10_token_im_besitz_der_anteils_id_ist_fuer_jeden_frei`, `a10_vertrag_vertraut_der_herkunft_der_anteile`, `a10_gegenprobe_l214_init_mit_bestehendem_token`, `a10_gegenprobe_l291_add_mit_kas_entnahme`, `a10_gegenprobe_l192_l193_inhaber_als_minter`, `a10_export_faelle_fuer_poolmath_ts` (nur mit `AUDIT10_EXPORT`).
- `protocol/tests/audit10_pool_sim.rs` — Simulator: `a10_einlage_nach_kursverschiebung_wird_verschenkt` und `a10_passende_einlage_zum_verschobenen_kurs` (A10-P-1), `a10_anteile_ueber_1e16_findet_resync_nicht` (A10-P-2), `a10_abziehen_nach_kursverschiebung` (A10-P-4/-5).
- `audit/a10_poolmath_check.mjs` + `audit/a10_cases.json` — TS-Abgleich (Node 25, ohne `node_modules`).
- `protocol/tests/m10_pool_tests.rs`, `protocol/tests/m10_audit10_pool.rs`, `audit/a10_mutate.sh`, `audit/a10-mutation.log` — Mutationslauf (Kopien, die den Vertrag aus `$A10_POOL_SIL` lesen; der Vertrag selbst wird nicht verändert).

```
cd protocol
CARGO_TARGET_DIR=../target cargo test --release --test audit10_pool --test audit10_pool_sim -- --nocapture
AUDIT10_EXPORT=../audit/a10_cases.json CARGO_TARGET_DIR=../target cargo test --release --test audit10_pool a10_export
cd .. && node audit/a10_poolmath_check.mjs audit/a10_cases.json
```

Protokoll (`audit/a10-testlog.txt`):

```
test a10_export_faelle_fuer_poolmath_ts ... ok
Inhaber als Minter: [Err("VerifyError"), Ok(()), Err("InvalidSigHashType(0)"), Ok(())]
Inhaber als Minter: [Err("VerifyError"), Ok(()), Err("InvalidSigHashType(0)"), Ok(())]
test a10_gegenprobe_l192_l193_inhaber_als_minter ... ok
Opfer-Verlust 9 sompi, Angreifer-Verlust 99999999001 sompi, Anteile Opfer 999001
test a10_spendeangriff_lohnt_nicht ... ok
test a10_gegenprobe_l214_init_mit_bestehendem_token ... ok
test a10_gegenprobe_l291_add_mit_kas_entnahme ... ok
festsitzend ohne Kappung: 71400000 sompi
test a10_letzter_einleger_und_1_kas_rest ... ok
test a10_token_im_besitz_der_anteils_id_ist_fuer_jeden_frei ... ok
Abzug mit 100000000000000 Anteilen bei S = 200000000: 9990.00 KAS und 459.54 GHOST von 10000.00 / 460.00
test a10_vertrag_vertraut_der_herkunft_der_anteile ... ok
test a10_grenzen_max_shares_und_max_reserven ... ok
test a10_rechnung_pool_rs_gleich_vertrag_bei_add_und_remove ... ok
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 25.59s
test a10_anteile_ueber_1e16_findet_resync_nicht ... ok
x = 286.33 KAS, S = 100000000000, x/S = 0.286 sompi je Anteil
pool-remove 100 %: So viel lässt der Pool nicht abziehen: mindestens 1 KAS bleibt. Bitte weniger Anteile abziehen.
test a10_abziehen_nach_kursverschiebung ... ok
Opfer: eingelegt 1009.02, Anspruch 200.34 → Verlust 80.1 %; Angreifer netto +808.67 KAS; Kurs danach 25.07
test a10_passende_einlage_zum_verschobenen_kurs ... ok
Opfer:     eingelegt 1998.00 KAS-Wert, Anspruch 1503.26 → Verlust 494.74 KAS (24.8 %)
Angreifer: eingesetzt 1008.03 KAS-Wert, Anspruch 1489.72 → Gewinn 481.69 KAS
Poolkurs danach: 1 GHOST = 49.53 KAS (fair 25)
test a10_einlage_nach_kursverschiebung_wird_verschenkt ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.51s
```

Bestehende Suiten in der Kopie: `pool_tests` 32/32 ok, `pool_e2e_tests` 1/1 ok (vor und nach dem Mutationslauf).
