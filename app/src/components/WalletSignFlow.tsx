// Signieren mit der Browser-Wallet für eine Aktion: Plan holen → in der
// Wallet signieren → prüfen → senden. Logik in src/wallet/actions.ts.
import { useEffect, useState } from "react";
import type { NetworkId } from "../config";
import type { ActionParams, CliAction } from "../lib/commands";
import { markBusy } from "../lib/busy";
import { de } from "../lib/status";
import { logTx } from "../lib/txlog";
import { tr } from "../lib/i18n";
import {
  addressProblem,
  buildBody,
  callWalletApi,
  canSendSigned,
  foreignOutputs,
  planProblem,
  reportLines,
  sendOutcome,
  signInWallet,
  submitBody,
  unclearSend,
  type BuildResult,
  type Fetch,
  type SubmitResult,
} from "../wallet/actions";
import { WALLET_NAMES } from "../wallet/providers";
import { useWallet } from "../wallet/WalletContext";
import { networkProblem } from "../probe/probe";
import { CopyButton } from "./CopyCode";
import { Callout } from "./ui";

/** Plan verfällt nach 2 Minuten (wie der Probelauf, A10-W-11) */
const PLAN_TTL_MS = 120_000;

type Phase = "idle" | "building" | "signing" | "sending";

export function WalletConnect() {
  const w = useWallet();
  const [searched, setSearched] = useState(false);
  useEffect(() => {
    const t = window.setTimeout(() => setSearched(true), 5_500);
    return () => window.clearTimeout(t);
  }, []);
  if (w.status === "connected") return null;
  return (
    <div className="wallet-connect">
      {w.installed.length === 0 ? (
        searched ? (
          <Callout kind="warn">
            {tr(
              "Keine Browser-Wallet gefunden (KasWare oder Kastle). Ist die Erweiterung installiert, entsperrt und für diese Seite erlaubt?",
              "No browser wallet found (KasWare or Kastle). Is the extension installed, unlocked and allowed on this site?",
            )}{" "}
            <button type="button" className="link-btn" onClick={w.rescan}>
              {tr("Erneut suchen", "Search again")}
            </button>
          </Callout>
        ) : (
          <p className="muted small">{tr("Suche Wallet …", "Looking for wallet …")}</p>
        )
      ) : (
        <div className="btn-row">
          {/* KasWare zuerst: in der Wallet-Probe bestätigt (05.10.2026), Kastle noch nicht */}
          {[...w.installed].sort((a) => (a === "kasware" ? -1 : 1)).map((k) => (
            <button key={k} type="button" className="btn btn-ghost" disabled={w.status === "connecting"} onClick={() => void w.connect(k)}>
              {tr(`Mit ${WALLET_NAMES[k]} verbinden`, `Connect ${WALLET_NAMES[k]}`)}
            </button>
          ))}
        </div>
      )}
      {w.error && <p className="small muted">{w.error}</p>}
    </div>
  );
}

