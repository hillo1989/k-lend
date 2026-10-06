import { useUsd } from "../lib/usd";

/** Dollarwert eines KAS- oder GHOST-Betrags zum aktuellen Kurs, klein hinter dem Betrag */
export function Usd({ amount, unit }: { amount: number | string | null | undefined; unit: string }) {
  const usd = useUsd();
  const t = usd(amount, unit);
  return t ? <span className="usd"> {t}</span> : null;
}
