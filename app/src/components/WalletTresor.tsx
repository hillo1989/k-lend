// Daueraufträge mit Tresor auf der öffentlichen Seite: anlegen, „Meine
// Tresore“, auffüllen und kündigen – jede Aktion signiert die Browser-Wallet
// (WalletSignFlow). Logik in src/lib/tresorWallet.ts, Server-Seite in
// server/walletActions.ts, Bau in protocol/src/wallet_ops.rs.
import { useEffect, useId, useState } from "react";
import { NETWORKS } from "../config";
import { MAX_MESSAGE_CHARS, messageProblem, schedule, type AboInterval } from "../lib/abo";
import { cliDecimal } from "../lib/commands";
import { parseUnits } from "../lib/format";
import { getLang, locale, tr } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { dueMs, formCount, leftLabel, localDateTime, suggestedFund, tresorIntervalLabel, tresorStatusLabel, utcDateTime, type TresorForm } from "../lib/tresor";
import {
  fetchWalletTresore,
  lastFirstDue,
  utcToday,
  walletCancelParams,
  walletTopupParams,
  walletTresorBasics,
  walletTresorActionable,
  walletTresorHints,
  walletTresorOpen,
  walletTresorParams,
  walletTresorStatus,
  type WalletTresor as WT,
} from "../lib/tresorWallet";
import { explorerTx } from "../lib/txlog";
import { useWallet } from "../wallet/WalletContext";
import { CopyButton } from "./CopyCode";
import { AmountInput, Callout } from "./ui";
import { WalletConnect, WalletSignFlow } from "./WalletSignFlow";

const POLL_MS = 60_000;
const kas = (a: string) => (getLang() === "de" ? a.replace(".", ",") : a);
const short = (s: string) => (s.length > 24 ? `${s.slice(0, 14)}…${s.slice(-8)}` : s);

