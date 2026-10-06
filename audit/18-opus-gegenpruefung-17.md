# Audit 18: Gegenprüfung der Fixes zu Audit 17 (Merge d3660b5)

Prüfer: Claude Opus 5.5, unabhängig, nur lesend. Datum: 06.10.2026. Stand `main` = d3660b5 (Diff gegen 6127bb3).
Kein cargo, kein Server, nichts gesendet, keys/ nicht gelesen. Ausgeführt: `vitest run server/walletActions.test.ts server/prod.test.ts src/wallet/actions.test.ts`, Ergebnis 55/55 grün.
Zeitrahmen etwa 30 Minuten. Geprüft wurde, was einem öffentlichen Besucher nützen könnte. „Vermutung“ heißt: nicht gemessen.

## Antwort vorweg: Kann das so öffentlich gehen?

**Ja, unter Bedingungen.** Ich habe keinen Pfad gefunden, über den ein Besucher fremde Mittel ausgeben, einen Server-Schlüssel benutzen oder das Orakel dauerhaft (2 h) einfrieren kann. A17-1, A17-4 und A17-6 sind in der Sache behoben. A17-2 ist so weit behoben, dass ein Wallet-Journal Orakel und Keeper nur noch höchstens etwa 180 s sperrt (Ausnahme: Tx hängt im Mempool, siehe G-4).
Die Bedingungen:
1. **Vor dem Freischalten** am Live-Server prüfen, welche Adresse Caddy als Client sieht (G-1). Sonst teilen sich womöglich alle IPv6-Besucher (oder alle Besucher hinter einem CDN) ein gemeinsames Kontingent.
2. **Bald nachziehen:** G-2 (verfrühtes Verwerfen des Journals während des Wartens ohne Sperre) und G-3 (der eine Sende-Platz lässt sich kostenlos belegen). Beide schaden der Verfügbarkeit bzw. der Buchführung, nicht dem Geld.

## Befunde nach Schwere

| # | Schwere | Kurz |
|---|---|---|
| G-1 | mittel (Aufbau, Vermutung) | Hinter Docker-Port-Weiterleitung oder einem CDN ist der letzte X-Forwarded-For-Eintrag nicht die Besucheradresse. Dann teilen sich wieder viele Besucher ein Kontingent. |
| G-2 | mittel (Zustand, Vermutung zum Zeitfenster) | Schritt 3 von `resolve_pending` verwirft ein Wallet-Journal sofort. Seit A17-6 laufen andere Abgleiche (status, Agent) während des Wartens. Eine gerade angenommene Tx kann so als „nicht angenommen“ gelten, und ihre GHOST-Ausgänge fallen aus dem Zustand. |
| G-3 | mittel (Verfügbarkeit) | Ein einmal gültig signierter, veralteter Plan lässt sich beliebig oft wiederholen. `precheck` lässt ihn offline durch. Jeder Aufruf belegt den einzigen Sende-Platz und führt einen vollen `resync` aus (Node und REST). |
| G-4 | niedrig (Vermutung) | Für „noch im Mempool“ gibt es bei Wallet-Journalen keine Frist. Eine hängende Tx sperrt Orakel und Keeper unbegrenzt. |
| G-5 | niedrig | `tx_accepted` (REST, fremder Dienst) wird nicht gegen den Node geprüft. `is_twin` vergleicht weder Covenant-IDs noch Payload. |
| G-6 | niedrig | Wird ein Journal verworfen, obwohl die Tx angenommen wurde (Frist, „verdrängt“, G-2), gehen GHOST- und LP-Ausgänge dauerhaft aus dem Zustand verloren, weil `resync` keine Token neu entdeckt. |
| — | ok | A17-1, A17-4 (Sperre), A17-6 (Sperre beim Warten), A17-9 (CSP): siehe unten |

---

## 1. A17-1: Ersatzschlüssel

