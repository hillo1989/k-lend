import { beforeEach, describe, expect, it } from "vitest";
import type { KeyEntry } from "./api";
import { cliDecimal } from "./commands";
import { setLangGlobal } from "./i18n";
import {
  codeProblem,
  coveredPayments,
  dueMs,
  effectiveFund,
  leftLabel,
  minFund,
  suggestedFund,
  tresorHints,
  tresorIntervalLabel,
  tresorParams,
  tresorRole,
  tresorStatus,
  type Tresor,
  type TresorForm,
} from "./tresor";

beforeEach(() => setLangGlobal("de"));

const E8 = 100_000_000n;
const key: KeyEntry = { file: "keys/a.json", type: "key", xonly: "aa".repeat(32), address: "kaspatest:qown", kas: 500, ghost: 0, vaults: [] };
const form = (x: Partial<TresorForm> = {}): TresorForm => ({
  to: "kaspatest:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz",
  amount: 10n * E8,
  amountText: "10",
  interval: "monthly",
  start: "2027-01-31",
  endMode: "count",
  count: "12",
  fund: null,
  fundText: "",
  message: "",
  onchain: false,
  ...x,
});
const params = (x: Partial<TresorForm> = {}, k: KeyEntry | null = key) => tresorParams(form(x), k, (u) => cliDecimal(u), "2027-01-01");

describe("Startguthaben", () => {
  it("Vorschlag n × (Betrag + 0,01) + 1 KAS wie tresor::suggested_fund", () => {
    expect(suggestedFund(10n * E8, 12)).toBe(12n * (10n * E8 + 1_000_000n) + E8);
    expect(suggestedFund(10n * E8, null)).toBeNull();
    expect(suggestedFund(null, 3)).toBeNull();
    expect(coveredPayments(12n * (10n * E8 + 1_000_000n) + E8, 10n * E8)).toBe(12);
    expect(coveredPayments(12n * (10n * E8 + 1_000_000n) + E8 - 1n, 10n * E8)).toBe(11);
    expect(coveredPayments(E8, 10n * E8)).toBe(0);
    expect(minFund(10n * E8)).toBe(11n * E8 + 1_000_000n);
  });
  it("leeres Feld mit Anzahl: Vorschlag; unbegrenzt: Eingabe nötig", () => {
    expect(effectiveFund(form())).toBe(suggestedFund(10n * E8, 12));
    expect(effectiveFund(form({ endMode: "none" }))).toBeNull();
    expect(effectiveFund(form({ fundText: "50", fund: 50n * E8 }))).toBe(50n * E8);
  });
});

describe("Formular", () => {
  it("baut genau die Parameter von tresor-open", () => {
    expect(params({ message: " Miete ", onchain: true })).toEqual({
      params: { key: "keys/a.json", to: form().to, amount: "10", interval: "monthly", start: "2027-01-31", fund: "121.12", count: 12, message: "Miete", onchain: true },
      problem: null,
    });
    expect(params({ endMode: "none", fundText: "60.5", fund: 6_050_000_000n, interval: { days: 14 } }).params).toEqual({
      key: "keys/a.json",
      to: form().to,
      amount: "10",
      interval: "14",
      start: "2027-01-31",
      fund: "60.5",
    });
  });
  it("lehnt ab", () => {
    const why = (x: Partial<TresorForm>, k: KeyEntry | null = key) => params(x, k).problem;
    expect(why({}, null)).toMatch(/Kein Schlüssel/);
    expect(why({ to: " " })).toMatch(/Empfänger/);
    expect(why({ to: "keys/a.json" })).toMatch(/eigene Konto/);
    expect(why({ to: "kaspatest:qown" })).toMatch(/eigene Konto/);
    expect(why({ amount: E8 - 1n, amountText: "0.99999999" })).toMatch(/mindestens 1 KAS/);
    expect(why({ amount: null, amountText: "x" })).toMatch(/ungültig/);
    expect(why({ interval: null })).toMatch(/Intervall/);
    expect(why({ start: "2026-12-31" })).toMatch(/Vergangenheit/);
    expect(why({ start: "2027-02-30" })).toMatch(/gültiges Datum/);
    expect(why({ count: "0" })).toMatch(/Anzahl/);
    expect(why({ endMode: "none" })).toMatch(/Startguthaben angeben/);
    expect(why({ fundText: "11", fund: 11n * E8 })).toMatch(/mindestens 11\.01 KAS/);
    expect(why({ fundText: "x", fund: null })).toMatch(/ungültig/);
    expect(why({ message: "a\nb" })).toMatch(/Steuerzeichen/);
    expect(why({ onchain: true })).toMatch(/erst eine Nachricht/);
  });
  it("Hinweise: gebunden, kündbar, Auslöser, Deckung, Guthaben", () => {
    const t = (x: Partial<TresorForm>, k: KeyEntry | null = key) => tresorHints(form(x), k).map((h) => `${h.level}:${h.text}`);
    expect(t({}).join(" ")).toMatch(/gebunden.*jederzeit kündigen/);
    expect(t({}).join(" ")).toMatch(/darf jeder die Zahlung auslösen, meist der Empfänger oder ein laufender GHOST-Agent/);
    expect(t({ fundText: "50", fund: 50n * E8 }).join(" ")).toMatch(/warn:.*reicht für 4 von 12/);
    expect(t({ endMode: "none", fundText: "50", fund: 50n * E8 }).join(" ")).toMatch(/reicht für 4 Zahlung/);
    expect(t({}, { ...key, kas: 100 }).join(" ")).toMatch(/nur 100 KAS/);
    expect(t({ onchain: true, message: "Miete" }).join(" ")).toMatch(/für alle lesbar/);
  });
  it("Termin eines Tages: 00:00 UTC", () => {
    expect(dueMs("2027-07-01")).toBe(Date.UTC(2027, 6, 1));
    expect(dueMs("2027-02-30")).toBeNull();
  });
});

