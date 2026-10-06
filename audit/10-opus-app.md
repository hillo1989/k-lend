# Audit 10: Web-Oberfläche und lokale API (GHOST / K.Lend)

Stand: 29.09.2026. Geprüft wurde die Arbeitskopie `…/scratchpad/audit10-app/kaspa-lending`. Gegenstand: `app/server/api.ts` und `actions.ts`, `app/vite.config.ts`, `app/src/lib/*` (vor allem `format.ts`, `i18n.tsx`, `poolMath.ts`, `vaultMath.ts`, `precheck.ts`, `commands.ts`, `api.ts`, `StatusContext.tsx`, `AccountContext.tsx`), `app/src/components/ActionForms.tsx`, `Network.tsx`, `app/src/pages/*` und `app/src/main.tsx`. Zum Abgleich dienten `protocol/src/pool.rs`, `contracts.rs`, `contracts/*.sil` und das Startskript des Agenten.

**Methode.** Der Code wurde vollständig gelesen, die Befunde sind mit Vitest in der Kopie belegt. Ein Server gegen ein echtes Netz lief nicht, eine Transaktion wurde nicht gesendet, `ghostctl` wurde nicht ausgeführt. Für die API-Tests startet ein Testserver auf `127.0.0.1` (Port 0) mit einem **falschen** `ghostctl`. Das ist ein Shell-Skript in `app/server/.audit10-fake/`, das der Test anlegt und wieder löscht. Den Vitest-Cache legt `app/vitest.audit10.config.ts` in den Scratch-Ordner, damit nichts in `node_modules/.vite` des Originals geschrieben wird. Den Zeitstempel dort habe ich geprüft: unverändert.

**Grenze.** `protocol/src/bin/ghostctl.rs` liegt nicht in der Kopie. Aussagen darüber, was `ghostctl` selbst tut, sind deshalb als **Vermutung** markiert. Das betrifft Standard-Mindestwerte, Dateisperren, den Neuaufbau beim Senden und den Inhalt von `keys --json`.

**Ergebnis der Suite:** `npx vitest run --config vitest.audit10.config.ts` ergibt 9 Dateien und 145 Tests, alle grün, davon 17 eigene. `npx tsc --noEmit -p tsconfig.json` und `-p tsconfig.node.json` laufen ohne Fehler.

## Kurzfassung

- **Kein kritischer Befund.** Der lokale Server ist gegen Befehlsinjektion, Pfad-Traversal, CSRF und DNS-Rebinding solide gebaut. Er ruft `execFile` ohne Shell auf, prüft jeden Parameter gegen eine Positivliste und akzeptiert Schlüssel nur als `keys/<name>.json`. Host, Origin und `X-Ghost-Client` werden geprüft, CORS-Kopfzeilen gibt es nicht, und der Server bindet nur an localhost.
- **Hoch (A10-W-1): Beträge können um das 10- bis 1000-fache falsch gelesen werden.** Das liegt an der Sprache und an der Zahl der Nachkommastellen:
  - DE: `1.50` ergibt 1,5, `1.500` ergibt **1500**, `1.5000` ergibt 1,5.
  - EN: `0,001` ergibt **1**, `1,5` ergibt **15**.
  - Weder der Probelauf noch das Mainnet-Häkchen zeigen den gelesenen Betrag an (A10-W-5). Ein Tippfehler kann so unbemerkt bis zum Senden durchgehen.
- **Mittel:**
  - Nach einer Zeitüberschreitung steht „Nicht gesendet“ über „Ob gesendet wurde, ist unklar“ (A10-W-2).
  - „Als Befehl“ unterschlägt bei einer Teil-Liquidation `--ghost` (A10-W-3).
  - Einlegen und Abziehen im Pool laufen über die Web-API ohne Mindestwerte (A10-W-6).
- Dazu kommen mehrere niedrige Punkte und Infos: Cache-Race, GET-Aufrufe fremder Seiten, zu breite Node-Fehlererkennung, ungefilterte Durchreichung, eingefrorene Texte beim Sprachwechsel und einzelne falsche oder widersprüchliche Aussagen.

## Befundtabelle

