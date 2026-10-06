import { useEffect, useMemo, useRef, useState } from "react";
import { ASSUMED_ORACLE_INTERVAL_DAA, NATIVE, NETWORKS, PARAMS, SCENARIO_FALLBACK, STABLE_SYMBOL } from "../config";
import { ActionForms, type Prefill } from "../components/ActionForms";
import type { CliAction } from "../lib/commands";
import { StatusNotices } from "../components/Network";
import { VaultList } from "../components/VaultList";
import { useStatus } from "../lib/StatusContext";
import { vaultSweepable } from "../lib/precheck";
import { de, indexToUnits, kasUsdToUnits, pctToBps } from "../lib/status";
import { AmountInput, Callout, HealthBar } from "../components/ui";
import { fmtBps, fmtKas, fmtStable, fmtUsdPrice, formatUnits, parseUnits } from "../lib/format";
import {
  accrued,
  collateralValue,
  healthFactorE4,
  maxMintable,
  minHealthyPrice,
  projectInterest,
  rateFromAprBps,
  ratioBps,
  simMint,
  type EntryResult,
  type OracleView,
  type VaultState,
} from "../lib/vaultMath";
import { useWallet } from "../wallet/WalletContext";
import { tr } from "../lib/i18n";

/** Aktionen am eigenen Vault (Liquidieren steht separat: es betrifft fremde Vaults) */
const OWN_ACTIONS: CliAction[] = ["mint", "repay", "deposit", "withdraw", "close", "open-vault"];

const PROJECTION_DAYS = [0, 30, 365, 3 * 365];

interface Metrics {
  value: bigint;
  debt: bigint;
  /** offener Zins bis zum Szenario-Index (USD × 1e8) */
  interest: bigint;
  ratio: bigint | null;
  hf: bigint | null;
  liqPrice: bigint | null;
  maxMint: bigint;
}

/** cap = Höchstschuld je Vault in Einheiten (null = unbegrenzt; gilt nur für die Schuld) */
function metrics(v: VaultState, o: OracleView, cap: bigint | null = null): Metrics | null {
  try {
    const interest = accrued(v, o.stableIndex);
    const owed = v.debt + interest; // für die Quoten zählen Schuld und Zins
    return {
      value: collateralValue(v.collateral, o.kasUsd),
      debt: v.debt,
      interest,
      ratio: ratioBps(v.collateral, owed, o.kasUsd),
      hf: healthFactorE4(v.collateral, owed, o.kasUsd, PARAMS.liqBps),
      liqPrice: minHealthyPrice(v.collateral, owed, PARAMS.liqBps),
      maxMint: maxMintable(v, o, PARAMS.mcrBps, cap),
    };
  } catch {
    return null; // Überlauf außerhalb der Vertragsgrenzen
  }
}

