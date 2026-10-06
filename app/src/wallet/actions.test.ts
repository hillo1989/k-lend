// Formularlogik der Aktionen mit Browser-Wallet (src/wallet/actions.ts)
import { describe, expect, it, vi } from "vitest";
import type { CliAction } from "../lib/commands";
import { ACTION_ORDER } from "../lib/commands";
import {
  buildBody,
  callWalletApi,
  canSendSigned,
  foreignOutputs,
  planProblem,
  sendOutcome,
  signInWallet,
  submitBody,
  toWalletParams,
  WALLET_ACTIONS,
  walletKeyEntry,
  walletSupports,
  type BuildResult,
  type Fetch,
  type SubmitResult,
} from "./actions";

const A = "kaspa:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const B = "kaspa:q" + "l7aum6echk45nj3s0wdvt2fg8x9yrzpq".repeat(2).slice(0, 60);

const goodPlan = (): BuildResult => ({
  ok: true,
  action: "mint",
  address: A,
  feeSompi: 6_000_000,
  outputs: [
    { index: 0, sompi: 1e8, kas: 1, address: "kaspa:p…", what: "Vertrag (Covenant)" },
    { index: 1, sompi: 5e8, kas: 5, address: B, what: "andere Adresse" },
    { index: 2, sompi: 9e8, kas: 9, address: A, what: "Wechselgeld an die Wallet" },
  ],
  signInputs: [
    { index: 0, kind: "entry", entry: "mint", argPos: 3, redeemHex: "abcd" },
    { index: 3, kind: "p2pk", argPos: 0 },
  ],
  kasware: { txJsonString: '{"tx":1}', options: { signInputs: [{ index: 0, sighashType: 1 }, { index: 3, sighashType: 1 }] } },
  kastle: { networkId: "mainnet", txJson: '{"tx":1}', scripts: [{ inputIndex: 0, scriptHex: "abcd", signType: "All" }] },
  plan: { kind: "ghost-wallet-action:1", network: "mainnet", address: A },
});

describe("welche Aktionen mit der Wallet gehen", () => {
  it("alle Nutzeraktionen, nicht Orakel und Pool-Anlage", () => {
    expect(WALLET_ACTIONS.length).toBe(14);
    for (const a of ACTION_ORDER) expect(walletSupports(a)).toBe(!["oracle-update", "pool-open"].includes(a));
  });
});

describe("Formular-Parameter → Wallet-Parameter", () => {
  it("lässt die Schlüsseldatei weg und behält die Werte", () => {
    expect(toWalletParams("mint", { key: "keys/a.json", vault: 2, ghost: "1.5" })).toEqual({ params: { vault: 2, ghost: "1.5" }, problem: null });
    expect(toWalletParams("repay", { key: "keys/a.json", vault: 0 })).toEqual({ params: { vault: 0 }, problem: null });
    expect(toWalletParams("send", { to: A, kas: "1", message: "Hi", onchain: true }).params).toEqual({ to: A, kas: "1", message: "Hi", onchain: true });
    expect(toWalletParams("pool-remove", { percent: "50", minKas: "1", minGhost: "0" }).params).toEqual({ percent: "50", minKas: "1", minGhost: "0" });
  });
  it("Empfänger als Schlüsseldatei geht nicht", () => {
    const r = toWalletParams("transfer", { to: "keys/b.json", ghost: "1" });
    expect(r.params).toBeNull();
    expect(r.problem).toMatch(/Adresse/);
  });
  it("Orakel-Update nur mit Schlüsseldatei; fremde Felder fallen weg", () => {
    expect(toWalletParams("oracle-update" as CliAction, { committee: "keys/c.json" }).params).toBeNull();
    expect(toWalletParams("close", { vault: 1, committee: "keys/c.json" }).params).toEqual({ vault: 1 });
  });
});

