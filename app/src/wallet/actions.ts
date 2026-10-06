// Nutzeraktionen mit Browser-Wallet (docs/wallet-aktionen.md): Logik ohne
// DOM, mit Vitest geprüft. Ablauf in der Seite (components/WalletSignFlow):
//   1. Plan holen       POST /api/wallet/build  (Adresse + Parameter, sendet nichts)
//   2. Signieren        KasWare signPskt bzw. Kastle signTx (sendet NICHT)
//   3. Prüfen           POST /api/wallet/submit (ohne send)
//   4. Senden           POST /api/wallet/submit {send:true, confirmMainnet} – nur per Knopf
// Gesendet wird nie über die Wallet (pushTx/signAndBroadcastTx werden nicht
// aufgerufen), sondern über ghostctl, das den Plan aus seinem Zustand neu baut.
import type { NetworkId } from "../config";
import type { ActionParams, CliAction } from "../lib/commands";
import type { KeyEntry } from "../lib/api";
import { getLang, tr } from "../lib/i18n";
import { addressProblem, kaswareSigner, kastleSigner, reportLines, type AttachResult, type ExportResult, type KaswareSignApi, type KastleSignApi } from "../probe/probe";
import type { WalletKind } from "./providers";

/** Aktionen, die mit der Browser-Wallet gehen (ghostctl wallet build) */
export const WALLET_ACTIONS: readonly CliAction[] = [
  "open-vault",
  "mint",
  "repay",
  "deposit",
  "withdraw",
  "close",
  "redeem",
  "liquidate",
  "sweep",
  "send",
  "transfer",
  "swap",
  "pool-add",
  "pool-remove",
];

export const walletSupports = (a: CliAction) => WALLET_ACTIONS.includes(a);

/**
 * Tresor-Aktionen der Browser-Wallet (Daueraufträge, ghostctl wallet build
 * tresor-…); Formular und Liste in components/WalletTresor.tsx
 */
export const TRESOR_WALLET_ACTIONS = ["tresor-open", "tresor-topup", "tresor-cancel"] as const;
export type TresorWalletAction = (typeof TRESOR_WALLET_ACTIONS)[number];
/** Alles, was über /api/wallet/build geht */
export type WalletActionName = CliAction | TresorWalletAction;

/** Erlaubte Parameter je Aktion (wie server/walletActions.ts) */
const ALLOWED: Record<string, string[]> = {
  "open-vault": ["kas"],
  mint: ["vault", "ghost"],
  repay: ["vault", "ghost"],
  deposit: ["vault", "kas"],
  withdraw: ["vault", "keep"],
  close: ["vault"],
  redeem: ["vault", "ghost"],
  liquidate: ["vault", "ghost"],
  sweep: ["vault"],
  send: ["to", "kas", "message", "onchain"],
  transfer: ["to", "ghost", "message", "onchain"],
  swap: ["kas", "ghost", "min"],
  "pool-add": ["kas", "ghost", "minShares"],
  "pool-remove": ["percent", "minKas", "minGhost"],
  "tresor-open": ["to", "amount", "interval", "start", "count", "fund", "maxFee", "message"],
  "tresor-topup": ["tresor", "kas"],
  "tresor-cancel": ["tresor"],
};

/** Erlaubte Parameter einer Wallet-Aktion (wie server/walletActions.ts WALLET_PARAMS) */
export const walletParamsOf = (a: WalletActionName): readonly string[] => ALLOWED[a] ?? [];

/**
 * Formular-Parameter (wie für /api/action) → Parameter für /api/wallet/build.
 * Schlüsseldateien gibt es hier nicht: `key` fällt weg, ein Empfänger als
 * Schlüsseldatei wird abgelehnt.
 */
/**
 * `vaultId`: Vault-Nummer → Covenant-ID. Mit der Wallet wird der Vault über
 * seine feste ID angesprochen, nicht über die Nummer, die sich verschiebt,
 * wenn ein Vault mit kleinerer Nummer endet (A11-O-15).
 */
