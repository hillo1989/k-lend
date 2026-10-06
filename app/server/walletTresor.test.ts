// Tresore mit der Browser-Wallet (Daueraufträge, docs/wallet-aktionen.md):
// Parameterprüfung für tresor-open/-topup/-cancel und die öffentliche, nur
// lesende Route GET /api/wallet/tresore (ghostctl tresor owned) mit einem
// Ersatz für ghostctl.
import { chmodSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createGhostApi } from "./api.ts";
import { publicRouteAllowed, ValidationError } from "./actions.ts";
import { buildWalletBuildArgs, buildWalletSubmitCall, buildWalletTresoreArgs, PLAN_KIND, WALLET_PARAMS } from "./walletActions.ts";

const A = "kaspa:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const B = "kaspa:q" + "pzry9x8gf2tvdw0s3jn54khce6mua7lq".repeat(2).slice(0, 60);
const T = "kaspatest:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const COV = "c0ffee".repeat(10) + "abcd";

function bad(f: () => unknown, re: RegExp) {
  try {
    f();
  } catch (e) {
    expect(e).toBeInstanceOf(ValidationError);
    expect((e as Error).message).toMatch(re);
    return;
  }
  throw new Error("keine Ablehnung");
}

const open = (params: Record<string, unknown>) => buildWalletBuildArgs({ network: "mainnet", action: "tresor-open", address: A, params });
const base = { to: B, amount: "10", interval: "monthly", start: "2027-02-01" };

