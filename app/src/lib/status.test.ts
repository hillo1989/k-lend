import { describe, expect, it } from "vitest";
import { indexToUnits, kasUsdToUnits, oracleStale, pctToBps, xOnlyKey } from "./status";

describe("xOnlyKey", () => {
  const x = "d64cbe281ed8bc5d22e521ce4534de0a176b6f5dd1d43762faf0a9e2fe90af6a";
  it("streicht das Präfix komprimierter Schlüssel", () => {
    expect(xOnlyKey("02" + x)).toBe(x);
    expect(xOnlyKey("03" + x.toUpperCase())).toBe(x);
  });
  it("lässt x-only-Schlüssel unverändert", () => {
    expect(xOnlyKey(x)).toBe(x);
  });
  it("lehnt Unsinn ab", () => {
    expect(xOnlyKey("04" + x)).toBe(null);
    expect(xOnlyKey("")).toBe(null);
    expect(xOnlyKey(undefined)).toBe(null);
  });
});

describe("Einheiten aus ghostctl-JSON", () => {
  it("rechnet Preis, Index und Prozent um", () => {
    expect(kasUsdToUnits(0.04743136)).toBe(4_743_136n);
    expect(indexToUnits(1.000000665)).toBe(1_000_000_665n);
    expect(pctToBps(5.000000002559999)).toBe(500n);
  });
});

describe("oracleStale", () => {
  const base = { covenantId: "", kasUsd: 0.05, seq: 1, ratePctYear: 5, index: 1, freshError: null };
  it("frisch und jung", () => expect(oracleStale({ ...base, fresh: true, ageMinutes: 17 })).toBe(false));
  it("älter als 6,5 h", () => expect(oracleStale({ ...base, fresh: true, ageMinutes: 391 })).toBe(true));
  it("5 h ist noch frisch (Dauerbetrieb aktualisiert spätestens nach 6 h)", () => expect(oracleStale({ ...base, fresh: true, ageMinutes: 300 })).toBe(false));
  it("von ghostctl als nicht frisch gemeldet", () => expect(oracleStale({ ...base, fresh: false, ageMinutes: 1 })).toBe(true));
});
