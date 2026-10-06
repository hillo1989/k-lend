#!/bin/bash
# Härtung nach dem Sicherheitstest vom 06.10.2026 (als root auf dem Server):
#   1. SSH: keine X11-, Port- und Agent-Weiterleitung (wird nicht gebraucht)
#   2. HSTS für k-lend.com auch für Subdomains (Caddy-Datei von Prüflotse)
# Aufruf vom Mac:  ssh root@162.55.185.54 'bash -s' < deploy/hetzner/haertung.sh
set -euo pipefail

echo "== 1. SSH"
cat > /etc/ssh/sshd_config.d/10-haertung.conf <<'CONF'
# K.Lend-Härtung (Sicherheitstest 06.10.2026): keine Weiterleitungen über SSH
X11Forwarding no
AllowTcpForwarding no
AllowAgentForwarding no
CONF
sshd -t
systemctl reload ssh
sshd -T | grep -E "^(x11forwarding|allowtcpforwarding|allowagentforwarding|passwordauthentication) "

echo "== 2. HSTS (Caddy von Prüflotse)"
# Audit 20 A20c-5: Die neue Fassung wird erst IM CONTAINER gegen Caddy geprüft
# (Kopie neben der echten Datei, damit relative imports gleich aufgelöst
# werden) und nur bei Erfolg übernommen. Bei einem Fehler bleibt die
# bisherige Datei unverändert, nichts wird neu geladen.
F=/opt/prueflotse/Caddyfile
C=prueflotse-web-1
PRUEF=/etc/caddy/Caddyfile.ghost-pruefung
NEU=$(mktemp /root/Caddyfile.neu.XXXXXX)
aufraeumen() { rm -f "$NEU"; docker exec "$C" rm -f "$PRUEF" >/dev/null 2>&1 || true; }
trap aufraeumen EXIT
cp -a "$F" "/root/Caddyfile.vor-hsts.$(date +%Y%m%d%H%M%S)"
sed 's|Strict-Transport-Security "max-age=31536000"$|Strict-Transport-Security "max-age=31536000; includeSubDomains"|' "$F" > "$NEU"
if cmp -s "$F" "$NEU"; then
  echo "Caddyfile ist schon gehärtet – nichts zu tun."
  exit 0
fi
diff "$F" "$NEU" || true
docker cp "$NEU" "$C:$PRUEF"
if ! docker exec "$C" caddy validate --config "$PRUEF" --adapter caddyfile; then
  echo "FEHLER: caddy validate lehnt die neue Fassung ab – $F bleibt unverändert, nichts neu geladen." >&2
  exit 1
fi
# Datei ist einzeln in den Container eingebunden: Inhalt ersetzen, nicht die Datei (sonst sieht der Container die alte)
cat "$NEU" > "$F"
docker exec "$C" caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
docker exec "$C" caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
echo "Fertig. Prüfen: curl -sI https://k-lend.com/ | grep -i strict"
