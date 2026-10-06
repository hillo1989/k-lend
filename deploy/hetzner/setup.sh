#!/usr/bin/env bash
# GHOST auf einem eigenen Server (Hetzner Cloud, Ubuntu/Debian) einrichten.
#
# Als root ausführen; beliebig oft wiederholbar (idempotent). Beim ersten Mal
# und nach jedem Code-Update (push-from-mac.sh) einfach erneut starten:
#
#   DOMAIN=ghost.example.org bash /opt/ghost/kaspa-lending/deploy/hetzner/setup.sh
#
# Was es tut (Einzelheiten in README.md):
#   1. Benutzer „ghost“ (ohne Login-Shell), Ordner /opt/ghost
#   2. 4 GB Swapdatei, falls es noch keinen Swap gibt
#   3. Pakete: build-essential, pkg-config, git, Node.js (NodeSource) – lädt der Server
#   4. Rust (rustup) für den Benutzer ghost
#   5. Code: liegt schon da (push-from-mac.sh) oder git clone von REPO_URL
#   6. Bauen: ghostctl (Release) und die Seite (vite build)
#   7. systemd: ghost-web (Seite, nur 127.0.0.1) und ghost-agent (GHOST-Agent)
#   8. Webserver: erkennt, was Port 80/443 belegt (nginx, Apache, Caddy, Docker),
#      erzeugt einen passenden Block unter deploy/hetzner/generated/ und
#      aktiviert ihn NUR nach Rückfrage, mit Konfigurationstest und Reload.
#      Sind 80/443 frei, bietet es einen eigenen Caddy an.
#
# Umgebungsvariablen (alle freiwillig):
#   DOMAIN       Domain der Seite, z. B. ghost.example.org (sonst Rückfrage)
#   GHOST_PORT   Port des Produktionsservers auf 127.0.0.1 (Standard 8787)
#   GHOST_DIR    Code-Ordner (Standard /opt/ghost/kaspa-lending)
#   REPO_URL     Git-Adresse, falls der Code per git clone kommen soll
#   GIT_PULL=1   vorhandenen git-Checkout vor dem Bauen aktualisieren
#   CARGO_JOBS   parallele Rust-Compiler (Standard 2; bei 2 GB RAM nicht mehr)
#
# Das Skript sendet keine Transaktionen und fasst keine Schlüssel an.
set -euo pipefail

GHOST_HOME=/opt/ghost
GHOST_DIR=${GHOST_DIR:-$GHOST_HOME/kaspa-lending}
GHOST_PORT=${GHOST_PORT:-8787}
CARGO_JOBS=${CARGO_JOBS:-2}
# Zusätzliche Lausch-Adresse der Seite (privat), z. B. das Gateway eines
# Docker-Netzes, wenn der Webserver in einem Container läuft (172.18.0.1)
GHOST_HOST=${GHOST_HOST:-}
# Proxys, deren X-Forwarded-For die Seite für die Ratenbegrenzung glaubt
# (Adressen/CIDR). Leer: Loopback und das /16 jeder Adresse aus GHOST_HOST.
GHOST_TRUSTED_PROXY=${GHOST_TRUSTED_PROXY:-}
# Höchstspeicher des Rust-Baus (Rest über Swap), damit andere Dienste auf dem
# Server (Datenbanken) nie vom OOM-Killer getroffen werden
BUILD_MEM=${BUILD_MEM:-1200M}
REPO_URL=${REPO_URL:-}
DOMAIN=${DOMAIN:-}
NODE_MAJOR=24
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# ------------------------------------------------------------ Hilfen ----
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
ok()   { printf '   ok: %s\n' "$*"; }
info() { printf '   %s\n' "$*"; }
warn() { printf '\033[33m   ACHTUNG: %s\033[0m\n' "$*"; }
die()  { printf '\033[31mFEHLER: %s\033[0m\n' "$*" >&2; exit 1; }
# Ja/Nein-Frage; ohne Terminal (z. B. per Pipe) immer „Nein“
ask() {
  local a
  [ -r /dev/tty ] || return 1
  read -r -p "   $1 [j/N] " a </dev/tty || return 1
  [[ $a =~ ^[jJyY] ]]
}
as_ghost() {
  runuser -u ghost -- env HOME="$GHOST_HOME" PATH="$GHOST_HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin" "$@"
}
render() { # Vorlage → Datei, Platzhalter ersetzen
  sed -e "s|@GHOST_DIR@|$GHOST_DIR|g" -e "s|@GHOST_PORT@|$GHOST_PORT|g" \
      -e "s|@DOMAIN@|$DOMAIN|g" -e "s|@NODE_BIN@|${NODE_BIN:-/usr/bin/node}|g" "$1" > "$2"
}
backup() { [ -e "$1" ] && cp -a "$1" "$1.bak-ghost-$(date +%Y%m%d-%H%M%S)" || true; }
# Wer lauscht auf Port $1? Gibt den Prozessnamen aus (leer = frei)
port_owner() {
  ss -ltnpH "sport = :$1" 2>/dev/null | grep -o 'users:(("[^"]*"' | head -n1 | sed -e 's/users:(("//' -e 's/"$//' || true
}
port_in_use() { [ -n "$(ss -ltnH "sport = :$1" 2>/dev/null)" ]; }

