// Hinweis „Neue Version verfügbar“: Die Seite lädt sich nach einem Update auf
// dem Server nicht von selbst neu, offene Fenster (auch im Browser der
// Wallet-App) zeigen sonst den alten Stand. Alle 5 Minuten und beim Zurückkehren
// zur Seite wird index.html geholt und das eingebundene Skript verglichen.
import { useEffect, useState } from "react";
import { tr } from "../lib/i18n";

const EVERY_MS = 5 * 60_000;

/** Name des Haupt-Skripts aus index.html (Vite: /assets/main-<hash>.js) */
export function mainScript(html: string): string | null {
  return html.match(/<script[^>]+src="([^"]*\/assets\/main-[^"]+\.js)"/)?.[1] ?? null;
}

function current(): string | null {
  const s = Array.from(document.querySelectorAll<HTMLScriptElement>("script[src]")).map((x) => x.getAttribute("src") ?? "");
  return s.find((x) => /\/assets\/main-[^/]+\.js$/.test(x)) ?? null;
}

export function UpdateHint() {
  const [stale, setStale] = useState(false);

  useEffect(() => {
    const mine = current();
    if (!mine) return; // Entwicklung (vite dev): kein gebautes Skript
    let stop = false;
    const check = () => {
      if (stop || document.visibilityState === "hidden") return;
      fetch("./", { cache: "no-store" })
        .then((r) => (r.ok ? r.text() : null))
        .then((h) => {
          const now = h ? mainScript(h) : null;
          if (now && now !== mine) setStale(true);
        })
        .catch(() => {});
    };
    const t = window.setInterval(check, EVERY_MS);
    document.addEventListener("visibilitychange", check);
    return () => {
      stop = true;
      clearInterval(t);
      document.removeEventListener("visibilitychange", check);
    };
  }, []);

  if (!stale) return null;
  return (
    <div className="install-hint" role="status">
      <div className="container install-hint-row">
        <span>{tr("Es gibt eine neue Version von K.Lend.", "A new version of K.Lend is available.")}</span>
        <button type="button" className="btn btn-primary btn-sm" onClick={() => window.location.reload()}>
          {tr("Neu laden", "Reload")}
        </button>
      </div>
    </div>
  );
}
