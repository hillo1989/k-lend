# Audit 17: öffentliche Seite mit Browser-Wallet (Version 4)

Prüfer: Claude Opus 5.5, unabhängig, nur lesend. Datum: 05./06.10.2026.
Stand: `main` bei cf37f2c. Während der Prüfung kamen 8e9eb59 und cdf926b hinzu (PWA, MIME `.webmanifest`, Testtext). Sie berühren die geprüften Pfade nicht; `app/public/sw.js` fängt nur Navigationen ab und speichert nichts zwischen.
Tests liefen nur in einer Worktree-Kopie, die danach entfernt wurde. keys/ wurde nicht gelesen, aus deployments/ nur Dateinamen. Es wurde nichts gesendet und kein Server gestartet.

Geprüft: `protocol/src/wallet_ops.rs`, `txb.rs` (with_wallet_fill/assemble), `ghostctl wallet build|submit` (ghostctl.rs:3735–3930), `store.rs` (Sperre, Journal), `app/server/{walletActions,api,prod,actions,walletProbe}.ts`, `app/src/wallet/actions.ts`, `app/src/components/WalletSignFlow.tsx`, `docs/wallet-aktionen.md`, `protocol/tests/wallet_ops_tests.rs`, `wallet_ops_mutation.sh`, `deploy/hetzner/templates/*`.

## Kurzfassung

| # | Schwere | Befund |
|---|---|---|
| A17-1 | **hoch** | Etwas GHOST an den öffentlich bekannten Ersatzschlüssel (0x42…) sperrt für **alle** Besucher die Wallet-Aktionen, die GHOST ausgeben. Im Test belegt. |
| A17-2 | **hoch** | Das Journal unterstellt, dass nur „unser Schlüssel“ das Wechselgeld ausgeben kann. Bei Wallet-Tx stimmt das nicht. Eine Tx ohne Wechselgeld kann `resolve_pending` dauerhaft blockieren, und damit Orakel, Keeper und Status, bis der Betreiber eingreift. Mechanismus im Code belegt, Auslösung im Netz nur vermutet. |
| A17-3 | mittel | Die Ratenbegrenzung hinter einem Webserver im Docker-Container gilt für alle Besucher gemeinsam. Mit IPv6 lässt sie sich je Adresse umgehen. |
| A17-4 | mittel | `submit --send` nimmt die Sperre, bevor die Signaturen geprüft sind. Mit Plänen für fremde Adressen und Müll-Signaturen lassen sich Sperre und die beiden ghostctl-Plätze kostenlos belegen. |
| A17-5 | mittel (Funktion) | Caddy, nginx und Apache begrenzen die Anfrage auf 64 KB. Der submit-Body für mint, repay, close, withdraw und deposit ist 58–126 KB groß. Diese Aktionen scheitern öffentlich mit 413. Im Test gemessen. |
| A17-6 | mittel | Bei `--send` hält der Lauf die Sperre bis zu 600 s (`wait_accepted`). Der Agent wartet nur 60 s auf die Sperre, dann fallen Orakel- und Keeper-Runde aus. |
| A17-7 | niedrig | Proxy-Zeitlimit 95 s (nginx/Apache) gegen bis zu 600 s Senden: Die Seite meldet dann „Nicht gesendet“ statt „unklar“. |
| A17-8 | niedrig | Fehlermeldungen geben absolute Server-Pfade preis (Sperre, Journal, Zustandsdatei). |
| A17-9 | niedrig | Kein Content-Security-Policy-Kopf. Kommentare in ghost-web.service und prod.ts sagen noch „kein Senden“. |
| A17-10 | Hinweis | Die mitgelieferte Rückbau-Probe stimmt. Vier eigene Rückbau-Stellen wurden erkannt, zwei überleben (zweite Linie bzw. ohne Test). |

**Keine** Befunde gibt es bei: Server-Schlüssel signieren oder ausgeben, Argument-Injektion, fremde UTXOs, Vaults oder Token ausgeben, XSS und Clickjacking (Einzelheiten unten).

---

## Frage 1: Kann ein Besucher einen Server-Schlüssel benutzen?

**Nein, kein Pfad gefunden.**

