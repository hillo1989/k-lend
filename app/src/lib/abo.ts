// Daueraufträge (ghostctl abo): Datentypen der Liste, Terminvorschau und
// Vorprüfung des Formulars. Die Terminrechnung folgt ghostctl (abo.rs):
// immer vom Start aus gerechnet, monatlich bei kürzeren Monaten der letzte Tag.
// Verbindlich ist, was ghostctl beim Anlegen sagt.
import type { KeyEntry } from "./api";
import { tr } from "./i18n";
import type { Hint } from "./precheck";

export type AboAsset = "KAS" | "GHOST";
export type AboInterval = "daily" | "weekly" | "monthly" | { days: number };

export interface AboHist {
  date: string;
  at: string;
  amount: string;
  txid?: string | null;
  ok: boolean;
  error?: string | null;
  skipped?: number;
  note?: string | null;
}

export interface Abo {
  id: string;
  key: string;
  asset: AboAsset;
  to: string;
  amount: string;
  message: string;
  onchain: boolean;
  /** Nachricht bei jeder Zahlung verschlüsselt an den Empfänger; fehlt bei alten Aufträgen (Nachricht nur lokal) */
  encrypt?: boolean;
  interval: AboInterval;
  start: string;
  end: string | null;
  count: number | null;
  paused: boolean;
  pauseReason: string | null;
  nextDue: string | null;
  history: AboHist[];
  created: string;
  ended: string | null;
  /** aktiv | pausiert | läuft | abgeschlossen | beendet (von ghostctl berechnet) */
  status: string;
  upcoming: string[];
  inflight?: { date: string; txid?: string | null } | null;
  retry?: { date: string; attempts: number } | null;
}

export interface AboList {
  ok: boolean;
  error?: string;
  abos?: Abo[];
  archive?: Abo[];
}

