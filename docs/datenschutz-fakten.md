# Datenverarbeitung auf k-lend.com – Faktenliste für die Datenschutzerklärung

Stand: 06.10.2026, Code-Stand Branch nach Audit 20 (A20d-6). **Kein Rechtstext.** Diese Liste beschreibt sachlich, was die öffentliche Seite technisch verarbeitet, jeweils mit Fundstelle im Code. Sie ist Vorlage für die Kanzlei; Rechtsgrundlagen, Formulierungen und Bewertungen trifft die Kanzlei. Punkte mit **[am Server prüfen]** lassen sich aus dem Repository allein nicht belegen.

Betrieb: öffentlicher Modus `app/server/prod.ts` (immer `public: true`), Webserver Caddy davor, systemd-Dienste `ghost-web` (Seite) und `ghost-agent` (Agent), Vorlagen in `deploy/hetzner/templates/`.

## 1. Hosting

| Was | Fakt | Beleg |
|---|---|---|
| Hoster | Hetzner Cloud, ein Server (Ubuntu), IP 162.55.185.54; Seite und GHOST-Agent laufen dort | `deploy/hetzner/README.md`, `deploy/hetzner/haertung.sh` |
| Webserver | Caddy im Docker-Container des Projekts „Prüflotse“ auf demselben Server terminiert TLS und leitet an `ghost-web` weiter (`reverse_proxy`) | `deploy/hetzner/haertung.sh` (Caddyfile `/opt/prueflotse/Caddyfile`), `deploy/hetzner/README.md` Abschnitt 9 D |
| TLS-Zertifikat | Let's Encrypt (Caddy holt es selbst) | `deploy/hetzner/templates/ghost.caddy`; Audit 20c „TLS“ |
| Zugriffsprotokoll des Webservers | Ob und wie lange Caddy Zugriffe (IP, Pfad, User-Agent, Zeit) protokolliert, steht in der Caddyfile von Prüflotse, nicht in diesem Repository. **[am Server prüfen: `log`-Direktive in /opt/prueflotse/Caddyfile, Docker-Log-Treiber und -Aufbewahrung]** | – |

## 2. Daten, die der Browser an den Server schickt

| Was | Wozu | Wohin / wie lange | Beleg |
|---|---|---|---|
| IP-Adresse (jede Anfrage) | Auslieferung; bei `/api/wallet/…` Ratenbegrenzung je Absender | Nur im Arbeitsspeicher von `ghost-web`: Zeitstempel der Aufrufe je Absender, gleitendes Fenster 60 s, höchstens 20 000 Absender gemerkt (älteste fallen heraus); IPv6 gekürzt auf /64. Nicht auf Platte, nicht ins Log. Weg bei Neustart. Dazu kurz die Zahl laufender Aufrufe je Absender (nur während der Anfrage) | `app/server/walletActions.ts` `createRateLimiter`, `clientKey`, `ipKey`; `app/server/api.ts` `walletLimiter`, `buildLimiter`, `nameLimiter`, `walletActive` |
| IP über X-Forwarded-For | Caddy setzt den Kopf; die Seite liest daraus den Absender für die Ratenbegrenzung | wie oben | `walletActions.ts` `clientKey`, `trustedProxies` |
| Kaspa-Adresse der verbundenen Wallet | Plan für eine Aktion bauen (`POST /api/wallet/build`), „Meine Tresore“ (`GET /api/wallet/tresore?owner=…`), GHOST-Eingang suchen (`POST /api/wallet/receive`) | an `ghostctl` (Prozess auf dem Server) als Argument; ghostctl fragt damit öffentliche Kaspa-Nodes nach UTXOs (siehe 4). Die Seite speichert die Adresse nicht; Ausnahmen: Tresore und Journal (siehe 3) | `app/server/walletActions.ts` `buildWalletBuildArgs`, `buildWalletTresoreArgs`, `buildWalletReceiveArgs` |
| Parameter einer Aktion (Beträge, Empfängeradresse, Vault-ID, Nachricht) | Plan bauen | an ghostctl; nicht gespeichert, außer Tresor-Anlage (siehe 3) und Journal | wie oben, `WALLET_PARAMS` |
| Signierplan und Signatur der Wallet | Prüfen und auf ausdrücklichen Knopfdruck senden (`POST /api/wallet/submit`) | als Dateien in ein frisches Temp-Verzeichnis (Rechte 0600, systemd `PrivateTmp`), nach der Anfrage gelöscht. Abgewiesene Pläne: nur SHA-256-Hash von Plan+Signatur 5 Minuten im Arbeitsspeicher (höchstens 5 000) | `app/server/api.ts` (`mkdtempSync`, `rmSync`, `rejectedPlans`, `planKey`) |
| eingegebener .k-Name | Name → Kaspa-Adresse auflösen (`GET /api/wallet/name?name=…`) | Server fragt `api.dotk.name` (mit der IP des Servers, nicht des Besuchers) und prüft die Adresse am Kaspa-Node; nicht gespeichert. Der Name steht in der URL (GET-Parameter) und damit ggf. im Zugriffsprotokoll des Webservers **[am Server prüfen]**. Ausgelöst erst ab 3 Zeichen, nicht bei Adressanfängen („kaspa…“), 700 ms nach dem letzten Tastendruck | `app/server/dotkNames.ts`, `app/src/lib/kname.ts`, `app/server/api.ts` Route `GET /api/wallet/name` |
| Netzwahl, Kursverlauf-Zeitraum | Status/Kurse laden (`/api/status`, `/api/price`, `/api/history?days=…`) | keine personenbezogenen Daten; Antworten werden für alle Besucher gemeinsam zwischengespeichert | `app/server/api.ts` `cached`, `app/server/history.ts` |