| ID | Schwere | Befund | Beleg | Behebung (Kurz) |
|---|---|---|---|---|
| A10-W-1 | **hoch** | `parseUnits` liest dieselbe Eingabe je nach Sprache und Stellenzahl um den Faktor 10–1000 verschieden. Falsche Tausendergruppen werden still akzeptiert. | `format.ts:13–33`; Test `src/lib/audit10.test.ts` „A10-W-1“ (5 Fälle) | Strenger Parser je Sprache mit Gruppenprüfung. Mehrdeutiges ablehnen statt raten. Gelesenen Betrag anzeigen. |
| A10-W-2 | mittel | Bei Zeitüberschreitung (300 s) oder einem Absturz ohne JSON zeigt die Seite „Nicht gesendet“ bzw. „nichts gesendet“, obwohl der Server „unklar“ meldet. Das lädt zum erneuten Senden ein. | `api.ts:218–223`; `ActionForms.tsx:488–491` und `Swap.tsx:277–279` werten `timeout` nicht aus | Eigener Zustand „unklar“. Senden sperren, bis der Status neu geladen ist. |
| A10-W-3 | mittel | „Als Befehl“ lässt bei der Teil-Liquidation `--ghost` weg. Der kopierte Befehl liquidiert die **ganze** Schuld. | `commands.ts:188` (`liquidate: ["key","vault"]`); Test „A10-W-3“ | `FLAG_ORDER.liquidate` um `"ghost"` ergänzen und per Test an `ALLOWED` im Server koppeln. |
| A10-W-4 | niedrig | Ein Sprachwechsel während „Senden“ baut die App neu auf. Ergebnis und TXIDs sind dann weg, nur `logTx` läuft noch. Wer nichts mehr sieht, sendet womöglich doppelt. | `main.tsx:13–16` (`<App key={lang}>`), `Network.tsx:81–87` (Umschalter nie gesperrt) | Umschalter sperren, solange eine Aktion läuft, oder den Aktionszustand über `LangKeyed` heben. |
| A10-W-5 | mittel | Der Probelauf zeigt nur Gebühren, keinen Betrag und keinen Empfänger. Das Mainnet-Häkchen ist nicht an die Werte gebunden und bleibt bei geänderten Eingaben gesetzt. | `ActionForms.tsx:415–445` und `:61, 104–106, 181` | Zusammenfassung aus `built.params` anzeigen. Häkchen bei jeder Änderung von `sig` zurücksetzen. |
| A10-W-6 | mittel (teils Vermutung) | `pool-add` und `pool-remove` nehmen über die Web-API keinen Mindestanteil und keine Mindestauszahlung an. `pool.rs` kann beides (`min_shares`, `min_kas`, `min_ghost`). Verschiebt sich der Pool zwischen Probelauf und Senden, ist der Überschuss „geschenkt“ (Sandwich möglich). | `actions.ts:124–126`; `pool.rs:363–404`; Test „A10-W-6“ | Parameter `minShares` bzw. `minKas`/`minGhost` durchreichen, in der Oberfläche aus der Vorschau mit 1 % Spielraum setzen, in `sig` aufnehmen. |
| A10-W-7 | niedrig | Der Lese-Cache nimmt nach einer Aktion einen veralteten Status auf, bis zu 20 s (bei `nodeDown` 60 s). `clear()` löscht laufende Abrufe nicht. | `api.ts:87–107`; Test `server/audit10.server.test.ts` „A10-W-7“ | Generationszähler: Ergebnisse aus der Zeit vor `clear()` verwerfen. |
| A10-W-8 | niedrig | Eine fremde Webseite kann per GET (`<img>`, `fetch` no-cors) `ghostctl status/keys/price` auslösen. Lesen kann sie die Antwort nicht. | `actions.ts:278–290` (GET ohne Origin erlaubt); Test „A10-W-8“ | `Sec-Fetch-Site` ≠ `same-origin`/`none` ablehnen oder `X-Ghost-Client` auch für GET verlangen. |
| A10-W-9 | niedrig | `isNodeError` wertet jede Zahl 502/503/504 als Node-Ausfall. So wird aus „zu wenig GHOST: 502.0 vorhanden“ die Meldung „Nodes nicht erreichbar“. | `actions.ts:255–257`; Test „A10-W-9“ | HTTP-Codes nur im Kontext erkennen (`HTTP 502`, `status 503`), besser ein strukturiertes Feld von ghostctl. |
| A10-W-10 | niedrig | `/api/keys`, `/api/keygen` und `/api/action` reichen die ghostctl-Ausgabe ungefiltert durch, dazu die letzten 4 stderr-Zeilen. Enthielte sie je ein Geheimnis, bekäme der Browser es. Ob ghostctl so etwas ausgibt: **nicht prüfbar**. | `api.ts:121–129, 243`; Test „A10-W-10“ | Positivliste der Felder je Route, stderr kürzen und von Hex-Blöcken ≥ 64 Zeichen bereinigen. |
| A10-W-11 | niedrig | Ein erfolgreicher Probelauf verfällt nie und ist nicht an den Kettenstand gebunden. Bei „Alles tilgen“ und „Ganze Schuld“ legt ghostctl den Betrag erst beim Senden fest. | `ActionForms.tsx:155–156`, `Swap.tsx:84–85` | `checkedAt` speichern, nach z. B. 120 s verfallen lassen. Bei „alles“ den angenommenen Betrag anzeigen. |
| A10-W-12 | niedrig | Zweisprachigkeit: Die Provider (`Status`, `Account`, `Wallet`) liegen über dem `key={lang}`. Ihre mit `tr()` gebauten Fehlertexte bleiben in der alten Sprache stehen. Das Verlaufs-Label bleibt dauerhaft in der Sprache des Sendezeitpunkts. | `main.tsx:20–26`; `status.ts:78–88`, `api.ts:67–69, 76`, `WalletContext.tsx:130, 150`; `ActionForms.tsx:191` bzw. `Wallet.tsx:227` | Fehler als Code oder DE/EN-Paar speichern und beim Rendern übersetzen. Im Verlauf die Aktions-ID speichern, nicht das Label. |
| A10-W-13 | niedrig | „Der Vertrag begrenzt Preissprünge nicht.“ Der Vertrag erzwingt aber ×2/÷2 je Update und mindestens 600 DAA Abstand. Das widerspricht auch der eigenen Orakel-Seite. | `Faq.tsx:51`, `HowItWorks.tsx:224` gegenüber `risk_oracle.sil:78–87`, `Oracle.tsx:51` | Genauer formulieren (Textvorschlag unten). |
| A10-W-14 | Info | Pool-Abziehen: Die Meldung verlangt „1 bis 100 %“, erlaubt sind aber alle Werte > 0 (Server wie Vorprüfung). | `precheck.ts:235–236`, `actions.ts:204–206`; Test „A10-W-14“ | Text auf „mehr als 0 bis 100 %“ ändern. |
| A10-W-15 | Info | Widerspruch Tausch: „Tauscht jemand vorher, scheitert deine“ (FAQ, Pool-Karte) gegenüber „ändert sich der Betrag; unter dem Mindestbetrag bricht ghostctl ab“ (Probelauf-Hinweis). | `Faq.tsx:86`, `Swap.tsx:362` gegenüber `Swap.tsx:229–231` | Beides nennen (Textvorschlag unten). |
| A10-W-16 | Info | „GHOST im Umlauf (Summe aller Schulden)“: Nach einer ausgebuchten Restschuld sind mehr GHOST im Umlauf als Schuld besteht. Gezählt werden außerdem nur lokal bekannte Vaults. | `Statistics.tsx:77`; `stable_vault.sil:259–274` | Beschriften als „Schuld aller bekannten Vaults“ oder Token-Summe anzeigen. |
| A10-W-17 | Info | Der Hilfetext „je Vault höchstens 50 GHOST“ ist fest eingetragen. Die Grenze ist ein Deploy-Parameter (`max_debt`, Testnetz unbegrenzt) und gilt laut Vertrag nur beim Prägen. | `commands.ts:52–53` gegenüber `contracts.rs:189–199`, `stable_vault.sil:227–229` | Live-Wert `status.params.maxDebtGhost` einsetzen und „beim Prägen“ ergänzen. |
| A10-W-18 | Info | Kleinigkeiten, jeweils mit eigener Behebung, siehe die Liste in A10-W-18 unten. | siehe unten | siehe unten |
| A10-W-19 | Info / offen | Nicht prüfbar ohne `ghostctl.rs`: Lesende Aufrufe laufen parallel zu sendenden (belegt). Ob ghostctl die Zustandsdatei sperrt, ist offen, ebenso die Abstimmung mit dem parallel laufenden Agenten. | Test „A10-W-7“, 2. Fall (`overlap: true`) | Im Server Lese- und Schreibaufrufe gegenseitig ausschließen, oder die Sperre in ghostctl dokumentieren und testen. |

