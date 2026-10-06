// Audit 12: Dauerauftrag mit Tresor auf der Seite (StandingOrders.tsx,
// TresorList.tsx). Übernommen aus der Prüfung (dort hielten die Tests die
// Befunde A12-6, A12-7, A12-9, A12-10 und A12-18 fest) und auf die Behebung
// umgeschrieben.
//
// Gerendert wird mit react-dom/server. useState wird je Aufrufnummer
// überschrieben, damit Zustände wie „Probelauf geprüft“ ohne Klicks entstehen.
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { KeyEntry } from "../lib/api";
import { localToday } from "../lib/abo";
import { cliDecimal } from "../lib/commands";
import { setLangGlobal } from "../lib/i18n";
import { tresorParams, type Tresor } from "../lib/tresor";
import type { Abo } from "../lib/abo";

const g = globalThis as unknown as { __over: Record<number, unknown>; __n: number; __net: string };

vi.mock("react", async (orig) => {
  const R = await orig<typeof import("react")>();
  const useState = ((init: unknown) => {
    const i = g.__n++;
    return i in g.__over ? [g.__over[i], () => {}] : R.useState(init);
  }) as typeof R.useState;
  return { ...R, default: R, useState };
});
const me: KeyEntry = { file: "keys/tn10-user.json", type: "key", xonly: "aa".repeat(32), address: "kaspatest:qown", kas: 500, ghost: 0, vaults: [] };
vi.mock("../lib/AccountContext", () => ({ useAccount: () => ({ selected: me, keys: [me] }) }));
vi.mock("../lib/StatusContext", () => ({ useStatus: () => ({ network: g.__net }) }));

const { StandingOrders } = await import("./StandingOrders");
const { TresorList } = await import("./TresorList");

// Node übernimmt eine zur Laufzeit gesetzte Zeitzone (process.env.TZ)
const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
const tz = env.TZ;

beforeEach(() => {
  setLangGlobal("de");
  g.__over = {};
  g.__n = 0;
  g.__net = "testnet-10";
});
afterEach(() => {
  if (tz === undefined) delete env.TZ;
  else env.TZ = tz;
});

const XONLY_TO = "bb".repeat(32); // Empfänger als x-only-Schlüssel: gilt in jedem Netz
const START = "2031-01-31";
// useState-Reihenfolge in StandingOrders: 3 exec, 5 check, 11 toFree, 12 amountStr, 15 start, 16 endMode
const tresorForm = (check: unknown) => ({ 3: "tresor", 5: check, 11: XONLY_TO, 12: "10", 15: START, 16: "count" });
const renderOrders = () => {
  g.__n = 0;
  return renderToStaticMarkup(createElement(StandingOrders));
};

const t0: Tresor = {
  id: "abababab",
  covenantId: "ab".repeat(32),
  owner: "cc".repeat(32),
  recipient: me.xonly,
  ownerAddress: "kaspatest:qqabsender000000000000000000000000000000000000000000",
  recipientAddress: me.address,
  amount: "10",
  maxFee: "0.01",
  anchorDay: 31,
  periodMs: 0,
  nextDue: Date.UTC(2027, 0, 31),
  left: 3,
  value: "50",
  covered: 4,
  outpoint: "x:1",
  message: "Miete Oktober",
  onchain: false,
  encrypted: true,
  key: null,
  created: "",
  ended: null,
  missing: null,
  lastError: null,
  due: false,
  history: [],
  code: "ghost-tresor:2:x",
};