- Öffentlicher Modus: `publicRouteAllowed` (actions.ts:660–663) lässt nur status, price, history, keys (beantwortet ohne ghostctl, api.ts:323–324) und `/api/wallet/…` zu. Der Pfad wird über `new URL` normalisiert, auch `%2e%2e`, also führt `/api/wallet/../action` zu `/api/action` und damit zu 403. `/api/wallet-probe` beginnt nicht mit `/api/wallet/` und bekommt 403. Rückbau TM3 zeigt, dass ein Test das absichert.
- `buildWalletBuildArgs` (walletActions.ts:118–192): feste Schlüssel je Anfrage und je Aktion, alle Werte durch Regex oder Zahlenprüfung. Adresse: `kaspa:q` mit genau 60 bech32-Zeichen (walletProbe.ts:28). Empfänger: bech32 oder 64 Hex. Beträge: `AMOUNT_RE`. Vault: Ziffern. Die Nachricht geht als ein Argument `--message=<text>` (walletActions.ts:105) und ist ohne Steuerzeichen. Kein Wert kann mit `-` beginnen. `--key`, `--state` und `--rpc` gibt es in diesen Argumenten nicht. Die globalen clap-Optionen sind nicht `global = true` (ghostctl.rs:39–60) und lassen sich deshalb nach dem Unterbefehl nicht setzen. `--ja` kommt nur bei `send === true` dazu (walletActions.ts:230). Das ist so gewollt, denn submit signiert selbst nichts.
- Plan und Antwort landen als Dateien in einem frischen `mkdtemp`-Ordner, Namen vom Server (api.ts:510–518).
- `wallet_action_cmd` (ghostctl.rs:3853ff.) lädt keine Schlüsseldatei. `run_action` signiert mit `Signer::Wallet(x)` (Platzhalter) oder in der Messkopie mit `mirror_key` (0x42, nur lokal). Keine der in wallet_ops verwendeten ops/pool-Funktionen braucht einen Betreiber-Schlüssel. sweep und liquidate brauchen keinen, Orakel und Pool werden nur als Covenant-Eingang gelesen.
- Zusätzlich: `InaccessiblePaths=-…/keys` (ghost-web.service).

## Frage 2: Schaden für andere Nutzer

### A17-1 (hoch): Staub-GHOST an den Ersatzschlüssel sperrt alle GHOST-Aktionen

`mirror_key()` ist fest auf `[0x42; 32]` gesetzt (wallet.rs:309–315). `mirror_dep` (wallet_ops.rs:369–382) ersetzt nur den Besitz des Besuchers durch `mirror_x`. Token, die **schon** `mirror_x` gehören, bleiben stehen. `own_tokens(md, mirror, …)` (wallet_ops.rs:216–231) findet in der Messkopie deshalb zusätzliche Token. Messkopie und echter Bau bekommen dann verschiedene Eingänge, und der Bau bricht ab.

Angriff über die Seite selbst: `transfer` mit `to` = 64 Hex von `mirror_x` und 1 Einheit GHOST. `ghostTarget` (walletActions.ts:90) und `ghost_target` (wallet_ops.rs:245) nehmen das an. Der Folgezustand trägt den Token des Empfängers ein (ops.rs:1100–1102).

Test in der Kopie (`audit17_staub_an_ersatzschluessel_blockiert_andere`, Code siehe Anhang). Nach einer Überweisung von 1e-8 GHOST an `mirror_x` scheitert für einen unbeteiligten Nutzer:
```
A17-1 transfer: Wallet-Bau: Budgets passen nicht zur Zahl der Eingänge
A17-1 repay:    Wallet-Bau: Budgets passen nicht zur Zahl der Eingänge
A17-1 redeem:   Wallet-Bau: Budgets passen nicht zur Zahl der Eingänge
```
Betroffen sind nach Code auch liquidate, swap (Verkauf) und pool-add. Mit LP-Anteilen an `mirror_x` gilt dasselbe für pool-remove (`pool::own`). Die Sperre hält an, bis jemand die Token mit dem öffentlichen Schlüssel ausgibt, und lässt sich für einen Bruchteil eines Cent wiederholen.
Abhilfe: In `own_tokens` und `pool::own` der Messkopie nur die Indizes verwenden, die der echte Bau für den Nutzer wählt (Indizes aus `d` berechnen und an die Messkopie übergeben). Oder `mirror_dep` entfernt fremde `mirror_x`-Token bzw. ordnet sie einem dritten Platzhalter zu. Dazu einen Regressionstest.

### A17-2 (hoch): Journal-Auflösung bei Wallet-Tx ohne Wechselgeld

