import type { MouseEvent } from "react";
import { NATIVE, PARAMS, STABLE_SYMBOL } from "../config";
import { ProtocolDiagram } from "../components/Diagram";
import { fmtBps, fmtKas, fmtStable, fmtUsdPrice, formatUnits } from "../lib/format";
import { INDEX_SCALE, UNIT, collateralValue, minHealthyPrice, ratioBps, simLiquidate, type VaultState } from "../lib/vaultMath";
import { tr } from "../lib/i18n";
import { useStatus } from "../lib/StatusContext";
import { shortHex } from "../lib/status";

// Rechenbeispiel – mit dem Rechenkern gerechnet, damit Text und Vertrag übereinstimmen.
// 35 GHOST: unter der Obergrenze von 50 GHOST je Vault; ohne Zins, damit die Zahlen rund bleiben.
const EX_COLL = 1_000n * UNIT;
const EX_MINT = 35n * UNIT;
const EX_P0 = 8_000_000n; // 0,08 USD
const EX_P1 = 5_000_000n; // 0,05 USD
const exVault: VaultState = { collateral: EX_COLL, debt: EX_MINT, interest: 0n, indexAt: INDEX_SCALE };
const ex = {
  value0: collateralValue(EX_COLL, EX_P0),
  ratio0: ratioBps(EX_COLL, EX_MINT, EX_P0)!,
  liqPrice: minHealthyPrice(EX_COLL, EX_MINT, PARAMS.liqBps)!,
  value1: collateralValue(EX_COLL, EX_P1),
  ratio1: ratioBps(EX_COLL, EX_MINT, EX_P1)!,
  liq: simLiquidate(exVault, { kasUsd: EX_P1, stableIndex: INDEX_SCALE }, PARAMS.liqBps, PARAMS.bonusBps),
};
const exSeized = ex.liq.ok ? (ex.liq.seized ?? 0n) : 0n;
const exBonus = (EX_MINT * PARAMS.bonusBps) / 10_000n;
// Unterdeckung: Kurs 0,036 USD → 36 USD Sicherheit < 35 GHOST + 10 %. Der Liquidator
// verbrennt nur so viel, wie die Sicherheit samt Bonus deckt; der Rest wird ausgebucht.
const EX_P2 = 3_600_000n;
const exValue2 = collateralValue(EX_COLL, EX_P2);
const exBurn2 = (exValue2 * 10_000n) / (10_000n + PARAMS.bonusBps);
const exLiq2 = simLiquidate(exVault, { kasUsd: EX_P2, stableIndex: INDEX_SCALE }, PARAMS.liqBps, PARAMS.bonusBps, exBurn2);
const exRest = ex.liq.ok && ex.liq.state ? ex.liq.state.collateral : 0n;

const usd = (v: bigint) => `${formatUnits(v, 8, 2)} USD`;

