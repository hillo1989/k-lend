import { useEffect, useState, useSyncExternalStore } from "react";
import { ActionForms } from "../components/ActionForms";
import { AccountCard } from "../components/AccountCard";
import { WalletPanel } from "../components/WalletPanel";
import { useWallet } from "../wallet/WalletContext";
import { CopyButton } from "../components/CopyCode";
import { StatusNotices } from "../components/Network";
import { StandingOrders } from "../components/StandingOrders";
import { WalletTresor } from "../components/WalletTresor";
import { IncomingMessages } from "../components/IncomingMessages";
import { QrCode } from "../components/QrCode";
import { AmountInput, Callout } from "../components/ui";
import { NATIVE, NETWORKS, STABLE_SYMBOL } from "../config";
import { href } from "../router";
import { ApiError, receiveGhost, receiveGhostWallet, type ReceiveResult } from "../lib/api";
import { useAccount } from "../lib/AccountContext";
import { actionMeta, cliDecimal } from "../lib/commands";
import { parseUnits } from "../lib/format";
import { useStatus } from "../lib/StatusContext";
import { explorerAddress, explorerTx, txLog, type TxLogEntry } from "../lib/txlog";
import { GhostIcon, KasIcon } from "../components/TokenIcons";
import { getLang, locale, tr } from "../lib/i18n";
import { Usd } from "../components/Usd";
import { useUsd } from "../lib/usd";

function useTxLog(network: Parameters<typeof txLog>[0], key: string | null): TxLogEntry[] {
  const snap = useSyncExternalStore(
    (cb) => {
      window.addEventListener("ghost-txlog", cb);
      window.addEventListener("storage", cb);
      return () => {
        window.removeEventListener("ghost-txlog", cb);
        window.removeEventListener("storage", cb);
      };
    },
    () => JSON.stringify(txLog(network, key)),
    () => "[]",
  );
  return JSON.parse(snap) as TxLogEntry[];
}

const shortAddr = (s: string) => (s.length > 24 ? `${s.slice(0, 14)}…${s.slice(-8)}` : s);
const when = (t: number) => new Date(t).toLocaleString(locale(), { dateStyle: "short", timeStyle: "short" });

/**
 * Einfache Wallet für KAS und GHOST, bis gängige Wallets Covenant-Token
 * anzeigen. Konto = Schlüsseldatei in keys/, signiert wird lokal von ghostctl.
 */
