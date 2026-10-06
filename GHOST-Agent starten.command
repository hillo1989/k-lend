#!/bin/zsh
# Doppelklick: betreibt den GHOST-Agenten im Mainnet, solange dieses Fenster offen ist.
#
# Alle 5 Minuten:
#  - Orakel (nur mit Unterzeichner-Datei, also beim Betreiber): Update über das
#    Unterzeichner-Register, wenn sich der KAS-Preis (Median aus 6 Quellen) um
#    mindestens 0,5 % bewegt hat, spätestens aber nach 60 Minuten (halbe
#    Einfrier-Frist von 2 h, Version 4). 0,5 % liegt unter der Rücknahmegebühr
#    von 1 %: So lohnt sich eine Rücknahme gegen ein leicht nachlaufendes Orakel
#    nicht (Audit 11 A11-V-2).
#  - Einfrieren (auch ohne Unterzeichner-Datei): Ist der Preis älter als die
#    Frist, friert der Agent das Orakel ein. Dann sind Prägen, Einlösen,
#    Liquidieren und Tauschen gesperrt, bis wieder ein Preis kommt.
#  - Zins (nur mit Komitee-Datei): je Runde eine Messung des GHOST-Kurses im
#    frisch abgeglichenen Pool; entschieden wird nach dem Median der Messungen
#    der letzten Stunde, frühestens ab 6 Messungen und nur, wenn der Pool
#    mindestens 10 GHOST hält. Liegt der Median unter 0,995 USD, steigt der Zins
#    um 0,5 Prozentpunkte (Schulden werden teurer, GHOST werden zurückgekauft und
#    getilgt); über 1,005 USD sinkt er um 0,5 Punkte. Rahmen 0–20 % p. a.,
#    höchstens einmal pro Stunde – auch über Neustarts hinweg (Messungen und
#    letzte Änderung in deployments/<netz>-zins.json).
#  - Liquidationen: Vaults unter 150 % werden abgelöst, aber nur, wenn auch der
#    aktuelle Marktpreis sie als unterdeckt zeigt, nie mit Verlust und höchstens
#    mit den GHOST, die der Schlüssel hat.
#  - Auflösen (auch ohne Komitee-Datei): ein Vault ohne Schuld, dessen Zins die
#    Sicherheit aufzehrt (nach Orakel- und Marktpreis), geht bis auf 0,1 KAS an
#    die Zinsadresse. Die 0,1 KAS tragen die Netzgebühr (etwa 0,055 KAS), den Rest
#    (etwa 0,045 KAS) bekommt der Keeper als Wechselgeld. Er braucht dafür aber
#    selbst eine KAS-UTXO als Eingang; ohne KAS löst er nicht auf (Audit 12 A12-17).
#  - Daueraufträge (deployments/<netz>-abos.json, anlegen mit `ghostctl abo add`
#    oder auf der Seite „Wallet“): fällige werden gesendet, verpasste einmal
#    nachgeholt. Jeder Auftrag zahlt mit seiner eigenen Schlüsseldatei.
#  - Tresore (auch ohne Komitee-Datei): fällige Zahlungen eigener und
#    übernommener Tresore werden ausgelöst.
#  Liquidationen, Auflösen, Daueraufträge und Tresore laufen auch ohne
#  Komitee-Datei; nur Orakel und Zins brauchen sie.
#
# Jede Transaktion wird ohne Rückfrage gesendet (--ja). Beenden: Fenster
# schließen oder Ctrl+C. Umgebungsvariablen wie beim Probelauf: NET, KEYS, STATE.
cd "$(dirname "$0")" || exit 1
NET=${NET:-mainnet}
KEYS=${KEYS:-keys/$NET}
STATE=${STATE:-deployments/$NET.json}
# Version 4: Unterzeichner-Datei (ghostctl committee-keygen, beim Umzug angelegt);
# ältere Komitee-Datei nur, wenn es keine gibt
COMMITTEE="$KEYS-signer.json"
[ -f "$COMMITTEE" ] || COMMITTEE="$KEYS-committee.json"
# Eigener Keeper-Schlüssel, falls vorhanden: der Agent verbrennt dessen GHOST
# automatisch. Ohne ihn nimmt er den Besitzer-Schlüssel – dann sind auch GHOST,
# die zum Tilgen des eigenen Vaults gedacht waren, für Liquidationen frei (A10-A-12).
if [ -f "$KEYS-keeper.json" ]; then
  KEY="$KEYS-keeper.json"
else
  KEY="$KEYS-owner.json"
  echo "Hinweis: kein $KEYS-keeper.json – der Agent nutzt $KEY und darf dessen GHOST für Liquidationen verbrennen."
  echo "Eigenen Keeper-Schlüssel anlegen: ./ghostctl keygen $KEYS-keeper.json, dann KAS und etwas GHOST dorthin senden.\n"
fi
ARGS=(--key "$KEY" --interval 300 --min-change 0.005 --max-age-min 60)
if [ ! -f "$STATE" ]; then
  echo "Keine Zustandsdatei $STATE. Beim Betreiber anlegen lassen oder von dort kopieren (enthält keine Schlüssel)."
  read -k 1 "?Taste drücken zum Schließen …"; exit 1
fi
if [ -f "$COMMITTEE" ]; then
  ARGS+=(--committee "$COMMITTEE")
  echo "GHOST-Agent ($NET): Orakel (Unterzeichner $COMMITTEE), Zinsregel, Liquidationen, Auflösen, Daueraufträge und Tresore. Fenster offen lassen; Mac nicht in den Ruhezustand."
else
  echo "GHOST-Agent ($NET): Liquidationen, Auflösen, Daueraufträge und Tresore – ohne Unterzeichner-Datei keine Preise und keine Zinsregel (Einfrieren geht trotzdem). Fenster offen lassen."
  echo "Orakel-Updates und Vaults anderer führt er selbst von der Kette nach (REST-API + Node)."
  echo "Auf einem anderen Rechner genügt einmal eine Kopie von $STATE (enthält keine Schlüssel).\n"
fi
echo "Für Liquidationen braucht $KEY eigene GHOST und etwas KAS für Gebühren, zum Auflösen von Zins-Vaults etwas KAS."
if [ -f "deployments/$NET-abos.json" ]; then
  echo "Daueraufträge aus deployments/$NET-abos.json werden in jeder Runde ausgeführt.\n"
else
  echo "Daueraufträge: keine angelegt (ghostctl abo add oder Seite „Wallet“).\n"
fi
# caffeinate verhindert den Ruhezustand; nach einem Absturz startet der Agent
# nach 30 s neu, bis das Fenster geschlossen oder Ctrl+C gedrückt wird.
while true; do
  caffeinate -i ./ghostctl --network "$NET" --state "$STATE" --ja agent $ARGS
  echo "[$(date '+%H:%M:%S')] Agent beendet (Code $?) – Neustart in 30 s. Ctrl+C beendet."
  sleep 30
done
