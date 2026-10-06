#!/bin/zsh
# Doppelklick: Notfallsatz für das Unterzeichner-Register von GHOST (Mainnet)
# nachtragen (Entscheidung des Betreibers nach Audit 20, A20e-2/A20a-7).
#
# Warum: Live gibt es einen einzigen Unterzeichner (keys/mainnet-signer.json)
# und KEINEN Notfallsatz. Geht dieser Schlüssel verloren, friert das Orakel
# nach 2 h ein und bleibt es für immer (Prägen, Rücknahme, Liquidation und
# Pool-Tausch gesperrt). Ein Notfallsatz mit einem kalt gelagerten Schlüssel
# kann dann – und nur dann – einen neuen Unterzeichner einsetzen.
#
# Was das Skript tut (es erkennt selbst, was dran ist; erneut doppelklicken
# setzt fort):
#   Schritt 1  Neuen Notfall-Schlüssel erzeugen (`ghostctl committee-keygen
#              --count 1`) in einem Ordner, den du angibst – Standard: Ordner
#              GHOST-Notfall auf einem USB-Stick, NIE im Projekt und nie auf
#              dem Server. Danach Anleitung zur Offline-Sicherung. Der
#              öffentliche Schlüssel (nicht geheim) wird zusätzlich in
#              deployments/mainnet-notfall.pub gemerkt, damit ein späterer
#              Lauf weiß, welcher Schlüssel gemeint ist.
#   Schritt 2  Ankündigung im Register: GLEICHER Hauptsatz (der bisherige
#              Unterzeichner, Schwellen unverändert, `--same-set`) + NEUER
#              Notfallsatz (1 von 1, der Notfall-Schlüssel). Signiert vom
#              aktuellen Satz (keys/mainnet-signer.json), Gebühr und 1 KAS
#              Ticket von keys/mainnet-owner.json (die 1 KAS kommt beim
#              Aktivieren zurück). Erst ein Probelauf (nichts gesendet), dann
#              sendet das Skript NUR nach deiner ausdrücklichen Bestätigung.
#   Schritt 3  Nach der Wartezeit (14 Tage) aktivieren – auch das erkennt das
#              Skript und bietet es an (Probelauf, dann Bestätigung). Bis zur
#              Aktivierung gilt der alte Stand; Preis-Updates des Agenten
#              laufen die ganze Zeit normal weiter.
#
# Wann greift der Notfallweg später? Nur wenn BEIDES gilt:
#   - das Register hat 30 Tage keinen Preis vom Hauptsatz gesehen (Stille), und
#   - das Orakel ist eingefroren (`ghostctl oracle-freeze`, darf jeder, sobald
#     der Preis älter als 2 h ist).
# Warum eingefroren (Audit 20 A20a-1): Die Signatur des letzten Preis-Updates
# steht öffentlich auf der Kette. Solange das Orakel nicht eingefroren ist,
# kann jeder sie mit einem Orakel-„read“ wiederholen; das Register hält das für
# ein Lebenszeichen des Hauptsatzes und entwertet die Notfall-Ankündigung –
# ohne Schlüssel, für Centbeträge. Ein eingefrorenes Orakel kann nur ein echtes
# Update des Hauptsatzes auftauen. ghostctl lehnt `signers propose --emergency`
# deshalb ab, solange das Orakel nicht eingefroren ist.
# Notfall dann: `./ghostctl --network mainnet oracle-freeze --key <Zahler>`,
# danach `./ghostctl --network mainnet signers propose --emergency --key <Zahler>
#   --committee <Notfall-Datei vom Stick> --new-keys <neuer Unterzeichner>`,
# nach weiteren 14 Tagen `signers activate`.
#
# Hinweis zum Server: Der Agent auf dem Server kennt diese Ankündigung nicht
# (seine Zustandsdatei stammt nicht von diesem Mac). Er meldet deshalb bis zur
# Aktivierung „Im Register wurde von außen ein Austausch angekündigt“ – das ist
# diese Ankündigung. Preis-Updates des Servers laufen weiter (der Hauptsatz
# bleibt derselbe; der Notfallsatz steht nur als Hash im Register).
#
# Probelauf ohne jedes Senden: DRY=1 ./GHOST-Notfallsatz.command
# Andere Schlüsseldateien: KEYS=keys/mainnet ./GHOST-Notfallsatz.command

