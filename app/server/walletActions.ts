// Nutzeraktionen mit Browser-Wallet (docs/wallet-aktionen.md): streng
// geprüfte Anfrage → Argumente für `ghostctl wallet build | submit`. Reine
// Logik ohne Ein-/Ausgabe (Vitest). Die Routen hängt api.ts ein:
//
//   POST /api/wallet/build   {network, action, address, params}
//        → ghostctl wallet build <aktion> --address … [Parameter]   (sendet nichts)
//   POST /api/wallet/submit  {network, plan, signed, send?, confirmMainnet?}
//        → ghostctl wallet submit --plan P --signed S [--send]
//
// Grundsätze (öffentlicher Modus, viele Besucher):
// - Es gibt KEINE Schlüsseldateien in diesen Routen: Adressen und Pläne, nie
//   Pfade. Jede Aktion hat eine feste Liste erlaubter Parameter.
// - Größen begrenzt (Anfrage, Plan, Wallet-Antwort). Plan und Antwort gehen
//   als Dateien in ein frisches Temp-Verzeichnis, dessen Namen der Server
//   wählt; die Anfrage bestimmt keinen Pfad.
// - Gesendet wird nur mit send === true, im Mainnet zusätzlich nur mit
//   confirmMainnet === true (Knopf „Jetzt senden“). ghostctl baut den Plan
//   aus seinem Zustand neu und vertraut dem Browser nichts an.
// - Ratenbegrenzung je Absender (createRateLimiter) zusätzlich zur
//   Begrenzung paralleler ghostctl-Aufrufe in api.ts.
import { isIP } from "node:net";
import { checkAmount, checkMessage, checkVault, isNetwork, ValidationError, type Network } from "./actions.ts";
import { checkProbeAddress } from "./walletProbe.ts";

export const WALLET_ACTIONS = [
  "open-vault",
  "mint",
  "repay",
  "deposit",
  "withdraw",
  "close",
  "redeem",
  "liquidate",
  "sweep",
  "send",
  "transfer",
  "swap",
  "pool-add",
  "pool-remove",
] as const;
export type WalletAction = (typeof WALLET_ACTIONS)[number];

export const PLAN_KIND = "ghost-wallet-action:1";
/** Größte Anfrage der Routen (Plan + Wallet-Antwort mit Redeem-Skripten) */
export const WALLET_BODY_LIMIT = 768 * 1024;
export const MAX_PLAN = 320 * 1024;
export const MAX_SIGNED = 320 * 1024;

/** erlaubte Parameter je Aktion (alles andere wird abgelehnt) */
export const WALLET_PARAMS: Record<WalletAction, string[]> = {
  "open-vault": ["kas"],
  mint: ["vault", "ghost"],
  repay: ["vault", "ghost"],
  deposit: ["vault", "kas"],
  withdraw: ["vault", "keep"],
  close: ["vault"],
  redeem: ["vault", "ghost"],
  liquidate: ["vault", "ghost"],
  sweep: ["vault"],
  send: ["to", "kas", "message", "onchain"],
  transfer: ["to", "ghost", "message", "onchain"],
  swap: ["kas", "ghost", "min"],
  "pool-add": ["kas", "ghost", "minShares"],
  "pool-remove": ["percent", "minKas", "minGhost"],
};

const XONLY_RE = /^[0-9a-f]{64}$/;
// Kaspa-Adresse beliebiger Version (q = Schnorr, p = Skript, …); bech32-Zeichen
const ANY_ADDRESS_RE = /^(kaspa|kaspatest):[qpzry9x8gf2tvdw0s3jn54khce6mua7l]{40,90}$/;

export function isWalletAction(a: unknown): a is WalletAction {
  return typeof a === "string" && (WALLET_ACTIONS as readonly string[]).includes(a);
}

function flag(v: unknown, what: string): boolean {
  if (v === undefined || v === null || v === false) return false;
  if (v === true) return true;
  throw new ValidationError(`${what}: true oder false erwartet.`);
}

