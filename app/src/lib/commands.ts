// Formular-Logik für die Aktionen: aus Eingaben werden die Parameter für
// POST /api/action, und daraus der gleichwertige ghostctl-Befehl („Als Befehl“).
// Optionen laut `./ghostctl <befehl> --help` (Stand 28.09.2026).
import type { NetworkId } from "../config";
import { tr } from "./i18n";

export type CliAction =
  | "open-vault"
  | "mint"
  | "repay"
  | "deposit"
  | "withdraw"
  | "close"
  | "sweep"
  | "liquidate"
  | "redeem"
  | "transfer"
  | "send"
  | "oracle-update"
  | "pool-open"
  | "pool-add"
  | "pool-remove"
  | "swap";

export interface ActionMeta {
  label: string;
  /** Betragsfeld: Einheit und Beschriftung */
  amount?: { unit: "KAS" | "GHOST" | "%"; param: "kas" | "ghost" | "keep" | "usd" | "percent"; label: string; optional?: boolean };
  /** zweites Betragsfeld (Pool: KAS- und GHOST-Reserve) */
  amount2?: { unit: "KAS" | "GHOST"; param: "ghost"; label: string };
  vault?: boolean;
  to?: "address" | "xonly";
  /** Nur der Besitzer des Vaults darf (Signatur im Vertrag) */
  ownerOnly?: boolean;
  help: string;
}

