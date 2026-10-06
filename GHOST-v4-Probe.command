#!/bin/zsh
# Doppelklick: Mainnet-Probe von GHOST Version 4 mit Kleinstbeträgen.
#
# Version 3 läuft dabei unberührt weiter: eigene Zustandsdatei
# (deployments/mainnet-v4probe.json), eigener Schlüssel (keys/v4probe-owner.json),
# eigenes Programm (aus dem Ordner kaspa-lending-v4 gebaut).
#
# Drei Stufen, das Skript erkennt selbst, welche dran ist. Zwischen den Stufen
# liegt je mindestens 1 Stunde (Fristen der Probe):
#   Stufe 1  Schlüssel anlegen, 25 KAS von keys/mainnet-owner.json an den
#            Probe-Schlüssel, Deployment (Register, Orakel, Register-Init,
#            Factory, GHOST), Preis-Update über das Register, Vault mit 10 KAS,
#            0,1 GHOST prägen
#   Stufe 2  (≥ 1 h ohne Preis) Orakel einfrieren, prüfen dass Prägen gesperrt
#            ist, Preis-Update taut auf, Austausch der Unterzeichner ankündigen
#            (Unterzeichner 1 → Unterzeichner 2)
#   Stufe 3  (≥ 1 h nach der Ankündigung) Austausch aktivieren – hier prüft der
#            Node die relative Sperre –, alter Schlüssel kann keine Preise mehr
#            setzen, neuer schon; tilgen, Vault schließen, Rest-KAS zurück an
#            keys/mainnet-owner.json
#
# Standard: EINMAL bestätigen, dann läuft alles von selbst. Das Skript wartet,
# bis die nächste Stufe dran ist (prüft alle 5 Minuten), und hält den Mac mit
# caffeinate wach. Das Fenster muss etwa 2½ Stunden offen bleiben. Scheitert
# ein Schritt vorübergehend, versucht es es nach 5 Minuten erneut; nach drei
# Fehlschlägen hintereinander hält es an. Erneut doppelklicken setzt fort.
# EINZELN=1 ./GHOST-v4-Probe.command: nur die fällige Stufe, mit eigener Frage.
# Dauerhaft gebunden bleiben etwa 7 KAS (4 Covenants je 1 KAS, 3 KAS
# Minter-Zweig des Vaults), dazu Gebühren von etwa 0,5 KAS.
# Probelauf ohne Senden: DRY=1 ./GHOST-v4-Probe.command (nur Stufe 1 sinnvoll).

V4="$(cd "$(dirname "$0")" && pwd)"
MAIN="$(cd "$V4/../kaspa-lending" 2>/dev/null && pwd)"
NET=mainnet
STATE=deployments/$NET-v4probe.json
STAGE_FILE=deployments/$NET-v4probe.stufe
PKEY=keys/v4probe-owner.json
SIGNER1=keys/v4probe-signer1.json
SIGNER2=keys/v4probe-signer2.json
OWNER=keys/mainnet-owner.json
FUND=${FUND:-25}
VAULT_KAS=10
MINT_GHOST=0.1
BIN="$V4/vendor/silverscript/target/release/ghostctl"

# INNER=1: eine Stufe im Auftrag der Warteschleife (keine Fragen, kein Tastendruck)
fail() { echo "\n✗ $1" >&2; [ -n "$INNER" ] && exit 1; read -k 1 "?Taste drücken zum Schließen …"; exit 1; }
# Stufe noch nicht dran: die Warteschleife wartet (Exit 2), von Hand ist es ein Abbruch
zufrueh() { [ -n "$INNER" ] && { echo "→ $1"; exit 2; }; fail "$1"; }
schluss() { [ -n "$INNER" ] || read -k 1 "?Taste drücken zum Schließen …"; }
py() { python3 -c "$@"; }

case "${DRY:-0}" in
  1) DRY_RUN=1 ;;
  0) DRY_RUN=0 ;;
  *) echo "DRY=$DRY verstehe ich nicht: DRY=1 ist ein Probelauf."; exit 1 ;;
esac

[ -n "$MAIN" ] || fail "Ordner kaspa-lending neben kaspa-lending-v4 nicht gefunden"
echo "=== GHOST v4 – Mainnet-Probe$( (( DRY_RUN )) && echo ' (PROBELAUF, nichts wird gesendet)') ==="
if [ -z "$INNER" ]; then
  echo "Baue ghostctl v4 …"
  (cd "$V4/protocol" && cargo build --release -q --bin ghostctl) || fail "Bauen fehlgeschlagen"
