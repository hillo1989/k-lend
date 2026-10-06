// Audit 20 c (Webserver): A20c-1 (wallet build belegt die Wallet-Plätze, bevor
// die Adresse geprüft ist) und A20c-4 (Namens- und Suchroute im Pool von
// status/price; „ausgelastet“ 20 s zwischengespeichert). Über den Server mit
// einem Ersatz für ghostctl.
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createGhostApi, isBusyBody } from "./api.ts";
import { encodeKaspaAddress } from "./kaspaAddress.ts";
import { buildWalletBuildArgs, buildWalletReceiveArgs, buildWalletTresoreArgs } from "./walletActions.ts";

const A = encodeKaspaAddress("kaspa", 0, Array(32).fill(0x11));
const B = encodeKaspaAddress("kaspa", 0, Array(32).fill(0x22));
/** Angriffsadresse aus dem Audit: Zeichensatz und Länge stimmen, die Prüfsumme nicht */
const BAD = "kaspa:q" + "q".repeat(60);
/** eine Stelle von A geändert */
const TYPO = A.slice(0, -1) + (A.endsWith("q") ? "p" : "q");

/** ghostctl-Ersatz: schreibt jeden Aufruf mit; `body` läuft davor (z. B. sleep) */
function stub(body = "") {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-a20c-"));
  mkdirSync(path.join(dir, "deployments"));
  writeFileSync(
    path.join(dir, "ghostctl"),
    `#!/bin/sh
echo "$*" >> "${dir}/calls.txt"
${body}
echo '{"ok":true,"deployed":true,"utxos":[],"plan":{"kind":"ghost-wallet-action:1"}}'
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

const servers: http.Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) s.close();
});

async function start(dir: string, opts: Parameters<typeof createGhostApi>[1] = {}) {
  const api = createGhostApi(dir, { public: true, ghostctl: path.join(dir, "ghostctl"), trustedProxies: ["127.0.0.0/8"], ...opts });
  const server = http.createServer((req, res) => api(req, res, () => res.end()));
  servers.push(server);
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return (server.address() as AddressInfo).port;
}

function request(port: number, method: string, p: string, body?: unknown, headers: Record<string, string> = {}): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const req = http.request(
      { host: "127.0.0.1", port, method, path: p, headers: { host: `localhost:${port}`, "content-type": "application/json", "x-ghost-client": "1", ...headers } },
      (res) => {
        let b = "";
        res.setEncoding("utf8");
        res.on("data", (c) => (b += c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body: b }));
      },
    );
    req.on("error", reject);
    if (body !== undefined) req.write(JSON.stringify(body));
    req.end();
  });
}

const build = (address: string) => ({ network: "mainnet", action: "close", address, params: { vault: 0 } });
const from = (ip: string) => ({ "x-forwarded-for": ip });

describe("A20c-1: Adresse mit Prüfsumme, bevor ghostctl startet", () => {
  it("falsche Prüfsumme → 400 ohne ghostctl (build, Empfänger, tresore, receive)", async () => {
    const p = stub();
    const port = await start(p.dir);
    for (const address of [BAD, TYPO]) {
      const r = await request(port, "POST", "/api/wallet/build", build(address));
      expect(r.status).toBe(400);
      expect(JSON.parse(r.body).error).toMatch(/Prüfsumme/);
    }
    const send = await request(port, "POST", "/api/wallet/build", { network: "mainnet", action: "send", address: A, params: { to: BAD, kas: "1" } });
    expect(send.status).toBe(400);
    const transfer = await request(port, "POST", "/api/wallet/build", { network: "mainnet", action: "transfer", address: A, params: { to: TYPO, ghost: "1" } });
    expect(transfer.status).toBe(400);
    const tresor = await request(port, "POST", "/api/wallet/build", {
      network: "mainnet",
      action: "tresor-open",
      address: A,
      params: { to: BAD, amount: "1", interval: "monthly", start: "2030-01-01", count: 1 },
    });
    expect(tresor.status).toBe(400);
    expect((await request(port, "GET", `/api/wallet/tresore?network=mainnet&owner=${encodeURIComponent(BAD)}`)).status).toBe(400);
    expect((await request(port, "POST", "/api/wallet/receive", { network: "mainnet", address: TYPO, ghost: "1" })).status).toBe(400);
    expect(p.calls()).toEqual([]);
  });

  it("die reinen Prüffunktionen lehnen ebenso ab; gültige Adressen gehen durch", () => {
    expect(() => buildWalletBuildArgs(build(BAD))).toThrow(/Prüfsumme/);
    expect(() => buildWalletTresoreArgs("mainnet", TYPO)).toThrow(/Prüfsumme/);
    expect(() => buildWalletReceiveArgs({ network: "mainnet", address: BAD, ghost: "1" })).toThrow(/Prüfsumme/);
    expect(buildWalletBuildArgs(build(A)).address).toBe(A);
    // Skript-Adresse (Version 8) als KAS-Empfänger erlaubt, als Wallet-Adresse nicht
    const P2SH = encodeKaspaAddress("kaspa", 8, Array(32).fill(0x33));
    expect(buildWalletBuildArgs({ network: "mainnet", action: "send", address: A, params: { to: P2SH, kas: "1" } }).args).toContain(P2SH);
    expect(() => buildWalletBuildArgs(build(P2SH))).toThrow();
  });

  it("ein Absender: nie zwei Bauten gleichzeitig – der zweite bekommt sofort 429", async () => {
    const p = stub("sleep 1");
    const port = await start(p.dir);
    const [a, b] = await Promise.all([request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.7")), request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.7"))]);
    expect([a.status, b.status].sort()).toEqual([200, 429]);
    const late = [a, b].find((x) => x.status === 429)!;
    expect(JSON.parse(late.body)).toMatchObject({ ok: false, busy: true });
    expect(p.calls()).toHaveLength(1);
    // zwei verschiedene Absender laufen nebeneinander
    const [c, d] = await Promise.all([request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.8")), request(port, "POST", "/api/wallet/build", build(B), from("203.0.113.9"))]);
    expect([c.status, d.status]).toEqual([200, 200]);
  });

  it("je Absender höchstens 6 Bauten je Minute (öffentlich), submit hat sein eigenes Kontingent", async () => {
    const p = stub();
    const port = await start(p.dir);
    for (let i = 0; i < 6; i++) expect((await request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.7"))).status).toBe(200);
    const r = await request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.7"));
    expect(r.status).toBe(429);
    expect(p.calls()).toHaveLength(6);
    // anderer Absender ist nicht betroffen
    expect((await request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.8"))).status).toBe(200);
  });

  it("globales Kontingent für build; Ungültiges verbraucht es nicht", async () => {
    const p = stub();
    const port = await start(p.dir, { buildsPerMinute: 2 });
    expect((await request(port, "POST", "/api/wallet/build", build(BAD), from("198.51.100.1"))).status).toBe(400);
    expect((await request(port, "POST", "/api/wallet/build", build(A), from("198.51.100.2"))).status).toBe(200);
    expect((await request(port, "POST", "/api/wallet/build", build(A), from("198.51.100.3"))).status).toBe(200);
    const r = await request(port, "POST", "/api/wallet/build", build(A), from("198.51.100.4"));
    expect(r.status).toBe(503);
    expect(JSON.parse(r.body)).toMatchObject({ ok: false, busy: true });
    expect(p.calls()).toHaveLength(2);
  });
});

describe("A20c-4: Suche und Namen nicht im Pool von status/price", () => {
  it("eine laufende GHOST-Suche verdrängt status nicht", async () => {
    const p = stub(`case "$*" in *receive*) sleep 1;; esac`);
    const port = await start(p.dir, { maxProcs: 1, maxQueue: 0 });
    const slow = request(port, "POST", "/api/wallet/receive", { network: "mainnet", address: A, ghost: "1" });
    await new Promise((r) => setTimeout(r, 150));
    const st = await request(port, "GET", "/api/status?network=mainnet");
    expect(isBusyBody(st.body)).toBe(false);
    expect(JSON.parse(st.body).deployed).toBe(true);
    expect((await slow).status).toBe(200);
  });

  it("„ausgelastet“ wird nicht zwischengespeichert", async () => {
    const p = stub(`case "$*" in *testnet-10*) sleep 1;; esac`);
    const port = await start(p.dir, { maxProcs: 1, maxQueue: 0 });
    const slow = request(port, "GET", "/api/status?network=testnet-10");
    await new Promise((r) => setTimeout(r, 150));
    const busy = await request(port, "GET", "/api/status?network=mainnet");
    expect(JSON.parse(busy.body)).toMatchObject({ ok: false, busy: true });
    await slow;
    const again = await request(port, "GET", "/api/status?network=mainnet");
    expect(JSON.parse(again.body).deployed).toBe(true);
    expect(p.calls().filter((c) => c.includes("mainnet"))).toHaveLength(1);
  });

  it("voller Wallet-Pool beim Senden heißt „nichts gesendet“, nicht „unklar“", async () => {
    const p = stub(`case "$*" in *build*) sleep 1;; esac`);
    const port = await start(p.dir, { walletMaxProcs: 1, maxQueue: 0 });
    const slow = request(port, "POST", "/api/wallet/build", build(A), from("203.0.113.7"));
    await new Promise((r) => setTimeout(r, 150));
    const plan = { kind: "ghost-wallet-action:1", network: "mainnet", address: A, action: { action: "close", vault: 0 } };
    const r = await request(port, "POST", "/api/wallet/submit", { network: "mainnet", plan, signed: '"{}"', send: true, confirmMainnet: true }, from("203.0.113.8"));
    expect(r.status).toBe(503);
    const j = JSON.parse(r.body);
    expect(j).toMatchObject({ ok: false, busy: true });
    expect(j.unclear).toBeUndefined();
    await slow;
  });
});
