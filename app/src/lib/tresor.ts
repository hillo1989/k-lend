// Daueraufträge mit Tresor (ghostctl tresor): Datentypen der Liste,
// Startguthaben-Vorschlag, Vorprüfung des Formulars und Anzeige.
// Verbindlich ist, was ghostctl beim Anlegen sagt (protocol/src/tresor.rs).
import type { AboInterval } from "./abo";
import { isDate, messageProblem } from "./abo";
import type { KeyEntry } from "./api";
import { cliDecimal } from "./commands";
import { parseUnits } from "./format";
import { tr } from "./i18n";
import type { Hint } from "./precheck";

/** Kleinster Betrag je Zahlung in sompi (tresor::MIN_AMOUNT) */
export const TRESOR_MIN_AMOUNT = 100_000_000n;
/** Höchstgebühr je Zahlung in sompi (tresor::DEFAULT_MAX_FEE) */
export const TRESOR_MAX_FEE = 1_000_000n;
/**
 * Untergrenze der Höchstgebühr beim Anlegen in sompi (tresor::MIN_MAX_FEE,
 * A13-tresor-4): gemessen kostet eine Zahlung mit der längsten Nachricht bis
 * 359.100 sompi; darunter trägt der Tresor die Netzgebühr nicht sicher.
 */
export const TRESOR_MIN_MAX_FEE = 400_000n;
/** Reserve im Vorschlag, bleibt nach der letzten Zahlung (tresor::RESERVE) */
export const TRESOR_RESERVE = 100_000_000n;
/**
 * So viel lassen ghostctl und diese Seite nach jeder Zahlung mindestens im
 * Tresor (tresor::MIN_KEEP). Der Vertrag verlangt nur einen Rest über 0 nach
 * Betrag und Höchstgebühr; ein fremder Auslöser kann mit weniger Rest zahlen.
 */
export const TRESOR_MIN_KEEP = 100_000_000n;
/**
 * Tresor-Code Version 2: Vertrag mit fest gebundener Nachricht (payloadHash).
 * Codes der Version 1 gehören zum alten Vertrag und werden abgelehnt.
 */
export const TRESOR_CODE_PREFIX = "ghost-tresor:2:";
const OLD_CODE_PREFIX = "ghost-tresor:1:";
const CODE_RE = /^ghost-tresor:2:[A-Za-z0-9_-]{16,4000}$/;

export interface TresorHist {
  at: string;
  action: "open" | "pay" | "topup" | "cancel" | "import" | string;
  txid?: string | null;
  due?: number | null;
  note?: string | null;
}

/** Ein Eintrag aus `ghostctl tresor list --json` */
export interface Tresor {
  id: string;
  covenantId: string;
  owner: string;
  recipient: string;
  ownerAddress: string;
  recipientAddress: string;
  amount: string;
  maxFee: string;
  anchorDay: number;
  periodMs: number;
  nextDue: number;
  left: number;
  value: string;
  covered: number;
  outpoint: string;
  /** Beschreibung, nach heutigem Filter bereinigt */
  message: string;
  /** die gespeicherte Beschreibung enthielt Zeichen, die heute nicht mehr erlaubt sind (älterer Tresor); angezeigt wird sie bereinigt */
  messageCleaned?: boolean;
  onchain: boolean;
  /** Nachricht verschlüsselt an den Empfänger in jeder Zahlung (fehlt bei älteren ghostctl) */
  encrypted?: boolean;
  /**
   * Trägt jede Zahlung nachweislich diese Beschreibung (A13-tresor-1)?
   * bound = öffentlich, im Vertrag gebunden; checked = verschlüsselt, hier
   * angelegt oder mit dem Schlüssel des Empfängers geprüft; unchecked = nur
   * laut Tresor-Code; mismatch = die Zahlungen tragen etwas anderes; none =
   * keine Nachricht. Fehlt bei älteren ghostctl.
   */
  messageCheck?: "none" | "bound" | "checked" | "unchecked" | "mismatch";
  key: string | null;
  created: string;
  ended: string | null;
  missing: string | null;
  lastError: string | null;
  due: boolean;
  history: TresorHist[];
  code: string;
}

export interface TresorList {
  ok: boolean;
  error?: string;
  tresore?: Tresor[];
}

