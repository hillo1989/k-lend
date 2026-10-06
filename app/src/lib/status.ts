// Live-Zustand des Protokolls, wie ihn `ghostctl status --json` liefert
// (über die Vite-Route /api/status, siehe vite.config.ts).
import { ORACLE_STALE_MINUTES, type NetworkId } from "../config";
import { locale, tr } from "./i18n";

export interface OracleStatus {
  covenantId: string;
  kasUsd: number; // USD je KAS
  seq: number;
  ageMinutes: number;
  ratePctYear: number; // wird vom GHOST-Agenten nach dem GHOST-Kurs angepasst (Version 3)
  index: number; // Zinsindex (1,0 = Start)
  fresh: boolean;
  freshError: string | null;
  /** Version 4: eingefroren (Frist ohne Preis abgelaufen); Prägen, Einlösen, Liquidieren und Tausch gesperrt */
  frozen?: boolean;
  /** Minuten, bis jeder einfrieren darf (negativ = Frist abgelaufen) */
  freezeInMinutes?: number;
}

/** Ein Unterzeichner-Satz des Registers (x-only-Pubkeys, Schwellen) */
export interface SignerSetStatus {
  keys: string[];
  threshold: number;
  rotateThreshold: number;
}

/** Version 4: Unterzeichner-Register */
export interface SignersStatus {
  registerCovenantId: string;
  set: SignerSetStatus;
  fallback: SignerSetStatus | null;
  rotateDelayHours: number;
  emergencyAfterDays: number;
  emergencyDelayHours: number;
  freezeAfterHours: number;
  /** Änderung im Register von außen erkannt (fremde Ankündigung, Absage oder Notfall) */
  foreignChange?: boolean;
  /** Notfall-Ankündigung offen */
  emergencyOpen?: boolean;
  /** Register hat einen Satz, den die Zustandsdatei nicht kennt: Preis-Updates gesperrt */
  unknownSet?: boolean;
  /** abgesagte oder überholte Tickets (je 1 KAS, `ghostctl signers clear`) */
  oldTickets?: number;
  /** angekündigter Austausch; valid = noch nicht abgesagt */
  rotation: { set: SignerSetStatus; fallback: SignerSetStatus | null; emergency: boolean; valid: boolean; readyInHours: number } | null;
}

export interface VaultStatus {
  index: number;
  owner: string; // x-only-Pubkey (hex)
  covenantId: string;
  collateralKas: number;
  /** geprägte, noch nicht getilgte GHOST (wächst nicht mit dem Zins) */
  debtGhost: number;
  /** Version 3: offener Zins in USD, bis jetzt aufgelaufen (fehlt bei älterem ghostctl) */
  interestUsd?: number;
  /** Quote auf Schuld + Zins */
  ratioPct: number | null;
  liquidationPriceUsd: number | null;
  maxMintGhost: number;
  /** von Dritten verändert (z. B. liquidiert) – Aktionen gesperrt, bis ghostctl nachgeladen hat */
  stale?: boolean;
  /** Schuld 0 und Zins ≥ Sicherheit: jeder darf ihn zugunsten der Zinsadresse auflösen (fehlt bei älterem ghostctl) */
  sweepable?: boolean;
}

/** Offener Tauschpool KAS/GHOST (contracts/ghost_pool.sil), sofern angelegt */
export interface PoolStatus {
  covenantId: string;
  lpCovenantId: string;
  kasSompi: string; // exakt, als Dezimalstring (bigint)
  ghostUnits: string;
  /** Anteile insgesamt (davon die Start-Einlage in sompi dauerhaft gesperrt) */
  shares: string;
  feeBps: number;
  /** Reserve ließ sich nicht bestimmen (REST-API nicht erreichbar o. Ä.): Pool-Aktionen gesperrt */
  unresolved?: string | null;
  /** Kursband in bps (GHOST bei 1 USD ± Band); null = alter Pool ohne Band */
  bandBps?: number | null;
  /** größter Kauf bzw. Verkauf, den das Band gerade zulässt (0 = Richtung gesperrt) */
  maxBuyKas?: number | null;
  maxSellGhost?: number | null;
}

export interface DeployedStatus {
  network: NetworkId;
  deployed: true;
  daa: number;
  oracle: OracleStatus;
  /** Version 4 (fehlt bei älterem ghostctl) */
  signers?: SignersStatus;
  /**
   * maxDebtGhost: Höchstschuld je Vault beim Prägen (null = unbegrenzt; gilt nur für die Schuld, nicht für den Zins).
   * redeemFeePct: Abschlag bei der Rücknahme (Version 3: 1), fehlt bei älterem ghostctl.
   */
  params: { mcrPct: number; liqPct: number; bonusPct: number; maxDebtGhost?: number | null; redeemFeePct?: number };
  factoryCovenantId: string;
  ghostCovenantId: string;
  /** interestUsd: offener Zins aller Vaults in USD (Version 3, fehlt bei älterem ghostctl) */
  totals: { vaults: number; collateralKas: number; debtGhost: number; interestUsd?: number };
  vaults: VaultStatus[];
  tokens: { owner: string; amountGhost: number }[];
  pool?: PoolStatus | null;
}