describe("A12-6: Probelauf gilt nur im Netz, in dem er lief, und nennt alles, was angelegt wird", () => {
  const form = { to: XONLY_TO, amount: 10n * 100_000_000n, amountText: "10", interval: "monthly" as const, start: START, endMode: "count" as const, count: "12", fund: null, fundText: "", message: "", onchain: false };
  const built = tresorParams(form, me, (u) => cliDecimal(u), localToday());
  // so, wie ghostctl den Tresor im Probelauf beschreibt (tresor_json)
  const dry: Tresor = { ...t0, owner: me.xonly, recipient: XONLY_TO, recipientAddress: "kaspa:qqempfaenger0000000000000000000000000000000000000000000", nextDue: Date.UTC(2031, 0, 31), left: 12, key: me.file };

  it("ein im Testnetz geprüfter Tresor zeigt im Mainnet wieder „Tresor prüfen“", () => {
    expect(built.problem).toBeNull();
    // alter Schlüssel (nur die Parameter) und neuer mit Testnetz: beide gelten im Mainnet nicht
    for (const key of [JSON.stringify(built.params), JSON.stringify({ network: "testnet-10", params: built.params })]) {
      g.__net = "mainnet";
      g.__over = tresorForm({ key, fee: 0.0012, fund: String(built.params!.fund), tresor: dry });
      const h = renderOrders();
      expect(h).toContain("Tresor prüfen");
      expect(h).not.toContain("Jetzt anlegen");
      expect(h).not.toContain("Geprüft im");
    }
  });

  it("Bestätigung im selben Netz: Empfänger, Betrag je Zahlung, Intervall, Anzahl, erster Termin, Höchstgebühr", () => {
    g.__net = "mainnet";
    g.__over = tresorForm({ key: JSON.stringify({ network: "mainnet", params: built.params }), fee: 0.0012, fund: String(built.params!.fund), tresor: dry });
    const h = renderOrders();
    expect(h).toContain("Jetzt anlegen");
    expect(h).toContain("callout-warn"); // Mainnet
    expect(h).toContain("Geprüft im Mainnet");
    const text = h.match(/121,12 KAS gehen in den Tresor[^<]*/)?.[0] ?? "";
    expect(text).toContain(dry.recipientAddress);
    expect(text).toContain("10 KAS monatlich am 31.");
    expect(text).toContain("12-mal");
    expect(text).toMatch(/erste Zahlung ab .*\(31\.01\.2031, 00:00 UTC\)/);
    expect(text).toContain("höchstens 0,01 KAS Gebühr");
    expect(text).toContain("Netzgebühr 0,0012 KAS");
    // A12-1 im Vertrag: die Bestätigung sagt, welche Nachricht fest gebunden wird
    expect(text).toContain("Jede Zahlung trägt die Nachricht „Miete Oktober“ (verschlüsselt).");
  });

  it("A12-1 im Vertrag: Bestätigung nennt die gebundene Nachricht und dass sie sich nicht ändern lässt", () => {
    g.__net = "mainnet";
    const mit = { ...dry, message: "Miete Mai", onchain: false };
    g.__over = tresorForm({ key: JSON.stringify({ network: "mainnet", params: built.params }), fee: 0.0012, fund: String(built.params!.fund), tresor: mit });
    const text = (renderOrders().replaceAll("&quot;", '"').match(/121,12 KAS gehen in den Tresor[^<]*/)?.[0] ?? "");
    expect(text).toContain("Jede Zahlung trägt die Nachricht „Miete Mai“ (verschlüsselt). Sie ist im Vertrag fest gebunden: Wer eine Zahlung auslöst, kann sie nicht ändern, und ändern lässt sie sich nur mit einem neuen Tresor.");
    g.__over = tresorForm({ key: JSON.stringify({ network: "mainnet", params: built.params }), fee: 0.0012, fund: String(built.params!.fund), tresor: { ...mit, onchain: true } });
    expect(renderOrders()).toContain("„Miete Mai“ (öffentlich)");
    g.__over = tresorForm({ key: JSON.stringify({ network: "mainnet", params: built.params }), fee: 0.0012, fund: String(built.params!.fund), tresor: { ...mit, message: "" } });
    expect(renderOrders()).toContain("Die Zahlungen tragen keine Nachricht; im Vertrag ist das fest, nachträglich lässt sich keine hinzufügen.");
  });
});

