# Wallet-Probe: signieren KasWare und Kastle unsere Covenants?

Mit dieser Probe prüfst du im Mainnet mit eigenen Kleinstbeträgen, ob KasWare und/oder Kastle Transaktionen mit den GHOST-Covenants signieren können. Jede Signatur bestätigst du selbst in der Wallet. Gesendet wird nur, wenn du ausdrücklich „Jetzt senden“ drückst (Seite) oder `--send` angibst (Kommandozeile).

Hintergrund und Quellen: Recherche „GHOST mit Browser-Wallets selbst signieren“, Abschnitt 5.3. Technik: `protocol/src/wallet.rs`, `ghostctl wallet …`, Seite `app/wallet-probe.html`.

## Was geprüft wird

Der Probe-Tresor ist ein Dauerauftrag (`contracts/standing_order.sil`), bei dem du Absender **und** Empfänger bist. Er hat eine Zahlung zu 1 KAS mit Termin in etwa einer Stunde und eine Höchstgebühr von 0,004 KAS.

| Stufe | Was passiert | Was die Wallet signiert | Kosten |
|---|---|---|---|
| 0 Trockenprobe | Kündigung eines **erfundenen** Tresors. ghostctl prüft die Signatur lokal gegen den Vertrag. | den Covenant-Eingang (Kernfrage) | 0 KAS, wird **nie** gesendet |
| 1 Anlegen | etwa 1,5 KAS gehen aus der Wallet in den Probe-Tresor | eigene KAS-Eingänge (v1-Tx mit Covenant-Ausgang) | Netzgebühr etwa 0,002 KAS (gemessen 0,0022) |
| 2 Kündigen | alles abzüglich Gebühr geht an deine Adresse zurück | den Covenant-Eingang des echten Tresors | Netzgebühr etwa 0,003 KAS (gemessen 0,0027) |

Stufe 1 und 2 zusammen kosten im Simulator 0,0049 KAS. Die 1,5 KAS bleiben deine.

Was du in der Wallet siehst: Ein- und Ausgänge mit Beträgen, aber nicht, was der Vertrag bedeutet. Vergleiche die Beträge mit der Übersicht der Seite. Bei Stufe 1 geht ein Ausgang an eine Skript-Adresse (`kaspa:p…`, der Tresor), der Rest kommt als Wechselgeld an deine Adresse zurück. Bei Stufe 0 und 2 gibt es nur einen Ausgang, und der geht an deine Adresse.

## Voraussetzungen

- KasWare oder Kastle als Browser-Erweiterung, im **Mainnet**, mit einem **Schnorr-Konto**: Die Adresse beginnt mit `kaspa:q`. Ledger-Konten gehen bei Kastle nicht (Kastle signiert dort nur Tx-Version 0).
- Auf der Wallet-Adresse mindestens etwa 1,6 KAS (Startguthaben plus Gebühr).
- Den Code aus dem Branch `wallet-probe` (Seite und `ghostctl wallet`).

## Start über die Seite

1. ghostctl einmal vorab bauen, damit der erste Aufruf der Seite nicht in die Zeitgrenze läuft: im Projektordner `./ghostctl --help`.
2. Die Seite wie gewohnt starten (`GHOST-Seite öffnen.command`, Port 5180). Dann **http://localhost:5180/wallet-probe.html** öffnen.
   Vor dem Mergen aus dem Worktree heraus: `cd kaspa-lending-wallet/app && npm ci && npm run dev -- --port 5181 --strictPort`, dann http://localhost:5181/wallet-probe.html. Achtung: Dieser Server stößt wie jeder Seitenserver auch fällige Daueraufträge und Tresore aus den `deployments/`-Dateien seines Ordners an.
3. Netz `mainnet` lassen und „Mit KasWare verbinden“ bzw. „Mit Kastle verbinden“ wählen.
4. **Stufe 0:** „Tx holen“, dann „In der Wallet signieren und prüfen“. In der Wallet bestätigen. Die Seite zeigt „Signatur GÜLTIG“ oder „UNGÜLTIG“ mit Einzelheiten. Ein „Jetzt senden“ gibt es hier nicht.
   - GÜLTIG heißt: Die Wallet signiert fremde Covenant-Eingänge so, dass `checkSig(owner)` besteht. Damit ist die Kernfrage beantwortet, ohne dass ein Sompi bewegt wurde.
   - Lehnt die Wallet ab oder signiert nichts („Wallet hat diesen Eingang NICHT signiert“), weiter mit Stufe 1. Möglicherweise schlägt die Wallet Eingänge im Netz nach und lehnt die erfundene UTXO deshalb ab.
5. **Stufe 1:** Startguthaben (Standard 1,5 KAS) und Termin (Standard 60 Minuten) prüfen. Dann „Tx holen“ und „In der Wallet signieren und prüfen“. Ist die Signatur gültig, auf **„Jetzt senden“** drücken und die Rückfrage bestätigen. Die Seite merkt sich den Probe-Tresor im Browser. Mit „Probe-Datei sichern“ legst du zusätzlich eine Kopie als Datei an (empfohlen).
6. Etwa eine Minute warten, bis die Anlage bestätigt ist.
7. **Stufe 2:** wählen, „Tx holen“, signieren, prüfen, „Jetzt senden“. Danach sind die KAS zurück auf deiner Adresse.