/** Startguthaben-Vorschlag: n × (Betrag + Höchstgebühr) + Reserve; unbegrenzt: null */
export function suggestedFund(amount: bigint | null, count: number | null): bigint | null {
  if (amount === null || amount <= 0n || count === null || count < 1) return null;
  return BigInt(count) * (amount + TRESOR_MAX_FEE) + TRESOR_RESERVE;
}

/** Wie viele Zahlungen ein Guthaben trägt (tresor::payments_covered) */
export function coveredPayments(fund: bigint, amount: bigint): number {
  if (amount <= 0n || fund <= TRESOR_RESERVE) return 0;
  return Number((fund - TRESOR_RESERVE) / (amount + TRESOR_MAX_FEE));
}

/** Kleinstes Startguthaben: eine Zahlung + Höchstgebühr + Reserve */
export function minFund(amount: bigint): bigint {
  return amount + TRESOR_MAX_FEE + TRESOR_RESERVE;
}

/** Termin eines Kalendertags: 00:00 UTC (wie ghostctl tresor open) als Unix-ms */
export function dueMs(date: string): number | null {
  return isDate(date) ? Date.parse(`${date}T00:00:00Z`) : null;
}

/** Zeitpunkt in der Ortszeit dieses Rechners, z. B. „30.01.2027, 19:00“ */
export function localDateTime(ms: number, loc = "de-DE"): string {
  return new Date(ms).toLocaleString(loc, { dateStyle: "medium", timeStyle: "short" });
}

/** Zeitpunkt in Weltzeit, z. B. „31.01.2027, 00:00 UTC“ */
export function utcDateTime(ms: number, loc = "de-DE"): string {
  return `${new Date(ms).toLocaleString(loc, { dateStyle: "medium", timeStyle: "short", timeZone: "UTC" })} UTC`;
}

export interface TresorForm {
  to: string;
  amount: bigint | null;
  amountText: string;
  interval: AboInterval | null;
  start: string;
  endMode: "none" | "count";
  count: string;
  /** Startguthaben in sompi; null = leer/ungültig */
  fund: bigint | null;
  fundText: string;
  message: string;
  onchain: boolean;
}

export function formCount(f: TresorForm): number | null {
  if (f.endMode !== "count") return null;
  const t = f.count.trim();
  return /^\d{1,4}$/.test(t) && Number(t) >= 1 ? Number(t) : null;
}

/** Tatsächliches Startguthaben: Eingabe oder (leer, mit Anzahl) der Vorschlag */
export function effectiveFund(f: TresorForm): bigint | null {
  if (f.fundText.trim() === "") return suggestedFund(f.amount, formCount(f));
  return f.fund;
}

/** Parameter für POST /api/action (tresor-open) oder ein Problem */
export function tresorParams(f: TresorForm, key: KeyEntry | null, cliDecimal: (u: bigint) => string, today: string): { params: Record<string, string | number | boolean> | null; problem: string | null } {
  const fail = (p: string) => ({ params: null, problem: p });
  if (!key) return fail(tr("Kein Schlüssel gewählt.", "No key selected."));
  if (!f.to.trim()) return fail(tr("Empfänger angeben.", "Enter a recipient."));
  if (f.to.trim() === key.file || f.to.trim() === key.address || f.to.trim() === key.xonly) return fail(tr("Empfänger ist das eigene Konto.", "The recipient is your own account."));
  if (f.amountText.trim() === "") return fail(tr("Betrag eingeben.", "Enter an amount."));
  if (f.amount === null || f.amount <= 0n) return fail(tr("Betrag: ungültige Zahl.", "Amount: invalid number."));
  if (f.amount < TRESOR_MIN_AMOUNT) return fail(tr("Mit Tresor: mindestens 1 KAS je Zahlung.", "With a vault: at least 1 KAS per payment."));
  if (f.interval === null) return fail(tr("Intervall: 1 bis 3650 Tage.", "Interval: 1 to 3650 days."));
  if (!isDate(f.start)) return fail(tr("Start: gültiges Datum wählen.", "Start: choose a valid date."));
  if (f.start < today) return fail(tr("Start liegt in der Vergangenheit.", "Start is in the past."));
  const count = formCount(f);
  if (f.endMode === "count" && count === null) return fail(tr("Anzahl: ganze Zahl ab 1.", "Count: whole number from 1."));
  const fund = effectiveFund(f);
  if (fund === null) return fail(f.endMode === "none" ? tr("Unbegrenzt: Startguthaben angeben.", "Unlimited: enter the starting balance.") : tr("Startguthaben: ungültige Zahl.", "Starting balance: invalid number."));
  if (fund < minFund(f.amount))
    return fail(tr(`Startguthaben: mindestens ${cliDecimal(minFund(f.amount))} KAS (eine Zahlung, Gebühr und 1 KAS Rest).`, `Starting balance: at least ${cliDecimal(minFund(f.amount))} KAS (one payment, fee and 1 KAS remainder).`));
  const mp = messageProblem(f.message);
  if (mp) return fail(mp);
  const p: Record<string, string | number | boolean> = {
    key: key.file,
    to: f.to.trim(),
    amount: cliDecimal(f.amount),
    interval: typeof f.interval === "string" ? f.interval : String(f.interval.days),
    start: f.start,
    fund: cliDecimal(fund),
  };
  if (count !== null) p.count = count;
  if (f.message.trim()) p.message = f.message.trim();
  if (f.onchain) {
    if (!f.message.trim()) return fail(tr("Für eine öffentliche Nachricht erst eine Nachricht eingeben.", "Enter a message first to publish it."));
    p.onchain = true;
  }
  return { params: p, problem: null };
}

