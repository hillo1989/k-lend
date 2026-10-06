#!/bin/zsh
# Szenario-Tests für GHOST-Umzug-v4.command (Umzug v3 → v4) gegen Attrappen:
# kein Netz, keine echten Schlüssel, nichts wird gesendet. Vorbild:
# tests/umzug/run.zsh (Umzug v2 → v3); dessen Szenarien sind auf v3 → v4
# übertragen, dazu kommen die Fälle von v4 (Teil V).
#
# Aufruf (aus dem Projektordner oder von überall):
#   tests/umzug-v4/run.zsh                  – prüft ../../GHOST-Umzug-v4.command
#   SCRIPT=<pfad> tests/umzug-v4/run.zsh    – prüft eine andere Fassung (Rückbau-Probe)
#   KEEP=1 tests/umzug-v4/run.zsh           – Fallordner auch bei Erfolg behalten
#
# Jeder Fall läuft in einem eigenen Ordner unter einem Temp-Verzeichnis ($CASES,
# Vorgabe mktemp): Kopie des Skripts, Attrappen als ./ghostctl (v4) und
# bin/ghostctl-v3, Schlüssel- und Zustandsdateien nur mit Attrappen-Inhalt.
# Nach Erfolg wird der Temp-Ordner entfernt, bei Fehlern bleibt er stehen.
#
# Teile: R = Regression (Audit 11), N = A12-4 (Schlüsselwahl, Schritt 5),
# J/D = A12-17 (Journal, Probelauf), C = Restpunkte der Nachprüfung (Gegenprobe
# ME = ME4, x-only, Symlink/Hardlink), E = einmal bestätigen (Plan, eine Frage,
# Abbruch und Fortsetzen, Abweichung vor Teil B und vor dem Pool, EINZELN=1,
# DRY=1), U = Audit 13, V = Version 4 (Versionserkennung, Zinsdatei,
# Unterzeichner, Deployment ohne --probe, Zins von Version 3, Abbruch und
# Fortsetzung an jedem Schritt, Ctrl+C, schon vorhandenes v4).
# Nicht übernommen: Teil P der Vorlage (Nachstellung der Audit-12-Prüfer mit
# deren stub.py und Sendefolge für v2 → v3) – die Fälle stecken in R, N, J, E.
# Attrappen: mock_ghostctl.py (Welt in mock.json); pgrep wird als Attrappe in
# $CASES/mockbin erzeugt (meldet einen Agenten nur mit MOCK_AGENT=1 bzw. MOCK_COUNT).
SP=${0:A:h}
SCRIPT=${SCRIPT:-$SP/../../GHOST-Umzug-v4.command}
SCRIPT=${SCRIPT:A}
[ -f "$SCRIPT" ] || { echo "Skript $SCRIPT fehlt"; exit 2; }
CASES=${CASES:-$(mktemp -d "${TMPDIR:-/tmp}/ghost-umzug-v4.XXXXXX")}
mkdir -p "$CASES"; CASES=${CASES:A}
mkdir -p $CASES/mockbin
cat > $CASES/mockbin/pgrep <<'EOF'
#!/bin/zsh
# Attrappe: meldet einen laufenden Agenten nur mit MOCK_AGENT=1, mit MOCK_COUNT erst ab dem 2. Aufruf
if [ -n "$MOCK_COUNT" ]; then
  n=$(( $(cat $MOCK_COUNT 2>/dev/null || echo 0) + 1 )); echo $n > $MOCK_COUNT
  [ $n -ge 2 ] && { echo "4712 ./ghostctl oracle-feed"; exit 0; }; exit 1
fi
[ "$MOCK_AGENT" = 1 ] && { echo "4711 caffeinate -i ./ghostctl --network mainnet --state deployments/mainnet.json --ja agent --key keys/mainnet-owner.json"; exit 0; }
[ "$MOCK_AGENT" = 3 ] && { echo "4713 bin/ghostctl-v3 --network mainnet --state deployments/mainnet-v3.json --ja agent --key keys/mainnet-owner.json"; exit 0; }
exit 1
EOF
chmod +x $CASES/mockbin/pgrep
MOCKBIN=$CASES/mockbin
export PATH="$MOCKBIN:$PATH"
A=$(python3 -c 'print("a"*64)'); B=$(python3 -c 'print("b"*64)'); C=$(python3 -c 'print("c"*64)')
DD=$(python3 -c 'print("d"*64)'); E=$(python3 -c 'print("e"*64)')
pass=0; failn=0; FAILED=()
ok() { if eval "$2"; then echo "  ✓ $1"; pass=$((pass+1)); else echo "  ✗ $1"; failn=$((failn+1)); FAILED+=("$CASE: $1"); fi; }
# setup <name> <mock.json mit Platzhaltern @A @B @C @D @E>
setup() {
  CASE=$1; D=$CASES/$1; rm -rf $D; mkdir -p $D/bin $D/keys $D/deployments
  cp $SCRIPT $D/GHOST-Umzug-v4.command; chmod +x $D/GHOST-Umzug-v4.command
  ln -s $SP/mock_ghostctl.py $D/ghostctl; ln -s $SP/mock_ghostctl.py $D/bin/ghostctl-v3
  print -r -- "$2" | sed "s/@A/$A/g; s/@B/$B/g; s/@C/$C/g; s/@D/$DD/g; s/@E/$E/g" > $D/mock.json
  python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); assert set(m) >= {"v3","v4"}' $D/mock.json 2>/dev/null || echo "  ! $1: mock.json ungültig"
  echo "{\"mock_xonly\":\"$A\"}" > $D/keys/mainnet-owner.json
  echo '{"secrets":["x","x","x","x","x"]}' > $D/keys/mainnet-committee.json
  echo '{"secrets":["x"]}' > $D/keys/mainnet-signer.json
  : > $D/calls.log
}
v3state() { echo '{"network":"mainnet","vault_params":{"mcr_bps":20000,"treasury":"ab"}}' > $D/deployments/${1:-mainnet.json}; }
v4state() { echo '{"network":"mainnet","register":{"cov":"r4"},"vault_params":{"treasury":"ab","max_debt":1}}' > $D/deployments/mainnet.json; }
# Echte Läufe fragen einmal „Alles so ausführen? [j/N]“ (und erneut, wenn Teil B
# vom Plan abweicht): $JA beantwortet bis zu zehn Fragen mit j. Probeläufe
# (DRY=1) fragen nie und laufen weiter mit </dev/null.
JA=$CASES/ja.txt; for i in {1..10}; do echo j; done > $JA
run() { (cd $D && ./GHOST-Umzug-v4.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc; }
# Lauf mit eigenen Antworten: runa '<antworten, je Zeile eine>' [VAR=wert …]
runa() { local a=$1; shift; print -r -- "$a" > $D/antw.txt; (cd $D && env "$@" ./GHOST-Umzug-v4.command <$D/antw.txt >$D/out.txt 2>&1); echo $? > $D/rc; }
rc() { cat $D/rc; }
sends() { grep -E "^v[34] .* (repay|close|withdraw|deploy|open-vault|mint|pool-[a-z]+) " $D/calls.log; }
nopen() { grep -c "^v4 .* open-vault " $D/calls.log; }
lockfree() { [ ! -d $D/deployments/.umzug-v4.lock ]; }
jtarget() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("target"))' "$1"; }
isv3() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if "register" not in d and "treasury" in (d.get("vault_params") or {}) else 1)' "$1" 2>/dev/null; }
isv4() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); sys.exit(0 if "register" in d else 1)' "$1" 2>/dev/null; }
hasjournal() { grep -q '"journal": *"v3"' "$1" 2>/dev/null; }

BASE='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300}]}}'
# Journal von Version 3, das nach einem unterbrochenen Umbenennen noch auf mainnet.json zeigt
JV3='{"action":"Tilgen","txid":"00","target":"%s","next":{"network":"mainnet","vault_params":{"mcr_bps":20000,"treasury":"ab"},"journal":"v3"}}'

echo "Geprüft: $SCRIPT"
echo "Fallordner: $CASES"
echo "\n== R: Regression =="
echo "R1) Agent läuft → Abbruch vor jedem Schritt"
setup r1-agent "$BASE"; v3state
MOCK_AGENT=1 run
ok "Exitcode ≠ 0" '[ $(rc) != 0 ]'
ok "Meldung nennt den Agenten" 'grep -q "läuft noch ein GHOST-Agent" $D/out.txt'
ok "nichts umbenannt" '[ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v3.json ]'
ok "kein ghostctl-Aufruf" '[ ! -s $D/calls.log ]'
ok "Sperre wieder frei" 'lockfree'

echo "R2) zweiter Umzug läuft (Sperre) → Abbruch, fremde Sperre bleibt"
setup r2-lock "$BASE"; v3state; mkdir $D/deployments/.umzug-v4.lock; echo 999 > $D/deployments/.umzug-v4.lock/pid
run
ok "Abbruch" '[ $(rc) != 0 ] && grep -q "anderer Umzug läuft schon (PID 999" $D/out.txt'
ok "fremde Sperre bleibt" '[ -f $D/deployments/.umzug-v4.lock/pid ]'
ok "nichts umbenannt, kein Aufruf" '[ -f $D/deployments/mainnet.json ] && [ ! -s $D/calls.log ]'

