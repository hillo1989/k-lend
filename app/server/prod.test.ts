// Öffentlicher Modus (server/prod.ts, GHOST_PUBLIC=1): Schlüssel-Endpunkte
// werden abgelehnt, bevor ghostctl startet; status/price kommen aus dem Cache;
// höchstens N ghostctl gleichzeitig; nur 127.0.0.1; statische Seite ohne „..“.
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createGhostApi } from "./api.ts";
import { createProdServer } from "./prod.ts";

function project(sleep = 0) {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-prod-"));
  mkdirSync(path.join(dir, "deployments"));
  mkdirSync(path.join(dir, "keys"));
  writeFileSync(path.join(dir, "keys", "mainnet-keeper.json"), '{"secret":"' + "ab".repeat(32) + '"}');
  // Ersatz für ghostctl: schreibt Beginn und Ende jedes Aufrufs mit
  writeFileSync(
    path.join(dir, "ghostctl"),
    `#!/bin/sh\necho "start $*" >> "${dir}/calls.txt"\nsleep ${sleep}\necho "end $*" >> "${dir}/calls.txt"\necho '{"ok":true,"deployed":true,"network":"mainnet"}'\n`,
  );
  chmodSync(path.join(dir, "ghostctl"), 0o755);
  const dist = path.join(dir, "dist");
  mkdirSync(path.join(dist, "assets"), { recursive: true });
  writeFileSync(path.join(dist, "index.html"), "<!doctype html><title>K.Lend</title>");
  writeFileSync(path.join(dist, "assets", "app-abc123.js"), "console.log(1)");
  writeFileSync(path.join(dist, "wallet-probe.html"), "probe");
  writeFileSync(path.join(dir, "geheim.txt"), "nicht ausliefern");
  const lines = () => {
    try {
      return readFileSync(path.join(dir, "calls.txt"), "utf8").trim().split("\n");
    } catch {
      return [];
    }
  };
  const calls = () => lines().filter((l) => l.startsWith("start "));
  /** größte Zahl gleichzeitig laufender Aufrufe */
  const peak = () => {
    let n = 0;
    let max = 0;
    for (const l of lines()) {
      n += l.startsWith("start ") ? 1 : -1;
      max = Math.max(max, n);
    }
    return max;
  };
  return { dir, dist, calls, peak };
}

interface Res {
  status: number;
  body: string;
  headers: http.IncomingHttpHeaders;
}

function request(port: number, method: string, p: string, headers: Record<string, string> = {}, body?: string): Promise<Res> {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: "127.0.0.1", port, method, path: p, headers: { host: `localhost:${port}`, ...headers } }, (res) => {
      let b = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (b += c));
      res.on("end", () => resolve({ status: res.statusCode ?? 0, body: b, headers: res.headers }));
    });
    req.on("error", reject);
    if (body) req.write(body);
    req.end();
  });
}

const POST = { "content-type": "application/json", "x-ghost-client": "1" };
const servers: http.Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) s.close();
  delete process.env.GHOST_PUBLIC;
});

async function start(p: ReturnType<typeof project>, extra: { domains?: string[]; maxProcs?: number } = {}) {
  const { server } = await createProdServer({ projectDir: p.dir, distDir: p.dist, ghostctl: path.join(p.dir, "ghostctl"), wallet: null, ...extra });
  servers.push(server);
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return (server.address() as AddressInfo).port;
}

