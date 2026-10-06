# Audit 20 c: Webserver, API und Betrieb (k-lend.com)

Prüfer: Claude Fable 5.1, unabhängig, nur lesend. Datum: 06.10.2026, Stand `main` bei fd4e28e.
Geprüft: `app/server/{prod,api,actions,walletActions,walletProbe,dotkNames,history}.ts`, `deploy/hetzner/{setup.sh,push-from-mac.sh,keys-upload.sh,haertung.sh,templates/*,README.md}`, `deploy/github-sync.sh`, `app/public/sw.js`, `app/index.html`, `app/package.json` samt Lockfile, dazu `ghostctl.rs` nur an den Stellen, die die Web-Routen aufrufen. Hintergrund: Audit 17, 18, 19 (dort Gemeldetes wird hier nicht wiederholt, nur nachgeprüft).

Black-Box von außen: etwa 110 Anfragen an https://k-lend.com über rund 20 Minuten, höchstens 2 je Sekunde; nur lesende Routen, ungültige Eingaben, kein Senden, nichts, was KAS bewegen kann. Sieben einzelne TCP-Verbindungsversuche auf 162.55.185.54 (Ports 22, 8787, 2019, 16110, 16111, 5432, 3000). Öffentliches GitHub-Repo über die GitHub-API gelesen.

**Nicht geprüft:** Der lesende SSH-Zugriff auf den Server (`systemctl cat/show`, `ls -l keys/`, `sshd -T`, `ufw status`, `docker ps`, Caddyfile) wurde vom Berechtigungssystem der Prüfumgebung verweigert („Production Reads“). Alles zu systemd, Docker, Firewall und Caddy stützt sich deshalb auf die Vorlagen im Repo, das README und das, was von außen sichtbar ist. Wo das eine Lücke lässt, steht es dabei. `npm audit` lieferte in der Prüfumgebung keine Antwort (kein Registry-Zugang).

Es wurde keine Datei im Projekt geändert; Testdateien lagen nur im Arbeitsverzeichnis der Prüfumgebung und sind entfernt.

## Kurzfassung

| # | Schwere | Befund |
|---|---|---|
| A20c-1 | **mittel** (Verfügbarkeit, mit einer Adresse auslösbar) | `POST /api/wallet/build` belegt je Aufruf 11–13 s einen der zwei Wallet-Plätze, **bevor** ghostctl die Adresse prüft. Eine einzige IPv4-Adresse (20 Aufrufe je Minute) kann beide Plätze dauerhaft belegen; dann scheitern build, submit und „Meine Tresore“ für alle. Gemessen. |
| A20c-2 | mittel (Betrieb) | `setup.sh` läuft als root, liegt aber samt systemd-Vorlagen in einem Verzeichnis, das dem Dienstbenutzer `ghost` gehört. `npm ci` und `cargo build` laufen als `ghost` mit Install-/Build-Skripten. Eine kompromittierte Abhängigkeit wird so beim nächsten `setup.sh` zu root. |
| A20c-3 | niedrig | Seite und Agent laufen als derselbe Benutzer. Eine übernommene Seite kann den Agenten mit Signalen anhalten (Orakel friert nach 2 h ein) und dessen Zustandsdateien verändern. Ob `ptrace_scope` das Auslesen des Agent-Speichers (Schlüssel) verhindert, konnte am Server nicht geprüft werden. |
| A20c-4 | niedrig | A19-4 ist nur zur Hälfte umgesetzt: `ghostctl utxos`/`receive` nehmen keine Sperre mehr, laufen aber weiter im `run`-Pool von status/price. Ein voller Pool liefert für status „ausgelastet“, und dieser Fehler wird 20 s lang wie ein Ergebnis zwischengespeichert. |
| A20c-5 | niedrig | `haertung.sh` schreibt die Caddyfile des anderen Projekts, **bevor** es sie prüft; bei Fehler bleibt die ungültige Datei liegen und beide Seiten starten nach dem nächsten Container-Neustart nicht mehr. Fester Dateiname unter /tmp als root. |
| A20c-6 | niedrig | GitHub-Sync arbeitet mit einer Sperrliste, nicht mit einer Freigabeliste. `deployments/mainnet.json` und `deployments/*-tresore.json` sind nicht in `.gitignore`; ein `git add .` brächte Besucher-Tresore (Empfänger, Nachrichten) in das öffentliche Repo. Heute ist das Repo sauber. |
| A20c-7 | Info | Kopfzeilen und DNS: kein `Permissions-Policy`, kein `Cross-Origin-Resource-Policy`, CSP ohne Meldeweg, kein CAA-Eintrag, kein `security.txt`/`robots.txt`; `via: 1.1 Caddy`. Status-JSON nennt die GHOST-Token des Betreibers (Kettendaten). |

