// Kaspa-Adressen vollständig prüfen (Audit 20 A20c-1): Präfix, Zeichensatz,
// Prüfsumme (CashAddr-Polymod wie rusty-kaspa crypto/addresses/src/bech32.rs),
// Version und Länge der Nutzlast. Vorher prüfte die Seite nur Zeichensatz und
// Länge; eine Adresse mit falscher Prüfsumme startete trotzdem ghostctl, und
// das verband sich erst mit dem Node (11–13 s), bevor es die Adresse ablehnte.
// Reine Logik ohne Ein-/Ausgabe (Vitest).

const CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const REV = new Map([...CHARSET].map((c, i) => [c, i]));

/** Versionen (Byte vor der Nutzlast) und ihre Länge in Byte */
export const ADDRESS_VERSIONS = { pubkey: 0, pubkeyEcdsa: 1, scriptHash: 8 } as const;
const PAYLOAD_LEN: Record<number, number> = { 0: 32, 1: 33, 8: 32 };

export type KaspaPrefix = "kaspa" | "kaspatest";

export interface KaspaAddress {
  prefix: KaspaPrefix;
  version: number;
  payload: Uint8Array;
}

const GEN = [0x98f2bc8e61n, 0x79b76d99e2n, 0xf33e5fb3c4n, 0xae2eabe2a8n, 0x1e4f43e470n];

function polymod(values: Iterable<number>): bigint {
  let c = 1n;
  for (const d of values) {
    const c0 = c >> 35n;
    c = ((c & 0x07ffffffffn) << 5n) ^ BigInt(d);
    for (let i = 0; i < 5; i++) if ((c0 >> BigInt(i)) & 1n) c ^= GEN[i];
  }
  return c ^ 1n;
}

function checksum(prefix: string, data5: number[]): bigint {
  return polymod([...[...prefix].map((ch) => ch.charCodeAt(0) & 0x1f), 0, ...data5, 0, 0, 0, 0, 0, 0, 0, 0]);
}

/** 8-Bit-Bytes → 5-Bit-Gruppen, rechts mit Nullen aufgefüllt */
function to5(bytes: readonly number[]): number[] {
  const out: number[] = [];
  let buf = 0;
  let bits = 0;
  for (const b of bytes) {
    buf = (buf << 8) | b;
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out.push((buf >> bits) & 31);
    }
    buf &= (1 << bits) - 1;
  }
  if (bits > 0) out.push((buf << (5 - bits)) & 31);
  return out;
}

/** 5-Bit-Gruppen → Bytes; Füllbits müssen null sein */
function to8(groups: readonly number[]): number[] | null {
  const out: number[] = [];
  let buf = 0;
  let bits = 0;
  for (const g of groups) {
    buf = (buf << 5) | g;
    bits += 5;
    while (bits >= 8) {
      bits -= 8;
      out.push((buf >> bits) & 0xff);
    }
    buf &= (1 << bits) - 1;
  }
  if (bits >= 5 || buf !== 0) return null;
  return out;
}

/** Adresse aus Präfix, Version und Nutzlast (für Tests und Anzeigen) */
export function encodeKaspaAddress(prefix: KaspaPrefix, version: number, payload: Uint8Array | readonly number[]): string {
  const data = to5([version, ...payload]);
  const sum = checksum(prefix, data);
  const sum5: number[] = [];
  for (let i = 7; i >= 0; i--) sum5.push(Number((sum >> BigInt(i * 5)) & 31n));
  return `${prefix}:${[...data, ...sum5].map((d) => CHARSET[d]).join("")}`;
}

/**
 * Zerlegt eine Kaspa-Adresse. null bei falschem Präfix, fremden Zeichen,
 * falscher Prüfsumme, unbekannter Version oder falscher Länge. Nur
 * Kleinbuchstaben (so schreiben Wallets und ghostctl die Adressen).
 */
export function decodeKaspaAddress(s: string): KaspaAddress | null {
  if (typeof s !== "string" || s.length > 120) return null;
  const colon = s.indexOf(":");
  if (colon < 0) return null;
  const prefix = s.slice(0, colon);
  if (prefix !== "kaspa" && prefix !== "kaspatest") return null;
  const body = s.slice(colon + 1);
  if (body.length < 9) return null;
  const groups: number[] = [];
  for (const ch of body) {
    const v = REV.get(ch);
    if (v === undefined) return null;
    groups.push(v);
  }
  const data = groups.slice(0, -8);
  let sum = 0n;
  for (const g of groups.slice(-8)) sum = (sum << 5n) | BigInt(g);
  if (checksum(prefix, data) !== sum) return null;
  const bytes = to8(data);
  if (!bytes || bytes.length < 1) return null;
  const [version, ...payload] = bytes;
  if (PAYLOAD_LEN[version] === undefined || payload.length !== PAYLOAD_LEN[version]) return null;
  return { prefix, version, payload: Uint8Array.from(payload) };
}

/** Präfix des Netzes */
export const prefixOf = (network: string): KaspaPrefix => (network === "mainnet" ? "kaspa" : "kaspatest");
