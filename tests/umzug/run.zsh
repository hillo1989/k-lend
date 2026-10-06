#!/bin/zsh
# Szenario-Tests für GHOST-Umzug-v3.command gegen Attrappen: kein Netz, keine
# echten Schlüssel, nichts wird gesendet (Audit 11 und 12, A12-4 und A12-17).
#
# Aufruf (aus dem Projektordner oder von überall):
#   tests/umzug/run.zsh                  – prüft ../../GHOST-Umzug-v3.command
#   SCRIPT=<pfad> tests/umzug/run.zsh    – prüft eine andere Fassung, z. B. den
#       Stand vor Audit 12: git show 8b6ee75:GHOST-Umzug-v3.command > /tmp/alt.command
#   KEEP=1 tests/umzug/run.zsh           – Fallordner auch bei Erfolg behalten
#
# Jeder Fall läuft in einem eigenen Ordner unter einem Temp-Verzeichnis ($CASES,
# Vorgabe mktemp): Kopie des Skripts, Attrappen als ./ghostctl und
# bin/ghostctl-v2, Schlüssel- und Zustandsdateien nur mit Attrappen-Inhalt.
# Nach Erfolg wird der Temp-Ordner entfernt, bei Fehlern bleibt er stehen.
#
# Teile: R = Regression (Audit 11), P = Szenarien der Prüfer von Audit 12 (deren
# pruefer/stub.py; ihre Vorlage wird hier aus Attrappen erzeugt), N = A12-4
# (Schlüsselwahl, Schritt 5), J/D = A12-17 (Journal, Probelauf), C = Restpunkte
# der Nachprüfung (Gegenprobe ME = ME3, x-only, Symlink/Hardlink), E = einmal
# bestätigen (Plan, eine Frage, Abbruch und Fortsetzen, Abweichung vor Teil B
# und vor dem Pool, EINZELN=1, DRY=1).
# Attrappen: mock_ghostctl.py (Welt in mock.json), mockbin/pgrep.
SP=${0:A:h}
SCRIPT=${SCRIPT:-$SP/../../GHOST-Umzug-v3.command}
SCRIPT=${SCRIPT:A}
[ -f "$SCRIPT" ] || { echo "Skript $SCRIPT fehlt"; exit 2; }
CASES=${CASES:-$(mktemp -d "${TMPDIR:-/tmp}/ghost-umzug.XXXXXX")}
mkdir -p "$CASES"; CASES=${CASES:A}
export PATH="$SP/mockbin:$PATH"
A=$(python3 -c 'print("a"*64)'); B=$(python3 -c 'print("b"*64)'); C=$(python3 -c 'print("c"*64)')
DD=$(python3 -c 'print("d"*64)'); E=$(python3 -c 'print("e"*64)')
pass=0; failn=0; FAILED=()
ok() { if eval "$2"; then echo "  ✓ $1"; pass=$((pass+1)); else echo "  ✗ $1"; failn=$((failn+1)); FAILED+=("$CASE: $1"); fi; }
# setup <name> <mock.json mit Platzhaltern @A @B @C @D @E>
setup() {
  CASE=$1; D=$CASES/$1; rm -rf $D; mkdir -p $D/bin $D/keys $D/deployments
  cp $SCRIPT $D/GHOST-Umzug-v3.command; chmod +x $D/GHOST-Umzug-v3.command
  ln -s $SP/mock_ghostctl.py $D/ghostctl; ln -s $SP/mock_ghostctl.py $D/bin/ghostctl-v2
  print -r -- "$2" | sed "s/@A/$A/g; s/@B/$B/g; s/@C/$C/g; s/@D/$DD/g; s/@E/$E/g" > $D/mock.json
  python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); assert set(m) >= {"v2","v3"}' $D/mock.json 2>/dev/null || echo "  ! $1: mock.json ungültig"
  echo "{\"mock_xonly\":\"$A\"}" > $D/keys/mainnet-owner.json
  echo '{"secrets":["x","x","x","x","x"]}' > $D/keys/mainnet-committee.json
  : > $D/calls.log
}
v2state() { echo '{"network":"mainnet","vault_params":{"mcr_bps":20000}}' > $D/deployments/${1:-mainnet.json}; }
v3state() { echo '{"network":"mainnet","vault_params":{"treasury":"ab"}}' > $D/deployments/mainnet.json; }
# Echte Läufe fragen einmal „Alles so ausführen? [j/N]“ (und erneut, wenn Teil B
# vom Plan abweicht): $JA beantwortet bis zu zehn Fragen mit j. Probeläufe
# (DRY=1) fragen nie und laufen weiter mit </dev/null.
JA=$CASES/ja.txt; for i in {1..10}; do echo j; done > $JA
run() { (cd $D && ./GHOST-Umzug-v3.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc; }
# Lauf mit eigenen Antworten: runa '<antworten, je Zeile eine>' [VAR=wert …]
runa() { local a=$1; shift; print -r -- "$a" > $D/antw.txt; (cd $D && env "$@" ./GHOST-Umzug-v3.command <$D/antw.txt >$D/out.txt 2>&1); echo $? > $D/rc; }
rc() { cat $D/rc; }
sends() { grep -E "^v[23] .* (repay|close|withdraw|deploy|open-vault|mint|pool-[a-z]+) " $D/calls.log; }
nopen() { grep -c "^v3 .* open-vault " $D/calls.log; }
lockfree() { [ ! -d $D/deployments/.umzug-v3.lock ]; }
jtarget() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("target"))' "$1"; }
isv2() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if "treasury" not in (d.get("vault_params") or {}) else 1)' "$1" 2>/dev/null; }
hasjournal() { grep -q '"journal": *"v2"' "$1" 2>/dev/null; }

BASE='{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300}]}}'
# Journal von Version 2, das nach einem unterbrochenen Umbenennen noch auf mainnet.json zeigt
JV2='{"action":"Tilgen","txid":"00","target":"%s","next":{"network":"mainnet","vault_params":{"mcr_bps":20000},"journal":"v2"}}'

echo "Geprüft: $SCRIPT"
echo "Fallordner: $CASES"
echo "\n== R: Regression =="
echo "R1) Agent läuft → Abbruch vor jedem Schritt"
setup r1-agent "$BASE"; v2state
MOCK_AGENT=1 run
ok "Exitcode ≠ 0" '[ $(rc) != 0 ]'
ok "Meldung nennt den Agenten" 'grep -q "läuft noch ein GHOST-Agent" $D/out.txt'
ok "nichts umbenannt" '[ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v2.json ]'
ok "kein ghostctl-Aufruf" '[ ! -s $D/calls.log ]'
ok "Sperre wieder frei" 'lockfree'

echo "R2) zweiter Umzug läuft (Sperre) → Abbruch, fremde Sperre bleibt"
setup r2-lock "$BASE"; v2state; mkdir $D/deployments/.umzug-v3.lock; echo 999 > $D/deployments/.umzug-v3.lock/pid
run
ok "Abbruch" '[ $(rc) != 0 ] && grep -q "anderer Umzug läuft schon (PID 999" $D/out.txt'
ok "fremde Sperre bleibt" '[ -f $D/deployments/.umzug-v3.lock/pid ]'
ok "nichts umbenannt, kein Aufruf" '[ -f $D/deployments/mainnet.json ] && [ ! -s $D/calls.log ]'

