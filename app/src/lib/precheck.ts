// Vorprüfungen im Browser mit Live-Daten und dem Rechenkern (vaultMath).
// Sie ersetzen nicht den Probelauf: Verbindlich ist, was ghostctl --dry-run
// (und am Ende der Vertrag) sagt. Hier geht es um frühe, verständliche Hinweise.
import type { KeyEntry } from "./api";
import type { CliAction } from "./commands";
import type { DeployedStatus, VaultStatus } from "./status";
import { payoutFor, POOL_MIN_KAS, sharesForDeposit } from "./poolMath";
import {
  BPS,
  DUST,
  MIN_REDEEM,
  PRICE_SCALE,
  SMALL_OUTPUT,
  SWEEP_FEE,
  SWEEP_MIN_TREASURY,
  closeFee,
  collateralValue,
  liquidationPreview,
  maxRedeemable,
  mulDivUp,
  redeemPayout,
} from "./vaultMath";
import { locale, tr } from "./i18n";

export interface Hint {
  level: "error" | "warn" | "info";
  text: string;
}

/** Grobe Obergrenze der Gebühr je Aktion (gemessen: Testnetz 0,036–0,051 KAS je Vault-Tx, Simulator 0,054–0,059 je Pool-Tx) */
export const FEE_RESERVE_KAS = 0.06;
/** open-vault: fester Minter-Zweig */
export const MINTER_BRANCH_KAS = 3;

const units = (x: number) => BigInt(Math.round(x * 1e8));
const fmt = (x: number, d = 8) => x.toLocaleString(locale(), { maximumFractionDigits: d });

export interface PrecheckInput {
  action: CliAction;
  /** Betrag in 1e8-Einheiten (KAS, GHOST oder verbleibende KAS), null = leer */
  amount: bigint | null;
  /** zweiter Betrag (Pool: GHOST) in 1e8-Einheiten */
  amount2?: bigint | null;
  repayAll?: boolean;
  /** liquidate: ganze Schuld verbrennen (Standard) */
  liquidateAll?: boolean;
  /** oracle-update: fester Preis (USD × 1e8) bzw. neuer Zins (bps), null = leer */
  usd?: bigint | null;
  rateBps?: bigint | null;
  vault: number | null;
  key: KeyEntry | null;
  status: DeployedStatus | null;
}

/** Preisgrenzen des Orakel-Vertrags (risk_oracle.sil, Version 2) */
export const ORACLE_MIN_USD = 0.00001;
export const ORACLE_MAX_USD = 900;

/** Rücknahme-Abschlag in bps (ghostctl params.redeemFeePct, sonst 1 % wie im Vertrag) */
export const redeemFeeBps = (s: DeployedStatus | null) => BigInt(Math.round((s?.params.redeemFeePct ?? 1) * 100));

/**
 * Darf jeder diesen Vault zugunsten der Zinsadresse auflösen (sweep)? Schuld 0 und
 * die Zinsgebühr zum Orakelpreis erreicht die ganze Sicherheit. ghostctl meldet
 * das exakt (sweepable); ältere Stände werden aus den Anzeigewerten gerechnet.
 * Ein Vault, dessen Kassen-Ausgang nach SWEEP_FEE zu klein wäre, gilt nie als
 * auflösbar, auch wenn ein älteres ghostctl es meldet (Audit 12 A12-2).
 */
export function vaultSweepable(v: VaultStatus, s: DeployedStatus): boolean {
  if (v.stale || tooSmallToSweep(v)) return false;
  if (v.sweepable !== undefined) return v.sweepable;
  if (v.debtGhost > 0 || !(v.interestUsd && v.interestUsd > 0)) return false;
  const f = feeAtOracle(v, s);
  return f !== null && f.fee === units(v.collateralKas);
}

/** Auflösen: nach SWEEP_FEE bliebe der Zinsadresse zu wenig für einen eigenen Ausgang (A12-2) */
const tooSmallToSweep = (v: VaultStatus) => units(v.collateralKas) < SWEEP_FEE + SWEEP_MIN_TREASURY;

/**
 * Zehrt der Zins die ganze Sicherheit auf (Schuld 0, Zinsgebühr zum Orakelpreis =
 * Sicherheit)? Anders als vaultSweepable auch bei Vaults, die zu klein zum
 * Auflösen sind: Dort zehrt der Zins weiter, nur auflösen kann sie niemand
 * (Audit 12, Restpunkt B-P5). Meldet ghostctl den Vault als auflösbar, gilt
 * das; sonst wird aus den Anzeigewerten gerechnet.
 */
export function vaultInterestEatsCollateral(v: VaultStatus, s: DeployedStatus): boolean {
  if (v.stale) return false;
  if (vaultSweepable(v, s)) return true;
  // ghostctl rechnet genau: ein ausreichend großer Vault, den es nicht als auflösbar meldet, ist es nicht
  if (v.sweepable === false && !tooSmallToSweep(v)) return false;
  if (v.debtGhost > 0 || !(v.interestUsd && v.interestUsd > 0)) return false;
  const f = feeAtOracle(v, s);
  return f !== null && f.fee === units(v.collateralKas);
}

/** Mindestsicherheit, ab der sich ein Vault auflösen lässt (0,125 KAS) */
export const SWEEP_MIN_COLLATERAL_KAS = Number(SWEEP_FEE + SWEEP_MIN_TREASURY) / 1e8;

/**
 * Zinsgebühr zum Orakelpreis wie der Vertrag. null, wenn der Vertrag dabei über
 * 64 Bit läuft (NumberTooBig): nur bei extremem Zins zum Tiefstpreis, dann
 * scheitern Schließen und Auflösen, bis der KAS-Preis steigt (Audit 12 A12-14).
 */
function feeAtOracle(v: VaultStatus, s: DeployedStatus): ReturnType<typeof closeFee> | null {
  try {
    return closeFee(units(v.interestUsd ?? 0), BigInt(Math.round(s.oracle.kasUsd * 1e8)), units(v.collateralKas));
  } catch {
    return null;
  }
}

