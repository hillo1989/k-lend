import { beforeEach, describe, expect, it } from "vitest";
import { aboHints, aboParams, encryptProblem, intervalLabel, localToday, messageProblem, occurrence, schedule, type AboForm } from "./abo";
import type { KeyEntry } from "./api";
import { cliDecimal } from "./commands";
import { setLangGlobal } from "./i18n";

beforeEach(() => setLangGlobal("de"));

describe("Termine wie ghostctl (abo.rs)", () => {
  it("monatlich am 31.: letzter Tag kürzerer Monate, danach wieder der 31.", () => {
    expect(schedule("2027-01-31", "monthly", 6)).toEqual(["2027-01-31", "2027-02-28", "2027-03-31", "2027-04-30", "2027-05-31", "2027-06-30"]);
    expect(occurrence("2027-12-31", "monthly", 2)).toBe("2028-02-29");
    expect(occurrence("2027-12-31", "monthly", 14)).toBe("2029-02-28");
  });
  it("Schaltjahr, wöchentlich und alle n Tage", () => {
    expect(schedule("2028-02-28", "daily", 3)).toEqual(["2028-02-28", "2028-02-29", "2028-03-01"]);
    expect(occurrence("2028-02-29", "monthly", 12)).toBe("2029-02-28");
    expect(occurrence("2028-02-29", "monthly", 48)).toBe("2032-02-29");
    expect(occurrence("2027-12-27", "weekly", 1)).toBe("2028-01-03");
    expect(occurrence("2028-02-20", { days: 14 }, 1)).toBe("2028-03-05");
  });
  it("Ende inklusive, Anzahl, beides", () => {
    expect(schedule("2027-01-01", "weekly", 10, "2027-01-15")).toEqual(["2027-01-01", "2027-01-08", "2027-01-15"]);
    expect(schedule("2027-01-31", "monthly", 10, null, 3)).toEqual(["2027-01-31", "2027-02-28", "2027-03-31"]);
    expect(schedule("2027-01-01", "weekly", 10, "2027-01-20", 2)).toHaveLength(2);
    expect(schedule("2027-02-30", "monthly", 3)).toEqual([]);
  });
  it("lokales Datum und Beschriftung", () => {
    expect(localToday(new Date(2026, 8, 29, 23, 59))).toBe("2026-09-29");
    expect(intervalLabel({ days: 14 })).toBe("alle 14 Tage");
    expect(intervalLabel("monthly")).toBe("monatlich");
  });
});

describe("Nachricht", () => {
  it("Länge und Steuerzeichen wie ghostctl", () => {
    expect(messageProblem("")).toBeNull();
    expect(messageProblem("Miete Oktober – Whg. 3")).toBeNull();
    expect(messageProblem("🏠".repeat(100))).toBeNull();
    expect(messageProblem("x".repeat(101))).toMatch(/höchstens 100/);
    expect(messageProblem("a\nb")).toMatch(/Steuerzeichen/);
    expect(messageProblem("‮gnudnewrebÜ")).toMatch(/Steuerzeichen/);
    expect(messageProblem("a​b")).toMatch(/Steuerzeichen/);
  });
});

const key: KeyEntry = { file: "keys/mainnet-owner.json", type: "key", xonly: "ab".repeat(32), address: "kaspa:qx", kas: 10, ghost: 2, vaults: [] };
const form = (f: Partial<AboForm> = {}): AboForm => ({
  asset: "KAS",
  to: "kaspa:qrecipient",
  amount: 150_000_000n,
  amountText: "1,5",
  interval: "monthly",
  start: "2026-10-01",
  endMode: "none",
  end: "",
  count: "12",
  message: "Miete",
  onchain: false,
  ...f,
});
const today = "2026-09-29";