describe("A12-9: Termine 00:00 UTC in Ortszeit mit Datum, je Termin", () => {
  it("New York: Vortag 19:00, nach der Zeitumstellung 20:00", () => {
    env.TZ = "America/New_York";
    g.__over = tresorForm(null);
    const h = renderOrders();
    expect(h).toContain("Nächste Termine (deine Ortszeit)");
    expect(h).toContain("30.01.2031, 19:00 · 27.02.2031, 19:00 · 30.03.2031, 20:00 · 29.04.2031, 20:00");
    expect(h).toContain("um 00:00 Uhr Weltzeit (UTC), die erste Zahlung also bei dir ab 30.01.2031, 19:00");
    expect(h).toContain("Mit Sommer- und Winterzeit verschiebt sich die Uhrzeit");
  });
  it("Berlin: 01:00 im Winter, 02:00 im Sommer", () => {
    env.TZ = "Europe/Berlin";
    g.__over = tresorForm(null);
    expect(renderOrders()).toContain("31.01.2031, 01:00 · 28.02.2031, 01:00 · 31.03.2031, 02:00 · 30.04.2031, 02:00");
  });
  it("Tresor-Liste: nächster Termin in Ortszeit und in UTC", () => {
    env.TZ = "America/New_York";
    const h = renderList(t0);
    expect(h).toContain("30.01.2027, 19:00");
    expect(h).toContain("31.01.2027, 00:00 UTC");
  });
});

function renderList(t: Tresor, account: KeyEntry | null = me, over: Record<number, unknown> = {}) {
  // useState-Reihenfolge in TresorList: 0 list, 5 confirm, 6 topup
  g.__over = { 0: { ok: true, tresore: [t] }, ...over };
  g.__n = 0;
  return renderToStaticMarkup(createElement(TresorList, { network: "testnet-10", account, isMain: false, refresh: 0 }));
}

describe("A12-7: „fällig“ nur, wenn ghostctl auch zahlt (1 KAS Rest)", () => {
  // Nachprüfung A12-8: „aufgebraucht“ war nur aus Sicht von ghostctl richtig –
  // laut Vertrag darf ein fremder Auslöser hier noch zahlen und 0,49 KAS lassen
  it("10,5 KAS für 10 KAS: knapp, kein Knopf; ein fremder Auslöser könnte noch zahlen", () => {
    const h = renderList({ ...t0, value: "10.5", covered: 0, due: true });
    expect(h).toContain("Guthaben knapp");
    expect(h).not.toContain("Guthaben aufgebraucht");
    expect(h).not.toContain("Zahlung fällig");
    expect(h).not.toContain("Fällige Zahlung abholen");
    expect(h).toContain("ghostctl und diese Seite lösen erst aus, wenn der Absender auffüllt");
    const owner = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), value: "10.5", covered: 0, due: true });
    expect(owner).toContain("Das Guthaben reicht nicht mehr für eine Zahlung mit 1 KAS Rest: ghostctl und diese Seite zahlen erst nach dem Auffüllen.");
    expect(owner).toContain("Ein fremder Auslöser kann noch zahlen, solange Betrag und Höchstgebühr gedeckt sind, und weniger Rest lassen.");
  });
  it("genau Betrag + Höchstgebühr: aufgebraucht, laut Vertrag zahlt niemand mehr", () => {
    const owner = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), value: "10.01", covered: 0, due: true });
    expect(owner).toContain("Guthaben aufgebraucht");
    expect(owner).toContain("laut Vertrag kann niemand mehr zahlen");
  });
  it("11,005 KAS: fällig, aber nur mit eigener Gebühr – ohne Schlüssel kein Knopf", () => {
    const t = { ...t0, value: "11.005", covered: 0, due: true };
    const h = renderList(t);
    expect(h).toContain("Zahlung fällig");
    expect(h).toContain("abholen geht hier deshalb nur, wenn dein Schlüssel die Gebühr zahlt");
    expect(h).toContain("Fällige Zahlung abholen");
    expect(renderList(t, null)).not.toContain("Fällige Zahlung abholen");
  });
});

