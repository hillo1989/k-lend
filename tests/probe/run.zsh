#!/bin/zsh
# Spielt GHOST-v4-Probe.command gegen eine Attrappe von ghostctl durch (ohne
# Netz, Audit 15): Normalablauf über drei Stufen, Fortsetzen nach Abbrüchen,
# Konsens-Probe, nicht lesbarer Status, Probelauf.
# Aufruf: tests/probe/run.zsh   (braucht /usr/bin/expect und python3)

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
FAKE="$HERE/fake_ghostctl.py"
TMP=$(mktemp -d)
# KEEP=1: Sandbox zur Fehlersuche behalten
trap "[ -n \"\$KEEP\" ] && echo \"Sandbox: \$TMP\" || rm -rf \"\$TMP\"" EXIT
fails=0

# frische Sandbox: v4-Ordner mit Skript und Attrappe, Hauptordner mit Owner-Schlüssel
sandbox() {
  local sb=$TMP/$1
  mkdir -p $sb/kaspa-lending-v4/protocol $sb/kaspa-lending-v4/vendor/silverscript/target/release $sb/kaspa-lending/keys $sb/kaspa-lending/deployments $sb/bin
  cp "$ROOT/GHOST-v4-Probe.command" $sb/kaspa-lending-v4/
  printf '#!/bin/sh\nexec %s "$@"\n' "$FAKE" > $sb/kaspa-lending-v4/vendor/silverscript/target/release/ghostctl
  printf '#!/bin/sh\nV3=1 exec %s "$@"\n' "$FAKE" > $sb/kaspa-lending/ghostctl
  printf '#!/bin/sh\nexit 0\n' > $sb/bin/cargo
  # sleep stellt die simulierte Uhr vor (Sekunden → Stunden in sim.json)
  printf '#!/bin/sh\nexec python3 -c "import json,sys; p=\\"$SIM\\"; d=json.load(open(p)); d[\\"now\\"]+=float(sys.argv[1])/3600; json.dump(d,open(p,\\"w\\"))" "$1"\n' > $sb/bin/sleep
  chmod +x $sb/kaspa-lending-v4/vendor/silverscript/target/release/ghostctl $sb/kaspa-lending/ghostctl $sb/bin/cargo $sb/bin/sleep
  echo '{}' > $sb/kaspa-lending/keys/mainnet-owner.json
  echo '{"now":0,"bal":{"keys/mainnet-owner.json":100},"seq":0,"frozen":false,"last":0,"vaults":[],"rot":null,"signer":"signer1","fail":{}}' > $sb/sim.json
  print -r -- $sb
}