echo "R3) DRY=ja → Abbruch; DRY=1 → Probelauf ohne Umbenennen"
setup r3-dryx "$BASE"; v3state
(cd $D && DRY=ja ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "DRY=ja abgelehnt" '[ $(rc) != 0 ] && grep -q "verstehe ich nicht" $D/out.txt'
setup r3-dry1 "$BASE"; v3state
(cd $D && DRY=1 ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "Probelauf endet mit 0" '[ $(rc) = 0 ]'
ok "Probelauf nennt sich so" 'grep -q "PROBELAUF" $D/out.txt'
ok "Probelauf benennt nicht um" '[ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v3.json ]'
ok "alle Sendebefehle mit --dry-run" '! sends | grep -vq -- "--dry-run"'
ok "Probelauf tilgt den eigenen Vault" 'grep -q "^v3 .*--dry-run repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log'

echo "R4) DRY=0 = echter Lauf: Umbenennen samt Journal, Tilgen/Schließen nur eigene, Vault/Prägen am eigenen, Pool"
setup r4-full "$BASE"; v3state
echo '{"action":"Tilgen","txid":"00","target":"deployments/mainnet.json","next":{"x":1}}' > $D/deployments/mainnet.pending.json
echo '{"network":"mainnet"}' > $D/deployments/mainnet.deploy.json
: > $D/deployments/mainnet.lock
echo '{"network":"mainnet","zins":"v3"}' > $D/deployments/mainnet-zins.json; : > $D/deployments/mainnet-zins.lock
(cd $D && DRY=0 ./GHOST-Umzug-v4.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "Zustand umbenannt" '[ -f $D/deployments/mainnet-v3.json ]'
ok "Journal umbenannt, Ziel umgeschrieben" '[ "$(jtarget $D/deployments/mainnet-v3.pending.json)" = deployments/mainnet-v3.json ]'
ok "Journal-Inhalt sonst gleich" 'python3 -c "import json,sys; j=json.load(open(\"$D/deployments/mainnet-v3.pending.json\")); sys.exit(0 if j[\"txid\"]==\"00\" and j[\"next\"]=={\"x\":1} else 1)"'
ok "Deploy-Fortschritt und Sperre umbenannt" '[ -f $D/deployments/mainnet-v3.deploy.json ] && [ -f $D/deployments/mainnet-v3.lock ]'
ok "alte Namen weg (verschoben, nichts gelöscht)" '[ ! -f $D/deployments/mainnet.pending.json ] && [ ! -f $D/deployments/mainnet.deploy.json ]'
ok "v3: nur eigener Vault getilgt und geschlossen" 'grep -q "^v3 .*repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log && grep -q "^v3 .*close --key keys/mainnet-owner.json --vault 1$" $D/calls.log && ! grep "^v3 .*\(repay\|close\|withdraw\)" $D/calls.log | grep -q "vault 0"'
ok "Schlüsselliste aus dem Ordner der Besitzer-Datei" 'grep -q "^v3 .*--json keys --dir keys$" $D/calls.log && grep -q "^v4 .*--json keys --dir keys$" $D/calls.log'
ok "Zinsdatei und ihre Sperre umbenannt, Inhalt von Version 3 erhalten" 'grep -q "\"zins\": *\"v3\"" $D/deployments/mainnet-v3-zins.json && [ -f $D/deployments/mainnet-v3-zins.lock ] && [ ! -f $D/deployments/mainnet-zins.lock ]'
ok "v4 deploy genau so: Besitzer, Unterzeichner, --ja, ohne --probe/--rate/--threshold" '[ "$(grep -c "^v4 .* deploy " $D/calls.log)" = 1 ] && grep -q "^v4 --network mainnet --state deployments/mainnet.json --ja deploy --key keys/mainnet-owner.json --committee keys/mainnet-signer.json$" $D/calls.log'
ok "mainnet.json ist Version 4, neue Zinsdatei von Version 4" 'isv4 $D/deployments/mainnet.json && grep -q "\"attrappe\": *\"v4\"" $D/deployments/mainnet-zins.json'
ok "Komitee-Datei von Version 3 unberührt" 'grep -q "\"secrets\":\[\"x\",\"x\",\"x\",\"x\",\"x\"\]" $D/keys/mainnet-committee.json'
ok "Schluss: Agent mit GHOST-Agent starten.command neu starten, Unterzeichner keys/mainnet-signer.json" 'grep -q "Jetzt den GHOST-Agenten mit „GHOST-Agent starten.command“ neu starten" $D/out.txt && grep -q "und läuft mit keys/mainnet-signer.json als Unterzeichner." $D/out.txt'
ok "Vault eröffnet und am NEUEN eigenen Vault (1) geprägt, nicht an 0" '[ $(nopen) = 1 ] && grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
ok "Pool angelegt" 'grep -q "^v4 .*pool-open --key keys/mainnet-owner.json --kas 6.25000000 --ghost 0.25$" $D/calls.log'
ok "kein Merker liegen geblieben" '[ ! -e $D/deployments/.umzug-v4-vault.lock ]'
ok "Sperre wieder frei" 'lockfree'

echo "R5) Wiederaufnahme: eigener Vault ohne Schuld → nur prägen; Pool fehlt GHOST/KAS → Abbruch vor pool-open"
setup r5-resume '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300},{"owner":"@A","covenantId":"y2","debtGhost":0,"collateralKas":50}],
 "fail":{"mint":"Zeitlimit (Attrappe)"}}}'
v4state
run
ok "kein zweites open-vault" '[ $(nopen) = 0 ]'
ok "Prägen an eigenem Vault 1 versucht" 'grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 1 " $D/calls.log'
ok "Prägen scheitert mit Hinweis, kein pool-open" '[ $(rc) != 0 ] && grep -q "prägt nach" $D/out.txt && ! grep -q "pool-open" $D/calls.log'
python3 - "$D/mock.json" "$A" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v4"].pop("fail"); m["v4"]["vaults"][1]["debtGhost"]=0.5; m["v4"]["keys"][sys.argv[2]]["ghost"]=0.1; json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "eigener Vault hat Schuld → Schritt 5 übersprungen" 'grep -q "schon einen eigenen Vault" $D/out.txt && ! grep -q "mint\|open-vault" $D/calls.log'
ok "zu wenig GHOST → klare Meldung, kein pool-open" '[ $(rc) != 0 ] && grep -q "Für den Pool fehlen GHOST" $D/out.txt && ! grep -q "pool-open" $D/calls.log'
python3 - "$D/mock.json" "$A" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v4"]["keys"][sys.argv[2]]["ghost"]=1; m["v4"]["keys"][sys.argv[2]]["kas"]=5; json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "zu wenig KAS → klare Meldung, kein pool-open" '[ $(rc) != 0 ] && grep -q "Für den Pool fehlen KAS" $D/out.txt && ! grep -q "pool-open" $D/calls.log'

echo "R6) Journal mit fremdem Ziel → nichts umbenannt"
setup r6-journal "$BASE"; v3state
echo '{"action":"x","txid":"00","target":"deployments/anderes.json","next":{}}' > $D/deployments/mainnet.pending.json
run
ok "Abbruch mit Hinweis" '[ $(rc) != 0 ] && grep -q "bitte erst mit bin/ghostctl-v3 status klären" $D/out.txt'
ok "nichts umbenannt" '[ -f $D/deployments/mainnet.json ] && [ -f $D/deployments/mainnet.pending.json ] && [ ! -f $D/deployments/mainnet-v3.json ]'

echo "R7) Ziel existiert schon (mainnet-v3.lock) → nichts umbenannt"
setup r7-clash "$BASE"; v3state; : > $D/deployments/mainnet-v3.lock
run
ok "Abbruch" '[ $(rc) != 0 ] && grep -q "Gibt es schon" $D/out.txt'
ok "nichts umbenannt" '[ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v3.json ]'

echo "R8) unlesbare Zustandsdatei → nicht als v3 umbenennen"
setup r8-broken "$BASE"; echo '{kaputt' > $D/deployments/mainnet.json
run
ok "Abbruch, Datei bleibt" '[ $(rc) != 0 ] && grep -q "nicht lesbar" $D/out.txt && [ -f $D/deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet-v3.json ]'

echo "R9) Agent startet zwischen Teil A und Teil B → Abbruch vor dem Deployment"
setup r9-agentB "$BASE"; v3state
(cd $D && MOCK_COUNT=$D/pgrep.count ./GHOST-Umzug-v4.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
ok "Teil A lief, dann Abbruch vor deploy" 'grep -q "^v3 .*close" $D/calls.log && ! grep -q "^v4 .* deploy " $D/calls.log && [ $(rc) != 0 ] && grep -q "oracle-feed" $D/out.txt'

echo "R10) Restschuld: teilweise tilgen, Sicherheit auf 220 % senken; zwei Umzüge gleichzeitig → höchstens ein Vault"
setup r10-rest '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.3,"lpShares":"0","kas":1}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":600}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state
run
ok "Teil-Tilgung 0,3 und Abheben bis 38,5 KAS" 'grep -q "^v3 .*repay --key keys/mainnet-owner.json --vault 0 --ghost 0.30000000$" $D/calls.log && grep -q "^v3 .*withdraw --key keys/mainnet-owner.json --vault 0 --keep 38.50$" $D/calls.log && ! grep -q "^v3 .*close" $D/calls.log'
setup r10-twice "$BASE"; v3state
(cd $D && ./GHOST-Umzug-v4.command <$JA >$D/out1.txt 2>&1 & ; cd $D && ./GHOST-Umzug-v4.command <$JA >$D/out2.txt 2>&1; wait)
ok "höchstens ein deploy und ein open-vault" '[ $(grep -c " deploy " $D/calls.log) -le 1 ] && [ $(nopen) -le 1 ]'

echo "\n== N: A12-4 Schlüsselwahl und Schritt 5 =="
N1='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100},"@C":{"ghost":0,"kas":1}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200},"@C":{"ghost":0,"kas":1}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300}]}}'
echo "N1) keys/alt-mainnet-owner.json (anderer Schlüssel) steht vor keys/mainnet-owner.json, zwei Doppelklicks"
setup n1-altkey "$N1"; v3state; echo "{\"mock_xonly\":\"$C\"}" > $D/keys/alt-mainnet-owner.json
run; cp $D/out.txt $D/out1.txt; cp $D/rc $D/rc1; run
ok "1. Lauf endet mit 0" '[ $(cat $D/rc1) = 0 ]'
ok "v3: eigener Vault (1) getilgt und geschlossen" 'grep -q "^v3 .*repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log && grep -q "^v3 .*close --key keys/mainnet-owner.json --vault 1$" $D/calls.log'
ok "über zwei Läufe genau ein open-vault" '[ $(nopen) = 1 ]'
ok "am eigenen neuen Vault 1 geprägt" 'grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
# (leerer Plan: das Skript endet vor Teil A und B, ohne Frage und ohne --ja-Lauf)
ok "2. Lauf: nichts mehr zu tun" '[ $(rc) = 0 ] && grep -q "Nichts mehr zu senden" $D/out.txt && ! grep -q "Teil B: Version 4 anlegen" $D/out.txt'

echo "N2) KEYS in anderem Ordner, in keys/ liegt eine gleichnamige fremde Datei"
setup n2-keysdir "$N1"; v3state; mkdir $D/schluessel
mv $D/keys/mainnet-owner.json $D/keys/mainnet-committee.json $D/schluessel/
echo "{\"mock_xonly\":\"$DD\"}" > $D/keys/mainnet-owner.json
(cd $D && KEYS=schluessel/mainnet ./GHOST-Umzug-v4.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "Schlüsselliste aus schluessel/" 'grep -q "^v4 .*--json keys --dir schluessel$" $D/calls.log'
ok "v3: eigener Vault getilgt und geschlossen" 'grep -q "^v3 .*close --key schluessel/mainnet-owner.json --vault 1$" $D/calls.log'
ok "genau ein open-vault, am eigenen Vault geprägt" '[ $(nopen) = 1 ] && grep -q "^v4 .*mint --key schluessel/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'

echo "N3) Besitzer-Datei nicht als Schlüssel gelistet, ähnlich benannte fremde schon → Abbruch vor jedem Senden"
setup n3-nomatch '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@E":{"ghost":1,"kas":200}},"vaults":[]}}'
v4state; echo '{"kaputt":1}' > $D/keys/mainnet-owner.json; echo "{\"mock_xonly\":\"$E\"}" > $D/keys/x-mainnet-owner.json
run
ok "Abbruch mit Hinweis auf den Schlüssel" '[ $(rc) != 0 ] && grep -q "nicht (eindeutig) in der Schlüsselliste" $D/out.txt'
ok "kein Sendebefehl" '[ -z "$(sends)" ]'

echo "N4) Besitzer-Datei doppelt gelistet, einmal mit anderem Schlüssel → Abbruch mit Grund"
setup n4-dup '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[],"dup_keys":"diff"}}'
v4state; run
ok "Abbruch, kein Sendebefehl" '[ $(rc) != 0 ] && grep -q "nicht (eindeutig)" $D/out.txt && [ -z "$(sends)" ]'
ok "Grund: mehrfach mit verschiedenen Schlüsseln" 'grep -q "Grund: keys/mainnet-owner.json steht mehrfach mit verschiedenen Schlüsseln" $D/out.txt'
echo "N4b) Besitzer-Datei doppelt gelistet, derselbe Schlüssel → zählt einmal (Restpunkt C-R1)"
setup n4b-dupsame '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[],"dup_keys":true}}'
v4state; run
ok "Exitcode 0, genau ein open-vault, am eigenen Vault geprägt" '[ $(rc) = 0 ] && [ $(nopen) = 1 ] && grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 0 --ghost 0.5$" $D/calls.log'

echo "N5) eigener v3-Vault ist gesperrt (stale) → kein zweiter Vault"
setup n5-stale '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":1,"kas":200}},
 "vaults":[{"owner":"@B","covenantId":"y1","debtGhost":3,"collateralKas":300},{"owner":"@A","covenantId":"y2","debtGhost":0,"collateralKas":50,"stale":true}]}}'
v4state; run
ok "kein open-vault, kein mint am gesperrten" '[ $(nopen) = 0 ] && ! grep -q " mint " $D/calls.log'
ok "Meldung: eigener Vault vorhanden" 'grep -q "schon einen eigenen Vault (mit Schuld oder gesperrt)" $D/out.txt'

echo "N6) neuer Vault erscheint nicht als eigener → Merker, zweiter Doppelklick eröffnet keinen weiteren"
setup n6-mark '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":1,"kas":200}},"vaults":[],"open_owner":"@E"}}'
v4state; run; cp $D/out.txt $D/out1.txt; cp $D/rc $D/rc1
ok "1. Lauf: ein open-vault, dann Abbruch mit Merker" '[ $(cat $D/rc1) != 0 ] && [ $(nopen) = 1 ] && [ -f $D/deployments/.umzug-v4-vault.lock ] && grep -q "nicht als eigener gefunden" $D/out1.txt'
run
ok "2. Lauf: kein weiteres open-vault, Hinweis auf den früheren Lauf" '[ $(rc) != 0 ] && [ $(nopen) = 1 ] && grep -q "früherer Lauf hat schon einen Vault eröffnet" $D/out.txt'
ok "Sperre wieder frei" 'lockfree'

echo "N7) Merker liegt, eigener Vault ist inzwischen da → Merker weg, weiter mit Prägen"
setup n7-markok '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},
 "vaults":[{"owner":"@A","covenantId":"y2","debtGhost":0,"collateralKas":50}]}}'
v4state; echo "Besitzer x" > $D/deployments/.umzug-v4-vault.lock; run
ok "Exitcode 0, Merker entfernt, geprägt, kein open-vault" '[ $(rc) = 0 ] && [ ! -e $D/deployments/.umzug-v4-vault.lock ] && grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 0 " $D/calls.log && [ $(nopen) = 0 ]'

echo "N8) Status ohne Vault-Liste → nichts eröffnen"
setup n8-status '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[],"status_broken":true}}'
v4state; run
ok "Abbruch, kein open-vault" '[ $(rc) != 0 ] && grep -q "Status nicht abrufbar" $D/out.txt && [ $(nopen) = 0 ]'

