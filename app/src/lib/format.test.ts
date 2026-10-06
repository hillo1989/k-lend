import { describe, expect, it } from "vitest";
import { afterEach } from "vitest";
import { formatUnits, isAmbiguousAmount, parseUnits, shortAddress } from "./format";
import { setLangGlobal } from "./i18n";

describe("parseUnits (deutsches Format)", () => {
  it.each([
    ["1", 100_000_000n],
    ["1,5", 150_000_000n],
    ["1.234,56", 123_456_000_000n],
    ["0.5", 50_000_000n],
    ["1.000.000", 100_000_000_000_000n], // Tausenderpunkte in Dreiergruppen
    ["1.000", null], // mehrdeutig: 1 oder 1000 (A10-W-1)
    ["1.500", null],
    ["0.500", 50_000_000n], // mit 0 vorn keine Tausendergruppe
    ["1.50", 150_000_000n],
    ["1.2.3", null],
    ["1.23.456", null],
    ["12.345,6", 1_234_560_000_000n],
    ["1,234.5", null],
    ["1,5,", null],
    ["0,00000001", 1n],
    ["", null],
    ["abc", null],
    ["0,000000001", null], // mehr als 8 Nachkommastellen
  ])("%s", (input, expected) => {
    expect(parseUnits(input, 8)).toBe(expected);
  });
});

describe("parseUnits (englisches Format)", () => {
  afterEach(() => setLangGlobal("de"));
  it.each([
    ["1", 100_000_000n],
    ["1.5", 150_000_000n],
    ["1,234.56", 123_456_000_000n],
    ["1,234,567", 123_456_700_000_000n],
    ["1,5", 150_000_000n], // deutsches Dezimalkomma, eindeutig
    ["0,001", 100_000n], // kein Tausender mit 0 vorn
    ["1,500", null], // mehrdeutig
    ["1,000", null],
    ["12,34,567", null],
    ["1.234,5", null],
    ["0.00000001", 1n],
  ])("%s", (input, expected) => {
    setLangGlobal("en");
    expect(parseUnits(input, 8)).toBe(expected);
  });
  it("meldet Mehrdeutigkeit", () => {
    setLangGlobal("en");
    expect(isAmbiguousAmount("1,500")).toBe(true);
    expect(isAmbiguousAmount("1,5")).toBe(false);
    setLangGlobal("de");
    expect(isAmbiguousAmount("1.500")).toBe(true);
    expect(isAmbiguousAmount("abc")).toBe(false);
  });
});

describe("formatUnits", () => {
  it("setzt Tausenderpunkte und Dezimalkomma", () => {
    expect(formatUnits(123_456_789_000_000n, 8)).toBe("1.234.567,89");
    expect(formatUnits(100_000_000n, 8)).toBe("1");
    expect(formatUnits(1n, 8, 8)).toBe("0,00000001");
    expect(formatUnits(8_000_000n, 8, 5, 2)).toBe("0,08");
  });
});

describe("shortAddress", () => {
  it("kürzt Kaspa-Adressen", () => {
    expect(shortAddress("kaspa:qzhkxxaully72gk23lyn7z3d9tdzdpw48ujsavrwlulekyk7pkxzzxj26hcsq")).toBe("kaspa:qzhkx…26hcsq");
  });
});