`store::resolve_pending` Schritt 4 (store.rs:153–176): Wenn kein Ausgang sichtbar ist, nicht im Mempool und alle Eingänge verbraucht, dann entscheidet das Wechselgeld. Bei `None` lautet das Ergebnis „Unklar … Bitte im Explorer prüfen und danach … löschen“. Das ist ein `Err`, und es bleibt so, bis jemand das Journal von Hand löscht. Der Kommentar (store.rs:158–160) unterstellt: „den kann nur unser Schlüssel ausgeben“. Bei `wallet submit` gehören aber Wechselgeld und alle Funding-Eingänge dem **Besucher** (wallet_ops.rs:324, `change_spk: p2pk_spk(&me)`).

- Eine Tx ohne Wechselgeld bekommt jeder, der fast alles sendet (Rest < `MIN_CHANGE` = 0,2 KAS, txb.rs:233/343). Test `audit17_send_ohne_wechselgeld`: `change_index=None`.
- Das Journal bleibt liegen, wenn `wait_accepted` (store.rs:502–522) scheitert oder ghostctl vorher beendet wird. Beispiele: Neustart von ghost-web, `MemoryMax=600M` gilt für die Seite und ihre ghostctl-Kinder zusammen, Node-Fehler, oder der Empfänger gibt seinen Ausgang so schnell weiter, dass `wait_accepted` (Abfrage alle 700 ms) ihn nie als UTXO sieht. Danach sieht `resolve_pending` keinen Ausgang mehr, alle Eingänge sind verbraucht, Wechselgeld fehlt, und es bleibt beim dauerhaften `Err`.
- Folge: `load_synced` scheitert für jeden. Das betrifft `oracle_round` (ghostctl.rs:2602), `keeper_round` (2075), jede Aktion mit Schlüsseldatei und den Abgleich in `status_json` (1945, mit Rückfall auf ungeprüften Stand). **Das Orakel friert ein**, und es wird nicht mehr liquidiert, bis der Betreiber das Journal löscht.
- *Vermutung:* Ein Angreifer kann das gezielt herbeiführen. Er kennt die txid vorher (submit ohne send liefert sie). Er sendet ohne Wechselgeld an eine eigene Adresse und gibt den Ausgang sofort weiter, oder er bringt mit einem Zwilling der Tx mit anderem Compute-Budget (txid ändert sich, Sighash nicht, wallet.rs:608 / Test `sighash_v1_deckt_budget_und_speichermasse_nicht_ab`, Gebührenspielraum durch den verschenkten Rest) einen Doppelausgaben-Wettlauf zustande. Im Netz nicht erprobt, aber billig zu wiederholen.
- Dieselbe falsche Annahme mit Wechselgeld: Die Tx des Besuchers wurde angenommen, er gibt aber Wechselgeld und Ausgänge sofort weiter. Dann gilt sie als „verdrängt – verworfen“ und der Folgezustand wird nicht geschrieben (veraltete Token-Einträge, eher Selbstschaden; niedrig).

Abhilfe: Für Wallet-Tx im Journal ein Merkmal setzen und bei Schritt 4 über die txid entscheiden statt über das Wechselgeld. Möglich ist eine Abfrage per REST oder `get_transaction`, ob sie in einem Block angenommen wurde. Ein „unklar“ darf andere Abläufe nicht für immer sperren: Orakel und Keeper sollten ein ungelöstes Journal einer fremden Wallet-Tx nach einer Frist überspringen können. Zusätzlich oder vorerst: Wallet-Aktionen ohne Wechselgeld-Ausgang ablehnen.

### Fremde UTXOs, Vaults, Token: kein Befund

- Funding: Nur P2PK der Adresse, kein Covenant, keine Coinbase (wallet_ops.rs:386–392, 556). Beim Senden wird am Node geprüft, dass die UTXOs vorhanden sind (ghostctl.rs:3839–3850).
- Besitzer-Aktionen: `owned()` (wallet_ops.rs:205). Der Covenant verlangt die Besitzersignatur ohnehin, und `check_scripts` läuft vor dem Senden (wallet_ops.rs:750).
- Token: `own_tokens` filtert nach Besitzer. Rückbau RM2 zeigt, dass die Prüfung durch Tests abgedeckt ist.
- redeem, liquidate und sweep auf fremde Vaults gehen nach Protokoll bewusst ohne Erlaubnis.

### Zustand deployments/mainnet.json verfälschen