echo "\n== J: A12-17 Journal nach unterbrochenem Umbenennen (Attrappe übernimmt Journale wie resolve_pending) =="
JB=$(print -r -- "$BASE" | sed 's/^{/{"journal":true,/')
echo "J1) mainnet.json und Journal schon umbenannt, Ziel noch mainnet.json, Sperre nicht"
setup j1-renamed "$JB"; v3state mainnet-v3.json; printf "$JV3" deployments/mainnet.json > $D/deployments/mainnet-v3.pending.json; : > $D/deployments/mainnet.lock
run
ok "Journal umgerichtet (Meldung)" 'grep -q "zeigt jetzt auf deployments/mainnet-v3.json" $D/out.txt'
ok "v3-Stand aus dem Journal landet in mainnet-v3.json" 'hasjournal $D/deployments/mainnet-v3.json'
ok "mainnet.json bekommt nie den v3-Stand, ist am Ende Version 4" '! hasjournal $D/deployments/mainnet.json && [ -f $D/deployments/mainnet.json ] && ! isv3 $D/deployments/mainnet.json'
ok "Exitcode 0" '[ $(rc) = 0 ]'

echo "J2) Journal schon umbenannt, mainnet.json (v3) noch nicht"
setup j2-half "$JB"; v3state; printf "$JV3" deployments/mainnet.json > $D/deployments/mainnet-v3.pending.json
echo '{"network":"mainnet"}' > $D/deployments/mainnet.deploy.json; : > $D/deployments/mainnet.lock
run
ok "umbenannt und Journal umgerichtet" '[ -f $D/deployments/mainnet-v3.json ] && [ -f $D/deployments/mainnet-v3.deploy.json ] && grep -q "zeigt jetzt auf deployments/mainnet-v3.json" $D/out.txt'
ok "v3-Stand in mainnet-v3.json, mainnet.json am Ende Version 4" 'hasjournal $D/deployments/mainnet-v3.json && ! hasjournal $D/deployments/mainnet.json && ! isv3 $D/deployments/mainnet.json && [ $(rc) = 0 ]'

echo "J3) wie J1 mit absolutem Ziel"
setup j3-abs "$JB"; v3state mainnet-v3.json; printf "$JV3" "$D/deployments/mainnet.json" > $D/deployments/mainnet-v3.pending.json
run
ok "absolut auf mainnet-v3.json umgerichtet, v3-Stand dort" 'grep -q "zeigt jetzt auf ${D:A}/deployments/mainnet-v3.json" $D/out.txt && hasjournal $D/deployments/mainnet-v3.json && ! hasjournal $D/deployments/mainnet.json && [ $(rc) = 0 ]'

echo "J4) Probelauf mit Journal aus J1 → bricht ab, verändert nichts"
setup j4-dry "$JB"; v3state mainnet-v3.json; printf "$JV3" deployments/mainnet.json > $D/deployments/mainnet-v3.pending.json
(cd $D && DRY=1 ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "Abbruch mit Hinweis" '[ $(rc) != 0 ] && grep -q "der Probelauf bricht hier ab" $D/out.txt'
ok "kein Aufruf von Version 3, Journal unverändert, keine mainnet.json" '! grep -q "^v3 " $D/calls.log && [ "$(jtarget $D/deployments/mainnet-v3.pending.json)" = deployments/mainnet.json ] && [ ! -f $D/deployments/mainnet.json ]'

echo "J5) Journal von Version 3 zeigt auf eine fremde Datei → Abbruch vor jedem Aufruf"
setup j5-foreign "$JB"; v3state mainnet-v3.json; printf "$JV3" deployments/anders.json > $D/deployments/mainnet-v3.pending.json
run
ok "Abbruch, kein v3-Aufruf, anders.json nicht geschrieben" '[ $(rc) != 0 ] && grep -q "bitte von Hand klären" $D/out.txt && ! grep -q "^v3 " $D/calls.log && [ ! -f $D/deployments/anders.json ]'

echo "J6) Journal zeigt schon richtig → normaler Lauf"
setup j6-right "$JB"; v3state mainnet-v3.json; printf "$JV3" deployments/mainnet-v3.json > $D/deployments/mainnet-v3.pending.json
run
ok "Exitcode 0, keine Umrichtung, v3-Stand in mainnet-v3.json" '[ $(rc) = 0 ] && ! grep -q "zeigt jetzt auf" $D/out.txt && hasjournal $D/deployments/mainnet-v3.json'

echo "\n== D: A12-17 Probelauf zieht die verplanten GHOST ab =="
setup d1-dry '{"v3":{"kasUsd":0.05,"keys":{"@A":{"ghost":1.0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c3","debtGhost":2.0,"collateralKas":100},{"owner":"@B","covenantId":"x1","debtGhost":0.05,"collateralKas":7},
 {"owner":"@A","covenantId":"c1","debtGhost":0.1,"collateralKas":5},{"owner":"@A","covenantId":"c2","debtGhost":0.3,"collateralKas":10}]},
 "v4":{"kasUsd":0.05,"keys":{},"vaults":[]}}'
v3state
(cd $D && DRY=1 ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "0,1 und 0,3 ganz, dann 2,0 nur mit den übrigen 0,6 GHOST" 'grep -q "^v3 .*--dry-run repay --key keys/mainnet-owner.json --vault 2 --ghost 0.10000000$" $D/calls.log && grep -q "^v3 .*--dry-run repay --key keys/mainnet-owner.json --vault 3 --ghost 0.30000000$" $D/calls.log && grep -q "^v3 .*--dry-run repay --key keys/mainnet-owner.json --vault 0 --ghost 0.60000000$" $D/calls.log'
ok "Anzeige: eigene GHOST nach den vorigen Tilgungen" 'grep -q "eigene GHOST 0.60000000 (nach den vorigen Tilgungen)" $D/out.txt'
ok "Restschuld 1,4 GHOST, Sicherheit auf 61,60 KAS" 'grep -q "Restschuld 1.40000000 GHOST bleibt – Sicherheit von 100.00000000 auf 61.60 KAS senken" $D/out.txt'
setup d2-real '{"v3":{"kasUsd":0.05,"keys":{"@A":{"ghost":1.0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c3","debtGhost":2.0,"collateralKas":100},{"owner":"@A","covenantId":"c1","debtGhost":0.1,"collateralKas":5},{"owner":"@A","covenantId":"c2","debtGhost":0.3,"collateralKas":10}]},
 "v4":{"kasUsd":0.05,"keys":{"@A":{"ghost":0,"kas":200}},"vaults":[]}}'
v3state; run
ok "echter Lauf tilgt gleich wie die Vorschau (0,6 GHOST am letzten Vault)" 'grep -q "^v3 .*repay --key keys/mainnet-owner.json --vault 0 --ghost 0.60000000$" $D/calls.log && grep -q "^v3 .*withdraw --key keys/mainnet-owner.json --vault 0 --keep 61.60$" $D/calls.log && [ $(rc) = 0 ]'

echo "\n== C: Restpunkte der Nachprüfung zu Audit 12 (C-T2, C-R1) =="
echo "C1) Version 4 nennt für die Besitzer-Datei ein anderes x-only als Version 3 → Gegenprobe ME = ME3, nichts eröffnet"
setup c1-me3 '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@E":{"ghost":0,"kas":200}},"vaults":[],"xonly_as":"@E"}}'
v3state; run
ok "Teil A lief mit dem v3-Schlüssel (getilgt, geschlossen)" 'grep -q "^v3 .*close --key keys/mainnet-owner.json --vault 0$" $D/calls.log'
ok "Abbruch: Version 3 und 4 nennen verschiedene Schlüssel" '[ $(rc) != 0 ] && grep -q "Version 3 und 4 nennen für keys/mainnet-owner.json verschiedene Schlüssel ($A / $E)" $D/out.txt'
ok "kein open-vault, kein mint" '[ $(nopen) = 0 ] && ! grep -q " mint " $D/calls.log'

echo "C2) x-only der Besitzer-Datei hat keine 64 Hex-Zeichen → Abbruch vor jedem Senden"
setup c2-xonly '{"v3":{"kasUsd":0.04,"keys":{"abc":{"ghost":2,"lpShares":"0","kas":100}},"vaults":[]},
 "v4":{"kasUsd":0.04,"keys":{"abc":{"ghost":0,"kas":200}},"vaults":[]}}'
v3state; echo '{"mock_xonly":"abc"}' > $D/keys/mainnet-owner.json
run
ok "Abbruch mit Grund" '[ $(rc) != 0 ] && grep -q "Grund: der Schlüssel zu keys/mainnet-owner.json ist kein x-only aus 64 Hex-Zeichen (abc)" $D/out.txt'
ok "kein Sendebefehl (kein deploy, kein open-vault)" '[ -z "$(sends)" ]'
setup c2b-xonly3 '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"A":{"ghost":0,"kas":200}},"vaults":[],"xonly_as":"A"}}'
v4state; run
ok "Version 4 allein: Abbruch vor open-vault" '[ $(rc) != 0 ] && grep -q "kein x-only aus 64 Hex-Zeichen (A)" $D/out.txt && [ $(nopen) = 0 ]'

echo "C3) Symlink auf die Besitzer-Datei im selben Ordner → dieselbe Datei, ein Treffer, normaler Lauf"
setup c3-symlink "$N1"; v3state; ln -s mainnet-owner.json $D/keys/zz-kopie.json
run
ok "Exitcode 0, v3 getilgt und geschlossen" '[ $(rc) = 0 ] && grep -q "^v3 .*close --key keys/mainnet-owner.json --vault 1$" $D/calls.log'
ok "genau ein open-vault, am eigenen Vault geprägt" '[ $(nopen) = 1 ] && grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
ok "die Attrappe listete wirklich beide Namen" 'grep -q "zz-kopie.json" <(cd $D && ./ghostctl --network mainnet --state deployments/mainnet.json --json keys --dir keys)'
echo "C4) Hardlink auf die Besitzer-Datei → ebenso"
setup c4-hardlink "$N1"; v3state; ln $D/keys/mainnet-owner.json $D/keys/zz-hart.json
run
ok "Exitcode 0, genau ein open-vault, am eigenen Vault geprägt" '[ $(rc) = 0 ] && [ $(nopen) = 1 ] && grep -q "^v4 .*mint --key keys/mainnet-owner.json --vault 1 --ghost 0.5$" $D/calls.log'
echo "C5) Kopie (eigene Datei, gleicher Inhalt) ist nicht die Besitzer-Datei → ein Treffer, normaler Lauf"
setup c5-copy "$N1"; v3state; cp $D/keys/mainnet-owner.json $D/keys/zz-kopie.json
run
ok "Exitcode 0, genau ein open-vault" '[ $(rc) = 0 ] && [ $(nopen) = 1 ]'

echo "\n== E: Einmal bestätigen (Plan, eine Frage, Abbruch und Fortsetzen, EINZELN=1) =="
# echte Sendebefehle (ohne die Proben mit --dry-run)
esends() { sends | grep -v -- "--dry-run"; }
nfrage() { grep -o "Alles so ausführen? \[j/N\]" $D/out.txt | wc -l | tr -d ' '; }
zeile() { grep -n -m1 -- "$1" $D/out.txt | cut -d: -f1; }
echo "E1) Zusammenfassung vor dem ersten Senden, genau eine Frage, alles mit --ja"
setup e1-plan "$BASE"; v3state; run
ok "Exitcode 0" '[ $(rc) = 0 ]'
ok "genau eine Frage, keine weitere" '[ $(nfrage) = 1 ] && ! grep -q "Teil B so ausführen\|Pool mit .* anlegen?" $D/out.txt'
ok "Plan mit allen Schritten" 'grep -q "=== Plan: 6 Schritte ===" $D/out.txt && grep -q "1. Vault 1: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "2. Vault 1 schließen" $D/out.txt && grep -q "3. Register, Orakel, Factory und GHOST (Version 4) anlegen" $D/out.txt && grep -q "4. Eigenen Vault mit 50 KAS eröffnen" $D/out.txt && grep -q "5. 0.5 GHOST prägen" $D/out.txt && grep -q "6. Tauschpool mit Kursband anlegen: 6.25000000 KAS und 0.25 GHOST" $D/out.txt'
ok "zurück, gebunden, Gebühren (Probe und Schätzung)" 'grep -q "zurück: 60.00000000 KAS Sicherheit" $D/out.txt && grep -q "gebunden: 4 KAS dauerhaft (Register, Orakel, Factory und GHOST-Wurzel je 1 KAS)" $D/out.txt && grep -q "3 KAS Minter-Zweig dauerhaft" $D/out.txt && grep -q "gebunden: Mindestliquidität" $D/out.txt && grep -q "Gebühr: 0.0412 KAS (mit --dry-run gebaut)" $D/out.txt && grep -q "Gebühr: etwa 0.05 KAS (geschätzt), 5 Transaktionen" $D/out.txt && grep -q "ohne --probe: Fristen 14 Tage / 2 h / 30 Tage, 1 Unterzeichner, Zins an deine Adresse" $D/out.txt'
ok "Endstand: KAS jetzt, vor Teil B, am Ende; GHOST" 'grep -q "Gebühren zusammen: etwa 0.4212 KAS für 12 Transaktionen" $D/out.txt && grep -q "KAS frei auf dem Schlüssel: jetzt 200.00, vor Teil B etwa 259.91, am Ende etwa 192.33" $D/out.txt && grep -q "GHOST der Version 4: am Ende 0.25000000" $D/out.txt'
ok "Plan und Frage vor dem ersten Schritt" '[ $(zeile "=== Plan") -lt $(zeile "Alles so ausführen") ] && [ $(zeile "Alles so ausführen") -lt $(zeile "Schritt 2/6") ]'
ok "jeder Sendebefehl mit --ja, keine Rückfrage von ghostctl" '[ $(esends | wc -l) = 6 ] && ! esends | grep -vq -- " --ja " && ! grep -q "# Rückfrage" $D/calls.log'
ok "Teil A im Plan nur geprobt (--dry-run auf mainnet.json vor dem Umbenennen)" 'grep -q "^v3 --network mainnet --state deployments/mainnet.json --dry-run --json repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" $D/calls.log'

