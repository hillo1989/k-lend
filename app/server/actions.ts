// Reine Prüf- und Argumentlogik für die lokale API (ohne Node-Ein-/Ausgabe,
// damit sie mit Vitest testbar ist). Aus einer Anfrage wird hier eine streng
// gebaute Argumentliste für ghostctl. Aufgerufen wird ohne Shell (execFile).
//
// Grundsätze:
// - Jede Aktion hat eine feste Liste erlaubter Parameter, alles andere wird abgelehnt.
// - Schlüsselpfade nur in der Form keys/<name>.json, und die Datei muss existieren.
// - Beträge als Dezimalzahl mit Punkt, > 0, höchstens 8 Nachkommastellen.
// - Immer --json und --ja. --dry-run bei Probeläufen. Echte Mainnet-Sendungen nur mit confirmMainnet === true.
// - Nachrichten (send/transfer/Daueraufträge): höchstens 100 Zeichen, keine
//   Steuer-, Format- oder unsichtbaren Zeichen, als --message=<text> (ein Text mit „-“ am
//   Anfang kann so nie als Option gelesen werden). Ohne --onchain-message
//   verschlüsselt ghostctl die Nachricht an den Empfänger.

export const NETWORKS = ["mainnet", "testnet-10"] as const;
export type Network = (typeof NETWORKS)[number];

export const ACTIONS = [
  "open-vault",
  "mint",
  "repay",
  "deposit",
  "withdraw",
  "close",
  "sweep",
  "liquidate",
  "redeem",
  "transfer",
  "send",
  "oracle-update",
  "pool-open",
  "pool-add",
  "pool-remove",
  "swap",
  "abo-add",
  "abo-pause",
  "abo-resume",
  "abo-remove",
  "tresor-open",
  "tresor-pay",
  "tresor-topup",
  "tresor-cancel",
  "tresor-import",
  "tresor-sync",
] as const;
export type ActionName = (typeof ACTIONS)[number];

export class ValidationError extends Error {
  status: number;
  constructor(message: string, status = 400) {
    super(message);
    this.name = "ValidationError";
    this.status = status;
  }
}

const KEY_RE = /^keys\/[A-Za-z0-9_-]+\.json$/;
const AMOUNT_RE = /^\d{1,12}(\.\d{1,8})?$/;
const XONLY_RE = /^[0-9a-f]{64}$/;
const KEYGEN_NAME_RE = /^[a-z0-9-]{3,40}$/;
const ABO_ID_RE = /^[0-9a-f]{8}$/;
const DATE_RE = /^(\d{4})-(\d{2})-(\d{2})$/;
/** Höchstlänge einer Nachricht in Zeichen (wie ghostctl, abo::MAX_MESSAGE_CHARS) */
export const MAX_MESSAGE_CHARS = 100;
// Verbotene Zeichen einer Nachricht – dieselbe Tabelle wie abo::check_message
// in ghostctl und messageProblem auf der Seite (app/src/lib/abo.ts):
// Steuer- und Formatzeichen, Unicode-„standardmäßig unsichtbare“ Zeichen
// (Hangul-Füller, Variation Selectors, Tag-Zeichen), Zeilen-/Absatztrenner,
// Private Use, Nichtzeichen und einzelne Surrogate. VS15/VS16 nur direkt
// hinter einem Bildzeichen (❤️).
const BAD_MESSAGE_RANGES: readonly (readonly [number, number])[] = [
  [0x0000, 0x001f],
  [0x007f, 0x009f],
  [0x00ad, 0x00ad],
  [0x034f, 0x034f],
  [0x0600, 0x0605],
  [0x061c, 0x061c],
  [0x06dd, 0x06dd],
  [0x070f, 0x070f],
  [0x0890, 0x0891],
  [0x08e2, 0x08e2],
  [0x115f, 0x1160],
  [0x17b4, 0x17b5],
  [0x180b, 0x180f],
  [0x200b, 0x200f],
  [0x2028, 0x202e],
  [0x2060, 0x206f],
  [0x3164, 0x3164],
  [0xd800, 0xdfff],
  [0xe000, 0xf8ff],
  [0xfdd0, 0xfdef],
  [0xfe00, 0xfe0f],
  [0xfeff, 0xfeff],
  [0xffa0, 0xffa0],
  [0xfff0, 0xfffb],
  [0x110bd, 0x110bd],
  [0x110cd, 0x110cd],
  [0x13430, 0x1343f],
  [0x1bca0, 0x1bca3],
  [0x1d173, 0x1d17a],
  [0xe0000, 0xe0fff],
  [0xf0000, 0x10ffff],
];
const isPictograph = (u: number) =>
  u === 0xa9 || u === 0xae || u === 0x203c || u === 0x2049 || (u >= 0x2100 && u <= 0x2bff) || u === 0x3030 || u === 0x303d || u === 0x3297 || u === 0x3299 || (u >= 0x1f000 && u <= 0x1faff);

