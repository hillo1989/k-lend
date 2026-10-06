// Platzhalter. Die Rechtstexte werden später geliefert – hier bewusst keine
// eigenen rechtlichen Aussagen erfinden.
import type { ReactNode } from "react";
import { tr } from "../lib/i18n";
import { href } from "../router";
import { IMPRESSUM } from "../config";

function Placeholder({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="container section prose">
      <h1 tabIndex={-1} data-route-heading>
        {title}
      </h1>
      <div className="placeholder" role="note">
        <strong>{tr("Inhalte folgen.", "Content coming soon.")}</strong>
        <p className="muted" style={{ margin: "8px 0 0" }}>
          {tr("Dieser Abschnitt ist ein Platzhalter. Der endgültige Text wird nachgereicht.", "This section is a placeholder. The final text will be added later.")}
        </p>
      </div>
      {children}
    </div>
  );
}

export function Rechtliches() {
  return (
    <Placeholder title={tr("Rechtliches & Risiken", "Legal & risks")}>
      <p style={{ marginTop: 24 }}>
        {tr("Experimentelles Open-Source-Projekt, nicht professionell geprüft, keine Anlageberatung.", "Experimental open-source project, not professionally audited, not investment advice.")}
      </p>
      <ul>
        <li>
          {tr(
            "Version 4 läuft im Mainnet. Die Verträge sind mit automatischen Tests, Mutationstests und KI-Prüfungen geprüft, aber nicht von einer professionellen Prüffirma. Der Vertrag begrenzt das Prägen auf 50 GHOST je Vault; Sicherheit, Sendungen und Pool-Einlagen sind nicht begrenzt. Setze nur Beträge ein, deren Verlust du verkraften kannst.",
            "Version 4 runs on mainnet. The contracts are checked with automated tests, mutation tests and AI reviews, but not by a professional audit firm. The contract limits minting to 50 GHOST per vault; collateral, transfers and pool deposits are not limited. Only use amounts you can afford to lose.",
          )}
        </li>
        <li>
          {tr(
            "Zum Start ist der Betreiber der einzige Unterzeichner des Orakels und setzt Preis und Zins allein. Kommt 2 Stunden lang kein Preis, kann das Orakel eingefroren werden; dann sind u. a. Prägen, Rücknahme und Liquidieren gesperrt.",
            "At launch the operator is the oracle's only signer and sets price and interest alone. If no price arrives for 2 hours, the oracle can be frozen; minting, redemption and liquidation, among others, are then blocked.",
          )}
        </li>
        <li>{tr("Eine Liquidation ist erlaubt, aber nicht garantiert. Kryptowerte können vollständig verloren gehen.", "A liquidation is permitted but not guaranteed. Crypto assets can be lost entirely.")}</li>
      </ul>
      <p>
        {tr("Mehr zu den technischen Risiken unter ", "More on the technical risks at ")}
        <a href={href("so-funktioniert-es")}>{tr("So funktioniert es", "How it works")}</a>.
      </p>
    </Placeholder>
  );
}

export function Impressum() {
  const i = IMPRESSUM;
  const filled = i.name && i.strasse && i.plzOrt && i.email;
  if (!filled) return <Placeholder title={tr("Impressum", "Imprint")} />;
  return (
    <div className="container section prose">
      <h1 tabIndex={-1} data-route-heading>
        {tr("Impressum", "Imprint")}
      </h1>
      <p>{tr("Angaben gemäß § 5 DDG", "Information pursuant to § 5 DDG (German Digital Services Act)")}</p>
      <p>
        {i.name}
        <br />
        {i.strasse}
        <br />
        {i.plzOrt}
      </p>
      <p>
        {tr("E-Mail", "Email")}: <a href={`mailto:${i.email}`}>{i.email}</a>
      </p>
    </div>
  );
}

export function Datenschutz() {
  return <Placeholder title={tr("Datenschutz", "Privacy")} />;
}
