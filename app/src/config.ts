import { tr } from "./lib/i18n";

// Zentrale Konstanten der Oberfläche. Name und Ticker sind Arbeitstitel und
// werden nur hier geändert.

// Bewusst OHNE „Kaspa" im Namen: Das Projekt ist unabhängig und soll nicht
// den Eindruck erwecken, vom Kaspa-Projekt betrieben zu werden.
export const PROTOCOL_NAME = "K.Lend";
/** Kurzform für enge Stellen (Kopfzeile auf dem Handy). */
export const PROTOCOL_SHORT = "K.Lend";
export const STABLE_SYMBOL = "GHOST";
/** Der Name spielt auf GHOSTDAG an, den Konsens von Kaspa. Die Verträge verwenden denselben Namen (contracts/stable_vault.sil). */
export const stableTagline = () => tr(`${STABLE_SYMBOL} – der überbesicherte Dollar auf Kaspa L1`, `${STABLE_SYMBOL} – the overcollateralized dollar on Kaspa L1`);
export const NATIVE = "KAS";


/**
 * Vertragsparameter, wie sie in den lokalen Tests verwendet werden
 * (protocol/tests/vault_tests.rs). Einheiten wie in contracts/stable_vault.sil:
 * Quoten in Basispunkten (10 000 = 100 %).
 */
export const PARAMS = {
  mcrBps: 20_000n, // Mindestquote für Prägen/Abheben: 200 %
  liqBps: 15_000n, // Liquidationsschwelle: 150 %
  bonusBps: 1_000n, // Liquidationsbonus: 10 %
} as const;

/** Kaspa erzeugt nach Crescendo 10 Blöcke je Sekunde, der DAA-Score steigt entsprechend. */
export const DAA_PER_SECOND = 10;

/**
 * Annahme für die Zinsprojektion: Die Orakel-Unterzeichner schreiben den Index etwa
 * stündlich fort. Der Vertrag verzinst nur bei jedem Update (diskret), daher
 * hängt der effektive Jahreszins leicht von diesem Intervall ab.
 */
export const ASSUMED_ORACLE_INTERVAL_DAA = 3_600n * 10n;

/**
 * Fallback-Werte für den Szenario-Rechner, solange keine Live-Daten da sind
 * (z. B. vor dem Mainnet-Deployment). Werden nie als Protokolldaten angezeigt.
 */
export const SCENARIO_FALLBACK = {
  kasUsd: "0.047", // Punkt: in beiden Sprachen als Dezimalpunkt gelesen
  stableAprPercent: "0", // ohne Live-Daten; sonst gilt der Satz des Orakels
  stableIndex: 1_000_000_000n,
} as const;

/** Pool: Vertrag in Arbeit, alles hier sind Annahmen zur Veranschaulichung. */
export const POOL_ASSUMPTIONS = {
  suppliedKas: 12_500_000,
  borrowedKas: 7_800_000,
  baseRatePct: 0,
  slope1Pct: 4,
  kinkPct: 80,
  slope2Pct: 60,
  reserveFactorPct: 10,
} as const;

export type NetworkId = "mainnet" | "testnet-10";

/**
 * Netze, die ghostctl kennt. Die Schlüsselpfade sind nur Vorschläge für die
 * angezeigten Befehle (MAINNET.md bzw. TESTNET_LOG.md) und in der Seite änderbar.
 */
export const NETWORKS: Record<NetworkId, { label: string; short: string; ownerKey: string; committeeKey: string }> = {
  mainnet: {
    label: "Mainnet",
    short: "Mainnet",
    ownerKey: "keys/mainnet-owner.json",
    committeeKey: "keys/mainnet-committee.json",
  },
  "testnet-10": {
    label: "Testnetz (TN10)",
    short: "Testnetz",
    ownerKey: "keys/tn10-user.json",
    committeeKey: "keys/tn10-committee.json",
  },
};
export const DEFAULT_NETWORK: NetworkId = "mainnet";

/**
 * Ab diesem Alter gilt der Orakelpreis als veraltet (Anzeige-Warnung).
 * Der Dauerbetrieb (GHOST-Agent starten.command) aktualisiert spätestens nach
 * 6 h, auch ohne Preisänderung. Deshalb liegt die Grenze bei 6,5 h und nicht bei 60 min.
 */
export const ORACLE_STALE_MINUTES = 390;

export const WALLET_LINKS = {
  kasware: "https://kasware.xyz/",
  kastle: "https://kastle.cc/",
} as const;

/**
 * Angaben für das Impressum (§ 5 DDG). Vor dem öffentlichen Start vom Betreiber
 * einzutragen; solange ein Feld leer ist, zeigt die Seite einen Hinweis statt
 * erfundener Angaben.
 */
export const IMPRESSUM = {
  name: "Andy Frank",
  strasse: "Bordenbergweg 14",
  plzOrt: "64367 Mühltal",
  email: "info@k-lend.com",
};
