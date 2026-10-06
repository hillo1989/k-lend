// Lokale API der Lending-Seite. Läuft als Middleware im Vite-Dev- und
// -Preview-Server (siehe vite.config.ts) und ruft ../ghostctl auf.
//
//   GET  /api/status?network=…   ghostctl status --json      (Cache 20 s)
//   GET  /api/keys?network=…     ghostctl keys               (Cache 10 s)
//   GET  /api/price              ghostctl price (Median aus 6 Quellen, Cache 60 s)
//   GET  /api/history?days=…     KAS-Kursverlauf 7/30/365 Tage (CoinGecko, Ersatz Kraken; Cache 10 min)
//   POST /api/action             eine Protokoll-Aktion       (immer nur eine gleichzeitig)
//   POST /api/keygen             neuen Schlüssel anlegen
//   POST /api/receive            eingegangene GHOST suchen (sendet nichts)
//   GET  /api/messages?network=…&key=keys/<name>.json
//                                eingegangene Nachrichten (ghostctl messages, Cache 30 s)
//   GET  /api/abos?network=…     Daueraufträge (ghostctl abo list, ohne Netz)
//   GET  /api/tresore?network=…  Tresore (ghostctl tresor list, ohne Netz)
//   POST /api/wallet-probe       Browser-Wallet-Probe (ghostctl wallet …, server/walletProbe.ts)
//   POST /api/wallet/build       Nutzeraktion für die Browser-Wallet bauen (ghostctl wallet build,
//                                server/walletActions.ts; nur Adressen, nie Schlüssel, sendet nichts)
//   POST /api/wallet/submit      Wallet-Antwort prüfen, auf Wunsch senden (ghostctl wallet submit)
//   POST /api/wallet/receive     eingegangene GHOST für eine Wallet-Adresse suchen (sendet nichts)
//   GET  /api/wallet/name        .k-Name (dotk.name) → Adresse, am eigenen Node geprüft
//   GET  /api/wallet/tresore     Tresore eines Besitzers (ghostctl tresor owned, nur lesend)
//
// Daueraufträge: Solange der Server läuft, stößt er jede Minute
// `ghostctl abo run --ja` an, aber nur wenn laut deployments/<netz>-abos.json
// etwas fällig ist. Doppelt anstoßen (auch zusammen mit dem GHOST-Agenten) ist
// harmlos: ghostctl sperrt den Lauf und schreibt den Termin vor dem Senden fort.
// Ebenso fällige Tresor-Zahlungen (`ghostctl tresor pay --ja`, Gebühr aus dem
// Tresor): doppelt auslösen ist harmlos, die zweite Tx findet die UTXO nicht mehr.
//
// Sicherheit: siehe checkRequest() in actions.ts. Es gibt absichtlich KEINE
// CORS-Kopfzeilen. Der Server lauscht nur auf localhost (vite.config.ts).
//
// Öffentlicher Modus (GHOST_PUBLIC=1 oder Option public, immer in
// server/prod.ts): nur status, price, history, keys (leer, ohne ghostctl) und
// /api/wallet/…; alles andere 403 (publicRouteAllowed in actions.ts). Dazu
// höchstens 2 ghostctl-Prozesse für Lesendes und eigene 2 für die
// Wallet-Routen (Audit 17 A17-4: Wallet-Aufrufe verdrängen Status und Preis
// nicht), einen eigenen für Namensauflösung und GHOST-Suche (A20c-4),
// höchstens 2 submit gleichzeitig, davon 1 mit Senden; wallet build je
// Absender höchstens 6 je Minute und nie zwei gleichzeitig, insgesamt höchstens
// 30 je Minute (A20c-1: ein Absender belegt die Wallet-Plätze nicht); 60 s
// Zeitlimit zum Lesen, 170 s zum Senden (unter den 180 s des Webservers,
// A17-7); kein Abo-Takt. Fehlermeldungen ohne absolute Pfade (A17-8).
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import type { IncomingMessage, ServerResponse } from "node:http";
import path from "node:path";
import {
  aboNeedsRun,
  buildAboListArgs,
  buildAboRunArgs,
  buildActionArgs,
  buildKeygenArgs,
  buildMessagesArgs,
  buildReceiveArgs,
  buildTresorListArgs,
  buildTresorPayArgs,
  checkRequest,
  isNetwork,
  isNodeError,
  localDate,
  NETWORKS,
  NODE_DOWN_MESSAGE,
  PUBLIC_DENIED_MESSAGE,
  publicRouteAllowed,
  tresorNeedsRun,
  ValidationError,
  type Network,
} from "./actions.ts";
import { historyCache, parseDays } from "./history.ts";
import { resolveName } from "./dotkNames.ts";
import { buildWalletProbeCall, PROBE_BODY_LIMIT } from "./walletProbe.ts";
import { buildWalletBuildArgs, buildWalletReceiveArgs, buildWalletSubmitCall, buildWalletTresoreArgs, clientKey, createRateLimiter, isRetryLater, LOOPBACK_PROXIES, WALLET_BODY_LIMIT } from "./walletActions.ts";

/** ghostctl meldet in `transactions` gesendete (sent) oder unklare (unclear) Tx */
function sentOrUnclear(txs: unknown): boolean {
  return Array.isArray(txs) && txs.some((t) => !!t && typeof t === "object" && ((t as { sent?: unknown }).sent === true || (t as { unclear?: unknown }).unclear === true));
}

const STATUS_CACHE_MS = 20_000;
const KEYS_CACHE_MS = 10_000;
const PRICE_CACHE_MS = 60_000;
const MESSAGES_CACHE_MS = 30_000;
const NODE_DOWN_CACHE_MS = 60_000;
const READ_TIMEOUT_MS = 180_000; // erster Aufruf baut ghostctl evtl. noch
/**
 * wallet submit --send: Vorprüfung, Verbinden, zwei Abgleiche, höchstens 30 s
 * auf die Sperre und 45 s (ohne Sperre) auf die Bestätigung (ghostctl.rs
 * WALLET_*). Muss unter dem Zeitlimit des Webservers liegen (nginx
 * proxy_read_timeout / Apache ProxyTimeout 180 s; Caddy hat keines), damit
 * die Seite „unklar“ als JSON bekommt statt eines 504 (A17-7).
 */