/** Beschriftungen und Hilfen in der aktuellen Sprache (bei jedem Aufruf neu berechnet) */
export const actionMeta = (): Record<CliAction, ActionMeta> => ({
  "open-vault": {
    label: tr("Vault eröffnen", "Open vault"),
    amount: { unit: "KAS", param: "kas", label: tr("Sicherheit", "Collateral") },
    help: tr(
      "Legt einen neuen Vault an. Dafür gehen zusätzlich 3 KAS fest in den Minter-Zweig. Dieser Betrag ist nicht zurückholbar.",
      "Creates a new vault. An additional 3 KAS go permanently into the minter branch and cannot be recovered.",
    ),
  },
  mint: {
    label: tr("Prägen", "Mint"),
    amount: { unit: "GHOST", param: "ghost", label: tr("Betrag", "Amount") },
    vault: true,
    ownerOnly: true,
    help: tr(
      "Erzeugt GHOST gegen die Sicherheit. Danach muss die Quote mindestens 200 % betragen, und Prägen darf die Schuld eines Vaults nicht über die Obergrenze heben (im Mainnet 50 GHOST).",
      "Creates GHOST against the collateral. Afterwards the ratio must be at least 200 %, and minting may not raise a vault's debt above the cap (50 GHOST on mainnet).",
    ),
  },
  repay: {
    label: tr("Tilgen", "Repay"),
    amount: { unit: "GHOST", param: "ghost", label: tr("Betrag", "Amount"), optional: true },
    vault: true,
    ownerOnly: true,
    help: tr(
      "Verbrennt eigene GHOST, höchstens so viel wie die Schuld. Nur mit dem Schlüssel des Besitzers. Der offene Zins bleibt stehen und wird beim Schließen in KAS bezahlt.",
      "Burns your own GHOST, at most as much as the debt. Owner key only. Open interest stays and is paid in KAS when closing.",
    ),
  },
  deposit: {
    label: tr("Einzahlen", "Deposit"),
    amount: { unit: "KAS", param: "kas", label: tr("Betrag", "Amount") },
    vault: true,
    ownerOnly: true,
    help: tr("Schießt KAS nach und verbessert die Quote. Nur mit dem Schlüssel des Besitzers.", "Adds KAS and improves the ratio. Owner key only."),
  },
  withdraw: {
    label: tr("Abheben", "Withdraw"),
    amount: { unit: "KAS", param: "keep", label: tr("Verbleibende Sicherheit", "Collateral to keep") },
    vault: true,
    ownerOnly: true,
    help: tr(
      "Du gibst an, wie viel KAS im Vault BLEIBEN. Der Rest geht an dich. Die Quote muss danach mindestens 200 % betragen.",
      "You enter how much KAS should REMAIN in the vault. The rest goes to you. Afterwards the ratio must be at least 200 %.",
    ),
  },
  close: {
    label: tr("Schließen", "Close"),
    vault: true,
    ownerOnly: true,
    help: tr(
      "Geht nur ohne Schuld. Der offene Zins wird in KAS an die Zinsadresse bezahlt (unter 0,2 KAS wird er erlassen), alle übrigen KAS gehen an dich zurück.",
      "Only possible without debt. Open interest is paid in KAS to the interest address (waived below 0.2 KAS); all remaining KAS go back to you.",
    ),
  },
  sweep: {
    label: tr("Auflösen (Zinsadresse)", "Dissolve (interest address)"),
    vault: true,
    help: tr(
      "Für Vaults ohne Schuld, deren offener Zins die ganze Sicherheit aufzehrt (das bleibt nach einer Liquidation manchmal übrig). Der Besitzer bekäme beim Schließen nichts mehr, deshalb darf jeder so einen Vault auflösen: Die Sicherheit geht an die Zinsadresse, der Vault endet. Die Netzgebühr von 0,1 KAS trägt der Vault (sie geht von der Sicherheit ab); wer auflöst, zahlt nichts.",
      "For vaults without debt whose open interest eats up all the collateral (this sometimes remains after a liquidation). The owner would get nothing when closing, so anyone may dissolve such a vault: the collateral goes to the interest address and the vault ends. The vault bears the network fee of 0.1 KAS (deducted from the collateral); whoever dissolves pays nothing.",
    ),
  },
  liquidate: {
    label: tr("Liquidieren", "Liquidate"),
    amount: { unit: "GHOST", param: "ghost", label: tr("Zu verbrennende GHOST", "GHOST to burn"), optional: true },
    vault: true,
    help: tr(
      "Nur unter 150 % erlaubt (Schuld und Zins zählen). Du verbrennst eigene GHOST, die ganze Schuld oder einen Teil, und bekommst KAS im Wert von Betrag plus 10 %. Reicht die Sicherheit dafür nicht, bekommst du die ganze Sicherheit, der Vault endet und die Restschuld wird ausgebucht.",
      "Only allowed below 150 % (debt and interest count). You burn your own GHOST, the whole debt or part of it, and receive KAS worth the amount plus 10 %. If the collateral does not cover that, you receive all of it, the vault ends and the remaining debt is written off.",
    ),
  },
  redeem: {
    label: tr("Rücknahme", "Redeem"),
    amount: { unit: "GHOST", param: "ghost", label: tr("Zurückzugebende GHOST", "GHOST to return") },
    vault: true,
    help: tr(
      "Jeder kann GHOST an einem Vault ab 150 % zurückgeben und bekommt dafür KAS im Wert von 1 USD je GHOST, abzüglich 1 % (das bleibt beim Vault-Besitzer). Die GHOST werden verbrannt, die Schuld des Vaults sinkt um denselben Betrag. Mindestens 1 GHOST oder die ganze Schuld des Vaults, höchstens die Schuld, und im Vault bleiben mindestens 0,2 KAS.",
      "Anyone can return GHOST to a vault at 150 % or more and receives KAS worth 1 USD per GHOST, minus 1 % (which stays with the vault owner). The GHOST are burned and the vault's debt drops by the same amount. At least 1 GHOST or the vault's whole debt, at most the debt, and at least 0.2 KAS stay in the vault.",
    ),
  },
  transfer: {
    label: tr("GHOST senden", "Send GHOST"),
    amount: { unit: "GHOST", param: "ghost", label: tr("Betrag", "Amount") },
    to: "address",
    help: tr(
      "Überweist GHOST an eine normale Kaspa-Adresse (kaspa:q…, dieselbe, an die man auch KAS schickt) oder eine eigene Schlüsseldatei. Adressen mit kaspa:p… (Skripte) oder ECDSA-Adressen können keine GHOST empfangen. Der Empfänger sieht GHOST auf dieser Seite; gängige Wallets zeigen GHOST noch nicht an.",
      "Sends GHOST to a regular Kaspa address (kaspa:q…, the same one used for KAS) or one of your key files. kaspa:p… (script) and ECDSA addresses cannot receive GHOST. The recipient sees GHOST on this site; common wallets do not show GHOST yet.",
    ),
  },
  send: {
    label: tr("KAS senden", "Send KAS"),
    amount: { unit: "KAS", param: "kas", label: tr("Betrag", "Amount") },
    to: "address",
    help: tr("Normale KAS-Überweisung an eine Adresse oder eine Schlüsseldatei.", "Regular KAS transfer to an address or a key file."),
  },
  "pool-open": {
    label: tr("Pool anlegen", "Create pool"),
    amount: { unit: "KAS", param: "kas", label: tr("KAS gesamt (mindestens 1)", "Total KAS (at least 1)") },
    amount2: { unit: "GHOST", param: "ghost", label: tr("GHOST gesamt", "Total GHOST") },
    help: tr(
      "Legt den Tauschpool an, einmalig je Netz, in drei Transaktionen. Das Verhältnis von KAS zu GHOST ist der Startkurs; leg ihn nahe am Orakelpreis an. 1 KAS und GHOST zum gleichen Kurs bleiben als Mindestliquidität für immer im Pool, für den Rest bekommst du Anteile.",
      "Creates the swap pool, once per network, in three transactions. The KAS to GHOST ratio is the starting price; set it close to the oracle price. 1 KAS and GHOST at the same price stay in the pool forever as minimum liquidity; for the rest you receive shares.",
    ),
  },
  "pool-add": {
    label: tr("Einlegen", "Add"),
    amount: { unit: "KAS", param: "kas", label: "KAS" },
    amount2: { unit: "GHOST", param: "ghost", label: "GHOST" },
    help: tr(
      "Legt KAS und GHOST in den Pool, dafür bekommst du Anteile. Genommen wird nur der zum Poolkurs passende Teil; weicht der Poolkurs um mehr als 1 % von deinem Verhältnis ab, bricht ghostctl ab.",
      "Adds KAS and GHOST to the pool; in return you receive shares. Only the part matching the pool price is taken; if the pool price deviates by more than 1 % from your ratio, ghostctl aborts.",
    ),
  },
  "pool-remove": {
    label: tr("Abziehen", "Remove"),
    amount: { unit: "%", param: "percent", label: tr("Anteil deiner Pool-Anteile", "Share of your pool shares") },
    help: tr(
      "Verbrennt Anteile und zahlt dir den entsprechenden Teil beider Reserven aus, einschließlich der aufgelaufenen Gebühren.",
      "Burns shares and pays out the corresponding part of both reserves, including accrued fees.",
    ),
  },
  swap: {
    label: tr("Tauschen", "Swap"),
    help: tr("Tauscht KAS gegen GHOST oder umgekehrt zum Kurs des Pools, abzüglich 0,3 % Gebühr.", "Swaps KAS for GHOST or vice versa at the pool price, minus a 0.3 % fee."),
  },
  "oracle-update": {
    label: tr("Orakel aktualisieren", "Update oracle"),
    help: tr(
      "Signiert mit den Schlüsseln der Unterzeichner; das Unterzeichner-Register prüft die Signaturen. Ohne festen Preis gilt der Median aus 6 Preisquellen. Der Vertrag erlaubt 0,00001–900 USD, und je Update darf sich der Preis höchstens verdoppeln oder halbieren.",
      "Signed with the signers' keys; the signer register checks the signatures. Without a fixed price the median of 6 price sources is used. The contract allows 0.00001–900 USD, and each update may at most double or halve the price.",
    ),
  },
});