echo "E2) Antwort N (oder keine Antwort) → Abbruch ohne Senden, nichts umbenannt"
setup e2-nein "$BASE"; v3state; runa n
ok "Abbruch mit Meldung" '[ $(rc) != 0 ] && grep -q "Abgebrochen – nichts gesendet, nichts umbenannt" $D/out.txt'
ok "kein Sendebefehl, nur Proben" '[ -z "$(esends)" ] && grep -q -- "--dry-run --json repay" $D/calls.log'
ok "nichts umbenannt, Sperre frei" '[ -f $D/deployments/mainnet.json ] && isv3 $D/deployments/mainnet.json && [ ! -e $D/deployments/mainnet-v3.json ] && [ ! -e $D/deployments/mainnet-v3.lock ] && lockfree'
setup e2-leer "$BASE"; v3state; (cd $D && ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "ohne Eingabe: Abbruch ohne Senden" '[ $(rc) != 0 ] && [ -z "$(esends)" ] && [ ! -e $D/deployments/mainnet-v3.json ]'

echo "E3) Abbruch mitten in Teil A → Meldung gesendet/offen; erneuter Doppelklick zeigt nur den Rest"
setup e3-mitte '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}],
 "fail":{"close":"Zeitlimit (Attrappe)"}},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; run; cp $D/out.txt $D/out1.txt
ok "1. Lauf: nach dem Tilgen von Vault 1 gestoppt" '[ $(rc) != 0 ] && grep -q "Schließen fehlgeschlagen" $D/out.txt && [ $(esends | wc -l) = 2 ] && ! grep -q "^v4 .* deploy " $D/calls.log'
ok "1. Lauf nennt Gesendetes" 'grep -A1 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "✓ Vault 1 getilgt (0.50000000 GHOST)"'
ok "1. Lauf nennt Offenes ab dem abgebrochenen Schritt" 'grep -A3 "Noch offen (laut Plan):" $D/out.txt | tr "\n" "|" | grep -q "– Vault 1 schließen|  – Vault 2: 1.00000000 von 1.00000000 GHOST Schuld tilgen (mit GHOST der Version 3)|  – Vault 2 schließen"'
python3 - "$D/mock.json" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v3"].pop("fail"); json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "2. Lauf: Plan zeigt nur die restlichen Schritte" 'grep -q "=== Plan: 7 Schritte ===" $D/out.txt && grep -q "1. Vault 1 schließen" $D/out.txt && ! grep -q "Vault 1: .* tilgen" $D/out.txt && grep -q "2. Vault 2: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt'
ok "2. Lauf: wieder genau eine Frage, dann fertig" '[ $(nfrage) = 1 ] && [ $(rc) = 0 ] && grep -q "Fertig: Version 4 läuft" $D/out.txt'
# (nach dem Schließen von Vault 1 rückt Vault 2 auf Nummer 1)
ok "2. Lauf: nur Vault 2 getilgt, beide geschlossen" '[ $(esends | grep -c " repay ") = 1 ] && esends | grep -q "repay --key keys/mainnet-owner.json --vault 1 --ghost 1.00000000$" && [ $(esends | grep -c " close ") = 2 ]'

echo "E4) Nach Teil A weniger KAS als geplant → vor Teil B anhalten und erneut fragen"
setup e4-weniger "$(print -r -- "$BASE" | sed 's/"v3":{/"v3":{"kas_back":0.5,/')"; v3state
runa $'j\nn'
ok "zweite Frage vor Teil B nennt die Abweichung" 'grep -q "Vor Teil B weicht der Stand vom bestätigten Plan ab" $D/out.txt && grep -q "KAS auf dem Schlüssel: geplant etwa 259.91, jetzt 230.00" $D/out.txt && grep -q "Teil B so ausführen? \[j/N\]" $D/out.txt'
ok "zeigt Teil B aus dem jetzigen Stand" 'grep -q "Teil B aus dem jetzigen Stand:" $D/out.txt && grep -q "KAS frei auf dem Schlüssel: jetzt 230.00, am Ende etwa 162.42" $D/out.txt'
ok "N: Teil A gesendet, von Teil B nichts" '[ $(rc) != 0 ] && grep -q "Teil B nicht begonnen" $D/out.txt && esends | grep -q " close " && ! esends | grep -q "^v4 "'
ok "Stand nennt Offenes: Teil B" 'grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Orakel, Factory und GHOST"'
: > $D/calls.log; run
ok "erneuter Doppelklick: nur Teil B, eine Frage, keine Abweichung mehr" '[ $(rc) = 0 ] && grep -q "=== Plan: 4 Schritte ===" $D/out.txt && [ $(nfrage) = 1 ] && ! grep -q "weicht" $D/out.txt && esends | grep -q " pool-open "'
setup e4-ja "$(print -r -- "$BASE" | sed 's/"v3":{/"v3":{"kas_back":0.5,/')"; v3state
runa $'j\nj'
ok "j auf die zweite Frage: Teil B läuft durch" '[ $(rc) = 0 ] && [ $(grep -c "so ausführen? \[j/N\]" $D/out.txt) = 2 ] && esends | grep -q " pool-open "'

echo "E5) Orakelkurs verschiebt den Pool-Betrag um mehr als 1 % → vor dem Pool erneut fragen"
setup e5-pool "$(print -r -- "$BASE" | sed 's/"v4":{/"v4":{"price":0.05,/')"; v3state
runa $'j\nn'
ok "Plan rechnete mit dem Börsenkurs (5 KAS)" 'grep -q "Tauschpool mit Kursband anlegen: 5.00000000 KAS und 0.25 GHOST" $D/out.txt'
ok "vor dem Pool: Abweichung genannt und gefragt" 'grep -q "Der Pool-Betrag weicht vom bestätigten Plan ab: jetzt 6.25000000 KAS statt 5.00000000" $D/out.txt && grep -q "Pool mit 6.25000000 KAS und 0.25 GHOST anlegen? \[j/N\]" $D/out.txt'
ok "N: kein pool-open, Deployment/Vault/Prägen gesendet" '[ $(rc) != 0 ] && ! grep -q "pool-open" $D/calls.log && esends | grep -q " mint "'
setup e5-gleich "$(print -r -- "$BASE" | sed 's/"v4":{/"v4":{"price":0.0402,/')"; v3state; run
ok "Abweichung unter 1 %: keine zweite Frage" '[ $(rc) = 0 ] && [ $(nfrage) = 1 ] && ! grep -q "Pool-Betrag weicht" $D/out.txt && esends | grep -q " pool-open "'

echo "E6) EINZELN=1: wie früher, ghostctl fragt je Transaktion; keine Sammelfrage"
setup e6-einzeln "$BASE"; v3state
runa "$(for i in {1..10}; do echo j; done)" EINZELN=1
ok "Exitcode 0, Plan gezeigt, keine Sammelfrage" '[ $(rc) = 0 ] && grep -q "=== Plan" $D/out.txt && [ $(nfrage) = 0 ] && grep -q "EINZELN=1: Jede Transaktion fragt gleich einzeln" $D/out.txt'
ok "je Transaktion eine Rückfrage, kein --ja" '[ $(esends | wc -l) = 6 ] && [ $(grep -c "# Rückfrage" $D/calls.log) = 6 ] && ! esends | grep -q -- " --ja "'
setup e6-nein "$BASE"; v3state
runa n EINZELN=1
ok "N auf die erste Rückfrage: Abbruch, nichts weiter gesendet" '[ $(rc) != 0 ] && [ $(grep -c "# Rückfrage" $D/calls.log) = 1 ] && grep -q "Tilgen fehlgeschlagen" $D/out.txt && ! grep -q " close " $D/calls.log'
setup e6-x "$BASE"; v3state; runa j EINZELN=ja
ok "EINZELN=ja wird abgelehnt" '[ $(rc) != 0 ] && grep -q "EINZELN=ja verstehe ich nicht" $D/out.txt && [ ! -s $D/calls.log ]'

echo "E7) DRY=1 bleibt reiner Probelauf: kein Plan, keine Frage"
setup e7-dry "$BASE"; v3state
(cd $D && DRY=1 ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "keine Frage, kein --ja, alles --dry-run" '[ $(rc) = 0 ] && ! grep -q "so ausführen\|=== Plan" $D/out.txt && ! grep -q -- " --ja " $D/calls.log && [ -z "$(esends)" ]'

echo "E8) Pool-Anteile: Plan zählt die GHOST aus dem Pool zum Tilgen, wie der echte Lauf"
setup e8-poolret '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; run
ok "Plan: Rückfluss 10 KAS und 0,25 GHOST, dann ganz tilgen und schließen" 'grep -q "zurück: etwa 10.00000000 KAS und 0.25000000 GHOST" $D/out.txt && grep -q "Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0 schließen" $D/out.txt'
ok "Tilgen mit Pool-GHOST ist geschätzt, nicht geprobt" '! grep -q -- "--dry-run --json repay" $D/calls.log && grep -q -- "--dry-run --json pool-remove" $D/calls.log'
ok "echter Lauf wie geplant, keine zweite Frage" '[ $(rc) = 0 ] && [ $(nfrage) = 1 ] && ! grep -q "weicht" $D/out.txt && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 1.00000000$" && esends | grep -q "close --key keys/mainnet-owner.json --vault 0$"'

# Seit Audit 13 (A13-umzug-2) geht pool-remove mit --min-kas/--min-ghost (Plan
# minus 1 %): Der Pool bewegt sich hier nur innerhalb dieser Toleranz (0,248
# statt 0,25 GHOST zurück), das reicht nicht mehr zum vollen Tilgen.
echo "E9) Pool bewegt sich zwischen Plan und Senden: weniger GHOST zurück → Teil A hält an und fragt erneut"
E9='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.75,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"49600000"}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup e9-weniger "$E9"; v3state
runa $'j\nn'
ok "Plan wie E8: ganz tilgen und schließen" 'grep -q "Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0 schließen" $D/out.txt'
ok "vor dem Tilgen: Abweichung genannt (geplant/jetzt)" 'grep -q "Teil A weicht vom bestätigten Plan ab" $D/out.txt && grep -q "geplant: Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "jetzt:   Vault 0: 0.99800000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt'
ok "zeigt Teil A aus dem jetzigen Stand und fragt" 'grep -q "Teil A aus dem jetzigen Stand:" $D/out.txt && grep -q "Sicherheit von 60.00000000 auf 0.30 KAS senken" $D/out.txt && grep -q "Teil A so ausführen? \[j/N\]" $D/out.txt'
ok "N: nur pool-remove gesendet, kein repay/withdraw/close, nichts von Teil B" '[ $(rc) != 0 ] && [ "$(esends | grep -c .)" = 1 ] && esends | grep -q " pool-remove " && grep -q "Teil A angehalten" $D/out.txt'
ok "Noch offen nennt die neu gerechneten Schritte" 'grep -A2 "Noch offen (laut Plan):" $D/out.txt | tr "\n" "|" | grep -q "– Vault 0: 0.99800000 von 1.00000000 GHOST Schuld tilgen (mit GHOST der Version 3)|  – Vault 0: Sicherheit von 60.00000000 auf 0.30 KAS senken"'
setup e9-ja "$E9"; v3state
runa $'j\nj\nj'
ok "j: gesendet wird, was die zweite Frage zeigte; Teil B ohne dritte Frage" '[ $(rc) = 0 ] && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.99800000$" && esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 0 --keep 0.30$" && ! esends | grep -q " close " && [ $(grep -c "so ausführen? \[j/N\]" $D/out.txt) = 2 ] && ! grep -q "Teil B so ausführen" $D/out.txt && esends | grep -q " pool-open "'

setup e9-einzeln "$E9"; v3state
runa "$(for i in {1..10}; do echo j; done)" EINZELN=1
ok "EINZELN=1: Abweichung gezeigt, keine Sammelfrage, ghostctl fragt je Transaktion (0,998 GHOST)" '[ $(rc) = 0 ] && grep -q "Teil A weicht vom bestätigten Plan ab" $D/out.txt && ! grep -q "so ausführen? \[j/N\]" $D/out.txt && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.99800000$" && ! esends | grep -q -- " --ja "'

echo "E10) Mehr GHOST zurück als geplant → nicht still ganz tilgen und schließen, erst fragen"
setup e10-mehr '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.5,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"100000000"}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; runa $'j\nn'
ok "Plan: teilweise tilgen und herausnehmen" 'grep -q "Vault 0: 0.75000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0: Sicherheit von 60.00000000 auf" $D/out.txt'
ok "vor dem Tilgen gefragt, N: kein repay, kein close" '[ $(rc) != 0 ] && grep -q "jetzt:   Vault 0: 1.00000000 von 1.00000000 GHOST Schuld tilgen" $D/out.txt && ! esends | grep -q " repay \| close \| withdraw "'

echo "E11) Orakel von Version 3 fällt zwischen Plan und Senden (beim Pool-Abzug) → Tilgen wie geplant, Herausnehmen fragt erneut"
setup e11-kurs '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.25,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"kasUsd":0.02},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; runa $'j\nn'
ok "Plan: auf 27.50 KAS senken; jetzt 55.00 → Frage" 'grep -q "geplant: Vault 0: Sicherheit von 60.00000000 auf 27.50 KAS senken" $D/out.txt && grep -q "jetzt:   Vault 0: Sicherheit von 60.00000000 auf 55.00 KAS senken" $D/out.txt && grep -q "Teil A so ausführen" $D/out.txt'
ok "N: getilgt, nicht herausgenommen; offen: Herausnehmen mit neuem Betrag" '[ $(rc) != 0 ] && esends | grep -q " repay --key keys/mainnet-owner.json --vault 0 --ghost 0.50000000$" && ! esends | grep -q " withdraw " && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "auf 55.00 KAS senken"'

echo "E12) Geplanter Schritt entfällt (Vault weg) → vor Teil B fragen"
setup e12-weg '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "move":{"vaults":[]},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; runa $'j\nn'
ok "entfallende Schritte genannt, Frage, N: nichts von Teil B" '[ $(rc) != 0 ] && grep -q "nach dem jetzigen Stand entfällt:" $D/out.txt && grep -q "Ohne diese Schritte mit Teil B weitermachen? \[j/N\]" $D/out.txt && ! esends | grep -q "^v4 "'

echo "E13) Leerer Plan → Ende ohne Frage und ohne Teil A/B"
setup e13-leer '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"pool":{"kas":"5","ghost":"0.25"},
 "vaults":[{"owner":"@A","covenantId":"y2","debtGhost":0.5,"collateralKas":50}]}}'