/** Hinweise zum Tresor-Formular */
export function tresorHints(f: TresorForm, key: KeyEntry | null): Hint[] {
  const out: Hint[] = [];
  const fund = effectiveFund(f);
  const count = formCount(f);
  out.push({
    level: "info",
    text: tr(
      "Das Startguthaben liegt im Tresor und ist dort gebunden. Du kannst jederzeit kündigen; dann kommt der Rest zu dir zurück.",
      "The starting balance sits in the vault and is locked there. You can cancel at any time; the remainder then comes back to you.",
    ),
  });
  out.push({
    level: "info",
    text: tr(
      "Zum Termin darf jeder die Zahlung auslösen, meist der Empfänger oder ein laufender GHOST-Agent – dein Rechner muss dafür nicht laufen. Je Zahlung gehen der Betrag und höchstens 0,01 KAS Höchstgebühr aus dem Tresor. Was davon nicht als Netzgebühr gebraucht wird, darf ein fremder Auslöser behalten; der GHOST-Agent und diese Seite lassen es im Tresor.",
      "On the due date anyone may trigger the payment, usually the recipient or a running GHOST agent – your computer does not need to be on. Each payment takes the amount and at most 0.01 KAS maximum fee from the vault. Whatever is not needed as network fee may be kept by a third-party trigger; the GHOST agent and this site leave it in the vault.",
    ),
  });
  if (fund !== null && f.amount !== null && f.amount >= TRESOR_MIN_AMOUNT && fund >= minFund(f.amount)) {
    const n = coveredPayments(fund, f.amount);
    if (count !== null && n < count)
      out.push({ level: "warn", text: tr(`Das Guthaben reicht für ${n} von ${count} Zahlungen; danach musst du auffüllen.`, `The balance covers ${n} of ${count} payments; after that you need to top up.`) });
    else if (count === null) out.push({ level: "info", text: tr(`Das Guthaben reicht für ${n} Zahlung(en); auffüllen geht jederzeit.`, `The balance covers ${n} payment(s); you can top up at any time.`) });
    if (key && key.kas !== null && Number(fund) / 1e8 > key.kas)
      out.push({ level: "warn", text: tr(`Der Schlüssel hat nur ${key.kas} KAS – zu wenig für dieses Startguthaben.`, `The key only has ${key.kas} KAS – not enough for this starting balance.`) });
  }
  if (f.onchain && f.message.trim())
    out.push({ level: "warn", text: tr("Die Nachricht steht bei jeder Zahlung dauerhaft und für alle lesbar in der Blockchain.", "The message is stored permanently and readable by anyone in the blockchain with every payment.") });
  out.push(
    f.message.trim()
      ? {
          level: "info",
          text: tr(
            "Die Nachricht ist im Vertrag fest gebunden: Jede Zahlung trägt genau diese Nachricht, und wer eine Zahlung auslöst, kann sie weder weglassen noch ändern. Auch du kannst sie später nicht mehr ändern – dafür kündigen und einen neuen Tresor anlegen.",
            "The message is fixed in the contract: every payment carries exactly this message, and whoever triggers a payment can neither leave it out nor change it. You cannot change it later either – to do so, cancel and create a new vault.",
          ),
        }
      : {
          level: "info",
          text: tr(
            "Ohne Nachricht tragen alle Zahlungen keinen Text; auch später lässt sich keiner hinzufügen (nur mit einem neuen Tresor).",
            "Without a message, no payment carries any text; none can be added later either (only with a new vault).",
          ),
        },
  );
  return out;
}

