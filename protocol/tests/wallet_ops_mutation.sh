#!/bin/zsh
# Rückbau-Probe für die Prüfungen im Wallet-Weg (src/wallet_ops.rs,
# src/wallet.rs, src/store.rs, src/bin/ghostctl.rs): Jede Prüfung wird
# einzeln abgeschaltet, dann laufen die zugehörigen Gegenproben – alle Tests
# aus tests/wallet_ops_tests.rs (auch die großen: alle_aktionen, liquidieren,
# zu_viele), die Journal- und Sperr-Tests in store.rs bzw. der
# Verdrahtungstest in ghostctl.rs. „erkannt“ = mindestens ein Test wird rot
# (gut). „überlebt“ = eine andere Prüfung fängt den Fall ebenfalls (zweite
# Linie) – das steht dann dabei.
#
# Das Skript verändert src/ an Ort und Stelle und stellt es am Ende (auch bei
# Abbruch) wieder her. Am besten in einer Kopie laufen lassen (git worktree).
# Aufruf im Ordner protocol: zsh tests/wallet_ops_mutation.sh [Namensfilter]
set -u
cd "$(dirname "$0")/.."
FILTER=${1:-}
FILES=(src/wallet_ops.rs src/wallet.rs src/store.rs src/bin/ghostctl.rs src/tresor.rs src/txb.rs)
BACKUP=$(mktemp -d)
for f in $FILES; do mkdir -p "$BACKUP/$(dirname $f)"; cp "$f" "$BACKUP/$f"; done
restore() { for f in $FILES; do cmp -s "$BACKUP/$f" "$f" || cp "$BACKUP/$f" "$f"; done; }
trap restore EXIT

WO="--test wallet_ops_tests"
ST="--lib store::"
GT="--bin ghostctl wallet_tresor"
GC="--bin ghostctl wallet_aktionen"

