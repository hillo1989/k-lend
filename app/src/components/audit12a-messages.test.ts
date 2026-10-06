// Audit 12, A12-1 und A12-19: Anzeige eingegangener Nachrichten
// (IncomingMessages.tsx). Übernommen aus der Prüfung (dort hielten die Tests
// den Befund fest: echte und untergeschobene Tresor-Nachricht sahen gleich
// aus, kein Wort zur Absender-Echtheit); hier umgeschrieben auf die Behebung.
//
// Gerendert wird mit react-dom/server; der Abruf (useEffect) läuft dabei nicht,
// daher liefert ein Mock von useState die Antwort von ghostctl messages direkt.
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InboxMessage, InboxResult } from "../lib/api";
import { setLangGlobal } from "../lib/i18n";

const g = globalThis as unknown as { __inbox: InboxResult | null; __n: number };

vi.mock("react", async (orig) => {
  const R = await orig<typeof import("react")>();
  // erster useState-Aufruf der Komponente = data (Antwort von /api/messages)
  const useState = ((init: unknown) => (g.__n++ === 0 ? [g.__inbox, () => {}] : R.useState(init))) as typeof R.useState;
  return { ...R, default: R, useState };
});
vi.mock("../lib/AccountContext", () => ({ useAccount: () => ({ selected: { file: "keys/tn10-empfaenger.json" } }) }));
vi.mock("../lib/StatusContext", () => ({ useStatus: () => ({ network: "testnet-10" }) }));

const { IncomingMessages } = await import("./IncomingMessages");
/** nur die Zeilen der Tabelle (ohne die festen Hinweise darüber) */
const rows = (h: string) => h.match(/<tbody>.*<\/tbody>/s)?.[0] ?? "";
/** die einzelnen Zeilen der Tabelle, je Zahlung eine */
const rowList = (h: string) => rows(h).match(/<tr>.*?<\/tr>/gs) ?? [];

function render(messages: InboxMessage[], extra: Partial<InboxResult> = {}): string {
  g.__inbox = { ok: true, messages, limit: 200, nodeChecked: true, hidden: 0, ...extra } as InboxResult;
  g.__n = 0;
  return renderToStaticMarkup(createElement(IncomingMessages));
}

const OWNER = "kaspatest:qqabsender000000000000000000000000000000000000000000";
const AUSLOESER = "kaspatest:qqausloeser00000000000000000000000000000000000000000";
// Tresor-Zahlung eines hier übernommenen Tresors: ghostctl erkennt den Zweig pay
// samt Ausgang an mich; „von“ = Eingänge (hier keine, Gebühr aus dem Tresor),
// der Besitzer nur als Angabe des Tresor-Codes
const echt: InboxMessage = {
  txid: "11".repeat(32),
  timeMs: 1_801_353_600_000,
  at: null,
  amount: "10",
  unit: "KAS",
  from: [],
  text: "Miete Oktober",
  kind: "encrypted",
  origin: "tresor-stored",
  tresor: "abababab",
  tresorOwner: OWNER,
  source: "node",
};
// Nachricht, die nicht zum Hash im Vertrag passt: seit A12-1 im Vertrag nimmt
// das Netz so etwas nicht an, nur falsche Daten der REST-API könnten es zeigen
const fremd: InboxMessage = { ...echt, txid: "22".repeat(32), text: "Vermieter hier: neue Adresse, Nebenkosten bitte an kaspa:qqangreifer", origin: "tresor-inserted" };
// Tresor hier nicht übernommen: Nachricht vom Vertrag erzwungen, Besitzer nicht geprüft
const gebunden: InboxMessage = { ...echt, txid: "33".repeat(32), origin: "tresor-bound", tresor: null, tresorOwner: null };

beforeEach(() => setLangGlobal("de"));