export function toWalletParams(action: CliAction, p: ActionParams, vaultId?: (index: number) => string | undefined): { params: ActionParams | null; problem: string | null } {
  if (!walletSupports(action)) return { params: null, problem: tr("Diese Aktion geht nur mit Schlüsseldatei.", "This action needs a key file.") };
  const out: ActionParams = {};
  for (const [k, v] of Object.entries(p)) {
    if (k === "key") continue;
    if (!ALLOWED[action].includes(k)) continue;
    if (k === "to" && typeof v === "string" && v.startsWith("keys/"))
      return { params: null, problem: tr("Mit der Browser-Wallet als Empfänger eine Adresse angeben (keine Schlüsseldatei).", "With the browser wallet, enter an address as recipient (no key file).") };
    if (k === "vault" && vaultId && typeof v === "number") {
      const id = vaultId(v);
      if (!id) return { params: null, problem: tr("Diesen Vault gibt es nicht (mehr) – Seite neu laden.", "This vault no longer exists – reload the page.") };
      out[k] = id;
      continue;
    }
    out[k] = v;
  }
  return { params: out, problem: null };
}

/**
 * Konto der Browser-Wallet als Eintrag für die Vorprüfung (precheck). GHOST
 * kennt die Seite nur aus dem Zustand; unbekannt = keine Warnung, ghostctl
 * prüft beim Bauen.
 */
export function walletKeyEntry(address: string, xonly: string | null, balanceSompi: bigint | null, ghost: number | null, vaults: number[]): KeyEntry {
  return {
    file: "wallet",
    type: "key",
    xonly: xonly ?? "",
    address,
    kas: balanceSompi === null ? null : Number(balanceSompi) / 1e8,
    ghost: ghost ?? Number.POSITIVE_INFINITY,
    vaults,
  };
}

/**
 * Art eines Ausgangs (A20d-9). ghostctl liefert bisher nur den deutschen Text
 * `what`; ein stabiles Feld `kind` ("other" | "self" | "covenant") wertet die
 * Seite aus, sobald es da ist (protocol: wallet_ops.rs describe_with).
 */
export type OutputKind = "other" | "self" | "covenant";
export type PlanOutput = ExportResult["outputs"][number] & { kind?: OutputKind };

export interface BuildResult {
  ok: boolean;
  error?: string;
  nodeDown?: boolean;
  detail?: string;
  action?: string;
  network?: NetworkId;
  address?: string;
  feeSompi?: number;
  outputs?: PlanOutput[];
  signInputs?: ExportResult["signInputs"];
  kastle?: ExportResult["kastle"];
  kasware?: ExportResult["kasware"];
  info?: Record<string, unknown>;
  plan?: { kind: string; network: string; address: string; [k: string]: unknown };
}

export type SubmitResult = AttachResult & {
  action?: string;
  info?: Record<string, unknown>;
  transactions?: unknown[];
  nodeDown?: boolean;
  unclear?: boolean;
  timeout?: boolean;
  /** gesendet, Bestätigung stand bei der Antwort noch aus (ghostctl wartet höchstens 45 s) */
  pending?: boolean;
  note?: string;
  busy?: boolean;
};

/** Wie das Ergebnis eines Sendens anzuzeigen ist */
export function sendOutcome(r: SubmitResult): "confirmed" | "pending" | "unclear" | "failed" {
  if (r.ok && r.sent) return r.confirmed === false || r.pending ? "pending" : "confirmed";
  if (r.unclear || r.timeout) return "unclear";
  return "failed";
}

/**
 * Bleibt die geprüfte Signatur nach einem Sendeversuch stehen? Nur wenn
 * sicher nichts gesendet wurde und ein neuer Versuch gleich gelingen kann
 * (`busy`: Seite ausgelastet oder Sperre belegt, Audit 19 A19-7).
 */
