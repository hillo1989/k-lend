// Umrechnung zwischen Texteingaben und Ganzzahl-Einheiten, im Format der
// aktuellen Sprache (Deutsch: 1.234,56 – Englisch: 1,234.56).
import { STABLE_SYMBOL } from "../config";
import { getLang, locale, tr } from "./i18n";
// Alles in BigInt, damit keine Gleitkomma-Rundung in Beträge gerät.

/**
 * Texteingabe → Einheiten mit `decimals` Nachkommastellen, streng nach Sprache
 * (Deutsch: 1.234,56 – Englisch: 1,234.56). Gibt null bei ungültiger oder
 * mehrdeutiger Eingabe zurück; lieber ablehnen als raten (Audit 10, A10-W-1).
 *
 * - Tausendertrenner nur in echten Dreiergruppen („1.234.567“, „1.234,5“).
 * - Ein einzelnes Trennzeichen der anderen Sprache gilt als Dezimalzeichen
 *   („0.5“ im Deutschen, „1,5“ im Englischen), außer genau drei Ziffern folgen:
 *   „1.500“ bzw. „1,500“ ist mehrdeutig (1,5 oder 1500) und wird abgelehnt,
 *   es sei denn, die Zahl beginnt mit 0.
 */
export function parseUnits(input: string, decimals: number): bigint | null {
  const r = readAmount(input, decimals);
  return r.ok ? r.value : null;
}

/** true, wenn die Eingabe nur wegen „1.500“/„1,500“ abgelehnt wird */
export function isAmbiguousAmount(input: string): boolean {
  return readAmount(input, 18).ambiguous;
}

/** Fehlertext für ein ungültiges Betragsfeld */
export function amountProblem(label: string, input: string): string {
  return isAmbiguousAmount(input.trim())
    ? tr(`${label}: mehrdeutig („${input.trim()}“), bitte eindeutig schreiben.`, `${label}: ambiguous (“${input.trim()}”), please write it unambiguously.`)
    : tr(`${label}: ungültige Zahl.`, `${label}: invalid number.`);
}

function readAmount(input: string, decimals: number): { ok: true; value: bigint; ambiguous: false } | { ok: false; ambiguous: boolean } {
  const s = input.trim().replace(/[\s_\u00a0\u202f']/g, "");
  const fail = (ambiguous = false) => ({ ok: false as const, ambiguous });
  if (s === "") return fail();
  const en = getLang() === "en";
  const G = en ? "," : "."; // Tausender
  const D = en ? "." : ","; // Dezimal
  const grouped = new RegExp(`^[1-9]\\d{0,2}(?:\\${G}\\d{3})+$`);
  let whole: string;
  let frac = "";
  if (s.includes(D)) {
    const parts = s.split(D);
    if (parts.length !== 2) return fail();
    [whole, frac] = parts;
    if (!/^\d*$/.test(frac)) return fail();
    if (whole.includes(G)) {
      if (!grouped.test(whole)) return fail();
      whole = whole.split(G).join("");
    }
  } else if (s.includes(G)) {
    const parts = s.split(G);
    if (parts.length === 2 && parts[1].length !== 3) {
      [whole, frac] = parts; // Dezimalzeichen der anderen Sprache
    } else if (parts.length === 2 && /^0*$/.test(parts[0])) {
      [whole, frac] = parts; // „0.500“ kann keine Tausendergruppe sein
    } else if (parts.length === 2) {
      return fail(/^\d+$/.test(parts[0]) && /^\d{3}$/.test(parts[1]));
    } else {
      if (!grouped.test(s)) return fail();
      whole = parts.join("");
    }
    if (!/^\d*$/.test(frac)) return fail();
  } else {
    whole = s;
  }
  if (!/^\d*$/.test(whole) || (whole === "" && frac === "")) return fail();
  if (frac.length > decimals) return fail();
  const value = BigInt(whole || "0") * 10n ** BigInt(decimals) + BigInt(frac.padEnd(decimals, "0") || "0");
  return { ok: true, value, ambiguous: false };
}

/** Einheiten → „1.234,56" (abgeschnitten auf maxFrac Stellen, Nullen am Ende entfernt). */
export function formatUnits(value: bigint, decimals: number, maxFrac = 2, minFrac = 0): string {
  const neg = value < 0n;
  let v = neg ? -value : value;
  const base = 10n ** BigInt(decimals);
  const whole = v / base;
  v = v % base;
  let frac = v.toString().padStart(decimals, "0").slice(0, maxFrac);
  while (frac.length > minFrac && frac.endsWith("0")) frac = frac.slice(0, -1);
  const en = getLang() === "en";
  const wholeStr = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, en ? "," : ".");
  return (neg ? "−" : "") + wholeStr + (frac ? (en ? "." : ",") + frac : "");
}

export const fmtKas = (sompi: bigint, maxFrac = 2) => `${formatUnits(sompi, 8, maxFrac)} KAS`;
export const fmtStable = (units: bigint, maxFrac = 2) => `${formatUnits(units, 8, maxFrac)} ${STABLE_SYMBOL}`;
export const fmtUsdPrice = (kasUsdE8: bigint) => `${formatUnits(kasUsdE8, 8, 5, 2)} USD`;
/** bps → „187,5 %" */
export const fmtBps = (bps: bigint, maxFrac = 1) => `${formatUnits(bps, 2, maxFrac)} %`;

/** Zahl im Format der aktuellen Sprache (für Demo-Kennzahlen, die keine Beträge im Vertrag sind). */
export const fmtNum = (n: number, frac = 0) =>
  n.toLocaleString(locale(), { minimumFractionDigits: frac, maximumFractionDigits: frac });

/** kaspa:qzhk…26hcsq */
export function shortAddress(addr: string): string {
  const [prefix, rest] = addr.includes(":") ? addr.split(":") : ["", addr];
  if (rest.length <= 12) return addr;
  return `${prefix ? prefix + ":" : ""}${rest.slice(0, 5)}…${rest.slice(-6)}`;
}
