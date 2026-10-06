import { useEffect, useState } from "react";
import { ActionForms } from "../components/ActionForms";
import { CopyButton } from "../components/CopyCode";
import { StatusNotices } from "../components/Network";
import { AmountInput, Callout, Stat } from "../components/ui";
import { NATIVE, NETWORKS, STABLE_SYMBOL } from "../config";
import { runAction, type ActionResult } from "../lib/api";
import { useAccount } from "../lib/AccountContext";
import { cliDecimal } from "../lib/commands";
import { amountProblem, formatUnits, parseUnits } from "../lib/format";
import { ghostOut, impactBps, kasOut, payoutFor, withSlippage } from "../lib/poolMath";
import { useStatus } from "../lib/StatusContext";
import { de } from "../lib/status";
import { frozenText } from "../lib/precheck";
import { logTx } from "../lib/txlog";
import { tr } from "../lib/i18n";
import { TokenLabel } from "../components/TokenIcons";
import { markBusy } from "../lib/busy";
import { Usd } from "../components/Usd";
import { useUsd } from "../lib/usd";
import { useWallet } from "../wallet/WalletContext";
import { walletKeyEntry } from "../wallet/actions";
import { xOnlyKey } from "../lib/status";
import { WalletSignFlow } from "../components/WalletSignFlow";
import type { SignMode } from "../components/ActionForms";

const SLIPPAGE_BPS = 100n; // 1 %
const E8 = 100_000_000n;
const CHECK_TTL_MS = 120_000;

type Dir = "buy" | "sell"; // buy = KAS → GHOST, sell = GHOST → KAS