export function keepSigned(r: SubmitResult): boolean {
  return r.busy === true && !r.sent && !r.unclear && !r.timeout;
}

export function buildBody(network: NetworkId, action: WalletActionName, address: string, params: ActionParams) {
  return { network, action, address, params };
}

/** Prüfen (send=false) oder senden (send=true, nur vom Knopf „Senden“) */
export function submitBody(network: NetworkId, plan: BuildResult["plan"], signed: string, send: boolean) {
  if (!plan) throw new Error(tr("Kein Plan.", "No plan."));
  const b: Record<string, unknown> = { network, plan, signed };
  if (send) {
    b.send = true;
    if (network === "mainnet") b.confirmMainnet = true;
  }
  return b;
}

/**
 * Passt der Plan zu dem, was der Nutzer will? Der Server baut ihn aus Adresse
 * und Parametern; trotzdem prüft die Seite vor dem Signieren, dass Netz und
 * Adresse stimmen und dass jeder zu signierende Eingang genannt ist.
 */
export function planProblem(b: BuildResult, network: NetworkId, address: string): string | null {
  if (!b.ok) return b.error ?? tr("Plan fehlgeschlagen.", "Plan failed.");
  if (!b.plan || b.plan.kind !== "ghost-wallet-action:1") return tr("Antwort ohne Signierplan.", "Response without signing plan.");
  if (b.plan.network !== network) return tr("Plan gehört zu einem anderen Netz.", "Plan belongs to another network.");
  if (b.plan.address !== address) return tr("Plan gehört zu einer anderen Adresse.", "Plan belongs to another address.");
  if (!b.kasware || !b.kastle || !b.signInputs?.length) return tr("Plan ohne Signier-Anfrage.", "Plan without signing request.");
  return null;
}

/** Ausgänge an fremde Adressen (zur Anzeige vor dem Signieren) */
export function foreignOutputs(b: BuildResult, address: string): NonNullable<BuildResult["outputs"]> {
  return (b.outputs ?? []).filter((o) => o.address !== address && !o.what.startsWith("Vertrag"));
}

/** Art eines Ausgangs: `kind` von ghostctl, sonst aus dem deutschen Text (A20d-9) */
export function outputKind(o: PlanOutput, address: string): OutputKind {
  if (o.kind === "other" || o.kind === "self" || o.kind === "covenant") return o.kind;
  if (o.what === "andere Adresse") return "other";
  if (o.address === address || /^(Wechselgeld an die Wallet|an die Wallet|Rest des Tresors zurück an die Wallet)$/.test(o.what)) return "self";
  return "covenant";
}

/** Zahlungen an fremde P2PK-Adressen – stehen vor dem Signieren immer offen da */
export function payeeOutputs(b: BuildResult, address: string): PlanOutput[] {
  return (b.outputs ?? []).filter((o) => o.address !== address && outputKind(o, address) === "other");
}

/** englische Fassung der Ausgangstexte von ghostctl (A20d-9); unbekannte bleiben deutsch */
const WHAT_EN: [RegExp, string][] = [
  [/^andere Adresse$/, "other address"],
  [/^Wechselgeld an die Wallet$/, "change to the wallet"],
  [/^an die Wallet$/, "to the wallet"],
  [/^Rest des Tresors zurück an die Wallet$/, "rest of the vault back to the wallet"],
  [/^Vertrag \(Covenant\)$/, "contract (covenant)"],
  [/^GHOST-Wurzel \(läuft weiter\)$/, "GHOST root (continues)"],
  [/^Minter-Zweig deines Vaults (\S+) \(läuft weiter\)$/, "minter branch of your vault $1 (continues)"],
  [/^Minter-Zweig des fremden Vaults (\S+) \(läuft weiter\)$/, "minter branch of the other vault $1 (continues)"],
  [/^Minter-Zweig des neuen Vaults – (.+), bleiben dauerhaft gebunden$/, "minter branch of the new vault – $1, stays locked permanently"],
  [/^GHOST-Token – (.+) stecken darin und kommen beim Weitergeben bzw\. Tilgen zurück$/, "GHOST token – holds $1, which comes back when passing it on or repaying"],
  [/^Dein neuer Vault – Sicherheit (.+)$/, "your new vault – collateral $1"],
  [/^Dein neuer Tresor – (.+?), zahlt (.+) an (\S+)$/, "your new vault – $1, pays $2 to $3"],
  [/^Dein Tresor (\S+) – (.+?), zahlt (.+) an (\S+)$/, "your vault $1 – $2, pays $3 to $4"],
  [/^Tresor (\S+) \(läuft weiter\)$/, "vault $1 (continues)"],
];
export function outputWhat(what: string): string {
  if (getLang() !== "en") return what;
  for (const [re, en] of WHAT_EN) if (re.test(what)) return what.replace(re, en);
  return what;
}

