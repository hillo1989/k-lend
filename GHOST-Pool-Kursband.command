#!/bin/zsh
# Doppelklick: ersetzt den ersten Tauschpool (ohne Kursband) durch einen Pool
# mit Kursband 1 USD ± 3 %.
#   1. deine Anteile aus dem alten Pool abziehen (100 %)
#   2. neuen Pool anlegen: alle verfügbaren GHOST und die dazu passenden KAS
#      zum Orakelkurs (Startkurs 1 USD je GHOST), 4 KAS bleiben als Puffer
# Jede Transaktion zeigt vorher die Gebühr und wartet auf deine Bestätigung (j).
# Probelauf ohne Senden: DRY=1 ./GHOST-Pool-Kursband.command

cd "$(dirname "$0")" || exit 1
NET=${NET:-mainnet}
KEYS=${KEYS:-keys/$NET}
STATE=${STATE:-deployments/$NET.json}
KEY="$KEYS-owner.json"
RESERVE_KAS=${RESERVE_KAS:-4}
DRYFLAG=()
[ -n "$DRY" ] && DRYFLAG=(--dry-run)

G() { ./ghostctl --network "$NET" --state "$STATE" $DRYFLAG "$@"; }
fail() { echo "\n✗ $1"; [ -z "$DRY" ] && read -k 1 "?Taste drücken zum Schließen …"; exit 1; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

echo "=== Tauschpool mit Kursband ($NET)${DRY:+ – PROBELAUF, nichts wird gesendet} ==="
[ -f "$KEY" ] || fail "Schlüsseldatei $KEY fehlt"

STATUS=$(./ghostctl --network "$NET" --state "$STATE" status --json 2>/dev/null) || fail "Status nicht abrufbar (Nodes?) – später erneut"
BAND=$(echo "$STATUS" | json '"keiner" if not d.get("pool") else ("ja" if d["pool"].get("bandBps") else "nein")')
KAS_USD=$(echo "$STATUS" | json 'd["oracle"]["kasUsd"]')
[ "$BAND" = "ja" ] && { echo "Der Pool hat schon ein Kursband – nichts zu tun."; [ -z "$DRY" ] && read -k 1 "?Taste drücken zum Schließen …"; exit 0; }

# 1. Anteile aus dem alten Pool abziehen
if [ "$BAND" = "nein" ]; then
  SHARES=$(./ghostctl --network "$NET" --state "$STATE" --json keys 2>/dev/null | python3 -c "
import json,sys; d=json.load(sys.stdin)
ks=d['keys'] if isinstance(d,dict) else d
print(next((k.get('lpShares','0') for k in ks if k.get('file','').endswith('$KEY'.split('/')[-1])),'0'))")
  if [ "${SHARES:-0}" != "0" ]; then
    echo "\nSchritt 1/2: deine $SHARES Anteile aus dem alten Pool abziehen"
    if [ -n "$DRY" ]; then
      # Probelauf: Ausgabe einfangen, erwartete Rückflüsse für die Planung von Schritt 2
      OUT=$(G pool-remove --key "$KEY" --percent 100 2>&1) || { echo "$OUT"; fail "Abziehen fehlgeschlagen"; }
      echo "$OUT"
      BACK_KAS=$(echo "$OUT" | sed -n 's/.*Anteile → \([0-9.]*\) KAS und \([0-9.]*\) GHOST.*/\1/p')
      BACK_GHOST=$(echo "$OUT" | sed -n 's/.*Anteile → \([0-9.]*\) KAS und \([0-9.]*\) GHOST.*/\2/p')
    else
      # echt: direkt im Fenster, damit die Rückfrage [j/N] sichtbar ist
      G pool-remove --key "$KEY" --percent 100 || fail "Abziehen fehlgeschlagen"
    fi
  else
    echo "Schritt 1/2: keine eigenen Anteile im alten Pool – übersprungen"
  fi
fi

# 2. Beträge für den neuen Pool: alle GHOST, KAS dazu zum Orakelkurs
KAS=$(./ghostctl --network "$NET" --state "$STATE" balance --key "$KEY" 2>/dev/null | awk '/^Guthaben:/ {print $2}')
GHOST=$(./ghostctl --network "$NET" --state "$STATE" balance --key "$KEY" 2>/dev/null | awk '/^GHOST/ {print $NF}')
if [ -n "$DRY" ]; then
  KAS=$(python3 -c 'import sys; print(float(sys.argv[1]) + float(sys.argv[2] or 0))' "${KAS:-0}" "${BACK_KAS:-0}")
  GHOST=$(python3 -c 'import sys; print(float(sys.argv[1]) + float(sys.argv[2] or 0))' "${GHOST:-0}" "${BACK_GHOST:-0}")
fi
PLAN=$(python3 -c '
import sys
kas, ghost, price, reserve = map(float, sys.argv[1:5])
g = min(ghost, max(0.0, kas - reserve) * price)
k = g / price if price > 0 else 0
print(f"{k:.8f} {g:.8f}")' "${KAS:-0}" "${GHOST:-0}" "$KAS_USD" "$RESERVE_KAS") || fail "Beträge nicht berechenbar"
POOL_KAS=${PLAN% *}
POOL_GHOST=${PLAN#* }

echo "\nSchritt 2/2: neuen Pool anlegen – $POOL_KAS KAS und $POOL_GHOST GHOST"
echo "  Startkurs: 1 GHOST = 1 USD (Orakel $KAS_USD USD je KAS), Guthaben $KAS KAS / $GHOST GHOST, Puffer $RESERVE_KAS KAS"
python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) >= 1 and float(sys.argv[2]) > 0 else 1)' "$POOL_KAS" "$POOL_GHOST" || fail "Zu wenig KAS oder GHOST für einen Pool (mindestens 1 KAS)"
if [ -n "$DRY" ]; then
  echo "  (Probelauf: Schritt 2 wird erst nach dem echten Abziehen gebaut)"
else
  G pool-open --key "$KEY" --kas "$POOL_KAS" --ghost "$POOL_GHOST" || fail "Pool anlegen fehlgeschlagen (erneuter Doppelklick setzt fort)"
fi

echo "\n=== Fertig ==="
./ghostctl --network "$NET" --state "$STATE" status 2>/dev/null | grep -E "^Tauschpool|^Orakel"
[ -z "$DRY" ] && read -k 1 "?Taste drücken zum Schließen …"
exit 0
