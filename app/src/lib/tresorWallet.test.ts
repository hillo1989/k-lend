// Tresor mit Browser-Wallet: Formular → Parameter (wie server/walletActions.ts
// sie annimmt), Hinweise, Anzeige-Zustand
import { describe, expect, it } from "vitest";
import component from "../components/WalletTresor.tsx?raw";
import { TRESOR_WALLET_ACTIONS, walletParamsOf } from "../wallet/actions";
import type { TresorForm } from "./tresor";
import { walletCancelParams, walletTopupParams, walletTresorBasics, walletTresorHints, walletTresorParams, walletTresorStatus } from "./tresorWallet";

const A = "kaspa:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const B = "kaspa:q" + "pzry9x8gf2tvdw0s3jn54khce6mua7lq".repeat(2).slice(0, 60);
const T = "kaspatest:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const COV = "c0ffee".repeat(10) + "abcd";
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
const today = "2027-01-15";

describe("Tresor anlegen: Parameter für /api/wallet/build", () => {
  it("nur erlaubte Parameter, Startguthaben-Vorschlag, Nachricht öffentlich", () => {
    const r = walletTresorParams(form(), A, "mainnet", today);
    expect(r.problem).toBeNull();
    expect(r.params).toEqual({ to: B, amount: "10", interval: "monthly", start: "2027-02-01", fund: "31.03", count: 3 });
    for (const k of Object.keys(r.params!)) expect(walletParamsOf("tresor-open")).toContain(k);
    const m = walletTresorParams(form({ message: " Miete ", onchain: false, endMode: "none", fund: 50n * E8, fundText: "50", interval: { days: 14 } }), A, "mainnet", today).params!;
    expect(m).toEqual({ to: B, amount: "10", interval: "14", start: "2027-02-01", fund: "50", message: "Miete" });
    expect(m).not.toHaveProperty("onchain");
  });

  it("Probleme statt Parameter", () => {
    const p = (f: Partial<TresorForm>, addr: string | null = A, net: "mainnet" | "testnet-10" = "mainnet") => walletTresorParams(form(f), addr, net, today).problem ?? "";
    expect(p({}, null)).toMatch(/Wallet verbinden/);
    expect(p({ to: "" })).toMatch(/Empfänger angeben/);
    expect(p({ to: "keys/a.json" })).toMatch(/Kaspa-Adresse/);
    expect(p({ to: "ab".repeat(32) })).toMatch(/Kaspa-Adresse/);
    expect(p({ to: T })).toMatch(/anderen Netz/);
    expect(p({ to: A })).toMatch(/eigene Adresse/);
    expect(p({ amount: 99_999_999n, amountText: "0.99999999" })).toMatch(/Mindestens 1 KAS/);
    expect(p({ interval: null })).toMatch(/Intervall/);
    expect(p({ start: "2027-01-14" })).toMatch(/Vergangenheit/);
    expect(p({ count: "0" })).toMatch(/Anzahl/);
    expect(p({ endMode: "none" })).toMatch(/Startguthaben angeben/);
    expect(p({ fund: 10n * E8, fundText: "10" })).toMatch(/mindestens 11.01 KAS/);
    expect(p({ message: "a‮b" })).toMatch(/Nachricht/);
    expect(walletTresorParams(form({ to: T }), T, "testnet-10", today).problem).toMatch(/eigene Adresse/);
  });

  it("Auffüllen und Kündigen: nur volle Covenant-ID und geprüfter Betrag", () => {
    expect(walletTopupParams({ covenantId: COV }, "1.000,5")).toEqual({ tresor: COV, kas: "1000.5" });
    expect(walletTopupParams({ covenantId: COV }, "")).toBeNull();
    expect(walletTopupParams({ covenantId: COV }, "0")).toBeNull();
    expect(walletTopupParams({ covenantId: "c0ffeec0" }, "1")).toBeNull();
    expect(walletCancelParams({ covenantId: COV })).toEqual({ tresor: COV });
    expect(walletCancelParams({ covenantId: "../x" })).toBeNull();
  });

  it("Seite kennt dieselben Parameter je Tresor-Aktion wie der Server (Liste wie in server/walletTresor.test.ts)", () => {
    const want: Record<string, string[]> = {
      "tresor-open": ["to", "amount", "interval", "start", "count", "fund", "maxFee", "message"],
      "tresor-topup": ["tresor", "kas"],
      "tresor-cancel": ["tresor"],
    };
    for (const a of TRESOR_WALLET_ACTIONS) expect([...walletParamsOf(a)]).toEqual(want[a]);
  });
});

describe("Hinweise und Anzeige", () => {
  it("die vier Grundsätze stehen da: Vertrag statt K.Lend, nur Besitzer, Agent, Gebühr aus dem Tresor", () => {
    const b = walletTresorBasics().join(" ");
    expect(b).toMatch(/nicht bei K\.Lend/);
    expect(b).toMatch(/Nur du – der Besitzer/);
    expect(b).toMatch(/K\.Lend-Agent/);
    expect(b).toMatch(/Netzgebühr jeder Zahlung kommt aus dem Tresor/);
  });

  it("Deckung, Guthaben der Wallet und öffentliche Nachricht", () => {
    const h = (f: Partial<TresorForm>, bal: bigint | null = null) => walletTresorHints(form(f), bal).map((x) => `${x.level}: ${x.text}`).join("\n");
    expect(h({ fund: 22n * E8, fundText: "22" })).toMatch(/warn: Das Guthaben reicht für 2 von 3 Zahlungen/);
    expect(h({ endMode: "none", fund: 50n * E8, fundText: "50" })).toMatch(/info: Das Guthaben reicht für 4 Zahlung/);
    expect(h({}, 5n * E8)).toMatch(/zu wenig KAS/);
    expect(h({}, 500n * E8)).not.toMatch(/zu wenig KAS/);
    expect(h({ message: "Miete" })).toMatch(/öffentlich/);
    expect(h({})).not.toMatch(/öffentlich/);
  });

  it("Zustand: der Agent zahlt bei Wallet-Tresoren keine Gebühr dazu", () => {
    const t = { ended: null, missing: null, left: 3, value: "31.03", amount: "10", maxFee: "0.01", due: false };
    expect(walletTresorStatus(t)).toBe("active");
    expect(walletTresorStatus({ ...t, due: true })).toBe("due");
    // trüge die Zahlung nur mit Gebühr vom Auslöser: für Wallet-Tresore „knapp“
    expect(walletTresorStatus({ ...t, value: "11.005" })).toBe("low");
    expect(walletTresorStatus({ ...t, value: "10.005" })).toBe("empty");
    expect(walletTresorStatus({ ...t, left: 0 })).toBe("done");
    expect(walletTresorStatus({ ...t, ended: "x" })).toBe("cancelled");
  });

  it("Oberfläche ohne inline-Styles und Skripte (CSP)", () => {
    expect(component).not.toMatch(/style=\{|dangerouslySetInnerHTML|<script/);
  });
});
