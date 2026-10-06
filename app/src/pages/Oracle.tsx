import { ActionForms } from "../components/ActionForms";
import { StatusNotices } from "../components/Network";
import { OracleCard } from "../components/OracleCard";
import { Callout } from "../components/ui";
import { MAX_SIGNERS, NATIVE, NETWORKS, STABLE_SYMBOL } from "../config";
import { useStatus } from "../lib/StatusContext";
import { tr } from "../lib/i18n";
import { usePublicMode } from "../lib/AccountContext";

/** Alles zum Preis-Orakel: Stand, Funktionsweise, Kontrolle, Dauerbetrieb, Update von Hand */
export function Oracle() {
  const { network } = useStatus();
  const pub = usePublicMode();
  return (
    <div className="container section">
      <div className="section-head">
        <h1 tabIndex={-1} data-route-heading>
          {tr("Orakel", "Oracle")}
        </h1>
        <span className="tag">{NETWORKS[network].label}</span>
      </div>
      <StatusNotices />
      <p className="lead">
        {tr(
          `Das Orakel liefert den ${NATIVE}-Preis in US-Dollar. Nach ihm richten sich Prägen, Abheben und Liquidieren. Es ist ein eigener Vertrag auf Kaspa L1. Die Signaturen jedes Preis-Updates prüft ein zweiter Vertrag, das Unterzeichner-Register.`,
          `The oracle provides the ${NATIVE} price in US dollars. Minting, withdrawing and liquidating depend on it. It is a separate contract on Kaspa L1. The signatures of every price update are checked by a second contract, the signer register.`,
        )}
      </p>

      <div className="grid-2 section-sm">
        <OracleCard />
        <section className="card" aria-labelledby="oracle-how">
          <div className="card-head">
            <h2 id="oracle-how">{tr("So arbeitet es", "How it works")}</h2>
          </div>
          <ul className="small">
            <li>
              <strong>{tr("Preis:", "Price:")}</strong>{" "}
              {tr(
                "Median aus 6 Quellen (api.kaspa.org, CoinGecko, MEXC, Gate, KuCoin, Bybit). Ausreißer fallen heraus.",
                "Median of 6 sources (api.kaspa.org, CoinGecko, MEXC, Gate, KuCoin, Bybit). Outliers are dropped.",
              )}
            </li>
            <li>
              <strong>{tr("Signatur:", "Signature:")}</strong>{" "}
              {tr(
                `Im Register steht nur der Hash des Unterzeichner-Satzes: n Schlüssel (höchstens ${MAX_SIGNERS}), Preis-Schwelle t und Austausch-Schwelle. t muss eine echte Mehrheit sein (2t > n). Fehlen Signaturen, nimmt der Vertrag den Preis nicht an. Zum Start gibt es 1 Unterzeichner, den Betreiber, und 1 Signatur genügt.`,
                `The register only stores the hash of the signer set: n keys (at most ${MAX_SIGNERS}), price threshold t and rotation threshold. t must be a true majority (2t > n). If signatures are missing, the contract rejects the price. At launch there is 1 signer, the operator, and 1 signature suffices.`,
              )}
            </li>
            <li>
              <strong>{tr("Grenzen im Vertrag:", "Limits in the contract:")}</strong>{" "}
              {tr(
                `0,00001 bis 900 USD je ${NATIVE}, je Update höchstens ×2 bzw. ÷2. Diese Grenzen schützen vor Rechenfehlern, nicht vor Unterzeichnern, die bewusst einen falschen Preis setzen.`,
                `0.00001 to 900 USD per ${NATIVE}, at most ×2 or ÷2 per update. These limits protect against calculation errors, not against signers deliberately setting a wrong price.`,
              )}
            </li>
            <li>
              <strong>{tr("Einfrieren:", "Freezing:")}</strong>{" "}
              {tr(
                "Kommt 2 Stunden lang kein Preis, darf jeder das Orakel einfrieren; der GHOST-Agent tut das automatisch. Eingefroren sind Prägen, Rücknahme, Liquidieren, Auflösen, Abheben bei offener Schuld und der Tausch im Pool gesperrt. Einzahlen, Tilgen, Schließen, Abheben ohne Schuld sowie Liquidität einlegen und abziehen gehen weiter. Das nächste gültige Preis-Update taut alles wieder auf.",
                "If no price arrives for 2 hours, anyone may freeze the oracle; the GHOST agent does this automatically. While frozen, minting, redemption, liquidation, dissolving, withdrawing with open debt and swapping in the pool are blocked. Depositing, repaying, closing, withdrawing without debt and adding or removing liquidity still work. The next valid price update unfreezes everything.",
              )}
            </li>
            <li>
              <strong>{tr("Immer nur der neueste Preis:", "Only the latest price:")}</strong>{" "}
              {tr(
                "Das Orakel ist eine einzige UTXO. Wer den Preis nutzt, gibt sie aus und erzeugt sie unverändert neu. Ältere Preise sind dadurch verbraucht.",
                "The oracle is a single UTXO. Whoever uses the price spends it and recreates it unchanged. Older prices are thereby consumed.",
              )}
            </li>
            <li>
              <strong>{tr("Kein Einfluss des Tauschpools:", "No influence from the swap pool:")}</strong>{" "}
              {tr(
                `Der Poolkurs von ${STABLE_SYMBOL} ist ein Marktsignal. Das Orakel liest ihn nicht.`,
                `The ${STABLE_SYMBOL} pool price is a market signal. The oracle does not read it.`,
              )}
            </li>
          </ul>
        </section>
      </div>

      <div className="grid-2 section-sm">
        <section className="card" aria-labelledby="oracle-who">
          <div className="card-head">
            <h2 id="oracle-who">{tr("Wer es kontrolliert", "Who controls it")}</h2>
          </div>
          <p className="small">
            {tr(
              "Zum Start im Alleinbetrieb ist der Betreiber der einzige Unterzeichner, 1 Signatur genügt. Er kann damit jeden Preis innerhalb der Grenzen setzen und so Vaults liquidierbar machen oder ungedeckte Prägung erlauben.",
              "At launch, in solo operation, the operator is the only signer and 1 signature suffices. They can therefore set any price within the limits and thereby make vaults liquidatable or allow uncovered minting.",
            )}
          </p>
          <p className="small">
            {tr(
              "Weitere unabhängige Unterzeichner lassen sich später ohne neues Deployment einsetzen. Jeder lässt einen eigenen Agenten laufen, der nur signiert, wenn sein eigener Preisabruf passt.",
              "Further independent signers can be added later without a new deployment. Each runs their own agent that only signs if its own price check matches.",
            )}
          </p>
          <p className="small">
            <strong>{tr("Austausch der Unterzeichner:", "Replacing the signers:")}</strong>{" "}
            {tr(
              "Ein neuer Satz wird öffentlich angekündigt (die Ankündigung steht in der Transaktion) und erst nach 14 Tagen Wartezeit aktiviert; aktivieren darf dann jeder. Die Wartezeit erzwingt der Konsens über eine relative Sperre. In dieser Zeit kann der aktuelle Satz die Ankündigung absagen, und wer dem neuen Satz nicht traut, kann seinen Vault schließen oder GHOST einlösen.",
              "A new set is announced publicly (the announcement is in the transaction) and only activated after a 14-day waiting period; anyone may then activate it. The waiting period is enforced by consensus via a relative lock. During it the current set can cancel the announcement, and anyone who does not trust the new set can close their vault or redeem GHOST.",
            )}
          </p>
          <p className="small">
            {tr(
              "Ein optionaler Notfallsatz darf erst nach 30 Tagen ohne Preis-Update einen Austausch ankündigen, danach gelten wieder 14 Tage Wartezeit. Jedes Preis-Update macht diese Ankündigung ungültig. Zum Start gibt es keinen Notfallsatz.",
              "An optional emergency set may only announce a replacement after 30 days without a price update, followed again by a 14-day waiting period. Every price update invalidates that announcement. At launch there is no emergency set.",
            )}
          </p>
        </section>
        {!pub && (
        <section className="card" aria-labelledby="oracle-agent">
          <div className="card-head">
            <h2 id="oracle-agent">{tr("Dauerbetrieb: GHOST-Agent", "Continuous operation: GHOST agent")}</h2>
          </div>
          <p className="small">
            {tr("Im Projektordner doppelt auf ", "Double-click ")}
            <code>GHOST-Agent starten.command</code>
            {tr(
              " klicken und das Fenster offen lassen. Alle 5 Minuten prüft der Agent den Preis und sendet ein Update, wenn er sich um 0,5 % bewegt hat oder das letzte Update 60 Minuten alt ist (die halbe Einfrier-Frist). Sprünge über 20 % sendet er erst nach 3 bestätigenden Runden und in Schritten.",
              " in the project folder and keep the window open. Every 5 minutes the agent checks the price and sends an update if it moved by 0.5 % or the last update is 60 minutes old (half the freeze deadline). Jumps above 20 % are sent only after 3 confirming rounds and in steps.",
            )}
          </p>
          <p className="small">
            {tr(
              "Nebenbei löst er Vaults unter 150 % ab, aber nur, wenn auch der aktuelle Marktpreis die Unterdeckung zeigt. Kommt 2 Stunden lang kein Preis, friert er das Orakel ein. Ohne Unterzeichner-Schlüssel arbeitet er nur als Liquidator und Einfrierer.",
              "It also liquidates vaults below 150 %, but only if the current market price confirms the shortfall. If no price arrives for 2 hours, it freezes the oracle. Without a signer key it works only as a liquidator and freezer.",
            )}
          </p>
        </section>
        )}
      </div>

      {!pub && (
        <>
      <Callout kind="info" title={tr("Update von Hand", "Manual update")}>
        {tr(
          "Nur mit der Schlüsseldatei der Unterzeichner möglich. Zwischen zwei Updates müssen mindestens 600 DAA liegen (etwa 1 Minute).",
          "Only possible with the signers' key file. At least 600 DAA (about 1 minute) must pass between two updates.",
        )}
      </Callout>
      <ActionForms prefill={null} actions={["oracle-update"]} title={tr("Orakel aktualisieren", "Update oracle")} id="orakel-update" />
        </>
      )}
    </div>
  );
}
