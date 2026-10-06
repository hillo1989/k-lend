import { NATIVE, STABLE_SYMBOL } from "../config";
import { useAccount } from "../lib/AccountContext";
import type { CliAction } from "../lib/commands";
import { useStatus } from "../lib/StatusContext";
import { de, shortHex, vaultLabel, xOnlyKey, type VaultStatus } from "../lib/status";
import { SWEEP_MIN_COLLATERAL_KAS, vaultInterestEatsCollateral, vaultSweepable } from "../lib/precheck";
import { useWallet } from "../wallet/WalletContext";
import { HealthBar } from "./ui";
import { tr } from "../lib/i18n";
import { Usd } from "./Usd";

function health(v: VaultStatus, liqPct: number) {
  if (v.ratioPct === null) return { ratioBps: null, hfE4: null };
  return {
    ratioBps: BigInt(Math.round(v.ratioPct * 100)),
    hfE4: BigInt(Math.round((v.ratioPct / liqPct) * 10_000)),
  };
}

const QUICK: { action: CliAction; label: () => string }[] = [
  { action: "mint", label: () => tr("Prägen", "Mint") },
  { action: "repay", label: () => tr("Tilgen", "Repay") },
  { action: "deposit", label: () => tr("Einzahlen", "Deposit") },
  { action: "withdraw", label: () => tr("Abheben", "Withdraw") },
  { action: "close", label: () => tr("Schließen", "Close") },
];

