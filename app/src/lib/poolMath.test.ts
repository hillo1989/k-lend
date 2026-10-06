import { describe, expect, it } from "vitest";
import { feeOf, ghostOut, impactBps, kasOut, payoutFor, sharesForDeposit, swapOk, withSlippage } from "./poolMath";

const E8 = 100_000_000n;
const X = 10_000n * E8; // wie protocol/tests/pool_tests.rs
const Y = 460n * E8;

describe("poolMath", () => {
  it("Gebühr rundet auf", () => {
    expect(feeOf(1n)).toBe(1n);
    expect(feeOf(10_000n)).toBe(30n);
    expect(feeOf(10_001n)).toBe(31n);
  });

  it("KAS gegen GHOST: Maximum hält die Regel, eine Einheit mehr nicht", () => {
    const dx = 100n * E8;
    const dy = ghostOut(X, Y, dx);
    expect(swapOk(X, Y, X + dx, Y - dy)).toBe(true);
    expect(swapOk(X, Y, X + dx, Y - dy - 1n)).toBe(false);
    // Grenzen wie im Rust-Test: zwischen 4,54 und 4,555 GHOST
    expect(dy > 454n * E8 / 100n && dy < 4555n * E8 / 1000n).toBe(true);
  });

  it("GHOST gegen KAS: Maximum hält die Regel, ein sompi mehr nicht", () => {
    const dy = 5n * E8;
    const dx = kasOut(X, Y, dy);
    expect(swapOk(X, Y, X - dx, Y + dy)).toBe(true);
    expect(swapOk(X, Y, X - dx - 1n, Y + dy)).toBe(false);
  });

  it("Pool behält mindestens 1 KAS", () => {
    const x = 2n * E8;
    const dx = kasOut(x, Y, 1_000_000n * E8);
    expect(x - dx).toBeGreaterThanOrEqual(E8);
    expect(x - dx).toBeLessThan(E8 + E8 / 100n);
  });

  it("Kursverschiebung wächst mit der Größe", () => {
    const small = impactBps(X, Y, E8, ghostOut(X, Y, E8));
    const big = impactBps(X, Y, 1_000n * E8, ghostOut(X, Y, 1_000n * E8));
    expect(small).toBeGreaterThanOrEqual(30n); // mindestens die Gebühr
    expect(small).toBeLessThan(40n);
    expect(big).toBeGreaterThan(900n);
  });

  it("Mindestbetrag", () => {
    expect(withSlippage(10_000n, 100n)).toBe(9_900n);
  });

  it("Anteile beim Einlegen und Auszahlung beim Abziehen (wie der Vertrag)", () => {
    const S = 20_000n * E8;
    expect(sharesForDeposit(S, X, Y, 1_000n * E8, 46n * E8)).toBe(S / 10n);
    expect(sharesForDeposit(S, X, Y, 1_000n * E8, 23n * E8)).toBe(S / 20n); // kleinere Seite zählt
    expect(payoutFor(S, X, Y, S / 10n)).toEqual([X / 10n, Y / 10n]);
  });
});