/** Höchstlänge wie ghostctl (abo::MAX_MESSAGE_CHARS) */
export const MAX_MESSAGE_CHARS = 100;
/** Kleinere KAS-Beträge treiben die Speichermasse (KIP-9) stark hoch */
export const MIN_KAS_AMOUNT = 0.2;
// Zeichen, die eine Nachricht nie enthalten darf – dieselbe Tabelle wie
// abo::check_message in ghostctl (Begründung dort bei BAD_CHARS): Steuer- und
// Formatzeichen (Cc, Cf), Unicode-„standardmäßig unsichtbare“ Zeichen
// (Default_Ignorable_Code_Point, darunter Hangul-Füller, Variation Selectors,
// Tag-Zeichen), Zeilen-/Absatztrenner, Private Use und Nichtzeichen.
// VS15/VS16 (U+FE0E/FE0F) nur direkt hinter einem Bildzeichen (❤️).
const BAD_RANGES: readonly (readonly [number, number])[] = [
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

/** Verbotenes Zeichen? `prev` = Codepunkt davor (für VS15/VS16) */
function isBad(u: number, prev: number | null): boolean {
  const vsOk = (u === 0xfe0e || u === 0xfe0f) && prev !== null && isPictograph(prev);
  return !vsOk && ((u & 0xfffe) === 0xfffe || BAD_RANGES.some(([a, b]) => u >= a && u <= b));
}

/** Enthält der Text ein verbotenes Zeichen? (wie abo::check_message; einzelne Surrogate zählen mit) */
export function hasBadChar(t: string): boolean {
  let prev: number | null = null;
  for (const ch of t) {
    const u = ch.codePointAt(0)!;
    if (isBad(u, prev)) return true;
    prev = u;
  }
  return false;
}

// Leerraum wie Rusts char::is_whitespace (White_Space); \s in JavaScript weicht ab (U+0085, U+FEFF)
const isWhiteSpace = (u: number) =>
  (u >= 0x09 && u <= 0x0d) || u === 0x20 || u === 0x85 || u === 0xa0 || u === 0x1680 || (u >= 0x2000 && u <= 0x200a) || u === 0x2028 || u === 0x2029 || u === 0x202f || u === 0x205f || u === 0x3000;

/**
 * Gespeicherte Nachricht so, wie ghostctl sie heute sendet (abo::sendable_message):
 * verbotene Zeichen fallen weg, verbotener Leerraum wird zum Leerzeichen.
 * Ältere Daueraufträge wurden mit dem schwächeren Filter angelegt.
 */
export function sendableMessage(m: string): string {
  let out = "";
  let prev: number | null = null;
  for (const ch of m) {
    let u = ch.codePointAt(0)!;
    let c = ch;
    if (isWhiteSpace(u) && isBad(u, prev)) [u, c] = [0x20, " "];
    if (!isBad(u, prev)) {
      out += c;
      prev = u;
    }
  }
  return [...out.trim()].slice(0, MAX_MESSAGE_CHARS).join("").trimEnd();
}

/** Fehlertext einer Nachricht oder null (leer ist erlaubt) */
export function messageProblem(m: string): string | null {
  const t = m.trim();
  if ([...t].length > MAX_MESSAGE_CHARS) return tr(`Nachricht: höchstens ${MAX_MESSAGE_CHARS} Zeichen.`, `Message: at most ${MAX_MESSAGE_CHARS} characters.`);
  if (hasBadChar(t)) return tr("Nachricht: nur eine Zeile normaler Text, ohne Steuerzeichen und unsichtbare Zeichen.", "Message: a single line of plain text, no control or invisible characters.");
  return null;
}

/** Beschriftung des Häkchens „öffentlich“ (Senden und Daueraufträge) */
export const PUBLIC_MESSAGE_LABEL = () =>
  tr(
    "Nachricht öffentlich in die Transaktion schreiben (für alle sichtbar). Ohne Häkchen wird sie verschlüsselt, nur der Empfänger kann sie lesen.",
    "Write the message publicly into the transaction (visible to everyone). Without the tick it is encrypted; only the recipient can read it.",
  );

/**
 * Kann an diesen Empfänger verschlüsselt werden? Verschlüsselt wird an den
 * Schnorr-Schlüssel der Adresse (Schlüsseldatei, x-only-Schlüssel, normale
 * Adresse kaspa:q… mit 61 Zeichen nach dem Präfix). Skript-Adressen
 * (kaspa:p…) und ECDSA-Adressen (kaspa:q… mit 63 Zeichen) haben keinen
 * solchen Schlüssel. Nur diese sicheren Fälle werden hier gemeldet;
 * verbindlich prüft ghostctl.
 */
export function encryptProblem(to: string): string | null {
  const t = to.trim();
  if (/^(kaspa|kaspatest):(p[a-z0-9]+|q[a-z0-9]{62})$/.test(t))
    return tr(
      "Verschlüsselt nur an normale Kaspa-Adressen (kaspa:q…) – Nachricht öffentlich schreiben oder weglassen.",
      "Encryption only works for normal Kaspa addresses (kaspa:q…) – make the message public or leave it out.",
    );
  return null;
}

// ---------------------------------------------------------- Kalender ----

const DATE_RE = /^(\d{4})-(\d{2})-(\d{2})$/;

function parse(d: string): [number, number, number] | null {
  const m = d.match(DATE_RE);
  if (!m) return null;
  const [y, mo, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const t = new Date(Date.UTC(y, mo - 1, day));
  if (t.getUTCMonth() !== mo - 1 || t.getUTCDate() !== day) return null;
  return [y, mo, day];
}

export function isDate(d: string): boolean {
  return parse(d) !== null;
}

const iso = (t: Date) => t.toISOString().slice(0, 10);
const daysIn = (y: number, mo: number) => new Date(Date.UTC(y, mo, 0)).getUTCDate();

/** Lokales Datum JJJJ-MM-TT (Standard für den Start) */
export function localToday(now = new Date()): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** Termin Nr. n (0 = Start), wie Abo::occurrence in ghostctl */
export function occurrence(start: string, interval: AboInterval, n: number): string | null {
  const p = parse(start);
  if (!p) return null;
  const [y, mo, d] = p;
  if (interval === "monthly") {
    const total = mo - 1 + n;
    const ty = y + Math.floor(total / 12);
    const tm = (total % 12) + 1;
    return iso(new Date(Date.UTC(ty, tm - 1, Math.min(d, daysIn(ty, tm)))));
  }
  const step = interval === "daily" ? 1 : interval === "weekly" ? 7 : interval.days;
  return iso(new Date(Date.UTC(y, mo - 1, d + step * n)));
}

/** Die ersten `k` Termine unter Beachtung von Ende und Anzahl */
export function schedule(start: string, interval: AboInterval, k: number, end: string | null = null, count: number | null = null): string[] {
  const out: string[] = [];
  for (let n = 0; out.length < k; n++) {
    if (count !== null && n >= count) break;
    const d = occurrence(start, interval, n);
    if (d === null || (end !== null && d > end)) break;
    out.push(d);
  }
  return out;
}

// ------------------------------------------------------------ Anzeige ----

export function intervalLabel(i: AboInterval): string {
  if (i === "daily") return tr("täglich", "daily");
  if (i === "weekly") return tr("wöchentlich", "weekly");
  if (i === "monthly") return tr("monatlich", "monthly");
  return i.days === 1 ? tr("täglich", "daily") : tr(`alle ${i.days} Tage`, `every ${i.days} days`);
}

export function statusLabel(s: string): string {
  const m: Record<string, [string, string]> = {
    aktiv: ["aktiv", "active"],
    pausiert: ["pausiert", "paused"],
    läuft: ["wird ausgeführt", "running"],
    abgeschlossen: ["abgeschlossen", "completed"],
    beendet: ["beendet", "ended"],
  };
  const t = m[s];
  return t ? tr(t[0], t[1]) : s;
}

// --------------------------------------------------------- Formular ----

export interface AboForm {
  asset: AboAsset;
  to: string;
  /** Betrag in 1e8-Einheiten, null = ungültig/leer */
  amount: bigint | null;
  amountText: string;
  interval: AboInterval | null;
  start: string;
  endMode: "none" | "date" | "count";
  end: string;
  count: string;
  message: string;
  onchain: boolean;
}

/** Parameter für POST /api/action (abo-add) oder ein Problem */
export function aboParams(f: AboForm, key: KeyEntry | null, cliDecimal: (u: bigint) => string, today: string): { params: Record<string, string | number | boolean> | null; problem: string | null } {
  const fail = (p: string) => ({ params: null, problem: p });
  if (!key) return fail(tr("Kein Schlüssel gewählt.", "No key selected."));
  if (!f.to.trim()) return fail(tr("Empfänger angeben.", "Enter a recipient."));
  if (f.to.trim() === key.file) return fail(tr("Empfänger ist das eigene Konto.", "The recipient is your own account."));
  if (f.amountText.trim() === "") return fail(tr("Betrag eingeben.", "Enter an amount."));
  if (f.amount === null || f.amount <= 0n) return fail(tr("Betrag: ungültige Zahl.", "Amount: invalid number."));
  if (f.interval === null) return fail(tr("Intervall: 1 bis 3650 Tage.", "Interval: 1 to 3650 days."));
  if (!isDate(f.start)) return fail(tr("Start: gültiges Datum wählen.", "Start: choose a valid date."));
  if (f.start < today) return fail(tr("Start liegt in der Vergangenheit.", "Start is in the past."));
  const p: Record<string, string | number | boolean> = {
    key: key.file,
    asset: f.asset,
    to: f.to.trim(),
    amount: cliDecimal(f.amount),
    interval: typeof f.interval === "string" ? f.interval : String(f.interval.days),
    start: f.start,
  };
  if (f.endMode === "date") {
    if (!isDate(f.end)) return fail(tr("Ende: gültiges Datum wählen.", "End: choose a valid date."));
    if (f.end < f.start) return fail(tr("Ende liegt vor dem Start.", "End is before the start."));
    p.end = f.end;
  }
  if (f.endMode === "count") {
    if (!/^\d{1,4}$/.test(f.count.trim()) || Number(f.count) < 1) return fail(tr("Anzahl: ganze Zahl ab 1.", "Count: whole number from 1."));
    p.count = Number(f.count);
  }
  const mp = messageProblem(f.message);
  if (mp) return fail(mp);
  if (f.message.trim()) p.message = f.message.trim();
  if (f.onchain) {
    if (!f.message.trim()) return fail(tr("Für eine öffentliche Nachricht erst eine Nachricht eingeben.", "Enter a message first to publish it."));
    p.onchain = true;
  } else if (f.message.trim()) {
    const ep = encryptProblem(f.to);
    if (ep) return fail(ep);
  }
  return { params: p, problem: null };
}

/** Hinweise zum Formular (Guthaben, Kleinstbeträge, öffentliche Nachricht) */
export function aboHints(f: AboForm, key: KeyEntry | null): Hint[] {
  const out: Hint[] = [];
  const amt = f.amount !== null ? Number(f.amount) / 1e8 : null;
  if (f.asset === "KAS" && amt !== null && amt > 0 && amt < MIN_KAS_AMOUNT)
    out.push({ level: "warn", text: tr("Beträge unter etwa 0,2 KAS machen die Transaktion sehr schwer (Speichermasse) – sie kann scheitern.", "Amounts below about 0.2 KAS make the transaction very heavy (storage mass) – it may fail.") });
  if (key && amt !== null && f.asset === "KAS" && key.kas !== null && amt > key.kas)
    out.push({ level: "warn", text: tr(`Der Schlüssel hat derzeit nur ${key.kas} KAS. Fehlt beim Termin Guthaben, wird es zweimal erneut versucht, dann pausiert.`, `The key currently has only ${key.kas} KAS. If funds are missing at a due date, it is retried twice, then paused.`) });
  if (key && amt !== null && f.asset === "GHOST" && amt > key.ghost)
    out.push({ level: "warn", text: tr(`Der Schlüssel hat derzeit nur ${key.ghost} GHOST.`, `The key currently has only ${key.ghost} GHOST.`) });
  if (f.asset === "GHOST")
    out.push({ level: "info", text: tr("Jede GHOST-Sendung braucht etwas KAS für die Gebühr und etwa 1 KAS, das mit dem Token beim Empfänger liegt.", "Each GHOST transfer needs some KAS for the fee and about 1 KAS that stays with the token at the recipient.") });
  if (f.onchain && f.message.trim())
    out.push({ level: "warn", text: tr("Die Nachricht steht bei jeder Zahlung dauerhaft und für alle lesbar in der Blockchain.", "The message is stored permanently and readable by anyone in the blockchain with every payment.") });
  else if (f.message.trim())
    out.push({
      level: "info",
      text: tr(
        "Die Nachricht wird bei jeder Zahlung neu verschlüsselt in die Transaktion geschrieben; lesen kann sie nur der Empfänger. Sichtbar bleibt, dass es eine Nachricht gibt und wie lang sie ist.",
        "The message is freshly encrypted into the transaction with every payment; only the recipient can read it. Visible to everyone: that there is a message and its length.",
      ),
    });
  return out;
}
