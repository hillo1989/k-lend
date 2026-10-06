// Hinweis „Als App installieren“ für Handys. Android/Chrome: eigener Knopf über
// das Ereignis beforeinstallprompt (sonst Menü ⋮). iPhone/Safari kennt kein
// Installations-Ereignis: dort ein Hinweis auf „Teilen → Zum Home-Bildschirm“.
// Nicht, wenn die Seite schon als App läuft; wegklicken merkt sich der Browser.
import { useEffect, useState } from "react";
import { tr } from "../lib/i18n";

type InstallEvent = Event & { prompt: () => Promise<void>; userChoice: Promise<{ outcome: string }> };

const KEY = "klend-install-hint-weg";

export type Platform = "ios" | "android" | null;

/** iPhone/iPad mit Safari bzw. Android; Desktop und andere: null */
export function platformOf(ua: string): Platform {
  if (/iphone|ipad|ipod/i.test(ua)) return /crios|fxios|edgios/i.test(ua) ? null : "ios";
  if (/android/i.test(ua)) return "android";
  return null;
}

function standalone(): boolean {
  try {
    return window.matchMedia?.("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone === true;
  } catch {
    return false;
  }
}

export function InstallHint() {
  const [evt, setEvt] = useState<InstallEvent | null>(null);
  const [gone, setGone] = useState(() => {
    try {
      return localStorage.getItem(KEY) === "1";
    } catch {
      return false;
    }
  });
  const platform = platformOf(navigator.userAgent);

  useEffect(() => {
    const onPrompt = (e: Event) => {
      e.preventDefault();
      setEvt(e as InstallEvent);
    };
    const onInstalled = () => setGone(true);
    window.addEventListener("beforeinstallprompt", onPrompt);
    window.addEventListener("appinstalled", onInstalled);
    return () => {
      window.removeEventListener("beforeinstallprompt", onPrompt);
      window.removeEventListener("appinstalled", onInstalled);
    };
  }, []);

  if (gone || platform === null || standalone()) return null;
  const close = () => {
    setGone(true);
    try {
      localStorage.setItem(KEY, "1");
    } catch {
      /* ohne Speicher erscheint der Hinweis beim nächsten Besuch wieder */
    }
  };
  const install = async () => {
    if (!evt) return;
    await evt.prompt();
    await evt.userChoice.catch(() => null);
    setEvt(null);
  };

  return (
    <div className="install-hint" role="note">
      <div className="container install-hint-row">
        <span>
          {platform === "ios"
            ? tr("K.Lend als App: unten auf „Teilen“ tippen, dann „Zum Home-Bildschirm“.", "K.Lend as an app: tap “Share” below, then “Add to Home Screen”.")
            : evt
              ? tr("K.Lend als App auf den Startbildschirm legen.", "Put K.Lend on your home screen as an app.")
              : tr("K.Lend als App: im Menü ⋮ „App installieren“ bzw. „Zum Startbildschirm hinzufügen“.", "K.Lend as an app: in the ⋮ menu choose “Install app” or “Add to Home screen”.")}
        </span>
        {platform === "android" && evt && (
          <button type="button" className="btn btn-primary btn-sm" onClick={() => void install()}>
            {tr("App installieren", "Install app")}
          </button>
        )}
        <button type="button" className="btn btn-ghost btn-sm" onClick={close} aria-label={tr("Hinweis schließen", "Close hint")}>
          ✕
        </button>
      </div>
    </div>
  );
}