describe("aboParams", () => {
  it("baut genau die Server-Parameter", () => {
    expect(aboParams(form(), key, cliDecimal, today).params).toEqual({
      key: "keys/mainnet-owner.json",
      asset: "KAS",
      to: "kaspa:qrecipient",
      amount: "1.5",
      interval: "monthly",
      start: "2026-10-01",
      message: "Miete",
    });
    const p = aboParams(form({ interval: { days: 14 }, endMode: "count", count: "3", onchain: true }), key, cliDecimal, today).params;
    expect(p).toMatchObject({ interval: "14", count: 3, onchain: true });
    expect(aboParams(form({ endMode: "date", end: "2027-06-30", message: "" }), key, cliDecimal, today).params).toMatchObject({ end: "2027-06-30" });
  });
  it("lehnt Unfertiges ab", () => {
    const problem = (f: Partial<AboForm>, k: KeyEntry | null = key) => aboParams(form(f), k, cliDecimal, today).problem;
    expect(problem({}, null)).toMatch(/Schlüssel/);
    expect(problem({ to: " " })).toMatch(/Empfänger/);
    expect(problem({ to: key.file })).toMatch(/eigene Konto/);
    expect(problem({ amount: null })).toMatch(/Betrag/);
    expect(problem({ amount: 0n })).toMatch(/Betrag/);
    expect(problem({ interval: null })).toMatch(/Intervall/);
    expect(problem({ start: "2026-09-28" })).toMatch(/Vergangenheit/);
    expect(problem({ endMode: "date", end: "2026-09-30" })).toMatch(/vor dem Start/);
    expect(problem({ endMode: "count", count: "0" })).toMatch(/Anzahl/);
    expect(problem({ message: "", onchain: true })).toMatch(/öffentliche Nachricht/);
    expect(problem({ message: "a\tb" })).toMatch(/Steuerzeichen/);
  });
});

describe("aboHints", () => {
  it("warnt vor Kleinstbeträgen, fehlendem Guthaben und öffentlicher Nachricht", () => {
    expect(aboHints(form({ amount: 10_000_000n }), key).map((h) => h.level)).toContain("warn");
    expect(aboHints(form({ amount: 20n * 100_000_000n }), key).some((h) => /nur 10 KAS/.test(h.text))).toBe(true);
    expect(aboHints(form({ asset: "GHOST", amount: 5n * 100_000_000n }), key).some((h) => /nur 2 GHOST/.test(h.text))).toBe(true);
    expect(aboHints(form({ onchain: true }), key).some((h) => /für alle lesbar/.test(h.text))).toBe(true);
    expect(aboHints(form({ message: "" }), key)).toEqual([]);
    // mit Nachricht ohne Häkchen nur der Hinweis auf die Verschlüsselung
    expect(aboHints(form(), key).map((h) => h.level)).toEqual(["info"]);
  });
});

describe("Verschlüsselte Nachricht", () => {
  const schnorr = "kaspa:qrcxpm930jhc2tha2k5ep4xxyd0757qf0qj5aazzu0z8q6rgd2kkx59tjyjcl";
  const p2sh = "kaspa:precqv0krj3r6uyyfa36ga7s0u9jct0v4wg8ctsfde2gkrsgwgw8jgxfzfc98";
  it("nur an Schnorr-Adressen, Schlüsseldateien und x-only-Schlüssel", () => {
    expect(encryptProblem(schnorr)).toBeNull();
    expect(encryptProblem("keys/tn10-user.json")).toBeNull();
    expect(encryptProblem("ab".repeat(32))).toBeNull();
    expect(encryptProblem(p2sh)).toMatch(/normale Kaspa-Adressen/);
    expect(encryptProblem("kaspa:q" + "x".repeat(62))).toMatch(/normale Kaspa-Adressen/);
  });
  it("Formular: ohne Häkchen an P2SH abgelehnt, öffentlich erlaubt, ohne Nachricht erlaubt", () => {
    const t = (f: Partial<AboForm>) => aboParams(form(f), key, (u) => String(u), "2026-10-01");
    expect(t({ to: p2sh }).problem).toMatch(/normale Kaspa-Adressen/);
    expect(t({ to: p2sh, onchain: true }).params?.onchain).toBe(true);
    expect(t({ to: p2sh, message: "" }).params).not.toBeNull();
    expect(t({ to: schnorr }).params?.message).toBe("Miete");
  });
  it("Hinweis: verschlüsselt bzw. öffentlich", () => {
    expect(aboHints(form({ to: schnorr }), key).some((h) => /verschlüsselt/.test(h.text))).toBe(true);
    expect(aboHints(form({ onchain: true }), key).some((h) => /für alle lesbar/.test(h.text))).toBe(true);
  });
});
