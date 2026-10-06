# GHOST auf dem eigenen Hetzner-Server

Diese Anleitung bringt zwei Dinge auf deinen Server:

1. **die Seite K.Lend**, öffentlich unter deiner Domain, nur zum Ansehen: Status, Preis und Kursverlauf. Funktionen, die Schlüsseldateien brauchen (Senden, Prägen, Schlüssel anlegen, Nachrichten, Daueraufträge, Tresore), sind dort **gesperrt**;
2. **den GHOST-Agenten**, der rund um die Uhr läuft: Orakel-Preise, Zinsregel, Liquidationen und Tresor-Zahlungen.

Du brauchst kein Vorwissen. Jeder Schritt sagt, **wo** du den Befehl eingibst:

- **[Mac]**: im Programm „Terminal“ auf deinem Mac, im Projektordner. Dorthin kommst du mit
  `cd ~/Desktop/Claude/kaspa-lending`
- **[Server]**: im Terminal, nachdem du dich mit `ssh root@<server-ip>` auf dem Server angemeldet hast.

`<server-ip>` steht für die IP-Adresse aus der Hetzner-Cloud-Konsole, z. B. `203.0.113.10`. `ghost.example.org` steht für deine Domain. Beides ersetzt du jedes Mal durch deine echten Werte.

---

## Überblick: was wo läuft

```
Besucher ──HTTPS──▶ Webserver auf Port 443 (dein vorhandener: nginx / Apache / Caddy)
                        │  nur für ghost.example.org
                        ▼
                   ghost-web  (127.0.0.1:8787, nur lesend, ohne Schlüssel)
                        │
                        ▼
                   ghostctl ◀── ghost-agent (Orakel, Liquidationen …)
                        │          nutzt keys/mainnet-keeper.json + keys/mainnet-signer.json
                        ▼
                   deployments/mainnet.json (Zustand, keine Schlüssel)
```