// ------------------------------------------------------------ Anzeige ----

/**
 * Was jede Zahlung als Nachricht trägt (A12-1 im Vertrag): fest gebunden über
 * ihren Hash; wer auslöst, kann sie nicht ändern, und ändern lässt sie sich nur
 * mit einem neuen Tresor. Für die Bestätigung beim Anlegen und die Liste.
 */
export function boundMessageText(t: Pick<Tresor, "message" | "onchain">): string {
  if (!t.message)
    return tr(
      "Die Zahlungen tragen keine Nachricht; im Vertrag ist das fest, nachträglich lässt sich keine hinzufügen.",
      "The payments carry no message; this is fixed in the contract, none can be added later.",
    );
  return tr(
    `Jede Zahlung trägt die Nachricht „${t.message}“ (${t.onchain ? "öffentlich" : "verschlüsselt"}). Sie ist im Vertrag fest gebunden: Wer eine Zahlung auslöst, kann sie nicht ändern, und ändern lässt sie sich nur mit einem neuen Tresor.`,
    `Every payment carries the message “${t.message}” (${t.onchain ? "public" : "encrypted"}). It is fixed in the contract: whoever triggers a payment cannot change it, and it can only be changed with a new vault.`,
  );
}

export type TresorRole = "owner" | "recipient" | "other";

export function tresorRole(t: Tresor, xonly: string | null | undefined): TresorRole {
  if (xonly && t.owner === xonly) return "owner";
  if (xonly && t.recipient === xonly) return "recipient";
  return "other";
}

export type TresorStatus = "cancelled" | "missing" | "done" | "empty" | "low" | "due" | "active";

/** Dezimaltext von ghostctl („10.5“) → sompi */
const units = (s: string) => {
  const [w, fr = ""] = s.split(".");
  return BigInt(w) * 100_000_000n + BigInt((fr + "00000000").slice(0, 8));
};

/**
 * Wer trägt die Netzgebühr der nächsten Zahlung (wie tresor::payable)?
 * tresor = der Tresor selbst (Betrag + Höchstgebühr, danach bleibt 1 KAS),
 * key = nur mit einem eigenen Schlüssel, der die Gebühr zahlt (Betrag + 1 KAS
 * Rest reichen noch), null = gar nicht zahlbar, der Absender muss auffüllen.
 */
export function feeSource(t: Pick<Tresor, "value" | "amount" | "maxFee">): "tresor" | "key" | null {
  const rest = units(t.value) - units(t.amount);
  if (rest - units(t.maxFee) >= TRESOR_MIN_KEEP) return "tresor";
  if (rest >= TRESOR_MIN_KEEP) return "key";
  return null;
}

/**
 * Parameter für „Fällige Zahlung abholen“ (tresor-pay): der eigene Schlüssel
 * nur, wenn der Tresor die Gebühr nicht mehr trägt – sonst zahlte ghostctl bei
 * einer Gebührenspitze still vom eigenen Schlüssel (A12-10). null = nicht möglich.
 */
export function payParams(t: Tresor, keyFile: string | null): Record<string, string> | null {
  const fee = feeSource(t);
  if (fee === "tresor") return { id: t.id };
  if (fee === "key" && keyFile) return { id: t.id, key: keyFile };
  return null;
}

/** Auffüllen: der geprüfte Betrag als Dezimaltext für ghostctl („1.000,5“ → „1000.5“), sonst null */
export function topupKas(text: string): string | null {
  const u = text.trim() ? parseUnits(text, 8) : null;
  return u !== null && u > 0n ? cliDecimal(u) : null;
}

/**
 * Trägt jede Zahlung nachweislich die angezeigte Beschreibung? Nur dann darf
 * die Liste sagen, die Nachricht sei im Vertrag fest gebunden (A13-tresor-1):
 * Bei einer verschlüsselten Nachricht bindet der Vertrag nur die
 * verschlüsselte Fassung, nicht den Text im Tresor-Code. Ohne `messageCheck`
 * (älteres ghostctl): nur öffentlich oder hier angelegt.
 */
