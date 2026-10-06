// Audit 20 A20c-7: Kopfzeilen (Permissions-Policy, Cross-Origin-Resource-Policy),
// security.txt und robots.txt aus app/public (vite kopiert sie nach dist).
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { createProdServer, PERMISSIONS_POLICY } from "./prod.ts";

const PUBLIC = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "public");

function project() {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-a20c7-"));
  const dist = path.join(dir, "dist");
  mkdirSync(path.join(dist, ".well-known"), { recursive: true });
  writeFileSync(path.join(dist, "index.html"), "<!doctype html><title>K.Lend</title>");
  // wie vite build: public/ → dist/
  copyFileSync(path.join(PUBLIC, "robots.txt"), path.join(dist, "robots.txt"));
  copyFileSync(path.join(PUBLIC, ".well-known", "security.txt"), path.join(dist, ".well-known", "security.txt"));
  writeFileSync(path.join(dir, "ghostctl"), "#!/bin/sh\necho '{}'\n");
  return { dir, dist };
}

const servers: http.Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) s.close();
});

function get(port: number, p: string): Promise<{ status: number; body: string; headers: http.IncomingHttpHeaders }> {
  return new Promise((resolve, reject) => {
    http
      .get({ host: "127.0.0.1", port, path: p, headers: { host: `localhost:${port}` } }, (res) => {
        let b = "";
        res.setEncoding("utf8");
        res.on("data", (c) => (b += c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body: b, headers: res.headers }));
      })
      .on("error", reject);
  });
}

async function start() {
  const p = project();
  const { server } = await createProdServer({ projectDir: p.dir, distDir: p.dist, ghostctl: path.join(p.dir, "ghostctl"), wallet: null });
  servers.push(server);
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return (server.address() as AddressInfo).port;
}

describe("A20c-7: Kopfzeilen und Dateien für Dritte", () => {
  it("Permissions-Policy und Cross-Origin-Resource-Policy auf Seite und API", async () => {
    const port = await start();
    for (const p of ["/", "/api/keys", "/robots.txt"]) {
      const r = await get(port, p);
      expect(r.headers["cross-origin-resource-policy"]).toBe("same-origin");
      expect(r.headers["permissions-policy"]).toBe(PERMISSIONS_POLICY);
    }
    for (const f of ["camera=()", "microphone=()", "geolocation=()", "payment=()"]) expect(PERMISSIONS_POLICY).toContain(f);
    // Kopieren-Knöpfe brauchen die Zwischenablage
    expect(PERMISSIONS_POLICY).not.toMatch(/clipboard/);
  });

  it("security.txt (RFC 9116) mit Kontakt info@k-lend.com und gültigem Ablaufdatum", async () => {
    const port = await start();
    const r = await get(port, "/.well-known/security.txt");
    expect(r.status).toBe(200);
    expect(r.headers["content-type"]).toMatch(/^text\/plain/);
    expect(r.body).toMatch(/^Contact: mailto:info@k-lend\.com$/m);
    const exp = /^Expires: (\S+)$/m.exec(r.body);
    expect(exp).not.toBeNull();
    const t = Date.parse(exp![1]);
    // RFC 9116: höchstens etwa ein Jahr im Voraus; abgelaufen darf sie nicht sein
    expect(t).toBeGreaterThan(Date.parse("2026-10-06T00:00:00Z"));
    expect(t - Date.parse("2026-10-06T00:00:00Z")).toBeLessThanOrEqual(366 * 24 * 3600 * 1000);
    expect(readFileSync(path.join(PUBLIC, ".well-known", "security.txt"), "utf8")).toContain("Canonical: https://k-lend.com/.well-known/security.txt");
  });

  it("robots.txt sperrt nur die API", async () => {
    const port = await start();
    const r = await get(port, "/robots.txt");
    expect(r.status).toBe(200);
    expect(r.body).toMatch(/^User-agent: \*$/m);
    expect(r.body).toMatch(/^Disallow: \/api\/$/m);
    expect(r.body).not.toMatch(/^Disallow: \/$/m);
  });
});
