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
    // Stand 06.10.2026, mit dem Betreiber am Handy geprüft: Kastles „Explore“ ist
    // eine feste Liste geprüfter Apps, KasWare 0.5.3 (Android) hat keinen frei
    // nutzbaren Browser. Am Handy gibt es daher noch keinen Weg zur Wallet.
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
            "Am Handy lässt sich K.Lend noch nicht mit einer Wallet verbinden: Handy-Browser kennen keine Wallet-Erweiterungen, und die Wallet-Apps (Kastle, KasWare) öffnen K.Lend noch nicht in ihrem eigenen Browser.",
            "K.Lend cannot connect to a wallet on a phone yet: mobile browsers have no wallet extensions, and the wallet apps (Kastle, KasWare) do not open K.Lend in their own browser yet.",
          )}
        </p>
        <p className={cls}>
          {tr(
            "Ansehen geht hier – zum Handeln bitte am Rechner mit der Browser-Erweiterung von KasWare oder Kastle (Chrome, Brave oder Edge).",
            "You can look around here – to act, please use a computer with the KasWare or Kastle browser extension (Chrome, Brave or Edge).",
          )}
        </p>
        <div className="btn-row">
          <button type="button" className="btn btn-primary btn-sm" onClick={() => void copy()}>
            {copied ? tr("Kopiert ✓", "Copied ✓") : tr("Link für den Rechner kopieren", "Copy link for your computer")}
          </button>
        </div>
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
