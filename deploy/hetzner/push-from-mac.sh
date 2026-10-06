#!/usr/bin/env bash
# Code vom Mac auf den Server kopieren (rsync über SSH) – OHNE keys/.
#
#   deploy/hetzner/push-from-mac.sh root@<server-ip>                 Code
#   deploy/hetzner/push-from-mac.sh root@<server-ip> --zustand       + Zustandsdateien (nur wenn dort noch keine sind)
#   deploy/hetzner/push-from-mac.sh root@<server-ip> --zustand-ueberschreiben
#                                                                     + Zustandsdateien ersetzen (Agent dort muss aus sein)
#
# Danach auf dem Server: bash /opt/ghost/kaspa-lending/deploy/hetzner/setup.sh
#
# Nie kopiert: keys/ (dafür keys-upload.sh), node_modules, Build-Ordner,
# .git, und deployments/ (außer mit --zustand). Was auf dem Server unter
# deployments/ liegt, bleibt unberührt: Dort führt der Agent den Zustand fort.
set -euo pipefail

SERVER=${1:-}
MODE=${2:-}
REMOTE_DIR=${GHOST_DIR:-/opt/ghost/kaspa-lending}
cd "$(dirname "$0")/../.."
ROOT=$(pwd)

die() { printf 'FEHLER: %s\n' "$*" >&2; exit 1; }
[ -n "$SERVER" ] || die "Aufruf: $0 root@<server-ip> [--zustand|--zustand-ueberschreiben]"
case "$MODE" in ""|--zustand|--zustand-ueberschreiben) ;; *) die "unbekannte Option $MODE" ;; esac
[ -f protocol/Cargo.toml ] && [ -f app/package.json ] || die "bitte aus dem Projektordner kaspa-lending starten."
[ -f vendor/silverscript/Cargo.toml ] || die "vendor/silverscript ist leer (git submodule update --init)."

echo "== Code nach $SERVER:$REMOTE_DIR (ohne keys/, ohne deployments/)"
ssh "$SERVER" "install -d -m 750 '$REMOTE_DIR'"
# Ausgeschlossene Pfade werden auf dem Server auch durch --delete NIE gelöscht
rsync -az --delete \
  --exclude '/keys/' \
  --exclude '/deployments/' \
  --exclude '/bin/' \
  --exclude 'node_modules/' \
  --exclude '/app/dist/' \
  --exclude 'target/' \
  --exclude '.git' \
  --exclude '/.claude/' \
  --exclude '.DS_Store' \
  --exclude '/deploy/hetzner/generated/' \
  --exclude '*.tsbuildinfo' \
  ./ "$SERVER:$REMOTE_DIR/"
ssh "$SERVER" "install -d -m 750 '$REMOTE_DIR/deployments'; if id ghost >/dev/null 2>&1; then chown -R ghost:ghost '$REMOTE_DIR'; fi"
echo "   ok"

if [ -n "$MODE" ]; then
  echo "== Zustandsdateien (enthalten keine Schlüssel)"
  if [ "$MODE" = --zustand-ueberschreiben ] && ssh "$SERVER" "systemctl is-active --quiet ghost-agent"; then
    die "ghost-agent läuft auf dem Server. Erst dort stoppen: ssh $SERVER systemctl stop ghost-agent"
  fi
  for f in deployments/mainnet.json deployments/mainnet-zins.json deployments/mainnet-tresore.json; do
    [ -f "$f" ] || { [ "$f" = deployments/mainnet.json ] && die "$f fehlt auf dem Mac."; continue; }
    grep -qiE '"(secret|mnemonic|seed|private)' "$f" && die "$f sieht nach Schlüsselmaterial aus – nicht kopiert."
    if [ "$MODE" = --zustand ] && ssh "$SERVER" "test -e '$REMOTE_DIR/$f'"; then
      echo "   $f gibt es auf dem Server schon – bleibt (ersetzen: --zustand-ueberschreiben)"
      continue
    fi
    ssh "$SERVER" "umask 027; cat > '$REMOTE_DIR/$f.upload' && mv '$REMOTE_DIR/$f.upload' '$REMOTE_DIR/$f' && if id ghost >/dev/null 2>&1; then chown ghost:ghost '$REMOTE_DIR/$f'; fi" < "$f"
    echo "   kopiert: $f"
  done
  cat <<'EOF'

   WICHTIG: Ab jetzt läuft der GHOST-Agent NUR auf dem Server.
   - Das Fenster „GHOST-Agent starten“ auf dem Mac nicht mehr öffnen.
   - Der Mac behält seine eigene deployments/mainnet.json. Vor eigenen
     Aktionen dort einmal abgleichen:  ./ghostctl sync
EOF
fi

echo
echo "Weiter auf dem Server:  ssh $SERVER 'bash $REMOTE_DIR/deploy/hetzner/setup.sh'"
echo "(Projekt: $ROOT)"
