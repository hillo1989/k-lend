import { describe, expect, it, vi } from "vitest";
import {
  addressProblem,
  attachBody,
  callProbeApi,
  canSend,
  exportBody,
  forgetProbe,
  kastleSigner,
  kaswareSigner,
  loadProbe,
  networkProblem,
  parseProbe,
  payBody,
  probeSigner,
  reportLines,
  saveProbe,
  signedToString,
  stages,
  type AttachResult,
  type ExportResult,
  type ProbeFile,
} from "./probe";

const ADDR = "kaspa:qpt7kd3c7505m3wg6knnyj68m7v7s9k0e3d4avfyt0yvnqpfl8n8gqh5jc602";

const exp = (over: Partial<ExportResult> = {}): ExportResult => ({
  ok: true,
  stage: "tresor-cancel",
  network: "mainnet",
  address: ADDR,
  dryOnly: false,
  feeSompi: 270375,
  outputs: [{ index: 0, sompi: 149729625, kas: 1.49729625, address: ADDR, what: "zurück an die Wallet" }],
  signInputs: [{ index: 0, kind: "tresor", entry: "cancel", argPos: 0, redeemHex: "6b08" }],
  kastle: { networkId: "mainnet", txJson: '{"id":"00"}', scripts: [{ inputIndex: 0, scriptHex: "6b08", signType: "All" }] },
  kasware: { txJsonString: '{"id":"00"}', options: { signInputs: [{ index: 0, sighashType: 1 }] } },
  probe: probe(),
  plan: { kind: "ghost-wallet-plan:1", dryOnly: false },
  ...over,
});

function probe(over: Partial<ProbeFile> = {}): ProbeFile {
  return { kind: "ghost-wallet-probe:1", network: "mainnet", outpoint: "ab:0", value: 150000000, state: { nextDue: 1800000000000, left: 1 }, ...over };
}

/** Wallet mit allen gefährlichen Methoden als Spione: sie dürfen nie aufgerufen werden */
function dangerous() {
  return { pushTx: vi.fn(), sendKaspa: vi.fn(), signAndBroadcastTx: vi.fn(), signMessage: vi.fn() };
}

describe("Wallet-Aufrufe", () => {
  it("KasWare: nur signPskt mit txJsonString und signInputs, nie senden", async () => {
    const d = dangerous();
    const w = {
      ...d,
      requestAccounts: vi.fn(async () => [ADDR]),
      getNetwork: vi.fn(async () => "kaspa_mainnet"),
      signPskt: vi.fn(async () => '{"signed":1}'),
    };
    const s = probeSigner("kasware", { kasware: w })!;
    expect(await s.connect()).toBe(ADDR);
    expect(networkProblem(await s.network(), "mainnet")).toBeNull();
    expect(await s.sign(exp())).toBe('{"signed":1}');
    expect(w.signPskt).toHaveBeenCalledWith({ txJsonString: '{"id":"00"}', options: { signInputs: [{ index: 0, sighashType: 1 }] } });
    for (const f of Object.values(d)) expect(f).not.toHaveBeenCalled();
  });

  it("Kastle: nur signTx(networkId, txJson, scripts), nie signAndBroadcastTx", async () => {
    const d = dangerous();
    const w = {
      ...d,
      connect: vi.fn(async () => true),
      getAccount: vi.fn(async () => ({ address: ADDR, publicKey: "02ab" })),
      getNetwork: vi.fn(async () => "mainnet"),
      signTx: vi.fn(async () => ({ id: "00", inputs: [] })),
    };
    const s = kastleSigner(w);
    expect(await s.connect()).toBe(ADDR);
    expect(await s.sign(exp())).toBe('{"id":"00","inputs":[]}');
    expect(w.signTx).toHaveBeenCalledWith("mainnet", '{"id":"00"}', [{ inputIndex: 0, scriptHex: "6b08", signType: "All" }]);
    for (const f of Object.values(d)) expect(f).not.toHaveBeenCalled();
  });

  it("Ablehnungen und leere Antworten werden Fehler", async () => {
    await expect(kastleSigner({ connect: async () => false, getAccount: async () => null, getNetwork: async () => "", signTx: async () => "" }).connect()).rejects.toThrow();
    await expect(kaswareSigner({ requestAccounts: async () => [], getNetwork: async () => "", signPskt: async () => "" }).connect()).rejects.toThrow();
    expect(() => signedToString("")).toThrow();
    expect(() => signedToString(undefined)).toThrow();
    expect(probeSigner("kastle", {})).toBeNull();
  });

  it("Netz und Adresse", () => {
    expect(networkProblem("kaspa_mainnet", "mainnet")).toBeNull();
    expect(networkProblem("testnet-10", "mainnet")).toMatch(/Testnet 10/);
    expect(networkProblem("kaspa_testnet_10", "testnet-10")).toBeNull();
    expect(addressProblem(ADDR, "mainnet")).toBeNull();
    expect(addressProblem("kaspa:pq1234", "mainnet")).toMatch(/Schnorr/);
    expect(addressProblem(ADDR, "testnet-10")).not.toBeNull();
  });
});