fi
cd "$MAIN" || fail "$MAIN nicht erreichbar"
[ -f "$OWNER" ] || fail "$OWNER fehlt"

JA=(--ja)
DRYF=()
(( DRY_RUN )) && { JA=(); DRYF=(--dry-run); }
# v4-Programm, immer mit der Probe-Zustandsdatei (nie deployments/mainnet.json)
g4() { "$BIN" --network $NET --state "$STATE" "${DRYF[@]}" "${JA[@]}" "$@"; }
g4check() { "$BIN" --network $NET --state "$STATE" --dry-run "$@" 2>&1; }
# nur ein frisch abgeglichener Status zählt (Audit 15 G-10: status --json meldet
# einen gescheiterten Abgleich nur als oracle.fresh = false)
status_field() { "$BIN" --network $NET --state "$STATE" status --json 2>/dev/null | py "import json,sys
d=json.load(sys.stdin)
if not d.get('oracle', {}).get('fresh', False): sys.exit(1)
print($1)" 2>/dev/null; }
# wie status_field, bricht aber ab, wenn der Status nicht lesbar ist (Audit 14 M1)
# Ein nicht lesbarer Status ist meist eine kurze Störung von Node oder REST-API
# (Probe 05.10.2026, 13:49–14:00): für die Warteschleife Exit 3 = später erneut,
# nicht als Fehlschlag zählen
must() {
  local v
  v=$(status_field "$1")
  if [ -z "$v" ]; then
    [ -n "$INNER" ] && { echo "→ Status gerade nicht lesbar (Node/REST) – später erneut" >&2; exit 3; }
    fail "Status nicht lesbar (Node erreichbar?) – einfach erneut starten, es setzt fort"
  fi
  print -r -- "$v"
}
kas_of() { "$BIN" --network $NET --state "$STATE" --json balance --key "$1" 2>/dev/null | py 'import json,sys; print(json.load(sys.stdin).get("kas",0))'; }

frage() {
  (( DRY_RUN )) && return 0
  [ -n "$INNER" ] && return 0
  read -r "a?Alles so ausführen? [j/N] "
  [ "$a" = "j" ] || fail "abgebrochen – nichts weiter gesendet"
}

# ----------------------------------------------------- Warteschleife (Standard)
if [ -z "$INNER" ] && (( ! DRY_RUN )) && [ "${EINZELN:-0}" != 1 ]; then
  st=1
  [ -f "$STAGE_FILE" ] && st=$(cat "$STAGE_FILE")
  [ "$st" = done ] && { echo "Die Probe ist schon abgeschlossen."; schluss; exit 0; }
  echo "\nAlles läuft automatisch nacheinander (jetzt bei Stufe $st):"
  echo "  Stufe 1  25 KAS an den Probe-Schlüssel, Deployment v4 (Probe), Preis, Vault mit $VAULT_KAS KAS, $MINT_GHOST GHOST prägen"
  echo "  Stufe 2  (ab 1 h nach dem letzten Preis) einfrieren, Sperre prüfen, auftauen, Austausch ankündigen,"
  echo "           zu frühe Aktivierung (der Node muss ablehnen)"
  echo "  Stufe 3  (ab 1 h nach der Ankündigung) aktivieren, alter Schlüssel abgewiesen, Preis mit dem neuen,"
  echo "           tilgen, Vault schließen, Rest-KAS zurück an $OWNER"
  echo "  Dauer etwa 2½ Stunden. Fenster offen lassen; der Mac bleibt so lange wach."
  echo "  Dauerhaft gebunden bleiben etwa 7 KAS, dazu Gebühren von etwa 0,5 KAS."
  frage
  caffeinate -i -w $$ >/dev/null 2>&1 &
  fehler=0
  stoerung=0
  while true; do
    st=1
    [ -f "$STAGE_FILE" ] && st=$(cat "$STAGE_FILE")
    [ "$st" = done ] && break
    echo "\n[$(date +%H:%M)] Stufe $st …"
    INNER=1 "$V4/GHOST-v4-Probe.command"
    rc=$?
    case $rc in
      0) fehler=0; stoerung=0 ;;
      2) stoerung=0; echo "[$(date +%H:%M)] warte 5 Minuten …"; sleep 300 ;;
      3)
        stoerung=$((stoerung + 1))
        (( stoerung >= 12 )) && fail "Der Status war eine Stunde lang nicht lesbar (Node/REST-API). Bitte später erneut doppelklicken; es setzt fort."
        echo "[$(date +%H:%M)] Störung $stoerung von 12 – neuer Versuch in 5 Minuten"
        sleep 300
        ;;
      *)
        fehler=$((fehler + 1))
        (( fehler >= 3 )) && fail "Dreimal hintereinander fehlgeschlagen (Meldungen oben). Bitte Claude die Ausgabe zeigen; erneut doppelklicken setzt fort."
        echo "[$(date +%H:%M)] Fehlschlag $fehler von 3 – neuer Versuch in 5 Minuten"
        sleep 300
        ;;
    esac
  done
  echo "\n✓ Probe abgeschlossen. Bitte Claude Bescheid geben."
  schluss
  exit 0