const feeOverflowHint = (): Hint => ({
  level: "error",
  text: tr(
    "Der offene Zins ist beim jetzigen KAS-Preis zu groß für die Rechnung des Vertrags. Schließen und Auflösen scheitern, bis der KAS-Preis wieder steigt.",
    "At the current KAS price the open interest is too large for the contract's arithmetic. Closing and dissolving fail until the KAS price rises again.",
  ),
});

/** Hinweis bei einem Auszahlungs-Ausgang unter etwa 0,02 KAS (Audit 11 A11-O-12) */
const smallOutputHint = (what: string, sompi: bigint): Hint => ({
  level: "warn",
  text: tr(
    `${what} wären nur ${fmt(Number(sompi) / 1e8, 8)} KAS. Ein so kleiner Ausgang ist für das Netz zu schwer (Speichermasse über der Blockgrenze), ghostctl kann die Transaktion dann nicht bauen. Vorher etwas KAS einzahlen oder den Betrag ändern.`,
    `${what} would be only ${fmt(Number(sompi) / 1e8, 8)} KAS. Such a small output is too heavy for the network (storage mass above the block limit), so ghostctl cannot build the transaction. Deposit some KAS first or change the amount.`,
  ),
});

/** Was für die Quoten zählt: Schuld + offener Zins (Version 3), in USD × 1e8 */
export const owedUnits = (v: { debtGhost: number; interestUsd?: number }) => units(v.debtGhost) + units(v.interestUsd ?? 0);

/** Mindest-Sicherheit (sompi), damit die Quote nach dem Abheben ≥ MCR bleibt. `interestUsd`: offener Zins. */
export function minKeepSompi(debtGhost: number, kasUsd: number, mcrPct: number, interestUsd = 0): bigint {
  const owed = units(debtGhost) + units(interestUsd);
  if (owed === 0n) return 1n;
  const price = BigInt(Math.round(kasUsd * 1e8));
  const need = mulDivUp(owed, BigInt(Math.round(mcrPct * 100)), BPS); // USD×1e8
  return (need * PRICE_SCALE + price - 1n) / price;
}

/** Quote (in %) nach Abheben auf `keep` sompi, auf Schuld + Zins */
export function ratioAfterKeep(keep: bigint, debtGhost: number, kasUsd: number, interestUsd = 0): number | null {
  const owed = units(debtGhost) + units(interestUsd);
  if (owed === 0n) return null;
  const value = collateralValue(keep, BigInt(Math.round(kasUsd * 1e8)));
  return Number((value * 1_000_000n) / owed) / 10_000;
}

/**
 * Version 4: Ist das Orakel eingefroren, lehnen Vault und Pool diese Aktionen
 * ab (stable_vault_v4.sil, ghost_pool_v4.sil mit stopWhenFrozen). Abheben ist
 * nur bei offener Schuld gesperrt.
 */
export function frozenBlocks(action: CliAction, v: VaultStatus | null): boolean {
  if (["mint", "redeem", "liquidate", "sweep", "swap"].includes(action)) return true;
  return action === "withdraw" && (v?.debtGhost ?? 0) > 0;
}

export const frozenText = () =>
  tr(
    "Das Orakel ist eingefroren: Seit über der Frist kam kein Preis. Prägen, Einlösen, Liquidieren, Auflösen, Tauschen und Abheben bei offener Schuld sind gesperrt, bis wieder ein Preis kommt. Einzahlen, Tilgen und Schließen gehen weiter.",
    "The oracle is frozen: no price has arrived within the deadline. Minting, redeeming, liquidating, dissolving, swapping and withdrawing with open debt are blocked until a price arrives again. Depositing, repaying and closing still work.",
  );

