#!/bin/zsh
# Doppelklick: Umzug von GHOST Version 2 auf Version 3 (Mainnet).
#
# Vorher: GHOST-Agent und oracle-feed beenden (das Skript prüft das und bricht
# sonst ab – ein laufender Agent der Version 2 würde die neue Zustandsdatei
# überschreiben, Audit 11 A11-O-3).
#
# Teil A – Version 2 abbauen (Programm bin/ghostctl-v2, Zustand deployments/mainnet-v2.json)
#   0. Zustand der Version 2 beiseitelegen: mainnet.json → mainnet-v2.json, dazu
#      Journal (mainnet.pending.json), Deploy-Fortschritt (mainnet.deploy.json)
#      und Sperre (mainnet.lock). Nur umbenennen, nie löschen. Zeigt das Journal
#      nach einem unterbrochenen Umbenennen noch auf mainnet.json, wird es vor
#      jedem Aufruf von Version 2 auf mainnet-v2.json gerichtet.
#   1. eigene Anteile aus dem Tauschpool abziehen (100 %), mit --min-kas/--min-ghost
#      = Rückfluss laut bestätigtem Plan minus 1 % (Audit 13 A13-umzug-2)
#   2. eigene Vaults mit den eigenen GHOST tilgen, kleinste Schuld zuerst;
#      schuldenfreie Vaults schließen (KAS kommen zurück). Tilgen immer mit
#      --ghost; vor jedem Senden an einen Vault prüft das Skript, dass die Nummer
#      noch die bestätigte Covenant-ID trägt, danach, dass sich genau dieser Vault
#      wie erwartet geändert hat – sonst hält es an (Audit 13 A13-umzug-1)
#   3. bleibt Restschuld (GHOST aus der Pool-Mindestliquidität sind für immer
#      gebunden), wird die Sicherheit bis auf 200 % + 10 % Puffer herausgenommen
# Teil B – Version 3 anlegen (./ghostctl, Zustand deployments/mainnet.json)
#   4. Orakel, Factory und GHOST v3 anlegen (30 KAS bleiben dauerhaft gebunden)
#   5. eigenen Vault mit VAULT_KAS KAS eröffnen und MINT_GHOST GHOST prägen
#      (gibt es schon einen eigenen Vault ohne Schuld, wird nur geprägt; gibt es
#      einen mit Schuld oder einen gesperrten, wird nichts eröffnet). Eigen heißt:
#      Besitzer ist der x-only-Schlüssel genau der Besitzer-Datei ($KEYS-owner.json,
#      im Normalfall keys/mainnet-owner.json).
#   6. Tauschpool mit Kursband (1 USD ± 3 %) anlegen: POOL_GHOST GHOST und KAS zum
#      Orakelkurs – nur, wenn der Schlüssel genug GHOST und KAS hat
#
# Einmal bestätigen: Vor dem ersten Senden rechnet das Skript den ganzen Plan
# (Teil A mit --dry-run gebaut, Teil B aus dem Stand gerechnet) und zeigt je
# Schritt Aktion, Beträge, was zurückkommt, was gebunden bleibt, Gebühren,
# Toleranzen, mögliche Zusatz-Transaktionen („GHOST zusammenführen“) und den
# Endstand; im Kopf Netz, Schlüsseldatei, Adresse und x-only. Dann EINE Frage „Alles so ausführen? [j/N]“; nur bei j laufen
# alle Transaktionen mit --ja durch. Vor der Frage wird nichts gesendet und
# nichts umbenannt.
# Nach Teil A wird Teil B aus dem echten Stand neu gerechnet. Weicht er vom
# bestätigten Plan ab (andere Schritte, weniger KAS als geplant, anderer
# Pool-Betrag), hält das Skript an und fragt erneut, statt still andere
# Beträge zu senden. Ebenso vor dem Pool, wenn der Orakelkurs den Pool-Betrag
# um mehr als 1 % verschiebt.
# Bricht ein Schritt ab (auch per Ctrl+C oder Signal), stoppt das Skript sofort
# und nennt, was in diesem Lauf gesendet ist und was fehlt. Einfach erneut doppelklicken: erledigte Schritte
# werden übersprungen, die Zusammenfassung zeigt nur die restlichen, und es
# wird wieder einmal gefragt.
# EINZELN=1 ./GHOST-Umzug-v3.command: wie früher, jede Transaktion fragt einzeln (j).
# Zwei Umzüge gleichzeitig verhindert eine Sperre (deployments/.umzug-v3.lock).
# Probelauf ohne Senden: DRY=1 ./GHOST-Umzug-v3.command (nur DRY=1 ist ein Probelauf)

cd "$(dirname "$0")" || exit 1
NET=mainnet
KEYS=${KEYS:-keys/$NET}
OWNER="$KEYS-owner.json"
COMMITTEE="$KEYS-committee.json"
S2=deployments/$NET-v2.json
S3=deployments/$NET.json
V2=bin/ghostctl-v2
# Merker „Vault eröffnet, aber noch nicht als eigener gefunden“: solange er liegt,
# eröffnet kein weiterer Lauf einen Vault (Audit 12 A12-4)
VAULTMARK=deployments/.umzug-v3-vault.lock
VAULT_KAS=${VAULT_KAS:-50}
MINT_GHOST=${MINT_GHOST:-0.5}
POOL_GHOST=${POOL_GHOST:-0.25}
# KAS über dem Pool-Betrag hinaus: Anteils-Minter und Token-UTXOs (je 1 KAS) und Gebühren
POOL_KAS_EXTRA=4
LOCKDIR=deployments/.umzug-v3.lock

# Probelauf nur mit DRY=1; leer oder 0 heißt echt (A11-O-14: DRY=0 galt als Probelauf)
case "${DRY:-0}" in
  1) DRY_RUN=1 ;;
  0) DRY_RUN=0 ;;
  *) echo "DRY=$DRY verstehe ich nicht: DRY=1 ist ein Probelauf, DRY=0 (oder nichts) sendet echt."; exit 1 ;;
esac
DRYFLAG=()
(( DRY_RUN )) && DRYFLAG=(--dry-run)
# Standard: einmal für alles bestätigen, dann sendet ghostctl mit --ja ohne
# weitere Rückfrage. EINZELN=1: ohne --ja, ghostctl fragt je Transaktion.
case "${EINZELN:-0}" in
  1) EINZELN=1 ;;
  0) EINZELN=0 ;;
  *) echo "EINZELN=$EINZELN verstehe ich nicht: EINZELN=1 fragt vor jeder Transaktion, EINZELN=0 (oder nichts) einmal für alles."; exit 1 ;;
esac
JAFLAG=()
(( DRY_RUN || EINZELN )) || JAFLAG=(--ja)

