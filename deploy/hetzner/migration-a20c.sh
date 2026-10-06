#!/usr/bin/env bash
# Umstellung eines LAUFENDEN Servers auf das Rechtemodell aus Audit 20
# (A20c-2/3). Einmal als root ausführen, NACH push-from-mac.sh (das legt
# dieses Skript root-eigen unter /root/ghost-deploy ab):
#
#   [Mac]    deploy/hetzner/push-from-mac.sh root@<server>
#   [Server] bash /root/ghost-deploy/migration-a20c.sh            (nur anzeigen)
#   [Server] bash /root/ghost-deploy/migration-a20c.sh --ausfuehren
#   [Server] bash /root/ghost-deploy/setup.sh                      (baut den neuen Code)
#
# Was es tut:
#   1. prüft, dass es selbst und die Vorlagen nur root gehören;
#   2. sichert die beiden Units nach /root/ghost-units.vor-a20c.<Zeit>/;
#   3. legt den Benutzer ghost-web an (Zusatzgruppe ghost);
#   4. deployments/: 2770 ghost:ghost, Dateien gruppen-les-/schreibbar
#      (Seite und Agent schreiben dort beide); keys/ bleibt 700 ghost;
#   5. erzeugt die Units aus den neuen Vorlagen (gleicher Port, gleiches
#      Node), prüft sie mit systemd-analyze, startet die Seite neu und prüft,
#      dass sie antwortet und als ghost-web in deployments/ schreiben kann;
#   6. startet den Agenten neu, wenn er lief, und prüft, dass er läuft;
#   7. geht etwas schief: alte Units zurück, beide Dienste wie vorher.
#      Benutzer und Gruppenrechte bleiben dann stehen (sie schaden den alten
#      Units nicht).
#
# Sendet keine Transaktionen, liest keine Schlüssel, ändert am Webserver nichts.
set -euo pipefail

GHOST_DIR=${GHOST_DIR:-/opt/ghost/kaspa-lending}
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
WEB_UNIT=/etc/systemd/system/ghost-web.service
AGENT_UNIT=/etc/systemd/system/ghost-agent.service
MODE=${1:-}

ok()   { printf '   ok: %s\n' "$*"; }
info() { printf '   %s\n' "$*"; }
warn() { printf '\033[33m   ACHTUNG: %s\033[0m\n' "$*"; }
die()  { printf '\033[31mFEHLER: %s\033[0m\n' "$*" >&2; exit 1; }

