// Audit 20 A20c-1: Kaspa-Adressen mit Prüfsumme, Präfix und Version prüfen.
// Testvektoren aus rusty-kaspa (crypto/addresses/src/lib.rs, Tests).
import { describe, expect, it } from "vitest";
import { decodeKaspaAddress, encodeKaspaAddress } from "./kaspaAddress.ts";

const VECTORS: [string, "kaspa" | "kaspatest", number, number[]][] = [
  ["kaspatest:qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqhqrxplya", "kaspatest", 0, Array(32).fill(0)],
  ["kaspatest:qyqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqhe837j2d", "kaspatest", 1, Array(33).fill(0)],
  [
    "kaspatest:qxaqrlzlf6wes72en3568khahq66wf27tuhfxn5nytkd8tcep2c0vrse6gdmpks",
    "kaspatest",
    1,
    [0xba, 0x01, 0xfc, 0x5f, 0x4e, 0x9d, 0x98, 0x79, 0x59, 0x9c, 0x69, 0xa3, 0xda, 0xfd, 0xb8, 0x35, 0xa7, 0x25, 0x5e, 0x5f, 0x2e, 0x93, 0x4e, 0x93, 0x22, 0xec, 0xd3, 0xaf, 0x19, 0x0a, 0xb0, 0xf6, 0x0e],
  ],
  ["kaspa:qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqkx9awp4e", "kaspa", 0, Array(32).fill(0)],
  [
    "kaspa:qp0l70zd5x85ttwd6jv7g3s3a8llzj96d8dncn4zmhv4tlzx5k2jyqh70xmfj",
    "kaspa",
    0,
    [0x5f, 0xff, 0x3c, 0x4d, 0xa1, 0x8f, 0x45, 0xad, 0xcd, 0xd4, 0x99, 0xe4, 0x46, 0x11, 0xe9, 0xff, 0xf1, 0x48, 0xba, 0x69, 0xdb, 0x3c, 0x4e, 0xa2, 0xdd, 0xd9, 0x55, 0xfc, 0x46, 0xa5, 0x95, 0x22],
  ],
];

describe("A20c-1: Kaspa-Adressen (bech32/CashAddr)", () => {
  it("Testvektoren von rusty-kaspa: zerlegen und wieder zusammensetzen", () => {
    for (const [addr, prefix, version, payload] of VECTORS) {
      const d = decodeKaspaAddress(addr);
      expect(d, addr).not.toBeNull();
      expect(d!.prefix).toBe(prefix);
      expect(d!.version).toBe(version);
      expect([...d!.payload]).toEqual(payload);
      expect(encodeKaspaAddress(prefix, version, payload)).toBe(addr);
    }
  });

  it("falsche Prüfsumme, falsches Zeichen, zu kurz, falsches Präfix → null", () => {
    expect(decodeKaspaAddress("kaspa:qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqkx9awp4l")).toBeNull(); // Prüfsumme
    expect(decodeKaspaAddress("kaspa:qqqqqqqqqqqqq1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqkx9awp4e")).toBeNull(); // „1“ ist kein bech32-Zeichen
    expect(decodeKaspaAddress("kaspa:qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqkx9awp4e")).toBeNull(); // ein Zeichen zu kurz
    // gleiche Nutzlast, anderes Netz: die Prüfsumme umfasst das Präfix
    expect(decodeKaspaAddress("kaspatest:qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqkx9awp4e")).toBeNull();
    expect(decodeKaspaAddress("bitcoincash:qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqkx9awp4e")).toBeNull();
    expect(decodeKaspaAddress("KASPA:QQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQQKX9AWP4E")).toBeNull();
    // die Angriffsadresse aus dem Audit: Zeichensatz und Länge passen, Prüfsumme nicht
    expect(decodeKaspaAddress("kaspa:q" + "q".repeat(60))).toBeNull();
  });

  it("unbekannte Version oder falsche Länge der Nutzlast → null", () => {
    expect(decodeKaspaAddress(encodeKaspaAddress("kaspa", 2, Array(32).fill(1)))).toBeNull();
    expect(decodeKaspaAddress(encodeKaspaAddress("kaspa", 0, Array(33).fill(1)))).toBeNull();
    expect(decodeKaspaAddress(encodeKaspaAddress("kaspa", 8, Array(32).fill(1)))?.version).toBe(8);
  });
});
