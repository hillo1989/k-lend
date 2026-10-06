import { useEffect, useId, useState } from "react";
import { NETWORKS } from "../config";
import { fetchAbos, runAction } from "../lib/api";
import { aboHints, aboParams, intervalLabel, localToday, MAX_MESSAGE_CHARS, messageProblem, PUBLIC_MESSAGE_LABEL, schedule, sendableMessage, statusLabel, type Abo, type AboAsset, type AboInterval, type AboList } from "../lib/abo";
import { useAccount } from "../lib/AccountContext";
import { cliDecimal } from "../lib/commands";
import { parseUnits } from "../lib/format";
import { useStatus } from "../lib/StatusContext";
import { explorerTx } from "../lib/txlog";
import { getLang, locale, tr } from "../lib/i18n";
import { boundMessageText, dueMs, formCount, localDateTime, suggestedFund, tresorHints, tresorIntervalLabel, tresorParams, utcDateTime, type Tresor } from "../lib/tresor";
import { CopyButton } from "./CopyCode";
import { TresorList } from "./TresorList";
import { AmountInput, Callout } from "./ui";
import { Usd } from "./Usd";
import { useUsd } from "../lib/usd";

const ABO_POLL_MS = 60_000;

const day = (d: string) => new Date(`${d}T00:00:00`).toLocaleDateString(locale(), { dateStyle: "medium" });
const shortTo = (s: string) => {
  const t = s.replace(/^keys\//, "");
  return t.length > 24 ? `${t.slice(0, 14)}…${t.slice(-8)}` : t;
};
const amountText = (a: string) => (getLang() === "de" ? a.replace(".", ",") : a);

/**
 * Daueraufträge: KAS oder GHOST in festen Abständen senden, optional mit
 * Nachricht. Angelegt und verwaltet über ghostctl abo; ausgeführt vom
 * GHOST-Agenten oder vom lokalen Server dieser Seite.
 */
export function StandingOrders() {
  const { network } = useStatus();
  const acc = useAccount();
  const key = acc.selected;
  const isMain = network === "mainnet";
  const ids = { exec: useId(), asset: useId(), to: useId(), toMode: useId(), iv: useId(), days: useId(), start: useId(), endMode: useId(), end: useId(), count: useId(), msg: useId() };

  const [list, setList] = useState<AboList | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [tick, setTick] = useState(0);

  const [exec, setExec] = useState<"local" | "tresor">("local");
  const [fundStr, setFundStr] = useState("");
  // Probelauf eines Tresors: gilt nur für genau diese Eingaben in genau diesem
  // Netz (A12-6); `tresor` = was ghostctl anlegen würde, für die Bestätigung
  const [check, setCheck] = useState<{ key: string; fee: number; fund: string; tresor: Tresor | null } | null>(null);
  const [newCode, setNewCode] = useState<string | null>(null);
  const [tresorTick, setTresorTick] = useState(0);
  const [asset, setAsset] = useState<AboAsset>("KAS");
  const [toMode, setToMode] = useState<"key" | "free">("free");
  const [toKey, setToKey] = useState("");
  const [toFree, setToFree] = useState("");
  const [amountStr, setAmountStr] = useState("");
  const usd = useUsd();
  const [ivMode, setIvMode] = useState<"daily" | "weekly" | "monthly" | "days">("monthly");
  const [daysStr, setDaysStr] = useState("14");
  const [start, setStart] = useState(() => localToday());
  const [endMode, setEndMode] = useState<"none" | "date" | "count">("none");
  const [end, setEnd] = useState("");
  const [count, setCount] = useState("12");
  const [message, setMessage] = useState("");
  const [onchain, setOnchain] = useState(false);

  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [confirmEnd, setConfirmEnd] = useState<string | null>(null);

  // Liste laden: beim Netzwechsel, nach Aktionen und jede Minute (Ausführungen des Agenten)
  useEffect(() => {
    const ctl = new AbortController();
    fetchAbos(network, ctl.signal)
      .then((l) => {
        setList(l);
        setLoadErr(l.ok ? null : (l.error ?? tr("Liste nicht lesbar.", "List not readable.")));
      })
      .catch((e: Error) => {
        if (!ctl.signal.aborted) setLoadErr(e.message);
      });
    const t = window.setTimeout(() => setTick((x) => x + 1), ABO_POLL_MS);
    return () => {
      ctl.abort();
      window.clearTimeout(t);
    };
  }, [network, tick]);
  useEffect(() => {
    const others = acc.keys.filter((x) => x.file !== key?.file);
    if (!others.some((x) => x.file === toKey)) setToKey(others[0]?.file ?? "");
  }, [acc.keys, key, toKey]);
  useEffect(() => {
    setResult(null);
    setNewCode(null);
    setCheck(null);
  }, [network, key?.file]);

  const today = localToday();
  const amount = amountStr.trim() ? parseUnits(amountStr, 8) : null;
  const days = /^\d{1,4}$/.test(daysStr.trim()) && Number(daysStr) >= 1 && Number(daysStr) <= 3650 ? Number(daysStr) : null;
  const interval: AboInterval | null = ivMode === "days" ? (days === null ? null : { days }) : ivMode;
  const form = {
    asset,
    to: toMode === "key" ? toKey : toFree,
    amount,
    amountText: amountStr,
    interval,
    start,
    endMode,
    end,
    count,
    message,
    onchain,
  };
  const tform = {
    to: form.to,
    amount,
    amountText: amountStr,
    interval,
    start,
    endMode: endMode === "count" ? ("count" as const) : ("none" as const),
    count,
    fund: fundStr.trim() ? parseUnits(fundStr, 8) : null,
    fundText: fundStr,
    message,
    onchain,
  };
  const isTresor = exec === "tresor";
  const built = isTresor ? tresorParams(tform, key, (u) => cliDecimal(u), today) : aboParams(form, key, (u) => cliDecimal(u), today);
  const hints = isTresor ? tresorHints(tform, key) : aboHints(form, key);
  const fundSuggestion = suggestedFund(amount, formCount(tform));
  const checkKey = (params: Record<string, string | number | boolean>) => JSON.stringify({ network, params });
  const builtKey = built.params ? checkKey(built.params) : null;
  // Probelauf gilt nur für genau diese Eingaben im selben Netz
  const confirmed = check !== null && check.key === builtKey;
  const preview =
    interval && built.params
      ? schedule(start, interval, 4, endMode === "date" ? end : null, endMode === "count" && /^\d+$/.test(count) ? Number(count) : null)
      : [];
  // Tresor: Termine liegen auf 00:00 UTC – in der Ortszeit mit Datum und Uhrzeit
  // jedes einzelnen Termins (westlich von UTC der Vortag, Sommerzeit; A12-9)
  const previewText = isTresor ? preview.map((d) => localDateTime(dueMs(d)!, locale())) : preview.map(day);
  const firstDue = isTresor ? dueMs(start) : null;

  /** Tresor: erst Probelauf (Gebühr zeigen), dann auf Knopfdruck anlegen */
  const openTresor = async (params: Record<string, string | number | boolean>, real: boolean) => {
    setBusy("tresor-open");
    setResult(null);
    try {
      const r = await runAction({ network, action: "tresor-open", params, dryRun: !real, confirmMainnet: real && isMain ? true : undefined });
      if (!r.ok) {
        setCheck(null);
        setResult({ ok: false, text: String(r.error ?? tr("Fehlgeschlagen.", "Failed.")) });
        return;
      }
      if (!real) {
        const fee = r.transactions?.[0]?.feeKas ?? 0;
        setCheck({ key: checkKey(params), fee, fund: String(params.fund), tresor: (r.tresor as Tresor | undefined) ?? null });
        return;
      }
      const t = r.tresor as Tresor | undefined;
      const first = t?.nextDue ?? dueMs(start);
      setCheck(null);
      setNewCode(t?.code ?? null);
      setResult({
        ok: true,
        text:
          first === null
            ? tr("Tresor angelegt.", "Vault created.")
            : tr(`Tresor angelegt. Erste Zahlung fällig ab ${localDateTime(first, locale())} (${utcDateTime(first, locale())}).`, `Vault created. First payment due from ${localDateTime(first, locale())} (${utcDateTime(first, locale())}).`),
      });
      setAmountStr("");
      setFundStr("");
      setMessage("");
      setOnchain(false);
      setTresorTick((x) => x + 1);
    } catch (e) {
      setResult({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(null);
    }
  };

  const act = async (action: string, params: Record<string, string | number | boolean>, what: string) => {
    setBusy(typeof params.id === "string" ? params.id : action);
    setResult(null);
    try {
      const r = await runAction({ network, action, params, dryRun: false, confirmMainnet: action === "abo-add" && isMain ? true : undefined });
      if (r.ok) {
        setResult({ ok: true, text: what });
        if (action === "abo-add") {
          setAmountStr("");
          setMessage("");
          setOnchain(false);
        }
      } else setResult({ ok: false, text: String(r.error ?? tr("Fehlgeschlagen.", "Failed.")) });
      setTick((x) => x + 1);
    } catch (e) {
      setResult({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(null);
      setConfirmEnd(null);
    }
  };

  const mine = (list?.abos ?? []).filter((a) => key !== null && a.key === key.file);
  const archived = (list?.archive ?? []).filter((a) => key !== null && a.key === key.file);
  const otherKeys = acc.keys.filter((x) => x.file !== key?.file);

  const item = (a: Abo, inArchive = false) => {
    const last = [...a.history].reverse().slice(0, 3);
    const planEnd = a.count !== null ? tr(`${a.count} Termine`, `${a.count} payments`) : a.end ? tr(`bis ${day(a.end)}`, `until ${day(a.end)}`) : tr("unbegrenzt", "unlimited");
    return (
      <li key={a.id} className="abo-item">
        <div className="abo-head">
          <strong>
            {amountText(a.amount)} {a.asset}
            <Usd amount={a.amount} unit={a.asset} /> → <code title={a.to}>{shortTo(a.to)}</code>
          </strong>
          <span className={a.status === "pausiert" ? "tag tag-warn" : a.status === "aktiv" || a.status === "läuft" ? "tag" : "tag tag-demo"}>{statusLabel(a.status)}</span>
        </div>
        <dl className="kv small">
          <div>
            <dt>{tr("Intervall", "Interval")}</dt>
            <dd>
              {intervalLabel(a.interval)} · {planEnd}
            </dd>
          </div>
          <div>
            <dt>{tr("Nächste Ausführung", "Next execution")}</dt>
            <dd>{a.nextDue && !inArchive ? day(a.nextDue) : "–"}</dd>
          </div>
          {a.message && (
            <div>
              <dt>{tr("Nachricht", "Message")}</dt>
              <dd>
                „{a.message}“{" "}
                {a.onchain ? (
                  <span className="tag tag-warn">{tr("öffentlich", "public")}</span>
                ) : a.encrypt ? (
                  <span className="tag">{tr("verschlüsselt", "encrypted")}</span>
                ) : (
                  <span className="tag tag-demo">{tr("nur auf diesem Rechner", "only on this computer")}</span>
                )}
                {(a.onchain || a.encrypt) && sendableMessage(a.message) !== a.message.trim() && (
                  <div className="small muted">
                    {sendableMessage(a.message)
                      ? tr(
                          `Enthält Zeichen, die nicht mehr gesendet werden (unsichtbar oder ohne feste Gestalt). Gesendet wird „${sendableMessage(a.message)}“.`,
                          `Contains characters that are no longer sent (invisible or without a fixed shape). Sent is “${sendableMessage(a.message)}”.`,
                        )
                      : tr("Enthält nur Zeichen, die nicht mehr gesendet werden; die Zahlungen gehen ohne Nachricht.", "Contains only characters that are no longer sent; payments go without a message.")}
                  </div>
                )}
              </dd>
            </div>
          )}
        </dl>
        {a.pauseReason && a.status === "pausiert" && <p className="small muted">{tr("Grund", "Reason")}: {a.pauseReason}</p>}
        {a.retry && <p className="small muted">{tr(`Termin ${day(a.retry.date)} wird erneut versucht (${a.retry.attempts} Fehlversuch(e)).`, `Due date ${day(a.retry.date)} will be retried (${a.retry.attempts} failed attempt(s)).`)}</p>}
        {last.length > 0 && (
          <ul className="abo-hist small">
            {last.map((h, i) => (
              <li key={`${h.at}-${i}`}>
                {day(h.date)}:{" "}
                {h.ok ? tr("gesendet", "sent") : h.error ? tr("fehlgeschlagen", "failed") : tr("Hinweis", "note")}
                {h.txid && (
                  <>
                    {" "}
                    <a href={explorerTx(network, h.txid)} target="_blank" rel="noreferrer noopener" className="txlink">
                      {h.txid.slice(0, 10)}…
                    </a>
                  </>
                )}
                {h.error && <span className="muted"> · {h.error}</span>}
                {h.note && <span className="muted"> · {h.note}</span>}
              </li>
            ))}
          </ul>
        )}
        {!inArchive && (
          <div className="btn-row tight">
            {a.paused ? (
              <button type="button" className="btn btn-ghost btn-sm" disabled={busy !== null} onClick={() => void act("abo-resume", { id: a.id }, tr("Dauerauftrag läuft wieder.", "Standing order resumed."))}>
                {tr("Fortsetzen", "Resume")}
              </button>
            ) : (
              a.status !== "abgeschlossen" && (
                <button type="button" className="btn btn-ghost btn-sm" disabled={busy !== null} onClick={() => void act("abo-pause", { id: a.id }, tr("Dauerauftrag pausiert.", "Standing order paused."))}>
                  {tr("Pausieren", "Pause")}
                </button>
              )
            )}
            {confirmEnd === a.id ? (
              <button type="button" className="btn btn-danger btn-sm" disabled={busy !== null} onClick={() => void act("abo-remove", { id: a.id }, tr("Dauerauftrag beendet.", "Standing order ended."))}>
                {tr("Wirklich beenden", "Really end")}
              </button>
            ) : (
              <button type="button" className="btn btn-ghost btn-sm" disabled={busy !== null} onClick={() => setConfirmEnd(a.id)}>
                {tr("Beenden", "End")}
              </button>
            )}
          </div>
        )}
      </li>
    );
  };

  return (
    <section className="card section-sm" id="dauerauftraege" aria-labelledby="dauerauftraege-title">
      <div className="card-head">
        <h2 id="dauerauftraege-title">{tr("Daueraufträge", "Standing orders")}</h2>
        <span className={isMain ? "tag tag-warn" : "tag"}>{NETWORKS[network].label}</span>
      </div>
      <p className="small">
        {tr(
          "KAS oder GHOST regelmäßig senden, zum Beispiel die Miete. Vom Rechner: ausgeführt wird, solange der GHOST-Agent oder diese Seite auf diesem Rechner läuft; verpasste Termine werden einmal nachgeholt. Mit Tresor: das Geld liegt vorab in einem Vertrag und wird auch gezahlt, wenn dein Rechner aus ist.",
          "Send KAS or GHOST regularly, for example the rent. From this computer: orders run while the GHOST agent or this page is running here; missed dates are made up once. With a vault: the money is placed in a contract beforehand and is paid even when your computer is off.",
        )}
      </p>

      {loadErr && <Callout kind="warn">{loadErr}</Callout>}
      {!key ? (
        <p className="muted small">{tr("Oben eine Schlüsseldatei wählen oder anlegen.", "Choose or create a key file above.")}</p>
      ) : mine.length === 0 ? (
        <p className="muted small">{tr("Für dieses Konto gibt es noch keine Daueraufträge.", "There are no standing orders for this account yet.")}</p>
      ) : (
        <ul className="abo-list">{mine.map((a) => item(a))}</ul>
      )}
      {archived.length > 0 && (
        <details className="as-cmd">
          <summary>{tr(`Beendete Aufträge (${archived.length})`, `Ended orders (${archived.length})`)}</summary>
          <ul className="abo-list">{archived.map((a) => item(a, true))}</ul>
        </details>
      )}

      <TresorList network={network} account={key} isMain={isMain} refresh={tresorTick} />

      <h3 className="section-sm">{tr("Neuer Dauerauftrag", "New standing order")}</h3>
      <form
        className="actions-grid"
        onSubmit={(e) => {
          e.preventDefault();
          if (built.params && isTresor) void openTresor(built.params, confirmed);
          else if (built.params)
            void act("abo-add", built.params, tr(`Dauerauftrag angelegt. Erste Ausführung: ${day(start)}.`, `Standing order created. First execution: ${day(start)}.`));
        }}
      >
        <fieldset className="plain" disabled={busy !== null || !key}>
          <legend className="sr-only">{tr("Neuer Dauerauftrag", "New standing order")}</legend>
          <div className="field" role="radiogroup" aria-label={tr("Ausführung", "Execution")}>
            <label className="radio">
              <input type="radio" name={ids.exec} checked={!isTresor} onChange={() => setExec("local")} /> {tr("Ausführung vom Rechner (wie bisher)", "Run from this computer (as before)")}
            </label>
            <label className="radio">
              <input
                type="radio"
                name={ids.exec}
                checked={isTresor}
                onChange={() => {
                  setExec("tresor");
                  setAsset("KAS");
                  if (endMode === "date") setEndMode("count");
                }}
              />{" "}
              {tr("Mit Tresor (zahlt auch, wenn dein Rechner aus ist)", "With a vault (pays even when your computer is off)")}
            </label>
          </div>
          {isTresor ? (
            <p className="small muted">{tr("Ein Tresor zahlt KAS.", "A vault pays KAS.")}</p>
          ) : (
            <div className="field" role="radiogroup" aria-label={tr("Was senden?", "What to send?")}>
              {(["KAS", "GHOST"] as const).map((x) => (
                <label className="radio" key={x}>
                  <input type="radio" name={ids.asset} checked={asset === x} onChange={() => setAsset(x)} /> {x}
                </label>
              ))}
            </div>
          )}

          <div className="field">
            <label htmlFor={ids.toMode}>{tr("Empfänger", "Recipient")}</label>
            <div className="input-wrap">
              <select id={ids.toMode} value={toMode} onChange={(e) => setToMode(e.target.value as "key" | "free")}>
                <option value="free">{asset === "KAS" ? tr("Kaspa-Adresse", "Kaspa address") : tr("Kaspa-Adresse oder x-only-Schlüssel", "Kaspa address or x-only key")}</option>
                <option value="key">{tr("Eigene Schlüsseldatei", "Own key file")}</option>
              </select>
            </div>
            {toMode === "key" ? (
              <div className="input-wrap mt-6">
                <select id={ids.to} aria-label={tr("Empfänger-Datei", "Recipient file")} value={toKey} onChange={(e) => setToKey(e.target.value)}>
                  {otherKeys.length === 0 && <option value="">{tr("keine weitere Datei", "no other file")}</option>}
                  {otherKeys.map((x) => (
                    <option key={x.file} value={x.file}>
                      {x.file.replace(/^keys\//, "")}
                    </option>
                  ))}
                </select>
              </div>
            ) : (
              <div className="input-wrap mt-6">
                <input
                  id={ids.to}
                  aria-label={tr("Empfänger", "Recipient")}
                  value={toFree}
                  onChange={(e) => setToFree(e.target.value)}
                  placeholder={isMain ? "kaspa:q…" : "kaspatest:q…"}
                  spellCheck={false}
                  autoComplete="off"
                />
              </div>
            )}
          </div>

          <AmountInput
            label={tr("Betrag je Ausführung", "Amount per execution")}
            value={amountStr}
            onChange={setAmountStr}
            suffix={asset}
            invalid={amountStr.trim() !== "" && amount === null}
            hint={amount !== null && amount > 0n ? (usd(amountStr, asset) ?? undefined) : undefined}
          />

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
            {ivMode === "monthly" && <div className="field-hint">{tr("Gleicher Kalendertag; in kürzeren Monaten der letzte Tag.", "Same day of the month; in shorter months the last day.")}</div>}
          </div>

          <div className="field">
            <label htmlFor={ids.start}>{tr("Erste Ausführung", "First execution")}</label>
            <div className="input-wrap">
              <input id={ids.start} type="date" min={today} value={start} onChange={(e) => setStart(e.target.value)} />
            </div>
            {firstDue !== null && (
              <div className="field-hint">
                {tr(
                  `Fällig jeweils am gewählten Kalendertag um 00:00 Uhr Weltzeit (UTC), die erste Zahlung also bei dir ab ${localDateTime(firstDue, locale())}. Mit Sommer- und Winterzeit verschiebt sich die Uhrzeit. Ausgezahlt wird, sobald jemand die Zahlung auslöst.`,
                  `Due on the chosen calendar day at 00:00 UTC each time, so the first payment from ${localDateTime(firstDue, locale())} your time. Daylight saving time shifts the local hour. It is paid out as soon as someone triggers the payment.`,
                )}
              </div>
            )}
          </div>

          <div className="field">
            <label htmlFor={ids.endMode}>{tr("Ende", "End")}</label>
            <div className="input-wrap">
              <select id={ids.endMode} value={endMode} onChange={(e) => setEndMode(e.target.value as typeof endMode)}>
                <option value="none">{tr("unbegrenzt", "unlimited")}</option>
                {!isTresor && <option value="date">{tr("bis Datum", "until date")}</option>}
                <option value="count">{tr("nach Anzahl", "after a number of payments")}</option>
              </select>
            </div>
            {endMode === "date" && (
              <div className="input-wrap mt-6">
                <input id={ids.end} aria-label={tr("Letzte mögliche Ausführung", "Last possible execution")} type="date" min={start} value={end} onChange={(e) => setEnd(e.target.value)} />
              </div>
            )}
            {endMode === "count" && (
              <div className="input-wrap mt-6">
                <input id={ids.count} aria-label={tr("Anzahl der Ausführungen", "Number of executions")} inputMode="numeric" value={count} onChange={(e) => setCount(e.target.value)} />
                <span className="suffix" aria-hidden="true">
                  {tr("mal", "times")}
                </span>
              </div>
            )}
          </div>

          {isTresor && (
            <AmountInput
              label={tr("Startguthaben im Tresor", "Starting balance in the vault")}
              value={fundStr}
              onChange={setFundStr}
              suffix="KAS"
              invalid={fundStr.trim() !== "" && parseUnits(fundStr, 8) === null}
              hint={
                fundSuggestion !== null
                  ? tr(`Vorschlag: ${amountText(cliDecimal(fundSuggestion))} KAS (alle Zahlungen, Gebühren und 1 KAS Reserve). Leer lassen übernimmt den Vorschlag.`, `Suggestion: ${cliDecimal(fundSuggestion)} KAS (all payments, fees and 1 KAS reserve). Leave empty to use it.`)
                  : tr("Unbegrenzt: frei wählbar, du kannst jederzeit auffüllen.", "Unlimited: your choice, you can top up at any time.")
              }
            />
          )}

          <div className="field">
            <label htmlFor={ids.msg}>{tr("Nachricht (optional)", "Message (optional)")}</label>
            <div className={messageProblem(message) ? "input-wrap invalid" : "input-wrap"}>
              <input id={ids.msg} value={message} onChange={(e) => setMessage(e.target.value)} maxLength={MAX_MESSAGE_CHARS * 2} placeholder={tr("z. B. Miete", "e.g. rent")} autoComplete="off" />
            </div>
          </div>
          <label className="checkbox">
            <input type="checkbox" checked={onchain} onChange={(e) => setOnchain(e.target.checked)} />
            <span>{PUBLIC_MESSAGE_LABEL()}</span>
          </label>

          {hints.length > 0 && (
            <ul className="hints" aria-label={tr("Vorprüfung", "Pre-check")}>
              {hints.map((h, i) => (
                <li key={i} className={`hint hint-${h.level}`}>
                  {h.text}
                </li>
              ))}
            </ul>
          )}

          <div className="btn-row">
            {isTresor ? (
              <button type="submit" className="btn btn-primary" disabled={!built.params || busy !== null} aria-busy={busy === "tresor-open"}>
                {busy === "tresor-open" ? tr("Einen Moment …", "One moment …") : confirmed ? tr("Jetzt anlegen", "Create now") : tr("Tresor prüfen", "Check vault")}
              </button>
            ) : (
              <button type="submit" className="btn btn-primary" disabled={!built.params || busy !== null} aria-busy={busy === "abo-add"}>
                {busy === "abo-add" ? tr("Lege an …", "Creating …") : tr("Dauerauftrag anlegen", "Create standing order")}
              </button>
            )}
          </div>
        </fieldset>

        <div className="result-col" aria-live="polite">
          {built.problem && <p className="muted small">{built.problem}</p>}
          {isTresor && confirmed && check && (
            <Callout kind={isMain ? "warn" : "info"} title={tr(`Geprüft im ${NETWORKS[network].label}`, `Checked on ${NETWORKS[network].label}`)}>
              {check.tresor
                ? tr(
                    `${amountText(check.fund)} KAS gehen in den Tresor (Netzgebühr ${amountText(check.fee.toFixed(4))} KAS). Daraus bekommt ${check.tresor.recipientAddress} ${amountText(check.tresor.amount)} KAS ${tresorIntervalLabel(check.tresor)}, ${check.tresor.left < 0 ? "ohne Ende" : `${check.tresor.left}-mal`}, erste Zahlung ab ${localDateTime(check.tresor.nextDue, locale())} (${utcDateTime(check.tresor.nextDue, locale())}). Je Zahlung höchstens ${amountText(check.tresor.maxFee)} KAS Gebühr aus dem Tresor. ${boundMessageText(check.tresor)} Mit „Jetzt anlegen“ wird gesendet.`,
                    `${check.fund} KAS go into the vault (network fee ${check.fee.toFixed(4)} KAS). From it ${check.tresor.recipientAddress} receives ${check.tresor.amount} KAS ${tresorIntervalLabel(check.tresor)}, ${check.tresor.left < 0 ? "without end" : `${check.tresor.left} times`}, first payment from ${localDateTime(check.tresor.nextDue, locale())} (${utcDateTime(check.tresor.nextDue, locale())}). At most ${check.tresor.maxFee} KAS fee per payment from the vault. ${boundMessageText(check.tresor)} “Create now” sends it.`,
                  )
                : tr(
                    `${amountText(check.fund)} KAS gehen in den Tresor, Netzgebühr ${amountText(check.fee.toFixed(4))} KAS. Mit „Jetzt anlegen“ wird gesendet.`,
                    `${check.fund} KAS go into the vault, network fee ${check.fee.toFixed(4)} KAS. “Create now” sends it.`,
                  )}
            </Callout>
          )}
          {preview.length > 0 && (
            <p className="small">
              {isTresor ? tr("Nächste Termine (deine Ortszeit)", "Next due dates (your local time)") : tr("Nächste Ausführungen", "Next executions")}: <strong>{previewText.join(isTresor ? " · " : ", ")}</strong>
              {preview.length === 4 ? " …" : ""}
            </p>
          )}
          {result && (
            <Callout kind={result.ok ? "info" : "danger"} title={result.ok ? undefined : tr("Fehler", "Error")}>
              {result.text}
            </Callout>
          )}
          {newCode && (
            <div className="btn-row tight">
              <CopyButton text={newCode} label={tr("Tresor-Code", "vault code")} />
              <span className="small">{tr("Tresor-Code kopieren und dem Empfänger geben – damit kann er jede fällige Zahlung selbst abholen.", "Copy the vault code and give it to the recipient – with it they can collect every due payment themselves.")}</span>
            </div>
          )}
        </div>
      </form>
    </section>
  );
}