describe("A12-1: Tresor-Nachrichten gegen die hinterlegte Fassung geprüft", () => {
  it("echte und untergeschobene Nachricht sehen verschieden aus", () => {
    const a = render([echt]);
    const b = render([fremd]);
    const norm = (h: string, m: InboxMessage) => h.replace(m.text!, "TEXT").replaceAll(m.txid, "TXID").replaceAll(m.txid.slice(0, 10), "TXID");
    expect(norm(a, echt)).not.toBe(norm(b, fremd));
    expect(a).toContain("vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)");
    expect(b).toContain("passt nicht zur im Vertrag gebundenen Nachricht – nicht vom Absender");
    expect(b).toMatch(/<strong class="tag tag-warn">passt nicht zur im Vertrag gebundenen Nachricht/);
    expect(a).not.toContain("nicht vom Absender");
    expect(a + b).not.toContain("beim Auslösen");
  });

  // A12-1 im Vertrag: auch ohne übernommenen Tresor-Code steht fest, dass die
  // Nachricht beim Anlegen hinterlegt wurde (vorher: „nicht prüfbar“)
  it("Tresor hier nicht übernommen: Nachricht vom Vertrag erzwungen, Besitzer nicht geprüft", () => {
    const h = rows(render([gebunden]));
    expect(h).toContain("vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)");
    expect(h).toContain("Tresor (Besitzer nicht geprüft)");
    expect(h).not.toContain("nicht prüfbar");
    expect(h).not.toContain("tag-warn");
    // übernommener Tresor, dessen Code die Nachricht anders beschreibt: erzwungen, aber gewarnt
    const anders = rows(render([{ ...echt, origin: "tresor-bound" }]));
    expect(anders).toContain("vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)");
    expect(anders).toMatch(/<strong class="tag tag-warn">weicht von der Beschreibung im Tresor-Code abababab ab/);
    setLangGlobal("en");
    expect(rows(render([gebunden]))).toContain("stored by the sender when creating the vault (enforced by the contract)");
  });

  it("anderer Vertrag: Herkunft der Nachricht nicht prüfbar", () => {
    expect(render([{ ...echt, origin: "contract", from: [] }])).toContain("über einen Vertrag – Herkunft der Nachricht nicht prüfbar");
    const direkt = rows(render([{ ...echt, origin: "direct", tresor: null, tresorOwner: null }]));
    expect(direkt).not.toMatch(/Tresor|nicht prüfbar|hinterlegt/);
  });

  // Nachprüfung A12-1: „Tresor von <Besitzer>“ ließ sich fälschen – den Besitzer
  // trägt beim Anlegen jeder frei ein, ohne dessen Signatur
  it("Besitzer nur bei übernommenem Tresor, als Angabe des Codes; „Von“ bleibt bei den Eingängen", () => {
    const h = render([echt]);
    expect(h).toContain("Tresor abababab");
    expect(h).toContain("Besitzer laut Tresor-Code");
    expect(h).toContain(`title="${OWNER}"`);
    expect(h).not.toContain("Tresor von");
    const fremd = render([{ ...gebunden, from: [] }]);
    expect(fremd).toContain("Tresor (Besitzer nicht geprüft)");
    expect(fremd).not.toContain("Besitzer laut");
    expect(fremd).not.toContain("unbekannt");
    // auch wenn ein älteres ghostctl einen Besitzer mitschickt: ohne übernommenen Tresor kein Name
    expect(render([{ ...gebunden, tresorOwner: OWNER }])).not.toContain(OWNER);
    const mitEingang = render([{ ...echt, origin: "tresor-inserted", from: [AUSLOESER] }]);
    expect(mitEingang).toContain("weitere Eingänge");
    expect(mitEingang).toContain(`title="${AUSLOESER}"`);
  });

  it("Spalte heißt „Von“, nicht „Absender“ (Hinweistext auf Wunsch des Nutzers entfernt, 05.10.2026)", () => {
    const h = render([fremd]);
    expect(h).toContain(">Von<");
    expect(h).not.toContain(">Absender<");
  });
});