**Behoben.**
- Zufall: `Keypair::new(&Secp256k1::new(), &mut secp256k1::rand::thread_rng())` (protocol/src/wallet.rs:317). Cargo.toml:17 aktiviert `rand-std`. `thread_rng` ist ChaCha12, aus `OsRng` geseedet und regelmäßig neu geseedet, also kryptografisch unvorhersagbar. Der Test-Override wirkt nur thread-lokal und nur über `with_mirror_key` (wallet.rs:318–342).
- `measure` erzeugt den Schlüssel **einmal** und benutzt ihn durchgehend (wallet_ops.rs:429–430). Weitere Aufrufe von `mirror_key()` gibt es nur in Probe-Tresor bzw. Export (wallet.rs:360, 533, 622), dort jeweils lokal.
- Besetzen des neutralen Besitzers G/2G/3G: wirkungslos. `mirror_dep` (wallet_ops.rs:387ff.) setzt **nur** Objekte, die dem (jetzt zufälligen) Ersatzschlüssel gehören, auf `nobody`. Objekte, die G gehören, bleiben bei G und werden weder dem Nutzer noch der Messkopie zugerechnet. Ist der Besucher selbst G (privater Schlüssel 1), wählt `neutral_owner` 2G (wallet_ops.rs:371–379). Neue Lücke: keine gefunden.
- Einschränkung (Vermutung, kein Fehler): Die bitgleiche Plan-Prüfung in `submit` setzt voraus, dass kein Feld des Plans vom Ersatzschlüssel abhängt. Laut Test `einheiten_unabhaengig_vom_schluessel` gilt das. Bei festem Schlüssel war es trivial, jetzt hängt es an diesem Test.

## 2. A17-2: resolve_wallet

Ein Besucher **kann keinen fremden Folgezustand erzwingen**, der dauerhaft falsch bleibt. Im Einzelnen:

- **Zwilling** (store.rs:277–287): Er vergleicht die Menge der Eingänge sowie Skript und Betrag je Ausgang. Ein Zwilling, der nur Budget oder Speichermasse ändert, kann von jedem gebaut werden. Er hat dieselbe Wirkung, und `replace_txid` stimmt (Covenant-IDs leiten sich aus Eingangs-Outpoints ab, nicht aus der eigenen Tx-ID). Für alles andere (Payload, Covenant-Bindung der Ausgänge) braucht es eine neue Signatur. Die kann nur der Besucher selbst liefern. **G-5:** `is_twin` vergleicht `TxOut.cov` nicht, und das Journal speichert keine Covenant-IDs und keinen Payload. Ein Zwilling „gleiche Skripte, aber ohne Covenant-Bindung“ würde übernommen. `follow` (store.rs:387–390) prüft nur, ob der Outpoint existiert, nicht die Covenant. Ein solcher Zwilling ist aber nur annehmbar, wenn das Covenant-Skript seine Fortsetzung nicht erzwingt. Dann wäre der Covenant ohnehin für jeden zerstörbar, ganz ohne Zwilling. Für gemeinsame UTXOs (Orakel, Pool, Factory) halte ich das für ausgeschlossen (*Vermutung*, Skripte nicht geprüft). Abhilfe: Covenant-ID je Ausgang und Payload-Hash ins Journal schreiben und in `is_twin` vergleichen.
- **Gefälschte REST-Antwort:** `tx_accepted` (chain.rs:615) und `accepted_spender` (chain.rs:633) werden ungeprüft geglaubt. Mit `Some(true)` wird der Folgezustand übernommen (store.rs:303–306). Ein Besucher kann api.kaspa.org nicht beeinflussen. Eine kompromittierte oder fehlerhafte REST-API könnte aber einen nie angenommenen Folgezustand eintragen. Das heilt `resync` teilweise: Orakel und Factory über Skript und Adresse, Vaults über `chain_vault` mit Gegenprobe `confirm`, nicht vorhandene Token werden entfernt. Ein betroffener Vault bliebe `stale`. Niedrig.
- **Verdrängung durch eigene Doppelausgabe:** korrekt. Eine konkurrierende Tx, die nur die P2PK-Eingänge des Besuchers ausgibt, lässt die Covenant-Eingänge unverbraucht. Das führt zu Schritt 3 und zu „verworfen“. Gibt sie auch die Covenant-Eingänge aus, findet `accepted_spender` die fremde Tx (store.rs:327–336), und das Journal wird verworfen. Den Rest führt `resync` nach.
- **Blockadedauer:** Höchstens `WALLET_JOURNAL_GRACE` = 180 s (store.rs:160), gemessen an der mtime des Journals (store.rs:346), danach wird verworfen. Ein Besucher kann die Auskunft nur verzögern, z. B. mit mehr als 300 eigenen Tx, damit `accepted_spender` nichts findet (Seitenlimit chain.rs:633ff.). `tx_accepted` über die Tx-ID kann er aber nicht verfälschen. Wiederholen kostet je Runde eine echte Tx. Der Agent bekommt die Sperre in den Lücken. Eine 2-h-Blockade habe ich nicht konstruieren können. **Ausnahme G-4:** Schritt 2 (store.rs:183–189, „noch unterwegs“) hat für Wallet-Journale keine Frist. Solange die Tx im Mempool liegt, scheitert jeder Abgleich. Gezielt herbeiführen kann ein Besucher das nach meiner Einschätzung nicht, denn die Gebühr legt der Server fest. Bei Stau mit Mindestgebühr ist es aber denkbar, und per RPC eingereichte Tx verfallen im Mempool vermutlich nicht (*Vermutung* zum Node-Verhalten). Abhilfe: Für Wallet-Journale nach z. B. 10 min auch in Schritt 2 verwerfen (resync führt nach), oder Orakel und Keeper ein offenes **Wallet**-Journal überspringen lassen.
- **G-6:** Jede Verwerfung einer tatsächlich angenommenen Wallet-Tx (Frist, Fehlurteil aus G-2) verliert deren GHOST- und LP-Ausgänge dauerhaft aus `d.tokens`/`d.lp_tokens`. `resync` entfernt nur, entdeckt aber nichts neu (store.rs:682–691). Betroffen ist bei `transfer` auch der **Empfänger**: Seine GHOST sind auf der Kette, aber über die Seite nicht mehr nutzbar. Minter-Zweige mit geändertem Zustand werden `stale`. Abhilfe: Token-Entdeckung über REST bzw. die Adresse des Token-Covenants, oder vor jedem Verwerfen einer Wallet-Tx `tx_accepted` erneut fragen.

