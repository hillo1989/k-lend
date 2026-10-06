#!/bin/zsh
# Doppelklick: Umzug von GHOST Version 3 auf Version 4 (Mainnet).
#
# Vorlage: GHOST-Umzug-v3.command (Umzug v2 → v3), Aufbau und Sicherheitsregeln
# übernommen; die Audit-Verweise (A11-…, A12-…, A13-…) nennen den Grund je Regel.
#
# Vorher: GHOST-Agent und oracle-feed beenden (das Skript prüft das und bricht
# sonst ab – ein laufender Agent der Version 3 oder 4 würde die neue
# Zustandsdatei überschreiben, Audit 11 A11-O-3).
#
# Versionen der Zustandsdatei: Version 4 hat den Schlüssel „register“ auf
# oberster Ebene, Version 3 hat vault_params.treasury und kein „register“.
# Alles andere (nicht lesbar, Version 1/2, unbekannt) führt zum Abbruch.
#
# Teil A – Version 3 abbauen (Programm bin/ghostctl-v3, Zustand deployments/mainnet-v3.json)
#   0. Zustand der Version 3 beiseitelegen: mainnet.json → mainnet-v3.json, dazu
#      Journal (mainnet.pending.json), Deploy-Fortschritt (mainnet.deploy.json),
#      Zinsdatei (mainnet-zins.json → mainnet-v3-zins.json) und die Sperren
#      (mainnet.lock, mainnet-zins.lock). Nur umbenennen, nie löschen. Die
#      Zinsdatei muss mit: ghostctl leitet ihren Namen aus der Zustandsdatei ab
#      (<name>-zins.json), sonst fände Version 4 die Messungen von Version 3.
#      Zeigt das Journal nach einem unterbrochenen Umbenennen noch auf
#      mainnet.json, wird es vor jedem Aufruf von Version 3 auf mainnet-v3.json
#      gerichtet.
#   1. eigene Anteile aus dem Tauschpool abziehen (100 %), mit --min-kas/--min-ghost
#      = Rückfluss laut bestätigtem Plan minus 1 % (Audit 13 A13-umzug-2)
#   2. eigene Vaults mit den eigenen GHOST tilgen, kleinste Schuld zuerst;
#      schuldenfreie Vaults schließen (KAS kommen zurück). Tilgen immer mit
#      --ghost; vor jedem Senden an einen Vault prüft das Skript, dass die Nummer
#      noch die bestätigte Covenant-ID trägt, danach, dass sich genau dieser Vault
#      wie erwartet geändert hat – sonst hält es an (Audit 13 A13-umzug-1)
#   3. bleibt Restschuld (GHOST aus der Pool-Mindestliquidität sind für immer
#      gebunden), wird die Sicherheit bis auf 200 % + 10 % Puffer herausgenommen.
#      Version 3 rechnet Zins: Die 200 % gelten für Schuld plus aufgelaufenen
#      Zins (ops::withdraw), also rechnet das Skript mit beidem. Beim Schließen
#      geht der Zins in KAS an die Zinskasse; zurück kommt die Sicherheit minus Zins.
# Teil B – Version 4 anlegen (./ghostctl, Zustand deployments/mainnet.json)
#   4. Unterzeichner-Datei $KEYS-signer.json (im Normalfall keys/mainnet-signer.json):
#      fehlt sie, wird sie mit `./ghostctl committee-keygen … --count 1` angelegt
#      (ein eigener Schritt im Plan, sendet nichts). Dann Register, Orakel,
#      Factory und GHOST v4 anlegen: deploy --key <Besitzer> --committee <Unterzeichner>,
#      ohne --probe (Fristen 14 Tage / 2 h / 30 Tage, 1 Unterzeichner, Zins an die
#      eigene Adresse – das setzt ghostctl selbst). Fünf Transaktionen; gebunden
#      bleiben GEBUNDEN_KAS (4 Covenants je 1 KAS, ops::CovValues::small).
#   5. eigenen Vault mit VAULT_KAS KAS eröffnen und MINT_GHOST GHOST prägen
#      (gibt es schon einen eigenen Vault ohne Schuld, wird nur geprägt; gibt es
#      einen mit Schuld oder einen gesperrten, wird nichts eröffnet). Eigen heißt:
#      Besitzer ist der x-only-Schlüssel genau der Besitzer-Datei ($KEYS-owner.json,
#      im Normalfall keys/mainnet-owner.json).
#   6. Tauschpool mit Kursband (1 USD ± 3 %) anlegen: POOL_GHOST GHOST und KAS zum
#      Orakelkurs – nur, wenn der Schlüssel genug GHOST und KAS hat
#
# Audit 16 (Prüfung des Umzugs v3 → v4):
#   M-1/N-6 Herausnehmen und Schließen in Version 3 nur, wenn mindestens
#           MIN_AUSGANG (0,5 KAS) zurückkommt; Herausnehmen ohne Tilgen davor
#           wird im Plan mit --dry-run gebaut
#   M-2     Rückfluss aus dem Pool baut ghostctl v3 (--dry-run --json, zwei
#           größte Anteils-UTXOs); bei mehr als zwei UTXOs Halt mit Erklärung
#   M-3     Endstand nennt, was in Version 3 für immer bleibt
#   M-4     halb angelegter Pool (pool-open nach Schritt 1 oder 2 abgebrochen)
#           wird fortgesetzt; „Fertig“ nur mit fertigem Pool
#   N-2     vor der Frage: reichen VAULT_KAS beim Kurs für MINT_GHOST?
#   N-3     eingefrorenes oder bald einfrierendes Orakel von Version 4: erst
#           oracle-update mit der Unterzeichner-Datei
#   N-5     gesperrte eigene Vaults von Version 3 werden genannt
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
# EINZELN=1 ./GHOST-Umzug-v4.command: wie früher, jede Transaktion fragt einzeln (j).
# Zwei Umzüge gleichzeitig verhindert eine Sperre (deployments/.umzug-v4.lock).
# Probelauf ohne Senden: DRY=1 ./GHOST-Umzug-v4.command (nur DRY=1 ist ein Probelauf)