export function WalletSignFlow({
  network,
  action,
  label,
  params,
  problem,
  summary,
  blocked,
  onDone,
}: {
  network: NetworkId;
  action: CliAction;
  label: string;
  /** Parameter für /api/wallet/build (ohne Schlüssel) oder null */
  params: ActionParams | null;
  problem: string | null;
  summary: string;
  blocked: boolean;
  /** nach dem Senden (Status neu laden) */
  onDone(sent: boolean): void;
}) {
  const w = useWallet();
  const [phase, setPhase] = useState<Phase>("idle");
  const [plan, setPlan] = useState<{ sig: string; at: number; b: BuildResult } | null>(null);
  const [checked, setChecked] = useState<{ signed: string; r: SubmitResult } | null>(null);
  const [sent, setSent] = useState<SubmitResult | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [, tick] = useState(0);
  const isMain = network === "mainnet";
  const address = w.status === "connected" ? w.address : null;
  const sig = params && address ? JSON.stringify([network, action, address, params]) : "";

  // Eingaben, Konto oder Netz geändert: alter Plan und alte Signatur gelten nicht mehr
  useEffect(() => {
    setChecked(null);
    if (plan && plan.sig !== sig) setPlan(null);
  }, [sig]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!plan) return;
    const left = PLAN_TTL_MS - (Date.now() - plan.at);
    if (left <= 0) return;
    const t = window.setTimeout(() => tick((x) => x + 1), left + 50);
    return () => window.clearTimeout(t);
  }, [plan]);

  if (w.status !== "connected" || !address || !w.kind) return <WalletConnect />;

  const walletProblem =
    (w.network && w.network.id !== network ? networkProblem(w.network.raw, network) : null) ?? addressProblem(address, network);
  const planFresh = plan !== null && plan.sig === sig && Date.now() - plan.at < PLAN_TTL_MS;
  const planOk = planFresh && plan!.b.ok;
  const fetchFn = fetch as unknown as Fetch;

  const getPlan = async () => {
    if (!params) return;
    setErr(null);
    setSent(null);
    setChecked(null);
    setPhase("building");
    try {
      const b = (await callWalletApi(fetchFn, "build", buildBody(network, action, address, params))) as unknown as BuildResult;
      const p = b.ok ? planProblem(b, network, address) : null;
      setPlan({ sig, at: Date.now(), b: p ? { ok: false, error: p } : b });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setPhase("idle");
    }
  };

  const signAndCheck = async () => {
    if (!plan?.b.plan) return;
    setErr(null);
    setPhase("signing");
    try {
      const signed = await signInWallet(w.kind!, window as unknown as { kasware?: unknown; kastle?: unknown }, plan.b);
      const r = (await callWalletApi(fetchFn, "submit", submitBody(network, plan.b.plan, signed, false))) as unknown as SubmitResult;
      setChecked({ signed, r });
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      setPhase("idle");
    }
  };

  const sendNow = async () => {
    if (!plan?.b.plan || !checked) return;
    setErr(null);
    setPhase("sending");
    const release = markBusy();
    try {
      const r = (await callWalletApi(fetchFn, "submit", submitBody(network, plan.b.plan, checked.signed, true), true)) as unknown as SubmitResult;
      setSent(r);
      setChecked(null);
      if (r.ok && r.sent) {
        setPlan(null);
        logTx({ at: Date.now(), network, key: `wallet:${address}`, action, label, amount: null, unit: null, to: typeof params?.to === "string" ? params.to : null, txids: r.txid ? [r.txid] : [] });
      }
      onDone(!!(r.sent || r.unclear || r.timeout));
    } catch (e) {
      // beim Senden heißt jeder Fehler „unklar“, nie „nicht gesendet“ (A17-7)
      setSent({ ok: false, unclear: true, error: `${unclearSend()} (${(e as Error).message})` });
      setChecked(null);
      onDone(true);
    } finally {
      release();
      setPhase("idle");
    }
  };

  const b = planFresh ? plan!.b : null;
  const others = b && b.ok ? foreignOutputs(b, address) : [];
  return (
    <div className="wallet-flow">
      <p className="small muted">
        {tr("Signiert wird in", "Signing in")} {WALLET_NAMES[w.kind]} · <code title={address}>{address.slice(0, 14)}…{address.slice(-6)}</code>
      </p>
      {walletProblem && <Callout kind="warn">{walletProblem}</Callout>}
      {problem && <p className="muted small">{problem}</p>}

      <div className="btn-row">
        <button type="button" className="btn btn-ghost" disabled={!params || !!walletProblem || blocked || phase !== "idle"} aria-busy={phase === "building"} onClick={() => void getPlan()}>
          {phase === "building" ? tr("Baue …", "Building …") : tr("1. Prüfen (Plan holen)", "1. Check (get plan)")}
        </button>
        <button type="button" className="btn btn-ghost" disabled={!planOk || !!walletProblem || blocked || phase !== "idle"} aria-busy={phase === "signing"} onClick={() => void signAndCheck()}>
          {phase === "signing" ? tr("Warte auf die Wallet …", "Waiting for the wallet …") : tr(`2. In ${WALLET_NAMES[w.kind]} signieren`, `2. Sign in ${WALLET_NAMES[w.kind]}`)}
        </button>
      </div>

      {plan && plan.sig === sig && !planFresh && plan.b.ok && <p className="muted small">{tr("Der Plan ist älter als 2 Minuten – bitte erneut prüfen.", "The plan is older than 2 minutes – please check again.")}</p>}
      {b && !b.ok && (
        <div className="result err">
          <strong>{b.nodeDown ? tr("Nodes nicht erreichbar", "Nodes unreachable") : tr("Nicht möglich", "Not possible")}</strong>
          <p>{b.error}</p>
        </div>
      )}
      {b && b.ok && (
        <div className="result ok">
          <strong>{tr("Plan fertig – nichts gesendet.", "Plan ready – nothing sent.")}</strong>
          <p className="small">
            <strong>{summary}</strong> · {tr("Gebühr", "Fee")} {de((b.feeSompi ?? 0) / 1e8, 8)} KAS · {b.signInputs?.length ?? 0} {tr("Eingänge signiert die Wallet", "inputs signed by the wallet")}
          </p>
          <ul className="tx-list small">
            {(b.outputs ?? []).map((o) => (
              <li key={o.index}>
                {tr("Ausgang", "Output")} {o.index}: {de(o.kas, 8)} KAS · {o.what}
                {o.what === "andere Adresse" ? ` → ${o.address}` : ""}
              </li>
            ))}
          </ul>
          {others.length > 0 && (
            <p className="small">{tr("Prüfe in der Wallet: Beträge und Empfänger müssen mit dieser Liste übereinstimmen.", "Check in the wallet: amounts and recipients must match this list.")}</p>
          )}
        </div>
      )}

      {checked && (
        <div className={checked.r.ok && checked.r.valid ? "result ok" : "result err"}>
          {reportLines(checked.r).map((l, i) => (
            <p key={i} className="small">
              {l}
            </p>
          ))}
        </div>
      )}

      <button type="button" className="btn btn-primary" disabled={!canSendSigned(checked?.r ?? null) || !planFresh || blocked || phase !== "idle"} aria-busy={phase === "sending"} onClick={() => void sendNow()}>
        {phase === "sending" ? tr("Sende … (bis zu 3 Minuten)", "Sending … (up to 3 minutes)") : isMain ? tr("3. Im Mainnet senden", "3. Send on mainnet") : tr("3. Senden", "3. Send")}
      </button>

      {err && <Callout kind="danger" title={tr("Fehler", "Error")}>{err}</Callout>}
      {sent && (
        <div className={sent.ok && sent.sent ? "result ok" : "result err"}>
          {sendOutcome(sent) === "confirmed" || sendOutcome(sent) === "pending" ? (
            <>
              <strong>
                {sendOutcome(sent) === "confirmed"
                  ? tr("Gesendet und bestätigt.", "Sent and confirmed.")
                  : tr("Gesendet – Bestätigung steht noch aus. Status prüfen, NICHT erneut senden.", "Sent – confirmation still pending. Check the status, do NOT send again.")}
              </strong>
              {sent.txid && (
                <div className="txid">
                  <code>{sent.txid}</code>
                  <CopyButton text={sent.txid} label="TXID" />
                </div>
              )}
            </>
          ) : sendOutcome(sent) === "unclear" ? (
            <>
              <strong>{tr("Ergebnis unklar – Status prüfen, NICHT sofort erneut senden", "Outcome unclear – check the status, do NOT send again right away")}</strong>
              <p>{sent.error}</p>
            </>
          ) : (
            <>
              <strong>{tr("Nicht gesendet", "Not sent")}</strong>
              <p>{sent.error ?? sent.report?.error}</p>
            </>
          )}
        </div>
      )}
    </div>
  );
}
