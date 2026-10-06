// Aufrufe der lokalen API (server/api.ts). Alle schreibenden Aufrufe tragen
// X-Ghost-Client: 1 und JSON – ohne das lehnt der Server ab.
import type { NetworkId } from "../config";
import type { AboList } from "./abo";
import type { TresorList } from "./tresor";
import { tr } from "./i18n";

export interface KeyEntry {
  file: string;
  type: "key";
  xonly: string;
  address: string;
  /** null = kein Node erreichbar, Guthaben unbekannt */
  kas: number | null;
  ghost: number;
  vaults: number[];
  /** Pool-Anteile dieses Schlüssels (Dezimalstring) */
  lpShares?: string;
}
export interface CommitteeEntry {
  file: string;
  type: "committee";
  signers: number;
}
export type KeyListEntry = KeyEntry | CommitteeEntry;

export interface TxInfo {
  action: string;
  txid: string;
  feeKas: number;
  inputs: number;
  outputs: number;
  sent: boolean;
  confirmed: boolean;
  /** Version 2: kleiner Rest, der als Gebühr an die Miner ging (lohnte keinen eigenen Ausgang) */
  donatedKas?: number;
  /** send/transfer: Nachricht und ob sie öffentlich bzw. verschlüsselt in der Tx steht */
  message?: string;
  onchain?: boolean;
  encrypted?: boolean;
}

export interface ActionResult {
  ok: boolean;
  error?: string;
  network?: NetworkId;
  dryRun?: boolean;
  transactions?: TxInfo[];
  vault?: number;
  /** öffentliche Kaspa-Nodes nicht erreichbar */
  nodeDown?: boolean;
  /** technische Originalmeldung */
  detail?: string;
  timeout?: boolean;
  /** Senden abgebrochen oder ohne Antwort: ob etwas hinausging, ist unklar */
  unclear?: boolean;
  [k: string]: unknown;
}

export class ApiError extends Error {
  status: number;
  nodeDown: boolean;
  constructor(message: string, status: number, nodeDown = false) {
    super(message);
    this.status = status;
    this.nodeDown = nodeDown;
  }
}

async function parse(res: Response): Promise<Record<string, unknown>> {
  const text = await res.text();
  try {
    return JSON.parse(text) as Record<string, unknown>;
  } catch {
    if (res.status === 404)
      throw new ApiError(tr("Keine lokale API: Die Seite muss über den Doppelklick-Starter bzw. „npm run dev“ laufen.", "No local API: the page must run via the double-click starter or “npm run dev”."), 404);
    throw new ApiError(tr(`Unerwartete Antwort (${res.status}).`, `Unexpected response (${res.status}).`), res.status);
  }
}

export async function fetchKeys(network: NetworkId, signal?: AbortSignal): Promise<{ keys: KeyListEntry[]; public: boolean }> {
  const res = await fetch(`./api/keys?network=${encodeURIComponent(network)}`, { signal, cache: "no-store" });
  const j = await parse(res);
  if (!res.ok || j.ok === false) throw new ApiError(String(j.error ?? tr(`Fehler ${res.status}`, `Error ${res.status}`)), res.status, j.nodeDown === true);
  return { keys: (j.keys as KeyListEntry[]) ?? [], public: j.public === true };
}

async function post(path: string, body: unknown): Promise<Record<string, unknown>> {
  const res = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Ghost-Client": "1" },
    body: JSON.stringify(body),
    cache: "no-store",
  });
  const j = await parse(res);
  if (!res.ok) throw new ApiError(String(j.error ?? tr(`Fehler ${res.status}`, `Error ${res.status}`)), res.status);
  return j;
}

export async function runAction(req: {
  network: NetworkId;
  action: string;
  params: Record<string, string | number | boolean>;
  dryRun: boolean;
  confirmMainnet?: boolean;
}): Promise<ActionResult> {
  return (await post("./api/action", req)) as ActionResult;
}

export async function keygen(network: NetworkId, name: string): Promise<{ ok: boolean; file?: string; xonly?: string; error?: string }> {
  return (await post("./api/keygen", { network, name })) as { ok: boolean; file?: string; xonly?: string; error?: string };
}

export interface ReceiveResult {
  ok: boolean;
  /** neu gefundene Token-UTXOs mit genau diesem Betrag */
  found?: number;
  /** schon bekannte */
  known?: number;
  error?: string;
  nodeDown?: boolean;
}

/** Sucht eingegangene GHOST mit genau diesem Betrag (Punkt-Dezimal) und merkt sie sich. Sendet nichts. */
export async function receiveGhost(network: NetworkId, key: string, ghost: string): Promise<ReceiveResult> {
  return (await post("./api/receive", { network, key, ghost })) as unknown as ReceiveResult;
}