export function VaultList({ onAction }: { onAction(action: CliAction, vault: number): void }) {
  const { status, network } = useStatus();
  const acc = useAccount();
  const w = useWallet();
  const walletKey = xOnlyKey(w.publicKey);
  // öffentliche Seite: eigene Vaults = die der verbundenen Wallet (keine Schlüsseldateien)
  const myKey = acc.publicMode ? (walletKey?.toLowerCase() ?? null) : (acc.selected?.xonly.toLowerCase() ?? null);
  if (!status?.deployed) return null;
  const vaults = [...status.vaults].sort(
    (a, b) => Number(b.owner.toLowerCase() === myKey) - Number(a.owner.toLowerCase() === myKey) || a.index - b.index,
  );

  // öffentliche Seite: nur die Vaults der verbundenen Wallet offen, fremde eingeklappt
  const mine = vaults.filter((v) => myKey !== null && v.owner.toLowerCase() === myKey);
  const others = vaults.filter((v) => !(myKey !== null && v.owner.toLowerCase() === myKey));
  const item = (v: VaultStatus) => {
            const own = myKey !== null && v.owner.toLowerCase() === myKey;
            const walletOwn = walletKey !== null && v.owner.toLowerCase() === walletKey;
            const h = health(v, status.params.liqPct);
            // Version 4: eingefrorenes Orakel sperrt Liquidieren, Rücknahme und Auflösen (Audit 14 N8)
            const frozen = !!status.oracle.frozen;
            const liquidatable = v.ratioPct !== null && v.ratioPct < status.params.liqPct;
            // Rücknahme zu 1 USD: an Vaults mit Schuld ab der Liquidationsschwelle (Version 3)
            const redeemable = !frozen && !own && v.debtGhost > 0 && v.ratioPct !== null && !liquidatable;
            // ohne Schuld, Zins ≥ Sicherheit: jeder darf zugunsten der Zinsadresse auflösen (Audit 11 A11-V-4)
            const sweepable = !frozen && vaultSweepable(v, status);
            // der Zins zehrt auch Vaults auf, die zu klein zum Auflösen sind: Etikett ohne Knopf (Audit 12, B-P5)
            const eaten = vaultInterestEatsCollateral(v, status);
            return (
              <li key={v.covenantId} className={`vault-item${own ? " own" : ""}${v.stale ? " stale" : ""}`}>
                <div className="vault-item-head">
                  <strong>{vaultLabel(v, status.vaults, myKey)}</strong>
                  {own && !acc.publicMode && <span className="tag">{tr("dein Schlüssel", "your key")}</span>}
                  {walletOwn && <span className="tag">{tr("deine Wallet", "your wallet")}</span>}
                  {liquidatable && <span className="tag tag-danger">{tr("liquidierbar", "liquidatable")}</span>}
                  {v.stale && <span className="tag tag-warn">{tr("von Dritten verändert – gesperrt", "changed by third party – blocked")}</span>}
                  {eaten && <span className="tag tag-warn">{tr("Zins zehrt Sicherheit auf", "interest exceeds collateral")}</span>}
                  <span className="muted small">
                    {tr("Besitzer", "Owner")} <code title={v.owner}>{shortHex(v.owner)}</code> · Covenant <code title={v.covenantId}>{shortHex(v.covenantId)}</code>
                  </span>
                </div>
                <dl className="kv kv-4">
                  <div>
                    <dt>{tr("Sicherheit", "Collateral")}</dt>
                    <dd>
                      {de(v.collateralKas, 4)} {NATIVE}
                      <Usd amount={v.collateralKas} unit="KAS" />
                    </dd>
                  </div>
                  <div>
                    <dt>{tr("Schuld", "Debt")}</dt>
                    <dd>
                      {de(v.debtGhost, 8)} {STABLE_SYMBOL}
                      <Usd amount={v.debtGhost} unit="GHOST" />
                      {v.interestUsd !== undefined && (
                        <div className="muted small">{tr(`+ ${de(v.interestUsd, 4)} USD Zins`, `+ ${de(v.interestUsd, 4)} USD interest`)}</div>
                      )}
                    </dd>
                  </div>
                  <div>
                    <dt>{tr("Liquidationspreis", "Liquidation price")}</dt>
                    <dd>{v.liquidationPriceUsd === null ? "–" : `${de(v.liquidationPriceUsd, 6, 2)} USD`}</dd>
                  </div>
                  <div>
                    <dt>{tr("Noch prägbar", "Still mintable")}</dt>
                    <dd>
                      {de(v.maxMintGhost, 4)} {STABLE_SYMBOL}
                      <Usd amount={v.maxMintGhost} unit="GHOST" />
                    </dd>
                  </div>
                </dl>
                <HealthBar ratioBps={h.ratioBps} hfE4={h.hfE4} />
                {eaten && !sweepable && (
                  <p className="small muted">
                    {tr(
                      `Zu klein zum Auflösen: Erst ab ${de(SWEEP_MIN_COLLATERAL_KAS, 3)} KAS bleibt der Zinsadresse nach den 0,1 KAS für das Auflösen genug für einen eigenen Ausgang. Der Vault bleibt liegen. Sein Besitzer kann ihn schließen; ein Zins unter 0,2 KAS wird dabei erlassen.`,
                      `Too small to dissolve: only from ${de(SWEEP_MIN_COLLATERAL_KAS, 3)} KAS does enough remain for an output to the interest address after the 0.1 KAS for dissolving. The vault stays as it is. Its owner can close it; interest below 0.2 KAS is waived then.`,
                    )}
                  </p>
                )}
                {v.stale ? (
                  <p className="small muted">
                    {tr(
                      "Dieser Vault wurde außerhalb dieses Rechners verändert, zum Beispiel liquidiert. Aktionen sind gesperrt, bis ghostctl den Stand nachgeladen hat.",
                      "This vault was changed outside this computer, for example liquidated. Actions are blocked until ghostctl has reloaded the state.",
                    )}
                  </p>
                ) : !own && !redeemable && !(liquidatable && !frozen) && !sweepable ? null : (
                <div className="btn-row tight quick" role="group" aria-label={tr(`Aktionen für Vault ${v.index}`, `Actions for vault ${v.index}`)}>
                  {(own ? QUICK : []).map((q) => (
                    <button key={q.action} type="button" className="btn btn-ghost btn-sm" onClick={() => onAction(q.action, v.index)}>
                      {q.label()}
                      <span className="sr-only"> {vaultLabel(v, status.vaults, myKey)}</span>
                    </button>
                  ))}
                  {redeemable && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => onAction("redeem", v.index)}>
                      {tr("Rücknahme", "Redeem")}
                      <span className="sr-only"> {vaultLabel(v, status.vaults, myKey)}</span>
                    </button>
                  )}
                  {liquidatable && !frozen && (
                    <button type="button" className="btn btn-danger btn-sm" onClick={() => onAction("liquidate", v.index)}>
                      {tr("Liquidieren", "Liquidate")}<span className="sr-only"> {vaultLabel(v, status.vaults, myKey)}</span>
                    </button>
                  )}
                  {sweepable && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => onAction("sweep", v.index)}>
                      {tr("Auflösen (Zinsadresse)", "Dissolve (interest address)")}
                      <span className="sr-only"> {vaultLabel(v, status.vaults, myKey)}</span>
                    </button>
                  )}
                </div>
                )}
              </li>
            );
  };

  return (
    <section className="card section-sm" aria-labelledby="vaults-title">
      <div className="card-head">
        <h2 id="vaults-title">{acc.publicMode ? tr("Deine Vaults", "Your vaults") : tr("Vaults im Mainnet", "Vaults on mainnet")}</h2>
        <span className="tag">live · {acc.publicMode ? mine.length : vaults.length}</span>
      </div>
      <p className="muted small">
        {acc.publicMode ? (
          tr(
            "Die Vaults deiner verbundenen Wallet. Vaults anderer Nutzer stehen eingeklappt darunter – nur für Rücknahme und Liquidation.",
            "The vaults of your connected wallet. Other users' vaults are folded below – only for redemption and liquidation.",
          )
        ) : (
          <>
            {tr("Quelle ist ", "Source is ")}
            <code>ghostctl status</code>
            {tr(", also die Zustandsdatei dieses Rechners (", ", i.e. this computer's state file (")}
            <code>deployments/{network}.json</code>
            {tr(
              "). Vaults anderer Rechner fehlen. Deine Vaults, die zum gewählten Schlüssel gehören, stehen oben und sind hervorgehoben.",
              "). Vaults of other computers are missing. Your vaults belonging to the selected key are listed first and highlighted.",
            )}
          </>
        )}
      </p>
      {acc.publicMode && mine.length === 0 ? (
        <p>
          {myKey === null
            ? tr("Wallet verbinden, um deine Vaults zu sehen.", "Connect your wallet to see your vaults.")
            : tr("Diese Wallet hat noch keinen Vault. Oben „Vault eröffnen“.", "This wallet has no vault yet. Use “Open vault” above.")}
        </p>
      ) : vaults.length === 0 ? (
        <p>{tr("Noch kein Vault angelegt. Unter „Aktionen“ → „Vault eröffnen“.", "No vault yet. Use “Actions” → “Open vault”.")}</p>
      ) : (
        <ul className="vault-list">
          {(acc.publicMode ? mine : vaults).map(item)}
        </ul>
      )}
      {acc.publicMode && others.length > 0 && (
        <details className="section-sm">
          <summary>
            {tr(`Fremde Vaults (${others.length}) – für Rücknahme und Liquidation`, `Other vaults (${others.length}) – for redemption and liquidation`)}
          </summary>
          <ul className="vault-list">{others.map(item)}</ul>
        </details>
      )}
    </section>
  );
}
