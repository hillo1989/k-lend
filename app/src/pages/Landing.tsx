import { NATIVE, NETWORKS, STABLE_SYMBOL, stableTagline } from "../config";
import { BlockDag } from "../components/BlockDag";
import { Stat } from "../components/ui";
import { tr } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { de } from "../lib/status";
import { pct } from "../lib/demo";
import { href } from "../router";

const facts = () => [
  {
    title: tr("L1-nativ, ohne Bridge", "L1-native, no bridge"),
    text: tr(
      "Vaults, Orakel und GHOST sind Verträge direkt auf Kaspa L1 (Covenants). Deine KAS verlassen die Kette nicht und müssen nicht über eine Brücke.",
      "Vaults, oracle and GHOST are contracts directly on Kaspa L1 (covenants). Your KAS never leave the chain and never cross a bridge.",
    ),
    icon: "M4 12h16M8 8l-4 4 4 4M16 8l4 4-4 4",
  },
  {
    title: tr("Überbesichert", "Overcollateralized"),
    text: tr(
      "Prägen geht nur bis zu einer Quote von 200 %. Unter 150 % darf jeder den Vault ablösen. Garantiert ist das nicht: Ein Keeper-Agent ist gebaut, aber ob einer läuft, hängt vom Betreiber ab. Bei einem Kurssturz können GHOST ungedeckt werden.",
      "Minting is only possible up to a ratio of 200 %. Below 150 % anyone may liquidate the vault. This is not guaranteed: a keeper agent exists, but whether one runs depends on the operator. In a crash GHOST can become undercollateralized.",
    ),
    icon: "M12 3l8 4v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7z",
  },
  {
    title: tr("Tauschpool und Marktpreis", "Swap pool and market price"),
    text: tr(
      "Im Pool tauschst du KAS und GHOST direkt auf L1, sein Kurs ist der Marktpreis von GHOST. Liegt er unter 1 USD, steigt der Zins automatisch, darüber sinkt er. Dazu kann jeder GHOST zu 1 USD (minus 1 %) gegen KAS zurückgeben.",
      "In the pool you swap KAS and GHOST directly on L1; its price is the market price of GHOST. Below 1 USD the interest rate rises automatically, above it falls. On top, anyone can redeem GHOST for KAS at 1 USD (minus 1 %).",
    ),
    icon: "M3 17l6-6 4 4 8-8M15 7h6v6",
  },
  {
    title: tr("Agent als Keeper und Wächter", "Agent as keeper and guardian"),
    text: tr(
      "Der GHOST-Agent hält das Orakel frisch und löst unterdeckte Vaults ab, aber nur, wenn auch der Marktpreis es bestätigt. Jeder kann ihn als Liquidator betreiben.",
      "The GHOST agent keeps the oracle fresh and liquidates undercollateralized vaults, but only if the market price confirms it. Anyone can run it as a liquidator.",
    ),
    icon: "M9 3h6v3H9zM5 8h14v11H5zM9 13h.01M15 13h.01M9 17h6",
  },
];