describe("A12-19: Quelle der Angaben", () => {
  // Restpunkt zu A12-19: Der Fußtext unter der Tabelle enthält beide Etiketten
  // („„am Node geprüft“: Dieselbe Transaktion …“, „… stehen „laut REST-API“
  // da“). Geprüft wird deshalb je Tabellenzeile: Eine Zeile mit source "rest"
  // darf nie „am Node geprüft“ tragen und umgekehrt (Rückbau → true bzw. false).
  it("je Zahlung: am Node geprüft oder nur laut REST-API", () => {
    const node = "am Node geprüft";
    const rest = "laut REST-API";
    const zeilen = rowList(render([echt, { ...echt, txid: "33".repeat(32), source: "rest" }, { ...echt, txid: "44".repeat(32), source: "node" }]));
    expect(zeilen).toHaveLength(3);
    const etikett = (z: string) => [z.includes(node), z.includes(rest)];
    expect(zeilen.map(etikett)).toEqual([
      [true, false],
      [false, true],
      [true, false],
    ]);
    // eine einzelne REST-Zeile: nie „am Node geprüft“
    const nurRest = rowList(render([{ ...echt, source: "rest" }]));
    expect(nurRest).toHaveLength(1);
    expect(nurRest[0]).toContain(rest);
    expect(nurRest[0]).not.toContain(node);
    setLangGlobal("en");
    const en = rowList(render([echt, { ...echt, txid: "33".repeat(32), source: "rest" }]));
    expect(en.map((z) => [z.includes("checked at the node"), z.includes("per REST API")])).toEqual([
      [true, false],
      [false, true],
    ]);
  });
  it("ohne Node: ehrlich gekennzeichnet; ausgeblendete Abweichungen gemeldet", () => {
    expect(render([{ ...echt, source: "rest" }], { nodeChecked: false })).toContain("kein Node erreichbar, nichts gegengeprüft");
    expect(render([echt])).toContain("Blöcke behält ein Node nur etwa 30 bis 42 Stunden, ältere Zahlungen stehen „laut REST-API“ da");
    expect(render([echt], { hidden: 2 })).toContain("2 Nachricht(en) ausgeblendet");
    expect(render([echt])).not.toContain("ausgeblendet");
  });
  // Zweite Nachprüfung: Die Aufbewahrung am Node liegt zwischen 30 h (Pruning-
  // Tiefe) und 42 h (der Pruning-Punkt rückt in 12-h-Schritten vor), gemessen
  // ≈ 37 h – „etwa 30 Stunden“ war die Untergrenze. Und ausgeblendet wird
  // nicht nur bei abweichendem Inhalt, sondern auch bei einem Block ohne die
  // Tx und bei einem Block, den der Node kennen müsste (NotInBlock, UnknownBlock).
  it("Aufbewahrung am Node und alle Gründe fürs Ausblenden genannt", () => {
    const h = render([echt], { hidden: 1 });
    expect(h).not.toContain("etwa 30 Stunden hat ein Node");
    expect(h).not.toContain("als der Block am Node sie enthält");
    expect(h).toContain("1 Nachricht(en) ausgeblendet: Die öffentliche REST-API meldete sie anders, als der Node sie kennt");
    expect(h).toContain("mit anderem Inhalt als im Block");
    expect(h).toContain("in einem Block ohne diese Transaktion");
    expect(h).toContain("in einem Block, den der Node nicht kennt, obwohl er ihn noch haben müsste");
    setLangGlobal("en");
    const e = render([echt], { hidden: 1 });
    expect(e).toContain("A node only keeps blocks for about 30 to 42 hours");
    expect(e).toContain("in a block without this transaction");
  });
});

describe("A12-1 (hält): Text wird von React escaped", () => {
  it("HTML im Nachrichtentext wird nicht ausgeführt", () => {
    const h = render([{ ...echt, text: '<img src=x onerror=alert(1)>"&' }]);
    expect(h).not.toContain("<img");
    expect(h).toContain("&lt;img src=x onerror=alert(1)&gt;");
  });
});

describe("Nachprüfung A12-11: Zahlungen mit unzulässigen Zeichen bleiben sichtbar", () => {
  it("der Text erscheint nicht, die Herkunft schon", () => {
    const h = rows(render([{ ...echt, text: null, kind: "invalid", invalid: "chars", origin: "tresor-inserted" }]));
    expect(h).toContain("Nachricht mit unsichtbaren oder unzulässigen Zeichen – nicht angezeigt");
    expect(h).toContain("passt nicht zur im Vertrag gebundenen Nachricht – nicht vom Absender");
    expect(h).not.toContain("„");
  });

  // Restpunkt „Invalid-Text-zu-lang“: Auch Texte über 100 Zeichen werden
  // nicht angezeigt (etwa JSON einer anderen Anwendung). Die Seite nannte
  // dafür „unsichtbare oder unzulässige Zeichen“. Jetzt je Zeile der Grund,
  // den ghostctl im Feld `invalid` mitschickt.
  it("je Zeile der richtige Grund: zu lang, unzulässige Zeichen oder beides", () => {
    const inv = (txid: string, invalid: InboxMessage["invalid"]): InboxMessage => ({ ...echt, txid: txid.repeat(32), text: null, kind: "invalid", invalid, origin: "direct", tresor: null, tresorOwner: null });
    const z = rowList(render([inv("51", "length"), inv("52", "chars"), inv("53", "both"), inv("54", undefined)]));
    expect(z).toHaveLength(4);
    const lang = "zu lang (über 100 Zeichen)";
    const zeichen = "mit unsichtbaren oder unzulässigen Zeichen";
    expect(z[0]).toContain(`Nachricht ${lang} – nicht angezeigt`);
    expect(z[0]).not.toContain("unzulässig");
    expect(z[1]).toContain(`Nachricht ${zeichen} – nicht angezeigt`);
    expect(z[1]).not.toContain("zu lang");
    expect(z[2]).toContain(`Nachricht ${lang} und ${zeichen} – nicht angezeigt`);
    // älteres ghostctl ohne Grund: beide Möglichkeiten, keine falsche Festlegung
    expect(z[3]).toContain("Nachricht zu lang oder mit unzulässigen Zeichen – nicht angezeigt");
    setLangGlobal("en");
    const e = rowList(render([inv("51", "length"), inv("52", "chars")]));
    expect(e[0]).toContain("message too long (over 100 characters) – not shown");
    expect(e[0]).not.toContain("disallowed");
    expect(e[1]).toContain("message with invisible or disallowed characters – not shown");
  });
});
