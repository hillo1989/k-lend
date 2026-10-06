import { useMemo, useState } from "react";
import { NATIVE, POOL_ASSUMPTIONS, SCENARIO_FALLBACK, STABLE_SYMBOL } from "../config";
import { useStatus } from "../lib/StatusContext";
import { kasUsdToUnits } from "../lib/status";
import { AmountInput, Callout, DemoTag, PlannedTag, Stat } from "../components/ui";
import { fmtKas, fmtNum, formatUnits, parseUnits } from "../lib/format";
import { demoPool, pct, poolRates } from "../lib/demo";
import { tr } from "../lib/i18n";

/** Annahme für die Planung: bis zu 60 % des GHOST-Werts in KAS leihbar. */
const ASSUMED_LTV = 60;

function RateCurve() {
  // Zinskurve des angenommenen Modells, 0–100 % Auslastung
  const pts: string[] = [];
  for (let i = 0; i <= 100; i += 2) {
    const r = poolRates(100, i).borrowApr;
    pts.push(`${20 + i * 3.4},${150 - Math.min(r, 64) * 2}`);
  }
  const u = demoPool.utilization * 100;
  const cx = 20 + u * 3.4;
  const cy = 150 - demoPool.borrowApr * 2;
  return (
    <svg
      className="curve"
      viewBox="0 0 380 180"
      role="img"
      aria-label={tr(
        `Kreditzins nach Auslastung. Aktuell ${pct(u, 1)} Auslastung, ${pct(demoPool.borrowApr)} Kreditzins.`,
        `Borrow rate by utilization. Currently ${pct(u, 1)} utilization, ${pct(demoPool.borrowApr)} borrow rate.`,
      )}
    >
      <line x1="20" y1="150" x2="360" y2="150" className="axis" />
      <line x1="20" y1="20" x2="20" y2="150" className="axis" />
      <line x1={20 + POOL_ASSUMPTIONS.kinkPct * 3.4} y1="20" x2={20 + POOL_ASSUMPTIONS.kinkPct * 3.4} y2="150" className="axis dashed" />
      <text x={20 + POOL_ASSUMPTIONS.kinkPct * 3.4} y="14" className="axis-label" textAnchor="middle">
        {tr("Knick", "Kink")} {POOL_ASSUMPTIONS.kinkPct} %
      </text>
      <polyline points={pts.join(" ")} className="curve-line" />
      <circle cx={cx} cy={cy} r="5" className="curve-dot" />
      <text x="20" y="170" className="axis-label">0 %</text>
      <text x="360" y="170" className="axis-label" textAnchor="end">
        100 % {tr("Auslastung", "utilization")}
      </text>
    </svg>
  );
}

