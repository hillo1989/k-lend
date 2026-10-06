import { useEffect, useId, useRef, useState } from "react";
import { NETWORKS, WALLET_LINKS } from "../config";
import { fmtKas, fmtStable, shortAddress } from "../lib/format";
import { xOnlyKey } from "../lib/status";
import { tr } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { WALLET_NAMES, type WalletKind } from "../wallet/providers";
import { useWallet } from "../wallet/WalletContext";
import { CopyButton } from "./CopyCode";
import { GhostIcon, KasIcon } from "./TokenIcons";
import { Usd } from "./Usd";

/**
 * Kopfzeile oben rechts: „Wallet verbinden“ bzw. im verbundenen Zustand das
 * KAS-Guthaben der Browser-Wallet. Nur lesend – die Seite fordert von der
 * Wallet nie eine Signatur an (Aktionen laufen über die Schlüsseldatei).
 */
export function WalletButton() {
  const w = useWallet();
  const { network, status } = useStatus();
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const panelId = useId();
  const connected = w.status === "connected" && !!w.address;
  const mismatch = connected && w.network !== null && w.network.id !== network;
  // GHOST kennt die Browser-Wallet nicht: Bestand aus den GHOST-UTXOs, die
  // ghostctl für den Schlüssel dieser Adresse kennt (Zustandsdatei + Kette)
  const xonly = xOnlyKey(w.publicKey);
  const ghostUnits =
    connected && xonly && status?.deployed
      ? status.tokens.filter((t) => t.owner === xonly).reduce((sum, t) => sum + BigInt(Math.round(t.amountGhost * 1e8)), 0n)
      : null;

  // Klick daneben oder Escape schließt das Fenster
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // Nach erfolgreichem Verbinden das Auswahlfenster schließen, bei einem Fehler öffnen
  useEffect(() => {
    if (connected) setOpen(false);
  }, [connected]);
  useEffect(() => {
    if (w.error) setOpen(true);
  }, [w.error]);

  const onMain = () => {
    if (!connected && w.installed.length === 1 && w.status !== "connecting") {
      void w.connect(w.installed[0]);
      return;
    }
    setOpen((o) => !o);
  };

  const label = connected
    ? w.balance !== null
      ? fmtKas(w.balance, 2)
      : shortAddress(w.address!)
    : w.status === "connecting"
      ? tr("Warte auf Wallet …", "Waiting for wallet …")
      : tr("Wallet verbinden", "Connect wallet");

  return (
    <div className="wallet-btn-wrap" ref={wrapRef}>
      <button
        type="button"
        className={connected ? (mismatch ? "btn btn-sm wallet-btn connected warn" : "btn btn-sm wallet-btn connected") : "btn btn-sm btn-primary wallet-btn"}
        onClick={onMain}
        disabled={w.status === "connecting"}
        aria-busy={w.status === "connecting"}
        aria-expanded={open}
        aria-controls={panelId}
        title={connected ? `${w.kind ? WALLET_NAMES[w.kind] : "Wallet"} · ${w.address}` : undefined}
      >
        {connected && (
          <>
            <span className="dot" aria-hidden="true" />
            <KasIcon size={16} />
          </>
        )}
        <span>{label}</span>
        {connected && ghostUnits !== null && (
          <span className="wallet-btn-ghost">
            <GhostIcon size={16} />
            {fmtStable(ghostUnits, 2)}
          </span>
        )}
        {connected && <span className="sr-only">{tr(" – Wallet-Details öffnen", " – open wallet details")}</span>}
      </button>

      {open && (
        <div className="wallet-pop card" id={panelId} role="dialog" aria-label={tr("Browser-Wallet", "Browser wallet")}>
          {w.error && <p className="small warn-text">{w.error}</p>}
          {connected ? (
            <>
              <dl className="kv small">
                <div>
                  <dt>Wallet</dt>
                  <dd>{w.kind ? WALLET_NAMES[w.kind] : "–"}</dd>
                </div>
                <div>
                  <dt>{tr("Guthaben", "Balance")}</dt>
                  <dd>
                    <strong>{w.balance === null ? tr("nicht verfügbar", "not available") : fmtKas(w.balance, 8)}</strong>
                    {w.balance !== null && <Usd amount={Number(w.balance) / 1e8} unit="KAS" />}
                  </dd>
                </div>
                <div>
                  <dt>{tr("GHOST", "GHOST")}</dt>
                  <dd>
                    <strong>{ghostUnits !== null ? fmtStable(ghostUnits, 8) : tr("nicht bekannt", "not known")}</strong>
                    {ghostUnits !== null && <Usd amount={Number(ghostUnits) / 1e8} unit="GHOST" />}
                  </dd>
                </div>
                <div>
                  <dt>{tr("Adresse", "Address")}</dt>
                  <dd className="addr">
                    <code title={w.address!}>{shortAddress(w.address!)}</code>
                    <CopyButton text={w.address!} label={tr("Adresse", "Address")} />
                  </dd>
                </div>
                <div>
                  <dt>{tr("Netz", "Network")}</dt>
                  <dd>
                    <span className={mismatch ? "tag tag-warn" : "tag"}>{w.network?.label ?? tr("unbekannt", "unknown")}</span>
                  </dd>
                </div>
              </dl>
              {mismatch && (
                <p className="small warn-text">
                  {tr(
                    `Die Wallet ist auf ${w.network?.label ?? "einem anderen Netz"}, die Seite zeigt ${NETWORKS[network].label}.`,
                    `The wallet is on ${w.network?.label ?? "another network"}, the page shows ${NETWORKS[network].label}.`,
                  )}
                </p>
              )}
              <div className="btn-row">
                <button type="button" className="btn btn-ghost btn-sm" onClick={() => void w.refresh()}>
                  {tr("Aktualisieren", "Refresh")}
                </button>
                <button
                  type="button"
                  className="btn btn-ghost btn-sm"
                  onClick={() => {
                    void w.disconnect();
                    setOpen(false);
                  }}
                >
                  {tr("Trennen", "Disconnect")}
                </button>
              </div>
              <p className="muted small">
                {tr(
                  "Nur lesend: Die Seite fordert von der Wallet nie eine Signatur an. GHOST zeigt die Wallet selbst nicht an; der Bestand hier stammt aus den GHOST-UTXOs, die ghostctl für diese Adresse kennt.",
                  "Read-only: the page never requests a signature from the wallet. The wallet itself does not show GHOST; the balance here comes from the GHOST UTXOs ghostctl knows for this address.",
                )}
              </p>
            </>
          ) : w.installed.length === 0 ? (
            <>
              <p className="small">
                {tr(
                  "Keine Kaspa-Wallet im Browser gefunden. Du brauchst eine Browser-Erweiterung, zum Beispiel:",
                  "No Kaspa wallet found in the browser. You need a browser extension, for example:",
                )}
              </p>
              <ul className="small">
                <li>
                  <a href={WALLET_LINKS.kasware} target="_blank" rel="noopener noreferrer">
                    KasWare Wallet
                  </a>
                </li>
                <li>
                  <a href={WALLET_LINKS.kastle} target="_blank" rel="noopener noreferrer">
                    Kastle Wallet
                  </a>
                </li>
              </ul>
              <p className="muted small">{tr("Nach der Installation die Seite neu laden.", "Reload the page after installing.")}</p>
            </>
          ) : (
            <>
              <p className="small">{tr("Welche Wallet?", "Which wallet?")}</p>
              <div className="btn-row">
                {w.installed.map((k: WalletKind) => (
                  <button key={k} type="button" className="btn btn-primary btn-sm" onClick={() => void w.connect(k)}>
                    {WALLET_NAMES[k]}
                  </button>
                ))}
              </div>
              <p className="muted small">{tr("Die Wallet fragt nach einer Freigabe. Die Seite liest nur Adresse, Netz und Guthaben.", "The wallet asks for approval. The page only reads address, network and balance.")}</p>
            </>
          )}
        </div>
      )}
    </div>
  );
}