## Einzelbefunde

### A10-W-1: Mehrdeutige Betragseingabe (hoch)

**Ort:** `app/src/lib/format.ts:13–33`. Genutzt in `ActionForms.tsx:113–116`, `Swap.tsx:49`, `Wallet.tsx:54` und `Vault.tsx`.

**Hergang:**

- **Englisch:** Jedes Komma wird ersatzlos gelöscht (`s.replace(/,/g, "")`), ohne zu prüfen, ob Dreiergruppen vorliegen.
- **Deutsch ohne Komma:** Ein einzelner Punkt gilt als Dezimalpunkt, *außer* es folgen genau drei Ziffern und die Eingabe beginnt nicht mit `0.`. Dann ist er ein Tausenderpunkt. Bei mehreren Punkten wird ohne Gruppenprüfung alles zusammengezogen.

Belegt in `src/lib/audit10.test.ts`:

| Sprache | Eingabe | Ergebnis | gemeint vermutlich |
|---|---|---|---|
| DE | `1.50` / `1.5000` | 1,5 | 1,5 |
| DE | `1.500` | **1500** | 1,5 (Faktor 1000) |
| DE | `1.2.3` / `1.0.5` | 123 / 105 | ungültig |
| EN | `0,001` | **1** | 0,001 (Faktor 1000) |
| EN | `1,5` / `0,5` | **15** / **5** | 1,5 / 0,5 (Faktor 10) |
| DE ↔ EN | `1,000` | 1 ↔ 1000 | – |

Der Server prüft nur die Form (`AMOUNT_RE`) und reicht `--kas 1` unverändert an ghostctl weiter (Test, 5. Fall). Schutz davor gibt es nur durch eine Guthabenwarnung, und die greift nur bei „send“, „deposit“ und „open-vault“, wenn das Guthaben nicht reicht. Der Probelauf zeigt den Betrag nicht (A10-W-5).

**Wirkung:** Irreversible Fehlsendung von KAS oder GHOST, Einlage oder Tausch mit dem 10- bis 1000-fachen Betrag, sofern das Guthaben reicht. Der Wechsel DE/EN ist ungefährlich, weil die App neu aufgebaut wird und die Eingaben leert. Gefährlich sind die Tippgewohnheit der jeweils anderen Sprache und die Regel „drei Ziffern nach dem Punkt“.

**Behebung:**

```ts
// format.ts – streng je Sprache; Mehrdeutiges → null (Feld wird rot, Hinweis nennt das Format)
export function parseUnits(input: string, decimals: number, lang: Lang = getLang()): bigint | null {
  const s0 = input.trim().replace(/[\s_  ']/g, "");
  if (s0 === "") return null;
  const re =
    lang === "en"
      ? /^(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.(\d*))?$/ // 1,234.56 oder 1234.56
      : /^(?:\d{1,3}(?:\.\d{3})+|\d+)(?:,(\d*))?$/; // 1.234,56 oder 1234,56
  const m = re.exec(s0);
  if (!m) return null; // z. B. EN "0,001", "1,5"; DE "1.2.3", "1.50"
  const whole = s0.split(lang === "en" ? "." : ",")[0].replace(/[.,]/g, "");
  const frac = m[1] ?? "";
  if (frac.length > decimals) return null;
  return BigInt(whole || "0") * 10n ** BigInt(decimals) + BigInt(frac.padEnd(decimals, "0") || "0");
}
```