/** Enthält die Nachricht ein verbotenes Zeichen? */
export function hasBadMessageChar(t: string): boolean {
  let prev: number | null = null;
  for (const ch of t) {
    const u = ch.codePointAt(0)!;
    const vsOk = (u === 0xfe0e || u === 0xfe0f) && prev !== null && isPictograph(prev);
    if (!vsOk && ((u & 0xfffe) === 0xfffe || BAD_MESSAGE_RANGES.some(([a, b]) => u >= a && u <= b))) return true;
    prev = u;
  }
  return false;
}
/** Daueraufträge, die nichts senden: ohne Schlüssel und ohne Mainnet-Bestätigung */
const ABO_BY_ID: readonly ActionName[] = ["abo-pause", "abo-resume", "abo-remove"];
/** Tresor-Aktionen, die nichts senden (am Node prüfen bzw. abgleichen) */
const TRESOR_READ: readonly ActionName[] = ["tresor-import", "tresor-sync"];
/** Tresor-ID: erste 8 Hex-Zeichen der Covenant-ID */
const TRESOR_ID_RE = /^[0-9a-f]{8}$/;
/** Tresor-Code: Präfix + base64url (ghostctl tresor code) */
// Version 2: Vertrag mit gebundener Nachricht; Version 1 (alter Vertrag) wird abgelehnt
const TRESOR_CODE_RE = /^ghost-tresor:2:[A-Za-z0-9_-]{16,4000}$/;
/** Kleinster Betrag je Tresor-Zahlung in sompi (wie tresor::MIN_AMOUNT) */
const TRESOR_MIN_AMOUNT = 100_000_000n;

export function isNetwork(n: unknown): n is Network {
  return typeof n === "string" && (NETWORKS as readonly string[]).includes(n);
}

export function isAction(a: unknown): a is ActionName {
  return typeof a === "string" && (ACTIONS as readonly string[]).includes(a);
}

/** Pfad zu einer Schlüsseldatei prüfen (Form + Existenz). */
export function checkKeyPath(p: unknown, exists: (rel: string) => boolean, what = "Schlüsseldatei"): string {
  if (typeof p !== "string" || !KEY_RE.test(p)) throw new ValidationError(`${what}: nur keys/<name>.json erlaubt.`);
  if (!exists(p)) throw new ValidationError(`${what} ${p} gibt es nicht.`);
  return p;
}

/** Betrag als Dezimaltext mit Punkt normalisieren. Zahl oder Text erlaubt. */
export function checkAmount(v: unknown, what: string, { allowZero = false } = {}): string {
  let s: string;
  if (typeof v === "number") {
    if (!Number.isFinite(v)) throw new ValidationError(`${what}: keine gültige Zahl.`);
    s = String(v);
  } else if (typeof v === "string") {
    s = v.trim();
  } else {
    throw new ValidationError(`${what} fehlt.`);
  }
  if (!AMOUNT_RE.test(s)) throw new ValidationError(`${what}: Zahl mit Punkt und höchstens 8 Nachkommastellen erwartet.`);
  const [w, f = ""] = s.split(".");
  const units = BigInt(w) * 100_000_000n + BigInt((f + "00000000").slice(0, 8));
  if (units === 0n && !allowZero) throw new ValidationError(`${what} muss größer als 0 sein.`);
  // führende Nullen und überflüssige Nachkommanullen entfernen
  const whole = BigInt(w).toString();
  const frac = f.replace(/0+$/, "");
  return frac ? `${whole}.${frac}` : whole;
}

export function checkVault(v: unknown): string {
  const n = typeof v === "string" && /^\d{1,6}$/.test(v) ? Number(v) : v;
  if (typeof n !== "number" || !Number.isInteger(n) || n < 0 || n > 999_999)
    throw new ValidationError("Vault-Nummer: ganze Zahl ≥ 0 erwartet.");
  return String(n);
}

function checkSendTarget(v: unknown, network: Network, exists: (rel: string) => boolean): string {
  if (typeof v !== "string") throw new ValidationError("Empfänger fehlt.");
  const s = v.trim();
  if (KEY_RE.test(s)) return checkKeyPath(s, exists, "Empfänger-Datei");
  const m = s.match(/^(kaspa|kaspatest):[a-z0-9]{40,90}$/);
  if (!m) throw new ValidationError("Empfänger: kaspa:/kaspatest:-Adresse oder keys/<name>.json erwartet.");
  const want = network === "mainnet" ? "kaspa" : "kaspatest";
  if (m[1] !== want) throw new ValidationError(`Adresse passt nicht zum Netz: für ${network} beginnt sie mit „${want}:“.`);
  return s;
}

