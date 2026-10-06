import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
// Schrift lokal ausgeliefert (kein Abruf bei Google, keine Dritt-Anfragen)
import "@fontsource-variable/rubik";
import "./styles.css";
import { App } from "./App";
import { WalletProvider } from "./wallet/WalletContext";
import { StatusProvider } from "./lib/StatusContext";
import { AccountProvider } from "./lib/AccountContext";
import { LangProvider, useLang } from "./lib/i18n";

/** Beim Sprachwechsel die App neu aufbauen, damit alle Texte neu berechnet werden. */
function LangKeyed() {
  const { lang } = useLang();
  return <App key={lang} />;
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LangProvider>
      <StatusProvider>
        <AccountProvider>
          <WalletProvider>
            <LangKeyed />
          </WalletProvider>
        </AccountProvider>
      </StatusProvider>
    </LangProvider>
  </StrictMode>,
);

// Als App vom Startbildschirm (PWA): Service Worker ohne Zwischenspeicher
// (public/sw.js). Nur über https bzw. localhost, nicht im Dev-Server nötig.
if ("serviceWorker" in navigator && import.meta.env.PROD) {
  window.addEventListener("load", () => {
    navigator.serviceWorker.register("./sw.js").catch(() => {
      /* ohne Service Worker geht die Seite genauso */
    });
  });
}