v4state; v3state mainnet-v3.json; run
ok "Exitcode 0, keine Frage, kein Teil A/B, nichts gesendet" '[ $(rc) = 0 ] && grep -q "Nichts mehr zu senden" $D/out.txt && [ $(nfrage) = 0 ] && ! grep -q "Teil A: Version 3 abbauen\|Teil B: Version 4 anlegen" $D/out.txt && [ -z "$(esends)" ]'

echo "\n== U: Audit 13 (Umzug) =="
# Welt nach dem Beweistest X1 der Prüfer: fremder Vault x1 vor den eigenen c1 und c2
U1W='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@B","covenantId":"x1","debtGhost":5,"collateralKas":500},{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}]MOVE},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
U1REST='[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}]'
# v3welt <python-Ausdruck über vs (Covenant-ID → Vault) und g (eigene GHOST v3)>
v3welt() { python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); vs={v["covenantId"]:v for v in m["v3"]["vaults"]}; g=m["v3"]["keys"][sys.argv[2]]["ghost"]; sys.exit(0 if eval(sys.argv[3]) else 1)' $D/mock.json $A "$1"; }

echo "U1) A13-umzug-1: fremder Vault verschwindet MIT dem Tilgen (Nummer trifft c2) → nur 0,5 GHOST, Prüfung danach hält an, kein close"
setup u1-nachher "${U1W/MOVE/,\"move\":{\"vaults\":$U1REST\}}"; v3state; run
ok "bestätigt war: Vault 1 (c1) mit 0,5 GHOST tilgen" 'grep -q "1. Vault 1: 0.50000000 von 0.50000000 GHOST Schuld tilgen" $D/out.txt'
ok "repay immer mit --ghost (gedeckelt auf den bestätigten Betrag)" 'esends | grep -q "repay --key keys/mainnet-owner.json --vault 1 --ghost 0.50000000$" && ! esends | grep -q "repay --key keys/mainnet-owner.json --vault [0-9]*$"'
ok "Prüfung nach dem Senden hält an: kein close, nichts von Teil B" '[ $(rc) != 0 ] && grep -q "nicht genau der bestätigte Vault wie erwartet geändert" $D/out.txt && ! esends | grep -q " close \| withdraw " && ! esends | grep -q "^v4 "'
ok "Welt: c1 unberührt, c2 nur um 0,5 getilgt und offen, 1,5 GHOST übrig" 'v3welt "vs[\"c1\"][\"debtGhost\"] == 0.5 and vs[\"c2\"][\"debtGhost\"] == 0.5 and vs[\"c2\"][\"collateralKas\"] == 60 and g == 1.5"'
ok "keine Erfolgsmeldung ohne Hinweis: Gesendetes als abweichend markiert, Offenes genannt" 'grep -A1 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "Vault 1 getilgt (0.50000000 GHOST) – gesendet, aber die Wirkung weicht ab" && grep -q "Vault c1 sollte jetzt sein: Schuld 0.00000000 GHOST" $D/out.txt && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Vault 1 schließen"'

echo "U2) Nummer verschiebt sich zwischen Tilgen und Schließen → Halt ohne Senden von close"
setup u2-schliessen "${U1W/MOVE/,\"move_after\":{\"repay\":{\"vaults\":$U1REST\}\}}"; v3state
python3 - "$D/mock.json" <<'PY'
# c1 ist nach dem Tilgen schuldenfrei: die Bewegung übernimmt das
import json,sys; m=json.load(open(sys.argv[1])); m["v3"]["move_after"]["repay"]["vaults"][0]["debtGhost"]=0; json.dump(m,open(sys.argv[1],"w"))
PY
run
ok "c1 getilgt (Nummer 1), dann angehalten: Vault-Nummer verschoben" 'esends | grep -q "repay --key keys/mainnet-owner.json --vault 1 --ghost 0.50000000$" && [ $(rc) != 0 ] && grep -q "Vault-Nummer verschoben: Nummer 1 trägt nicht mehr den bestätigten Vault c1 (er hat jetzt Nummer 0)" $D/out.txt'
ok "kein close gesendet, nichts von Teil B" '! esends | grep -q " close " && ! esends | grep -q "^v4 "'
ok "Welt: c2 unberührt (1 GHOST Schuld, 60 KAS)" 'v3welt "vs[\"c2\"][\"debtGhost\"] == 1 and vs[\"c2\"][\"collateralKas\"] == 60 and vs[\"c1\"][\"debtGhost\"] == 0"'

echo "U3) Nummer verschiebt sich zwischen Status und Senden (vor dem ersten Tilgen) → Halt ohne jedes Senden"
# status#7 = der Status, aus dem Teil A die Nummer von c1 liest (4 im Plan, 3 im echten Lauf)
setup u3-vorher "${U1W/MOVE/,\"move_after\":{\"status#7\":{\"vaults\":$U1REST\}\}}"; v3state; run
ok "Halt vor dem Senden: Vault-Nummer verschoben" '[ $(rc) != 0 ] && grep -q "Vault-Nummer verschoben: Nummer 1 trägt nicht mehr den bestätigten Vault c1 (er hat jetzt Nummer 0)" $D/out.txt && grep -q "repay ist nicht gesendet" $D/out.txt'
ok "nichts gesendet, Welt unverändert" '[ -z "$(esends)" ] && v3welt "vs[\"c1\"][\"debtGhost\"] == 0.5 and vs[\"c2\"][\"debtGhost\"] == 1 and g == 2"'
ok "Stand: nichts gesendet, offen ab dem Tilgen" 'grep -q "In diesem Lauf wurde nichts gesendet." $D/out.txt && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Vault 1: 0.50000000 von 0.50000000 GHOST Schuld tilgen"'
: > $D/calls.log; run
ok "erneuter Doppelklick: Plan mit den neuen Nummern, dann durch" '[ $(rc) = 0 ] && grep -q "1. Vault 0: 0.50000000 von 0.50000000 GHOST Schuld tilgen" $D/out.txt && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.50000000$" && [ $(esends | grep -c " close ") = 2 ]'

echo "U4) A13-umzug-2/6: pool-remove mit Mindestbeträgen aus dem Plan minus 1 %"
U4W='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"}MOVE,
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup u4-min "${U4W/MOVE/}"; v3state; run
ok "Zusammenfassung nennt Mindestbeträge und Toleranz" 'grep -q "mindestens 9.90000000 KAS und 0.24750000 GHOST (Toleranz 1 %) – kommt weniger zurück, sendet ghostctl nicht" $D/out.txt'
ok "Zusammenfassung nennt die Pool-Toleranz beim Anlegen" 'grep -q "bis ± 1 % Abweichung geht ohne erneute Frage hinaus" $D/out.txt'
ok "Probe und Senden mit --min-kas/--min-ghost" 'grep -q -- "--dry-run --json pool-remove --key keys/mainnet-owner.json --percent 100 --min-kas 9.90000000 --min-ghost 0.24750000$" $D/calls.log && esends | grep -q -- "--ja pool-remove --key keys/mainnet-owner.json --percent 100 --min-kas 9.90000000 --min-ghost 0.24750000$"'
ok "Lauf endet mit 0" '[ $(rc) = 0 ]'
# wie X2 der Prüfer: der Pool fällt beim Senden auf ein Zehntel
setup u4-verschoben "${U4W/MOVE/,\"move\":{\"pool\":{\"shares\":\"1000\",\"kasSompi\":\"200000000\",\"ghostUnits\":\"5000000\"\}\}}"; v3state; run
ok "Pool verschoben: ghostctl sendet nicht (Mindestbeträge), Abbruch vor dem Tilgen" '[ $(rc) != 0 ] && grep -q "Der Pool hat sich verschoben" $D/out.txt && grep -q "Abziehen fehlgeschlagen" $D/out.txt && ! esends | grep -q " repay \| close \| withdraw "'
ok "Welt: Anteile und GHOST unverändert" 'python3 -c "import json,sys; m=json.load(open(sys.argv[1])); k=m[\"v3\"][\"keys\"][sys.argv[2]]; sys.exit(0 if k[\"lpShares\"]==\"500\" and k[\"ghost\"]==0.8 else 1)" $D/mock.json $A'

echo "U5) A13-umzug-4: SIGTERM während des Prägens → Stand (gesendet/offen) wird gezeigt"
setup u5-signal '{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[],"sleep_after":{"mint":4}}}'
v4state
(cd $D && ./GHOST-Umzug-v4.command <$JA >$D/out.txt 2>&1 & echo $! > $D/spid; wait $!; echo $? > $D/rc) &
for i in {1..100}; do grep -q "wirkt, wartet mint" $D/calls.log 2>/dev/null && break; python3 -c 'import time; time.sleep(0.2)'; done
kill -TERM $(cat $D/spid); wait
ok "Skript endet mit 130, Sperre frei" '[ $(rc) = 130 ] && lockfree'
ok "Abbruch nennt den laufenden Schritt" 'grep -q "Abgebrochen (Signal)" $D/out.txt && grep -q "Unterbrochen während: 0.5 GHOST geprägt (Vault 0)" $D/out.txt'
ok "Stand: gesendet (Vault eröffnet) und offen (Prägen, Pool)" 'grep -A1 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "✓ Vault mit 50 KAS eröffnet" && grep -A2 "Noch offen (laut Plan):" $D/out.txt | tr "\n" "|" | grep -q "– 0.5 GHOST prägen|  – Tauschpool"'

echo "U6) A13-umzug-5/3: Zusammenfassung nennt Netz, Schlüsseldatei, Adresse, x-only und mögliche Zusammenführungen"
setup u6-kopf "$BASE"; v3state; runa n
ok "Netz, Schlüsseldatei, Adresse, x-only vor der Frage" 'sed -n "/=== Plan/,/so ausführen/p" $D/out.txt > $D/plan.txt; grep -q "^Netz: *mainnet$" $D/plan.txt && grep -q "^Schlüsseldatei: keys/mainnet-owner.json (${D:A}/keys/mainnet-owner.json)$" $D/plan.txt && grep -q "^Adresse: *kaspa:attrappeaaaaaaaaaaaaaaaa$" $D/plan.txt && grep -q "^x-only: *$A$" $D/plan.txt'
ok "mögliche Zusammenführung bei Tilgen und Pool, in den Gebühren gezählt" '[ $(grep -c "dazu ggf. 1 Transaktion „GHOST zusammenführen“" $D/plan.txt) = 2 ] && grep -q "dazu ggf. bis zu 2 × „GHOST zusammenführen“" $D/plan.txt'
ok "N: nichts gesendet" '[ $(rc) != 0 ] && [ -z "$(esends)" ]'
setup u6-keys "$BASE"; v3state; mkdir -p $D/alt; echo "{\"mock_xonly\":\"$A\"}" > $D/alt/x-owner.json; echo '{"secrets":["x","x","x","x","x"]}' > $D/alt/x-committee.json
runa n KEYS=alt/x
ok "KEYS aus der Umgebung: Zusammenfassung zeigt alt/x-owner.json" 'sed -n "/=== Plan/,/so ausführen/p" $D/out.txt | grep -q "^Schlüsseldatei: alt/x-owner.json (${D:A}/alt/x-owner.json)$"'

