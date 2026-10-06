import { useEffect, useId, useMemo, useRef, useState } from "react";
import { NETWORKS } from "../config";
import { runAction, type ActionResult } from "../lib/api";
import { useAccount } from "../lib/AccountContext";
import { actionMeta, ACTION_ORDER, cliDecimal, commandFor, type ActionParams, type CliAction } from "../lib/commands";
import { amountProblem, formatUnits, parseUnits } from "../lib/format";
import { markBusy } from "../lib/busy";
import { MAX_MESSAGE_CHARS, PUBLIC_MESSAGE_LABEL, encryptProblem, messageProblem } from "../lib/abo";
import { precheck } from "../lib/precheck";
import { useStatus } from "../lib/StatusContext";
import { de, vaultLabel } from "../lib/status";
import { logTx } from "../lib/txlog";
import { tr } from "../lib/i18n";
import { CopyButton, CopyCode } from "./CopyCode";
import { AmountInput, Callout } from "./ui";
import { useUsd } from "../lib/usd";
import { Usd } from "./Usd";
import { useWallet } from "../wallet/WalletContext";
import { toWalletParams, walletKeyEntry, walletSupports } from "../wallet/actions";
import { xOnlyKey } from "../lib/status";
import { WalletSignFlow } from "./WalletSignFlow";
import { isKName, useKName } from "../lib/kname";

/** Wer signiert: lokale Schlüsseldatei (ghostctl) oder Browser-Wallet */
export type SignMode = "key" | "wallet";

export interface Prefill {
  action: CliAction;
  vault?: number;
  nonce: number;
}

type Phase = "idle" | "checking" | "sending";

interface Checked {
  sig: string;
  result: ActionResult;
  /** Zeitpunkt des Probelaufs; nach CHECK_TTL_MS muss neu geprüft werden (A10-W-11) */
  at: number;
}

const CHECK_TTL_MS = 120_000;

/** Mindestwert aus einem Probelauf-Ergebnis: 1 % Spielraum, abgerundet */
function minFromDry(v: unknown): string | null {
  if (typeof v !== "number" || !Number.isFinite(v) || v < 0) return null;
  const units = (BigInt(Math.round(v * 1e8)) * 99n) / 100n;
  return cliDecimal(units);
}

/**
 * Formulare für alle Protokoll-Aktionen. Ablauf: ausfüllen → „Prüfen“
 * (Probelauf, nichts wird gesendet) → „Senden“ ist nur für genau die
 * geprüften Werte frei. Im Mainnet zusätzlich ausdrückliche Bestätigung.
 */