describe("Anfragen an den lokalen Server", () => {
  it("export je Stufe", () => {
    expect(exportBody("dry-cancel", "mainnet", ADDR)).toEqual({ op: "export", network: "mainnet", stage: "dry-cancel", address: ADDR });
    expect(exportBody("tresor-open", "mainnet", ADDR, { fund: "1.5", dueMinutes: 60 })).toEqual({
      op: "export", network: "mainnet", stage: "tresor-open", address: ADDR, fund: "1.5", dueMinutes: 60,
    });
    expect(() => exportBody("tresor-cancel", "mainnet", ADDR)).toThrow(/Stufe 1/);
    expect(exportBody("tresor-cancel", "mainnet", ADDR, { probe: probe() }).probe).toEqual(probe());
  });

  it("attach: prüfen ohne Senden; senden nur mit Bestätigung, nie Trockenprobe", () => {
    const plan = { kind: "ghost-wallet-plan:1", dryOnly: false };
    expect(attachBody("mainnet", plan, "{}", false)).toEqual({ op: "attach", network: "mainnet", plan, signed: "{}" });
    expect(attachBody("mainnet", plan, "{}", true)).toMatchObject({ send: true, confirmMainnet: true });
    expect(attachBody("testnet-10", plan, "{}", true)).not.toHaveProperty("confirmMainnet");
    expect(() => attachBody("mainnet", { ...plan, dryOnly: true }, "{}", true)).toThrow(/nie gesendet/);
    expect(payBody("mainnet", probe(), false)).toEqual({ op: "pay", network: "mainnet", probe: probe() });
    expect(payBody("mainnet", probe(), true)).toMatchObject({ send: true, confirmMainnet: true });
  });

  it("callProbeApi setzt die Kopfzeilen, die der Server verlangt", async () => {
    const f = vi.fn(async () => ({ json: async () => ({ ok: true }) }));
    expect(await callProbeApi(f, { op: "export" })).toEqual({ ok: true });
    expect(f).toHaveBeenCalledWith("/api/wallet-probe", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Ghost-Client": "1" },
      body: '{"op":"export"}',
    });
  });
});

describe("Senden-Knopf und Bericht", () => {
  const valid: AttachResult = { ok: true, valid: true, dryOnly: false, sent: false };
  it("canSend nur bei gültiger, sendbarer, noch nicht gesendeter Tx", () => {
    expect(canSend("tresor-open", valid)).toBe(true);
    expect(canSend("tresor-cancel", valid)).toBe(true);
    expect(canSend("dry-cancel", valid)).toBe(false);
    expect(canSend("tresor-open", { ...valid, dryOnly: true })).toBe(false);
    expect(canSend("tresor-open", { ...valid, valid: false })).toBe(false);
    expect(canSend("tresor-open", { ...valid, sent: true })).toBe(false);
    expect(canSend("tresor-open", null)).toBe(false);
    expect(stages().filter((s) => s.sendable).map((s) => s.id)).toEqual(["tresor-open", "tresor-cancel"]);
  });

  it("reportLines nennt Gültigkeit, Hashtype und Gründe", () => {
    const r: AttachResult = {
      ok: true,
      valid: false,
      report: {
        valid: false,
        changed: ["Ausgang 0: Betrag"],
        ignored: ["Speichermasse"],
        inputs: [{ index: 0, kind: "tresor", signed: true, scriptLen: 66, hashType: 0x81, sigValid: false, note: "Hashtype 0x81 statt 0x01 (ALL) – abgelehnt" }],
        plannedUnits: [102061],
        usedUnits: [],
        budgets: [],
        budgetsRaised: false,
        fee: 0,
        minFee: 0,
        error: "Die Wallet hat die Tx verändert",
      },
    };
    const t = reportLines(r).join("\n");
    expect(t).toMatch(/UNGÜLTIG/);
    expect(t).toMatch(/Hashtype 0x81/);
    expect(t).toMatch(/Ausgang 0: Betrag/);
    expect(t).toMatch(/Speichermasse/);
    expect(reportLines({ ok: false, error: "kein Node" })[0]).toMatch(/kein Node/);
  });
});

describe("Probe-Tresor merken", () => {
  it("speichern, laden, vergessen; erfundene und fremde Netze abgelehnt", () => {
    const m = new Map<string, string>();
    const kv = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
    expect(loadProbe(kv, "mainnet", ADDR)).toBeNull();
    saveProbe(kv, "mainnet", ADDR, probe());
    expect(loadProbe(kv, "mainnet", ADDR)).toEqual(probe());
    expect(loadProbe(kv, "testnet-10", ADDR)).toBeNull();
    forgetProbe(kv, "mainnet", ADDR);
    expect(loadProbe(kv, "mainnet", ADDR)).toBeNull();
    expect(() => parseProbe(JSON.stringify(probe({ fictional: true })), "mainnet")).toThrow(/erfundene/);
    expect(() => parseProbe(JSON.stringify(probe()), "testnet-10")).toThrow();
    expect(() => parseProbe("nix", "mainnet")).toThrow(/JSON/);
  });
});
