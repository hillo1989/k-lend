// Prüft den Rechenkern (Version 3) gegen exakte BigInt-Rechnung – dieselben
// Grenz- und Zufallsfälle wie protocol/tests/vault_math_tests.rs und die
// Referenzwerte aus protocol/tests/vault_tests.rs (v3_…), dort gegen u128.
import { describe, expect, it } from "vitest";
import {
  DUST,
  INDEX_SCALE,
  MIN_REDEEM,
  SWEEP_FEE,
  MAX_COLLATERAL,
  MAX_DEBT,
  MAX_GROWTH,
  OverflowError,
  UNIT,
  accrual,
  accrued,
  closeFee,
  growth,
  healthFactorE4,
  healthy,
  interestFee,
  liquidationPreview,
  maxMintable,
  maxRedeemable,
  maxWithdrawable,
  minHealthyPrice,
  mulDivDown,
  mulDivUp,
  oracleIndexAfter,
  owedOf,
  projectInterest,
  rateFromAprBps,
  ratioBps,
  redeemPayout,
  simClose,
  simDeposit,
  simLiquidate,
  simMint,
  simRedeem,
  simRepay,
  simSweep,
  simWithdraw,
  sweepable,
  type VaultState,
} from "./vaultMath";

// ------------------------------------------------ exakte Referenz ----------
const ceilDiv = (n: bigint, d: bigint) => (n + d - 1n) / d;
const refDown = (a: bigint, b: bigint, d: bigint) => (a * b) / d;
const refUp = (a: bigint, b: bigint, d: bigint) => ceilDiv(a * b, d);
const refGrowth = (from: bigint, to: bigint) => {
  const g = ceilDiv((to - from) * INDEX_SCALE, from);
  return g > MAX_GROWTH ? MAX_GROWTH : g;
};
const refAccrual = (d: bigint, from: bigint, to: bigint) => (d > 0n && to > from && from > 0n ? refUp(d, refGrowth(from, to), INDEX_SCALE) : 0n);
const refHealthy = (c: bigint, owed: bigint, p: bigint, r: bigint) => owed === 0n || refDown(c, p, 100_000_000n) >= refUp(owed, r, 10_000n);

// deterministischer Zufall (xorshift64*), Seed wie im Rust-Test
function rng(seed: bigint) {
  let x = seed;
  const M = (1n << 64n) - 1n;
  const next = () => {
    x ^= x >> 12n;
    x ^= (x << 25n) & M;
    x ^= x >> 27n;
    return (x * 2685821657736338717n) & M;
  };
  return (lo: bigint, hi: bigint) => lo + (next() % (hi - lo + 1n));
}

const MCR = 20_000n;
const LIQ = 15_000n;
const BONUS = 1_000n;
const E8 = UNIT;
const INDEX0 = INDEX_SCALE; // 1,00
const INDEX = 1_050_000_000n; // 1,05
const PRICE = 4_000_000n; // 0,04 USD
const COLL = 10_000n * E8; // 10 000 KAS = 400 USD
const O = { kasUsd: PRICE, stableIndex: INDEX };
const st = (debt: bigint, interest = 0n, indexAt = INDEX, collateral = COLL): VaultState => ({ collateral, debt, interest, indexAt });

describe("mulDivDown / mulDivUp", () => {
  const cases: [bigint, bigint, bigint][] = [
    [0n, 4_000_000n, 100_000_000n],
    [1n, 1n, 100_000_000n],
    [MAX_COLLATERAL, 92_000_000_000n, 100_000_000n],
    [MAX_COLLATERAL - 1n, 4_000_000n, 100_000_000n],
    [123_456_789_012_345n, 30_000n, 10_000n],
    [99_999_999n, 92_000_000_000n, 100_000_000n], // größter Rest × Preisgrenze
  ];
  it.each(cases)("Grenzfall %s·%s/%s", (a, b, d) => {
    expect(mulDivDown(a, b, d)).toBe(refDown(a, b, d));
    expect(mulDivUp(a, b, d)).toBe(refUp(a, b, d));
  });

  it("rundet ab bzw. auf, wenn ein Rest bleibt", () => {
    expect(mulDivDown(1n, 1n, 3n)).toBe(0n);
    expect(mulDivUp(1n, 1n, 3n)).toBe(1n);
    expect(mulDivUp(3n, 1n, 3n)).toBe(1n); // kein Rest → kein Aufrunden
  });

  it("bricht über der Preisgrenze (≈ 920 USD/KAS) ab wie die Engine", () => {
    expect(() => mulDivDown(99_999_999n, 99_999_999_999n, 100_000_000n)).toThrow(OverflowError);
  });
});