describe("A12-10: Abholen nur mit Rückfrage, eigene Gebühr nur ausdrücklich", () => {
  it("erster Klick fragt nur nach", () => {
    const h = renderList({ ...t0, due: true });
    expect(h).toContain("Fällige Zahlung abholen");
    expect(h).not.toContain("Wirklich abholen");
  });
  it("Rückfrage: Gebühr aus dem Tresor", () => {
    const h = renderList({ ...t0, due: true }, me, { 5: "pay-abababab" });
    expect(h).toContain("Wirklich abholen");
    expect(h).toContain("10 KAS an kaspatest:qown; die Netzgebühr (höchstens 0,01 KAS) trägt der Tresor.");
    expect(h).not.toContain("Fällige Zahlung abholen");
  });
  it("Rückfrage: Gebühr vom eigenen Schlüssel wird genannt", () => {
    const h = renderList({ ...t0, value: "11.005", covered: 0, due: true }, me, { 5: "pay-abababab" });
    expect(h).toContain("die Netzgebühr zahlt dein Schlüssel tn10-user.json");
  });
});

describe("A12-18 (Tresor-Teile) und A12-1: Höchstgebühr, Herkunft der Nachricht, geprüfter Auffüllbetrag", () => {
  it("Höchstgebühr sichtbar, samt dem, was der Auslöser behalten darf", () => {
    const h = renderList({ ...t0, maxFee: "0.1" });
    expect(h).toContain("Höchstgebühr je Zahlung");
    expect(h).toContain("0,1 KAS");
    expect(h).toContain("darf ein fremder Auslöser behalten");
  });
  it("Nachricht aus einem übernommenen Code: als Angabe des Codes gekennzeichnet", () => {
    const h = renderList(t0);
    expect(h).toMatch(/„Miete Oktober“.*laut Tresor-Code/);
    expect(h).not.toContain("Wer eine Zahlung auslöst, bestimmt deren Nachricht.");
    const own = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file });
    expect(own).not.toContain("laut Tresor-Code");
  });
  // A12-1 im Vertrag: fest gebunden – für Empfänger und Absender, kein Auslöser ändert sie
  it("Nachricht fest im Vertrag: wer auslöst, ändert sie nicht; nachträglich nur mit neuem Tresor", () => {
    const fest = "Die Nachricht ist im Vertrag fest gebunden: Jede Zahlung trägt genau sie, wer auslöst, kann sie nicht ändern.";
    const nie = "Ändern lässt sie sich auch nachträglich nicht – nur mit einem neuen Tresor.";
    // A13-tresor-1: beim Empfänger nur, wenn die verschlüsselte Nachricht mit
    // seinem Schlüssel gegen die Beschreibung geprüft ist (t0: nicht geprüft)
    expect(renderList(t0)).not.toContain(fest);
    const empf = renderList({ ...t0, messageCheck: "checked" });
    expect(empf).toContain(fest);
    expect(empf).toContain(nie);
    expect(empf).toContain("Absender und Beschreibung stammen aus dem Tresor-Code");
    const own = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file });
    expect(own).toContain(fest);
    expect(own).toContain(nie);
    expect(own).not.toContain("stammen aus dem Tresor-Code");
    expect(renderList({ ...t0, message: "" })).not.toContain(fest);
    setLangGlobal("en");
    expect(renderList({ ...t0, messageCheck: "checked" })).toContain("The message is fixed in the contract");
  });
  it("Tresor-Code: Feld zeigt das Präfix der Version 2", () => {
    expect(renderList(t0)).toContain('placeholder="ghost-tresor:2:…"');
  });
  it("Auffüllen zeigt und sendet den geprüften Betrag, nicht den Rohtext", () => {
    const h = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file }, me, { 5: "topup-abababab", 6: { abababab: "1.000,5" } });
    expect(h).toContain("Wirklich 1000,5 KAS nachlegen");
    expect(h).not.toContain("1.000,5 KAS nachlegen");
  });
});

describe("Nachprüfung A12-8: Regeln von ghostctl nicht als Vertragsgarantie", () => {
  it("Absender, Gebühr nur noch mit eigenem Schlüssel: ein fremder Auslöser darf sie weiter aus dem Tresor nehmen", () => {
    const h = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file, value: "11.005", covered: 0, due: true });
    expect(h).toContain("ghostctl und diese Seite zahlen deshalb nur noch, wenn der Auslöser die Gebühr selbst übernimmt; ein fremder Auslöser darf sie weiter aus dem Tresor nehmen.");
    expect(h).not.toContain("zahlen kann nur noch, wer die Gebühr selbst übernimmt");
  });
});