# --------------------------------------------------------- Vorprüfung ----
step "Vorprüfung"
[ "$(id -u)" -eq 0 ] || die "bitte als root ausführen (ssh root@server, dann bash setup.sh)."
command -v apt-get >/dev/null || die "nur für Ubuntu/Debian (apt) gebaut."
command -v systemctl >/dev/null || die "systemd fehlt."
. /etc/os-release
ok "System: ${PRETTY_NAME:-unbekannt}, Architektur $(uname -m)"
case "$(uname -m)" in
  x86_64|aarch64) ;;
  *) die "Architektur $(uname -m) wird nicht unterstützt (erwartet x86_64 oder aarch64)." ;;
esac
[[ $GHOST_PORT =~ ^[0-9]+$ ]] || die "GHOST_PORT muss eine Zahl sein."
free_gb=$(df -BG --output=avail / | tail -n1 | tr -dc 0-9)
[ "${free_gb:-0}" -ge 10 ] || warn "nur ${free_gb} GB frei auf / – der Rust-Build braucht etwa 6–8 GB."

# Domain: Umgebung, sonst gespeicherter Wert, sonst Rückfrage (leer erlaubt)
mkdir -p /etc/ghost
if [ -z "$DOMAIN" ] && [ -f /etc/ghost/web.env ]; then
  DOMAIN=$(sed -n 's/^GHOST_DOMAIN=//p' /etc/ghost/web.env | head -n1)
fi
if [ -z "$DOMAIN" ] && [ -r /dev/tty ]; then
  read -r -p "   Domain der Seite (z. B. ghost.example.org, leer = später): " DOMAIN </dev/tty || true
fi
DOMAIN=$(printf '%s' "$DOMAIN" | tr 'A-Z' 'a-z' | tr -d ' ')
if [ -n "$DOMAIN" ] && ! [[ $DOMAIN =~ ^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$ ]]; then
  die "„$DOMAIN“ ist keine gültige Domain."
fi
ok "Domain: ${DOMAIN:-(noch keine – Webserver-Teil wird übersprungen)}"

# ------------------------------------------------------- 1. Benutzer ----
step "1. Benutzer ghost"
if id ghost >/dev/null 2>&1; then
  ok "gibt es schon"
else
  useradd --system --home-dir "$GHOST_HOME" --create-home --shell /usr/sbin/nologin --user-group ghost
  ok "angelegt (ohne Login-Shell)"
fi
install -d -m 750 -o ghost -g ghost "$GHOST_HOME"

# --------------------------------------------------------- 2. Swap ----
step "2. Swap"
SWAP_MB=$(free -m | awk '/^Swap:/ {print $2}')
if [ -n "$(swapon --noheadings --show 2>/dev/null)" ] && [ "${SWAP_MB:-0}" -lt 3900 ] && [ ! -f /swapfile-ghost ]; then
  # zu wenig für den Rust-Bau: zweite Swapdatei bis insgesamt etwa 6 GB
  fallocate -l 4G /swapfile-ghost 2>/dev/null || dd if=/dev/zero of=/swapfile-ghost bs=1M count=4096 status=none
  chmod 600 /swapfile-ghost
  mkswap /swapfile-ghost >/dev/null
  swapon /swapfile-ghost
  grep -q '^/swapfile-ghost ' /etc/fstab || echo '/swapfile-ghost none swap sw 0 0' >> /etc/fstab
  ok "Swap war nur ${SWAP_MB} MB: 4 GB /swapfile-ghost dazu"
elif [ -n "$(swapon --noheadings --show 2>/dev/null)" ]; then
  ok "Swap ist schon aktiv: $(swapon --noheadings --show=NAME,SIZE | tr '\n' ' ')"
