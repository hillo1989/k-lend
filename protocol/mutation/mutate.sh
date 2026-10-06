#!/bin/zsh
# Mutationstest: ersetzt jede require-Zeile eines Vertrags einzeln durch
# require(true) und meldet, welche Tests dadurch rot werden. "LUECKE" = kein
# Test fällt auf. Der Vertrag wird am Ende wiederhergestellt.
#
# Aufruf aus dem Projektordner:
#   protocol/mutation/mutate.sh contracts/stable_vault.sil vault_tests vault_math_tests
#   protocol/mutation/mutate.sh contracts/vault_factory.sil factory_tests e2e_tests
#   protocol/mutation/mutate.sh contracts/risk_oracle.sil oracle_tests
# schneller: CARGO_FLAGS=--release protocol/mutation/mutate.sh …
set -u
cd "$(dirname "$0")/../.." || exit 1
CONTRACT=$1; shift
TESTS=(); for t in "$@"; do TESTS+=(--test "$t"); done
BACKUP=$(mktemp)
cp "$CONTRACT" "$BACKUP"
# Bei Abbruch wiederherstellen UND beenden: ohne exit lief die Schleife nach dem
# Trap weiter und schrieb aus der gelöschten Sicherung einen leeren Vertrag
# (Audit 10, Abschnitt 5 von audit/10-opus-pool.md)
restore() { [ -s "$BACKUP" ] && cp "$BACKUP" "$CONTRACT"; rm -f "$BACKUP"; }
trap 'restore' EXIT
trap 'restore; trap - EXIT; exit 130' INT TERM
for n in $(grep -n "require(" "$BACKUP" | grep -v "^[0-9]*:[[:space:]]*//" | cut -d: -f1); do
  line=$(sed -n "${n}p" "$BACKUP" | sed 's/^ *//')
  [ -s "$BACKUP" ] || { echo "Sicherung fehlt – Abbruch"; exit 1; }
  awk -v n="$n" 'NR==n { match($0, /^ */); print substr($0, 1, RLENGTH) "require(true); // MUTANT"; next } { print }' "$BACKUP" > "$CONTRACT"
  out=$(cd protocol && cargo test ${=CARGO_FLAGS:-} "${TESTS[@]}" 2>&1)
  if echo "$out" | grep -q "error\[E\|could not compile"; then echo "BUILD   L$n | $line"; continue; fi
  failed=$(echo "$out" | grep "^test .* FAILED" | sed 's/^test \(.*\) \.\.\. FAILED/\1/' | tr '\n' ' ')
  [ -z "$failed" ] && echo "LUECKE  L$n | $line" || echo "rot     L$n | $line | $failed"
done