**Keine** Befunde bei: Zugriff auf Schlüsseldateien über die Seite, Argument-Injektion in ghostctl, gesperrten Routen im öffentlichen Modus, CSRF/Origin/Host-Prüfung, X-Forwarded-For-Fälschung, Pfadtricks gegen den Dateiserver, Fehlermeldungen mit Interna, Größenbegrenzung, TLS, Service Worker und Versionswechsel, npm-Lockfile, Schlüssel-Upload, Zustands-Upload (Einzelheiten unten unter „Geprüft und sauber“).

---

## A20c-1 (mittel): Wallet-Bau kostet 11–13 s je Aufruf, Adresse wird erst danach geprüft

**Ort:** `protocol/src/bin/ghostctl.rs:4084–4088` (`wallet_action_cmd`): `Net::connect(network, rpc)` steht **vor** `xonly_of_address(address, prefix)` und vor `Address::try_from`. `app/server/api.ts:631–650`: `build` geht nach Ratenbegrenzung und Regex-Prüfung (`checkProbeAddress`, nur Zeichensatz und Länge, keine bech32-Prüfsumme) sofort in `runWallet` (2 Plätze, Warteschlange 20, Zeitlimit 60 s). Dieselben zwei Plätze nutzen `submit` und `GET /api/wallet/tresore`.

**Szenario:** Ein Angreifer schickt `{"action":"send","address":"kaspa:q"+60×"q","params":{"to":…,"kas":"1"}}`. Die Adresse besteht die Regex, hat aber keine gültige Prüfsumme. ghostctl verbindet sich zuerst mit dem öffentlichen Node (das dauert hier 11–13 s), lehnt dann die Adresse ab. Mit 20 Aufrufen je Minute und Adresse ergibt das 220–260 Platz-Sekunden je Minute; zwei Plätze bieten 120. **Eine** IPv4-Adresse reicht also, um beide Wallet-Plätze dauernd zu belegen. Alle anderen Besucher warten in der Schlange (bis 20) und laufen in das Zeitlimit, oder sie bekommen „Server ausgelastet“. Kosten für den Angreifer: keine, kein Guthaben, keine gültige Adresse nötig. Status, Preis und der Agent sind nicht betroffen (eigener Pool, `wallet build` nimmt keine Sperre).

**Beleg (von außen gemessen, drei Aufrufe nacheinander):**
```
POST /api/wallet/build  send   (Adresse ohne gültige Prüfsumme)  200  13,05 s  {"error":"keine gültige Kaspa-Adresse","ok":false,…}
POST /api/wallet/build  send   (dito, Nachricht „$(id)`id`;id“)    200  11,09 s  {"error":"keine gültige Kaspa-Adresse",…}
POST /api/wallet/build  tresor-cancel (dito)                       200  11,09 s  {"error":"keine gültige Kaspa-Adresse",…}
GET  /api/wallet/tresore?owner=<dito>                               200   0,06 s  {"error":"Besitzer: keine gültige Kaspa-Adresse",…}
```
Zum Vergleich: Eingaben, die schon die Seite ablehnt (Regex), antworten in 0,04 s. Warum der Verbindungsaufbau 11 s dauert, ist nicht gemessen (Vermutung: Resolver bzw. wRPC-Handshake zum öffentlichen Node bei jedem Prozessstart).

