import { useEffect, useState } from "react";
import { NETWORKS, oracleStaleMinutes } from "../config";
import { tr, useLang } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { de, oracleStale } from "../lib/status";
import { GhostIcon, KasIcon } from "./TokenIcons";
import { Callout } from "./ui";
import { useBusy } from "../lib/busy";
import { useKasUsd } from "../lib/usd";

/** Aktueller KAS-Marktpreis (Median aus 6 Quellen über ghostctl price), jede
 * Minute neu – ein gemeinsamer Abruf für die ganze Seite (lib/usd.ts) */
export function useKasPrice(): number | null {
  return useKasUsd();
}

/** Netz, Status-Pille, KAS-Preis und Neu laden – sitzt unter der Kopfzeile. */
export function NetworkBar() {
  const busy = useBusy();
  const s = useStatus();
  const net = NETWORKS[s.network];
  const price = useKasPrice();
  const { lang, setLang } = useLang();
  const [, force] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => force((x) => x + 1), 5_000);
    return () => clearInterval(t);
  }, []);

  let pill: { text: string; cls: string };
  if (s.nodeDown) pill = { text: `${net.short} · ${tr("Nodes nicht erreichbar", "nodes unreachable")}`, cls: "pill pill-err" };
  else if (s.error && !s.status) pill = { text: `${net.short} · ${tr("keine Daten", "no data")}`, cls: "pill pill-err" };
  else if (!s.status) pill = { text: `${net.short} · ${tr("lädt …", "loading …")}`, cls: "pill" };
  else if (!s.status.deployed) pill = { text: `${net.short} · ${tr("v2 noch nicht angelegt", "v2 not deployed yet")}`, cls: "pill pill-warn" };
  else if (oracleStale(s.status.oracle, s.status.signers?.freezeAfterHours)) pill = { text: `${net.short} · ${tr("live, Preis veraltet", "live, price stale")}`, cls: "pill pill-warn" };
  else pill = { text: `${net.short} · live`, cls: "pill pill-live" };

  // GHOST-Marktpreis aus dem Tauschpool, bewertet mit dem KAS-Marktpreis
  const pool = s.status?.deployed ? (s.status.pool ?? null) : null;
  const kasPerGhost = pool && !pool.unresolved && Number(pool.ghostUnits) > 0 ? Number(pool.kasSompi) / Number(pool.ghostUnits) : null;
  const ghostUsd = kasPerGhost !== null && price !== null ? kasPerGhost * price : null;

  const ago = s.updatedAt ? Math.max(0, Math.round((Date.now() - s.updatedAt) / 1000)) : null;

  return (
    <div className="netbar">
      <div className="container netbar-row">
        <div className="netbar-left">
          <span className="netbar-label">
            {tr("Netz", "Network")}: {net.label}
          </span>
          <span className={pill.cls} role="status" title={pill.text}>
            <span className="pill-dot" aria-hidden="true" />
            <span className="pill-text">{pill.text}</span>
          </span>
        </div>
        <span className="netbar-prices">
          <span className="netbar-price" title={tr("KAS-Marktpreis: Median aus 6 Quellen, jede Minute neu", "KAS market price: median of 6 sources, refreshed every minute")}>
            <KasIcon size={16} /> {price !== null ? `${de(price, 6, 4)} USD` : "–"}
          </span>
          {ghostUsd !== null && (
            <span
              className="netbar-price"
              title={tr(
                `GHOST-Marktpreis: Kurs im Tauschpool (${de(kasPerGhost!, 2)} KAS je GHOST) × KAS-Marktpreis`,
                `GHOST market price: swap pool rate (${de(kasPerGhost!, 2)} KAS per GHOST) × KAS market price`,
              )}
            >
              <GhostIcon size={16} /> {`${de(ghostUsd, 4, 3)} USD`}
            </span>
          )}
        </span>
        <div className="netbar-right">
          <span className="netbar-meta muted small">
            {s.loading
              ? tr("lädt …", "loading …")
              : ago !== null
                ? tr(`Stand vor ${ago} s`, `updated ${ago} s ago`)
                : ""}
          </span>
          <button className="btn btn-ghost btn-sm netbar-reload" onClick={s.refresh} disabled={s.loading}>
            {tr("Neu laden", "Reload")}
          </button>
          <div className="lang-switch" role="group" aria-label={tr("Sprache", "Language")}>
            {(["de", "en"] as const).map((l) => (
              <button
                key={l}
                type="button"
                aria-pressed={lang === l}
                className={lang === l ? "active" : ""}
                onClick={() => setLang(l)}
                disabled={busy && lang !== l}
                title={busy ? tr("Während eine Aktion läuft, nicht möglich", "Not possible while an action is running") : undefined}
              >
                {l.toUpperCase()}
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/** Hinweise zum Datenstand: Nodes weg, Fehler, kein Deployment, veraltetes Orakel, fremd veränderte Vaults. */
export function StatusNotices() {
  const s = useStatus();
  const notDeployed = s.status && !s.status.deployed && !s.error;
  const stale = s.status?.deployed ? s.status.vaults.filter((v) => v.stale) : [];
  return (
    <>
      {s.nodeDown ? (
        <Callout kind="warn" title={tr("Öffentliche Kaspa-Nodes nicht erreichbar", "Public Kaspa nodes unreachable")}>
          {tr(
            "ghostctl bekommt gerade keine Verbindung zu den öffentlichen Kaspa-Nodes. Bitte später erneut versuchen. Die Seite fragt alle 30 Sekunden neu an. ",
            "ghostctl cannot reach the public Kaspa nodes right now. Please try again later. The page retries every 30 seconds. ",
          )}
          {s.status?.deployed ? tr("Angezeigt wird der letzte erfolgreiche Stand. ", "Showing the last successful state. ") : ""}
          {tr("Aktionen und Guthaben sind so lange nicht verfügbar.", "Actions and balances are unavailable until then.")}
        </Callout>
      ) : (
        s.error && (
          <Callout kind="danger" title={tr("Daten konnten nicht geladen werden", "Could not load data")}>
            {s.error}
            {s.status?.deployed && tr(" Angezeigt wird der letzte erfolgreiche Stand.", " Showing the last successful state.")}
          </Callout>
        )
      )}
      {notDeployed && (
        <div className="card notice-card">
          <h2>{tr("Version 2 noch nicht im Mainnet angelegt", "Version 2 not yet deployed on mainnet")}</h2>
          <p>
            {tr("Zum Anlegen im Projektordner doppelt auf ", "To deploy, double-click ")}
            <code>GHOST-Mainnet-Test.command</code>
            {tr(" klicken (siehe ", " in the project folder (see ")}
            <code>MAINNET.md</code>
            {tr("). Danach erscheinen hier die Live-Daten.", "). Live data will appear here afterwards.")}
          </p>
        </div>
      )}
      {stale.length > 0 && (
        <Callout kind="warn" title={tr("Von Dritten veränderte Vaults", "Vaults changed by third parties")}>
          {stale.map((v) => `Vault ${v.index}`).join(", ")}{" "}
          {tr(
            `${stale.length === 1 ? "wurde" : "wurden"} außerhalb dieses Rechners verändert, zum Beispiel liquidiert. Aktionen darauf sind gesperrt, bis ghostctl den Stand nachgeladen hat.`,
            `${stale.length === 1 ? "was" : "were"} changed outside this computer, for example liquidated. Actions on them are blocked until ghostctl has reloaded the state.`,
          )}
        </Callout>
      )}
      {s.status?.deployed && oracleStale(s.status.oracle, s.status.signers?.freezeAfterHours) && (
        <Callout kind="warn" title={tr("Orakelpreis veraltet", "Oracle price stale")}>
          {tr(`Der letzte Preis ist ${de(s.status.oracle.ageMinutes, 0)} Minuten alt`, `The last price is ${de(s.status.oracle.ageMinutes, 0)} minutes old`)}
          {s.status.oracle.ageMinutes > oracleStaleMinutes(s.status.signers?.freezeAfterHours) ? tr(` (Grenze ${oracleStaleMinutes(s.status.signers?.freezeAfterHours)} min)`, ` (limit ${oracleStaleMinutes(s.status.signers?.freezeAfterHours)} min)`) : ""}
          {s.status.oracle.frozen
            ? tr(" – das Orakel ist eingefroren", " – the oracle is frozen")
            : (s.status.oracle.freezeInMinutes ?? 1) <= 0
              ? tr(" – jeder kann es jetzt einfrieren", " – anyone can freeze it now")
              : typeof s.status.oracle.freezeInMinutes === "number"
                ? tr(` – einfrierbar in ${de(s.status.oracle.freezeInMinutes, 0)} min`, ` – can be frozen in ${de(s.status.oracle.freezeInMinutes, 0)} min`)
                : ""}
          .
          {s.status.oracle.freshError ? tr(` ghostctl meldet: ${s.status.oracle.freshError}.`, ` ghostctl reports: ${s.status.oracle.freshError}.`) : ""}{" "}
          {tr(
            "Kennzahlen, Quoten und Liquidationspreise beruhen auf diesem alten Preis. Aktualisieren im Reiter „Orakel“.",
            "Figures, ratios and liquidation prices are based on this old price. Update it in the “Oracle” tab.",
          )}
        </Callout>
      )}
    </>
  );
}