# eine Ausführung; $2 = Antwort auf [j/N]; Ausgabe nach $sb/out.
# Standard EINZELN=1 (nur die fällige Stufe); AUTO=1 = Warteschleife über alle Stufen
probe() {
  local sb=$1 ans=${2:-j} einzeln=1
  [ -n "$AUTO" ] && einzeln=0
  SIM=$sb/sim.json PATH=$sb/bin:$PATH DRY=${DRY:-0} EINZELN=$einzeln /usr/bin/expect -c "
    set timeout 30
    spawn zsh -c {cd $sb/kaspa-lending-v4 && ./GHOST-v4-Probe.command}
    expect {
      {\[j/N\]} { send \"$ans\r\"; exp_continue }
      {Taste dr} { send \"x\"; exp_continue }
      eof
    }
  " 2>&1 | tr -d '\r' > $sb/out
}

sim() { python3 -c "import json,sys; d=json.load(open('$1/sim.json')); print($2)"; }
setsim() { python3 -c "import json; p='$1/sim.json'; d=json.load(open(p)); $2; json.dump(d, open(p,'w'))"; }
stufe() { cat $1/kaspa-lending/deployments/mainnet-v4probe.stufe 2>/dev/null || echo 1; }
# Aufrufe der Attrappe mit diesem Befehl (ganzes Wort, nicht „deployments/…“)
calls() { local n; n=$(grep -cE -- " $2( |\$)" $1/sim.json.log 2>/dev/null); echo ${n:-0}; }

check() {
  if eval "$2"; then echo "  ✓ $1"; else echo "  ✗ $1"; fails=$((fails + 1)); fi
}

echo "P1 Normalablauf über drei Stufen"
sb=$(sandbox p1)
probe $sb
check "Stufe 1 fertig" '[ "$(stufe $sb)" = 2 ]'
check "Vault mit 0,1 GHOST Schuld" '[ "$(sim $sb "d[\"vaults\"]")" = "[0.1]" ]'
setsim $sb 'd["now"]=1.1'
probe $sb
check "Stufe 2 fertig" '[ "$(stufe $sb)" = 3 ]'
check "Konsens-Probe: Sperre des Nodes erkannt" 'grep -q "vom Node abgelehnt" $sb/out'
setsim $sb 'd["now"]=2.2'
probe $sb
check "Probe abgeschlossen" '[ "$(stufe $sb)" = done ]'
check "neuer Unterzeichner aktiv" '[ "$(sim $sb "d[\"signer\"]")" = signer2 ]'
check "Vault geschlossen" '[ "$(sim $sb "d[\"vaults\"]")" = "[]" ]'
check "Rest zurück an den Owner (bis auf 0,005 KAS)" '[ "$(sim $sb "round(d[\"bal\"][\"keys/v4probe-owner.json\"],3)")" = 0.005 ]'

echo "P2 Stufe 2 erst nach der Wartezeit fortgesetzt (Audit 15 G-1)"
sb=$(sandbox p2)
probe $sb
setsim $sb 'd["now"]=1.1; d["fail"]["propose_sent_err"]=1'
probe $sb
check "Abbruch nach gesendeter Ankündigung" '[ "$(stufe $sb)" = 2b ]'
setsim $sb 'd["now"]=2.5'
probe $sb
check "Konsens-Probe übersprungen statt Fehlalarm" 'grep -q "Konsens-Probe übersprungen" $sb/out && ! grep -q ANGENOMMEN $sb/out'
check "weiter zu Stufe 3" '[ "$(stufe $sb)" = 3 ]'
probe $sb
check "danach fertig" '[ "$(stufe $sb)" = done ]'

echo "P3 anderer Fehler bei der Probe-Aktivierung zählt nicht als Erfolg (Audit 15 G-2)"
sb=$(sandbox p3)
probe $sb
setsim $sb 'd["now"]=1.1; d["fail"]["activate"]=1'
probe $sb
check "Stufe bleibt 2b" '[ "$(stufe $sb)" = 2b ]'
check "Meldung „anderer Grund“" 'grep -q "anderen Grund" $sb/out'
probe $sb
check "erneuter Start: Probe gelingt, Stufe 3" '[ "$(stufe $sb)" = 3 ] && grep -q "vom Node abgelehnt" $sb/out'

echo "P4 nicht lesbarer Status bricht ab (Audit 15 G-3/G-10)"
sb=$(sandbox p4)
setsim $sb 'd["fail"]["status_stale"]=1'
probe $sb
check "Stufe 1 nicht weitergeschaltet" '[ "$(stufe $sb)" = 1 ]'
check "Meldung „Status nicht lesbar“" 'grep -q "Status nicht lesbar" $sb/out'
probe $sb
check "erneuter Start setzt fort" '[ "$(stufe $sb)" = 2 ] && [ "$(calls $sb deploy)" = 1 ]'

echo "P4b nicht lesbarer Status mitten in Stufe 3: kein Weiterlaufen bis zur Rücküberweisung (Audit 15 G-3)"
sb=$(sandbox p4b)
probe $sb
setsim $sb 'd["now"]=1.1'
probe $sb
# Statusaufrufe in Stufe 3: Ankündigung, Restwartezeit, seq vorher, seq nachher, Zahl der Vaults
setsim $sb 'd["now"]=2.2; d["stale_at"]=d.get("status_calls",0)+5'
probe $sb
check "abgebrochen, Stufe 3b" '[ "$(stufe $sb)" = 3b ]'
check "nichts zurücküberwiesen, Vault noch offen" '[ "$(calls $sb "send --key keys/v4probe-owner.json")" = 0 ] && [ "$(sim $sb "d[\"vaults\"]")" = "[0.1]" ]'
probe $sb
check "erneuter Start: fertig" '[ "$(stufe $sb)" = done ]'

echo "P5 Stufe 3 nach Abbruch beim Tilgen fortgesetzt: kein zweites Preis-Update (Audit 15 G-8)"
sb=$(sandbox p5)
probe $sb
setsim $sb 'd["now"]=1.1'
probe $sb
setsim $sb 'd["now"]=2.2; d["fail"]["repay"]=1'
probe $sb
check "Stufe 3b" '[ "$(stufe $sb)" = 3b ]'
n=$(calls $sb "oracle-update --key keys/v4probe-owner.json --committee keys/v4probe-signer2.json")
probe $sb
check "fertig" '[ "$(stufe $sb)" = done ]'
check "kein weiteres Preis-Update mit Unterzeichner 2" '[ "$(calls $sb "oracle-update --key keys/v4probe-owner.json --committee keys/v4probe-signer2.json")" = "$n" ]'

echo "P6 Probelauf schaltet keine Stufe weiter (Audit 14 M3)"
sb=$(sandbox p6)
setsim $sb 'd["bal"]["keys/v4probe-owner.json"]=30'
DRY=1 probe $sb
check "keine Stufendatei, keine Zustandsdatei" '[ ! -f $sb/kaspa-lending/deployments/mainnet-v4probe.stufe ] && [ ! -f $sb/kaspa-lending/deployments/mainnet-v4probe.json ]'

echo "P7 „N“ sendet nichts"
sb=$(sandbox p7)
probe $sb N
check "kein Senden, keine Stufe" '[ "$(calls $sb send)" = 0 ] && [ "$(stufe $sb)" = 1 ]'

echo "P8 Automatik: einmal bestätigen, alle drei Stufen laufen mit Wartezeiten durch"
sb=$(sandbox p8)
AUTO=1 probe $sb
check "Probe abgeschlossen" '[ "$(stufe $sb)" = done ]'
check "genau eine Frage" '[ "$(grep -c "\[j/N\]" $sb/out)" = 1 ]'
check "dazwischen gewartet" 'grep -q "warte 5 Minuten" $sb/out'
check "Konsens-Probe vom Node abgelehnt" 'grep -q "vom Node abgelehnt" $sb/out'
check "neuer Unterzeichner, Vault zu, Rest zurück" '[ "$(sim $sb "d[\"signer\"]")" = signer2 ] && [ "$(sim $sb "d[\"vaults\"]")" = "[]" ] && [ "$(calls $sb "send --key keys/v4probe-owner.json")" = 1 ]'

echo "P9 Automatik: vorübergehender Fehler wird wiederholt, dreimal hintereinander hält an"
sb=$(sandbox p9)
setsim $sb 'd["fail"]["open-vault"]=1'
AUTO=1 probe $sb
check "trotz eines Fehlers fertig" '[ "$(stufe $sb)" = done ] && grep -q "Fehlschlag 1 von 3" $sb/out'
sb=$(sandbox p9b)
setsim $sb 'd["fail"]["open-vault"]=5'
AUTO=1 probe $sb
check "nach drei Fehlschlägen angehalten" '[ "$(stufe $sb)" = 1 ] && grep -q "Dreimal hintereinander" $sb/out && [ "$(calls $sb open-vault)" = 3 ]'

echo "P9c Automatik: Status eine Weile nicht lesbar zählt nicht als Fehlschlag (Probe 05.10.2026)"
sb=$(sandbox p9c)
setsim $sb 'd["fail"]["status_stale"]=5'
AUTO=1 probe $sb
check "trotz fünf Störungen fertig" '[ "$(stufe $sb)" = done ] && grep -q "Störung 5 von 12" $sb/out && ! grep -q "Fehlschlag" $sb/out'
sb=$(sandbox p9d)
setsim $sb 'd["fail"]["status_stale"]=40'
AUTO=1 probe $sb
check "nach einer Stunde Störung angehalten" '[ "$(stufe $sb)" = 1 ] && grep -q "eine Stunde lang nicht lesbar" $sb/out'

echo "P10 Automatik: „N“ sendet nichts"
sb=$(sandbox p10)
AUTO=1 probe $sb N
check "kein Senden, keine Stufe" '[ "$(calls $sb send)" = 0 ] && [ "$(stufe $sb)" = 1 ]'

echo
if (( fails )); then echo "✗ $fails Prüfung(en) fehlgeschlagen"; exit 1; fi
echo "✓ alle Prüfungen bestanden"