elif [ -f /swapfile ]; then
  chmod 600 /swapfile
  swapon /swapfile 2>/dev/null || { mkswap /swapfile >/dev/null && swapon /swapfile; }
  grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
  ok "/swapfile gab es schon, jetzt aktiv"
else
  fallocate -l 4G /swapfile 2>/dev/null || dd if=/dev/zero of=/swapfile bs=1M count=4096 status=none
  chmod 600 /swapfile
  mkswap /swapfile >/dev/null
  swapon /swapfile
  grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
  ok "4 GB /swapfile angelegt und dauerhaft eingetragen"
fi

# ------------------------------------------------------- 3. Pakete ----
step "3. Pakete"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config git curl ca-certificates gnupg rsync iproute2 >/dev/null
ok "build-essential, pkg-config, git, curl, rsync"

node_ok() { command -v node >/dev/null && node -e 'const [a,b]=process.versions.node.split(".").map(Number);process.exit(a>22||(a===22&&b>=18)?0:1)'; }
if node_ok; then
  ok "Node.js $(node --version) ist schon da"
else
  if command -v node >/dev/null; then
    warn "Node.js $(node --version) ist zu alt (nötig: ab 22.18). Vielleicht nutzt das andere Projekt diese Version!"
    ask "Node.js systemweit auf Version $NODE_MAJOR (NodeSource) anheben?" || die "abgebrochen – Node.js bitte selbst passend einrichten und setup.sh erneut starten."
  fi
  install -d -m 755 /etc/apt/keyrings
  curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key | gpg --dearmor --yes -o /etc/apt/keyrings/nodesource.gpg
  echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" > /etc/apt/sources.list.d/nodesource.list
  apt-get update -qq
  apt-get install -y -qq nodejs >/dev/null
  node_ok || die "Node.js-Installation fehlgeschlagen."
  ok "Node.js $(node --version) aus NodeSource installiert"
fi
NODE_BIN=$(command -v node)

# -------------------------------------------------------- 4. Rust ----
step "4. Rust für ghost"
if [ -x "$GHOST_HOME/.cargo/bin/cargo" ]; then
  ok "rustup ist schon da ($(as_ghost cargo --version))"
else
  as_ghost bash -c 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path' >/dev/null
  ok "rustup + stable installiert ($(as_ghost cargo --version))"
fi

# --------------------------------------------------------- 5. Code ----
step "5. Code nach $GHOST_DIR"
if [ -f "$GHOST_DIR/protocol/Cargo.toml" ]; then
  ok "Code liegt schon da"
  if [ "${GIT_PULL:-0}" = 1 ] && [ -d "$GHOST_DIR/.git" ]; then
    as_ghost git -C "$GHOST_DIR" pull --ff-only
    as_ghost git -C "$GHOST_DIR" submodule update --init --recursive
    ok "git pull erledigt"
  fi
elif [ -n "$REPO_URL" ]; then
  install -d -m 750 -o ghost -g ghost "$(dirname "$GHOST_DIR")"
  as_ghost git clone --recurse-submodules "$REPO_URL" "$GHOST_DIR"
  ok "geklont von $REPO_URL"
else
  die "kein Code in $GHOST_DIR. Erst auf dem Mac deploy/hetzner/push-from-mac.sh ausführen (oder REPO_URL=… angeben)."
fi
[ -f "$GHOST_DIR/vendor/silverscript/Cargo.toml" ] || die "vendor/silverscript fehlt (git: --recurse-submodules; rsync: push-from-mac.sh erneut)."
chown -R ghost:ghost "$GHOST_DIR"
chmod 750 "$GHOST_DIR"
install -d -m 700 -o ghost -g ghost "$GHOST_DIR/keys"
install -d -m 750 -o ghost -g ghost "$GHOST_DIR/deployments"
[ -f "$GHOST_DIR/keys/mainnet-owner.json" ] && warn "keys/mainnet-owner.json liegt auf dem Server! Der Hauptschlüssel gehört nur auf den Mac – bitte hier löschen."
# Vorlagen aus dem Code nehmen (aktueller als eine einzeln kopierte setup.sh)
TEMPLATES="$GHOST_DIR/deploy/hetzner/templates"
[ -d "$TEMPLATES" ] || TEMPLATES="$SCRIPT_DIR/templates"
GENERATED="$GHOST_DIR/deploy/hetzner/generated"
install -d -m 755 "$GENERATED"