## 3. Daten, die auf dem Server dauerhaft liegen (Ordner `deployments/`)

| Was | Wozu | Wie lange | Beleg |
|---|---|---|---|
| Über die Seite angelegte Tresore (Daueraufträge) in `deployments/mainnet-tresore.json`: Besitzer-Adresse (x-only-Schlüssel), Empfänger-Adresse, Betrag, Rhythmus, Termine, Anzahl, Höchstgebühr, öffentliche Nachricht, Zahlungsverlauf (je Tresor höchstens 50 Einträge) | Der GHOST-Agent löst die fälligen Zahlungen aus; „Meine Tresore“ zeigt sie dem Besitzer | Kein festes Löschdatum. Beendete oder seit über einer Woche verschwundene Wallet-Tresore werden erst entfernt, wenn die Datei voll ist (1 000 Einträge). Höchstens 10 laufende je Adresse. Die Daten stehen außerdem dauerhaft öffentlich auf der Kaspa-Blockchain (Vertrag, Nachricht in jeder Zahlung) | `protocol/src/tresor.rs` `TresorRec`, `MAX_HISTORY`, `evictable`, `make_room_for_wallet`, `MAX_FILE_TRESORE`, `MAX_WALLET_PER_OWNER`; `app/server/walletActions.ts` (Nachricht nur öffentlich) |
| Journal einer gesendeten Transaktion (`deployments/<netz>.pending.json` bzw. zur Tresor-Datei): Tx-ID, Ausgänge (Skripte/Adressen, Beträge), Eingänge | Doppelsenden verhindern; klären, ob eine Tx angenommen wurde | bis zur Klärung (Sekunden bis wenige Minuten), dann gelöscht | `protocol/src/store.rs` `write_pending`, `clear_pending` |
| Zustand des Protokolls (`deployments/mainnet.json`): Vaults mit Besitzer-Schlüssel, GHOST-Token mit Besitzer-Schlüssel | Betrieb des Protokolls | dauerhaft; dieselben Daten stehen öffentlich auf der Blockchain. `GET /api/status` gibt Vaults und Token mit Besitzer-Schlüssel öffentlich aus | `protocol/src/bin/ghostctl.rs` Status-JSON (`owner`, `tokens`) |
| Verlauf gesendeter Transaktionen des Servers (`deployments/*-txlog.jsonl`) | nur für Aktionen mit Schlüsseldateien (lokaler Modus bzw. Agent), **nicht** für Besucher-Transaktionen über die Wallet | – | `protocol/src/bin/ghostctl.rs` `note_sent` (nur Schlüssel-Aktionen) |

## 4. Weitergabe an Dritte durch den Server (der Besucher hat keine direkte Verbindung)

| Empfänger | Was | Beleg |
|---|---|---|
| Öffentliche Kaspa-Nodes (wRPC, über den Resolver von rusty-kaspa) | Kaspa-Adressen der Besucher bei UTXO-Abfragen; signierte Transaktionen beim Senden (werden ohnehin öffentlich) | `protocol/src/net.rs` (`Resolver`) |
| REST-API `api.kaspa.org` | Tx-IDs/Outpoints zur Klärung, ob eine Tx angenommen wurde; Adressen bei Abfragen von Transaktionen | `protocol/src/chain.rs`, `protocol/src/store.rs` `JournalNet` |
| `api.dotk.name` | eingegebene .k-Namen | `app/server/dotkNames.ts` (`@dotk/sdk` 2.1.0, Zeitlimit 15 s) |
| Kursquellen (CoinGecko, Kraken, Bybit, Gate.io, KuCoin, MEXC, api.kaspa.org) | keine Besucherdaten; nur Kursabfragen des Servers | `app/server/history.ts`, `protocol/src/price.rs` |