`next` stammt nur aus dem Neubau unter Sperre (wallet_ops.rs:650–656, 686–689) und wird erst nach `wait_accepted` geschrieben (ghostctl.rs:869–873). Wenn der Node die Tx ablehnt, wird das Journal nur bei `in_mempool == Ok(false)` gelöscht (ghostctl.rs:857–862). Vom Besucher gelieferte Inhalte gehen nicht in den Zustand ein. Eine Ausnahme ist die Journal-Auflösung oben (A17-2).

## Frage 3: submit

- Es wird wirklich der Neubau verwendet: `let plan = fresh.plan;` (wallet_ops.rs:656). Der Vergleich ist bitgleich über serde (652). Rückbau RM5 (Plan statt Neubau und kein Vergleich) wird erkannt.
- Die Antwort der Wallet wird geprüft: `diff` (wallet.rs:716–779) prüft Eingänge, UTXOs, Ausgänge, Locktime, Subnetz, Gas und Payload. Budget und Speichermasse werden nur vermerkt, das ist richtig, denn der Sighash deckt sie nicht ab. `read_sigs` (wallet.rs:785–815) liest den ersten 65-Byte-Push, prüft Hashtype 0x01 und Schnorr gegen den Sighash der **neu gebauten** unsignierten Tx. Danach wird mit den Signaturen neu gebaut, und es laufen `check_scripts`, Sigops, Blockgrenzen und Mindestgebühr (wallet_ops.rs:700–757).
- Wettlauf um Orakel- oder Pool-UTXO: Sperre (ghostctl.rs:3897), `load_synced` und Neubau-Vergleich. Der zweite Besucher bekommt „Plan passt nicht“ (Test `veralteter_plan_wird_abgelehnt`). Wer zu lange auf die Sperre wartet, bekommt nach 120 s einen Fehler. In Ordnung.
- Der Zustand wird nur bei angenommener Tx geschrieben, siehe oben, mit der Einschränkung A17-2.

## Frage 4: Ressourcen

### A17-3 (mittel): Ratenbegrenzung

`clientKey` (walletActions.ts:273–282) vertraut X-Forwarded-For nur, wenn die Gegenstelle Loopback ist. Mit einem Webserver im Docker-Container kommt die Anfrage aber von 172.x (GHOST_HOST, setup.sh:36–39, Commit 55b58f7). Dann teilen sich **alle** Besucher den Schlüssel der Container-Adresse, also 20 Aufrufe pro Minute für die ganze Seite (api.ts:265). Ein einzelner Angreifer sperrt damit die Wallet für alle. Umgekehrt ist der Schlüssel bei IPv6 die volle Adresse, ein /64 bringt also beliebig viele Kontingente. `maxKeys = 5000` mit Verdrängung erlaubt das zusätzlich.
Abhilfe: Vertrauenswürdige Proxy-Adressen aus GHOST_HOST übernehmen, IPv6 auf /64 kürzen, ein globales Wallet-Kontingent zusätzlich zum Kontingent je Absender.

### A17-4 (mittel): Sperre vor der Signaturprüfung

Bei `send=true` sieht die Reihenfolge so aus: `store::lock` (ghostctl.rs:3897), `load_synced` (resolve_pending, resync, Pool-REST), Funding-Prüfung, erst dann `wo::submit`. `build` nimmt **jede** gültige Schnorr-Adresse mit Guthaben an, auch eine fremde. Ein Angreifer baut also Pläne für eine fremde Adresse und schickt `send:true, confirmMainnet:true` mit Müll-Signatur. Jeder Aufruf hält die Sperre für die Dauer des Abgleichs (mehrere Sekunden) und belegt einen der 2 ghostctl-Plätze. Bei 20 pro Minute und etwa 3–6 s Abgleich sind beide Plätze fast dauernd belegt. Status und Preis (gleicher `run()`-Pool, api.ts:124–137) warten dann in der Schlange (max. 20, danach „ausgelastet“). Der Agent konkurriert mit 60 s Wartezeit um die Sperre. *Wie stark das die Agent-Runden trifft, ist geschätzt, nicht gemessen.*
Abhilfe: Erst ohne Sperre `load_readonly` und `wo::submit` ausführen. Nur wenn das gültig ist, die Sperre nehmen, `load_synced`, neu bauen, senden. Wallet-Aufrufe sollten eigene Plätze bekommen, getrennt von status/price.