# -------------------------------------------------------- 6. Bauen ----
step "6. Bauen (beim ersten Mal 20–60 Minuten, danach schneller)"
info "ghostctl (Rust, Release, $CARGO_JOBS Jobs) …"
# Bau in einer begrenzten Speicher-Scope (cgroup): reicht der Speicher nicht,
# trifft es höchstens den Bau, nie andere Dienste; niedrige CPU-Priorität
BUILD_CMD="cd '$GHOST_DIR/protocol' && nice -n 15 cargo build --release --bin ghostctl -j '$CARGO_JOBS'"
if command -v systemd-run >/dev/null 2>&1; then
  systemd-run --quiet --scope -p MemoryHigh="$BUILD_MEM" -p MemoryMax="$BUILD_MEM" -p CPUWeight=20 \
    runuser -u ghost -- env HOME="$GHOST_HOME" PATH="$GHOST_HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin" bash -c "$BUILD_CMD" || die "Bau von ghostctl fehlgeschlagen (Speicher? CARGO_JOBS=1 BUILD_MEM=1500M versuchen)"
else
  as_ghost bash -c "$BUILD_CMD"
fi
GHOSTCTL="$GHOST_DIR/vendor/silverscript/target/release/ghostctl"
[ -x "$GHOSTCTL" ] || die "ghostctl wurde nicht gebaut ($GHOSTCTL fehlt)."
ok "ghostctl: $GHOSTCTL"
info "Seite (npm ci + vite build) …"
as_ghost bash -c "cd '$GHOST_DIR/app' && npm ci --no-audit --no-fund --loglevel=error && npx vite build --logLevel warn"
[ -f "$GHOST_DIR/app/dist/index.html" ] || die "vite build hat keine app/dist/index.html erzeugt."
ok "Seite: $GHOST_DIR/app/dist"

# ------------------------------------------------------ 7. systemd ----
step "7. Dienste (systemd)"
# Port des Produktionsservers muss frei sein (oder schon uns gehören)
if port_in_use "$GHOST_PORT" && ! systemctl is-active --quiet ghost-web; then
  die "Port $GHOST_PORT ist schon belegt ($(port_owner "$GHOST_PORT")). Anderen wählen: GHOST_PORT=8788 bash setup.sh"
fi
{ printf 'GHOST_DOMAIN=%s\n' "$DOMAIN"; [ -n "$GHOST_HOST" ] && printf 'GHOST_HOST=127.0.0.1,%s\n' "$GHOST_HOST"; if [ -n "$GHOST_TRUSTED_PROXY" ]; then printf 'GHOST_TRUSTED_PROXY=%s\n' "$GHOST_TRUSTED_PROXY"; fi; } > /etc/ghost/web.env
chmod 644 /etc/ghost/web.env
render "$TEMPLATES/ghost-web.service" /etc/systemd/system/ghost-web.service
render "$TEMPLATES/ghost-agent.service" /etc/systemd/system/ghost-agent.service
systemctl daemon-reload
systemctl enable ghost-web >/dev/null 2>&1
systemctl restart ghost-web
sleep 2
if systemctl is-active --quiet ghost-web && curl -fsS -o /dev/null "http://127.0.0.1:$GHOST_PORT/"; then
  ok "ghost-web läuft auf 127.0.0.1:$GHOST_PORT"
else
  warn "ghost-web antwortet nicht – Log: journalctl -u ghost-web -n 50"
fi

agent_ready=1
for f in keys/mainnet-keeper.json keys/mainnet-signer.json deployments/mainnet.json; do
  [ -f "$GHOST_DIR/$f" ] || { info "fehlt noch: $f"; agent_ready=0; }
done
if [ "$agent_ready" = 0 ]; then
  info "ghost-agent wird noch nicht gestartet. Erst keys-upload.sh und push-from-mac.sh --zustand (README Schritt 7–8), dann setup.sh erneut."
elif systemctl is-active --quiet ghost-agent; then
  systemctl restart ghost-agent
  ok "ghost-agent mit neuem Programm neu gestartet"
else
  warn "Der Agent darf NUR an einer Stelle laufen. Ist der GHOST-Agent auf dem Mac beendet (Fenster „GHOST-Agent starten“ geschlossen)?"
  if ask "Mac-Agent ist aus – ghost-agent jetzt hier starten und beim Booten automatisch starten?"; then
    systemctl enable --now ghost-agent >/dev/null 2>&1
    ok "ghost-agent gestartet – Log: journalctl -u ghost-agent -f"
  else
    info "nicht gestartet. Später: systemctl enable --now ghost-agent"
  fi