describe("öffentlicher Modus: Schlüssel-Endpunkte gesperrt", () => {
  it("Aktion, Schlüssel anlegen, Empfang, Nachrichten, Daueraufträge, Tresore und Wallet-Probe → 403, ghostctl startet nie", async () => {
    const p = project();
    const port = await start(p);
    const action = JSON.stringify({ network: "mainnet", action: "send", params: { key: "keys/mainnet-keeper.json", to: "keys/mainnet-keeper.json", kas: "1" }, dryRun: false, confirmMainnet: true });
    const denied = [
      await request(port, "POST", "/api/action", POST, action),
      await request(port, "POST", "/api/keygen", POST, JSON.stringify({ network: "mainnet", name: "neu-key" })),
      await request(port, "POST", "/api/receive", POST, JSON.stringify({ network: "mainnet", key: "keys/mainnet-keeper.json", ghost: "1" })),
      await request(port, "POST", "/api/wallet-probe", POST, "{}"),
      await request(port, "GET", "/api/messages?network=mainnet&key=keys/mainnet-keeper.json"),
      await request(port, "GET", "/api/abos?network=mainnet"),
      await request(port, "GET", "/api/tresore?network=mainnet"),
    ];
    for (const r of denied) {
      expect(r.status).toBe(403);
      expect(JSON.parse(r.body)).toMatchObject({ ok: false, public: true });
    }
    expect(p.calls()).toEqual([]);
  });

  it("auch createGhostApi selbst mit GHOST_PUBLIC=1 (z. B. vite preview) lehnt ab", async () => {
    process.env.GHOST_PUBLIC = "1";
    const p = project();
    const api = createGhostApi(p.dir);
    const server = http.createServer((req, res) => api(req, res, () => res.end("statisch")));
    servers.push(server);
    await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
    const port = (server.address() as AddressInfo).port;
    const r = await request(port, "POST", "/api/action", POST, JSON.stringify({ network: "mainnet", action: "mint", params: { key: "keys/mainnet-keeper.json", vault: 0, ghost: "1" }, dryRun: true }));
    expect(r.status).toBe(403);
    expect(p.calls()).toEqual([]);
    // und der Takt für Daueraufträge läuft nicht
    writeFileSync(path.join(p.dir, "deployments", "mainnet-abos.json"), JSON.stringify({ abos: [{ id: "0a1b2c3d", nextDue: "2000-01-01" }], archive: [] }));
    await api.aboTick();
    expect(p.calls()).toEqual([]);
  });

  it("keys antwortet mit leerer Liste, ohne ghostctl und ohne Schlüsseldateien zu öffnen", async () => {
    const p = project();
    const port = await start(p);
    const r = await request(port, "GET", "/api/keys?network=mainnet");
    expect(r.status).toBe(200);
    expect(JSON.parse(r.body)).toMatchObject({ ok: true, public: true, keys: [] });
    expect(r.body).not.toContain("abab"); // kein Geheimnis aus keys/
    expect(p.calls()).toEqual([]);
  });
});

describe("öffentlicher Modus: lesende Endpunkte", () => {
  it("status und price: viele Besucher, je ein ghostctl-Aufruf (Cache)", async () => {
    const p = project(0.2);
    const port = await start(p);
    const rs = await Promise.all([
      ...Array.from({ length: 10 }, () => request(port, "GET", "/api/status?network=mainnet")),
      ...Array.from({ length: 10 }, () => request(port, "GET", "/api/price")),
    ]);
    for (const r of rs) expect(r.status).toBe(200);
    await request(port, "GET", "/api/status?network=mainnet");
    expect(p.calls().sort()).toEqual(["start --json price", "start --network mainnet status --json"]);
  });

  it("höchstens maxProcs ghostctl gleichzeitig", async () => {
    const p = project(0.4);
    const port = await start(p, { maxProcs: 1 });
    const rs = await Promise.all([
      request(port, "GET", "/api/status?network=mainnet"),
      request(port, "GET", "/api/status?network=testnet-10"),
      request(port, "GET", "/api/price"),
    ]);
    for (const r of rs) expect(r.status).toBe(200);
    expect(p.calls()).toHaveLength(3);
    expect(p.peak()).toBe(1);
  });

  it("Zeitlimit: hängendes ghostctl liefert eine Fehlermeldung statt ewig zu warten", async () => {
    const p = project(5);
    const api = createGhostApi(p.dir, { public: true, readTimeoutMs: 300 });
    const server = http.createServer((req, res) => api(req, res, () => res.end()));
    servers.push(server);
    await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
    const t0 = Date.now();
    const r = await request((server.address() as AddressInfo).port, "GET", "/api/price");
    expect(Date.now() - t0).toBeLessThan(3000);
    expect(JSON.parse(r.body)).toMatchObject({ ok: false });
    expect(JSON.parse(r.body).error).toMatch(/Zeitüberschreitung/);
  });
});

describe("Host und Herkunft hinter Caddy", () => {
  it("eigene Domain mit HTTPS-Origin erlaubt, fremde Domain und fremder Origin abgelehnt", async () => {
    const p = project();
    const port = await start(p, { domains: ["ghost.example.org"] });
    expect((await request(port, "GET", "/api/keys", { host: "ghost.example.org", origin: "https://ghost.example.org" })).status).toBe(200);
    expect((await request(port, "GET", "/api/keys", { host: "boese.example" })).status).toBe(403);
    expect((await request(port, "GET", "/api/keys", { host: "ghost.example.org", origin: "https://boese.example" })).status).toBe(403);
    expect((await request(port, "GET", "/api/keys", { host: "ghost.example.org", "sec-fetch-site": "cross-site" })).status).toBe(403);
  });
});