describe("Tresor mit Browser-Wallet: Argumente für ghostctl wallet build", () => {
  it("feste Parameterlisten (dieselben wie in src/wallet/actions.ts, src/lib/tresorWallet.test.ts)", () => {
    expect(WALLET_PARAMS["tresor-open"]).toEqual(["to", "amount", "interval", "start", "count", "fund", "maxFee", "message"]);
    expect(WALLET_PARAMS["tresor-topup"]).toEqual(["tresor", "kas"]);
    expect(WALLET_PARAMS["tresor-cancel"]).toEqual(["tresor"]);
  });

  it("tresor-open: feste Argumente, Nachricht nur öffentlich", () => {
    expect(open({ ...base, count: 3 }).args.slice(8)).toEqual(["--to", B, "--amount", "10", "--interval", "monthly", "--start", "2027-02-01", "--count", "3"]);
    const a = open({ ...base, interval: "14", fund: "100.5", maxFee: "0.02", message: "Miete" }).args.slice(8);
    expect(a).toEqual(["--to", B, "--amount", "10", "--interval", "14", "--start", "2027-02-01", "--fund", "100.5", "--max-fee", "0.02", "--message=Miete", "--onchain-message"]);
    // eine Nachricht mit Bindestrich bleibt ein Wert
    expect(open({ ...base, count: 1, message: "--send --ja" }).args).toContain("--message=--send --ja");
    expect(open({ ...base, count: 1, message: "--send --ja" }).args).not.toContain("--send");
  });

  it("tresor-open: Empfänger nur Schnorr-Adresse dieses Netzes, nicht man selbst, nie eine Schlüsseldatei", () => {
    bad(() => open({ ...base, to: "keys/a.json", count: 1 }), /Empfänger/);
    bad(() => open({ ...base, to: "ab".repeat(32), count: 1 }), /Empfänger/);
    bad(() => open({ ...base, to: T, count: 1 }), /Empfänger/);
    bad(() => open({ ...base, to: "kaspa:p" + "q".repeat(60), count: 1 }), /Empfänger/);
    bad(() => open({ ...base, to: A, count: 1 }), /eigene Adresse/);
  });

  it("tresor-open: Beträge, Termine, Anzahl, Höchstgebühr und fremde Felder", () => {
    bad(() => open({ ...base, amount: "0.99", count: 1 }), /mindestens 1 KAS/);
    bad(() => open({ ...base, amount: "1e3", count: 1 }), /Zahl mit Punkt/);
    bad(() => open({ ...base, interval: "yearly", count: 1 }), /Intervall/);
    bad(() => open({ ...base, interval: "0", count: 1 }), /Intervall/);
    bad(() => open({ ...base, start: "2027-02-30", count: 1 }), /Datum/);
    bad(() => open({ ...base, count: 0 }), /Anzahl/);
    bad(() => open({ ...base, count: "12345" }), /Anzahl/);
    bad(() => open({ ...base }), /Startguthaben/);
    bad(() => open({ ...base, count: 1, maxFee: "0.001" }), /Höchstgebühr/);
    bad(() => open({ ...base, count: 1, maxFee: "0.2" }), /Höchstgebühr/);
    bad(() => open({ ...base, count: 1, message: "a‮b" }), /Nachricht/);
    bad(() => open({ ...base, count: 1, message: "x".repeat(101) }), /höchstens 100/);
    // verschlüsselt geht mit der Wallet nicht: es gibt kein onchain-Feld
    bad(() => open({ ...base, count: 1, message: "Miete", onchain: false }), /Unerwarteter Parameter „onchain“/);
    for (const k of ["key", "id", "time", "owner", "state"]) bad(() => open({ ...base, count: 1, [k]: "x" }), new RegExp(`Unerwarteter Parameter „${k}“`));
  });

  it("tresor-topup und tresor-cancel: nur die volle Covenant-ID", () => {
    expect(buildWalletBuildArgs({ action: "tresor-topup", address: A, params: { tresor: COV, kas: "5" } }).args.slice(8)).toEqual(["--tresor", COV, "--kas", "5"]);
    expect(buildWalletBuildArgs({ action: "tresor-cancel", address: A, params: { tresor: COV } }).args.slice(8)).toEqual(["--tresor", COV]);
    for (const t of [COV.slice(0, 8), COV.toUpperCase(), `${COV}0`, "../x", 5, null]) {
      bad(() => buildWalletBuildArgs({ action: "tresor-cancel", address: A, params: { tresor: t } }), /Covenant-ID/);
    }
    bad(() => buildWalletBuildArgs({ action: "tresor-topup", address: A, params: { tresor: COV } }), /KAS-Betrag/);
    bad(() => buildWalletBuildArgs({ action: "tresor-cancel", address: A, params: { tresor: COV, key: "keys/a.json" } }), /Unerwarteter Parameter/);
    bad(() => buildWalletBuildArgs({ action: "tresor-pay", address: A, params: { id: "abcdef12" } }), /Unbekannte Aktion/);
    bad(() => buildWalletBuildArgs({ action: "tresor-import", address: A, params: {} }), /Unbekannte Aktion/);
  });

  it("submit eines Tresor-Plans wie jeder Wallet-Plan (Mainnet nur mit Bestätigung)", () => {
    const plan = { kind: PLAN_KIND, network: "mainnet", address: A, action: { action: "tresor-cancel", tresor: COV } };
    bad(() => buildWalletSubmitCall({ plan, signed: "x", send: true }), /Bestätigung/);
    expect(buildWalletSubmitCall({ plan, signed: "x", send: true, confirmMainnet: true }).args).toContain("--send");
  });
});

describe("GET /api/wallet/tresore: Argumente", () => {
  it("nur eine Schnorr-Adresse des gewählten Netzes", () => {
    expect(buildWalletTresoreArgs("mainnet", A)).toEqual(["--network", "mainnet", "--json", "tresor", "owned", A]);
    expect(buildWalletTresoreArgs(undefined, A)).toEqual(["--network", "mainnet", "--json", "tresor", "owned", A]);
    expect(buildWalletTresoreArgs("testnet-10", T)).toEqual(["--network", "testnet-10", "--json", "tresor", "owned", T]);
    bad(() => buildWalletTresoreArgs("mainnet", T), /Netz/);
    bad(() => buildWalletTresoreArgs("devnet", A), /Unbekanntes Netz/);
    bad(() => buildWalletTresoreArgs("mainnet", "keys/a.json"), /Schnorr-Adresse/);
    bad(() => buildWalletTresoreArgs("mainnet", "ab".repeat(32)), /Schnorr-Adresse/);
    bad(() => buildWalletTresoreArgs("mainnet", null), /Adresse/);
  });

  it("öffentlich erlaubt (unter /api/wallet/), die Schlüssel-Routen der Tresore nicht", () => {
    expect(publicRouteAllowed("GET", "/api/wallet/tresore")).toBe(true);
    expect(publicRouteAllowed("GET", "/api/tresore")).toBe(false);
    expect(publicRouteAllowed("POST", "/api/action")).toBe(false);
  });
});