function checkTransferTarget(v: unknown, network: Network, exists: (rel: string) => boolean): string {
  if (typeof v !== "string") throw new ValidationError("Empfänger fehlt.");
  const s = v.trim();
  if (KEY_RE.test(s)) return checkKeyPath(s, exists, "Empfänger-Datei");
  if (XONLY_RE.test(s)) return s;
  // Kaspa-Adresse: ghostctl liest daraus den x-only-Schlüssel (nur Schnorr-Adressen)
  if (/^(kaspa|kaspatest):/.test(s)) return checkSendTarget(s, network, exists);
  throw new ValidationError("Empfänger: Kaspa-Adresse, 64-stelliger x-only-Schlüssel (hex, klein) oder keys/<name>.json erwartet.");
}

/** Nachricht prüfen; leer = keine Nachricht (""). */
export function checkMessage(v: unknown): string {
  if (v === undefined || v === null) return "";
  if (typeof v !== "string") throw new ValidationError("Nachricht: Text erwartet.");
  const m = v.trim();
  if ([...m].length > MAX_MESSAGE_CHARS) throw new ValidationError(`Nachricht: höchstens ${MAX_MESSAGE_CHARS} Zeichen.`);
  if (hasBadMessageChar(m)) throw new ValidationError("Nachricht: nur eine Zeile normaler Text, ohne Steuerzeichen und unsichtbare Zeichen.");
  return m;
}

/** Datum JJJJ-MM-TT, das es im Kalender gibt */
export function checkDate(v: unknown, what: string): string {
  const m = typeof v === "string" ? v.match(DATE_RE) : null;
  if (!m) throw new ValidationError(`${what}: Datum als JJJJ-MM-TT erwartet.`);
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const t = new Date(Date.UTC(y, mo - 1, d));
  if (y < 2000 || y > 2200 || t.getUTCMonth() !== mo - 1 || t.getUTCDate() !== d) throw new ValidationError(`${what}: kein gültiges Datum.`);
  return v as string;
}

/** daily | weekly | monthly | Anzahl Tage 1–3650 */
export function checkInterval(v: unknown): string {
  if (v === "daily" || v === "weekly" || v === "monthly") return v;
  const s = typeof v === "number" ? String(v) : v;
  if (typeof s === "string" && /^\d{1,4}$/.test(s) && Number(s) >= 1 && Number(s) <= 3650) return String(Number(s));
  throw new ValidationError("Intervall: täglich, wöchentlich, monatlich oder 1–3650 Tage.");
}

function checkFlag(v: unknown, what: string): boolean {
  if (v === undefined || v === null || v === false) return false;
  if (v === true) return true;
  throw new ValidationError(`${what}: true oder false erwartet.`);
}

/** --message und --onchain-message für send/transfer/abo-add */
function messageArgs(params: Record<string, unknown>): string[] {
  const m = checkMessage(params.message);
  const onchain = checkFlag(params.onchain, "Öffentliche Nachricht");
  if (onchain && !m) throw new ValidationError("Öffentliche Nachricht gewählt, aber keine Nachricht angegeben.");
  const out: string[] = [];
  if (m) out.push(`--message=${m}`);
  if (onchain) out.push("--onchain-message");
  return out;
}

type Params = Record<string, unknown>;

/** Welche Parameter jede Aktion kennt (alles andere wird abgelehnt). */
export const ALLOWED: Record<ActionName, string[]> = {
  "open-vault": ["key", "kas"],
  mint: ["key", "vault", "ghost"],
  repay: ["key", "vault", "ghost"],
  deposit: ["key", "vault", "kas"],
  withdraw: ["key", "vault", "keep"],
  close: ["key", "vault"],
  sweep: ["key", "vault"],
  liquidate: ["key", "vault", "ghost"],
  redeem: ["key", "vault", "ghost"],
  transfer: ["key", "to", "ghost", "message", "onchain"],
  send: ["key", "to", "kas", "message", "onchain"],
  "oracle-update": ["key", "committee", "usd", "rate"],
  "pool-open": ["key", "kas", "ghost"],
  "pool-add": ["key", "kas", "ghost", "minShares"],
  "pool-remove": ["key", "percent", "minKas", "minGhost"],
  swap: ["key", "kas", "ghost", "min"],
  "abo-add": ["key", "asset", "to", "amount", "interval", "start", "end", "count", "message", "onchain"],
  "abo-pause": ["id"],
  "abo-resume": ["id"],
  "abo-remove": ["id"],
  "tresor-open": ["key", "to", "amount", "interval", "start", "count", "fund", "message", "onchain"],
  "tresor-pay": ["id", "key"],
  "tresor-topup": ["id", "key", "kas"],
  "tresor-cancel": ["id", "key"],
  "tresor-import": ["code"],
  "tresor-sync": [],
};

