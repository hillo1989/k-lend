import { useSyncExternalStore } from "react";
import { StatusNotices, useKasPrice } from "../components/Network";
import { PriceChart } from "../components/PriceChart";
import { TokenLabel } from "../components/TokenIcons";
import { Stat } from "../components/ui";
import { NATIVE, NETWORKS, STABLE_SYMBOL } from "../config";
import { tr } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { formatUnits } from "../lib/format";
import { de } from "../lib/status";
import { txLog } from "../lib/txlog";
import { href } from "../router";
import { Usd } from "../components/Usd";

/** Anzahl der von diesem Browser gesendeten Aktionen (Verlauf der Wallet) */
function useLocalTxCount(): number {
  return useSyncExternalStore(
    (cb) => {
      window.addEventListener("ghost-txlog", cb);
      return () => window.removeEventListener("ghost-txlog", cb);
    },
    () => txLog("mainnet", null).length,
    () => 0,
  );
}

const pctText = (x: number | null, frac = 1) => (x === null ? "–" : `${de(x, frac)} %`);

/** Kennzahlen des Protokolls: Vaults, Orakel, Tauschpool, Marktpreis */
export function Statistics() {
  const { status, network } = useStatus();
  const live = status?.deployed ? status : null;
  const market = useKasPrice();
  const localTx = useLocalTxCount();
  const dash = "–";

  const oracle = live?.oracle.kasUsd ?? null;
  const collUsd = live && oracle !== null ? live.totals.collateralKas * oracle : null;
  // Version 3: für die Quote zählen Schuld und offener Zins
  const owedUsd = live ? live.totals.debtGhost + (live.totals.interestUsd ?? 0) : 0;
  const totalRatio = live && collUsd !== null && owedUsd > 0 ? (collUsd / owedUsd) * 100 : null;
  const withDebt = live ? live.vaults.filter((v) => v.debtGhost > 0) : [];
  const liq = live?.params.liqPct ?? 150;
  const below = (pct: number) => withDebt.filter((v) => v.ratioPct !== null && v.ratioPct < pct).length;
  const largest = withDebt.reduce((m, v) => Math.max(m, v.debtGhost), 0);
  const avgRatio = withDebt.length ? withDebt.reduce((s, v) => s + (v.ratioPct ?? 0), 0) / withDebt.length : null;
  const oracleVsMarket = oracle !== null && market !== null ? ((oracle - market) / market) * 100 : null;

  const pool = live?.pool ?? null;
  const pk = pool ? Number(BigInt(pool.kasSompi)) / 1e8 : null;
  const pg = pool ? Number(BigInt(pool.ghostUnits)) / 1e8 : null;
  const kasPerGhost = pk !== null && pg ? pk / pg : null;
  const ghostUsd = kasPerGhost !== null && oracle !== null ? kasPerGhost * oracle : null;
  const poolUsd = pk !== null && pg !== null && oracle !== null ? pk * oracle + pg * (ghostUsd ?? 1) : null;

  // Verteilung der Quoten über die Vaults mit Schuld
  const buckets = [
    { label: tr(`unter ${liq} % (liquidierbar)`, `below ${liq} % (liquidatable)`), n: below(liq) },
    { label: `${liq}–200 %`, n: below(200) - below(liq) },
    { label: "200–300 %", n: below(300) - below(200) },
    { label: tr("über 300 %", "above 300 %"), n: withDebt.length - below(300) },
  ];
  const maxBucket = Math.max(1, ...buckets.map((b) => b.n));

  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          {tr("Statistiken", "Statistics")}
        </h1>
        <span className="tag">{live ? `live · ${NETWORKS[network].label}` : NETWORKS[network].label}</span>
      </div>
      <StatusNotices />
      <PriceChart />
      <p className="lead section-sm">
        {tr(
          "Alle Kennzahlen stammen aus der Zustandsdatei dieses Rechners und der Kette. Vaults, die nur auf anderen Rechnern bekannt sind, fehlen.",
          "All figures come from this computer's state file and the chain. Vaults known only on other computers are missing.",
        )}
      </p>

      <h2 className="section-sm">{tr(`${STABLE_SYMBOL} und Vaults`, `${STABLE_SYMBOL} and vaults`)}</h2>
      <div className="stats-grid">
        <Stat
          label={<TokenLabel token="GHOST" size={16} />}
          value={live ? de(live.totals.debtGhost, 4) : dash}
          hint={
            <>
              {live?.totals.interestUsd !== undefined
                ? tr(`Schuld aller bekannten Vaults, dazu ${de(live.totals.interestUsd, 2)} USD offener Zins`, `debt of all known vaults, plus ${de(live.totals.interestUsd, 2)} USD open interest`)
                : tr("Schuld aller bekannten Vaults", "debt of all known vaults")}
              {live && <Usd amount={live.totals.debtGhost} unit="GHOST" />}
            </>
          }
        />
        <Stat
          label={<TokenLabel token="KAS" size={16} />}
          value={live ? de(live.totals.collateralKas, 2) : dash}
          hint={collUsd !== null ? tr(`Sicherheit ≈ ${de(collUsd, 2)} USD`, `collateral ≈ ${de(collUsd, 2)} USD`) : tr("Sicherheit", "collateral")}
        />
        <Stat label={tr("Gesamtquote", "Overall ratio")} value={pctText(totalRatio, 0)} hint={tr("Sicherheit / (Schuld + Zins)", "collateral / (debt + interest)")} />
        <Stat label="Vaults" value={live ? String(live.totals.vaults) : dash} hint={tr(`davon ${withDebt.length} mit Schuld`, `${withDebt.length} with debt`)} />
      </div>
      <div className="stats-grid section-sm">
        <Stat label={tr("Durchschnittliche Quote", "Average ratio")} value={pctText(avgRatio, 0)} hint={tr("über Vaults mit Schuld", "across vaults with debt")} />
        <Stat
          label={tr("Liquidierbar", "Liquidatable")}
          value={live ? String(below(liq)) : dash}
          hint={tr(`unter ${live?.params.liqPct ?? 150} %`, `below ${live?.params.liqPct ?? 150} %`)}
        />
        <Stat label={tr("Größte Schuld", "Largest debt")} value={live ? `${de(largest, 4)} ${STABLE_SYMBOL}` : dash} hint={tr("einzelner Vault", "single vault")} />
        <Stat
          label={tr("Obergrenze je Vault", "Limit per vault")}
          value={!live ? dash : live.params.maxDebtGhost != null ? `${de(live.params.maxDebtGhost, 0)} ${STABLE_SYMBOL}` : tr("keine", "none")}
          hint={tr("Vaults unbegrenzt", "unlimited vaults")}
        />
      </div>

      {live && withDebt.length > 0 && (
        <section className="card section-sm" aria-labelledby="verteilung">
          <div className="card-head">
            <h2 id="verteilung">{tr("Verteilung der Quoten", "Ratio distribution")}</h2>
          </div>
          <ul className="bars">
            {buckets.map((b) => (
              <li key={b.label}>
                <span className="bars-label">{b.label}</span>
                <span className="bars-track" aria-hidden="true">
                  <span className="bars-fill" style={{ width: `${(b.n / maxBucket) * 100}%` }} />
                </span>
                <span className="bars-value">{b.n}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      <h2 className="section-sm">{tr("Preise", "Prices")}</h2>
      <div className="stats-grid">
        <Stat label={tr("KAS Marktpreis", "KAS market price")} value={market !== null ? `${de(market, 6, 4)} USD` : dash} hint={tr("Median aus 6 Quellen", "median of 6 sources")} />
        <Stat label={tr("KAS Orakelpreis", "KAS oracle price")} value={oracle !== null ? `${de(oracle, 6, 4)} USD` : dash} hint={live ? tr(`Update Nr. ${live.oracle.seq}`, `update no. ${live.oracle.seq}`) : undefined} />
        <Stat
          label={tr("Orakel zu Markt", "Oracle vs. market")}
          value={oracleVsMarket !== null ? `${oracleVsMarket >= 0 ? "+" : ""}${de(oracleVsMarket, 2)} %` : dash}
          hint={tr("ab 0,5 % aktualisiert der Agent", "the agent updates from 0.5 %")}
        />
        <Stat
          label={tr(`${STABLE_SYMBOL} am Markt`, `${STABLE_SYMBOL} on the market`)}
          value={ghostUsd !== null ? `${de(ghostUsd, 4)} USD` : dash}
          hint={ghostUsd !== null ? tr(`${ghostUsd >= 1 ? "+" : ""}${de((ghostUsd - 1) * 100, 2)} % zum Dollar`, `${ghostUsd >= 1 ? "+" : ""}${de((ghostUsd - 1) * 100, 2)} % vs. the dollar`) : tr("aus dem Tauschpool", "from the swap pool")}
        />
      </div>

      <h2 className="section-sm">{tr("Tauschpool", "Swap pool")}</h2>
      <div className="stats-grid">
        <Stat label={<TokenLabel token="KAS" size={16} />} value={pk !== null ? de(pk, 2) : dash} hint={<>{tr("Reserve", "reserve")}{pk !== null && <Usd amount={pk} unit="KAS" />}</>} />
        <Stat label={<TokenLabel token="GHOST" size={16} />} value={pg !== null ? de(pg, 4) : dash} hint={<>{tr("Reserve", "reserve")}{pg !== null && <Usd amount={pg} unit="GHOST" />}</>} />
        <Stat label={tr("Gesamtwert", "Total value")} value={poolUsd !== null ? `${de(poolUsd, 2, 2)} USD` : dash} hint={tr("beide Reserven", "both reserves")} />
        <Stat
          label={tr("Anteile", "Shares")}
          value={pool ? formatUnits(BigInt(pool.shares), 0) : dash}
          hint={tr(`davon ${formatUnits(100_000_000n, 0)} dauerhaft gesperrt (1 KAS)`, `incl. ${formatUnits(100_000_000n, 0)} locked forever (1 KAS)`)}
        />
      </div>
      {!pool && live && (
        <p className="muted small">
          {tr("Noch kein Tauschpool angelegt. ", "No swap pool created yet. ")}
          <a href={href("tauschen")}>{tr("Zum Tauschen →", "To swap →")}</a>
        </p>
      )}

      <h2 className="section-sm">{tr("Dieser Browser", "This browser")}</h2>
      <div className="stats-grid">
        <Stat label={tr("Gesendete Aktionen", "Actions sent")} value={String(localTx)} hint={tr("laut Verlauf der Wallet-Seite", "per the wallet page history")} />
        <Stat label={tr("Netzgebühr je Aktion", "Network fee per action")} value={`≈ ${de(0.03, 2)}–${de(0.06, 2)} KAS`} hint={tr("gemessen im Testnetz und Simulator", "measured on testnet and in the simulator")} />
      </div>
      <p className="muted small section-sm">
        {tr(
          `Hinweis: ${NATIVE}-Werte in USD rechnen mit dem Orakelpreis, nicht mit dem Marktpreis.`,
          `Note: ${NATIVE} values in USD use the oracle price, not the market price.`,
        )}
      </p>
    </div>
  );
}