echo "U7) A13-umzug-7: --keep in Sompi ohne Float-Rauschen (0,1 GHOST bei 0,022 USD → 10,00 KAS, nicht 10,01)"
setup u7-keep '{"v3":{"kasUsd":0.022,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.1,"collateralKas":60},{"owner":"@A","covenantId":"c2","debtGhost":0.3,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; run
ok "Plan und Senden: --keep 10.00 (statt 10.01)" 'grep -q "Vault 0: Sicherheit von 60.00000000 auf 10.00 KAS senken" $D/out.txt && esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 0 --keep 10.00$"'
ok "0,3 GHOST bei 0,022 USD → 30,00 KAS" 'esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 1 --keep 30.00$"'
ok "Prüfung nach dem Herausnehmen besteht, Lauf endet mit 0" '[ $(rc) = 0 ] && ! grep -q "nicht genau der bestätigte Vault" $D/out.txt'

echo "Z1) Verstreute GHOST: Probe des Tilgens scheitert nur im Probelauf (ghostctl führt erst zusammen) – Plan und DRY=1 laufen weiter"
Z='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}],
 "dry_fail":{"repay":"Fehler: Die GHOST liegen auf mehr als zwei UTXOs verteilt; vor dieser Aktion wird zuerst zusammengeführt. Das lässt sich nicht als Probelauf vorab zeigen"}},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup z1-plan "$Z"; v3state; runa n
ok "Plan erscheint trotz gescheiterter Probe, Tilgen geschätzt mit Hinweis aufs Zusammenführen" 'grep -q "=== Plan" $D/out.txt && grep -q "vorher 1–2 Transaktionen „GHOST zusammenführen“" $D/out.txt && ! grep -q "Probe (--dry-run) für" $D/out.txt'
ok "N: nichts gesendet" '[ $(rc) != 0 ] && [ -z "$(esends)" ]'
setup z1-echt "$Z"; v3state; run
ok "echter Lauf tilgt und schließt beide Vaults (der zweite rückt nach dem Schließen auf Nummer 0)" '[ $(rc) = 0 ] && [ "$(esends | grep -c "repay --key keys/mainnet-owner.json --vault 0 --ghost ")" = 2 ] && esends | grep -q "vault 0 --ghost 1.00000000$" && [ "$(esends | grep -c " close ")" = 2 ]'
setup z1-dry "$Z"; v3state; runa "" DRY=1
ok "DRY=1: Hinweis statt Abbruch, weiter bis Teil B, nichts gesendet" '[ $(rc) = 0 ] && grep -q "vorher werden deine GHOST zusammengeführt" $D/out.txt && ! grep -q "Tilgen fehlgeschlagen" $D/out.txt && [ -z "$(esends)" ]'
setup z1-andere '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20}],"dry_fail":{"repay":"Fehler: etwas ganz anderes"}},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; runa n
ok "andere Probe-Fehler brechen weiterhin ab, nichts gesendet" '[ $(rc) != 0 ] && grep -q "Probe (--dry-run) für „Vault 0 tilgen“ gescheitert" $D/out.txt && [ -z "$(esends)" ]'

echo "\n== V: Version 4 (Versionserkennung, Zinsdatei, Unterzeichner, Zins, Fortsetzen, Ctrl+C, schon vorhandenes v4) =="
LEER='{"v3":{"kasUsd":0.04,"keys":{},"vaults":[]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
echo "V1) Versionserkennung: mainnet.json weder Version 3 noch 4 → Abbruch, nichts umbenannt, kein Aufruf"
for f in v2 leer liste; do
  setup v1-$f "$BASE"
  case $f in
    v2)    echo '{"network":"mainnet","vault_params":{"mcr_bps":20000}}' > $D/deployments/mainnet.json ;;
    leer)  echo '{}' > $D/deployments/mainnet.json ;;
    liste) echo '[1,2]' > $D/deployments/mainnet.json ;;
  esac
  run
  ok "$f: Abbruch mit Grund" '[ $(rc) != 0 ] && grep -q "nicht lesbar oder weder Version 3 noch Version 4" $D/out.txt'
  ok "$f: nichts umbenannt, kein ghostctl-Aufruf, Sperre frei" '[ -f $D/deployments/mainnet.json ] && [ ! -e $D/deployments/mainnet-v3.json ] && [ ! -s $D/calls.log ] && lockfree'
done
setup v1-v3istv4 "$BASE"; v4state; mv $D/deployments/mainnet.json $D/deployments/mainnet-v3.json
run
ok "mainnet-v3.json ist Version 4 → Abbruch, kein Aufruf" '[ $(rc) != 0 ] && grep -q "mainnet-v3.json ist nicht lesbar oder nicht Version 3" $D/out.txt && [ ! -s $D/calls.log ]'
setup v1-v4ok "$LEER"; v4state; run
ok "mainnet.json mit „register“ gilt als Version 4: nicht umbenannt, kein Deployment" '[ ! -e $D/deployments/mainnet-v3.json ] && ! grep -q " deploy " $D/calls.log && grep -q "Version 4 ist schon angelegt" $D/out.txt'

echo "V2) Zinsdatei: Ziel gibt es schon → nichts umbenannt; liegt nach dem Umbenennen noch da → Abbruch vor der Frage"
setup v2-zinsziel "$BASE"; v3state; echo '{"zins":"v3"}' > $D/deployments/mainnet-zins.json; echo '{"alt":1}' > $D/deployments/mainnet-v3-zins.json
run
ok "Abbruch „Gibt es schon“, nichts umbenannt, nichts gesendet" '[ $(rc) != 0 ] && grep -q "Gibt es schon: deployments/mainnet-v3-zins.json" $D/out.txt && [ -f $D/deployments/mainnet.json ] && [ -f $D/deployments/mainnet-zins.json ] && [ -z "$(esends)" ]'
setup v2-zinsrest "$LEER"; v3state mainnet-v3.json; echo '{"zins":"v3"}' > $D/deployments/mainnet-zins.json
run
ok "Zinsdatei ohne mainnet.json: Abbruch vor der Frage, nichts gesendet, Datei bleibt" '[ $(rc) != 0 ] && grep -q "mainnet-zins.json liegt noch da" $D/out.txt && [ $(nfrage) = 0 ] && [ -z "$(esends)" ] && grep -q "v3" $D/deployments/mainnet-zins.json'

echo "V3) Unterzeichner-Datei fehlt → eigener Schritt im Plan, committee-keygen --count 1, dann Deployment damit"
setup v3-signer "$BASE"; v3state; rm $D/keys/mainnet-signer.json; run
ok "Exitcode 0, eine Frage" '[ $(rc) = 0 ] && [ $(nfrage) = 1 ]'
ok "Plan: Schritt „Unterzeichner-Datei anlegen“ ohne Gebühr, vor dem Deployment" 'grep -q "3. Unterzeichner-Datei keys/mainnet-signer.json anlegen (1 Schlüssel, ./ghostctl committee-keygen --count 1)" $D/out.txt && grep -q "4. Register, Orakel, Factory und GHOST (Version 4) anlegen" $D/out.txt && grep -q "Gebühr: keine (sendet nichts)" $D/out.txt && grep -q "=== Plan: 7 Schritte ===" $D/out.txt'
ok "Kopf nennt die fehlende Unterzeichner-Datei" 'grep -q "^Unterzeichner:  keys/mainnet-signer.json (fehlt" $D/out.txt'
ok "genau ein committee-keygen, genau so, vor dem deploy" '[ "$(grep -c " committee-keygen " $D/calls.log)" = 1 ] && grep -q "^v4 committee-keygen keys/mainnet-signer.json --count 1$" $D/calls.log && [ $(grep -n "committee-keygen" $D/calls.log | cut -d: -f1) -lt $(grep -n " deploy " $D/calls.log | cut -d: -f1) ]'
ok "Datei mit einem Schlüssel, deploy damit, ohne --probe" 'python3 -c "import json,sys; sys.exit(0 if len(json.load(open(\"$D/keys/mainnet-signer.json\"))[\"secrets\"])==1 else 1)" && grep -q " deploy --key keys/mainnet-owner.json --committee keys/mainnet-signer.json$" $D/calls.log && ! grep -q -- "--probe" $D/calls.log'
setup v3-signer-n "$BASE"; v3state; rm $D/keys/mainnet-signer.json; runa n
ok "N: keine Unterzeichner-Datei angelegt, nichts gesendet" '[ $(rc) != 0 ] && [ ! -e $D/keys/mainnet-signer.json ] && ! grep -q "committee-keygen" $D/calls.log && [ -z "$(esends)" ]'
setup v3-signer-dry "$BASE"; v3state; rm $D/keys/mainnet-signer.json
(cd $D && DRY=1 ./GHOST-Umzug-v4.command </dev/null >$D/out.txt 2>&1); echo $? > $D/rc
ok "DRY=1: nennt das Anlegen, legt nichts an" '[ $(rc) = 0 ] && grep -q "der echte Lauf legt sie an" $D/out.txt && [ ! -e $D/keys/mainnet-signer.json ] && ! grep -q "committee-keygen" $D/calls.log'

echo "V4) Unterzeichner-Datei fehlt, aber ein Deployment von Version 4 ist angefangen → Abbruch vor der Frage"
setup v4-fortschritt "$LEER"; v3state mainnet-v3.json; rm $D/keys/mainnet-signer.json; echo '{"network":"mainnet","register":null}' > $D/deployments/mainnet.deploy.json
run
ok "Abbruch mit Grund, keine Frage, keine neue Datei, nichts gesendet" '[ $(rc) != 0 ] && grep -q "angefangenes Deployment von Version 4" $D/out.txt && [ $(nfrage) = 0 ] && [ ! -e $D/keys/mainnet-signer.json ] && ! grep -q "committee-keygen" $D/calls.log && [ -z "$(esends)" ]'

echo "V5) Unterzeichner-Datei unbrauchbar → Abbruch vor der Frage"
setup v5-drei "$BASE"; v3state; echo '{"secrets":["x","x","x"]}' > $D/keys/mainnet-signer.json; run
ok "3 Schlüssel: Abbruch, nichts gesendet, nichts umbenannt" '[ $(rc) != 0 ] && grep -q "enthält 3 Schlüssel statt 1" $D/out.txt && [ $(nfrage) = 0 ] && [ -z "$(esends)" ] && [ ! -e $D/deployments/mainnet-v3.json ]'
setup v5-key "$BASE"; v3state; echo "{\"mock_xonly\":\"$C\"}" > $D/keys/mainnet-signer.json; run
ok "Schlüsseldatei statt Unterzeichner: Abbruch, nichts gesendet" '[ $(rc) != 0 ] && grep -q "keys/mainnet-signer.json ist keine Unterzeichner-Datei" $D/out.txt && [ -z "$(esends)" ]'

echo "V6) Schon vorhandenes v4 (mit Pool), Version 3 liegt beiseite mit eigenem Vault, Unterzeichner fehlt"
setup v6-v4da '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"pool":{"kas":"5","ghost":"0.25"},"vaults":[]}}'
v4state; v3state mainnet-v3.json; rm $D/keys/mainnet-signer.json; run
ok "Exitcode 0: Teil A auf mainnet-v3.json, nichts umbenannt" '[ $(rc) = 0 ] && grep -q "^v3 --network mainnet --state deployments/mainnet-v3.json --ja close --key keys/mainnet-owner.json --vault 0$" $D/calls.log && ! grep -q "umbenannt:" $D/out.txt'
ok "kein deploy, kein committee-keygen, Vault und Prägen, kein zweiter Pool" '! grep -q " deploy \|committee-keygen" $D/calls.log && grep -q "Version 4 ist schon angelegt – übersprungen" $D/out.txt && [ $(nopen) = 1 ] && esends | grep -q " mint " && ! esends | grep -q " pool-open " && grep -q "Tauschpool existiert schon" $D/out.txt'
ok "Schluss warnt: Unterzeichner-Datei fehlt" 'grep -q "keys/mainnet-signer.json fehlt – ohne sie signiert der Agent keine Preise" $D/out.txt'

echo "V7) Zins von Version 3: Schließen bringt Sicherheit minus Zins; Herausnehmen rechnet 220 % von Schuld + Zins"
setup v7-zins '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60,"interestUsd":0.2}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; run
ok "Plan: zurück 55 KAS nach 5 KAS Zins" 'grep -q "zurück: 55.00000000 KAS Sicherheit (nach 5.00000000 KAS Zins an die Zinskasse)" $D/out.txt'
ok "Lauf endet mit 0, ohne Abweichung" '[ $(rc) = 0 ] && ! grep -q "weicht" $D/out.txt'
setup v7-rest '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.3,"lpShares":"0","kas":1}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":600,"interestUsd":0.1}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; run
ok "Restschuld 0,7 + 0,1 USD Zins → --keep 44.00 (nur Schuld wären 38.50: zu wenig)" 'esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 0 --keep 44.00$" && [ $(rc) = 0 ] && grep -q "Restschuld 0.70000000 GHOST (dazu 0.10000000 USD Zins) bleibt" $D/out.txt'