/** Betrag als Dezimaltext → sompi (nach checkAmount) */
function units(s: string): bigint {
  const [w, f = ""] = s.split(".");
  return BigInt(w) * 100_000_000n + BigInt((f + "00000000").slice(0, 8));
}

function checkTresorId(v: unknown): string {
  if (typeof v !== "string" || !TRESOR_ID_RE.test(v)) throw new ValidationError("Tresor: ID aus 8 Hex-Zeichen erwartet.");
  return v;
}

/** Argumente für die Tresor-Aktionen (ghostctl tresor …) */
function tresorArgs(action: ActionName, params: Params, network: Network, dryRun: boolean, exists: (rel: string) => boolean): string[] {
  const has = (k: string) => params[k] !== undefined && params[k] !== null && params[k] !== "";
  switch (action) {
    case "tresor-open": {
      const key = checkKeyPath(params.key, exists);
      const to = checkTransferTarget(params.to, network, exists);
      if (to === key) throw new ValidationError("Empfänger ist der Absender selbst.");
      const amount = checkAmount(params.amount, "Betrag");
      if (units(amount) < TRESOR_MIN_AMOUNT) throw new ValidationError("Betrag: mindestens 1 KAS je Zahlung.");
      const args = ["tresor", "open", "--key", key, "--to", to, "--amount", amount, "--interval", checkInterval(params.interval), "--start", checkDate(params.start, "Start")];
      if (has("count")) {
        const c = String(params.count);
        if (!/^\d{1,4}$/.test(c) || Number(c) < 1) throw new ValidationError("Anzahl: ganze Zahl von 1 bis 9999.");
        args.push("--count", String(Number(c)));
      }
      if (has("fund")) args.push("--fund", checkAmount(params.fund, "Startguthaben"));
      else if (!has("count")) throw new ValidationError("Unbegrenzter Tresor: Startguthaben angeben.");
      args.push(...messageArgs(params));
      return args;
    }
    case "tresor-pay": {
      const args = ["tresor", "pay", checkTresorId(params.id)];
      if (has("key")) args.push("--key", checkKeyPath(params.key, exists));
      return args;
    }
    case "tresor-topup":
      return ["tresor", "topup", checkTresorId(params.id), "--key", checkKeyPath(params.key, exists), "--kas", checkAmount(params.kas, "KAS-Betrag")];
    case "tresor-cancel":
      return ["tresor", "cancel", checkTresorId(params.id), "--key", checkKeyPath(params.key, exists)];
    case "tresor-import": {
      const c = typeof params.code === "string" ? params.code.trim() : "";
      if (c.startsWith("ghost-tresor:1:")) throw new ValidationError("Alter Tresor-Code (Vertrag ohne gebundene Nachricht) – wird nicht mehr unterstützt; bitte beim Absender einen neuen Tresor anfordern.");
      if (!TRESOR_CODE_RE.test(c)) throw new ValidationError("Tresor-Code: beginnt mit „ghost-tresor:2:“, danach nur Buchstaben, Ziffern, „-“ und „_“.");
      if (dryRun) throw new ValidationError("Übernehmen sendet nichts – ohne Probelauf aufrufen.");
      return ["tresor", "import", c];
    }
    case "tresor-sync":
      return ["tresor", "sync"];
    default:
      throw new ValidationError("Unbekannte Tresor-Aktion.");
  }
}

export interface ActionRequest {
  network: unknown;
  action: unknown;
  params: unknown;
  dryRun: unknown;
  confirmMainnet?: unknown;
}

export interface BuiltAction {
  network: Network;
  action: ActionName;
  dryRun: boolean;
  args: string[];
}