cd "$(dirname "$0")" || exit 1
PROJ="$(pwd -P)"
NET=mainnet
KEYS=${KEYS:-keys/$NET}
OWNER="$KEYS-owner.json"
SIGNER="$KEYS-signer.json"
STATE=deployments/$NET.json
PUBFILE=deployments/$NET-notfall.pub

case "${DRY:-0}" in
  1) DRY_RUN=1 ;;
  0) DRY_RUN=0 ;;
  *) echo "DRY=$DRY verstehe ich nicht: DRY=1 ist ein Probelauf."; exit 1 ;;
esac

fail() { echo "\n✗ $1" >&2; read -k 1 "?Taste drücken zum Schließen …"; exit 1; }
schluss() { read -k 1 "?Taste drücken zum Schließen …"; exit 0; }
py() { python3 -c "$@"; }
g() { ./ghostctl --network $NET --state "$STATE" "$@"; }

echo "=== GHOST – Notfallsatz nachtragen ($NET)$( (( DRY_RUN )) && echo ' – PROBELAUF, nichts wird gesendet') ==="
command -v python3 >/dev/null || fail "python3 fehlt (Xcode-Kommandozeilenwerkzeuge installieren)"
[ -f "$STATE" ] || fail "$STATE fehlt"
# Nur Pfade prüfen – das Skript liest keine Schlüsseldatei selbst, das tut ghostctl
[ -f "$OWNER" ] || fail "$OWNER fehlt (zahlt Gebühr und 1 KAS Ticket)"
[ -f "$SIGNER" ] || fail "$SIGNER fehlt (Unterschrift des aktuellen Satzes)"