Wenn DE-Nutzer weiter `0.5` tippen können sollen: `^\d+\.\d{1,2}$|^\d+\.\d{4,}$` als Dezimalpunkt zulassen, `\d+\.\d{3}` aber **ablehnen**, mit dem Hinweis „mehrdeutig: 1,500 oder 1.500,00 schreiben“, statt zu raten. Zusätzlich unter jedem Betragsfeld den gelesenen Wert zeigen:

```tsx
{amount !== null && <p className="small muted">= {formatUnits(amount, 8, 8)} {meta.amount.unit}</p>}
```

Und Tests für EN ergänzen. Bisher prüft `format.test.ts` nur DE.

### A10-W-2: „Nicht gesendet“ bei unklarem Ausgang (mittel)

**Ort:** `api.ts:218–219` liefert `{ ok:false, timeout:true, error:"… Ob gesendet wurde, ist unklar …" }`. `ActionForms.tsx:476–491` wertet `timeout` nicht aus. Ohne `transactions` zeigt die Seite als Überschrift **„Nicht gesendet“** bzw. **„Nodes nicht erreichbar – nichts gesendet“** (`sent.nodeDown`). Dasselbe gilt für den Pfad „ghostctl lieferte kein JSON“ (`api.ts:221–223`), etwa bei einem Absturz *nach* dem Einreichen. `Swap.tsx:277–279` zeigt „Nicht getauscht“.

**Wirkung:** Die Überschrift widerspricht dem Text. Wer ihr glaubt, prüft und sendet erneut, was möglicherweise eine doppelte Überweisung ergibt. Ob das Journal von ghostctl eine Doppelsendung verhindert, ist ohne `ghostctl.rs` nicht prüfbar.

**Behebung:**

```ts
// api.ts – Aktion ohne JSON oder mit Zeitüberschreitung ist bei !dryRun „unklar“
if (r.timedOut) return send(res, 200, { ok: false, unclear: !built.dryRun, timeout: true, error: "…" });
if (!j) return send(res, 200, { ok: false, unclear: !built.dryRun, error: detail, ...(isNodeError(detail) ? { nodeDown: true } : {}) });
```

```tsx
// ActionForms.tsx / Swap.tsx
const unclear = sent && !sent.ok && (sent.timeout || sent.unclear);
<strong>{unclear ? tr("Ergebnis unklar – NICHT erneut senden. Erst „Neu laden“ und Guthaben prüfen.", "Outcome unclear – do NOT send again. Reload and check balances first.") : …}</strong>
// und Senden sperren, bis nach dem Vorfall ein neuer Status geladen wurde:
const [lockUntilStatus, setLockUntilStatus] = useState<number | null>(null); // updatedAt zum Zeitpunkt des Vorfalls
const blocked = staleVault || nodeDown || (lockUntilStatus !== null && (updatedAt ?? 0) <= lockUntilStatus);
```

### A10-W-3: „Als Befehl“ bei Teil-Liquidation (mittel)

`commands.ts:188` hat `liquidate: ["key", "vault"]`, der Server dagegen `["key", "vault", "ghost"]` (`actions.ts:120`). Der angezeigte und kopierbare Befehl ist also eine **Voll**-Liquidation, obwohl das Formular einen Teilbetrag hat (Test „A10-W-3“). Ohne `--ja` fragt ghostctl im Mainnet zwar nach, der Betrag steht aber nicht im kopierten Befehl.

```ts
// commands.ts
liquidate: ["key", "vault", "ghost"],
// Regressionstest: FLAG_ORDER und server/actions.ts ALLOWED müssen gleich sein
import { ALLOWED } from "../../server/actions"; // ALLOWED dafür exportieren
it("FLAG_ORDER = ALLOWED", () => { for (const a of Object.keys(ALLOWED)) expect(FLAG_ORDER[a]).toEqual(ALLOWED[a]); });
```

### A10-W-4: Sprachwechsel während des Sendens (niedrig)

`<App key={lang}>` baut beim Umschalten alles unter `App` neu auf (`main.tsx:13–16`). Der Umschalter in `Network.tsx:81–87` ist auch bei `phase === "sending"` aktiv. Das laufende `exec()` endet danach in einer abgebauten Komponente: `setSent` geht ins Leere, die TXIDs werden nicht angezeigt. Nur `logTx` wird geschrieben, und das auch nur bei `ok`. Der Server ist bis zum Ende „busy“ (409), danach ist ein neuer Probelauf und ein neues Senden möglich.

```tsx
// main.tsx: Aktionszustand oberhalb von LangKeyed
export const BusyCtx = createContext<{ busy: boolean; setBusy(b: boolean): void }>({ busy: false, setBusy() {} });
// ActionForms/Swap: setBusy(true) vor runAction, setBusy(false) im finally
// Network.tsx:
<button … disabled={busy} title={busy ? tr("Während einer Aktion nicht möglich", "Not possible during an action") : undefined}>
// zusätzlich: window.onbeforeunload setzen, solange busy
```

### A10-W-5: Probelauf und Bestätigung zeigen den Betrag nicht (mittel)