export function ActionForms({
  prefill,
  actions = ACTION_ORDER,
  title,
  id = "aktionen",
  ownVaults = false,
}: {
  prefill: Prefill | null;
  /** Welche Aktionen dieses Formular anbietet (Wallet-Seite: nur Senden) */
  actions?: CliAction[];
  title?: string;
  id?: string;
  /** nur die Vaults des gewählten Schlüssels anbieten (Bereich „Eigener Vault“) */
  ownVaults?: boolean;
}) {
  const { network, status, refresh: refreshStatus, nodeDown, updatedAt } = useStatus();
  const acc = useAccount();
  const live = status?.deployed ? status : null;
  const wallet = useWallet();
  const usdOf = useUsd();
  // Browser-Wallet: Standard, wenn der Server keine Schlüsseldateien zeigt
  // (öffentlicher Modus); sonst bleibt die Schlüsseldatei der Standard
  const [modeChoice, setModeChoice] = useState<SignMode | null>(null);
  const walletOk = actions.some(walletSupports);
  const keyOk = acc.keys.length > 0;
  const signMode: SignMode = !walletOk ? "key" : !keyOk ? "wallet" : (modeChoice ?? "key");
  const walletX = xOnlyKey(wallet.publicKey);
  const walletKey =
    wallet.status === "connected" && wallet.address
      ? walletKeyEntry(wallet.address, walletX, wallet.balance, null, (status?.deployed ? status.vaults : []).filter((v) => walletX !== null && v.owner === walletX).map((v) => v.index))
      : null;
  // „key“ = wer handelt: Schlüsseldatei oder Wallet-Konto (Vorprüfung, eigene Vaults)
  const key = signMode === "wallet" ? walletKey : acc.selected;

  const [action, setAction] = useState<CliAction>(actions.includes("mint") ? "mint" : actions[0]);
  const [vault, setVault] = useState<number | null>(null);
  const [amountStr, setAmountStr] = useState("");
  const [amount2Str, setAmount2Str] = useState("");
  const [fullAmount, setFullAmount] = useState(true);
  const [toMode, setToMode] = useState<"key" | "free">("key");
  const [toKey, setToKey] = useState("");
  const [toFree, setToFree] = useState("");
  // .k-Name im Empfängerfeld (dotk.name), vom Server am Node geprüft
  const kname = useKName(toMode === "free" || signMode === "wallet" ? toFree : "", network);
  const [committee, setCommittee] = useState("");
  const [usdStr, setUsdStr] = useState("");
  const [rateStr, setRateStr] = useState("");
  // Senden: Nachricht (lokal im Verlauf) und ob sie öffentlich in die Tx kommt
  const [message, setMessage] = useState("");
  const [onchain, setOnchain] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");
  // Ausgang unklar (Zeitüberschreitung o. Ä.): Senden gesperrt, bis ein neuer Status da ist (A10-W-2)
  const [unclearAt, setUnclearAt] = useState<number | null>(null);
  const [, setExpiryTick] = useState(0);
  const [checked, setChecked] = useState<Checked | null>(null);
  const [sent, setSent] = useState<ActionResult | null>(null);
  const [apiError, setApiError] = useState<string | null>(null);
  const panelRef = useRef<HTMLElement>(null);
  const ids = { vault: useId(), to: useId(), toMode: useId(), com: useId(), msg: useId() };

  const META = actionMeta();
  const meta = META[action];
  const isMain = network === "mainnet";
  const hasMessage = action === "send" || action === "transfer";

  // Vorbelegung aus der Vault-Liste bzw. Orakel-Karte
  useEffect(() => {
    if (!prefill) return;
    setAction(prefill.action);
    if (prefill.vault !== undefined) setVault(prefill.vault);
    setAmountStr("");
    setFullAmount(true);
    panelRef.current?.scrollIntoView({ block: "start" });
    panelRef.current?.querySelector<HTMLElement>("h2")?.focus({ preventScroll: true });
  }, [prefill]);

  // Standardwerte, wenn Daten da sind
  useEffect(() => {
    if (!live) return;
    // „Eigener Vault“: nur Vaults des gewählten Schlüssels bzw. der Wallet.
    // Wechselt der Signierer (Wallet verbunden, Modus umgestellt), springt die
    // Auswahl auf einen eigenen Vault statt auf einem fremden stehen zu bleiben.
    const sel = vault === null ? undefined : live.vaults.find((v) => v.index === vault);
    const foreign = ownVaults && sel !== undefined && (key === null || sel.owner.toLowerCase() !== key.xonly.toLowerCase());
    if (sel === undefined || foreign) {
      const own = key ? live.vaults.find((v) => v.owner.toLowerCase() === key.xonly.toLowerCase()) : undefined;
      const next = own?.index ?? (ownVaults ? null : (live.vaults[0]?.index ?? null));
      if (next !== vault) setVault(next);
    }
  }, [live, key, vault, ownVaults]);
  useEffect(() => {
    if (!committee || !acc.committees.some((c) => c.file === committee)) {
      const c = acc.committees.find((x) => acc.isNetworkKey(x.file)) ?? acc.committees[0];
      setCommittee(c?.file ?? "");
    }
  }, [acc.committees, acc.isNetworkKey, committee]);
  useEffect(() => {
    const others = acc.keys.filter((x) => x.file !== key?.file);
    if (!others.some((x) => x.file === toKey)) setToKey(others[0]?.file ?? "");
  }, [acc.keys, key, toKey]);
  // Netzwechsel: Bestätigung zurücksetzen
  useEffect(() => {
    setChecked(null);
    setSent(null);
  }, [network]);

  // ---- Eingaben → Parameter (Punkt-Dezimal wie ghostctl)
  // Tilgen und Liquidieren: „alles“ = ohne Betrag (ghostctl nimmt die ganze Schuld)
  const hasFullMode = action === "repay" || action === "liquidate";
  const useFull = hasFullMode && fullAmount;
  const amount = meta.amount && !useFull ? parseUnits(amountStr, 8) : null;
  const amount2 = meta.amount2 ? parseUnits(amount2Str, 8) : null;
  const usd = usdStr.trim() ? parseUnits(usdStr, 8) : null;
  const rate = rateStr.trim() ? parseUnits(rateStr, 2) : null;

  const built = useMemo((): { params: ActionParams | null; problem: string | null } => {
    if (signMode === "wallet" && !walletSupports(action)) return { params: null, problem: tr("Diese Aktion geht nur mit Schlüsseldatei.", "This action needs a key file.") };
    if (signMode === "key" && !key) return { params: null, problem: tr("Kein Schlüssel gewählt – unter „Wallet“ eine Schlüsseldatei wählen.", "No key selected – choose a key file under “Wallet”.") };
    const p: ActionParams = signMode === "key" && key ? { key: key.file } : {};
    if (meta.vault) {
      if (vault === null) return { params: null, problem: tr("Vault wählen.", "Choose a vault.") };
      p.vault = vault;
    }
    if (meta.to) {
      const t = toMode === "key" && signMode === "key" ? toKey : toFree.trim();
      if (!t) return { params: null, problem: tr("Empfänger angeben.", "Enter a recipient.") };
      if (isKName(t)) {
        // Neuprüfung alle 60 s: die bisherige Adresse gilt weiter, bis eine neue Antwort da ist –
        // sonst verwirft der Signierablauf Plan und Signatur (Audit 19 A19-5)
        if (!kname || kname.input !== t || (kname.loading && !kname.address)) return { params: null, problem: tr(`${t} wird geprüft …`, `Checking ${t} …`) };
        if (!kname.address) return { params: null, problem: kname.error ?? tr("Name nicht auflösbar.", "Name cannot be resolved.") };
        p.to = kname.address;
      } else p.to = t;
    }
    if (meta.amount && !useFull) {
      if (amountStr.trim() === "") return { params: null, problem: tr(`${meta.amount.label} eingeben.`, `Enter ${meta.amount.label}.`) };
      if (amount === null || amount <= 0n) return { params: null, problem: amountProblem(meta.amount.label, amountStr) };
      p[meta.amount.param] = cliDecimal(amount);
    }
    if (meta.amount2) {
      if (amount2Str.trim() === "") return { params: null, problem: tr(`${meta.amount2.label} eingeben.`, `Enter ${meta.amount2.label}.`) };
      if (amount2 === null || amount2 <= 0n) return { params: null, problem: amountProblem(meta.amount2.label, amount2Str) };
      p[meta.amount2.param] = cliDecimal(amount2);
    }
    if (hasMessage) {
      const mp = messageProblem(message);
      if (mp) return { params: null, problem: mp };
      if (message.trim()) p.message = message.trim();
      if (onchain) {
        if (!message.trim()) return { params: null, problem: tr("Für eine öffentliche Nachricht erst eine Nachricht eingeben.", "Enter a message first to publish it.") };
        p.onchain = true;
      } else if (message.trim() && typeof p.to === "string") {
        // ohne Häkchen wird verschlüsselt: das geht nur an normale Kaspa-Adressen
        const ep = encryptProblem(p.to);
        if (ep) return { params: null, problem: ep };
      }
    }
    if (action === "oracle-update") {
      if (!committee) return { params: null, problem: tr("Unterzeichner-Datei wählen.", "Choose the signer file.") };
      p.committee = committee;
      if (usdStr.trim()) {
        if (usd === null || usd <= 0n) return { params: null, problem: amountProblem(tr("Preis", "Price"), usdStr) };
        p.usd = cliDecimal(usd);
      }
      if (rateStr.trim()) {
        if (rate === null) return { params: null, problem: amountProblem(tr("Zins", "Interest"), rateStr) };
        p.rate = cliDecimal(rate, 2);
      }
    }
    if (signMode === "wallet") return toWalletParams(action, p, (i) => live?.vaults.find((v) => v.index === i)?.covenantId);
    return { params: p, problem: null };
  }, [signMode, key, meta, vault, toMode, toKey, toFree, kname, action, useFull, amountStr, amount, amount2Str, amount2, committee, usdStr, usd, rateStr, rate, hasMessage, message, onchain]);

  // „Alles tilgen/liquidieren“: die Schuld gehört zur Signatur, sonst bliebe ein
  // alter Probelauf nach einer Schuldänderung gültig (A10-W-11)
  const fullDebt = useFull && vault !== null ? (live?.vaults.find((v) => v.index === vault)?.debtGhost ?? null) : null;
  const sig = built.params ? JSON.stringify([network, action, built.params, fullDebt]) : "";
  const checkFresh = checked !== null && Date.now() - checked.at < CHECK_TTL_MS;
  const checkValid = checked !== null && checked.sig === sig && checked.result.ok && checkFresh;
  const unclearLock = unclearAt !== null && (updatedAt ?? 0) <= unclearAt;
  // Nach Ablauf des Probelaufs neu zeichnen, damit „Senden“ wieder gesperrt wird
  useEffect(() => {
    if (!checked) return;
    const left = CHECK_TTL_MS - (Date.now() - checked.at);
    if (left <= 0) return;
    const t = window.setTimeout(() => setExpiryTick((x) => x + 1), left + 50);
    return () => window.clearTimeout(t);
  }, [checked]);

  // Was genau gesendet wird, in Worten – für Probelauf und Bestätigung (A10-W-5)
  const summary = (() => {
    if (!built.params) return "";
    const parts: string[] = [meta.label];
    if (meta.amount) {
      if (useFull) parts.push(tr("ganze Schuld", "whole debt"));
      else if (amount !== null) parts.push(`${formatUnits(amount, 8, 8)} ${meta.amount.unit}`);
    }
    if (meta.amount2 && amount2 !== null) parts.push(`${formatUnits(amount2, 8, 8)} ${meta.amount2.unit}`);
    if (typeof built.params.vault === "number") parts.push(`Vault ${built.params.vault}`);
    if (typeof built.params.to === "string") parts.push(`→ ${built.params.to}`);
    if (action === "oracle-update" && usd !== null) parts.push(`${formatUnits(usd, 8, 8)} USD`);
    if (typeof built.params.message === "string")
      parts.push(
        `„${built.params.message}“${built.params.onchain ? tr(" (öffentlich in der Transaktion)", " (public in the transaction)") : tr(" (verschlüsselt, nur der Empfänger kann sie lesen)", " (encrypted, only the recipient can read it)")}`,
      );
    return parts.join(" · ");
  })();
  // Wallet noch nicht verbunden: keine Vorprüfung (sie verlangt sonst eine Schlüsseldatei)
  const hints =
    signMode === "wallet" && !key
      ? []
      : precheck({
          action,
          amount,
          amount2,
          repayAll: fullAmount,
          liquidateAll: fullAmount,
          usd: action === "oracle-update" ? usd : null,
          rateBps: action === "oracle-update" ? rate : null,
          vault,
          key,
          status: live,
        });

  const exec = async (dryRun: boolean) => {
    if (!built.params) return;
    // Pool: Mindestwerte aus dem Probelauf mitschicken; verschiebt sich der Pool
    // bis zum Senden, bricht ghostctl ab (A10-W-6)
    let params: ActionParams = built.params;
    if (!dryRun && checked) {
      if (action === "pool-add") {
        const m = checked.result.shares;
        if (typeof m !== "number" || m < 1) return setApiError(tr("Probelauf ohne Anteilszahl – bitte neu prüfen.", "Dry run without share count – please check again."));
        params = { ...params, minShares: Math.max(1, Math.floor((m * 99) / 100)) };
      } else if (action === "pool-remove") {
        const mk = minFromDry(checked.result.kas);
        const mg = minFromDry(checked.result.ghost);
        if (mk === null || mg === null) return setApiError(tr("Probelauf ohne Auszahlung – bitte neu prüfen.", "Dry run without payout – please check again."));
        params = { ...params, minKas: mk, minGhost: mg };
      }
    }
    setApiError(null);
    setPhase(dryRun ? "checking" : "sending");
    if (!dryRun) setSent(null);
    const release = dryRun ? () => {} : markBusy();
    try {
      const r = await runAction({ network, action, params, dryRun, confirmMainnet: !dryRun && isMain ? true : undefined });
      if (dryRun) setChecked({ sig, result: r, at: Date.now() });
      else {
        setSent(r);
        setChecked(null);
        const anySent = (r.transactions ?? []).some((t) => t.sent);
        if (r.ok) {
          setAmountStr("");
          setAmount2Str("");
          setMessage("");
          setOnchain(false);
        }
        if (!r.dryRun && key && (r.ok || anySent))
          logTx({
            at: Date.now(),
            network,
            key: key.file,
            action,
            label: meta.label,
            amount: built.params[meta.amount?.param ?? ""] !== undefined ? String(built.params[meta.amount!.param]) : null,
            unit: meta.amount?.unit ?? null,
            to: typeof built.params.to === "string" ? built.params.to : null,
            txids: (r.transactions ?? []).filter((t) => t.sent).map((t) => t.txid),
            ...(r.ok ? {} : { partial: true }),
            ...(typeof built.params.message === "string"
              ? { message: built.params.message, onchain: built.params.onchain === true, encrypted: built.params.onchain !== true }
              : {}),
          });
        if (r.unclear || r.timeout) setUnclearAt(Date.now());
        if (r.ok || anySent || r.unclear || r.timeout) {
          refreshStatus();
          acc.refresh();
        }
      }
    } catch (e) {
      setApiError((e as Error).message);
    } finally {
      release();
      setPhase("idle");
    }
  };

  const vaults = (live?.vaults ?? []).filter((x) => !ownVaults || (key !== null && x.owner === key.xonly));
  const selectedVault = vault !== null ? (vaults.find((x) => x.index === vault) ?? null) : null;
  const staleVault = meta.vault && vault !== null ? (vaults.find((x) => x.index === vault)?.stale ?? false) : false;
  const blocked = staleVault || nodeDown || unclearLock;
  const feeSum = (r: ActionResult) => (r.transactions ?? []).reduce((s, t) => s + t.feeKas, 0);
  const donatedSum = (r: ActionResult) => (r.transactions ?? []).reduce((s, t) => s + (t.donatedKas ?? 0), 0);
  const txList = (r: ActionResult, withIds: boolean) => (
    <ul className="tx-list">
      {(r.transactions ?? []).map((t) => (
        <li key={t.txid}>
          {t.action}
          {withIds ? ` · ${t.confirmed ? tr("bestätigt", "confirmed") : t.sent ? tr("gesendet, noch unbestätigt", "sent, not yet confirmed") : tr("nicht gesendet", "not sent")}` : ""} · {tr("Gebühr", "Fee")}{" "}
          {de(t.feeKas, 8)} KAS
          {t.donatedKas ? ` · ${tr("Rest an Miner", "Remainder to miners")}: ${de(t.donatedKas, 8)} KAS` : ""}
          {!withIds && ` · ${t.inputs} ${tr("Eingänge", "inputs")}, ${t.outputs} ${tr("Ausgänge", "outputs")}`}
          {withIds && (
            <div className="txid">
              <code>{t.txid}</code>
              <CopyButton text={t.txid} label="TXID" />
            </div>
          )}
        </li>
      ))}
    </ul>
  );
  const errorText = (r: ActionResult) => (
    <>
      <p>{r.error}</p>
      {r.nodeDown && r.detail && <p className="small muted">{tr("Technisch", "Technical")}: {r.detail}</p>}
    </>
  );
  const otherKeys = acc.keys.filter((x) => x.file !== key?.file);

  return (
    <section className="card section-sm" id={id} ref={panelRef} aria-labelledby={`${id}-title`}>
      <div className="card-head">
        <div className="head-title">
          <h2 id={`${id}-title`} tabIndex={-1}>
            {title ?? tr("Aktionen", "Actions")}
          </h2>
          {ownVaults && (
            <select id={`${id}-vault`} className="head-select" aria-label={tr("Vault wählen", "Choose vault")} disabled={phase !== "idle"} value={vault === null ? "" : String(vault)} onChange={(e) => setVault(e.target.value === "" ? null : Number(e.target.value))}>
              {vaults.length === 0 && (
                <option value="">{ownVaults ? tr("keine eigenen Vaults – erst „Vault eröffnen“", "no own vaults – first “Open vault”") : tr("keine Vaults", "no vaults")}</option>
              )}
              {vaults.map((x) => (
                <option key={x.index} value={x.index}>
                  {vaultLabel(x, live?.vaults ?? [], key?.xonly ?? null)}
                  {!ownVaults && key && x.owner === key.xonly ? tr(" (deiner)", " (yours)") : ""}
                  {x.stale ? tr(" (gesperrt)", " (blocked)") : ""} · {de(x.collateralKas, 2)} KAS · {tr("Schuld", "Debt")} {de(x.debtGhost, 4)}
                </option>
              ))}
            </select>
          )}
        </div>
        <span className={isMain ? "tag tag-warn" : "tag"}>{NETWORKS[network].label}</span>
      </div>

      {actions.length > 1 && (
      <div className="tabs" role="group" aria-label={tr("Aktion wählen", "Choose action")}>
        {actions.map((a) => (
          <button
            key={a}
            type="button"
            aria-pressed={action === a}
            className={action === a ? "tab active" : "tab"}
            onClick={() => {
              setAction(a);
              setAmountStr("");
              setApiError(null);
            }}
          >
            {META[a].label}
          </button>
        ))}
      </div>
      )}

      {walletOk && keyOk && (
        <div className="tabs" role="radiogroup" aria-label={tr("Signieren mit", "Sign with")}>
          {(["key", "wallet"] as SignMode[]).map((m) => (
            <button key={m} type="button" role="radio" aria-checked={signMode === m} className={signMode === m ? "tab active" : "tab"} onClick={() => setModeChoice(m)}>
              {m === "key" ? tr("Schlüsseldatei (lokal)", "Key file (local)") : tr("Browser-Wallet", "Browser wallet")}
            </button>
          ))}
        </div>
      )}

      <form
        className="actions-grid"
        onSubmit={(e) => {
          e.preventDefault();
          if (signMode === "key") void exec(true);
        }}
      >
        <fieldset className="plain" disabled={phase !== "idle"}>
          <legend className="sr-only">{meta.label}</legend>
          <p className="small">{meta.help}</p>

          {meta.vault && !ownVaults && (
            <div className="field">
              <label htmlFor={ids.vault}>Vault</label>
              <div className="input-wrap">
                <select id={ids.vault} value={vault === null ? "" : String(vault)} onChange={(e) => setVault(e.target.value === "" ? null : Number(e.target.value))}>
                  {vaults.length === 0 && (
                    <option value="">{ownVaults ? tr("keine eigenen Vaults – erst „Vault eröffnen“", "no own vaults – first “Open vault”") : tr("keine Vaults", "no vaults")}</option>
                  )}
                  {vaults.map((x) => (
                    <option key={x.index} value={x.index}>
                      {vaultLabel(x, live?.vaults ?? [], key?.xonly ?? null)}
                      {!ownVaults && key && x.owner === key.xonly ? tr(" (deiner)", " (yours)") : ""}
                      {x.stale ? tr(" (gesperrt)", " (blocked)") : ""} · {de(x.collateralKas, 2)} KAS · {tr("Schuld", "Debt")} {de(x.debtGhost, 4)}
                      {!ownVaults && x.ratioPct !== null ? ` · ${tr("Quote", "Ratio")} ${de(x.ratioPct, 0)} %` : ""}
                    </option>
                  ))}
                </select>
              </div>
            </div>
          )}

          {ownVaults && meta.vault && selectedVault && (
            <dl className="kv small vault-summary">
              <div>
                <dt>{tr("Sicherheit", "Collateral")}</dt>
                <dd>
                  {de(selectedVault.collateralKas, 4)} KAS
                  <Usd amount={selectedVault.collateralKas} unit="KAS" />
                </dd>
              </div>
              <div>
                <dt>{tr("Schuld", "Debt")}</dt>
                <dd>
                  {de(selectedVault.debtGhost, 8)} GHOST
                  <Usd amount={selectedVault.debtGhost} unit="GHOST" />
                </dd>
              </div>
              {selectedVault.interestUsd !== undefined && (
                <div>
                  <dt>{tr("Offener Zins", "Open interest")}</dt>
                  <dd>{de(selectedVault.interestUsd, 4)} USD</dd>
                </div>
              )}
              <div>
                <dt>{tr("Quote", "Ratio")}</dt>
                <dd>{selectedVault.ratioPct !== null ? `${de(selectedVault.ratioPct, 1)} %` : "–"}</dd>
              </div>
              <div>
                <dt>{tr("Liquidation unter", "Liquidation below")}</dt>
                <dd>{selectedVault.liquidationPriceUsd !== null ? `${de(selectedVault.liquidationPriceUsd, 6)} USD` : "–"}</dd>
              </div>
              <div>
                <dt>{tr("Noch prägbar", "Still mintable")}</dt>
                <dd>
                  {de(selectedVault.maxMintGhost, 8)} GHOST
                  <Usd amount={selectedVault.maxMintGhost} unit="GHOST" />
                </dd>
              </div>
            </dl>
          )}

          {hasFullMode && (
            <div className="field" role="radiogroup" aria-label={action === "repay" ? tr("Umfang der Tilgung", "Repayment scope") : tr("Umfang der Liquidation", "Liquidation scope")}>
              <label className="radio">
                <input type="radio" name="full-mode" checked={fullAmount} onChange={() => setFullAmount(true)} />{" "}
                {action === "repay" ? tr("Alles tilgen", "Repay all") : tr("Ganze Schuld", "Whole debt")}
              </label>
              <label className="radio">
                <input type="radio" name="full-mode" checked={!fullAmount} onChange={() => setFullAmount(false)} /> {tr("Teilbetrag", "Partial amount")}
              </label>
            </div>
          )}

          {meta.to && (
            <div className="field">
              <label htmlFor={ids.toMode}>{tr("Empfänger", "Recipient")}</label>
              <div className="input-wrap">
                <select id={ids.toMode} value={signMode === "wallet" ? "free" : toMode} onChange={(e) => setToMode(e.target.value as "key" | "free")}>
                  {signMode === "key" && <option value="key">{tr("Eigene Schlüsseldatei", "Own key file")}</option>}
                  <option value="free">{meta.to === "address" ? tr("Kaspa-Adresse", "Kaspa address") : tr("x-only-Schlüssel (64 hex)", "x-only key (64 hex)")}</option>
                </select>
              </div>
              {toMode === "key" && signMode === "key" ? (
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
                    placeholder={meta.to === "address" ? (isMain ? "kaspa:q… oder name.k" : "kaspatest:q… oder name.k") : tr("64 Hex-Zeichen oder name.k", "64 hex characters or name.k")}
                    spellCheck={false}
                    autoComplete="off"
                  />
                </div>
              )}
              {kname && (
                <p className={kname.error ? "small kname kname-bad" : "small kname"} aria-live="polite">
                  {kname.loading && !kname.address
                    ? tr("Name wird bei dotk.name nachgeschlagen und am Kaspa-Node geprüft …", "Looking up the name at dotk.name and checking it at the Kaspa node …")
                    : kname.address
                      ? (
                          <>
                            {kname.display} → <code className="addr-break">{kname.address}</code>{" "}
                            <span className="tag">{tr("am Node geprüft", "checked at node")}</span>
                            <br />
                            <span className="muted">{tr("Bitte die Adresse vergleichen, bevor du signierst.", "Please compare the address before signing.")}</span>
                          </>
                        )
                      : kname.error}
                </p>
              )}
            </div>
          )}

          {meta.amount && !useFull && (
            <AmountInput
              label={meta.amount.label}
              value={amountStr}
              onChange={setAmountStr}
              suffix={meta.amount.unit}
              invalid={amountStr.trim() !== "" && amount === null}
              hint={amount !== null && amount > 0n ? (usdOf(amountStr, meta.amount.unit) ?? undefined) : undefined}
            />
          )}
          {meta.amount2 && (
            <AmountInput
              label={meta.amount2.label}
              value={amount2Str}
              onChange={setAmount2Str}
              suffix={meta.amount2.unit}
              invalid={amount2Str.trim() !== "" && amount2 === null}
              hint={amount2 !== null && amount2 > 0n ? (usdOf(amount2Str, meta.amount2.unit) ?? undefined) : undefined}
            />
          )}

          {hasMessage && (
            <>
              <div className="field">
                <label htmlFor={ids.msg}>{tr("Nachricht (optional)", "Message (optional)")}</label>
                <div className={messageProblem(message) ? "input-wrap invalid" : "input-wrap"}>
                  <input
                    id={ids.msg}
                    value={message}
                    onChange={(e) => setMessage(e.target.value)}
                    maxLength={MAX_MESSAGE_CHARS * 2}
                    placeholder={tr("z. B. Miete Oktober", "e.g. rent October")}
                    autoComplete="off"
                  />
                </div>
                <div className="field-hint">
                  {tr(
                    `Steht auch im Verlauf auf diesem Rechner. Höchstens ${MAX_MESSAGE_CHARS} Zeichen.`,
                    `Also kept in the history on this computer. At most ${MAX_MESSAGE_CHARS} characters.`,
                  )}
                </div>
              </div>
              <label className="checkbox">
                <input type="checkbox" checked={onchain} onChange={(e) => setOnchain(e.target.checked)} />
                <span>{PUBLIC_MESSAGE_LABEL()}</span>
              </label>
            </>
          )}

          {action === "oracle-update" && (
            <>
              <div className="field">
                <label htmlFor={ids.com}>{tr("Unterzeichner-Datei", "Signer file")}</label>
                <div className="input-wrap">
                  <select id={ids.com} value={committee} onChange={(e) => setCommittee(e.target.value)}>
                    {acc.committees.length === 0 && <option value="">{tr("keine gefunden", "none found")}</option>}
                    {acc.committees.map((c) => (
                      <option key={c.file} value={c.file}>
                        {c.file.replace(/^keys\//, "")} ({c.signers} {tr("Signierer", "signers")})
                      </option>
                    ))}
                  </select>
                </div>
              </div>
              <AmountInput label={tr("Fester Preis (leer = Median)", "Fixed price (empty = median)")} value={usdStr} onChange={setUsdStr} suffix="USD" invalid={usdStr.trim() !== "" && usd === null} />
              <AmountInput label={tr("Neuer Zins (leer = unverändert)", "New interest (empty = unchanged)")} value={rateStr} onChange={setRateStr} suffix="% p. a." decimals={2} invalid={rateStr.trim() !== "" && rate === null} />
            </>
          )}

          {hints.length > 0 && (
            <ul className="hints" aria-label={tr("Vorprüfung", "Pre-check")}>
              {hints.map((h, i) => (
                <li key={i} className={`hint hint-${h.level}`}>
                  {h.text}
                </li>
              ))}
            </ul>
          )}

          {signMode === "key" && (
            <div className="btn-row">
              <button type="submit" className="btn btn-ghost" disabled={!built.params || phase !== "idle" || blocked} aria-busy={phase === "checking"}>
                {phase === "checking" ? tr("Prüfe …", "Checking …") : tr("Prüfen", "Check")}
              </button>
            </div>
          )}
        </fieldset>

        {signMode === "wallet" ? (
          <div className="result-col" aria-live="polite">
            {nodeDown && <Callout kind="warn">{tr("Öffentliche Kaspa-Nodes nicht erreichbar – später erneut versuchen.", "Public Kaspa nodes unreachable – try again later.")}</Callout>}
            {staleVault && !nodeDown && <Callout kind="warn">{tr("Dieser Vault wurde von Dritten verändert. Aktionen sind gesperrt, bis ghostctl nachgeladen hat.", "This vault was changed by a third party. Actions are blocked until ghostctl has reloaded.")}</Callout>}
            <WalletSignFlow
              network={network}
              action={action}
              label={meta.label}
              params={built.params}
              problem={built.problem}
              summary={summary}
              blocked={blocked}
              onDone={(any) => {
                if (any) {
                  refreshStatus();
                  acc.refresh();
                  void wallet.refresh();
                }
              }}
            />
          </div>
        ) : (
        <div className="result-col" aria-live="polite">
          {built.problem && <p className="muted small">{built.problem}</p>}
          {nodeDown && <Callout kind="warn">{tr("Öffentliche Kaspa-Nodes nicht erreichbar – später erneut versuchen.", "Public Kaspa nodes unreachable – try again later.")}</Callout>}
          {staleVault && !nodeDown && <Callout kind="warn">{tr("Dieser Vault wurde von Dritten verändert. Aktionen sind gesperrt, bis ghostctl nachgeladen hat.", "This vault was changed by a third party. Actions are blocked until ghostctl has reloaded.")}</Callout>}

          {checked && checked.sig === sig && (
            <div className={checked.result.ok ? "result ok" : "result err"}>
              {checked.result.ok ? (
                <>
                  <strong>{tr("Probelauf erfolgreich – nichts gesendet.", "Dry run successful – nothing sent.")}</strong>
                  <p className="small">
                    <strong>{summary}</strong>
                    {typeof checked.result.shares === "number" && action === "pool-add"
                      ? tr(` · ${checked.result.shares} Anteile (mindestens ${Math.max(1, Math.floor((checked.result.shares * 99) / 100))} beim Senden)`, ` · ${checked.result.shares} shares (at least ${Math.max(1, Math.floor((checked.result.shares * 99) / 100))} when sending)`)
                      : ""}
                    {action === "pool-add" && typeof checked.result.kas === "number" && typeof checked.result.ghost === "number"
                      ? tr(` · genommen werden ${de(checked.result.kas, 8)} KAS und ${de(checked.result.ghost, 8)} GHOST`, ` · taken: ${de(checked.result.kas, 8)} KAS and ${de(checked.result.ghost, 8)} GHOST`)
                      : ""}
                    {action === "pool-remove" && typeof checked.result.kas === "number" && typeof checked.result.ghost === "number"
                      ? tr(` · Auszahlung ${de(checked.result.kas, 8)} KAS und ${de(checked.result.ghost, 8)} GHOST (höchstens 1 % weniger beim Senden)`, ` · payout ${de(checked.result.kas, 8)} KAS and ${de(checked.result.ghost, 8)} GHOST (at most 1 % less when sending)`)
                      : ""}
                  </p>
                  {txList(checked.result, false)}
                  <p className="small">
                    {tr("Gebühr gesamt", "Total fee")}: <strong>{de(feeSum(checked.result), 8)} KAS</strong>
                    {donatedSum(checked.result) > 0 ? tr(` · dazu ${de(donatedSum(checked.result), 8)} KAS Rest an die Miner`, ` · plus ${de(donatedSum(checked.result), 8)} KAS remainder to miners`) : ""}
                    {typeof checked.result.vault === "number" ? tr(` · neuer Vault: Nr. ${checked.result.vault}`, ` · new vault: no. ${checked.result.vault}`) : ""}
                  </p>
                </>
              ) : (
                <>
                  <strong>{checked.result.nodeDown ? tr("Nodes nicht erreichbar", "Nodes unreachable") : tr("Probelauf abgelehnt", "Dry run rejected")}</strong>
                  {errorText(checked.result)}
                </>
              )}
            </div>
          )}
          {checked && checked.sig !== sig && built.params && (
            <p className="muted small">{tr("Die Eingaben haben sich seit dem Prüfen geändert – bitte erneut prüfen.", "The inputs changed since the check – please check again.")}</p>
          )}
          {checked && checked.sig === sig && checked.result.ok && !checkFresh && (
            <p className="muted small">{tr("Der Probelauf ist älter als 2 Minuten – bitte erneut prüfen.", "The dry run is older than 2 minutes – please check again.")}</p>
          )}
          {unclearLock && (
            <Callout kind="warn">
              {tr("Senden ist gesperrt, bis der Status neu geladen ist. Prüfe danach Guthaben und Verlauf, bevor du erneut sendest.", "Sending is blocked until the status has reloaded. Then check balances and history before sending again.")}
            </Callout>
          )}


          <button
            type="button"
            className="btn btn-primary"
            disabled={!checkValid || phase !== "idle" || blocked}
            aria-busy={phase === "sending"}
            onClick={() => void exec(false)}
          >
            {phase === "sending" ? tr("Sende … (bis zu einigen Minuten)", "Sending … (up to a few minutes)") : isMain ? tr("Im Mainnet senden", "Send on mainnet") : tr("Senden", "Send")}
          </button>


          {phase === "sending" && (
            <p className="progress" role="status">
              <span className="spinner" aria-hidden="true" /> {tr("ghostctl signiert, sendet und wartet auf die Bestätigung …", "ghostctl signs, sends and waits for confirmation …")}
            </p>
          )}

          {apiError && <Callout kind="danger" title={tr("Fehler", "Error")}>{apiError}</Callout>}

          {sent && (
            <div className={sent.ok ? "result ok" : "result err"}>
              {sent.ok ? (
                <>
                  <strong>{tr("Gesendet.", "Sent.")}</strong>
                  {txList(sent, true)}
                  {typeof sent.vault === "number" && <p className="small">{tr("Neuer Vault: Nr.", "New vault: no.")} {sent.vault}</p>}
                  <p className="small muted">{tr("Status und Guthaben werden neu geladen.", "Status and balances are being reloaded.")}</p>
                </>
              ) : (
                <>
                  {(sent.transactions ?? []).some((t) => t.sent) ? (
                    <>
                      <strong>{tr("Teilweise gesendet – ein späterer Schritt ist fehlgeschlagen", "Partially sent – a later step failed")}</strong>
                      {errorText(sent)}
                      {txList(sent, true)}
                      <p className="small muted">
                        {tr(
                          "ghostctl führt ein Journal gesendeter Transaktionen und schließt es beim nächsten Aufruf selbst ab. Erst „Neu laden“, dann erneut prüfen.",
                          "ghostctl keeps a journal of sent transactions and completes it on the next call. First “Reload”, then check again.",
                        )}
                      </p>
                    </>
                  ) : sent.unclear || sent.timeout ? (
                    <>
                      <strong>{tr("Ergebnis unklar – NICHT sofort erneut senden", "Outcome unclear – do NOT send again right away")}</strong>
                      {errorText(sent)}
                      <p className="small muted">
                        {tr("Erst „Neu laden“ und Guthaben bzw. Vault prüfen. Erst wenn dort nichts angekommen ist, erneut prüfen und senden.", "First “Reload” and check balances or the vault. Only if nothing arrived there, check and send again.")}
                      </p>
                    </>
                  ) : (
                    <>
                      <strong>{sent.nodeDown ? tr("Nodes nicht erreichbar – nichts gesendet", "Nodes unreachable – nothing sent") : tr("Nicht gesendet", "Not sent")}</strong>
                      {errorText(sent)}
                    </>
                  )}
                </>
              )}
            </div>
          )}

          {built.params && (
            <details className="as-cmd">
              <summary>{tr("Als Befehl", "As command")}</summary>
              <p className="small muted">
                {tr("Gleichwertig im Terminal (Projektordner). Ohne ", "Equivalent in the terminal (project folder). Without ")}
                <code>--ja</code>
                {tr(" fragt ghostctl im Mainnet vor dem Senden nach.", " ghostctl asks for confirmation on mainnet before sending.")}
              </p>
              <CopyCode code={commandFor(network, action, built.params)} label={`${tr("Befehl", "Command")} ${meta.label}`} />
            </details>
          )}
        </div>
        )}
      </form>
    </section>
  );
}