describe("Anfragen", () => {
  it("build: Netz, Aktion, Adresse, Parameter", () => {
    expect(buildBody("mainnet", "close", A, { vault: 1 })).toEqual({ network: "mainnet", action: "close", address: A, params: { vault: 1 } });
  });
  it("submit: prüfen ohne send; senden im Mainnet nur mit confirmMainnet", () => {
    const p = goodPlan().plan;
    expect(submitBody("mainnet", p, "s", false)).toEqual({ network: "mainnet", plan: p, signed: "s" });
    expect(submitBody("mainnet", p, "s", true)).toEqual({ network: "mainnet", plan: p, signed: "s", send: true, confirmMainnet: true });
    expect(submitBody("testnet-10", p, "s", true)).toEqual({ network: "testnet-10", plan: p, signed: "s", send: true });
    expect(() => submitBody("mainnet", undefined, "s", true)).toThrow();
  });
});

describe("Plan vor dem Signieren prüfen", () => {
  it("passender Plan", () => {
    expect(planProblem(goodPlan(), "mainnet", A)).toBeNull();
  });
  it("andere Adresse, anderes Netz, falsche Art, ohne Anfrage, Fehler", () => {
    expect(planProblem(goodPlan(), "mainnet", B)).toMatch(/anderen Adresse/);
    expect(planProblem(goodPlan(), "testnet-10", A)).toMatch(/anderen Netz/);
    expect(planProblem({ ...goodPlan(), plan: { kind: "ghost-wallet-plan:1", network: "mainnet", address: A } }, "mainnet", A)).toMatch(/Signierplan/);
    expect(planProblem({ ...goodPlan(), signInputs: [] }, "mainnet", A)).toMatch(/Signier-Anfrage/);
    expect(planProblem({ ok: false, error: "zu wenig KAS" }, "mainnet", A)).toBe("zu wenig KAS");
  });
  it("Ausgänge an fremde Adressen werden hervorgehoben", () => {
    expect(foreignOutputs(goodPlan(), A).map((o) => o.index)).toEqual([1]);
  });
});

describe("Signieren in der Wallet (sendet nicht)", () => {
  it("KasWare: signPskt mit txJsonString und signInputs", async () => {
    const signPskt = vi.fn(async () => '{"signed":true}');
    const win = { kasware: { requestAccounts: vi.fn(), getNetwork: vi.fn(), signPskt } };
    expect(await signInWallet("kasware", win, goodPlan())).toBe('{"signed":true}');
    expect(signPskt).toHaveBeenCalledWith({ txJsonString: '{"tx":1}', options: { signInputs: [{ index: 0, sighashType: 1 }, { index: 3, sighashType: 1 }] } });
  });
  it("Kastle: signTx(networkId, txJson, scripts); Objekt-Antwort wird zu JSON-Text", async () => {
    const signTx = vi.fn(async () => ({ id: "x" }));
    const win = { kastle: { connect: vi.fn(), getAccount: vi.fn(), getNetwork: vi.fn(), signTx } };
    expect(await signInWallet("kastle", win, goodPlan())).toBe('{"id":"x"}');
    expect(signTx).toHaveBeenCalledWith("mainnet", '{"tx":1}', [{ inputIndex: 0, scriptHex: "abcd", signType: "All" }]);
  });
  it("fehlende Wallet oder leere Antwort → Fehler", async () => {
    await expect(signInWallet("kasware", {}, goodPlan())).rejects.toThrow(/KasWare/);
    await expect(signInWallet("kastle", {}, goodPlan())).rejects.toThrow(/Kastle/);
    const win = { kasware: { requestAccounts: vi.fn(), getNetwork: vi.fn(), signPskt: vi.fn(async () => "") } };
    await expect(signInWallet("kasware", win, goodPlan())).rejects.toThrow();
  });
});

