#!/bin/zsh
# Doppelklick: startet die Lending-Seite (Live-Daten über ghostctl) und öffnet sie im Browser.
# Beenden: dieses Terminal-Fenster schließen oder Ctrl+C drücken.

cd "$(dirname "$0")/app" || exit 1
PORT=5180
URL="http://localhost:$PORT/"

# In Brave öffnen, wenn installiert (Wallet-Erweiterungen wie KasWare laufen
# dort wie in Chrome), sonst im Standardbrowser.
open_page() {
  if [ -d "/Applications/Brave Browser.app" ]; then open -a "Brave Browser" "$URL"; else open "$URL"; fi
}

# Läuft die Seite schon (z. B. aus einem früheren Doppelklick)? Dann nur öffnen.
if curl -s -o /dev/null "$URL"; then
  echo "Die Seite läuft bereits: $URL"
  open_page
  exit 0
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "Node.js/npm wurde nicht gefunden. Bitte Node.js installieren: https://nodejs.org"
  read -k 1 "?Taste drücken zum Schließen …"
  exit 1
fi

# Beim ersten Start fehlen die Pakete
if [ ! -d node_modules ]; then
  echo "Erster Start: installiere Pakete (einmalig, dauert etwa eine Minute) …"
  npm install || { read -k 1 "?Fehler bei npm install. Taste drücken …"; exit 1; }
fi

# Browser öffnen, sobald der Server antwortet
( until curl -s -o /dev/null "$URL"; do sleep 0.5; done; open_page ) &

echo "Starte die Seite auf $URL"
echo "Zum Beenden dieses Fenster schließen oder Ctrl+C drücken."
npm run dev -- --port $PORT --strictPort