### A17-6 (mittel): Sperre bis 600 s beim Senden

`send_to` hält die Sperre während `wait_accepted(…, 120 s, 600 s)` (ghostctl.rs:869). Solange die Tx im Mempool liegt (z. B. bei Stau mit Mindestgebühr), wartet der Agent nur 60 s (ghostctl.rs:1454/1509) und lässt Orakel- und Keeper-Runde ausfallen. Senden mit Schlüsseldatei verhält sich genauso, öffentlich kann aber jeder das auslösen.
Abhilfe: Die Sperre nach `write_pending` und `submit` freigeben. Das Journal deckt den Rest ab, `resolve_pending` macht das schon.

## Frage 5: Web

- CORS/Origin: keine CORS-Kopfzeilen. Ein vorhandener Origin muss `https://<domain>` sein, Sec-Fetch-Site same-origin/none, bei POST `X-Ghost-Client: 1` und JSON (actions.ts:625–651). Host-Whitelist gegen DNS-Rebinding. In Ordnung.
- Header (prod.ts:98–103): nosniff, no-referrer, `X-Frame-Options: DENY` (Schutz gegen Clickjacking), COOP. HSTS kommt von Caddy. **A17-9:** Ein `Content-Security-Policy` fehlt (z. B. `default-src 'self'; frame-ancestors 'none'`). Empfehlung, kein aktueller Fehler.
- XSS: WalletSignFlow.tsx rendert Adresse, `what`, Beträge, Fehlermeldungen und Report-Zeilen nur als React-Text (Zeilen 204, 214–218, 229–233, 257–262). Im ganzen `app/src` gibt es kein `dangerouslySetInnerHTML`. Nachrichten-Payloads werden in diesem Ablauf nicht angezeigt. Kein Befund.
- **A17-8:** `tail(stderr)` und `j.error` von ghostctl gehen ungefiltert an den Browser (api.ts:499–506, 531–538). Darin stehen absolute Pfade, z. B. „Sperre /opt/ghost/kaspa-lending/deployments/mainnet.lock“ (store.rs:46) oder der Journalpfad (store.rs:171–175). Dazu `Interner Fehler: ${message}` (api.ts:549). Niedrig, Pfade relativ ausgeben.
- **A17-7:** nginx `proxy_read_timeout 95s` bzw. Apache `ProxyTimeout 95` gegen bis zu 600 s Senden. Das Ergebnis ist ein 504 mit HTML, und `callWalletApi` (actions.ts:163–178) macht daraus `{ok:false}`. Die Seite zeigt „Nicht gesendet“, obwohl gesendet worden sein kann. Bei 5xx bzw. Netzfehler im Senden-Schritt sollte `unclear: true` gesetzt werden. Caddy hat standardmäßig kein solches Limit.

### A17-5 (mittel, Funktion): 64 KB im Webserver

`ghost.caddy`: `request_body max_size 64KB`, nginx `client_max_body_size 64k`, Apache `LimitRequestBody 65536`. Die Seite selbst erlaubt 768 KB (walletActions.ts:44). Gemessen in der Kopie (Plan plus Antwort als submit-Body):

| Aktion | KasWare | Kastle |
|---|---|---|
| mint | 58 380 B | 108 784 B |
| repay | 68 537 B | 126 473 B |
| transfer | 13 658 B | 21 190 B |
| swap | 9 418 B | 9 418 B |

Plan allein: mint, close, deposit und withdraw je etwa 51–52 kB, repay 61 kB (Testausgabe). Öffentlich scheitern damit mint/repay (beide Wallets) und close/withdraw/deposit (Kastle, vermutlich auch KasWare knapp darüber oder darunter) am Webserver mit 413. Abhilfe: Limit für `/api/wallet/submit` auf ca. 800 KB anheben, oder der Server merkt sich den Plan (Hash als Kennung), damit nur die Antwort zurückkommt.

## Frage 6: Rückbau-Proben

### Mitgelieferte Probe (`wallet_ops_mutation.sh`)