export function messageSure(t: Pick<Tresor, "message" | "onchain" | "key" | "messageCheck">): boolean {
  if (!t.message) return false;
  if (t.messageCheck) return t.messageCheck === "bound" || t.messageCheck === "checked";
  return t.onchain || t.key !== null;
}

/** Höchstgebühr unter der Untergrenze (älterer oder fremd angelegter Tresor, A13-tresor-4) */
export function maxFeeTooLow(t: Pick<Tresor, "maxFee">): boolean {
  return units(t.maxFee) < TRESOR_MIN_MAX_FEE;
}

/**
 * Kann laut Vertrag überhaupt noch jemand zahlen? pay() verlangt nur
 * Wert − Betrag − Höchstgebühr > 0 – ohne die 1 KAS Rest, die ghostctl lässt.
 */
export function contractPayable(t: Pick<Tresor, "value" | "amount" | "maxFee">): boolean {
  return units(t.value) - units(t.amount) - units(t.maxFee) > 0n;
}

/**
 * Zustand für die Anzeige; „fällig“ nur, wenn ghostctl die Zahlung auch
 * auslöst. „aufgebraucht“ nur, wenn auch der Vertrag keine Zahlung mehr zulässt;
 * „knapp“, wenn ghostctl nicht mehr zahlt (1 KAS Rest), ein fremder Auslöser
 * aber noch könnte (A12-8).
 */
export function tresorStatus(t: Tresor): TresorStatus {
  if (t.ended) return "cancelled";
  if (t.missing) return "missing";
  if (t.left === 0) return "done";
  if (feeSource(t) === null) return contractPayable(t) ? "low" : "empty";
  if (t.due) return "due";
  return "active";
}

export function tresorStatusLabel(s: TresorStatus): string {
  const m: Record<TresorStatus, [string, string]> = {
    cancelled: ["gekündigt", "cancelled"],
    missing: ["nicht auffindbar", "not found"],
    done: ["alle Zahlungen erledigt", "all payments done"],
    empty: ["Guthaben aufgebraucht", "balance used up"],
    low: ["Guthaben knapp", "balance low"],
    due: ["Zahlung fällig", "payment due"],
    active: ["aktiv", "active"],
  };
  return tr(m[s][0], m[s][1]);
}

export function leftLabel(left: number): string {
  if (left < 0) return tr("unbegrenzt", "unlimited");
  if (left === 0) return tr("keine", "none");
  return left === 1 ? tr("noch 1 Zahlung", "1 payment left") : tr(`noch ${left} Zahlungen`, `${left} payments left`);
}

export function tresorIntervalLabel(t: Pick<Tresor, "anchorDay" | "periodMs">): string {
  const day = 86_400_000;
  if (t.anchorDay > 0) return tr(`monatlich am ${t.anchorDay}.`, `monthly on day ${t.anchorDay}`);
  if (t.periodMs === day) return tr("täglich", "daily");
  if (t.periodMs === 7 * day) return tr("wöchentlich", "weekly");
  return tr(`alle ${Math.round(t.periodMs / day)} Tage`, `every ${Math.round(t.periodMs / day)} days`);
}

/** Fehlertext eines eingefügten Tresor-Codes oder null */
export function codeProblem(code: string): string | null {
  const c = code.trim();
  if (!c) return tr("Tresor-Code einfügen.", "Paste a vault code.");
  if (c.startsWith(OLD_CODE_PREFIX))
    return tr(
      "Alter Tresor-Code: Dieser Tresor läuft mit dem alten Vertrag, der die Nachricht nicht bindet, und wird nicht mehr unterstützt. Bitte beim Absender einen neuen Tresor anfordern.",
      "Old vault code: this vault uses the old contract, which does not bind the message, and is no longer supported. Please ask the sender for a new vault.",
    );
  if (!c.startsWith(TRESOR_CODE_PREFIX)) return tr(`Das ist kein Tresor-Code (er beginnt mit „${TRESOR_CODE_PREFIX}“).`, `This is not a vault code (it starts with “${TRESOR_CODE_PREFIX}”).`);
  if (!CODE_RE.test(c)) return tr("Tresor-Code unvollständig oder beschädigt – bitte vollständig kopieren.", "Vault code incomplete or damaged – please copy it completely.");
  return null;
}