**Vorschlag:**
1. In ghostctl die Adresse (und bei `build` die übrigen Argumente) **vor** `Net::connect` prüfen; bei `submit` ist das für Plan und Signatur mit `precheck` schon so (A17-4), die Adresse im Plan geht dort ebenfalls erst nach dem Verbinden an den Node.
2. In `walletActions.ts` die bech32-Prüfsumme der Kaspa-Adresse prüfen (CashAddr-Polymod, etwa 30 Zeilen; Testvektoren aus `kaspa_addresses`), damit nichts Ungültiges einen ghostctl-Start kostet. Regressionstest: Adresse mit falscher Prüfsumme → 400 ohne ghostctl.
3. Zusätzlich ein **globales** Kontingent für `build` (z. B. 30 je Minute, wie `sendLimiter` für Sendungen) und ein engeres je Absender (z. B. 6 build je Minute; die Seite braucht je Aktion einen Bau). Auch mit gültigen Adressen bleibt sonst ein Aufruf 11 s teuer, und drei Adressen genügen.
4. Längerfristig den Verbindungsaufbau aus dem Pfad nehmen (fester RPC-Endpunkt statt Resolver, oder ein dauerhaft verbundener Dienst statt eines Prozesses je Anfrage).

## A20c-2 (mittel, Betrieb): root führt Skripte und Vorlagen aus dem Verzeichnis des Dienstbenutzers aus

**Ort:** `deploy/hetzner/setup.sh:195` (`chown -R ghost:ghost "$GHOST_DIR"`), `:201–202` (Vorlagen aus `$GHOST_DIR/deploy/hetzner/templates`), `:234–235` (`render … > /etc/systemd/system/ghost-web.service` bzw. `ghost-agent.service`), `:222` (`npm ci` als ghost, mit Install-Skripten), `:211–214` (`cargo build` als ghost, build.rs/proc-macros der Abhängigkeiten); `push-from-mac.sh:44` (`chown -R ghost:ghost`). Aufruf laut Kopf: `bash /opt/ghost/kaspa-lending/deploy/hetzner/setup.sh` als root.

**Szenario:** Ein Paket in der npm- oder cargo-Lieferkette bringt ein Install- oder Build-Skript mit. Es läuft als `ghost` ohne systemd-Sandbox und schreibt `deploy/hetzner/templates/ghost-web.service` (z. B. `User=root`, anderes `ExecStart`) oder `setup.sh` selbst. Beim nächsten `setup.sh` (nach jedem Code-Update) installiert root die Vorlage bzw. führt das Skript aus. Die Dienste selbst können das nicht (`ProtectSystem=strict`, nur `deployments/` beschreibbar), der Weg führt über den Bau. Das heutige Lockfile ist unauffällig (nur `fsevents` hat ein Install-Skript, macOS-only), die Zahl der Pakete ist klein (95). Ein Ausbruch bräuchte also eine neue bösartige Version einer Abhängigkeit.

**Vorschlag:** `npm ci --ignore-scripts` (die Seite braucht keine Install-Skripte; `vite build` läuft trotzdem). `setup.sh` und `templates/` root-eigen halten: entweder `chown root:ghost` mit 750 für `deploy/` (dann `chown -R` in setup.sh und push-from-mac.sh auf `app/`, `protocol/`, `vendor/`, `deployments/`, `keys/` beschränken) oder setup.sh kopiert sich und die Vorlagen beim ersten Lauf nach `/root/ghost-deploy/` und nimmt sie nur von dort. Zusätzlich vor dem Rendern einen Vergleich der Vorlagen mit dem Stand im Git (`git diff --quiet HEAD -- deploy/hetzner/templates` auf dem Mac vor dem Push).

## A20c-3 (niedrig): Seite und Agent teilen sich den Benutzer `ghost`

**Ort:** `templates/ghost-web.service` und `ghost-agent.service` (`User=ghost`), `setup.sh:197–198` (`keys/` 700, `deployments/` 750, beides ghost).

