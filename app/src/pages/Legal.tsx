// Platzhalter. Die Rechtstexte werden später geliefert – hier bewusst keine
// eigenen rechtlichen Aussagen erfinden.
import type { ReactNode } from "react";
import { getLang, tr } from "../lib/i18n";
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

/**
 * Datenschutzerklärung. Grundlage: docs/datenschutz-fakten.md (aus dem Code
 * belegt) und Prüfung am Server 06.10.2026 (k-lend.com ohne Zugriffsprotokoll).
 * ENTWURF – vor der Veröffentlichung durch die Kanzlei prüfen lassen.
 */
export function Datenschutz() {
  const i = IMPRESSUM;
  return (
    <div className="container section prose">
      <h1 tabIndex={-1} data-route-heading>
        {tr("Datenschutzerklärung", "Privacy policy")}
      </h1>
      {getLang() === "en" && <p className="muted">This privacy policy is available in German only; the German version applies.</p>}
      <p className="muted small">Stand: 6. Oktober 2026</p>

      <h2>1. Verantwortlicher</h2>
      <p>
        {i.name}
        <br />
        {i.strasse}
        <br />
        {i.plzOrt}
        <br />
        E-Mail: <a href={`mailto:${i.email}`}>{i.email}</a>
      </p>

      <h2>2. Kurz zusammengefasst</h2>
      <ul>
        <li>Keine Benutzerkonten, keine Cookies, keine Analyse- oder Tracking-Werkzeuge, keine Werbung, keine eingebundenen Inhalte Dritter.</li>
        <li>Deine privaten Schlüssel bleiben in deiner Wallet (KasWare oder Kastle). Wir erhalten sie nie.</li>
        <li>
          Alles, was du über K.Lend auf der Kaspa-Blockchain ausführst (Vaults, Zahlungen, Daueraufträge, öffentliche Nachrichten), ist öffentlich und dauerhaft auf der Blockchain gespeichert. Das können
          wir weder ändern noch löschen.
        </li>
      </ul>

      <h2>3. Hosting</h2>
      <p>
        Die Seite läuft auf einem Server der Hetzner Online GmbH, Industriestr. 25, 91710 Gunzenhausen, in Deutschland. Hetzner verarbeitet die Daten in unserem Auftrag (Vertrag zur
        Auftragsverarbeitung nach Art. 28 DSGVO). Die Verbindung ist per TLS verschlüsselt (Zertifikat von Let's Encrypt).
      </p>

      <h2>4. Aufruf der Seite und Server-Protokolle</h2>
      <p>
        Beim Aufruf überträgt dein Browser technisch notwendig deine IP-Adresse und übliche Angaben (z. B. aufgerufene Adresse, Browsertyp). Für k-lend.com führen wir kein Zugriffsprotokoll, IP-Adressen
        werden nicht auf Dauer gespeichert. Unsere Server-Programme schreiben nur Betriebs- und Fehlermeldungen in das Systemprotokoll, ohne IP-Adressen und ohne Inhalte deiner Anfragen; dieses Protokoll
        wird nach Größe automatisch überschrieben.
      </p>
      <p>
        Zum Schutz vor Missbrauch und Überlastung zählen wir bei Wallet-Funktionen, wie oft eine IP-Adresse (bei IPv6 gekürzt auf /64) in der letzten Minute angefragt hat. Diese Zählung liegt nur im
        Arbeitsspeicher, wird nach 60 Sekunden bedeutungslos und bei jedem Neustart gelöscht. Rechtsgrundlage ist unser berechtigtes Interesse an einem sicheren und verfügbaren Betrieb (Art. 6 Abs. 1 lit. f
        DSGVO).
      </p>

      <h2>5. Verbindung mit deiner Wallet und Aktionen</h2>
      <p>
        Verbindest du KasWare oder Kastle, liest die Seite deine Kaspa-Adresse, das Netz, dein Guthaben und deinen öffentlichen Schlüssel aus der Wallet. Für eine Aktion (z. B. Vault eröffnen, senden,
        tauschen) schickt die Seite deine Adresse und die eingegebenen Werte an unseren Server. Er baut daraus die Transaktion, die du in deiner Wallet prüfst und signierst. Die signierte Transaktion prüft
        der Server und sendet sie auf deinen Knopfdruck an das Kaspa-Netz. Pläne und Signaturen liegen dabei nur kurz in einem geschützten temporären Verzeichnis und werden nach der Anfrage gelöscht; von
        abgewiesenen Plänen merken wir uns 5 Minuten lang nur einen Prüfwert (Hash), um Wiederholungen abzuwehren. Rechtsgrundlage ist die Ausführung der von dir gewünschten Funktion (Art. 6 Abs. 1 lit. b
        DSGVO).
      </p>
      <p>
        Für Abfragen des Kontostands und zum Senden nutzt unser Server öffentliche Kaspa-Nodes und die öffentliche Schnittstelle api.kaspa.org. Dabei werden Kaspa-Adressen und Transaktionen übermittelt,
        die ohnehin öffentlich auf der Blockchain stehen. Der Zustand des Protokolls (Vaults und GHOST-Token mit dem öffentlichen Schlüssel ihrer Besitzer) liegt auf unserem Server und wird auf der Seite
        öffentlich angezeigt; dieselben Daten stehen öffentlich auf der Blockchain.
      </p>

      <h2>6. Daueraufträge (Tresore)</h2>
      <p>
        Legst du einen Tresor an, speichern wir zusätzlich auf unserem Server: deine Adresse, die Empfänger-Adresse, Betrag, Rhythmus und Termine, Anzahl der Zahlungen, Höchstgebühr, eine öffentliche
        Nachricht (falls angegeben) und den Zahlungsverlauf. Wir brauchen diese Angaben, damit unser Agent die Zahlungen zum Termin auslöst und dir „Meine Tresore“ anzeigen kann (Art. 6 Abs. 1 lit. b
        DSGVO). Wer eine Adresse kennt, kann deren Tresore auf der Seite abfragen; dieselben Angaben werden mit der ersten Zahlung ohnehin auf der Blockchain sichtbar. Gekündigte und beendete Tresore
        löschen wir spätestens, wenn der Platz gebraucht wird; die Einträge auf der Blockchain bleiben bestehen.
      </p>

      <h2>7. .k-Namen (dotk.name)</h2>
      <p>
        Gibst du im Empfängerfeld einen Namen wie „alice.k“ ein, fragt unser Server den Namensdienst api.dotk.name, welche Adresse dazugehört, und prüft die Antwort an einem Kaspa-Node. Übermittelt wird nur
        der Name, und zwar von unserem Server aus – deine IP-Adresse erhält dotk.name nicht. Wir speichern den Namen nicht.
      </p>

      <h2>8. Speicher in deinem Browser</h2>
      <p>Die Seite speichert einige Angaben nur in deinem Browser (localStorage); sie werden nicht an uns übertragen:</p>
      <ul>
        <li>den Verlauf deiner über diese Seite gesendeten Transaktionen (höchstens 200 Einträge, mit Betrag, Empfänger, Transaktions-ID und ggf. deiner Nachricht),</li>
        <li>welche Wallet du zuletzt verbunden hast, deine Sprache und das gewählte Netz,</li>
        <li>dass du den Hinweis „als App installieren“ geschlossen hast.</li>
      </ul>
      <p>
        Diese Speicherung ist für die von dir genutzten Funktionen erforderlich (§ 25 Abs. 2 Nr. 2 TDDDG). Du kannst sie jederzeit über die Einstellungen deines Browsers löschen („Websitedaten
        löschen“).
      </p>

      <h2>9. Kontakt per E-Mail</h2>
      <p>
        Schreibst du uns an {i.email}, verarbeiten wir deine Angaben, um deine Anfrage zu beantworten (Art. 6 Abs. 1 lit. b bzw. f DSGVO), und löschen sie, wenn sie dafür nicht mehr nötig sind und
        keine gesetzlichen Aufbewahrungspflichten bestehen.
      </p>

      <h2>10. Deine Rechte</h2>
      <p>
        Du hast das Recht auf Auskunft (Art. 15 DSGVO), Berichtigung (Art. 16), Löschung (Art. 17), Einschränkung der Verarbeitung (Art. 18), Datenübertragbarkeit (Art. 20) und Widerspruch gegen
        Verarbeitungen aufgrund berechtigter Interessen (Art. 21). Wende dich dafür an {i.email}. Daten auf der Kaspa-Blockchain können technisch von niemandem geändert oder gelöscht werden.
      </p>
      <p>
        Du kannst dich außerdem bei einer Datenschutz-Aufsichtsbehörde beschweren, zum Beispiel beim Hessischen Beauftragten für Datenschutz und Informationsfreiheit, Gustav-Stresemann-Ring 1, 65189
        Wiesbaden.
      </p>

      <h2>11. Keine automatisierten Entscheidungen</h2>
      <p>
        Wir treffen keine automatisierten Entscheidungen über dich im Sinne von Art. 22 DSGVO. Regeln wie Liquidation, Zins und Rücknahme ergeben sich aus den öffentlichen Verträgen auf der Blockchain und
        gelten für alle gleich.
      </p>
    </div>
  );
}