function kasAddress(v: unknown, network: Network): string {
  if (typeof v !== "string") throw new ValidationError("Empfänger fehlt.");
  const s = v.trim();
  const m = s.match(ANY_ADDRESS_RE);
  if (!m) throw new ValidationError("Empfänger: Kaspa-Adresse (kaspa:…) erwartet.");
  const want = network === "mainnet" ? "kaspa" : "kaspatest";
  if (m[1] !== want) throw new ValidationError(`Empfänger: Adresse passt nicht zum Netz ${network}.`);
  return s;
}

function ghostTarget(v: unknown, network: Network): string {
  if (typeof v === "string" && XONLY_RE.test(v.trim())) return v.trim();
  try {
    return checkProbeAddress(v, network);
  } catch {
    throw new ValidationError("Empfänger: Schnorr-Adresse (kaspa:q…) oder 64-stelliger x-only-Schlüssel (hex, klein) erwartet.");
  }
}

function messageArgs(params: Record<string, unknown>): string[] {
  const m = checkMessage(params.message);
  const onchain = flag(params.onchain, "Öffentliche Nachricht");
  if (onchain && !m) throw new ValidationError("Öffentliche Nachricht gewählt, aber keine Nachricht angegeben.");
  const out: string[] = [];
  // als --message=<text>: ein Text mit „-“ am Anfang wird nie als Option gelesen
  if (m) out.push(`--message=${m}`);
  if (onchain) out.push("--onchain-message");
  return out;
}

export interface WalletBuildRequest {
  network?: unknown;
  action?: unknown;
  address?: unknown;
  params?: unknown;
}

