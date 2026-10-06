// Anbindung an Browser-Wallets – NUR LESEND.
//
// Diese Seite sendet und signiert nie etwas. Die Typen unten enthalten deshalb
// absichtlich nur lesende Methoden (plus Verbinden/Trennen). sendKaspa,
// signPskt, signMessage, signTx, signAndBroadcastTx usw. sind nicht deklariert
// und werden hier nirgends aufgerufen. Signiert (nicht gesendet) wird nur an
// zwei Stellen: der lokalen Probe-Seite /wallet-probe.html (src/probe/probe.ts)
// und den Aktionsformularen mit Browser-Wallet (src/wallet/actions.ts,
// components/WalletSignFlow.tsx), beide über signPskt bzw. signTx.
//
// KasWare (window.kasware), geprüft am 28.09.2026 gegen:
//   https://docs.kasware.xyz/wallet/developer-documentation/kaspa
//   (Volltext: https://docs.kasware.xyz/wallet/llms-full.txt, Abschnitte
//   „Connection API", „Account API", „Events") und den Quelltext
//   https://github.com/kasware-wallet/extension/blob/main/src/content-script/pageProvider/index.ts
//   sowie .../pageProvider/pushEventHandlers.ts
//   - requestAccounts(): Promise<string[]>        öffnet den Freigabe-Dialog
//   - getAccounts():     Promise<string[]>        ohne Dialog, leer wenn nicht verbunden
//   - getNetwork():      Promise<string>          kaspa_mainnet | kaspa_testnet_10 |
//                                                 kaspa_testnet_11 | kaspa_testnet_12 | kaspa_devnet
//   - getBalance():      Promise<{confirmed,unconfirmed,total} | {} | null>, Strings in sompi
//   - getPublicKey():    Promise<string>          hex, Beispiel in der Doku „03cbae…296f“
//                                                 (33 Byte komprimiert; Abschnitt „Account API“,
//                                                 im Quelltext pageProvider/index.ts getPublicKey)
//   - disconnect(origin: string)
//   - on/removeListener: 'accountsChanged' (string[]), 'networkChanged' (string),
//                        'balanceChanged' (Form weicht von getBalance ab → wir laden neu)
//   - Laut Quelltext zusätzlich 'disconnect' (sendet vorher accountsChanged([])).
//   - Ablehnung: Fehler aus eth-rpc-errors, Code 4001 (userRejectedRequest).
//     Den Code haben wir NICHT in einer laufenden Extension beobachtet.
//
// Kastle (window.kastle), geprüft am 28.09.2026 gegen:
//   https://docs.kastle.cc/readme/how-to-integrate/kastle-wallet-api
//   (Volltext: https://docs.kastle.cc/llms-full.txt, Abschnitt „Kastle Wallet API")
//   - connect(): Promise<boolean>
//   - getAccount(): Promise<{ address, publicKey }>   Format von publicKey NICHT dokumentiert
//                   (wir akzeptieren 33-Byte-komprimiert oder 32-Byte-x-only)
//   - getNetwork(): Promise<string>   "mainnet" | "testnet-10" | "testnet-11"
//   - getBalance(): Promise<{ balance: string }> in sompi (ab Extension 2.47.0)
//   - on/removeListener 'accountsChanged' (string[], leer = getrennt), 'networkChanged'
//   - Eine disconnect-Methode ist NICHT dokumentiert → wir vergessen die Verbindung nur lokal.

import { tr } from "../lib/i18n";

export type WalletKind = "kasware" | "kastle";

type Handler = (...args: never[]) => void;

interface KaswareProvider {
  requestAccounts(): Promise<string[]>;
  getAccounts(): Promise<string[]>;
  getNetwork(): Promise<string>;
  getBalance(): Promise<{ confirmed?: string; unconfirmed?: string; total?: string } | null>;
  getPublicKey(): Promise<string>;
  disconnect?(origin: string): Promise<void>;
  on(event: string, handler: Handler): void;
  removeListener(event: string, handler: Handler): void;
}

interface KastleProvider {
  connect(): Promise<boolean>;
  getAccount(): Promise<{ address: string; publicKey: string } | null>;
  getNetwork(): Promise<string>;
  getBalance?(): Promise<{ balance: string } | null>;
  on(event: string, handler: Handler): void;
  removeListener(event: string, handler: Handler): void;
}

declare global {
  interface Window {
    kasware?: KaswareProvider;
    kastle?: KastleProvider;
  }
}

export interface NetworkInfo {
  raw: string;
  /** Netz-ID wie in ghostctl, falls zuordenbar */
  id: "mainnet" | "testnet-10" | null;
  label: string;
  isMainnet: boolean;
  isTestnet: boolean;
}

export function describeNetwork(raw: string): NetworkInfo {
  const r = raw.toLowerCase();
  const isMainnet = r === "kaspa_mainnet" || r === "mainnet";
  const tn = r.match(/testnet[_-]?(\d+)/);
  const isTestnet = !!tn;
  let label = raw || tr("unbekannt", "unknown");
  if (isMainnet) label = "Mainnet";
  else if (tn) label = `Testnet ${tn[1]}`;
  else if (r.includes("devnet")) label = "Devnet";
  const id = isMainnet ? "mainnet" : tn?.[1] === "10" ? "testnet-10" : null;
  return { raw, id, label, isMainnet, isTestnet };
}

