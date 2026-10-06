// Daueraufträge mit Tresor über die Browser-Wallet (öffentliche Seite):
// Formular → Parameter für /api/wallet/build (tresor-open, tresor-topup,
// tresor-cancel), Liste „Meine Tresore“ (GET /api/wallet/tresore) und deren
// Anzeige. Logik ohne DOM, mit Vitest geprüft. Verbindlich ist, was ghostctl
// beim Bauen sagt (protocol/src/wallet_ops.rs, protocol/src/tresor.rs).
import type { NetworkId } from "../config";
import { isDate, messageProblem } from "./abo";
import { cliDecimal } from "./commands";
import { parseUnits } from "./format";
import { tr } from "./i18n";
import type { Hint } from "./precheck";
import { contractPayable, coveredPayments, effectiveFund, feeSource, formCount, minFund, TRESOR_MIN_AMOUNT, type Tresor, type TresorForm, type TresorStatus } from "./tresor";

/** Schnorr-Adresse des Netzes (wie server/walletProbe.ts) */
const ADDRESS_RE = /^(kaspa|kaspatest):q[qpzry9x8gf2tvdw0s3jn54khce6mua7l]{60}$/;
/** volle Covenant-ID eines Tresors */
const COVENANT_ID_RE = /^[0-9a-f]{64}$/;

/** Ein Eintrag aus GET /api/wallet/tresore (ghostctl tresor owned): ohne Pfad der Schlüsseldatei */
export type WalletTresor = Omit<Tresor, "key" | "lastError"> & {
  /** über die Browser-Wallet angelegt */
  wallet?: boolean;
  /** trägt der Tresor die Netzgebühr der nächsten Zahlung selbst (tresor::fee_from_tresor)? */
  feeFromTresor?: boolean;
  valueSompi?: number;
  /** gesendet, aber noch nicht bestätigt bzw. übernommen (offenes Journal, A19-6) */
  pending?: { txid: string; action: string };
};

/**
 * Erster Termin höchstens so viele Tage nach heute (UTC). ghostctl lässt
 * 366 Tage ab der Past Median Time zu (tresor::MAX_FIRST_DUE_AHEAD_MS); ein
 * Tag Abstand, weil die Past Median Time der Uhr ein paar Minuten nachläuft.
 */
export const MAX_FIRST_DUE_DAYS = 365;

/**
 * Heutiges Datum in UTC (JJJJ-MM-TT). Termine sind 00:00 UTC, und ghostctl
 * prüft den ersten Termin gegen die Zeit des Netzes in UTC (A19-8). Mit dem
 * Datum der Ortszeit ließ die Seite westlich von UTC abends „heute“ zu, das
 * der Server als vergangen ablehnte.
 */
export function utcToday(now = new Date()): string {
  return now.toISOString().slice(0, 10);
}

/** Spätester erster Termin zu `today` (UTC-Datum) */
export function lastFirstDue(today: string): string {
  const t = Date.parse(`${today}T00:00:00Z`);
  return Number.isNaN(t) ? today : new Date(t + MAX_FIRST_DUE_DAYS * 86_400_000).toISOString().slice(0, 10);
}

export interface WalletTresorList {
  ok: boolean;
  error?: string;
  owner?: string;
  tresore?: WalletTresor[];
}

/** „Meine Tresore“ laden (nur lesend; HTTP-Fehler als {ok:false}) */
export async function fetchWalletTresore(network: NetworkId, owner: string, signal?: AbortSignal): Promise<WalletTresorList> {
  const q = new URLSearchParams({ network, owner });
  const r = await fetch(`./api/wallet/tresore?${q.toString()}`, { cache: "no-store", signal });
  let j: unknown = null;
  try {
    j = await r.json();
  } catch {
    j = null;
  }
  if (!j || typeof j !== "object") return { ok: false, error: tr(`Unerwartete Antwort (${r.status}).`, `Unexpected response (${r.status}).`) };
  return j as WalletTresorList;
}

/**
 * Parameter für /api/wallet/build (tresor-open) oder ein Problem. Empfänger
 * nur als Schnorr-Adresse dieses Netzes, nicht die eigene. Die Nachricht ist
 * mit der Wallet immer öffentlich (`onchain` des Formulars zählt hier nicht).
 */
