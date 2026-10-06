// Zweisprachigkeit Deutsch/Englisch.
//
// Texte stehen direkt am Verwendungsort als Paar: tr("Deutsch", "English").
// tr() liest die aktuelle Sprache aus einer Modulvariable und funktioniert
// daher auch außerhalb von Komponenten (Vorprüfung, Aktionstexte). Beim
// Umschalten wird die App neu aufgebaut (LangKeyed in main.tsx), damit jeder
// Text neu berechnet wird. Meldungen von ghostctl und vom lokalen Server
// bleiben deutsch.
import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

export type Lang = "de" | "en";

const STORE_KEY = "gh-lang";

function detect(): Lang {
  try {
    const v = localStorage.getItem(STORE_KEY);
    if (v === "de" || v === "en") return v;
  } catch {
    /* Speicher gesperrt */
  }
  const nav = typeof navigator !== "undefined" ? navigator.language : "de";
  return (nav || "de").toLowerCase().startsWith("de") ? "de" : "en";
}

let current: Lang = typeof window === "undefined" ? "de" : detect();

export function getLang(): Lang {
  return current;
}

/** Nur für Tests und den Provider */
export function setLangGlobal(l: Lang) {
  current = l;
}

/** Text in der aktuellen Sprache */
export function tr(de: string, en: string): string {
  return current === "en" ? en : de;
}

/** Locale für Zahlen und Datum */
export function locale(): string {
  return current === "en" ? "en-US" : "de-DE";
}

const Ctx = createContext<{ lang: Lang; setLang(l: Lang): void }>({ lang: current, setLang: () => {} });

export function LangProvider({ children }: { children: ReactNode }) {
  const [lang, set] = useState<Lang>(current);
  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);
  const setLang = (l: Lang) => {
    current = l;
    try {
      localStorage.setItem(STORE_KEY, l);
    } catch {
      /* egal */
    }
    set(l);
  };
  return <Ctx.Provider value={{ lang, setLang }}>{children}</Ctx.Provider>;
}

export function useLang() {
  return useContext(Ctx);
}
