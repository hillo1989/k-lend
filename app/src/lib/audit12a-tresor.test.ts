// Audit 12: Tresor-Logik der Seite (tresor.ts) nach der Behebung von A12-7,
// A12-8, A12-9, A12-10 und den Tresor-Teilen von A12-18. Übernommen aus der
// Prüfung (lib/audit12.test.ts, dort als Befund festgehalten) und umgeschrieben.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { checkAmount } from "../../server/actions";
import type { KeyEntry } from "./api";
import { setLangGlobal } from "./i18n";
import { contractPayable, dueMs, feeSource, localDateTime, payParams, topupKas, tresorHints, tresorStatus, tresorStatusLabel, utcDateTime, type Tresor, type TresorForm } from "./tresor";

const E8 = 100_000_000n;
beforeEach(() => setLangGlobal("de"));

const key: KeyEntry = { file: "keys/tn10-user.json", type: "key", xonly: "aa".repeat(32), address: "kaspatest:qown", kas: 500, ghost: 0, vaults: [] };
const form = (x: Partial<TresorForm> = {}): TresorForm => ({
  to: "bb".repeat(32),
  amount: 10n * E8,
  amountText: "10",
  interval: "monthly",
  start: "2031-01-31",
  endMode: "count",
  count: "12",
  fund: null,
  fundText: "",
  message: "",
  onchain: false,
  ...x,
});
const t: Tresor = {
  id: "abababab", covenantId: "ab".repeat(32), owner: "cc".repeat(32), recipient: key.xonly, ownerAddress: "kaspa:qa", recipientAddress: "kaspa:qb",
  amount: "10", maxFee: "0.01", anchorDay: 31, periodMs: 0, nextDue: 1_801_353_600_000, left: 3, value: "50", covered: 4,
  outpoint: "x:1", message: "", onchain: false, key: null, created: "", ended: null, missing: null, lastError: null, due: true, history: [], code: "ghost-tresor:2:x",
};

describe("A12-8: Texte sagen, wer auslöst und was mit dem Rest der Höchstgebühr geschieht", () => {
  it("Hinweise beim Anlegen", () => {
    const all = tresorHints(form(), key).map((h) => h.text).join(" ");
    expect(all).toContain("Zum Termin darf jeder die Zahlung auslösen, meist der Empfänger oder ein laufender GHOST-Agent");
    expect(all).toContain("höchstens 0,01 KAS Höchstgebühr aus dem Tresor");
    expect(all).toContain("darf ein fremder Auslöser behalten; der GHOST-Agent und diese Seite lassen es im Tresor");
    expect(all).not.toContain("Die Netzgebühr (höchstens 0,01 KAS) kommt aus dem Tresor");
  });
  it("A12-1 im Vertrag: die Nachricht ist fest gebunden – kein Auslöser und auch der Absender ändert sie nicht", () => {
    for (const onchain of [false, true]) {
      const all = tresorHints(form({ message: "Miete", onchain }), key).map((h) => h.text).join(" ");
      expect(all).toContain("Die Nachricht ist im Vertrag fest gebunden: Jede Zahlung trägt genau diese Nachricht, und wer eine Zahlung auslöst, kann sie weder weglassen noch ändern.");
      expect(all).toContain("Auch du kannst sie später nicht mehr ändern – dafür kündigen und einen neuen Tresor anlegen.");
      expect(all).not.toContain("bindet die Nachricht nicht");
      expect(all).not.toContain("durch eine eigene ersetzen");
    }
    const ohne = tresorHints(form(), key).map((h) => h.text).join(" ");
    expect(ohne).toContain("Ohne Nachricht tragen alle Zahlungen keinen Text; auch später lässt sich keiner hinzufügen (nur mit einem neuen Tresor).");
    expect(ohne).not.toContain("fest gebunden");
    setLangGlobal("en");
    expect(tresorHints(form({ message: "Rent" }), key).map((h) => h.text).join(" ")).toContain("The message is fixed in the contract");
    setLangGlobal("de");
  });
});