echo "R3) DRY=ja → Abbruch; DRY=1 → Probelauf ohne Umbenennen"
setup r3-dryx "$BASE"; v2state
(cd $D && DRY=ja ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "DRY=ja abgelehnt" '[ $(rc) != 0 ] && grep -q "verstehe ich nicht" $D/out.txt'
setup r3-dry1 "$BASE"; v2state
(cd $D && DRY=1 ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "Probelauf endet mit 0" '[ $(rc) = 0 ]'
ok "Probelauf nennt sich so" 'grep -q "PROBELAUF" $D/out.txt'
ok "Probelauf benennt nicht um" '[ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v2.json ]'
ok "alle Sendebefehle mit --dry-run" '! sends | grep -vq -- "--dry-run"'
ok "Probelauf tilgt den eigenen Vault" 'grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log'

echo "R4) DRY=0 = echter Lauf: Umbenennen samt Journal, Tilgen/Schließen nur eigene, Vault/Prägen am eigenen, Pool"
setup r4-full "$BASE"; v2state
echo '{"action":"Tilgen","txid":"00","target":"deployments/mainnet.json","next":{"x":1}}' > $D/deployments/mainnet.pending.json
echo '{"network":"mainnet"}' > $D/deployments/mainnet.deploy.json
: > $D/deployments/mainnet.lock
(cd $D && DRY=0 ./GHOST-Umzug-v3.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "Zustand umbenannt" '[ -f $D/deployments/mainnet-v2.json ]'
ok "Journal umbenannt, Ziel umgeschrieben" '[ "$(jtarget $D/deployments/mainnet-v2.pending.json)" = deployments/mainnet-v2.json ]'
ok "Journal-Inhalt sonst gleich" 'python3 -c "import json,sys; j=json.load(open(\"$D/deployments/mainnet-v2.pending.json\")); sys.exit(0 if j[\"txid\"]==\"00\" and j[\"next\"]=={\"x\":1} else 1)"'
ok "Deploy-Fortschritt und Sperre umbenannt" '[ -f $D/deployments/mainnet-v2.deploy.json ] && [ -f $D/deployments/mainnet-v2.lock ]'
ok "alte Namen weg (verschoben, nichts gelöscht)" '[ ! -f $D/deployments/mainnet.pending.json ] && [ ! -f $D/deployments/mainnet.deploy.json ]'
ok "v2: nur eigener Vault getilgt und geschlossen" 'grep -q "^v2 .*repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log && grep -q "^v2 .*close --key keys/mainnet-owner.json --vault 1$" $D/calls.log && ! grep "^v2 .*\(repay\|close\|withdraw\)" $D/calls.log | grep -q "vault 0"'
ok "Schlüsselliste aus dem Ordner der Besitzer-Datei" 'grep -q "^v2 .*--json keys --dir keys$" $D/calls.log && grep -q "^v3 .*--json keys --dir keys$" $D/calls.log'
ok "v3 deploy" 'grep -q "^v3 .*deploy" $D/calls.log'
ok "Vault eröffnet und am NEUEN eigenen Vault (1) geprägt, nicht an 0" '[ $(nopen) = 1 ] && grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
ok "Pool angelegt" 'grep -q "^v3 .*pool-open --key keys/mainnet-owner.json --kas 6.25000000 --ghost 0.25$" $D/calls.log'
ok "kein Merker liegen geblieben" '[ ! -e $D/deployments/.umzug-v3-vault.lock ]'
ok "Sperre wieder frei" 'lockfree'

echo "R5) Wiederaufnahme: eigener Vault ohne Schuld → nur prägen; Pool fehlt GHOST/KAS → Abbruch vor pool-open"
setup r5-resume '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300},{"owner":"@A","covenantId":"y2","debtGhost":0,"collateralKas":50}],
 "fail":{"mint":"Zeitlimit (Attrappe)"}}}'
v3state
run
ok "kein zweites open-vault" '[ $(nopen) = 0 ]'
ok "Prägen an eigenem Vault 1 versucht" 'grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 1 " $D/calls.log'
ok "Prägen scheitert mit Hinweis, kein pool-open" '[ $(rc) != 0 ] && grep -q "prägt nach" $D/out.txt && ! grep -q "pool-open" $D/calls.log'
python3 - "$D/mock.json" "$A" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v3"].pop("fail"); m["v3"]["vaults"][1]["debtGhost"]=0.5; m["v3"]["keys"][sys.argv[2]]["ghost"]=0.1; json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "eigener Vault hat Schuld → Schritt 5 übersprungen" 'grep -q "schon einen eigenen Vault" $D/out.txt && ! grep -q "mint\|open-vault" $D/calls.log'
ok "zu wenig GHOST → klare Meldung, kein pool-open" '[ $(rc) != 0 ] && grep -q "Für den Pool fehlen GHOST" $D/out.txt && ! grep -q "pool-open" $D/calls.log'
python3 - "$D/mock.json" "$A" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v3"]["keys"][sys.argv[2]]["ghost"]=1; m["v3"]["keys"][sys.argv[2]]["kas"]=5; json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "zu wenig KAS → klare Meldung, kein pool-open" '[ $(rc) != 0 ] && grep -q "Für den Pool fehlen KAS" $D/out.txt && ! grep -q "pool-open" $D/calls.log'

echo "R6) Journal mit fremdem Ziel → nichts umbenannt"
setup r6-journal "$BASE"; v2state
echo '{"action":"x","txid":"00","target":"deployments/anderes.json","next":{}}' > $D/deployments/mainnet.pending.json
run
ok "Abbruch mit Hinweis" '[ $(rc) != 0 ] && grep -q "bitte erst mit bin/ghostctl-v2 status klären" $D/out.txt'
ok "nichts umbenannt" '[ -f $D/deployments/mainnet.json ] && [ -f $D/deployments/mainnet.pending.json ] && [ ! -f $D/deployments/mainnet-v2.json ]'

echo "R7) Ziel existiert schon (mainnet-v2.lock) → nichts umbenannt"
setup r7-clash "$BASE"; v2state; : > $D/deployments/mainnet-v2.lock
run
ok "Abbruch" '[ $(rc) != 0 ] && grep -q "Gibt es schon" $D/out.txt'
ok "nichts umbenannt" '[ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v2.json ]'

echo "R8) unlesbare Zustandsdatei → nicht als v2 umbenennen"
setup r8-broken "$BASE"; echo '{kaputt' > $D/deployments/mainnet.json
run
ok "Abbruch, Datei bleibt" '[ $(rc) != 0 ] && grep -q "nicht lesbar" $D/out.txt && [ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v2.json ]'

echo "R9) Agent startet zwischen Teil A und Teil B → Abbruch vor dem Deployment"
setup r9-agentB "$BASE"; v2state
(cd $D && MOCK_COUNT=$D/pgrep.count ./GHOST-Umzug-v3.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
ok "Teil A lief, dann Abbruch vor deploy" 'grep -q "^v2 .*close" $D/calls.log && ! grep -q "^v3 .* deploy " $D/calls.log && [ $(rc) != 0 ] && grep -q "oracle-feed" $D/out.txt'

echo "R10) Restschuld: teilweise tilgen, Sicherheit auf 220 % senken; zwei Umzüge gleichzeitig → höchstens ein Vault"
setup r10-rest '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.3,"lpShares":"0","kas":1}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":600}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state
run
ok "Teil-Tilgung 0,3 und Abheben bis 38,5 KAS" 'grep -q "^v2 .*repay --key keys/mainnet-owner.json --vault 0 --ghost 0.30000000$" $D/calls.log && grep -q "^v2 .*withdraw --key keys/mainnet-owner.json --vault 0 --keep 38.50$" $D/calls.log && ! grep -q "^v2 .*close" $D/calls.log'
setup r10-twice "$BASE"; v2state
(cd $D && ./GHOST-Umzug-v3.command <$JA >$D/out1.txt 2>&1 & ; cd $D && ./GHOST-Umzug-v3.command <$JA >$D/out2.txt 2>&1; wait)
ok "höchstens ein deploy und ein open-vault" '[ $(grep -c " deploy " $D/calls.log) -le 1 ] && [ $(nopen) -le 1 ]'

echo "\n== P: Szenarien der Prüfer (Audit 12, deren stub.py) =="
# Vorlage der Prüfer: Zustand von Version 2 mit dem Modell von stub.py (Feld _m):
# eigene Vaults 0 (0,3 GHOST Schuld), 2 (0,1) und 3 (2,0 GHOST, 60 KAS), ein
# fremder (1), 1,0 GHOST, Orakel 0,05 USD; offenes Journal auf mainnet.json.
# Die Schlüsseldateien liest stub.py nicht, sie tragen nur Attrappen-Inhalt.
PMODEL='{"network":"mainnet","vault_params":{"mcr_bps":20000},"_m":{"vaults":[
 {"owner":"@O","covenantId":"c2","debtGhost":0.3,"collateralKas":10,"stale":false},
 {"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500,"stale":false},
 {"owner":"@O","covenantId":"c1","debtGhost":0.1,"collateralKas":5,"stale":false},
 {"owner":"@O","covenantId":"c3","debtGhost":2.0,"collateralKas":60,"stale":false}],
 "ghost":1.0,"lp":"0","kas":100,"pool":false,"kasUsd":0.05}}'