fi

stufe=1
[ -f "$STAGE_FILE" ] && stufe=$(cat "$STAGE_FILE")

# ------------------------------------------------------------------ Stufe 1
if [ "$stufe" = 1 ]; then
  for f in "$PKEY"; do [ -f "$f" ] || "$BIN" keygen "$f" >/dev/null || fail "Schlüssel $f"; done
  for f in "$SIGNER1" "$SIGNER2"; do [ -f "$f" ] || "$BIN" committee-keygen "$f" --count 1 >/dev/null || fail "Schlüssel $f"; done
  have=$(kas_of "$PKEY")
  [ -n "$have" ] || fail "Guthaben von $PKEY nicht lesbar (Node erreichbar?)"
  echo "\nStufe 1 von 3:"
  echo "  1. ${FUND} KAS von $OWNER an $PKEY senden (dort jetzt: ${have:-?} KAS; entfällt ab 20 KAS)"
  echo "  2. Deployment v4 als Probe: Register, Orakel, Register-Init, Factory, GHOST"
  echo "     (1 Unterzeichner = $SIGNER1, Fristen 1 h, höchstens 5 GHOST je Vault, 4 KAS dauerhaft gebunden)"
  echo "  3. Preis-Update über das Register"
  echo "  4. Vault mit $VAULT_KAS KAS eröffnen (+3 KAS Minter-Zweig, dauerhaft) und $MINT_GHOST GHOST prägen"
  echo "  Version 3 und dein Agent bleiben unberührt."
  frage
  # Fortsetzen nach einem Abbruch: Erledigtes wird übersprungen
  if [ -f "$STATE" ] || py "import sys; sys.exit(0 if float('${have:-0}' or 0) >= 20 else 1)"; then
    echo "→ Probe-Schlüssel hat genug KAS"
  else
    (( DRY_RUN )) && fail "Probelauf: Der Probe-Schlüssel hat noch keine KAS – ab hier geht ein Probelauf nicht weiter."
    ./ghostctl --network $NET --ja send --key "$OWNER" --to "$PKEY" --kas "$FUND" || fail "Senden an den Probe-Schlüssel fehlgeschlagen (läuft gerade ein Orakel-Update des Agenten? Einfach erneut starten)"
    sleep 5
  fi
  if [ ! -f "$STATE" ]; then
    g4 deploy --key "$PKEY" --committee "$SIGNER1" --probe || fail "Deployment abgebrochen – einfach erneut starten, es setzt fort"
    (( DRY_RUN )) && { echo "\nProbelauf fertig."; exit 0; }
  fi
  seq=$(must 'd["oracle"]["seq"]') || exit $?
  if [ "$seq" = 0 ]; then
    echo "→ warte gut 1 Minute (Mindestabstand des Orakels) …"
    sleep 75
    g4 oracle-update --key "$PKEY" --committee "$SIGNER1" || fail "Preis-Update fehlgeschlagen – erneut starten setzt fort"
  fi
  nv=$(must 'len(d["vaults"])') || exit $?
  if [ "$nv" = 0 ]; then
    g4 open-vault --key "$PKEY" --kas $VAULT_KAS || fail "Vault eröffnen fehlgeschlagen – erneut starten setzt fort"
  fi
  debt=$(must 'd["vaults"][0]["debtGhost"]') || exit $?
  if [ "$debt" = "0.0" ]; then
    g4 mint --key "$PKEY" --vault 0 --ghost $MINT_GHOST || fail "Prägen fehlgeschlagen – erneut starten setzt fort"
  fi
  # Audit 15 G-3: must immer über eine Variable, sonst bricht ein Fehler nicht ab
  debt=$(must 'd["vaults"][0]["debtGhost"]') || exit $?
  [ "$debt" = "$MINT_GHOST" ] || fail "Vault hat nicht die erwartete Schuld von $MINT_GHOST GHOST – bitte Claude fragen"
  (( DRY_RUN )) || echo 2 > "$STAGE_FILE"
  g4 signers show
  echo "\n✓ Stufe 1 fertig. Stufe 2 frühestens in 1 Stunde: dann wieder doppelklicken."
  schluss
  exit 0
