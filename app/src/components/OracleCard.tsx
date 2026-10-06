import { ORACLE_STALE_MINUTES, STABLE_SYMBOL } from "../config";
import { useStatus } from "../lib/StatusContext";
import { de, oracleStale, shortHex } from "../lib/status";
import { href } from "../router";
import { tr } from "../lib/i18n";

const age = (min: number) => (min < 90 ? `${de(min, 0)} min` : `${de(min / 60, 1)} h`);

/**
 * Orakel-Stand. `compact`: nur Preis und Alter mit Link auf den Reiter
 * „Orakel“ (Vault, Märkte); sonst alle Werte (Reiter „Orakel“).
 */
export function OracleCard({ compact = false }: { compact?: boolean }) {
  const { status } = useStatus();
  const live = status?.deployed ? status : null;
  const stale = live ? oracleStale(live.oracle) : false;
  const frozen = !!live?.oracle.frozen;
  const sg = live?.signers;
  const rot = sg?.rotation?.valid ? sg.rotation : null;
  return (
    <section className="card" aria-labelledby="oracle-title">
      <div className="card-head">
        <h2 id="oracle-title">{compact ? tr("Orakelpreis", "Oracle price") : tr("Aktueller Stand", "Current state")}</h2>
        {live && (
          <span className={frozen || stale ? "tag tag-warn" : "tag"}>{frozen ? tr("eingefroren", "frozen") : stale ? tr("veraltet", "stale") : tr("frisch", "fresh")}</span>
        )}
      </div>
      {frozen && (
        <p className="warn-text small" role="alert">
          {tr(
            "Seit über der Frist kam kein Preis. Prägen, Einlösen, Liquidieren und Tauschen sind gesperrt, bis wieder ein Preis kommt. Einzahlen, Tilgen und Schließen gehen weiter.",
            "No price has arrived within the deadline. Minting, redeeming, liquidating and swapping are blocked until a price arrives again. Depositing, repaying and closing still work.",
          )}
        </p>
      )}
      {sg && (sg.foreignChange || sg.emergencyOpen || sg.unknownSet) && (
        <p className="warn-text small" role="alert">
          {sg.unknownSet
            ? tr(
                "Im Register steht ein Unterzeichner-Satz, den diese Seite nicht kennt (Austausch von einem anderen Rechner). Preis-Updates von hier aus sind gesperrt.",
                "The register holds a signer set this page does not know (changed on another computer). Price updates from here are blocked.",
              )
            : sg.emergencyOpen
              ? tr(
                  "Im Register ist ein Notfall-Austausch der Unterzeichner angekündigt. Jedes Preis-Update macht ihn ungültig.",
                  "An emergency change of signers is announced in the register. Any price update invalidates it.",
                )
              : tr(
                  "Im Register wurde von außen ein Austausch angekündigt oder abgesagt. Prüfen; eine fremde Ankündigung lässt sich in der Wartezeit absagen (ghostctl signers cancel).",
                  "A change of signers was announced or cancelled from outside. Check it; a foreign announcement can be cancelled during the waiting period (ghostctl signers cancel).",
                )}
        </p>
      )}
      {rot && (
        <p className="warn-text small" role="status">
          {tr(
            `Angekündigt: ${rot.emergency ? "Notfall-" : ""}Austausch der Unterzeichner auf ${rot.set.keys.length} Schlüssel (Schwelle ${rot.set.threshold}). Gültig frühestens in ${age(Math.max(0, rot.readyInHours) * 60)}; bis dahin kann der jetzige Satz absagen.`,
            `Announced: ${rot.emergency ? "emergency " : ""}change of signers to ${rot.set.keys.length} key(s) (threshold ${rot.set.threshold}). Takes effect in ${age(Math.max(0, rot.readyInHours) * 60)} at the earliest; until then the current set can cancel it.`,
          )}
        </p>
      )}
      {live ? (
        <dl className="kv">
          <div>
            <dt>{tr("KAS-Preis", "KAS price")}</dt>
            <dd>{de(live.oracle.kasUsd, 8, 2)} USD</dd>
          </div>
          <div>
            <dt>{tr("Alter", "Age")}</dt>
            <dd>{age(live.oracle.ageMinutes)}</dd>
          </div>
          {!compact && (
            <>
              <div>
                <dt>{tr("Frisch laut ghostctl", "Fresh according to ghostctl")}</dt>
                <dd>{live.oracle.fresh ? tr("ja", "yes") : `${tr("nein", "no")}${live.oracle.freshError ? ` – ${live.oracle.freshError}` : ""}`}</dd>
              </div>
              <div>
                <dt>{tr("Update Nr.", "Update no.")}</dt>
                <dd>{live.oracle.seq}</dd>
              </div>
              <div>
                <dt>{tr(`${STABLE_SYMBOL}-Zins`, `${STABLE_SYMBOL} interest`)}</dt>
                <dd>{de(live.oracle.ratePctYear, 2)} % {tr("p. a.", "p.a.")}</dd>
              </div>
              <div>
                <dt>{tr("Zinsindex", "Interest index")}</dt>
                <dd>{de(live.oracle.index, 9)}</dd>
              </div>
              <div>
                <dt>{tr("DAA-Score der Kette", "Chain DAA score")}</dt>
                <dd>{de(live.daa, 0)}</dd>
              </div>
              <div>
                <dt>{tr("Unterzeichner", "Signers")}</dt>
                <dd>
                  {sg
                    ? tr(`${sg.set.threshold} von ${sg.set.keys.length} Signaturen`, `${sg.set.threshold} of ${sg.set.keys.length} signatures`)
                    : tr("unbekannt", "unknown")}
                  {sg && (
                    <>
                      {" "}
                      {sg.set.keys.map((k) => (
                        <code key={k} title={k}>
                          {shortHex(k)}
                        </code>
                      ))}
                    </>
                  )}
                </dd>
              </div>
              {sg && (
                <>
                  <div>
                    <dt>{tr("Austausch der Unterzeichner", "Changing signers")}</dt>
                    <dd>{tr(`öffentlich angekündigt, gültig nach ${age(sg.rotateDelayHours * 60)}`, `announced publicly, effective after ${age(sg.rotateDelayHours * 60)}`)}</dd>
                  </div>
                  <div>
                    <dt>{tr("Notfallsatz", "Fallback set")}</dt>
                    <dd>
                      {sg.fallback
                        ? tr(`${sg.fallback.threshold} von ${sg.fallback.keys.length}, erst nach ${de(sg.emergencyAfterDays, 0)} Tagen ohne Preis`, `${sg.fallback.threshold} of ${sg.fallback.keys.length}, only after ${de(sg.emergencyAfterDays, 0)} days without a price`)
                        : tr("keiner", "none")}
                    </dd>
                  </div>
                  <div>
                    <dt>{tr("Einfrieren", "Freezing")}</dt>
                    <dd>
                      {frozen
                        ? tr("eingefroren", "frozen")
                        : (live.oracle.freezeInMinutes ?? 0) > 0
                          ? tr(`möglich in ${age(live.oracle.freezeInMinutes ?? 0)} ohne neuen Preis`, `possible in ${age(live.oracle.freezeInMinutes ?? 0)} without a new price`)
                          : tr("jetzt möglich (Frist abgelaufen)", "possible now (deadline passed)")}
                    </dd>
                  </div>
                  <div>
                    <dt>{tr("Register-Covenant", "Register covenant")}</dt>
                    <dd>
                      <code title={sg.registerCovenantId}>{shortHex(sg.registerCovenantId)}</code>
                    </dd>
                  </div>
                </>
              )}
              <div>
                <dt>{tr("Orakel-Covenant", "Oracle covenant")}</dt>
                <dd>
                  <code title={live.oracle.covenantId}>{shortHex(live.oracle.covenantId)}</code>
                </dd>
              </div>
            </>
          )}
        </dl>
      ) : (
        <p className="muted">{tr("Keine Live-Daten.", "No live data.")}</p>
      )}
      {compact ? (
        <p className="small">
          <a href={href("orakel")}>{tr("Alles zum Orakel →", "All about the oracle →")}</a>
        </p>
      ) : (
        <p className="small muted">{tr(`Als veraltet gilt ein Preis ab ${de(ORACLE_STALE_MINUTES / 60, 1)} h.`, `A price counts as stale after ${de(ORACLE_STALE_MINUTES / 60, 1)} h.`)}</p>
      )}
    </section>
  );
}
