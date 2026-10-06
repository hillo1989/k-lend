// Anzeige, wenn keine Wallet im Browser steckt. Am Rechner: Links zu den
// Erweiterungen. Am Handy gibt es keine Erweiterungen; dort läuft die Seite nur
// im Browser der Wallet-App. Einen Link, der die App öffnet und die Seite darin
// lädt, dokumentieren weder Kastle noch KasWare (recherchiert 06.10.2026).
// Kastles „Explore“ ist eine feste Liste geprüfter Apps (Screenshot des
// Nutzers 06.10.2026), kein freier Browser.
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
            "Auf dem Handy gibt es keine Browser-Erweiterungen; K.Lend muss im Browser einer Wallet-App laufen.",
            "Phones have no browser extensions; K.Lend has to run inside a wallet app's browser.",
          )}
        </p>
        <ul className={cls}>
          {platform === "android" && (
            <li>
              {tr(
                "KasWare (Android-App): im eingebauten Browser der App k-lend.com öffnen (Link unten kopieren).",
                "KasWare (Android app): open k-lend.com in the app's built-in browser (copy the link below).",
              )}
            </li>
          )}
          <li>
            {tr(
              "Kastle: zeigt unter „Explore“ nur geprüfte Apps. K.Lend ist dort noch nicht aufgenommen – bis dahin am Rechner mit der Browser-Erweiterung arbeiten.",
              "Kastle: “Explore” only lists verified apps. K.Lend is not listed there yet – until then, use the browser extension on a computer.",
            )}
          </li>
        </ul>
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