/** Anfrage für /api/wallet/build → Argumente für ghostctl (sendet nichts) */
export function buildWalletBuildArgs(r: WalletBuildRequest): { network: Network; action: WalletAction; address: string; args: string[] } {
  if (!r || typeof r !== "object" || Array.isArray(r)) throw new ValidationError("Leere Anfrage.");
  for (const k of Object.keys(r)) if (!["network", "action", "address", "params"].includes(k)) throw new ValidationError(`Unbekannter Parameter „${k}“.`);
  const network = r.network ?? "mainnet";
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  if (!isWalletAction(r.action)) throw new ValidationError("Unbekannte Aktion.");
  const action = r.action;
  const address = checkProbeAddress(r.address, network);
  const p = r.params ?? {};
  if (typeof p !== "object" || p === null || Array.isArray(p)) throw new ValidationError("params: Objekt erwartet.");
  const params = p as Record<string, unknown>;
  for (const k of Object.keys(params)) if (!WALLET_PARAMS[action].includes(k)) throw new ValidationError(`Unerwarteter Parameter „${k}“ für ${action}.`);
  const has = (k: string) => params[k] !== undefined && params[k] !== null && params[k] !== "";
  const args = ["--network", network, "--json", "wallet", "build", action, "--address", address];
  const vault = () => args.push("--vault", checkVault(params.vault));
  switch (action) {
    case "open-vault":
      args.push("--kas", checkAmount(params.kas, "KAS-Betrag"));
      break;
    case "mint":
    case "redeem":
      vault();
      args.push("--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "repay":
    case "liquidate":
      vault();
      if (has("ghost")) args.push("--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "deposit":
      vault();
      args.push("--kas", checkAmount(params.kas, "KAS-Betrag"));
      break;
    case "withdraw":
      vault();
      args.push("--keep", checkAmount(params.keep, "Verbleibende Sicherheit"));
      break;
    case "close":
    case "sweep":
      vault();
      break;
    case "send":
      args.push("--to", kasAddress(params.to, network), "--kas", checkAmount(params.kas, "KAS-Betrag"), ...messageArgs(params));
      break;
    case "transfer":
      args.push("--to", ghostTarget(params.to, network), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"), ...messageArgs(params));
      break;
    case "swap": {
      if (has("kas") === has("ghost")) throw new ValidationError("Tauschen: entweder KAS oder GHOST angeben.");
      if (has("kas")) args.push("--kas", checkAmount(params.kas, "KAS-Betrag"));
      else args.push("--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      if (has("min")) args.push("--min", checkAmount(params.min, "Mindestbetrag"));
      break;
    }
    case "pool-add":
      args.push("--kas", checkAmount(params.kas, "KAS-Betrag"), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      if (has("minShares")) {
        const m = String(params.minShares);
        if (!/^\d{1,18}$/.test(m) || BigInt(m) === 0n) throw new ValidationError("Mindestanteile: ganze Zahl > 0 erwartet.");
        args.push("--min-shares", BigInt(m).toString());
      }
      break;
    case "pool-remove": {
      if (has("percent")) {
        const pct = checkAmount(params.percent, "Anteil in Prozent");
        if (Number(pct) > 100) throw new ValidationError("Anteil in Prozent: höchstens 100.");
        args.push("--percent", pct);
      }
      if (has("minKas")) args.push("--min-kas", checkAmount(params.minKas, "Mindest-KAS", { allowZero: true }));
      if (has("minGhost")) args.push("--min-ghost", checkAmount(params.minGhost, "Mindest-GHOST", { allowZero: true }));
      break;
    }
  }
  return { network, action, address, args };
}

export interface WalletSubmitRequest {
  network?: unknown;
  plan?: unknown;
  signed?: unknown;
  send?: unknown;
  confirmMainnet?: unknown;
}

export interface WalletSubmitCall {
  network: Network;
  /** Platzhalter {plan} {signed} ersetzt api.ts durch Pfade im eigenen Temp-Verzeichnis */
  args: string[];
  files: { plan: string; signed: string };
  sends: boolean;
}

/** Anfrage für /api/wallet/submit → Argumente und Dateiinhalte für ghostctl */
export function buildWalletSubmitCall(r: WalletSubmitRequest): WalletSubmitCall {
  if (!r || typeof r !== "object" || Array.isArray(r)) throw new ValidationError("Leere Anfrage.");
  for (const k of Object.keys(r)) if (!["network", "plan", "signed", "send", "confirmMainnet"].includes(k)) throw new ValidationError(`Unbekannter Parameter „${k}“.`);
  const network = r.network ?? "mainnet";
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  const plan = r.plan;
  if (!plan || typeof plan !== "object" || Array.isArray(plan)) throw new ValidationError("Signierplan fehlt.");
  const pl = plan as { kind?: unknown; network?: unknown; action?: unknown };
  if (pl.kind !== PLAN_KIND) throw new ValidationError(`Signierplan: falsche Art (erwartet ${PLAN_KIND}).`);
  if (pl.network !== network) throw new ValidationError("Signierplan gehört zu einem anderen Netz.");
  const planText = JSON.stringify(plan);
  if (planText.length > MAX_PLAN) throw new ValidationError("Signierplan zu groß.");
  if (typeof r.signed !== "string" || !r.signed.trim()) throw new ValidationError("Antwort der Wallet fehlt.");
  if (r.signed.length > MAX_SIGNED) throw new ValidationError("Antwort der Wallet zu groß.");
  const send = flag(r.send, "Senden");
  const confirm = flag(r.confirmMainnet, "Mainnet-Bestätigung");
  if (send && network === "mainnet" && !confirm) throw new ValidationError("Senden im Mainnet nur nach ausdrücklicher Bestätigung (Knopf „Jetzt senden“).");
  if (!send && confirm) throw new ValidationError("Mainnet-Bestätigung ohne Senden ergibt keinen Sinn.");
  // --ja ersetzt die Rückfrage am Terminal; die Bestätigung kam von der Seite
  const args = ["--network", network, "--json", ...(send ? ["--ja"] : []), "wallet", "submit", "--plan", "{plan}", "--signed", "{signed}"];
  if (send) args.push("--send");
  return { network, args, files: { plan: planText, signed: r.signed }, sends: send };
}

// --------------------------------------------------- Ratenbegrenzung ----

export interface RateLimiter {
  /** null = erlaubt, sonst Sekunden bis zum nächsten Versuch */
  take(key: string, now?: number): number | null;
}

/**
 * Einfache Ratenbegrenzung je Schlüssel (Absender): höchstens `perMinute`
 * Aufrufe in einem gleitenden Fenster von 60 s. Höchstens `maxKeys`
 * Absender werden gemerkt (älteste fallen heraus), damit der Speicher
 * begrenzt bleibt.
 */
export function createRateLimiter(perMinute: number, maxKeys = 20_000): RateLimiter {
  const hits = new Map<string, number[]>();
  return {
    take(key, now = Date.now()) {
      const from = now - 60_000;
      const list = (hits.get(key) ?? []).filter((t) => t > from);
      if (list.length >= perMinute) {
        hits.set(key, list);
        return Math.max(1, Math.ceil((list[0] + 60_000 - now) / 1000));
      }
      list.push(now);
      hits.delete(key);
      hits.set(key, list);
      if (hits.size > maxKeys) hits.delete(hits.keys().next().value as string);
      return null;
    },
  };
}

// ------------------------------------------- Absender hinter dem Proxy ----

/** Immer vertrauenswürdig: der eigene Rechner */
export const LOOPBACK_PROXIES: readonly string[] = ["127.0.0.0/8", "::1/128"];

/** IPv4 bzw. IPv6 in einheitlicher Form ("::ffff:1.2.3.4" → "1.2.3.4"), sonst null */
export function normalizeIp(s: string | undefined): string | null {
  if (!s) return null;
  let x = s.trim();
  if (x.startsWith("[") && x.endsWith("]")) x = x.slice(1, -1);
  const zone = x.indexOf("%");
  if (zone >= 0) x = x.slice(0, zone);
  const mapped = /^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/i.exec(x);
  if (mapped) x = mapped[1];
  if (isIP(x) === 4) return x;
  if (isIP(x) === 6) return x.toLowerCase();
  return null;
}

/** IPv6 als 8 Gruppen (Zahlen) */
function v6groups(ip: string): number[] {
  let s = ip;
  // eingebettete IPv4 am Ende (::1.2.3.4)
  const v4 = /(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(s);
  if (v4) {
    const [a, b, c, d] = v4.slice(1).map(Number);
    s = s.slice(0, v4.index) + ((a << 8) | b).toString(16) + ":" + ((c << 8) | d).toString(16);
  }
  const [head, tail] = s.includes("::") ? s.split("::") : [s, null];
  const h = head ? head.split(":") : [];
  const t = tail ? tail.split(":") : [];
  const fill = tail === null ? [] : Array(8 - h.length - t.length).fill("0");
  return [...h, ...fill, ...t].map((g) => parseInt(g, 16));
}

function bits(ip: string): number[] {
  if (isIP(ip) === 4) return ip.split(".").flatMap((o) => Array.from({ length: 8 }, (_, i) => (Number(o) >> (7 - i)) & 1));
  return v6groups(ip).flatMap((g) => Array.from({ length: 16 }, (_, i) => (g >> (15 - i)) & 1));
}

/** Liegt `ip` in einem der Netze (CIDR oder einzelne Adresse)? */
export function ipInNets(ip: string, nets: readonly string[]): boolean {
  const a = normalizeIp(ip);
  if (!a) return false;
  const ab = bits(a);
  return nets.some((n) => {
    const [base, len] = n.split("/");
    const b = normalizeIp(base);
    if (!b || isIP(a) !== isIP(b)) return false;
    const bb = bits(b);
    const l = len === undefined ? bb.length : Number(len);
    if (!Number.isInteger(l) || l < 0 || l > bb.length) return false;
    for (let i = 0; i < l; i++) if (ab[i] !== bb[i]) return false;
    return true;
  });
}

/**
 * Schlüssel der Ratenbegrenzung: IPv4 je Adresse, IPv6 je /64 (ein
 * Anschluss bekommt meist ein ganzes /64 – je Adresse zu zählen hieße
 * beliebig viele Kontingente, Audit 17 A17-3).
 */
export function ipKey(ip: string): string {
  if (isIP(ip) !== 6) return ip;
  return v6groups(ip).slice(0, 4).map((g) => g.toString(16)).join(":") + "::/64";
}

/**
 * Vertrauenswürdige Proxys (Audit 17 A17-3): immer Loopback; dazu
 * GHOST_TRUSTED_PROXY (Adressen oder CIDR, durch Komma getrennt), sonst je
 * private IPv4 aus GHOST_HOST deren /16 – das ist das Docker-Netz, aus dem ein
 * Webserver im Container kommt (GHOST_HOST=172.18.0.1 → 172.18.0.0/16).
 */
export function trustedProxies(envTrusted: string | undefined, ghostHost: string | undefined): string[] {
  const list = (s: string | undefined) => (s ?? "").split(",").map((x) => x.trim()).filter(Boolean);
  const out = [...LOOPBACK_PROXIES];
  const own = list(envTrusted);
  if (own.length > 0) {
    for (const n of own) {
      const [base, len] = n.split("/");
      const b = normalizeIp(base);
      const max = b && isIP(b) === 4 ? 32 : 128;
      if (!b || (len !== undefined && !(Number.isInteger(Number(len)) && Number(len) >= 8 && Number(len) <= max)))
        throw new Error(`GHOST_TRUSTED_PROXY: ${n} ist keine Adresse bzw. kein Netz (CIDR ab /8)`);
      out.push(len === undefined ? b : `${b}/${len}`);
    }
    return out;
  }
  for (const h of list(ghostHost)) {
    const ip = normalizeIp(h);
    if (ip && isIP(ip) === 4 && !ip.startsWith("127.")) out.push(ip.split(".").slice(0, 2).join(".") + ".0.0/16");
  }
  return out;
}

/**
 * Absender für die Ratenbegrenzung. Kommt die Verbindung von einem
 * vertrauenswürdigen Proxy (Loopback, Docker-Netz aus GHOST_HOST bzw.
 * GHOST_TRUSTED_PROXY), zählt X-Forwarded-For – von RECHTS gelesen: den
 * letzten Eintrag setzt der eigene Webserver (Caddy ersetzt den Kopf, nginx
 * hängt an); weiter links stehende Einträge kann der Besucher selbst
 * schreiben. Weitere eigene Proxys rechts werden übersprungen. Sonst zählt
 * die Gegenstelle. IPv6 je /64 (ipKey).
 */
export function clientKey(remote: string | undefined, forwarded: string | string[] | undefined, trusted: readonly string[] = LOOPBACK_PROXIES): string {
  const r = normalizeIp(remote);
  if (!r) return remote ?? "?";
  const f = Array.isArray(forwarded) ? forwarded.join(",") : forwarded;
  if (f && ipInNets(r, trusted)) {
    const list = f.split(",").map((x) => x.trim()).filter(Boolean);
    for (let i = list.length - 1; i >= 0; i--) {
      const ip = normalizeIp(list[i]);
      if (!ip) break; // Unsinn: weiter links ist alles fälschbar
      if (i > 0 && ipInNets(ip, trusted)) continue;
      return ipKey(ip);
    }
  }
  return ipKey(r);
}

/**
 * /api/wallet/receive: eingegangene GHOST mit genau diesem Betrag für die Adresse
 * der Browser-Wallet suchen und in den Zustand übernehmen (sendet nichts, kein
 * Geheimnis nötig). Nur Schnorr-Adresse, nie ein Dateipfad (ghostctl --owner).
 */
export function buildWalletReceiveArgs(r: Record<string, unknown>): { args: string[] } {
  if (!r || typeof r !== "object" || Array.isArray(r)) throw new ValidationError("Leere Anfrage.");
  for (const k of Object.keys(r)) if (!["network", "address", "ghost"].includes(k)) throw new ValidationError(`Unbekannter Parameter „${k}“.`);
  const network = r.network ?? "mainnet";
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  const address = checkProbeAddress(r.address, network);
  return { args: ["--network", network, "--json", "receive", "--owner", address, "--ghost", checkAmount(r.ghost, "GHOST-Betrag")] };
}
