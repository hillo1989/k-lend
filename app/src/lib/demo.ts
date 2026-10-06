// Abgeleitete Demo-Größen. Der Pool-Vertrag existiert noch nicht; das
// Zinsmodell hier ist eine Annahme nach Aave-Art (Knickpunkt bei hoher Auslastung).
import { POOL_ASSUMPTIONS } from "../config";
import { locale } from "./i18n";

export interface PoolRates {
  utilization: number; // 0…1
  borrowApr: number; // in %
  supplyApr: number; // in %
}

export function poolRates(supplied: number, borrowed: number, p = POOL_ASSUMPTIONS): PoolRates {
  const u = supplied > 0 ? Math.min(1, borrowed / supplied) : 0;
  const k = p.kinkPct / 100;
  let borrowApr = p.baseRatePct + p.slope1Pct * (Math.min(u, k) / k);
  if (u > k) borrowApr += p.slope2Pct * ((u - k) / (1 - k));
  const supplyApr = borrowApr * u * (1 - p.reserveFactorPct / 100);
  return { utilization: u, borrowApr, supplyApr };
}

export const demoPool = poolRates(POOL_ASSUMPTIONS.suppliedKas, POOL_ASSUMPTIONS.borrowedKas);

/** Deutsche Prozentangabe */
export const pct = (x: number, frac = 2) =>
  `${x.toLocaleString(locale(), { minimumFractionDigits: frac, maximumFractionDigits: frac })} %`;