fi

# ------------------------------------------------------------------ Stufe 2
if [ "$stufe" = 2 ] || [ "$stufe" = 2b ]; then
  rot=$(must 'd["signers"]["rotation"] is not None') || exit $?
  if [ "$rot" = "False" ]; then
    frozen=$(must 'd["oracle"].get("frozen")') || exit $?
    left=$(must 'd["oracle"]["freezeInMinutes"]') || exit $?
    if [ "$stufe" = 2 ] && [ "$frozen" = "False" ] && ! py "import sys; sys.exit(0 if float('$left') <= 0 else 1)"; then
      zufrueh "Noch zu früh: Einfrieren geht erst in $(py "print(round(float('$left')))") Minuten."
    fi
    echo "\nStufe 2 von 3:"
    echo "  1. Orakel einfrieren (Frist ohne Preis abgelaufen)"
    echo "  2. prüfen, dass Prägen jetzt gesperrt ist (Vorprüfung von ghostctl, nichts gesendet)"
    echo "  3. Preis-Update taut das Orakel wieder auf"
    echo "  4. Austausch ankündigen: Unterzeichner 1 → $SIGNER2 (1 KAS Ticket, kommt beim Aktivieren zurück)"
    echo "  5. sofort zu früh aktivieren – der Node muss das ablehnen (kostet nichts)"
    frage
    # 2b: Einfrieren und Auftauen sind erledigt, nur die Ankündigung fehlt noch
    if [ "$stufe" = 2 ]; then
      if [ "$frozen" = "False" ]; then
        g4 oracle-freeze --key "$PKEY" || fail "Einfrieren fehlgeschlagen"
      fi
      frozen=$(must 'd["oracle"].get("frozen")') || exit $?
      [ "$frozen" = "True" ] || fail "Orakel ist nach dem Einfrieren nicht als eingefroren gemeldet"
      out=$(g4check mint --key "$PKEY" --vault 0 --ghost 0.01)
      echo "$out" | grep -q "eingefroren" && echo "✓ Prägen ist gesperrt" || fail "Prägen war NICHT gesperrt:\n$out"
      g4 oracle-update --key "$PKEY" --committee "$SIGNER1" || fail "Preis-Update (Auftauen) fehlgeschlagen – erneut starten setzt fort"
      frozen=$(must 'd["oracle"].get("frozen")') || exit $?
      [ "$frozen" = "False" ] || fail "Orakel ist nach dem Preis-Update noch eingefroren"
      echo "✓ aufgetaut"
      (( DRY_RUN )) || echo 2b > "$STAGE_FILE"
    fi
    g4 signers propose --key "$PKEY" --committee "$SIGNER1" --new-committee "$SIGNER2" || fail "Ankündigung fehlgeschlagen – erneut starten setzt fort"
  else
    echo "→ Ankündigung ist schon gesendet"
    frage
  fi
  # Konsens-Probe: die relative Sperre muss der Node selbst durchsetzen (Audit 14 M4).
  # Nur solange die Wartezeit sicher noch läuft (Audit 15 G-1); als Erfolg zählt
  # nur die Sperr-Meldung des Nodes, nicht jeder Fehler (Audit 15 G-2).
  ready=$(must '(d["signers"]["rotation"] or {}).get("readyInHours", -1)') || exit $?
  if (( DRY_RUN )); then
    echo "→ Konsens-Probe im Probelauf übersprungen"
  elif py "import sys; sys.exit(0 if float('$ready') > 0.25 else 1)"; then
    out=$(g4 signers activate --key "$PKEY" 2>&1)
    rc=$?
    echo "$out" | tail -3
    if [ $rc -eq 0 ]; then
      fail "Der Node hat die zu frühe Aktivierung ANGENOMMEN – die Wartezeit wird nicht erzwungen! Bitte sofort Claude Bescheid geben."
    elif echo "$out" | grep -qi "sequence lock"; then
      echo "✓ zu frühe Aktivierung vom Node abgelehnt (relative Sperre)"
    else
      fail "Die Probe-Aktivierung scheiterte aus einem anderen Grund (siehe oben) – erneut starten; bleibt es so, Claude die Meldung zeigen."
    fi
  else
    echo "→ Konsens-Probe übersprungen: die Wartezeit ist fast oder ganz um"
  fi
  (( DRY_RUN )) || echo 3 > "$STAGE_FILE"
  echo "\n✓ Stufe 2 fertig. Stufe 3 frühestens in 1 Stunde: dann wieder doppelklicken."
  schluss
  exit 0