export function Wallet() {
  const { network, status } = useStatus();
  const acc = useAccount();
  const w = useWallet();
  // Öffentliche Seite: keine Schlüsseldateien des Servers, sondern die Browser-Wallet
  const pub = acc.publicMode;
  const key = pub ? null : acc.selected;
  const address = pub ? (w.status === "connected" ? w.address : null) : (key?.address ?? null);
  const log = useTxLog(network, pub ? (address ? `wallet:${address}` : null) : (key?.file ?? null));
  const usd = useUsd();
  const deployed = Boolean(status?.deployed);

  // Empfang prüfen
  const [recvStr, setRecvStr] = useState("");
  const [recvBusy, setRecvBusy] = useState(false);
  const [recv, setRecv] = useState<ReceiveResult | null>(null);
  const [recvErr, setRecvErr] = useState<string | null>(null);
  const recvAmount = recvStr.trim() ? parseUnits(recvStr, 8) : null;
  useEffect(() => {
    setRecv(null);
    setRecvErr(null);
  }, [network, key?.file, address]);

  const checkReceive = async () => {
    if ((!key && !(pub && address)) || recvAmount === null || recvAmount <= 0n) return;
    setRecvBusy(true);
    setRecv(null);
    setRecvErr(null);
    try {
      const r = key ? await receiveGhost(network, key.file, cliDecimal(recvAmount)) : await receiveGhostWallet(network, address!, cliDecimal(recvAmount));
      setRecv(r);
      if (r.ok && (r.found ?? 0) > 0) acc.refresh();
    } catch (e) {
      setRecvErr(e instanceof ApiError ? e.message : (e as Error).message);
    } finally {
      setRecvBusy(false);
    }
  };

  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          Wallet
        </h1>
        <span className="tag">{NETWORKS[network].label}</span>
      </div>
      <StatusNotices />
      <p className="lead">
        {pub
          ? tr(
              `Deine Wallet (Kastle oder KasWare) bleibt deine Wallet: Die Schlüssel liegen nur dort, K.Lend sieht sie nie. KAS und ${STABLE_SYMBOL} gehören zur selben Adresse. Gängige Wallets zeigen ${STABLE_SYMBOL} noch nicht an, weil es ein Covenant-Token ist. Hier siehst du beides und kannst senden; jede Sendung bestätigst du in deiner Wallet.`,
              `Your wallet (Kastle or KasWare) stays your wallet: the keys stay there, K.Lend never sees them. KAS and ${STABLE_SYMBOL} belong to the same address. Common wallets do not show ${STABLE_SYMBOL} yet because it is a covenant token. Here you see both and can send; you confirm every transfer in your wallet.`,
            )
          : tr(
          `Deine Schlüsseldatei ist deine Wallet. KAS und ${STABLE_SYMBOL} gehören zur selben Adresse. Gängige Wallets zeigen ${STABLE_SYMBOL} noch nicht an, weil es ein Covenant-Token ist. Hier siehst du beides, kannst senden und empfangen.`,
          `Your key file is your wallet. KAS and ${STABLE_SYMBOL} belong to the same address. Common wallets do not show ${STABLE_SYMBOL} yet because it is a covenant token. Here you see both and can send and receive.`,
        )}
      </p>

      <nav className="quick-grid section-sm" aria-label={tr("Was möchtest du tun?", "What do you want to do?")}>
        <a className="card quick" href={href("vault")}>
          <strong className="ghost-label">
            <GhostIcon size={22} /> {tr(`${STABLE_SYMBOL} prägen`, `Mint ${STABLE_SYMBOL}`)}
          </strong>
          <span className="small muted">{tr(`${NATIVE} als Sicherheit hinterlegen und Dollar-Stablecoin erzeugen`, `Deposit ${NATIVE} as collateral and create a dollar stablecoin`)}</span>
        </a>
        <a className="card quick" href={href("tauschen")}>
          <strong className="ghost-label">
            <KasIcon size={22} />
            <GhostIcon size={22} /> {tr("Tauschen", "Swap")}
          </strong>
          <span className="small muted">{tr(`${NATIVE} gegen ${STABLE_SYMBOL} und zurück, Liquidität einlegen`, `${NATIVE} for ${STABLE_SYMBOL} and back, add liquidity`)}</span>
        </a>
        <a
          className="card quick"
          href={href("wallet")}
          onClick={(e) => {
            // Hash-Routing: kein #senden-Anker, sondern zum Abschnitt scrollen
            e.preventDefault();
            document.getElementById("senden")?.scrollIntoView({ behavior: "smooth", block: "start" });
          }}
        >
          <strong>{tr("Senden und empfangen", "Send and receive")}</strong>
          <span className="small muted">{tr(`${NATIVE} und ${STABLE_SYMBOL} an eine Kaspa-Adresse, weiter unten`, `${NATIVE} and ${STABLE_SYMBOL} to a Kaspa address, further down`)}</span>
        </a>
      </nav>

      <div className="stack section-sm">
        {pub ? <WalletPanel /> : <AccountCard />}

        <section className="card" aria-labelledby="empfangen-title">
          <div className="card-head">
            <h2 id="empfangen-title">{tr("Empfangen", "Receive")}</h2>
          </div>
          {address ? (
            <>
              <div className="receive">
                <QrCode text={address} label={tr(`QR-Code der Adresse ${address}`, `QR code of address ${address}`)} />
                <div className="receive-text">
                  <p className="small muted">{tr(`Für KAS und ${STABLE_SYMBOL}:`, `For KAS and ${STABLE_SYMBOL}:`)}</p>
                  <code className="addr-big">{address}</code>
                  <div className="btn-row tight">
                    <CopyButton text={address} label={tr("Adresse", "Address")} />
                    <a className="btn btn-ghost btn-sm" href={explorerAddress(network, address)} target="_blank" rel="noreferrer noopener">
                      {tr("Im Explorer", "In explorer")}
                    </a>
                  </div>
                </div>
              </div>
              {(key || pub) && (
                <>
              <p className="small">
                {tr(
                  `KAS erscheinen von selbst. ${STABLE_SYMBOL} liegen in eigenen Token-UTXOs, die der Explorer unter deiner Adresse nicht zeigt. Kam die Sendung nicht über diese Seite, trag den Betrag ein, den dir der Absender nennt. Die Seite sucht dann genau diesen Token.`,
                  `KAS appear by themselves. ${STABLE_SYMBOL} sit in separate token UTXOs that the explorer does not show under your address. If the transfer did not come through this site, enter the amount the sender tells you. The page then looks for exactly that token.`,
                )}
              </p>
              <form
                className="receive-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void checkReceive();
                }}
              >
                <AmountInput
                  label={tr("Erwarteter Betrag", "Expected amount")}
                  value={recvStr}
                  onChange={setRecvStr}
                  suffix={STABLE_SYMBOL}
                  invalid={recvStr.trim() !== "" && recvAmount === null}
                  hint={`${tr("Muss genau stimmen, auf die letzte Stelle.", "Must match exactly, to the last digit.")}${recvAmount !== null && recvAmount > 0n ? ` ${usd(recvStr, STABLE_SYMBOL) ?? ""}` : ""}`}
                />
                <button type="submit" className="btn btn-ghost" disabled={recvBusy || !deployed || recvAmount === null || recvAmount <= 0n} aria-busy={recvBusy}>
                  {recvBusy ? tr("Suche …", "Searching …") : tr(`${STABLE_SYMBOL}-Eingang suchen`, `Find incoming ${STABLE_SYMBOL}`)}
                </button>
              </form>
              <div aria-live="polite">
                {!deployed && <p className="muted small">{tr("In diesem Netz ist GHOST noch nicht angelegt.", "GHOST is not deployed on this network yet.")}</p>}
                {recv?.ok && (recv.found ?? 0) > 0 && (
                  <Callout kind="info" title={tr("Gefunden", "Found")}>
                    {tr(
                      `${recv.found === 1 ? "Ein Eingang" : `${recv.found} Eingänge`} über ${recvStr} ${STABLE_SYMBOL} ist jetzt in deinem Guthaben.`,
                      `${recv.found === 1 ? "One transfer" : `${recv.found} transfers`} of ${recvStr} ${STABLE_SYMBOL} now in your balance.`,
                    )}
                  </Callout>
                )}
                {recv?.ok && (recv.found ?? 0) === 0 && (
                  <Callout kind="warn" title={tr("Nichts Neues gefunden", "Nothing new found")}>
                    {(recv.known ?? 0) > 0
                      ? tr("Ein Token mit genau diesem Betrag ist schon in deinem Guthaben.", "A token with exactly this amount is already in your balance.")
                      : tr(
                          "Kein Token mit genau diesem Betrag für diese Adresse. Stimmt der Betrag auf die letzte Stelle? Ist die Sendung schon bestätigt (einige Sekunden)?",
                          "No token with exactly this amount for this address. Is the amount correct to the last digit? Is the transfer confirmed yet (a few seconds)?",
                        )}
                  </Callout>
                )}
                {recv && !recv.ok && <Callout kind="danger" title={recv.nodeDown ? tr("Nodes nicht erreichbar", "Nodes unreachable") : tr("Fehler", "Error")}>{recv.error}</Callout>}
                {recvErr && <Callout kind="danger" title={tr("Fehler", "Error")}>{recvErr}</Callout>}
              </div>
                </>
              )}
            </>
          ) : (
            <p className="muted small">
              {pub
                ? tr("Erst die Wallet verbinden, dann steht hier deine Adresse.", "Connect your wallet first, then your address appears here.")
                : tr("Links eine Schlüsseldatei wählen oder anlegen.", "Choose or create a key file on the left.")}
            </p>
          )}
        </section>
      </div>

      <ActionForms prefill={null} actions={["transfer", "send"]} title={tr("Senden", "Send")} id="senden" />

      {/* Daueraufträge: öffentlich nur mit Tresor über die Browser-Wallet (Besitzer = Wallet,
          zahlen löst der Agent aus); mit Schlüsseldateien wie bisher. Nachrichten nutzen
          Schlüsseldateien des Servers – öffentlich gesperrt */}
      {pub ? <WalletTresor /> : <StandingOrders />}

      {!pub && <IncomingMessages />}

      <section className="card section-sm" aria-labelledby="verlauf-title">
        <div className="card-head">
          <h2 id="verlauf-title">{tr("Verlauf", "History")}</h2>
          <span className="tag">{tr("nur dieser Browser", "this browser only")}</span>
        </div>
        {log.length === 0 ? (
          <p className="muted small">
            {tr(
              "Noch nichts von dieser Seite gesendet. Der Verlauf liegt nur in diesem Browser. Vollständig ist er im Explorer unter deiner Adresse (KAS).",
              "Nothing sent from this page yet. The history is stored in this browser only. The complete history is in the explorer under your address (KAS).",
            )}
          </p>
        ) : (
          <div className="table-wrap">
            <table className="simple">
              <thead>
                <tr>
                  <th scope="col">{tr("Zeit", "Time")}</th>
                  <th scope="col">{tr("Aktion", "Action")}</th>
                  <th scope="col">{tr("Betrag", "Amount")}</th>
                  <th scope="col">{tr("Empfänger", "Recipient")}</th>
                  <th scope="col">{tr("Nachricht", "Message")}</th>
                  <th scope="col">{tr("Transaktion", "Transaction")}</th>
                </tr>
              </thead>
              <tbody>
                {log.map((e) => (
                  <tr key={`${e.at}-${e.txids[0] ?? ""}`}>
                    <td>{when(e.at)}</td>
                    <td>
                      {e.action !== "swap" ? ((actionMeta() as Record<string, { label: string }>)[e.action]?.label ?? e.label) : e.label}
                      {e.partial ? ` (${tr("teilweise", "partial")})` : ""}
                    </td>
                    <td>
                      {e.amount ? `${getLang() === "de" ? e.amount.replace(".", ",") : e.amount} ${e.unit ?? ""}` : "–"}
                      {e.amount && e.unit && <Usd amount={e.amount} unit={e.unit} />}
                    </td>
                    <td>{e.to ? <code title={e.to}>{shortAddr(e.to.replace(/^keys\//, ""))}</code> : "–"}</td>
                    <td>
                      {e.message ? `„${e.message}“` : "–"}
                      {e.message && e.onchain ? ` (${tr("öffentlich", "public")})` : ""}
                      {e.message && !e.onchain && e.encrypted ? ` (${tr("verschlüsselt an den Empfänger", "encrypted to the recipient")})` : ""}
                    </td>
                    <td>
                      {e.txids.length === 0
                        ? "–"
                        : e.txids.map((t) => (
                            <a key={t} href={explorerTx(network, t)} target="_blank" rel="noreferrer noopener" className="txlink">
                              {t.slice(0, 10)}…
                            </a>
                          ))}
                    </td>
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
