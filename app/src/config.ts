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
 * Höchstzahl der Unterzeichner im Register (contracts/signer_register_v4.sil
 * und protocol/src/contracts.rs MAX_SIGNERS = 7; mit 9 lag das Register bei
 * 28 Sigops). Audit 20 A20d-4.
 */
export const MAX_SIGNERS = 7;

/**
 * Automatische Zinsregel des GHOST-Agenten, wie sie die Texte beschreiben
 * (HowItWorks, FAQ, Vault-Hilfe, Landing, Vorprüfung).
 * Abgeglichen mit protocol/src/math.rs (RATE_ZONE 0,03, RATE_MIN_PCT 2,
 * RATE_MAX_PCT 20, RATE_STEP_PCT 0,5) und rate.rs (MIN_SAMPLES 6,
 * MIN_POOL_GHOST 10, MIN_TRADE_MOVE 2 % Bewegung des Tauschverhältnisses je Stunde).
 */
export const RATE_RULE = {
  /** darunter (USD je GHOST, Median der Stunde) steigt der Zins */
  lowUsd: 0.97,
  /** darüber sinkt er */
  highUsd: 1.03,
  /** Schritt je Stunde in Prozentpunkten (vom Vertrag begrenzt) */
  stepPp: 0.5,
  /** Grundzins = Untergrenze der Regel (% p. a.) */
  basePct: 2,
  /** Obergrenze des Vertrags (% p. a.) */
  maxPct: 20,
  /** so viele Messungen der Stunde mindestens */
  minSamples: 6,
  /** so viele GHOST muss der Pool mindestens halten */
  minPoolGhost: 10,
} as const;

/** Zahl für die Zinsregel-Texte: deutsch mit Komma, englisch mit Punkt */
export const ruleNum = (x: number, lang: "de" | "en") => (lang === "de" ? String(x).replace(".", ",") : String(x));


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
 * Version 4: Kommt so lange kein Preis, darf jeder das Orakel einfrieren
 * (Prägen, Einlösen, Liquidieren und Tausch gesperrt). Live-Wert:
 * status.signers.freezeAfterHours; dies ist nur der Ersatz ohne Live-Daten.
 */
export const ORACLE_FREEZE_AFTER_HOURS = 2;

/**
 * Ab diesem Alter gilt der Orakelpreis als veraltet (Anzeige-Warnung), bei
 * 2 h Einfrier-Frist 75 min. Der Agent aktualisiert spätestens nach der
 * halben Frist (60 min, ghostctl `heartbeat_min`), auch ohne Preisänderung;
 * „veraltet“ heißt also: das Herzschlag-Update ist eine Viertelstunde
 * überfällig, und das Einfrieren rückt näher (Audit 20 A20d-3; die frühere
 * Grenze 390 min stammte aus Version 3 ohne Einfrieren).
 */
export function oracleStaleMinutes(freezeAfterHours: number = ORACLE_FREEZE_AFTER_HOURS): number {
  const f = Number.isFinite(freezeAfterHours) && freezeAfterHours > 0 ? freezeAfterHours * 60 : ORACLE_FREEZE_AFTER_HOURS * 60;
  return Math.round(Math.min(f / 2 + 15, f - 15));
}
export const ORACLE_STALE_MINUTES = oracleStaleMinutes();

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