Die Kopplung selbst ist korrekt. `sig = JSON.stringify([network, action, built.params])` enthält Netz, Aktion, Schlüsseldatei, Vault, Empfänger und alle Beträge als ghostctl-Dezimaltext. `exec()` hält `sig` und `params` aus derselben Darstellung fest, und das Formular ist während des Laufs gesperrt. Ein Netzwechsel setzt `checked` zurück, und eine späte Antwort eines alten Probelaufs passt nicht mehr zu `sig`. Einen Weg, mit *anderen Parametern* als den geprüften zu senden, habe ich nicht gefunden.

Das Problem ist die Anzeige. Das Ergebnisfeld (`ActionForms.tsx:415–434`) nennt Aktion, Gebühr sowie Ein- und Ausgänge, aber nicht **was** an **wen** geht. Das Mainnet-Häkchen (`:439–444`) hat einen festen Text und wird nur beim Netzwechsel und nach dem Senden zurückgesetzt, nicht bei geänderten Eingaben. Zusammen mit A10-W-1 prüft der Nutzer damit nie den tatsächlich gelesenen Betrag.

```tsx
const summary = built.params && [
  meta.amount && !useFull ? `${formatUnits(amount!, 8, 8)} ${meta.amount.unit}` : hasFullMode ? tr("ganze Schuld", "whole debt") : null,
  built.params.to ? `→ ${built.params.to}` : null,
  built.params.vault !== undefined ? `Vault ${built.params.vault}` : null,
].filter(Boolean).join(" · ");
// im Probelauf-Ergebnis: <p><strong>{summary}</strong></p>
// Häkchen-Text: tr(`Ich sende ${summary} im Mainnet …`, …)
useEffect(() => setConfirmMain(false), [sig]); // Bestätigung an genau diese Werte binden
```

### A10-W-6: Pool einlegen und abziehen ohne Mindestwerte (mittel, teils Vermutung)

**Belegt:** Die Web-API lässt für `pool-add` nur `key/kas/ghost` zu, für `pool-remove` nur `key/percent` (`actions.ts:124–126`). Ein Mindestwert wird mit „Unerwarteter Parameter“ abgelehnt (Test „A10-W-6“). `pool.rs` hat dagegen `add(…, min_shares)` mit der Meldung „Der Pool hat sich verschoben“ und `remove(…, min_kas, min_ghost)`.

**Vermutung**, da `ghostctl.rs` fehlt: Ohne Option setzt ghostctl den Mindestwert aus dem Stand *beim Senden*. Audit 9 beschreibt das für `swap` als „1 % unter dem eigenen Angebot, rechnerisch immer erfüllt“. Dann schützt der Mindestwert nicht gegen eine Verschiebung zwischen Probelauf und Senden. Laut Hilfetext gilt beim Einlegen: „was darüber hinausgeht, ist geschenkt“. Wer den Kurs vorher verschiebt, kann einen Teil dieses Geschenks abschöpfen. Die Oberfläche sieht davon nichts, weil `sig` den Pool-Stand nicht enthält.

```ts
// actions.ts
"pool-add": ["key", "kas", "ghost", "minShares"],
"pool-remove": ["key", "percent", "minKas", "minGhost"],
// …
case "pool-add":
  args.push("--kas", checkAmount(params.kas, "KAS-Betrag"), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
  if (!has("minShares") || !/^\d{1,19}$/.test(String(params.minShares))) throw new ValidationError("Mindestanteile fehlen.");
  args.push("--min-shares", String(params.minShares));
  break;
```

In `ActionForms` dann `minShares = sharesForDeposit(...) * 99n / 100n` aus derselben Vorschau wie in `precheck.ts:211` berechnen, in `built.params` aufnehmen (damit in `sig`) und anzeigen. Für `pool-remove` entsprechend `payoutFor` mit 1 % Abschlag. Die Optionen `--min-shares`, `--min-kas` und `--min-ghost` muss es in ghostctl geben oder sie müssen ergänzt werden.

### A10-W-7: Veralteter Status nach einer Aktion (niedrig)

`cached().clear()` leert nur `store`, nicht `inflight`. Ein Statusabruf, der *vor* der Aktion begann, schreibt sein altes Ergebnis *nach* `clear()` in den Cache. Die nächsten 20 s (bei `nodeDown` 60 s) liefert `/api/status` dann den Stand vor der Aktion. Das ist mit dem falschen ghostctl belegt (`seq` bleibt alt). Folgen: Guthaben und Vaults werden kurz falsch angezeigt, und die Tausch-Vorschau und `min` rechnen mit alten Reserven. Das Senden bleibt durch den Neuaufbau in ghostctl und `min` geschützt.

```ts
function cached(ttl: number, fetcher: (n: Network) => Promise<string>) {
  const store = new Map<Network, { at: number; body: string; ttl: number }>();
  const inflight = new Map<Network, { p: Promise<string>; gen: number }>();
  let gen = 0;
  return {
    async get(n: Network) {
      const hit = store.get(n);
      if (hit && Date.now() - hit.at < hit.ttl) return hit.body;
      let f = inflight.get(n);
      if (!f || f.gen !== gen) {
        const g = gen;
        const p = fetcher(n).finally(() => { if (inflight.get(n)?.p === p) inflight.delete(n); });
        f = { p, gen: g };
        inflight.set(n, f);
      }
      const body = await f.p;
      if (f.gen === gen) store.set(n, { at: Date.now(), body, ttl: /* wie bisher */ ttl });
      return body;
    },
    clear() { gen++; store.clear(); inflight.clear(); },
  };
}
```

