// Audit 20 A20d-10: .k-Namen erst ab 3 Zeichen und nicht, solange die Eingabe
// der Anfang einer Adresse sein kann; längere Wartezeit. (Gleiche Regel im
// Server: server/audit20d-kname.test.ts.)
import { describe, expect, it } from "vitest";
import { isKName, KNAME_DEBOUNCE_MS, KNAME_MIN_CHARS } from "./kname";

describe("A20d-10: Tippfragmente gehen nicht an Server und dotk.name", () => {
  it("Adressanfänge und kurze Eingaben sind keine Namen", () => {
    for (const t of ["k", "ka", "kas", "kasp", "kaspa", "kaspat", "kaspatest", "KASPA", " kaspa ", "ab", "a"]) expect(isKName(t), t).toBe(false);
  });

  it("echte Namen gehen weiter durch, auch solche, die mit „kaspa“ beginnen", () => {
    for (const t of ["alice.k", "bob", "a.k", "kaspalover", "kaspa.k", "kaspafan.k"]) expect(isKName(t), t).toBe(true);
    expect(isKName("kaspa:qqq")).toBe(false);
    expect(isKName("ab".repeat(32))).toBe(false);
  });

  it("mindestens 3 Zeichen, Wartezeit nach dem Tippen mindestens 700 ms", () => {
    expect(KNAME_MIN_CHARS).toBe(3);
    expect(KNAME_DEBOUNCE_MS).toBeGreaterThanOrEqual(700);
  });
});
