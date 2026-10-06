import { useEffect, useId, useState } from "react";
import type { NetworkId } from "../config";
import type { KeyEntry } from "../lib/api";
import { fetchTresore, runAction } from "../lib/api";
import { getLang, locale, tr } from "../lib/i18n";
import {
  codeProblem,
  feeSource,
  leftLabel,
  localDateTime,
  maxFeeTooLow,
  messageSure,
  payParams,
  topupKas,
  TRESOR_CODE_PREFIX,
  tresorIntervalLabel,
  tresorRole,
  tresorStatus,
  tresorStatusLabel,
  utcDateTime,
  type Tresor,
  type TresorList as TList,
} from "../lib/tresor";
import { explorerTx } from "../lib/txlog";
import { CopyButton } from "./CopyCode";
import { Callout } from "./ui";

const TRESOR_POLL_MS = 60_000;

const kas = (a: string) => (getLang() === "de" ? a.replace(".", ",") : a);
const short = (s: string) => (s.length > 24 ? `${s.slice(0, 14)}…${s.slice(-8)}` : s);

/**
 * Tresore (Dauerauftrag mit Tresor) des gewählten Kontos: als Absender mit
 * Auffüllen, Kündigen und dem Tresor-Code für den Empfänger; als Empfänger mit
 * „Fällige Zahlung abholen“ (mit Rückfrage; die Gebühr zahlt der Tresor, nur
 * wenn er sie nicht mehr trägt, nach ausdrücklicher Bestätigung der eigene
 * Schlüssel). Dazu das Feld für einen erhaltenen Tresor-Code.
 */