function SwapCard() {
  const { network, status, refresh: refreshStatus, nodeDown, updatedAt } = useStatus();
  const acc = useAccount();
  const wallet = useWallet();
  // Browser-Wallet: Standard ohne Schlüsseldateien (öffentlicher Modus)
  const [modeChoice, setModeChoice] = useState<SignMode | null>(null);
  const keyOk = acc.keys.length > 0;
  const signMode: SignMode = !keyOk ? "wallet" : (modeChoice ?? "key");
  const walletKey =
    wallet.status === "connected" && wallet.address ? walletKeyEntry(wallet.address, xOnlyKey(wallet.publicKey), wallet.balance, null, []) : null;
  const key = signMode === "wallet" ? walletKey : acc.selected;
  const pool = status?.deployed ? (status.pool ?? null) : null;
  const isMain = network === "mainnet";
  const usd = useUsd();

  const [dir, setDir] = useState<Dir>("buy");
  const [amountStr, setAmountStr] = useState("");
  const [phase, setPhase] = useState<"idle" | "checking" | "sending">("idle");
  const [checked, setChecked] = useState<{ sig: string; result: ActionResult; at: number } | null>(null);
  const [unclearAt, setUnclearAt] = useState<number | null>(null);
  const [, setExpiryTick] = useState(0);
  const [sent, setSent] = useState<ActionResult | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    setChecked(null);
    setSent(null);
  }, [network]);

  const x = pool ? BigInt(pool.kasSompi) : 0n;
  const y = pool ? BigInt(pool.ghostUnits) : 0n;
  const fee = pool ? BigInt(pool.feeBps) : 30n;
  const amount = amountStr.trim() ? parseUnits(amountStr, 8) : null;
  const out = pool && amount && amount > 0n ? (dir === "buy" ? ghostOut(x, y, amount, fee) : kasOut(x, y, amount, fee)) : null;
  const min = out !== null ? withSlippage(out, SLIPPAGE_BPS) : null;
  const impact = out !== null && amount ? (dir === "buy" ? impactBps(x, y, amount, out) : impactBps(y, x, amount, out)) : null;
  const inUnit = dir === "buy" ? NATIVE : STABLE_SYMBOL;
  const outUnit = dir === "buy" ? STABLE_SYMBOL : NATIVE;
  const have = key ? (dir === "buy" ? key.kas : key.ghost) : null;
  // exakt in Einheiten vergleichen (Audit 9 P-5: Number("1.000") versagte)
  const haveUnits = have !== null ? BigInt(Math.round(have * 1e8)) : null;
  // Kauf: der neue GHOST-Token bindet 1 KAS (kommt beim Weitergeben zurück)
  const extraKas = dir === "buy" ? E8 : 0n;

  // Kursband: größter Einsatz, den der Pool gerade zulässt (0 = Richtung gesperrt)
  const bandMax = pool && pool.bandBps != null ? (dir === "buy" ? pool.maxBuyKas : pool.maxSellGhost) : null;
  const bandMaxUnits = bandMax != null ? BigInt(Math.floor(bandMax * 1e8)) : null;
  const bandPct = pool?.bandBps != null ? de(pool.bandBps / 100, 1) : "";

  const problem = !pool
    ? tr("In diesem Netz gibt es noch keinen Pool.", "There is no pool on this network yet.")
    : status?.deployed && status.oracle.frozen
      ? frozenText()
    : bandMaxUnits === 0n
      ? tr(
          `Kursband: In diese Richtung ist der Pool gerade gesperrt, weil GHOST sonst das Band 1 USD ± ${bandPct} % verlassen würde. Er öffnet wieder, wenn jemand in die Gegenrichtung tauscht, Liquidität einlegt oder sich der KAS-Preis bewegt.`,
          `Price band: the pool is currently closed in this direction because GHOST would leave the band 1 USD ± ${bandPct} %. It reopens when someone swaps the other way, adds liquidity or the KAS price moves.`,
        )
    : pool.unresolved
      ? tr(`Pool-Reserve unbekannt: ${pool.unresolved}`, `Pool reserve unknown: ${pool.unresolved}`)
      : !key
        ? tr("Unter „Wallet“ eine Schlüsseldatei wählen.", "Choose a key file under “Wallet”.")
        : amountStr.trim() === ""
          ? tr("Betrag eingeben.", "Enter an amount.")
          : amount === null || amount <= 0n
            ? amountProblem(tr("Betrag", "Amount"), amountStr)
            : out === null || out === 0n || min === null || min === 0n
              ? tr("Betrag zu klein für einen Tausch.", "Amount too small for a swap.")
              : bandMaxUnits !== null && amount > bandMaxUnits
                ? tr(
                    `Kursband: Höchstens ${formatUnits(bandMaxUnits, 8, 8)} ${inUnit} sind gerade möglich, sonst verlässt GHOST das Band 1 USD ± ${bandPct} %. Mehr geht, wenn mehr Liquidität im Pool ist.`,
                    `Price band: at most ${formatUnits(bandMaxUnits, 8, 8)} ${inUnit} is possible right now, otherwise GHOST leaves the band 1 USD ± ${bandPct} %. More is possible with more liquidity in the pool.`,
                  )
              : haveUnits !== null && amount + extraKas > haveUnits
                ? tr(
                    `Der Schlüssel hat nur ${de(have!, 8)} ${inUnit}${dir === "buy" ? " (nötig: Betrag + 1 KAS für den neuen Token + Gebühr)" : ""}.`,
                    `The key only has ${de(have!, 8)} ${inUnit}${dir === "buy" ? " (needed: amount + 1 KAS for the new token + fee)" : ""}.`,
                  )
                : null;

  const params: Record<string, string> | null =
    !problem && key && amount && min
      ? { ...(signMode === "key" ? { key: key.file } : {}), [dir === "buy" ? "kas" : "ghost"]: cliDecimal(amount), min: cliDecimal(min) }
      : null;
  const sig = params ? JSON.stringify([network, params]) : "";
  // Probelauf verfällt nach 2 Minuten (A10-W-11); unklarer Ausgang sperrt bis zum neuen Status (A10-W-2)
  const checkFresh = checked !== null && Date.now() - checked.at < CHECK_TTL_MS;
  const checkValid = checked !== null && checked.sig === sig && checked.result.ok && checkFresh;
  const unclearLock = unclearAt !== null && (updatedAt ?? 0) <= unclearAt;
  useEffect(() => {
    if (!checked) return;
    const left = CHECK_TTL_MS - (Date.now() - checked.at);
    if (left <= 0) return;
    const t = window.setTimeout(() => setExpiryTick((v) => v + 1), left + 50);
    return () => window.clearTimeout(t);
  }, [checked]);
  const summary =
    amount && min
      ? tr(
          `${formatUnits(amount, 8, 8)} ${inUnit} gegen mindestens ${formatUnits(min, 8, 8)} ${outUnit}`,
          `${formatUnits(amount, 8, 8)} ${inUnit} for at least ${formatUnits(min, 8, 8)} ${outUnit}`,
        )
      : "";

  const exec = async (dryRun: boolean) => {
    if (!params) return;
    setErr(null);
    setPhase(dryRun ? "checking" : "sending");
    if (!dryRun) setSent(null);
    const release = dryRun ? () => {} : markBusy();
    try {
      const r = await runAction({ network, action: "swap", params, dryRun, confirmMainnet: !dryRun && isMain ? true : undefined });
      if (dryRun) setChecked({ sig, result: r, at: Date.now() });
      else {
        setSent(r);
        setChecked(null);
        if (r.ok && key) {
          setAmountStr("");
          logTx({
            at: Date.now(),
            network,
            key: key.file,
            action: "swap",
            label: dir === "buy" ? `${NATIVE} → ${STABLE_SYMBOL}` : `${STABLE_SYMBOL} → ${NATIVE}`,
            amount: params[dir === "buy" ? "kas" : "ghost"],
            unit: dir === "buy" ? "KAS" : "GHOST",
            to: null,
            txids: (r.transactions ?? []).filter((t) => t.sent).map((t) => t.txid),
          });
        }
        if (r.unclear || r.timeout) setUnclearAt(Date.now());
        if (r.ok || (r.transactions ?? []).some((t) => t.sent) || r.unclear || r.timeout) {
          refreshStatus();
          acc.refresh();
        }
      }
    } catch (e) {
      setErr((e as Error).message);
    } finally {
      release();
      setPhase("idle");
    }
  };

  return (
    <section className="card section-sm" aria-labelledby="tausch-title">
      <div className="card-head">
        <h2 id="tausch-title">{tr("Tauschen", "Swap")}</h2>
        <span className={isMain ? "tag tag-warn" : "tag"}>{NETWORKS[network].label}</span>
      </div>
      <div className="tabs" role="group" aria-label={tr("Richtung", "Direction")}>
        {(["buy", "sell"] as Dir[]).map((d) => (
          <button
            key={d}
            type="button"
            aria-pressed={dir === d}
            className={dir === d ? "tab active" : "tab"}
            onClick={() => {
              setDir(d);
              setAmountStr("");
            }}
          >
            {d === "buy" ? (
              <>
                <TokenLabel token="KAS" size={16} /> → <TokenLabel token="GHOST" size={16} />
              </>
            ) : (
              <>
                <TokenLabel token="GHOST" size={16} /> → <TokenLabel token="KAS" size={16} />
              </>
            )}
          </button>
        ))}
      </div>
      {keyOk && (
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
          <legend className="sr-only">Tausch</legend>
          <AmountInput
            label={tr("Du gibst", "You pay")}
            value={amountStr}
            onChange={setAmountStr}
            suffix={inUnit}
            invalid={amountStr.trim() !== "" && amount === null}
            hint={
              [
                amount !== null && amount > 0n ? usd(amountStr, inUnit) : null,
                have !== null ? tr(`Vorhanden: ${de(have, 8)} ${inUnit}`, `Available: ${de(have, 8)} ${inUnit}`) : null,
                bandMaxUnits !== null && bandMaxUnits > 0n ? tr(`Kursband: höchstens ${formatUnits(bandMaxUnits, 8, 4)} ${inUnit}`, `Price band: at most ${formatUnits(bandMaxUnits, 8, 4)} ${inUnit}`) : null,
              ]
                .filter(Boolean)
                .join(" · ") || undefined
            }
          />
          {out !== null && out > 0n && (
            <dl className="kv">
              <div>
                <dt>{tr("Du bekommst etwa", "You receive about")}</dt>
                <dd>
                  <strong>
                    {formatUnits(out, 8, 8)} {outUnit}
                  </strong>
                  <Usd amount={Number(out) / 1e8} unit={outUnit} />
                </dd>
              </div>
              <div>
                <dt>{tr("Mindestens (1 % Spielraum)", "At least (1 % tolerance)")}</dt>
                <dd>
                  {formatUnits(min!, 8, 8)} {outUnit}
                  <Usd amount={Number(min!) / 1e8} unit={outUnit} />
                </dd>
              </div>
              {dir === "buy" && (
                <div>
                  <dt>{tr("Dazu gebunden", "Also bound")}</dt>
                  <dd>
                    {tr(
                      `1 ${NATIVE} im neuen ${STABLE_SYMBOL}-Token (kommt beim Weitergeben zurück) + Netzgebühr`,
                      `1 ${NATIVE} in the new ${STABLE_SYMBOL} token (returned when you pass it on) + network fee`,
                    )}
                  </dd>
                </div>
              )}
              <div>
                <dt>{tr("Kursverschiebung inkl. Gebühr", "Price impact incl. fee")}</dt>
                <dd className={impact !== null && impact > 300n ? "warn-text" : ""}>{impact !== null ? formatUnits(impact, 2, 2) : "–"} %</dd>
              </div>
            </dl>
          )}
          {impact !== null && impact > 300n && (
            <Callout kind="warn">
              {tr(
                "Der Tausch verschiebt den Kurs um mehr als 3 %. Der Pool ist klein – ein kleinerer Betrag ist günstiger.",
                "This swap moves the price by more than 3 %. The pool is small – a smaller amount is cheaper.",
              )}
            </Callout>
          )}
          {signMode === "key" && (
            <div className="btn-row">
              <button type="submit" className="btn btn-ghost" disabled={!params || phase !== "idle" || nodeDown} aria-busy={phase === "checking"}>
                {phase === "checking" ? tr("Prüfe …", "Checking …") : tr("Prüfen", "Check")}
              </button>
            </div>
          )}
        </fieldset>

        {signMode === "wallet" ? (
          <div className="result-col" aria-live="polite">
            <WalletSignFlow
              network={network}
              action="swap"
              label={dir === "buy" ? `${NATIVE} → ${STABLE_SYMBOL}` : `${STABLE_SYMBOL} → ${NATIVE}`}
              params={params}
              problem={problem}
              summary={summary}
              blocked={nodeDown}
              onDone={(any) => {
                if (any) {
                  refreshStatus();
                  void wallet.refresh();
                }
              }}
            />
          </div>
        ) : (
        <div className="result-col" aria-live="polite">
          {problem && <p className="muted small">{problem}</p>}
          {checked && checked.sig === sig && (
            <div className={checked.result.ok ? "result ok" : "result err"}>
              {checked.result.ok ? (
                <>
                  <strong>{tr("Probelauf erfolgreich – nichts gesendet.", "Dry run successful – nothing sent.")}</strong>
                  <p className="small">
                    <strong>{summary}</strong>
                  </p>
                  <p className="small">
                    {tr("Gebühr des Netzes", "Network fee")}: {de((checked.result.transactions ?? []).reduce((s, t) => s + t.feeKas, 0), 8)} KAS.{" "}
                    {tr(
                      "Tauschen andere vorher, baut ghostctl gegen den neuen Pool-Stand neu; unter dem Mindestbetrag bricht es ab und sendet nichts.",
                      "If others swap first, ghostctl rebuilds against the new pool state; below the minimum it aborts and sends nothing.",
                    )}
                  </p>
                </>
              ) : (
                <>
                  <strong>{tr("Probelauf abgelehnt", "Dry run rejected")}</strong>
                  <p>{checked.result.error}</p>
                </>
              )}
            </div>
          )}
          {checked && checked.sig === sig && checked.result.ok && !checkFresh && (
            <p className="muted small">{tr("Der Probelauf ist älter als 2 Minuten – bitte erneut prüfen.", "The dry run is older than 2 minutes – please check again.")}</p>
          )}
          {unclearLock && (
            <Callout kind="warn">
              {tr("Tauschen ist gesperrt, bis der Status neu geladen ist. Prüfe danach dein Guthaben.", "Swapping is blocked until the status has reloaded. Then check your balance.")}
            </Callout>
          )}
          <button
            type="button"
            className="btn btn-primary"
            disabled={!checkValid || phase !== "idle" || nodeDown || unclearLock}
            aria-busy={phase === "sending"}
            onClick={() => void exec(false)}
          >
            {phase === "sending" ? tr("Sende …", "Sending …") : isMain ? tr("Im Mainnet tauschen", "Swap on mainnet") : tr("Tauschen", "Swap")}
          </button>

          {err && <Callout kind="danger" title={tr("Fehler", "Error")}>{err}</Callout>}
          {sent && (
            <div className={sent.ok ? "result ok" : "result err"}>
              {sent.ok ? (
                <>
                  <strong>
                    {tr("Getauscht", "Swapped")}
                    {typeof sent.out === "number" ? tr(`: ${de(sent.out, 8)} ${dir === "buy" ? STABLE_SYMBOL : NATIVE} erhalten`, `: received ${de(sent.out, 8)} ${dir === "buy" ? STABLE_SYMBOL : NATIVE}`) : ""}.
                  </strong>
                  <ul className="tx-list">
                    {(sent.transactions ?? []).map((t) => (
                      <li key={t.txid}>
                        <code>{t.txid}</code> <CopyButton text={t.txid} label="TXID" />
                      </li>
                    ))}
                  </ul>
                </>
              ) : sent.unclear || sent.timeout || (sent.transactions ?? []).some((t) => t.sent) ? (
                <>
                  <strong>{tr("Ergebnis unklar – NICHT sofort erneut tauschen", "Outcome unclear – do NOT swap again right away")}</strong>
                  <p>{sent.error}</p>
                  <p className="small muted">{tr("Erst „Neu laden“ und das Guthaben prüfen.", "First “Reload” and check your balance.")}</p>
                </>
              ) : (
                <>
                  <strong>{tr("Nicht getauscht", "Not swapped")}</strong>
                  <p>{sent.error}</p>
                </>
              )}
            </div>
          )}
        </div>
        )}
      </form>
    </section>
  );
}

/** Tauschpool KAS/GHOST: Marktkurs, Tauschen, Liquidität des Besitzers */
export function Swap() {
  const { network, status } = useStatus();
  const acc = useAccount();
  const live = status?.deployed ? status : null;
  const pool = live?.pool ?? null;
  const x = pool ? BigInt(pool.kasSompi) : null;
  const y = pool ? BigInt(pool.ghostUnits) : null;
  // Kurs: KAS je GHOST und daraus mit dem Orakel der Dollarwert von 1 GHOST
  const kasPerGhost = x && y ? Number(x) / Number(y) : null;
  const ghostUsd = kasPerGhost !== null && live ? kasPerGhost * live.oracle.kasUsd : null;
  // eigene Anteile: Wert zum aktuellen Stand (Anteil an beiden Reserven)
  const myShares = acc.selected?.lpShares ? BigInt(acc.selected.lpShares) : 0n;
  const totalShares = pool ? BigInt(pool.shares) : 0n;
  const mine = pool && myShares > 0n && totalShares > 0n ? payoutFor(totalShares, x!, y!, myShares) : null;

  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          {tr("Tauschen", "Swap")}
        </h1>
        <span className="tag">{NETWORKS[network].label}</span>
      </div>
      <StatusNotices />
      <p className="lead">
        {tr(
          `Ein Tauschpool für ${NATIVE} und ${STABLE_SYMBOL} direkt auf Kaspa L1. Der Kurs ergibt sich aus dem Verhältnis der beiden Reserven (konstantes Produkt wie bei Uniswap). Er ist damit der Marktpreis von ${STABLE_SYMBOL}: Liegt er unter 1 USD, ist ${STABLE_SYMBOL} günstig zu haben.`,
          `A swap pool for ${NATIVE} and ${STABLE_SYMBOL} directly on Kaspa L1. The price follows from the ratio of the two reserves (constant product as in Uniswap). It is therefore the market price of ${STABLE_SYMBOL}: below 1 USD, ${STABLE_SYMBOL} is cheap.`,
        )}
      </p>

      <div className="stats-grid section-sm">
        <Stat label={<><TokenLabel token="KAS" size={16} /> {tr("im Pool", "in pool")}</>} value={x !== null ? de(Number(x) / 1e8, 2) : "–"} hint={x !== null ? <Usd amount={Number(x) / 1e8} unit="KAS" /> : undefined} />
        <Stat label={<><TokenLabel token="GHOST" size={16} /> {tr("im Pool", "in pool")}</>} value={y !== null ? de(Number(y) / 1e8, 4) : "–"} hint={y !== null ? <Usd amount={Number(y) / 1e8} unit="GHOST" /> : undefined} />
        <Stat label={tr(`1 ${STABLE_SYMBOL} kostet`, `1 ${STABLE_SYMBOL} costs`)} value={kasPerGhost !== null ? `${de(kasPerGhost, 2)} ${NATIVE}` : "–"} />
        <Stat
          label={tr(`Marktwert 1 ${STABLE_SYMBOL}`, `Market value 1 ${STABLE_SYMBOL}`)}
          value={ghostUsd !== null ? `${de(ghostUsd, 4)} USD` : "–"}
          hint={ghostUsd !== null ? `${ghostUsd >= 1 ? "+" : ""}${de((ghostUsd - 1) * 100, 2)} % ${tr("zum Dollar (mit Orakelpreis)", "vs. the dollar (using oracle price)")}` : undefined}
        />
      </div>

      {pool && pool.bandBps != null && (
        <Callout kind="info" title={tr(`Kursband: 1 USD ± ${de(pool.bandBps / 100, 1)} %`, `Price band: 1 USD ± ${de(pool.bandBps / 100, 1)} %`)}>
          {tr(
            "Der Pool lässt nur Tausche zu, nach denen GHOST (zum Orakelpreis) im Band liegt oder sich darauf zubewegt. Reicht die Liquidität nicht, ist die Richtung gesperrt, bis jemand einlegt oder zurücktauscht. ",
            "The pool only allows swaps after which GHOST (at the oracle price) is inside the band or moving towards it. If liquidity is insufficient, that direction is closed until someone adds liquidity or swaps back. ",
          )}
          {tr(
            `Gerade möglich: Kauf bis ${de(pool.maxBuyKas ?? 0, 4)} KAS, Verkauf bis ${de(pool.maxSellGhost ?? 0, 4)} GHOST.`,
            `Currently possible: buy up to ${de(pool.maxBuyKas ?? 0, 4)} KAS, sell up to ${de(pool.maxSellGhost ?? 0, 4)} GHOST.`,
          )}
        </Callout>
      )}
      {pool && pool.bandBps == null && (
        <Callout kind="warn" title={tr("Alter Pool ohne Kursband", "Old pool without price band")}>
          {tr(
            "Dieser Pool hält GHOST noch nicht bei 1 USD. Er wird durch einen Pool mit Kursband 1 USD ± 3 % ersetzt.",
            "This pool does not yet keep GHOST at 1 USD. It is being replaced by a pool with a 1 USD ± 3 % price band.",
          )}
        </Callout>
      )}

      {live && !live.oracle.fresh && (
        <Callout kind="warn" title={tr("Daten nicht aktuell", "Data not current")}>
          {tr(
            "Der letzte Abgleich mit der Kette ist fehlgeschlagen. Reserven und Dollarwert können veraltet sein.",
            "The last sync with the chain failed. Reserves and dollar value may be outdated.",
          )}
          {pool?.unresolved ? ` ${pool.unresolved}` : ""}
        </Callout>
      )}

      <div className="section-sm">
        <section className="card" aria-labelledby="pool-info">
          <div className="card-head">
            <h2 id="pool-info">{tr("So funktioniert der Pool", "How the pool works")}</h2>
          </div>
          <ul className="small">
            <li>{tr("Jeder darf tauschen. Der Vertrag prüft nur, dass das Produkt der Reserven nach Abzug von 0,3 % Gebühr nicht sinkt.", "Anyone may swap. The contract only checks that the product of the reserves, after the 0.3 % fee, does not decrease.")}</li>
            <li>{tr("Die Gebühr bleibt im Pool. Sie gehört allen Einlegern im Verhältnis ihrer Anteile.", "The fee stays in the pool. It belongs to all liquidity providers in proportion to their shares.")}</li>
            <li>{tr("Jeder darf einlegen und bekommt dafür Anteils-Token. Mit ihnen zieht man seinen Teil beider Reserven wieder ab.", "Anyone may add liquidity and receives share tokens. With them you withdraw your part of both reserves.")}</li>
            <li>
              {tr(
                "Jeder Tausch ist eine einzelne Transaktion gegen genau den aktuellen Pool. Tauscht jemand vorher, baut ghostctl gegen den neuen Stand neu und bricht unter dem Mindestbetrag ab. Weniger als den Mindestbetrag bekommst du nie.",
                "Each swap is a single transaction against exactly the current pool. If someone swaps first, ghostctl rebuilds against the new state and aborts below the minimum. You never get less than the minimum amount.",
              )}
            </li>
            <li>{tr(`Im Pool bleibt immer mindestens 1 ${NATIVE}.`, `At least 1 ${NATIVE} always stays in the pool.`)}</li>
          </ul>
          {pool && (
            <p className="small muted">
              {tr("Pool-Vertrag", "Pool contract")} <code>{pool.covenantId.slice(0, 16)}…</code> · {tr("Gebühr", "Fee")} {de(pool.feeBps / 100, 2)} %
            </p>
          )}
        </section>
      </div>

      <SwapCard />

      {live && !pool && (
        <>
          {!pool && (
            <Callout kind="info" title={tr("Noch kein Pool", "No pool yet")}>
              {tr(
                "In diesem Netz ist noch kein Tauschpool angelegt. Wer ihn anlegt, stellt die erste Liquidität. Der Startkurs muss bei 1 USD ± 3 % liegen (Orakelpreis).",
                "No swap pool exists on this network yet. Whoever creates it provides the first liquidity. The starting price must be 1 USD ± 3 % (oracle price).",
              )}
            </Callout>
          )}
          <ActionForms
            prefill={null}
            actions={["pool-open"]}
            title={pool ? tr("Neuen Pool mit Kursband anlegen", "Create new pool with price band") : tr("Pool anlegen", "Create pool")}
            id="pool-anlegen"
          />
        </>
      )}
      {pool && (
        <>
          <section className="card section-sm" aria-labelledby="anteile-title">
            <div className="card-head">
              <h2 id="anteile-title">{tr("Deine Pool-Anteile", "Your pool shares")}</h2>
            </div>
            {mine ? (
              <dl className="kv">
                <div>
                  <dt>{tr("Anteile", "Shares")}</dt>
                  <dd>
                    {myShares.toString()} {tr("von", "of")} {totalShares.toString()} ({de((Number(myShares) / Number(totalShares)) * 100, 4)} %)
                  </dd>
                </div>
                <div>
                  <dt>{tr("Heutiger Gegenwert", "Current value")}</dt>
                  <dd>
                    {formatUnits(mine[0], 8, 4)} {NATIVE} {tr("und", "and")} {formatUnits(mine[1], 8, 6)} {STABLE_SYMBOL}
                  </dd>
                </div>
              </dl>
            ) : (
              <p className="muted small">
                {tr(
                  "Dieser Schlüssel hat keine Anteile. Wer einlegt, bekommt Anteile und damit einen Teil der Tauschgebühren.",
                  "This key has no shares. Whoever adds liquidity receives shares and thus part of the swap fees.",
                )}
              </p>
            )}
            <p className="small muted">
              {tr(
                `Der Gegenwert ändert sich mit jedem Tausch: Gebühren erhöhen ihn, eine Kursbewegung verschiebt ihn zwischen KAS und ${STABLE_SYMBOL} (bekannt als „impermanenter Verlust“). 1 ${NATIVE} Mindestliquidität gehört niemandem und bleibt immer im Pool.`,
                `The value changes with every swap: fees increase it, a price move shifts it between KAS and ${STABLE_SYMBOL} (known as “impermanent loss”). 1 ${NATIVE} of minimum liquidity belongs to no one and always stays in the pool.`,
              )}
            </p>
          </section>
          <ActionForms prefill={null} actions={["pool-add", "pool-remove"]} title={tr("Liquidität", "Liquidity")} id="liquiditaet" />
        </>
      )}
      {!live && <p className="muted small section-sm">{tr("Ohne Live-Daten gibt es hier nichts zu tauschen.", "Without live data there is nothing to swap here.")}</p>}
    </div>
  );
}