describe("growth und accrual – Zins rundet AUF, Deckel Index ×10", () => {
  it("Referenzwert: 100 GHOST von Index 1,00 bis 1,05 → 5 USD", () => {
    expect(growth(INDEX0, INDEX)).toBe(50_000_000n);
    expect(accrual(100n * E8, INDEX0, INDEX)).toBe(5n * E8);
  });

  it("Deckel: accrual(1e17, 1e9, 5e10) = 9e17", () => {
    expect(accrual(100_000_000_000_000_000n, 1_000_000_000n, 50_000_000_000n)).toBe(900_000_000_000_000_000n);
  });

  // wie protocol/tests/vault_math_tests.rs zins_grenzfaelle
  const cases: [bigint, bigint, bigint][] = [
    [0n, 1_000_000_000n, 2_000_000_000n], // ohne Schuld kein Zins
    [1n, 1_000_000_000n, 1_000_000_000n], // ohne Zuwachs kein Zins
    [1n, 1_000_000_000n, 1_000_000_001n], // aufrunden: 1 Einheit
    [100n * E8, 1_000_000_000n, 1_050_000_000n],
    [MAX_DEBT, 1_000_000_000n, 10_000_000_000n], // größte Schuld, Index ×10
    [MAX_DEBT, 5_000_000_000n, 5_000_000_001n],
    [123_456_789_123n, 1_052_345_678n, 1_300_000_001n],
    [5n, 0n, 1_000_000_000n], // indexAt 0: nichts
    [5n, 2_000_000_000n, 1_000_000_000n], // Index kleiner: nichts
    [MAX_DEBT, 1_000_000_000n, 50_000_000_000n], // über dem Deckel
    [MAX_DEBT, 9_000_000_000_000n, 9_000_000_000_001n], // größter exakter Index
    [MAX_DEBT, 2_000_000_000_000n, 7_000_000_000_000n],
  ];
  it.each(cases)("Grenzfall d=%s from=%s to=%s", (d, from, to) => {
    expect(accrual(d, from, to)).toBe(refAccrual(d, from, to));
  });

  it("accrued: stehender Zins + Zins auf Schuld UND offenen Zins seit indexAt (Zinseszins)", () => {
    // {debt 10, interest 3, indexAt 1,00} bei 1,05 → 3 + 13·5 % = 3,65 USD
    expect(accrued(st(10n * E8, 3n * E8, INDEX0), INDEX)).toBe(365_000_000n);
    // ohne Schuld verzinst sich der offene Zins weiter: 2 · 1,05 = 2,1 USD
    expect(accrued(st(0n, 2n * E8, INDEX0), INDEX)).toBe(210_000_000n);
    // ohne Schuld und Zins wächst nichts
    expect(accrued(st(0n, 0n, INDEX0), INDEX)).toBe(0n);
    expect(owedOf(st(10n * E8, 3n * E8, INDEX0), INDEX)).toBe(1_365_000_000n);
  });

  it("accrued hängt nicht davon ab, wie oft abgerechnet wird (Audit 11 A11-V-5)", () => {
    // einmal von 1,00 auf 1,21 gegenüber zweimal je +10 %
    const s = st(100n * E8, 0n, INDEX0);
    const once = accrued(s, 1_210_000_000n);
    const mid = accrued(s, 1_100_000_000n);
    const twice = accrued(st(100n * E8, mid, 1_100_000_000n), 1_210_000_000n);
    expect(once).toBe(21n * E8);
    expect(twice).toBe(once);
  });
});

describe("Zufallswerte gegen exakte Rechnung", () => {
  const r = rng(20260928n);
  it("150 Runden je Funktion", () => {
    for (let k = 0; k < 150; k++) {
      const coll = r(0n, MAX_COLLATERAL);
      const price = r(1n, 92_000_000_000n);
      expect(mulDivDown(coll, price, 100_000_000n)).toBe(refDown(coll, price, 100_000_000n));

      const debt = r(0n, 1_000_000_000_000_000_000n);
      const bps = r(10_000n, 30_000n);
      expect(mulDivUp(debt, bps, 10_000n)).toBe(refUp(debt, bps, 10_000n));

      const d = r(0n, MAX_DEBT);
      const from = r(1_000_000_000n, 9_000_000_000_000n);
      const to = from + r(0n, from * 12n);
      expect(accrual(d, from, to)).toBe(refAccrual(d, from, to));

      const owed = r(0n, 10n ** 13n);
      expect(healthy(coll, owed, price, MCR)).toBe(refHealthy(coll, owed, price, MCR));

      // accrued wie im Vertrag: interest + accrual(debt + interest, …)
      const iv = r(0n, MAX_DEBT);
      expect(accrued(st(d, iv, from), to)).toBe(iv + refAccrual(d + iv, from, to));
    }
  });
});