export function walletTresorParams(f: TresorForm, address: string | null, network: NetworkId, today: string): { params: Record<string, string | number> | null; problem: string | null } {
  const fail = (p: string) => ({ params: null, problem: p });
  if (!address) return fail(tr("Erst die Wallet verbinden.", "Connect your wallet first."));
  const to = f.to.trim();
  if (!to) return fail(tr("Empfänger angeben.", "Enter a recipient."));
  const m = to.match(ADDRESS_RE);
  if (!m) return fail(tr("Empfänger: Kaspa-Adresse (kaspa:q…) erwartet.", "Recipient: Kaspa address (kaspa:q…) expected."));
  if (m[1] !== (network === "mainnet" ? "kaspa" : "kaspatest")) return fail(tr("Empfänger: Adresse gehört zu einem anderen Netz.", "Recipient: the address belongs to another network."));
  if (to === address) return fail(tr("Empfänger ist deine eigene Adresse.", "The recipient is your own address."));
  if (f.amountText.trim() === "") return fail(tr("Betrag eingeben.", "Enter an amount."));
  if (f.amount === null || f.amount <= 0n) return fail(tr("Betrag: ungültige Zahl.", "Amount: invalid number."));
  if (f.amount < TRESOR_MIN_AMOUNT) return fail(tr("Mindestens 1 KAS je Zahlung.", "At least 1 KAS per payment."));
  if (f.interval === null) return fail(tr("Intervall: 1 bis 3650 Tage.", "Interval: 1 to 3650 days."));
  if (!isDate(f.start)) return fail(tr("Erster Termin: gültiges Datum wählen.", "First date: choose a valid date."));
  if (f.start < today) return fail(tr("Erster Termin liegt in der Vergangenheit (Datum in UTC).", "The first date is in the past (date in UTC)."));
  if (f.start > lastFirstDue(today)) return fail(tr("Erster Termin: höchstens ein Jahr im Voraus.", "First date: at most one year ahead."));
  const count = formCount(f);
  if (f.endMode === "count" && count === null) return fail(tr("Anzahl: ganze Zahl von 1 bis 9999.", "Count: whole number from 1 to 9999."));
  const fund = effectiveFund(f);
  if (fund === null)
    return fail(f.endMode === "none" ? tr("Unbegrenzt: Startguthaben angeben.", "Unlimited: enter the starting balance.") : tr("Startguthaben: ungültige Zahl.", "Starting balance: invalid number."));
  if (fund < minFund(f.amount))
    return fail(
      tr(
        `Startguthaben: mindestens ${cliDecimal(minFund(f.amount))} KAS (eine Zahlung, Gebühr und 1 KAS Rest).`,
        `Starting balance: at least ${cliDecimal(minFund(f.amount))} KAS (one payment, fee and 1 KAS remainder).`,
      ),
    );
  const mp = messageProblem(f.message);
  if (mp) return fail(mp);
  const p: Record<string, string | number> = {
    to,
    amount: cliDecimal(f.amount),
    interval: typeof f.interval === "string" ? f.interval : String(f.interval.days),
    start: f.start,
    fund: cliDecimal(fund),
  };
  if (count !== null) p.count = count;
  if (f.message.trim()) p.message = f.message.trim();
  return { params: p, problem: null };
}

/** Auffüllen: Parameter oder null (Betrag ungültig) */
export function walletTopupParams(t: Pick<WalletTresor, "covenantId">, kasText: string): Record<string, string> | null {
  const u = kasText.trim() ? parseUnits(kasText, 8) : null;
  if (u === null || u <= 0n || !COVENANT_ID_RE.test(t.covenantId)) return null;
  return { tresor: t.covenantId, kas: cliDecimal(u) };
}

/** Kündigen: Parameter oder null */
export function walletCancelParams(t: Pick<WalletTresor, "covenantId">): Record<string, string> | null {
  return COVENANT_ID_RE.test(t.covenantId) ? { tresor: t.covenantId } : null;
}

