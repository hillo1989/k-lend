import { useEffect, useRef } from "react";
import { Datenschutz, Impressum, Rechtliches } from "./pages/Legal";
import { PROTOCOL_NAME } from "./config";
import { Footer, Header } from "./components/Layout";
import { Faq } from "./pages/Faq";
import { HowItWorks } from "./pages/HowItWorks";
import { Landing } from "./pages/Landing";
import { Markets } from "./pages/Markets";
import { Pool } from "./pages/Pool";
import { Vault } from "./pages/Vault";
import { Wallet } from "./pages/Wallet";
import { Swap } from "./pages/Swap";
import { Oracle } from "./pages/Oracle";
import { Statistics } from "./pages/Statistics";
import { ROUTES, routeLabel, useRoute } from "./router";
import { tr } from "./lib/i18n";

const ROUTE_TITLES_SEP = " · ";

export function App() {
  const route = useRoute();
  const first = useRef(true);

  useEffect(() => {
    const r = ROUTES.find((x) => x.path === route);
    const label = r ? routeLabel(r) : "";
    document.title = route ? `${label}${ROUTE_TITLES_SEP}${PROTOCOL_NAME}` : PROTOCOL_NAME;
    if (first.current) {
      first.current = false;
      return;
    }
    // Bei Seitenwechsel nach oben und Fokus auf die Überschrift (Screenreader)
    window.scrollTo(0, 0);
    document.querySelector<HTMLElement>("[data-route-heading]")?.focus({ preventScroll: true });
  }, [route]);

  return (
    <>
      <a className="skip-link" href="#inhalt" onClick={(e) => {
        e.preventDefault();
        document.getElementById("inhalt")?.focus();
      }}>
        {tr("Zum Inhalt springen", "Skip to content")}
      </a>
      <Header route={route} />
      <main id="inhalt" tabIndex={-1}>
        {route === "" && <Landing />}
        {route === "maerkte" && <Markets />}
        {route === "wallet" && <Wallet />}
        {route === "tauschen" && <Swap />}
        {route === "vault" && <Vault />}
        {route === "orakel" && <Oracle />}
        {route === "statistiken" && <Statistics />}
        {route === "pool" && <Pool />}
        {route === "so-funktioniert-es" && <HowItWorks />}
        {route === "faq" && <Faq />}
        {route === "rechtliches" && <Rechtliches />}
        {route === "impressum" && <Impressum />}
        {route === "datenschutz" && <Datenschutz />}
      </main>
      <Footer />
    </>
  );
}