export function installedWallets(): WalletKind[] {
  if (typeof window === "undefined") return [];
  const out: WalletKind[] = [];
  if (window.kasware) out.push("kasware");
  if (window.kastle) out.push("kastle");
  return out;
}

export const WALLET_NAMES: Record<WalletKind, string> = {
  kasware: "KasWare",
  kastle: "Kastle",
};

/** Gemeinsame, rein lesende Schnittstelle für beide Wallets. */
export interface WalletAdapter {
  kind: WalletKind;
  /** Öffnet den Freigabe-Dialog der Wallet. Nur nach Klick aufrufen. */
  connect(): Promise<string | null>;
  /** Bestehende Freigabe ohne Dialog abfragen (null, wenn keine). */
  currentAccount(): Promise<string | null>;
  network(): Promise<string>;
  balanceSompi(): Promise<bigint | null>;
  /** Öffentlicher Schlüssel (hex), falls die Wallet ihn herausgibt */
  publicKey(): Promise<string | null>;
  disconnect(): Promise<void>;
  subscribe(h: { accounts(a: string[]): void; network(n: string): void; balance(): void }): () => void;
}

const toBig = (s: string | undefined | null): bigint | null => {
  if (s === undefined || s === null || s === "") return null;
  try {
    return BigInt(s);
  } catch {
    return null;
  }
};

function kaswareAdapter(p: KaswareProvider): WalletAdapter {
  return {
    kind: "kasware",
    async connect() {
      const acc = await p.requestAccounts();
      return acc?.[0] ?? null;
    },
    async currentAccount() {
      const acc = await p.getAccounts();
      return acc?.[0] ?? null;
    },
    network: () => p.getNetwork(),
    async balanceSompi() {
      const b = await p.getBalance();
      return toBig(b?.total);
    },
    async publicKey() {
      return (await p.getPublicKey()) || null;
    },
    async disconnect() {
      await p.disconnect?.(window.location.origin);
    },
    subscribe(h) {
      const onAcc = (a: string[]) => h.accounts(Array.isArray(a) ? a : []);
      const onNet = (n: string) => h.network(String(n ?? ""));
      const onBal = () => h.balance();
      const onDisc = () => h.accounts([]);
      p.on("accountsChanged", onAcc as Handler);
      p.on("networkChanged", onNet as Handler);
      p.on("balanceChanged", onBal as Handler);
      p.on("disconnect", onDisc as Handler);
      return () => {
        p.removeListener("accountsChanged", onAcc as Handler);
        p.removeListener("networkChanged", onNet as Handler);
        p.removeListener("balanceChanged", onBal as Handler);
        p.removeListener("disconnect", onDisc as Handler);
      };
    },
  };
}

function kastleAdapter(p: KastleProvider): WalletAdapter {
  return {
    kind: "kastle",
    async connect() {
      const ok = await p.connect();
      if (!ok) throw Object.assign(new Error("Verbindung abgelehnt"), { code: 4001 });
      const acc = await p.getAccount();
      return acc?.address ?? null;
    },
    async currentAccount() {
      // Kastle dokumentiert keine dialogfreie Abfrage; wir stellen nur nach
      // einem früheren Klick wieder her und fangen Fehler ab.
      const acc = await p.getAccount();
      return acc?.address ?? null;
    },
    network: () => p.getNetwork(),
    async balanceSompi() {
      if (!p.getBalance) return null; // ältere Kastle-Versionen
      const b = await p.getBalance();
      return toBig(b?.balance);
    },
    async publicKey() {
      const acc = await p.getAccount();
      return acc?.publicKey ?? null;
    },
    async disconnect() {
      /* nicht dokumentiert – nur lokal vergessen */
    },
    subscribe(h) {
      const onAcc = (a: string[]) => h.accounts(Array.isArray(a) ? a : []);
      const onNet = (n: string) => h.network(String(n ?? ""));
      p.on("accountsChanged", onAcc as Handler);
      p.on("networkChanged", onNet as Handler);
      return () => {
        p.removeListener("accountsChanged", onAcc as Handler);
        p.removeListener("networkChanged", onNet as Handler);
      };
    },
  };
}

export function getAdapter(kind: WalletKind): WalletAdapter | null {
  if (kind === "kasware" && window.kasware) return kaswareAdapter(window.kasware);
  if (kind === "kastle" && window.kastle) return kastleAdapter(window.kastle);
  return null;
}

/** Wandelt Wallet-Fehler in verständliche Meldungen (deutsch/englisch). */
export function describeWalletError(e: unknown): string {
  const err = e as { code?: number; message?: string } | undefined;
  const msg = (err?.message ?? String(e ?? "")).toLowerCase();
  if (err?.code === 4001 || msg.includes("reject") || msg.includes("denied") || msg.includes("abgelehnt"))
    return tr(
      "Du hast die Verbindung in der Wallet abgelehnt. Kein Problem – du kannst es jederzeit erneut versuchen.",
      "You rejected the connection in the wallet. No problem – you can try again anytime.",
    );
  if (msg.includes("lock")) return tr("Die Wallet ist gesperrt. Bitte entsperre sie und versuche es erneut.", "The wallet is locked. Please unlock it and try again.");
  return tr(`Die Wallet hat einen Fehler gemeldet: ${err?.message ?? String(e)}`, `The wallet reported an error: ${err?.message ?? String(e)}`);
}
