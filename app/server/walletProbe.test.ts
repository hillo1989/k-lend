import { describe, expect, it } from "vitest";
import { decodeKaspaAddress, encodeKaspaAddress } from "./kaspaAddress.ts";
import { ValidationError } from "./actions.ts";
import { buildWalletProbeCall, checkProbeAddress } from "./walletProbe.ts";

const ADDR = "kaspa:qpt7kd3c7505m3wg6knnyj68m7v7s9k0e3d4avfyt0yvnqpfl8n8gqh5jc602";
// A20c-1: gleiche Nutzlast im Testnetz – mit eigener Prüfsumme (die alte Zeichenfolge hatte die des Mainnets)
const TADDR = encodeKaspaAddress("kaspatest", 0, decodeKaspaAddress(ADDR)!.payload);
const plan = { kind: "ghost-wallet-plan:1", dryOnly: false, tx: {} };
const probe = { kind: "ghost-wallet-probe:1", network: "mainnet", outpoint: "ab:0" };

const fails = (r: Record<string, unknown>, msg?: RegExp) => {
  let err: unknown;
  try {
    buildWalletProbeCall(r);
  } catch (e) {
    err = e;
  }
  expect(err).toBeInstanceOf(ValidationError);
  if (msg) expect((err as Error).message).toMatch(msg);
};

describe("buildWalletProbeCall", () => {
  it("export: Trockenprobe ohne Netz-Senden", () => {
    const c = buildWalletProbeCall({ op: "export", network: "mainnet", stage: "dry-cancel", address: ADDR });
    expect(c.args).toEqual(["--network", "mainnet", "--json", "wallet", "export-unsigned", "dry-cancel", "--address", ADDR]);
    expect(c.sends).toBe(false);
    expect(c.files).toEqual({});
  });

  it("export: anlegen mit Startguthaben und Termin, Grenzen", () => {
    const c = buildWalletProbeCall({ op: "export", network: "mainnet", stage: "tresor-open", address: ADDR, fund: "1.50", dueMinutes: 60 });
    expect(c.args.slice(-4)).toEqual(["--fund", "1.5", "--due-minutes", "60"]);
    fails({ op: "export", network: "mainnet", stage: "tresor-open", address: ADDR, fund: "11" }, /10 KAS/);
    fails({ op: "export", network: "mainnet", stage: "tresor-open", address: ADDR, dueMinutes: 5 }, /Minuten/);
    fails({ op: "export", network: "mainnet", stage: "dry-cancel", address: ADDR, fund: "1" }, /nur beim Anlegen/);
  });

  it("export: kündigen braucht die Probe-Datei als Datei", () => {
    const c = buildWalletProbeCall({ op: "export", network: "mainnet", stage: "tresor-cancel", address: ADDR, probe });
    expect(c.args.slice(-2)).toEqual(["--probe", "{probe}"]);
    expect(JSON.parse(c.files.probe!)).toEqual(probe);
    fails({ op: "export", network: "mainnet", stage: "tresor-cancel", address: ADDR }, /Probe-Tresor/);
    fails({ op: "export", network: "mainnet", stage: "tresor-cancel", address: ADDR, probe: { kind: "x" } }, /falsche Art/);
  });

  it("Adressen: nur Schnorr, passendes Netz, keine Optionen einschleusbar", () => {
    expect(checkProbeAddress(` ${ADDR} `, "mainnet")).toBe(ADDR);
    expect(checkProbeAddress(TADDR, "testnet-10")).toBe(TADDR);
    expect(() => checkProbeAddress(TADDR, "mainnet")).toThrow();
    expect(() => checkProbeAddress("--send", "mainnet")).toThrow();
    expect(() => checkProbeAddress("kaspa:pqt7kd3c7505m3wg6knnyj68m7v7s9k0e3d4avfyt0yvnqpfl8n8gqh5jc602", "mainnet")).toThrow();
    fails({ op: "export", network: "mainnet", stage: "dry-cancel", address: ADDR, extra: 1 }, /Unbekannter Parameter/);
    fails({ op: "export", network: "mainnet", stage: "send", address: ADDR }, /Stufe/);
    fails({ op: "push", network: "mainnet" }, /op/);
    fails({ op: "export", network: "devnet", stage: "dry-cancel", address: ADDR }, /Netz/);
  });

  it("attach: prüfen ohne --send und ohne --ja", () => {
    const c = buildWalletProbeCall({ op: "attach", network: "mainnet", plan, signed: '"{}"' });
    expect(c.args).toEqual(["--network", "mainnet", "--json", "wallet", "attach-sigs", "--plan", "{plan}", "--signed", "{signed}"]);
    expect(c.sends).toBe(false);
    expect(c.files.signed).toBe('"{}"');
  });

  it("attach: senden im Mainnet nur mit confirmMainnet, nie die Trockenprobe", () => {
    fails({ op: "attach", network: "mainnet", plan, signed: "{}", send: true }, /ausdrücklicher Bestätigung/);
    const c = buildWalletProbeCall({ op: "attach", network: "mainnet", plan, signed: "{}", send: true, confirmMainnet: true });
    expect(c.args).toEqual(["--network", "mainnet", "--json", "--ja", "wallet", "attach-sigs", "--plan", "{plan}", "--signed", "{signed}", "--send"]);
    expect(c.sends).toBe(true);
    fails({ op: "attach", network: "mainnet", plan: { ...plan, dryOnly: true }, signed: "{}", send: true, confirmMainnet: true }, /nie gesendet/);
    fails({ op: "attach", network: "mainnet", plan, signed: "{}", confirmMainnet: true }, /ohne Senden/);
    fails({ op: "attach", network: "mainnet", plan, signed: "{}", send: "yes" }, /true oder false/);
    fails({ op: "attach", network: "mainnet", plan: { kind: "anders" }, signed: "{}" }, /falsche Art/);
    fails({ op: "attach", network: "mainnet", plan, signed: "" }, /Antwort der Wallet/);
    fails({ op: "attach", network: "mainnet", plan, signed: "x".repeat(60_000) }, /zu groß/);
  });

  it("pay: Sicherheitsnetz", () => {
    expect(buildWalletProbeCall({ op: "pay", network: "mainnet", probe }).args).toEqual(["--network", "mainnet", "--json", "wallet", "probe-pay", "--probe", "{probe}"]);
    expect(buildWalletProbeCall({ op: "pay", network: "testnet-10", probe, send: true }).args.slice(-1)).toEqual(["--send"]);
    fails({ op: "pay", network: "mainnet", probe, send: true }, /Bestätigung/);
  });
});