describe("A12-7: Zahlbarkeit wie ghostctl (tresor::payable, MIN_KEEP 1 KAS)", () => {
  it("Gebührenquelle und Zustand", () => {
    expect(feeSource({ ...t, value: "10.5" })).toBeNull();
    // Nachprüfung A12-8: ghostctl zahlt nicht mehr, laut Vertrag ginge es noch
    expect(tresorStatus({ ...t, value: "10.5" })).toBe("low");
    expect(feeSource({ ...t, value: "11.00999999" })).toBe("key");
    expect(tresorStatus({ ...t, value: "11.00999999" })).toBe("due");
    expect(feeSource({ ...t, value: "11.01" })).toBe("tresor");
    expect(feeSource({ ...t, value: "11", maxFee: "0.1" })).toBe("key");
  });
});

describe("A12-10: eigener Schlüssel nur, wenn der Tresor die Gebühr nicht trägt", () => {
  it("Parameter von tresor-pay", () => {
    expect(payParams(t, key.file)).toEqual({ id: "abababab" });
    expect(payParams(t, null)).toEqual({ id: "abababab" });
    expect(payParams({ ...t, value: "11.005" }, key.file)).toEqual({ id: "abababab", key: key.file });
    expect(payParams({ ...t, value: "11.005" }, null)).toBeNull();
    expect(payParams({ ...t, value: "10.5" }, key.file)).toBeNull();
  });
});

describe("A12-18: Auffüllen mit dem geprüften Betrag", () => {
  it("Seite und Server lesen denselben Betrag", () => {
    expect(topupKas("1.000,5")).toBe("1000.5");
    expect(topupKas("1 000")).toBe("1000");
    expect(topupKas("12,5")).toBe("12.5");
    expect(topupKas(" 3 ")).toBe("3");
    for (const bad of ["", "0", "abc", "-1"]) expect(topupKas(bad)).toBeNull();
    for (const s of ["1.000,5", "1 000", "0,25", "3,00000001"]) expect(checkAmount(topupKas(s), "KAS-Betrag")).toBe(topupKas(s));
  });
});

describe("A12-9: Zeitpunkte mit Datum in Ortszeit und in UTC", () => {
  const env = (globalThis as unknown as { process: { env: Record<string, string | undefined> } }).process.env;
  const tz = env.TZ;
  afterEach(() => {
    if (tz === undefined) delete env.TZ;
    else env.TZ = tz;
  });
  it("New York: fällig am Vortag 19:00", () => {
    env.TZ = "America/New_York";
    const ms = dueMs("2031-01-31")!;
    expect(ms).toBe(Date.UTC(2031, 0, 31));
    expect(localDateTime(ms)).toBe("30.01.2031, 19:00");
    expect(utcDateTime(ms)).toBe("31.01.2031, 00:00 UTC");
    expect(localDateTime(dueMs("2031-07-31")!)).toBe("30.07.2031, 20:00");
  });
  it("Berlin: Winter 01:00, Sommer 02:00", () => {
    env.TZ = "Europe/Berlin";
    expect(localDateTime(dueMs("2031-01-31")!)).toBe("31.01.2031, 01:00");
    expect(localDateTime(dueMs("2031-07-31")!)).toBe("31.07.2031, 02:00");
  });
});

describe("Nachprüfung A12-8: „aufgebraucht“ nur, wenn auch der Vertrag keine Zahlung mehr zulässt", () => {
  it("Vertrag verlangt nur Wert − Betrag − Höchstgebühr > 0, ghostctl 1 KAS Rest", () => {
    // Beispiel der Nachprüfung: 10,5 KAS für 10 KAS – ein fremder Auslöser darf zahlen und 0,49 KAS zurücklassen
    expect(contractPayable({ ...t, value: "10.5" })).toBe(true);
    expect(tresorStatus({ ...t, value: "10.5" })).toBe("low");
    expect(tresorStatusLabel("low")).toBe("Guthaben knapp");
    // genau Betrag + Höchstgebühr: kein Rest, laut Vertrag nicht zahlbar
    expect(contractPayable({ ...t, value: "10.01" })).toBe(false);
    expect(tresorStatus({ ...t, value: "10.01" })).toBe("empty");
    expect(tresorStatusLabel("empty")).toBe("Guthaben aufgebraucht");
    expect(tresorStatus({ ...t, value: "10.01000001" })).toBe("low");
  });
  it("A12-1 im Vertrag: kein Hinweis mehr auf „nicht prüfbar“ – der Vertrag erzwingt die Nachricht", () => {
    const all = tresorHints(form({ message: "Miete" }), key).map((h) => h.text).join(" ");
    expect(all).not.toContain("nicht prüfbar");
    expect(all).not.toContain("ob sie der hinterlegten entspricht");
  });
});
