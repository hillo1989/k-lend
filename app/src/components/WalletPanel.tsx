import { NETWORKS } from "../config";
import { NoWallet } from "./NoWallet";
import { useStatus } from "../lib/StatusContext";
import { shortHex, xOnlyKey } from "../lib/status";
import { fmtKas, shortAddress } from "../lib/format";
import { WALLET_NAMES, type WalletKind } from "../wallet/providers";
import { useWallet } from "../wallet/WalletContext";
import { Callout } from "./ui";
import { tr } from "../lib/i18n";

export function WalletPanel() {
  const w = useWallet();
  const { network } = useStatus();
  const selected = NETWORKS[network];
  const mismatch = w.network !== null && w.network.id !== network;
  const xonly = xOnlyKey(w.publicKey);

  return (
    <section className="card wallet-panel" aria-labelledby="wallet-title">
      <div className="card-head">
        <h2 id="wallet-title">{tr("Browser-Wallet (optional)", "Browser wallet (optional)")}</h2>
        <span className="tag">{tr("Wallet nur lesend", "wallet read-only")}</span>
      </div>

      {w.error && (
        <Callout kind="warn">
          <span>{w.error}</span>{" "}
          <button className="link-btn" onClick={w.clearError}>
            {tr("Ausblenden", "Dismiss")}
          </button>
        </Callout>
      )}

      {w.status === "connected" && w.address ? (
        <>
          <dl className="kv">
            <div>
              <dt>Wallet</dt>
              <dd>{w.kind ? WALLET_NAMES[w.kind] : "–"}</dd>
            </div>
            <div>
              <dt>{tr("Adresse", "Address")}</dt>
              <dd>
                <code title={w.address}>{shortAddress(w.address)}</code>
              </dd>
            </div>
            <div>
              <dt>{tr("Netzwerk", "Network")}</dt>
              <dd>
                <span className={mismatch ? "tag tag-warn" : "tag"}>{w.network?.label ?? tr("unbekannt", "unknown")}</span>
              </dd>
            </div>
            <div>
              <dt>{tr("Guthaben", "Balance")}</dt>
              <dd>{w.balance === null ? tr("nicht verfügbar", "not available") : fmtKas(w.balance, 4)}</dd>
            </div>
            <div>
              <dt>{tr("Schlüssel (x-only)", "Key (x-only)")}</dt>
              <dd>{xonly ? <code title={xonly}>{shortHex(xonly)}</code> : tr("nicht verfügbar", "not available")}</dd>
            </div>
          </dl>
          {mismatch && (
            <Callout kind="warn" title={tr("Anderes Netz", "Different network")}>
              {tr(
                `Die Wallet ist auf ${w.network?.label ?? "einem anderen Netz"}, die Seite zeigt ${selected.label}. Wechsle das Netz in der Wallet.`,
                `The wallet is on ${w.network?.label ?? "another network"}, the page shows ${selected.label}. Switch the network in the wallet.`,
              )}
            </Callout>
          )}
          <div className="btn-row">
            <button className="btn btn-ghost" onClick={() => void w.refresh()}>
              {tr("Aktualisieren", "Refresh")}
            </button>
            <button className="btn btn-ghost" onClick={() => void w.disconnect()}>
              {tr("Trennen", "Disconnect")}
            </button>
          </div>
          <p className="muted small">
            {tr(
              "Die Seite liest nur Adresse, Netzwerk, Guthaben und öffentlichen Schlüssel. Sie fordert keine Signatur an und sendet keine Transaktion.",
              "The page only reads address, network, balance and public key. It never requests a signature and sends no transaction.",
            )}
          </p>
        </>
      ) : w.installed.length === 0 ? (
        <div className="no-wallet">
          <NoWallet />
          <p className="muted small">{tr("Die Live-Daten und den Rechner siehst du auch ohne Wallet.", "You can see live data and the calculator without a wallet too.")}</p>
        </div>
      ) : (
        <>
          <p>{tr("Verbinde deine Wallet, um Adresse, Netzwerk und KAS-Guthaben anzuzeigen.", "Connect your wallet to show address, network and KAS balance.")}</p>
          <div className="btn-row">
            {w.installed.map((k: WalletKind) => (
              <button
                key={k}
                className="btn btn-primary"
                disabled={w.status === "connecting"}
                aria-busy={w.status === "connecting"}
                onClick={() => void w.connect(k)}
              >
                {w.status === "connecting" ? tr("Warte auf Wallet …", "Waiting for wallet …") : tr(`Mit ${WALLET_NAMES[k]} verbinden`, `Connect ${WALLET_NAMES[k]}`)}
              </button>
            ))}
          </div>
          <p className="muted small">
            {tr(
              "Die Wallet fragt nach einer Freigabe. Von der Wallet wird nie eine Signatur angefordert. Aktionen laufen über das Konto (Schlüsseldatei) oben.",
              "The wallet asks for approval. No signature is ever requested from the wallet. Actions run through the account (key file) above.",
            )}
          </p>
        </>
      )}
    </section>
  );
}