fi

# ----------------------------------------------------- 8. Webserver ----
step "8. Webserver (Port 80/443)"
p80=$(port_owner 80); p443=$(port_owner 443)
owner=${p443:-$p80}
if port_in_use 80 || port_in_use 443; then
  case "$owner" in
    nginx*)                 kind=nginx ;;
    apache2*|httpd*)        kind=apache ;;
    caddy*)                 kind=caddy ;;
    docker-proxy*|containerd*|docker*) kind=docker ;;
    *)                      kind=unknown ;;
  esac
  info "Port 80: ${p80:-frei}   Port 443: ${p443:-frei}   → erkannt: $kind"
  info "Dort läuft schon ein Webserver (vermutlich das andere Projekt). Es wird KEIN eigener Caddy gestartet."
else
  kind=free
  info "Port 80 und 443 sind frei."
fi

if [ -z "$DOMAIN" ]; then
  info "Keine Domain angegeben – Webserver-Teil übersprungen. Später: DOMAIN=… bash setup.sh"
else
  # immer alle drei Vorlagen erzeugen, damit man auch von Hand wählen kann
  render "$TEMPLATES/nginx-ghost.conf" "$GENERATED/nginx-ghost.conf"
  render "$TEMPLATES/apache-ghost.conf" "$GENERATED/apache-ghost.conf"
  render "$TEMPLATES/ghost.caddy" "$GENERATED/ghost.caddy"
  ok "erzeugt: $GENERATED/{nginx-ghost.conf,apache-ghost.conf,ghost.caddy}"

  case "$kind" in
  nginx)
    if [ -d /etc/nginx/sites-available ]; then
      dest=/etc/nginx/sites-available/ghost.conf; link=/etc/nginx/sites-enabled/ghost.conf
    else
      dest=/etc/nginx/conf.d/ghost.conf; link=
    fi
    other=$(grep -rlsE "server_name[^;]*[[:space:]]$DOMAIN[[:space:];]" /etc/nginx 2>/dev/null | grep -v "/ghost.conf" || true)
    [ -n "$other" ] && warn "$DOMAIN steht schon in: $other – bitte prüfen, bevor du aktivierst."
    info "Vorschlag: $GENERATED/nginx-ghost.conf → $dest${link:+ (+ Link in sites-enabled)}"
    if ask "nginx-Block jetzt aktivieren (nginx -t, dann reload – das andere Projekt läuft weiter)?"; then
      backup "$dest"
      cp "$GENERATED/nginx-ghost.conf" "$dest"
      [ -n "$link" ] && ln -sfn "$dest" "$link"
      if nginx -t; then
        systemctl reload nginx
        ok "nginx neu geladen; http://$DOMAIN zeigt jetzt auf GHOST"
        if ask "HTTPS-Zertifikat mit certbot holen (installiert certbot, DNS muss schon stimmen)?"; then
          apt-get install -y -qq certbot python3-certbot-nginx >/dev/null
          certbot --nginx -d "$DOMAIN" --redirect && ok "HTTPS aktiv: https://$DOMAIN"
        else
          info "später: apt install certbot python3-certbot-nginx && certbot --nginx -d $DOMAIN --redirect"
        fi
      else
        warn "nginx -t meldet Fehler – GHOST-Block wieder entfernt, nichts neu geladen."
        rm -f "$dest"; [ -n "$link" ] && rm -f "$link"
        latest=$(ls -t "$dest".bak-ghost-* 2>/dev/null | head -n1 || true)
        [ -n "$latest" ] && cp -a "$latest" "$dest"
      fi
    fi
    ;;
  apache)
    if [ -d /etc/apache2/sites-available ]; then
      dest=/etc/apache2/sites-available/ghost.conf
      info "Vorschlag: $GENERATED/apache-ghost.conf → $dest (a2enmod proxy proxy_http, a2ensite ghost)"
      if ask "Apache-VirtualHost jetzt aktivieren (configtest, dann reload – das andere Projekt läuft weiter)?"; then
        backup "$dest"
        cp "$GENERATED/apache-ghost.conf" "$dest"
        a2enmod -q proxy proxy_http >/dev/null
        a2ensite -q ghost >/dev/null
        if apachectl configtest; then
          systemctl reload apache2
          ok "Apache neu geladen; http://$DOMAIN zeigt jetzt auf GHOST"
          if ask "HTTPS-Zertifikat mit certbot holen (installiert certbot, DNS muss schon stimmen)?"; then
            apt-get install -y -qq certbot python3-certbot-apache >/dev/null
            certbot --apache -d "$DOMAIN" --redirect && ok "HTTPS aktiv: https://$DOMAIN"
          else
            info "später: apt install certbot python3-certbot-apache && certbot --apache -d $DOMAIN --redirect"
          fi
        else
          warn "apachectl configtest meldet Fehler – GHOST-Site wieder ausgeschaltet, nichts neu geladen."
          a2dissite -q ghost >/dev/null || true
          rm -f "$dest"
        fi
      fi
    else
      warn "Apache ohne /etc/apache2/sites-available (kein Debian-Aufbau): $GENERATED/apache-ghost.conf bitte von Hand einbinden."
    fi
    ;;
  caddy)
    if systemctl is-active --quiet caddy && [ -f /etc/caddy/Caddyfile ]; then
      info "Vorschlag: $GENERATED/ghost.caddy → /etc/caddy/ghost.caddy, dazu „import /etc/caddy/ghost.caddy“ am Ende der Caddyfile"
      if ask "Caddy-Site-Block jetzt einbinden (caddy validate, dann reload – das andere Projekt läuft weiter)?"; then
        backup /etc/caddy/Caddyfile
        cp "$GENERATED/ghost.caddy" /etc/caddy/ghost.caddy
        grep -qF 'import /etc/caddy/ghost.caddy' /etc/caddy/Caddyfile || printf '\n# GHOST (deploy/hetzner/setup.sh)\nimport /etc/caddy/ghost.caddy\n' >> /etc/caddy/Caddyfile
        if caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile; then
          systemctl reload caddy
          ok "Caddy neu geladen; HTTPS für $DOMAIN holt Caddy selbst"
        else
          warn "caddy validate meldet Fehler – alte Caddyfile zurückgelegt, nichts neu geladen."
          latest=$(ls -t /etc/caddy/Caddyfile.bak-ghost-* | head -n1)
          cp -a "$latest" /etc/caddy/Caddyfile
          rm -f /etc/caddy/ghost.caddy
        fi
      fi
    else
      warn "Caddy lauscht, ist aber kein systemd-Dienst mit /etc/caddy/Caddyfile (Docker?). $GENERATED/ghost.caddy bitte von Hand in dessen Konfiguration übernehmen."
    fi
    ;;
  docker)
    warn "Port 80/443 gehören einem Docker-Container. Den kann dieses Skript nicht sicher ändern."
    info "Je nach Container passt $GENERATED/nginx-ghost.conf oder $GENERATED/ghost.caddy."
    info "Achtung: im Container zeigt 127.0.0.1 auf den Container selbst – dort die Adresse des Servers (z. B. host.docker.internal oder 172.17.0.1) statt 127.0.0.1 eintragen. README, Abschnitt „Docker“."
    ;;
  unknown)
    warn "Unbekannter Prozess „$owner“ auf Port 80/443. Nichts geändert. Vorlagen liegen in $GENERATED/."
    ;;
  free)
    if ask "Eigenen Caddy installieren (offizielle Paketquelle) und auf 80/443 für $DOMAIN starten?"; then
      apt-get install -y -qq debian-keyring debian-archive-keyring apt-transport-https >/dev/null 2>&1 || true
      curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor --yes -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
      curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' > /etc/apt/sources.list.d/caddy-stable.list
      apt-get update -qq
      apt-get install -y -qq caddy >/dev/null
      backup /etc/caddy/Caddyfile
      cp "$GENERATED/ghost.caddy" /etc/caddy/Caddyfile
      if caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile; then
        systemctl enable caddy >/dev/null 2>&1
        systemctl reload-or-restart caddy
        ok "Caddy läuft; HTTPS für $DOMAIN holt Caddy selbst (DNS muss stimmen)"
      else
        warn "caddy validate meldet Fehler – bitte /etc/caddy/Caddyfile prüfen."
      fi
    else
      info "kein Caddy installiert. Später: setup.sh erneut starten."
    fi
    ;;
  esac
fi

# ------------------------------------------------------- Zusammenfassung ----
step "Fertig"
info "Seite intern:   http://127.0.0.1:$GHOST_PORT  (systemctl status ghost-web)"
[ -n "$DOMAIN" ] && info "Seite außen:    https://$DOMAIN"
info "Agent:          systemctl status ghost-agent   |   journalctl -u ghost-agent -f"
info "Vorlagen:       $GENERATED/"