describe("Einträge wie im Vertrag (Version 3)", () => {
  it("mint: Grenze bei genau 200 %, Zins zählt mit", () => {
    // 400 USD tragen 200 USD; 100 GHOST + 50 USD Zins → 50 GHOST frei
    expect(maxMintable(st(100n * E8, 50n * E8), O, MCR)).toBe(50n * E8);
    expect(maxMintable(st(100n * E8), O, MCR)).toBe(100n * E8); // ohne Zins wären es 100
    expect(simMint(st(100n * E8, 50n * E8), 50n * E8, O, MCR).ok).toBe(true);
    expect(simMint(st(100n * E8, 50n * E8), 50n * E8 + 1n, O, MCR).ok).toBe(false);
  });

  it("mint verbucht den Zins und zieht indexAt nach; die Schuld wächst nur um den Betrag", () => {
    const r = simMint(st(100n * E8, 0n, INDEX0), 10n * E8, O, MCR);
    expect(r.ok && r.state).toEqual(st(110n * E8, 5n * E8, INDEX));
  });

  it("mint: Obergrenze je Vault gilt nur für die Schuld, nicht für den Zins", () => {
    const big = st(30n * E8, 10n * E8, INDEX, 1_000_000n * E8);
    expect(simMint(big, 20n * E8, O, MCR, 50n * E8).ok).toBe(true);
    expect(simMint(big, 20n * E8 + 1n, O, MCR, 50n * E8).ok).toBe(false);
    expect(maxMintable(big, O, MCR, 50n * E8)).toBe(20n * E8);
  });

  it("repay: höchstens die Schuld, der Zins bleibt stehen", () => {
    const s = st(10n * E8, 3n * E8, INDEX0);
    const r = simRepay(s, 10n * E8, O);
    expect(r.ok && r.state).toEqual(st(0n, 365_000_000n, INDEX));
    expect(simRepay(s, 10n * E8 + 1n, O).ok).toBe(false); // Zins lässt sich nicht mit GHOST tilgen
    expect(simRepay(s, 0n, O).ok).toBe(false);
    const part = simRepay(s, 1n, O); // jeder Betrag > 0 tilgt genau so viel
    expect(part.ok && part.state?.debt).toBe(10n * E8 - 1n);
  });

  it("withdraw: Zins zählt für die Mindestquote", () => {
    // 50 GHOST + 25 USD Zins → 150 USD bei 200 % = 3 750 KAS müssen bleiben
    const s = st(50n * E8, 25n * E8);
    const w = maxWithdrawable(s, O, MCR);
    expect(COLL - w).toBe(3_750n * E8);
    expect(simWithdraw(s, w, O, MCR).ok).toBe(true);
    expect(simWithdraw(s, w + 1n, O, MCR).ok).toBe(false);
    // ohne Schuld und Zins: Abheben lässt mindestens 1 sompi
    expect(simWithdraw(st(0n), COLL, O, MCR).ok).toBe(false);
    expect(maxWithdrawable(st(0n), O, MCR)).toBe(COLL - 1n);
  });

  it("deposit: nur echte Erhöhung, höchstens MAX_COLLATERAL, Zins unverändert", () => {
    const s = st(0n, 7n, INDEX0);
    expect(simDeposit(s, 0n).ok).toBe(false);
    const r = simDeposit(s, E8);
    expect(r.ok && r.state).toEqual({ ...s, collateral: COLL + E8 });
    expect(simDeposit(s, MAX_COLLATERAL).ok).toBe(false);
  });

  it("closeFee: Referenzwerte, Grenze 0,2 KAS", () => {
    expect(closeFee(2n * E8, PRICE).fee).toBe(5_000_000_000n);
    const edge = closeFee(800_000n, PRICE);
    expect(edge.fee).toBe(20_000_000n);
    expect(edge.waived).toBe(false); // genau an der Grenze wird gezahlt
    expect(edge.due).toBe(DUST);
    const small = closeFee(700_000n, PRICE);
    expect(small.fee).toBe(17_500_000n);
    expect(small.waived).toBe(true);
    expect(small.due).toBe(0n);
    expect(closeFee(E8, PRICE, E8).fee).toBe(E8); // höchstens die Sicherheit
  });

  it("close: nur ohne Schuld; Zins an die Zinsadresse, Rest an den Besitzer", () => {
    expect(simClose(st(1n), O).ok).toBe(false);
    const r = simClose(st(0n, 2n * E8), O);
    expect(r.ok && r.state).toBe(null);
    expect(r.ok && r.fee).toBe(50n * E8);
    expect(r.ok && r.payout).toBe(COLL - 50n * E8);
    const waived = simClose(st(0n, 700_000n), O);
    expect(waived.ok && waived.fee).toBe(0n);
    expect(waived.ok && waived.payout).toBe(COLL);
  });

  it("redeem: Referenzwerte der Auszahlung (1 USD je GHOST minus 1 %)", () => {
    expect(redeemPayout(10n * E8, PRICE)).toBe(24_750_000_000n);
    expect(redeemPayout(800_000n, PRICE)).toBe(19_800_000n);
  });

  it("redeem: jeder, höchstens die Schuld, Zins wird verbucht", () => {
    const s = st(100n * E8, 0n, INDEX0);
    const r = simRedeem(s, 10n * E8, O, LIQ);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    expect(r.paid).toBe(24_750_000_000n);
    expect(r.state).toEqual(st(90n * E8, 5n * E8, INDEX, COLL - 24_750_000_000n));
    expect(simRedeem(st(5n * E8), 5n * E8, O, LIQ).ok).toBe(true); // ganze Schuld geht
    expect(simRedeem(st(5n * E8), 5n * E8 + 1n, O, LIQ).ok).toBe(false);
    expect(simRedeem(s, 0n, O, LIQ).ok).toBe(false);
  });

  it("redeem: nur ab der Liquidationsschwelle", () => {
    const s = st(150n * E8);
    const p = minHealthyPrice(COLL, 150n * E8, LIQ)!;
    expect(simRedeem(s, 10n * E8, { kasUsd: p - 1n, stableIndex: INDEX }, LIQ).ok).toBe(false);
    expect(simRedeem(s, 10n * E8, { kasUsd: p, stableIndex: INDEX }, LIQ).ok).toBe(true);
  });

  it("redeem: mindestens 1 GHOST oder die ganze Schuld (Audit 11 A11-V-3)", () => {
    const s = st(100n * E8);
    expect(simRedeem(s, MIN_REDEEM - 1n, O, LIQ).ok).toBe(false);
    expect(simRedeem(s, MIN_REDEEM, O, LIQ).ok).toBe(true);
    expect(simRedeem(s, 2n, O, LIQ).ok).toBe(false); // 2 Einheiten bewegten früher jeden Vault
    expect(simRedeem(st(E8 / 2n), E8 / 2n, O, LIQ).ok).toBe(true); // ganze Schuld unter 1 GHOST geht
    expect(simRedeem(st(E8 / 2n), E8 / 4n, O, LIQ).ok).toBe(false);
  });

  it("redeem: im Vault bleiben mindestens 0,2 KAS", () => {
    const coll = 30_000_000n; // 0,3 KAS = 0,012 USD
    const s = st(800_000n, 0n, INDEX, coll);
    expect(healthy(coll, 800_000n, PRICE, LIQ)).toBe(true);
    expect(simRedeem(s, 800_000n, O, LIQ).ok).toBe(false); // ganze Schuld, aber Rest 0,102 KAS
    expect(maxRedeemable(coll, 800_000n, PRICE)).toBe(0n); // Teilbeträge unter 1 GHOST gehen nicht
    // Grenze 0,2 KAS: 25 KAS (1 USD), 10 GHOST Schuld
    const c2 = 25n * E8;
    const m = maxRedeemable(c2, 10n * E8, PRICE);
    expect(m).toBeGreaterThanOrEqual(MIN_REDEEM);
    expect(c2 - redeemPayout(m, PRICE)).toBeGreaterThanOrEqual(DUST);
    expect(c2 - redeemPayout(m + 1n, PRICE)).toBeLessThan(DUST);
    expect(maxRedeemable(COLL, 10n * E8, PRICE)).toBe(10n * E8); // sonst die ganze Schuld
  });

  it("sweep: nur ohne Schuld und mit Zinsgebühr ≥ Sicherheit (Audit 11 A11-V-4)", () => {
    // 2 250 KAS à 0,04 USD = 90 USD Rest, 150 USD Zins → auflösbar
    const zombie = st(0n, 150n * E8, INDEX, 2_250n * E8);
    expect(interestFee(zombie, O)).toBe(zombie.collateral);
    expect(sweepable(zombie, O)).toBe(true);
    const r = simSweep(zombie, O);
    expect(r.ok && r.state).toBe(null);
    expect(r.ok && r.fee).toBe(2_250n * E8 - SWEEP_FEE);
    // Grenze: Zins genau = Sicherheit geht, 1 Einheit weniger Zins nicht
    const edge = st(0n, 90n * E8, INDEX, 2_250n * E8);
    expect(sweepable(edge, O)).toBe(true);
    expect(sweepable({ ...edge, interest: 90n * E8 - 4n }, O)).toBe(false); // ⌈…⌉ = coll − 1
    // mit Schuld nie, egal wie hoch der Zins
    expect(sweepable(st(1n, 150n * E8, INDEX, 2_250n * E8), O)).toBe(false);
    expect(simSweep(st(1n, 150n * E8, INDEX, 2_250n * E8), O).ok).toBe(false);
    // normaler Vault ohne Schuld: nur der Besitzer schließt
    expect(sweepable(st(0n, 2n * E8), O)).toBe(false);
    expect(simSweep(st(0n, 2n * E8), O).ok).toBe(false);
  });

  it("liquidate: erst unter 150 % (mit Zins), Liquidator bekommt Betrag + 10 %", () => {
    const s = st(100n * E8);
    expect(simLiquidate(s, O, LIQ, BONUS).ok).toBe(false); // 400 %: gesund
    const liqPrice = minHealthyPrice(COLL, 100n * E8, LIQ)!;
    expect(healthy(COLL, 100n * E8, liqPrice, LIQ)).toBe(true);
    expect(healthy(COLL, 100n * E8, liqPrice - 1n, LIQ)).toBe(false);
    const low = { kasUsd: liqPrice - 1n, stableIndex: INDEX };
    const r = simLiquidate(s, low, LIQ, BONUS);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const claim = refUp(100n * E8, 11_000n, 10_000n);
    expect(r.seized).toBe(refUp(claim, 100_000_000n, low.kasUsd));
    expect(r.state?.collateral).toBe(COLL - r.seized!);
    expect(r.state?.debt).toBe(0n);
  });

  it("liquidate: Zins macht liquidierbar", () => {
    const withI = st(150n * E8, 30n * E8);
    const p = minHealthyPrice(COLL, 180n * E8, LIQ)! - 1n;
    expect(healthy(COLL, 150n * E8, p, LIQ)).toBe(true); // ohne Zins gesund
    const o = { kasUsd: p, stableIndex: INDEX };
    expect(simLiquidate(withI, o, LIQ, BONUS).ok).toBe(true);
    expect(simLiquidate(st(150n * E8), o, LIQ, BONUS).ok).toBe(false);
  });

  it("Teil-Liquidation lässt den Zins stehen", () => {
    const s = st(150n * E8, 30n * E8, INDEX0);
    const acc = accrued(s, INDEX); // 30 + (150 + 30)·5 % = 39 USD
    expect(acc).toBe(3_900_000_000n);
    const p = minHealthyPrice(COLL, 150n * E8 + acc, LIQ)! - 1n;
    const o = { kasUsd: p, stableIndex: INDEX };
    const burn = 40n * E8;
    const r = simLiquidate(s, o, LIQ, BONUS, burn);
    expect(r.ok).toBe(true);
    if (!r.ok) return;
    const seize = refUp(refUp(burn, 11_000n, 10_000n), 100_000_000n, p);
    expect(r.state).toEqual(st(110n * E8, acc, INDEX, COLL - seize));
    const full = simLiquidate(s, o, LIQ, BONUS);
    expect(full.ok && full.state?.debt).toBe(0n);
    expect(full.ok && full.state?.interest).toBe(acc); // Zins bleibt bis zum Schließen
  });

  it("liquidate: Rest unter 0,2 KAS nur bei voller Tilgung", () => {
    const o = { kasUsd: 100_000_000n, stableIndex: INDEX }; // 1 USD/KAS
    const s = st(100n * E8, 0n, INDEX, 110n * E8 + DUST - 1n);
    const r = simLiquidate(s, o, LIQ, BONUS);
    expect(r.ok && r.state).toBe(null);
    expect(r.ok && r.seized).toBe(s.collateral);
    expect(r.ok && r.writtenOff).toBe(0n);
    // wie vault_tests.rs kleiner_rest_nur_bei_voller_tilgung
    const small = st(315_000n, 0n, INDEX, 10_000_000n);
    const o2 = { kasUsd: PRICE, stableIndex: INDEX };
    expect(simLiquidate(small, o2, LIQ, BONUS, 105_000n).ok).toBe(false);
    expect(simLiquidate(small, o2, LIQ, BONUS).ok).toBe(true);
    expect(liquidationPreview(small.collateral, PRICE, small.debt, 105_000n, BONUS).allowed).toBe(false);
    expect(liquidationPreview(small.collateral, PRICE, small.debt, small.debt, BONUS).allowed).toBe(true);
  });

  it("Unterdeckung: ganze Sicherheit an den Liquidator, Restschuld ausgebucht", () => {
    const o = { kasUsd: 100_000_000n, stableIndex: INDEX };
    const s = st(100n * E8, 0n, INDEX, 105n * E8); // 105 % < 110 %
    const r = simLiquidate(s, o, LIQ, BONUS, 96n * E8);
    expect(r.ok && r.state).toBe(null);
    expect(r.ok && r.seized).toBe(s.collateral);
    expect(r.ok && r.writtenOff).toBe(4n * E8);
  });

  it("liquidationPreview rechnet wie simLiquidate", () => {
    const o = { kasUsd: 100_000_000n, stableIndex: INDEX };
    const debt = 100n * E8;
    for (const [coll, burn] of [
      [140n * E8, 30n * E8],
      [140n * E8, debt],
      [105n * E8, 96n * E8],
      [110n * E8 + DUST - 1n, debt],
    ] as const) {
      const r = simLiquidate(st(debt, 0n, INDEX, coll), o, LIQ, BONUS, burn);
      const p = liquidationPreview(coll, o.kasUsd, debt, burn, BONUS);
      expect(r.ok).toBe(true);
      if (!r.ok) continue;
      expect(p.seize).toBe(r.seized);
      expect(p.ends).toBe(r.state === null);
      expect(p.writtenOff).toBe(r.writtenOff);
    }
  });
});