describe("Nachprüfung A12-1: Besitzer eines übernommenen Tresors nur laut Tresor-Code", () => {
  it("Empfänger sieht den Besitzer mit Herkunft der Angabe", () => {
    const h = renderList({ ...t0, message: "Miete Oktober" });
    expect(h).toContain("(laut Tresor-Code)");
    expect(h).toContain("den Besitzer eines Tresors kann beim Anlegen jeder frei eintragen");
    // die Nachricht selbst ist vom Vertrag erzwungen; nicht mehr „nur mit übernommenem Code geprüft“
    expect(h).not.toContain("für Tresore, deren Code hier übernommen ist");
    // hier angelegt (eigene Schlüsseldatei): keine solche Angabe
    const own = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file });
    expect(own).not.toContain("laut Tresor-Code)");
  });
});

describe("Nachprüfung A12-11: ältere Daueraufträge mit inzwischen verbotenen Zeichen", () => {
  const abo: Abo = {
    id: "a1",
    key: me.file,
    asset: "KAS",
    to: "kaspatest:qqempfaenger",
    amount: "10",
    message: "Miete\u2028Mai",
    onchain: true,
    interval: "monthly",
    start: "2026-01-01",
    end: null,
    count: null,
    paused: false,
    pauseReason: null,
    nextDue: "2026-10-01",
    history: [],
    created: "",
    ended: null,
    status: "aktiv",
    upcoming: [],
  };
  const renderAbos = (a: Abo) => {
    g.__over = { 0: { ok: true, abos: [a] } };
    return renderOrders();
  };
  it("die Liste zeigt, was ghostctl heute sendet", () => {
    expect(renderAbos(abo)).toContain("Enthält Zeichen, die nicht mehr gesendet werden (unsichtbar oder ohne feste Gestalt). Gesendet wird „Miete Mai“.");
    expect(renderAbos({ ...abo, message: "\ue000", onchain: false, encrypt: true })).toContain("die Zahlungen gehen ohne Nachricht");
    // erlaubter Text oder nur lokale Nachricht: kein Hinweis
    expect(renderAbos({ ...abo, message: "Miete Mai" })).not.toContain("nicht mehr gesendet");
    expect(renderAbos({ ...abo, onchain: false, encrypt: false })).not.toContain("nicht mehr gesendet");
  });
});

// Restpunkt „Import-alter-Tresor-Codes“: Ein vor der Verschärfung des Filters
// angelegter Tresor darf in der Beschreibung heute verbotene Zeichen haben.
// ghostctl übernimmt seinen Code jetzt, schickt die Beschreibung bereinigt
// (`message`) und kennzeichnet sie (`messageCleaned`); die Liste sagt das.
describe("Restpunkt: älterer Tresor-Code mit heute verbotenen Zeichen", () => {
  it("die Beschreibung erscheint bereinigt und gekennzeichnet", () => {
    const h = renderList({ ...t0, message: "Miete Mai 1⃣", messageCleaned: true });
    expect(h).toMatch(/„Miete Mai 1⃣“.*laut Tresor-Code.*bereinigt: enthielt unsichtbare oder heute unzulässige Zeichen/s);
    // nichts blieb übrig: nur das Kennzeichen
    const leer = renderList({ ...t0, message: "", messageCleaned: true });
    expect(leer).toContain("bereinigt: enthielt unsichtbare oder heute unzulässige Zeichen");
    expect(leer).not.toContain("„“");
    // heute zulässige Beschreibung oder älteres ghostctl ohne das Feld: kein Kennzeichen
    expect(renderList(t0)).not.toContain("bereinigt");
    expect(renderList({ ...t0, messageCleaned: false })).not.toContain("bereinigt");
    setLangGlobal("en");
    expect(renderList({ ...t0, messageCleaned: true })).toContain("cleaned: contained invisible or now disallowed characters");
  });
});
