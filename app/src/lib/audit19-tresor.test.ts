// Audit 19 (audit/19-opus-tresor-wallet-namen.md), Seite: Startdatum in UTC
// und höchstens ein Jahr voraus (A19-8, A19-1/A19-3), ehrlicher Text zum
// Auslösen (A19-8), offene Tx in „Meine Tresore“ (A19-6), Signatur behalten,
// wenn der Server „gleich erneut“ meldet (A19-7).
import { describe, expect, it } from "vitest";
import component from "../components/WalletTresor.tsx?raw";
import signFlow from "../components/WalletSignFlow.tsx?raw";
import { keepSigned, sendOutcome } from "../wallet/actions";
import type { TresorForm } from "./tresor";
import { lastFirstDue, MAX_FIRST_DUE_DAYS, utcToday, walletTresorActionable, walletTresorBasics, walletTresorOpen, walletTresorParams } from "./tresorWallet";

const A = "kaspa:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const B = "kaspa:q" + "pzry9x8gf2tvdw0s3jn54khce6mua7lq".repeat(2).slice(0, 60);
const E8 = 100_000_000n;

const form = (f: Partial<TresorForm> = {}): TresorForm => ({
  to: B,
  amount: 10n * E8,
  amountText: "10",
  interval: "monthly",
  start: "2027-02-01",
  endMode: "count",
  count: "3",
  fund: null,
  fundText: "",
  message: "",
  onchain: false,
  ...f,
});

describe("A19-8: Startdatum in UTC", () => {
  it("heute ist das UTC-Datum, auch wenn die Ortszeit westlich von UTC noch beim Vortag ist", () => {
    // 06.10.2026, 23:30 in New York (UTC−4) = 07.10.2026, 03:30 UTC
    const abends = new Date(Date.UTC(2026, 9, 7, 3, 30));
    expect(utcToday(abends)).toBe("2026-10-07");
    // den Vortag (Ortszeit) lehnt die Seite jetzt ab wie der Server
    const p = walletTresorParams(form({ start: "2026-10-06" }), A, "mainnet", utcToday(abends));
    expect(p.problem).toMatch(/Vergangenheit \(Datum in UTC\)/);
    expect(walletTresorParams(form({ start: "2026-10-07" }), A, "mainnet", utcToday(abends)).problem).toBeNull();
  });

  it("die Oberfläche nimmt das UTC-Datum, kennzeichnet es und nicht mehr die Ortszeit", () => {
    expect(component).toContain("useState(() => utcToday())");
    expect(component).toContain("const today = utcToday();");
    expect(component).not.toContain("localToday");
    expect(component).toContain("Erster Termin (Datum in UTC)");
    expect(component).toContain("max={lastFirstDue(today)}");
  });
});

describe("A19-1/A19-3: erster Termin höchstens ein Jahr voraus (wie ghostctl)", () => {
  it("365 Tage gehen, einer mehr nicht", () => {
    expect(MAX_FIRST_DUE_DAYS).toBe(365);
    expect(lastFirstDue("2027-01-15")).toBe("2028-01-15");
    expect(lastFirstDue("2028-01-15")).toBe("2029-01-14"); // Schaltjahr
    expect(walletTresorParams(form({ start: "2028-01-15" }), A, "mainnet", "2027-01-15").problem).toBeNull();
    expect(walletTresorParams(form({ start: "2028-01-16" }), A, "mainnet", "2027-01-15").problem).toMatch(/höchstens ein Jahr/);
    expect(walletTresorParams(form({ start: "2199-01-01" }), A, "mainnet", "2027-01-15").problem).toMatch(/höchstens ein Jahr/);
  });
});

describe("A19-8: ehrlicher Text zum Auslösen", () => {
  it("kein Versprechen „zum Termin“, sondern „in der Regel innerhalb weniger Minuten“ mit Bedingungen", () => {
    const b = walletTresorBasics().join(" ");
    expect(b).not.toMatch(/Zum Termin löst/);
    expect(b).toMatch(/in der Regel innerhalb weniger Minuten nach dem Termin/);
    expect(b).toMatch(/solange der Agent läuft und das Guthaben reicht/);
    expect(component).not.toContain("Ausgelöst wird kurz danach.");
    expect(component).toContain("Ausgelöst wird in der Regel wenige Minuten danach, solange der K.Lend-Agent läuft und das Guthaben reicht.");
  });
});

describe("A19-6: gesendet, noch nicht bestätigt", () => {
  it("ein Tresor mit offener Tx steht bei den laufenden, aber ohne Auffüllen und Kündigen", () => {
    const t = { ended: null, missing: null, pending: { txid: "ab".repeat(32), action: "Tresor anlegen (Wallet)" } };
    expect(walletTresorOpen(t)).toBe(true);
    expect(walletTresorActionable(t)).toBe(false);
    expect(walletTresorActionable({ ended: null, missing: null })).toBe(true);
    expect(walletTresorActionable({ ended: "x", missing: null })).toBe(false);
  });

  it("die Oberfläche zeigt den Vermerk und nach dem Anlegen den Hinweis", () => {
    expect(component).toContain("noch nicht bestätigt");
    expect(component).toContain("{actionable && (");
    expect(component).toContain("Der neue Tresor erscheint unter „Meine Tresore“, sobald die Transaktion bestätigt ist");
    expect(component).toContain("if (any) setOpened(true);");
  });
});

describe("A19-7: „gleich erneut“ behält die geprüfte Signatur", () => {
  it("nur wenn sicher nichts gesendet wurde", () => {
    expect(keepSigned({ ok: false, busy: true })).toBe(true);
    expect(sendOutcome({ ok: false, busy: true })).toBe("failed");
    expect(keepSigned({ ok: false, busy: true, unclear: true })).toBe(false);
    expect(keepSigned({ ok: false, busy: true, timeout: true })).toBe(false);
    expect(keepSigned({ ok: true, busy: true, sent: true })).toBe(false);
    expect(keepSigned({ ok: false, error: "Plan passt nicht" })).toBe(false);
    expect(signFlow).toContain("if (!keepSigned(r)) setChecked(null);");
  });
});
