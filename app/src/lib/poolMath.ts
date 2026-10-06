// Tauschrechnung des GhostPool (contracts/ghost_pool.sil), exakt in bigint:
//   (x' − fee(x'−x)) · (y' − fee(y'−y)) ≥ x · y,  fee(d) = ⌈d · feeBps / 10 000⌉
// x = KAS-Reserve in sompi, y = GHOST-Reserve in Einheiten (je 1e8).

export const POOL_FEE_BPS = 30n; // 0,3 %
export const POOL_MIN_KAS = 100_000_000n; // 1 KAS bleibt immer im Pool
export const POOL_MAX_RESERVE = 10_000_000_000_000_000n; // Vertrag: MAX_KAS / MAX_GHOST

const ceilDiv = (a: bigint, b: bigint) => (a + b - 1n) / b;

export function feeOf(d: bigint, feeBps = POOL_FEE_BPS): bigint {
  return ceilDiv(d * feeBps, 10_000n);
}

/** Genau die Regel des Vertrags */
export function swapOk(x: bigint, y: bigint, x2: bigint, y2: bigint, feeBps = POOL_FEE_BPS): boolean {
  if (x2 < POOL_MIN_KAS || y2 <= 0n || x2 > POOL_MAX_RESERVE || y2 > POOL_MAX_RESERVE) return false;
  const xa = x2 > x ? x2 - feeOf(x2 - x, feeBps) : x2;
  const ya = y2 > y ? y2 - feeOf(y2 - y, feeBps) : y2;
  return xa * ya >= x * y;
}

function maxOut(ok: (m: bigint) => boolean, hi: bigint): bigint {
  let lo = 0n;
  while (lo < hi) {
    const m = (lo + hi + 1n) / 2n;
    if (ok(m)) lo = m;
    else hi = m - 1n;
  }
  return lo;
}

/** Größte GHOST-Menge für `dx` sompi */
export function ghostOut(x: bigint, y: bigint, dx: bigint, feeBps = POOL_FEE_BPS): bigint {
  if (dx <= 0n || y <= 1n) return 0n;
  return maxOut((m) => swapOk(x, y, x + dx, y - m, feeBps), y - 1n);
}

/** Größte KAS-Menge (sompi) für `dy` GHOST-Einheiten */
export function kasOut(x: bigint, y: bigint, dy: bigint, feeBps = POOL_FEE_BPS): bigint {
  if (dy <= 0n || x <= POOL_MIN_KAS) return 0n;
  return maxOut((m) => swapOk(x, y, x - m, y + dy, feeBps), x - POOL_MIN_KAS);
}

/** Mindestbetrag bei `slippageBps` Abweichung (abgerundet) */
export function withSlippage(out: bigint, slippageBps: bigint): bigint {
  return (out * (10_000n - slippageBps)) / 10_000n;
}

/**
 * Kursverschiebung durch den Tausch in bps: wie viel schlechter der
 * Durchschnittskurs ist als der Kurs vor dem Tausch (Gebühr eingerechnet).
 */
export function impactBps(reserveIn: bigint, reserveOut: bigint, amountIn: bigint, amountOut: bigint): bigint {
  if (amountIn <= 0n || reserveIn <= 0n) return 0n;
  // Spot: reserveOut/reserveIn; erhalten: amountOut/amountIn
  const spot = (amountIn * reserveOut) / reserveIn; // was man zum alten Kurs bekäme
  if (spot <= 0n) return 0n;
  return ((spot - amountOut) * 10_000n) / spot;
}

/** Neue Anteile für eine Einlage: min(⌊S·dx/x⌋, ⌊S·dy/y⌋) (Vertrag: add) */
export function sharesForDeposit(s: bigint, x: bigint, y: bigint, dx: bigint, dy: bigint): bigint {
  if (x <= 0n || y <= 0n || dx < 0n || dy < 0n) return 0n;
  const a = (s * dx) / x;
  const b = (s * dy) / y;
  return a < b ? a : b;
}

/** Auszahlung für m von S Anteilen: (⌊x·m/S⌋, ⌊y·m/S⌋) (Vertrag: remove) */
export function payoutFor(s: bigint, x: bigint, y: bigint, m: bigint): [bigint, bigint] {
  if (s <= 0n) return [0n, 0n];
  return [(x * m) / s, (y * m) / s];
}