# Name | Datei | Testziel | perl-Ersetzung
MUTANTS=(
  "Plan-Vergleich (Neubau ≠ Plan)|src/wallet_ops.rs|$WO|s/    if !same \{/    if false \&\& !same {/"
  "Veränderte Felder der Wallet-Antwort|src/wallet_ops.rs|$WO|s/    if !rep.changed.is_empty\(\) \{\n        rep.error = Some\(format!\(\"Die Wallet hat die Tx verändert: \{\}\", rep.changed.join\(\", \"\)\)\);\n        return none/    if false \&\& !rep.changed.is_empty() {\n        rep.error = Some(format!(\"Die Wallet hat die Tx verändert: {}\", rep.changed.join(\", \")));\n        return none/"
  "Signatur gültig (sig_valid)|src/wallet_ops.rs|$WO|s/rep.inputs.iter\(\).any\(\|i: &InputReport\| !i.sig_valid\) \|\| //"
  "Skriptprüfung nach dem Einsetzen|src/wallet_ops.rs|$WO|s/        check_scripts\(tx, ue\)\?;\n//"
  "Hashtype 0x01|src/wallet.rs|$WO|s/Some\(s\) if s\[64\] != SIG_HASH_ALL.to_u8\(\) =>/Some(s) if false =>/"
  "Schnorr-Prüfung gegen die Adresse|src/wallet.rs|$WO|s/map\(\|m\| sg.verify\(&m, &owner\).is_ok\(\)\)/map(|_m| true)/"
  "RM2 Besitzer-Filter der eigenen GHOST (own_tokens)|src/wallet_ops.rs|$WO|s/filter\(\|\(_, t\)\| t.state.owner == x \&\& /filter(|(_, t)| /"
  "RM3 Messkopie = echte Tx (same_shape)|src/wallet_ops.rs|$WO|s/(pub fn same_shape\(m: &Built, r: &Built\) -> Result<\(\), String> \{\n)/\$1    return Ok(());\n/"
  "A17-1 Ersatzschlüssel je Bau zufällig|src/wallet.rs|$WO|s/Keypair::new\(&Secp256k1::new\(\), &mut secp256k1::rand::thread_rng\(\)\)/Keypair::from_secret_key(\&Secp256k1::new(), \&SecretKey::from_slice(\&[0x42; 32]).unwrap())/"
  "A17-1 Vorbesitz des Ersatzschlüssels neutralisieren (mirror_dep)|src/wallet_ops.rs|$WO|s/if (v|t\.state)\.owner == mirror \{/if false \&\& \$1.owner == mirror {/g"
  "A17-4 Vorprüfung der Signaturen ohne Netz (precheck)|src/wallet_ops.rs|$WO|s/(        rep.error = Some\(\"Mindestens eine Signatur fehlt oder ist ungültig\".into\(\)\);\n    \}\n)    Ok\(rep\)/\$1    rep.error = None;\n    Ok(rep)/"
  "A17-4 Vorprüfung vor Netz und Sperre verdrahtet|src/bin/ghostctl.rs|$GC|s/            if let Some\(e\) = &pre.error \{/            if let Some(e) = \&None::<String> {/"
  "A17-4 Sperre erst nach der Prüfung|src/bin/ghostctl.rs|$GC|s/            let d = load_readonly\(&ctx\).await\?;\n            check_funding_at_node/            let _frueh = if *send { Some(store::lock(\&ctx.state_path, WALLET_LOCK_WAIT)?) } else { None };\n            let d = load_readonly(\&ctx).await?;\n            check_funding_at_node/"
  "A17-2 Wallet-Journal über die Tx-ID klären|src/store.rs|$ST|s/        if p.wallet \{/        if false \&\& p.wallet {/"
  "A17-2 Frist für unklare Wallet-Journale|src/store.rs|$ST|s/    if age >= grace \{\n        clear_pending/    if false \&\& age >= grace {\n        clear_pending/"
  "A17-2 Zwilling erkennen|src/store.rs|$ST|s/(fn is_twin\(t: &chain::TxView, p: &Pending\) -> bool \{\n)/\$1    if true { return false; }\n/"
  "A17-6 Sperre vor dem Warten freigeben|src/store.rs|$ST|s/    drop\(lock\);\n    let id = sent\?;\n    let waited = wait\(\).await;/    let id = sent?;\n    let waited = wait().await;\n    drop(lock);/"
  "A17-6 Wallet-Senden über send_then_wait|src/bin/ghostctl.rs|$GC|s/store::send_then_wait\(lock, send, wait\)/store::send_then_wait(store::lock(\&self.state_path, Duration::ZERO).unwrap_or(lock), send, wait)/"
  "A17-8 Dateiname statt Pfad in Meldungen|src/store.rs|$ST|s/p.file_name\(\).map\(\|n\| n.to_string_lossy\(\).into_owned\(\)\).unwrap_or_default\(\)/p.display().to_string()/"
  "Tresor: Auffüllen und Kündigen nur der Besitzer (own_tresor)|src/wallet_ops.rs|$WO|s/    if r.params.owner != me \{/    if false \&\& r.params.owner != me {/"
  "Tresor: Messkopie neutralisiert Vorbesitz des Ersatzschlüssels|src/wallet_ops.rs|$WO|s/            if r.params.owner == mirror \{/            if false \&\& r.params.owner == mirror {/"
  "Tresor: höchstens MAX_WALLET_PER_OWNER je Besitzer|src/tresor.rs|$WO|s/        if running >= MAX_WALLET_PER_OWNER \{/        if false \&\& running >= MAX_WALLET_PER_OWNER {/"
  "Tresor: Kündigen mit der gezahlten Gebühr der Messkopie|src/txb.rs|$WO|s/then_some\(f.paid\)/then_some(f.fee)/"
  "Tresor: Agent zahlt bei Wallet-Tresoren keine Gebühr dazu|src/tresor.rs|$GT|s/with_key \&\& !self.wallet/with_key/"
  "Tresor: Sperre der Tresor-Datei erst nach der Prüfung|src/bin/ghostctl.rs|$GT|s/(async fn wallet_tresor_submit\([^\n]*\n)/\$1    let _frueh = if send { Some(store::lock(\&ctx.state_path, WALLET_LOCK_WAIT)?) } else { None };\n/"
  "A19-1 Terminregel im Neubau (run_tresor)|src/wallet_ops.rs|$WO|s/            tresor::check_wallet_first_due\(\*first_due, b.pmt\)\?;\n//"
  "A19-1 Agent bedient fällige Tresore reihum|src/tresor.rs|$WO|s/            v.sort_by_key\(\|&i\| \{/            v.sort_by_key(|\&i| -> (i32, i64) { if true { return (0, 0); }/"
  "A19-2 Besitzer vor der Suche (follow_for_wallet)|src/wallet_ops.rs|$WO|s/    let i = own_tresor\(file, id, owner\)\?;\n    tresor::follow/    let i = file.tresore.iter().position(|r| r.utxo.cov.to_string() == *id).ok_or(\"x\")?;\n    tresor::follow/"
  "A19-2 Suche der Seite begrenzt (PUBLIC_FOLLOW)|src/tresor.rs|$WO|s/        Search::Public => PUBLIC_FOLLOW,/        Search::Public => MAX_FOLLOW,/"
  "A19-2 Ergebnis der Suche sofort sichern|src/tresor.rs|$WO|s/            io.save\(file\)\?;\n        \}\n        let r = file.tresore\[i\].clone\(\);/        }\n        let r = file.tresore[i].clone();/"
  "A19-3 ruhende Tresore belegen keinen Platz (busy)|src/tresor.rs|$WO|s/            && self.utxo.state.next_due <= now_ms \+ BUSY_HORIZON_MS//"
)

for m in "${MUTANTS[@]}"; do
  name=${m%%|*}; rest=${m#*|}; file=${rest%%|*}; rest=${rest#*|}; target=${rest%%|*}; expr=${rest#*|}
  [[ -n "$FILTER" && "$name" != *"$FILTER"* ]] && continue
  restore
  perl -0pi -e "$expr" "$file"
  if cmp -s "$file" "$BACKUP/$file"; then
    echo "FEHLER: Mutation „$name“ greift nicht (Muster nicht gefunden)"; exit 1
  fi
  out=$(cargo test --release --offline ${=target} 2>&1)
  if echo "$out" | grep -q "test result: FAILED"; then
    echo "erkannt:  $name  ($(echo "$out" | grep -c '\.\.\. FAILED') Test(s) rot: $(echo "$out" | grep '\.\.\. FAILED' | sed -E 's/^test ([^ ]+).*/\1/' | tr '\n' ' '))"
  elif echo "$out" | grep -q "test result: ok"; then
    echo "überlebt: $name"
  else
    echo "Bau gescheitert: $name"; echo "$out" | grep -E '^error' -A5 | head -20
  fi
done
restore
