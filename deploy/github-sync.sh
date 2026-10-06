#!/bin/zsh
# Bringt das öffentliche GitHub-Repo (github.com/hillo1989/k-lend) auf den Stand von main.
# Hochgeladen wird nur der Dateistand als neuer Commit, nie die lokale Historie:
# darin liegen Dateien, die nicht öffentlich sein sollen (siehe AUSLASSEN).
set -eu
REPO=${0:A:h:h}
WORK=${TMPDIR:-/tmp}/klend-public
AUSLASSEN=(RECHT_PRUEFUNG.md bin/ghostctl-v1 audit/6-aussagen-und-doku.md)
# öffentlich unter deployments/: nur die Kettendaten der ersten Version
OEFFENTLICH_DEPLOYMENTS='deployments/(mainnet|testnet-10)-v1\.json'
# nie veröffentlichen (Schlüssel, Rechtsprüfung, Betriebsdateien mit Besucherdaten)
VERBOTEN='^keys/|RECHT_PRUEFUNG|ghostctl-v1|^deployments/mainnet\.json$|-tresore\.json$|-zins\.json$|-abos\.json$|-txlog\.jsonl$|\.pending\.json$|^deployments/.*\.lock|^deployments/\.umzug-'
# Freigabeliste der veröffentlichten Pfade
ERLAUBT="^(app|protocol|contracts|docs|deploy|audit|tests|vendor|\.cargo|\.claude)/|^$OEFFENTLICH_DEPLOYMENTS\$|^[^/]+\.(md|command)\$|^(\.gitignore|\.gitmodules|ghostctl)\$"
MSG=${1:-"Stand $(date +%d.%m.%Y)"}

if [[ ! -d $WORK/.git ]]; then
  git clone -q https://github.com/hillo1989/k-lend.git "$WORK"
fi
cd "$WORK"
git fetch -q origin main && git checkout -q -B main origin/main
git fetch -q "$REPO" main
git read-tree -u --reset FETCH_HEAD
git rm -q --cached --ignore-unmatch "${AUSLASSEN[@]}"
rm -f "${AUSLASSEN[@]}"
# Verweise auf ausgelassene Dateien entfernen
sed -i '' 's|; Marken-/Tickerprüfung in RECHT_PRUEFUNG.md)|)|' ARCHITEKTUR.md
sed -i '' '/Vorprüfung in `RECHT_PRUEFUNG.md`/d' ARCHITEKTUR.md
sed -i '' '/audit\/6-aussagen-und-doku.md/d' AUDIT.md
# Betriebsdateien unter deployments/ nie veröffentlichen (Audit 20 A20c-6):
# Zustand, Besucher-Tresore (Empfänger, Nachrichten), Zinsregel, Journale,
# Sperren. Öffentlich sind nur die Kettendaten der ersten Version.
BETRIEB=(${(f)"$(git -c core.quotepath=off ls-files deployments | grep -vE "^$OEFFENTLICH_DEPLOYMENTS\$" || true)"})
if (( ${#BETRIEB} )); then
  git rm -q --cached --ignore-unmatch -- "${BETRIEB[@]}"
  rm -f -- "${BETRIEB[@]}"
fi
git add -A
# Sperrliste: was nie öffentlich werden darf, auch wenn es oben durchrutscht
if git -c core.quotepath=off ls-files | grep -E "$VERBOTEN"; then
  echo "Abbruch: verbotene Datei im Stand (siehe oben)" >&2; exit 1
fi
# Freigabeliste: alles andere muss in einem bekannten Ordner liegen; Neues
# (z. B. ein Ordner mit Zustandsdateien) bricht ab, bis es hier eingetragen ist
if git -c core.quotepath=off ls-files | grep -vE "$ERLAUBT"; then
  echo "Abbruch: Dateien außerhalb der Freigabeliste (siehe oben) – prüfen und ggf. ERLAUBT in deploy/github-sync.sh ergänzen" >&2; exit 1
fi
if git diff --cached --quiet; then echo "GitHub ist schon aktuell."; exit 0; fi
git -c user.name="Peter Pan" -c user.email="peterpan@Mini-von-Peter.local" commit -q -m "$MSG"
git push -q origin main
echo "GitHub aktualisiert: $(git log --oneline -1)"