echo "V8) Abbruch und Fortsetzung an jedem Schritt (Senden scheitert vor der Wirkung, dann erneuter Doppelklick)"
FW='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpShares":"500","kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":20},{"owner":"@A","covenantId":"c2","debtGhost":1,"collateralKas":60}]FAILA},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]FAILB}}'
# echte Befehle (ohne Proben) als Folge von Namen
folge() { grep -E "^v[34] " $D/calls.log | grep -v -- "--dry-run" | grep -oE " (pool-remove|repay|close|withdraw|committee-keygen|deploy|open-vault|mint|pool-open) " | tr -d ' ' | tr '\n' ' ' | sed 's/ $//'; }
REIHE=(pool-remove repay close repay withdraw committee-keygen deploy open-vault mint pool-open)
for i in {1..${#REIHE}}; do
  x=${REIHE[i]}
  # beim zweiten repay: das erste gelingt, das zweite scheitert (über move_after)
  case $x in
    pool-remove|close|withdraw) fa=",\"fail_echt\":{\"$x\":\"Zeitlimit (Attrappe)\"}"; fb="" ;;
    repay) if (( i == 2 )); then fa=",\"fail_echt\":{\"repay\":\"Zeitlimit (Attrappe)\"}"; else fa=",\"move_after\":{\"close\":{\"fail_echt\":{\"repay\":\"Zeitlimit (Attrappe)\"}}}"; fi; fb="" ;;
    *) fa=""; fb=",\"fail_echt\":{\"$x\":\"Zeitlimit (Attrappe)\"}" ;;
  esac
  w=${FW/FAILA/$fa}; w=${w/FAILB/$fb}
  setup v8-$i-$x "$w"; v3state; rm $D/keys/mainnet-signer.json; run
  cp $D/out.txt $D/out1.txt; cp $D/rc $D/rc1; f1=$(folge)
  ok "$i/$x: 1. Lauf bricht genau dort ab und nennt Gesendetes/Offenes" '[ $(cat $D/rc1) != 0 ] && [ "$f1" = "${REIHE[1,i]}" ] && grep -q "Noch offen (laut Plan):" $D/out1.txt && grep -q "Erneuter Doppelklick setzt fort" $D/out1.txt'
  python3 - "$D/mock.json" <<'PY'
import json,sys; m=json.load(open(sys.argv[1]))
for v in ("v3","v4"): m[v].pop("fail_echt", None); m[v].pop("move_after", None)
json.dump(m,open(sys.argv[1],"w"))
PY
  : > $D/calls.log; run; f2=$(folge)
  ok "$i/$x: 2. Lauf setzt genau dort fort, eine Frage, fertig" '[ $(rc) = 0 ] && [ "$f2" = "${REIHE[i,-1]}" ] && [ $(nfrage) = 1 ] && grep -q "Fertig: Version 4 läuft" $D/out.txt'
  ok "$i/$x: Endstand stimmt (c2 mit 0,45 GHOST und 24,75 KAS, ein eigener v4-Vault, Pool, Unterzeichner)" 'python3 -c "