## 3. A17-3: X-Forwarded-For

Die Logik in `clientKey` (app/server/walletActions.ts:372–390) ist richtig. Sie liest von rechts, überspringt nur vertraute Proxys rechts vom ersten Eintrag (Zeile 381) und bricht bei Unsinn ab. IPv6 wird je /64 gezählt. Ein Besucher, der selbst `X-Forwarded-For` mitschickt, erreicht damit nichts:
- **Caddy v2 ohne `trusted_proxies`** (Standard): Caddy verwirft eingehende `X-Forwarded-*` von nicht vertrauten Clients und setzt `X-Forwarded-For` auf die Gegenstelle. Die Seite sieht genau einen Eintrag. Fälschen ist nicht möglich.
- **Caddy mit `trusted_proxies`** (z. B. `private_ranges`): Für einen nicht vertrauten Client verhält er sich ebenso. Er hängt nur bei vertrauter Gegenstelle an, dann steht rechts die echte Gegenstelle. Fälschen ist auch hier nicht möglich.

**G-1 (mittel, Vermutung, am Live-Server prüfen):** Entscheidend ist, was **Caddy als Gegenstelle sieht**:
- **IPv6 über Docker-Portweiterleitung:** Ist das Docker-Netz nur IPv4 (Standard) und `userland-proxy` aktiv (Standard), nimmt `docker-proxy` IPv6-Verbindungen an und verbindet sich selbst zum Container. Caddy sieht dann als Client das Gateway (172.18.0.1) und schreibt `X-Forwarded-For: 172.18.0.1`. In `clientKey` ist das der einzige Eintrag (i = 0, wird nicht übersprungen), also wird `172.18.0.1` zum Schlüssel. **Alle IPv6-Besucher teilen sich dann 20 Aufrufe pro Minute**, und einer kann sie alle aussperren. Für IPv4 bleibt die Quelladresse durch DNAT normalerweise erhalten.
- **CDN davor** (z. B. Cloudflare): Rechts steht immer eine Edge-Adresse des CDN. Alle Besucher teilen sich wenige Schlüssel, das ist der alte A17-3-Fall.
- Prüfen: im Caddy-Access-Log `remote_ip`/`client_ip` einer IPv6-Anfrage ansehen, `docker inspect` des Caddy-Containers (Port-Bindung, IPv6), DNS von k-lend.com (Proxy-Dienst?). Abhilfe je nach Ergebnis: IPv6 im Docker-Netz einschalten bzw. Caddy mit `network_mode: host`, oder bei einem CDN `trusted_proxies` in Caddy plus `header_up X-Forwarded-For {client_ip}`. Zusätzlich das in A17-3 empfohlene **globale** Wallet-Kontingent (bisher nicht umgesetzt, api.ts:323 zählt nur je Schlüssel).
- Am Rande: Vertraut wird das ganze /16 des Docker-Netzes (walletActions.ts:341ff.). Jeder Container des fremden Projekts in diesem Netz kann also beliebige Absender vortäuschen. Von außen ist das nicht erreichbar. Es ist nur dann relevant, wenn dort ein Container kompromittiert ist.

