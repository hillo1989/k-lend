// Audit 20 d, Aussagen: A20d-4 (7 statt 9 Unterzeichner), A20d-5 (Auflösen:
// 0,1 KAS aus dem Vault, Auslöser zahlt nichts), A20d-7 (Aussagen des lokalen
// Modus im öffentlichen Modus), Zinsregel mit Totzone (Abgleich nach dem
// Zusammenführen, siehe RATE_RULE in config.ts).
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setLangGlobal } from "../lib/i18n";

const g = globalThis as unknown as { __pub: boolean };
vi.mock("../lib/AccountContext", () => ({ usePublicMode: () => g.__pub }));
vi.mock("../lib/StatusContext", () => ({ useStatus: () => ({ status: null, network: "mainnet" }) }));

const { Faq } = await import("./Faq");
const { HowItWorks } = await import("./HowItWorks");
const { Landing } = await import("./Landing");
const { actionMeta } = await import("../lib/commands");
const { MAX_SIGNERS, RATE_RULE } = await import("../config");
const srcOf = import.meta.glob(["./Oracle.tsx", "../components/WalletButton.tsx"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

const html = (c: () => unknown) => renderToStaticMarkup(createElement(c as never)).replaceAll("&quot;", '"').replaceAll("&#x27;", "'");

beforeEach(() => {
  setLangGlobal("de");
  g.__pub = true;
});

describe("A20d-4: höchstens 7 Unterzeichner (signer_register_v4.sil MAX_SIGNERS)", () => {
  it("FAQ, So funktioniert es und Orakel-Seite nennen 7, nirgends 9", () => {
    expect(MAX_SIGNERS).toBe(7);
    for (const lang of ["de", "en"] as const) {
      setLangGlobal(lang);
      const all = html(Faq) + html(HowItWorks);
      expect(all).not.toMatch(/höchstens 9|at most 9/);
      expect(all).toMatch(lang === "de" ? /höchstens 7 Schlüssel/ : /at most 7 keys/);
      expect(all).toMatch(lang === "de" ? /n Schlüssel \(höchstens 7\)/ : /n keys \(at most 7\)/);
    }
    const oracle = srcOf["./Oracle.tsx"];
    expect(oracle).not.toMatch(/höchstens 9|at most 9/);
    expect(oracle).toContain("(höchstens ${MAX_SIGNERS})");
  });
});

describe("A20d-5: Auflösen – 0,1 KAS aus dem Vault, wer auflöst, zahlt nichts", () => {
  it("Hilfetext der Aktion, So funktioniert es und FAQ", () => {
    const help = actionMeta().sweep.help;
    expect(help).not.toMatch(/0,01 KAS|0,05 KAS|zahlst du/);
    expect(help).toMatch(/0,1 KAS trägt der Vault/);
    expect(help).toMatch(/wer auflöst, zahlt nichts/);
    const how = html(HowItWorks);
    expect(how).not.toMatch(/bis auf 0,01/);
    expect(how).toMatch(/nach Abzug von 0,1 KAS Netzgebühr an die Zinsadresse; die Gebühr trägt der Vault, wer auflöst, zahlt nichts/);
    expect(html(Faq)).toMatch(/Die Netzgebühr von 0,1 KAS dafür kommt aus dem Vault; wer auflöst, zahlt nichts/);
    setLangGlobal("en");
    expect(actionMeta().sweep.help).toMatch(/0\.1 KAS/);
    expect(html(HowItWorks)).not.toMatch(/except for 0\.01/);
  });
});

describe("A20d-7: öffentlicher Modus ohne Aussagen des lokalen Modus", () => {
  it("Landing Schritt 3: öffentlich signiert die Wallet, lokal ghostctl", () => {
    const pub = html(Landing);
    expect(pub).not.toContain("signiert und sendet der lokale Server");
    expect(pub).toContain("Du signierst ihn in deiner Wallet");
    g.__pub = false;
    expect(html(Landing)).toContain("signiert und sendet der lokale Server");
  });
  it("FAQ Tresor-Gebühr: öffentlich legt der Agent nie etwas dazu", () => {
    const pub = html(Faq);
    expect(pub).toContain("der Agent legt nie etwas dazu");
    expect(pub).not.toContain("vom Schlüssel seines Betreibers");
    g.__pub = false;
    expect(html(Faq)).toContain("vom Schlüssel seines Betreibers");
  });
  it("keine „Version 2.1 im Mainnet“ mehr; Version 4 läuft dort", () => {
    for (const lang of ["de", "en"] as const) {
      setLangGlobal(lang);
      const how = html(HowItWorks);
      expect(how).not.toMatch(/2\.1/);
      expect(how).toMatch(lang === "de" ? /im Mainnet läuft Version 4/ : /version 4 runs on mainnet/);
    }
  });
  it("WalletButton nennt auch den öffentlichen Schlüssel", () => {
    const wb = srcOf["../components/WalletButton.tsx"];
    expect(wb).not.toContain("Die Seite liest nur Adresse, Netz und Guthaben.");
    expect(wb).toContain("Die Seite liest Adresse, Netz, Guthaben und den öffentlichen Schlüssel");
  });
});

describe("Zinsregel mit Totzone (ABGLEICH nach dem Zusammenführen mit dem Protokoll-Zweig)", () => {
  it("Zahlen kommen aus RATE_RULE; alte Grenzen 0,995/1,005 stehen nirgends mehr", () => {
    expect(RATE_RULE).toMatchObject({ lowUsd: 0.97, highUsd: 1.03, basePct: 2, maxPct: 20, stepPp: 0.5 });
    for (const lang of ["de", "en"] as const) {
      setLangGlobal(lang);
      const all = html(Faq) + html(HowItWorks) + html(Landing);
      expect(all).not.toMatch(/0[,.]995|1[,.]005/);
      expect(all).toMatch(lang === "de" ? /unter 0,97 USD/ : /below 0\.97 USD/);
      expect(all).toMatch(lang === "de" ? /über 1,03 USD/ : /above 1\.03 USD/);
      expect(all).toMatch(lang === "de" ? /Totzone ±3 %/ : /dead zone ±3 %/);
      expect(all).toMatch(lang === "de" ? /tatsächlich getauscht/ : /actually traded/);
      expect(all).toMatch(lang === "de" ? /2 % \(Grundzins, Untergrenze\)/ : /2 % \(base rate, floor\)/);
    }
  });
});