const norm = (s: unknown) => (typeof s === "string" ? s.trim().toLowerCase() : null);

/**
 * Empfänger laut Plan (ghostctl wallet build: plan.action.to) für send,
 * transfer und tresor-open; sonst null (A20d-8).
 */
export function planRecipient(b: BuildResult): string | null {
  const a = b.plan?.action as { to?: unknown } | undefined;
  return a && typeof a.to === "string" ? a.to.trim() : null;
}

/**
 * Passt der Plan zu dem, was der Nutzer eingegeben hat (A20d-8, A20d-2)?
 * - send/transfer/tresor-open: plan.action.to muss params.to sein; beim
 *   Senden muss eine Zahlung genau an diese Adresse unter den Ausgängen
 *   stehen, beim Tresor der neue Tresor diese Adresse nennen.
 * - Aktionen an einem Vault: Die Nummer im Plan (plan.action.vault, Nummer
 *   der Zustandsdatei des Servers) muss laut Status zur gewählten
 *   Covenant-ID gehören. Weicht etwas ab, wird nicht signiert.
 * `vaultIdOf`: Nummer → Covenant-ID aus dem Status (unbekannt: undefined).
 */
export function planMismatch(b: BuildResult, action: WalletActionName, params: ActionParams, address: string, vaultIdOf?: (index: number) => string | undefined): string | null {
  if (!b.ok || !b.plan) return null;
  if (action === "send" || action === "transfer" || action === "tresor-open") {
    const want = norm(params.to);
    const got = norm(planRecipient(b));
    if (!got) return tr("Der Plan nennt keinen Empfänger – nicht signieren, bitte erneut prüfen.", "The plan names no recipient – do not sign, check again.");
    if (want !== got) return tr(`Empfänger im Plan (${planRecipient(b)}) weicht von deiner Eingabe ab – nicht signieren.`, `Recipient in the plan (${planRecipient(b)}) differs from your input – do not sign.`);
    if (action === "send" && !payeeOutputs(b, address).some((o) => norm(o.address) === want))
      return tr("Keine Zahlung an den Empfänger unter den Ausgängen – nicht signieren.", "No payment to the recipient among the outputs – do not sign.");
    if (action === "tresor-open" && !(b.outputs ?? []).some((o) => o.what.toLowerCase().includes(want!)))
      return tr("Der neue Tresor im Plan nennt einen anderen Empfänger – nicht signieren.", "The new vault in the plan names another recipient – do not sign.");
  }
  if (typeof params.vault === "string" && vaultIdOf) {
    const idx = (b.plan.action as { vault?: unknown } | undefined)?.vault;
    if (typeof idx !== "number") return tr("Der Plan nennt keinen Vault – nicht signieren.", "The plan names no vault – do not sign.");
    const id = vaultIdOf(idx);
    if (id === undefined) return tr("Der Vault-Stand hat sich geändert – Status wird neu geladen, dann erneut prüfen.", "The vault state changed – status is reloading, then check again.");
    if (id.toLowerCase() !== params.vault.toLowerCase()) return tr("Der Plan betrifft einen anderen Vault als gewählt – nicht signieren, erneut prüfen.", "The plan concerns a different vault than selected – do not sign, check again.");
  }
  return null;
}