export function Vault() {
  const wallet = useWallet();
  const { status, network } = useStatus();
  const live = status?.deployed ? status : null;
  const capGhost = live?.params.maxDebtGhost ?? null;
  const cap = capGhost !== null ? BigInt(Math.round(capGhost * 1e8)) : null;
  // Formular „Auflösen“ nur, wenn es einen auflösbaren Vault gibt (Audit 11 A11-V-4)
  const anySweepable = live !== null && live.vaults.some((v) => vaultSweepable(v, live));

  // --- Szenario: startet mit den Live-Werten des Orakels, bleibt verstellbar
  const liveVals = useMemo(() => {
    if (!live) return null;
    return {
      price: formatUnits(kasUsdToUnits(live.oracle.kasUsd), 8, 8),
      apr: formatUnits(pctToBps(live.oracle.ratePctYear), 2, 2, 2),
      index: indexToUnits(live.oracle.index),
    };
  }, [live?.oracle.kasUsd, live?.oracle.ratePctYear, live?.oracle.index]); // eslint-disable-line react-hooks/exhaustive-deps
  const [priceStr, setPriceStr] = useState<string>(SCENARIO_FALLBACK.kasUsd);
  const [aprStr, setAprStr] = useState<string>(SCENARIO_FALLBACK.stableAprPercent);
  const [scenarioIndex, setScenarioIndex] = useState<bigint>(SCENARIO_FALLBACK.stableIndex);
  const touched = useRef(false);
  const applyLive = () => {
    if (!liveVals) return;
    setPriceStr(liveVals.price);
    setAprStr(liveVals.apr);
    setScenarioIndex(liveVals.index);
    touched.current = false;
  };
  useEffect(() => {
    if (liveVals && !touched.current) applyLive();
  }, [liveVals]); // eslint-disable-line react-hooks/exhaustive-deps
  const editPrice = (v: string) => {
    touched.current = true;
    setPriceStr(v);
  };
  const editApr = (v: string) => {
    touched.current = true;
    setAprStr(v);
  };
  const kasUsd = parseUnits(priceStr, 8);
  const aprBps = parseUnits(aprStr, 2);
  const oracle: OracleView | null = kasUsd && kasUsd > 0n ? { kasUsd, stableIndex: scenarioIndex } : null;
  const stableRate = aprBps !== null ? rateFromAprBps(aprBps) : null;
  const shiftPrice = (pct: bigint) => {
    if (!kasUsd) return;
    editPrice(formatUnits((kasUsd * (100n + pct)) / 100n, 8, 6));
  };

  // --- Rechner
  const [collStr, setCollStr] = useState("150");
  const [mintStr, setMintStr] = useState("1");
  const coll = parseUnits(collStr, 8);
  const mint = parseUnits(mintStr, 8);

  const plan = useMemo(() => {
    if (!oracle || coll === null || mint === null) return null;
    // neuer Vault: noch kein Zins, abgerechnet bis zum aktuellen Index
    const empty: VaultState = { collateral: coll, debt: 0n, interest: 0n, indexAt: oracle.stableIndex };
    const check = mint > 0n ? simMint(empty, mint, oracle, PARAMS.mcrBps, cap) : ({ ok: true, state: empty } as EntryResult);
    const state: VaultState = { ...empty, debt: mint };
    return { check, state, m: metrics(state, oracle, cap), maxMint: metrics(empty, oracle, cap)?.maxMint ?? 0n };
  }, [oracle?.kasUsd, oracle?.stableIndex, coll, mint, cap]); // eslint-disable-line react-hooks/exhaustive-deps

  // Satz der Projektion: der aktuelle Satz des Orakels, solange er im Szenario nicht verstellt ist
  const liveRate = liveVals !== null && aprStr === liveVals.apr;
  const projection = useMemo(() => {
    if (!plan || !oracle || stableRate === null || plan.state.debt === 0n) return null;
    try {
      return projectInterest(plan.state, oracle.stableIndex, stableRate, PROJECTION_DAYS, ASSUMED_ORACLE_INTERVAL_DAA);
    } catch {
      return null;
    }
  }, [plan, oracle?.stableIndex, stableRate]); // eslint-disable-line react-hooks/exhaustive-deps

  // --- Live-Aktionen
  const [prefill, setPrefill] = useState<Prefill | null>(null);
  const openAction = (action: CliAction, vault?: number) => setPrefill({ action, vault, nonce: Date.now() });


  const overBalance = wallet.balance !== null && coll !== null && coll > wallet.balance;

  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          Vault
        </h1>
        <span className="tag">{NETWORKS[network].label}</span>
      </div>
      <StatusNotices />

      <ActionForms
        prefill={prefill && prefill.action !== "liquidate" && prefill.action !== "redeem" && prefill.action !== "sweep" ? prefill : null}
        actions={OWN_ACTIONS}
        title={tr("Eigener Vault", "Your vault")}
        id="eigener-vault"
        ownVaults
      />
      <VaultList onAction={openAction} />
      <ActionForms
        prefill={prefill && prefill.action === "liquidate" ? prefill : null}
        actions={["liquidate"]}
        title={tr("Fremde Vaults liquidieren", "Liquidate other vaults")}
        id="liquidieren"
      />
      <ActionForms
        prefill={prefill && prefill.action === "redeem" ? prefill : null}
        actions={["redeem"]}
        title={tr("Rücknahme zu 1 USD", "Redemption at 1 USD")}
        id="ruecknahme"
      />
      {(prefill?.action === "sweep" || anySweepable) && (
        <ActionForms
          prefill={prefill && prefill.action === "sweep" ? prefill : null}
          actions={["sweep"]}
          title={tr("Vault zugunsten der Zinsadresse auflösen", "Dissolve a vault for the interest address")}
          id="aufloesen"
        />
      )}

      <section className="card section-sm" aria-labelledby="rechner">
        <div className="card-head">
          <h2 id="rechner">{tr("Vault-Rechner", "Vault calculator")}</h2>
          <span className="tag tag-demo">{liveVals ? tr("Start: Live-Orakel", "start: live oracle") : tr("ohne Live-Daten", "no live data")}</span>
        </div>
        <p className="muted small">{tr("Rechnet mit denselben Rundungsregeln wie der Vertrag.", "Uses the same rounding rules as the contract.")}</p>
        <div className="calc-scenario">
        <AmountInput
          label={tr("KAS-Preis", "KAS price")}
          value={priceStr}
          onChange={editPrice}
          suffix="USD"
          invalid={!oracle}
          hint={tr(
            "Startet mit dem Live-Orakelpreis, zum Durchspielen verstellbar. Der Vertrag akzeptiert höchstens rund 920 USD je KAS.",
            "Starts with the live oracle price, adjustable for what-if scenarios. The contract accepts at most about 920 USD per KAS.",
          )}
        />
        <div className="btn-row tight" role="group" aria-label={tr("Preis schnell ändern", "Quick price change")}>
          <button className="btn btn-ghost btn-sm" onClick={() => shiftPrice(-30n)}>
            −30 %
          </button>
          <button className="btn btn-ghost btn-sm" onClick={() => shiftPrice(-10n)}>
            −10 %
          </button>
          <button className="btn btn-ghost btn-sm" onClick={() => shiftPrice(10n)}>
            +10 %
          </button>
          <button className="btn btn-ghost btn-sm" onClick={applyLive} disabled={!liveVals}>
            {tr("Live-Werte", "Live values")}
          </button>
        </div>
        <AmountInput
          label={tr(`${STABLE_SYMBOL}-Zins p. a.`, `${STABLE_SYMBOL} interest p.a.`)}
          value={aprStr}
          onChange={editApr}
          suffix="%"
          decimals={2}
          invalid={aprBps === null}
          hint={tr(
            "Startet mit dem aktuellen Satz des Orakels. Der GHOST-Agent passt ihn höchstens stündlich an, nach dem Median seiner Kursmessungen der letzten Stunde: unter 0,995 USD je GHOST +0,5 Prozentpunkte, über 1,005 USD −0,5, zwischen 2 % (Grundzins) und 20 %.",
            "Starts with the oracle's current rate. The GHOST agent adjusts it at most hourly, by the median of its price measurements over the last hour: below 0.995 USD per GHOST +0.5 percentage points, above 1.005 USD −0.5, between 2 % (base rate) and 20 %.",
          )}
        />
        <p className="muted small">
          {tr("Zinsindex", "Interest index")} {formatUnits(scenarioIndex, 9, 9)} ·{" "}
          {tr("für die Projektion: 1 Orakel-Update je Stunde angenommen", "projection assumes 1 oracle update per hour")}
        </p>
        </div>
        <div className="calc">
          <div className="calc-inputs">
            <AmountInput
              label={tr("Sicherheit", "Collateral")}
              value={collStr}
              onChange={setCollStr}
              suffix={NATIVE}
              invalid={coll === null}
              onMax={wallet.balance !== null ? () => setCollStr(formatUnits(wallet.balance!, 8, 8)) : undefined}
              hint={wallet.balance !== null ? tr(`Wallet-Guthaben: ${fmtKas(wallet.balance, 4)}`, `Wallet balance: ${fmtKas(wallet.balance, 4)}`) : undefined}
            />
            <AmountInput
              label={tr(`${STABLE_SYMBOL} prägen`, `Mint ${STABLE_SYMBOL}`)}
              value={mintStr}
              onChange={setMintStr}
              suffix={STABLE_SYMBOL}
              invalid={mint === null}
              onMax={plan ? () => setMintStr(formatUnits(plan.maxMint, 8, 8)) : undefined}
              hint={
                plan
                  ? tr(
                      `Höchstens ${fmtStable(plan.maxMint)} bei ${fmtBps(PARAMS.mcrBps, 0)} Mindestquote${capGhost !== null ? ` und höchstens ${de(capGhost, 0)} ${STABLE_SYMBOL} je Vault` : ""}`,
                      `At most ${fmtStable(plan.maxMint)} at ${fmtBps(PARAMS.mcrBps, 0)} minimum ratio${capGhost !== null ? ` and at most ${de(capGhost, 0)} ${STABLE_SYMBOL} per vault` : ""}`,
                    )
                  : undefined
              }
            />
            {overBalance && (
              <Callout kind="warn">{tr("Das ist mehr, als deine Wallet gerade hält. Der Rechner rechnet trotzdem weiter.", "That is more than your wallet currently holds. The calculator continues anyway.")}</Callout>
            )}
          </div>

          <div className="calc-results" aria-live="polite">
            {plan?.m ? (
              <>
                <HealthBar ratioBps={plan.m.ratio} hfE4={plan.m.hf} />
                <dl className="kv">
                  <div>
                    <dt>{tr("Wert der Sicherheit", "Collateral value")}</dt>
                    <dd>{formatUnits(plan.m.value, 8, 2)} USD</dd>
                  </div>
                  <div>
                    <dt>{tr("Schuld", "Debt")}</dt>
                    <dd>{fmtStable(plan.m.debt, 8)}</dd>
                  </div>
                  <div>
                    <dt>{tr("Besicherungsquote", "Collateral ratio")}</dt>
                    <dd>{plan.m.ratio === null ? "–" : fmtBps(plan.m.ratio)}</dd>
                  </div>
                  <div>
                    <dt>{tr("Liquidationspreis", "Liquidation price")}</dt>
                    <dd>
                      {plan.m.liqPrice === null ? "–" : fmtUsdPrice(plan.m.liqPrice)}
                      {plan.m.liqPrice !== null && oracle && (
                        <span className="muted small">
                          {" "}
                          ({formatUnits(((oracle.kasUsd - plan.m.liqPrice) * 10_000n) / oracle.kasUsd, 2, 1)} % {tr("Abstand", "away")})
                        </span>
                      )}
                    </dd>
                  </div>
                </dl>
                {!plan.check.ok && <Callout kind="danger">{plan.check.error}</Callout>}
                {live && coll !== null && mint !== null && (
                  <p className="muted small">
                    {tr("Echt anlegen: unter „Aktionen“ erst „Vault eröffnen“, dann „Prägen“.", "For real: under “Actions” first “Open vault”, then “Mint”.")}
                  </p>
                )}
              </>
            ) : (
              <p className="muted">{tr("Bitte gültige Beträge und einen KAS-Preis eingeben.", "Please enter valid amounts and a KAS price.")}</p>
            )}
          </div>
        </div>

        {projection && (
          <div className="table-wrap section-sm">
            <table className="simple">
              <caption>
                {liveRate
                  ? tr(
                      `Schuld und Zins über die Zeit beim aktuellen Satz des Orakels von ${aprStr} % p. a., falls er gleich bliebe. Die Schuld bleibt, der Zins wächst und zählt für die Quote mit.`,
                      `Debt and interest over time at the oracle's current rate of ${aprStr} % p.a., if it stayed the same. The debt stays, the interest grows and counts toward the ratio.`,
                    )
                  : tr(
                      `Schuld und Zins über die Zeit bei einem gleichbleibenden Satz von ${aprStr} % p. a. (Szenario). Die Schuld bleibt, der Zins wächst und zählt für die Quote mit.`,
                      `Debt and interest over time at a constant rate of ${aprStr} % p.a. (scenario). The debt stays, the interest grows and counts toward the ratio.`,
                    )}
              </caption>
              <thead>
                <tr>
                  <th scope="col">{tr("Zeitpunkt", "Time")}</th>
                  <th scope="col">{tr("Schuld", "Debt")}</th>
                  <th scope="col">{tr("Zins", "Interest")}</th>
                  <th scope="col">{tr("Liquidationspreis", "Liquidation price")}</th>
                </tr>
              </thead>
              <tbody>
                {projection.map((p) => (
                  <tr key={p.day}>
                    <th scope="row">{p.day === 0 ? tr("heute", "today") : p.day === 30 ? tr("in 30 Tagen", "in 30 days") : tr(`in ${p.day / 365} Jahr${p.day > 365 ? "en" : ""}`, `in ${p.day / 365} year${p.day > 365 ? "s" : ""}`)}</th>
                    <td>{fmtStable(p.debt, 4)}</td>
                    <td>{formatUnits(p.interest, 8, 4)} USD</td>
                    <td>{coll ? fmtUsdPrice(minHealthyPrice(coll, p.debt + p.interest, PARAMS.liqBps) ?? 0n) : "–"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </div>
  );
}