**Szenario:** Wer Code auf der Seite ausführen kann (Node, ghostctl im Wallet-Pfad), ist `ghost`. `InaccessiblePaths=keys` hält ihn von den Schlüsseldateien fern, nicht aber vom Agenten-Prozess desselben Benutzers: `kill -STOP` hält den Agenten an (keine Orakel-Updates, nach 2 h friert das Orakel ein, keine Liquidationen), `kill` beendet ihn (systemd startet neu, höchstens 10-mal in 10 Minuten, dann bleibt er aus). `deployments/` ist für beide beschreibbar, der Zustand des Agenten lässt sich also verfälschen (was ghostctl daraus macht, ist Thema von 20 a/b). Mit `kernel.yama.ptrace_scope=0` ließe sich der Speicher des Agenten lesen, in dem Keeper- und Unterzeichner-Schlüssel liegen; Ubuntu setzt standardmäßig 1, am Server **nicht geprüft** (SSH verweigert).

**Vorschlag:** Eigener Benutzer `ghost-web` mit Gruppe `ghost`; `deployments/` 770 (`g+s`), `keys/` 700 nur `ghost`. In beiden Units ergänzen: `ProtectProc=invisible`, `ProcSubset=pid`, `SystemCallFilter=@system-service`, `SystemCallErrorNumber=EPERM`; `kernel.yama.ptrace_scope` ≥ 1 einmal prüfen (`sysctl kernel.yama.ptrace_scope`). Nur für die Seite zusätzlich `RestrictAddressFamilies=AF_INET AF_INET6` (AF_UNIX braucht Node nicht).

## A20c-4 (niedrig): Namens- und Suchroute weiter im Pool von status/price

**Ort:** `api.ts:580` (`/api/wallet/name` → `run(…utxos…)`), `api.ts:600` (`/api/wallet/receive` → `run`), `api.ts:302–311` (`readJson` liefert bei vollem Pool `BUSY_RESULT` als Fehlertext, `cached()` speichert ihn 20 s), A19-4 im Bericht 19 als „behoben in 850b8ac“ geführt. 850b8ac hat in ghostctl die Sperre für `utxos` und `receive --owner` entfernt (geprüft, Diff gelesen), die Routen aber nicht aus dem `run`-Pool genommen.

**Szenario:** Sind die zwei `run`-Plätze und die Warteschlange (20) mit Namensauflösungen belegt, bekommt der nächste Status-Abruf „Server ausgelastet“, und das wird 20 s lang allen Besuchern gezeigt. Für die Warteschlange braucht es etwa 22 gleichzeitige Aufrufe; bei 0,2 s je Auflösung eines unbekannten Namens (gemessen) und 20 je Minute je Adresse ist das mit normalen Mitteln schwer, mit registrierten Namen (Node-Abfrage, `ghostctl utxos` startet und verbindet sich) deutlich leichter – nicht gemessen. Gegenüber A20c-1 ist das zweitrangig.

**Vorschlag:** `name` und `receive` auf `runWallet` oder einen dritten Pool legen; `BUSY_RESULT` in `readJson` nicht zwischenspeichern (nur echte Ergebnisse und Node-Ausfälle).

## A20c-5 (niedrig): haertung.sh ändert fremde Konfiguration vor der Prüfung

**Ort:** `deploy/hetzner/haertung.sh:23–28`: `sed … > /tmp/Caddyfile.neu`, dann `cat /tmp/Caddyfile.neu > "$F"`, **dann** `docker exec … caddy validate`. Mit `set -e` endet das Skript beim Validierungsfehler, die geänderte Datei bleibt. Der laufende Container merkt davon nichts, bis er neu startet, dann startet Caddy nicht mehr, und Prüflotse **und** K.Lend sind weg. Außerdem fester Name unter `/tmp` als root (bei `fs.protected_symlinks=1`, Ubuntu-Standard, harmlos).

**Vorschlag:** Neue Fassung in `mktemp` schreiben, mit `docker cp` bzw. `docker exec -i caddy validate --config /dev/stdin --adapter caddyfile < neu` prüfen, erst dann in `$F` schreiben, danach `caddy reload`. Das Skript ist einmal gelaufen (HSTS `includeSubDomains` ist live sichtbar), der Befund betrifft künftige Härtungen nach demselben Muster.

