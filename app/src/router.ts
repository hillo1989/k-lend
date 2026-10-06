import { useSyncExternalStore } from "react";
import { tr } from "./lib/i18n";

export const ROUTES = [
  { path: "", label: "Start", en: "Home" },
  { path: "maerkte", label: "Märkte", en: "Markets" },
  { path: "wallet", label: "Wallet", en: "Wallet" },
  { path: "tauschen", label: "Tauschen", en: "Swap" },
  { path: "vault", label: "Vault", en: "Vault" },
  { path: "orakel", label: "Orakel", en: "Oracle" },
  { path: "statistiken", label: "Statistiken", en: "Statistics" },
  { path: "pool", label: "Pool", en: "Pool" },
  { path: "so-funktioniert-es", label: "So funktioniert es", en: "How it works" },
  // nur im Fuß verlinkt
  { path: "faq", label: "FAQ", en: "FAQ", footer: true },
  { path: "rechtliches", label: "Rechtliches & Risiken", en: "Legal & risks", footer: true },
  { path: "impressum", label: "Impressum", en: "Imprint", footer: true },
  { path: "datenschutz", label: "Datenschutz", en: "Privacy", footer: true },
] as const;

/** Beschriftung eines Reiters in der aktuellen Sprache */
export const routeLabel = (r: { label: string; en: string }) => tr(r.label, r.en);

export const NAV_ROUTES = ROUTES.filter((r) => !("footer" in r));
export const FOOTER_ROUTES = ROUTES.filter((r) => "footer" in r);

export type RoutePath = (typeof ROUTES)[number]["path"];

function read(): RoutePath {
  const h = window.location.hash.replace(/^#\/?/, "").split(/[?#]/)[0];
  return (ROUTES.find((r) => r.path === h)?.path ?? "") as RoutePath;
}

function subscribe(cb: () => void) {
  window.addEventListener("hashchange", cb);
  return () => window.removeEventListener("hashchange", cb);
}

export function useRoute(): RoutePath {
  return useSyncExternalStore(subscribe, read, () => "" as RoutePath);
}

export const href = (p: RoutePath) => `#/${p}`;