# Nach der Bestätigung (UNTERWEGS=1) nennt jeder Abbruch auch, was in diesem Lauf
# schon gesendet ist und was fehlt (stand_zeigen).
UNTERWEGS=0
fail() { echo "\n✗ $1"; (( UNTERWEGS )) && stand_zeigen; (( DRY_RUN )) || read -k 1 "?Taste drücken zum Schließen …"; exit 1; }
py() { python3 -c "$@"; }
# 0 = Version 3, 1 = Version 2, 2 = nicht lesbar
state_version() { py '
import json,sys
try: d=json.load(open(sys.argv[1]))
except Exception: sys.exit(2)
sys.exit(0 if "treasury" in (d.get("vault_params") or {}) else 1)' "$1" 2>/dev/null; }

# Läuft noch ein GHOST-Agent oder oracle-feed (Version 2 oder 3)? Dann abbrechen:
# er würde nach dem Deployment die neue Zustandsdatei im alten Format zurückschreiben (A11-O-3)
no_agent() {
  local p
  p=$(pgrep -fl "ghostctl.*(agent|oracle-feed)" 2>/dev/null)
  [ -z "$p" ] && return 0
  if (( DRY_RUN )); then
    echo "Hinweis: Es läuft noch ein GHOST-Agent oder oracle-feed – vor dem echten Umzug beenden:\n$p"
    return 0
  fi
  fail "Es läuft noch ein GHOST-Agent oder oracle-feed:
$p
Bitte zuerst beenden (Fenster „GHOST-Agent starten“ schließen oder dort Ctrl+C), dann den Umzug erneut starten."
}

echo "=== Umzug GHOST Version 2 → Version 3 ($NET)$( (( DRY_RUN )) && echo ' – PROBELAUF, nichts wird gesendet') ==="
[ -f "$OWNER" ] || fail "Schlüsseldatei $OWNER fehlt"
[ -x "$V2" ] || fail "$V2 fehlt (Programm für den Abbau von Version 2)"

# Sperre gegen einen zweiten, gleichzeitig gestarteten Umzug (A11-O-6). mkdir ist
# atomar; der Ordner wird beim Beenden (auch Ctrl+C, Fenster schließen) entfernt.
mkdir -p deployments
if ! mkdir "$LOCKDIR" 2>/dev/null; then
  OTHER=$(cat "$LOCKDIR/pid" 2>/dev/null)
  fail "Ein anderer Umzug läuft schon (PID ${OTHER:-unbekannt}, Sperre $LOCKDIR).
Läuft sicher keiner mehr (z. B. nach einem Absturz), den Ordner $LOCKDIR entfernen und neu starten."
fi
echo $$ > "$LOCKDIR/pid"
trap 'rm -f "$LOCKDIR/pid" "$LOCKDIR/probe.log" "$LOCKDIR/plan-a.zsh"; rmdir "$LOCKDIR" 2>/dev/null' EXIT
trap sig_ende INT TERM HUP

no_agent

# 0. Zustandsdatei von Version 2 beiseitelegen (wird nur umbenannt, nicht gelöscht).
# Im echten Lauf wird vor der Bestätigung nur geprüft, ob das Umbenennen geht
# (Journal-Ziel, vorhandene Ziele); umbenannt wird erst nach „j“. Bis dahin
# rechnet der Plan mit Version 2 an ihrem alten Ort.
# Unter der Sperre der Zustandsdatei (flock wie ghostctl), damit kein
# ghostctl-Aufruf (z. B. die Seite) dazwischen ein Journal klärt. Das Journal
# zeigt auf die Datei, die es bei Annahme schreibt: Das Ziel wird auf den neuen
# Namen umgeschrieben, sonst schriebe Version 3 den v2-Inhalt nach mainnet.json (A11-O-4).
# rename_v2 1 = nur prüfen, rename_v2 0 = umbenennen
rename_v2() { py '
import fcntl, json, os, sys, time
s3, s2 = sys.argv[1], sys.argv[2]
r3, r2 = s3[:-len(".json")], s2[:-len(".json")]
pairs = [(r3 + ".pending.json", r2 + ".pending.json"), (r3 + ".deploy.json", r2 + ".deploy.json"), (s3, s2)]
lock = open(r3 + ".lock", "a")
t0 = time.time()
while True:
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        break
    except BlockingIOError:
        if time.time() - t0 > 60:
            sys.exit("Sperre " + r3 + ".lock ist seit 60 s belegt – läuft noch ein ghostctl? Nichts umbenannt.")
        time.sleep(0.3)
todo = [(a, b) for a, b in pairs if os.path.exists(a)]
clash = [b for a, b in todo if os.path.exists(b)] + [p for p in [r2 + ".lock"] if os.path.exists(p)]
if clash:
    sys.exit("Gibt es schon: " + ", ".join(clash) + " – nichts umbenannt, bitte von Hand klären.")
pend = r3 + ".pending.json"
new_target = None
if os.path.exists(pend):
    j = json.load(open(pend))
    tgt = j.get("target")
    if tgt:
        names = {os.path.abspath(a): b for a, b in pairs}
        nb = names.get(os.path.abspath(tgt))
        if nb is None:
            sys.exit("Journal " + pend + " zeigt auf " + tgt + " – bitte erst mit " + sys.argv[3] + " status klären. Nichts umbenannt.")
        new_target = os.path.abspath(nb) if os.path.isabs(tgt) else nb
if sys.argv[4] == "1":
    sys.exit(0)  # nur prüfen (vor der Bestätigung)
for a, b in todo:
    os.rename(a, b)
    print("  umbenannt: " + a + " → " + b)
if new_target is not None:
    p2 = r2 + ".pending.json"
    j = json.load(open(p2))
    j["target"] = new_target
    tmp = r2 + ".pending.tmp"
    with open(tmp, "w") as f:
        json.dump(j, f, indent=2)
    os.rename(tmp, p2)
    print("  Journal zeigt jetzt auf " + new_target)
os.rename(r3 + ".lock", r2 + ".lock")
print("  umbenannt: " + r3 + ".lock → " + r2 + ".lock")
' "$S3" "$S2" "$V2" "$1"; }
V2_AT_S3=0
if [ -f "$S3" ]; then
  state_version "$S3"
  V=$?
  [ $V = 2 ] && fail "$S3 ist nicht lesbar – bitte von Hand prüfen (nichts verändert)"
  if [ $V = 1 ]; then
    [ -f "$S2" ] && fail "$S3 ist Version 2, aber $S2 gibt es schon – bitte von Hand klären"
    if (( DRY_RUN )); then
      echo "Probelauf: $S3 würde in $S2 umbenannt (samt Journal, Deploy-Fortschritt und Sperre, soweit vorhanden)"
      S2=$S3
    else
      rename_v2 1 || fail "Umbenennen nicht möglich – nichts verändert, nichts gesendet"
      V2_AT_S3=1
    fi
  fi
fi

# Wurde das Umbenennen unterbrochen (Fenster zu, Absturz), liegt das Journal schon
# als mainnet-v2.pending.json, zeigt aber noch auf mainnet.json. Der nächste Aufruf
# von Version 2 schriebe den v2-Stand bei Annahme dorthin, wo Version 3 hinkommt
# (Audit 12 A12-17). Deshalb vor jedem Aufruf von Version 2 auf $S2 prüfen und
# umrichten: gleich hier, oder – liegt Version 2 noch als $S3 (V2_AT_S3=1) – erst
# nach dem Umbenennen (vorher arbeitet der Plan auf $S3, und die Sperre
# mainnet-v2.lock darf vor dem Umbenennen nicht entstehen).
journal_v2() {
  [ "$S2" != "$S3" ] && [ -f "${S2:r}.pending.json" ] || return 0
  py '
import fcntl, json, os, sys, time
s3, s2, dry = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
r2 = s2[:-len(".json")]
pend = r2 + ".pending.json"
if not dry:
    # unter der Sperre der Zustandsdatei von Version 2 (flock wie ghostctl)
    lock = open(r2 + ".lock", "a")
    t0 = time.time()
    while True:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            if time.time() - t0 > 60:
                sys.exit("Sperre " + r2 + ".lock ist seit 60 s belegt – läuft noch ein ghostctl? Nichts verändert.")
            time.sleep(0.3)
if not os.path.exists(pend):
    sys.exit(0)
try:
    j = json.load(open(pend))
    tgt = j.get("target")
except Exception:
    sys.exit("Journal " + pend + " ist nicht lesbar – bitte von Hand prüfen. Nichts verändert.")
if not tgt or os.path.realpath(tgt) == os.path.realpath(s2):
    sys.exit(0)
if os.path.realpath(tgt) != os.path.realpath(s3):
    sys.exit("Journal " + pend + " zeigt auf " + tgt + " statt auf " + s2 + " – bitte von Hand klären. Nichts verändert.")
new = os.path.abspath(s2) if os.path.isabs(tgt) else s2
if dry:
    sys.exit("Journal " + pend + " zeigt nach einem unterbrochenen Umbenennen noch auf " + tgt + ". Der echte Lauf richtet es auf " + new + "; der Probelauf bricht hier ab, damit kein Aufruf den Stand von Version 2 nach " + tgt + " schreibt.")
j["target"] = new
tmp = r2 + ".pending.tmp"
with open(tmp, "w") as f:
    json.dump(j, f, indent=2)
os.rename(tmp, pend)
print("  Journal " + pend + " zeigte noch auf " + tgt + " (Umbenennen war unterbrochen) – zeigt jetzt auf " + new)
' "$S3" "$S2" "$DRY_RUN" || fail "Journal von Version 2 nicht in Ordnung – nichts gesendet"
}
(( V2_AT_S3 )) || journal_v2

# Teil A arbeitet auf $SA: im Plan vor dem Umbenennen noch $S3, danach $S2
SA=$S2
G2() { "$V2" --network "$NET" --state "$SA" $JAFLAG $DRYFLAG "$@"; }
G3() { ./ghostctl --network "$NET" --state "$S3" $JAFLAG $DRYFLAG "$@"; }
# Feld des eigenen Schlüssels aus `keys --json`: MYKEY <programm> <zustand> <feld>.
# Eigen ist genau die Datei $OWNER: gelistet wird ihr Ordner, verglichen wird der
# volle Pfad (bzw. dieselbe Datei), nie die Endung des Namens. Per Endung passte
# auch keys/alt-mainnet-owner.json, und jeder Doppelklick eröffnete einen weiteren
# Vault (Audit 12 A12-4). Ein Symlink oder Hardlink auf die Besitzer-Datei im
# selben Ordner ist dieselbe Datei mit demselben Schlüssel und zählt einmal
# (Restpunkt C-R1: vorher Abbruch ohne Grund). Ohne genau einen Schlüssel, oder
# wenn er kein x-only aus 64 Hex-Zeichen ist: keine Ausgabe, der Grund auf
# stderr, Rückgabe 1.
MYKEY() { "$1" --network "$NET" --state "$2" --json keys --dir "$(dirname "$OWNER")" 2>/dev/null | py '
import json, os, re, sys
name = sys.argv[1]
own = os.path.realpath(name)
def same(f):
    try:
        return os.path.realpath(f) == own or os.path.samefile(f, own)
    except (OSError, TypeError):
        return False
try:
    ks = [k for k in json.load(sys.stdin)["keys"] if k.get("type") == "key" and same(k.get("file"))]
except Exception:
    sys.exit("  Grund: Schlüsselliste nicht lesbar")
xs = sorted({str(k.get("xonly")) for k in ks})
if not ks:
    sys.exit("  Grund: " + name + " steht nicht als Schlüssel in der Liste (unlesbar oder kein Schlüssel?)")
if len(xs) != 1:
    sys.exit("  Grund: " + name + " steht mehrfach mit verschiedenen Schlüsseln in der Liste (" + ", ".join(str(k.get("file")) for k in ks) + ")")
if not re.fullmatch("[0-9a-f]{64}", xs[0]):
    sys.exit("  Grund: der Schlüssel zu " + name + " ist kein x-only aus 64 Hex-Zeichen (" + xs[0][:70] + ")")
v = ks[0].get(sys.argv[2])
print("" if v is None else v)' "$OWNER" "$3"; }

# eigene Vaults (nicht die anderer Besitzer, A11-O-5): Anzahl samt gesperrten und
# Nummer eines eigenen ohne Schuld. Ohne lesbare Vault-Liste: Rückgabe 1, denn
# eröffnet wird nur, wenn es sicher keinen eigenen gibt (Audit 12 A12-4).
own_vaults() { ./ghostctl --network "$NET" --state "$S3" status --json 2>/dev/null | py '
import json,sys
try:
    vs = json.load(sys.stdin)["vaults"]
    own = [v for v in vs if str(v["owner"]).lower() == sys.argv[1]]
    z = next((v["index"] for v in own if not v.get("stale") and v["debtGhost"] == 0), "")
except Exception:
    sys.exit(1)
print(len(own), z)' "$ME3"; }
VAULTMARK_MSG="Nichts eröffnet. Bitte auf der Seite „Vault“ prüfen; erst wenn sicher kein Vault offen ist, $VAULTMARK entfernen und erneut doppelklicken."

# Rechnen mit Python: fx '<Ausdruck mit a[0], a[1] …>' <Zahl> … (leer zählt als 0);
# fq wie fx, aber als Bedingung (Rückgabe 0 = wahr)
fx() { py '
import sys
a = [float(x or 0) for x in sys.argv[2:]]
print("%.8f" % eval(sys.argv[1], {"__builtins__": {}}, {"a": a, "max": max, "min": min, "abs": abs}))' "$@"; }
fq() { py '
import sys
a = [float(x or 0) for x in sys.argv[2:]]
sys.exit(0 if eval(sys.argv[1], {"__builtins__": {}}, {"a": a, "max": max, "min": min, "abs": abs}) else 1)' "$@"; }
r2() { py 'import sys; print("%.2f" % float(sys.argv[1] or 0))' "$1"; }
summe() { py 'import sys; print("%.8f" % sum(float(x or 0) for x in sys.argv[1:]))' "$@"; }
# Beträge, die an Grenzen oder in --keep/--ghost/--min-* gehen, rechnet das Skript
# in Sompi bzw. GHOST-Einheiten (1e-8, ganze Zahlen) statt in Float: Float-Rauschen
# hob --keep sonst um 0,01 KAS an (Audit 13 A13-umzug-7). SU ist Python-Vorspann:
# su(x) Dezimalzahl → Sompi (kaufmännisch gerundet wie to_units in ghostctl),
# ks(s) Sompi → Dezimalzahl mit 8 Stellen. sompi <Betrag> für die Shell.
SU='
from decimal import Decimal, ROUND_HALF_UP
def su(x):
    return int((Decimal(str(x or 0)) * 100000000).to_integral_value(ROUND_HALF_UP))
def ks(s):
    return ("-" if s < 0 else "") + "%d.%08d" % divmod(abs(int(s)), 100000000)
'
sompi() { py "$SU"'
import sys; print(su(sys.argv[1]))' "$1"; }
ks() { py "$SU"'
import sys; print(ks(int(sys.argv[1])))' "$1"; }
# Prozent aus Basispunkten für die Anzeige: pct 100 → 1
pct() { py 'import sys; print("%g" % (int(sys.argv[1]) / 100))' "$1"; }

# Eine Frage mit j/N. Alles außer j (auch keine Eingabe) heißt nein.
frage() { local a; print -n -- "\n$1 [j/N] "; read -r a || a=""; [[ "$a" = [jJ] ]]; }

# ------------------------------------------------------------------- Plan ----
# Ein Eintrag je Schritt: Teil (A/B), Text, KAS an den Schlüssel (+ zurück,
# − ab, ohne Gebühr), GHOST der Version 3 (+/−), was zurückkommt, was gebunden
# bleibt, Anzahl Transaktionen, Gebühr in KAS und woher sie stammt („Probe“ =
# mit --dry-run gebaut, „geschätzt“ = Erfahrungswert aus MAINNET.md).
FEE_VAULT=0.05   # Vault-Aktion ≈ 0,04–0,05 KAS
FEE_POOL1=0.06   # eine Pool-Transaktion
FEE_POOL3=0.18   # Pool anlegen: drei Transaktionen
FEE_DEPLOY=0.05  # Deployment: drei Transaktionen
TOL_KAS=0.5      # so viel weniger KAS vor Teil B gilt noch als planmäßig (Gebühren)
TOL_POOL=0.01    # Pool-Betrag: 1 % Abweichung (Orakelkurs) gilt noch als planmäßig
TOL_ABZUG_BPS=100  # Pool-Anteile abziehen: so viel weniger (Basispunkte) als geplant darf zurückkommen, sonst sendet ghostctl nicht
PL_TEIL=(); PL_T=(); PL_K=(); PL_G=(); PL_Z=(); PL_B=(); PL_N=(); PL_F=(); PL_Q=(); PL_SA=(); PL_H=()
PL_BASIS=0       # so viele gesendete Schritte liegen vor dem ersten Eintrag des Plans
GESENDET=()      # in diesem Lauf gesendete Schritte
REST_V2=()       # was von Version 2 stehen bleibt
# Teil A trägt als zehntes Feld die Kennung des Sendebefehls ein (PL_SA: Befehl,
# Covenant-ID und Betrag); der echte Lauf vergleicht vor jedem Senden damit.
# Elftes Feld (PL_H): Hinweise zum Schritt, je Zeile einer (Mindestbeträge,
# Toleranz, mögliche Zusatz-Transaktion „GHOST zusammenführen“).
plan_add() { PL_TEIL+=("$1"); PL_T+=("$2"); PL_K+=("$3"); PL_G+=("$4"); PL_Z+=("$5"); PL_B+=("$6"); PL_N+=("$7"); PL_F+=("$8"); PL_Q+=("$9"); PL_SA+=("${10}"); PL_H+=("${11}"); }
plan_leeren() { PL_TEIL=(); PL_T=(); PL_K=(); PL_G=(); PL_Z=(); PL_B=(); PL_N=(); PL_F=(); PL_Q=(); PL_SA=(); PL_H=(); }
# ghostctl führt vor Tilgen und Pool-Anlegen eigene GHOST zusammen, wenn sie auf
# mehr als zwei UTXOs liegen: eine eigene Transaktion, mit --ja ohne Rückfrage
# (Audit 13 A13-umzug-3). Die Zusammenfassung nennt sie als möglich.
H_ZUS="dazu ggf. 1 Transaktion „GHOST zusammenführen“ (nur wenn deine GHOST auf mehr als zwei UTXOs liegen; Selbstüberweisung, kostet nur Gebühr)"
# Gebühr aus der Probe, sonst die Schätzung: gebuehr <aus der Probe> <Schätzung> → "<KAS> <Quelle>"
gebuehr() { [ -n "$1" ] && echo "$1 Probe" || echo "$2 geschätzt"; }

# Liegen die GHOST auf mehr als zwei UTXOs, führt ghostctl sie vor dem Tilgen
# erst zusammen (1–2 Selbstüberweisungen) – das lässt sich nicht mit --dry-run
# bauen, echt geht es. Die Probe scheitert dann mit genau dieser Meldung.
H_ZUS_PLAN="vorher 1–2 Transaktionen „GHOST zusammenführen“ (deine GHOST liegen auf mehr als zwei UTXOs; Selbstüberweisung an dich, kostet nur Gebühr) – das Tilgen selbst ist deshalb nur geschätzt"
braucht_zusammenfuehren() { tail -n 5 "$PROBELOG" 2>/dev/null | grep -q "mehr als zwei UTXOs"; }

plan_zeigen() {
  local i t="" f
  for (( i = 1; i <= ${#PL_T}; i++ )); do
    if [ "${PL_TEIL[i]}" != "$t" ]; then
      t=${PL_TEIL[i]}
      [ "$t" = A ] && echo "Teil A – Version 2 abbauen (bin/ghostctl-v2):" || echo "Teil B – Version 3 anlegen (./ghostctl):"
    fi
    echo "  $i. ${PL_T[i]}"
    [ -n "${PL_Z[i]}" ] && echo "       zurück: ${PL_Z[i]}"
    [ -n "${PL_B[i]}" ] && echo "       gebunden: ${PL_B[i]}"
    if [ "${PL_Q[i]}" = Probe ]; then f="${PL_F[i]} KAS (mit --dry-run gebaut)"; else f="etwa ${PL_F[i]} KAS (geschätzt)"; fi
    (( ${PL_N[i]} > 1 )) && f="$f, ${PL_N[i]} Transaktionen"
    echo "       Gebühr: $f"
    [ -n "${PL_H[i]}" ] && print -r -- "${PL_H[i]}" | while IFS= read -r h; do echo "       $h"; done
  done
  true
}

# Kopf der Zusammenfassung: womit gesendet wird (Audit 13 A13-umzug-5). KEYS aus
# der Umgebung stellt die Schlüsseldatei um – hier sieht man es vor dem „j“.
kopf_zeigen() {
  local adr xo
  adr=$(MYKEY ./ghostctl "$S3" address 2>/dev/null)
  xo=$(MYKEY ./ghostctl "$S3" xonly 2>/dev/null)
  [ -n "$xo" ] || xo=$ME
  echo "Netz:           $NET"
  echo "Schlüsseldatei: $OWNER (${OWNER:A})"
  echo "Adresse:        ${adr:-unbekannt}"
  echo "x-only:         ${xo:-unbekannt}"
}

# Endstand: endstand <KAS jetzt> <KAS vor Teil B oder leer>
endstand() {
  local fs n w z=0
  fs=$(summe "${PL_F[@]}"); n=0
  for w in "${PL_N[@]}"; do n=$((n + w)); done
  for w in "${PL_H[@]}"; do [[ "$w" = *"GHOST zusammenführen"* ]] && z=$((z + 1)); done
  echo "\nGebühren zusammen: etwa $(py 'import sys; print("%.4f" % float(sys.argv[1]))' "$fs") KAS für $n Transaktionen$( (( z )) && echo ", dazu ggf. bis zu $z × „GHOST zusammenführen“ (je etwa $FEE_VAULT KAS)")"
  echo "Endstand (geschätzt):"
  echo "  KAS frei auf dem Schlüssel: jetzt $(r2 "$1")$( [ -n "$2" ] && echo ", vor Teil B etwa $(r2 "$2")"), am Ende etwa $(r2 "$PB_KAS")"
  echo "  GHOST der Version 3: am Ende $PB_G"
  [ -n "$G2_REST" ] && echo "  GHOST der Version 2: danach $G2_REST (gelten in Version 3 nicht)"
  for w in "${REST_V2[@]}"; do echo "  Version 2 bleibt: $w"; done
  for w in "${PB_WARN[@]}"; do echo "  ⚠ $w"; done
  (( ${#PL_T} > 1 )) && echo "  (Vault-Nummern: Stand jetzt. Nach dem Schließen rücken spätere Nummern nach.)"
  true
}

# Stand nach einem Abbruch: was in diesem Lauf gesendet ist, was fehlt
stand_zeigen() {
  local i von
  if (( ${#GESENDET} )); then
    echo "In diesem Lauf schon gesendet:"
    for i in "${GESENDET[@]}"; do echo "  ✓ $i"; done
  else
    echo "In diesem Lauf wurde nichts gesendet."
  fi
  # Weicht Teil A ab, ersetzt a_neu_planen die offenen Einträge (die gesendeten
  # bleiben stehen): die Zählung trifft dann die neu gerechneten Schritte.
  von=$(( ${#GESENDET} - PL_BASIS + 1 ))
  (( von < 1 )) && von=1
  if (( von <= ${#PL_T} )); then
    echo "Noch offen (laut Plan):"
    for (( i = von; i <= ${#PL_T}; i++ )); do echo "  – ${PL_T[i]}"; done
  fi
  echo "Ob eine abgebrochene Transaktion doch angenommen wurde, klärt ghostctl beim nächsten Aufruf (Journal)."
  echo "Erneuter Doppelklick setzt fort: Er zeigt die restlichen Schritte und fragt noch einmal."
}

# Senden: tx <G2|G3> <Text für „gesendet“> <Meldung bei Abbruch> <Befehl …>.
# Im Probelauf (DRY=1) wie bisher nur mit --dry-run.
LAUFEND=""   # Text des Schritts, dessen ghostctl gerade läuft (für den Signal-Abbruch)
tx() {
  local fn=$1 was=$2 msg=$3
  shift 3
  if (( DRY_RUN )); then
    $fn "$@" || fail "$msg"
    return 0
  fi
  LAUFEND=$was
  $fn "$@" || { LAUFEND=""; fail "$msg"; }
  LAUFEND=""
  GESENDET+=("$was")
}

# Abbruch per Signal (Ctrl+C, TERM, Fenster zu): nach der Bestätigung wie fail()
# den Stand nennen – was gesendet ist, was fehlt, und welcher Schritt gerade
# lief (Audit 13 A13-umzug-4). zsh wartet ein laufendes ghostctl erst ab.
sig_ende() {
  trap - INT TERM HUP
  echo "\n✗ Abgebrochen (Signal)."
  if (( UNTERWEGS )); then
    [ -n "$LAUFEND" ] && echo "Unterbrochen während: $LAUFEND – ob diese Transaktion angenommen wurde, klärt ghostctl beim nächsten Aufruf (Journal)."
    stand_zeigen
  fi
  exit 130
}

# Senden in Teil A: txa <Kennung> <Schritt, wie er jetzt ist> <Text für „gesendet“>
# <Meldung bei Abbruch> <Befehl …>. Tilgen, Schließen und Herausnehmen rechnet
# Teil A aus dem jeweils aktuellen Stand (eigene GHOST nach dem Pool, Orakel von
# Version 2). Deshalb vergleicht der echte Lauf vor jedem Senden die Kennung
# (Befehl, Covenant-ID, Betrag) mit dem bestätigten Plan (PL_SA[AI]). Weicht sie
# ab, wird Teil A aus dem jetzigen Stand neu gerechnet, gezeigt und erneut
# gefragt – andere Beträge oder Schritte gehen nie still hinaus.
AI=0   # im echten Teil A: Nummer des nächsten Planeintrags; sonst 0
NA=0   # so viele Einträge des Plans gehören zu Teil A
txa() {
  local sig=$1 jetzt=$2 n=0
  shift 2
  if [ $AM = echt ]; then
    while (( AI > NA )) || [ "$sig" != "${PL_SA[AI]}" ]; do
      n=$((n + 1))
      (( n > 3 )) && fail "Teil A weicht auch nach dreimal neu Rechnen vom Plan ab – der Stand ändert sich laufend. Der Schritt ist nicht gesendet; später erneut doppelklicken."
      echo "\n⚠ Teil A weicht vom bestätigten Plan ab:"
      if (( AI <= NA )); then echo "  geplant: ${PL_T[AI]}"; else echo "  geplant: kein weiterer Schritt in Teil A"; fi
      echo "  jetzt:   $jetzt"
      a_neu_planen
      if (( EINZELN )); then
        echo "\nEINZELN=1: Jede Transaktion fragt gleich einzeln nach j."
        break
      fi
      frage "Teil A so ausführen?" || fail "Teil A angehalten, der abweichende Schritt ist nicht gesendet. Ein erneuter Doppelklick zeigt den Plan aus dem jetzigen Stand und fragt erneut."
    done
  fi
  # erst direkt vor dem Senden (nach einer möglichen Frage) die Nummer prüfen
  (( ${#VC} )) && ! (( DRY_RUN )) && vault_vor
  tx G2 "$@"
  (( ${#VC} )) && ! (( DRY_RUN )) && vault_nach
  VC=()
  [ $AM = echt ] && AI=$((AI + 1))
  true
}

# Vault-Prüfung um jedes Senden an einen Vault von Version 2 (Audit 13
# A13-umzug-1). bin/ghostctl-v2 kennt den Vault nur als Nummer (--vault), bestätigt
# ist aber der Vault mit der Covenant-ID. Verschwindet ein Vault mit kleinerer
# Nummer (ein fremder wird geschlossen oder liquidiert), rücken die Nummern nach,
# und dieselbe Nummer träfe einen anderen eigenen Vault. Deshalb:
#  - unmittelbar vor dem Senden den Status neu lesen: trägt die Nummer nicht mehr
#    die bestätigte Covenant-ID, anhalten, ohne zu senden (vault_vor);
#  - nach dem Senden prüfen, dass sich genau dieser Vault wie erwartet geändert
#    hat und die übrigen eigenen Vaults nicht; sonst anhalten (vault_nach).
# Der Aufrufer setzt vor txa VC=(<repay|close|withdraw> <Covenant-ID> <Nummer>
# <GHOST bei repay bzw. --keep bei withdraw, in Sompi>).
VC=()
# v2_abdruck <Covenant-ID>: „<Nummer> <Schuld> <Sicherheit> <übrige eigene>“ aus
# einem frisch gelesenen Status, Schuld und Sicherheit in Sompi; die übrigen
# eigenen als „CID:Schuld/Sicherheit,…“. Fehlt der Vault, „- - - <übrige>“.
v2_abdruck() {
  "$V2" --network "$NET" --state "$SA" status --json 2>/dev/null | py "$SU"'
import json, sys
try:
    vs = json.load(sys.stdin)["vaults"]
except Exception:
    sys.exit(1)
cid, me = sys.argv[1], sys.argv[2]
v = next((v for v in vs if v["covenantId"] == cid), None)
rest = ",".join(sorted("%s:%s/%s" % (w["covenantId"], ks(su(w["debtGhost"])), ks(su(w["collateralKas"]))) for w in vs if w["owner"] == me and w["covenantId"] != cid)) or "-"
print(("%d %d %d" % (v["index"], su(v["debtGhost"]), su(v["collateralKas"])) if v else "- - -") + " " + rest)' "$1" "$ME"
}
VC_VI=""; VC_D=0; VC_C=0; VC_REST=""
vault_vor() {
  local ab
  ab=$(v2_abdruck "${VC[2]}") && [ -n "$ab" ] || fail "Status von Version 2 direkt vor dem Senden nicht lesbar – ${VC[1]} an Vault ${VC[3]} ist nicht gesendet. Später erneut doppelklicken."
  read VC_VI VC_D VC_C VC_REST <<<"$ab"
  [ "$VC_VI" = "${VC[3]}" ] || fail "Vault-Nummer verschoben: Nummer ${VC[3]} trägt nicht mehr den bestätigten Vault ${VC[2]} ($( [ "$VC_VI" = - ] && echo "den gibt es nicht mehr" || echo "er hat jetzt Nummer $VC_VI")). ${VC[1]} ist nicht gesendet. Ein erneuter Doppelklick rechnet mit den neuen Nummern und fragt erneut."
}
vault_nach() {
  local ab vi d c rest ok=1 soll
  ab=$(v2_abdruck "${VC[2]}") && [ -n "$ab" ] || fail "Status von Version 2 nach dem Senden nicht lesbar – ob ${VC[1]} den Vault ${VC[2]} wie bestätigt getroffen hat, ist offen. Nichts weiter gesendet; bitte auf der Seite prüfen, dann erneut doppelklicken."
  read vi d c rest <<<"$ab"
  case ${VC[1]} in
    repay)    soll="Schuld $(ks $(( VC_D > VC[4] ? VC_D - VC[4] : 0 ))) GHOST, Sicherheit $(ks $VC_C) KAS"
              [ "$vi" != - ] && (( d == (VC_D > VC[4] ? VC_D - VC[4] : 0) && c == VC_C )) || ok=0 ;;
    close)    soll="geschlossen"
              [ "$vi" = - ] || ok=0 ;;
    withdraw) soll="Schuld $(ks $VC_D) GHOST, Sicherheit $(ks ${VC[4]}) KAS"
              [ "$vi" != - ] && (( d == VC_D && c == VC[4] )) || ok=0 ;;
  esac
  [ "$rest" = "$VC_REST" ] || ok=0
  (( ok )) && return 0
  GESENDET[-1]+=" – gesendet, aber die Wirkung weicht ab (siehe oben)"
  fail "Nach dem Senden hat sich nicht genau der bestätigte Vault wie erwartet geändert:
  Vault ${VC[2]} sollte jetzt sein: $soll
  ist: $( [ "$vi" = - ] && echo "nicht mehr da" || echo "Nummer $vi, Schuld $(ks $d) GHOST, Sicherheit $(ks $c) KAS")
  übrige eigene Vaults $( [ "$rest" = "$VC_REST" ] && echo "unverändert" || echo "verändert (Covenant-ID:Schuld/Sicherheit vorher $VC_REST, jetzt $rest)")
Angehalten – nichts weiter gesendet. Bitte den Stand auf der Seite „Vault“ prüfen; ein erneuter Doppelklick rechnet aus dem jetzigen Stand und fragt erneut."
}

# Teil A aus dem jetzigen Stand neu rechnen (wie der Plan, mit --dry-run, in einer
# Subshell), zeigen und die offenen Einträge von Teil A im Plan (ab AI) ersetzen.
# Die KAS vor Teil B (PLAN_KAS_B) werden mit dem neuen Rest von Teil A gerechnet.
a_neu_planen() {
  local f="$LOCKDIR/plan-a.zsh" a kas
  local -a N_TEIL N_T N_K N_G N_Z N_B N_N N_F N_Q N_SA N_H
  rm -f "$f"
  ( AM=plan; DRY_RUN=1; UNTERWEGS=0; AI=0; plan_leeren; REST_V2=(); G2_REST=""
    teil_a
    for a in TEIL T K G Z B N F Q SA H; do
      eval "print -r -- \"N_$a=(\${(@qq)PL_$a})\""
    done > "$f"
    echo "\nTeil A aus dem jetzigen Stand:"
    (( ${#PL_T} )) && plan_zeigen || echo "  (kein Schritt mehr)"
    for a in "${REST_V2[@]}"; do echo "  Version 2 bleibt: $a"; done
    true
  ) && [ -f "$f" ] || fail "Teil A ließ sich nicht neu rechnen – der Schritt ist nicht gesendet."
  eval "$(<"$f")"
  rm -f "$f"
  for a in TEIL T K G Z B N F Q SA H; do
    eval "PL_${a}=(\"\${(@)PL_${a}[1,AI-1]}\" \"\${(@)N_${a}}\" \"\${(@)PL_${a}[NA+1,-1]}\")"
  done
  NA=$(( AI - 1 + ${#N_T} ))
  kas=$(MYKEY ./ghostctl "$S3" kas) && [ -n "$kas" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – der Schritt ist nicht gesendet, später erneut"
  PLAN_KAS_B=$(fx 'a[0] + a[1] - a[2]' "$kas" "$(summe "${N_K[@]}")" "$(summe "${N_F[@]}")")
  echo "  KAS frei auf dem Schlüssel: jetzt $(r2 "$kas"), vor Teil B etwa $(r2 "$PLAN_KAS_B")"
}

# Probe eines Schritts von Version 2 (--dry-run --json): gibt die Gebühr aus
# (Summe der gebauten Transaktionen) oder nichts, wenn ghostctl keine nennt.
# Meldungen landen in $PROBELOG und werden nur bei einem Fehler gezeigt.
PROBELOG="$LOCKDIR/probe.log"
probe2() {
  local out
  # mit --json steht ein Fehler als {"ok":false,"error":…} auf stdout: ins Protokoll
  out=$("$V2" --network "$NET" --state "$SA" --dry-run --json "$@" 2>>"$PROBELOG") || { print -r -- "$out" >>"$PROBELOG"; return 1; }
  echo "$out" | py '
import json,sys
try:
    t = json.load(sys.stdin)["transactions"]
    print("%.4f" % sum(float(x["feeKas"]) for x in t) if t else "")
except Exception:
    print("")'
}
probe_fail() { fail "Probe (--dry-run) für „$1“ gescheitert – nichts gesendet:
$(tail -n 5 "$PROBELOG" 2>/dev/null)"; }

# ---------------------------------------------------------------- Teil A ----
# AM=dry: Probelauf DRY=1 (Ausgabe wie bisher). AM=plan: rechnet wie der
# Probelauf, baut mit --dry-run, trägt die Schritte in den Plan ein, sendet
# nichts und gibt nichts aus. AM=echt: sendet.
teil_a() {
  local ST CIDS CID VI DEBT COLL HAVE HAVE0 PAY USED=0 KAS_USD KEEP SHARES PK=0 PG=0 PRET="" FEE Q F VOLL i
  local DEBT_S COLL_S HAVE_S HAVE0_S PAY_S KEEP_S USED_S=0 PG_S=0 MINK MING
  [ $AM = plan ] || echo "\n--- Teil A: Version 2 abbauen ---"
  ST=$("$V2" --network "$NET" --state "$SA" status --json 2>/dev/null) || fail "Status von Version 2 nicht abrufbar (Nodes?) – später erneut"
  ME=$(MYKEY "$V2" "$SA" xonly) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 2"
  V2_USD=$(echo "$ST" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])' 2>/dev/null)

  # 1. Pool-Anteile
  SHARES=$(MYKEY "$V2" "$SA" lpShares) || fail "Eigener Schlüssel nicht lesbar"
  if [ -n "$(echo "$ST" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")')" ] && [ "${SHARES:-0}" != "0" ]; then
    # Rückfluss wie pool::remove in ghostctl: Anteil m/S beider Reserven (ganzzahlig
    # abgerundet), KAS-Seite auf die 1-KAS-Mindestreserve gekappt. Mindestbeträge
    # für --min-kas/--min-ghost: das minus TOL_ABZUG_BPS. Sie stehen in der Kennung
    # (PL_SA): verschiebt sich der Pool bis zum Senden, wird neu gerechnet und
    # gefragt; verschiebt er sich danach, sendet ghostctl nicht (Audit 13 A13-umzug-2).
    PRET=$(echo "$ST" | py "$SU"'
import json,sys
p = json.load(sys.stdin).get("pool") or {}
try:
    s, x, y, m, t = int(p["shares"]), int(p["kasSompi"]), int(p["ghostUnits"]), int(sys.argv[1]), int(sys.argv[2])
    dx = max(0, min(x * m // s, x - 100000000))
    dy = y * m // s
    print(ks(dx), ks(dy), ks(dx * (10000 - t) // 10000), ks(dy * (10000 - t) // 10000))
except Exception:
    print("")' "$SHARES" "$TOL_ABZUG_BPS")
    [ -n "$PRET" ] || fail "Pool von Version 2 nicht lesbar (Reserven/Anteile) – ohne Mindestbeträge wird nicht abgezogen. Nichts gesendet, später erneut."
    read PK PG MINK MING <<<"$PRET"
    PG_S=$(sompi "$PG")
    if [ $AM = plan ]; then
      FEE=$(probe2 pool-remove --key "$OWNER" --percent 100 --min-kas "$MINK" --min-ghost "$MING") || probe_fail "Pool-Anteile abziehen"
      read F Q <<<"$(gebuehr "$FEE" $FEE_POOL1)"
      plan_add A "Pool-Anteile abziehen ($SHARES Anteile, 100 %)" "$PK" 0 \
        "etwa $PK KAS und $PG GHOST (Version 2, zum Tilgen)" "" 1 "$F" "$Q" \
        "pool-remove $SHARES $MINK $MING" \
        "mindestens $MINK KAS und $MING GHOST (Toleranz $(pct $TOL_ABZUG_BPS) %) – kommt weniger zurück, sendet ghostctl nicht"
    else
      echo "\nSchritt 1/6: deine $SHARES Pool-Anteile abziehen (mindestens $MINK KAS und $MING GHOST zurück)"
      txa "pool-remove $SHARES $MINK $MING" "Pool-Anteile abziehen ($SHARES Anteile, 100 %)" \
        "Pool-Anteile abgezogen" "Abziehen fehlgeschlagen (erneuter Doppelklick setzt fort)" pool-remove --key "$OWNER" --percent 100 --min-kas "$MINK" --min-ghost "$MING"
      [ $AM = dry ] && echo "  (Probelauf: die folgenden Schritte rechnen ohne die Rückflüsse aus dem Pool)"
    fi
    [ $AM = plan ] || PG_S=0
  else
    [ $AM = plan ] || echo "\nSchritt 1/6: keine eigenen Pool-Anteile – übersprungen"
  fi

  # 2./3. Vaults: kleinste Schuld zuerst tilgen und schließen, Rest herausnehmen.
  # Über die Covenant-ID, denn nach dem Schließen rücken die Vault-Nummern nach.
  # Gesendet wird trotzdem über die Nummer – vault_vor/vault_nach prüfen sie.
  # Grenzen und Beträge in Sompi (ganze Zahlen, Audit 13 A13-umzug-7).
  HAVE0=$(MYKEY "$V2" "$SA" ghost) || fail "Eigener Schlüssel nicht lesbar"
  HAVE0_S=$(sompi "$HAVE0")
  ST=$("$V2" --network "$NET" --state "$SA" status --json 2>/dev/null) || fail "Status nicht abrufbar"
  CIDS=$(echo "$ST" | py '
import json,sys
d=json.load(sys.stdin)
vs=[v for v in d["vaults"] if v["owner"]==sys.argv[1] and not v.get("stale")]
print("\n".join(v["covenantId"] for v in sorted(vs, key=lambda v: v["debtGhost"])))' "$ME")
  for CID in ${(f)CIDS}; do
    ST=$("$V2" --network "$NET" --state "$SA" status --json 2>/dev/null) || fail "Status nicht abrufbar"
    read VI DEBT_S COLL_S <<<$(echo "$ST" | py "$SU"'
import json,sys
v=next((v for v in json.load(sys.stdin)["vaults"] if v["covenantId"]==sys.argv[1]), None)
# ohne Backslashes: v[\"index\"] in einem f-String ist in Python ein Syntaxfehler, und
# der Schritt übersprang dann stillschweigend jeden Vault
print("%d %d %d" % (v["index"], su(v["debtGhost"]), su(v["collateralKas"])) if v else "")' "$CID")
    [ -n "$VI" ] || continue
    DEBT=$(ks $DEBT_S); COLL=$(ks $COLL_S)
    HAVE=$(MYKEY "$V2" "$SA" ghost) || fail "Eigener Schlüssel nicht lesbar"
    HAVE_S=$(sompi "$HAVE")
    # Probelauf und Plan tilgen nicht wirklich: die für vorige Vaults verplanten GHOST
    # abziehen, sonst zeigte die Vorschau jeden Vault als getilgt (Audit 12 A12-17).
    # Der Plan zählt die GHOST aus dem Pool (Schritt 1) dazu.
    [ $AM = echt ] || HAVE_S=$(( HAVE_S + PG_S - USED_S ))
    (( HAVE_S < 0 )) && HAVE_S=0
    HAVE=$(ks $HAVE_S)
    PAY_S=$(( DEBT_S < HAVE_S ? DEBT_S : HAVE_S ))
    PAY=$(ks $PAY_S)
    [ $AM = plan ] || echo "\nSchritt 2/6: Vault $VI – Schuld $DEBT GHOST, Sicherheit $COLL KAS, eigene GHOST $HAVE$( [ $AM = dry ] && (( USED_S )) && echo " (nach den vorigen Tilgungen)")"
    if (( PAY_S > 0 )); then
      VOLL=0
      (( PAY_S >= DEBT_S )) && VOLL=1
      # Immer mit --ghost: der Betrag ist gedeckelt, auch wenn die Nummer doch
      # einen anderen Vault träfe (A13-umzug-1)
      if [ $AM = plan ]; then
        # gebaut wird, was die GHOST von jetzt decken; was erst der Pool bringt, ist geschätzt
        FEE=""
        HINW="$H_ZUS"
        if (( USED_S + PAY_S <= HAVE0_S )); then
          : >"$PROBELOG"   # nur die Meldung dieser Probe zählt
          if ! FEE=$(probe2 repay --key "$OWNER" --vault "$VI" --ghost "$PAY"); then
            braucht_zusammenfuehren || probe_fail "Vault $VI tilgen"
            FEE=""
            HINW="$H_ZUS_PLAN"
          fi
        fi
        read F Q <<<"$(gebuehr "$FEE" $FEE_VAULT)"
        plan_add A "Vault $VI: $PAY von $DEBT GHOST Schuld tilgen (mit GHOST der Version 2)" 0 0 "" "" 1 "$F" "$Q" "repay $CID $PAY $VOLL" "$HINW"
      elif [ $AM = dry ] && : >"$PROBELOG" && ! probe2 repay --key "$OWNER" --vault "$VI" --ghost "$PAY" >/dev/null && braucht_zusammenfuehren; then
        echo "→ Tilgen: vorher werden deine GHOST zusammengeführt (1–2 Selbstüberweisungen) – das lässt sich nicht vorab bauen; im echten Lauf geschieht es automatisch – Probelauf, nicht gesendet"
      else
        VC=(repay "$CID" "$VI" "$PAY_S")
        txa "repay $CID $PAY $VOLL" "Vault $VI: $PAY von $DEBT GHOST Schuld tilgen" \
          "Vault $VI $( (( VOLL )) || echo "teilweise ")getilgt ($PAY GHOST)" "Tilgen fehlgeschlagen (erneuter Doppelklick setzt fort)" repay --key "$OWNER" --vault "$VI" --ghost "$PAY"
      fi
      DEBT_S=$(( DEBT_S - PAY_S )); DEBT=$(ks $DEBT_S)
      [ $AM = echt ] || USED_S=$(( USED_S + PAY_S ))
    fi
    if (( DEBT_S <= 0 )); then
      case $AM in
        plan) plan_add A "Vault $VI schließen" "$COLL" 0 "$COLL KAS Sicherheit" "" 1 $FEE_VAULT geschätzt "close $CID $COLL" ;;
        dry)  echo "  Vault $VI ist schuldenfrei – schließen"
              echo "  (Probelauf: Schließen wird erst nach dem echten Tilgen gebaut)" ;;
        echt) echo "  Vault $VI ist schuldenfrei – schließen"
              VC=(close "$CID" "$VI" 0)
              txa "close $CID $COLL" "Vault $VI schließen, zurück $COLL KAS" \
                "Vault $VI geschlossen" "Schließen fehlgeschlagen (erneuter Doppelklick setzt fort)" close --key "$OWNER" --vault "$VI" ;;
      esac
    else
      KAS_USD=$(echo "$ST" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])')
      # 220 % der Restschuld in KAS, auf 0,01 KAS aufgerundet, mindestens 0,3 KAS –
      # exakt mit Brüchen (Kurs als Dezimalzahl aus dem Status), Ergebnis in Sompi
      KEEP_S=$(py 'import sys
from fractions import Fraction
from decimal import Decimal
d, p = int(sys.argv[1]), Fraction(Decimal(sys.argv[2]))
c = -((-Fraction(d, 100000000) * Fraction(22, 10) / p * 100) // 1)
print(max(30000000, int(c) * 1000000))' "$DEBT_S" "$KAS_USD")
      KEEP=$(py 'import sys; s=int(sys.argv[1]); print("%d.%02d" % (s // 100000000, s % 100000000 // 1000000))' "$KEEP_S")
      [ $AM = plan ] || echo "  Schritt 3/6: Restschuld $DEBT GHOST bleibt – Sicherheit von $COLL auf $KEEP KAS senken"
      if (( KEEP_S < COLL_S )); then
        case $AM in
          plan) plan_add A "Vault $VI: Sicherheit von $COLL auf $KEEP KAS senken (Restschuld $DEBT GHOST bleibt)" "$(ks $(( COLL_S - KEEP_S )))" 0 \
                  "$(ks $(( COLL_S - KEEP_S ))) KAS Sicherheit" "$KEEP KAS im Rest-Vault der Version 2 (Restschuld $DEBT GHOST)" 1 $FEE_VAULT geschätzt \
                  "withdraw $CID $COLL $KEEP"
                REST_V2+=("Vault $VI mit $KEEP KAS Sicherheit und $DEBT GHOST Restschuld") ;;
          dry)  echo "  (Probelauf: Herausnehmen wird erst nach dem echten Tilgen gebaut)" ;;
          echt) VC=(withdraw "$CID" "$VI" "$KEEP_S")
                txa "withdraw $CID $COLL $KEEP" "Vault $VI: Sicherheit von $COLL auf $KEEP KAS senken (Restschuld $DEBT GHOST bleibt)" \
                  "Vault $VI: Sicherheit auf $KEEP KAS gesenkt" "Herausnehmen fehlgeschlagen (erneuter Doppelklick setzt fort)" withdraw --key "$OWNER" --vault "$VI" --keep "$KEEP" ;;
        esac
      elif [ $AM = plan ]; then
        REST_V2+=("Vault $VI mit $COLL KAS Sicherheit und $DEBT GHOST Restschuld")
      fi
    fi
  done
  if [ $AM = plan ]; then
    G2_REST=$(ks $(( HAVE0_S + PG_S - USED_S > 0 ? HAVE0_S + PG_S - USED_S : 0 )))
  elif [ $AM = echt ] && (( AI <= NA )); then
    # Geplante Schritte, die nach dem jetzigen Stand entfallen: auch das ist eine
    # Abweichung. Gesendet wird dadurch nichts anderes, aber vor Teil B wird gefragt.
    echo "\n⚠ Teil A weicht vom bestätigten Plan ab – nach dem jetzigen Stand entfällt:"
    for (( i = AI; i <= NA; i++ )); do echo "  – ${PL_T[i]}"; done
    a_neu_planen
    (( AI <= NA )) && fail "Teil A ist nach dem jetzigen Stand noch nicht fertig (siehe oben) – nichts weiter gesendet. Ein erneuter Doppelklick zeigt den Plan und fragt erneut."
    if (( EINZELN )); then
      echo "\nEINZELN=1: Jede Transaktion fragt gleich einzeln nach j."
    else
      frage "Ohne diese Schritte mit Teil B weitermachen?" || fail "Teil B nicht begonnen. Ein erneuter Doppelklick zeigt den Plan aus dem jetzigen Stand und fragt erneut."
    fi
  fi
  if [ $AM != plan ]; then
    echo "\nTeil A fertig. Stand Version 2:"
    "$V2" --network "$NET" --state "$SA" status 2>/dev/null | grep -E "^Vault|^Tauschpool" || true
  fi
}

# ---------------------------------------------------------- Plan Teil B ----
# plan_b <KAS vor Teil B> <GHOST der Version 3 vor Teil B>: rechnet Teil B wie
# der echte Lauf und hängt die Schritte an den Plan. Setzt PB_SIG (Schritte und
# feste Beträge), PB_POOLKAS (KAS für den Pool, leer = Kurs unbekannt), PB_KAS
# und PB_G (Stand am Ende) und PB_WARN (was so voraussichtlich nicht geht).
plan_b() {
  local k=$1 g=$2 dep=0 own=0 zero="" haspool="" usd="" st3 ov pk
  PB_SIG=""; PB_POOLKAS=""; PB_WARN=()
  [ -f "$S3" ] && state_version "$S3" && dep=1
  if (( dep )); then
    ME3=$(MYKEY ./ghostctl "$S3" xonly) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 3 – nichts eröffnet"
    [ -z "$ME" ] || [ "$ME" = "$ME3" ] || fail "Version 2 und 3 nennen für $OWNER verschiedene Schlüssel ($ME / $ME3) – nichts eröffnet, bitte von Hand prüfen"
    ov=$(own_vaults) || fail "Status nicht abrufbar (Nodes?) – später erneut"
    read own zero <<<"$ov"
    [ -f "$VAULTMARK" ] && [ "$own" = "0" ] && fail "Ein früherer Lauf hat schon einen Vault eröffnet ($(cat "$VAULTMARK" 2>/dev/null)), er ist aber nicht als eigener zu finden.
$VAULTMARK_MSG"
    st3=$(./ghostctl --network "$NET" --state "$S3" status --json 2>/dev/null) || fail "Status nicht abrufbar"
    haspool=$(echo "$st3" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")' 2>/dev/null)
    usd=$(echo "$st3" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])' 2>/dev/null)
  else
    [ -f "$COMMITTEE" ] || fail "Komitee-Datei $COMMITTEE fehlt – nichts gesendet"
    plan_add B "Orakel, Factory und GHOST (Version 3) anlegen" -30 0 "" "30 KAS dauerhaft (Orakel, Factory und GHOST-Wurzel je 10 KAS)" 3 $FEE_DEPLOY geschätzt
    k=$(fx 'a[0] - 30 - a[1]' "$k" $FEE_DEPLOY)
    PB_SIG+="deploy;"
    # Startkurs von Version 3 = Median der Börsen beim Deployment; sonst das Orakel von Version 2
    usd=$(./ghostctl --network "$NET" --json price 2>/dev/null | py 'import json,sys; print(float(json.load(sys.stdin)["median"]))' 2>/dev/null)
    [ -n "$usd" ] || usd=$V2_USD
  fi
  if [ "$own" = "0" ]; then
    plan_add B "Eigenen Vault mit $VAULT_KAS KAS eröffnen" "$(fx '-a[0] - 3' "$VAULT_KAS")" 0 "" \
      "$VAULT_KAS KAS Sicherheit (zurück beim Schließen), 3 KAS Minter-Zweig dauerhaft" 1 $FEE_VAULT geschätzt
    plan_add B "$MINT_GHOST GHOST prägen" -1 "$MINT_GHOST" "$MINT_GHOST GHOST an dich (Schuld $MINT_GHOST GHOST)" \
      "1 KAS im GHOST-Token-UTXO (kommt beim Tilgen oder Überweisen zurück)" 1 $FEE_VAULT geschätzt
    k=$(fx 'a[0] - a[1] - 3 - 1 - 2 * a[2]' "$k" "$VAULT_KAS" $FEE_VAULT)
    g=$(fx 'a[0] + a[1]' "$g" "$MINT_GHOST")
    PB_SIG+="vault $VAULT_KAS;mint $MINT_GHOST;"
  elif [ -n "$zero" ]; then
    plan_add B "Am eigenen Vault $zero (noch ohne Schuld) $MINT_GHOST GHOST prägen" -1 "$MINT_GHOST" "$MINT_GHOST GHOST an dich (Schuld $MINT_GHOST GHOST)" \
      "1 KAS im GHOST-Token-UTXO (kommt beim Tilgen oder Überweisen zurück)" 1 $FEE_VAULT geschätzt
    k=$(fx 'a[0] - 1 - a[1]' "$k" $FEE_VAULT)
    g=$(fx 'a[0] + a[1]' "$g" "$MINT_GHOST")
    PB_SIG+="mint $zero $MINT_GHOST;"
  fi
  if [ -z "$haspool" ]; then
    if [ -n "$usd" ]; then
      pk=$(py 'import sys; print(f"{float(sys.argv[1])/float(sys.argv[2]):.8f}")' "$POOL_GHOST" "$usd")
      PB_POOLKAS=$pk
      fq 'a[0] >= a[1]' "$g" "$POOL_GHOST" \
        || PB_WARN+=("Für den Pool fehlen voraussichtlich GHOST: nötig $POOL_GHOST, dann vorhanden $g – Schritt 6 bricht dann vor dem Senden ab.")
      fq 'a[0] >= a[1] + a[2]' "$k" "$pk" "$POOL_KAS_EXTRA" \
        || PB_WARN+=("Für den Pool fehlen voraussichtlich KAS: nötig etwa $(fx 'a[0] + a[1]' "$pk" "$POOL_KAS_EXTRA"), dann vorhanden etwa $(r2 "$k") – Schritt 6 bricht dann vor dem Senden ab.")
      plan_add B "Tauschpool mit Kursband anlegen: $pk KAS und $POOL_GHOST GHOST (Startkurs 1 USD, Kurs $usd USD je KAS)" "$(fx '-a[0] - 3' "$pk")" "-$POOL_GHOST" \
        "Anteile für deine Einlage (zurück mit pool-remove)" "Mindestliquidität 1 KAS und GHOST im Wert von 1 KAS, dazu je 1 KAS in Reserve- und Anteils-UTXO – dauerhaft" 3 $FEE_POOL3 geschätzt "" \
        "der KAS-Betrag folgt dem Orakelkurs beim Senden: bis ± $(py 'import sys; print("%g" % (float(sys.argv[1]) * 100))' "$TOL_POOL") % Abweichung geht ohne erneute Frage hinaus, darüber wird erneut gefragt"$'\n'"$H_ZUS"
    else
      pk=0
      PB_WARN+=("Kurs unbekannt: Der Pool-Betrag in KAS steht erst nach dem Deployment fest. Vor dem Pool wird dann noch einmal gefragt.")
      plan_add B "Tauschpool mit Kursband anlegen: $POOL_GHOST GHOST und KAS zum Orakelkurs (Startkurs 1 USD)" -3 "-$POOL_GHOST" \
        "Anteile für deine Einlage (zurück mit pool-remove)" "Mindestliquidität 1 KAS und GHOST im Wert von 1 KAS, dazu je 1 KAS in Reserve- und Anteils-UTXO – dauerhaft" 3 $FEE_POOL3 geschätzt "" \
        "den KAS-Betrag zeigt das Skript vor dem Pool und fragt dann noch einmal"$'\n'"$H_ZUS"
    fi
    k=$(fx 'a[0] - a[1] - 3 - a[2]' "$k" "$pk" $FEE_POOL3)
    g=$(fx 'a[0] - a[1]' "$g" "$POOL_GHOST")
    PB_SIG+="pool $POOL_GHOST;"
  fi
  fq 'a[0] >= 0' "$k" || PB_WARN+=("Die KAS reichen voraussichtlich nicht: am Ende fehlen etwa $(r2 "$(fx '-a[0]' "$k")") KAS.")
  PB_KAS=$k; PB_G=$g
}
# Pool-Betrag wie bestätigt? pool_wie_geplant <jetzt> <bestätigt (leer = unbekannt)>
pool_wie_geplant() { [ -n "$2" ] && fq 'abs(a[0] - a[1]) <= a[2] * a[1]' "$1" "$2" "$TOL_POOL"; }

# ----------------------------------------------------------- Probelauf ----
if (( DRY_RUN )); then
  AM=dry
  [ -f "$S2" ] && teil_a
  echo "\n--- Teil B: Version 3 anlegen ---"
  no_agent
  [ -f "$COMMITTEE" ] || fail "Komitee-Datei $COMMITTEE fehlt"
  echo "Probelauf: Schritte 4–6 (Anlegen, Vault, Pool) laufen erst echt – eine Probe ohne Deployment ist nicht möglich."
  echo "Geplant: Deployment (30 KAS gebunden), eigener Vault mit $VAULT_KAS KAS, $MINT_GHOST GHOST prägen, Pool mit $POOL_GHOST GHOST."
  exit 0
fi

# ------------------------------------------------ Plan und eine Bestätigung ----
# Bis zur Antwort wird nichts gesendet und nichts umbenannt: Teil A wird auf dem
# Zustand von Version 2 an seinem jetzigen Ort geprobt, Teil B gerechnet.
echo "\nRechne den Plan (nichts wird gesendet) …"
AM=plan
(( V2_AT_S3 )) && SA=$S3
A_DA=0
[ -f "$SA" ] && A_DA=1
(( A_DA )) && teil_a
NA=${#PL_T}
KAS_NOW=$(MYKEY ./ghostctl "$S3" kas) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 3 – nichts gesendet"
[ -n "$KAS_NOW" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – nichts gesendet, später erneut"
G3_NOW=0
if [ -f "$S3" ] && state_version "$S3"; then
  G3_NOW=$(MYKEY ./ghostctl "$S3" ghost) || fail "Eigener Schlüssel nicht lesbar"
fi
KAS_B=$(fx 'a[0]' "$KAS_NOW")
(( NA )) && KAS_B=$(fx 'a[0] + a[1] - a[2]' "$KAS_NOW" "$(summe "${PL_K[@]}")" "$(summe "${PL_F[@]}")")
plan_b "$KAS_B" "$G3_NOW"
PLAN_SIG=$PB_SIG; PLAN_POOLKAS=$PB_POOLKAS; PLAN_KAS_B=$KAS_B

# Leerer Plan: hier enden. Ohne bestätigten Schritt läuft kein Teil mit --ja.
if (( ${#PL_T} == 0 )); then
  echo "\nNichts mehr zu senden: Alle Schritte sind erledigt oder werden übersprungen."
  ./ghostctl --network "$NET" --state "$S3" status 2>/dev/null
  read -k 1 "?Taste drücken zum Schließen …" || true
  exit 0
fi
echo "\n=== Plan: ${#PL_T} Schritte ==="
kopf_zeigen
plan_zeigen
endstand "$KAS_NOW" "$( (( A_DA )) && echo "$KAS_B")"
if (( EINZELN )); then
  echo "\nEINZELN=1: Jede Transaktion fragt gleich einzeln nach j."
else
  frage "Alles so ausführen?" || fail "Abgebrochen – nichts gesendet, nichts umbenannt."
fi
UNTERWEGS=1

# 0. jetzt erst umbenennen
if (( V2_AT_S3 )); then
  rename_v2 0 || fail "Umbenennen fehlgeschlagen"
  echo "Zustand von Version 2 liegt jetzt in $S2"
  journal_v2
fi
SA=$S2

# ---------------------------------------------------------------- Teil A ----
AM=echt
if (( A_DA )); then
  AI=1
  teil_a
  AI=0
fi

# ---------------------------------------------------------------- Teil B ----
echo "\n--- Teil B: Version 3 anlegen ---"
no_agent
[ -f "$COMMITTEE" ] || fail "Komitee-Datei $COMMITTEE fehlt"
# Nach Teil A haben sich die Guthaben geändert (Rückflüsse aus Pool und Vaults):
# Teil B aus dem echten Stand neu rechnen. Weicht er vom bestätigten Plan ab,
# anhalten und erneut fragen, statt still andere Beträge zu senden.
if (( A_DA )); then
  KAS_JETZT=$(MYKEY ./ghostctl "$S3" kas) || fail "Eigener Schlüssel nicht lesbar"
  [ -n "$KAS_JETZT" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – Teil B nicht begonnen, später erneut"
  G3_JETZT=0
  if [ -f "$S3" ] && state_version "$S3"; then
    G3_JETZT=$(MYKEY ./ghostctl "$S3" ghost) || fail "Eigener Schlüssel nicht lesbar"
  fi
  PL_BASIS=${#GESENDET}
  plan_leeren
  REST_V2=(); G2_REST=""
  plan_b "$KAS_JETZT" "$G3_JETZT"
  ABW=()
  [ "$PB_SIG" = "$PLAN_SIG" ] || ABW+=("Die Schritte von Teil B haben sich geändert.")
  fq 'a[0] >= a[1] - a[2]' "$KAS_JETZT" "$PLAN_KAS_B" "$TOL_KAS" \
    || ABW+=("KAS auf dem Schlüssel: geplant etwa $(r2 "$PLAN_KAS_B"), jetzt $(r2 "$KAS_JETZT").")
  if [ -n "$PB_POOLKAS$PLAN_POOLKAS" ] && ! pool_wie_geplant "$PB_POOLKAS" "$PLAN_POOLKAS"; then
    [ -n "$PB_POOLKAS" ] && [ -n "$PLAN_POOLKAS" ] && ABW+=("Pool: geplant $PLAN_POOLKAS KAS, jetzt $PB_POOLKAS KAS.")
  fi
  if (( ${#ABW} )); then
    echo "\n⚠ Vor Teil B weicht der Stand vom bestätigten Plan ab:"
    for w in "${ABW[@]}"; do echo "  $w"; done
    echo "\nTeil B aus dem jetzigen Stand:"
    plan_zeigen
    endstand "$KAS_JETZT" ""
    if (( EINZELN )); then
      echo "\nEINZELN=1: Jede Transaktion fragt gleich einzeln nach j."
    else
      frage "Teil B so ausführen?" || fail "Teil B nicht begonnen. Teil A ist erledigt; ein erneuter Doppelklick zeigt Teil B wieder und fragt erneut."
    fi
    PLAN_POOLKAS=$PB_POOLKAS
  fi
fi

if [ ! -f "$S3" ]; then
  echo "\nSchritt 4/6: Orakel, Factory und GHOST (Version 3) anlegen – 30 KAS bleiben dauerhaft gebunden"
  tx G3 "Orakel, Factory und GHOST (Version 3) angelegt" "Deployment fehlgeschlagen (erneuter Doppelklick setzt fort)" deploy --key "$OWNER" --committee "$COMMITTEE" --rate 0
else
  state_version "$S3" || fail "$S3 ist nicht Version 3"
  echo "\nSchritt 4/6: Version 3 ist schon angelegt – übersprungen"
fi

ME3=$(MYKEY ./ghostctl "$S3" xonly) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 3 – nichts eröffnet"
[ -z "$ME" ] || [ "$ME" = "$ME3" ] || fail "Version 2 und 3 nennen für $OWNER verschiedene Schlüssel ($ME / $ME3) – nichts eröffnet, bitte von Hand prüfen"
OV=$(own_vaults) || fail "Status nicht abrufbar (Nodes?) – später erneut"
read OWN ZERO <<<"$OV"
if [ -f "$VAULTMARK" ]; then
  [ "$OWN" = "0" ] && fail "Ein früherer Lauf hat schon einen Vault eröffnet ($(cat "$VAULTMARK" 2>/dev/null)), er ist aber nicht als eigener zu finden.
$VAULTMARK_MSG"
  rm -f "$VAULTMARK"
fi
if [ "$OWN" = "0" ]; then
  echo "\nSchritt 5/6: eigenen Vault mit $VAULT_KAS KAS eröffnen und $MINT_GHOST GHOST prägen"
  tx G3 "Vault mit $VAULT_KAS KAS eröffnet" "Vault eröffnen fehlgeschlagen (erneuter Doppelklick setzt fort)" open-vault --key "$OWNER" --kas "$VAULT_KAS"
  # angenommen: bis er als eigener gefunden ist, eröffnet kein weiterer Lauf einen Vault
  echo "Besitzer $ME3, $(date '+%d.%m.%Y %H:%M')" > "$VAULTMARK"
  OV=$(own_vaults) || fail "Status nicht abrufbar – erneuter Doppelklick prägt nach"
  read OWN ZERO <<<"$OV"
  [ -n "$ZERO" ] || fail "Neuer Vault nicht als eigener gefunden. Ein erneuter Doppelklick eröffnet keinen weiteren ($VAULTMARK); bitte auf der Seite „Vault“ prüfen."
  rm -f "$VAULTMARK"
  tx G3 "$MINT_GHOST GHOST geprägt (Vault $ZERO)" "Prägen fehlgeschlagen (erneuter Doppelklick prägt nach)" mint --key "$OWNER" --vault "$ZERO" --ghost "$MINT_GHOST"
elif [ -n "$ZERO" ]; then
  echo "\nSchritt 5/6: eigener Vault $ZERO hat noch keine Schuld – $MINT_GHOST GHOST prägen"
  tx G3 "$MINT_GHOST GHOST geprägt (Vault $ZERO)" "Prägen fehlgeschlagen (erneuter Doppelklick prägt nach)" mint --key "$OWNER" --vault "$ZERO" --ghost "$MINT_GHOST"
else
  echo "\nSchritt 5/6: es gibt schon einen eigenen Vault (mit Schuld oder gesperrt) – übersprungen"
fi

ST3=$(./ghostctl --network "$NET" --state "$S3" status --json 2>/dev/null) || fail "Status nicht abrufbar"
if [ -z "$(echo "$ST3" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")')" ]; then
  KAS_USD=$(echo "$ST3" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])')
  POOL_KAS=$(py 'import sys; print(f"{float(sys.argv[1])/float(sys.argv[2]):.8f}")' "$POOL_GHOST" "$KAS_USD")
  # erst prüfen, dann senden: pool-open schickt die Genesis (1 KAS) vor der GHOST-Prüfung (A11-O-5)
  HAVE_G=$(MYKEY ./ghostctl "$S3" ghost) || fail "Eigener Schlüssel nicht lesbar"
  HAVE_K=$(MYKEY ./ghostctl "$S3" kas) || fail "Eigener Schlüssel nicht lesbar"
  [ -n "$HAVE_K" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – später erneut"
  # Grenzen in Sompi (ganze Zahlen, A13-umzug-7)
  NEED_K=$(ks $(( $(sompi "$POOL_KAS") + $(sompi "$POOL_KAS_EXTRA") )))
  (( $(sompi "$HAVE_G") >= $(sompi "$POOL_GHOST") )) \
    || fail "Für den Pool fehlen GHOST: nötig $POOL_GHOST, vorhanden ${HAVE_G:-0}. Erst prägen (Seite „Vault“), dann erneut doppelklicken."
  (( $(sompi "$HAVE_K") >= $(sompi "$NEED_K") )) \
    || fail "Für den Pool fehlen KAS: nötig etwa $NEED_K ($POOL_KAS für den Pool + $POOL_KAS_EXTRA für Minter, Token-UTXOs und Gebühren), vorhanden $HAVE_K."
  # Der Orakelkurs bestimmt die KAS: weicht der Betrag um mehr als 1 % vom
  # bestätigten ab (oder war der Kurs unbekannt), erst noch einmal fragen
  if ! (( EINZELN )) && ! pool_wie_geplant "$POOL_KAS" "$PLAN_POOLKAS"; then
    echo "\n⚠ Der Pool-Betrag weicht vom bestätigten Plan ab: jetzt $POOL_KAS KAS statt ${PLAN_POOLKAS:-unbekannt} (Orakel $KAS_USD USD je KAS)."
    frage "Pool mit $POOL_KAS KAS und $POOL_GHOST GHOST anlegen?" || fail "Pool nicht angelegt – erneuter Doppelklick setzt fort (oder auf der Seite „Tauschen“)"
  fi
  echo "\nSchritt 6/6: Tauschpool mit Kursband – $POOL_KAS KAS und $POOL_GHOST GHOST (Startkurs 1 USD, Orakel $KAS_USD USD je KAS)"
  tx G3 "Tauschpool mit $POOL_KAS KAS und $POOL_GHOST GHOST angelegt" "Pool anlegen fehlgeschlagen – erneuter Doppelklick setzt fort (oder auf der Seite „Tauschen“)" pool-open --key "$OWNER" --kas "$POOL_KAS" --ghost "$POOL_GHOST"
else
  echo "\nSchritt 6/6: Tauschpool existiert schon – übersprungen"
fi

echo "\n=== Fertig: Version 3 läuft ==="
./ghostctl --network "$NET" --state "$S3" status 2>/dev/null
./ghostctl --network "$NET" --state "$S3" balance --key "$OWNER" 2>/dev/null
echo "\nJetzt den GHOST-Agenten neu starten (Doppelklick auf „GHOST-Agent starten“): er hält Orakel und Zins nach."
read -k 1 "?Taste drücken zum Schließen …" || true
