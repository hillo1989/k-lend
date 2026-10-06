import { useEffect, useState } from "react";
import { fetchMessages, type InboxMessage, type InboxResult } from "../lib/api";
import { useAccount } from "../lib/AccountContext";
import { useStatus } from "../lib/StatusContext";
import { explorerTx } from "../lib/txlog";
import { getLang, locale, tr } from "../lib/i18n";
import { Callout } from "./ui";
import { Usd } from "./Usd";

const shortAddr = (s: string) => (s.length > 24 ? `${s.slice(0, 14)}…${s.slice(-8)}` : s);

/**
 * Warum der Text einer Zahlung nicht angezeigt wird: zu lang (etwa JSON einer
 * anderen Anwendung) ist etwas anderes als unzulässige Zeichen (Restpunkt zu
 * A12-11). Ohne Angabe (älteres ghostctl) beide Möglichkeiten.
 */
function hiddenReason(m: Pick<InboxMessage, "kind" | "invalid">): string {
  if (m.kind === "unreadable") return tr("verschlüsselt, nicht für diesen Schlüssel lesbar", "encrypted, not readable with this key");
  switch (m.invalid) {
    case "length":
      return tr("Nachricht zu lang (über 100 Zeichen) – nicht angezeigt", "message too long (over 100 characters) – not shown");
    case "chars":
      return tr("Nachricht mit unsichtbaren oder unzulässigen Zeichen – nicht angezeigt", "message with invisible or disallowed characters – not shown");
    case "both":
      return tr("Nachricht zu lang (über 100 Zeichen) und mit unsichtbaren oder unzulässigen Zeichen – nicht angezeigt", "message too long (over 100 characters) and with invisible or disallowed characters – not shown");
    default:
      return tr("Nachricht zu lang oder mit unzulässigen Zeichen – nicht angezeigt", "message too long or with disallowed characters – not shown");
  }
}
const when = (ms: number | null) => (ms === null ? "–" : new Date(ms).toLocaleString(locale(), { dateStyle: "short", timeStyle: "short" }));

/**
 * Was sich über die Herkunft der Nachricht sagen lässt (A12-1). Tresor-Zahlungen
 * tragen die beim Anlegen hinterlegte Nachricht, der Vertrag erzwingt sie; wer
 * auslöst, kann sie nicht ändern.
 */
function OriginTag({ m }: { m: InboxMessage }) {
  const bound = tr("vom Absender beim Anlegen hinterlegt (vom Vertrag erzwungen)", "stored by the sender when creating the vault (enforced by the contract)");
  switch (m.origin) {
    case "tresor-stored":
      return <span className="tag">{bound}</span>;
    case "tresor-bound":
      return (
        <>
          <span className="tag">{bound}</span>
          {m.tresor && (
            <>
              {" "}
              <strong className="tag tag-warn">{tr(`weicht von der Beschreibung im Tresor-Code ${m.tresor} ab`, `differs from the description in vault code ${m.tresor}`)}</strong>
            </>
          )}
        </>
      );
    case "tresor-inserted":
      return (
        <strong className="tag tag-warn">
          {tr("passt nicht zur im Vertrag gebundenen Nachricht – nicht vom Absender", "does not match the message bound in the contract – not from the sender")}
        </strong>
      );
    case "contract":
      return <span className="tag tag-demo">{tr("über einen Vertrag – Herkunft der Nachricht nicht prüfbar", "via a contract – origin of the message cannot be checked")}</span>;
    default:
      return null;
  }
}

/**
 * Spalte „Von“: Schlüssel-Adressen unter den Eingängen. Bei einer Tresor-Zahlung
 * zuerst der Tresor; dessen Besitzer nur, wenn der Tresor-Code hier übernommen
 * wurde, und auch dann als Angabe des Codes – einen Tresor mit beliebigem
 * Besitzer kann jeder anlegen und auslösen, ohne dessen Signatur (A12-1).
 */
function FromCell({ m }: { m: InboxMessage }) {
  const addrs = m.from.map((f) => (
    <code key={f} title={f}>
      {shortAddr(f)}
    </code>
  ));
  if (!m.origin?.startsWith("tresor")) return <>{addrs.length === 0 ? tr("unbekannt", "unknown") : addrs}</>;
  return (
    <>
      <span className="muted">{m.tresor ? tr(`Tresor ${m.tresor}`, `vault ${m.tresor}`) : tr("Tresor (Besitzer nicht geprüft)", "vault (owner not verified)")}</span>
      {m.tresor && m.tresorOwner && (
        <div className="small muted">
          {tr("Besitzer laut Tresor-Code", "owner per vault code")}{" "}
          <code title={m.tresorOwner}>{shortAddr(m.tresorOwner)}</code>
        </div>
      )}
      {addrs.length > 0 && (
        <div className="small">
          {tr("weitere Eingänge", "other inputs")}: {addrs}
        </div>
      )}
    </>
  );
}

/**
 * Eingegangene Nachrichten an die Adresse des gewählten Schlüssels. ghostctl
 * liest die Transaktionen über die öffentliche REST-API, prüft sie am Node nach,
 * solange der Node den Block noch hat, und entschlüsselt mit der Schlüsseldatei
 * auf diesem Rechner; die Seite sieht den Schlüssel nie. Absender-Echtheit gibt
 * es nicht: jeder kann an eine Adresse verschlüsseln.
 */
