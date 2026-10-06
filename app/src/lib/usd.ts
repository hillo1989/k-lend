// Aktueller Dollarwert von KAS- und GHOST-Beträgen für die Anzeige beim
// Senden und Empfangen. KAS: Marktpreis (Median aus 6 Quellen über
// ghostctl price), einmal je Minute für die ganze Seite. GHOST: Kurs im
// Tauschpool (KAS je GHOST × KAS-Marktpreis); ohne Pool das Ziel 1 USD.
import { useSyncExternalStore } from "react";
import { getLang } from "./i18n";
import { useStatus } from "./StatusContext";
import { de } from "./status";

let kasUsd: number | null = null;
let started = false;
const listeners = new Set<() => void>();

async function load() {
  try {
    const r = await fetch("./api/price", { cache: "no-store" });
    const j = (await r.json()) as { ok?: boolean; median?: number };
    if (j.ok && typeof j.median === "number" && j.median > 0 && j.median !== kasUsd) {
      kasUsd = j.median;
      listeners.forEach((l) => l());
    }
  } catch {
    /* ohne Preis weiter */
  }
}

function subscribe(l: () => void) {
  listeners.add(l);
  if (!started) {
    started = true;
    void load();
    window.setInterval(load, 60_000);
  }
  return () => listeners.delete(l);
}

/** KAS-Marktpreis in USD (null, solange keiner abrufbar ist) */
export function useKasUsd(): number | null {
  return useSyncExternalStore(
    subscribe,
    () => kasUsd,
    () => null,
  );
}

/** GHOST in USD aus den Pool-Reserven; ohne Pool oder Preis null */
export function ghostUsdFromPool(pool: { kasSompi: string; ghostUnits: string } | null | undefined, kas: number | null): number | null {
  if (!pool || kas === null) return null;
  const k = Number(pool.kasSompi);
  const g = Number(pool.ghostUnits);
  return k > 0 && g > 0 ? (k / g) * kas : null;
}

/** „≈ 1,23 USD“; kleine Beträge mit mehr Stellen */
export function usdText(usd: number): string {
  const abs = Math.abs(usd);
  const digits = abs >= 1 ? 2 : abs >= 0.01 ? 4 : 6;
  return getLang() === "de" ? `≈ ${de(usd, digits, 2)} USD` : `≈ ${usd.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: digits })} USD`;
}

/** Dollarwert eines Betrags in KAS oder GHOST zum aktuellen Kurs */
export function usdOf(amount: number, unit: string, kas: number | null, ghost: number | null): number | null {
  if (!Number.isFinite(amount)) return null;
  if (unit === "KAS") return kas === null ? null : amount * kas;
  if (unit === "GHOST") return amount * (ghost ?? 1);
  return null;
}

/**
 * Für Komponenten: Betrag (Zahl oder Text mit Punkt/Komma) → „≈ … USD“ oder
 * null. GHOST ohne Pool rechnet mit dem Ziel 1 USD.
 */
export function useUsd(): (amount: number | string | null | undefined, unit: string | null | undefined) => string | null {
  const kas = useKasUsd();
  const { status } = useStatus();
  const ghost = ghostUsdFromPool(status?.deployed ? status.pool : null, kas);
  return (amount, unit) => {
    if (amount === null || amount === undefined || !unit) return null;
    const n = typeof amount === "number" ? amount : Number(String(amount).replace(",", "."));
    const usd = usdOf(n, unit, kas, ghost);
    return usd === null ? null : usdText(usd);
  };
}