describe("statische Seite", () => {
  it("Startseite, Assets mit langem Cache, Rückfall auf index.html", async () => {
    const p = project();
    const port = await start(p);
    const home = await request(port, "GET", "/");
    expect(home.status).toBe(200);
    expect(home.headers["content-type"]).toMatch(/text\/html/);
    expect(home.headers["x-frame-options"]).toBe("DENY");
    // A17-9: Content-Security-Policy auf Seite und API
    const csp = String(home.headers["content-security-policy"]);
    for (const d of ["default-src 'self'", "script-src 'self'", "style-src 'self'", "frame-ancestors 'none'", "manifest-src 'self'", "worker-src 'self'", "connect-src 'self'", "object-src 'none'"])
      expect(csp).toContain(d);
    expect(csp).not.toMatch(/unsafe-inline|unsafe-eval|\*/);
    expect((await request(port, "GET", "/api/keys")).headers["content-security-policy"]).toBe(csp);
    const js = await request(port, "GET", "/assets/app-abc123.js");
    expect(js.headers["cache-control"]).toMatch(/immutable/);
    expect((await request(port, "GET", "/statistik")).body).toMatch(/K.Lend/);
  });
  it("nichts außerhalb von dist, keine Wallet-Probe, kein POST", async () => {
    const p = project();
    const port = await start(p);
    expect((await request(port, "GET", "/../geheim.txt")).body).not.toMatch(/nicht ausliefern/);
    expect((await request(port, "GET", "/%2e%2e/geheim.txt")).body).not.toMatch(/nicht ausliefern/);
    expect((await request(port, "GET", "/wallet-probe.html")).status).toBe(404);
    expect((await request(port, "POST", "/", POST, "{}")).status).toBe(405);
  });
});

describe("A17-9: Seite verträgt die CSP", () => {
  it("index.html ohne Inline-Skript und ohne style-Attribut; Service Worker ohne fremde Quellen", () => {
    const app = path.join(__dirname, "..");
    const html = readFileSync(path.join(app, "index.html"), "utf8");
    const scripts = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)];
    expect(scripts.length).toBeGreaterThan(0);
    for (const m of scripts) {
      expect(m[0]).toMatch(/\bsrc=/);
      expect(m[1].trim()).toBe("");
    }
    expect(html).not.toMatch(/\sstyle=|<style|https?:\/\//);
    expect(html).toContain('rel="manifest" href="./manifest.webmanifest"');
    const sw = readFileSync(path.join(app, "public", "sw.js"), "utf8");
    expect(sw).not.toMatch(/importScripts|https?:\/\//);
  });
});

describe("Wallet-Routen", () => {
  it("/api/wallet/… geht an das Wallet-Modul, mit denselben Host-Prüfungen", async () => {
    const p = project();
    const seen: string[] = [];
    const { server } = await createProdServer({
      projectDir: p.dir,
      distDir: p.dist,
      ghostctl: path.join(p.dir, "ghostctl"),
      wallet: (req, res) => {
        seen.push(String(req.url));
        res.end('{"ok":true}');
      },
    });
    servers.push(server);
    await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
    const port = (server.address() as AddressInfo).port;
    expect((await request(port, "GET", "/api/wallet/balance?address=x")).body).toBe('{"ok":true}');
    expect((await request(port, "GET", "/api/wallet/balance", { host: "boese.example" })).status).toBe(403);
    expect(seen).toEqual(["/api/wallet/balance?address=x"]);
  });
});

describe("GHOST_HOST", () => {
  it("nur Loopback und private Netze", async () => {
    const { listenHosts } = await import("./prod.ts");
    expect(listenHosts(undefined)).toEqual(["127.0.0.1"]);
    expect(listenHosts("127.0.0.1,172.18.0.1")).toEqual(["127.0.0.1", "172.18.0.1"]);
    expect(listenHosts("10.0.0.5")).toEqual(["10.0.0.5"]);
    for (const bad of ["0.0.0.0", "162.55.185.54", "172.32.0.1", "8.8.8.8", "::", "localhost"]) {
      expect(() => listenHosts(bad), bad).toThrow(/keine Loopback- oder private Adresse/);
    }
  });
});