export function HowItWorks() {
  const { status } = useStatus();
  const live = status?.deployed ? status : null;
  return (
    <div className="container section prose">
      <h1 tabIndex={-1} data-route-heading>
        {tr("So funktioniert es", "How it works")}
      </h1>
      <p className="lead">
        {tr(
          `Eine Erklärung ohne Fachchinesisch – für alle, die wissen wollen, was mit ihren ${NATIVE} passiert, bevor sie etwas tun.`,
          `A plain-language explanation – for anyone who wants to know what happens to their ${NATIVE} before doing anything.`,
        )}
      </p>

      <nav className="toc card" aria-label={tr("Inhalt dieser Seite", "Contents of this page")}>
        <ol>
          <li><a href="#vault" onClick={jump}>{tr("Was ist ein Vault?", "What is a vault?")}</a></li>
          <li><a href="#ueberbesichert" onClick={jump}>{tr("Überbesicherung", "Overcollateralization")}</a></li>
          <li><a href="#zins" onClick={jump}>{tr("Zins und Kurs", "Interest and price")}</a></li>
          <li><a href="#liquidation" onClick={jump}>{tr("Liquidation mit Beispiel", "Liquidation with example")}</a></li>
          <li><a href="#orakel" onClick={jump}>{tr("Das Orakel", "The oracle")}</a></li>
          <li><a href="#keeper" onClick={jump}>{tr("Keeper und KI-Agenten", "Keepers and AI agents")}</a></li>
          <li><a href="#risiken" onClick={jump}>{tr("Risiken", "Risks")}</a></li>
        </ol>
      </nav>

      <ProtocolDiagram />

      <section id="vault">
        <h2>{tr("Was ist ein Vault?", "What is a vault?")}</h2>
        <p>
          {tr(
            `Ein Vault ist ein kleiner Tresor auf der Kaspa-Kette, der nur dir gehört. Du legst ${NATIVE} hinein und darfst dafür ${STABLE_SYMBOL} erzeugen („prägen“) – einen Stablecoin, der 1 US-Dollar wert sein soll. Die ${NATIVE} bleiben im Vault, bis du die ${STABLE_SYMBOL} zurückgibst („tilgst“). Den Vault regelt ein Vertrag: Er rechnet nach, ob genug Sicherheit da ist, und lehnt alles ab, was die Regeln bricht – auch Anfragen von dir selbst.`,
            `A vault is a small safe on the Kaspa chain that belongs only to you. You deposit ${NATIVE} and may create (“mint”) ${STABLE_SYMBOL} against it – a stablecoin meant to be worth 1 US dollar. The ${NATIVE} stay in the vault until you give the ${STABLE_SYMBOL} back (“repay”). A contract governs the vault: it checks whether there is enough collateral and rejects anything that breaks the rules – even requests from you yourself.`,
          )}
        </p>
        <p>
          {tr(
            `Jeder Vault hat einen eigenen Zweig zum Prägen von ${STABLE_SYMBOL}. Deshalb gibt es keinen zentralen Topf, um den sich alle drängeln, und niemand sonst kann an deinen ${NATIVE} vorbei ${STABLE_SYMBOL} erzeugen.`,
            `Each vault has its own branch for minting ${STABLE_SYMBOL}. So there is no central pool everyone competes for, and no one else can create ${STABLE_SYMBOL} bypassing your ${NATIVE}.`,
          )}
        </p>
      </section>

      <section id="ueberbesichert">
        <h2>{tr("Überbesicherung", "Overcollateralization")}</h2>
        <p>
          {tr(
            `${NATIVE} schwankt stark im Preis. Damit ${STABLE_SYMBOL} gedeckt bleibt, verlangt der Vertrag beim Prägen deutlich mehr Wert, als geprägt wird. Eine Garantie ist das nicht:`,
            `${NATIVE}'s price swings widely. To keep ${STABLE_SYMBOL} backed, the contract requires significantly more value at minting time than is minted. This is not a guarantee:`,
          )}
        </p>
        <ul>
          <li>
            <strong>{tr(`Mindestquote ${fmtBps(PARAMS.mcrBps, 0)}:`, `Minimum ratio ${fmtBps(PARAMS.mcrBps, 0)}:`)}</strong>{" "}
            {tr(
              "gilt nur im Moment von Prägen und Abheben. Danach kann die Quote mit dem Kurs beliebig fallen, und offener Zins senkt sie langsam weiter. Für beide Quoten zählen Schuld und Zins zusammen.",
              "only applies at the moment of minting and withdrawing. After that, the ratio can fall freely with the price, and open interest slowly lowers it further. Both ratios count debt and interest together.",
            )}
          </li>
          <li>
            <strong>{tr(`Liquidationsschwelle ${fmtBps(PARAMS.liqBps, 0)}:`, `Liquidation threshold ${fmtBps(PARAMS.liqBps, 0)}:`)}</strong>{" "}
            {tr("Darunter ", "Below that, anyone ")}
            <em>{tr("darf", "may")}</em>
            {tr(
              ` jeder den Vault ablösen. Ob es jemand tut, ist nicht sicher. Ein Keeper-Agent ist gebaut (siehe unten), aber ob einer läuft, ist nicht sicher, und der Liquidator braucht eigene ${STABLE_SYMBOL}. Er darf auch nur einen Teil liquidieren. Fällt die Deckung unter etwa 110 %, bekommt er die ganze Sicherheit, der Vault endet, und die Restschuld wird ausgebucht. Diese ${STABLE_SYMBOL} sind dann nicht mehr gedeckt.`,
              ` take over the vault. Whether anyone does is not guaranteed. A keeper agent has been built (see below), but whether one is running is not guaranteed, and the liquidator needs its own ${STABLE_SYMBOL}. It may also liquidate only part of it. If the collateral ratio falls below about 110 %, it gets the entire collateral, the vault ends, and the remaining debt is written off. These ${STABLE_SYMBOL} are then no longer backed.`,
            )}
          </li>
          <li>
            <strong>{tr("Gesundheitsfaktor:", "Health factor:")}</strong>{" "}
            {tr(
              "Quote geteilt durch 150 %. Über 1 ist alles in Ordnung, unter 1 droht die Liquidation.",
              "Ratio divided by 150 %. Above 1 everything is fine, below 1 liquidation looms.",
            )}
          </li>
        </ul>
      </section>

      <section id="zins">
        <h2>{tr("Zins und Kurs", "Interest and price")}</h2>
        <p>
          {tr(
            `Auf geprägte ${STABLE_SYMBOL} fällt ein Zins an. Er wird getrennt von der Schuld in USD verbucht: Deine Schuld bleibt genau die Menge ${STABLE_SYMBOL}, die du geprägt und noch nicht getilgt hast, der Zins wächst mit der Zeit dazu. Offener Zins verzinst sich mit, solange er nicht bezahlt ist. So hängt der Betrag nicht davon ab, wie oft abgerechnet wird.`,
            `Interest accrues on minted ${STABLE_SYMBOL}. It is booked separately from the debt, in USD: your debt stays exactly the amount of ${STABLE_SYMBOL} you minted and have not yet repaid, and the interest grows on top over time. Open interest also bears interest as long as it is unpaid, so the amount does not depend on how often it is settled.`,
          )}
        </p>
        <ul>
          <li>
            <strong>{tr("Der Satz passt sich selbst an:", "The rate adjusts itself:")}</strong>{" "}
            {tr(
              `Der GHOST-Agent misst in jeder Runde, was ${STABLE_SYMBOL} am Markt kostet (Kurs im Tauschpool mal ${NATIVE}-Preis), und entscheidet nach dem Median der Messungen der letzten Stunde. Liegt er unter 0,995 USD, steigt der Zins um 0,5 Prozentpunkte: Schulden werden teurer, Schuldner kaufen ${STABLE_SYMBOL} und tilgen, das hebt den Kurs. Liegt er über 1,005 USD, sinkt der Zins um 0,5 Prozentpunkte: Prägen lohnt sich, mehr ${STABLE_SYMBOL} kommen in Umlauf. Der Satz ändert sich höchstens einmal pro Stunde, auch wenn der Agent neu startet, und bleibt zwischen 2 % (Grundzins) und 20 % pro Jahr. Liegt er unter 2 %, hebt der Agent ihn stündlich um 0,5 Punkte bis auf 2 % an. Nach unten ändert er sich nur, wenn mindestens 6 Messungen vorliegen und der Pool mindestens 10 ${STABLE_SYMBOL} hält; ein einzelner Tausch bewegt ihn also nicht.`,
              `In every round the GHOST agent measures what ${STABLE_SYMBOL} costs on the market (pool price times the ${NATIVE} price) and decides by the median of the last hour's measurements. If it is below 0.995 USD, the rate rises by 0.5 percentage points: debt gets more expensive, debtors buy ${STABLE_SYMBOL} and repay, which lifts the price. If it is above 1.005 USD, the rate falls by 0.5 percentage points: minting pays off and more ${STABLE_SYMBOL} enter circulation. The rate changes at most once per hour, even across agent restarts, and stays between 2 % (base rate) and 20 % per year. If it is below 2 %, the agent raises it by 0.5 points per hour up to 2 %. Downward it only changes when there are at least 6 measurements and the pool holds at least 10 ${STABLE_SYMBOL}, so a single swap does not move it.`,
            )}
          </li>
          <li>
            <strong>{tr("Der Zins zählt für die Quote:", "Interest counts toward the ratio:")}</strong>{" "}
            {tr(
              `Für die Mindestquote (${fmtBps(PARAMS.mcrBps, 0)}) und die Liquidation (unter ${fmtBps(PARAMS.liqBps, 0)}) zählen Schuld und Zins zusammen.`,
              `For the minimum ratio (${fmtBps(PARAMS.mcrBps, 0)}) and liquidation (below ${fmtBps(PARAMS.liqBps, 0)}) debt and interest count together.`,
            )}
          </li>
          <li>
            <strong>{tr("Bezahlt wird beim Schließen:", "You pay when closing:")}</strong>{" "}
            {tr(
              `Tilgen verbrennt ${STABLE_SYMBOL} und senkt nur die Schuld; der Zins bleibt stehen. Schließt du den schuldenfreien Vault, geht der Zins in ${NATIVE} zum Orakelpreis an die Zinsadresse, der Rest an dich. Unter 0,2 ${NATIVE} wird er erlassen. Zehrt der Zins die ganze Sicherheit auf (das kann nach einer Liquidation übrig bleiben), darf jeder den Vault auflösen: Die Sicherheit geht bis auf 0,01 ${NATIVE} an die Zinsadresse.`,
              `Repaying burns ${STABLE_SYMBOL} and only lowers the debt; the interest stays. When you close the debt-free vault, the interest goes to the interest address in ${NATIVE} at the oracle price, the rest to you. Below 0.2 ${NATIVE} it is waived. If the interest eats up all the collateral (this can remain after a liquidation), anyone may dissolve the vault: the collateral goes to the interest address except for 0.01 ${NATIVE}.`,
            )}
          </li>
          <li>
            <strong>{tr("Rücknahme zu 1 USD:", "Redemption at 1 USD:")}</strong>{" "}
            {tr(
              `Jeder kann ${STABLE_SYMBOL} an einem Vault mit mindestens ${fmtBps(PARAMS.liqBps, 0)} zurückgeben und bekommt dafür ${NATIVE} im Wert von 1 USD je ${STABLE_SYMBOL}, abzüglich 1 %. Die ${STABLE_SYMBOL} werden verbrannt, die Schuld des Vaults sinkt um denselben Betrag, und das 1 % bleibt beim Vault-Besitzer. Zurückgegeben wird mindestens 1 ${STABLE_SYMBOL} oder die ganze Schuld des Vaults. Fällt ${STABLE_SYMBOL} deutlich unter 1 USD, lohnt es sich, günstig zu kaufen und zurückzugeben. Das setzt eine Untergrenze für den Kurs.`,
              `Anyone can return ${STABLE_SYMBOL} to a vault at ${fmtBps(PARAMS.liqBps, 0)} or more and receives ${NATIVE} worth 1 USD per ${STABLE_SYMBOL}, minus 1 %. The ${STABLE_SYMBOL} are burned, the vault's debt drops by the same amount, and the 1 % stays with the vault owner. At least 1 ${STABLE_SYMBOL} or the vault's whole debt is returned. If ${STABLE_SYMBOL} falls well below 1 USD, it pays to buy cheaply and redeem. That sets a floor for the price.`,
            )}
          </li>
          <li>
            <strong>{tr("Kursband im Pool:", "Price band in the pool:")}</strong>{" "}
            {tr(
              `Der Tauschpool (Reiter „Tauschen“) lässt nur Tausche zu, solange sein Kurs bei 1 USD ± 3 % bleibt, gemessen am Orakelpreis.`,
              `The swap pool (“Swap” tab) only allows swaps while its price stays within 1 USD ± 3 %, measured against the oracle price.`,
            )}
          </li>
          <li>
            <strong>{tr("Obergrenze:", "Limit:")}</strong>{" "}
            {tr(
              `Je Vault lassen sich höchstens 50 ${STABLE_SYMBOL} prägen; der Zins zählt dabei nicht mit. Die Zahl der Vaults ist nicht begrenzt.`,
              `Each vault can mint at most 50 ${STABLE_SYMBOL}; interest does not count here. The number of vaults is not limited.`,
            )}
          </li>
        </ul>
        <p>
          {tr(
            "Den Satz setzt der Agent über das Orakel, mit den Unterschriften der Unterzeichner. Der Vertrag lässt höchstens 20 % pro Jahr zu und ändert den Satz höchstens um 0,5 Prozentpunkte je Stunde. Wie viel Zins aufläuft, rechnet der Vertrag selbst aus der vergangenen Zeit. Den Zins eines Vaults rundet er zugunsten der Zinsadresse auf. Den Zinsindex des Orakels schreibt er dagegen bei jedem Update abgerundet fort, zugunsten der Schuldner. Bei stündlichen Updates fehlen so etwa 0,1 % des Zinses, bei Updates im Minutentakt und sehr niedrigem Satz bis zu einige Prozent.",
            "The agent sets the rate through the oracle, with the signers' signatures. The contract allows at most 20 % per year and changes the rate by at most 0.5 percentage points per hour. How much interest accrues is calculated by the contract itself from the elapsed time. It rounds a vault's interest up, in favor of the interest address. The oracle's interest index, however, is rounded down at every update, in favor of borrowers. With hourly updates about 0.1 % of the interest is lost this way, with minute-by-minute updates and a very low rate up to a few percent.",
          )}
        </p>
        <p className="muted small">
          {tr(
            "Zum Start ist der Betreiber der einzige Unterzeichner und setzt Orakelpreis und Zins allein. Der Zins geht beim Schließen in KAS direkt an seine Adresse.",
            "At launch the operator is the only signer and sets oracle price and interest alone. On closing, the interest goes in KAS directly to the operator's address.",
          )}
        </p>
        {live && (
          <dl className="kv small">
            <div>
              <dt>{STABLE_SYMBOL}-Covenant</dt>
              <dd>
                <code title={live.ghostCovenantId}>{shortHex(live.ghostCovenantId)}</code>
              </dd>
            </div>
            <div>
              <dt>{tr("Orakel-Covenant", "Oracle covenant")}</dt>
              <dd>
                <code title={live.oracle.covenantId}>{shortHex(live.oracle.covenantId)}</code>
              </dd>
            </div>
            <div>
              <dt>Factory-Covenant</dt>
              <dd>
                <code title={live.factoryCovenantId}>{shortHex(live.factoryCovenantId)}</code>
              </dd>
            </div>
          </dl>
        )}
      </section>

      <section id="liquidation">
        <h2>{tr("Liquidation – ein Beispiel", "Liquidation – an example")}</h2>
        <ol className="example">
          <li>
            {tr("Du legst ", "You deposit ")}<strong>{fmtKas(EX_COLL, 0)}</strong>
            {tr(" in den Vault. Bei ", " in the vault. At ")}{fmtUsdPrice(EX_P0)}{tr(" je ", " per ")}{NATIVE}{tr(" sind das ", " that is ")}
            <strong>{usd(ex.value0)}</strong>.
          </li>
          <li>
            {tr("Du prägst ", "You mint ")}<strong>{fmtStable(EX_MINT, 0)}</strong>
            {tr(". Die Quote liegt bei ", ". The ratio is ")}{fmtBps(ex.ratio0)}
            {tr(" – über der Mindestquote, also erlaubt.", " – above the minimum ratio, so it's allowed.")}
          </li>
          <li>
            {tr("Der Liquidationspreis liegt bei ", "The liquidation price is ")}<strong>{fmtUsdPrice(ex.liqPrice)}</strong>
            {tr(`. Darunter sind die ${NATIVE} weniger als 150 % der Schuld wert.`, `. Below that, the ${NATIVE} are worth less than 150 % of the debt.`)}
          </li>
          <li>
            {tr(
              `Der Kurs fällt auf ${fmtUsdPrice(EX_P1)}. Der Vault ist nur noch ${usd(ex.value1)} wert, die Quote sinkt auf ${fmtBps(ex.ratio1)}.`,
              `The price falls to ${fmtUsdPrice(EX_P1)}. The vault is now worth only ${usd(ex.value1)}, the ratio drops to ${fmtBps(ex.ratio1)}.`,
            )}
          </li>
          <li>
            {tr(`Jemand liquidiert: Er verbrennt ${fmtStable(EX_MINT, 0)} und erhält dafür `, `Someone liquidates: they burn ${fmtStable(EX_MINT, 0)} and receive `)}
            <strong>{fmtKas(exSeized, 0)}</strong>
            {tr(" – den Gegenwert der Schuld plus 10 % Bonus.", " in return – the value of the debt plus a 10 % bonus.")}
          </li>
          <li>
            {tr(`Dir bleiben die geprägten ${fmtStable(EX_MINT, 0)} und `, `You keep the minted ${fmtStable(EX_MINT, 0)} and `)}
            <strong>{fmtKas(exRest, 0)}</strong>
            {tr(
              ` im Vault, jetzt schuldenfrei. Dein Verlust gegenüber dem Stand vor der Liquidation ist der Bonus: ${usd(exBonus)}.`,
              ` in the vault, now debt-free. Your loss compared to the state before liquidation is the bonus: ${usd(exBonus)}.`,
            )}
          </li>
        </ol>
        <p className="muted small">
          {tr(
            `Liegt nach der Liquidation weniger als 0,2 ${NATIVE} im Vault, bekommt der Liquidator auch diesen Rest und der Vault endet. Einen eigenen Ausgang für so kleine Beträge lässt Kaspa nicht sinnvoll zu.`,
            `If less than 0.2 ${NATIVE} remains in the vault after liquidation, the liquidator also gets this remainder and the vault ends. Kaspa does not sensibly allow a separate output for such small amounts.`,
          )}
        </p>
        <h3>{tr("Teil-Liquidation und Ausbuchen (seit Version 2)", "Partial liquidation and write-off (since version 2)")}</h3>
        <p>
          {tr(
            `Ein Liquidator muss nicht die ganze Schuld aufbringen. Er kann auch einen Teil verbrennen und bekommt dafür ${NATIVE} im Wert dieses Teils plus 10 %. Der Vault bleibt mit der Restschuld bestehen.`,
            `A liquidator does not have to cover the entire debt. They can also burn just part of it and receive ${NATIVE} worth that part plus 10 %. The vault continues to exist with the remaining debt.`,
          )}
        </p>
        <p>
          {tr(
            `Fällt der Kurs im Beispiel stattdessen auf ${fmtUsdPrice(EX_P2)}, ist die Sicherheit nur noch ${usd(exValue2)} wert. Das reicht nicht für ${fmtStable(EX_MINT, 0)} plus Bonus. Der Liquidator verbrennt dann etwa ${fmtStable(exBurn2, 2)} und bekommt die `,
            `If instead the price in the example falls to ${fmtUsdPrice(EX_P2)}, the collateral is worth only ${usd(exValue2)}. That's not enough for ${fmtStable(EX_MINT, 0)} plus bonus. The liquidator then burns about ${fmtStable(exBurn2, 2)} and receives the `,
          )}
          <strong>{tr("ganze Sicherheit", "entire collateral")}</strong>
          {tr(". Der Vault endet, und ", ". The vault ends, and ")}
          <strong>{fmtStable(exLiq2.ok ? (exLiq2.writtenOff ?? 0n) : 0n, 2)}</strong>{" "}
          {tr(
            `Restschuld werden ausgebucht. Diese Schuld ist uneinbringlich: Die ${STABLE_SYMBOL} im Umlauf sind dann nicht mehr voll gedeckt.`,
            `in remaining debt is written off. This debt is uncollectible: the ${STABLE_SYMBOL} in circulation are then no longer fully backed.`,
          )}
        </p>
      </section>

      <section id="orakel">
        <h2>{tr("Das Orakel", "The oracle")}</h2>
        <p>
          {tr(`Die Kette kennt den Kurs von ${NATIVE} nicht. Der Vertrag nimmt einen neuen Preis nur an, wenn `, `The chain does not know the price of ${NATIVE}. The contract only accepts a new price if `)}
          <strong>{tr("eine echte Mehrheit der Unterzeichner", "a true majority of the signers")}</strong>
          {tr(
            " ihn unterschrieben hat. Das prüft ein eigener Vertrag, das Unterzeichner-Register. Es speichert nur den Hash des Unterzeichner-Satzes: n Schlüssel (höchstens 9), Preis-Schwelle t mit 2t > n und eine Austausch-Schwelle.",
            " have signed it. This is checked by a separate contract, the signer register. It only stores the hash of the signer set: n keys (at most 9), price threshold t with 2t > n, and a rotation threshold.",
          )}
        </p>
        <div className="callout callout-warn">
          <strong className="callout-title">{tr("Im jetzigen Mainnet-Probelauf", "In the current mainnet trial")}</strong>
          <div>
            {tr(
              "Der Start läuft im Alleinbetrieb: 1 Unterzeichner, der Betreiber, 1 Signatur. Wer diesen Schlüssel hält, kann jeden beliebigen Preis setzen. Damit kann er jeden Vault liquidierbar machen oder ungedeckte Prägung ermöglichen. Der Vertrag begrenzt Sprünge nur auf ×2 bzw. ÷2 je Update bei mindestens etwa 1 Minute Abstand; in wenigen Minuten ist damit jeder Preis zwischen 0,00001 und 900 USD erreichbar. Weitere unabhängige Unterzeichner lassen sich später ohne neues Deployment einsetzen, heute gibt es sie nicht.",
              "Launch runs in solo operation: 1 signer, the operator, 1 signature. Whoever holds this key can set any price. This lets them make any vault liquidatable or allow uncovered minting. The contract only limits jumps to ×2 or ÷2 per update with at least about 1 minute between updates; within a few minutes any price between 0.00001 and 900 USD can be reached. Further independent signers can be added later without a new deployment; today there are none.",
            )}
          </div>
        </div>
        <p>
          <strong>{tr("Warum nur der neueste Stand zählt:", "Why only the latest state counts:")}</strong>{" "}
          {tr(
            "Ein Vertrag auf Kaspa kann nicht prüfen, wie alt ein unterschriebener Preis ist. Deshalb liegt das Orakel als einzelner Eintrag auf der Kette, der bei jeder Nutzung verbraucht und unverändert neu angelegt wird. Ein älterer Preis ist damit schon „ausgegeben“ und kann nicht mehr verwendet werden – niemand kann sich einen günstigeren alten Kurs heraussuchen.",
            "A contract on Kaspa cannot check how old a signed price is. That's why the oracle exists as a single entry on the chain, which is spent and recreated unchanged with every use. An older price is thereby already “spent” and can no longer be used – no one can pick out a cheaper old price.",
          )}
        </p>
        <p>
          {tr(
            "Kommt 2 Stunden lang kein Preis, darf jeder das Orakel einfrieren; der GHOST-Agent tut das automatisch. Eingefroren sind Prägen, Rücknahme, Liquidieren, Auflösen, Abheben bei offener Schuld und der Tausch im Pool gesperrt. Einzahlen, Tilgen, Schließen, Abheben ohne Schuld sowie Liquidität einlegen und abziehen gehen weiter. Das nächste gültige Preis-Update taut alles wieder auf.",
            "If no price arrives for 2 hours, anyone may freeze the oracle; the GHOST agent does this automatically. While frozen, minting, redemption, liquidation, dissolving, withdrawing with open debt and swapping in the pool are blocked. Depositing, repaying, closing, withdrawing without debt and adding or removing liquidity still work. The next valid price update unfreezes everything.",
          )}
        </p>
        <p>
          <strong>{tr("Austausch der Unterzeichner:", "Replacing the signers:")}</strong>{" "}
          {tr(
            "Ein neuer Satz wird öffentlich angekündigt (die Ankündigung steht in der Transaktion) und erst nach 14 Tagen Wartezeit aktiviert; aktivieren darf dann jeder. Die Wartezeit erzwingt der Konsens über eine relative Sperre. In dieser Zeit kann der aktuelle Satz die Ankündigung absagen, und wer dem neuen Satz nicht traut, kann seinen Vault schließen oder GHOST einlösen. Ein optionaler Notfallsatz darf erst nach 30 Tagen ohne Preis-Update einen Austausch ankündigen, danach gelten wieder 14 Tage; jedes Preis-Update macht diese Ankündigung ungültig. Zum Start gibt es keinen Notfallsatz.",
            "A new set is announced publicly (the announcement is in the transaction) and only activated after a 14-day waiting period; anyone may then activate it. The waiting period is enforced by consensus via a relative lock. During it the current set can cancel the announcement, and anyone who does not trust the new set can close their vault or redeem GHOST. An optional emergency set may only announce a replacement after 30 days without a price update, followed again by 14 days; every price update invalidates that announcement. At launch there is no emergency set.",
          )}
        </p>
      </section>

      <section id="keeper">
        <h2>
          {tr("Keeper und KI-Agenten ", "Keepers and AI agents ")}<span className="tag">{tr("erster Teil gebaut", "first part built")}</span>
        </h2>
        <p>
          <strong>{tr("Gebaut ist der GHOST-Agent", "The GHOST agent is built")}</strong>
          {" ("}„GHOST-Agent starten.command“, <code>ghostctl agent</code>
          {tr(
            "). Alle 5 Minuten prüft er zwei Dinge. Beim Betreiber aktualisiert er das Orakel, wenn der Preis sich um 0,5 % bewegt hat oder das letzte Update 60 Minuten alt ist (die halbe Einfrier-Frist), und passt höchstens stündlich den Zins an den GHOST-Kurs an. Und er löst Vaults unter 150 % ab. Als Wächter liquidiert er nur, wenn auch der aktuelle Marktpreis die Unterdeckung zeigt; ein veralteter Orakelpreis allein reicht ihm nicht. Er verbrennt nie mehr GHOST, als die Sicherheit samt Bonus deckt, und nur eigene. Beim Betreiber löst er außerdem schuldenfreie Vaults auf, deren Zins die Sicherheit aufzehrt, zugunsten der Zinsadresse. Kommt 2 Stunden lang kein Preis, friert er das Orakel ein. Jeder kann ihn ohne Unterzeichner-Schlüssel als Liquidator betreiben.",
            "). Every 5 minutes it checks two things. At the operator it updates the oracle when the price has moved by 0.5 % or the last update is 60 minutes old (half the freeze deadline), and adjusts the interest rate to the GHOST price at most hourly. And it liquidates vaults below 150 %. As a watchdog it only liquidates when the current market price also confirms the shortfall; a stale oracle price alone is not enough for it. It never burns more GHOST than the collateral plus bonus covers, and only its own. At the operator it also dissolves debt-free vaults whose interest eats up the collateral, in favor of the interest address. If no price arrives for 2 hours, it freezes the oracle. Anyone can run it as a liquidator without a signer key.",
          )}
        </p>
        <p>
          {tr(
            "Noch nicht gebaut: unterschriebene Aufträge (Aktion, Betrag, Empfänger, Ablaufzeit), die Keeper-Programme oder KI-Agenten gebündelt einreichen und deren Ergebnis der Vertrag erzwingt, sowie eine Schnittstelle (MCP), über die eigene Agenten den Vault verwalten. Als Preisquelle ist KI nicht vorgesehen.",
            "Not built yet: signed orders (action, amount, recipient, expiry) that keeper programs or AI agents submit in batches and whose outcome the contract enforces, and an interface (MCP) through which your own agents manage the vault. AI is not intended as a price source.",
          )}
        </p>
      </section>

      <section id="risiken">
        <h2>{tr("Risiken", "Risks")}</h2>
        <ul className="risks">
          <li>
            <strong>{tr("Kursrisiko:", "Price risk:")}</strong>{" "}
            {tr(
              `${NATIVE} kann in Stunden stark fallen. Dann droht die Liquidation, und du verlierst den Bonus – bei sehr schnellen Stürzen auch mehr. Eine rechtzeitige Liquidation ist nicht garantiert.`,
              `${NATIVE} can fall sharply within hours. Liquidation then looms, and you lose the bonus – in very fast crashes, possibly more. Timely liquidation is not guaranteed.`,
            )}
          </li>
          <li>
            <strong>{tr("Rücknahme an deinem Vault:", "Redemption at your vault:")}</strong>{" "}
            {tr(
              `Solange dein Vault über ${fmtBps(PARAMS.liqBps, 0)} liegt, kann jeder dort ${STABLE_SYMBOL} zurückgeben. Dann sinkt deine Schuld, und aus deinem Vault gehen ${NATIVE} im Gegenwert ab (abzüglich 1 %, das dir bleibt). Deine Position wird kleiner, ohne dass du etwas tust. Gerechnet wird zum Orakelpreis: Steigt ${NATIVE} sprunghaft um mehr als 20 %, übernimmt der Agent das erst nach drei bestätigenden Runden (etwa 10 bis 15 Minuten). In dieser Zeit bekommt ein Rücknehmer mehr ${NATIVE}, als der Markt hergibt, und die Differenz fehlt dir.`,
              `As long as your vault is above ${fmtBps(PARAMS.liqBps, 0)}, anyone can return ${STABLE_SYMBOL} there. Your debt then drops and ${NATIVE} of equal value leave your vault (minus 1 %, which you keep). Your position shrinks without you doing anything. The oracle price is used: if ${NATIVE} jumps by more than 20 %, the agent only adopts it after three confirming rounds (about 10 to 15 minutes). During that time a redeemer receives more ${NATIVE} than the market would give, and you lose the difference.`,
            )}
          </li>
          <li>
            <strong>{tr("Orakel- und Betreiber-Risiko:", "Oracle and operator risk:")}</strong>{" "}
            {tr(
              "Zum Start setzt der Betreiber Preis und Zins allein, denn er ist der einzige Unterzeichner. Ein falscher, manipulierter oder ausbleibender Preis kann Vaults liquidierbar machen oder ungedeckte Prägung erlauben.",
              "At launch, the operator sets the price and interest rate alone, since they are the only signer. A wrong, manipulated, or missing price can make vaults liquidatable or allow uncovered minting.",
            )}
          </li>
          <li>
            <strong>{tr("Vertragsrisiko:", "Contract risk:")}</strong>{" "}
            {tr("Die Verträge sind neu und ", "The contracts are new and ")}
            <strong>{tr("nicht professionell geprüft", "not professionally audited")}</strong>{" "}
            {tr(
              "(mehrere KI-Audits haben u. a. einen kritischen Fehler in Version 1 gefunden, behoben in Version 2 und 2.1 – siehe AUDIT.md). Getestet sind sie lokal (mit Mutationstest). Version 1 lief vollständig im Testnetz, Version 2.1 wird direkt im Mainnet mit Kleinstbeträgen erprobt. Fehler können zum Totalverlust führen.",
              "(several AI audits found, among other things, a critical bug in version 1, fixed in version 2 and 2.1 – see AUDIT.md). They are tested locally (with mutation testing). Version 1 ran fully on testnet; version 2.1 is being tried directly on mainnet with tiny amounts. Bugs can lead to total loss.",
            )}
          </li>
          <li>
            <strong>{tr("Peg-Risiko:", "Peg risk:")}</strong>{" "}
            {tr(
              `${STABLE_SYMBOL} kann vom Dollar abweichen. Zins und Rücknahme sollen den Kurs nahe 1 USD halten, garantieren es aber nicht: Die Rücknahme wirkt nur, solange Vaults über ${fmtBps(PARAMS.liqBps, 0)} genug ${NATIVE} halten, der Zins nur, wenn Schuldner darauf reagieren. Der Tauschpool lässt nur Tausche im Band 1 USD ± 3 % zu, außerhalb des Pools gibt es keine solche Grenze.`,
              `${STABLE_SYMBOL} can deviate from the dollar. Interest and redemption are meant to keep the price near 1 USD but do not guarantee it: redemption only works while vaults above ${fmtBps(PARAMS.liqBps, 0)} hold enough ${NATIVE}, interest only if debtors react to it. The swap pool only allows swaps within the band 1 USD ± 3 %; outside the pool there is no such limit.`,
            )}
          </li>
          <li>
            <strong>{tr("Rechtliches:", "Legal:")}</strong>{" "}
            {tr("Experimentelles Open-Source-Projekt, nicht professionell geprüft, keine Anlageberatung.", "Experimental open-source project, not professionally audited, not investment advice.")}
          </li>
          <li>
            <strong>{tr("Keine Anlageberatung:", "Not investment advice:")}</strong>{" "}
            {tr("Diese Seite erklärt eine Technik. Sie empfiehlt nichts.", "This page explains a technology. It does not recommend anything.")}
          </li>
        </ul>
      </section>
    </div>
  );
}

/** Sprung zu Abschnitten, ohne die Hash-Route zu verändern. */
function jump(e: MouseEvent<HTMLAnchorElement>) {
  const id = e.currentTarget.getAttribute("href")?.slice(1);
  const el = id ? document.getElementById(id) : null;
  if (!el) return;
  e.preventDefault();
  el.scrollIntoView({ behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  const h = el.querySelector("h2") as HTMLElement | null;
  if (h) {
    h.tabIndex = -1;
    h.focus({ preventScroll: true });
  }
}