## A20c-6 (niedrig): GitHub-Sync mit Sperrliste; Zustands- und Tresordateien nicht ignoriert

**Ort:** `deploy/github-sync.sh:8` (`AUSLASSEN`), `:25` (Abbruch nur bei `^keys/|RECHT_PRUEFUNG|ghostctl-v1`), `.gitignore` (ignoriert `*-abos.json`, `*-txlog.jsonl`, `*-zins.json`, **nicht** `deployments/mainnet.json`, `deployments/*-tresore.json`, `deployments/mainnet-v2.json` …). `git status` zeigt `deployments/mainnet.json` und weitere als untracked, nicht als ignoriert.

**Szenario:** Ein `git add .` nimmt `deployments/mainnet-tresore.json` auf, sobald der Betreiber einen Tresor lokal anlegt oder die Datei vom Server zurückholt. Sie enthält Empfänger, Beträge, Termine und Nachrichten der über die Seite angelegten Besucher-Tresore (nach Audit 19 nur öffentliche Nachrichten, dennoch personenbezogen). Der Sync schiebt alles Getrackte ins öffentliche Repo, die Sperrliste kennt diese Datei nicht. Geprüft wurde, dass das Repo heute sauber ist: keine `keys/`, in `deployments/` nur `mainnet-v1.json` und `testnet-10-v1.json` (Kettendaten), Stand identisch mit lokal (letzter Commit 10:36 UTC). Auch getrackt und öffentlich, harmlos: `audit/1-…testlog.txt` und `audit/3-…lauf.txt` mit dem lokalen Mac-Pfad `/Users/peterpan/…`, `prod.test.ts` und `haertung.sh` mit der Server-IP (steht ohnehin im DNS).