import json,sys; m=json.load(open(sys.argv[1])); a=sys.argv[2]
v3=m[\"v3\"][\"vaults\"]; v4=[v for v in m[\"v4\"][\"vaults\"] if v[\"owner\"]==a]
sys.exit(0 if len(v3)==1 and v3[0][\"covenantId\"]==\"c2\" and v3[0][\"debtGhost\"]==0.45 and v3[0][\"collateralKas\"]==24.75 and len(v4)==1 and v4[0][\"debtGhost\"]==0.5 and m[\"v4\"].get(\"pool\") else 1)" $D/mock.json $A && isv4 $D/deployments/mainnet.json && [ -f $D/keys/mainnet-signer.json ]'
done

echo "V9) Ctrl+C (SIGINT an die ganze Prozessgruppe wie im Terminal)"
# ctrlc <muster in calls.log oder out.txt> <datei>: startet das Skript in eigener
# Prozessgruppe, wartet auf das Muster und schickt SIGINT an die Gruppe
ctrlc() { python3 - "$D" "$1" "$2" <<'PY'
import os, signal, subprocess, sys, time
d, pat, f = sys.argv[1], sys.argv[2], sys.argv[3]
out = open(os.path.join(d, "out.txt"), "w")
p = subprocess.Popen(["./GHOST-Umzug-v4.command"], cwd=d, stdin=subprocess.PIPE, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
if pat != "Alles so ausführen":
    p.stdin.write(b"j\n" * 10); p.stdin.flush()
for _ in range(300):
    try:
        if pat in open(os.path.join(d, f)).read():
            break
    except OSError:
        pass
    time.sleep(0.1)
time.sleep(0.3)
os.killpg(p.pid, signal.SIGINT)
rc = p.wait(timeout=60)
open(os.path.join(d, "rc"), "w").write(str(rc if rc >= 0 else 128 - rc) + "\n")
PY
}
setup v9-frage "$BASE"; v3state; ctrlc "Alles so ausführen" out.txt
ok "an der Frage: Exit 130, nichts gesendet, nichts umbenannt, Sperre frei" '[ $(rc) = 130 ] && grep -q "Abgebrochen (Signal)" $D/out.txt && [ -z "$(esends)" ] && [ -f $D/deployments/mainnet.json ] && [ ! -e $D/deployments/mainnet-v3.json ] && lockfree'
setup v9-tilgen "$(print -r -- "$BASE" | sed 's/"v3":{/"v3":{"sleep_after":{"repay":4},/')"; v3state; ctrlc "wirkt, wartet repay" calls.log
ok "beim Tilgen (Teil A): Exit 130, Schritt und Stand genannt, Sperre frei" '[ $(rc) = 130 ] && grep -q "Unterbrochen während: Vault 1 getilgt (1.00000000 GHOST)" $D/out.txt && grep -q "In diesem Lauf wurde nichts gesendet." $D/out.txt && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Vault 1: 1.00000000 von 1.00000000 GHOST Schuld tilgen" && lockfree'
ok "nichts nach dem Tilgen gesendet" '! esends | grep -q " close \|^v4 "'
python3 - "$D/mock.json" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v3"].pop("sleep_after"); json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "erneuter Doppelklick: Tilgen nicht wiederholt, Rest durch" '[ $(rc) = 0 ] && ! esends | grep -q " repay " && esends | grep -q " close " && esends | grep -q " pool-open "'
setup v9-deploy "$(print -r -- "$BASE" | sed 's/"v4":{/"v4":{"sleep_after":{"deploy":4},/')"; v3state; rm $D/keys/mainnet-signer.json; ctrlc "wirkt, wartet deploy" calls.log
ok "beim Deployment (Teil B): Exit 130, Stand nennt Teil A und Unterzeichner-Datei als erledigt" '[ $(rc) = 130 ] && grep -q "Unterbrochen während: Register, Orakel, Factory und GHOST (Version 4) angelegt" $D/out.txt && grep -A3 "In diesem Lauf schon gesendet:" $D/out.txt | grep -q "Unterzeichner-Datei keys/mainnet-signer.json angelegt" && grep -A1 "Noch offen (laut Plan):" $D/out.txt | grep -q "Register, Orakel, Factory und GHOST" && lockfree'

echo "V10) Agent läuft: auch einer von Version 3 (bin/ghostctl-v3) führt zum Abbruch"
setup v10-agent3 "$BASE"; v3state; MOCK_AGENT=3 run
ok "Attrappe: Abbruch, kein Aufruf, nichts umbenannt" '[ $(rc) != 0 ] && grep -q "läuft noch ein GHOST-Agent" $D/out.txt && [ ! -s $D/calls.log ] && [ ! -e $D/deployments/mainnet-v3.json ] && lockfree'
for wer in "./ghostctl --network mainnet --state deployments/mainnet.json --ja agent --key x" "bin/ghostctl-v3 --network mainnet --state deployments/mainnet-v3.json oracle-feed --key x"; do
  setup v10-echt "$BASE"; v3state
  ( cd $D && exec -a "$wer" sleep 15 ) &
  AGENT=$!; sleep 0.5
  (cd $D && PATH="${PATH#$MOCKBIN:}" ./GHOST-Umzug-v4.command <$JA >$D/out.txt 2>&1); echo $? > $D/rc
  kill $AGENT 2>/dev/null; wait $AGENT 2>/dev/null
  ok "echter pgrep sieht „${wer%% *}“ → Abbruch, nichts umbenannt, Sperre frei" '[ $(rc) != 0 ] && grep -q "läuft noch ein GHOST-Agent" $D/out.txt && [ ! -e $D/deployments/mainnet-v3.json ] && lockfree'
done

echo "V11) Abweichung in Teil B: Unterzeichner-Datei verschwindet während Teil A → Schritte geändert, erneut fragen"
VW="$(print -r -- "$BASE" | sed 's/"v3":{/"v3":{"rm_after":{"close":"keys\/mainnet-signer.json"},/')"
setup v11-n "$VW"; v3state; runa $'j\nn'
ok "nennt die geänderten Schritte und den neuen Schritt, fragt erneut" 'grep -q "Die Schritte von Teil B haben sich geändert." $D/out.txt && grep -q "Unterzeichner-Datei keys/mainnet-signer.json anlegen" $D/out.txt && grep -q "Teil B so ausführen? \[j/N\]" $D/out.txt'
ok "N: Teil A gesendet, keine Datei angelegt, nichts von Teil B" '[ $(rc) != 0 ] && esends | grep -q " close " && ! grep -q "committee-keygen" $D/calls.log && ! esends | grep -q "^v4 "'
setup v11-j "$VW"; v3state; runa $'j\nj'
ok "j: Datei angelegt und Teil B durch" '[ $(rc) = 0 ] && grep -q "committee-keygen keys/mainnet-signer.json --count 1$" $D/calls.log && esends | grep -q " pool-open "'

echo "\n== W: Audit 16 (Befunde M-1 bis M-4, N-2/N-3/N-5/N-6 und die Rückbau-Lücken R1–R5 des Prüfers) =="
echo "W1) Echter Stand des Nutzers: Vault 0 mit 50 KAS und 0,5 GHOST Schuld, 0,25 GHOST frei, 1 LP-UTXO, Pool 5,73 KAS / 0,25 GHOST (1 KAS Mindestliquidität)"
ECHT='{"v3":{"kasUsd":0.0436,"keys":{"@A":{"ghost":0.25,"lpUtxos":["473000000"],"kas":100}},
 "pool":{"shares":"573000000","kasSompi":"573000000","ghostUnits":"25000000"},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.5,"collateralKas":50,"interestUsd":0}]},
 "v4":{"kasUsd":0.0436,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":100}},"vaults":[]}}'
setup w1-echt "$ECHT"; v3state; rm $D/keys/mainnet-signer.json; run
ok "Plan: Abzug 4,73 KAS und 0,20636998 GHOST, Tilgen 0,45636998, Sicherheit 50 → 2,21 KAS" 'grep -q "zurück: etwa 4.73000000 KAS und 0.20636998 GHOST" $D/out.txt && grep -q "Vault 0: 0.45636998 von 0.50000000 GHOST Schuld tilgen" $D/out.txt && grep -q "Vault 0: Sicherheit von 50.00000000 auf 2.21 KAS senken (Restschuld 0.04363002 GHOST bleibt)" $D/out.txt'
ok "R5: --keep aufgerundet (2,2015 → 2,21), gesendet wie geplant" 'esends | grep -q "withdraw --key keys/mainnet-owner.json --vault 0 --keep 2.21$" && esends | grep -q "pool-remove --key keys/mainnet-owner.json --percent 100 --min-kas 4.68270000 --min-ghost 0.20430628$" && esends | grep -q "repay --key keys/mainnet-owner.json --vault 0 --ghost 0.45636998$"'
ok "M-3: Block „Version 3 bleibt für immer“ mit Pool, Covenants, Minter-Zweig und Grund der Restschuld" 'grep -q "^Version 3 bleibt für immer (nicht zurückholbar):" $D/out.txt && grep -q "im Pool von Version 3: 1.00000000 KAS und 0.04363002 GHOST, darunter die Mindestliquidität" $D/out.txt && grep -q "Covenants von Version 3: etwa 30 KAS (Orakel, Factory und GHOST-Wurzel je 10 KAS)" $D/out.txt && grep -q "Minter-Zweige: je eigener Vault 3 KAS, auch nach dem Schließen – jetzt 1 eigene(r) Vault(s) = 3 KAS" $D/out.txt && grep -q "Grund: die GHOST zum Tilgen fehlen, sie liegen in der Mindestliquidität des Pools von Version 3" $D/out.txt'
ok "M-3: Abzug nennt das Gebundene im Pool" 'grep -q "gebunden: im Pool von Version 3 bleiben 1.00000000 KAS und 0.04363002 GHOST, darunter die Mindestliquidität (1 KAS und GHOST im Wert von 1 KAS) – für immer" $D/out.txt'
ok "Lauf endet mit 0, eine Frage, Fertig" '[ $(rc) = 0 ] && [ $(nfrage) = 1 ] && grep -q "Fertig: Version 4 läuft" $D/out.txt'

echo "W2) M-1: Herausnehmen nur ab 0,5 KAS; ohne Tilgen davor mit --dry-run gebaut"
setup w2-klein '{"v3":{"kasUsd":0.0436,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.04,"collateralKas":2.21}]},"v4":{"kasUsd":0.0436,"keys":{"@A":{"ghost":0,"kas":100}},"vaults":[]}}'
v3state; run
ok "Rest-Vault 2,21 KAS (Keep 2,02): kein withdraw, als bleibend genannt, Lauf durch" '[ $(rc) = 0 ] && ! grep -q " withdraw " $D/calls.log && grep -q "Vault 0 mit 2.21000000 KAS Sicherheit und 0.04000000 GHOST Restschuld (Herausnehmen brächte unter 0.5 KAS)" $D/out.txt'
: > $D/calls.log; run
ok "Fortsetzung hängt nicht am Herausnehmen: zweiter Lauf ohne Teil A, nichts zu senden" '[ $(rc) = 0 ] && grep -q "Nichts mehr zu senden" $D/out.txt'
setup w2-probe '{"v3":{"kasUsd":0.0436,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0.04,"collateralKas":5}]},"v4":{"kasUsd":0.0436,"keys":{"@A":{"ghost":0,"kas":100}},"vaults":[]}}'
v3state; runa n
ok "Herausnehmen ohne Tilgen davor: im Plan gebaut (Probe)" 'grep -q "^v3 .*--dry-run --json withdraw --key keys/mainnet-owner.json --vault 0 --keep 2.02$" $D/calls.log && grep -A3 "Sicherheit von 5.00000000 auf 2.02 KAS senken" $D/out.txt | grep -q "Gebühr: 0.0412 KAS (mit --dry-run gebaut)"'

echo "W3) N-6: schuldenfreier Vault mit weniger als 0,5 KAS wird nicht geschlossen"
setup w3-schliessen '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":0,"collateralKas":0.4}]},"v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"kas":100}},"vaults":[]}}'
v3state; run
ok "kein close, im Endstand als bleibend genannt, Lauf durch" '[ $(rc) = 0 ] && ! grep -q " close " $D/calls.log && grep -q "Vault 0 ohne Schuld mit 0.40000000 KAS – Schließen brächte nur 0.40000000 KAS zurück" $D/out.txt'

echo "W4) M-2: Pool-Anteile auf mehreren UTXOs; R1: Kappung auf die 1-KAS-Reserve"
PW='{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.8,"lpUtxos":LPS,"kas":100}},
 "pool":{"shares":"1000","kasSompi":"2000000000","ghostUnits":"50000000"},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
setup w4-drei "${PW/LPS/[\"300\",\"100\",\"100\"]}"; v3state; run
ok "drei LP-UTXOs: Abbruch vor der Frage mit Erklärung, nichts gesendet, nichts umbenannt" '[ $(rc) != 0 ] && grep -q "auf mehr als zwei UTXOs" $D/out.txt && grep -q "keinen Befehl" $D/out.txt && [ $(nfrage) = 0 ] && [ -z "$(esends)" ] && [ -f $D/deployments/mainnet.json ] && [ ! -e $D/deployments/mainnet-v3.json ] && lockfree'
setup w4-zwei "${PW/LPS/[\"300\",\"200\"]}"; v3state; run
ok "zwei LP-UTXOs: Abzug wie gebaut (10 KAS, 0,25 GHOST), durch" '[ $(rc) = 0 ] && grep -q "zurück: etwa 10.00000000 KAS und 0.25000000 GHOST" $D/out.txt && esends | grep -q "pool-remove .* --min-kas 9.90000000 --min-ghost 0.24750000$"'
setup w4-kapp '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpUtxos":["950"],"kas":100}},
 "pool":{"shares":"1000","kasSompi":"1000000000","ghostUnits":"100000000"},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; run
ok "R1: KAS-Seite auf 1 KAS Mindestreserve gekappt (9,5 → 9,0), Mindestbetrag daraus" 'grep -q "zurück: etwa 9.00000000 KAS und 0.95000000 GHOST" $D/out.txt && esends | grep -q "pool-remove .* --min-kas 8.91000000 --min-ghost 0.94050000$" && [ $(rc) = 0 ]'

echo "W5) M-4: pool-open bricht nach Schritt 2 bzw. 3 ab → erneuter Doppelklick setzt fort, „Fertig“ erst mit fertigem Pool"
setup w5-schritt3 "$(print -r -- "$BASE" | sed 's/"v4":{/"v4":{"pool_open_fail":3,/')"; v3state; run
ok "1. Lauf: Abbruch beim Pool, kein „Fertig“" '[ $(rc) != 0 ] && grep -q "Pool anlegen fehlgeschlagen" $D/out.txt && ! grep -q "Fertig: Version 4" $D/out.txt'
python3 - "$D/mock.json" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v4"].pop("pool_open_fail"); json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "2. Lauf: Plan „Pool fertig anlegen“, pool-add mit dem Rest, dann Fertig" '[ $(rc) = 0 ] && grep -q "Pool fertig anlegen: 5.25000000 KAS und 0.21000000 GHOST einlegen" $D/out.txt && esends | grep -q "^v4 .*--ja pool-add --key keys/mainnet-owner.json --kas 5.25000000 --ghost 0.21000000$" && ! esends | grep -q " pool-open " && grep -q "Fertig: Version 4 läuft" $D/out.txt'
setup w5-schritt2 "$(print -r -- "$BASE" | sed 's/"v4":{/"v4":{"pool_open_fail":2,/')"; v3state; run
ok "Abbruch nach der Genesis: kein „Fertig“" '[ $(rc) != 0 ] && ! grep -q "Fertig: Version 4" $D/out.txt'
python3 - "$D/mock.json" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); m["v4"].pop("pool_open_fail"); json.dump(m,open(sys.argv[1],"w"))
PY
: > $D/calls.log; run
ok "2. Lauf: pool-open setzt nach der Genesis fort, Fertig" '[ $(rc) = 0 ] && grep -q "pool-open setzt fort" $D/calls.log && grep -q "Fertig: Version 4 läuft" $D/out.txt'
setup w5-halb "$LEER"; v4state
python3 - "$D/mock.json" "$A" <<'PY'
import json,sys; m=json.load(open(sys.argv[1])); a=sys.argv[2]
m["v4"]["vaults"]=[{"owner":a,"covenantId":"z","debtGhost":0.5,"collateralKas":50}]; m["v4"]["keys"][a]["ghost"]=0.21
m["v4"]["pool"]={"kasSompi":"100000000","ghostUnits":"4000000","shares":"100000000"}; json.dump(m,open(sys.argv[1],"w"))
PY
runa n
ok "halb angelegter Pool ohne eigene Anteile: nicht „Nichts mehr zu senden“, sondern Schritt im Plan" 'grep -q "Pool fertig anlegen" $D/out.txt && ! grep -q "Nichts mehr zu senden" $D/out.txt'

echo "W6) N-2: Kurs zu niedrig für 0,5 GHOST aus 50 KAS → Abbruch vor der Frage"
setup w6-kurs "$(print -r -- "$BASE" | sed 's/"v4":{/"v4":{"price":0.019,/')"; v3state; run
ok "Abbruch mit Grund, keine Frage, nichts gesendet, nichts umbenannt" '[ $(rc) != 0 ] && grep -q "Bei 0.019 USD je KAS reichen 50 KAS Sicherheit nicht, um 0.5 GHOST zu prägen" $D/out.txt && [ $(nfrage) = 0 ] && [ -z "$(esends)" ] && [ ! -e $D/deployments/mainnet-v3.json ]'

echo "W7) N-3: Orakel von Version 4 eingefroren oder kurz davor → Preis-Update als eigener Schritt vor Prägen und Pool"
setup w7-frozen "$(print -r -- "$LEER" | sed 's/"v4":{/"v4":{"frozen":true,/')"; v4state; run
ok "Plan: Orakel-Schritt vor dem Vault, gesendet vor dem Prägen, Lauf durch" '[ $(rc) = 0 ] && grep -q "1. Orakel von Version 4 aktualisieren (es ist eingefroren)" $D/out.txt && grep -q "^v4 .*--ja oracle-update --key keys/mainnet-owner.json --committee keys/mainnet-signer.json$" $D/calls.log && [ $(grep -n " oracle-update " $D/calls.log | head -1 | cut -d: -f1) -lt $(grep -n "^v4 .* mint " $D/calls.log | cut -d: -f1) ] && [ $(nfrage) = 1 ]'
setup w7-bald "$(print -r -- "$LEER" | sed 's/"v4":{/"v4":{"freezeInMinutes":10,/')"; v4state; runa n
ok "friert in 10 Minuten ein → ebenfalls Schritt im Plan" 'grep -q "Orakel von Version 4 aktualisieren (es ist in 10 Minuten eingefroren)" $D/out.txt'
setup w7-unterwegs "$(print -r -- "$LEER" | sed 's/"v4":{/"v4":{"move_after":{"open-vault":{"frozen":true}},/')"; v4state; runa $'j\nj'
ok "friert erst unterwegs ein → fragt, j: Update, dann Prägen und Pool" '[ $(rc) = 0 ] && grep -q "Orakel jetzt aktualisieren" $D/out.txt && grep -q "^v4 .*--ja oracle-update " $D/calls.log && esends | grep -q " mint " && esends | grep -q " pool-open "'

echo "W8) R2: Orakel von Version 4 bewegt sich während Teil A (v4 schon angelegt) → vor Teil B Pool-Abweichung nennen und fragen"
setup w8-poolkurs '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60}],"move_other_after":{"close":{"kasUsd":0.05}}},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0.5,"lpShares":"0","kas":200}},"vaults":[{"owner":"@A","covenantId":"z","debtGhost":0.5,"collateralKas":50}]}}'
v4state; v3state mainnet-v3.json; runa $'j\nn'
ok "Pool: geplant 6,25, jetzt 5,00 KAS → „Teil B so ausführen?“, N: kein pool-open" 'grep -q "Pool: geplant 6.25000000 KAS, jetzt 5.00000000 KAS." $D/out.txt && grep -q "Teil B so ausführen? \[j/N\]" $D/out.txt && [ $(rc) != 0 ] && ! esends | grep -q " pool-open "'

echo "W9) R3: Zinsdatei wird VOR der Zustandsdatei umbenannt"
setup w9-reihe "$BASE"; v3state; echo '{"zins":"v3"}' > $D/deployments/mainnet-zins.json; run
ok "Reihenfolge: Zinsdatei, dann Zustand" '[ $(grep -n "umbenannt: deployments/mainnet-zins.json → deployments/mainnet-v3-zins.json" $D/out.txt | cut -d: -f1) -lt $(grep -n "umbenannt: deployments/mainnet.json → deployments/mainnet-v3.json" $D/out.txt | cut -d: -f1) ]'

echo "W10) R4: Zinsdatei taucht nach der Frage auf (ohne Teil A) → Abbruch direkt vor dem Deployment, sie bleibt erhalten"
setup w10-spaet "$(print -r -- "$LEER" | sed 's#"v4":{#"v4":{"write_after":{"committee-keygen":["deployments/mainnet-zins.json","{\\"spaet\\":1}"]},#')"
rm $D/keys/mainnet-signer.json; run
ok "Plan war ohne Zinsdatei; kein deploy, Meldung, Inhalt unverändert" '[ $(nfrage) = 1 ] && [ $(rc) != 0 ] && grep -q "committee-keygen" $D/calls.log && ! grep -q " deploy " $D/calls.log && grep -q "mainnet-zins.json liegt noch da" $D/out.txt && grep -q "spaet" $D/deployments/mainnet-zins.json'

echo "W11) N-5: eigener gesperrter Vault von Version 3 wird genannt"
setup w11-stale '{"v3":{"kasUsd":0.04,"keys":{"@A":{"ghost":2,"lpShares":"0","kas":100}},
 "vaults":[{"owner":"@A","covenantId":"c1","debtGhost":1,"collateralKas":60},{"owner":"@A","covenantId":"c9","debtGhost":0.2,"collateralKas":30,"stale":true}]},
 "v4":{"kasUsd":0.04,"keys":{"@A":{"ghost":0,"lpShares":"0","kas":200}},"vaults":[]}}'
v3state; runa n
ok "Endstand nennt den gesperrten Vault" 'grep -q "gesperrt und übersprungen: Vault 1 (30 KAS, Schuld 0.2 GHOST)" $D/out.txt'

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