// ---------------------------------------------------- über den Server ----

function project() {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-wallet-tresor-"));
  writeFileSync(
    path.join(dir, "ghostctl"),
    `#!/bin/sh
echo "$*" >> "${dir}/calls.txt"
echo '{"ok":true,"network":"mainnet","owner":"${A}","tresore":[{"id":"c0ffeec0","covenantId":"${COV}","amount":"10","value":"31.03","covered":3,"left":3,"nextDue":1801468800000,"wallet":true,"secretKey":"weg"}]}'
`,
  );
  chmodSync(path.join(dir, "ghostctl"), 0o755);
  const calls = () => {
    try {
      return readFileSync(path.join(dir, "calls.txt"), "utf8").trim().split("\n");
    } catch {
      return [];
    }
  };
  return { dir, calls };
}

function get(port: number, p: string, headers: Record<string, string> = {}): Promise<{ status: number; body: string; headers: http.IncomingHttpHeaders }> {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: "127.0.0.1", port, method: "GET", path: p, headers: { host: `localhost:${port}`, ...headers } }, (res) => {
      let b = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (b += c));
      res.on("end", () => resolve({ status: res.statusCode ?? 0, body: b, headers: res.headers }));
    });
    req.on("error", reject);
    req.end();
  });
}

const servers: http.Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) s.close();
});

async function start(dir: string, opts: Parameters<typeof createGhostApi>[1] = {}) {
  const api = createGhostApi(dir, { public: true, ghostctl: path.join(dir, "ghostctl"), ...opts });
  const server = http.createServer((req, res) =>
    api(req, res, () => {
      res.statusCode = 404;
      res.end();
    }),
  );
  servers.push(server);
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return (server.address() as AddressInfo).port;
}

describe("GET /api/wallet/tresore im öffentlichen Modus", () => {
  it("liefert die Tresore der Adresse; ghostctl bekommt nur geprüfte Argumente", async () => {
    const p = project();
    const port = await start(p.dir);
    const r = await get(port, `/api/wallet/tresore?network=mainnet&owner=${encodeURIComponent(A)}`);
    expect(r.status).toBe(200);
    const j = JSON.parse(r.body);
    expect(j.ok).toBe(true);
    expect(j.tresore[0].covenantId).toBe(COV);
    expect(j.tresore[0].secretKey).toBeUndefined();
    expect(p.calls()).toEqual([`--network mainnet --json tresor owned ${A}`]);
  });

  it("ungültige Anfragen erreichen ghostctl nie", async () => {
    const p = project();
    const port = await start(p.dir);
    for (const q of ["", "?owner=keys%2Fa.json", `?network=testnet-10&owner=${A}`, `?network=x&owner=${A}`, "?owner=--key"]) {
      expect((await get(port, `/api/wallet/tresore${q}`)).status).toBe(400);
    }
    // fremde Seite (no-cors, A10-W-8)
    expect((await get(port, `/api/wallet/tresore?owner=${A}`, { "sec-fetch-site": "cross-site" })).status).toBe(403);
    expect(p.calls()).toEqual([]);
  });

  it("Ratenbegrenzung wie build/submit", async () => {
    const p = project();
    const port = await start(p.dir, { walletPerMinute: 2 });
    const q = `/api/wallet/tresore?owner=${A}`;
    expect((await get(port, q)).status).toBe(200);
    expect((await get(port, q)).status).toBe(200);
    const r = await get(port, q);
    expect(r.status).toBe(429);
    expect(Number(r.headers["retry-after"])).toBeGreaterThan(0);
    expect(p.calls().length).toBe(2);
  });
});