### A10-W-8: GET-Anfragen fremder Seiten (niedrig)

Browser senden bei `no-cors`-GET (etwa `<img src="http://localhost:5173/api/keys">`) keinen `Origin`. Host ist `localhost:<port>`, also erlaubt `checkRequest` die Anfrage. Folge: ghostctl startet (status, keys, price mit 6 externen Preisquellen). Durch den Cache geschieht das höchstens alle 10–60 s je Route und Netz. Lesen kann die fremde Seite die Antwort nicht: Es gibt keine CORS-Kopfzeilen, der Typ ist `application/json` mit `nosniff`, und JSON-Hijacking scheitert an der Objekt-Syntax. Belegt mit `Sec-Fetch-Site: cross-site`: 200, und ghostctl wurde aufgerufen.

```ts
// actions.ts checkRequest – RequestMeta um fetchSite ergänzen (h["sec-fetch-site"])
if (m.fetchSite !== undefined && m.fetchSite !== "same-origin" && m.fetchSite !== "none")
  return { status: 403, error: "Fremde Herkunft abgelehnt." };
```

Alternativ `X-Ghost-Client: 1` auch für GET verlangen. `fetchStatus`, `fetchKeys` und der Preisabruf müssten die Kopfzeile dann mitsenden. Für fremde Seiten löst sie einen Preflight aus, der scheitert.

### A10-W-9: `isNodeError` zu breit (niedrig)

`/\b50[234]\b/` trifft jede Zahl 502, 503 oder 504, auch Beträge und Vault-Nummern (Test). Die fachliche Meldung wird dann durch „Öffentliche Kaspa-Nodes nicht erreichbar“ ersetzt. Die Seite zeigt „Nodes nicht erreichbar – nichts gesendet“, die Originalmeldung steht nur klein unter „Technisch“. `/timed? ?out|nicht erreichbar/` ist ähnlich breit.

```ts
export function isNodeError(msg: string): boolean {
  return /Verbindung fehlgeschlagen|WebSocket|wRPC|Connection (timeout|refused|reset)|resolver|(?:HTTP|status(?: code)?)[ /:]*50[234]\b|Bad Gateway|timed out/i.test(msg);
}
```

Besser wäre ein strukturiertes Feld `errorKind: "node"` aus ghostctl.

### A10-W-10: Ungefilterte Durchreichung (niedrig, Verteidigung in der Tiefe)

`readJson` gibt `JSON.stringify(j)` vollständig zurück. Keygen und Aktionen reichen `tail(stderr)`, also die letzten 4 Zeilen, an den Browser weiter. Belegt mit dem falschen ghostctl: das Feld `secretKey` in `keys` und eine stderr-Zeile landen in der Antwort. **Ob das echte ghostctl je Geheimes ausgibt, ist ohne `ghostctl.rs` nicht prüfbar.** In `protocol/src/*.rs` gibt es keinen Treffer für „secret“ oder „private“. Die Aussage „Der Browser sieht den Schlüssel nie“ (`ActionForms.tsx:252`) hängt damit allein an ghostctl.

```ts
const KEY_FIELDS = ["file", "type", "xonly", "address", "kas", "ghost", "vaults", "lpShares", "signers"] as const;
const pick = (o: Record<string, unknown>, ks: readonly string[]) => Object.fromEntries(ks.filter((k) => k in o).map((k) => [k, o[k]]));
// keysCache: { ok, keys: keys.map((k) => pick(k, KEY_FIELDS)), ...(nodeDown ? { nodeDown } : {}) }
// keygen:    pick(j, ["ok", "file", "xonly", "address", "error"])
const tail = (s: string) => s.trim().split("\n").slice(-4).join("\n").replace(/[0-9a-fA-F]{64,}/g, "‹hex›").slice(0, 500);
```

### A10-W-11: Probelauf ohne Verfall (niedrig)

`checked` bleibt gültig, solange `sig` gleich bleibt, auch Stunden später. Ob der Pool, das Orakel oder die Schuld sich inzwischen geändert haben, ist nicht Teil von `sig`. Bei „Alles tilgen“ und „Ganze Schuld“ fehlt der Betrag in `sig` ganz. ghostctl baut beim Senden neu, deshalb ist das keine Umgehung der Vertragsregeln. Angezeigte Gebühr und angezeigter Ausgang können aber veraltet sein.

```ts
const [checked, setChecked] = useState<{ sig: string; result: ActionResult; at: number } | null>(null);
const checkValid = checked !== null && checked.sig === sig && checked.result.ok && Date.now() - checked.at < 120_000;
// bei „alles“: v.debtGhost aus dem Status in sig aufnehmen, dann macht eine Schuldänderung den Probelauf ungültig
```

### A10-W-12: Zweisprachigkeit (niedrig)

- `LangKeyed` baut nur `App` neu auf. `StatusProvider`, `AccountProvider` und `WalletProvider` liegen darüber (`main.tsx:20–26`). Ihre Fehlertexte (`error` aus `fetchStatus`/`fetchKeys`, `WalletContext.tsx:130/150/157`) entstehen mit `tr()` beim Abruf und bleiben nach dem Umschalten in der alten Sprache. Der Status korrigiert sich nach spätestens 30 s, der Kontofehler erst beim nächsten Neuladen.
- `logTx` speichert `label: meta.label`, also den übersetzten Text. Der Verlauf (`Wallet.tsx:227`) zeigt ihn dauerhaft in der Sprache des Sendezeitpunkts.
- Texte, die für Logik verglichen werden, habe ich keine gefunden. Die Vorprüfungs-Hinweise sperren nichts. Einzige Ausnahme ist `isNodeError`, das mit deutschen und englischen Mustern auf ghostctl-Meldungen arbeitet (A10-W-9).

