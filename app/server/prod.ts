// Produktionsserver der Lending-Seite für den eigenen Server (deploy/hetzner/).
//
//   node server/prod.ts          (Node ≥ 22.18 führt TypeScript direkt aus)
//
// - liefert die mit `vite build` gebaute Seite aus app/dist aus;
// - beantwortet die API IMMER im öffentlichen Modus (server/api.ts): status,
//   price, history und keys (leer) sowie die Browser-Wallet-Routen
//   /api/wallet/build und /api/wallet/submit (server/walletActions.ts,
//   docs/wallet-aktionen.md). Über diese Routen wird auch gesendet – aber nur
//   Tx, die der Besucher in seiner Wallet signiert hat; der Server hat keine
//   Schlüssel. Alles mit Schlüsseldateien (Aktionen, Daueraufträge, Tresore,
//   Wallet-Probe) wird abgelehnt, bevor ghostctl startet;
// - reicht /api/wallet/… an ein eigenes Wallet-Modul weiter, wenn es eines
//   gibt (server/wallet.ts o. ä., siehe WALLET_MODULES), sonst an api.ts;
// - setzt Sicherheitskopfzeilen inkl. Content-Security-Policy (CSP unten);
// - lauscht nur auf Loopback bzw. privaten Adressen (GHOST_HOST); davor steht
//   Caddy (HTTPS), nginx oder Apache, ggf. in einem Docker-Container.
//
// Umgebungsvariablen (deploy/hetzner/ghost-web.service):
//   GHOST_PORT         Port (Standard 8787)
//   GHOST_HOST         Adresse(n), durch Komma getrennt (Standard 127.0.0.1). Nur
//                      Loopback oder private Netze, z. B. das Gateway eines
//                      Docker-Netzes, damit ein Webserver im Container die Seite
//                      erreicht (172.18.0.1); 0.0.0.0 und öffentliche Adressen
//                      werden abgelehnt.
//   GHOST_DOMAIN       eigene Domain(s), durch Komma getrennt; Host-Kopfzeile,
//                      die die API außer localhost annimmt (gegen DNS-Rebinding)
//   GHOSTCTL_BIN       Programm (Standard: Release-Build unter
//                      vendor/silverscript/target/release/ghostctl, sonst ./ghostctl)
//   GHOST_PROJECT_DIR  Projektordner mit deployments/ (Standard: Ordner über app/)
//   GHOST_DIST         gebaute Seite (Standard: app/dist)
//   GHOST_MAX_PROCS    höchstens so viele ghostctl gleichzeitig (Standard 2;
//                      die Wallet-Routen haben zusätzlich eigene 2)
//   GHOST_TRUSTED_PROXY Proxys, deren X-Forwarded-For für die Ratenbegrenzung
//                      zählt (Adressen oder CIDR, durch Komma getrennt).
//                      Standard: Loopback und je private IPv4 aus GHOST_HOST
//                      deren /16, also das Docker-Netz des Webservers
//                      (172.18.0.1 → 172.18.0.0/16). Audit 17 A17-3.
import { existsSync, readFileSync, statSync } from "node:fs";
import http, { type IncomingMessage, type ServerResponse } from "node:http";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createGhostApi } from "./api.ts";
import { trustedProxies } from "./walletActions.ts";

const APP_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

type Middleware = (req: IncomingMessage, res: ServerResponse, next: () => void) => void;

/** Kandidaten für die Browser-Wallet-Routen (/api/wallet/…), werden parallel gebaut */
const WALLET_MODULES = ["wallet.ts", "walletApi.ts", "wallet-api.ts"];
const WALLET_FACTORIES = ["createWalletApi", "createWalletHandler", "default"];

const MIME: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".webmanifest": "application/manifest+json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".ico": "image/x-icon",
  ".webp": "image/webp",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
  ".wasm": "application/wasm",
  ".map": "application/json; charset=utf-8",
};

/** Seiten, die öffentlich nicht ausgeliefert werden (nur für den eigenen Rechner) */
const HIDDEN = new Set(["/wallet-probe.html"]);

export interface ProdOptions {
  projectDir?: string;
  distDir?: string;
  ghostctl?: string;
  domains?: string[];
  maxProcs?: number;
  /** fertiges Wallet-Modul (Tests); sonst wird WALLET_MODULES gesucht */
  wallet?: Middleware | null;
  /** vertrauenswürdige Proxys (Tests); sonst GHOST_TRUSTED_PROXY bzw. GHOST_HOST */
  trustedProxies?: readonly string[];
}