describe("Kennzahlen", () => {
  it("Quote und Gesundheitsfaktor auf Schuld + Zins", () => {
    // 300 KAS à 1 USD bei 100 GHOST → 300 %, HF 2,0; mit 50 USD Zins → 200 %
    expect(ratioBps(300n * E8, 100n * E8, 100_000_000n)).toBe(30_000n);
    expect(healthFactorE4(300n * E8, 100n * E8, 100_000_000n, LIQ)).toBe(20_000n);
    expect(ratioBps(300n * E8, 150n * E8, 100_000_000n)).toBe(20_000n);
    expect(ratioBps(300n * E8, 0n, 100_000_000n)).toBe(null);
  });

  it("Orakel-Index wie risk_oracle.sil update()", () => {
    const rate = rateFromAprBps(500n);
    expect(rate).toBe(158_548_959n); // Wert aus dem Orakel-Kommentar
    const year = 365n * 86_400n * 10n;
    const after = oracleIndexAfter(INDEX_SCALE, rate, year);
    const g = (rate * year) / 1_000_000_000n;
    expect(after).toBe(INDEX_SCALE + (INDEX_SCALE * g) / 1_000_000_000n);
    expect(after).toBe(1_049_999_999n); // knapp 5 % (Abrundung zugunsten der Schuldner)
  });

  it("Projektion: Schuld bleibt, Zins wächst; häufigere Updates verzinsen leicht stärker", () => {
    const rate = rateFromAprBps(500n);
    const s = st(100n * E8, 0n, INDEX_SCALE);
    const pts = projectInterest(s, INDEX_SCALE, rate, [0, 30, 365], 36_000n);
    expect(pts.map((p) => p.debt)).toEqual([100n * E8, 100n * E8, 100n * E8]);
    expect(pts[0].interest).toBe(0n);
    expect(pts[1].interest).toBeGreaterThan(0n);
    const [yearly] = projectInterest(s, INDEX_SCALE, rate, [365], 365n * 864_000n);
    const hourly = pts[2];
    expect(hourly.interest).toBeGreaterThan(yearly.interest);
    expect(hourly.interest).toBeLessThan((513n * E8) / 100n); // < e^0,05 − 1 ≈ 5,127 %
    expect(hourly.interest).toBe(accrual(100n * E8, INDEX_SCALE, hourly.index));
  });
});