export function IncomingMessages() {
  const { network } = useStatus();
  const acc = useAccount();
  const key = acc.selected;
  const [data, setData] = useState<InboxResult | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    setData(null);
    setErr(null);
    if (!key) return;
    const ctl = new AbortController();
    setLoading(true);
    fetchMessages(network, key.file, ctl.signal)
      .then((r) => setData(r))
      .catch((e: Error) => {
        if (!ctl.signal.aborted) setErr(e.message);
      })
      .finally(() => {
        if (!ctl.signal.aborted) setLoading(false);
      });
    return () => ctl.abort();
  }, [network, key?.file, tick]);

  const list = data?.messages ?? [];
  return (
    <section className="card section-sm" aria-labelledby="eingang-title">
      <div className="card-head">
        <h2 id="eingang-title">{tr("Eingegangene Nachrichten", "Received messages")}</h2>
        <button type="button" className="btn btn-ghost btn-sm" onClick={() => setTick((x) => x + 1)} disabled={!key || loading}>
          {loading ? tr("lädt …", "loading …") : tr("Aktualisieren", "Refresh")}
        </button>
      </div>
      {!key ? (
        <p className="muted small">{tr("Links eine Schlüsseldatei wählen.", "Choose a key file on the left.")}</p>
      ) : err ? (
        <Callout kind="warn">{err}</Callout>
      ) : data === null ? (
        <p className="muted small">{tr("Transaktionen werden gelesen …", "Reading transactions …")}</p>
      ) : list.length === 0 ? (
        <p className="muted small">
          {tr(`Keine Nachrichten in den letzten ${data.limit ?? 200} Transaktionen dieser Adresse.`, `No messages in the last ${data.limit ?? 200} transactions of this address.`)}
        </p>
      ) : (
        <div className="table-wrap">
          <table className="simple">
            <thead>
              <tr>
                <th scope="col">{tr("Zeit", "Time")}</th>
                <th scope="col">{tr("Betrag", "Amount")}</th>
                <th scope="col">{tr("Von", "From")}</th>
                <th scope="col">{tr("Nachricht", "Message")}</th>
                <th scope="col">{tr("Transaktion", "Transaction")}</th>
              </tr>
            </thead>
            <tbody>
              {list.map((m) => (
                <tr key={m.txid}>
                  <td>{when(m.timeMs)}</td>
                  <td>
                    {getLang() === "de" ? m.amount.replace(".", ",") : m.amount} {m.unit}
                    <Usd amount={m.amount} unit={m.unit} />
                  </td>
                  <td>
                    <FromCell m={m} />
                  </td>
                  <td>
                    {m.kind === "unreadable" || m.kind === "invalid" ? (
                      <>
                        <span className="muted">{hiddenReason(m)}</span>{" "}
                        <OriginTag m={m} />
                      </>
                    ) : (
                      <>
                        „{m.text}“{" "}
                        {m.kind === "public" ? (
                          <span className="tag tag-warn">{tr("öffentlich", "public")}</span>
                        ) : (
                          <span className="tag">{tr("verschlüsselt", "encrypted")}</span>
                        )}{" "}
                        <OriginTag m={m} />
                      </>
                    )}
                  </td>
                  <td>
                    <a href={explorerTx(network, m.txid)} target="_blank" rel="noreferrer noopener" className="txlink">
                      {m.txid.slice(0, 10)}…
                    </a>
                    <div className="small muted">{m.source === "node" ? tr("am Node geprüft", "checked at the node") : tr("laut REST-API", "per REST API")}</div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {(data?.hidden ?? 0) > 0 && (
        <Callout kind="warn">
          {tr(
            `${data?.hidden} Nachricht(en) ausgeblendet: Die öffentliche REST-API meldete sie anders, als der Node sie kennt – mit anderem Inhalt als im Block, in einem Block ohne diese Transaktion oder in einem Block, den der Node nicht kennt, obwohl er ihn noch haben müsste.`,
            `${data?.hidden} message(s) hidden: the public REST API reported them differently from what the node knows – with content other than in the block, in a block without this transaction, or in a block the node does not know although it should still have it.`,
          )}
        </Callout>
      )}
      {list.length > 0 && (
        <p className="small muted" title={(data?.checks ?? []).join("\n") || undefined}>
          {data?.nodeChecked
            ? tr(
                "Gelesen über die öffentliche REST-API. „am Node geprüft“: Dieselbe Transaktion steht so im Block am Node. Blöcke behält ein Node nur etwa 30 bis 42 Stunden, ältere Zahlungen stehen „laut REST-API“ da; ob eine Zahlung angenommen wurde, meldet in jedem Fall die REST-API.",
                "Read via the public REST API. “checked at the node”: the same transaction is in the block at the node. A node only keeps blocks for about 30 to 42 hours, older payments are shown “per REST API”; whether a payment was accepted is always reported by the REST API.",
              )
            : tr("Gelesen über die öffentliche REST-API; kein Node erreichbar, nichts gegengeprüft.", "Read via the public REST API; no node reachable, nothing cross-checked.")}
        </p>
      )}
      {(data?.notes ?? []).length > 0 && (
        <p className="small muted" title={(data?.notes ?? []).join("\n")}>
          {tr("Einige GHOST-Eingänge ließen sich gerade nicht prüfen. Später erneut versuchen.", "Some GHOST receipts could not be checked right now. Try again later.")}
        </p>
      )}
    </section>
  );
}