export function Pool() {
  const [supply, setSupply] = useState("10.000");
  const [coll, setColl] = useState("500");
  const [msg, setMsg] = useState<string | null>(null);
  const { status } = useStatus();
  const kasUsd = status?.deployed ? kasUsdToUnits(status.oracle.kasUsd) : parseUnits(SCENARIO_FALLBACK.kasUsd, 8)!;

  const supplySompi = parseUnits(supply, 8);
  const collUnits = parseUnits(coll, 8);
  const yearly = useMemo(() => {
    if (supplySompi === null) return null;
    return (supplySompi * BigInt(Math.round(demoPool.supplyApr * 100))) / 10_000n;
  }, [supplySompi]);
  const maxBorrow = useMemo(() => {
    if (collUnits === null) return null;
    // GHOST (1 USD) → KAS: Wert × LTV / Preis
    return (collUnits * BigInt(ASSUMED_LTV) * 100_000_000n) / (100n * kasUsd);
  }, [collUnits, kasUsd]);

  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          {NATIVE}-Pool
        </h1>
        <PlannedTag />
      </div>
      <Callout kind="info" title={tr("Geplant, Vertrag in Arbeit", "Planned, contract in progress")}>
        {tr(
          "Den LendingPool gibt es bisher nur als Entwurf. Zinsmodell, Beleihungsgrenze und alle Zahlen auf dieser Seite sind Annahmen zur Veranschaulichung. Geplant ist, dass Einlagen und Kredite später gebündelt über Keeper laufen. Solche bündelnden Keeper gibt es noch nicht.",
          "The lending pool exists only as a draft so far. Rate model, loan-to-value limit and all figures on this page are assumptions for illustration. The plan is for deposits and loans to later run bundled through keepers. Such bundling keepers don't exist yet.",
        )}
      </Callout>

      <div className="stats-grid section-sm">
        <Stat label={tr("Einlagezins p. a.", "Supply rate p.a.")} value={pct(demoPool.supplyApr)} hint={<DemoTag>{tr("Annahme", "Assumption")}</DemoTag>} />
        <Stat label={tr("Kreditzins p. a.", "Borrow rate p.a.")} value={pct(demoPool.borrowApr)} hint={<DemoTag>{tr("Annahme", "Assumption")}</DemoTag>} />
        <Stat label={tr("Auslastung", "Utilization")} value={pct(demoPool.utilization * 100, 1)} hint={<DemoTag>{tr("Annahme", "Assumption")}</DemoTag>} />
        <Stat label={tr("Eingezahlt", "Supplied")} value={`${fmtNum(POOL_ASSUMPTIONS.suppliedKas)} ${NATIVE}`} hint={<DemoTag>{tr("Annahme", "Assumption")}</DemoTag>} />
      </div>

      <div className="grid-2">
        <section className="card" aria-labelledby="einlage">
          <div className="card-head">
            <h2 id="einlage">{tr(`${NATIVE} verleihen`, `Lend ${NATIVE}`)}</h2>
          </div>
          <AmountInput label={tr("Einlage", "Deposit")} value={supply} onChange={setSupply} suffix={NATIVE} invalid={supplySompi === null} />
          <dl className="kv">
            <div>
              <dt>{tr("Zinsertrag in 1 Jahr (bei gleichem Satz)", "Interest earned in 1 year (at the same rate)")}</dt>
              <dd>{yearly === null ? "–" : fmtKas(yearly)}</dd>
            </div>
            <div>
              <dt>{tr("Du erhältst", "You receive")}</dt>
              <dd>{tr("Anteils-Token, deren Gegenwert mit dem Pool-Index steigt", "share tokens whose value rises with the pool index")}</dd>
            </div>
          </dl>
          <button
            className="btn btn-primary"
            onClick={() =>
              setMsg(
                tr(
                  `Pool-Einlage simuliert: ${supply} ${NATIVE}. Nur simuliert – der Pool-Vertrag existiert noch nicht.`,
                  `Pool deposit simulated: ${supply} ${NATIVE}. Simulated only – the pool contract doesn't exist yet.`,
                ),
              )
            }
          >
            {tr("Pool-Einlage (Simulation)", "Pool deposit (simulation)")}
          </button>
        </section>

        <section className="card" aria-labelledby="kredit">
          <div className="card-head">
            <h2 id="kredit">{tr(`${NATIVE} leihen gegen ${STABLE_SYMBOL}`, `Borrow ${NATIVE} against ${STABLE_SYMBOL}`)}</h2>
          </div>
          <AmountInput label={tr(`${STABLE_SYMBOL} als Sicherheit`, `${STABLE_SYMBOL} as collateral`)} value={coll} onChange={setColl} suffix={STABLE_SYMBOL} invalid={collUnits === null} />
          <dl className="kv">
            <div>
              <dt>{tr(`Höchstens leihbar (Annahme ${ASSUMED_LTV} %)`, `Maximum borrowable (assumed ${ASSUMED_LTV} %)`)}</dt>
              <dd>{maxBorrow === null ? "–" : fmtKas(maxBorrow)}</dd>
            </div>
            <div>
              <dt>{tr("Kreditzins p. a.", "Borrow rate p.a.")}</dt>
              <dd>{pct(demoPool.borrowApr)}</dd>
            </div>
            <div>
              <dt>{tr("Risiko", "Risk")}</dt>
              <dd>{tr("Steigt der KAS-Kurs, wächst deine Schuld in USD – dann droht die Liquidation.", "If the KAS price rises, your debt in USD grows – then liquidation threatens.")}</dd>
            </div>
          </dl>
          <button
            className="btn btn-primary"
            onClick={() =>
              setMsg(
                tr(
                  `Kredit simuliert: ${maxBorrow === null ? "–" : formatUnits(maxBorrow, 8, 2)} ${NATIVE} gegen ${coll} ${STABLE_SYMBOL}. Nur simuliert – der Pool-Vertrag existiert noch nicht.`,
                  `Loan simulated: ${maxBorrow === null ? "–" : formatUnits(maxBorrow, 8, 2)} ${NATIVE} against ${coll} ${STABLE_SYMBOL}. Simulated only – the pool contract doesn't exist yet.`,
                ),
              )
            }
          >
            {tr("Kredit aufnehmen (Simulation)", "Take out a loan (simulation)")}
          </button>
        </section>
      </div>

      <div aria-live="polite" className="live">
        {msg && (
          <Callout kind="info" title={tr("Simulation", "Simulation")}>
            {msg}
          </Callout>
        )}
      </div>

      <section className="card section-sm" aria-labelledby="zinsmodell">
        <div className="card-head">
          <h2 id="zinsmodell">{tr("Zinsmodell (Annahme)", "Rate model (assumption)")}</h2>
          <DemoTag>{tr("Annahme", "Assumption")}</DemoTag>
        </div>
        <p>
          {tr(
            `Je mehr ${NATIVE} verliehen sind, desto höher der Zins. Ab ${POOL_ASSUMPTIONS.kinkPct} % Auslastung steigt er steil, damit immer genug ${NATIVE} für Auszahlungen im Pool bleiben. Einleger bekommen den Kreditzins anteilig nach Auslastung, abzüglich ${POOL_ASSUMPTIONS.reserveFactorPct} % Reserve.`,
            `The more ${NATIVE} is lent out, the higher the rate. From ${POOL_ASSUMPTIONS.kinkPct} % utilization it rises steeply, so enough ${NATIVE} always remains in the pool for withdrawals. Liquidity providers receive the borrow rate proportional to utilization, minus ${POOL_ASSUMPTIONS.reserveFactorPct} % reserve.`,
          )}
        </p>
        <RateCurve />
      </section>
    </div>
  );
}
