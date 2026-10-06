// Anzeige, wenn keine Wallet im Browser steckt. Am Rechner: Links zu den
// Erweiterungen. Am Handy gibt es keine Erweiterungen; dort läuft die Seite nur
// im Browser der Wallet-App. Einen Link, der die App öffnet und die Seite darin
// lädt, dokumentieren weder Kastle noch KasWare (recherchiert 06.10.2026),
// daher Anleitung plus „Link kopieren“.
import { useState } from "react";
import { WALLET_LINKS } from "../config";
import { tr } from "../lib/i18n";

export function NoWallet({ small = false }: { small?: boolean }) {
  const [copied, setCopied] = useState(false);
  // jeder Browser am Handy, auch Chrome auf dem iPhone
  const ua = navigator.userAgent;
  const platform = /iphone|ipad|ipod/i.test(ua) ? "ios" : /android/i.test(ua) ? "android" : null;
  const cls = small ? "small" : undefined;
  const url = "https://k-lend.com";

  if (platform !== null) {
    const copy = async () => {
      try {
        await navigator.clipboard.writeText(url);
        setCopied(true);
      } catch {
        setCopied(false);
      }
    };
    return (
      <>
        <p className={cls}>
          {tr(
            "Auf dem Handy kann die Seite die Wallet-App nicht von außen ansprechen. So geht es:",
            "On a phone the page cannot reach the wallet app from outside. Here is how:",
          )}
        </p>
        <ol className={cls}>
          <li>{tr("Link kopieren (Knopf unten).", "Copy the link (button below).")}</li>
          <li>
            {platform === "ios"
              ? tr("Kastle-App öffnen, unten auf „Explore“ tippen.", "Open the Kastle app and tap “Explore” at the bottom.")
              : tr("Kastle-App öffnen und auf „Explore“ tippen – oder die KasWare-App öffnen und ihren Browser starten.", "Open the Kastle app and tap “Explore” – or open the KasWare app and start its browser.")}
          </li>
          <li>{tr("Dort den Link einfügen. K.Lend läuft dann in der Wallet-App und kann sich verbinden.", "Paste the link there. K.Lend then runs inside the wallet app and can connect.")}</li>
        </ol>
        <div className="btn-row">
          <button type="button" className="btn btn-primary btn-sm" onClick={() => void copy()}>
            {copied ? tr("Kopiert ✓", "Copied ✓") : tr("Link kopieren", "Copy link")}
          </button>
        </div>
        <p className="muted small">
          {tr("Noch keine Wallet-App? ", "No wallet app yet? ")}
          <a href={WALLET_LINKS.kastle} target="_blank" rel="noopener noreferrer">
            Kastle
          </a>
          {platform === "android" && (
            <>
              {" · "}
              <a href={WALLET_LINKS.kasware} target="_blank" rel="noopener noreferrer">
                KasWare
              </a>
            </>
          )}
        </p>
      </>
    );
  }

  return (
    <>
      <p className={cls}>
        {tr(
          "Keine Kaspa-Wallet im Browser gefunden. Du brauchst eine Browser-Erweiterung, zum Beispiel:",
          "No Kaspa wallet found in the browser. You need a browser extension, for example:",
        )}
      </p>
      <ul className={cls}>
        <li>
          <a href={WALLET_LINKS.kasware} target="_blank" rel="noopener noreferrer">
            KasWare Wallet
          </a>
        </li>
        <li>
          <a href={WALLET_LINKS.kastle} target="_blank" rel="noopener noreferrer">
            Kastle Wallet
          </a>
        </li>
      </ul>
      <p className="muted small">{tr("Nach der Installation die Seite neu laden.", "Reload the page after installing.")}</p>
    </>
  );
}
