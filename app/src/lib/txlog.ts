// Verlauf der von dieser Seite gesendeten Transaktionen, nur im Browser
// (localStorage). Er ist eine Bequemlichkeit, keine Buchhaltung: Was von
// anderswo gesendet wurde, fehlt, und in privaten Fenstern bleibt er leer.
import type { NetworkId } from "../config";

export interface TxLogEntry {
  at: number;
  network: NetworkId;
  key: string;
  action: string;
  label: string;
  amount: string | null;
  unit: string | null;
  to: string | null;
  txids: string[];
  /** nur ein Teil der Transaktionen ging hinaus */
  partial?: boolean;
  /** Nachricht zur Sendung (z. B. „Miete“); onchain = öffentlich in der Tx,
   *  encrypted = verschlüsselt an den Empfänger in der Tx (hier lesbar, weil der
   *  Absender sie nur aus diesem Verlauf kennt) */
  message?: string;
  onchain?: boolean;
  encrypted?: boolean;
}

const STORE = "ghost.txlog.v1";
const MAX = 200;

function readAll(): TxLogEntry[] {
  try {
    const raw = window.localStorage.getItem(STORE);
    const v = raw ? JSON.parse(raw) : [];
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export function logTx(e: TxLogEntry): void {
  try {
    const all = [e, ...readAll()].slice(0, MAX);
    window.localStorage.setItem(STORE, JSON.stringify(all));
    window.dispatchEvent(new Event("ghost-txlog"));
  } catch {
    // Speicher gesperrt (privates Fenster, Einstellungen): dann ohne Verlauf
  }
}

export function txLog(network: NetworkId, key: string | null): TxLogEntry[] {
  return readAll().filter((e) => e.network === network && (key === null || e.key === key));
}

export function explorerTx(network: NetworkId, txid: string): string {
  return network === "mainnet" ? `https://explorer.kaspa.org/txs/${txid}` : `https://explorer-tn10.kaspa.org/txs/${txid}`;
}

export function explorerAddress(network: NetworkId, address: string): string {
  return network === "mainnet" ? `https://explorer.kaspa.org/addresses/${address}` : `https://explorer-tn10.kaspa.org/addresses/${address}`;
}
