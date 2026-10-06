#!/bin/zsh
# Bringt das öffentliche GitHub-Repo (github.com/hillo1989/k-lend) auf den Stand von main.
# Hochgeladen wird nur der Dateistand als neuer Commit, nie die lokale Historie:
# darin liegen Dateien, die nicht öffentlich sein sollen (siehe AUSLASSEN).
set -eu
REPO=${0:A:h:h}
WORK=${TMPDIR:-/tmp}/klend-public
AUSLASSEN=(RECHT_PRUEFUNG.md bin/ghostctl-v1 audit/6-aussagen-und-doku.md)
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
git add -A
if git ls-files | grep -qE '^keys/|RECHT_PRUEFUNG|ghostctl-v1'; then
  echo "Abbruch: verbotene Datei im Stand" >&2; exit 1
fi
if git diff --cached --quiet; then echo "GitHub ist schon aktuell."; exit 0; fi
git -c user.name="Peter Pan" -c user.email="peterpan@Mini-von-Peter.local" commit -q -m "$MSG"
git push -q origin main
echo "GitHub aktualisiert: $(git log --oneline -1)"