export function Landing() {
  const { status, network } = useStatus();
  const live = status?.deployed ? status : null;
  const dash = "–";
  return (
    <>
      <section className="hero">
        <div className="hero-dag">
          <BlockDag />
        </div>
        <div className="container hero-inner">
          <p className="eyebrow">
            {tr(
              "Experimentell · nicht professionell geprüft · Mainnet-Probelauf mit Kleinstbeträgen",
              "Experimental · not professionally audited · mainnet trial with tiny amounts",
            )}
          </p>
          <h1 tabIndex={-1} data-route-heading>
            {tr("Stablecoin und Kredite", "Stablecoin and loans")} <span className="grad">{tr("direkt auf Kaspa L1", "directly on Kaspa L1")}</span>
          </h1>
          <p className="tagline">{stableTagline()}</p>
          <p className="lead">
            {tr(
              `Hinterlege ${NATIVE} als Sicherheit und präge ${STABLE_SYMBOL}, einen Dollar-Stablecoin. Tausche ${NATIVE} und ${STABLE_SYMBOL} im Pool und sende beides mit der eingebauten Wallet. Alles über Verträge auf der Basiskette, ohne Verwahrer und ohne Bridge.`,
              `Deposit ${NATIVE} as collateral and mint ${STABLE_SYMBOL}, a dollar stablecoin. Swap ${NATIVE} and ${STABLE_SYMBOL} in the pool and send both with the built-in wallet. Everything through contracts on the base chain, without custodians and without a bridge.`,
            )}
          </p>
          <div className="btn-row">
            <a className="btn btn-primary btn-lg" href={href("vault")}>
              {tr(`${STABLE_SYMBOL} prägen`, `Mint ${STABLE_SYMBOL}`)}
            </a>
            <a className="btn btn-ghost btn-lg" href={href("tauschen")}>
              {tr("Tauschen", "Swap")}
            </a>
            <a className="btn btn-ghost btn-lg" href={href("wallet")}>
              Wallet
            </a>
          </div>
          <p className="small hero-more">
            <a href={href("so-funktioniert-es")}>{tr("So funktioniert es →", "How it works →")}</a>
          </p>
        </div>
      </section>

      <section className="container section" aria-labelledby="kennzahlen">
        <div className="section-head">
          <h2 id="kennzahlen">{tr("Kennzahlen", "Key figures")}</h2>
          <span className="tag">
            {live
              ? `live · ${NETWORKS[network].label}`
              : status
                ? `${NETWORKS[network].label} · ${tr("noch nicht angelegt", "not deployed yet")}`
                : tr("lädt …", "loading …")}
          </span>
        </div>
        <div className="stats-grid">
          <Stat label={tr(`${STABLE_SYMBOL} im Umlauf`, `${STABLE_SYMBOL} in circulation`)} value={live ? `${de(live.totals.debtGhost, 8)} ${STABLE_SYMBOL}` : dash} />
          <Stat label={tr("Hinterlegte Sicherheit", "Collateral deposited")} value={live ? `${de(live.totals.collateralKas, 2)} ${NATIVE}` : dash} />
          <Stat label={tr(`${STABLE_SYMBOL}-Zins p. a.`, `${STABLE_SYMBOL} interest p.a.`)} value={live ? pct(live.oracle.ratePctYear) : dash} />
          <Stat label={tr("KAS-Preis (Orakel)", "KAS price (oracle)")} value={live ? `${de(live.oracle.kasUsd, 6, 2)} USD` : dash} />
        </div>
        {status && !status.deployed && (
          <p className="muted small">{tr(`Auf dem ${NETWORKS[network].label} ist noch nichts angelegt.`, `Nothing is deployed on ${NETWORKS[network].label} yet.`)}</p>
        )}
        <p className="small">
          <a href={href("statistiken")}>{tr("Alle Statistiken →", "All statistics →")}</a>
        </p>
      </section>

      <section className="container section" aria-labelledby="warum">
        <h2 id="warum">{tr("Was das Protokoll ausmacht", "What sets the protocol apart")}</h2>
        <div className="facts-grid">
          {facts().map((f) => (
            <article className="card fact" key={f.title}>
              <svg className="fact-icon" viewBox="0 0 24 24" aria-hidden="true">
                <path d={f.icon} />
              </svg>
              <h3>{f.title}</h3>
              <p>{f.text}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="container section" aria-labelledby="schritte">
        <h2 id="schritte">{tr("In drei Schritten", "In three steps")}</h2>
        <ol className="steps">
          <li>
            <strong>{tr("Konto anlegen.", "Create an account.")}</strong>{" "}
            {tr(
              `Unter „Wallet“ eine Schlüsseldatei anlegen oder wählen und die Adresse mit ${NATIVE} aufladen. Die Datei liegt nur auf deinem Rechner.`,
              `Under “Wallet”, create or choose a key file and fund the address with ${NATIVE}. The file stays only on your computer.`,
            )}
          </li>
          <li>
            <strong>{tr("Vault planen.", "Plan a vault.")}</strong>{" "}
            {tr(
              `Im Rechner ${NATIVE}-Sicherheit und ${STABLE_SYMBOL}-Betrag eintragen und Gesundheitsfaktor und Liquidationspreis prüfen. Startwerte kommen vom Live-Orakel.`,
              `In the calculator, enter ${NATIVE} collateral and ${STABLE_SYMBOL} amount and check the health factor and liquidation price. Starting values come from the live oracle.`,
            )}
          </li>
          <li>
            <strong>{tr("Prüfen, dann senden.", "Check, then send.")}</strong>{" "}
            {tr(
              "Jede Aktion läuft erst als Probelauf mit Gebühr und Ergebnis. Danach signiert und sendet der lokale Server sie mit ghostctl. Im Mainnet ist zusätzlich eine ausdrückliche Bestätigung nötig.",
              "Every action first runs as a dry run showing fee and result. Then the local server signs and sends it with ghostctl. On mainnet an explicit confirmation is required as well.",
            )}
          </li>
        </ol>
      </section>
    </>
  );
}