Am besten beide Wallets nacheinander durchspielen, jede mit eigenem Probe-Tresor. Notiere für jede Wallet: signiert / verweigert / stumm unsigniert, den Hashtype (sollte 0x01 sein), ob die Wallet Felder verändert hat (Abschnitt „Von der Wallet verändert“) und, falls möglich, ein Bildschirmfoto des Signierdialogs.

## Start über die Kommandozeile (ohne Seite)

Die Wallet selbst lässt sich nur aus dem Browser bedienen. Die ghostctl-Seite geht aber auch von Hand:

```
./ghostctl --json wallet export-unsigned dry-cancel --address kaspa:q… > plan.json
#   in der Browser-Konsole einer beliebigen Seite (Wallet-Erweiterung aktiv):
#     const p = <Inhalt von plan.json einfügen>
#   Kastle
#     await kastle.signTx(p.kastle.networkId, p.kastle.txJson, p.kastle.scripts)
#   oder KasWare
#     await kasware.signPskt(p.kasware)
#   Ergebnis als signed.json speichern
./ghostctl wallet attach-sigs --plan plan.json --signed signed.json            # nur prüfen
./ghostctl wallet attach-sigs --plan plan.json --signed signed.json --send --save-probe probe.json   # Stufe 1/2 senden (Mainnet fragt nach)
./ghostctl --json wallet export-unsigned tresor-open --address kaspa:q… [--fund 1.5] [--due-minutes 60]
./ghostctl --json wallet export-unsigned tresor-cancel --address kaspa:q… --probe probe.json
./ghostctl wallet probe-pay --probe probe.json [--send]                        # Sicherheitsnetz
```

`wallet …` liest und schreibt nichts unter `keys/` oder `deployments/` und braucht keine Schlüsseldatei. Den Node fragt es nur lesend (UTXOs der Adresse bzw. des Tresors), außer bei `--send`.

## Wenn etwas schiefgeht

- **Stufe 0 oder 2 „UNGÜLTIG“:** Es wurde nichts gesendet. Den Grund zeigt die Seite: Signatur fehlt, falscher Schlüssel, Hashtype ≠ 0x01 oder ein von der Wallet verändertes Feld. Das Ergebnis ist der eigentliche Befund der Probe.
- **Stufe 2 scheitert bei einer Wallet (verweigert, ungültig):** Mit der anderen Wallet geht es nicht, denn der Tresor gehört dem Schlüssel des ersten Kontos. Das **Sicherheitsnetz** greift nach dem Termin, also eine Stunde nach der Anlage. Jeder darf dann die eine Zahlung auslösen, ohne Signatur:
  Seite → „Zahlung prüfen“, dann „Zahlung senden“ (oder `ghostctl wallet probe-pay --probe probe.json --send`).
  1 KAS geht an **deine** Adresse, weil der Empfänger deine Adresse ist. Die Gebühr kommt aus dem Tresor (gemessen 0,0026 KAS). Etwa **0,497 KAS bleiben im Tresor**, bis eine Kündigung mit einer Wallet-Signatur desselben Kontos gelingt. Das kann später mit einer neueren Wallet-Version sein, oder mit ghostctl, falls du den Schlüssel des Kontos einmal als Datei hast (`tresor cancel` kennt den Probe-Tresor aber nicht: dann `wallet export-unsigned tresor-cancel` und selbst signieren). Verloren ist dieser Rest nicht, aber gebunden.
  Hinweis: `ghostctl tresor pay` löst die Probe-Zahlung **nicht** aus, denn es lässt immer mindestens 1 KAS im Tresor. Darum gibt es `wallet probe-pay`.
- **Ein Fremder löst die Zahlung aus:** Das ist erlaubt, die 1 KAS gehen trotzdem an dich. Der Auslöser darf höchstens den ungenutzten Teil der Höchstgebühr (unter 0,004 KAS) behalten.
- **Zeitüberschreitung beim Senden:** Ob gesendet wurde, ist unklar. Erst im Explorer nach der Adresse schauen und nicht einfach erneut senden. ghostctl prüft vor jedem Senden, ob die Eingänge noch unverbraucht sind.
- **Probe-Tresor vergessen** (Browserdaten gelöscht): Die gesicherte Datei unter Stufe 2 einfügen („Probe-Datei übernehmen“).

## Was technisch geprüft wird (`attach-sigs`)

1. Der Plan geht durch den Browser. Deshalb prüft ghostctl ihn erneut: Alle Ausgänge gehen an das Wallet-Konto oder in dessen Probe-Tresor, alle Eingänge gehören dem Konto.
2. Die Antwort der Wallet muss bis auf die Signaturskripte bitgleich zur unsignierten Tx sein. Compute-Budget und Speichermasse dürfen abweichen, weil der v1-Sighash sie nicht abdeckt. Beide werden überschrieben.
3. Je Eingang wird der erste 65-Byte-Push gelesen und der Hashtype 0x01 (ALL) verlangt. Die Schnorr-Signatur wird gegen den Schlüssel der Adresse und den Sighash geprüft.
4. Daraus baut ghostctl das SilverScript-Signaturskript (`cancel(sig)` plus Redeem-Skript) bzw. das P2PK-Signaturskript. Anschließend misst es die Skript-Einheiten erneut und setzt Budgets und Speichermasse. Dann prüft es lokal wie der Konsens: Skripte, Masse, Mindestgebühr und Blockgrenzen.
5. Gesendet wird nur mit `--send` und nur über ghostctl (eigener Node bzw. Resolver), nie über die Wallet (`pushTx`/`signAndBroadcastTx` werden nicht aufgerufen).
