# Lending-Seite

Weboberfläche für den Stablecoin **GHOST** und den geplanten KAS-Pool auf Kaspa L1. Die Seite zeigt Live-Daten der
Verträge. Protokoll-Aktionen lassen sich per Knopfdruck ausführen: Der lokale Server ruft dafür `../ghostctl` auf. Standard
ist das **Mainnet**, umschaltbar auf Testnet-10.

Unabhängiges Community-Projekt, nicht mit Kaspa verbunden. Die Seite zeigt **Protokoll-Version 2**. Der alte
Mainnet-Vault aus Version 1 ist zurückgebaut. Name, Ticker, Netze und die Annahmen für den Pool stehen in
`src/config.ts`.

> **Experimentell, nicht auditiert.** Der Testnetz-Lauf ist abgeschlossen (`../TESTNET_LOG.md`), auf dem Mainnet laufen nur
> Versuche mit Kleinstbeträgen (`../MAINNET.md`). Im Mainnet-Probelauf hält **der Betreiber alle 5 Orakel-Komitee-Schlüssel**
> (`keys/mainnet-committee.json`). Er kann damit jeden Preis setzen, jeden Vault liquidierbar machen oder ungedeckte Prägung
> erlauben. Seit Version 2 darf sich der Preis je Update höchstens verdoppeln oder halbieren und muss zwischen 0,00001 und
> 900 USD liegen. Eine Liquidation unter 150 % ist erlaubt, aber nicht garantiert, denn es laufen keine Keeper. Unter etwa
> 110 % Deckung wird die Restschuld bei der Liquidation ausgebucht. Der Zins steht auf **0 %**. Zinsmechanismus (Peg),
> Keeper, signierte Aufträge und MCP-Server sind **geplant, nicht gebaut**. Setze nur Beträge ein, die du verlieren kannst.

### Version 2 in der Oberfläche

| Regel | Umsetzung |
|---|---|
| Einzahlen nur durch den Besitzer | Vorprüfung „gehört nicht zu …“; in der Vault-Liste haben fremde Vaults nur noch „Tilgen“. |
| Teil-Tilgung ohne Anteilswirkung wird abgelehnt | Vorprüfung mit dem Live-Index. |
| Teil-Liquidation | „Ganze Schuld“ oder „Teilbetrag“. Die Vorschau zeigt die erwarteten KAS (`liquidationPreview` in `vaultMath.ts`) und warnt, wenn Restschuld ausgebucht wird. |
| Orakel-Grenzen | 0,00001–900 USD und höchstens ×2/÷2 je Update. Größere Sprünge in Schritten aktualisieren. |
| Zins 0 % | Die Texte sagen: geplant, derzeit 0 %. Eine Zins-Eingabe über 0 wird gewarnt. |
| `stale` (von Dritten verändert) | Kennzeichen am Vault. Prüfen und Senden sind dafür gesperrt. |
| `donatedKas` | als „Rest an Miner“ je Transaktion. |
| Fehler nach Teil-Sendung | Die gesendeten TXIDs bleiben sichtbar. Das Journal schließt ghostctl beim nächsten Aufruf ab. |
| Nodes nicht erreichbar | Status, Konto und Aktionen melden „Öffentliche Kaspa-Nodes nicht erreichbar – später erneut versuchen“. Die API antwortet dann mit HTTP 200 und `{ok:false, nodeDown:true}` und speichert das 60 s zwischen, weil jeder Versuch 1–2 Minuten dauert. |

## Starten

Voraussetzungen: Node.js 22 oder neuer und `../ghostctl`. Beim ersten Aufruf baut sich ghostctl mit cargo.

Am einfachsten per Doppelklick auf `GHOST-Seite öffnen.command` im Projektordner. Die Seite läuft dann auf
http://localhost:5180/.

```bash
cd app
npm install
npm run dev      # Entwicklungsserver mit lokaler API, http://localhost:5173
npm test         # Unit-Tests (Vitest)
npm run build    # Typprüfung + Produktions-Build nach dist/
npm run preview  # Build ansehen, ebenfalls mit lokaler API
```

## Benutzung

1. **Netz wählen:** oben in der Leiste Mainnet oder Testnetz.
2. **Konto:** unter „Vault“ → „Konto“ eine Schlüsseldatei aus `keys/` wählen oder unter „Neuen Schlüssel anlegen“ erzeugen.
   Dann die angezeigte Adresse aus der eigenen Wallet mit KAS aufladen. Die Seite merkt sich die Wahl je Netz.
3. **Aktion:** unter „Aktionen“ eine Aktion wählen:
   - Vault eröffnen, Prägen, Tilgen (Teil oder alles), Einzahlen, Abheben, Schließen, Liquidieren
   - GHOST senden, KAS senden
   - Orakel aktualisieren

   Aus der Vault-Liste belegen Schnellknöpfe das Formular vor.