```ts
// Fehler als Paar speichern, beim Rendern übersetzen
type Msg = { de: string; en: string };
const t = (m: Msg | string | null) => (m === null ? null : typeof m === "string" ? m : tr(m.de, m.en));
// txlog: action speichern, Label beim Anzeigen aus actionMeta()[e.action]?.label ?? e.label
```

### A10-W-13: „Der Vertrag begrenzt Preissprünge nicht“ (niedrig)

`risk_oracle.sil:78–87` verlangt 0,00001 ≤ Preis ≤ 900 USD, `newKasUsd·2 ≥ kasUsd`, `newKasUsd ≤ 2·kasUsd` und mindestens 600 DAA Abstand. `Oracle.tsx:51` sagt das auch richtig. Die Risikohinweise in `Faq.tsx:51` und `HowItWorks.tsx:224` sagen das Gegenteil. In der Sache stimmt die Warnung, weil ×2 je Minute nach zehn Updates Faktor 1024 ergibt. Wörtlich ist sie falsch und widerspricht der eigenen Seite. Vorschlag:

> „Der Vertrag begrenzt Sprünge nur auf ×2 bzw. ÷2 je Update bei mindestens etwa 1 Minute Abstand. In wenigen Minuten ist damit jeder Preis innerhalb von 0,00001–900 USD erreichbar.“

### A10-W-14 bis A10-W-17 (Info)

- **W-14:** `precheck.ts:236` sagt „Bitte 1 bis 100 % angeben“, geprüft wird aber `0 < p ≤ 100`. Der Text sollte „mehr als 0 bis 100 %“ lauten.
- **W-15:** FAQ und Pool-Karte sagen „scheitert“, der Probelauf-Hinweis sagt „Betrag ändert sich“. Vorschlag: „Tauscht jemand zwischen Prüfen und Senden, baut ghostctl neu gegen den aktuellen Pool; liegt das Ergebnis unter dem Mindestbetrag, bricht es ab. Landet ein fremder Tausch genau während des Einreichens, scheitert deiner und du prüfst neu.“ (Vermutung zum Neuaufbau, siehe Audit 9 P-3)
- **W-16:** `stable_vault.sil` bucht bei erschöpfter Sicherheit die Restschuld aus. Die dazugehörigen GHOST bleiben im Umlauf. „Summe aller Schulden“ unterschätzt den Umlauf also genau im Krisenfall.
- **W-17:** `commands.ts:52–53` sollte `params.maxDebtGhost` verwenden. Beispiel: `cap != null ? tr(\`… je Vault darf die Schuld durch Prägen höchstens ${fmt(cap)} GHOST erreichen.\`, …) : ""`

### A10-W-18: Kleinigkeiten (Info)

1. Der Kommentar in `status.ts:36` sagt „davon 10 KAS in sompi gesperrt“. Richtig sind 1 KAS (`pool.rs` `pool_init`: S0 = 1 KAS in sompi). Die Anzeige in `Statistics.tsx:141` stimmt.
2. Netzgebühr: `Statistics.tsx:153` sagt „gemessen im Simulator“ mit 0,03–0,06. `precheck.ts:16` sagt „gemessen im Testnetz“. ARCHITEKTUR.md nennt 0,036–0,051 (Testnetz, Vault) und 0,054–0,059 (Simulator, Pool). Das sollte vereinheitlicht werden.
3. `logTx` schreibt nur bei `r.ok`. Teilweise gesendete Aktionen, etwa `pool-open` mit drei Transaktionen, fehlen im Verlauf und in „Gesendete Aktionen“. Sie sollten auch bei `transactions.some(t => t.sent)` mit einem Status-Feld protokolliert werden.
4. Die Liquidationsschwelle ist in `Statistics.tsx:38/52–55` fest auf 150 eingetragen, der Hinweis nutzt `params.liqPct`. `live.params.liqPct` verwenden.
5. `Legal.tsx:33` spricht von „nur mit Kleinstbeträgen“. Durchgesetzt wird nur die 50-GHOST-Grenze beim Prägen. Sicherheit, KAS-Sendungen und Pool-Einlagen sind unbegrenzt. Entweder als Empfehlung formulieren oder eine Obergrenze in der Oberfläche setzen.
6. `transfer` an einen 64-stelligen x-only-Schlüssel hat keine Prüfsumme. Ein Tippfehler schickt GHOST an einen fremden oder nicht nutzbaren Schlüssel. Ob ghostctl prüft, ob der Punkt auf der Kurve liegt: **nicht prüfbar**. Das Formular bietet nur Adressen an, die API akzeptiert aber Hex (`actions.ts:104`).
7. Die Hülle `ghostctl` baut beim ersten Aufruf mit `cargo build`, bevor sie `exec` ausführt. Eine Zeitüberschreitung in dieser Phase beendet nur die zsh, `cargo` läuft weiter. Bei `maxBuffer`-Überlauf meldet `run()` vermutlich fälschlich `timedOut`, weil Node das Kind dann mit SIGTERM beendet und `killed=true` setzt (aus dem Node-Verhalten abgeleitet, nicht getestet).