Sauber wiederholt (ohne meine Zusatztests), das Ergebnis stimmt mit docs/wallet-aktionen.md überein:
```
erkannt:  Plan-Vergleich (Neubau ≠ Plan)  (2 Test(s) rot)
erkannt:  Veränderte Felder der Wallet-Antwort  (1 Test(s) rot)
erkannt:  Signatur gültig (sig_valid)  (2 Test(s) rot)
überlebt: Skriptprüfung nach dem Einsetzen
überlebt: Hashtype 0x01
erkannt:  Schnorr-Prüfung gegen die Adresse  (1 Test(s) rot)
```
Anmerkungen: Das Skript überspringt `alle_aktionen`, `liquidieren` und `zu_viele`. Prüfungen, die nur diese Tests abdecken (z. B. RM2), sieht es nicht. Außerdem verändert es `src/` an Ort und Stelle (mit Wiederherstellung per trap). Es sollte nur in einer Kopie laufen.

### Eigene Proben

Rust (`cargo test --release --test wallet_ops_tests`, ohne Überspringen der großen Tests):

| Stelle | Ergebnis |
|---|---|
| RM1 `owned()` bei mint entfernt (wallet_ops.rs:275) | erkannt (`fremder_vault_baut_keinen_plan`) |
| RM2 Besitzer-Filter in `own_tokens` entfernt (wallet_ops.rs:217) | erkannt (`alle_aktionen…`, `liquidieren…`), aber **nicht** von der mitgelieferten Probe |
| RM3 `same_shape` immer Ok (wallet_ops.rs:429) | **überlebt**: Kein Test baut eine abweichende Messkopie. A17-1 zeigt, dass so etwas vorkommt. Folge wäre nur ein späterer Fehler (Budget/Gebühr), kein Diebstahl |
| RM4 Funding nur eigene P2PK (wallet_ops.rs:556) abgeschaltet | überlebt, zweite Linie: `select_funding`/`plan_funding` filtern, Signaturen und Prüfung am Node |
| RM5 submit nimmt Plan statt Neubau und ohne Vergleich (wallet_ops.rs:653/656) | erkannt (`veraenderter_plan…`, `veralteter_plan…`) |

TypeScript (`npx vitest run server`, Grundlauf 106/106 grün):

| Stelle | Ergebnis |
|---|---|
| TM1 Mainnet-Bestätigung beim Senden (walletActions.ts:227) | erkannt (2 rot) |
| TM2 XFF erster statt letzter Eintrag (walletActions.ts:278) | erkannt (1 rot) |
| TM3 öffentlich `/api/wallet` ohne `/` (actions.ts:661), würde /api/wallet-probe öffnen | erkannt (1 rot) |
| TM4 Nachricht als getrenntes Argument (walletActions.ts:105) | erkannt (2 rot) |
| TM5 Parameterliste je Aktion abgeschaltet (walletActions.ts:129) | erkannt (2 rot) |

Nicht durch Tests abgedeckt, weil nur mit Netz: `check_funding_at_node`, Sperre vor Signaturprüfung, `resolve_pending` mit Wallet-Tx.

## Anhang: Zusatztests (nur in der Kopie, an tests/wallet_ops_tests.rs angehängt)

```rust
#[test]
fn audit17_staub_an_ersatzschluessel_blockiert_andere() {
    let mut w = World::new();
    let (a, v) = (key(), key());
    w.sim.faucet(&a, 5_000 * E8 as u64);
    w.sim.faucet(&v, 5_000 * E8 as u64);
    w.by_wallet("open-vault A", &a, Action::OpenVault { kas: 2_000 * E8 as u64 }, Wallet::KasWare);
    w.by_wallet("mint A", &a, Action::Mint { vault: 0, ghost: 10 * E8 }, Wallet::KasWare);
    w.by_wallet("open-vault V", &v, Action::OpenVault { kas: 2_000 * E8 as u64 }, Wallet::KasWare);
    w.by_wallet("mint V", &v, Action::Mint { vault: 1, ghost: 10 * E8 }, Wallet::KasWare);
    assert!(w.plan(&v, &Action::Transfer { to: addr(&a), ghost: E8, payload: vec![] }).is_ok());
    let mx = faster_hex::hex_string(&w::mirror_x());
    w.by_wallet("staub an mirror", &a, Action::Transfer { to: mx, ghost: 1, payload: vec![] }, Wallet::KasWare);
    // danach: transfer/repay/redeem von V → Err("Wallet-Bau: Budgets passen nicht zur Zahl der Eingänge")
}

#[test]
fn audit17_send_ohne_wechselgeld() {
    // send mit kas = Guthaben − Gebühr − 0,01 KAS → plan.change_index == None,
    // gebaut und vom Simulator angenommen, built.change_index == None
}
```