- Alles von GHOST liegt unter **/opt/ghost/**. Der Agent läuft als Benutzer **ghost**, die Seite als eigener Benutzer **ghost-web** (siehe „Benutzer und Rechte“). Dein anderes Projekt bleibt davon unberührt.
- Einrichtungsskript und Dienst-Vorlagen liegen root-eigen unter **/root/ghost-deploy/**; nur von dort führt root sie aus.
- Der Hauptschlüssel **keys/mainnet-owner.json bleibt auf dem Mac**. Auf den Server kommen nur der Unterzeichner (signer) und eine eigene Gebühren-Wallet (keeper).
- **Der Agent läuft nur an einer Stelle.** Sobald er auf dem Server läuft, bleibt das Fenster „GHOST-Agent starten“ auf dem Mac zu. Sonst greifen beide auf dieselben Coins zu und behindern sich.

---

## Schritt 1: Mit dem Server verbinden

**[Mac]**

```
ssh root@<server-ip>
```

Beim ersten Mal fragt der Mac, ob du dem Server vertraust. Antworte `yes`. Danach steht vorn `root@…`, und du bist auf dem Server. Mit `exit` kommst du zurück zum Mac.

## Schritt 2: Architektur prüfen (Kontrolle)

**[Server]**

```
uname -m
```

Erwartet wird `x86_64`, denn ein Hetzner CPX12 hat einen AMD-Prozessor. `aarch64` würde auch funktionieren. Bei etwas anderem: nicht weitermachen und nachfragen.

Prüfe außerdem das Betriebssystem:

```
cat /etc/os-release | head -2
```

Dort sollte Ubuntu (oder Debian) stehen.

## Schritt 3: Herausfinden, welcher Webserver schon läuft

Auf dem Server läuft schon ein anderes Projekt mit Webserver. Deshalb sind die Ports 80 (http) und 443 (https) belegt. GHOST startet dort **keinen** eigenen Webserver, sondern hängt sich an den vorhandenen an. Dafür musst du wissen, welcher es ist.

**[Server]**

```
ss -ltnp | grep -E ':(80|443)\s'
```

Am Ende jeder Zeile steht in `users:(("…"` der Programmname:

| steht dort        | Webserver  | weiter mit         |
|-------------------|------------|--------------------|
| `nginx`           | nginx      | Abschnitt 9 A      |
| `apache2`/`httpd` | Apache     | Abschnitt 9 B      |
| `caddy`           | Caddy      | Abschnitt 9 C      |
| `docker-proxy`    | ein Docker-Container | Abschnitt 9 D |
| keine Zeile       | Ports frei | Abschnitt 9 E      |

Zur Gegenprobe kannst du nacheinander das hier eingeben (Beenden mit `q`):

```
systemctl status nginx
systemctl status apache2
systemctl status caddy
docker ps          # nur falls Docker installiert ist
```

Wo `active (running)` steht, ist dein Webserver. Das Einrichtungsskript erkennt das später auch selbst.

## Schritt 4: Domain und DNS-Eintrag

Die Seite braucht einen Namen, z. B. **ghost.example.org**. Am einfachsten nimmst du eine **Subdomain** einer Domain, die du schon hast. Dann bleibt die Domain des anderen Projekts unberührt.

1. Melde dich dort an, wo deine Domain verwaltet wird (Domain-Anbieter oder Hetzner DNS).
2. Lege einen neuen DNS-Eintrag an:
   - **Typ:** `A`
   - **Name:** `ghost` (ergibt ghost.example.org)
   - **Wert:** die IPv4-Adresse des Servers
   - **TTL:** Standard
3. Hat der Server auch eine IPv6-Adresse (steht in der Hetzner-Konsole), lege zusätzlich einen Eintrag vom **Typ `AAAA`** mit derselben Subdomain und der IPv6-Adresse an.
4. Warte 5 bis 30 Minuten und prüfe dann **[Mac]**:
   ```
   dig +short ghost.example.org
   ```
   Wenn die Server-IP erscheint, stimmt der Eintrag. Erst dann klappt HTTPS.

## Schritt 5: Code auf den Server bringen

### Variante A (empfohlen): vom Mac mit rsync

**[Mac]**

```
deploy/hetzner/push-from-mac.sh root@<server-ip>
```

Das kopiert den Code nach `/opt/ghost/kaspa-lending` und dazu `deploy/hetzner/` (Einrichtungsskript, Vorlagen) nach `/root/ghost-deploy/`, root-eigen und nur für root lesbar. **Nicht** kopiert werden `keys/`, `deployments/`, `node_modules` und Build-Ordner. Was auf dem Server unter `deployments/` liegt, bleibt immer unberührt.

### Variante B: per git clone

Nur wenn dein Projekt in einem Git-Repository im Netz liegt, z. B. ein privates GitHub-Repository mit Zugang vom Server aus:

**[Server]**

```
REPO_URL=https://github.com/<du>/kaspa-lending.git bash /root/ghost-deploy/setup.sh
```

Dafür muss `deploy/hetzner/` schon root-eigen unter `/root/ghost-deploy/` liegen (z. B. `scp -r deploy/hetzner root@<server-ip>:/root/ghost-deploy`, danach `chown -R root:root /root/ghost-deploy && chmod -R go-rwx /root/ghost-deploy`); aus einem anderen Ordner startet `setup.sh` nicht. Das Skript klont dann samt `vendor/silverscript` (`--recurse-submodules`). Später holst du neuen Code mit `GIT_PULL=1 bash …/setup.sh`.

## Schritt 6: Einrichtungsskript ausführen

**[Server]**

```
DOMAIN=ghost.example.org bash /root/ghost-deploy/setup.sh
```

Das Skript arbeitet diese Punkte ab und sagt bei jedem, was es tut:

1. prüft, dass es selbst und die Vorlagen nur root gehören (sonst Abbruch); legt die Benutzer `ghost` (Agent, Schlüssel, Bau) und `ghost-web` (Seite) an, beide ohne Login-Shell;
2. legt eine 4-GB-Swapdatei an, falls noch kein Swap da ist. Bei 2 GB RAM braucht der Rust-Build sie;
3. installiert Pakete (build-essential, pkg-config, git) und Node.js 24 aus der offiziellen NodeSource-Quelle. Ist schon ein älteres Node.js da, **fragt es vorher**, denn das andere Projekt könnte es brauchen;
4. installiert Rust (rustup) nur für den Benutzer ghost;
5. baut `ghostctl` und die Seite (`npm ci --ignore-scripts`: kein Paket führt beim Installieren eigenen Code aus). **Beim ersten Mal dauert das 20 bis 60 Minuten.** Das ist normal. Das Fenster offen lassen;
6. richtet die Dienste `ghost-web` (die Seite) und `ghost-agent` ein. Der Agent startet erst, wenn Schlüssel und Zustandsdatei da sind (Schritte 7 und 8);
7. erkennt deinen Webserver, legt passende Konfigurationsdateien unter `/etc/ghost/generated/` ab und **fragt**, ob es sie aktivieren soll (siehe Abschnitt 9).

Das Skript darfst du jederzeit erneut ausführen. Was schon erledigt ist, überspringt es.

## Schritt 7: Schlüssel hochladen (nur signer und keeper)

Der Agent braucht zwei Schlüssel:

- **keys/mainnet-signer.json**: signiert die Orakel-Preise. Hast du schon, beim Umzug auf v4 angelegt.
- **keys/mainnet-keeper.json**: eine **eigene Gebühren-Wallet** nur für den Agenten. Sie zahlt die Netzgebühren der Orakel-Updates und hält GHOST für Liquidationen.

Falls es den Keeper noch nicht gibt, lege ihn **[Mac]** an:

```
./ghostctl keygen keys/mainnet-keeper.json
./ghostctl keys
```

Bei `keys` siehst du die Adresse des Keepers. Sende von deiner Wallet etwas **KAS** dorthin (für Gebühren, z. B. 20 bis 50 KAS) und, wenn er liquidieren soll, etwas **GHOST**.

Dann hochladen **[Mac]**:

```
deploy/hetzner/keys-upload.sh root@<server-ip>
```

Das Skript lädt **nur** diese beiden Dateien hoch. Auf dem Server gilt dann: Ordner `keys/` mit Rechten 700, Dateien mit 600, Besitzer ghost. Den Hauptschlüssel `keys/mainnet-owner.json` lädt es nie hoch. Es lehnt auch ab, wenn der Keeper in Wahrheit der Besitzer-Schlüssel ist.

> **Warum ein eigener Keeper?** Der Agent verbrennt bei Liquidationen GHOST des Schlüssels, den er bekommt. Mit dem Hauptschlüssel würde er auch GHOST verbrauchen, die du für deinen eigenen Vault brauchst. Außerdem bleibt so der wertvollste Schlüssel vom Internet-Server fern.

## Schritt 8: Zustandsdatei hochladen, Mac-Agent ausschalten

1. **Auf dem Mac den Agenten beenden**: Fenster „GHOST-Agent starten“ schließen bzw. dort `Ctrl+C` drücken. Ab jetzt nicht mehr öffnen.
2. **[Mac]**
   ```
   deploy/hetzner/push-from-mac.sh root@<server-ip> --zustand
   ```
   Das kopiert `deployments/mainnet.json` (enthält keine Schlüssel) und, falls vorhanden, `mainnet-zins.json` (Takt der Zinsregel) und `mainnet-tresore.json`. Liegen sie schon auf dem Server, bleiben sie dort, denn der Server-Agent führt sie dann selbst weiter.
3. Agent starten **[Server]**:
   ```
   bash /root/ghost-deploy/setup.sh
   ```
   Das Skript fragt: „Mac-Agent ist aus – ghost-agent jetzt hier starten?“ Antworte `j`.
   Alternativ direkt: `systemctl enable --now ghost-agent`
4. Kontrolle **[Server]**:
   ```
   journalctl -u ghost-agent -f
   ```
   Nach einigen Sekunden steht dort „GHOST-Agent gestartet: Orakel, Zinsregel, …“. Mit `Ctrl+C` verlässt du die Anzeige. Der Agent läuft weiter.

**Der Mac danach:** Für eigene Aktionen (Vault, Senden …) nutzt du die Seite weiter lokal auf dem Mac. Der Mac behält seine eigene `deployments/mainnet.json`. Vor eigenen Aktionen gleichst du sie einmal mit der Kette ab **[Mac]**:

```
./ghostctl sync
```

Den Keeper-Schlüssel benutzt du auf dem Mac nicht mehr für Aktionen; er gehört jetzt dem Server-Agenten.

## Schritt 9: Webserver und HTTPS

Das Einrichtungsskript hat drei Vorlagen erzeugt, alle mit deiner Domain und als Reverse Proxy auf `127.0.0.1:8787`:

```
/etc/ghost/generated/nginx-ghost.conf
/etc/ghost/generated/apache-ghost.conf
/etc/ghost/generated/ghost.caddy
```

Ansehen kannst du sie mit `cat <datei>`. Aktiviert wird **nur nach deiner Zustimmung** im Skript. Vorher prüft es die Konfiguration, und danach lädt es den Webserver nur **neu** (reload), statt ihn neu zu starten. Dein anderes Projekt fällt dabei nicht aus. Schlägt die Prüfung fehl, nimmt das Skript seine Änderung zurück und lädt nichts neu.

### 9 A: nginx

Das Skript legt den Block als `/etc/nginx/sites-available/ghost.conf` ab und verlinkt ihn in `sites-enabled`. Dann prüft es mit `nginx -t` und lädt mit `systemctl reload nginx` neu. Danach fragt es, ob es mit **certbot** ein HTTPS-Zertifikat holen soll (`certbot --nginx -d ghost.example.org --redirect`). certbot fragt nach einer E-Mail-Adresse und danach, ob du den Bedingungen von Let's Encrypt zustimmst.

Von Hand geht es genauso **[Server]**:

```
cp /etc/ghost/generated/nginx-ghost.conf /etc/nginx/sites-available/ghost.conf
ln -s /etc/nginx/sites-available/ghost.conf /etc/nginx/sites-enabled/ghost.conf
nginx -t && systemctl reload nginx
apt install certbot python3-certbot-nginx
certbot --nginx -d ghost.example.org --redirect
```

### 9 B: Apache

Das Skript legt `/etc/apache2/sites-available/ghost.conf` an und schaltet die Proxy-Module ein (`a2enmod proxy proxy_http`). Dann aktiviert es die Seite (`a2ensite ghost`), prüft mit `apachectl configtest` und lädt mit `systemctl reload apache2` neu. Für HTTPS fragt es nach `certbot --apache -d ghost.example.org --redirect`.

### 9 C: Caddy

Das Skript legt `/etc/caddy/ghost.caddy` an und hängt **eine** Zeile an deine bestehende Caddyfile: `import /etc/caddy/ghost.caddy`. Vorher legt es eine Sicherung an (`Caddyfile.bak-ghost-…`). Dann prüft es mit `caddy validate` und lädt mit `systemctl reload caddy` neu. Das HTTPS-Zertifikat holt Caddy automatisch; certbot ist nicht nötig.

### 9 D: Docker

Gehört Port 80/443 einem Docker-Container, ändert das Skript nichts, denn es kennt den Aufbau des Containers nicht. Je nach Container passt die nginx- oder die Caddy-Vorlage. **Wichtig:** Im Container bedeutet `127.0.0.1` der Container selbst. Trage dort statt `127.0.0.1:8787` die Adresse des Servers aus Sicht des Containers ein, meist das Gateway des Docker-Netzes (`172.17.0.1:8787` bzw. `172.18.0.1:8787`). Zusätzlich muss ghost-web dann auf dieser Adresse lauschen (`GHOST_HOST=172.18.0.1` beim Aufruf von setup.sh). Das ist ein Fall für eine Rückfrage.

Block für einen Caddy im Container (so läuft k-lend.com, die Caddyfile gehört dem anderen Projekt):

```
k-lend.com {
	encode gzip
	header {
		Strict-Transport-Security "max-age=31536000"
		-Server
	}
	request_body {
		max_size 1MB
	}
	reverse_proxy 172.18.0.1:8787
}
```

- `request_body max_size 1MB`: Die Wallet-Aktionen schicken Plan und signierte Tx in einer Anfrage (gemessen bis etwa 130 kB, die Seite selbst nimmt bis 768 kB an). Ohne Angabe begrenzt Caddy nicht, mit 64 KB scheitern mint, repay und close (Audit 17 A17-5). nginx: `client_max_body_size 1m;`, Apache: `LimitRequestBody 1048576`.
- Zeitlimit: Caddy wartet ohne eigenes Limit auf die Antwort. nginx (`proxy_read_timeout 180s`) und Apache (`ProxyTimeout 180`) müssen länger warten als die Seite beim Senden (170 s), sonst meldet sie statt „unklar“ einen Fehler (A17-7).
- Absender: Caddy setzt `X-Forwarded-For` selbst. Die Seite glaubt diesem Kopf nur, wenn die Verbindung von Loopback oder aus dem Docker-Netz kommt (Standard: das /16 jeder Adresse aus `GHOST_HOST`, also `172.18.0.0/16`; anders mit `GHOST_TRUSTED_PROXY=…`, Adressen oder CIDR). Sie zählt den letzten Eintrag und IPv6 je /64 (A17-3). Ohne das teilen sich alle Besucher eine Ratenbegrenzung.

### 9 E: Ports frei

Sind 80/443 frei, bietet das Skript an, **Caddy** aus der offiziellen Caddy-Paketquelle zu installieren. Caddy holt das HTTPS-Zertifikat dann selbst.

### Testen

Öffne im Browser `https://ghost.example.org`. Die Seite K.Lend erscheint. Auch die Seite des anderen Projekts sollte weiter erreichbar sein; prüfe sie einmal.

## Schritt 10: Firewall (ufw)

Eine Firewall lässt nur die nötigen Ports herein. **Vorsicht:** Braucht dein anderes Projekt weitere Ports (z. B. 8080 oder einen Datenbank-Port von außen), musst du die ebenfalls freigeben. Sonst ist das andere Projekt nicht mehr erreichbar.

**[Server]** zuerst nachsehen, was offen ist:

```
ss -ltnp | grep -v '127.0.0.1\|::1'
ufw status
```

Steht bei `ufw status` schon `active`, ist die Firewall an. Dann füge nur hinzu:

```
ufw allow 80/tcp
ufw allow 443/tcp
```

Ist sie aus (`inactive`): **zuerst SSH erlauben**, sonst sperrst du dich aus!

```
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
# ggf. weitere Ports des anderen Projekts, z. B.: ufw allow 8080/tcp
ufw enable
ufw status
```

Zusätzlich gibt es in der Hetzner-Cloud-Konsole unter „Firewalls“ eine Firewall vor dem Server. Ist dort eine eingerichtet, müssen 80 und 443 auch dort offen sein.

Der Port 8787 von ghost-web muss **nicht** freigegeben werden. Er ist nur innerhalb des Servers erreichbar (127.0.0.1).

## Schritt 11: SSH absichern (Empfehlung)

Das ist eine Empfehlung, kein Muss. Dein bestehender Zugang bleibt, wie er ist, bis du selbst etwas änderst.

1. Hast du auf dem Mac schon einen SSH-Schlüssel? **[Mac]** `ls ~/.ssh/id_ed25519.pub`. Falls nicht: `ssh-keygen -t ed25519` und dreimal Enter drücken.
2. Schlüssel auf den Server **[Mac]**: `ssh-copy-id root@<server-ip>`
3. Testen in einem **neuen** Terminal-Fenster **[Mac]**: `ssh root@<server-ip>`. Klappt das ohne Passwort, funktioniert der Schlüssel.
4. **Erst dann**, und während die alte Verbindung offen bleibt, kannst du die Passwort-Anmeldung abschalten **[Server]**:
   ```
   nano /etc/ssh/sshd_config.d/90-nur-schluessel.conf
   ```
   Hineinschreiben:
   ```
   PasswordAuthentication no
   PermitRootLogin prohibit-password
   ```
   Speichern mit `Ctrl+O`, Enter, `Ctrl+X`. Dann `sshd -t && systemctl reload ssh`.
   Im neuen Fenster noch einmal anmelden. Klappt es, ist alles gut. Klappt es nicht: im alten Fenster die Datei wieder löschen (`rm /etc/ssh/sshd_config.d/90-nur-schluessel.conf && systemctl reload ssh`).

Melden sich für das andere Projekt Benutzer mit Passwort an (z. B. ein Deploy-Werkzeug), lass Schritt 4 weg.

## Schritt 12: Updates des Systems

Etwa einmal im Monat **[Server]**:

```
apt update && apt upgrade
```

Sicherheits-Updates automatisch (meist schon aktiv): `apt install unattended-upgrades`.
Verlangt ein Update einen Neustart (`ls /var/run/reboot-required` zeigt die Datei), starte mit `reboot` neu. Beide GHOST-Dienste starten danach von selbst.

---

## Im Alltag

### Läuft alles?

**[Server]**

```
systemctl status ghost-agent ghost-web
```

`active (running)` = gut. Beenden der Anzeige mit `q`.

### Logs ansehen

```
journalctl -u ghost-agent -f          # Agent live (Ctrl+C beendet nur die Anzeige)
journalctl -u ghost-agent -n 200      # die letzten 200 Zeilen
journalctl -u ghost-agent --since "1 hour ago"
journalctl -u ghost-web -n 100        # die Seite
```

### Neustart

```
systemctl restart ghost-agent
systemctl restart ghost-web
```

Anhalten mit `systemctl stop ghost-agent`, dauerhaft aus mit `systemctl disable --now ghost-agent`. Das brauchst du z. B., wenn der Agent wieder auf dem Mac laufen soll.

### Aktualisieren mit neuem Code

1. **[Mac]** `deploy/hetzner/push-from-mac.sh root@<server-ip>` (ohne `--zustand`)
2. **[Server]** `bash /root/ghost-deploy/setup.sh`

Das Skript baut neu und startet beide Dienste neu. Sendet der Agent im Moment des Neustarts gerade etwas, klärt ghostctl das beim nächsten Start selbst über sein Journal.

### Sicherung von deployments/

In `deployments/` steht der Zustand: Vaults, Orakel, Zinsregel, Tresore. Er enthält **keine Schlüssel**, ist aber wichtig. Sichere ihn regelmäßig auf den Mac **[Mac]**:

```
mkdir -p ~/GHOST-Sicherung
rsync -az root@<server-ip>:/opt/ghost/kaspa-lending/deployments/ ~/GHOST-Sicherung/$(date +%F)/
```

Lege die Sicherung **nicht** über die `deployments/` deines Mac-Projekts. Sie ist nur für den Notfall gedacht. Zum Wiederherstellen auf dem Server: Agent stoppen, Dateien zurückkopieren, Agent starten.

Die Schlüssel selbst sicherst du nur auf dem Mac, so wie bisher.

### Wenn der Agent hängt

1. Log ansehen: `journalctl -u ghost-agent -n 200`.
   - Steht dort wiederholt „Keine Verbindung zum Node“ oder „kein Marktpreis“, haben öffentliche Kaspa-Nodes oder Preisquellen gerade ein Problem. Der Agent versucht es jede Runde (5 Minuten) neu. Abwarten.
   - Meldungen wie „Runde nach … s abgebrochen (Zeitlimit)“ sind einzeln harmlos.
2. Kommt seit mehr als 15 Minuten **keine neue Zeile**: `systemctl restart ghost-agent`.
3. Startet er immer wieder neu und gibt dann auf (`systemctl status ghost-agent` zeigt `failed`): Fehlermeldung im Log lesen. Häufige Gründe:
   - `keys/mainnet-keeper.json` fehlt → Schritt 7;
   - Zustandsdatei fehlt oder ist von einer alten Version → Schritt 8;
   - zu wenig KAS auf dem Keeper → KAS an die Keeper-Adresse senden.
   Danach: `systemctl reset-failed ghost-agent && systemctl start ghost-agent`.
4. Sperrdateien (`deployments/*.lock`) musst du nie löschen. Das Betriebssystem gibt die Sperre frei, sobald ein Prozess endet.
5. Wird der Speicher knapp (`free -h`), hilft ein Neustart des Servers (`reboot`).

### Trennung vom anderen Projekt

- GHOST läuft in `/opt/ghost` als Benutzer `ghost` (Agent) und `ghost-web` (Seite). Die Dienste dürfen nur in `deployments/` schreiben und sehen keine Home-Ordner. Die Seite sieht den Ordner `keys/` überhaupt nicht. Einzelheiten: „Benutzer und Rechte“.
- Die Seite lauscht nur auf `127.0.0.1:8787`. Nach außen geht es ausschließlich über deinen Webserver und dort nur für deine GHOST-Domain. Die anderen Seiten auf dem Webserver bleiben, wie sie sind.
- Das Skript ändert am Webserver nur nach Rückfrage. Es prüft vorher und lädt ihn nur neu, statt ihn neu zu starten.
- Die Seite ist auf 600 MB Speicher begrenzt und startet höchstens 2 ghostctl-Prozesse gleichzeitig. Status und Preis werden zwischengespeichert, sodass viele Besucher kaum Last erzeugen.
- Node.js wird nur dann systemweit angehoben, wenn du zustimmst.
- Brauchst du Port 8787 für etwas anderes: `GHOST_PORT=8788 bash …/setup.sh`.

### Benutzer und Rechte (Audit 20 A20c-2/3)

| Pfad | Besitzer | Rechte | Wer schreibt |
|---|---|---|---|
| `/root/ghost-deploy/` (setup.sh, Vorlagen) | root:root | 700/600 | nur root (push-from-mac.sh) |
| `/opt/ghost/kaspa-lending/` (Code, Bau) | ghost:ghost | 750 | ghost beim Bau (cargo, npm); die Dienste nie (`ProtectSystem=strict`) |
| `keys/` | ghost:ghost | 700/600 | niemand im Betrieb; nur der Agent liest |
| `deployments/` | ghost:ghost | 2770, Dateien 660 | Agent **und** Seite |
| `/etc/ghost/` (web.env, generated/) | root | 755/644 | nur setup.sh |

- **ghost-agent** läuft als `ghost`: als einziger kann er `keys/` lesen.
- **ghost-web** läuft als `ghost-web` mit Zusatzgruppe `ghost`: liest den Code, schreibt in `deployments/`, sieht `keys/` nicht (700 und `InaccessiblePaths`). Den Agenten kann sie weder sehen (`ProtectProc=invisible`) noch mit Signalen erreichen (anderer Benutzer).
- **Warum die Seite in `deployments/` schreiben muss:** `ghostctl status` und `wallet submit` nehmen dort die Sperre (`mainnet.lock`), gleichen den Zustand ab (`mainnet.json` über `mainnet.tmp`), führen das Journal (`*.pending.json`) und legen Wallet-Tresore an (`mainnet-tresore.json`). Deshalb `UMask=0007` in beiden Diensten und das setgid-Bit: neue Dateien gehören der Gruppe `ghost` und sind für beide les- und schreibbar. Eine übernommene Seite kann den Zustand also weiterhin verändern (ghostctl gleicht ihn mit der Kette ab), aber keine Schlüssel lesen und den Agenten nicht anhalten.
- Beide Dienste: `SystemCallFilter=@system-service ~@privileged @obsolete`, `NoExecPaths` für `deployments/` und `/tmp`, keine Capabilities, `NoNewPrivileges`. Die Seite behält `AF_UNIX` (Node verbindet ghostctl über socketpair) und darf JIT (Node).
- `kernel.yama.ptrace_scope` soll mindestens 1 sein; setup.sh zeigt den Wert und warnt bei 0.
- Offen: Gebaut wird weiter als `ghost`. Ein Bau-Skript einer Abhängigkeit (cargo `build.rs`) läuft damit als der Benutzer, dem `keys/` gehört. Root erreicht es nicht mehr (Skript und Vorlagen liegen in `/root/ghost-deploy/`).

### Umstellung eines laufenden Servers (einmalig, Audit 20)

1. **[Mac]** `deploy/hetzner/push-from-mac.sh root@<server-ip>` – bringt Code und `/root/ghost-deploy/`.
2. **[Server]** `bash /root/ghost-deploy/migration-a20c.sh` – zeigt nur, was es ändern würde.
3. **[Server]** `bash /root/ghost-deploy/migration-a20c.sh --ausfuehren` – legt `ghost-web` an, setzt die Rechte von `deployments/`, erzeugt die Units neu, startet erst die Seite, dann (falls er lief) den Agenten und prüft beide. Bei einem Fehler nimmt es die alten Units zurück; die Sicherung liegt unter `/root/ghost-units.vor-a20c.<Zeit>/`.
4. **[Server]** `bash /root/ghost-deploy/setup.sh` – baut den neuen Code.
5. Kontrolle: `ps -o user=,comm= -C node,ghostctl` zeigt `ghost-web node` und `ghost ghostctl`; `systemctl status ghost-web ghost-agent`.
6. Die alte Kopie `/opt/ghost/kaspa-lending/deploy/hetzner/generated/` wird nicht mehr benutzt; Blöcke erzeugt setup.sh jetzt unter `/etc/ghost/generated/`.

### Was die öffentliche Seite kann und was nicht

- **Kann:** Status (Orakel, Vaults, Pool), KAS-Preis, Kursverlauf, Browser-Wallet (`/api/wallet/…`, sobald fertig).
- **Gesperrt** (Antwort 403, ghostctl startet gar nicht): Aktionen, Schlüssel anlegen, GHOST-Empfang prüfen, Nachrichten, Daueraufträge, Tresore, Wallet-Probe. Die Liste „Schlüssel“ ist dort immer leer.
- Für alles mit eigenen Schlüsseln nutzt du die Seite wie bisher lokal auf dem Mac („GHOST-Seite öffnen“).

---

## Kurzfassung

| Wo | Befehl |
|---|---|
| Mac | `deploy/hetzner/push-from-mac.sh root@<ip>` |
| Server | `DOMAIN=ghost.example.org bash /root/ghost-deploy/setup.sh` |
| Mac | `deploy/hetzner/keys-upload.sh root@<ip>` |
| Mac | Agent-Fenster schließen, dann `deploy/hetzner/push-from-mac.sh root@<ip> --zustand` |
| Server | `setup.sh` erneut ausführen und den Agentenstart mit `j` bestätigen |
| Server | `journalctl -u ghost-agent -f` |