/** Anfrage prüfen und die Argumentliste für ghostctl bauen. Wirft ValidationError. */
export function buildActionArgs(req: ActionRequest, exists: (rel: string) => boolean): BuiltAction {
  if (!isNetwork(req.network)) throw new ValidationError("Unbekanntes Netz.");
  if (!isAction(req.action)) throw new ValidationError("Unbekannte Aktion.");
  if (typeof req.dryRun !== "boolean") throw new ValidationError("dryRun muss true oder false sein.");
  const network = req.network;
  const action = req.action;
  const dryRun = req.dryRun;
  const byId = ABO_BY_ID.includes(action);
  if (network === "mainnet" && !dryRun && !byId && !TRESOR_READ.includes(action) && req.confirmMainnet !== true)
    throw new ValidationError("Mainnet-Sendung ohne ausdrückliche Bestätigung (confirmMainnet) abgelehnt.");

  const p = req.params;
  if (typeof p !== "object" || p === null || Array.isArray(p)) throw new ValidationError("params fehlt.");
  const params = p as Params;
  for (const k of Object.keys(params)) {
    if (!ALLOWED[action].includes(k)) throw new ValidationError(`Unerwarteter Parameter „${k}“ für ${action}.`);
  }
  const has = (k: string) => params[k] !== undefined && params[k] !== null && params[k] !== "";

  const args = ["--network", network, "--json", "--ja"];
  if (dryRun) args.push("--dry-run");
  if (action.startsWith("tresor-")) {
    args.push(...tresorArgs(action, params, network, dryRun, exists));
    return { network, action, dryRun, args };
  }
  // Daueraufträge: „abo-add“ → ghostctl abo add
  if (action.startsWith("abo-")) args.push("abo", action.slice(4));
  else args.push(action);
  if (byId) {
    if (typeof params.id !== "string" || !ABO_ID_RE.test(params.id)) throw new ValidationError("Dauerauftrag: ID aus 8 Hex-Zeichen erwartet.");
    args.push(params.id);
    return { network, action, dryRun, args };
  }
  args.push("--key", checkKeyPath(params.key, exists));

  switch (action) {
    case "open-vault":
      args.push("--kas", checkAmount(params.kas, "KAS-Betrag"));
      break;
    case "mint":
      args.push("--vault", checkVault(params.vault), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "repay":
      args.push("--vault", checkVault(params.vault));
      if (has("ghost")) args.push("--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "deposit":
      args.push("--vault", checkVault(params.vault), "--kas", checkAmount(params.kas, "KAS-Betrag"));
      break;
    case "withdraw":
      args.push("--vault", checkVault(params.vault), "--keep", checkAmount(params.keep, "Verbleibende Sicherheit"));
      break;
    case "close":
    case "sweep":
      // sweep: jeder löst einen Vault ohne Schuld auf, dessen Zins die Sicherheit aufzehrt
      args.push("--vault", checkVault(params.vault));
      break;
    case "liquidate":
      // Version 2: optional Teil-Liquidation (--ghost), sonst ganze Schuld
      args.push("--vault", checkVault(params.vault));
      if (has("ghost")) args.push("--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "redeem":
      // Version 3: Rücknahme zu 1 USD (−1 %, mindestens 1 GHOST oder die ganze Schuld), Betrag immer nötig
      args.push("--vault", checkVault(params.vault), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "transfer":
      args.push("--to", checkTransferTarget(params.to, network, exists), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"), ...messageArgs(params));
      break;
    case "send":
      args.push("--to", checkSendTarget(params.to, network, exists), "--kas", checkAmount(params.kas, "KAS-Betrag"), ...messageArgs(params));
      break;
    case "abo-add": {
      const asset = params.asset;
      if (asset !== "KAS" && asset !== "GHOST") throw new ValidationError("Dauerauftrag: KAS oder GHOST wählen.");
      const to = asset === "KAS" ? checkSendTarget(params.to, network, exists) : checkTransferTarget(params.to, network, exists);
      if (to === params.key) throw new ValidationError("Empfänger ist der Absender selbst.");
      args.push("--asset", asset, "--to", to, "--amount", checkAmount(params.amount, "Betrag"), "--interval", checkInterval(params.interval));
      const start = checkDate(params.start, "Start");
      args.push("--start", start);
      if (has("end") && has("count")) throw new ValidationError("Entweder ein Enddatum oder eine Anzahl angeben, nicht beides.");
      if (has("end")) {
        const end = checkDate(params.end, "Ende");
        if (end < start) throw new ValidationError("Ende liegt vor dem Start.");
        args.push("--end", end);
      }
      if (has("count")) {
        const c = String(params.count);
        if (!/^\d{1,4}$/.test(c) || Number(c) < 1) throw new ValidationError("Anzahl: ganze Zahl von 1 bis 9999.");
        args.push("--count", String(Number(c)));
      }
      args.push(...messageArgs(params));
      break;
    }
    case "pool-open":
      args.push("--kas", checkAmount(params.kas, "KAS-Betrag"), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      break;
    case "pool-add":
      // Mindestanteile aus dem Probelauf: verschiebt sich der Pool bis zum
      // Senden, bricht ghostctl ab statt den Überschuss zu verschenken (A10-W-6)
      args.push("--kas", checkAmount(params.kas, "KAS-Betrag"), "--ghost", checkAmount(params.ghost, "GHOST-Betrag"));
      if (has("minShares")) {
        const m = String(params.minShares);
        if (!/^\d{1,18}$/.test(m) || BigInt(m) === 0n) throw new ValidationError("Mindestanteile: ganze Zahl > 0 erwartet.");
        args.push("--min-shares", BigInt(m).toString());
      } else if (!dryRun) throw new ValidationError("Einlegen: Mindestanteile fehlen.");
      break;
    case "pool-remove": {
      const pct = checkAmount(params.percent, "Anteil in Prozent");
      if (Number(pct) > 100) throw new ValidationError("Anteil in Prozent: höchstens 100.");
      args.push("--percent", pct);
      if (has("minKas") || has("minGhost")) {
        args.push("--min-kas", checkAmount(params.minKas ?? "0", "Mindest-KAS", { allowZero: true }));
        args.push("--min-ghost", checkAmount(params.minGhost ?? "0", "Mindest-GHOST", { allowZero: true }));
      } else if (!dryRun) throw new ValidationError("Abziehen: Mindestbeträge fehlen.");
      break;
    }
    case "swap": {
      // genau eine Seite; der Mindestbetrag schützt, falls sich der Pool
      // zwischen Probelauf und Senden durch andere Tauschvorgänge verschiebt
      if (has("kas") === has("ghost")) throw new ValidationError("Tauschen: entweder KAS oder GHOST angeben.");
      if (!has("min")) throw new ValidationError("Tauschen: Mindestbetrag fehlt.");
      const min = checkAmount(params.min, "Mindestbetrag");
      if (has("kas")) args.push("--kas", checkAmount(params.kas, "KAS-Betrag"), "--min-ghost", min);
      else args.push("--ghost", checkAmount(params.ghost, "GHOST-Betrag"), "--min-kas", min);
      break;
    }
    case "oracle-update":
      args.push("--committee", checkKeyPath(params.committee, exists, "Komitee-Datei"));
      if (has("usd")) args.push("--usd", checkAmount(params.usd, "Preis"));
      if (has("rate")) {
        const r = checkAmount(params.rate, "Zinssatz", { allowZero: true });
        if (Number(r) > 100) throw new ValidationError("Zinssatz: höchstens 100 % p. a.");
        args.push("--rate", r);
      }
      break;
  }
  return { network, action, dryRun, args };
}

/** Name für einen neuen Schlüssel prüfen → Argumente für `ghostctl keygen`. */
export function buildKeygenArgs(network: unknown, name: unknown, exists: (rel: string) => boolean): { file: string; args: string[] } {
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  if (typeof name !== "string" || !KEYGEN_NAME_RE.test(name))
    throw new ValidationError("Name: 3–40 Zeichen, nur a–z, 0–9 und Bindestrich.");
  const file = `keys/${name}.json`;
  if (exists(file)) throw new ValidationError(`${file} gibt es schon – bitte anderen Namen wählen.`, 409);
  return { file, args: ["--network", network, "--json", "keygen", file] };
}

/** Empfang prüfen: sucht GHOST-UTXOs mit genau diesem Betrag für den Schlüssel (sendet nichts). */
export function buildReceiveArgs(network: unknown, key: unknown, ghost: unknown, exists: (rel: string) => boolean): { args: string[] } {
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  return { args: ["--network", network, "--json", "receive", "--key", checkKeyPath(key, exists, "Schlüsseldatei"), "--ghost", checkAmount(ghost, "GHOST-Betrag")] };
}

/**
 * Eingegangene Nachrichten eines eigenen Schlüssels (sendet nichts, liest nur
 * über die REST-API). Nur keys/<name>.json, die Datei muss existieren.
 */
export function buildMessagesArgs(network: unknown, key: unknown, exists: (rel: string) => boolean): { network: Network; key: string; args: string[] } {
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  const k = checkKeyPath(key, exists, "Schlüsseldatei");
  return { network, key: k, args: ["--network", network, "--json", "messages", "--key", k] };
}

// ------------------------------------------------------- Daueraufträge ----

/** Liste der Daueraufträge (ohne Netz) */
export function buildAboListArgs(network: unknown): string[] {
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  return ["--network", network, "--json", "abo", "list"];
}

/** Fällige ausführen, ohne Rückfrage (die Seite läuft unbeaufsichtigt) */
export function buildAboRunArgs(network: Network): string[] {
  return ["--network", network, "--json", "--ja", "abo", "run"];
}

// ------------------------------------------------------------- Tresore ----

/** Liste der Tresore (ohne Netz) */
export function buildTresorListArgs(network: unknown): string[] {
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  return ["--network", network, "--json", "tresor", "list"];
}

/** Fällige Tresor-Zahlungen auslösen; ohne Schlüssel, die Gebühr trägt der Tresor */
export function buildTresorPayArgs(network: Network): string[] {
  return ["--network", network, "--json", "--ja", "tresor", "pay"];
}

/** Past Median Time hinkt der Uhr hinterher (wie tresor::PMT_LAG_MS) */
export const TRESOR_PMT_LAG_MS = 180_000;
const TRESOR_MIN_KEEP = 100_000_000;
/** Nicht auffindbare Tresore: so lange sieht die Automatik erneut nach (tresor::MISSING_RECHECK_FOR_MS) */
export const TRESOR_MISSING_RECHECK_FOR_MS = 7 * 86_400_000;

/**
 * Muss `tresor pay` laufen? Wie TresorRec::looks_due(now, false) in ghostctl:
 * aktiver Tresor, Termin (mit Nachlauf der Past Median Time) erreicht, Zahlung
 * samt Höchstgebühr aus dem Tresor möglich, keine Wartezeit nach einem Fehlschlag.
 * Nicht auffindbare Tresore sieht ghostctl eine Woche lang stündlich erneut
 * nach (retryAfter), damit ein kurzer Aussetzer des Nodes nicht alles stilllegt.
 */
export function tresorNeedsRun(file: unknown, nowMs: number): boolean {
  if (!file || typeof file !== "object") return false;
  const list = (file as { tresore?: unknown }).tresore;
  if (!Array.isArray(list)) return false;
  return list.some((r: unknown) => {
    if (!r || typeof r !== "object") return false;
    const t = r as { ended?: unknown; missing?: unknown; missingMs?: unknown; retryAfter?: unknown; params?: { amount?: unknown; maxFee?: unknown }; utxo?: { value?: unknown; state?: { nextDue?: unknown; left?: unknown } } };
    if (t.ended) return false;
    const waiting = typeof t.retryAfter === "number" && t.retryAfter > nowMs;
    if (t.missing) return !waiting && (typeof t.missingMs !== "number" || nowMs - t.missingMs <= TRESOR_MISSING_RECHECK_FOR_MS);
    const due = t.utxo?.state?.nextDue;
    const left = t.utxo?.state?.left;
    const value = t.utxo?.value;
    const amount = t.params?.amount;
    const maxFee = t.params?.maxFee;
    if (typeof due !== "number" || typeof left !== "number" || typeof value !== "number" || typeof amount !== "number" || typeof maxFee !== "number") return false;
    if (waiting) return false;
    return left !== 0 && due + TRESOR_PMT_LAG_MS <= nowMs && value - amount - maxFee >= TRESOR_MIN_KEEP;
  });
}

/** Lokales Datum JJJJ-MM-TT */
export function localDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/**
 * Muss `abo run` laufen? Wie abo::needs_run in ghostctl: offene Zahlung,
 * fälliger Termin oder fälliger Wiederholungsversuch eines aktiven Auftrags.
 * Ungültige Dateien: nein (ghostctl meldet den Fehler beim nächsten Aufruf).
 */
export function aboNeedsRun(file: unknown, today: string, nowUnix: number): boolean {
  if (!file || typeof file !== "object") return false;
  const f = file as { abos?: unknown; archive?: unknown };
  const all = [...(Array.isArray(f.abos) ? f.abos : []), ...(Array.isArray(f.archive) ? f.archive : [])] as Record<string, unknown>[];
  return all.some((a) => {
    if (!a || typeof a !== "object") return false;
    if (a.inflight) return true;
    if (a.ended || a.paused) return false;
    const due = typeof a.nextDue === "string" && a.nextDue <= today;
    const retry = a.retry as { after?: unknown } | null | undefined;
    return due || (!!retry && typeof retry.after === "number" && retry.after <= nowUnix);
  });
}

// --------------------------------------------------------- Node-Fehler ----

/** Freundliche Meldung, wenn ghostctl keinen öffentlichen Kaspa-Node erreicht. */
export const NODE_DOWN_MESSAGE = "Öffentliche Kaspa-Nodes nicht erreichbar – später erneut versuchen.";

/** Erkennt Verbindungsfehler von ghostctl (wRPC/WebSocket/Resolver, HTTP 502 usw.). */
export function isNodeError(msg: string): boolean {
  // HTTP-Codes nur im Kontext, sonst wird aus „502 GHOST vorhanden“ ein Node-Ausfall (A10-W-9)
  return /Verbindung fehlgeschlagen|WebSocket|wRPC|Connection (timeout|refused|reset)|resolver|(?:HTTP|status(?: code)?)[ /:]*50[234]\b|Bad Gateway|Gateway Time-?out|timed out|Node[s]? nicht erreichbar|keinen? (?:öffentlichen )?(?:Kaspa-)?Node/i.test(msg);
}

// ------------------------------------------------------------ Sicherheit ----

export interface RequestMeta {
  method: string;
  host: string | undefined;
  origin: string | undefined;
  contentType: string | undefined;
  clientHeader: string | undefined;
  /** tatsächlicher Port des Servers (socket.localPort) */
  port: number | undefined;
  /** Sec-Fetch-Site des Browsers (fehlt bei curl und alten Browsern) */
  fetchSite?: string | undefined;
}

/**
 * Prüft Herkunft und Kopfzeilen. Rückgabe null = in Ordnung, sonst Status + Meldung.
 * - Host muss localhost:<port>, 127.0.0.1:<port> oder [::1]:<port> sein (gegen DNS-Rebinding).
 * - Ein vorhandener Origin muss genau die eigene Herkunft sein.
 * - POST braucht X-Ghost-Client: 1 und Content-Type application/json.
 *   Beides erzwingt bei fremden Seiten einen CORS-Preflight, den wir nie erlauben.
 */
export function checkRequest(m: RequestMeta, opts: { publicHosts?: readonly string[] } = {}): { status: number; error: string } | null {
  if (!m.port) return { status: 403, error: "Unbekannter Server-Port." };
  const allowedHosts = [`localhost:${m.port}`, `127.0.0.1:${m.port}`, `[::1]:${m.port}`];
  const host = m.host?.toLowerCase();
  // Öffentlicher Modus (server/prod.ts hinter Caddy): zusätzlich die eigene
  // Domain; der Browser kommt dann über HTTPS (Origin https://<domain>)
  const publicHost = !!host && (opts.publicHosts ?? []).some((h) => h.toLowerCase() === host);
  if (!host || (!allowedHosts.includes(host) && !publicHost)) return { status: 403, error: "Zugriff nur über localhost." };
  if (m.origin !== undefined && m.origin !== (publicHost ? `https://${host}` : `http://${host}`))
    return { status: 403, error: "Fremde Herkunft abgelehnt." };
  // Auch GET: fremde Seiten dürfen ghostctl nicht per <img>/no-cors anstoßen (A10-W-8)
  if (m.fetchSite !== undefined && m.fetchSite !== "same-origin" && m.fetchSite !== "none")
    return { status: 403, error: "Fremde Herkunft abgelehnt." };
  if (m.method === "POST") {
    if (m.clientHeader !== "1") return { status: 403, error: "Kopfzeile X-Ghost-Client fehlt." };
    if (!m.contentType || !m.contentType.toLowerCase().startsWith("application/json"))
      return { status: 415, error: "Content-Type application/json erwartet." };
  }
  return null;
}

// ------------------------------------------------------ Öffentlicher Modus ----

/**
 * Was der öffentliche Modus (GHOST_PUBLIC=1, immer im Produktionsserver
 * server/prod.ts) beantwortet. Alles andere nutzt Schlüsseldateien, sendet
 * oder zeigt private Daten des Betreibers (Daueraufträge, Tresore) und wird
 * mit 403 abgelehnt, bevor ghostctl startet.
 * - status, price, history: nur lesend, ohne Schlüssel
 * - keys: antwortet ohne ghostctl mit leerer Liste (ghostctl keys würde die
 *   Schlüsseldateien des Servers öffnen und deren Adressen zeigen)
 * - /api/wallet/…: Browser-Wallet, Schlüssel bleiben beim Besucher
 */
export const PUBLIC_ROUTES: readonly string[] = ["GET /api/status", "GET /api/price", "GET /api/history", "GET /api/keys"];

export function publicRouteAllowed(method: string, pathname: string): boolean {
  if (pathname.startsWith("/api/wallet/")) return true;
  return PUBLIC_ROUTES.includes(`${method.toUpperCase()} ${pathname}`);
}

export const PUBLIC_DENIED_MESSAGE =
  "Auf der öffentlichen Seite gesperrt: Diese Funktion nutzt Schlüsseldateien des Servers. Bitte die Browser-Wallet nutzen oder GHOST auf dem eigenen Rechner betreiben.";