4. **Prüfen:** Das startet einen Probelauf (`ghostctl --dry-run`). Die Transaktion wird vollständig gebaut und geprüft, aber
   nicht gesendet. Angezeigt werden Gebühr und Ergebnis oder die Fehlermeldung von ghostctl. Vorher weist die Seite schon
   mit Live-Daten und dem Rechenkern auf absehbare Probleme hin: maximal prägbare Menge, Quote nach dem Abheben, Schuld
   für die volle Tilgung, benötigte GHOST für eine Liquidation.
5. **Senden:** Der Knopf wird erst nach einem erfolgreichen Probelauf frei, und nur für genau die geprüften Werte. Im
   Mainnet muss zusätzlich das Häkchen „Ich sende echte KAS im Mainnet …“ gesetzt sein. Danach erscheinen die TXIDs, und
   Status und Guthaben werden neu geladen.
6. **„Als Befehl“** zeigt den gleichwertigen Terminal-Befehl. Ohne `--ja` fragt ghostctl im Mainnet selbst nach.

**Nachricht und Daueraufträge (Wallet).** Beim Senden von KAS und GHOST gibt es ein Feld „Nachricht (optional)“ und das
Häkchen „Nachricht öffentlich in die Transaktion schreiben (für alle sichtbar). Ohne Häkchen wird sie verschlüsselt, nur
der Empfänger kann sie lesen.“ Die Nachricht steht immer im Verlauf dieses Browsers (und in
`deployments/<netz>-txlog.jsonl`). Mit Häkchen steht sie im Klartext im Payload der Tx, ohne Häkchen verschlüsselt an
den Empfänger (nur an normale Adressen `kaspa:q…`, Schlüsseldateien und x-only-Schlüssel). Öffentlich bleibt auch dann,
dass es eine Nachricht gibt, ihre Länge, Absender, Empfänger und Betrag. Der Absender liest seine verschlüsselte
Nachricht nur im eigenen Verlauf; wer den Schlüssel des Empfängers erfährt, kann alle alten Nachrichten an ihn lesen.
Details in `MAINNET.md`. Die Karte „Eingegangene Nachrichten“ zeigt für den gewählten Schlüssel, was andere an diese
Adresse geschickt haben (`ghostctl messages`); lesbar sind verschlüsselte Nachrichten nur an Adressen, deren
Schlüsseldatei auf diesem Rechner liegt. Darunter der Bereich
„Daueraufträge“: Liste mit Empfänger, Betrag, Intervall, nächster Ausführung, Nachricht, Status und den letzten
Ausführungen samt Explorer-Link, dazu Pausieren/Fortsetzen/Beenden und ein Formular zum Anlegen mit Vorschau der nächsten
Termine. Solange der Server läuft, stößt er jede Minute `ghostctl abo run --ja` an, aber nur, wenn laut
`deployments/<netz>-abos.json` etwas fällig ist. Doppelt anstoßen (Seite und Agent) ist harmlos, siehe `MAINNET.md`.

Orakel und Liquidationen laufen dauerhaft mit dem Agenten: Doppelklick auf `GHOST-Agent starten.command`. Das gilt für
das Mainnet; das Fenster muss offen bleiben. Er aktualisiert bei 1 % Preisänderung, spätestens nach 6 h. Die Seite warnt, wenn der Preis
älter als 6,5 h ist oder ghostctl ihn als nicht frisch meldet.

## Was läuft wo? Sicherheitsmodell

| Teil | Was er tut |
|---|---|
| Browser (diese Seite) | zeigt Daten, baut Formulare. Sieht **keine** privaten Schlüssel. |
| Browser-Wallet (KasWare, Kastle) | optional und **nur lesend**: Adresse, Netz, Guthaben, öffentlicher Schlüssel. Von der Wallet wird nie eine Signatur angefordert. |
| Lokaler Server (`server/api.ts`, im Vite-Dev- bzw. Preview-Server) | ruft `../ghostctl` auf. ghostctl **signiert und sendet** mit Schlüsseldateien aus `keys/` auf diesem Rechner, und zwar immer mit `--json --ja`. |
| ghostctl | baut, prüft und sendet Transaktionen über einen öffentlichen Kaspa-Node und pflegt `deployments/<netz>.json`. |

Die API:

| Route | Zweck |
|---|---|
| `GET /api/status?network=` | `ghostctl status --json`, 20 s Cache |
| `GET /api/keys?network=` | `ghostctl --json keys`: Dateien mit Adresse und Guthaben, keine Geheimnisse, 10 s Cache |
| `POST /api/action` | `{network, action, params, dryRun, confirmMainnet}` |
| `POST /api/keygen` | `{network, name}` → `keys/<name>.json` |
| `GET /api/abos?network=` | `ghostctl --json abo list`: Daueraufträge und Archiv, ohne Node |
| `GET /api/messages?network=&key=` | `ghostctl --json messages --key keys/<name>.json`: eingegangene Nachrichten (REST-API, ohne Node), 30 s Cache; `key` wie bei Aktionen nur `keys/<name>.json`, die Datei muss existieren |

Schutzmaßnahmen in `server/actions.ts` und `server/api.ts`; die Tests liegen in `server/actions.test.ts`:

- **Nur lokal.** Der Server lauscht auf `localhost`. Jede API-Anfrage braucht den Host `localhost:<port>`, `127.0.0.1:<port>`
  oder `[::1]:<port>`, das schützt gegen DNS-Rebinding. Vite prüft fremde Hosts zusätzlich selbst.
- **Keine fremden Seiten.** Ist ein `Origin` gesetzt, muss er die eigene Herkunft sein. Jeder POST braucht
  `X-Ghost-Client: 1` und `Content-Type: application/json`. Das erzwingt bei fremden Seiten einen CORS-Preflight, und
  CORS-Kopfzeilen setzt der Server nie.
- **Strenge Argumente.** Jede Aktion hat eine feste Parameterliste, alles andere wird abgelehnt.
  - Schlüsselpfade nur als `keys/<name>.json`, die Datei muss existieren.
  - Beträge > 0 mit höchstens 8 Nachkommastellen, Vault-Nummer ganzzahlig ≥ 0.
  - Kaspa-Adressen mit dem Präfix des gewählten Netzes, x-only-Schlüssel mit 64 Hex-Zeichen.
  - Nachrichten höchstens 100 Zeichen ohne Steuer- und Formatzeichen, übergeben als `--message=<text>`.
  - Daueraufträge: `abo-add` (Asset, Empfänger, Betrag, Intervall, Datum JJJJ-MM-TT, Ende oder Anzahl, Nachricht),
    `abo-pause`/`abo-resume`/`abo-remove` nur mit einer ID aus 8 Hex-Zeichen. Letztere senden nichts und brauchen
    deshalb keine Mainnet-Bestätigung.
  - Aufruf per `execFile` ohne Shell.
- **Mainnet-Schutz.** Ohne `dryRun` verlangt der Server im Mainnet `confirmMainnet: true`, sonst antwortet er mit 400. Die
  Seite setzt das nur mit dem Häkchen.
- **Eine Aktion zur Zeit.** Eine zweite gleichzeitige Anfrage bekommt 409. Die Zeitgrenze liegt bei 300 s, der Body darf
  höchstens 10 kB groß sein.

Antwortcodes: 200 = ghostctl hat geantwortet (Erfolg `ok:true`, Ablehnung `ok:false` mit `error`, bei Verbindungsproblemen zusätzlich `nodeDown:true`, bei Zeitüberschreitung `timeout:true`) · 400 Eingabe · 403 Herkunft/Kopfzeile · 409 beschäftigt oder Datei vorhanden · 413 zu groß · 415
Content-Type.

**Wichtig:**
- **Nicht öffentlich betreiben.** Wer den Server erreicht, kann mit den Schlüsseln in `keys/` senden. Deshalb nur lokal.
- **Nur lokal zählt.** Ein statisch ausgelieferter `dist/`-Ordner hat keine API.
- **Nur bekannte Vaults.** `ghostctl status` kennt nur die Vaults aus der Zustandsdatei dieses Rechners.
- **Schlüssel parallel nutzen:** Laufen gleichzeitig `oracle-feed` oder andere ghostctl-Befehle mit demselben Schlüssel,
  können sich Transaktionen gegenseitig die UTXOs wegnehmen. Dann einfach neu prüfen und senden.

## Rechenkern

`src/lib/vaultMath.ts` spiegelt `contracts/stable_vault.sil` wörtlich wider:

- `mulDivDown`, `mulDivUp`, `debtOf`, `sharesFor`, `healthy` und die Einträge;
- die Index-Fortschreibung aus `contracts/risk_oracle.sil`.

Alles läuft in BigInt, und jede Zwischenrechnung prüft auf 64-Bit-Überlauf.

Rundung wie im Vertrag:

- Beim Prägen werden Anteile aufgerundet, die Schuld ebenfalls.
- Beim Tilgen werden die Anteile abgerundet.
- Eine Überzahlung beim Tilgen wird abgelehnt.

`src/lib/vaultMath.test.ts` vergleicht gegen exakte BigInt-Rechnung. **Ändert sich der Vertrag, muss diese Datei
nachgezogen werden.**

Die Vorprüfungen im Formular (`src/lib/precheck.ts`) nutzen diesen Kern und die Live-Daten. Sie sind Hinweise.
Verbindlich ist der Probelauf.

## Wallet-Anbindung

`src/wallet/providers.ts` nennt die geprüften Quellen (Stand 28.09.2026) und deklariert nur lesende Methoden:

- **KasWare:** `requestAccounts`, `getAccounts`, `getNetwork`, `getBalance`, `getPublicKey`, `disconnect`
- **Kastle:** `connect`, `getAccount`, `getNetwork`, `getBalance`

## Datenschutz

- Keine Tracker, keine Analyse. Die Schrift (Rubik) wird lokal ausgeliefert.
- Einziger Server ist der lokale. ghostctl verbindet sich mit einem öffentlichen Kaspa-Node.
- Im `localStorage` stehen:
  - das gewählte Netz (`gh-network`),
  - die gewählte Schlüsseldatei je Netz (`gh-key-<netz>`, nur der Dateiname),
  - welche Browser-Wallet zuletzt verbunden war.