export function precheck(i: PrecheckInput): Hint[] {
  const out: Hint[] = [];
  const s = i.status;
  const k = i.key;
  const amt = i.amount === null ? null : Number(i.amount) / 1e8;
  if (!k) out.push({ level: "error", text: tr("Bitte oben im Konto eine Schlüsseldatei wählen.", "Please choose a key file in the account card above.") });

  const needsVault = ["mint", "repay", "deposit", "withdraw", "close", "sweep", "liquidate", "redeem"].includes(i.action);
  const v = needsVault && s && i.vault !== null ? (s.vaults.find((x) => x.index === i.vault) ?? null) : null;
  if (needsVault && i.vault === null) out.push({ level: "error", text: tr("Bitte einen Vault wählen.", "Please choose a vault.") });
  if (needsVault && s && i.vault !== null && !v) out.push({ level: "warn", text: tr(`Vault ${i.vault} ist in der Zustandsdatei nicht bekannt.`, `Vault ${i.vault} is not known in the state file.`) });

  if (v?.stale)
    out.push({ level: "error", text: tr(`Vault ${v.index} wurde von Dritten verändert (zum Beispiel liquidiert). Aktionen sind gesperrt, bis ghostctl den Stand nachgeladen hat.`, `Vault ${v.index} was changed by a third party (for example liquidated). Actions are blocked until ghostctl has reloaded the state.`) });
  // Version 4: eingefrorenes Orakel sperrt alles, was den Preis braucht
  if (s?.oracle.frozen && frozenBlocks(i.action, v)) out.push({ level: "error", text: frozenText() });
  // Einzahlen und Tilgen nur mit Besitzer-Signatur (Audit V-04, Fix-Review N-4); Rücknahme darf jeder
  const ownerOnly = i.action === "mint" || i.action === "withdraw" || i.action === "close" || i.action === "deposit" || i.action === "repay";
  if (v && k && ownerOnly && v.owner.toLowerCase() !== k.xonly.toLowerCase())
    out.push({ level: "error", text: tr(`Vault ${v.index} gehört nicht zu ${k.file}. Nur der Besitzer darf das – der Vertrag prüft die Signatur.`, `Vault ${v.index} does not belong to ${k.file}. Only the owner may do this – the contract checks the signature.`) });

  switch (i.action) {
    case "open-vault": {
      out.push({
        level: "warn",
        text: tr(
          `Jeder neue Vault bindet zusätzlich ${MINTER_BRANCH_KAS} KAS für immer (Minter-Zweig). Die Sicherheit bekommst du beim Schließen zurück, diese ${MINTER_BRANCH_KAS} KAS nicht. Wenn du schon einen Vault hast, lieber dort einzahlen.`,
          `Every new vault additionally locks ${MINTER_BRANCH_KAS} KAS forever (minter branch). You get the collateral back when closing, but not these ${MINTER_BRANCH_KAS} KAS. If you already have a vault, rather deposit there.`,
        ),
      });
      // Mindestsicherheit: wie viel GHOST lässt sich damit prägen?
      const mcr = (s?.params.mcrPct ?? 200) / 100;
      const price = s?.oracle.kasUsd ?? null;
      if (amt !== null && amt > 0 && price !== null && price > 0) {
        const maxGhost = (amt * price) / mcr;
        const forOne = Math.ceil(mcr / price);
        if (maxGhost < 1)
          out.push({
            level: "warn",
            text: tr(
              `Mit ${fmt(amt, 4)} KAS kannst du höchstens ${fmt(maxGhost, 4)} GHOST prägen. Für 1 GHOST brauchst du beim jetzigen Kurs mindestens etwa ${forOne} KAS Sicherheit, mit Puffer gegen Kursschwankungen eher das Doppelte.`,
              `With ${fmt(amt, 4)} KAS you can mint at most ${fmt(maxGhost, 4)} GHOST. For 1 GHOST you need at least about ${forOne} KAS of collateral at the current price, rather twice that as a buffer against price swings.`,
            ),
          });
        else out.push({ level: "info", text: tr(`Damit kannst du höchstens etwa ${fmt(maxGhost, 4)} GHOST prägen (Quote ${fmt(mcr * 100, 0)} %).`, `This lets you mint at most about ${fmt(maxGhost, 4)} GHOST (ratio ${fmt(mcr * 100, 0)} %).`) });
      }
      if (k && k.kas !== null && amt !== null && amt + MINTER_BRANCH_KAS + FEE_RESERVE_KAS > k.kas)
        out.push({ level: "warn", text: tr(`Dafür braucht der Schlüssel etwa ${fmt(amt + MINTER_BRANCH_KAS + FEE_RESERVE_KAS, 4)} KAS (Sicherheit + 3 KAS Minter-Zweig + Gebühr), vorhanden sind ${fmt(k.kas, 4)} KAS.`, `This needs about ${fmt(amt + MINTER_BRANCH_KAS + FEE_RESERVE_KAS, 4)} KAS on the key (collateral + 3 KAS minter branch + fee); it has ${fmt(k.kas, 4)} KAS.`) });
      break;
    }
    case "mint":
      if (v && amt !== null && amt > v.maxMintGhost) {
        const cap = s?.params.maxDebtGhost ?? null;
        const byCap = cap !== null && v.debtGhost + amt > cap;
        out.push({
          level: "error",
          text: byCap
            ? tr(`Höchstens ${fmt(v.maxMintGhost)} GHOST prägbar: Je Vault sind höchstens ${fmt(cap)} GHOST Schuld erlaubt.`, `At most ${fmt(v.maxMintGhost)} GHOST can be minted: each vault may owe at most ${fmt(cap)} GHOST.`)
            : tr(`Höchstens ${fmt(v.maxMintGhost)} GHOST prägbar (Mindestquote ${s?.params.mcrPct ?? 200} %).`, `At most ${fmt(v.maxMintGhost)} GHOST can be minted (minimum ratio ${s?.params.mcrPct ?? 200} %).`),
        });
      }
      else if (v && amt !== null) out.push({ level: "info", text: tr(`Noch prägbar: ${fmt(v.maxMintGhost)} GHOST.`, `Still mintable: ${fmt(v.maxMintGhost)} GHOST.`) });
      if (k && k.kas !== null && k.kas < FEE_RESERVE_KAS + 1) out.push({ level: "warn", text: tr("Für Gebühr und Token-UTXO (etwa 1 KAS) braucht der Schlüssel etwas KAS.", "The key needs some KAS for the fee and the token UTXO (about 1 KAS).") });
      break;
    case "repay":
      if (v) {
        if (v.debtGhost <= 0) out.push({ level: "error", text: tr("Der Vault hat keine Schuld.", "The vault has no debt.") });
        else if (i.repayAll) {
          out.push({ level: "info", text: tr(`Schuld: ${fmt(v.debtGhost)} GHOST.`, `Debt: ${fmt(v.debtGhost)} GHOST.`) });
          if (k && k.ghost < v.debtGhost)
            out.push({ level: "error", text: tr(`Dem Schlüssel fehlen ${fmt(v.debtGhost - k.ghost)} GHOST für die volle Tilgung (vorhanden ${fmt(k.ghost)}).`, `The key is missing ${fmt(v.debtGhost - k.ghost)} GHOST for full repayment (has ${fmt(k.ghost)}).`) });
        } else if (amt !== null && i.amount !== null) {
          if (i.amount > units(v.debtGhost))
            out.push({ level: "error", text: tr(`Höchstens die Schuld (${fmt(v.debtGhost)} GHOST) lässt sich tilgen – mehr lehnt der Vertrag ab.`, `At most the debt (${fmt(v.debtGhost)} GHOST) can be repaid – the contract rejects more.`) });
          if (k && amt > k.ghost) out.push({ level: "error", text: tr(`Der Schlüssel hat nur ${fmt(k.ghost)} GHOST.`, `The key only has ${fmt(k.ghost)} GHOST.`) });
        }
        if ((v.interestUsd ?? 0) > 0)
          out.push({ level: "info", text: tr(`Der offene Zins (${fmt(v.interestUsd!, 4)} USD) bleibt stehen und wird beim Schließen in KAS bezahlt.`, `The open interest (${fmt(v.interestUsd!, 4)} USD) stays and is paid in KAS when closing.`) });
      }
      break;
    case "deposit":
      if (k && k.kas !== null && amt !== null && amt + FEE_RESERVE_KAS > k.kas)
        out.push({ level: "error", text: tr(`Der Schlüssel hat nur ${fmt(k.kas, 4)} KAS (plus Gebühr nötig).`, `The key only has ${fmt(k.kas, 4)} KAS (plus a fee is needed).`) });
      break;
    case "send":
      if (k && k.kas !== null && amt !== null && amt + FEE_RESERVE_KAS > k.kas)
        out.push({ level: "error", text: tr(`Der Schlüssel hat nur ${fmt(k.kas, 4)} KAS (plus Gebühr nötig).`, `The key only has ${fmt(k.kas, 4)} KAS (plus a fee is needed).`) });
      break;
    case "withdraw":
      if (v && s && i.amount !== null) {
        const keep = i.amount;
        if (keep >= units(v.collateralKas)) out.push({ level: "error", text: tr(`Die verbleibende Sicherheit muss kleiner als die jetzige sein (${fmt(v.collateralKas, 4)} KAS).`, `The remaining collateral must be less than the current one (${fmt(v.collateralKas, 4)} KAS).`) });
        else {
          // beide Ausgänge (Auszahlung und Vault-Rest) brauchen etwa 0,02 KAS
          const payout = units(v.collateralKas) - keep;
          if (payout < SMALL_OUTPUT) out.push(smallOutputHint(tr("Ausgezahlt", "The payout"), payout));
          if (keep > 0n && keep < SMALL_OUTPUT) out.push(smallOutputHint(tr("Im Vault blieben", "What remains in the vault"), keep));
        }
        const interest = v.interestUsd ?? 0;
        const min = minKeepSompi(v.debtGhost, s.oracle.kasUsd, s.params.mcrPct, interest);
        if (owedUnits(v) > 0n) {
          if (keep < min) out.push({ level: "error", text: tr(`Bei dieser Schuld${interest > 0 ? " samt Zins" : ""} müssen mindestens ${fmt(Number(min) / 1e8, 4)} KAS bleiben (Mindestquote ${s.params.mcrPct} %).`, `With this debt${interest > 0 ? " including interest" : ""} at least ${fmt(Number(min) / 1e8, 4)} KAS must remain (minimum ratio ${s.params.mcrPct} %).`) });
          const r = ratioAfterKeep(keep, v.debtGhost, s.oracle.kasUsd, interest);
          if (r !== null) out.push({ level: "info", text: tr(`Quote danach: ${fmt(r, 1)} %. Auszahlung: ${fmt(v.collateralKas - Number(keep) / 1e8, 4)} KAS.`, `Ratio afterwards: ${fmt(r, 1)} %. Payout: ${fmt(v.collateralKas - Number(keep) / 1e8, 4)} KAS.`) });
        } else out.push({ level: "info", text: tr("Ohne Schuld kannst du auch „Schließen“ nehmen – dann gehen alle KAS zurück.", "Without debt you can also use “Close” – then all KAS come back.") });
      }
      break;
    case "close":
      if (v && v.debtGhost > 0) out.push({ level: "error", text: tr(`Der Vault hat noch ${fmt(v.debtGhost)} GHOST Schuld. Erst vollständig tilgen.`, `The vault still owes ${fmt(v.debtGhost)} GHOST. Repay it fully first.`) });
      else if (v && s && (v.interestUsd ?? 0) > 0) {
        const coll = units(v.collateralKas);
        const f = feeAtOracle(v, s);
        if (!f) out.push(feeOverflowHint());
        if (f && f.waived)
          out.push({ level: "info", text: tr(`Offener Zins: ${fmt(v.interestUsd!, 4)} USD, das sind ${fmt(Number(f.fee) / 1e8, 4)} KAS – unter 0,2 KAS, wird erlassen. Alle ${fmt(v.collateralKas, 4)} KAS gehen an dich zurück.`, `Open interest: ${fmt(v.interestUsd!, 4)} USD, i.e. ${fmt(Number(f.fee) / 1e8, 4)} KAS – below 0.2 KAS, waived. All ${fmt(v.collateralKas, 4)} KAS go back to you.`) });
        else if (f)
          out.push({ level: "info", text: tr(`Zinsgebühr: ${fmt(Number(f.due) / 1e8, 4)} KAS an die Zinsadresse (offener Zins ${fmt(v.interestUsd!, 4)} USD zum Orakelpreis). An dich gehen etwa ${fmt(Number(coll - f.due) / 1e8, 4)} KAS.`, `Interest fee: ${fmt(Number(f.due) / 1e8, 4)} KAS to the interest address (open interest ${fmt(v.interestUsd!, 4)} USD at the oracle price). About ${fmt(Number(coll - f.due) / 1e8, 4)} KAS go to you.`) });
        if (f && coll - f.due > 0n && coll - f.due < SMALL_OUTPUT) out.push(smallOutputHint(tr("An dich gingen", "What goes to you"), coll - f.due));
        if (f && coll - f.due === 0n && !f.waived)
          out.push({ level: "info", text: tr("Der Zins zehrt die ganze Sicherheit auf, an dich geht nichts.", "The interest eats up all the collateral; nothing goes to you.") });
      } else if (v && v.debtGhost <= 0 && units(v.collateralKas) < SMALL_OUTPUT) out.push(smallOutputHint(tr("An dich gingen", "What goes to you"), units(v.collateralKas)));
      break;
    case "sweep":
      if (v && s) {
        if (v.debtGhost > 0) {
          out.push({ level: "error", text: tr("Der Vault hat noch Schuld. Auflösen geht nur ohne Schuld.", "The vault still has debt. Dissolving only works without debt.") });
          break;
        }
        if (tooSmallToSweep(v)) {
          out.push({
            level: "error",
            text: tr(
              `Die Sicherheit (${fmt(v.collateralKas, 8)} KAS) ist zu klein zum Auflösen: Vorab gehen ${fmt(Number(SWEEP_FEE) / 1e8)} KAS für das Auflösen ab (die Netzgebühr, den Rest bekommt, wer auflöst). Danach bliebe für die Zinsadresse zu wenig für einen eigenen Ausgang. Der Vault bleibt liegen.`,
              `The collateral (${fmt(v.collateralKas, 8)} KAS) is too small to dissolve: ${fmt(Number(SWEEP_FEE) / 1e8)} KAS go to dissolving first (the network fee, the rest goes to whoever dissolves). Too little would remain for an output to the interest address. The vault stays as it is.`,
            ),
          });
          break;
        }
        if ((v.interestUsd ?? 0) > 0 && !feeAtOracle(v, s)) {
          out.push(feeOverflowHint());
          break;
        }
        if (!vaultSweepable(v, s)) {
          out.push({
            level: "error",
            text: tr(
              "Der offene Zins ist kleiner als die Sicherheit. Diesen Vault kann nur sein Besitzer schließen.",
              "The open interest is smaller than the collateral. Only the owner can close this vault.",
            ),
          });
          break;
        }
        const coll = units(v.collateralKas);
        out.push({
          level: "info",
          text: tr(
            `${fmt(Number(coll - SWEEP_FEE) / 1e8, 4)} KAS gehen an die Zinsadresse, der Vault endet. Der offene Zins (${fmt(v.interestUsd ?? 0, 4)} USD) ist mehr wert als die Sicherheit.`,
            `${fmt(Number(coll - SWEEP_FEE) / 1e8, 4)} KAS go to the interest address and the vault ends. The open interest (${fmt(v.interestUsd ?? 0, 4)} USD) is worth more than the collateral.`,
          ),
        });
      }
      if (k && k.kas !== null && k.kas < FEE_RESERVE_KAS)
        out.push({ level: "warn", text: tr(`Für die Netzgebühr braucht der Schlüssel etwa ${fmt(FEE_RESERVE_KAS, 2)} KAS.`, `The key needs about ${fmt(FEE_RESERVE_KAS, 2)} KAS for the network fee.`) });
      break;
    case "liquidate":
      if (v && s) {
        if (v.ratioPct === null || v.debtGhost <= 0) {
          out.push({ level: "error", text: tr("Keine Schuld – nichts zu liquidieren.", "No debt – nothing to liquidate.") });
          break;
        }
        if (v.ratioPct >= s.params.liqPct)
          out.push({ level: "error", text: tr(`Quote ${fmt(v.ratioPct, 1)} % liegt über ${s.params.liqPct} % – nicht liquidierbar.`, `Ratio ${fmt(v.ratioPct, 1)} % is above ${s.params.liqPct} % – not liquidatable.`) });
        const debt = units(v.debtGhost);
        const burn = i.liquidateAll !== false ? debt : i.amount;
        if (burn === null) break;
        if (burn > debt) {
          out.push({ level: "error", text: tr(`Höchstens die ganze Schuld (${fmt(v.debtGhost)} GHOST) kann verbrannt werden.`, `At most the whole debt (${fmt(v.debtGhost)} GHOST) can be burned.`) });
          break;
        }
        if (k && Number(burn) / 1e8 > k.ghost)
          out.push({ level: "error", text: tr(`Du brauchst ${fmt(Number(burn) / 1e8)} GHOST, der Schlüssel hat ${fmt(k.ghost)}.`, `You need ${fmt(Number(burn) / 1e8)} GHOST; the key has ${fmt(k.ghost)}.`) });
        const p = liquidationPreview(
          units(v.collateralKas),
          BigInt(Math.round(s.oracle.kasUsd * 1e8)),
          debt,
          burn,
          BigInt(Math.round(s.params.bonusPct * 100)),
        );
        out.push({
          level: "info",
          text: tr(`Du erhältst etwa ${fmt(Number(p.seize) / 1e8, 4)} KAS (Wert von ${fmt(Number(burn) / 1e8)} GHOST plus ${s.params.bonusPct} % Bonus, beim aktuellen Orakelpreis).`, `You receive about ${fmt(Number(p.seize) / 1e8, 4)} KAS (worth ${fmt(Number(burn) / 1e8)} GHOST plus a ${s.params.bonusPct} % bonus, at the current oracle price).`),
        });
        if (!p.allowed)
          out.push({
            level: "error",
            text: tr("Danach bliebe weniger als 0,2 KAS im Vault – das ist nur erlaubt, wenn die ganze Schuld verbrannt wird. Bitte „Ganze Schuld“ wählen oder weniger liquidieren.", "Less than 0.2 KAS would remain in the vault – that is only allowed if the whole debt is burned. Please choose “Whole debt” or liquidate less."),
          });
        else if (p.ends && p.writtenOff > 0n)
          out.push({
            level: "warn",
            text: tr(`Die Sicherheit reicht nicht (Deckung unter etwa ${100 + s.params.bonusPct} %): Du bekommst die ganze Sicherheit, der Vault endet, und ${fmt(Number(p.writtenOff) / 1e8)} GHOST Restschuld werden ausgebucht (uneinbringlich).`, `The collateral is not enough (coverage below about ${100 + s.params.bonusPct} %): you receive all of it, the vault ends, and ${fmt(Number(p.writtenOff) / 1e8)} GHOST of remaining debt are written off (unrecoverable).`),
          });
        else if (p.ends) out.push({ level: "info", text: tr("Danach bliebe weniger als 0,2 KAS: Der Rest geht mit an dich, der Vault endet.", "Less than 0.2 KAS would remain: the rest goes to you as well and the vault ends.") });
      }
      break;
    case "redeem":
      if (v && s) {
        if (v.debtGhost <= 0 || v.ratioPct === null) {
          out.push({ level: "error", text: tr("Dieser Vault hat keine Schuld – hier lässt sich nichts zurückgeben.", "This vault has no debt – nothing can be redeemed here.") });
          break;
        }
        if (v.ratioPct < s.params.liqPct)
          out.push({ level: "error", text: tr(`Quote ${fmt(v.ratioPct, 1)} % liegt unter ${s.params.liqPct} %. Dort ist keine Rücknahme möglich, nur Liquidation.`, `Ratio ${fmt(v.ratioPct, 1)} % is below ${s.params.liqPct} %. Redemption is not possible there, only liquidation.`) });
        if (i.amount === null) break;
        if (i.amount <= 0n) {
          out.push({ level: "error", text: tr("Der Betrag muss größer als 0 sein.", "The amount must be greater than 0.") });
          break;
        }
        const debt = units(v.debtGhost);
        if (i.amount > debt)
          out.push({ level: "error", text: tr(`Höchstens die Schuld dieses Vaults (${fmt(v.debtGhost)} GHOST) lässt sich zurückgeben.`, `At most this vault's debt (${fmt(v.debtGhost)} GHOST) can be redeemed.`) });
        else if (i.amount < MIN_REDEEM && i.amount !== debt)
          out.push({
            level: "error",
            text: tr(
              `Zurückgeben lässt sich mindestens 1 GHOST oder die ganze Schuld dieses Vaults (${fmt(v.debtGhost)} GHOST).`,
              `You can redeem at least 1 GHOST or this vault's whole debt (${fmt(v.debtGhost)} GHOST).`,
            ),
          });
        if (k && amt !== null && amt > k.ghost)
          out.push({ level: "error", text: tr(`Du brauchst ${fmt(amt)} GHOST, der Schlüssel hat ${fmt(k.ghost)}.`, `You need ${fmt(amt)} GHOST; the key has ${fmt(k.ghost)}.`) });
        const price = BigInt(Math.round(s.oracle.kasUsd * 1e8));
        const fee = redeemFeeBps(s);
        const coll = units(v.collateralKas);
        let paid: bigint;
        try {
          paid = redeemPayout(i.amount, price, fee);
        } catch {
          break; // Überlauf außerhalb der Vertragsgrenzen: ghostctl entscheidet
        }
        if (paid <= 0n) {
          out.push({ level: "error", text: tr("Der Betrag ist zu klein für eine Auszahlung.", "The amount is too small for a payout.") });
          break;
        }
        out.push({
          level: "info",
          text: tr(
            `Du erhältst etwa ${fmt(Number(paid) / 1e8, 4)} KAS (1 USD je GHOST minus ${fmt(Number(fee) / 100)} %, beim aktuellen Orakelpreis).`,
            `You receive about ${fmt(Number(paid) / 1e8, 4)} KAS (1 USD per GHOST minus ${fmt(Number(fee) / 100)} %, at the current oracle price).`,
          ),
        });
        // winzige Auszahlung (Audit 12 A12-18): sie fließt ins Wechselgeld, ist
        // also kein eigener, zu schwerer Ausgang, aber weniger als die Netzgebühr
        if (paid < units(FEE_RESERVE_KAS))
          out.push({
            level: "warn",
            text: tr(
              `Die Rücknahme zahlt nur ${fmt(Number(paid) / 1e8, 4)} KAS aus, die Netzgebühr dafür liegt bei etwa ${fmt(FEE_RESERVE_KAS, 2)} KAS. Unterm Strich zahlst du womöglich drauf.`,
              `The redemption pays out only ${fmt(Number(paid) / 1e8, 4)} KAS, while its network fee is about ${fmt(FEE_RESERVE_KAS, 2)} KAS. You may end up paying more than you get.`,
            ),
          });
        if (coll - paid < DUST) {
          const max = maxRedeemable(coll, debt, price, fee);
          out.push({
            level: "error",
            text:
              max > 0n
                ? tr(
                    `Im Vault müssen mindestens 0,2 KAS bleiben. An diesem Vault gehen höchstens ${fmt(Number(max) / 1e8)} GHOST.`,
                    `At least 0.2 KAS must remain in the vault. At most ${fmt(Number(max) / 1e8)} GHOST are possible at this vault.`,
                  )
                : tr(
                    "Im Vault müssen mindestens 0,2 KAS bleiben. An diesem Vault ist keine Rücknahme möglich.",
                    "At least 0.2 KAS must remain in the vault. No redemption is possible at this vault.",
                  ),
          });
        }
      }
      if (k && k.kas !== null && k.kas < FEE_RESERVE_KAS + 1) out.push({ level: "warn", text: tr("Für Gebühr und Token-UTXO (etwa 1 KAS) braucht der Schlüssel etwas KAS.", "The key needs some KAS for the fee and the token UTXO (about 1 KAS).") });
      break;
    case "transfer":
      if (k && amt !== null && amt > k.ghost) out.push({ level: "error", text: tr(`Der Schlüssel hat nur ${fmt(k.ghost)} GHOST.`, `The key only has ${fmt(k.ghost)} GHOST.`) });
      break;
    case "pool-open":
    case "pool-add": {
      const g = i.amount2 ?? null;
      if (k && g !== null && g > units(k.ghost)) out.push({ level: "error", text: tr(`Der Schlüssel hat nur ${fmt(k.ghost)} GHOST.`, `The key only has ${fmt(k.ghost)} GHOST.`) });
      if (i.action === "pool-open") {
        if (s?.pool && s.pool.bandBps != null) out.push({ level: "error", text: tr("In diesem Netz gibt es schon einen Pool – bitte „Einlegen“.", "There is already a pool on this network – please use “Add”.") });
        else if (s?.pool && k?.lpShares && BigInt(k.lpShares) > 0n)
          out.push({ level: "error", text: tr("Der bisherige Pool hat noch kein Kursband. Zuerst deine Anteile dort abziehen (100 %), dann den neuen Pool anlegen.", "The current pool has no price band yet. First remove your shares there (100 %), then create the new pool.") });
        else if (s?.pool)
          out.push({ level: "info", text: tr("Ersetzt den bisherigen Pool ohne Kursband. Der neue hält GHOST bei 1 USD ± 3 %.", "Replaces the current pool without price band. The new one keeps GHOST at 1 USD ± 3 %.") });
        if (i.amount !== null && i.amount < POOL_MIN_KAS) out.push({ level: "error", text: tr("Der Pool braucht mindestens 1 KAS.", "The pool needs at least 1 KAS.") });
        else if (i.amount !== null && g !== null && g > 0n && s)
          out.push({
            level: "info",
            text: tr(`Startkurs: 1 GHOST = ${fmt(Number(i.amount) / Number(g), 4)} KAS, also ${fmt((Number(i.amount) / Number(g)) * s.oracle.kasUsd, 4)} USD je GHOST (Orakel ${fmt(s.oracle.kasUsd, 6)} USD je KAS). 1 KAS und ${fmt(Number((g * POOL_MIN_KAS) / i.amount) / 1e8)} GHOST bleiben für immer im Pool.`, `Starting price: 1 GHOST = ${fmt(Number(i.amount) / Number(g), 4)} KAS, i.e. ${fmt((Number(i.amount) / Number(g)) * s.oracle.kasUsd, 4)} USD per GHOST (oracle ${fmt(s.oracle.kasUsd, 6)} USD per KAS). 1 KAS and ${fmt(Number((g * POOL_MIN_KAS) / i.amount) / 1e8)} GHOST stay in the pool forever.`),
          });
        // Kursband: der Startkurs muss bei 1 USD ± 3 % liegen (Vertrag: init)
        if (i.amount !== null && g !== null && g > 0n && s && i.amount >= POOL_MIN_KAS) {
          const usd = (Number(i.amount) / Number(g)) * s.oracle.kasUsd;
          if (Math.abs(usd - 1) > 0.03) {
            const fitG = Number(i.amount) / 1e8 * s.oracle.kasUsd;
            out.push({
              level: "error",
              text: tr(
                `Der Startkurs muss bei 1 USD ± 3 % liegen (Orakelpreis). Zu ${fmt(Number(i.amount) / 1e8)} KAS passen etwa ${fmt(fitG, 8)} GHOST.`,
                `The starting price must be 1 USD ± 3 % (oracle price). ${fmt(Number(i.amount) / 1e8)} KAS match about ${fmt(fitG, 8)} GHOST.`,
              ),
            });
          }
        }
        break;
      }
      const p = s?.pool;
      if (!p) {
        out.push({ level: "error", text: tr("In diesem Netz gibt es noch keinen Pool.", "There is no pool on this network yet.") });
        break;
      }
      if (i.amount !== null && g !== null && i.amount > 0n && g > 0n) {
        const [x, y, sh] = [BigInt(p.kasSompi), BigInt(p.ghostUnits), BigInt(p.shares)];
        const m = sharesForDeposit(sh, x, y, i.amount, g);
        if (m <= 0n) out.push({ level: "error", text: tr("Die Einlage ist zu klein für einen Anteil.", "The deposit is too small for a share.") });
        else {
          const needG = (i.amount * y) / x;
          const needK = (g * x) / y;
          out.push({ level: "info", text: tr(`Du bekommst ${m.toString()} Anteile, ${fmt((Number(m) / Number(sh + m)) * 100, 4)} % des Pools.`, `You receive ${m.toString()} shares, ${fmt((Number(m) / Number(sh + m)) * 100, 4)} % of the pool.`) });
          // ghostctl nimmt nur den passenden Teil und bricht ab, wenn der Poolkurs
          // um mehr als 1 % vom eingegebenen Verhältnis abweicht (Audit 10, A10-P-1)
          const off = (have: bigint, need: bigint) => have > 0n && (have - need) * 10_000n > have * 100n;
          if (g > needG && off(g, needG))
            out.push({ level: "error", text: tr(`Zum Kurs des Pools passen zu ${fmt(Number(i.amount) / 1e8)} KAS etwa ${fmt(Number(needG) / 1e8)} GHOST. Bei mehr als 1 % Abweichung bricht ghostctl ab – GHOST-Betrag anpassen.`, `At the pool price, ${fmt(Number(i.amount) / 1e8)} KAS match about ${fmt(Number(needG) / 1e8)} GHOST. With more than 1 % deviation ghostctl aborts – adjust the GHOST amount.`) });
          else if (i.amount > needK && off(i.amount, needK))
            out.push({ level: "error", text: tr(`Zum Kurs des Pools passen zu ${fmt(Number(g) / 1e8)} GHOST etwa ${fmt(Number(needK) / 1e8)} KAS. Bei mehr als 1 % Abweichung bricht ghostctl ab – KAS-Betrag anpassen.`, `At the pool price, ${fmt(Number(g) / 1e8)} GHOST match about ${fmt(Number(needK) / 1e8)} KAS. With more than 1 % deviation ghostctl aborts – adjust the KAS amount.`) });
          else if (g > needG + 1n || i.amount > needK + 1n)
            out.push({ level: "info", text: tr("Genommen wird nur der zum Poolkurs passende Teil; der kleine Rest bleibt bei dir.", "Only the part matching the pool price is taken; the small remainder stays with you.") });
          // Poolkurs weit weg vom Orakel: jemand könnte ihn verschoben haben – wer
          // dazu passend einlegt, verliert, sobald der Kurs zurückgeholt wird
          const ghostUsd = (Number(x) / Number(y)) * s!.oracle.kasUsd;
          if (Math.abs(ghostUsd - 1) > 0.05)
            out.push({ level: "warn", text: tr(`Der Pool bewertet 1 GHOST mit ${fmt(ghostUsd, 4)} USD (Orakel). Weicht das stark von 1 USD ab, kann der Kurs verschoben sein – wer jetzt einlegt, verliert, wenn er zurückkommt.`, `The pool values 1 GHOST at ${fmt(ghostUsd, 4)} USD (oracle). If that is far from 1 USD, the price may have been pushed – depositing now loses when it comes back.`) });
        }
      }
      break;
    }
    case "pool-remove": {
      const p = s?.pool;
      const have = k?.lpShares ? BigInt(k.lpShares) : 0n;
      if (!p) {
        out.push({ level: "error", text: tr("In diesem Netz gibt es keinen Pool.", "There is no pool on this network.") });
        break;
      }
      if (have === 0n) {
        out.push({ level: "error", text: tr("Dieser Schlüssel hat keine Pool-Anteile.", "This key has no pool shares.") });
        break;
      }
      if (i.amount !== null) {
        if (i.amount > 100n * 100_000_000n || i.amount <= 0n) {
          out.push({ level: "error", text: tr("Bitte mehr als 0 bis 100 % angeben.", "Please enter more than 0 up to 100 %.") });
          break;
        }
        const m = (have * i.amount) / (100n * 100_000_000n);
        const [x, y, sh] = [BigInt(p.kasSompi), BigInt(p.ghostUnits), BigInt(p.shares)];
        const [dx, dy] = payoutFor(sh, x, y, m > 0n ? m : 1n);
        if (x - dx < POOL_MIN_KAS) out.push({ level: "error", text: tr("So viel lässt der Pool nicht abziehen: Mindestens 1 KAS bleibt. Bitte weniger Prozent.", "The pool does not allow removing that much: at least 1 KAS stays. Please use a lower percentage.") });
        else out.push({ level: "info", text: tr(`Auszahlung etwa ${fmt(Number(dx) / 1e8)} KAS und ${fmt(Number(dy) / 1e8)} GHOST (${m.toString()} von ${have.toString()} Anteilen).`, `Payout about ${fmt(Number(dx) / 1e8)} KAS and ${fmt(Number(dy) / 1e8)} GHOST (${m.toString()} of ${have.toString()} shares).`) });
      }
      break;
    }
    case "oracle-update": {
      const cur = s?.oracle.kasUsd ?? null;
      // ghostctl rechnet mit 20 DAA Sicherheitsabstand zur Locktime, also 620 DAA
      // (Fix-Review 8 NEU-2). daa = 0 heißt: Abfrage fehlgeschlagen, nicht „zu frisch“.
      if (s && s.daa > 0 && s.oracle.ageMinutes >= 0 && s.oracle.ageMinutes < 620 / 600)
        out.push({
          level: "error",
          text: tr("Der Vertrag verlangt seit Version 2.1 mindestens 600 DAA (etwa 1 Minute) Abstand zum letzten Update. Kurz warten und neu laden.", "Since version 2.1 the contract requires at least 600 DAA (about 1 minute) since the last update. Wait briefly and reload."),
        });
      if (i.usd !== undefined && i.usd !== null) {
        const usd = Number(i.usd) / 1e8;
        if (usd < ORACLE_MIN_USD || usd > ORACLE_MAX_USD)
          out.push({ level: "error", text: tr(`Der Vertrag erlaubt nur Preise von 0,00001 bis 900 USD je KAS.`, `The contract only allows prices from 0.00001 to 900 USD per KAS.`) });
        else if (cur !== null && (usd * 2 < cur || usd > cur * 2))
          out.push({
            level: "error",
            text: tr(`Je Update darf sich der Preis höchstens verdoppeln oder halbieren (jetzt ${fmt(cur)} USD, also ${fmt(cur / 2)} bis ${fmt(cur * 2)} USD). Größere Änderungen in Schritten aktualisieren.`, `Each update may at most double or halve the price (now ${fmt(cur)} USD, so ${fmt(cur / 2)} to ${fmt(cur * 2)} USD). Update larger changes in steps.`),
          });
      } else {
        out.push({
          level: "info",
          text: tr("Ohne festen Preis nimmt ghostctl den Median der Preisquellen. Springt er über ×2 bzw. ÷2, lehnt ghostctl ab – dann in Schritten aktualisieren. Der Dauerbetrieb sendet Sprünge über 20 % erst, wenn sie über 3 Runden bestehen, und dann schrittweise.", "Without a fixed price ghostctl uses the median of the price sources. If it jumps beyond ×2 or ÷2, ghostctl refuses – then update in steps. The continuous service sends jumps above 20 % only after they persist for 3 rounds, and then in steps."),
        });
      }
      if (i.rateBps !== undefined && i.rateBps !== null) {
        out.push({ level: "info", text: tr("Normalerweise passt der GHOST-Agent den Zins selbst an den GHOST-Kurs an (Median der letzten Stunde, Totzone ±3 % um 1 USD, nur bei Handel im Pool): höchstens einmal pro Stunde, in Schritten von 0,5 Prozentpunkten, zwischen 2 % (Grundzins) und 20 % p. a. Einen Satz von Hand nur in Ausnahmefällen setzen.", "Normally the GHOST agent adjusts the rate to the GHOST price by itself (median of the last hour, dead zone ±3 % around 1 USD, only when the pool was traded): at most once per hour, in steps of 0.5 percentage points, between 2 % (base rate) and 20 % p.a. Only set a rate by hand in exceptional cases.") });
        if (i.rateBps > 2_000n)
          out.push({ level: "warn", text: tr("Das liegt über dem Rahmen des GHOST-Agenten (0–20 % p. a.).", "That is above the GHOST agent's range (0–20 % p.a.).") });
      }
      break;
    }
  }
  return out;
}