cd "$(dirname "$0")" || exit 1
NET=mainnet
KEYS=${KEYS:-keys/$NET}
OWNER="$KEYS-owner.json"
# Unterzeichner des Orakels von Version 4 (die 5er-Komitee-Datei von Version 3
# wird für den Umzug nicht gebraucht und bleibt unberührt)
SIGNER="$KEYS-signer.json"
S3=deployments/$NET-v3.json
S4=deployments/$NET.json
V3=bin/ghostctl-v3
# KAS, die das Deployment von Version 4 dauerhaft bindet: Register, Orakel,
# Factory und GHOST-Wurzel je 1 KAS (protocol/src/ops.rs, CovValues::small)
GEBUNDEN_KAS=4
# Merker „Vault eröffnet, aber noch nicht als eigener gefunden“: solange er liegt,
# eröffnet kein weiterer Lauf einen Vault (Audit 12 A12-4)
VAULTMARK=deployments/.umzug-v4-vault.lock
VAULT_KAS=${VAULT_KAS:-50}
MINT_GHOST=${MINT_GHOST:-0.5}
POOL_GHOST=${POOL_GHOST:-0.25}
# KAS über dem Pool-Betrag hinaus: Anteils-Minter und Token-UTXOs (je 1 KAS) und Gebühren
POOL_KAS_EXTRA=4
LOCKDIR=deployments/.umzug-v4.lock

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
# 0 = Version 4 („register“ auf oberster Ebene), 1 = Version 3
# (vault_params.treasury, kein „register“), 2 = nicht lesbar oder unbekannt
# (Version 1/2, leer, anderes Format). Wie old_version in ghostctl v4. Die
# Vorlage zählte alles ohne treasury als die alte Version und hätte so auch
# eine fremde Datei beiseitegelegt und abgebaut; hier nur noch genau Version 3.
state_version() { py '
import json,sys
try:
    d = json.load(open(sys.argv[1]))
    assert isinstance(d, dict)
except Exception:
    sys.exit(2)
if "register" in d:
    sys.exit(0)
vp = d.get("vault_params")
sys.exit(1 if isinstance(vp, dict) and "treasury" in vp else 2)' "$1" 2>/dev/null; }

# Meldung, wenn die Zinsdatei von Version 3 noch dort liegt, wo das Deployment
# die neue anlegt (Audit 16 N-4: gibt es das Ziel schon, nicht „dorthin legen“ raten)
zins_meldung() {
  if [ -e "${S3:r}-zins.json" ]; then
    echo "${S4:r}-zins.json und ${S3:r}-zins.json gibt es beide ($(datei_zeit "${S4:r}-zins.json") bzw. $(datei_zeit "${S3:r}-zins.json")), $S4 aber nicht. Das Deployment würde ${S4:r}-zins.json überschreiben. Bitte von Hand klären, welche zu Version 3 gehört (die andere beiseitelegen, nicht löschen) – nichts gesendet."
  else
    echo "${S4:r}-zins.json liegt noch da (Zinsdatei von Version 3?), $S4 aber nicht – das Deployment würde sie überschreiben. Bitte von Hand nach ${S3:r}-zins.json legen; nichts gesendet."
  fi
}
# Zeitpunkt der letzten Änderung einer Datei, für Meldungen
datei_zeit() { python3 -c 'import os,sys,time; print("geändert " + time.strftime("%d.%m.%Y %H:%M", time.localtime(os.path.getmtime(sys.argv[1]))))' "$1" 2>/dev/null; }
# Läuft noch ein GHOST-Agent oder oracle-feed (Version 3 oder 4)? Dann abbrechen:
# einer von Version 3 würde nach dem Deployment die neue Zustandsdatei im alten
# Format zurückschreiben (A11-O-3), einer von Version 4 liefe gegen den Abbau.
# Das Muster trifft ./ghostctl wie bin/ghostctl-v3.
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

echo "=== Umzug GHOST Version 3 → Version 4 ($NET)$( (( DRY_RUN )) && echo ' – PROBELAUF, nichts wird gesendet') ==="
[ -f "$OWNER" ] || fail "Schlüsseldatei $OWNER fehlt"
[ -x "$V3" ] || fail "$V3 fehlt (Programm für den Abbau von Version 3)"

# Sperre gegen einen zweiten, gleichzeitig gestarteten Umzug (A11-O-6). mkdir ist
# atomar; der Ordner wird beim Beenden (auch Ctrl+C, Fenster schließen) entfernt.
mkdir -p deployments
if ! mkdir "$LOCKDIR" 2>/dev/null; then
  OTHER=$(cat "$LOCKDIR/pid" 2>/dev/null)
  fail "Ein anderer Umzug läuft schon (PID ${OTHER:-unbekannt}, Sperre $LOCKDIR).
Läuft sicher keiner mehr (z. B. nach einem Absturz), den Ordner $LOCKDIR entfernen und neu starten."
fi
echo $$ > "$LOCKDIR/pid"
trap 'rm -f "$LOCKDIR/pid" "$LOCKDIR/probe.log" "$LOCKDIR/plan-a.zsh" "$LOCKDIR/abzug.err"; rmdir "$LOCKDIR" 2>/dev/null' EXIT
# Audit 16 N-1: vor dem trap definiert (sonst meldete ein früher Ctrl+C nur
# „command not found“, und das Skript liefe weiter).
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
trap sig_ende INT TERM HUP

no_agent

# 0. Zustandsdatei von Version 3 beiseitelegen (wird nur umbenannt, nicht gelöscht).
# Im echten Lauf wird vor der Bestätigung nur geprüft, ob das Umbenennen geht
# (Journal-Ziel, vorhandene Ziele); umbenannt wird erst nach „j“. Bis dahin
# rechnet der Plan mit Version 3 an ihrem alten Ort.
# Unter der Sperre der Zustandsdatei (flock wie ghostctl), damit kein
# ghostctl-Aufruf (z. B. die Seite) dazwischen ein Journal klärt. Das Journal
# zeigt auf die Datei, die es bei Annahme schreibt: Das Ziel wird auf den neuen
# Namen umgeschrieben, sonst schriebe Version 4 den v3-Inhalt nach mainnet.json (A11-O-4).
# Die Zinsdatei (mainnet-zins.json) wird vor der Zustandsdatei umbenannt und
# ebenfalls unter ihrer Sperre (mainnet-zins.lock, wie rate::update): Bricht das
# Umbenennen ab, liegt die Zustandsdatei noch als mainnet.json, und der nächste
# Lauf holt den Rest nach – nie bleibt die Zinsdatei von Version 3 dort liegen,
# wo Version 4 beim Deployment ihre neue anlegt.
# rename_v3 1 = nur prüfen, rename_v3 0 = umbenennen
rename_v3() { py '
import fcntl, json, os, sys, time
s3, s2 = sys.argv[1], sys.argv[2]
r3, r2 = s3[:-len(".json")], s2[:-len(".json")]
pairs = [(r3 + ".pending.json", r2 + ".pending.json"), (r3 + ".deploy.json", r2 + ".deploy.json"),
         (r3 + "-zins.json", r2 + "-zins.json"), (s3, s2)]
def sperre(name):
    lock = open(name, "a")
    t0 = time.time()
    while True:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return lock
        except BlockingIOError:
            if time.time() - t0 > 60:
                sys.exit("Sperre " + name + " ist seit 60 s belegt – läuft noch ein ghostctl? Nichts umbenannt.")
            time.sleep(0.3)
locks = [sperre(r3 + ".lock")]
# Sperre der Zinsdatei nur, wenn es eine Zinsdatei oder ihre Sperre gibt (sonst
# legte schon die Prüfung eine neue mainnet-zins.lock an)
zl = os.path.exists(r3 + "-zins.json") or os.path.exists(r3 + "-zins.lock")
if zl:
    locks.append(sperre(r3 + "-zins.lock"))
lockpairs = [(r3 + ".lock", r2 + ".lock")] + ([(r3 + "-zins.lock", r2 + "-zins.lock")] if zl else [])
todo = [(a, b) for a, b in pairs if os.path.exists(a)]
clash = [b for a, b in todo + lockpairs if os.path.exists(b)]
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
for a, b in lockpairs:
    os.rename(a, b)
    print("  umbenannt: " + a + " → " + b)
' "$S4" "$S3" "$V3" "$1"; }
V3_AT_S4=0
if [ -f "$S4" ]; then
  state_version "$S4"
  V=$?
  [ $V = 2 ] && fail "$S4 ist nicht lesbar oder weder Version 3 noch Version 4 – bitte von Hand prüfen (nichts verändert)"
  if [ $V = 1 ]; then
    [ -f "$S3" ] && fail "$S4 ist Version 3, aber $S3 gibt es schon – bitte von Hand klären: $S4 $(datei_zeit "$S4"), $S3 $(datei_zeit "$S3"). Die neuere ist meist die gültige; die andere beiseitelegen, nicht löschen."
    if (( DRY_RUN )); then
      echo "Probelauf: $S4 würde in $S3 umbenannt (samt Journal, Deploy-Fortschritt, Zinsdatei und Sperren, soweit vorhanden)"
      S3=$S4
    else
      rename_v3 1 || fail "Umbenennen nicht möglich – nichts verändert, nichts gesendet"
      V3_AT_S4=1
    fi
  fi
fi
# Liegt Version 3 schon beiseite, muss es auch Version 3 sein: Teil A baut sonst
# mit bin/ghostctl-v3 auf einer fremden Datei ab
if [ "$S3" != "$S4" ] && [ -f "$S3" ]; then
  state_version "$S3"
  [ $? = 1 ] || fail "$S3 ist nicht lesbar oder nicht Version 3 – bitte von Hand prüfen (nichts verändert)"
fi

# Wurde das Umbenennen unterbrochen (Fenster zu, Absturz), liegt das Journal schon
# als mainnet-v3.pending.json, zeigt aber noch auf mainnet.json. Der nächste Aufruf
# von Version 3 schriebe den v3-Stand bei Annahme dorthin, wo Version 4 hinkommt
# (Audit 12 A12-17). Deshalb vor jedem Aufruf von Version 3 auf $S3 prüfen und
# umrichten: gleich hier, oder – liegt Version 3 noch als $S4 (V3_AT_S4=1) – erst
# nach dem Umbenennen (vorher arbeitet der Plan auf $S4, und die Sperre
# mainnet-v3.lock darf vor dem Umbenennen nicht entstehen).
journal_v3() {
  [ "$S3" != "$S4" ] && [ -f "${S3:r}.pending.json" ] || return 0
  py '
import fcntl, json, os, sys, time
s3, s2, dry = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
r2 = s2[:-len(".json")]
pend = r2 + ".pending.json"
if not dry:
    # unter der Sperre der Zustandsdatei von Version 3 (flock wie ghostctl)
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
    sys.exit("Journal " + pend + " zeigt nach einem unterbrochenen Umbenennen noch auf " + tgt + ". Der echte Lauf richtet es auf " + new + "; der Probelauf bricht hier ab, damit kein Aufruf den Stand von Version 3 nach " + tgt + " schreibt.")
j["target"] = new
tmp = r2 + ".pending.tmp"
with open(tmp, "w") as f:
    json.dump(j, f, indent=2)
os.rename(tmp, pend)
print("  Journal " + pend + " zeigte noch auf " + tgt + " (Umbenennen war unterbrochen) – zeigt jetzt auf " + new)
' "$S4" "$S3" "$DRY_RUN" || fail "Journal von Version 3 nicht in Ordnung – nichts gesendet"
}
(( V3_AT_S4 )) || journal_v3

# Teil A arbeitet auf $SA: im Plan vor dem Umbenennen noch $S4, danach $S3
SA=$S3
G3() { "$V3" --network "$NET" --state "$SA" $JAFLAG $DRYFLAG "$@"; }
G4() { ./ghostctl --network "$NET" --state "$S4" $JAFLAG $DRYFLAG "$@"; }
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
own_vaults() { ./ghostctl --network "$NET" --state "$S4" status --json 2>/dev/null | py '
import json,sys
try:
    vs = json.load(sys.stdin)["vaults"]
    own = [v for v in vs if str(v["owner"]).lower() == sys.argv[1]]
    z = next((v["index"] for v in own if not v.get("stale") and v["debtGhost"] == 0), "")
except Exception:
    sys.exit(1)
print(len(own), z)' "$ME4"; }
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
# − ab, ohne Gebühr), GHOST der Version 4 (+/−), was zurückkommt, was gebunden
# bleibt, Anzahl Transaktionen, Gebühr in KAS und woher sie stammt („Probe“ =
# mit --dry-run gebaut, „geschätzt“ = Erfahrungswert aus MAINNET.md).
FEE_VAULT=0.05   # Vault-Aktion ≈ 0,04–0,05 KAS
FEE_POOL1=0.06   # eine Pool-Transaktion
FEE_POOL3=0.18   # Pool anlegen: drei Transaktionen
FEE_ORAKEL=0.01  # Orakel-Update ≈ 0,0065 KAS (MAINNET.md)
ORAKEL_MIN=30    # friert das Orakel in weniger Minuten ein, vorher aktualisieren
FEE_DEPLOY=0.05  # Deployment v4: fünf Transaktionen (Mainnet-Probe 05.10.2026: zusammen ≈ 0,031 KAS, docs/v4-entwurf.md)
TOL_KAS=0.5      # so viel weniger KAS vor Teil B gilt noch als planmäßig (Gebühren)
TOL_POOL=0.01    # Pool-Betrag: 1 % Abweichung (Orakelkurs) gilt noch als planmäßig
# Kleinster Betrag, den Herausnehmen oder Schließen in Version 3 zurückholen soll
# (Audit 16 M-1/N-6): Ausgänge unter etwa 0,02–0,025 KAS lassen sich wegen der
# Speichermasse gar nicht bauen (math.rs, txb.rs), und knapp darüber frisst die
# Gebühr (≈ 0,05 KAS) den Ertrag. Darunter bleibt der Rest im Vault – sonst
# hinge jede Fortsetzung an einem winzigen Herausnehmen, das nie gebaut wird.
MIN_AUSGANG=0.5
MIN_AUSGANG_S=50000000
TOL_ABZUG_BPS=100  # Pool-Anteile abziehen: so viel weniger (Basispunkte) als geplant darf zurückkommen, sonst sendet ghostctl nicht
PL_TEIL=(); PL_T=(); PL_K=(); PL_G=(); PL_Z=(); PL_B=(); PL_N=(); PL_F=(); PL_Q=(); PL_SA=(); PL_H=()
PL_BASIS=0       # so viele gesendete Schritte liegen vor dem ersten Eintrag des Plans
GESENDET=()      # in diesem Lauf gesendete Schritte
REST_V3=()       # was von Version 3 stehen bleibt
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
      [ "$t" = A ] && echo "Teil A – Version 3 abbauen (bin/ghostctl-v3):" || echo "Teil B – Version 4 anlegen (./ghostctl):"
    fi
    echo "  $i. ${PL_T[i]}"
    [ -n "${PL_Z[i]}" ] && echo "       zurück: ${PL_Z[i]}"
    [ -n "${PL_B[i]}" ] && echo "       gebunden: ${PL_B[i]}"
    if (( ${PL_N[i]} == 0 )); then f="keine (sendet nichts)"
    elif [ "${PL_Q[i]}" = Probe ]; then f="${PL_F[i]} KAS (mit --dry-run gebaut)"; else f="etwa ${PL_F[i]} KAS (geschätzt)"; fi
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
  adr=$(MYKEY ./ghostctl "$S4" address 2>/dev/null)
  xo=$(MYKEY ./ghostctl "$S4" xonly 2>/dev/null)
  [ -n "$xo" ] || xo=$ME
  echo "Netz:           $NET"
  echo "Schlüsseldatei: $OWNER (${OWNER:A})"
  echo "Adresse:        ${adr:-unbekannt}"
  echo "x-only:         ${xo:-unbekannt}"
  echo "Unterzeichner:  $SIGNER$( [ -f "$SIGNER" ] && echo " (vorhanden)" || echo " (fehlt – wird angelegt, falls Version 4 noch angelegt wird)")"
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
  echo "  GHOST der Version 4: am Ende $PB_G"
  [ -n "$G3_REST" ] && echo "  GHOST der Version 3: danach $G3_REST (gelten in Version 4 nicht)"
  # Was in Version 3 für immer bleibt (Audit 16 M-3): nur im Plan mit Teil A
  if [ -n "$2" ]; then
    echo "Version 3 bleibt für immer (nicht zurückholbar):"
    [ -n "$V3_POOL_REST" ] && echo "  – im Pool von Version 3: ${V3_POOL_REST% *} KAS und ${V3_POOL_REST#* } GHOST, darunter die Mindestliquidität (1 KAS und GHOST im Wert von 1 KAS)"
    echo "  – Covenants von Version 3: etwa 30 KAS (Orakel, Factory und GHOST-Wurzel je 10 KAS)"
    (( ${V3_VAULTS_N:-0} )) && echo "  – Minter-Zweige: je eigener Vault 3 KAS, auch nach dem Schließen – jetzt ${V3_VAULTS_N} eigene(r) Vault(s) = $(( V3_VAULTS_N * 3 )) KAS (frühere, schon geschlossene Vaults ebenso)"
    for w in "${REST_V3[@]}"; do echo "  – $w"; done
  else
    for w in "${REST_V3[@]}"; do echo "  Version 3 bleibt: $w"; done
  fi
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

# Senden: tx <G3|G4> <Text für „gesendet“> <Meldung bei Abbruch> <Befehl …>.
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


# Senden in Teil A: txa <Kennung> <Schritt, wie er jetzt ist> <Text für „gesendet“>
# <Meldung bei Abbruch> <Befehl …>. Tilgen, Schließen und Herausnehmen rechnet
# Teil A aus dem jeweils aktuellen Stand (eigene GHOST nach dem Pool, Orakel von
# Version 3). Deshalb vergleicht der echte Lauf vor jedem Senden die Kennung
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
  tx G3 "$@"
  (( ${#VC} )) && ! (( DRY_RUN )) && vault_nach
  VC=()
  [ $AM = echt ] && AI=$((AI + 1))
  true
}

# Vault-Prüfung um jedes Senden an einen Vault von Version 3 (Audit 13
# A13-umzug-1). bin/ghostctl-v3 kennt den Vault nur als Nummer (--vault), bestätigt
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
# v3_abdruck <Covenant-ID>: „<Nummer> <Schuld> <Sicherheit> <übrige eigene>“ aus
# einem frisch gelesenen Status, Schuld und Sicherheit in Sompi; die übrigen
# eigenen als „CID:Schuld/Sicherheit,…“. Fehlt der Vault, „- - - <übrige>“.
v3_abdruck() {
  "$V3" --network "$NET" --state "$SA" status --json 2>/dev/null | py "$SU"'
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
  ab=$(v3_abdruck "${VC[2]}") && [ -n "$ab" ] || fail "Status von Version 3 direkt vor dem Senden nicht lesbar – ${VC[1]} an Vault ${VC[3]} ist nicht gesendet. Später erneut doppelklicken."
  read VC_VI VC_D VC_C VC_REST <<<"$ab"
  [ "$VC_VI" = "${VC[3]}" ] || fail "Vault-Nummer verschoben: Nummer ${VC[3]} trägt nicht mehr den bestätigten Vault ${VC[2]} ($( [ "$VC_VI" = - ] && echo "den gibt es nicht mehr" || echo "er hat jetzt Nummer $VC_VI")). ${VC[1]} ist nicht gesendet. Ein erneuter Doppelklick rechnet mit den neuen Nummern und fragt erneut."
}
vault_nach() {
  local ab vi d c rest ok=1 soll
  ab=$(v3_abdruck "${VC[2]}") && [ -n "$ab" ] || fail "Status von Version 3 nach dem Senden nicht lesbar – ob ${VC[1]} den Vault ${VC[2]} wie bestätigt getroffen hat, ist offen. Nichts weiter gesendet; bitte auf der Seite prüfen, dann erneut doppelklicken."
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
  ( AM=plan; DRY_RUN=1; UNTERWEGS=0; AI=0; plan_leeren; REST_V3=(); G3_REST=""
    teil_a
    for a in TEIL T K G Z B N F Q SA H; do
      eval "print -r -- \"N_$a=(\${(@qq)PL_$a})\""
    done > "$f"
    echo "\nTeil A aus dem jetzigen Stand:"
    (( ${#PL_T} )) && plan_zeigen || echo "  (kein Schritt mehr)"
    for a in "${REST_V3[@]}"; do echo "  Version 3 bleibt: $a"; done
    true
  ) && [ -f "$f" ] || fail "Teil A ließ sich nicht neu rechnen – der Schritt ist nicht gesendet."
  eval "$(<"$f")"
  rm -f "$f"
  for a in TEIL T K G Z B N F Q SA H; do
    eval "PL_${a}=(\"\${(@)PL_${a}[1,AI-1]}\" \"\${(@)N_${a}}\" \"\${(@)PL_${a}[NA+1,-1]}\")"
  done
  NA=$(( AI - 1 + ${#N_T} ))
  kas=$(MYKEY ./ghostctl "$S4" kas) && [ -n "$kas" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – der Schritt ist nicht gesendet, später erneut"
  PLAN_KAS_B=$(fx 'a[0] + a[1] - a[2]' "$kas" "$(summe "${N_K[@]}")" "$(summe "${N_F[@]}")")
  echo "  KAS frei auf dem Schlüssel: jetzt $(r2 "$kas"), vor Teil B etwa $(r2 "$PLAN_KAS_B")"
}

# Probe eines Schritts von Version 3 (--dry-run --json): gibt die Gebühr aus
# (Summe der gebauten Transaktionen) oder nichts, wenn ghostctl keine nennt.
# Meldungen landen in $PROBELOG und werden nur bei einem Fehler gezeigt.
PROBELOG="$LOCKDIR/probe.log"
probe3() {
  local out
  # mit --json steht ein Fehler als {"ok":false,"error":…} auf stdout: ins Protokoll
  out=$("$V3" --network "$NET" --state "$SA" --dry-run --json "$@" 2>>"$PROBELOG") || { print -r -- "$out" >>"$PROBELOG"; return 1; }
  echo "$out" | py '
import json,sys
try:
    t = json.load(sys.stdin)["transactions"]
    print("%.4f" % sum(float(x["feeKas"]) for x in t) if t else "")
except Exception:
    print("")'
}
# Abzug aus dem Pool von ghostctl v3 bauen lassen (--dry-run --json, ohne
# Mindestbeträge): gibt „<KAS> <GHOST> <min KAS> <min GHOST> <Gebühr>“ aus, sonst
# den Grund und Rückgabe 1 (Audit 16 M-2). Sendet nichts.
abzug_probe() {
  local out err
  err="$LOCKDIR/abzug.err"
  out=$("$V3" --network "$NET" --state "$SA" --dry-run --json pool-remove --key "$OWNER" --percent 100 2>"$err")
  if grep -q "mehr als zwei UTXOs" "$err" 2>/dev/null; then
    echo "Deine Pool-Anteile von Version 3 liegen auf mehr als zwei UTXOs (nach mehreren pool-add).
ghostctl v3 zieht je Aufruf nur aus den zwei größten ab und hat keinen Befehl, sie
zusammenzuführen. Das Skript hält deshalb vor jedem Senden an – nichts gesendet,
nichts umbenannt. So geht es weiter: die übrigen Anteile in Schritten abziehen,
je Aufruf die zwei größten UTXOs (Seite „Tauschen“ oder
  $V3 --network $NET --state $SA pool-remove --key $OWNER --percent 100
so oft, bis höchstens zwei Anteils-UTXOs übrig sind), dann erneut doppelklicken."
    rm -f "$err"; return 1
  fi
  print -r -- "$out" | py "$SU"'
import json, sys
try:
    j = json.load(sys.stdin)
    assert j.get("ok")
    k, g, t = su(j["kas"]), su(j["ghost"]), int(sys.argv[1])
    fee = sum(float(x["feeKas"]) for x in j.get("transactions") or [])
except Exception:
    sys.exit(1)
print(ks(k), ks(g), ks(k * (10000 - t) // 10000), ks(g * (10000 - t) // 10000), "%.4f" % fee)' "$TOL_ABZUG_BPS" && { rm -f "$err"; return 0; }
  echo "Abzug aus dem Pool von Version 3 lässt sich nicht bauen – nichts gesendet:
$(print -r -- "$out" | tail -n 3)$(tail -n 3 "$err" 2>/dev/null)"
  rm -f "$err"; return 1
}
probe_fail() { fail "Probe (--dry-run) für „$1“ gescheitert – nichts gesendet:
$(tail -n 5 "$PROBELOG" 2>/dev/null)"; }

# ---------------------------------------------------------------- Teil A ----
# AM=dry: Probelauf DRY=1 (Ausgabe wie bisher). AM=plan: rechnet wie der
# Probelauf, baut mit --dry-run, trägt die Schritte in den Plan ein, sendet
# nichts und gibt nichts aus. AM=echt: sendet.
teil_a() {
  local ST CIDS CID VI DEBT COLL HAVE HAVE0 PAY USED=0 KAS_USD KEEP SHARES PK=0 PG=0 PRET="" FEE Q F VOLL i
  local DEBT_S COLL_S HAVE_S HAVE0_S PAY_S KEEP_S USED_S=0 PG_S=0 MINK MING INT_S ZINS_S ZURUECK GRUND
  [ $AM = plan ] || echo "\n--- Teil A: Version 3 abbauen ---"
  ST=$("$V3" --network "$NET" --state "$SA" status --json 2>/dev/null) || fail "Status von Version 3 nicht abrufbar (Nodes?) – später erneut"
  ME=$(MYKEY "$V3" "$SA" xonly) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 3"
  V3_USD=$(echo "$ST" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])' 2>/dev/null)

  # 1. Pool-Anteile
  SHARES=$(MYKEY "$V3" "$SA" lpShares) || fail "Eigener Schlüssel nicht lesbar"
  if [ -n "$(echo "$ST" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")')" ] && [ "${SHARES:-0}" != "0" ]; then
    # Rückfluss so, wie ghostctl v3 ihn baut (Audit 16 M-2): pool-remove zieht nur
    # aus den ZWEI GRÖSSTEN eigenen Anteils-UTXOs ab (pool::own(…, 2)), lpShares
    # in `keys` zählt aber alle. Deshalb rechnet das Skript nicht selbst, sondern
    # lässt ghostctl den Abzug ohne Mindestbeträge mit --dry-run --json bauen und
    # nimmt dessen Auszahlung („kas“/„ghost“, schon auf die 1-KAS-Mindestreserve
    # gekappt). Liegen die Anteile auf mehr als zwei UTXOs, hat v3 keinen Befehl
    # zum Zusammenführen: dann vor der Frage anhalten und erklären.
    # Mindestbeträge für --min-kas/--min-ghost: Auszahlung minus TOL_ABZUG_BPS. Sie
    # stehen in der Kennung (PL_SA): verschiebt sich der Pool bis zum Senden, wird
    # neu gerechnet und gefragt; danach sendet ghostctl nicht (Audit 13 A13-umzug-2).
    PRET=$(abzug_probe) || fail "$PRET"
    read PK PG MINK MING FEE <<<"$PRET"
    PG_S=$(sompi "$PG")
    if [ $AM = plan ]; then
      : >"$PROBELOG"
      FEE=$(probe3 pool-remove --key "$OWNER" --percent 100 --min-kas "$MINK" --min-ghost "$MING") || probe_fail "Pool-Anteile abziehen"
      read F Q <<<"$(gebuehr "$FEE" $FEE_POOL1)"
      V3_POOL_REST=$(echo "$ST" | py "$SU"'
import json,sys
p = json.load(sys.stdin).get("pool") or {}
print(ks(int(p["kasSompi"]) - su(sys.argv[1])), ks(int(p["ghostUnits"]) - su(sys.argv[2])))' "$PK" "$PG")
      plan_add A "Pool-Anteile abziehen ($SHARES Anteile, 100 %)" "$PK" 0 \
        "etwa $PK KAS und $PG GHOST (Version 3, zum Tilgen)" "im Pool von Version 3 bleiben ${V3_POOL_REST% *} KAS und ${V3_POOL_REST#* } GHOST, darunter die Mindestliquidität (1 KAS und GHOST im Wert von 1 KAS) – für immer" 1 "$F" "$Q" \
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
    [ $AM = plan ] && V3_POOL_REST=$(echo "$ST" | py "$SU"'
import json,sys
p = json.load(sys.stdin).get("pool")
print("%s %s" % (ks(int(p["kasSompi"])), ks(int(p["ghostUnits"]))) if p else "")')
  fi

  # 2./3. Vaults: kleinste Schuld zuerst tilgen und schließen, Rest herausnehmen.
  # Über die Covenant-ID, denn nach dem Schließen rücken die Vault-Nummern nach.
  # Gesendet wird trotzdem über die Nummer – vault_vor/vault_nach prüfen sie.
  # Grenzen und Beträge in Sompi (ganze Zahlen, Audit 13 A13-umzug-7).
  HAVE0=$(MYKEY "$V3" "$SA" ghost) || fail "Eigener Schlüssel nicht lesbar"
  HAVE0_S=$(sompi "$HAVE0")
  ST=$("$V3" --network "$NET" --state "$SA" status --json 2>/dev/null) || fail "Status nicht abrufbar"
  CIDS=$(echo "$ST" | py '
import json,sys
d=json.load(sys.stdin)
vs=[v for v in d["vaults"] if v["owner"]==sys.argv[1] and not v.get("stale")]
print("\n".join(v["covenantId"] for v in sorted(vs, key=lambda v: v["debtGhost"])))' "$ME")
  # Eigene gesperrte (stale) Vaults: von Dritten verändert, Stand unbekannt – Teil A
  # fasst sie nicht an, nennt sie aber (Audit 16 N-5). Dazu die Zahl aller eigenen
  # Vaults für die Minter-Zweige, die in Version 3 bleiben (Audit 16 M-3).
  read V3_VAULTS_N V3_GESPERRT_TXT <<<"$(echo "$ST" | py '
import json,sys
vs=[v for v in json.load(sys.stdin)["vaults"] if v["owner"]==sys.argv[1]]
st=["Vault %d (%s KAS, Schuld %s GHOST)" % (v["index"], v["collateralKas"], v["debtGhost"]) for v in vs if v.get("stale")]
print(len(vs), "; ".join(st))' "$ME")"
  if [ -n "$V3_GESPERRT_TXT" ]; then
    [ $AM = plan ] || echo "\n⚠ Gesperrt (von Dritten verändert, Stand unbekannt) und daher übersprungen: $V3_GESPERRT_TXT. Bitte auf der Seite „Vault“ prüfen (bin/ghostctl-v3 … sync)."
    [ $AM = plan ] && REST_V3+=("gesperrt und übersprungen: $V3_GESPERRT_TXT – erst nach einem Abgleich (bin/ghostctl-v3 … sync) wieder bedienbar")
  fi
  for CID in ${(f)CIDS}; do
    ST=$("$V3" --network "$NET" --state "$SA" status --json 2>/dev/null) || fail "Status nicht abrufbar"
    # INT_S: aufgelaufener Zins in USD (1e-8), Version 3 (status --json „interestUsd“)
    read VI DEBT_S COLL_S INT_S <<<$(echo "$ST" | py "$SU"'
import json,sys
v=next((v for v in json.load(sys.stdin)["vaults"] if v["covenantId"]==sys.argv[1]), None)
# ohne Backslashes: v[\"index\"] in einem f-String ist in Python ein Syntaxfehler, und
# der Schritt übersprang dann stillschweigend jeden Vault
print("%d %d %d %d" % (v["index"], su(v["debtGhost"]), su(v["collateralKas"]), su(v.get("interestUsd") or 0)) if v else "")' "$CID")
    [ -n "$VI" ] || continue
    DEBT=$(ks $DEBT_S); COLL=$(ks $COLL_S)
    KAS_USD=$(echo "$ST" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])')
    # Zins beim Schließen in KAS wie ops::close_fee: ⌈Zins/Kurs⌉, höchstens die
    # Sicherheit, unter 0,2 KAS erlassen; zurück kommt die Sicherheit minus Zins
    ZINS_S=$(py 'import sys
from fractions import Fraction
from decimal import Decimal
i, c, p = int(sys.argv[1]), int(sys.argv[3]), Fraction(Decimal(sys.argv[2]))
f = min(-((-Fraction(i) / p) // 1), c) if i > 0 else 0
print(int(f) if f >= 20000000 else 0)' "${INT_S:-0}" "$KAS_USD" "$COLL_S")
    ZURUECK=$(ks $(( COLL_S - ZINS_S )))
    HAVE=$(MYKEY "$V3" "$SA" ghost) || fail "Eigener Schlüssel nicht lesbar"
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
          if ! FEE=$(probe3 repay --key "$OWNER" --vault "$VI" --ghost "$PAY"); then
            braucht_zusammenfuehren || probe_fail "Vault $VI tilgen"
            FEE=""
            HINW="$H_ZUS_PLAN"
          fi
        fi
        read F Q <<<"$(gebuehr "$FEE" $FEE_VAULT)"
        plan_add A "Vault $VI: $PAY von $DEBT GHOST Schuld tilgen (mit GHOST der Version 3)" 0 0 "" "" 1 "$F" "$Q" "repay $CID $PAY $VOLL" "$HINW"
      elif [ $AM = dry ] && : >"$PROBELOG" && ! probe3 repay --key "$OWNER" --vault "$VI" --ghost "$PAY" >/dev/null && braucht_zusammenfuehren; then
        echo "→ Tilgen: vorher werden deine GHOST zusammengeführt (1–2 Selbstüberweisungen) – das lässt sich nicht vorab bauen; im echten Lauf geschieht es automatisch – Probelauf, nicht gesendet"
      else
        VC=(repay "$CID" "$VI" "$PAY_S")
        txa "repay $CID $PAY $VOLL" "Vault $VI: $PAY von $DEBT GHOST Schuld tilgen" \
          "Vault $VI $( (( VOLL )) || echo "teilweise ")getilgt ($PAY GHOST)" "Tilgen fehlgeschlagen (erneuter Doppelklick setzt fort)" repay --key "$OWNER" --vault "$VI" --ghost "$PAY"
      fi
      DEBT_S=$(( DEBT_S - PAY_S )); DEBT=$(ks $DEBT_S)
      [ $AM = echt ] || USED_S=$(( USED_S + PAY_S ))
    fi
    if (( DEBT_S <= 0 && COLL_S - ZINS_S < MIN_AUSGANG_S )); then
      # Schließen brächte unter MIN_AUSGANG zurück (Audit 16 N-6): bleibt stehen
      [ $AM = plan ] && REST_V3+=("Vault $VI ohne Schuld mit $COLL KAS – Schließen brächte nur $ZURUECK KAS zurück (unter $MIN_AUSGANG KAS lässt es sich kaum oder gar nicht bauen), bleibt deshalb stehen")
      [ $AM = plan ] || echo "  Vault $VI ist schuldenfrei, Schließen brächte nur $ZURUECK KAS zurück (unter $MIN_AUSGANG KAS) – bleibt stehen"
    elif (( DEBT_S <= 0 )); then
      case $AM in
        plan) plan_add A "Vault $VI schließen" "$ZURUECK" 0 "$ZURUECK KAS Sicherheit$( (( ZINS_S )) && echo " (nach $(ks $ZINS_S) KAS Zins an die Zinskasse)")" "" 1 $FEE_VAULT geschätzt "close $CID $COLL" ;;
        dry)  echo "  Vault $VI ist schuldenfrei – schließen"
              echo "  (Probelauf: Schließen wird erst nach dem echten Tilgen gebaut)" ;;
        echt) echo "  Vault $VI ist schuldenfrei – schließen"
              VC=(close "$CID" "$VI" 0)
              txa "close $CID $COLL" "Vault $VI schließen, zurück $ZURUECK KAS" \
                "Vault $VI geschlossen" "Schließen fehlgeschlagen (erneuter Doppelklick setzt fort)" close --key "$OWNER" --vault "$VI" ;;
      esac
    else
      # 220 % von Restschuld plus Zins in KAS, auf 0,01 KAS aufgerundet, mindestens
      # 0,3 KAS – exakt mit Brüchen (Kurs als Dezimalzahl aus dem Status), Ergebnis
      # in Sompi. Version 3 prüft die 200 % gegen Schuld + Zins (ops::withdraw);
      # mit der Schuld allein schlüge das Herausnehmen bei viel Zins fehl.
      KEEP_S=$(py 'import sys
from fractions import Fraction
from decimal import Decimal
d, p = int(sys.argv[1]) + int(sys.argv[3]), Fraction(Decimal(sys.argv[2]))
c = -((-Fraction(d, 100000000) * Fraction(22, 10) / p * 100) // 1)
print(max(30000000, int(c) * 1000000))' "$DEBT_S" "$KAS_USD" "${INT_S:-0}")
      KEEP=$(py 'import sys; s=int(sys.argv[1]); print("%d.%02d" % (s // 100000000, s % 100000000 // 1000000))' "$KEEP_S")
      [ $AM = plan ] || echo "  Schritt 3/6: Restschuld $DEBT GHOST$( (( INT_S )) && echo " (dazu $(ks $INT_S) USD Zins)") bleibt – Sicherheit von $COLL auf $KEEP KAS senken"
      # Grund, warum Schuld bleibt: die GHOST dazu fehlen – sie liegen (fast nur
      # noch) in der Mindestliquidität des Pools von Version 3 (Audit 16 M-3)
      GRUND="Grund: die GHOST zum Tilgen fehlen, sie liegen in der Mindestliquidität des Pools von Version 3. Der Vault zahlt weiter Zins und kann bei fallendem KAS-Kurs liquidiert werden – praktisch verloren"
      # Herausnehmen nur ab MIN_AUSGANG (Audit 16 M-1)
      if (( COLL_S - KEEP_S >= MIN_AUSGANG_S )); then
        case $AM in
          plan) # gebaut, wenn kein Tilgen davor nötig ist (sonst passt der Stand erst danach)
                FEE=""
                if (( PAY_S == 0 )); then
                  : >"$PROBELOG"
                  FEE=$(probe3 withdraw --key "$OWNER" --vault "$VI" --keep "$KEEP") || probe_fail "Vault $VI: Sicherheit senken"
                fi
                read F Q <<<"$(gebuehr "$FEE" $FEE_VAULT)"
                plan_add A "Vault $VI: Sicherheit von $COLL auf $KEEP KAS senken (Restschuld $DEBT GHOST bleibt)" "$(ks $(( COLL_S - KEEP_S )))" 0 \
                  "$(ks $(( COLL_S - KEEP_S ))) KAS Sicherheit" "$KEEP KAS im Rest-Vault der Version 3 (Restschuld $DEBT GHOST)" 1 "$F" "$Q" \
                  "withdraw $CID $COLL $KEEP"
                REST_V3+=("Vault $VI mit $KEEP KAS Sicherheit und $DEBT GHOST Restschuld$( (( INT_S )) && echo " (dazu $(ks $INT_S) USD Zins)"). $GRUND") ;;
          dry)  echo "  (Probelauf: Herausnehmen wird erst nach dem echten Tilgen gebaut)" ;;
          echt) VC=(withdraw "$CID" "$VI" "$KEEP_S")
                txa "withdraw $CID $COLL $KEEP" "Vault $VI: Sicherheit von $COLL auf $KEEP KAS senken (Restschuld $DEBT GHOST bleibt)" \
                  "Vault $VI: Sicherheit auf $KEEP KAS gesenkt" "Herausnehmen fehlgeschlagen (erneuter Doppelklick setzt fort)" withdraw --key "$OWNER" --vault "$VI" --keep "$KEEP" ;;
        esac
      else
        [ $AM = plan ] || echo "  Herausnehmen brächte weniger als $MIN_AUSGANG KAS – unterbleibt"
        [ $AM = plan ] && REST_V3+=("Vault $VI mit $COLL KAS Sicherheit und $DEBT GHOST Restschuld (Herausnehmen brächte unter $MIN_AUSGANG KAS). $GRUND")
      fi
    fi
  done
  if [ $AM = plan ]; then
    G3_REST=$(ks $(( HAVE0_S + PG_S - USED_S > 0 ? HAVE0_S + PG_S - USED_S : 0 )))
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
    echo "\nTeil A fertig. Stand Version 3:"
    "$V3" --network "$NET" --state "$SA" status 2>/dev/null | grep -E "^Vault|^Tauschpool" || true
  fi
}

# ------------------------------------------------------ Unterzeichner ----
# Deploy-Fortschritt von Version 4: nur, wenn mainnet.json noch fehlt (liegt dort
# noch Version 3, ist mainnet.deploy.json deren Rest und wird mit umbenannt).
fortschritt_v4() { [ ! -f "$S4" ] && [ -f "${S4:r}.deploy.json" ]; }
# Die Unterzeichner-Datei muss genau 1 Schlüssel enthalten (1 Unterzeichner) und
# als solche gelistet sein; gelistet wird ihr Ordner, verglichen wird der volle
# Pfad bzw. dieselbe Datei (wie MYKEY, Audit 12 A12-4). Rückgabe 1 mit Grund auf stdout.
signer_ok() { ./ghostctl --network "$NET" --state "$S4" --json keys --dir "$(dirname "$SIGNER")" 2>/dev/null | py '
import json, os, sys
name = sys.argv[1]
own = os.path.realpath(name)
def same(f):
    try:
        return os.path.realpath(f) == own or os.path.samefile(f, own)
    except (OSError, TypeError):
        return False
try:
    ks = [k for k in json.load(sys.stdin)["keys"] if same(k.get("file"))]
except Exception:
    sys.exit(print("Schlüsselliste nicht lesbar") or 1)
if not ks or any(k.get("type") != "committee" for k in ks):
    sys.exit(print(name + " ist keine Unterzeichner-Datei (nicht als solche gelistet)") or 1)
n = {k.get("signers") for k in ks}
if n != {1}:
    sys.exit(print(name + " enthält " + "/".join(str(x) for x in n) + " Schlüssel statt 1 – Version 4 wird mit genau 1 Unterzeichner angelegt") or 1)' "$SIGNER"; }
# Vor dem Deployment: fehlt die Unterzeichner-Datei bei einem angefangenen
# Deployment, passt keine neue mehr (ghostctl lehnt die Fortsetzung mit anderen
# Schlüsseln ab) – dann abbrechen, statt eine neue anzulegen.
signer_vorher() {
  local grund
  if [ -f "$SIGNER" ]; then
    grund=$(signer_ok) || fail "Unterzeichner-Datei nicht verwendbar: ${grund:-Grund unbekannt} – nichts gesendet"
    return 0
  fi
  fortschritt_v4 && fail "Ein angefangenes Deployment von Version 4 liegt in ${S4:r}.deploy.json, aber $SIGNER fehlt.
Eine neu angelegte Unterzeichner-Datei passt nicht dazu (ghostctl setzt nur mit denselben Schlüsseln fort). Bitte die ursprüngliche Datei zurücklegen – nichts gesendet."
  return 1
}

# ---------------------------------------------------------- Plan Teil B ----
# plan_b <KAS vor Teil B> <GHOST der Version 4 vor Teil B>: rechnet Teil B wie
# der echte Lauf und hängt die Schritte an den Plan. Setzt PB_SIG (Schritte und
# feste Beträge), PB_POOLKAS (KAS für den Pool, leer = Kurs unbekannt), PB_KAS
# und PB_G (Stand am Ende) und PB_WARN (was so voraussichtlich nicht geht).
plan_b() {
  local k=$1 g=$2 dep=0 own=0 zero="" haspool="" usd="" st3 ov pk orakel="" zcoll="" padd
  PB_SIG=""; PB_POOLKAS=""; PB_WARN=(); PB_POOLADD=""
  [ -f "$S4" ] && state_version "$S4" && dep=1
  if (( dep )); then
    ME4=$(MYKEY ./ghostctl "$S4" xonly) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 4 – nichts eröffnet"
    [ -z "$ME" ] || [ "$ME" = "$ME4" ] || fail "Version 3 und 4 nennen für $OWNER verschiedene Schlüssel ($ME / $ME4) – nichts eröffnet, bitte von Hand prüfen"
    ov=$(own_vaults) || fail "Status nicht abrufbar (Nodes?) – später erneut"
    read own zero <<<"$ov"
    [ -f "$VAULTMARK" ] && [ "$own" = "0" ] && fail "Ein früherer Lauf hat schon einen Vault eröffnet ($(cat "$VAULTMARK" 2>/dev/null)), er ist aber nicht als eigener zu finden.
$VAULTMARK_MSG"
    st3=$(./ghostctl --network "$NET" --state "$S4" status --json 2>/dev/null) || fail "Status nicht abrufbar"
    haspool=$(echo "$st3" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")' 2>/dev/null)
    usd=$(echo "$st3" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])' 2>/dev/null)
    orakel=$(orakel_zustand "$st3")
    [ -n "$zero" ] && zcoll=$(echo "$st3" | py 'import json,sys; print(next(v["collateralKas"] for v in json.load(sys.stdin)["vaults"] if v["index"] == int(sys.argv[1])))' "$zero" 2>/dev/null)
    # Pool halb angelegt (Audit 16 M-4): pool-open scheiterte nach init, der Pool
    # hält nur die Mindestliquidität, eigene Anteile gibt es keine
    [ -n "$haspool" ] && padd=$(pool_rest "$st3") && PB_POOLADD=$padd
  else
    # liegt Version 3 nicht mehr als mainnet.json, darf auch ihre Zinsdatei dort
    # nicht mehr liegen (sie wird vor der Zustandsdatei umbenannt)
    [ ! -f "$S4" ] && [ -e "${S4:r}-zins.json" ] && fail "$(zins_meldung)"
    if ! signer_vorher; then
      plan_add B "Unterzeichner-Datei $SIGNER anlegen (1 Schlüssel, ./ghostctl committee-keygen --count 1)" 0 0 "" "" 0 0 geschätzt "" \
        "nur auf diesem Rechner; mit ihr signiert der GHOST-Agent die Preise von Version 4"
      PB_SIG+="signer;"
    fi
    plan_add B "Register, Orakel, Factory und GHOST (Version 4) anlegen" "$(fx '-a[0]' $GEBUNDEN_KAS)" 0 "" \
      "$GEBUNDEN_KAS KAS dauerhaft (Register, Orakel, Factory und GHOST-Wurzel je $(py 'import sys; print("%g" % (float(sys.argv[1]) / 4))' $GEBUNDEN_KAS) KAS)" 5 $FEE_DEPLOY geschätzt "" \
      "deploy --key $OWNER --committee $SIGNER, ohne --probe: Fristen 14 Tage / 2 h / 30 Tage, 1 Unterzeichner, Zins an deine Adresse"
    k=$(fx 'a[0] - a[1] - a[2]' "$k" $GEBUNDEN_KAS $FEE_DEPLOY)
    PB_SIG+="deploy;"
    # Startkurs von Version 4 = Median der Börsen beim Deployment; sonst das Orakel von Version 3
    usd=$(./ghostctl --network "$NET" --json price 2>/dev/null | py 'import json,sys; print(float(json.load(sys.stdin)["median"]))' 2>/dev/null)
    [ -n "$usd" ] || usd=$V3_USD
  fi
  # N-2 (Audit 16): reicht die Sicherheit beim jetzigen Kurs für MINT_GHOST bei
  # 200 % (1 % Puffer)? Sonst scheitert mint – und ohne GHOST gibt es keinen Pool.
  if [ -n "$usd" ] && { [ "$own" = "0" ] || [ -n "$zero" ]; }; then
    local coll=${zcoll:-$VAULT_KAS}
    fq 'a[0] * a[1] >= 2 * a[2] * 1.01' "$coll" "$usd" "$MINT_GHOST" \
      || fail "Bei $usd USD je KAS reichen $coll KAS Sicherheit nicht, um $MINT_GHOST GHOST zu prägen (200 % Mindestdeckung, 1 % Puffer): nötig wären mindestens $(fx '2 * a[0] * 1.01 / a[1]' "$MINT_GHOST" "$usd") KAS. Mit größerem VAULT_KAS oder kleinerem MINT_GHOST erneut starten (z. B. VAULT_KAS=80 ./GHOST-Umzug-v4.command) – nichts gesendet."
  fi
  # N-3 (Audit 16): eingefrorenes oder bald einfrierendes Orakel von Version 4 sperrt
  # Prägen und den Pool mit Kursband – vorher ein Preis-Update als eigener Schritt
  if [ -n "$orakel" ] && { [ "$own" = "0" ] || [ -n "$zero" ] || [ -z "$haspool" ] || [ -n "$PB_POOLADD" ]; }; then
    local grund
    grund=$(signer_ok) || fail "Das Orakel von Version 4 ist $orakel, aber ohne brauchbare Unterzeichner-Datei geht kein Preis-Update: ${grund:-$SIGNER fehlt} – nichts gesendet"
    plan_add B "Orakel von Version 4 aktualisieren (es ist $orakel)" 0 0 "" "" 1 $FEE_ORAKEL geschätzt "" \
      "oracle-update --key $OWNER --committee $SIGNER: ohne frischen Preis sperrt Version 4 Prägen und den Pool"
    k=$(fx 'a[0] - a[1]' "$k" $FEE_ORAKEL)
    PB_SIG+="oracle;"
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
  elif [ -n "$PB_POOLADD" ]; then
    local ak ag
    read ak ag <<<"$PB_POOLADD"
    fq 'a[0] >= a[1]' "$g" "$ag" \
      || PB_WARN+=("Für den Rest des Pools fehlen voraussichtlich GHOST: nötig $ag, dann vorhanden $g.")
    plan_add B "Pool fertig anlegen: $ak KAS und $ag GHOST einlegen (Schritt 3 von pool-open fehlt, der Pool hält nur die Mindestliquidität)" "$(fx '-a[0] - 1' "$ak")" "-$ag" \
      "Anteile für deine Einlage (zurück mit pool-remove)" "1 KAS im Anteils-UTXO" 1 $FEE_POOL1 geschätzt "" \
      "pool-add zum jetzigen Verhältnis des Pools; weicht es beim Senden um mehr als 1 % ab, sendet ghostctl nicht"$'\n'"$H_ZUS"
    k=$(fx 'a[0] - a[1] - 1 - a[2]' "$k" "$ak" $FEE_POOL1)
    g=$(fx 'a[0] - a[1]' "$g" "$ag")
    PB_SIG+="pooladd $ag;"
  fi
  fq 'a[0] >= 0' "$k" || PB_WARN+=("Die KAS reichen voraussichtlich nicht: am Ende fehlen etwa $(r2 "$(fx '-a[0]' "$k")") KAS.")
  PB_KAS=$k; PB_G=$g
}
# Zustand des Orakels von Version 4 aus status --json: „eingefroren“ oder „in N
# Minuten eingefroren“, wenn es in weniger als ORAKEL_MIN Minuten einfriert;
# sonst nichts (Audit 16 N-3)
orakel_zustand() { print -r -- "$1" | py '
import json,sys
o = json.load(sys.stdin).get("oracle") or {}
if o.get("frozen"):
    print("eingefroren")
elif o.get("freezeInMinutes") is not None and float(o["freezeInMinutes"]) < float(sys.argv[1]):
    print("in %d Minuten eingefroren" % max(0, int(float(o["freezeInMinutes"]))))' "$ORAKEL_MIN" 2>/dev/null; }
# Pool von Version 4 halb angelegt? Gibt „<KAS> <GHOST>“ für den Rest aus, wenn der
# Pool nur die Mindestliquidität (1 KAS) hält und der Schlüssel keine Anteile hat
# (pool-open: Schritt 3 „Liquidität einlegen“ scheiterte). Der Rest: POOL_GHOST
# minus die GHOST im Pool, KAS im Verhältnis des Pools (pool-add prüft es auf 1 %).
pool_rest() {
  local lp
  lp=$(MYKEY ./ghostctl "$S4" lpShares 2>/dev/null) || return 1
  [ "${lp:-0}" = 0 ] || return 1
  print -r -- "$1" | py "$SU"'
import json,sys
p = json.load(sys.stdin).get("pool") or {}
try:
    x, y = int(p["kasSompi"]), int(p["ghostUnits"])
except Exception:
    sys.exit(1)
if x > 100000000 or y <= 0:
    sys.exit(1)
g = su(sys.argv[1]) - y
if g <= 0:
    sys.exit(1)
print(ks(g * x // y), ks(g))' "$POOL_GHOST"
}
# Vor Prägen und Pool: Orakel von Version 4 frisch? Sonst ein Preis-Update senden
# (Audit 16 N-3). Steht es im bestätigten Plan, ohne weitere Frage; sonst erst
# fragen. Ein nicht geplantes Update zählt nicht als Planschritt (PL_BASIS).
ORAKEL_GESENDET=0
orakel_frisch() {
  local st z grund
  st=$(./ghostctl --network "$NET" --state "$S4" status --json 2>/dev/null) || fail "Status nicht abrufbar – später erneut"
  z=$(orakel_zustand "$st")
  [ -z "$z" ] && return 0
  grund=$(signer_ok) || fail "Das Orakel von Version 4 ist $z, aber ohne brauchbare Unterzeichner-Datei geht kein Preis-Update: ${grund:-$SIGNER fehlt}"
  if [[ "$PLAN_SIG" != *"oracle;"* ]] || (( ORAKEL_GESENDET )); then
    echo "\n⚠ Das Orakel von Version 4 ist $z – ohne frischen Preis scheitern Prägen und Pool."
    (( EINZELN )) || frage "Orakel jetzt aktualisieren (etwa $FEE_ORAKEL KAS Gebühr)?" \
      || fail "Orakel nicht aktualisiert, Prägen und Pool nicht begonnen. Ein erneuter Doppelklick zeigt den Schritt im Plan."
    PL_BASIS=$(( PL_BASIS + 1 ))
  fi
  echo "\nOrakel von Version 4 aktualisieren (es ist $z)"
  tx G4 "Orakel von Version 4 aktualisiert" "Orakel-Update fehlgeschlagen (erneuter Doppelklick setzt fort)" oracle-update --key "$OWNER" --committee "$SIGNER"
  ORAKEL_GESENDET=1
}
# Pool-Betrag wie bestätigt? pool_wie_geplant <jetzt> <bestätigt (leer = unbekannt)>
pool_wie_geplant() { [ -n "$2" ] && fq 'abs(a[0] - a[1]) <= a[2] * a[1]' "$1" "$2" "$TOL_POOL"; }

# ----------------------------------------------------------- Probelauf ----
if (( DRY_RUN )); then
  AM=dry
  [ -f "$S3" ] && teil_a
  echo "\n--- Teil B: Version 4 anlegen ---"
  no_agent
  if [ -f "$S4" ] && state_version "$S4"; then
    echo "Version 4 ist schon angelegt ($S4)."
  elif signer_vorher; then
    echo "Unterzeichner-Datei: $SIGNER (1 Schlüssel) – wird für das Deployment genommen."
  else
    echo "Probelauf: $SIGNER fehlt – der echte Lauf legt sie an (./ghostctl committee-keygen $SIGNER --count 1)."
  fi
  echo "Probelauf: Schritte 4–6 (Anlegen, Vault, Pool) laufen erst echt – eine Probe ohne Deployment ist nicht möglich."
  echo "Geplant: Deployment ohne --probe ($GEBUNDEN_KAS KAS gebunden, 1 Unterzeichner $SIGNER), eigener Vault mit $VAULT_KAS KAS, $MINT_GHOST GHOST prägen, Pool mit $POOL_GHOST GHOST."
  exit 0
fi

# ------------------------------------------------ Plan und eine Bestätigung ----
# Bis zur Antwort wird nichts gesendet und nichts umbenannt: Teil A wird auf dem
# Zustand von Version 3 an seinem jetzigen Ort geprobt, Teil B gerechnet.
echo "\nRechne den Plan (nichts wird gesendet) …"
AM=plan
(( V3_AT_S4 )) && SA=$S4
A_DA=0
[ -f "$SA" ] && A_DA=1
(( A_DA )) && teil_a
NA=${#PL_T}
KAS_NOW=$(MYKEY ./ghostctl "$S4" kas) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 4 – nichts gesendet"
[ -n "$KAS_NOW" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – nichts gesendet, später erneut"
G4_NOW=0
if [ -f "$S4" ] && state_version "$S4"; then
  G4_NOW=$(MYKEY ./ghostctl "$S4" ghost) || fail "Eigener Schlüssel nicht lesbar"
fi
KAS_B=$(fx 'a[0]' "$KAS_NOW")
(( NA )) && KAS_B=$(fx 'a[0] + a[1] - a[2]' "$KAS_NOW" "$(summe "${PL_K[@]}")" "$(summe "${PL_F[@]}")")
plan_b "$KAS_B" "$G4_NOW"
PLAN_SIG=$PB_SIG; PLAN_POOLKAS=$PB_POOLKAS; PLAN_KAS_B=$KAS_B

# Leerer Plan: hier enden. Ohne bestätigten Schritt läuft kein Teil mit --ja.
if (( ${#PL_T} == 0 )); then
  echo "\nNichts mehr zu senden: Alle Schritte sind erledigt oder werden übersprungen."
  ./ghostctl --network "$NET" --state "$S4" status 2>/dev/null
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
if (( V3_AT_S4 )); then
  rename_v3 0 || fail "Umbenennen fehlgeschlagen"
  echo "Zustand von Version 3 liegt jetzt in $S3"
  journal_v3
fi
SA=$S3

# ---------------------------------------------------------------- Teil A ----
AM=echt
if (( A_DA )); then
  AI=1
  teil_a
  AI=0
fi

# ---------------------------------------------------------------- Teil B ----
echo "\n--- Teil B: Version 4 anlegen ---"
no_agent
# Nach Teil A haben sich die Guthaben geändert (Rückflüsse aus Pool und Vaults):
# Teil B aus dem echten Stand neu rechnen. Weicht er vom bestätigten Plan ab,
# anhalten und erneut fragen, statt still andere Beträge zu senden.
if (( A_DA )); then
  KAS_JETZT=$(MYKEY ./ghostctl "$S4" kas) || fail "Eigener Schlüssel nicht lesbar"
  [ -n "$KAS_JETZT" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – Teil B nicht begonnen, später erneut"
  G4_JETZT=0
  if [ -f "$S4" ] && state_version "$S4"; then
    G4_JETZT=$(MYKEY ./ghostctl "$S4" ghost) || fail "Eigener Schlüssel nicht lesbar"
  fi
  PL_BASIS=${#GESENDET}
  plan_leeren
  REST_V3=(); G3_REST=""
  plan_b "$KAS_JETZT" "$G4_JETZT"
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

if [ ! -f "$S4" ]; then
  if ! signer_vorher; then
    echo "\nSchritt 4/6: Unterzeichner-Datei $SIGNER anlegen (1 Schlüssel)"
    ./ghostctl committee-keygen "$SIGNER" --count 1 || fail "Unterzeichner-Datei nicht angelegt – nichts gesendet. Erneuter Doppelklick setzt fort."
    [ -f "$SIGNER" ] || fail "Unterzeichner-Datei $SIGNER fehlt nach committee-keygen – nichts gesendet"
    GESENDET+=("Unterzeichner-Datei $SIGNER angelegt (sendet nichts)")
    signer_vorher || fail "Unterzeichner-Datei $SIGNER fehlt"
  fi
  # Die Zinsdatei von Version 3 darf hier nicht mehr liegen: das Deployment legt
  # an ihrem Platz die neue an (rate_restart). Direkt vor dem Senden geprüft, nach
  # allem, was davor lief (Audit 16, Rückbau-Probe R4)
  [ -e "${S4:r}-zins.json" ] && fail "$(zins_meldung)"
  echo "\nSchritt 4/6: Register, Orakel, Factory und GHOST (Version 4) anlegen – $GEBUNDEN_KAS KAS bleiben dauerhaft gebunden, Unterzeichner $SIGNER"
  # ohne --probe (Fristen 14 Tage / 2 h / 30 Tage) und ohne --rate/--threshold:
  # Startzins 0 und Schwelle 1 setzt ghostctl selbst; ein angefangenes Deployment
  # setzt ghostctl nur mit denselben Einstellungen fort (Audit 14 N4)
  tx G4 "Register, Orakel, Factory und GHOST (Version 4) angelegt" "Deployment fehlgeschlagen (erneuter Doppelklick setzt fort)" deploy --key "$OWNER" --committee "$SIGNER"
else
  state_version "$S4" || fail "$S4 ist nicht Version 4"
  echo "\nSchritt 4/6: Version 4 ist schon angelegt – übersprungen"
fi

ME4=$(MYKEY ./ghostctl "$S4" xonly) || fail "Eigener Schlüssel $OWNER nicht (eindeutig) in der Schlüsselliste von Version 4 – nichts eröffnet"
[ -z "$ME" ] || [ "$ME" = "$ME4" ] || fail "Version 3 und 4 nennen für $OWNER verschiedene Schlüssel ($ME / $ME4) – nichts eröffnet, bitte von Hand prüfen"
OV=$(own_vaults) || fail "Status nicht abrufbar (Nodes?) – später erneut"
read OWN ZERO <<<"$OV"
if [ -f "$VAULTMARK" ]; then
  [ "$OWN" = "0" ] && fail "Ein früherer Lauf hat schon einen Vault eröffnet ($(cat "$VAULTMARK" 2>/dev/null)), er ist aber nicht als eigener zu finden.
$VAULTMARK_MSG"
  rm -f "$VAULTMARK"
fi
if [ "$OWN" = "0" ]; then
  echo "\nSchritt 5/6: eigenen Vault mit $VAULT_KAS KAS eröffnen und $MINT_GHOST GHOST prägen"
  tx G4 "Vault mit $VAULT_KAS KAS eröffnet" "Vault eröffnen fehlgeschlagen (erneuter Doppelklick setzt fort)" open-vault --key "$OWNER" --kas "$VAULT_KAS"
  # angenommen: bis er als eigener gefunden ist, eröffnet kein weiterer Lauf einen Vault
  echo "Besitzer $ME4, $(date '+%d.%m.%Y %H:%M')" > "$VAULTMARK"
  OV=$(own_vaults) || fail "Status nicht abrufbar – erneuter Doppelklick prägt nach"
  read OWN ZERO <<<"$OV"
  [ -n "$ZERO" ] || fail "Neuer Vault nicht als eigener gefunden. Ein erneuter Doppelklick eröffnet keinen weiteren ($VAULTMARK); bitte auf der Seite „Vault“ prüfen."
  rm -f "$VAULTMARK"
  orakel_frisch
  tx G4 "$MINT_GHOST GHOST geprägt (Vault $ZERO)" "Prägen fehlgeschlagen (erneuter Doppelklick prägt nach)" mint --key "$OWNER" --vault "$ZERO" --ghost "$MINT_GHOST"
elif [ -n "$ZERO" ]; then
  echo "\nSchritt 5/6: eigener Vault $ZERO hat noch keine Schuld – $MINT_GHOST GHOST prägen"
  orakel_frisch
  tx G4 "$MINT_GHOST GHOST geprägt (Vault $ZERO)" "Prägen fehlgeschlagen (erneuter Doppelklick prägt nach)" mint --key "$OWNER" --vault "$ZERO" --ghost "$MINT_GHOST"
else
  echo "\nSchritt 5/6: es gibt schon einen eigenen Vault (mit Schuld oder gesperrt) – übersprungen"
fi

ST3=$(./ghostctl --network "$NET" --state "$S4" status --json 2>/dev/null) || fail "Status nicht abrufbar"
if [ -z "$(echo "$ST3" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")')" ]; then
  KAS_USD=$(echo "$ST3" | py 'import json,sys; print(json.load(sys.stdin)["oracle"]["kasUsd"])')
  POOL_KAS=$(py 'import sys; print(f"{float(sys.argv[1])/float(sys.argv[2]):.8f}")' "$POOL_GHOST" "$KAS_USD")
  # erst prüfen, dann senden: pool-open schickt die Genesis (1 KAS) vor der GHOST-Prüfung (A11-O-5)
  HAVE_G=$(MYKEY ./ghostctl "$S4" ghost) || fail "Eigener Schlüssel nicht lesbar"
  HAVE_K=$(MYKEY ./ghostctl "$S4" kas) || fail "Eigener Schlüssel nicht lesbar"
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
  orakel_frisch
  echo "\nSchritt 6/6: Tauschpool mit Kursband – $POOL_KAS KAS und $POOL_GHOST GHOST (Startkurs 1 USD, Orakel $KAS_USD USD je KAS)"
  # pool-open hat drei Schritte (Genesis, init mit Mindestliquidität, Rest
  # einlegen). Scheitert einer, setzt der nächste Doppelklick fort: nach der
  # Genesis zeigt status noch keinen Pool, und pool-open setzt selbst fort; nach
  # init hält der Pool nur die Mindestliquidität – das erkennt pool_rest unten.
  tx G4 "Tauschpool mit $POOL_KAS KAS und $POOL_GHOST GHOST angelegt" "Pool anlegen fehlgeschlagen – erneuter Doppelklick setzt fort (bei halb angelegtem Pool legt er den Rest ein)" pool-open --key "$OWNER" --kas "$POOL_KAS" --ghost "$POOL_GHOST"
elif PADD=$(pool_rest "$ST3"); then
  # Audit 16 M-4: Pool angelegt, aber Schritt 3 von pool-open fehlt
  read ADD_K ADD_G <<<"$PADD"
  HAVE_G=$(MYKEY ./ghostctl "$S4" ghost) || fail "Eigener Schlüssel nicht lesbar"
  HAVE_K=$(MYKEY ./ghostctl "$S4" kas) || fail "Eigener Schlüssel nicht lesbar"
  [ -n "$HAVE_K" ] || fail "KAS-Guthaben unbekannt (Node nicht erreichbar?) – später erneut"
  (( $(sompi "$HAVE_G") >= $(sompi "$ADD_G") )) \
    || fail "Für den Rest des Pools fehlen GHOST: nötig $ADD_G, vorhanden ${HAVE_G:-0}. Erst prägen (Seite „Vault“), dann erneut doppelklicken."
  (( $(sompi "$HAVE_K") >= $(sompi "$ADD_K") + 200000000 )) \
    || fail "Für den Rest des Pools fehlen KAS: nötig etwa $ADD_K + 2 (Anteils-UTXO und Gebühren), vorhanden $HAVE_K."
  if ! (( EINZELN )) && [[ "$PLAN_SIG" != *"pooladd $ADD_G;"* ]]; then
    echo "\n⚠ Der Pool von Version 4 hält nur die Mindestliquidität (Schritt 3 von pool-open fehlt), das stand so nicht im bestätigten Plan."
    frage "Pool fertig anlegen: $ADD_K KAS und $ADD_G GHOST einlegen?" || fail "Pool nicht fertig angelegt – erneuter Doppelklick setzt fort (oder auf der Seite „Tauschen“ mit pool-add)"
  fi
  orakel_frisch
  echo "\nSchritt 6/6: Pool fertig anlegen – $ADD_K KAS und $ADD_G GHOST einlegen (pool-open war nach der Mindestliquidität abgebrochen)"
  tx G4 "Rest in den Pool eingelegt ($ADD_K KAS und $ADD_G GHOST)" "Einlegen fehlgeschlagen – erneuter Doppelklick setzt fort (oder auf der Seite „Tauschen“)" pool-add --key "$OWNER" --kas "$ADD_K" --ghost "$ADD_G"
else
  echo "\nSchritt 6/6: Tauschpool existiert schon – übersprungen"
fi

# „Fertig“ nur mit vollständigem Pool (Audit 16 M-4)
ST3=$(./ghostctl --network "$NET" --state "$S4" status --json 2>/dev/null) || fail "Status nicht abrufbar – ob der Pool fertig ist, ist offen; erneut doppelklicken"
[ -n "$(echo "$ST3" | py 'import json,sys; print("ja" if json.load(sys.stdin).get("pool") else "")')" ] \
  || fail "Der Pool von Version 4 ist noch nicht angelegt (pool-open nicht abgeschlossen) – erneuter Doppelklick setzt fort."
pool_rest "$ST3" >/dev/null && fail "Der Pool von Version 4 hält nur die Mindestliquidität, deine Einlage fehlt – erneuter Doppelklick legt den Rest ein."

echo "\n=== Fertig: Version 4 läuft ==="
./ghostctl --network "$NET" --state "$S4" status 2>/dev/null
./ghostctl --network "$NET" --state "$S4" balance --key "$OWNER" 2>/dev/null
echo "\nJetzt den GHOST-Agenten mit „GHOST-Agent starten.command“ neu starten: er hält Orakel und Zins nach"
echo "und läuft mit $SIGNER als Unterzeichner."
[ -f "$SIGNER" ] || echo "⚠ $SIGNER fehlt – ohne sie signiert der Agent keine Preise für Version 4."
# Das Startskript nahm bisher $KEYS-committee.json (die 5 Schlüssel von Version 3);
# mit denen lehnt das Orakel von Version 4 jedes Update ab
grep -q -- "-signer.json" "GHOST-Agent starten.command" 2>/dev/null \
  || echo "⚠ „GHOST-Agent starten.command“ nimmt noch $KEYS-committee.json statt $SIGNER – vor dem Start anpassen."
read -k 1 "?Taste drücken zum Schließen …" || true