**Vorschlag:** `.gitignore` um `deployments/*.json` mit Ausnahmen (`!deployments/*-v1.json`) ergänzen; im Sync-Skript eine Freigabeliste (`git ls-files | grep -vE '^(app|protocol|contracts|docs|deploy|audit|tests|vendor)/|^[A-Z_]+\.md$' → Abbruch, wenn etwas übrig bleibt) oder wenigstens `deployments/` außer den v1-Dateien in `AUSLASSEN`.

## A20c-7 (Info): Kopfzeilen, DNS, Sichtbares

- Vorhanden und richtig: HSTS mit `includeSubDomains`, CSP wie in prod.ts, `X-Frame-Options: DENY`, `nosniff`, `Referrer-Policy: no-referrer`, COOP, `Server` entfernt, HTTP → HTTPS 308, TLS 1.2 und 1.3 (1.0/1.1 abgelehnt), Let's-Encrypt-Zertifikat nur für `k-lend.com` (kein `www`, kein AAAA).
- Fehlend, Empfehlung: `Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=()`; `Cross-Origin-Resource-Policy: same-origin`; `report-to`/`report-uri` für die CSP, um Wallet-Konflikte zu sehen (A18 Abschnitt 6); DNS-CAA (`0 issue "letsencrypt.org"`); `/.well-known/security.txt` mit Kontakt; `robots.txt`. `via: 1.1 Caddy` verrät den Proxy (unkritisch).
- `GET /api/status` nennt unter `tokens[]` die GHOST-Token mit Besitzer-Schlüssel, darunter die des Betreibers (Keeper). Das sind Kettendaten, aber es verknüpft „Betreiber“ mit einem Schlüssel. Wenn das nicht gewollt ist, `tokens` öffentlich weglassen.
- `index.html` lädt per `modulepreload` auch `assets/probe-*.js` (Code der lokalen Wallet-Probe). Harmlos, nur Code; die Seite `/wallet-probe.html` selbst ist gesperrt (404 geprüft, auch mit `%2E`, `./`, `x/../`).

---

## Geprüft und sauber

**Schlüssel und Dateien.** Öffentlicher Modus ist in `prod.ts` fest (`public: true`), `publicRouteAllowed` lässt nur status/price/history/keys und `/api/wallet/…` durch; `keys` antwortet ohne ghostctl. Von außen bestätigt: `POST /api/action|keygen|receive|wallet-probe`, `GET /api/abos|tresore|messages?key=keys/…` → 403 „gesperrt“, ebenso `/api/wallet/../action`, `/api/wallet/%2e%2e/receive`, `/api/Status`, `/api/status/`, `/api/wallet` (ohne Schrägstrich), OPTIONS/HEAD/TRACE/POST auf `/api/status`. Kein Argument der Wallet-Routen ist ein Pfad; Plan und Antwort gehen in ein `mkdtemp`-Verzeichnis (0600, `PrivateTmp`). `keys-upload.sh` lädt nur zwei feste Dateien, weist Verknüpfungen, den Besitzer-Schlüssel (auch inhaltlich) und Duplikate ab, 700/600 per `umask 077`. `push-from-mac.sh` schließt `keys/` und `deployments/` aus, prüft die Zustandsdateien auf Schlüsselmaterial. `setup.sh` warnt vor `mainnet-owner.json` auf dem Server. `ghost-web.service`: `InaccessiblePaths=keys`, `ProtectSystem=strict`, `NoNewPrivileges`, leere `CapabilityBoundingSet` (live nicht verifiziert, siehe oben).

**Argument-Injektion.** Alle Werte durch Regex bzw. Zahlprüfung; `--message=<text>` als ein Argument; Steuerzeichen abgelehnt. Von außen: `to: "--key"` → 400 (Regex), `vault: "../x"` → 400, Nachricht mit Zeilenumbruch → 400, `kas: "-1"` → 400, Nachricht `-x --state=/etc/passwd` und `` $(id)`id`;id `` → an ghostctl durchgereicht und dort nur als Text behandelt (Antwort: Adressfehler, keine Ausführung, `execFile` ohne Shell). Unbekannte Parameter → 400, `__proto__`-Schlüssel → 400 (nicht in der Liste).

**CSRF, Origin, Host.** POST ohne `X-Ghost-Client` → 403; fremder Origin → 403; `Origin: http://k-lend.com` → 403; `Sec-Fetch-Site: cross-site` → 403 (auch GET); `Content-Type: text/plain` → 415; OPTIONS-Preflight ohne CORS-Kopfzeilen → fremde Seiten kommen nie durch den Preflight. `Host: evil.example` über TLS → Caddy antwortet leer, erreicht die Seite nicht. Kein Gegenbeispiel gefunden.

**Ratenbegrenzung und X-Forwarded-For.** 21. Aufruf einer Wallet-Route innerhalb einer Minute → 429 mit `Retry-After`; andere Wallet-Routen teilen das Kontingent (richtig); `/api/status` bleibt frei (eigener Pool, bestätigt). Mitgeschickte `X-Forwarded-For` (einzeln, Kette, `::1`) ändern den Schlüssel nicht: Caddy ersetzt den Kopf, die Seite liest von rechts. Einschränkung: Ob jeder Besucher ein eigenes Kontingent hat oder alle eines teilen (A18 G-1), lässt sich von einem Standort aus nicht unterscheiden; das Verhalten passt zum Caddy-Block im README (`reverse_proxy 172.18.0.1:8787`). IPv6: `k-lend.com` hat keinen AAAA-Eintrag, der docker-proxy-Fall aus G-1 ist damit zurzeit gegenstandslos. **Wird später ein AAAA-Eintrag gesetzt, G-1 vorher klären.** Die Ratenbegrenzung greift vor dem Lesen des Körpers und vor der Validierung (billige Ablehnung).

**Größen und Zeitlimits.** 1,2 MB und 800 kB → 413 „höchstens 768 kB“ von der Seite, 20 kB an `receive` → 413 „10 kB“; die Verbindung wird geschlossen. `headersTimeout` 15 s, `requestTimeout` 90 s, `keepAliveTimeout` 5 s. Ob Caddys `max_size 1MB` greift, war nicht zu sehen (die Seite antwortet vorher), ist aber zweitrangig.

**Fehlermeldungen.** Alle beobachteten Fehler sind kurz und ohne Pfade; `redactPaths` greift öffentlich, `tail()` kürzt stderr und ersetzt lange Hex-Blöcke, 500 → „Interner Fehler.“ (Stack nur ins Journal). ghostctl-Fehlertexte kamen ohne Dateinamen an. A17-8 ist umgesetzt.

**Dateiserver.** `serveStatic` löst `..` über `path.resolve` auf und prüft den Präfix; `%00` → 404; `/%2e%2e/%2e%2e/etc/passwd`, `/.git/HEAD`, `/.env` → Startseite (SPA-Rückfall, keine Datei); `/server/prod.ts`, `/package.json`, `/.vite/manifest.json`, `*.map` → 404; keine Verzeichnislisten. Assets unter `/assets/` mit Hash und `immutable`, alles andere `no-cache`.

**Service Worker und Versionswechsel.** `sw.js` speichert nichts, fängt nur Navigationen ab und liefert offline eine feste Seite; `skipWaiting`/`clients.claim`; wird mit `no-cache` ausgeliefert, `index.html` ebenso. Eine neue Version gilt beim nächsten Laden; die Seite zeigt laut e7c175a einen Hinweis „neu laden“. CSP-konform (A18 Abschnitt 6 bestätigt; live keine Inline-Skripte in `index.html`).

**Abhängigkeiten.** Lockfile: 95 Pakete, alle von `registry.npmjs.org` mit `integrity`, einziges Install-Skript `fsevents` (macOS, optional). `@dotk/sdk` fest auf 2.1.0, nur `@noble/curves`, `@noble/hashes`; ruft ausschließlich `api.dotk.name` auf, Name vorher auf `[a-z0-9.-_]` ≤ 80 geprüft, 15 s Zeitlimit; `node`-Callback liefert höchstens 50 Adressen an `ghostctl utxos` (je eigenes `--address`). `history.ts` holt CoinGecko/Kraken mit Zeitlimit und säubert die Werte. `npm ci` ist an das Lockfile gebunden. (`npm audit` in der Prüfumgebung nicht möglich; auf dem Mac nachholen.)

**TLS/Netz.** Nur TLS 1.2/1.3, HTTP/2, 308 auf HTTPS, HSTS. Von außen geschlossen: 8787 (Seite), 2019 (Caddy-Admin), 16110/16111 (kaspad), 5432, 3000; offen: 22 und 80/443. `listenHosts` lässt nur Loopback und private Adressen zu (Test `nur Loopback und private Netze` vorhanden).

**Deploy-Skripte bei Fehlern.** `set -euo pipefail` überall; nginx/Apache/Caddy-Blöcke werden nach gescheiterter Prüfung zurückgenommen (setup.sh:316–321, 342–346, 362–367); Port-Konflikt → Abbruch vor dem Start; `rsync --delete` kann die ausgeschlossenen Pfade (`keys/`, `deployments/`) nicht löschen; `--zustand-ueberschreiben` verweigert bei laufendem Agenten. Ausnahme: A20c-5.

**Nachprüfung früherer Befunde (soweit ohne Server möglich).** A17-3/A18 G-1 (XFF): umgesetzt, von außen nicht fälschbar. A17-5 (64 KB): Vorlagen 1 MB, Seite 768 kB, live greift die Seite. A17-7: Caddy ohne Antwort-Zeitlimit, Seite 170 s. A17-8: umgesetzt. A17-9 (CSP): live gesetzt. A18 G-3: `sendLimiter` 12/min und `rejectedPlans` vorhanden. A19-4: nur teilweise (A20c-4). A19-7 (`isRetryLater`, `busy: true`): vorhanden.

## Nicht geprüft / Grenzen

Live-Zustand von systemd, ufw, sshd, Docker-Netz, Caddyfile und `ptrace_scope` (SSH verweigert). Verhalten unter Last (nur 110 Anfragen, nie mehr als 2 je Sekunde). ghostctl-Interna jenseits der Aufrufstellen (Audit 20 a/b). Der 11-s-Verbindungsaufbau wurde gemessen, seine Ursache nicht.