export type ProtocolStatus = DeployedStatus | { network: NetworkId; deployed: false; error?: string; nodeDown?: boolean };

/** Fehler beim Laden; nodeDown = öffentliche Kaspa-Nodes nicht erreichbar */
export class StatusError extends Error {
  nodeDown: boolean;
  detail?: string;
  constructor(message: string, nodeDown = false, detail?: string) {
    super(message);
    this.nodeDown = nodeDown;
    this.detail = detail;
  }
}

export async function fetchStatus(network: NetworkId, signal?: AbortSignal): Promise<ProtocolStatus> {
  const res = await fetch(`./api/status?network=${encodeURIComponent(network)}`, { signal, cache: "no-store" });
  const text = await res.text();
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Error(
      res.status === 404
        ? tr(
            "Keine Datenquelle: /api/status gibt es nur, wenn die Seite über „npm run dev“ bzw. den Doppelklick-Starter läuft.",
            "No data source: /api/status only exists when the page runs via “npm run dev” or the double-click starter.",
          )
        : tr(`Unerwartete Antwort (${res.status}).`, `Unexpected response (${res.status}).`),
    );
  }
  const b = body as { ok?: boolean; error?: string; nodeDown?: boolean; detail?: string };
  if (!res.ok || b.ok === false) throw new StatusError(b.error ?? tr(`Fehler ${res.status}`, `Error ${res.status}`), !!b.nodeDown, b.detail);
  const s = body as ProtocolStatus;
  if (typeof s !== "object" || s === null || typeof s.deployed !== "boolean") throw new Error(tr("Unbekanntes Datenformat.", "Unknown data format."));
  return s;
}

/** Orakelpreis zu alt oder von ghostctl als nicht frisch gemeldet. */
export function oracleStale(o: OracleStatus): boolean {
  return !o.fresh || o.ageMinutes > ORACLE_STALE_MINUTES;
}

// ---- Umrechnung in die Ganzzahl-Einheiten des Vertrags (für den Rechner)
export const kasUsdToUnits = (usd: number) => BigInt(Math.round(usd * 1e8));
export const indexToUnits = (idx: number) => BigInt(Math.round(idx * 1e9));
export const pctToBps = (pct: number) => BigInt(Math.round(pct * 100));

/**
 * Wallet-Pubkey → x-only (32 Byte hex). Komprimierte Schlüssel (33 Byte,
 * Präfix 02/03) verlieren das erste Byte; 32-Byte-Schlüssel bleiben.
 */
export function xOnlyKey(pub: string | null | undefined): string | null {
  if (!pub) return null;
  const h = pub.trim().toLowerCase().replace(/^0x/, "");
  if (/^0[23][0-9a-f]{64}$/.test(h)) return h.slice(2);
  if (/^[0-9a-f]{64}$/.test(h)) return h;
  return null;
}

export const shortHex = (h: string, head = 6, tail = 6) => (h.length <= head + tail + 1 ? h : `${h.slice(0, head)}…${h.slice(-tail)}`);

/** Zahl im Format der aktuellen Sprache (Name historisch: „de“) */
export const de = (n: number, maxFrac = 2, minFrac = 0) =>
  n.toLocaleString(locale(), { minimumFractionDigits: minFrac, maximumFractionDigits: maxFrac });

/**
 * Anzeige-Name eines Vaults: Jeder Nutzer zählt seine eigenen Vaults ab 1
 * („Vault 1“, „Vault 2“ …, in der Reihenfolge des Anlegens). Fremde Vaults
 * heißen nach den ersten 8 Zeichen ihrer Covenant-ID. Nur Anzeige – intern
 * zählt die feste Covenant-ID bzw. die Nummer der Zustandsdatei.
 */
export function vaultLabel(v: Pick<VaultStatus, "index" | "owner" | "covenantId">, all: readonly Pick<VaultStatus, "index" | "owner">[], me: string | null): string {
  const mine = me !== null && me !== "" && v.owner.toLowerCase() === me.toLowerCase();
  if (!mine) return `${tr("Fremder Vault", "Other vault")} ${v.covenantId.slice(0, 8)}`;
  const n = all.filter((x) => x.owner.toLowerCase() === me.toLowerCase() && x.index <= v.index).length;
  return `Vault ${n}`;
}