export const WALLET_SEND_TIMEOUT_MS = 170_000;
const ACTION_TIMEOUT_MS = 300_000;
const BODY_LIMIT = 10 * 1024;
const ABO_TICK_MS = 60_000;
// Senden wartet in ghostctl bis zu 600 s auf die Bestätigung
const ABO_RUN_TIMEOUT_MS = 700_000;

interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

type Middleware = (req: IncomingMessage, res: ServerResponse, next: () => void) => void;

export interface GhostApiOptions {
  /**
   * Öffentlicher Modus: nur lesende Routen ohne Schlüsseldateien
   * (publicRouteAllowed in actions.ts), keine Daueraufträge/Tresore aus dem
   * Server. Standard: Umgebungsvariable GHOST_PUBLIC=1.
   */
  public?: boolean;
  /** Programm statt <projekt>/ghostctl (z. B. das Release-Binary auf dem Server) */
  ghostctl?: string;
  /** zusätzlich erlaubte Host-Kopfzeilen (eigene Domain hinter Caddy) */
  publicHosts?: readonly string[];
  /** höchstens so viele ghostctl-Prozesse gleichzeitig (öffentlich 2, sonst unbegrenzt) */
  maxProcs?: number;
  /** höchstens so viele wartende Aufrufe, darüber sofort „ausgelastet“ (öffentlich 20) */
  maxQueue?: number;
  /** Zeitlimit lesender Aufrufe in ms (öffentlich 60 s, sonst 180 s) */
  readTimeoutMs?: number;
  /** Browser-Wallet-Routen /api/wallet/… aus einem eigenen Modul (server/prod.ts) */
  wallet?: Middleware;
  /** /api/wallet/build und /submit: höchstens so viele Aufrufe je Absender und Minute (öffentlich 20, sonst 120) */
  walletPerMinute?: number;
  /**
   * Proxys, deren X-Forwarded-For für die Ratenbegrenzung zählt (Adressen oder
   * CIDR; Loopback immer). server/prod.ts: GHOST_TRUSTED_PROXY bzw. das
   * Docker-Netz aus GHOST_HOST (trustedProxies in walletActions.ts).
   */
  trustedProxies?: readonly string[];
  /** eigene ghostctl-Plätze der Wallet-Routen (öffentlich 2, sonst unbegrenzt) */
  walletMaxProcs?: number;
  /** höchstens so viele Sendungen insgesamt je Minute (öffentlich 12) – Audit 18 G-3 */
  sendsPerMinute?: number;
  /** höchstens so viele /api/wallet/submit gleichzeitig (öffentlich 2), davon mit Senden höchstens maxSends (1) */
  maxSubmits?: number;
  maxSends?: number;
  /** Zeitlimit für submit mit Senden (Standard WALLET_SEND_TIMEOUT_MS) */
  walletSendTimeoutMs?: number;
  /** /api/wallet/build: höchstens so viele je Absender und Minute (öffentlich 6) – A20c-1 */
  buildPerMinute?: number;
  /** /api/wallet/build: höchstens so viele insgesamt je Minute (öffentlich 30) – A20c-1 */
  buildsPerMinute?: number;
  /** /api/wallet/name: höchstens so viele je Absender und Minute (öffentlich 30, eigenes Kontingent) */
  namePerMinute?: number;
  /** ghostctl-Plätze für Namensauflösung und GHOST-Suche (öffentlich 1) – A20c-4 */
  lookupMaxProcs?: number;
}

/**
 * Absolute Pfade aus Texten für die öffentliche Seite entfernen (A17-8):
 * „/opt/ghost/kaspa-lending/deployments/mainnet.lock“ → „mainnet.lock“.
 * URLs bleiben (vor ihrem „/“ steht „:“ bzw. ein Buchstabe).
 */
export function redactPaths(s: string): string {
  return s.replace(/(^|[\s"'(=\[])\/(?:[^\s"'():,;\]/]+\/)+([^\s"'():,;\]/]*)/g, (_m, pre: string, last: string) => `${pre}${last || "…"}`);
}

/** redactPaths auf alle Texte eines JSON-Werts unter Fehler-Schlüsseln */
function redactJson(v: unknown, key = ""): unknown {
  if (typeof v === "string") return /error|detail|note|message|hint/i.test(key) ? redactPaths(v) : v;
  if (Array.isArray(v)) return v.map((x) => redactJson(x, key));
  if (v && typeof v === "object") return Object.fromEntries(Object.entries(v as Record<string, unknown>).map(([k, x]) => [k, redactJson(x, k)]));
  return v;
}

/** Ergebnis, wenn zu viele ghostctl-Aufrufe warten */
const BUSY_RESULT: RunResult = { code: -1, stdout: "", stderr: "Server ausgelastet – bitte gleich erneut versuchen.", timedOut: false };
const BUSY_TEXT = BUSY_RESULT.stderr;

/**
 * Antwort „ausgelastet“ (voller Pool): kein Ergebnis, das ein Lese-Cache
 * merken darf (A20c-4) – sonst sähen alle Besucher sie 20 s lang.
 */
export function isBusyBody(body: string): boolean {
  return body.includes('"busy":true');
}