/** Wie receiveGhost, aber für die Adresse der Browser-Wallet (öffentliche Seite) */
export async function receiveGhostWallet(network: NetworkId, address: string, ghost: string): Promise<ReceiveResult> {
  return (await post("./api/wallet/receive", { network, address, ghost })) as unknown as ReceiveResult;
}

/** Eine eingegangene Nachricht (ghostctl messages) */
export interface InboxMessage {
  txid: string;
  timeMs: number | null;
  /** lokale Zeit „JJJJ-MM-TT hh:mm“ */
  at: string | null;
  amount: string;
  unit: "KAS" | "GHOST";
  /** Schlüssel-Adressen unter den Eingängen der Zahlung (nie der Besitzer eines Tresors, nie die eigene Adresse) */
  from: string[];
  /** null bei „unreadable“ und „invalid“ */
  text: string | null;
  /** encrypted = an diesen Schlüssel verschlüsselt und entschlüsselt; public = Klartext für alle; unreadable = verschlüsselt, aber nicht für diesen Schlüssel; invalid = Text zu lang oder mit unzulässigen Zeichen, nicht angezeigt */
  kind: "encrypted" | "public" | "unreadable" | "invalid";
  /** bei „invalid“ der Grund: length = über 100 Zeichen, chars = unzulässige Zeichen, both = beides (fehlt bei älteren ghostctl) */
  invalid?: "length" | "chars" | "both" | null;
  /**
   * Herkunft (fehlt bei älteren ghostctl): direct = Zahlung von Schlüssel-Adressen;
   * Tresor-Zahlungen tragen die beim Anlegen hinterlegte Nachricht, der Vertrag
   * erzwingt sie (payloadHash): tresor-stored = Tresor hier übernommen, Text wie
   * im Tresor-Code; tresor-bound = Tresor hier nicht übernommen oder sein Code
   * beschreibt die Nachricht anders; tresor-inserted = passt nicht zum Hash im
   * Vertrag (nur bei falschen Daten der REST-API möglich); contract = aus einem
   * anderen Vertrag
   */
  origin?: "direct" | "tresor-stored" | "tresor-bound" | "tresor-inserted" | "contract";
  /** ID des Tresors dieses Rechners */
  tresor?: string | null;
  /** Besitzer laut übernommenem Tresor-Code (nur bei Tresoren dieses Rechners; vom Vertrag nicht geprüft) */
  tresorOwner?: string | null;
  /** node = am Node im Block gegengeprüft; rest = nur laut REST-API */
  source?: "node" | "rest";
}

export interface InboxResult {
  ok: boolean;
  address?: string;
  limit?: number;
  messages?: InboxMessage[];
  /** GHOST-Eingänge, die sich nicht prüfen ließen */
  notes?: string[];
  /** Hinweise zur Prüfung der Nachrichten (Node, Tresore) */
  checks?: string[];
  /** ein Node war erreichbar und hat nachgeprüft */
  nodeChecked?: boolean;
  /** ausgeblendet, weil die REST-API vom Block am Node abwich */
  hidden?: number;
  error?: string;
}

/** Eingegangene Nachrichten an die Adresse dieses Schlüssels (liest nur, sendet nichts) */
export async function fetchMessages(network: NetworkId, key: string, signal?: AbortSignal): Promise<InboxResult> {
  const res = await fetch(`./api/messages?network=${encodeURIComponent(network)}&key=${encodeURIComponent(key)}`, { signal, cache: "no-store" });
  const j = await parse(res);
  if (!res.ok || j.ok === false) throw new ApiError(String(j.error ?? tr(`Fehler ${res.status}`, `Error ${res.status}`)), res.status, j.nodeDown === true);
  return j as unknown as InboxResult;
}

/** Daueraufträge dieses Netzes (ghostctl abo list, ohne Node) */
export async function fetchAbos(network: NetworkId, signal?: AbortSignal): Promise<AboList> {
  const res = await fetch(`./api/abos?network=${encodeURIComponent(network)}`, { signal, cache: "no-store" });
  const j = await parse(res);
  if (!res.ok) throw new ApiError(String(j.error ?? tr(`Fehler ${res.status}`, `Error ${res.status}`)), res.status);
  return j as unknown as AboList;
}

/** Tresore dieses Netzes (ghostctl tresor list, ohne Node) */
export async function fetchTresore(network: NetworkId, signal?: AbortSignal): Promise<TresorList> {
  const res = await fetch(`./api/tresore?network=${encodeURIComponent(network)}`, { signal, cache: "no-store" });
  const j = await parse(res);
  if (!res.ok) throw new ApiError(String(j.error ?? tr(`Fehler ${res.status}`, `Error ${res.status}`)), res.status);
  return j as unknown as TresorList;
}