### A10-W-19: Parallelität (Info, offen)

Der Server schließt nur Aktionen, Keygen und Empfang gegenseitig aus (`busy`). Lesende Aufrufe laufen gleichzeitig mit sendenden (Test: `overlap: true`). Der Agent (`GHOST-Agent starten.command`) nutzt denselben Schlüssel und dieselbe Zustandsdatei in einem eigenen Prozess. Laut ARCHITEKTUR.md hat er „eine Dateisperre“. Ob auch `status` und `keys` sowie jede Aktion von ghostctl diese Sperre nehmen, lässt sich hier nicht prüfen. Empfehlung: in ghostctl eine prozessübergreifende Sperre (`flock`) auf die Zustandsdatei für *alle* Befehle, die schreiben können, auch `status` wegen `resync`, und einen Test dafür.

## Ohne Befund (geprüft)

- **Befehlsinjektion und Pfad-Traversal:** `execFile` ohne Shell. Aktion, Netz und Parameter werden gegen Positivlisten geprüft. `KEY_RE = ^keys/[A-Za-z0-9_-]+\.json$` lässt weder `..` noch `/` noch führende Bindestriche zu. Beträge sind reine Ziffern (`AMOUNT_RE`, BigInt-normalisiert). Vault ist eine Ganzzahl, Adressen und x-only-Schlüssel folgen einem Regex. Kein Argument kann mit `-` beginnen. `shellArg` quotet korrekt (bestehender Test).
- **CSRF und DNS-Rebinding:** POST braucht `X-Ghost-Client: 1` und `application/json`, was bei Fremdseiten einen Preflight erzwingt. Der OPTIONS-Preflight scheitert am Origin-Vergleich, `Origin: null` wird abgelehnt, Host muss `localhost|127.0.0.1|[::1]:<eigener Port>` sein. Belegt mit echten HTTP-Anfragen an den Testserver.
- **Mainnet-Schutz serverseitig:** `confirmMainnet === true` ist Pflicht für jedes nicht-trockene Senden im Mainnet (bestehender Test).
- **Parallele Aktionen:** Zwei gleichzeitige Aktionen ergeben 200 und 409 (Test). Zwischen `if (busy)` und `busy = true` liegt kein `await`.
- **Bindung:** `server.host` und `preview.host` stehen auf `localhost`, `cors: false`. Das Body-Limit liegt bei 10 kB.
- **Browser-Wallet nur lesend:** `wallet/providers.ts` deklariert keine Signier- oder Sendemethoden.
- **Poolrechnung:** Die Konstanten in `poolMath.ts` stimmen mit `pool.rs` überein: `POOL_FEE_BPS = 30` (0,3 %), aufgerundete Gebühr `ceil(d·30/10 000)`, `POOL_MIN_KAS = 1e8` (1 KAS), `MAX = 1e16`. `swapOk`, `ghostOut` und `kasOut` sind wortgleich (Abgleich über 400 Fälle in Audit 9). `impactBps` und `withSlippage` runden zugunsten des Nutzers auf bzw. ab und zeigen eher zu viel Kursverschiebung und einen zu niedrigen Mindestbetrag. Unkritisch.
- **Aussagen, die stimmen:**
  - 50 GHOST je Vault stimmt mit `MAINNET_MAX_DEBT = 5e9` und `require(debtOf ≤ maxDebt)` beim Prägen überein.
  - 200 %, 150 % und 10 % stimmen mit `config.PARAMS` überein, live kommen sie aus `status.params`.
  - 0,2 KAS stimmt mit `DUST` überein, „unter etwa 110 %“ ergibt sich aus 100 % plus Bonus.
  - Orakel-Grenzen 0,00001–900 USD, ×2/÷2 und 600 DAA stimmen mit `risk_oracle.sil` überein.
  - Die Agent-Beschreibung (5 Minuten, 1 %, 6 h, nur mit Komitee-Datei, Wächter-Prüfung am Marktpreis) stimmt mit dem Startskript und ARCHITEKTUR.md überein. Die Regel „Sprünge über 20 % nach 3 Runden“ ist ohne `ghostctl.rs` nicht prüfbar.
- **Anzeige:** `formatUnits` schneidet ab und rundet nicht. „Du bekommst etwa“ ist damit nie zu hoch. `de()` rundet Floats zur Anzeige, etwa eine Schuld von 49,99999999 bei 4 Stellen als „50“. Das ist nur kosmetisch und wird nicht für Beträge an ghostctl genutzt.

## Testdateien

- `app/src/lib/audit10.test.ts` enthält 9 Tests zu A10-W-1, -3, -6, -9 und -14 (Einheiten, ohne I/O).
- `app/server/audit10.server.test.ts` enthält 8 Tests: 3 Kontrollen sowie A10-W-7, -8 und -10 mit einem HTTP-Testserver auf 127.0.0.1 und einem falschen ghostctl, der unter `app/server/.audit10-fake/` angelegt und danach gelöscht wird.
- `app/vitest.audit10.config.ts` ist die Vite-Konfiguration mit Cache im Scratch-Ordner.

Ausführen (in `app/`):

```
npx vitest run --config vitest.audit10.config.ts
npx tsc --noEmit -p tsconfig.json && npx tsc --noEmit -p tsconfig.node.json
```

Mit „BEFUND“ markierte Erwartungen beschreiben den heutigen Fehler. Nach der Behebung sind sie umzudrehen.