/** Wallet-Modul laden, falls vorhanden. Ohne Modul: null (Routen dann aus api.ts oder 404). */
export async function loadWalletModule(projectDir: string, ghostctl: string): Promise<Middleware | null> {
  for (const file of WALLET_MODULES) {
    const p = path.join(APP_DIR, "server", file);
    if (!existsSync(p)) continue;
    const mod = (await import(pathToFileURL(p).href)) as Record<string, unknown>;
    for (const name of WALLET_FACTORIES) {
      const f = mod[name];
      if (typeof f === "function") {
        const handler = (f as (dir: string, o: object) => unknown)(projectDir, { ghostctl, public: true });
        if (typeof handler === "function") {
          console.log(`[ghost-web] /api/wallet/ aus server/${file} (${name})`);
          return handler as Middleware;
        }
      }
    }
    console.warn(`[ghost-web] server/${file} gefunden, aber keine Funktion ${WALLET_FACTORIES.join("/")} – /api/wallet/ bleibt bei api.ts`);
  }
  return null;
}

function defaultGhostctl(projectDir: string): string {
  const release = path.join(projectDir, "vendor", "silverscript", "target", "release", "ghostctl");
  return existsSync(release) ? release : path.join(projectDir, "ghostctl");
}

/**
 * Content-Security-Policy der Seite (Audit 17 A17-9):
 * - Skripte, Styles, Schriften, Bilder, Manifest und Service Worker (sw.js)
 *   nur von der eigenen Domain; Vite baut alles in Dateien unter assets/,
 *   ohne Inline-Skript. Bilder auch als data: (kleine Grafiken im CSS).
 * - React setzt `style={{…}}` über das CSSOM (element.style), das fällt nicht
 *   unter style-src – deshalb kein 'unsafe-inline'.
 * - Wallet-Erweiterungen (KasWare, Kastle) laufen als Content-Scripts, die
 *   die CSP der Seite nicht betrifft; Skripte, die sie aus der Erweiterung
 *   nachladen, erlauben chrome-extension: und moz-extension:.
 * - connect-src: nur die eigene API (./api/…); Kurse, Node und REST holt der
 *   Server. frame-ancestors 'none' ersetzt X-Frame-Options (bleibt zusätzlich).
 */
export const CONTENT_SECURITY_POLICY = [
  "default-src 'self'",
  "script-src 'self' chrome-extension: moz-extension:",
  "style-src 'self'",
  "img-src 'self' data:",
  "font-src 'self'",
  "connect-src 'self'",
  "manifest-src 'self'",
  "worker-src 'self'",
  "object-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
  "frame-ancestors 'none'",
].join("; ");

function setCommonHeaders(res: ServerResponse) {
  res.setHeader("X-Content-Type-Options", "nosniff");
  res.setHeader("Referrer-Policy", "no-referrer");
  res.setHeader("X-Frame-Options", "DENY");
  res.setHeader("Cross-Origin-Opener-Policy", "same-origin");
  res.setHeader("Content-Security-Policy", CONTENT_SECURITY_POLICY);
}

/** Statische Dateien aus dist; nie außerhalb davon (keine „..“-Pfade) */
function serveStatic(distDir: string, req: IncomingMessage, res: ServerResponse) {
  const end = (code: number, text: string) => {
    res.statusCode = code;
    res.setHeader("Content-Type", "text/plain; charset=utf-8");
    res.end(text);
  };
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.setHeader("Allow", "GET, HEAD");
    return end(405, "Methode nicht erlaubt");
  }
  let pathname: string;
  try {
    pathname = decodeURIComponent(new URL(req.url ?? "/", "http://localhost").pathname);
  } catch {
    return end(400, "Ungültige Adresse");
  }
  if (pathname.includes("\0") || HIDDEN.has(pathname)) return end(404, "Nicht gefunden");
  if (pathname.endsWith("/")) pathname += "index.html";
  const root = path.resolve(distDir);
  let file = path.resolve(root, "." + pathname);
  if (file !== root && !file.startsWith(root + path.sep)) return end(404, "Nicht gefunden");
  let st = existsSync(file) ? statSync(file) : null;
  // Seitenpfade ohne Dateiendung fallen auf die Startseite zurück
  if ((!st || !st.isFile()) && !path.extname(pathname)) {
    file = path.join(root, "index.html");
    st = existsSync(file) ? statSync(file) : null;
  }
  if (!st || !st.isFile()) return end(404, "Nicht gefunden");
  const ext = path.extname(file).toLowerCase();
  res.statusCode = 200;
  res.setHeader("Content-Type", MIME[ext] ?? "application/octet-stream");
  res.setHeader("Content-Length", String(st.size));
  // Vite-Dateien unter assets/ tragen einen Inhalts-Hash im Namen
  const hashed = pathname.startsWith("/assets/");
  res.setHeader("Cache-Control", hashed ? "public, max-age=31536000, immutable" : "no-cache");
  if (req.method === "HEAD") return res.end();
  res.end(readFileSync(file));
}

