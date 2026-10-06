// .k-Namen (dotk.name) → Kaspa-Adresse, für die Empfängerfelder der Seite.
//
// Die Registry-API von dotk.name nennt zu einem Namen den Besitzer und die
// Adresse seiner Urkunde (deed, ein Covenant-UTXO). @dotk/sdk leitet diese
// Adresse aus Name und Besitzer selbst ab und prüft an UNSEREM Node (über
// ghostctl utxos), dass dort genau ein UTXO des Registers liegt. Erst dann gilt
// die Adresse als bewiesen (`proven`); ohne Beweis oder bei Widerspruch gibt es
// keine Adresse (RefutedError). Subnamen (bob.alice.k) sind nur Einträge des
// Eigentümers, nicht auf der Kette bewiesen – daher abgelehnt.
// Recherche 06.10.2026: api.dotk.name, github.com/supertypo/dotk-sdk (MIT).
import { Dotk, RefutedError, type Node, type Utxo } from "@dotk/sdk";
import { ValidationError, type Network } from "./actions.ts";

/** UTXOs an Adressen vom eigenen Node (ghostctl utxos) */
export type UtxoSource = (network: Network, addresses: string[]) => Promise<Utxo[]>;

export interface NameAnswer {
  ok: boolean;
  /** so wie Leser ihn sehen, z. B. „alice.k“ */
  display?: string;
  address?: string;
  /** am eigenen Node nachgeprüft (immer true, wenn address gesetzt ist) */
  proven?: boolean;
  error?: string;
  /** Name nicht vergeben bzw. ohne zahlbare Adresse */
  notFound?: boolean;
}

const clients = new Map<Network, Dotk>();

function client(network: Network, utxos: UtxoSource, fetchImpl?: typeof fetch): Dotk {
  const node: Node = { getUtxosByAddresses: (addresses) => utxos(network, addresses) };
  if (fetchImpl) return new Dotk({ network, node, fetch: fetchImpl, timeoutMs: 15_000 });
  let c = clients.get(network);
  if (!c) {
    c = new Dotk({ network, node, timeoutMs: 15_000 });
    clients.set(network, c);
  }
  return c;
}

/** Sieht die Eingabe wie ein .k-Name aus (und nicht wie Adresse oder x-only-Schlüssel)? */
export function looksLikeName(s: string): boolean {
  const t = s.trim().toLowerCase();
  if (t.length === 0 || t.length > 80 || t.includes(":")) return false;
  if (/^[0-9a-f]{64}$/.test(t)) return false;
  return /^[a-z0-9][a-z0-9.\-_]*$/.test(t);
}

export async function resolveName(network: Network, input: unknown, utxos: UtxoSource, fetchImpl?: typeof fetch): Promise<NameAnswer> {
  if (typeof input !== "string" || !looksLikeName(input)) throw new ValidationError("Name: z. B. alice.k erwartet.");
  const d = client(network, utxos, fetchImpl);
  const c = d.classify(input);
  if (c.kind === "subname") return { ok: false, error: "Unternamen (z. B. bob.alice.k) sind nicht auf der Kette bewiesen und werden hier nicht aufgelöst." };
  if (c.kind !== "name") return { ok: false, error: "Kein gültiger .k-Name." };
  try {
    const r = await d.recipientFor(input);
    if (r.kind !== "name") return { ok: false, error: "Kein gültiger .k-Name." };
    if (!r.address) {
      return r.fault === "in-covenant"
        ? { ok: false, display: r.display, error: `${r.display} gehört einem Vertrag und hat keine Adresse.` }
        : { ok: false, display: r.display, notFound: true, error: `${r.display} ist nicht vergeben.` };
    }
    if (r.proven !== true) return { ok: false, display: r.display, error: `${r.display} ließ sich am Node nicht nachprüfen – bitte die Adresse direkt eingeben.` };
    return { ok: true, display: r.display, address: r.address, proven: true };
  } catch (e) {
    if (e instanceof RefutedError) return { ok: false, error: "Der Node widerspricht der Namens-Auskunft – nicht verwenden, Adresse direkt eingeben." };
    return { ok: false, error: `Namensdienst gerade nicht erreichbar (${(e as Error).name}). Bitte die Adresse direkt eingeben.` };
  }
}