## 5. Daten nur im Browser des Besuchers (localStorage, nicht übertragen)

| Schlüssel | Inhalt | Beleg |
|---|---|---|
| `ghost.txlog.v1` | Verlauf der über diese Seite gesendeten Transaktionen (höchstens 200): Zeit, Netz, Aktion, Betrag, Empfänger, Tx-IDs, ggf. Nachricht – auch der Klartext verschlüsselt gesendeter Nachrichten | `app/src/lib/txlog.ts` |
| `kl-demo-wallet` | welche Wallet-Erweiterung zuletzt verbunden war (KasWare/Kastle) | `app/src/wallet/WalletContext.tsx` |
| `gh-lang` | Sprache | `app/src/lib/i18n.tsx` |
| `gh-network` | gewähltes Netz | `app/src/lib/StatusContext.tsx` |
| `gh-key-<netz>` | gewählte Schlüsseldatei (nur lokaler Modus; öffentlich gibt es keine) | `app/src/lib/AccountContext.tsx` |
| `klend-install-hint-weg` | Hinweis „als App installieren“ geschlossen | `app/src/components/InstallHint.tsx` |

Keine Cookies, kein `sessionStorage`, keine IndexedDB (grep über `app/src`). Der Service Worker (`app/public/sw.js`) speichert nichts zwischen.

## 6. Wallet-Erweiterungen und Links

- Die Seite spricht die Browser-Erweiterungen KasWare bzw. Kastle im Browser an: liest Adresse, Netz, Guthaben und öffentlichen Schlüssel; lässt Transaktionen signieren. Die Erweiterung selbst ist ein Angebot Dritter. Beleg: `app/src/wallet/providers.ts`, `app/src/wallet/WalletContext.tsx`.
- Links zu `explorer.kaspa.org` / `explorer-tn10.kaspa.org` (Adresse bzw. Tx-ID im Pfad), `kasware.xyz`, `kastle.cc`; nur bei Klick, mit `rel="noopener noreferrer"` und `Referrer-Policy: no-referrer`. Beleg: Audit 20d „XSS/Injection/Datenschutz“, `app/server/prod.ts`.

## 7. Was es nicht gibt

- Keine Analyse-/Tracking-Werkzeuge, keine Werbung, keine eingebundenen Inhalte Dritter: Content-Security-Policy erlaubt Skripte, Styles, Schriften, Bilder und Verbindungen nur von der eigenen Domain (`connect-src 'self'`). Die Schrift (Rubik) wird aus `@fontsource-variable/rubik` mitgebaut, nicht von Google geladen. Beleg: `app/server/prod.ts` `CONTENT_SECURITY_POLICY`, `app/package.json`.
- Keine Benutzerkonten, keine Anmeldung, keine E-Mail-Adressen, keine privaten Schlüssel der Besucher (Signieren nur in der Wallet).
- Permissions-Policy sperrt Kamera, Mikrofon, Standort, Zahlungs-API und Sensoren (`app/server/prod.ts` `PERMISSIONS_POLICY`).

## 8. Server-Logs (journald)

- `ghost-web` schreibt nach journald nur Startmeldungen und bei internen Fehlern Route und Stacktrace (`console.error("[ghost-api] <Route>: …")`); keine IP-Adressen, keine Anfrageinhalte. Beleg: `app/server/api.ts`, `app/server/prod.ts`.
- `ghost-agent` (ghostctl) protokolliert Orakel-Updates, Liquidationen und Tresor-Zahlungen, dabei Tresor-IDs, Beträge und Tx-IDs. **[am Server prüfen: Aufbewahrung von journald (`/etc/systemd/journald.conf`, `SystemMaxUse`/`MaxRetentionSec`)]**
- Sicherheits-Kontakt: `/.well-known/security.txt` (info@k-lend.com).

## Offene Punkte für die Kanzlei

1. Zugriffsprotokoll von Caddy (Prüflotse-Container) und journald-Aufbewahrung am Server feststellen.
2. Aufbewahrung der Tresor-Daten: heute kein festes Löschdatum (siehe 3).
3. Veröffentlichung der Besitzer-Schlüssel von Vaults/Token in `/api/status` (Kettendaten, Audit 20 A20c-7).
