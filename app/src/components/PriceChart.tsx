import { useEffect, useMemo, useState } from "react";
import { getLang, tr } from "../lib/i18n";
import { de } from "../lib/status";

type Days = 7 | 30 | 365;
type Point = [number, number];

const RANGES: { days: Days; label: () => string }[] = [
  { days: 7, label: () => tr("7 Tage", "7 days") },
  { days: 30, label: () => tr("30 Tage", "30 days") },
  { days: 365, label: () => tr("1 Jahr", "1 year") },
];

// Zeichenfläche (viewBox); die Beschriftung liegt als HTML außerhalb, damit sie nicht mitskaliert
const W = 600;
const H = 200;
const PAD = 6;

interface Loaded {
  days: Days;
  points: Point[];
  source: string;
}

/** Punkte → SVG-Koordinaten, Minimum/Maximum mit etwas Luft */
export function scale(points: Point[]) {
  const t0 = points[0][0];
  const t1 = points[points.length - 1][0];
  const prices = points.map((p) => p[1]);
  const lo = Math.min(...prices);
  const hi = Math.max(...prices);
  const span = hi - lo || hi || 1;
  const x = (t: number) => (t1 === t0 ? W / 2 : ((t - t0) / (t1 - t0)) * W);
  const y = (p: number) => PAD + (1 - (p - (lo - span * 0.05)) / (span * 1.1)) * (H - 2 * PAD);
  return { x, y, lo, hi };
}

function fmtDate(t: number, days: Days) {
  const locale = getLang() === "de" ? "de-DE" : "en-GB";
  const d = new Date(t);
  return days === 7
    ? d.toLocaleString(locale, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(locale, { day: "numeric", month: "short", year: days === 365 ? "numeric" : undefined });
}

/** KAS-Kurs in USD über 7 Tage, 30 Tage oder 1 Jahr (Quelle über /api/history) */
export function PriceChart() {
  const [days, setDays] = useState<Days>(30);
  const [data, setData] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [hover, setHover] = useState<number | null>(null);

  useEffect(() => {
    const ctl = new AbortController();
    setError(null);
    setHover(null);
    fetch(`./api/history?days=${days}`, { signal: ctl.signal, cache: "no-store" })
      .then((r) => r.json())
      .then((j: { ok: boolean; points?: Point[]; source?: string; error?: string }) => {
        if (j.ok && j.points && j.points.length >= 2) setData({ days, points: j.points, source: j.source ?? "" });
        else setError(j.error ?? tr("Kursverlauf nicht verfügbar.", "Price history unavailable."));
      })
      .catch((e: Error) => {
        if (e.name !== "AbortError") setError(tr("Kursverlauf nicht abrufbar.", "Could not load price history."));
      });
    return () => ctl.abort();
  }, [days]);

  const shown = data && data.days === days ? data : null;
  const geo = useMemo(() => (shown ? scale(shown.points) : null), [shown]);
  const path = useMemo(() => {
    if (!shown || !geo) return null;
    const line = shown.points.map(([t, p], i) => `${i ? "L" : "M"}${geo.x(t).toFixed(1)},${geo.y(p).toFixed(1)}`).join("");
    return { line, area: `${line}L${W},${H}L0,${H}Z` };
  }, [shown, geo]);

  const first = shown?.points[0][1] ?? null;
  const last = shown?.points[shown.points.length - 1][1] ?? null;
  const change = first !== null && last !== null ? ((last - first) / first) * 100 : null;
  const hp = shown && hover !== null ? shown.points[hover] : null;

  const onMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!shown || !geo) return;
    const r = e.currentTarget.getBoundingClientRect();
    const fx = ((e.clientX - r.left) / r.width) * W;
    let best = 0;
    for (let i = 1; i < shown.points.length; i++) {
      if (Math.abs(geo.x(shown.points[i][0]) - fx) < Math.abs(geo.x(shown.points[best][0]) - fx)) best = i;
    }
    setHover(best);
  };

  return (
    <section className="card section-sm" aria-labelledby="kas-verlauf">
      <div className="card-head">
        <h2 id="kas-verlauf">{tr("KAS-Kurs", "KAS price")}</h2>
        <div className="tabs chart-tabs" role="group" aria-label={tr("Zeitraum", "Period")}>
          {RANGES.map((r) => (
            <button key={r.days} type="button" className={days === r.days ? "tab active" : "tab"} aria-pressed={days === r.days} onClick={() => setDays(r.days)}>
              {r.label()}
            </button>
          ))}
        </div>
      </div>

      <div className="chart-summary" aria-live="polite">
        <span className="chart-price">{hp ? `${de(hp[1], 5, 4)} USD` : last !== null ? `${de(last, 5, 4)} USD` : "–"}</span>
        {hp ? (
          <span className="muted small">{fmtDate(hp[0], days)}</span>
        ) : (
          change !== null && (
            <span className={change >= 0 ? "chart-up" : "chart-down"}>
              {change >= 0 ? "+" : ""}
              {de(change, 1)} % {tr("im Zeitraum", "in period")}
            </span>
          )
        )}
      </div>

      <div className="chart-box">
        {/* Punkt als HTML, damit er bei gestreckter Zeichenfläche rund bleibt */}
        {hp && geo && <span className="chart-dot" style={{ left: `${(geo.x(hp[0]) / W) * 100}%`, top: `${(geo.y(hp[1]) / H) * 100}%` }} aria-hidden="true" />}
        {path && geo ? (
          <svg
            className="chart"
            viewBox={`0 0 ${W} ${H}`}
            preserveAspectRatio="none"
            role="img"
            aria-label={tr(
              `KAS-Kurs: ${de(first!, 5, 4)} bis ${de(last!, 5, 4)} USD, Tief ${de(geo.lo, 5, 4)}, Hoch ${de(geo.hi, 5, 4)}`,
              `KAS price: ${de(first!, 5, 4)} to ${de(last!, 5, 4)} USD, low ${de(geo.lo, 5, 4)}, high ${de(geo.hi, 5, 4)}`,
            )}
            onPointerMove={onMove}
            onPointerLeave={() => setHover(null)}
          >
            <defs>
              <linearGradient id="chart-fill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.28" />
                <stop offset="100%" stopColor="var(--accent)" stopOpacity="0" />
              </linearGradient>
            </defs>
            <path d={path.area} fill="url(#chart-fill)" />
            <path d={path.line} fill="none" stroke="var(--accent)" strokeWidth="2" vectorEffect="non-scaling-stroke" />
            {hp && (
              <>
                <line x1={geo.x(hp[0])} x2={geo.x(hp[0])} y1={0} y2={H} stroke="var(--muted)" strokeWidth="1" strokeDasharray="3 3" vectorEffect="non-scaling-stroke" />
              </>
            )}
          </svg>
        ) : (
          <p className="muted chart-empty">{error ?? tr("Lädt …", "Loading …")}</p>
        )}
      </div>

      {shown && geo && (
        <div className="chart-foot muted small">
          <span>{fmtDate(shown.points[0][0], days)}</span>
          <span>
            {tr("Tief", "Low")} {de(geo.lo, 5, 4)} · {tr("Hoch", "High")} {de(geo.hi, 5, 4)} USD · {tr("Quelle", "Source")} {shown.source}
          </span>
          <span>{fmtDate(shown.points[shown.points.length - 1][0], days)}</span>
        </div>
      )}
    </section>
  );
}
