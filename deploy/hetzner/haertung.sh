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
F=/opt/prueflotse/Caddyfile
cp "$F" "/root/Caddyfile.vor-hsts.$(date +%Y%m%d%H%M%S)"
# Datei ist einzeln in den Container eingebunden: Inhalt ersetzen, nicht die Datei (sonst sieht der Container die alte)
sed 's|Strict-Transport-Security "max-age=31536000"$|Strict-Transport-Security "max-age=31536000; includeSubDomains"|' "$F" > /tmp/Caddyfile.neu
diff "$F" /tmp/Caddyfile.neu || true
cat /tmp/Caddyfile.neu > "$F"
rm -f /tmp/Caddyfile.neu
docker exec prueflotse-web-1 caddy validate --config /etc/caddy/Caddyfile
docker exec prueflotse-web-1 caddy reload --config /etc/caddy/Caddyfile
echo "Fertig. Prüfen: curl -sI https://k-lend.com/ | grep -i strict"