export function TresorList({ network, account, isMain, refresh }: { network: NetworkId; account: KeyEntry | null; isMain: boolean; refresh: number }) {
  const ids = { code: useId() };
  const [list, setList] = useState<TList | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [confirm, setConfirm] = useState<string | null>(null);
  const [topup, setTopup] = useState<Record<string, string>>({});
  const [code, setCode] = useState("");

  useEffect(() => {
    const ctl = new AbortController();
    fetchTresore(network, ctl.signal)
      .then((l) => {
        setList(l);
        setLoadErr(l.ok ? null : (l.error ?? tr("Liste nicht lesbar.", "List not readable.")));
      })
      .catch((e: Error) => {
        if (!ctl.signal.aborted) setLoadErr(e.message);
      });
    const t = window.setTimeout(() => setTick((x) => x + 1), TRESOR_POLL_MS);
    return () => {
      ctl.abort();
      window.clearTimeout(t);
    };
  }, [network, tick, refresh]);
  useEffect(() => setResult(null), [network, account?.file]);

  const act = async (tag: string, action: string, params: Record<string, string | number | boolean>, done: (r: Record<string, unknown>) => string) => {
    setBusy(tag);
    setResult(null);
    try {
      const sends = action !== "tresor-import" && action !== "tresor-sync";
      const r = await runAction({ network, action, params, dryRun: false, confirmMainnet: sends && isMain ? true : undefined });
      if (r.ok) {
        setResult({ ok: true, text: done(r) });
        if (action === "tresor-import") setCode("");
        if (action === "tresor-topup") setTopup((x) => ({ ...x, [String(params.id)]: "" }));
      } else setResult({ ok: false, text: String(r.error ?? tr("Fehlgeschlagen.", "Failed.")) });
      setTick((x) => x + 1);
    } catch (e) {
      setResult({ ok: false, text: (e as Error).message });
    } finally {
      setBusy(null);
      setConfirm(null);
    }
  };

  const all = list?.tresore ?? [];
  const xonly = account?.xonly ?? null;
  const own = all.filter((t) => tresorRole(t, xonly) === "owner");
  const incoming = all.filter((t) => tresorRole(t, xonly) === "recipient");
  const others = all.filter((t) => tresorRole(t, xonly) === "other");
  const codeErr = code.trim() ? codeProblem(code) : null;

  const item = (t: Tresor) => {
    const role = tresorRole(t, xonly);
    const st = tresorStatus(t);
    const open = st !== "cancelled" && st !== "missing";
    const last = [...t.history].reverse().slice(0, 3);
    const add = topup[t.id] ?? "";
    // an ghostctl geht der geprüfte Wert, nicht der Rohtext („1.000,5“ → 1000.5)
    const addText = topupKas(add);
    const fee = feeSource(t);
    const pay = payParams(t, account?.file ?? null);
    const sure = messageSure(t);
    return (
      <li key={t.covenantId} className="abo-item">
        <div className="abo-head">
          <strong>
            {kas(t.amount)} KAS {tresorIntervalLabel(t)}{" "}
            {role === "recipient" ? (
              <>
                {tr("von", "from")} <code title={t.ownerAddress}>{short(t.ownerAddress)}</code>
                {t.key === null && <span className="muted small"> {tr("(laut Tresor-Code)", "(per vault code)")}</span>}
              </>
            ) : (
              <>
                → <code title={t.recipientAddress}>{short(t.recipientAddress)}</code>
              </>
            )}
          </strong>
          <span className={st === "due" || st === "empty" || st === "low" ? "tag tag-warn" : st === "active" ? "tag" : "tag tag-demo"}>{tresorStatusLabel(st)}</span>
        </div>
        <dl className="kv small">
          <div>
            <dt>{tr("Guthaben im Tresor", "Balance in the vault")}</dt>
            <dd>
              {kas(t.value)} KAS{" "}
              {open && t.left !== 0 && <span className="muted">· {tr(`reicht für ${t.covered} Zahlung(en)`, `covers ${t.covered} payment(s)`)}</span>}
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
            <dd>
              {kas(t.maxFee)} KAS <span className="muted">· {tr("was davon nicht als Netzgebühr gebraucht wird, darf ein fremder Auslöser behalten", "whatever is not needed as network fee may be kept by a third-party trigger")}</span>
              {/* A13-tresor-4: darunter trägt der Tresor die Netzgebühr nicht sicher */}
              {maxFeeTooLow(t) && (
                <span className="tag tag-warn">
                  {tr("unter der Mindestgebühr einer Zahlung – auslösen meist nur mit eigenem Schlüssel", "below the minimum fee of a payment – usually only triggerable with your own key")}
                </span>
              )}
            </dd>
          </div>
          {(t.message || t.messageCleaned) && (
            <div>
              <dt>{tr("Nachricht", "Message")}</dt>
              <dd>
                {t.message && `„${t.message}“ `}
                {t.onchain && <span className="tag tag-warn">{tr("öffentlich", "public")}</span>}
                {!t.onchain && t.encrypted && <span className="tag">{tr("verschlüsselt", "encrypted")}</span>}
                {t.messageCheck === "mismatch" ? (
                  <span className="tag tag-warn">{tr("weicht von der Nachricht in den Zahlungen ab", "differs from the message in the payments")}</span>
                ) : sure ? (
                  t.key === null && <span className="tag tag-demo">{tr("laut Tresor-Code", "as stated in the vault code")}</span>
                ) : (
                  <span className="tag tag-demo">{tr("laut Tresor-Code, nicht geprüft", "as stated in the vault code, not verified")}</span>
                )}
                {/* älterer Tresor: Zeichen, die der heutige Filter ablehnt, sind nur in der Anzeige entfernt (Restpunkt zu A12-11) */}
                {t.messageCleaned && (
                  <span className="tag tag-demo">
                    {tr("bereinigt: enthielt unsichtbare oder heute unzulässige Zeichen", "cleaned: contained invisible or now disallowed characters")}
                  </span>
                )}
              </dd>
            </div>
          )}
        </dl>
        {/* A13-tresor-1: „genau sie“ nur, wenn der Vertrag die Beschreibung erzwingt oder sie geprüft ist */}
        {t.message && (
          <p className="small muted">
            {sure
              ? tr(
                  "Die Nachricht ist im Vertrag fest gebunden: Jede Zahlung trägt genau sie, wer auslöst, kann sie nicht ändern. Ändern lässt sie sich auch nachträglich nicht – nur mit einem neuen Tresor.",
                  "The message is fixed in the contract: every payment carries exactly this message, and whoever triggers a payment cannot change it. It cannot be changed later either – only with a new vault.",
                )
              : t.messageCheck === "mismatch"
                ? tr(
                    "Die verschlüsselte Nachricht, die jede Zahlung trägt, ist eine andere als diese Beschreibung – der Tresor-Code wurde verändert. Nicht auf die Beschreibung verlassen.",
                    "The encrypted message carried by every payment differs from this description – the vault code was altered. Do not rely on the description.",
                  )
                : tr(
                    "Im Vertrag gebunden ist die verschlüsselte Nachricht, die jede Zahlung trägt, nicht diese Beschreibung. Ob sie übereinstimmen, ließ sich ohne den Schlüssel des Empfängers nicht prüfen; was die Zahlungen wirklich tragen, zeigt der Eingang des Empfängers.",
                    "The contract binds the encrypted message carried by every payment, not this description. Whether they match could not be verified without the recipient's key; the recipient's inbox shows what the payments really carry.",
                  )}
            {t.key === null &&
              ` ${tr("Absender und Beschreibung stammen aus dem Tresor-Code; den Besitzer eines Tresors kann beim Anlegen jeder frei eintragen.", "Sender and description come from the vault code; anyone creating a vault can enter any owner.")}`}
          </p>
        )}
        {st === "missing" && <p className="small muted">{tr("Beim letzten Abgleich nicht auffindbar – vermutlich vom Absender gekündigt.", "Not found at the last check – probably cancelled by the sender.")}</p>}
        {st === "empty" && role === "owner" && (
          <p className="small muted">
            {tr("Das Guthaben deckt Betrag und Höchstgebühr nicht mehr; laut Vertrag kann niemand mehr zahlen. Bitte auffüllen.", "The balance no longer covers amount and maximum fee; per the contract nobody can pay any more. Please top up.")}
          </p>
        )}
        {/* ghostctl lässt 1 KAS im Tresor, der Vertrag nicht (A12-8) */}
        {st === "low" && (
          <p className="small muted">
            {role === "owner"
              ? tr(
                  "Das Guthaben reicht nicht mehr für eine Zahlung mit 1 KAS Rest: ghostctl und diese Seite zahlen erst nach dem Auffüllen. Ein fremder Auslöser kann noch zahlen, solange Betrag und Höchstgebühr gedeckt sind, und weniger Rest lassen. Bitte auffüllen.",
                  "The balance no longer covers a payment with 1 KAS remainder: ghostctl and this site only pay after a top-up. A third-party trigger can still pay as long as amount and maximum fee are covered, leaving less remainder. Please top up.",
                )
              : tr("Das Guthaben reicht nicht mehr für eine Zahlung mit 1 KAS Rest; ghostctl und diese Seite lösen erst aus, wenn der Absender auffüllt.", "The balance no longer covers a payment with 1 KAS remainder; ghostctl and this site only trigger once the sender tops up.")}
          </p>
        )}
        {open && t.left !== 0 && fee === "key" && (
          <p className="small muted">
            {role === "owner"
              ? tr(
                  "Mit der Höchstgebühr aus dem Tresor blieben weniger als 1 KAS darin. ghostctl und diese Seite zahlen deshalb nur noch, wenn der Auslöser die Gebühr selbst übernimmt; ein fremder Auslöser darf sie weiter aus dem Tresor nehmen. Bitte auffüllen.",
                  "With the maximum fee taken from the vault, less than 1 KAS would remain. ghostctl and this site therefore only pay if the trigger covers the fee; a third-party trigger may still take it from the vault. Please top up.",
                )
              : tr("Mit der Höchstgebühr aus dem Tresor blieben weniger als 1 KAS darin; abholen geht hier deshalb nur, wenn dein Schlüssel die Gebühr zahlt.", "With the maximum fee taken from the vault, less than 1 KAS would remain; collecting here therefore only works if your key pays the fee.")}
          </p>
        )}
        {st === "done" && role === "owner" && <p className="small muted">{tr("Alle Zahlungen sind erledigt. Mit „Kündigen“ holst du den Rest zurück.", "All payments are done. Use “Cancel” to get the remainder back.")}</p>}
        {t.lastError && open && <p className="small muted">{tr("Letzter Versuch", "Last attempt")}: {t.lastError}</p>}
        {last.length > 0 && (
          <ul className="abo-hist small">
            {last.map((h, i) => (
              <li key={`${h.at}-${i}`}>
                {h.at}:{" "}
                {h.action === "pay"
                  ? tr("Zahlung ausgelöst", "payment triggered")
                  : h.action === "open"
                    ? tr("angelegt", "created")
                    : h.action === "topup"
                      ? tr("aufgefüllt", "topped up")
                      : h.action === "cancel"
                        ? tr("gekündigt", "cancelled")
                        : tr("übernommen", "added")}
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
        {open && (
          <div className="btn-row tight">
            {st === "due" && pay && confirm !== `pay-${t.id}` && (
              <button type="button" className="btn btn-primary btn-sm" disabled={busy !== null} onClick={() => setConfirm(`pay-${t.id}`)}>
                {tr("Fällige Zahlung abholen", "Collect due payment")}
              </button>
            )}
            {st === "due" && pay && confirm === `pay-${t.id}` && (
              <>
                <span className="small">
                  {fee === "tresor"
                    ? tr(
                        `${kas(t.amount)} KAS an ${short(t.recipientAddress)}; die Netzgebühr (höchstens ${kas(t.maxFee)} KAS) trägt der Tresor.`,
                        `${t.amount} KAS to ${short(t.recipientAddress)}; the vault pays the network fee (at most ${t.maxFee} KAS).`,
                      )
                    : tr(
                        `${kas(t.amount)} KAS an ${short(t.recipientAddress)}; die Netzgebühr zahlt dein Schlüssel ${account?.file.replace(/^keys\//, "") ?? ""}.`,
                        `${t.amount} KAS to ${short(t.recipientAddress)}; your key ${account?.file.replace(/^keys\//, "") ?? ""} pays the network fee.`,
                      )}
                </span>
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  disabled={busy !== null}
                  aria-busy={busy === `pay-${t.id}`}
                  onClick={() =>
                    void act(`pay-${t.id}`, "tresor-pay", pay, () => tr(`Zahlung über ${kas(t.amount)} KAS ausgelöst.`, `Payment of ${t.amount} KAS triggered.`))
                  }
                >
                  {busy === `pay-${t.id}` ? tr("Löse aus …", "Triggering …") : tr("Wirklich abholen", "Really collect")}
                </button>
              </>
            )}
            {role === "owner" && (
              <>
                <CopyButton text={t.code} label={tr("Tresor-Code", "vault code")} />
                <span className="small muted">{tr("Tresor-Code für den Empfänger", "Vault code for the recipient")}</span>
              </>
            )}
          </div>
        )}
        {open && role === "owner" && account && (
          <div className="tresor-topup">
            <div className={add.trim() && addText === null ? "input-wrap invalid" : "input-wrap"}>
              <input aria-label={tr("KAS nachlegen", "KAS to add")} inputMode="decimal" value={add} placeholder="10" onChange={(e) => setTopup((x) => ({ ...x, [t.id]: e.target.value }))} />
              <span className="suffix" aria-hidden="true">
                KAS
              </span>
            </div>
            {confirm === `topup-${t.id}` ? (
              <button
                type="button"
                className="btn btn-primary btn-sm"
                disabled={busy !== null || addText === null}
                onClick={() => addText && void act(`topup-${t.id}`, "tresor-topup", { id: t.id, key: account.file, kas: addText }, () => tr("Tresor aufgefüllt.", "Vault topped up."))}
              >
                {tr(`Wirklich ${kas(addText ?? "")} KAS nachlegen`, `Really add ${addText ?? ""} KAS`)}
              </button>
            ) : (
              <button type="button" className="btn btn-ghost btn-sm" disabled={busy !== null || addText === null} onClick={() => setConfirm(`topup-${t.id}`)}>
                {tr("Auffüllen", "Top up")}
              </button>
            )}
            {confirm === `cancel-${t.id}` ? (
              <button
                type="button"
                className="btn btn-danger btn-sm"
                disabled={busy !== null}
                onClick={() => void act(`cancel-${t.id}`, "tresor-cancel", { id: t.id, key: account.file }, (r) => tr(`Tresor gekündigt, ${String(r.back ?? "")} KAS zurück.`, `Vault cancelled, ${String(r.back ?? "")} KAS returned.`))}
              >
                {tr(`Wirklich kündigen (${kas(t.value)} KAS zurück)`, `Really cancel (${t.value} KAS back)`)}
              </button>
            ) : (
              <button type="button" className="btn btn-ghost btn-sm" disabled={busy !== null} onClick={() => setConfirm(`cancel-${t.id}`)}>
                {tr("Kündigen", "Cancel")}
              </button>
            )}
          </div>
        )}
      </li>
    );
  };

  return (
    <div className="section-sm">
      <div className="abo-head">
        <h3>{tr("Tresore", "Vaults")}</h3>
        <button type="button" className="btn btn-ghost btn-sm" disabled={busy !== null} aria-busy={busy === "sync"} onClick={() => void act("sync", "tresor-sync", {}, () => tr("Tresore abgeglichen.", "Vaults updated."))}>
          {busy === "sync" ? tr("Gleiche ab …", "Updating …") : tr("Aktualisieren", "Refresh")}
        </button>
      </div>
      {loadErr && <Callout kind="warn">{loadErr}</Callout>}
      {own.length === 0 && incoming.length === 0 && <p className="muted small">{tr("Für dieses Konto gibt es keine Tresore.", "There are no vaults for this account.")}</p>}
      {own.length > 0 && (
        <>
          <p className="small">
            <strong>{tr("Deine Tresore", "Your vaults")}</strong>
          </p>
          <ul className="abo-list">{own.map(item)}</ul>
        </>
      )}
      {incoming.length > 0 && (
        <>
          <p className="small">
            <strong>{tr("Tresore mit Zahlungen an dich", "Vaults paying you")}</strong>
          </p>
          <ul className="abo-list">{incoming.map(item)}</ul>
        </>
      )}
      {others.length > 0 && (
        <details className="as-cmd">
          <summary>{tr(`Weitere Tresore auf diesem Rechner (${others.length})`, `Other vaults on this computer (${others.length})`)}</summary>
          <ul className="abo-list">{others.map(item)}</ul>
        </details>
      )}

      <div className="field">
        <label htmlFor={ids.code}>{tr("Tresor-Code einfügen", "Paste vault code")}</label>
        <div className={codeErr ? "input-wrap invalid" : "input-wrap"}>
          <textarea id={ids.code} rows={3} value={code} onChange={(e) => setCode(e.target.value)} placeholder={`${TRESOR_CODE_PREFIX}…`} spellCheck={false} autoComplete="off" />
        </div>
        <div className="field-hint">
          {codeErr ??
            tr(
              "Den Code bekommst du vom Absender. Danach siehst du den Tresor hier und kannst jede fällige Zahlung selbst abholen.",
              "You get the code from the sender. Afterwards you see the vault here and can collect every due payment yourself.",
            )}
        </div>
      </div>
      <div className="btn-row">
        <button
          type="button"
          className="btn btn-ghost"
          disabled={busy !== null || !code.trim() || codeErr !== null}
          aria-busy={busy === "import"}
          onClick={() =>
            void act("import", "tresor-import", { code: code.trim() }, (r) => {
              const t = r.tresor as Tresor | undefined;
              return t ? tr(`Tresor übernommen: ${kas(t.amount)} KAS ${tresorIntervalLabel(t)}.`, `Vault added: ${t.amount} KAS ${tresorIntervalLabel(t)}.`) : tr("Tresor übernommen.", "Vault added.");
            })
          }
        >
          {busy === "import" ? tr("Prüfe …", "Checking …") : tr("Tresor übernehmen", "Add vault")}
        </button>
      </div>
      {result && (
        <Callout kind={result.ok ? "info" : "danger"} title={result.ok ? undefined : tr("Fehler", "Error")}>
          {result.text}
        </Callout>
      )}
    </div>
  );
}