# wie in setup.sh (A20c-2)
pruefe_root_eigen() {
  local d=$1 uid=${ERWARTE_UID:-0} p st owner mode bad
  [ -d "$d" ] || { echo "$d ist kein Ordner"; return 1; }
  bad=$(find "$d" \( ! -user "$uid" -o -perm -g+w -o -perm -o+w -o -type l \) -print 2>/dev/null | head -n1)
  [ -z "$bad" ] || { echo "$bad gehört nicht root, ist eine Verknüpfung oder ist für andere beschreibbar"; return 1; }
  p=$d
  while :; do
    p=$(dirname "$p")
    st=$(stat -c '%u %a' "$p" 2>/dev/null || stat -f '%u %Lp' "$p")
    owner=${st%% *}; mode=${st##* }
    if [ "$owner" != 0 ] && [ "$owner" != "$uid" ]; then echo "$p gehört nicht root (uid $owner)"; return 1; fi
    if (( (8#$mode & 8#022) != 0 )); then echo "$p ist für andere beschreibbar ($mode)"; return 1; fi
    [ "$p" = / ] && break
  done
  return 0
}

echo "== Vorprüfung"
[ "$(id -u)" -eq 0 ] || die "bitte als root ausführen."
why=$(pruefe_root_eigen "$SCRIPT_DIR") || die "unsicherer Ordner: $why – erst push-from-mac.sh, dann aus /root/ghost-deploy starten."
[ -f "$SCRIPT_DIR/templates/ghost-web.service" ] && [ -f "$SCRIPT_DIR/templates/ghost-agent.service" ] || die "Vorlagen fehlen in $SCRIPT_DIR/templates."
[ -f "$WEB_UNIT" ] || die "$WEB_UNIT fehlt – das ist kein eingerichteter Server; setup.sh benutzen."
[ -d "$GHOST_DIR/deployments" ] || die "$GHOST_DIR/deployments fehlt."
id ghost >/dev/null 2>&1 || die "Benutzer ghost fehlt."
GHOST_PORT=$(sed -n 's/^Environment=GHOST_PORT=//p' "$WEB_UNIT" | head -n1)
NODE_BIN=$(sed -n 's/^ExecStart=\([^ ]*\) .*/\1/p' "$WEB_UNIT" | head -n1)
[[ ${GHOST_PORT:-} =~ ^[0-9]+$ ]] || die "Port aus $WEB_UNIT nicht lesbar."
[ -x "${NODE_BIN:-}" ] || die "Node aus $WEB_UNIT nicht gefunden ($NODE_BIN)."
agent_was_active=0
systemctl is-active --quiet ghost-agent && agent_was_active=1
ok "Code $GHOST_DIR, Port $GHOST_PORT, Node $NODE_BIN, Agent läuft: $([ $agent_was_active = 1 ] && echo ja || echo nein)"
info "Seite läuft heute als: $(systemctl show -p User --value ghost-web)"
info "kernel.yama.ptrace_scope = $(sysctl -n kernel.yama.ptrace_scope 2>/dev/null || echo '?') (1 oder höher ist gut)"

if [ "$MODE" != --ausfuehren ]; then
  cat <<EOF

Geplante Änderungen (noch nichts geändert):
  - Benutzer ghost-web anlegen (ohne Login, Zusatzgruppe ghost)
  - $GHOST_DIR/deployments → 2770 ghost:ghost, Dateien g+rw (Schlüssel bleiben 700 ghost)
  - $WEB_UNIT und $AGENT_UNIT aus $SCRIPT_DIR/templates neu erzeugen
    (Sicherung nach /root/ghost-units.vor-a20c.<Zeit>/)
  - ghost-web neu starten$([ $agent_was_active = 1 ] && echo ", danach ghost-agent")

Ausführen:  bash $SCRIPT_DIR/migration-a20c.sh --ausfuehren
EOF
  exit 0
fi

echo "== 1. Sicherung der Units"
BACKUP=/root/ghost-units.vor-a20c.$(date +%Y%m%d-%H%M%S)
install -d -m 700 "$BACKUP"
cp -a /etc/systemd/system/ghost-web.service "$BACKUP/"
[ -f "$AGENT_UNIT" ] && cp -a /etc/systemd/system/ghost-agent.service "$BACKUP/"
ok "$BACKUP"

zuruecknehmen() {
  warn "Rücknahme: alte Units aus $BACKUP"
  cp -a "$BACKUP/ghost-web.service" "$WEB_UNIT"
  [ -f "$BACKUP/ghost-agent.service" ] && cp -a "$BACKUP/ghost-agent.service" "$AGENT_UNIT"
  systemctl daemon-reload
  systemctl restart ghost-web || true
  if [ "$agent_was_active" = 1 ]; then systemctl restart ghost-agent || true; fi
  sleep 3
  systemctl --no-pager --lines=0 status ghost-web ghost-agent || true
  die "Umstellung zurückgenommen – Log: journalctl -u ghost-web -u ghost-agent -n 100"
}

echo "== 2. Benutzer ghost-web"
if id ghost-web >/dev/null 2>&1; then
  usermod -a -G ghost ghost-web
  ok "gibt es schon (Zusatzgruppe ghost gesetzt)"
else
  useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin --user-group --groups ghost ghost-web
  ok "angelegt"
fi

echo "== 3. Rechte"
chown ghost:ghost "$GHOST_DIR/deployments"
chmod 2770 "$GHOST_DIR/deployments"
find "$GHOST_DIR/deployments" -mindepth 1 -exec chgrp ghost {} +
find "$GHOST_DIR/deployments" -mindepth 1 -type d -exec chmod 2770 {} +
find "$GHOST_DIR/deployments" -mindepth 1 -type f -exec chmod g+rw,o-rwx {} +
[ -d "$GHOST_DIR/keys" ] && chown ghost:ghost "$GHOST_DIR/keys" && chmod 700 "$GHOST_DIR/keys"
ok "deployments/ 2770, keys/ 700"
# Kann ghost-web lesen, was es braucht, und in deployments/ schreiben?
runuser -u ghost-web -- test -r "$GHOST_DIR/app/server/prod.ts" || die "ghost-web kann den Code nicht lesen ($GHOST_DIR) – Rechte prüfen (Gruppe ghost, 750)."
runuser -u ghost-web -- test -x "$GHOST_DIR/vendor/silverscript/target/release/ghostctl" || die "ghost-web kann ghostctl nicht ausführen."
probe="$GHOST_DIR/deployments/.a20c-probe.$$"
runuser -u ghost-web -- sh -c "umask 007; : > '$probe'" || die "ghost-web kann nicht in deployments/ schreiben."
rm -f "$probe"
if runuser -u ghost-web -- test -r "$GHOST_DIR/keys" 2>/dev/null && runuser -u ghost-web -- ls "$GHOST_DIR/keys" >/dev/null 2>&1; then
  die "ghost-web könnte keys/ lesen – abgebrochen."
fi
ok "ghost-web: Code lesbar, deployments/ beschreibbar, keys/ verschlossen"

echo "== 4. Units aus den neuen Vorlagen"
render() { sed -e "s|@GHOST_DIR@|$GHOST_DIR|g" -e "s|@GHOST_PORT@|$GHOST_PORT|g" -e "s|@NODE_BIN@|$NODE_BIN|g" "$1" > "$2"; }
render "$SCRIPT_DIR/templates/ghost-web.service" "$WEB_UNIT"
render "$SCRIPT_DIR/templates/ghost-agent.service" "$AGENT_UNIT"
if ! systemd-analyze verify "$WEB_UNIT" "$AGENT_UNIT"; then
  zuruecknehmen
fi
systemctl daemon-reload
ok "erzeugt und geprüft"

echo "== 5. Seite neu starten"
systemctl restart ghost-web
for _ in 1 2 3 4 5 6 7 8 9 10; do
  sleep 2
  curl -fsS -o /dev/null "http://127.0.0.1:$GHOST_PORT/" && break
done
systemctl is-active --quiet ghost-web && curl -fsS -o /dev/null "http://127.0.0.1:$GHOST_PORT/" || zuruecknehmen
# einmal ghostctl über die Seite (status nimmt die Sperre in deployments/)
st=$(curl -fsS "http://127.0.0.1:$GHOST_PORT/api/status?network=mainnet" || true)
case "$st" in
  *'"deployed"'*) ok "Seite antwortet, /api/status liefert den Zustand" ;;
  *Permission*|*'Keine Berechtigung'*|*'denied'*) warn "Antwort: ${st:0:200}"; zuruecknehmen ;;
  *) warn "/api/status: ${st:0:200} (Node-Ausfall möglich – bitte selbst ansehen)" ;;
esac
[ "$(systemctl show -p User --value ghost-web)" = ghost-web ] || zuruecknehmen
ok "ghost-web läuft als ghost-web"

echo "== 6. Agent"
if [ "$agent_was_active" = 1 ]; then
  systemctl restart ghost-agent
  sleep 20
  systemctl is-active --quiet ghost-agent || zuruecknehmen
  if journalctl -u ghost-agent --since "-25s" --no-pager 2>/dev/null | grep -qiE 'permission denied|keine berechtigung|operation not permitted'; then
    zuruecknehmen
  fi
  ok "ghost-agent läuft (Log: journalctl -u ghost-agent -f)"
else
  info "Agent lief nicht – nicht gestartet."
fi

echo "== Fertig"
info "Prozesse: $(ps -o user=,comm= -C node,ghostctl 2>/dev/null | tr '\n' ' ')"
info "Sicherung der alten Units: $BACKUP"
info "Weiter: bash $SCRIPT_DIR/setup.sh (baut den neuen Code mit npm ci --ignore-scripts)"