export function createGhostApi(projectDir: string, opts: GhostApiOptions = {}) {
  const isPublic = opts.public ?? process.env.GHOST_PUBLIC === "1";
  const ghostctl = opts.ghostctl ?? path.join(projectDir, "ghostctl");
  const exists = (rel: string) => existsSync(path.join(projectDir, rel));
  const maxProcs = opts.maxProcs ?? (isPublic ? 2 : Infinity);
  const maxQueue = opts.maxQueue ?? (isPublic ? 20 : Infinity);
  const readTimeout = opts.readTimeoutMs ?? (isPublic ? 60_000 : READ_TIMEOUT_MS);

  // Begrenzung paralleler ghostctl-Prozesse (jeder braucht Speicher und eine
  // Node-Verbindung); Überzählige warten, zu viele Wartende werden abgewiesen.
  // Die Wallet-Routen haben eigene Plätze (A17-4).
  function createPool(max: number) {
    let active = 0;
    const waiting: (() => void)[] = [];
    return async function runIn(args: string[], timeout: number): Promise<RunResult> {
      if (active >= max) {
        if (waiting.length >= maxQueue) return BUSY_RESULT;
        // der Platz wird beim Freiwerden direkt übergeben (active bleibt gleich)
        await new Promise<void>((r) => waiting.push(r));
      } else active++;
      try {
        return await spawnGhostctl(args, timeout);
      } finally {
        const nextInLine = waiting.shift();
        if (nextInLine) nextInLine();
        else active--;
      }
    };
  }
  const run = createPool(maxProcs);
  const runWallet = createPool(opts.walletMaxProcs ?? (isPublic ? 2 : Infinity));
  // Namensauflösung (.k → ghostctl utxos) und GHOST-Suche (receive) haben
  // eigene Plätze: sie verdrängen weder status/price noch build/submit (A20c-4)
  const runLookup = createPool(opts.lookupMaxProcs ?? (isPublic ? 1 : Infinity));
  const maxSubmits = opts.maxSubmits ?? (isPublic ? 2 : Infinity);
  const maxSends = opts.maxSends ?? (isPublic ? 1 : Infinity);
  const walletSendTimeout = opts.walletSendTimeoutMs ?? WALLET_SEND_TIMEOUT_MS;
  const trusted = opts.trustedProxies ?? LOOPBACK_PROXIES;
  let submitsActive = 0;
  let sendsActive = 0;

  function spawnGhostctl(args: string[], timeout: number): Promise<RunResult> {
    return new Promise((resolve) => {
      execFile(
        ghostctl,
        args,
        { cwd: projectDir, timeout, killSignal: "SIGTERM", maxBuffer: 16 * 1024 * 1024, env: process.env },
        (err, stdout, stderr) => {
          const e = err as (NodeJS.ErrnoException & { code?: number | string; killed?: boolean; signal?: string }) | null;
          resolve({
            code: e ? (typeof e.code === "number" ? e.code : -1) : 0,
            stdout: String(stdout ?? ""),
            stderr: String(stderr ?? ""),
            timedOut: !!e?.killed && e.signal === "SIGTERM",
          });
        },
      );
    });
  }

  /** Das JSON-Objekt aus stdout holen (bei --json genau eines). */
  function parseJson(out: string): Record<string, unknown> | null {
    const t = out.trim();
    if (!t) return null;
    let j: unknown;
    try {
      j = JSON.parse(t);
    } catch {
      const line = t.split("\n").reverse().find((l) => l.trim().startsWith("{"));
      if (!line) return null;
      try {
        j = JSON.parse(line);
      } catch {
        return null;
      }
    }
    return j && typeof j === "object" && !Array.isArray(j) ? (scrub(j) as Record<string, unknown>) : null;
  }

  // stderr kürzen; lange Hex-Blöcke (mögliche Schlüssel) nie an den Browser (A10-W-10)
  const tail = (s: string) =>
    s.trim().split("\n").slice(-4).join("\n").replace(/[0-9a-fA-F]{64,}/g, "‹hex›").slice(0, 600);

  /** Felder mit geheimem Inhalt entfernen, bevor JSON den Browser erreicht (Verteidigung in der Tiefe) */
  function scrub(v: unknown): unknown {
    if (Array.isArray(v)) return v.map(scrub);
    if (v && typeof v === "object")
      return Object.fromEntries(
        Object.entries(v as Record<string, unknown>)
          .filter(([k]) => !/secret|private|mnemonic|seed/i.test(k))
          .map(([k, x]) => [k, scrub(x)]),
      );
    return v;
  }

  // ---- Lese-Caches (je Netz) mit Zusammenlegen gleichzeitiger Abrufe
  function cached(ttl: number, fetcher: (n: Network) => Promise<string>) {
    const store = new Map<Network, { at: number; body: string; ttl: number }>();
    const inflight = new Map<Network, { p: Promise<string>; gen: number }>();
    // Generation: ein Abruf, der vor clear() begann, darf den Cache danach
    // nicht mit dem alten Stand füllen (A10-W-7)
    let gen = 0;
    return {
      async get(n: Network): Promise<string> {
        const hit = store.get(n);
        if (hit && Date.now() - hit.at < hit.ttl) return hit.body;
        let f = inflight.get(n);
        if (!f || f.gen !== gen) {
          const p: Promise<string> = fetcher(n).finally(() => {
            if (inflight.get(n)?.p === p) inflight.delete(n);
          });
          f = { p, gen };
          inflight.set(n, f);
        }
        const body = await f.p;
        // „ausgelastet“ ist kein Ergebnis: nicht merken, der nächste Abruf versucht es neu (A20c-4)
        if (isBusyBody(body)) return body;
        // Nodes nicht erreichbar: länger merken, jeder Versuch dauert sonst 1–2 Minuten
        if (f.gen === gen)
          store.set(n, { at: Date.now(), body, ttl: body.includes('"nodeDown":true') || body.includes('"offline":true') ? NODE_DOWN_CACHE_MS : ttl });
        return body;
      },
      clear() {
        gen++;
        store.clear();
        inflight.clear();
      },
    };
  }

  /** Fehler als JSON-Antwort (HTTP 200, damit der Browser keine Netzwerkfehler loggt). */
  const errorBody = (detail: string) =>
    JSON.stringify(
      isNodeError(detail)
        ? { ok: false, nodeDown: true, error: NODE_DOWN_MESSAGE, detail }
        : { ok: false, error: detail },
    );

  // Lesende Abfragen liefern immer JSON: Erfolg unverändert, Fehler als {ok:false,…}.
  // Fehler werden wie Erfolge zwischengespeichert, damit bei ausgefallenen Nodes
  // nicht jede Anfrage erneut 1–2 Minuten auf Zeitüberschreitungen wartet.
  const readJson = async (args: string[], what: string) => {
    const r = await run(args, readTimeout);
    if (r === BUSY_RESULT) return JSON.stringify({ ok: false, busy: true, error: BUSY_TEXT });
    if (r.timedOut) return errorBody(`${what}: Zeitüberschreitung (Nodes antworten nicht)`);
    const j = parseJson(r.stdout);
    if (!j) return errorBody(`${what} fehlgeschlagen: ${tail(r.stderr) || "keine Ausgabe"}`);
    if (j.ok === false) return errorBody(String(j.error ?? `${what} fehlgeschlagen`));
    if (typeof j.error === "string" && isNodeError(j.error)) return JSON.stringify({ ...j, nodeDown: true });
    return JSON.stringify(j);
  };
  const statusCache = cached(STATUS_CACHE_MS, (n) => readJson(["--network", n, "status", "--json"], "ghostctl status"));
  const keysCache = cached(KEYS_CACHE_MS, (n) => readJson(["--network", n, "--json", "keys"], "ghostctl keys"));
  // Marktpreis braucht keinen Node; netzunabhängig, daher immer unter „mainnet“ gecacht
  const priceCache = cached(PRICE_CACHE_MS, () => readJson(["--json", "price"], "ghostctl price"));
  const history = historyCache();
  // Nachrichten je Netz und Schlüssel; gleichzeitige Abrufe zusammengelegt
  const messagesStore = new Map<string, { at: number; p: Promise<string> }>();
  function messagesGet(args: string[], id: string): Promise<string> {
    const hit = messagesStore.get(id);
    if (hit && Date.now() - hit.at < MESSAGES_CACHE_MS) return hit.p;
    const p = readJson(args, "ghostctl messages");
    messagesStore.set(id, { at: Date.now(), p });
    if (messagesStore.size > 50) messagesStore.delete(messagesStore.keys().next().value as string);
    return p;
  }

  let busy = false; // immer nur eine Aktion gleichzeitig
  // Wallet-Routen: viele Besucher, deshalb kein globales „busy“, sondern
  // Ratenbegrenzung je Absender plus die Prozessbegrenzung von run()
  const walletLimiter = createRateLimiter(opts.walletPerMinute ?? (isPublic ? 20 : 120));
  // A20c-1: wallet build kostet je Aufruf einen Wallet-Platz für 11–13 s
  // (ghostctl verbindet sich mit dem Node). Je Absender höchstens
  // buildPerMinute und nie zwei gleichzeitig, insgesamt höchstens
  // buildsPerMinute – so belegt kein Absender beide Plätze dauerhaft.
  const buildLimiter = createRateLimiter(opts.buildPerMinute ?? (isPublic ? 6 : 120));
  const buildsLimiter = createRateLimiter(opts.buildsPerMinute ?? (isPublic ? 30 : 600));
  // .k-Namen: eigenes Kontingent, damit Tippen nicht das von build/submit aufbraucht (A20d-10)
  const nameLimiter = createRateLimiter(opts.namePerMinute ?? (isPublic ? 30 : 240));
  /** laufende ghostctl-Aufrufe der Wallet-Plätze je Absender (build, tresore) */
  const walletActive = new Map<string, number>();
  async function oneAtATime<T>(sender: string, f: () => Promise<T>): Promise<T | null> {
    if ((walletActive.get(sender) ?? 0) >= 1) return null;
    walletActive.set(sender, 1);
    try {
      return await f();
    } finally {
      walletActive.delete(sender);
    }
  }
  const tooMany = (res: ServerResponse, wait: number) => {
    res.setHeader("Retry-After", String(wait));
    return send(res, 429, { ok: false, error: `Zu viele Anfragen – bitte in ${wait} s erneut versuchen.` });
  };
  const stillRunning = (res: ServerResponse) => {
    res.setHeader("Retry-After", "5");
    return send(res, 429, { ok: false, busy: true, error: "Deine vorige Anfrage läuft noch – bitte kurz warten. Es wurde nichts gesendet." });
  };
  // Audit 18 G-3: Sendungen insgesamt begrenzen (unabhängig vom Absender) und
  // endgültig abgewiesene Pläne einige Minuten sofort ablehnen, damit ein alter,
  // gültig signierter Plan den Sende-Platz nicht immer wieder belegt
  const sendLimiter = createRateLimiter(opts.sendsPerMinute ?? (isPublic ? 12 : 120));
  const rejectedPlans = new Map<string, number>();
  const REJECT_MS = 5 * 60_000;
  const planKey = (files: Record<string, string>) => createHash("sha256").update(files.plan).update("\0").update(files.signed).digest("hex");

  function send(res: ServerResponse, code: number, body: unknown) {
    res.statusCode = code;
    res.setHeader("Content-Type", "application/json; charset=utf-8");
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("X-Content-Type-Options", "nosniff");
    if (code === 413) res.setHeader("Connection", "close");
    let text = typeof body === "string" ? body : JSON.stringify(body);
    // öffentlich: keine absoluten Server-Pfade in Fehlermeldungen (A17-8)
    if (isPublic && /"(error|detail|note|report)"/.test(text)) {
      try {
        text = JSON.stringify(redactJson(JSON.parse(text)));
      } catch {
        text = redactPaths(text);
      }
    }
    res.end(text);
  }

  function readBody(req: IncomingMessage, limit = BODY_LIMIT): Promise<unknown> {
    return new Promise((resolve, reject) => {
      let size = 0;
      let tooBig = false;
      const chunks: Buffer[] = [];
      req.on("data", (c: Buffer) => {
        if (tooBig) return; // Rest verwerfen, Antwort 413 ist schon unterwegs
        size += c.length;
        if (size > limit) {
          tooBig = true;
          chunks.length = 0;
          reject(new ValidationError(`Anfrage zu groß (höchstens ${Math.round(limit / 1024)} kB).`, 413));
          return;
        }
        chunks.push(c);
      });
      req.on("end", () => {
        if (tooBig) return;
        try {
          resolve(JSON.parse(Buffer.concat(chunks).toString("utf8") || "null"));
        } catch {
          reject(new ValidationError("Ungültiges JSON."));
        }
      });
      req.on("error", reject);
    });
  }

  async function handle(req: IncomingMessage, res: ServerResponse, next: () => void) {
    const url = new URL(req.url ?? "/", "http://localhost");
    if (!url.pathname.startsWith("/api/")) return next();

    const h = req.headers;
    const denied = checkRequest({
      method: req.method ?? "GET",
      host: h.host,
      origin: typeof h.origin === "string" ? h.origin : undefined,
      contentType: h["content-type"],
      clientHeader: typeof h["x-ghost-client"] === "string" ? h["x-ghost-client"] : undefined,
      port: req.socket.localPort,
      fetchSite: typeof h["sec-fetch-site"] === "string" ? h["sec-fetch-site"] : undefined,
    }, { publicHosts: isPublic ? opts.publicHosts : undefined });
    if (denied) return send(res, denied.status, { ok: false, error: denied.error });

    // Öffentlich: alles mit Schlüsseldateien ablehnen, bevor ghostctl startet
    if (isPublic && !publicRouteAllowed(req.method ?? "GET", url.pathname))
      return send(res, 403, { ok: false, public: true, error: PUBLIC_DENIED_MESSAGE });
    if (isPublic && req.method === "GET" && url.pathname === "/api/keys")
      return send(res, 200, { ok: true, public: true, keys: [], offline: false, transactions: [] });
    if (url.pathname.startsWith("/api/wallet/") && opts.wallet) return opts.wallet(req, res, next);

    const route = `${req.method} ${url.pathname}`;
    try {
      switch (route) {
        case "GET /api/price":
          return send(res, 200, await priceCache.get("mainnet"));

        case "GET /api/history": {
          const days = parseDays(url.searchParams.get("days"));
          if (days === null) return send(res, 400, { ok: false, error: "days muss 7, 30 oder 365 sein." });
          return send(res, 200, await history(days));
        }

        case "GET /api/status":
        case "GET /api/keys": {
          const n = url.searchParams.get("network") ?? "mainnet";
          if (!isNetwork(n)) return send(res, 400, { ok: false, error: "Unbekanntes Netz." });
          const body = await (url.pathname === "/api/status" ? statusCache : keysCache).get(n);
          return send(res, 200, body);
        }

        case "POST /api/action": {
          const body = (await readBody(req)) as Record<string, unknown> | null;
          if (!body || typeof body !== "object") throw new ValidationError("Leere Anfrage.");
          const built = buildActionArgs(
            { network: body.network, action: body.action, params: body.params, dryRun: body.dryRun, confirmMainnet: body.confirmMainnet },
            exists,
          );
          if (busy) return send(res, 409, { ok: false, error: "Es läuft bereits eine Aktion." });
          busy = true;
          try {
            const r = await run(built.args, ACTION_TIMEOUT_MS);
            if (!built.dryRun) {
              statusCache.clear();
              keysCache.clear();
            }
            // Ohne JSON oder nach Zeitüberschreitung ist beim Senden unklar, ob etwas
            // hinausging – das muss die Seite so sagen, sonst droht Doppelsenden (A10-W-2)
            if (r.timedOut)
              return send(res, 200, {
                ok: false,
                timeout: true,
                unclear: !built.dryRun,
                error: "Zeitüberschreitung nach 300 s. Ob gesendet wurde, ist unklar: erst Status prüfen, dann erneut versuchen.",
              });
            const j = parseJson(r.stdout);
            if (!j) {
              const detail = `ghostctl lieferte kein JSON: ${tail(r.stderr) || "keine Ausgabe"}`;
              if (!built.dryRun) return send(res, 200, { ok: false, unclear: true, error: detail });
              return send(res, 200, isNodeError(detail) ? { ok: false, nodeDown: true, error: NODE_DOWN_MESSAGE, detail } : { ok: false, error: detail });
            }
            if (j.ok === false && typeof j.error === "string" && isNodeError(j.error))
              return send(res, 200, { ...j, nodeDown: true, detail: j.error, error: NODE_DOWN_MESSAGE });
            return send(res, 200, j); // ok:false von ghostctl ist ein Ergebnis, kein HTTP-Fehler
          } finally {
            busy = false;
          }
        }

        case "GET /api/messages": {
          const { network, key, args } = buildMessagesArgs(url.searchParams.get("network") ?? "mainnet", url.searchParams.get("key"), exists);
          return send(res, 200, await messagesGet(args, `${network}|${key}`));
        }

        case "GET /api/abos": {
          const args = buildAboListArgs(url.searchParams.get("network") ?? "mainnet");
          const r = await run(args, readTimeout);
          const j = parseJson(r.stdout);
          if (!j) return send(res, 200, { ok: false, error: `ghostctl abo list: ${tail(r.stderr) || "keine Ausgabe"}` });
          return send(res, 200, j);
        }

        case "GET /api/tresore": {
          const args = buildTresorListArgs(url.searchParams.get("network") ?? "mainnet");
          const r = await run(args, readTimeout);
          const j = parseJson(r.stdout);
          if (!j) return send(res, 200, { ok: false, error: `ghostctl tresor list: ${tail(r.stderr) || "keine Ausgabe"}` });
          return send(res, 200, j);
        }

        case "POST /api/keygen": {
          const body = (await readBody(req)) as Record<string, unknown> | null;
          if (!body || typeof body !== "object") throw new ValidationError("Leere Anfrage.");
          const { args } = buildKeygenArgs(body.network, body.name, exists);
          if (busy) return send(res, 409, { ok: false, error: "Es läuft bereits eine Aktion." });
          busy = true;
          try {
            const r = await run(args, readTimeout);
            keysCache.clear();
            const j = parseJson(r.stdout);
            if (!j) return send(res, 200, { ok: false, error: `ghostctl keygen: ${tail(r.stderr) || "keine Ausgabe"}` });
            return send(res, 200, j); // ok:false von ghostctl ist ein Ergebnis, kein HTTP-Fehler
          } finally {
            busy = false;
          }
        }

        case "POST /api/receive": {
          const body = (await readBody(req)) as Record<string, unknown> | null;
          if (!body || typeof body !== "object") throw new ValidationError("Leere Anfrage.");
          const { args } = buildReceiveArgs(body.network, body.key, body.ghost, exists);
          if (busy) return send(res, 409, { ok: false, error: "Es läuft bereits eine Aktion." });
          busy = true;
          try {
            const r = await run(args, readTimeout);
            statusCache.clear();
            keysCache.clear();
            const j = parseJson(r.stdout);
            if (!j) {
              const detail = `ghostctl receive: ${tail(r.stderr) || "keine Ausgabe"}`;
              return send(res, 200, isNodeError(detail) ? { ok: false, nodeDown: true, error: NODE_DOWN_MESSAGE, detail } : { ok: false, error: detail });
            }
            if (j.ok === false && typeof j.error === "string" && isNodeError(j.error))
              return send(res, 200, { ...j, nodeDown: true, detail: j.error, error: NODE_DOWN_MESSAGE });
            return send(res, 200, j);
          } finally {
            busy = false;
          }
        }

        case "POST /api/wallet-probe": {
          const body = (await readBody(req, PROBE_BODY_LIMIT)) as Record<string, unknown> | null;
          const call = buildWalletProbeCall(body ?? {});
          if (busy) return send(res, 409, { ok: false, error: "Es läuft bereits eine Aktion." });
          busy = true;
          // Plan, Wallet-Antwort, Probe: in ein frisches, nur für uns lesbares Verzeichnis
          const dir = mkdtempSync(path.join(os.tmpdir(), "ghost-wallet-probe-"));
          try {
            const paths: Record<string, string> = {};
            for (const [name, content] of Object.entries(call.files)) {
              const f = path.join(dir, `${name}.json`);
              writeFileSync(f, content as string, { mode: 0o600 });
              paths[`{${name}}`] = f;
            }
            const args = call.args.map((a) => paths[a] ?? a);
            const r = await run(args, call.sends ? ABO_RUN_TIMEOUT_MS : readTimeout);
            if (call.sends) {
              statusCache.clear();
              keysCache.clear();
            }
            if (r.timedOut)
              return send(res, 200, {
                ok: false,
                timeout: true,
                unclear: call.sends,
                error: call.sends ? "Zeitüberschreitung. Ob gesendet wurde, ist unklar – erst im Explorer nachsehen." : "Zeitüberschreitung (Nodes antworten nicht).",
              });
            const j = parseJson(r.stdout);
            if (!j) {
              const detail = `ghostctl wallet: ${tail(r.stderr) || "keine Ausgabe"}`;
              return send(res, 200, call.sends ? { ok: false, unclear: true, error: detail } : { ok: false, error: detail });
            }
            if (j.ok === false && typeof j.error === "string" && isNodeError(j.error))
              return send(res, 200, { ...j, nodeDown: true, detail: j.error, error: NODE_DOWN_MESSAGE });
            return send(res, 200, j);
          } finally {
            rmSync(dir, { recursive: true, force: true });
            busy = false;
          }
        }

        case "GET /api/wallet/name": {
          // .k-Name → Adresse, am eigenen Node nachgeprüft (sendet nichts)
          const wait = nameLimiter.take(clientKey(req.socket.remoteAddress, h["x-forwarded-for"], trusted));
          if (wait !== null) return tooMany(res, wait);
          const network = url.searchParams.get("network") ?? "mainnet";
          if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
          const utxos = async (net: typeof network, addresses: string[]) => {
            if (addresses.length === 0) return [];
            if (addresses.length > 50) throw new Error("zu viele Adressen");
            const r = await runLookup(["--network", net, "--json", "utxos", ...addresses.flatMap((a) => ["--address", a])], readTimeout);
            if (r === BUSY_RESULT) throw new Error(BUSY_TEXT);
            const j = parseJson(r.stdout);
            if (!j || j.ok === false || !Array.isArray(j.utxos)) throw new Error("Node nicht erreichbar");
            return j.utxos as { address: string; covenantId?: string | null; transactionId?: string; index?: number; daaScore?: number }[];
          };
          return send(res, 200, await resolveName(network, url.searchParams.get("name"), utxos));
        }

        case "POST /api/wallet/receive": {
          // eingegangene GHOST für die Adresse der Browser-Wallet (sendet nichts)
          const wait = walletLimiter.take(clientKey(req.socket.remoteAddress, h["x-forwarded-for"], trusted));
          if (wait !== null) {
            res.setHeader("Retry-After", String(wait));
            return send(res, 429, { ok: false, error: `Zu viele Anfragen – bitte in ${wait} s erneut versuchen.` });
          }
          const body = (await readBody(req)) as Record<string, unknown> | null;
          const { args } = buildWalletReceiveArgs(body ?? {});
          if (busy) return send(res, 409, { ok: false, error: "Es läuft bereits eine Aktion – bitte gleich erneut versuchen." });
          busy = true;
          try {
            const r = await runLookup(args, readTimeout);
            if (r === BUSY_RESULT) return send(res, 503, { ok: false, busy: true, error: BUSY_TEXT });
            statusCache.clear();
            const j = parseJson(r.stdout);
            if (!j) {
              const detail = `ghostctl receive: ${tail(r.stderr) || "keine Ausgabe"}`;
              return send(res, 200, isNodeError(detail) ? { ok: false, nodeDown: true, error: NODE_DOWN_MESSAGE, detail } : { ok: false, error: detail });
            }
            if (j.ok === false && typeof j.error === "string" && isNodeError(j.error))
              return send(res, 200, { ...j, nodeDown: true, detail: j.error, error: NODE_DOWN_MESSAGE });
            return send(res, 200, j);
          } finally {
            busy = false;
          }
        }

        case "GET /api/wallet/tresore": {
          // „Meine Tresore“ (Browser-Wallet): liest nur die Tresor-Datei; gleiche
          // Ratenbegrenzung und gleiche ghostctl-Plätze wie build/submit
          const wait = walletLimiter.take(clientKey(req.socket.remoteAddress, h["x-forwarded-for"], trusted));
          if (wait !== null) {
            res.setHeader("Retry-After", String(wait));
            return send(res, 429, { ok: false, error: `Zu viele Anfragen – bitte in ${wait} s erneut versuchen.` });
          }
          const sender = clientKey(req.socket.remoteAddress, h["x-forwarded-for"], trusted);
          const args = buildWalletTresoreArgs(url.searchParams.get("network") ?? undefined, url.searchParams.get("owner"));
          const r = await oneAtATime(sender, () => runWallet(args, readTimeout));
          if (r === null) return stillRunning(res);
          if (r === BUSY_RESULT) return send(res, 503, { ok: false, busy: true, error: BUSY_TEXT });
          if (r.timedOut) return send(res, 200, { ok: false, timeout: true, error: "Zeitüberschreitung." });
          const j = parseJson(r.stdout);
          if (!j) return send(res, 200, { ok: false, error: `ghostctl tresor owned: ${tail(r.stderr) || "keine Ausgabe"}` });
          return send(res, 200, j);
        }

        case "POST /api/wallet/build":
        case "POST /api/wallet/submit": {
          const sender = clientKey(req.socket.remoteAddress, h["x-forwarded-for"], trusted);
          const isBuild = url.pathname === "/api/wallet/build";
          const wait = walletLimiter.take(sender) ?? (isBuild ? buildLimiter.take(sender) : null);
          if (wait !== null) return tooMany(res, wait);
          const body = (await readBody(req, WALLET_BODY_LIMIT)) as Record<string, unknown> | null;
          if (isBuild) {
            // erst vollständig prüfen (Adresse mit Prüfsumme), dann Kontingente – Ungültiges kostet keinen Platz
            const { args } = buildWalletBuildArgs(body ?? {});
            if ((walletActive.get(sender) ?? 0) >= 1) return stillRunning(res);
            const all = buildsLimiter.take("alle");
            if (all !== null) {
              res.setHeader("Retry-After", String(all));
              return send(res, 503, { ok: false, busy: true, error: "Gerade werden sehr viele Pläne gebaut – bitte gleich erneut versuchen. Es wurde nichts gesendet." });
            }
            const r = await oneAtATime(sender, () => runWallet(args, readTimeout));
            if (r === null) return stillRunning(res);
            if (r === BUSY_RESULT) return send(res, 503, { ok: false, busy: true, error: BUSY_TEXT });
            if (r.timedOut) return send(res, 200, { ok: false, timeout: true, error: "Zeitüberschreitung (Nodes antworten nicht)." });
            const j = parseJson(r.stdout);
            if (!j) {
              const detail = `ghostctl wallet build: ${tail(r.stderr) || "keine Ausgabe"}`;
              return send(res, 200, isNodeError(detail) ? { ok: false, nodeDown: true, error: NODE_DOWN_MESSAGE, detail } : { ok: false, error: detail });
            }
            if (j.ok === false && typeof j.error === "string" && isNodeError(j.error))
              return send(res, 200, { ...j, nodeDown: true, detail: j.error, error: NODE_DOWN_MESSAGE });
            return send(res, 200, j);
          }
          const call = buildWalletSubmitCall(body ?? {});
          const key = planKey(call.files);
          const until = rejectedPlans.get(key);
          if (until !== undefined && until > Date.now())
            return send(res, 409, { ok: false, rejected: true, error: "Dieser Plan wurde gerade abgewiesen. Bitte neu prüfen und neu signieren. Es wurde nichts gesendet." });
          if (call.sends) {
            const wait = sendLimiter.take("alle");
            if (wait !== null) {
              res.setHeader("Retry-After", String(wait));
              return send(res, 503, { ok: false, busy: true, error: "Gerade werden sehr viele Transaktionen gesendet – bitte gleich erneut versuchen. Es wurde nichts gesendet." });
            }
          }
          // gleichzeitige Submits begrenzen (A17-4): sofort „ausgelastet“ statt Schlange
          if (submitsActive >= maxSubmits || (call.sends && sendsActive >= maxSends)) {
            res.setHeader("Retry-After", "10");
            return send(res, 503, { ok: false, busy: true, error: "Gerade laufen andere Prüfungen bzw. Sendungen – bitte in einigen Sekunden erneut versuchen. Es wurde nichts gesendet." });
          }
          submitsActive++;
          if (call.sends) sendsActive++;
          // Plan und Wallet-Antwort: frisches, nur für uns lesbares Verzeichnis; Namen wählt der Server
          const dir = mkdtempSync(path.join(os.tmpdir(), "ghost-wallet-"));
          try {
            const paths: Record<string, string> = {};
            for (const [name, content] of Object.entries(call.files)) {
              const f = path.join(dir, `${name}.json`);
              writeFileSync(f, content, { mode: 0o600 });
              paths[`{${name}}`] = f;
            }
            const args = call.args.map((a) => paths[a] ?? a);
            const r = await runWallet(args, call.sends ? walletSendTimeout : readTimeout);
            // Warteschlange voll: ghostctl lief nie, also sicher nichts gesendet (nicht „unklar“)
            if (r === BUSY_RESULT) {
              res.setHeader("Retry-After", "10");
              return send(res, 503, { ok: false, busy: true, error: `${BUSY_TEXT} Es wurde nichts gesendet.` });
            }
            if (call.sends) {
              statusCache.clear();
              keysCache.clear();
            }
            if (r.timedOut)
              return send(res, 200, {
                ok: false,
                timeout: true,
                unclear: call.sends,
                error: call.sends
                  ? "Zeitüberschreitung. Ob gesendet wurde, ist unklar – erst den Status bzw. den Explorer prüfen, nicht erneut senden."
                  : "Zeitüberschreitung (Nodes antworten nicht).",
              });
            const j = parseJson(r.stdout);
            if (!j) {
              const detail = `ghostctl wallet submit: ${tail(r.stderr) || "keine Ausgabe"}`;
              return send(res, 200, call.sends ? { ok: false, unclear: true, error: detail } : { ok: false, error: detail });
            }
            // Fehler NACH dem Senden (Tx ging hinaus bzw. Journal steht noch): unklar, nicht „nicht gesendet“
            if (call.sends && j.ok === false && sentOrUnclear(j.transactions)) return send(res, 200, { ...j, unclear: true });
            // Sperre belegt bzw. Journal gleich geklärt (A19-7): nichts gesendet, Plan nicht sperren, gleich erneut
            if (j.ok === false && isRetryLater(j.error)) return send(res, 200, { ...j, busy: true });
            // endgültig abgewiesen (nicht Node-Ausfall): Plan einige Minuten sperren (G-3)
            if (j.ok === false && !(typeof j.error === "string" && isNodeError(j.error))) {
              rejectedPlans.set(key, Date.now() + REJECT_MS);
              if (rejectedPlans.size > 5_000) rejectedPlans.delete(rejectedPlans.keys().next().value as string);
            }
            if (j.ok === false && typeof j.error === "string" && isNodeError(j.error))
              return send(res, 200, { ...j, nodeDown: true, detail: j.error, error: NODE_DOWN_MESSAGE });
            return send(res, 200, j);
          } finally {
            rmSync(dir, { recursive: true, force: true });
            submitsActive--;
            if (call.sends) sendsActive--;
          }
        }

        default:
          return send(res, 404, { ok: false, error: "Unbekannte API-Route." });
      }
    } catch (e) {
      if (e instanceof ValidationError) return send(res, e.status, { ok: false, error: e.message });
      // Einzelheiten nur ins Protokoll des Servers, nicht an den Browser (A17-8)
      console.error(`[ghost-api] ${route}: ${(e as Error).stack ?? (e as Error).message}`);
      return send(res, 500, { ok: false, error: isPublic ? "Interner Fehler." : `Interner Fehler: ${(e as Error).message}` });
    }
  }

  // ---- Daueraufträge: jede Minute prüfen, nur bei Fälligem ghostctl starten
  let aboRunning = false;
  let aboTimer: ReturnType<typeof setInterval> | null = null;

  /** Eine Runde für alle Netze (Daueraufträge, dann Tresore). Nie zwei Runden gleichzeitig aus diesem Server. */
  async function aboTick(): Promise<void> {
    if (aboRunning || isPublic) return;
    aboRunning = true;
    try {
      for (const n of NETWORKS) {
        let file: unknown;
        try {
          file = JSON.parse(readFileSync(path.join(projectDir, "deployments", `${n}-abos.json`), "utf8"));
        } catch {
          continue; // keine Aufträge in diesem Netz
        }
        if (!aboNeedsRun(file, localDate(new Date()), Math.floor(Date.now() / 1000))) continue;
        const r = await run(buildAboRunArgs(n), ABO_RUN_TIMEOUT_MS);
        const j = parseJson(r.stdout);
        const reports = Array.isArray(j?.reports) ? (j.reports as { id: string; text: string; paid?: boolean }[]) : [];
        for (const x of reports) console.log(`[Daueraufträge ${n}] ${x.id}: ${x.text}`);
        if (!j || j.ok === false) console.warn(`[Daueraufträge ${n}] ${String(j?.error ?? (tail(r.stderr) || "ohne Ausgabe"))}`);
        if (reports.some((x) => x.paid)) {
          statusCache.clear();
          keysCache.clear();
        }
      }
      for (const n of NETWORKS) {
        let file: unknown;
        try {
          file = JSON.parse(readFileSync(path.join(projectDir, "deployments", `${n}-tresore.json`), "utf8"));
        } catch {
          continue; // keine Tresore in diesem Netz
        }
        if (!tresorNeedsRun(file, Date.now())) continue;
        const r = await run(buildTresorPayArgs(n), ABO_RUN_TIMEOUT_MS);
        const j = parseJson(r.stdout);
        const reports = Array.isArray(j?.reports) ? (j.reports as { id: string; text: string; paid?: boolean }[]) : [];
        for (const x of reports) console.log(`[Tresore ${n}] ${x.id}: ${x.text}`);
        if (!j || j.ok === false) console.warn(`[Tresore ${n}] ${String(j?.error ?? (tail(r.stderr) || "ohne Ausgabe"))}`);
        if (reports.some((x) => x.paid)) {
          statusCache.clear();
          keysCache.clear();
        }
      }
    } finally {
      aboRunning = false;
    }
  }

  const middleware = (req: IncomingMessage, res: ServerResponse, next: () => void) => {
    void handle(req, res, next);
  };
  return Object.assign(middleware, {
    /** vom Dev-/Preview-Server aufgerufen (nicht beim Laden der Konfiguration für Tests) */
    startAboTimer() {
      // öffentlich nie: Daueraufträge und Tresore führt dort der GHOST-Agent aus
      if (aboTimer || isPublic) return;
      aboTimer = setInterval(() => void aboTick(), ABO_TICK_MS);
      aboTimer.unref?.();
      setTimeout(() => void aboTick(), 5_000).unref?.();
    },
    aboTick,
  });
}