/**
 * Unklar-Sperre im Wallet-Ablauf (A20d-1) wie im Schlüsselmodus (A10-W-2):
 * nach einem Senden mit unklarem Ausgang bleiben Prüfen, Signieren und
 * Senden gesperrt, bis ein Status geladen ist, der NACH dem Vorfall begann.
 */
export function unclearLocked(unclearAt: number | null, updatedAt: number | null): boolean {
  return unclearAt !== null && (updatedAt ?? 0) <= unclearAt;
}

/** Signieren in der Wallet; Rückgabe: signierte Tx als JSON-Text. Sendet nicht. */
export async function signInWallet(kind: WalletKind, win: { kasware?: unknown; kastle?: unknown }, b: BuildResult): Promise<string> {
  const e = b as unknown as ExportResult;
  if (kind === "kasware") {
    if (!win.kasware) throw new Error(tr("KasWare nicht gefunden.", "KasWare not found."));
    return kaswareSigner(win.kasware as KaswareSignApi).sign(e);
  }
  if (!win.kastle) throw new Error(tr("Kastle nicht gefunden.", "Kastle not found."));
  return kastleSigner(win.kastle as KastleSignApi).sign(e);
}

/** Darf „Senden“ erscheinen? */
export function canSendSigned(r: SubmitResult | null): boolean {
  return !!r && r.ok === true && r.valid === true && r.sent !== true;
}

export { addressProblem, reportLines };

export type Fetch = (url: string, init: { method: string; headers: Record<string, string>; body: string; cache?: RequestCache }) => Promise<{ status: number; json(): Promise<unknown> }>;

/** Meldung, wenn beim Senden offen ist, ob die Tx hinausging (A17-7) */
export const unclearSend = () =>
  tr(
  "Ergebnis unklar: Die Verbindung zum Server brach ab oder er antwortete nicht rechtzeitig. Die Transaktion kann trotzdem gesendet worden sein – bitte erst den Status bzw. den Explorer prüfen und NICHT sofort erneut senden.",
  "Outcome unclear: the connection to the server broke or it did not answer in time. The transaction may still have been sent – check the status or the explorer first and do NOT send again right away.",
);

/**
 * POST an /api/wallet/<route>; HTTP-Fehler (400, 403, 413, 429, 503) als
 * {ok:false}. Beim Senden (`sending`) heißt eine Antwort ohne JSON, ein 5xx
 * vom Webserver (z. B. 502/504 nach dessen Zeitlimit) oder ein Netzfehler
 * NICHT „nicht gesendet“, sondern „unklar“ (A17-7). Ausnahme: 503 mit JSON
 * (busy) kommt von der Seite selbst, bevor ghostctl startet.
 */
export async function callWalletApi(fetchFn: Fetch, route: "build" | "submit", body: unknown, sending = false): Promise<Record<string, unknown>> {
  let r: { status: number; json(): Promise<unknown> };
  try {
    r = await fetchFn(`./api/wallet/${route}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Ghost-Client": "1" },
      body: JSON.stringify(body),
      cache: "no-store",
    });
  } catch (e) {
    if (sending) return { ok: false, unclear: true, error: unclearSend() };
    throw e;
  }
  let j: unknown;
  try {
    j = await r.json();
  } catch {
    j = null;
  }
  if (!j || typeof j !== "object") {
    if (sending) return { ok: false, unclear: true, error: `${unclearSend()} (HTTP ${r.status})` };
    return { ok: false, error: tr(`Unerwartete Antwort (${r.status}).`, `Unexpected response (${r.status}).`) };
  }
  const o = j as Record<string, unknown>;
  if (sending && r.status >= 500 && o.busy !== true && o.ok !== true) return { ...o, ok: false, unclear: true, error: `${unclearSend()} (HTTP ${r.status})` };
  return o;
}