fi

# ------------------------------------------------------------------ Stufe 3
if [ "$stufe" = 3 ] || [ "$stufe" = 3b ]; then
  rot=$(must 'd["signers"]["rotation"] is not None') || exit $?
  if [ "$rot" = "True" ]; then
    left=$(must 'd["signers"]["rotation"]["readyInHours"]') || exit $?
    if ! py "import sys; sys.exit(0 if float('$left') <= 0 else 1)"; then
      zufrueh "Noch zu früh: Aktivieren geht in etwa $(py "print(max(1, round(float('$left')*60)))") Minuten."
    fi
  fi
  echo "\nStufe 3 von 3:"
  echo "  1. Austausch aktivieren (der Node prüft die Wartezeit)"
  echo "  2. prüfen, dass Unterzeichner 1 keine Preise mehr setzen kann (Vorprüfung von ghostctl)"
  echo "  3. Preis-Update mit Unterzeichner 2"
  echo "  4. Schuld tilgen, Vault schließen (KAS zurück an den Probe-Schlüssel)"
  echo "  5. Rest-KAS des Probe-Schlüssels zurück an $OWNER"
  frage
  # 3b: Austausch und Preis mit Unterzeichner 2 sind erledigt (Audit 15 G-8)
  if [ "$stufe" = 3 ]; then
    if [ "$rot" = "True" ]; then
      g4 signers activate --key "$PKEY" || fail "Aktivieren fehlgeschlagen (zu früh? in ein paar Minuten erneut starten)"
    fi
    out=$(g4check oracle-update --key "$PKEY" --committee "$SIGNER1")
    echo "$out" | grep -q "nötigen Schlüsseln" && echo "✓ alter Unterzeichner abgewiesen" || fail "Alter Unterzeichner wurde NICHT abgewiesen:\n$out"
    seq_before=$(must 'd["oracle"]["seq"]') || exit $?
    g4 oracle-update --key "$PKEY" --committee "$SIGNER2" || fail "Preis-Update mit Unterzeichner 2 fehlgeschlagen – erneut starten setzt fort"
    seq_after=$(must 'd["oracle"]["seq"]') || exit $?
    [ "$seq_after" -gt "$seq_before" ] || fail "Preis-Update mit Unterzeichner 2 nicht sichtbar"
    (( DRY_RUN )) || echo 3b > "$STAGE_FILE"
  fi
  nv=$(must 'len(d["vaults"])') || exit $?
  if [ "$nv" != 0 ]; then
    debt=$(must 'd["vaults"][0]["debtGhost"]') || exit $?
    if [ "$debt" != "0.0" ]; then
      g4 repay --key "$PKEY" --vault 0 || fail "Tilgen fehlgeschlagen – erneut starten setzt fort"
    fi
    g4 close --key "$PKEY" --vault 0 || fail "Schließen fehlgeschlagen – erneut starten setzt fort"
  fi
  rest=$(kas_of "$PKEY")
  [ -n "$rest" ] || fail "Guthaben nicht lesbar – Rücküberweisung nicht gesendet; erneut starten (Audit 14 M2)"
  # fast alles zurück: Rest unter 0,2 KAS wäre kein eigener Ausgang und ginge
  # ganz an die Miner (Probe 05.10.2026: 0,2 KAS so verloren); 0,005 KAS deckt die Gebühr
  back=$(py "print(f'{max(0, float(\"$rest\") - 0.005):.8f}')")
  if py "import sys; sys.exit(0 if float('$back') > 0.3 else 1)"; then
    g4 send --key "$PKEY" --to "$OWNER" --kas "$back" || fail "Rücküberweisung fehlgeschlagen – erneut starten"
    echo "✓ $back KAS zurück an $OWNER"
  fi
  (( DRY_RUN )) || echo done > "$STAGE_FILE"
  g4 signers show
  [ -n "$INNER" ] || echo "\n✓ Probe abgeschlossen. Bitte Claude Bescheid geben."
  schluss
  exit 0
fi

echo "Die Probe ist schon abgeschlossen ($STAGE_FILE = $stufe)."
schluss
