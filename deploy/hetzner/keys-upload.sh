#!/usr/bin/env bash
# Die zwei Schlüssel für den Agenten vom Mac auf den Server laden:
#
#   deploy/hetzner/keys-upload.sh root@<server-ip>
#
#   keys/mainnet-signer.json   Unterzeichner des Orakels (--committee)
#   keys/mainnet-keeper.json   eigene Gebühren-Wallet des Agenten (--key):
#                              zahlt Netzgebühren, hält GHOST für Liquidationen
#
# Der Hauptschlüssel keys/mainnet-owner.json bleibt auf dem Mac. Das Skript
# lädt ausschließlich die zwei Dateien oben und verweigert alles, was der
# Besitzer-Schlüssel ist (auch unter anderem Namen oder als Verknüpfung).
# Auf dem Server: Ordner keys/ 700, Dateien 600, Besitzer ghost.
set -euo pipefail

SERVER=${1:-}
REMOTE_DIR=${GHOST_DIR:-/opt/ghost/kaspa-lending}
cd "$(dirname "$0")/../.."

die() { printf 'FEHLER: %s\n' "$*" >&2; exit 1; }
[ -n "$SERVER" ] || die "Aufruf: $0 root@<server-ip>"
[ $# -le 1 ] || die "nur der Server als Argument – welche Dateien hochgehen, ist fest (signer + keeper)."

OWNER=keys/mainnet-owner.json
FILES=(keys/mainnet-signer.json keys/mainnet-keeper.json)
# Geheimnis des Besitzers nur im Speicher, zum Vergleich; wird nie ausgegeben
OWNER_SECRET=""
if [ -f "$OWNER" ]; then
  OWNER_SECRET=$(tr -d ' \t\r\n' < "$OWNER" | sed -n 's/.*"secret":"\([0-9a-fA-F]\{64\}\)".*/\1/p')
fi

for f in "${FILES[@]}"; do
  [ -f "$f" ] || die "$f fehlt. Keeper anlegen: ./ghostctl keygen keys/mainnet-keeper.json, dann KAS (Gebühren) und GHOST (Liquidationen) dorthin senden."
  [ -L "$f" ] && die "$f ist eine Verknüpfung – aus Sicherheitsgründen nicht hochgeladen."
  case "$(basename "$f")" in *owner*) die "$f: Besitzer-Schlüssel werden nie hochgeladen." ;; esac
  if [ -f "$OWNER" ] && cmp -s "$f" "$OWNER"; then
    die "$f ist inhaltlich der Besitzer-Schlüssel ($OWNER) – verweigert. Für den Agenten einen eigenen Keeper anlegen."
  fi
  # auch anders formatiert: steckt das Geheimnis des Besitzers in der Datei?
  if [ -n "$OWNER_SECRET" ] && tr -d ' \t\r\n' < "$f" | grep -qiF "$OWNER_SECRET"; then
    die "$f enthält den Besitzer-Schlüssel – verweigert. Für den Agenten einen eigenen Keeper anlegen."
  fi
done
cmp -s "${FILES[0]}" "${FILES[1]}" && die "signer und keeper sind dieselbe Datei – bitte prüfen."

ssh "$SERVER" "id ghost >/dev/null 2>&1" || die "Benutzer ghost fehlt auf dem Server – erst setup.sh ausführen."
ssh "$SERVER" "install -d -m 700 -o ghost -g ghost '$REMOTE_DIR/keys'"
for f in "${FILES[@]}"; do
  # über stdin, mit umask 077: die Datei ist nie für andere lesbar
  ssh "$SERVER" "umask 077; cat > '$REMOTE_DIR/$f.upload' && chown ghost:ghost '$REMOTE_DIR/$f.upload' && chmod 600 '$REMOTE_DIR/$f.upload' && mv '$REMOTE_DIR/$f.upload' '$REMOTE_DIR/$f'" < "$f"
  echo "   hochgeladen: $f (600, ghost)"
done

if ssh "$SERVER" "test -e '$REMOTE_DIR/$OWNER'"; then
  echo
  echo "ACHTUNG: Auf dem Server liegt $OWNER! Der gehört nur auf den Mac."
  echo "         Löschen: ssh $SERVER rm '$REMOTE_DIR/$OWNER'"
fi
echo
echo "Fertig. Agent (neu) starten: ssh $SERVER systemctl restart ghost-agent"
echo "Vorher den Agenten auf dem Mac beenden – er darf nur an einer Stelle laufen."