describe("Anzeige", () => {
  const t: Tresor = {
    id: "abababab",
    covenantId: "ab".repeat(32),
    owner: "aa".repeat(32),
    recipient: "bb".repeat(32),
    ownerAddress: "kaspa:qa",
    recipientAddress: "kaspa:qb",
    amount: "10",
    maxFee: "0.01",
    anchorDay: 31,
    periodMs: 0,
    nextDue: 0,
    left: 3,
    value: "50",
    covered: 4,
    outpoint: "x:1",
    message: "",
    onchain: false,
    key: null,
    created: "",
    ended: null,
    missing: null,
    lastError: null,
    due: false,
    history: [],
    code: "ghost-tresor:2:x",
  };
  it("Rolle und Zustand", () => {
    expect(tresorRole(t, "aa".repeat(32))).toBe("owner");
    expect(tresorRole(t, "bb".repeat(32))).toBe("recipient");
    expect(tresorRole(t, null)).toBe("other");
    expect(tresorStatus(t)).toBe("active");
    expect(tresorStatus({ ...t, due: true })).toBe("due");
    expect(tresorStatus({ ...t, left: 0 })).toBe("done");
    // nicht einmal Betrag + 1 KAS Rest (auch nicht mit eigener Gebühr): ghostctl
    // zahlt nicht mehr, laut Vertrag ginge es noch („knapp“, Nachprüfung A12-8)
    expect(tresorStatus({ ...t, value: "10.99999999", due: true })).toBe("low");
    expect(tresorStatus({ ...t, value: "11", due: true })).toBe("due");
    expect(tresorStatus({ ...t, missing: "x" })).toBe("missing");
    expect(tresorStatus({ ...t, ended: "x", missing: "x" })).toBe("cancelled");
  });
  it("Texte", () => {
    expect(leftLabel(-1)).toBe("unbegrenzt");
    expect(leftLabel(1)).toBe("noch 1 Zahlung");
    expect(tresorIntervalLabel({ anchorDay: 31, periodMs: 0 })).toBe("monatlich am 31.");
    expect(tresorIntervalLabel({ anchorDay: 0, periodMs: 14 * 86_400_000 })).toBe("alle 14 Tage");
    setLangGlobal("en");
    expect(tresorIntervalLabel({ anchorDay: 0, periodMs: 7 * 86_400_000 })).toBe("weekly");
    expect(leftLabel(2)).toBe("2 payments left");
  });
  it("Tresor-Code", () => {
    expect(codeProblem("")).toMatch(/einfügen/);
    expect(codeProblem("kaspa:qxyz")).toMatch(/kein Tresor-Code/);
    expect(codeProblem("ghost-tresor:2:abc")).toMatch(/unvollständig/);
    expect(codeProblem("ghost-tresor:2:abc def ghi jkl mno pqr")).toMatch(/unvollständig/);
    expect(codeProblem(`  ghost-tresor:2:${"A".repeat(40)}\n`)).toBeNull();
    // A12-1 im Vertrag: alte Codes (Version 1) klar abgelehnt
    expect(codeProblem(`ghost-tresor:1:${"A".repeat(40)}`)).toMatch(/^Alter Tresor-Code: .*neuen Tresor anfordern/);
    expect(codeProblem("ghost-tresor:3:AAAAAAAAAAAAAAAAAAAA")).toMatch(/kein Tresor-Code.*ghost-tresor:2:/);
  });
});