/** Was jeder wissen muss, bevor er einen Tresor mit der Wallet anlegt */
export function walletTresorBasics(): string[] {
  return [
    tr(
      "Das Geld liegt in einem Vertrag auf der Kaspa-Blockchain, nicht bei K.Lend. K.Lend kann es weder nehmen noch umleiten.",
      "The money sits in a contract on the Kaspa blockchain, not with K.Lend. K.Lend can neither take nor redirect it.",
    ),
    tr(
      "Nur du – der Besitzer, also diese Wallet-Adresse – kannst auffüllen oder kündigen. Beim Kündigen kommt der Rest an deine Adresse zurück.",
      "Only you – the owner, i.e. this wallet address – can top up or cancel. When you cancel, the remainder comes back to your address.",
    ),
    tr(
      "Ausgelöst wird die Zahlung vom K.Lend-Agenten, in der Regel innerhalb weniger Minuten nach dem Termin – solange der Agent läuft und das Guthaben reicht. Laut Vertrag darf auch jeder andere auslösen, etwa der Empfänger. Dein Gerät muss dafür nicht laufen.",
      "The payment is triggered by the K.Lend agent, usually within a few minutes after the due date – as long as the agent is running and the balance suffices. By contract anyone else may trigger it too, e.g. the recipient. Your device does not need to be on.",
    ),
    tr(
      "Die Netzgebühr jeder Zahlung kommt aus dem Tresor: höchstens die Höchstgebühr (0,01 KAS), meist deutlich weniger. Reicht das Guthaben nicht mehr für Betrag, Gebühr und 1 KAS Rest, zahlt der Agent nicht – dann auffüllen.",
      "The network fee of each payment comes from the vault: at most the maximum fee (0.01 KAS), usually much less. If the balance no longer covers amount, fee and 1 KAS remainder, the agent does not pay – then top up.",
    ),
  ];
}

/** Hinweise zum Formular; `balance` = KAS der Wallet in sompi (unbekannt: null) */
export function walletTresorHints(f: TresorForm, balance: bigint | null): Hint[] {
  const out: Hint[] = [];
  const fund = effectiveFund(f);
  const count = formCount(f);
  if (fund !== null && f.amount !== null && f.amount >= TRESOR_MIN_AMOUNT && fund >= minFund(f.amount)) {
    const n = coveredPayments(fund, f.amount);
    if (count !== null && n < count)
      out.push({ level: "warn", text: tr(`Das Guthaben reicht für ${n} von ${count} Zahlungen; danach musst du auffüllen.`, `The balance covers ${n} of ${count} payments; after that you need to top up.`) });
    else if (count === null) out.push({ level: "info", text: tr(`Das Guthaben reicht für ${n} Zahlung(en); auffüllen geht jederzeit.`, `The balance covers ${n} payment(s); you can top up at any time.`) });
    if (balance !== null && fund > balance)
      out.push({ level: "warn", text: tr("Die Wallet hat zu wenig KAS für dieses Startguthaben (plus Netzgebühr).", "The wallet has too few KAS for this starting balance (plus network fee).") });
  }
  if (f.message.trim()) {
    out.push({
      level: "warn",
      text: tr(
        "Die Nachricht steht bei jeder Zahlung öffentlich und dauerhaft in der Blockchain (mit der Browser-Wallet nur öffentlich, nicht verschlüsselt). Sie ist im Vertrag fest gebunden und lässt sich nur mit einem neuen Tresor ändern.",
        "The message is stored publicly and permanently in the blockchain with every payment (with the browser wallet only public, not encrypted). It is fixed in the contract and can only be changed with a new vault.",
      ),
    });
  }
  return out;
}

/**
 * Zustand für die Anzeige. Anders als bei Tresoren mit Schlüsseldatei zahlt
 * der Agent die Gebühr eines Wallet-Tresors nie dazu: trägt der Tresor sie
 * nicht mehr selbst, ist er „knapp“ (auffüllen) bzw. „aufgebraucht“.
 */
export function walletTresorStatus(t: Pick<WalletTresor, "ended" | "missing" | "left" | "value" | "amount" | "maxFee" | "due">): TresorStatus {
  if (t.ended) return "cancelled";
  if (t.missing) return "missing";
  if (t.left === 0) return "done";
  if (feeSource(t) !== "tresor") return contractPayable(t) ? "low" : "empty";
  if (t.due) return "due";
  return "active";
}

/** Laufend (nicht gekündigt, nicht verschwunden)? */
export function walletTresorOpen(t: Pick<WalletTresor, "ended" | "missing">): boolean {
  return !t.ended && !t.missing;
}

/**
 * Auffüllen und Kündigen möglich? Nicht, solange eine gesendete Tx dieses
 * Tresors noch nicht bestätigt ist (`pending`): der Server kennt den neuen
 * Stand erst danach (A19-6).
 */
export function walletTresorActionable(t: Pick<WalletTresor, "ended" | "missing" | "pending">): boolean {
  return walletTresorOpen(t) && !t.pending;
}