export async function createProdServer(opts: ProdOptions = {}) {
  const projectDir = path.resolve(opts.projectDir ?? process.env.GHOST_PROJECT_DIR ?? path.join(APP_DIR, ".."));
  const distDir = path.resolve(opts.distDir ?? process.env.GHOST_DIST ?? path.join(APP_DIR, "dist"));
  const ghostctl = opts.ghostctl ?? process.env.GHOSTCTL_BIN ?? defaultGhostctl(projectDir);
  const domains =
    opts.domains ??
    (process.env.GHOST_DOMAIN ?? "")
      .split(",")
      .map((d) => d.trim().toLowerCase())
      .filter(Boolean);
  const maxProcs = opts.maxProcs ?? (Number(process.env.GHOST_MAX_PROCS) > 0 ? Number(process.env.GHOST_MAX_PROCS) : 2);
  const wallet = opts.wallet !== undefined ? opts.wallet : await loadWalletModule(projectDir, ghostctl);

  const trusted = opts.trustedProxies ?? trustedProxies(process.env.GHOST_TRUSTED_PROXY, process.env.GHOST_HOST);

  // immer öffentlich, unabhängig von GHOST_PUBLIC
  const api = createGhostApi(projectDir, { public: true, ghostctl, publicHosts: domains, maxProcs, wallet: wallet ?? undefined, trustedProxies: trusted });

  const server = http.createServer((req, res) => {
    setCommonHeaders(res);
    api(req, res, () => serveStatic(distDir, req, res));
  });
  // Zeitlimits: langsame Verbindungen nicht ewig offen halten. requestTimeout
  // gilt nur für das EMPFANGEN der Anfrage (höchstens 768 kB); die Antwort
  // auf ein Senden darf bis WALLET_SEND_TIMEOUT_MS (170 s) dauern, der
  // Webserver davor wartet 180 s (A17-7).
  server.headersTimeout = 15_000;
  server.requestTimeout = 90_000;
  server.keepAliveTimeout = 5_000;
  return { server, projectDir, distDir, ghostctl, domains, trusted };
}

async function main() {
  const port = Number(process.env.GHOST_PORT ?? 8787);
  const { server, projectDir, distDir, ghostctl, domains, trusted } = await createProdServer();
  if (!existsSync(path.join(distDir, "index.html"))) console.warn(`[ghost-web] ${distDir}/index.html fehlt – erst „npx vite build“ ausführen.`);
  if (!existsSync(ghostctl)) console.warn(`[ghost-web] ghostctl nicht gefunden: ${ghostctl}`);
  if (domains.length === 0) console.warn("[ghost-web] GHOST_DOMAIN ist leer – die API antwortet nur über localhost (z. B. SSH-Tunnel).");
  // nur Loopback bzw. private Adressen: von außen erreichbar ausschließlich über Caddy/nginx
  const hosts = listenHosts(process.env.GHOST_HOST);
  const servers = [server, ...hosts.slice(1).map(() => http.createServer(server.listeners("request")[0] as http.RequestListener))];
  hosts.forEach((h, i) =>
    servers[i].listen(port, h, () => {
      console.log(`[ghost-web] öffentlicher Modus auf http://${h}:${port} (Projekt ${projectDir}, Domain ${domains.join(", ") || "–"}, Proxys ${trusted.join(", ")})`);
    }),
  );
  const stop = () => {
    for (const s of servers.slice(1)) s.close();
    server.close(() => process.exit(0));
  };
  process.on("SIGTERM", stop);
  process.on("SIGINT", stop);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) void main();

/** Lausch-Adressen aus GHOST_HOST: nur 127.0.0.0/8, ::1 und private IPv4-Netze */
export function listenHosts(env: string | undefined): string[] {
  const list = (env ?? "127.0.0.1")
    .split(",")
    .map((x) => x.trim())
    .filter(Boolean);
  if (list.length === 0) return ["127.0.0.1"];
  for (const h of list) {
    const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(h);
    const a = m ? m.slice(1).map(Number) : null;
    const ok =
      h === "::1" ||
      (a !== null &&
        a.every((x) => x <= 255) &&
        (a[0] === 127 || a[0] === 10 || (a[0] === 172 && a[1] >= 16 && a[1] <= 31) || (a[0] === 192 && a[1] === 168)));
    if (!ok) throw new Error(`GHOST_HOST: ${h} ist keine Loopback- oder private Adresse – die Seite darf nur hinter dem Webserver erreichbar sein`);
  }
  return list;
}