export const ACTION_ORDER: CliAction[] = [
  "open-vault",
  "mint",
  "repay",
  "deposit",
  "withdraw",
  "close",
  "liquidate",
  "redeem",
  "transfer",
  "send",
];

/** 1e8-Einheiten → „1.5“ (Punkt als Dezimaltrenner, wie ghostctl es erwartet) */
export function cliDecimal(units: bigint, decimals = 8): string {
  const base = 10n ** BigInt(decimals);
  const whole = units / base;
  const frac = (units % base).toString().padStart(decimals, "0").replace(/0+$/, "");
  return frac ? `${whole}.${frac}` : `${whole}`;
}

/** Pfade nur bei Bedarf in einfache Anführungszeichen setzen (zsh/bash). */
export function shellArg(s: string): string {
  if (/^[A-Za-z0-9_./:-]+$/.test(s)) return s;
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

export type ActionParams = Record<string, string | number | boolean>;

/** Reihenfolge der Optionen je Aktion (wie server/actions.ts) */
export const FLAG_ORDER: Record<CliAction, string[]> = {
  "open-vault": ["key", "kas"],
  mint: ["key", "vault", "ghost"],
  repay: ["key", "vault", "ghost"],
  deposit: ["key", "vault", "kas"],
  withdraw: ["key", "vault", "keep"],
  close: ["key", "vault"],
  sweep: ["key", "vault"],
  liquidate: ["key", "vault", "ghost"],
  redeem: ["key", "vault", "ghost"],
  transfer: ["key", "to", "ghost", "message", "onchain"],
  send: ["key", "to", "kas", "message", "onchain"],
  "oracle-update": ["key", "committee", "usd", "rate"],
  "pool-open": ["key", "kas", "ghost"],
  "pool-add": ["key", "kas", "ghost", "minShares"],
  "pool-remove": ["key", "percent", "minKas", "minGhost"],
  swap: ["key", "kas", "ghost", "min"],
};

/** Gleichwertiger Terminal-Befehl. Ohne --json und --ja, damit ghostctl selbst nachfragt. */
export function commandFor(network: NetworkId, action: CliAction, params: ActionParams, dryRun = false): string {
  const parts = ["./ghostctl", "--network", network];
  if (dryRun) parts.push("--dry-run");
  parts.push(action);
  for (const k of FLAG_ORDER[action]) {
    const v = params[k];
    if (v === undefined || v === "" || v === false) continue;
    // Öffentliche Nachricht ist ein Schalter ohne Wert
    if (k === "onchain") {
      parts.push("--onchain-message");
      continue;
    }
    // Tauschen: der Mindestbetrag heißt je nach Richtung --min-ghost bzw. --min-kas
    const flag =
      action === "swap" && k === "min" ? (params.kas !== undefined ? "min-ghost" : "min-kas") : k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
    // Nachricht mit „=“: ein Text mit „-“ am Anfang wird so nie als Option gelesen
    if (k === "message") parts.push(`--message=${shellArg(String(v))}`);
    else parts.push(`--${flag}`, shellArg(String(v)));
  }
  return parts.join(" ");
}
