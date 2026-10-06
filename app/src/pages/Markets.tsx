import { NATIVE, NETWORKS, STABLE_SYMBOL } from "../config";
import { StatusNotices } from "../components/Network";
import { GhostIcon, KasIcon } from "../components/TokenIcons";
import { PlannedTag } from "../components/ui";
import { demoPool, pct } from "../lib/demo";
import { tr } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { de } from "../lib/status";
import { href } from "../router";
import { Usd } from "../components/Usd";

export function Markets() {
  const { status, network } = useStatus();
  const live = status?.deployed ? status : null;
  const pool = live?.pool ?? null;
  const dash = "–";
  const poolKas = pool ? Number(BigInt(pool.kasSompi)) / 1e8 : null;
  const poolGhost = pool ? Number(BigInt(pool.ghostUnits)) / 1e8 : null;
  const ghostUsd = poolKas !== null && poolGhost && live ? (poolKas / poolGhost) * live.oracle.kasUsd : null;
  const colZins = tr("Zins p. a.", "Rate p.a.");
  const colVol = tr("Volumen", "Volume");
  const colKey = tr("Kennzahl", "Key figure");

  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          {tr("Märkte", "Markets")}
        </h1>
        <span className="tag">{live ? `live · ${NETWORKS[network].label}` : NETWORKS[network].label}</span>
      </div>
      <p className="lead">
        {tr(
          `${STABLE_SYMBOL} gegen ${NATIVE}-Sicherheit prägen und im Pool tauschen – live aus den Verträgen im ${NETWORKS[network].label}. Der ${NATIVE}-Verleih-Pool ist geplant.`,
          `Mint ${STABLE_SYMBOL} against ${NATIVE} collateral and swap in the pool – live from the contracts on ${NETWORKS[network].label}. The ${NATIVE} lending pool is planned.`,
        )}
      </p>
      <StatusNotices />

      <div className="table-wrap">
        <table className="markets">
          <caption className="sr-only">{tr("Übersicht der Märkte", "Market overview")}</caption>
          <thead>
            <tr>
              <th scope="col">{tr("Markt", "Market")}</th>
              <th scope="col">{colZins}</th>
              <th scope="col">{colVol}</th>
              <th scope="col">{colKey}</th>
              <th scope="col">
                <span className="sr-only">{tr("Aktion", "Action")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <th scope="row">
                <div className="asset">
                  <span className="asset-icon asset-icon-svg" aria-hidden="true">
                    <GhostIcon size={38} />
                  </span>
                  <div>
                    <div className="asset-name">{tr(`${STABLE_SYMBOL} prägen`, `Mint ${STABLE_SYMBOL}`)}</div>
                    <div className="muted small">{tr(`gegen ${NATIVE}-Sicherheit · live`, `against ${NATIVE} collateral · live`)}</div>
                  </div>
                </div>
              </th>
              <td data-label={colZins}>
                {live ? pct(live.oracle.ratePctYear) : dash}
                <div className="muted small">{tr("Stablecoin-Zins", "Stablecoin rate")}</div>
              </td>
              <td data-label={colVol}>
                {live ? `${de(live.totals.debtGhost, 8)} ${STABLE_SYMBOL}` : dash}
                {live && <Usd amount={live.totals.debtGhost} unit="GHOST" />}
                <div className="muted small">{tr("Gesamtschuld", "Total debt")}</div>
              </td>
              <td data-label={colKey}>
                {live ? tr(`Mindestquote ${de(live.params.mcrPct, 0)} %`, `Minimum ratio ${de(live.params.mcrPct, 0)} %`) : dash}
                <div className="muted small">
                  {live
                    ? tr(
                        `Liquidation unter ${de(live.params.liqPct, 0)} %, Bonus ${de(live.params.bonusPct, 0)} %`,
                        `Liquidation below ${de(live.params.liqPct, 0)} %, bonus ${de(live.params.bonusPct, 0)} %`,
                      )
                    : ""}
                </div>
              </td>
              <td className="cell-action">
                <a className="btn btn-primary" href={href("vault")}>
                  {tr("Zu den Vaults", "To the vaults")}
                </a>
              </td>
            </tr>
            <tr>
              <th scope="row">
                <div className="asset">
                  <span className="asset-icon asset-icon-svg" aria-hidden="true">
                    <KasIcon size={38} />
                  </span>
                  <div>
                    <div className="asset-name">{tr(`Tauschpool ${NATIVE}/${STABLE_SYMBOL}`, `Swap pool ${NATIVE}/${STABLE_SYMBOL}`)}</div>
                    <div className="muted small">{pool ? tr("offen für alle · live", "open to all · live") : tr("noch nicht angelegt", "not created yet")}</div>
                  </div>
                </div>
              </th>
              <td data-label={colZins}>
                {de(0.3, 1)} %
                <div className="muted small">{tr("Gebühr je Tausch, an die Einleger", "fee per swap, to liquidity providers")}</div>
              </td>
              <td data-label={colVol}>
                {poolKas !== null && poolGhost !== null ? `${de(poolKas, 2)} ${NATIVE} / ${de(poolGhost, 4)} ${STABLE_SYMBOL}` : dash}
                {poolKas !== null && <Usd amount={poolKas} unit="KAS" />}
                <div className="muted small">{tr("Reserven", "Reserves")}</div>
              </td>
              <td data-label={colKey}>
                {ghostUsd !== null ? `1 ${STABLE_SYMBOL} = ${de(ghostUsd, 4)} USD` : dash}
                <div className="muted small">{tr("Marktpreis (mit Orakelpreis für KAS)", "market price (KAS at oracle price)")}</div>
              </td>
              <td className="cell-action">
                <a className="btn btn-primary" href={href("tauschen")}>
                  {tr("Zum Tauschen", "To swap")}
                </a>
              </td>
            </tr>
            <tr>
              <th scope="row">
                <div className="asset">
                  <span className="asset-icon asset-icon-svg" aria-hidden="true">
                    <KasIcon size={38} />
                  </span>
                  <div>
                    <div className="asset-name">{tr(`${NATIVE}-Verleih-Pool`, `${NATIVE} lending pool`)}</div>
                    <div className="muted small">
                      <PlannedTag />
                    </div>
                  </div>
                </div>
              </th>
              <td data-label={colZins}>
                {pct(demoPool.supplyApr)} {tr("Einlage", "supply")}
                <div className="muted small">{tr("Annahme", "Assumption")}</div>
              </td>
              <td data-label={colVol}>
                {dash}
                <div className="muted small">{tr("noch kein Vertrag", "no contract yet")}</div>
              </td>
              <td data-label={colKey}>
                {tr("Zinsmodell", "Rate model")}
                <div className="muted small">{tr("Annahme, siehe Pool", "assumption, see Pool")}</div>
              </td>
              <td className="cell-action">
                <a className="btn btn-primary" href={href("pool")}>
                  {tr("Zum Pool", "To the pool")}
                </a>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <p className="small section-sm">
        <a href={href("statistiken")}>{tr("Alle Kennzahlen im Reiter „Statistiken“ →", "All figures in the “Statistics” tab →")}</a>
      </p>

    </div>
  );
}