## 4. A17-4: precheck vor der Sperre

**Behoben, was die Sperre betrifft.** `precheck` (wallet_ops.rs, aufgerufen in ghostctl.rs:3951) prüft offline. Pläne für fremde Adressen und Müll-Signaturen kommen nicht mehr bis zur Sperre und nicht mehr bis zum Node. Zur Sperre (ghostctl.rs:4016) gelangt nur, wer einen **aktuellen**, vom Server baubaren Plan mit gültiger eigener Signatur schickt. Im Regelfall wird dann auch gesendet, und das kostet Gebühr. Ein Abbruch unter der Sperre, ohne zu senden, gelingt nur durch ein Wettrennen mit der eigenen Funding-UTXO zwischen den beiden Prüfungen. Auch das kostet eine Tx und hält die Sperre nur für `load_synced` (Sekunden). Den Agenten trifft das nicht nennenswert, denn zwischen zwei Versuchen liegen mehrere Sekunden ohne Sperre.

**G-3 (mittel, Verfügbarkeit):** Mit vielen gültig signierten Plänen für die eigene Adresse kann ein Angreifer die **Plätze** belegen:
- Ein Plan, der einmal gültig signiert war, bleibt für `precheck` gültig. Er lässt sich unbegrenzt wiederholen, auch nachdem die Funding-UTXO ausgegeben ist.
- Jeder Wiederholungsaufruf mit `send:true` belegt `sendsActive` (öffentlich **1**, api.ts:577) und einen der 2 `runWallet`-Plätze. Er führt `Net::connect`, `load_readonly` und damit einen vollen `store::resync` aus, mit Node-Abfragen je Vault und gegebenenfalls REST-Verfolgung (ghostctl.rs:3784–3794, 3994). Erst danach scheitert `check_funding_at_node`.
- Bei 20 pro Minute je Schlüssel und einigen Sekunden je Lauf hält eine einzige IPv4-Adresse den Sende-Platz einen großen Teil der Zeit besetzt, zwei bis drei Adressen (oder mehrere /64) praktisch ganz. Alle anderen Besucher bekommen dann 503 „ausgelastet“. Kosten für den Angreifer: keine. Status und Preis sind nicht betroffen (eigener Pool, gut).
- Abhilfe: `check_funding_at_node` **vor** `load_readonly` ausführen (eine Node-Abfrage statt resync). Hashes abgewiesener bzw. gesendeter Pläne einige Minuten merken und sofort ablehnen. Ein globales Kontingent für `send:true`.

## 5. A17-6: send_then_wait

**Behoben.** Die Sperre wird nach `write_pending` und `submit` freigegeben (store.rs:371–381, ghostctl.rs:887ff.). Ein zweites Senden auf veraltetem Stand ist nicht möglich: Jeder andere Abgleich unter Sperre (`load_synced`) trifft zuerst auf das Journal und scheitert, solange die Tx im Mempool liegt (Schritt 2), oder übernimmt den Folgezustand (Schritt 1). Er baut also nie auf dem alten Stand. Das Nachsperren des Senders (5 s) und ein bereits geklärtes Journal stören sich nicht, im zweiten Fall liefert `resolve_pending` `None`.