psetup() {
  CASE=$1; D=$CASES/$1; rm -rf $D; mkdir -p $D/bin $D/keys $D/deployments
  cp $SCRIPT $D/GHOST-Umzug-v3.command; chmod +x $D/GHOST-Umzug-v3.command
  ln -s $SP/pruefer/stub.py $D/ghostctl; ln -s $SP/pruefer/stub.py $D/bin/ghostctl-v2
  echo '{"attrappe":"Besitzer, kein Schlüssel"}' > $D/keys/mainnet-owner.json
  echo '{"attrappe":"Komitee, kein Schlüssel"}' > $D/keys/mainnet-committee.json
  print -r -- "$PMODEL" | sed "s/@O/$A/g; s/@B/$B/g" > $D/deployments/mainnet.json  # stub.py: eigener Schlüssel "aa"*32 = $A
  echo '{"action":"Tilgen","txid":"00","target":"deployments/mainnet.json","next":{"network":"mainnet"}}' > $D/deployments/mainnet.pending.json
  echo '{"network":"mainnet"}' > $D/deployments/mainnet.deploy.json
  : > $D/calls.log
}
psetup p-dry1
(cd $D && DRY=1 ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "dry1: Probelauf endet mit 0, nichts umbenannt" '[ $(rc) = 0 ] && [ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v2.json ]'
ok "dry1: Vorschau tilgt 0,1 und 0,3 ganz, Vault 3 nur mit den übrigen 0,6 GHOST" 'grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 2 --ghost 0.10000000$" $D/calls.log && grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 0 --ghost 0.30000000$" $D/calls.log && grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 3 --ghost 0.60000000$" $D/calls.log'
ok "dry1: Vorschau nennt Restschuld 1,4 und 61,60 KAS" 'grep -q "Restschuld 1.40000000 GHOST bleibt – Sicherheit von 60.00000000 auf 61.60 KAS" $D/out.txt'
psetup p-real1; run
ok "real1: Exitcode 0" '[ $(rc) = 0 ]'
# ohne Proben (--dry-run) und ohne --ja (einmal bestätigt), sonst Zeile für Zeile gleich;
# seit Audit 13 (A13-umzug-1) tilgt repay immer mit --ghost (Vorlage entsprechend)
ok "real1: gleiche Sendefolge wie bei den Prüfern" 'diff <(grep -E " (repay|close|withdraw|deploy|open-vault|mint|pool-open) " $D/calls.log | grep -v -- "--dry-run" | sed "s/ --ja / /") <(grep -E " (repay|close|withdraw|deploy|open-vault|mint|pool-open) " $SP/pruefer/real1-calls.log) >/dev/null'
psetup p-sa; echo '{"txid": "00", "target": "/tmp/anderswo.json"}' > $D/deployments/mainnet.pending.json; : > $D/deployments/mainnet.lock; run
ok "sa: fremdes Ziel → Abbruch, nichts umbenannt" '[ $(rc) != 0 ] && grep -q "zeigt auf /tmp/anderswo.json" $D/out.txt && [ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v2.json ] && lockfree'
psetup p-sb; echo "{\"txid\": \"00\", \"target\": \"$D/deployments/mainnet.json\"}" > $D/deployments/mainnet.pending.json; run
ok "sb: absolutes Ziel → absolut auf mainnet-v2.json" '[ $(rc) = 0 ] && [ "$(jtarget $D/deployments/mainnet-v2.pending.json)" = "${D:A}/deployments/mainnet-v2.json" ]'
psetup p-sc; mv $D/deployments/mainnet.pending.json $D/deployments/mainnet-v2.pending.json; run
ok "sc: Journal lag schon als mainnet-v2.pending.json (Abbruch beim Umbenennen) → zeigt danach auf mainnet-v2.json" '[ $(rc) = 0 ] && [ "$(jtarget $D/deployments/mainnet-v2.pending.json)" = deployments/mainnet-v2.json ]'
psetup p-sd
( cd $D && exec -a "./ghostctl --network mainnet --state deployments/mainnet.json --ja agent --key x" sleep 15 ) &
AGENT=$!; sleep 0.5
(cd $D && PATH="${PATH#$SP/mockbin:}" ./GHOST-Umzug-v3.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
kill $AGENT 2>/dev/null; wait $AGENT 2>/dev/null
ok "sd: echter pgrep sieht den Agenten → Abbruch, nichts umbenannt, Sperre frei" '[ $(rc) != 0 ] && grep -q "läuft noch ein GHOST-Agent" $D/out.txt && [ ! -f $D/deployments/mainnet-v2.json ] && lockfree'
psetup p-se; mkdir $D/deployments/.umzug-v3.lock; echo 4711 > $D/deployments/.umzug-v3.lock/pid; run
ok "se: fremde Sperre → Abbruch, Sperre bleibt" '[ $(rc) != 0 ] && grep -q "PID 4711" $D/out.txt && [ -f $D/deployments/.umzug-v3.lock/pid ]'
psetup p-sf; python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); d["_m"]["vaults"]=None; json.dump(d,open(p,"w"))' $D/deployments/mainnet.json; run
ok "sf: Status v2 scheitert → Abbruch, Sperre durch trap frei" '[ $(rc) != 0 ] && grep -q "Status von Version 2 nicht abrufbar" $D/out.txt && lockfree'
psetup p-sg; rm -f $D/deployments/*; export STUB_ALTKEY=1; run; cp $D/out.txt $D/out1.txt; run; unset STUB_ALTKEY
ok "sg: fremder Schlüssel …alt-mainnet-owner.json zuerst gelistet → über zwei Läufe genau ein open-vault" '[ $(grep -c " open-vault " $D/calls.log) = 1 ]'
ok "sg: am eigenen Vault geprägt, Pool angelegt" 'grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 0 --ghost 0.5$" $D/calls.log && grep -q "pool-open" $D/calls.log'

echo "\n== N: A12-4 Schlüsselwahl und Schritt 5 =="
N1='{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100},"@C":{"ghost":0,"kas":1}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200},"@C":{"ghost":0,"kas":1}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300}]}}'
echo "N1) keys/alt-mainnet-owner.json (anderer Schlüssel) steht vor keys/mainnet-owner.json, zwei Doppelklicks"
setup n1-altkey "$N1"; v2state; echo "{\"mock_xonly\":\"$C\"}" > $D/keys/alt-mainnet-owner.json
run; cp $D/out.txt $D/out1.txt; cp $D/rc $D/rc1; run
ok "1. Lauf endet mit 0" '[ $(cat $D/rc1) = 0 ]'
ok "v2: eigener Vault (1) getilgt und geschlossen" 'grep -q "^v2 .*repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log && grep -q "^v2 .*close --key keys/mainnet-owner.json --vault 1$" $D/calls.log'
ok "über zwei Läufe genau ein open-vault" '[ $(nopen) = 1 ]'
ok "am eigenen neuen Vault 1 geprägt" 'grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
# (leerer Plan: das Skript endet vor Teil A und B, ohne Frage und ohne --ja-Lauf)
ok "2. Lauf: nichts mehr zu tun" '[ $(rc) = 0 ] && grep -q "Nichts mehr zu senden" $D/out.txt && ! grep -q "Teil B: Version 3 anlegen" $D/out.txt'

echo "N2) KEYS in anderem Ordner, in keys/ liegt eine gleichnamige fremde Datei"
setup n2-keysdir "$N1"; v2state; mkdir $D/schluessel
mv $D/keys/mainnet-owner.json $D/keys/mainnet-committee.json $D/schluessel/
echo "{\"mock_xonly\":\"$DD\"}" > $D/keys/mainnet-owner.json
(cd $D && KEYS=schluessel/mainnet ./GHOST-Umzug-v3.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "Schlüsselliste aus schluessel/" 'grep -q "^v3 .*--json keys --dir schluessel$" $D/calls.log'
ok "v2: eigener Vault getilgt und geschlossen" 'grep -q "^v2 .*close --key schluessel/mainnet-owner.json --vault 1$" $D/calls.log'
ok "genau ein open-vault, am eigenen Vault geprägt" '[ $(nopen) = 1 ] && grep -q "^v3 .*mint --key schluessel/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'

echo "N3) Besitzer-Datei nicht als Schlüssel gelistet, ähnlich benannte fremde schon → Abbruch vor jedem Senden"
setup n3-nomatch '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@E":{"ghost":1,"kas":200}},"vaults":[]}}'
v3state; echo '{"kaputt":1}' > $D/keys/mainnet-owner.json; echo "{\"mock_xonly\":\"$E\"}" > $D/keys/x-mainnet-owner.json
run
ok "Abbruch mit Hinweis auf den Schlüssel" '[ $(rc) != 0 ] && grep -q "nicht (eindeutig) in der Schlüsselliste" $D/out.txt'
ok "kein Sendebefehl" '[ -z "$(sends)" ]'

echo "N4) Besitzer-Datei doppelt gelistet, einmal mit anderem Schlüssel → Abbruch mit Grund"
setup n4-dup '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[],"dup_keys":"diff"}}'
v3state; run
ok "Abbruch, kein Sendebefehl" '[ $(rc) != 0 ] && grep -q "nicht (eindeutig)" $D/out.txt && [ -z "$(sends)" ]'
ok "Grund: mehrfach mit verschiedenen Schlüsseln" 'grep -q "Grund: keys/mainnet-owner.json steht mehrfach mit verschiedenen Schlüsseln" $D/out.txt'
echo "N4b) Besitzer-Datei doppelt gelistet, derselbe Schlüssel → zählt einmal (Restpunkt C-R1)"
setup n4b-dupsame '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[],"dup_keys":true}}'
v3state; run
ok "Exitcode 0, genau ein open-vault, am eigenen Vault geprägt" '[ $(rc) = 0 ] && [ $(nopen) = 1 ] && grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 0 --ghost 0.5$" $D/calls.log'

echo "N5) eigener v3-Vault ist gesperrt (stale) → kein zweiter Vault"
setup n5-stale '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":1,"kas":200}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300},{"owner":"@A","covenantId":"y2","debtGhost":0,"collateralKas":50,"stale":true}]}}'
v3state; run
ok "kein open-vault, kein mint am gesperrten" '[ $(nopen) = 0 ] && ! grep -q " mint " $D/calls.log'
ok "Meldung: eigener Vault vorhanden" 'grep -q "schon einen eigenen Vault (mit Schuld oder gesperrt)" $D/out.txt'

echo "N6) neuer Vault erscheint nicht als eigener → Merker, zweiter Doppelklick eröffnet keinen weiteren"
setup n6-mark '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":1,"kas":200}},"vaults":[],"open_owner":"@E"}}'
v3state; run; cp $D/out.txt $D/out1.txt; cp $D/rc $D/rc1
ok "1. Lauf: ein open-vault, dann Abbruch mit Merker" '[ $(cat $D/rc1) != 0 ] && [ $(nopen) = 1 ] && [ -f $D/deployments/.umzug-v3-vault.lock ] && grep -q "nicht als eigener gefunden" $D/out1.txt'
run
ok "2. Lauf: kein weiteres open-vault, Hinweis auf den früheren Lauf" '[ $(rc) != 0 ] && [ $(nopen) = 1 ] && grep -q "früherer Lauf hat schon einen Vault eröffnet" $D/out.txt'
ok "Sperre wieder frei" 'lockfree'

echo "N7) Merker liegt, eigener Vault ist inzwischen da → Merker weg, weiter mit Prägen"
setup n7-markok '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},
 "vaults":[{"owner":"@A","covenantId":"y2","debtGhost":0,"collateralKas":50}]}}'
v3state; echo "Besitzer x" > $D/deployments/.umzug-v3-vault.lock; run
ok "Exitcode 0, Merker entfernt, geprägt, kein open-vault" '[ $(rc) = 0 ] && [ ! -e $D/deployments/.umzug-v3-vault.lock ] && grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 0 " $D/calls.log && [ $(nopen) = 0 ]'

echo "N8) Status ohne Vault-Liste → nichts eröffnen"
setup n8-status '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[],"status_broken":true}}'
v3state; run
ok "Abbruch, kein open-vault" '[ $(rc) != 0 ] && grep -q "Status nicht abrufbar" $D/out.txt && [ $(nopen) = 0 ]'

echo "\n== J: A12-17 Journal nach unterbrochenem Umbenennen (Attrappe übernimmt Journale wie resolve_pending) =="
JB=$(print -r -- "$BASE" | sed 's/^{/{"journal":true,/')
echo "J1) mainnet.json und Journal schon umbenannt, Ziel noch mainnet.json, Sperre nicht"
setup j1-renamed "$JB"; v2state mainnet-v2.json; printf "$JV2" deployments/mainnet.json > $D/deployments/mainnet-v2.pending.json; : > $D/deployments/mainnet.lock
run
ok "Journal umgerichtet (Meldung)" 'grep -q "zeigt jetzt auf deployments/mainnet-v2.json" $D/out.txt'
ok "v2-Stand aus dem Journal landet in mainnet-v2.json" 'hasjournal $D/deployments/mainnet-v2.json'
ok "mainnet.json bekommt nie den v2-Stand, ist am Ende Version 3" '! hasjournal $D/deployments/mainnet.json && [ -f $D/deployments/mainnet.json ] && ! isv2 $D/deployments/mainnet.json'
ok "Exitcode 0" '[ $(rc) = 0 ]'

echo "J2) Journal schon umbenannt, mainnet.json (v2) noch nicht"
setup j2-half "$JB"; v2state; printf "$JV2" deployments/mainnet.json > $D/deployments/mainnet-v2.pending.json
echo '{"network":"mainnet"}' > $D/deployments/mainnet.deploy.json; : > $D/deployments/mainnet.lock
run
ok "umbenannt und Journal umgerichtet" '[ -f $D/deployments/mainnet-v2.json ] && [ -f $D/deployments/mainnet-v2.deploy.json ] && grep -q "zeigt jetzt auf deployments/mainnet-v2.json" $D/out.txt'
ok "v2-Stand in mainnet-v2.json, mainnet.json am Ende Version 3" 'hasjournal $D/deployments/mainnet-v2.json && ! hasjournal $D/deployments/mainnet.json && ! isv2 $D/deployments/mainnet.json && [ $(rc) = 0 ]'

echo "J3) wie J1 mit absolutem Ziel"
setup j3-abs "$JB"; v2state mainnet-v2.json; printf "$JV2" "$D/deployments/mainnet.json" > $D/deployments/mainnet-v2.pending.json
run
ok "absolut auf mainnet-v2.json umgerichtet, v2-Stand dort" 'grep -q "zeigt jetzt auf ${D:A}/deployments/mainnet-v2.json" $D/out.txt && hasjournal $D/deployments/mainnet-v2.json && ! hasjournal $D/deployments/mainnet.json && [ $(rc) = 0 ]'

echo "J4) Probelauf mit Journal aus J1 → bricht ab, verändert nichts"
setup j4-dry "$JB"; v2state mainnet-v2.json; printf "$JV2" deployments/mainnet.json > $D/deployments/mainnet-v2.pending.json
(cd $D && DRY=1 ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "Abbruch mit Hinweis" '[ $(rc) != 0 ] && grep -q "der Probelauf bricht hier ab" $D/out.txt'
ok "kein Aufruf von Version 2, Journal unverändert, keine mainnet.json" '! grep -q "^v2 " $D/calls.log && [ "$(jtarget $D/deployments/mainnet-v2.pending.json)" = deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet.json ]'

echo "J5) Journal von Version 2 zeigt auf eine fremde Datei → Abbruch vor jedem Aufruf"
setup j5-foreign "$JB"; v2state mainnet-v2.json; printf "$JV2" deployments/anders.json > $D/deployments/mainnet-v2.pending.json
run
ok "Abbruch, kein v2-Aufruf, anders.json nicht geschrieben" '[ $(rc) != 0 ] && grep -q "bitte von Hand klären" $D/out.txt && ! grep -q "^v2 " $D/calls.log && [ ! -f $D/deployments/anders.json ]'

echo "J6) Journal zeigt schon richtig → normaler Lauf"
setup j6-right "$JB"; v2state mainnet-v2.json; printf "$JV2" deployments/mainnet-v2.json > $D/deployments/mainnet-v2.pending.json
run
ok "Exitcode 0, keine Umrichtung, v2-Stand in mainnet-v2.json" '[ $(rc) = 0 ] && ! grep -q "zeigt jetzt auf" $D/out.txt && hasjournal $D/deployments/mainnet-v2.json'

echo "\n== D: A12-17 Probelauf zieht die verplanten GHOST ab =="
setup d1-dry '{"v2":{"kasUsd":0.05,"keys":{"@A":{"ghost":1.0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c3","debtGhost":2.0,"collateralKas":100},{"owner":"@B","covenantId":"x1","debtGhost":0.05,"collateralKas":7},
 {"owner":"@A","covenantId":"c1","debtGhost":0.1,"collateralKas":5},{"owner":"@A","covenantId":"c2","debtGhost":0.3,"collateralKas":10}]},
 "v3":{"kasUsd":0.05,"keys":{},"vaults":[]}}'
v2state
(cd $D && DRY=1 ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "0,1 und 0,3 ganz, dann 2,0 nur mit den übrigen 0,6 GHOST" 'grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 2 --ghost 0.10000000$" $D/calls.log && grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 3 --ghost 0.30000000$" $D/calls.log && grep -q "^v2 .*--dry-run repay --key keys/mainnet-owner.json --vault 0 --ghost 0.60000000$" $D/calls.log'
ok "Anzeige: eigene GHOST nach den vorigen Tilgungen" 'grep -q "eigene GHOST 0.60000000 (nach den vorigen Tilgungen)" $D/out.txt'
ok "Restschuld 1,4 GHOST, Sicherheit auf 61,60 KAS" 'grep -q "Restschuld 1.40000000 GHOST bleibt – Sicherheit von 100.00000000 auf 61.60 KAS senken" $D/out.txt'
setup d2-real '{"v2":{"kasUsd":0.05,"keys":{"@A":{"ghost":1.0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c3","debtGhost":2.0,"collateralKas":100},{"owner":"@A","covenantId":"c1","debtGhost":0.1,"collateralKas":5},{"owner":"@A","covenantId":"c2","debtGhost":0.3,"collateralKas":10}]},
 "v3":{"kasUsd":0.05,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[]}}'
v2state; run
ok "echter Lauf tilgt gleich wie die Vorschau (0,6 GHOST am letzten Vault)" 'grep -q "^v2 .*repay --key keys/mainnet-owner.json --vault 0 --ghost 0.60000000$" $D/calls.log && grep -q "^v2 .*withdraw --key keys/mainnet-owner.json --vault 0 --keep 61.60$" $D/calls.log && [ $(rc) = 0 ]'

echo "\n== C: Restpunkte der Nachprüfung zu Audit 12 (C-T2, C-R1) =="
echo "C1) Version 3 nennt für die Besitzer-Datei ein anderes x-only als Version 2 → Gegenprobe ME = ME3, nichts eröffnet"
setup c1-me3 '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@E":{"ghost":0,"kas":200}},"vaults":[],"xonly_as":"@E"}}'
v2state; run
ok "Teil A lief mit dem v2-Schlüssel (getilgt, geschlossen)" 'grep -q "^v2 .*close --key keys/mainnet-owner.json --vault 0$" $D/calls.log'
ok "Abbruch: Version 2 und 3 nennen verschiedene Schlüssel" '[ $(rc) != 0 ] && grep -q "Version 2 und 3 nennen für keys/mainnet-owner.json verschiedene Schlüssel ($A / $E)" $D/out.txt'
ok "kein open-vault, kein mint" '[ $(nopen) = 0 ] && ! grep -q " mint " $D/calls.log'

echo "C2) x-only der Besitzer-Datei hat keine 64 Hex-Zeichen → Abbruch vor jedem Senden"
setup c2-xonly '{"v2":{"kasUsd":0.04,"keys":{"abc":{"ghost":2,"lpShares":"0","kas":100}},"vaults":[]},
 "v3":{"kasUsd":0.04,"keys":{"abc":{"ghost":0,"kas":200}},"vaults":[]}}'
v2state; echo '{"mock_xonly":"abc"}' > $D/keys/mainnet-owner.json
run
ok "Abbruch mit Grund" '[ $(rc) != 0 ] && grep -q "Grund: der Schlüssel zu keys/mainnet-owner.json ist kein x-only aus 64 Hex-Zeichen (abc)" $D/out.txt'
ok "kein Sendebefehl (kein deploy, kein open-vault)" '[ -z "$(sends)" ]'
setup c2b-xonly3 '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},"v3":{"kasUsd":0.04,"keys":{"A":{"ghost":0,"kas":200}},"vaults":[],"xonly_as":"A"}}'
v3state; run
ok "Version 3 allein: Abbruch vor open-vault" '[ $(rc) != 0 ] && grep -q "kein x-only aus 64 Hex-Zeichen (A)" $D/out.txt && [ $(nopen) = 0 ]'

echo "C3) Symlink auf die Besitzer-Datei im selben Ordner → dieselbe Datei, ein Treffer, normaler Lauf"
setup c3-symlink "$N1"; v2state; ln -s mainnet-owner.json $D/keys/zz-kopie.json
run
ok "Exitcode 0, v2 getilgt und geschlossen" '[ $(rc) = 0 ] && grep -q "^v2 .*close --key keys/mainnet-owner.json --vault 1$" $D/calls.log'
ok "genau ein open-vault, am eigenen Vault geprägt" '[ $(nopen) = 1 ] && grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
ok "die Attrappe listete wirklich beide Namen" 'grep -q "zz-kopie.json" <(cd $D && ./ghostctl --network mainnet --state deployments/mainnet.json --json keys --dir keys)'
echo "C4) Hardlink auf die Besitzer-Datei → ebenso"
setup c4-hardlink "$N1"; v2state; ln $D/keys/mainnet-owner.json $D/keys/zz-hart.json
run
ok "Exitcode 0, genau ein open-vault, am eigenen Vault geprägt" '[ $(rc) = 0 ] && [ $(nopen) = 1 ] && grep -q "^v3 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
echo "C5) Kopie (eigene Datei, gleicher Inhalt) ist nicht die Besitzer-Datei → ein Treffer, normaler Lauf"
setup c5-copy "$N1"; v2state; cp $D/keys/mainnet-owner.json $D/keys/zz-kopie.json
run
ok "Exitcode 0, genau ein open-vault" '[ $(rc) = 0 ] && [ $(nopen) = 1 ]'

echo "\n== E: Einmal bestätigen (Plan, eine Frage, Abbruch und Fortsetzen, EINZELN=1) =="
# echte Sendebefehle (ohne die Proben mit --dry-run)
esends() { sends | grep -v -- "--dry-run"; }
nfrage() { grep -o "Alles so ausführen? \[j/N\]" $D/out.txt | wc -l | tr -d ' '; }
zeile() { grep -n -m1 -- "$1" $D/out.txt | cut -d: -f1; }
echo "E1) Zusammenfassung vor dem ersten Senden, genau eine Frage, alles mit --ja"
setup e1-plan "$BASE"; v2state; run
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "genau eine Frage, keine weitere" '[ $(nfrage) = 1 ] && ! grep -q "Teil B so ausführen\|Pool mit .* anlegen?" $D/out.txt'
ok "Plan mit allen Schritten" 'grep -q "=== Plan: 6 Schritte ===" $D/out.txt && grep -q "1. Vault 1: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "2. Vault 1 schließen" $D/out.txt && grep -q "3. Orakel, Factory und GHOST (Version 3) anlegen" $D/out.txt && grep -q "4. Eigenen Vault mit 50 KAS eröffnen" $D/out.txt && grep -q "5. 0.5 GHOST prägen" $D/out.txt && grep -q "6. Tauschpool mit Kursband anlegen: 6.25000000 KAS und 0.25 GHOST" $D/out.txt'
ok "zurück, gebunden, Gebühren (Probe und Schätzung)" 'grep -q "zurück: 60.00000000 KAS Sicherheit" $D/out.txt && grep -q "gebunden: 30 KAS dauerhaft" $D/out.txt && grep -q "3 KAS Minter-Zweig dauerhaft" $D/out.txt && grep -q "gebunden: Mindestliquidität" $D/out.txt && grep -q "Gebühr: 0.0412 KAS (mit --dry-run gebaut)" $D/out.txt && grep -q "Gebühr: etwa 0.05 KAS (geschätzt), 3 Transaktionen" $D/out.txt'
ok "Endstand: KAS jetzt, vor Teil B, am Ende; GHOST" 'grep -q "Gebühren zusammen: etwa 0.4212 KAS für 10 Transaktionen" $D/out.txt && grep -q "KAS frei auf dem Schlüssel: jetzt 200.00, vor Teil B etwa 259.91, am Ende etwa 166.33" $D/out.txt && grep -q "GHOST der Version 3: am Ende 0.25000000" $D/out.txt'
ok "Plan und Frage vor dem ersten Schritt" '[ $(zeile "=== Plan") -lt $(zeile "Alles so ausführen") ] && [ $(zeile "Alles so ausführen") -lt $(zeile "Schritt 2/6") ]'
ok "jeder Sendebefehl mit --ja, keine Rückfrage von ghostctl" '[ $(esends | wc -l) = 6 ] && ! esends | grep -vq -- " --ja " && ! grep -q "# Rückfrage" $D/calls.log'
ok "Teil A im Plan nur geprobt (--dry-run auf mainnet.json vor dem Umbenennen)" 'grep -q "^v2 --network mainnet --state deployments/mainnet.json --dry-run --json repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log'

echo "E2) Antwort N (oder keine Antwort) → Abbruch ohne Senden, nichts umbenannt"
setup e2-nein "$BASE"; v2state; runa n
ok "Abbruch mit Meldung" '[ $(rc) != 0 ] && grep -q "Abgebrochen – nichts gesendet, nichts umbenannt" $D/out.txt'
ok "kein Sendebefehl, nur Proben" '[ -z "$(esends)" ] && grep -q -- "--dry-run --json repay" $D/calls.log'
ok "nichts umbenannt, Sperre frei" '[ -f $D/deployments/mainnet.json ] && isv2 $D/deployments/mainnet.json && [ ! -e $D/deployments/mainnet-v2.json ] && [ ! -e $D/deployments/mainnet-v2.lock ] && lockfree'
setup e2-leer "$BASE"; v2state; (cd $D && ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "ohne Eingabe: Abbruch ohne Senden" '[ $(rc) != 0 ] && [ -z "$(esends)" ] && [ ! -e $D/deployments/mainnet-v2.json ]'

echo "E3) Abbruch mitten in Teil A → Meldung gesendet/offen; erneuter Doppelklick zeigt nur den Rest"
setup e3-mitte '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}],
 "fail":{"close":"Zeitlimit (Attrappe)"}},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; run; cp $D/out.txt $D/out1.txt
ok "1. Lauf: nach dem Tilgen von Vault 1 gestoppt" '[ $(rc) != 0 ] && grep -q "Schließen fehlgeschlagen" $D/out.txt && [ $(esends | wc -l) = 2 ] && ! grep -q "^v3 .* deploy " $D/calls.log'
ok "1. Lauf nennt Gesendetes" 'grep -A1 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "✓ Vault 1 getilgt (0.50000000 GHOST)"'
ok "1. Lauf nennt Offenes ab dem abgebrochenen Schritt" 'grep -A3 "Noch offen (laut Plan):" $D/out.txt | tr "\n" "|" | grep -q "– Vault 1 schließen|  – Vault 2: 1.00000000 von 1.00000000 GHOST Schuld tilgen (mit GHOST der Version 2)|  – Vault 2 schließen"'
python3 - "$D/mock.json" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v2"].pop("fail"); json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "2. Lauf: Plan zeigt nur die restlichen Schritte" 'grep -q "=== Plan: 7 Schritte ===" $D/out.txt && grep -q "1. Vault 1 schließen" $D/out.txt && ! grep -q "Vault 1: .* tilgen" $D/out.txt && grep -q "2. Vault 2: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt'
ok "2. Lauf: wieder genau eine Frage, dann fertig" '[ $(nfrage) = 1 ] && [ $(rc) = 0 ] && grep -q "Fertig: Version 3 läuft" $D/out.txt'
# (nach dem Schließen von Vault 1 rückt Vault 2 auf Nummer 1)
ok "2. Lauf: nur Vault 2 getilgt, beide geschlossen" '[ $(esends | grep -c " repay ") = 1 ] && esends | grep -q "repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" && [ $(esends | grep -c " close ") = 2 ]'

echo "E4) Nach Teil A weniger KAS als geplant → vor Teil B anhalten und erneut fragen"
setup e4-weniger "$(print -r -- "$BASE" | sed 's/"v2":{/"v2":{"kas_back":0.5,/')"; v2state
runa $'j\nn'
ok "zweite Frage vor Teil B nennt die Abweichung" 'grep -q "Vor Teil B weicht der Stand vom bestätigten Plan ab" $D/out.txt && grep -q "KAS auf dem Schlüssel: geplant etwa 259.91, jetzt 230.00" $D/out.txt && grep -q "Teil B so ausführen? \[j/N\]" $D/out.txt'
ok "zeigt Teil B aus dem jetzigen Stand" 'grep -q "Teil B aus dem jetzigen Stand:" $D/out.txt && grep -q "KAS frei auf dem Schlüssel: jetzt 230.00, am Ende etwa 136.42" $D/out.txt'
ok "N: Teil A gesendet, von Teil B nichts" '[ $(rc) != 0 ] && grep -q "Teil B nicht begonnen" $D/out.txt && esends | grep -q " close " && ! esends | grep -q "^v3 "'
ok "Stand nennt Offenes: Teil B" 'grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Orakel, Factory und GHOST"'
: > $D/calls.log; run
ok "erneuter Doppelklick: nur Teil B, eine Frage, keine Abweichung mehr" '[ $(rc) = 0 ] && grep -q "=== Plan: 4 Schritte ===" $D/out.txt && [ $(nfrage) = 1 ] && ! grep -q "weicht" $D/out.txt && esends | grep -q " pool-open "'
setup e4-ja "$(print -r -- "$BASE" | sed 's/"v2":{/"v2":{"kas_back":0.5,/')"; v2state
runa $'j\nj'
ok "j auf die zweite Frage: Teil B läuft durch" '[ $(rc) = 0 ] && [ $(grep -c "so ausführen? \[j/N\]" $D/out.txt) = 2 ] && esends | grep -q " pool-open "'

echo "E5) Orakelkurs verschiebt den Pool-Betrag um mehr als 1 % → vor dem Pool erneut fragen"
setup e5-pool "$(print -r -- "$BASE" | sed 's/"v3":{/"v3":{"price":0.05,/')"; v2state
runa $'j\nn'
ok "Plan rechnete mit dem Börsenkurs (5 KAS)" 'grep -q "Tauschpool mit Kursband anlegen: 5.00000000 KAS und 0.25 GHOST" $D/out.txt'
ok "vor dem Pool: Abweichung genannt und gefragt" 'grep -q "Der Pool-Betrag weicht vom bestätigten Plan ab: jetzt 6.25000000 KAS statt 5.00000000" $D/out.txt && grep -q "Pool mit 6.25000000 KAS und 0.25 GHOST anlegen? \[j/N\]" $D/out.txt'
ok "N: kein pool-open, Deployment/Vault/Prägen gesendet" '[ $(rc) != 0 ] && ! grep -q "pool-open" $D/calls.log && esends | grep -q " mint "'
setup e5-gleich "$(print -r -- "$BASE" | sed 's/"v3":{/"v3":{"price":0.0402,/')"; v2state; run
ok "Abweichung unter 1 %: keine zweite Frage" '[ $(rc) = 0 ] && [ $(nfrage) = 1 ] && ! grep -q "Pool-Betrag weicht" $D/out.txt && esends | grep -q " pool-open "'

echo "E6) EINZELN=1: wie früher, ghostctl fragt je Transaktion; keine Sammelfrage"
setup e6-einzeln "$BASE"; v2state
runa "$(for i in {1..10}; do echo j; done)" EINZELN=1
ok "Exitcode 0, Plan gezeigt, keine Sammelfrage" '[ $(rc) = 0 ] && grep -q "=== Plan" $D/out.txt && [ $(nfrage) = 0 ] && grep -q "EINZELN=1: Jede Transaktion fragt gleich einzeln" $D/out.txt'
ok "je Transaktion eine Rückfrage, kein --ja" '[ $(esends | wc -l) = 6 ] && [ $(grep -c "# Rückfrage" $D/calls.log) = 6 ] && ! esends | grep -q -- " --ja "'
setup e6-nein "$BASE"; v2state
runa n EINZELN=1
ok "N auf die erste Rückfrage: Abbruch, nichts weiter gesendet" '[ $(rc) != 0 ] && [ $(grep -c "# Rückfrage" $D/calls.log) = 1 ] && grep -q "Tilgen fehlgeschlagen" $D/out.txt && ! grep -q " close " $D/calls.log'
setup e6-x "$BASE"; v2state; runa j EINZELN=ja
ok "EINZELN=ja wird abgelehnt" '[ $(rc) != 0 ] && grep -q "EINZELN=ja verstehe ich nicht" $D/out.txt && [ ! -s $D/calls.log ]'

echo "E7) DRY=1 bleibt reiner Probelauf: kein Plan, keine Frage"
setup e7-dry "$BASE"; v2state
(cd $D && DRY=1 ./GHOST-Umzug-v3.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "keine Frage, kein --ja, alles --dry-run" '[ $(rc) = 0 ] && ! grep -q "so ausführen\|=== Plan" $D/out.txt && ! grep -q -- " --ja " $D/calls.log && [ -z "$(esends)" ]'

echo "E8) Pool-Anteile: Plan zählt die GHOST aus dem Pool zum Tilgen, wie der echte Lauf"
setup e8-poolret '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; run
ok "Plan: Rückfluss 10 KAS und 0,25 GHOST, dann ganz tilgen und schließen" 'grep -q "zurück: etwa 10.00000000 KAS und 0.25000000 GHOST" $D/out.txt && grep -q "Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0 schließen" $D/out.txt'
ok "Tilgen mit Pool-GHOST ist geschätzt, nicht geprobt" '! grep -q -- "--dry-run --json repay" $D/calls.log && grep -q -- "--dry-run --json pool-remove" $D/calls.log'
ok "echter Lauf wie geplant, keine zweite Frage" '[ $(rc) = 0 ] && [ $(nfrage) = 1 ] && ! grep -q "weicht" $D/out.txt && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 1.00000000$" && esends | grep -q "close --key keys/mainnet-owner.json --vault 0$"'

# Seit Audit 13 (A13-umzug-2) geht pool-remove mit --min-kas/--min-ghost (Plan
# minus 1 %): Der Pool bewegt sich hier nur innerhalb dieser Toleranz (0,248
# statt 0,25 GHOST zurück), das reicht nicht mehr zum vollen Tilgen.
echo "E9) Pool bewegt sich zwischen Plan und Senden: weniger GHOST zurück → Teil A hält an und fragt erneut"
E9='{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.75,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"49600000"}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup e9-weniger "$E9"; v2state
runa $'j\nn'
ok "Plan wie E8: ganz tilgen und schließen" 'grep -q "Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0 schließen" $D/out.txt'
ok "vor dem Tilgen: Abweichung genannt (geplant/jetzt)" 'grep -q "Teil A weicht vom bestätigten Plan ab" $D/out.txt && grep -q "geplant: Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "jetzt:   Vault 0: 0.99800000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt'
ok "zeigt Teil A aus dem jetzigen Stand und fragt" 'grep -q "Teil A aus dem jetzigen Stand:" $D/out.txt && grep -q "Sicherheit von 60.00000000 auf 0.30 KAS senken" $D/out.txt && grep -q "Teil A so ausführen? \[j/N\]" $D/out.txt'
ok "N: nur pool-remove gesendet, kein repay/withdraw/close, nichts von Teil B" '[ $(rc) != 0 ] && [ "$(esends | grep -c .)" = 1 ] && esends | grep -q " pool-remove " && grep -q "Teil A angehalten" $D/out.txt'
ok "Noch offen nennt die neu gerechneten Schritte" 'grep -A2 "Noch offen (laut Plan):" $D/out.txt | tr "\n" "|" | grep -q "– Vault 0: 0.99800000 von 1.00000000 GHOST Schuld tilgen (mit GHOST der Version 2)|  – Vault 0: Sicherheit von 60.00000000 auf 0.30 KAS senken"'
setup e9-ja "$E9"; v2state
runa $'j\nj\nj'
ok "j: gesendet wird, was die zweite Frage zeigte; Teil B ohne dritte Frage" '[ $(rc) = 0 ] && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.99800000$" && esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 0 --keep 0.30$" && ! esends | grep -q " close " && [ $(grep -c "so ausführen? \[j/N\]" $D/out.txt) = 2 ] && ! grep -q "Teil B so ausführen" $D/out.txt && esends | grep -q " pool-open "'

setup e9-einzeln "$E9"; v2state
runa "$(for i in {1..10}; do echo j; done)" EINZELN=1
ok "EINZELN=1: Abweichung gezeigt, keine Sammelfrage, ghostctl fragt je Transaktion (0,998 GHOST)" '[ $(rc) = 0 ] && grep -q "Teil A weicht vom bestätigten Plan ab" $D/out.txt && ! grep -q "so ausführen? \[j/N\]" $D/out.txt && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.99800000$" && ! esends | grep -q -- " --ja "'

echo "E10) Mehr GHOST zurück als geplant → nicht still ganz tilgen und schließen, erst fragen"
setup e10-mehr '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.5,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"100000000"}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; runa $'j\nn'
ok "Plan: teilweise tilgen und herausnehmen" 'grep -q "Vault 0: 0.75000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0: Sicherheit von 60.00000000 auf" $D/out.txt'
ok "vor dem Tilgen gefragt, N: kein repay, kein close" '[ $(rc) != 0 ] && grep -q "jetzt:   Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && ! esends | grep -q " repay \| close \| withdraw "'

echo "E11) Orakel von Version 2 fällt zwischen Plan und Senden (beim Pool-Abzug) → Tilgen wie geplant, Herausnehmen fragt erneut"
setup e11-kurs '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.25,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"kasUsd":0.02},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; runa $'j\nn'
ok "Plan: auf 27.50 KAS senken; jetzt 55.00 → Frage" 'grep -q "geplant: Vault 0: Sicherheit von 60.00000000 auf 27.50 KAS senken" $D/out.txt && grep -q "jetzt:   Vault 0: Sicherheit von 60.00000000 auf 55.00 KAS senken" $D/out.txt && grep -q "Teil A so ausführen" $D/out.txt'
ok "N: getilgt, nicht herausgenommen; offen: Herausnehmen mit neuem Betrag" '[ $(rc) != 0 ] && esends | grep -q " repay --key keys/mainnet-owner.json --vault 0 --ghost 0.50000000$" && ! esends | grep -q " withdraw " && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "auf 55.00 KAS senken"'

echo "E12) Geplanter Schritt entfällt (Vault weg) → vor Teil B fragen"
setup e12-weg '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"vaults":[]},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; runa $'j\nn'
ok "entfallende Schritte genannt, Frage, N: nichts von Teil B" '[ $(rc) != 0 ] && grep -q "nach dem jetzigen Stand entfällt:" $D/out.txt && grep -q "Ohne diese Schritte mit Teil B weitermachen? \[j/N\]" $D/out.txt && ! esends | grep -q "^v3 "'

echo "E13) Leerer Plan → Ende ohne Frage und ohne Teil A/B"
setup e13-leer '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"pool":{"kas":"5","ghost":"0.25"},
 "vaults":[{"owner":"@A","covenantId":"y2","debtGhost":0.5,"collateralKas":50}]}}'
v3state; v2state mainnet-v2.json; run
ok "Exitcode 0, keine Frage, kein Teil A/B, nichts gesendet" '[ $(rc) = 0 ] && grep -q "Nichts mehr zu senden" $D/out.txt && [ $(nfrage) = 0 ] && ! grep -q "Teil A: Version 2 abbauen\|Teil B: Version 3 anlegen" $D/out.txt && [ -z "$(esends)" ]'

echo "\n== U: Audit 13 (Umzug) =="
# Welt nach dem Beweistest X1 der Prüfer: fremder Vault x1 vor den eigenen c1 und c2
U1W='{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}]MOVE},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
U1REST='[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}]'
# v2welt <python-Ausdruck über vs (Covenant-ID → Vault) und g (eigene GHOST v2)>
v2welt() { python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); vs={v["covenantId"]:v for v in m["v2"]["vaults"]}; g=m["v2"]["keys"][sys.argv[2]]["ghost"]; sys.exit(0 if eval(sys.argv[3]) else 1)' $D/mock.json $A "$1"; }

echo "U1) A13-umzug-1: fremder Vault verschwindet MIT dem Tilgen (Nummer trifft c2) → nur 0,5 GHOST, Prüfung danach hält an, kein close"
setup u1-nachher "${U1W/MOVE/,\"move\":{\"vaults\":$U1REST\}}"; v2state; run
ok "bestätigt war: Vault 1 (c1) mit 0,5 GHOST tilgen" 'grep -q "1. Vault 1: 0.50000000 von 0.50000000 GHOST Schuld tilgen" $D/out.txt'
ok "repay immer mit --ghost (gedeckelt auf den bestätigten Betrag)" 'esends | grep -q "repay --key keys/mainnet-owner.json --vault 1 --ghost 0.50000000$" && ! esends | grep -q "repay --key keys/mainnet-owner.json --vault [0-9]*$"'
ok "Prüfung nach dem Senden hält an: kein close, nichts von Teil B" '[ $(rc) != 0 ] && grep -q "nicht genau der bestätigte Vault wie erwartet geändert" $D/out.txt && ! esends | grep -q " close \| withdraw " && ! esends | grep -q "^v3 "'
ok "Welt: c1 unberührt, c2 nur um 0,5 getilgt und offen, 1,5 GHOST übrig" 'v2welt "vs[\"c1\"][\"debtGhost\"] == 0.5 and vs[\"c2\"][\"debtGhost\"] == 0.5 and vs[\"c2\"][\"collateralKas\"] == 60 and g == 1.5"'
ok "keine Erfolgsmeldung ohne Hinweis: Gesendetes als abweichend markiert, Offenes genannt" 'grep -A1 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "Vault 1 getilgt (0.50000000 GHOST) – gesendet, aber die Wirkung weicht ab" && grep -q "Vault c1 sollte jetzt sein: Schuld 0.00000000 GHOST" $D/out.txt && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Vault 1 schließen"'

echo "U2) Nummer verschiebt sich zwischen Tilgen und Schließen → Halt ohne Senden von close"
setup u2-schliessen "${U1W/MOVE/,\"move_after\":{\"repay\":{\"vaults\":$U1REST\}\}}"; v2state
python3 - "$D/mock.json" <<'PY'
# c1 ist nach dem Tilgen schuldenfrei: die Bewegung übernimmt das
import json,sys; m=json.load(open(sys.argv[1])); m["v2"]["move_after"]["repay"]["vaults"][0]["debtGhost"]=0; json.dump(m,open(sys.argv[1],"w"))
PY
run
ok "c1 getilgt (Nummer 1), dann angehalten: Vault-Nummer verschoben" 'esends | grep -q "repay --key keys/mainnet-owner.json --vault 1 --ghost 0.50000000$" && [ $(rc) != 0 ] && grep -q "Vault-Nummer verschoben: Nummer 1 trägt nicht mehr den bestätigten Vault c1 (er hat jetzt Nummer 0)" $D/out.txt'
ok "kein close gesendet, nichts von Teil B" '! esends | grep -q " close " && ! esends | grep -q "^v3 "'
ok "Welt: c2 unberührt (1 GHOST Schuld, 60 KAS)" 'v2welt "vs[\"c2\"][\"debtGhost\"] == 1 and vs[\"c2\"][\"collateralKas\"] == 60 and vs[\"c1\"][\"debtGhost\"] == 0"'

echo "U3) Nummer verschiebt sich zwischen Status und Senden (vor dem ersten Tilgen) → Halt ohne jedes Senden"
# status#7 = der Status, aus dem Teil A die Nummer von c1 liest (4 im Plan, 3 im echten Lauf)
setup u3-vorher "${U1W/MOVE/,\"move_after\":{\"status#7\":{\"vaults\":$U1REST\}\}}"; v2state; run
ok "Halt vor dem Senden: Vault-Nummer verschoben" '[ $(rc) != 0 ] && grep -q "Vault-Nummer verschoben: Nummer 1 trägt nicht mehr den bestätigten Vault c1 (er hat jetzt Nummer 0)" $D/out.txt && grep -q "repay ist nicht gesendet" $D/out.txt'
ok "nichts gesendet, Welt unverändert" '[ -z "$(esends)" ] && v2welt "vs[\"c1\"][\"debtGhost\"] == 0.5 and vs[\"c2\"][\"debtGhost\"] == 1 and g == 2"'
ok "Stand: nichts gesendet, offen ab dem Tilgen" 'grep -q "In diesem Lauf wurde nichts gesendet." $D/out.txt && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Vault 1: 0.50000000 von 0.50000000 GHOST Schuld tilgen"'
: > $D/calls.log; run
ok "erneuter Doppelklick: Plan mit den neuen Nummern, dann durch" '[ $(rc) = 0 ] && grep -q "1. Vault 0: 0.50000000 von 0.50000000 GHOST Schuld tilgen" $D/out.txt && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.50000000$" && [ $(esends | grep -c " close ") = 2 ]'

echo "U4) A13-umzug-2/6: pool-remove mit Mindestbeträgen aus dem Plan minus 1 %"
U4W='{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"}MOVE,
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup u4-min "${U4W/MOVE/}"; v2state; run
ok "Zusammenfassung nennt Mindestbeträge und Toleranz" 'grep -q "mindestens 9.90000000 KAS und 0.24750000 GHOST (Toleranz 1 %) – kommt weniger zurück, sendet ghostctl nicht" $D/out.txt'
ok "Zusammenfassung nennt die Pool-Toleranz beim Anlegen" 'grep -q "bis ± 1 % Abweichung geht ohne erneute Frage hinaus" $D/out.txt'
ok "Probe und Senden mit --min-kas/--min-ghost" 'grep -q -- "--dry-run --json pool-remove --key keys/mainnet-owner.json --percent 100 --min-kas 9.90000000 --min-ghost 0.24750000$" $D/calls.log && esends | grep -q -- "--ja pool-remove --key keys/mainnet-owner.json --percent 100 --min-kas 9.90000000 --min-ghost 0.24750000$"'
ok "Lauf endet mit 0" '[ $(rc) = 0 ]'
# wie X2 der Prüfer: der Pool fällt beim Senden auf ein Zehntel
setup u4-verschoben "${U4W/MOVE/,\"move\":{\"pool\":{\"shares\":\"1000\",\"kasSompi\":\"200000000\",\"ghostUnits\":\"5000000\"\}\}}"; v2state; run
ok "Pool verschoben: ghostctl sendet nicht (Mindestbeträge), Abbruch vor dem Tilgen" '[ $(rc) != 0 ] && grep -q "Der Pool hat sich verschoben" $D/out.txt && grep -q "Abziehen fehlgeschlagen" $D/out.txt && ! esends | grep -q " repay \| close \| withdraw "'
ok "Welt: Anteile und GHOST unverändert" 'python3 -c "import json,sys; m=json.load(open(sys.argv[1])); k=m[\"v2\"][\"keys\"][sys.argv[2]]; sys.exit(0 if k[\"lpShares\"]==\"500\" and k[\"ghost\"]==0.8 else 1)" $D/mock.json $A'

echo "U5) A13-umzug-4: SIGTERM während des Prägens → Stand (gesendet/offen) wird gezeigt"
setup u5-signal '{"v2":{"kasUsd":0.04,"keys":{},"vaults":[]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[],"sleep_after":{"mint":4}}}'
v3state
(cd $D && ./GHOST-Umzug-v3.command <$JA >$D/out.txt 2>&1 & echo $! > $D/spid; wait $!; echo $? > $D/rc) &
for i in {1..100}; do grep -q "wirkt, wartet mint" $D/calls.log 2>/dev/null && break; python3 -c 'import time; time.sleep(0.2)'; done
kill -TERM $(cat $D/spid); wait
ok "Skript endet mit 130, Sperre frei" '[ $(rc) = 130 ] && lockfree'
ok "Abbruch nennt den laufenden Schritt" 'grep -q "Abgebrochen (Signal)" $D/out.txt && grep -q "Unterbrochen während: 0.5 GHOST geprägt (Vault 0)" $D/out.txt'
ok "Stand: gesendet (Vault eröffnet) und offen (Prägen, Pool)" 'grep -A1 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "✓ Vault mit 50 KAS eröffnet" && grep -A2 "Noch offen (laut Plan):" $D/out.txt | tr "\n" "|" | grep -q "– 0.5 GHOST prägen|  – Tauschpool"'

echo "U6) A13-umzug-5/3: Zusammenfassung nennt Netz, Schlüsseldatei, Adresse, x-only und mögliche Zusammenführungen"
setup u6-kopf "$BASE"; v2state; runa n
ok "Netz, Schlüsseldatei, Adresse, x-only vor der Frage" 'sed -n "/=== Plan/,/so ausführen/p" $D/out.txt > $D/plan.txt; grep -q "^Netz: *mainnet$" $D/plan.txt && grep -q "^Schlüsseldatei: keys/mainnet-owner.json (${D:A}/keys/mainnet-owner.json)$" $D/plan.txt && grep -q "^Adresse: *kaspa:attrappeaaaaaaaaaaaaaaaa$" $D/plan.txt && grep -q "^x-only: *$A$" $D/plan.txt'
ok "mögliche Zusammenführung bei Tilgen und Pool, in den Gebühren gezählt" '[ $(grep -c "dazu ggf. 1 Transaktion „GHOST zusammenführen“" $D/plan.txt) = 2 ] && grep -q "dazu ggf. bis zu 2 × „GHOST zusammenführen“" $D/plan.txt'
ok "N: nichts gesendet" '[ $(rc) != 0 ] && [ -z "$(esends)" ]'
setup u6-keys "$BASE"; v2state; mkdir -p $D/alt; echo "{\"mock_xonly\":\"$A\"}" > $D/alt/x-owner.json; echo '{"secrets":["x","x","x","x","x"]}' > $D/alt/x-committee.json
runa n KEYS=alt/x
ok "KEYS aus der Umgebung: Zusammenfassung zeigt alt/x-owner.json" 'sed -n "/=== Plan/,/so ausführen/p" $D/out.txt | grep -q "^Schlüsseldatei: alt/x-owner.json (${D:A}/alt/x-owner.json)$"'

echo "U7) A13-umzug-7: --keep in Sompi ohne Float-Rauschen (0,1 GHOST bei 0,022 USD → 10,00 KAS, nicht 10,01)"
setup u7-keep '{"v2":{"kasUsd":0.022,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.1,"collateralKas":60},{"owner":"@A","covenantId":"c2","debtGhost":0.3,"collateralKas":60}]},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; run
ok "Plan und Senden: --keep 10.00 (statt 10.01)" 'grep -q "Vault 0: Sicherheit von 60.00000000 auf 10.00 KAS senken" $D/out.txt && esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 0 --keep 10.00$"'
ok "0,3 GHOST bei 0,022 USD → 30,00 KAS" 'esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 1 --keep 30.00$"'
ok "Prüfung nach dem Herausnehmen besteht, Lauf endet mit 0" '[ $(rc) = 0 ] && ! grep -q "nicht genau der bestätigte Vault" $D/out.txt'

echo "Z1) Verstreute GHOST: Probe des Tilgens scheitert nur im Probelauf (ghostctl führt erst zusammen) – Plan und DRY=1 laufen weiter"
Z='{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}],
 "dry_fail":{"repay":"Fehler: Die GHOST liegen auf mehr als zwei UTXOs verteilt; vor dieser Aktion wird zuerst zusammengeführt. Das lässt sich nicht als Probelauf vorab zeigen"}},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup z1-plan "$Z"; v2state; runa n
ok "Plan erscheint trotz gescheiterter Probe, Tilgen geschätzt mit Hinweis aufs Zusammenführen" 'grep -q "=== Plan" $D/out.txt && grep -q "vorher 1–2 Transaktionen „GHOST zusammenführen“" $D/out.txt && ! grep -q "Probe (--dry-run) für" $D/out.txt'
ok "N: nichts gesendet" '[ $(rc) != 0 ] && [ -z "$(esends)" ]'
setup z1-echt "$Z"; v2state; run
ok "echter Lauf tilgt und schließt beide Vaults (der zweite rückt nach dem Schließen auf Nummer 0)" '[ $(rc) = 0 ] && [ "$(esends | grep -c "repay --key keys/mainnet-owner.json --vault 0 --ghost ")" = 2 ] && esends | grep -q "vault 0 --ghost 1.00000000$" && [ "$(esends | grep -c " close ")" = 2 ]'
setup z1-dry "$Z"; v2state; runa "" DRY=1
ok "DRY=1: Hinweis statt Abbruch, weiter bis Teil B, nichts gesendet" '[ $(rc) = 0 ] && grep -q "vorher werden deine GHOST zusammengeführt" $D/out.txt && ! grep -q "Tilgen fehlgeschlagen" $D/out.txt && [ -z "$(esends)" ]'
setup z1-andere '{"v2":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20}],"dry_fail":{"repay":"Fehler: etwas ganz anderes"}},
 "v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v2state; runa n
ok "andere Probe-Fehler brechen weiterhin ab, nichts gesendet" '[ $(rc) != 0 ] && grep -q "Probe (--dry-run) für „Vault 0 tilgen“ gescheitert" $D/out.txt && [ -z "$(esends)" ]'

echo "\nErgebnis: $pass bestanden, $failn fehlgeschlagen"
if (( failn )); then
  printf '  – %s\n' "${FAILED[@]}"
  echo "Fallordner bleibt zur Ansicht: $CASES"
elif [ "$KEEP" = 1 ]; then
  echo "Fallordner: $CASES"
else
  rm -rf "$CASES"
fi
exit $(( failn > 0 ))