# Stand des Registers (frisch von der Kette abgeglichen)
signers_json() { g --json signers show 2>/dev/null; }
J=$(signers_json) || fail "Stand des Registers nicht lesbar (Node/REST-API erreichbar?) – später erneut"
field() { print -r -- "$J" | py "import json,sys
d=json.load(sys.stdin)['signers']
print($1)" 2>/dev/null; }

PUB=""
[ -f "$PUBFILE" ] && PUB=$(tr -d ' \n' < "$PUBFILE")
fb_now=$(field "','.join((d.get('fallback') or {}).get('keys', []))")
rot=$(field "'ja' if d.get('rotation') else 'nein'")
rot_fb=$(field "','.join(((d.get('rotation') or {}).get('fallback') or {}).get('keys', []))")
rot_valid=$(field "(d.get('rotation') or {}).get('valid', False)")
ready=$(field "(d.get('rotation') or {}).get('readyInHours', 0)")
emerg_open=$(field "d.get('emergencyOpen', False)")
unknown=$(field "d.get('unknownSet', False)")
[ "$unknown" = "True" ] && fail "Im Register steht ein Unterzeichner-Satz, den $STATE nicht kennt – erst klären (Claude fragen)."
[ "$emerg_open" = "True" ] && fail "Im Register ist eine NOTFALL-Ankündigung offen. Das ist nicht dieses Skript – sofort prüfen (\`./ghostctl --network $NET signers show\`)."

# ------------------------------------------------------------ fertig?
if [ -n "$PUB" ] && [ "$fb_now" = "$PUB" ]; then
  echo "✓ Der Notfallsatz ist aktiv: Notfall-Schlüssel $PUB"
  echo "  Er greift nur nach 30 Tagen Stille des Hauptsatzes UND bei eingefrorenem Orakel (siehe Kopf dieses Skripts)."
  schluss
fi
if [ -n "$fb_now" ]; then
  echo "Im Register steht schon ein Notfallsatz: $fb_now"
  [ -n "$PUB" ] && echo "Gemerkt ($PUBFILE) ist aber: $PUB"
  fail "Nichts geändert. Einen Notfallsatz zu ersetzen ist ein eigener Austausch – bitte mit Claude besprechen."
fi

# ------------------------------------------------------------ Schritt 3: aktivieren
if [ "$rot" = "ja" ] && [ "$rot_valid" = "True" ]; then
  [ -n "$PUB" ] && [ "$rot_fb" = "$PUB" ] || fail "Offen ist eine Ankündigung, die nicht von diesem Skript stammt (Notfallsatz: ${rot_fb:-keiner}). Erst klären: \`./ghostctl --network $NET signers show\`."
  if ! py "import sys; sys.exit(0 if float('$ready') <= 0 else 1)"; then
    echo "Die Ankündigung (gleicher Hauptsatz + Notfall-Schlüssel $PUB) ist gesendet."
    echo "Aktivieren geht in etwa $(py "print(round(float('$ready'), 1))") Stunden (≈ $(py "print(round(float('$ready')/24, 1))") Tage)."
    echo "Dann dieses Skript erneut doppelklicken – es bietet das Aktivieren an."
    schluss
  fi
  echo "\nSchritt 3 von 3: Austausch aktivieren (Wartezeit ist um)."
  echo "  Danach steht der Notfall-Schlüssel $PUB als Notfallsatz im Register; der Hauptsatz bleibt."
  echo "  Gebühr von $OWNER, die 1 KAS des Tickets kommt dorthin zurück.\n"
  echo "Probelauf (nichts wird gesendet):"
  g --dry-run signers activate --key "$OWNER" || fail "Probelauf des Aktivierens fehlgeschlagen – nichts gesendet"
  (( DRY_RUN )) && { echo "\nPROBELAUF: nicht gesendet."; schluss; }
  read -r "a?MAINNET: Aktivieren jetzt wirklich senden? Tippe ja: "
  [ "$a" = "ja" ] || fail "abgebrochen – nichts gesendet"
  g --ja signers activate --key "$OWNER" || fail "Aktivieren fehlgeschlagen (zu früh? in ein paar Minuten erneut doppelklicken)"
  J=$(signers_json) && fb_now=$(field "','.join((d.get('fallback') or {}).get('keys', []))")
  [ "$fb_now" = "$PUB" ] && echo "\n✓ Notfallsatz aktiv." || echo "\nGesendet; der Stand zeigt den Notfallsatz noch nicht – in ein paar Minuten erneut doppelklicken zum Prüfen."
  schluss
fi

# ------------------------------------------------------------ Schritt 1: Schlüssel
if [ -z "$PUB" ]; then
  echo "\nSchritt 1 von 3: Notfall-Schlüssel erzeugen."
  stick=""
  for v in /Volumes/*(N); do
    [ -L "$v" ] && continue
    case "$v" in "/Volumes/Macintosh HD"*|"/Volumes/Recovery"*|"/Volumes/Preboot"*) continue ;; esac
    [ -w "$v" ] && { stick="$v"; break; }
  done
  DEF=""
  [ -n "$stick" ] && DEF="$stick/GHOST-Notfall"
  echo "Ordner für den Notfall-Schlüssel (USB-Stick empfohlen, NICHT im Projekt, NICHT auf dem Server)."
  [ -n "$DEF" ] && echo "Gefunden: $stick – Enter übernimmt $DEF" || echo "Kein USB-Stick gefunden. Stick einstecken und neu starten, oder einen Ordner angeben."
  read -r "dir?Ordner: "
  [ -z "$dir" ] && dir="$DEF"
  [ -n "$dir" ] || fail "kein Ordner angegeben – nichts erzeugt"
  dir="${dir/#\~/$HOME}"
  mkdir -p "$dir" || fail "Ordner $dir nicht anlegbar"
  real="$(cd "$dir" && pwd -P)"
  case "$real/" in "$PROJ"/*) fail "$real liegt im Projekt – dort gehört der Notfall-Schlüssel nicht hin (er käme mit Sicherungen/Push mit). Nichts erzeugt." ;; esac
  case "$real" in /Volumes/*) ;; *) echo "⚠ $real liegt nicht auf einem externen Laufwerk. Bitte nach dem Erzeugen auf einen Stick verschieben und hier löschen." ;; esac
  FILE="$real/ghost-notfall-$(date +%Y-%m-%d).json"
  [ -e "$FILE" ] && fail "$FILE gibt es schon – nichts überschrieben"
  if (( DRY_RUN )); then
    echo "PROBELAUF: würde jetzt ./ghostctl committee-keygen $FILE --count 1 ausführen."
    PUB="(Probelauf: noch kein Schlüssel)"
  else
    out=$(./ghostctl --json committee-keygen "$FILE" --count 1) || fail "Schlüssel nicht erzeugt – nichts gesendet"
    PUB=$(print -r -- "$out" | py 'import json,sys; print(json.load(sys.stdin)["xonly"][0])') || fail "öffentlicher Schlüssel nicht lesbar"
    [[ "$PUB" =~ '^[0-9a-f]{64}$' ]] || fail "öffentlicher Schlüssel unerwartet: $PUB"
    print -r -- "$PUB" > "$PUBFILE"
    print -r -- "$PUB" > "$real/ghost-notfall-oeffentlich.txt"
    echo "\n✓ Notfall-Schlüssel erzeugt: $FILE (Rechte 600)"
    echo "  Öffentlich (nicht geheim): $PUB"
  fi
  echo "
  SO SICHERST DU IHN OFFLINE:
   1. Kopiere die Datei auf einen ZWEITEN Stick (oder eine SD-Karte) und lagere
      beide getrennt (z. B. zuhause und Bankschließfach).
   2. Optional Papier: Datei öffnen, die 64 Hex-Zeichen unter „secrets“ auf
      Papier abschreiben und gegenlesen. Nicht fotografieren, nicht in die Cloud.
   3. Danach die Sticks auswerfen und NICHT am Server oder am Agent-Rechner lassen.
      Auf diesem Mac bleibt nur der öffentliche Teil ($PUBFILE).
   4. Gebraucht wird die Datei NUR im Notfall (Hauptschlüssel verloren),
      dann mit --committee <Datei> bei signers propose --emergency."
  (( DRY_RUN )) || read -k 1 "?Gesichert? Taste drücken, dann folgt die Ankündigung (erst als Probelauf) …"
fi

# ------------------------------------------------------------ Schritt 2: ankündigen
echo "\n\nSchritt 2 von 3: Ankündigung im Register (Mainnet)"
echo "  Hauptsatz:   bleibt der bisherige Unterzeichner (gleiche Schlüssel und Schwellen, --same-set)"
echo "  Notfallsatz: NEU, 1 von 1: $PUB"
echo "  Signiert von: $SIGNER (aktueller Satz)"
echo "  Zahlt:        $OWNER (Gebühr ≈ 0,01 KAS + 1 KAS Ticket, kommt beim Aktivieren zurück)"
echo "  Wirkung erst nach 14 Tagen Wartezeit und dem Aktivieren (Schritt 3); bis dahin"
echo "  kann der Hauptsatz die Ankündigung jederzeit absagen (\`signers cancel\`).\n"
if (( DRY_RUN )) && [[ ! "$PUB" =~ '^[0-9a-f]{64}$' ]]; then
  echo "PROBELAUF ohne Schlüssel: Ankündigung nicht gebaut."
  schluss
fi
ARGS=(signers propose --key "$OWNER" --committee "$SIGNER" --same-set --fallback-keys "$PUB" --fallback-threshold 1)
echo "Probelauf (nichts wird gesendet):"
g --dry-run "${ARGS[@]}" || fail "Probelauf der Ankündigung fehlgeschlagen – nichts gesendet"
(( DRY_RUN )) && { echo "\nPROBELAUF: nicht gesendet."; schluss; }
echo ""
read -r "a?MAINNET: Ankündigung jetzt wirklich senden? Tippe ja: "
[ "$a" = "ja" ] || fail "abgebrochen – nichts gesendet. Erneut doppelklicken setzt bei Schritt 2 fort (der Schlüssel bleibt)."
g --ja "${ARGS[@]}" || fail "Ankündigung fehlgeschlagen – erneut doppelklicken (der Journal-Abgleich klärt, ob sie doch angekommen ist)"
echo "\n✓ Angekündigt. In 14 Tagen dieses Skript erneut doppelklicken: es bietet das Aktivieren an."
schluss