**G-2 (mittel, Zeitfenster vermutet):** Das neue Problem ist nicht das zweite Senden, sondern **Schritt 3** (store.rs:190–207). Sind irgendein Eingang noch sichtbar und die Tx nicht im Mempool, wird das Journal **sofort** verworfen. Beim eigenen Senden mit Schlüsseldatei hielt der Sender die Sperre, und `wait_accepted` toleriert genau dieses Bild bis zu 20 s (`WALLET_WAIT_BASE`, store.rs:697ff.). Jetzt laufen während des Wartens fremde `resolve_pending`: die öffentliche Statusabfrage (`ghostctl status` unter Sperre, alle 20 s, api.ts:304) und Agent-Runden.
- Fenster (*Vermutung*): Der Node nimmt eine Tx aus dem Mempool, sobald ein Block sie enthält. `exists` fragt den UTXO-Index nach Adresse (net.rs:218–219), und der wird asynchron nachgeführt. Auch die Annahme durch die Chain kommt erst mit dem nächsten Chain-Block. In diesem kurzen Fenster sieht ein fremder Abgleich: Ausgänge fehlen, Mempool sagt nein, Eingänge sind noch da. Er urteilt dann „nicht angenommen – verworfen“.
- Folge: G-6. GHOST-Ausgänge (auch die eines `transfer`-Empfängers) fehlen danach im Zustand, ein geänderter Minter-Zweig wird `stale`. Der Sender meldet trotzdem „bestätigt“ (ghostctl.rs:938ff. findet kein Journal mehr). Ein Angreifer gewinnt dadurch nichts, aber ehrliche Nutzer und Empfänger verlieren die Nutzbarkeit ihrer GHOST über die Seite.
- Abhilfe: Für `p.wallet` in Schritt 3 nicht sofort verwerfen, sondern erst wenn das Journal älter als z. B. 60 s ist **und** `tx_accepted` nicht `Some(true)` liefert. Davor `Err("noch unterwegs")`.

## 6. A17-9: CSP

**Bricht nach Quelltext und dem vorhandenen Build nichts.** Einschränkung: `app/dist` ist vom 29.09.2026 und damit älter als PWA und Wallet-Änderungen. Bauen durfte ich nicht. Geprüft habe ich deshalb `app/index.html`, `app/src`, `vite.config.ts` und den alten Build.
- Keine Inline-Skripte: `dist/index.html` lädt nur `./assets/index-*.js` und `*.css`. `index.html` hat nur `<script type="module" src>`. Das passt zu `script-src 'self'` (prod.ts:129).
- Kein `eval`, kein `new Function`, kein `<style>`-Einfügen, kein `insertRule`, kein `cssText`, kein `setAttribute('style')` im Bundle (gezählt: 0). React-`style={{}}` setzt Werte über das CSSOM, `style-src 'self'` erlaubt das.
- Schriften: `@fontsource` Rubik als woff2 unter `assets/`, kein `data:font` im CSS. Das passt zu `font-src 'self'`.
- `data:` kommt im Bundle nur als QR-Code (`data:image/gif;base64`) vor. Das passt zu `img-src 'self' data:`.
- `fetch` nur an `./api/…` (src/lib/*.ts, PriceChart.tsx:57). Das passt zu `connect-src 'self'`. Explorer-Links sind Navigation, keine Abrufe.
- `sw.js` liefert offline eine Seite mit Inline-`style`. Diese synthetische Antwort trägt keinen CSP-Kopf, also kein Problem. `worker-src`/`manifest-src 'self'` passen.
- Wallets (*Vermutung*, Erweiterungen nicht geprüft): KasWare und Kastle laufen als Content-Scripts bzw. per `<script src=chrome-extension://…>`. Das ist mit `chrome-extension:`/`moz-extension:` erlaubt. Würde eine Wallet ihren Seiten-Provider als **Inline**-Skript einfügen oder aus der Seitenwelt fremde Hosts abrufen, bräche das. Empfehlung: nach dem Ausrollen einmal mit beiden Wallets signieren und die Konsole auf „Refused to …“ prüfen.
- Nebenbei: `src/probe/main.ts:53` setzt Attribute per `setAttribute`. Ein `style` dort würde blockiert, das betrifft aber nur die lokale Probe-Seite.

## Nicht geprüft / Grenzen

Rust-Tests nicht ausgeführt (Vorgabe). Covenant-Skripte nicht darauf geprüft, ob sie ihre Fortsetzung erzwingen (G-5). Node-Interna (Mempool-Verfall, Reihenfolge Mempool/UTXO-Index) nur aus der Erinnerung, daher als Vermutung gekennzeichnet. Live-Aufbau (Caddyfile, Docker, DNS) nicht eingesehen.
