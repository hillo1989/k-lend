import { useId, type ReactNode } from "react";
import { PARAMS } from "../config";
import { formatUnits, isAmbiguousAmount, parseUnits } from "../lib/format";
import { tr } from "../lib/i18n";

export function DemoTag({ children }: { children?: ReactNode }) {
  return <span className="tag tag-demo">{children ?? tr("Annahme", "Assumption")}</span>;
}

export function PlannedTag() {
  return <span className="tag tag-planned">{tr("geplant, Vertrag in Arbeit", "planned, contract in progress")}</span>;
}

export function Stat({ label, value, hint }: { label: ReactNode; value: ReactNode; hint?: ReactNode }) {
  return (
    <div className="stat">
      <div className="stat-label">{label}</div>
      <div className="stat-value">{value}</div>
      {hint && <div className="stat-hint">{hint}</div>}
    </div>
  );
}

export function AmountInput({
  label,
  value,
  onChange,
  suffix,
  hint,
  onMax,
  invalid,
  decimals = 8,
}: {
  label: string;
  value: string;
  onChange(v: string): void;
  suffix: string;
  hint?: ReactNode;
  onMax?: () => void;
  invalid?: boolean;
  /** Nachkommastellen für die Anzeige des gelesenen Werts */
  decimals?: number;
}) {
  const id = useId();
  const hintId = `${id}-hint`;
  // Gelesenen Wert zeigen, sobald ein Trennzeichen im Spiel ist (A10-W-1)
  const raw = value.trim();
  const parsed = /[.,]/.test(raw) ? parseUnits(raw, decimals) : null;
  const readBack = !/[.,]/.test(raw)
    ? null
    : isAmbiguousAmount(raw)
      ? (() => {
          const [w, f] = raw.split(/[.,]/);
          const small = `${w}${tr(",", ".")}${f.replace(/0+$/, "") || "0"}`.replace(/[.,]0$/, "");
          const big = `${w}${f}`;
          return tr(
            `Mehrdeutig: „${raw}“ kann ${small} oder ${big} heißen. Bitte eindeutig schreiben, also ${small} oder ${big}.`,
            `Ambiguous: “${raw}” could mean ${small} or ${big}. Please write ${small} or ${big}.`,
          );
        })()
      : parsed !== null
        ? `= ${formatUnits(parsed, decimals, decimals)} ${suffix}`
        : tr("Keine gültige Zahl.", "Not a valid number.");
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <div className={invalid ? "input-wrap invalid" : "input-wrap"}>
        <input
          id={id}
          inputMode="decimal"
          autoComplete="off"
          spellCheck={false}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          aria-invalid={invalid || undefined}
          aria-describedby={hint ? hintId : undefined}
        />
        <span className="suffix" aria-hidden="true">
          {suffix}
        </span>
        {onMax && (
          <button type="button" className="btn-max" onClick={onMax}>
            Max<span className="sr-only"> {tr("für", "for")} {label}</span>
          </button>
        )}
      </div>
      {readBack && (
        <div className={parsed === null ? "field-hint field-hint-warn" : "field-hint"} aria-live="polite">
          {readBack}
        </div>
      )}
      {hint && (
        <div id={hintId} className="field-hint">
          {hint}
        </div>
      )}
    </div>
  );
}

/**
 * Balken für die Besicherungsquote (100 % … 400 %) mit Marken bei der
 * Liquidationsschwelle und der Mindestquote. Zusätzlich der Gesundheitsfaktor.
 */
export function HealthBar({ ratioBps, hfE4 }: { ratioBps: bigint | null; hfE4: bigint | null }) {
  const min = 10_000;
  const max = 40_000;
  const liq = Number(PARAMS.liqBps);
  const mcr = Number(PARAMS.mcrBps);
  const pos = (bps: number) => `${((Math.min(Math.max(bps, min), max) - min) / (max - min)) * 100}%`;
  const r = ratioBps === null ? null : Number(ratioBps > 1_000_000n ? 1_000_000n : ratioBps);
  const zone = r === null ? "none" : r < liq ? "danger" : r < mcr ? "warn" : r < mcr * 1.25 ? "caution" : "ok";
  const zoneText: Record<string, string> = {
    none: tr("Keine Schuld – nichts zu liquidieren.", "No debt – nothing to liquidate."),
    danger: tr("Liquidierbar: Die Quote liegt unter 150 %.", "Liquidatable: the ratio is below 150 %."),
    warn: tr("Unter der Mindestquote: Prägen und Abheben gesperrt, Liquidation droht.", "Below the minimum ratio: minting and withdrawing blocked, liquidation looms."),
    caution: tr("Knapp über der Mindestquote – wenig Puffer bei Kursrückgang.", "Just above the minimum ratio – little buffer if the price drops."),
    ok: tr("Komfortabler Puffer.", "Comfortable buffer."),
  };
  const hfText = hfE4 === null ? "∞" : formatUnits(hfE4 > 990_000n ? 990_000n : hfE4, 4, 2, 2);
  return (
    <div className={`health health-${zone}`}>
      <div className="health-head">
        <span>
          {tr("Gesundheitsfaktor", "Health factor")} <strong className="health-hf">{hfText}</strong>
        </span>
        <span className="muted small">{tr("Quote", "Ratio")} {r === null ? "–" : `${formatUnits(BigInt(r), 2, 1)} %`}</span>
      </div>
      <div
        className="health-track"
        role="meter"
        aria-label={tr("Besicherungsquote", "Collateral ratio")}
        aria-valuemin={100}
        aria-valuemax={400}
        aria-valuenow={r === null ? 400 : Math.min(400, Math.max(100, r / 100))}
        aria-valuetext={r === null ? tr("keine Schuld", "no debt") : `${formatUnits(BigInt(r), 2, 1)} %. ${zoneText[zone]}`}
      >
        <div className="health-fill" style={{ width: r === null ? "100%" : pos(r) }} />
        <div className="health-mark" style={{ left: pos(liq) }}>
          <span>150 %</span>
        </div>
        <div className="health-mark mark-mcr" style={{ left: pos(mcr) }}>
          <span>200 %</span>
        </div>
      </div>
      <p className="health-text small">{zoneText[zone]}</p>
    </div>
  );
}

export function Callout({ kind = "info", title, children }: { kind?: "info" | "warn" | "danger"; title?: string; children: ReactNode }) {
  return (
    <div className={`callout callout-${kind}`} role={kind === "danger" ? "alert" : undefined}>
      {title && <strong className="callout-title">{title}</strong>}
      <div>{children}</div>
    </div>
  );
}
