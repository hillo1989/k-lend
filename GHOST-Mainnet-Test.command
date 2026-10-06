#!/bin/zsh
# Doppelklick: führt den GHOST-Probelauf Schritt für Schritt aus.
# Jede Transaktion zeigt vorher die Gebühr und wartet auf deine Bestätigung (j).
# Standard: Mainnet. Zum Ausprobieren: NET=testnet-10 STATE=... KEYS=... ./GHOST-Mainnet-Test.command

cd "$(dirname "$0")" || exit 1
NET=${NET:-mainnet}
KEYS=${KEYS:-keys/$NET}
STATE=${STATE:-deployments/$NET.json}
OWNER="$KEYS-owner.json"
COMMITTEE="$KEYS-committee.json"
VAULT_KAS=${VAULT_KAS:-100}
MINT_GHOST=${MINT_GHOST:-1}
POOL_GHOST=${POOL_GHOST:-0.5}
NEED_KAS=${NEED_KAS:-155}  # 30 Deployment + 100 Vault + 4 Minter-Zweig/Token + ~12 Pool + Gebühren

G() { ./ghostctl --network "$NET" --state "$STATE" "$@"; }
pause() { read -k 1 "?$1 (Taste drücken) "; echo; }
fail() { echo "\n✗ $1"; read -k 1 "?Taste drücken zum Schließen …"; exit 1; }

echo "=== GHOST-Probelauf im Netz: $NET ==="
echo "Schlüssel: $OWNER, $COMMITTEE   Zustand: $STATE\n"

# 1. Schlüssel (werden nie überschrieben)
[ -f "$OWNER" ] || ./ghostctl keygen "$OWNER" || fail "Schlüssel konnte nicht erzeugt werden"
[ -f "$COMMITTEE" ] || ./ghostctl committee-keygen "$COMMITTEE" || fail "Komitee-Schlüssel konnten nicht erzeugt werden"

# 2. Guthaben abwarten
balance() { G balance --key "$OWNER" 2>/dev/null | awk '/^Guthaben:/ {print $2}'; }
ADDR=$(G balance --key "$OWNER" | awk '/^Adresse:/ {print $2}')
[ -n "$ADDR" ] || fail "Keine Verbindung zum Netz"
BAL=$(balance)
if [ ! -f "$STATE" ] && (( ${BAL:-0} < NEED_KAS )); then
  echo "Bitte jetzt etwa 160 KAS an diese Adresse senden:\n\n    $ADDR\n"
  echo "(Guthaben: ${BAL:-0} KAS – ich warte, bis mindestens $NEED_KAS KAS da sind …)"
  while (( ${BAL:-0} < NEED_KAS )); do sleep 10; BAL=$(balance); done
  echo "Angekommen: $BAL KAS\n"
fi

# 3. Deployment (einmalig)
if [ ! -f "$STATE" ]; then
  echo "Schritt 1/4: Orakel, Factory und GHOST (Version 2) anlegen – 3 Transaktionen, 30 KAS bleiben dauerhaft gebunden"
  G deploy --key "$OWNER" --committee "$COMMITTEE" --rate 0 || fail "Deployment fehlgeschlagen (erneuter Doppelklick setzt fort)"
else
  echo "Deployment existiert schon ($STATE) – wird übersprungen"
fi

# 4. Vault + Prägen (nur wenn noch kein eigener Vault existiert)
STATUS=$(G status) || fail "Status nicht abrufbar (Netz gestört?) – bitte später erneut versuchen"
if echo "$STATUS" | grep -q "^Vault 0:"; then
  echo "Vault 0 existiert schon – Schritt 2/3 übersprungen. Fehlt noch das Prägen: auf der Seite „Vault“ nachholen."
else
  echo "\nSchritt 2/4: Vault mit $VAULT_KAS KAS eröffnen"
  G open-vault --key "$OWNER" --kas "$VAULT_KAS" || fail "Vault eröffnen fehlgeschlagen"
  echo "\nSchritt 3/4: $MINT_GHOST GHOST prägen"
  G mint --key "$OWNER" --vault 0 --ghost "$MINT_GHOST" || fail "Prägen fehlgeschlagen"
fi

# 5. Offener Tauschpool (nur wenn noch keiner existiert): POOL_GHOST GHOST und KAS
#    im gleichen Dollarwert zum aktuellen Median-Kurs – das ist der Startkurs.
#    1 KAS und GHOST zum gleichen Kurs bleiben für immer im Pool, für den Rest
#    gibt es Anteile (pool-remove holt sie zurück). Drei Transaktionen.
HAS_POOL=$(G status --json 2>/dev/null | python3 -c 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "nein")' 2>/dev/null)
if [ "$HAS_POOL" = "nein" ]; then
  MEDIAN=$(./ghostctl --json price 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["median"])' 2>/dev/null)
  if [ -n "$MEDIAN" ]; then
    POOL_KAS=$(python3 -c 'import sys; print(max(2, round(float(sys.argv[1]) / float(sys.argv[2]), 2)))' "$POOL_GHOST" "$MEDIAN") || fail "POOL_GHOST oder Preis ist keine Zahl"
    echo "\nSchritt 4/4: Tauschpool anlegen – $POOL_KAS KAS und $POOL_GHOST GHOST (Kurs $MEDIAN USD je KAS)"
    G pool-open --key "$OWNER" --kas "$POOL_KAS" --ghost "$POOL_GHOST" || echo "Pool anlegen fehlgeschlagen – später auf der Seite „Tauschen“ nachholen."
  else
    echo "\nKein Preis abrufbar – Tauschpool bitte später auf der Seite „Tauschen“ anlegen."
  fi
fi

echo "\n=== Stand ==="
G status
G balance --key "$OWNER"
echo "\nFertig. Zurückholen: siehe MAINNET.md, Schritt 8."
read -k 1 "?Taste drücken zum Schließen …"