/** Daueraufträge (Tresor) mit der Browser-Wallet */
export function WalletTresor() {
  const { network } = useStatus();
  const w = useWallet();
  const address = w.status === "connected" ? w.address : null;
  const isMain = network === "mainnet";
  const ids = { to: useId(), iv: useId(), days: useId(), start: useId(), endMode: useId(), count: useId(), msg: useId() };

  const [list, setList] = useState<WT[] | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const [panel, setPanel] = useState<{ id: string; kind: "topup" | "cancel" } | null>(null);
  const [topup, setTopup] = useState("");

  const [to, setTo] = useState("");
  const [amountStr, setAmountStr] = useState("");
  const [ivMode, setIvMode] = useState<"daily" | "weekly" | "monthly" | "days">("monthly");
  const [daysStr, setDaysStr] = useState("14");
  // Datum in UTC wie die Termine und die Prüfung in ghostctl (A19-8)
  const [start, setStart] = useState(() => utcToday());
  const [endMode, setEndMode] = useState<"none" | "count">("count");
  const [count, setCount] = useState("12");
  const [fundStr, setFundStr] = useState("");
  const [message, setMessage] = useState("");
  const [opened, setOpened] = useState(false);

  // „Meine Tresore“: beim Verbinden, Netzwechsel, nach dem Senden und jede Minute (Zahlungen des Agenten)
  useEffect(() => {
    if (!address) {
      setList(null);
      return;
    }
    const ctl = new AbortController();
    fetchWalletTresore(network, address, ctl.signal)
      .then((l) => {
        setList(l.ok ? (l.tresore ?? []) : null);
        setLoadErr(l.ok ? null : (l.error ?? tr("Liste nicht lesbar.", "List not readable.")));
      })
      .catch((e: Error) => {
        if (!ctl.signal.aborted) setLoadErr(e.message);
      });
    const t = window.setTimeout(() => setTick((x) => x + 1), POLL_MS);
    return () => {
      ctl.abort();
      window.clearTimeout(t);
    };
  }, [network, address, tick]);
  useEffect(() => setPanel(null), [network, address]);

  const today = utcToday();
  const amount = amountStr.trim() ? parseUnits(amountStr, 8) : null;
  const days = /^\d{1,4}$/.test(daysStr.trim()) && Number(daysStr) >= 1 && Number(daysStr) <= 3650 ? Number(daysStr) : null;
  const interval: AboInterval | null = ivMode === "days" ? (days === null ? null : { days }) : ivMode;
  const form: TresorForm = {
    to,
    amount,
    amountText: amountStr,
    interval,
    start,
    endMode,
    count,
    fund: fundStr.trim() ? parseUnits(fundStr, 8) : null,
    fundText: fundStr,
    message,
    onchain: true,
  };
  const built = walletTresorParams(form, address, network, today);
  const hints = walletTresorHints(form, w.balance);
  const suggestion = suggestedFund(amount, formCount(form));
  const firstDue = dueMs(start);
  const preview = interval && built.params ? schedule(start, interval, 4, null, formCount(form)) : [];
  const ivText =
    ivMode === "monthly"
      ? tr("monatlich", "monthly")
      : ivMode === "weekly"
        ? tr("wöchentlich", "weekly")
        : ivMode === "daily"
          ? tr("täglich", "daily")
          : tr(`alle ${days ?? "?"} Tage`, `every ${days ?? "?"} days`);
  const summary = built.params
    ? tr(
        `${kas(String(built.params.fund))} KAS in den Tresor, daraus ${kas(String(built.params.amount))} KAS ${ivText} an ${short(to.trim())}`,
        `${built.params.fund} KAS into the vault, from it ${built.params.amount} KAS ${ivText} to ${short(to.trim())}`,
      )
    : "";
  const refresh = (any: boolean) => {
    if (any) {
      setTick((x) => x + 1);
      void w.refresh();
    }
  };

  const item = (t: WT) => {
    const st = walletTresorStatus(t);
    const open = walletTresorOpen(t);
    const actionable = walletTresorActionable(t);
    const last = [...t.history].reverse().slice(0, 3);
    const p = panel?.id === t.covenantId ? panel.kind : null;
    const topParams = p === "topup" ? walletTopupParams(t, topup) : null;
    return (
      <li key={t.covenantId} className="abo-item">
        <div className="abo-head">
          <strong>
            {kas(t.amount)} KAS {tresorIntervalLabel(t)} → <code title={t.recipientAddress}>{short(t.recipientAddress)}</code>
          </strong>
          {t.pending ? (
            <span className="tag tag-warn">{tr("noch nicht bestätigt", "not confirmed yet")}</span>
          ) : (
            <span className={st === "due" || st === "empty" || st === "low" ? "tag tag-warn" : st === "active" ? "tag" : "tag tag-demo"}>{tresorStatusLabel(st)}</span>
          )}
        </div>
        <dl className="kv small">
          <div>
            <dt>{tr("Restbetrag im Tresor", "Balance left in the vault")}</dt>
            <dd>
              {kas(t.value)} KAS {open && t.left !== 0 && <span className="muted">· {tr(`reicht für ${t.covered} Zahlung(en)`, `covers ${t.covered} payment(s)`)}</span>}
            </dd>
          </div>
          {open && t.left !== 0 && (
            <div>
              <dt>{tr("Nächster Termin", "Next due date")}</dt>
              <dd>
                {localDateTime(t.nextDue, locale())} <span className="muted">· {utcDateTime(t.nextDue, locale())}</span>
              </dd>
            </div>
          )}
          <div>
            <dt>{tr("Verbleibende Zahlungen", "Remaining payments")}</dt>
            <dd>{leftLabel(t.left)}</dd>
          </div>
          <div>
            <dt>{tr("Höchstgebühr je Zahlung", "Maximum fee per payment")}</dt>
            <dd>{kas(t.maxFee)} KAS</dd>
          </div>
          {t.message && (
            <div>
              <dt>{tr("Nachricht", "Message")}</dt>
              <dd>
                „{t.message}“ <span className="tag tag-warn">{tr("öffentlich", "public")}</span>
              </dd>
            </div>
          )}
          <div>
            <dt>{tr("Tresor", "Vault")}</dt>
            <dd>
              <code title={t.covenantId}>{t.id}</code>
            </dd>
          </div>
        </dl>
        {(st === "low" || st === "empty") && (
          <p className="small muted">
            {tr(
              "Das Guthaben trägt die nächste Zahlung samt Netzgebühr und 1 KAS Rest nicht mehr; der K.Lend-Agent zahlt erst nach dem Auffüllen.",
              "The balance no longer covers the next payment plus network fee and 1 KAS remainder; the K.Lend agent only pays after a top-up.",
            )}
          </p>
        )}
        {t.pending && (
          <p className="small muted">
            {tr(
              "Eine Transaktion zu diesem Tresor ist gesendet, aber noch nicht bestätigt. Der Stand wird übernommen, sobald sie bestätigt ist (meist nach Sekunden, spätestens mit der nächsten Runde des K.Lend-Agenten). Bis dahin bitte nichts wiederholen.",
              "A transaction for this vault has been sent but is not confirmed yet. The state is taken over once it is confirmed (usually within seconds, at the latest with the next round of the K.Lend agent). Please do not repeat anything until then.",
            )}{" "}
            <a href={explorerTx(network, t.pending.txid)} target="_blank" rel="noreferrer noopener" className="txlink">
              {t.pending.txid.slice(0, 10)}…
            </a>
          </p>
        )}
        {st === "done" && <p className="small muted">{tr("Alle Zahlungen sind erledigt. Mit „Kündigen“ holst du den Rest zurück.", "All payments are done. Use “Cancel” to get the remainder back.")}</p>}
        {st === "missing" && <p className="small muted">{tr("Beim letzten Abgleich nicht auffindbar.", "Not found at the last check.")}</p>}
        {last.length > 0 && (
          <ul className="abo-hist small">
            {last.map((h, i) => (
              <li key={`${h.at}-${i}`}>
                {h.at}:{" "}
                {h.action === "pay" ? tr("Zahlung", "payment") : h.action === "open" ? tr("angelegt", "created") : h.action === "topup" ? tr("aufgefüllt", "topped up") : h.action === "cancel" ? tr("gekündigt", "cancelled") : h.action}
                {h.txid && (
                  <>
                    {" "}
                    <a href={explorerTx(network, h.txid)} target="_blank" rel="noreferrer noopener" className="txlink">
                      {h.txid.slice(0, 10)}…
                    </a>
                  </>
                )}
              </li>
            ))}
          </ul>
        )}
        {actionable && (
          <div className="btn-row tight">
            <button type="button" className="btn btn-ghost btn-sm" aria-expanded={p === "topup"} onClick={() => setPanel(p === "topup" ? null : { id: t.covenantId, kind: "topup" })}>
              {tr("Auffüllen", "Top up")}
            </button>
            <button type="button" className="btn btn-ghost btn-sm" aria-expanded={p === "cancel"} onClick={() => setPanel(p === "cancel" ? null : { id: t.covenantId, kind: "cancel" })}>
              {tr("Kündigen", "Cancel")}
            </button>
            {t.code && <CopyButton text={t.code} label={tr("Tresor-Code", "vault code")} />}
          </div>
        )}
        {p === "topup" && (
          <div className="tresor-panel">
            <AmountInput label={tr("KAS nachlegen", "Add KAS")} value={topup} onChange={setTopup} suffix="KAS" invalid={topup.trim() !== "" && topParams === null} />
            <WalletSignFlow
              network={network}
              action="tresor-topup"
              label={tr("Tresor auffüllen", "Top up vault")}
              params={topParams}
              problem={topParams ? null : tr("Betrag eingeben.", "Enter an amount.")}
              summary={topParams ? tr(`${kas(topParams.kas)} KAS in den Tresor ${t.id}`, `${topParams.kas} KAS into vault ${t.id}`) : ""}
              blocked={false}
              onDone={(any) => {
                if (any) setTopup("");
                refresh(any);
              }}
            />
          </div>
        )}
        {p === "cancel" && (
          <div className="tresor-panel">
            <Callout kind="warn">
              {tr(
                `Kündigen beendet den Tresor ${t.id}: Es gibt keine weiteren Zahlungen, der Rest (${kas(t.value)} KAS abzüglich Netzgebühr) geht an deine Adresse. Die Gebühr kommt aus dem Tresor.`,
                `Cancelling ends vault ${t.id}: no further payments, the remainder (${t.value} KAS minus network fee) goes to your address. The fee comes from the vault.`,
              )}
            </Callout>
            <WalletSignFlow
              network={network}
              action="tresor-cancel"
              label={tr("Tresor kündigen", "Cancel vault")}
              params={walletCancelParams(t)}
              problem={null}
              summary={tr(`Tresor ${t.id} kündigen`, `Cancel vault ${t.id}`)}
              blocked={false}
              onDone={(any) => {
                if (any) setPanel(null);
                refresh(any);
              }}
            />
          </div>
        )}
      </li>
    );
  };

  const running = (list ?? []).filter((t) => walletTresorOpen(t));
  const ended = (list ?? []).filter((t) => !walletTresorOpen(t));

  return (
    <section className="card section-sm" id="dauerauftraege" aria-labelledby="wallet-tresor-title">
      <div className="card-head">
        <h2 id="wallet-tresor-title">{tr("Daueraufträge (Tresor)", "Standing orders (vault)")}</h2>
        <span className={isMain ? "tag tag-warn" : "tag"}>{NETWORKS[network].label}</span>
      </div>
      <p className="small">
        {tr(
          "KAS regelmäßig senden, zum Beispiel die Miete: Du legst ein Guthaben in einen Tresor, und zu jedem Termin geht der feste Betrag an den Empfänger – auch wenn dein Gerät aus ist.",
          "Send KAS regularly, for example the rent: you put a balance into a vault, and on every due date the fixed amount goes to the recipient – even when your device is off.",
        )}
      </p>
      <ul className="hints">
        {walletTresorBasics().map((t, i) => (
          <li key={i} className="hint hint-info">
            {t}
          </li>
        ))}
      </ul>

      {!address ? (
        <WalletConnect />
      ) : (
        <>
          <h3 className="section-sm">{tr("Meine Tresore", "My vaults")}</h3>
          {loadErr && <Callout kind="warn">{loadErr}</Callout>}
          {list === null && !loadErr ? (
            <p className="muted small">{tr("lädt …", "loading …")}</p>
          ) : running.length === 0 ? (
            <p className="muted small">{tr("Mit dieser Adresse gibt es noch keinen laufenden Tresor.", "There is no running vault for this address yet.")}</p>
          ) : (
            <ul className="abo-list">{running.map(item)}</ul>
          )}
          {ended.length > 0 && (
            <details className="as-cmd">
              <summary>{tr(`Beendete Tresore (${ended.length})`, `Ended vaults (${ended.length})`)}</summary>
              <ul className="abo-list">{ended.map(item)}</ul>
            </details>
          )}
        </>
      )}

      <h3 className="section-sm">{tr("Neuer Tresor", "New vault")}</h3>
      <div className="actions-grid">
        <fieldset className="plain" disabled={!address}>
          <legend className="sr-only">{tr("Neuer Tresor", "New vault")}</legend>
          <div className="field">
            <label htmlFor={ids.to}>{tr("Empfänger", "Recipient")}</label>
            <div className="input-wrap">
              <input id={ids.to} value={to} onChange={(e) => setTo(e.target.value)} placeholder={isMain ? "kaspa:q…" : "kaspatest:q…"} spellCheck={false} autoComplete="off" />
            </div>
          </div>
          <AmountInput label={tr("Betrag je Zahlung", "Amount per payment")} value={amountStr} onChange={setAmountStr} suffix="KAS" invalid={amountStr.trim() !== "" && amount === null} />
          <div className="field">
            <label htmlFor={ids.iv}>{tr("Intervall", "Interval")}</label>
            <div className="input-wrap">
              <select id={ids.iv} value={ivMode} onChange={(e) => setIvMode(e.target.value as typeof ivMode)}>
                <option value="monthly">{tr("monatlich", "monthly")}</option>
                <option value="weekly">{tr("wöchentlich", "weekly")}</option>
                <option value="daily">{tr("täglich", "daily")}</option>
                <option value="days">{tr("alle … Tage", "every … days")}</option>
              </select>
            </div>
            {ivMode === "days" && (
              <div className={days === null ? "input-wrap mt-6 invalid" : "input-wrap mt-6"}>
                <input id={ids.days} aria-label={tr("Abstand in Tagen", "Interval in days")} inputMode="numeric" value={daysStr} onChange={(e) => setDaysStr(e.target.value)} />
                <span className="suffix" aria-hidden="true">
                  {tr("Tage", "days")}
                </span>
              </div>
            )}
            {ivMode === "monthly" && <div className="field-hint">{tr("Gleicher Kalendertag wie der erste Termin; in kürzeren Monaten der letzte Tag.", "Same day of the month as the first date; in shorter months the last day.")}</div>}
          </div>
          <div className="field">
            <label htmlFor={ids.start}>{tr("Erster Termin (Datum in UTC)", "First due date (date in UTC)")}</label>
            <div className="input-wrap">
              <input id={ids.start} type="date" min={today} max={lastFirstDue(today)} value={start} onChange={(e) => setStart(e.target.value)} />
            </div>
            {firstDue !== null && (
              <div className="field-hint">
                {tr(
                  `Fällig jeweils um 00:00 Uhr Weltzeit (UTC), die erste Zahlung also ab ${localDateTime(firstDue, locale())} deiner Zeit. Ausgelöst wird in der Regel wenige Minuten danach, solange der K.Lend-Agent läuft und das Guthaben reicht. Höchstens ein Jahr im Voraus.`,
                  `Due at 00:00 UTC each time, so the first payment from ${localDateTime(firstDue, locale())} your time. It is usually triggered a few minutes later, as long as the K.Lend agent is running and the balance suffices. At most one year ahead.`,
                )}
                {firstDue <= Date.now() &&
                  tr(" Dieser Termin ist schon erreicht: Die erste Zahlung kommt kurz nach dem Anlegen.", " This date has already been reached: the first payment follows shortly after creating the vault.")}
              </div>
            )}
          </div>
          <div className="field">
            <label htmlFor={ids.endMode}>{tr("Ende", "End")}</label>
            <div className="input-wrap">
              <select id={ids.endMode} value={endMode} onChange={(e) => setEndMode(e.target.value as typeof endMode)}>
                <option value="count">{tr("nach Anzahl", "after a number of payments")}</option>
                <option value="none">{tr("unbegrenzt", "unlimited")}</option>
              </select>
            </div>
            {endMode === "count" && (
              <div className="input-wrap mt-6">
                <input id={ids.count} aria-label={tr("Anzahl der Zahlungen", "Number of payments")} inputMode="numeric" value={count} onChange={(e) => setCount(e.target.value)} />
                <span className="suffix" aria-hidden="true">
                  {tr("mal", "times")}
                </span>
              </div>
            )}
          </div>
          <AmountInput
            label={tr("Startguthaben im Tresor", "Starting balance in the vault")}
            value={fundStr}
            onChange={setFundStr}
            suffix="KAS"
            invalid={fundStr.trim() !== "" && parseUnits(fundStr, 8) === null}
            hint={
              suggestion !== null
                ? tr(`Vorschlag: ${kas(cliDecimal(suggestion))} KAS (alle Zahlungen, Höchstgebühren und 1 KAS Reserve). Leer lassen übernimmt den Vorschlag.`, `Suggestion: ${cliDecimal(suggestion)} KAS (all payments, maximum fees and 1 KAS reserve). Leave empty to use it.`)
                : tr("Unbegrenzt: frei wählbar, auffüllen geht jederzeit.", "Unlimited: your choice, you can top up at any time.")
            }
          />
          <div className="field">
            <label htmlFor={ids.msg}>{tr("Öffentliche Nachricht (optional)", "Public message (optional)")}</label>
            <div className={messageProblem(message) ? "input-wrap invalid" : "input-wrap"}>
              <input id={ids.msg} value={message} onChange={(e) => setMessage(e.target.value)} maxLength={MAX_MESSAGE_CHARS * 2} placeholder={tr("z. B. Miete", "e.g. rent")} autoComplete="off" />
            </div>
            <div className="field-hint">
              {tr(
                "Mit der Browser-Wallet nur öffentlich: Sie steht bei jeder Zahlung für alle lesbar in der Blockchain. Verschlüsselte Nachrichten gibt es hier nicht.",
                "With the browser wallet only public: it is readable by anyone in the blockchain with every payment. Encrypted messages are not available here.",
              )}
            </div>
          </div>
          {hints.length > 0 && (
            <ul className="hints" aria-label={tr("Vorprüfung", "Pre-check")}>
              {hints.map((h, i) => (
                <li key={i} className={`hint hint-${h.level}`}>
                  {h.text}
                </li>
              ))}
            </ul>
          )}
        </fieldset>
        <div className="result-col" aria-live="polite">
          {preview.length > 0 && (
            <p className="small">
              {tr("Nächste Termine (deine Ortszeit)", "Next due dates (your local time)")}: <strong>{preview.map((d) => localDateTime(dueMs(d)!, locale())).join(" · ")}</strong>
              {preview.length === 4 ? " …" : ""}
            </p>
          )}
          <WalletSignFlow
            network={network}
            action="tresor-open"
            label={tr("Tresor anlegen", "Create vault")}
            params={built.params}
            problem={built.problem}
            summary={summary}
            blocked={false}
            onDone={(any) => {
              if (any) setOpened(true);
              refresh(any);
            }}
          />
          {opened && (
            <Callout kind="info">
              {tr(
                "Der neue Tresor erscheint unter „Meine Tresore“, sobald die Transaktion bestätigt ist – bis dahin mit dem Vermerk „noch nicht bestätigt“. Bitte nicht ein zweites Mal anlegen.",
                "The new vault appears under “My vaults” once the transaction is confirmed – until then marked “not confirmed yet”. Please do not create it a second time.",
              )}
            </Callout>
          )}
        </div>
      </div>
    </section>
  );
}
