import { describe, expect, it } from "vitest";
import { ghostUsdFromPool, usdOf, usdText } from "./usd";

describe("Dollarwert", () => {
  it("KAS zum Marktpreis, GHOST zum Poolkurs oder zum Ziel 1 USD", () => {
    expect(usdOf(100, "KAS", 0.045, null)).toBeCloseTo(4.5);
    expect(usdOf(100, "KAS", null, null)).toBeNull();
    expect(usdOf(2, "GHOST", 0.045, 1.02)).toBeCloseTo(2.04);
    expect(usdOf(2, "GHOST", null, null)).toBe(2);
    expect(usdOf(5, "%", 0.045, 1)).toBeNull();
    expect(usdOf(Number.NaN, "KAS", 0.045, 1)).toBeNull();
  });
  it("GHOST-Kurs aus den Reserven: KAS je GHOST × KAS-Preis", () => {
    // 11,28 KAS / 0,476 GHOST bei 0,045 USD ⇒ ≈ 1,066 USD
    expect(ghostUsdFromPool({ kasSompi: "1128000000", ghostUnits: "47600000" }, 0.045)).toBeCloseTo(1.0664, 3);
    expect(ghostUsdFromPool(null, 0.045)).toBeNull();
    expect(ghostUsdFromPool({ kasSompi: "1", ghostUnits: "0" }, 0.045)).toBeNull();
  });
  it("Anzeige: kleine Beträge mit mehr Stellen", () => {
    expect(usdText(1234.5)).toMatch(/1\.234,5|1\.234,50/);
    expect(usdText(0.0421)).toContain("0,0421");
    expect(usdText(0.000123)).toContain("0,000123");
  });
});