describe("Senden erst nach gültiger Prüfung", () => {
  it("nur ok + valid und noch nicht gesendet", () => {
    expect(canSendSigned(null)).toBe(false);
    expect(canSendSigned({ ok: true, valid: false })).toBe(false);
    expect(canSendSigned({ ok: false, valid: true })).toBe(false);
    expect(canSendSigned({ ok: true, valid: true, sent: true })).toBe(false);
    expect(canSendSigned({ ok: true, valid: true })).toBe(true);
  });
});

describe("Server-Aufruf", () => {
  it("POST mit X-Ghost-Client und JSON; HTTP-Fehler kommen als {ok:false}", async () => {
    const calls: { url: string; init: Parameters<Fetch>[1] }[] = [];
    const f: Fetch = async (url, init) => {
      calls.push({ url, init });
      return { status: 429, json: async () => ({ ok: false, error: "Zu viele Anfragen" }) };
    };
    const r = await callWalletApi(f, "build", { a: 1 });
    expect(r).toEqual({ ok: false, error: "Zu viele Anfragen" });
    expect(calls[0].url).toBe("./api/wallet/build");
    expect(calls[0].init.method).toBe("POST");
    expect(calls[0].init.headers["X-Ghost-Client"]).toBe("1");
    const bad: Fetch = async () => ({ status: 502, json: async () => { throw new Error("kein JSON"); } });
    expect((await callWalletApi(bad, "submit", {})).ok).toBe(false);
  });

  it("A17-7: beim Senden sind Gateway-Fehler, Netzfehler und 5xx „unklar“, nie „nicht gesendet“", async () => {
    const html504: Fetch = async () => ({ status: 504, json: async () => { throw new Error("HTML"); } });
    const r1 = (await callWalletApi(html504, "submit", {}, true)) as unknown as SubmitResult;
    expect(r1).toMatchObject({ ok: false, unclear: true });
    expect(sendOutcome(r1)).toBe("unclear");
    const down: Fetch = async () => { throw new TypeError("Failed to fetch"); };
    const r2 = (await callWalletApi(down, "submit", {}, true)) as unknown as SubmitResult;
    expect(sendOutcome(r2)).toBe("unclear");
    const json500: Fetch = async () => ({ status: 500, json: async () => ({ ok: false, error: "Interner Fehler." }) });
    expect(sendOutcome((await callWalletApi(json500, "submit", {}, true)) as unknown as SubmitResult)).toBe("unclear");
    // „ausgelastet“ kommt von der Seite selbst, bevor ghostctl startet: nicht gesendet
    const busy: Fetch = async () => ({ status: 503, json: async () => ({ ok: false, busy: true, error: "ausgelastet" }) });
    expect(sendOutcome((await callWalletApi(busy, "submit", {}, true)) as unknown as SubmitResult)).toBe("failed");
    // ohne Senden bleibt es ein gewöhnlicher Fehler
    expect((await callWalletApi(html504, "submit", {})).unclear).toBeUndefined();
    await expect(callWalletApi(down, "build", {})).rejects.toThrow();
  });

  it("Senden: bestätigt, gesendet ohne Bestätigung, abgelehnt", () => {
    expect(sendOutcome({ ok: true, sent: true, confirmed: true })).toBe("confirmed");
    expect(sendOutcome({ ok: true, sent: true, confirmed: false, pending: true })).toBe("pending");
    expect(sendOutcome({ ok: false, error: "Signatur ungültig – nicht gesendet" })).toBe("failed");
    expect(sendOutcome({ ok: false, unclear: true })).toBe("unclear");
  });
});

describe("Wallet-Konto für die Vorprüfung", () => {
  it("KAS aus der Wallet, GHOST unbekannt (keine falsche Warnung)", () => {
    const k = walletKeyEntry(A, "ab".repeat(32), 250_000_000n, null, [3]);
    expect(k).toMatchObject({ file: "wallet", address: A, kas: 2.5, vaults: [3] });
    expect(k.ghost).toBe(Number.POSITIVE_INFINITY);
    expect(walletKeyEntry(A, null, null, 1, []).kas).toBeNull();
  });
});
