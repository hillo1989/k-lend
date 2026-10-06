// Rechenkern der Oberfläche – Spiegel von contracts/stable_vault.sil (Version 3)
// (Block MATH-BEGIN … MATH-END, accrued() und healthy() sowie die Einträge
// deposit, withdraw, close, sweep, mint, repay, redeem, liquidate) und der
// Index-Fortschreibung aus contracts/risk_oracle.sil (entry update).
//
// Die Formeln sind WÖRTLICH übernommen, inklusive der Zerlegung, die der
// Vertrag gegen 64-Bit-Überlauf braucht. Jede Zwischenrechnung läuft über
// i64(), das wie die Skript-Engine abbricht (NumberTooBig), statt mit einem
// umgebrochenen Wert weiterzurechnen. So liefert die Oberfläche genau das
// Ergebnis, das der Vertrag erzwingen würde – nicht nur ein ähnliches.
//
// Version 3: Je Vault gibt es debt (geprägte, noch nicht getilgte GHOST; wächst
// NICHT mit dem Zins), interest (aufgelaufener Zins in USD × 1e8) und indexAt
// (Orakelindex, bis zu dem abgerechnet ist). Für die Quoten zählt debt + Zins.
// Der offene Zins verzinst sich mit (Zinseszins, Audit 11 A11-V-5).
//
// Wenn sich stable_vault.sil ändert, muss diese Datei nachgezogen werden;
// die Tests in vaultMath.test.ts vergleichen gegen exakte BigInt-Rechnung.
//
// Einheiten: sompi (1e8 je KAS), GHOST-Einheiten (1e8 je GHOST),
// USD × 1e8 (1 GHOST ≙ 1 USD), kasUsd = USD je KAS × 1e8,
// stableIndex × 1e9, Quoten in bps.

import { tr } from "./i18n";

export const BPS = 10_000n;
export const PRICE_SCALE = 100_000_000n;
export const INDEX_SCALE = 1_000_000_000n;
export const UNIT = 100_000_000n; // 1 KAS in sompi bzw. 1 GHOST in Einheiten
/** Zinswachstum je Abrechnung höchstens Index ×10 (growth ≤ 9·1e9) */
export const MAX_GROWTH = 9_000_000_000n;
export const MAX_COLLATERAL = 10_000_000_000_000_000n; // 1e8 KAS
export const MAX_DEBT = 100_000_000_000_000_000n; // 1e9 GHOST
export const MAX_INTEREST = 100_000_000_000_000_000n; // 1e9 USD
/** Rücknahme: 1 % bleibt als Ausgleich beim Vault-Besitzer (über der Nachführschwelle des Orakels von 0,5 %) */
export const REDEEM_FEE_BPS = 100n;
/** Kleinste Rücknahme: 1 GHOST, außer es ist die ganze Schuld des Vaults */
export const MIN_REDEEM = 100_000_000n;
export const DUST = 20_000_000n; // 0,2 KAS
/**
 * Auflösen zugunsten der Zinsadresse (sweep): so viel trägt der Vault (0,1 KAS).
 * Die Netzgebühr ist kleiner (gemessen ≈ 0,055 KAS), den Rest darf der
 * Auslöser als Wechselgeld behalten (Anreiz für jeden Agenten).
 */
export const SWEEP_FEE = 10_000_000n;
/**
 * Auflösen: so viel muss nach SWEEP_FEE mindestens an die Zinsadresse gehen, sonst
 * ist deren Ausgang für das Netz zu schwer (gemessen baubar ab ≈ 0,016–0,021 KAS,
 * mit Abstand 0,025 KAS; wie math::SWEEP_MIN_TREASURY in ghostctl, Audit 12 A12-2).
 */
export const SWEEP_MIN_TREASURY = 2_500_000n;
/**
 * Kleinster sinnvoller Auszahlungs-Ausgang: Unter etwa 0,02 KAS ist die
 * Speichermasse eines Ausgangs (≈ 10^12 / Betrag in sompi Gramm) größer als
 * ein Block erlaubt (500 000 g). Mit Abstand zur Grenze: 0,025 KAS.
 */
export const SMALL_OUTPUT = 2_500_000n;
/** stableRate im Orakel: Zins je DAA × 1e18, geteilt wird zweimal durch 1e9. */
export const RATE_SCALE = 1_000_000_000n;

const I64_MAX = 9_223_372_036_854_775_807n;
const I64_MIN = -9_223_372_036_854_775_808n;

export class OverflowError extends Error {
  constructor() {
    super(tr("64-Bit-Überlauf: Der Vertrag würde hier abbrechen (NumberTooBig).", "64-bit overflow: the contract would abort here (NumberTooBig)."));
    this.name = "OverflowError";
  }
}

/** Wert muss in ein vorzeichenbehaftetes 64-Bit-int passen (SilverScript `int`). */
export function i64(x: bigint): bigint {
  if (x > I64_MAX || x < I64_MIN) throw new OverflowError();
  return x;
}
const mul = (a: bigint, b: bigint) => i64(a * b);
const add = (a: bigint, b: bigint) => i64(a + b);
const sub = (a: bigint, b: bigint) => i64(a - b);

// ------------------------------------------------ MATH-BEGIN (Vertrag) ----

/** floor(a·b/d) – stable_vault.sil mulDivDown */
export function mulDivDown(a: bigint, b: bigint, d: bigint): bigint {
  return add(mul(a / d, b), mul(a % d, b) / d);
}

/** ceil(a·b/d) – stable_vault.sil mulDivUp */
export function mulDivUp(a: bigint, b: bigint, d: bigint): bigint {
  const rest = mul(a % d, b);
  return add(mul(a / d, b), sub(add(rest, d), 1n) / d);
}

/**
 * Zinswachstum vom Index `from` bis `to` × 1e9: ⌈(to − from)·1e9/from⌉,
 * gedeckelt auf MAX_GROWTH (Index ×10) – stable_vault.sil growth
 */
export function growth(from: bigint, to: bigint): bigint {
  const delta = sub(to, from);
  const q = delta / from;
  let g = MAX_GROWTH;
  if (q < 9n) {
    const r = delta % from;
    const t = mul(r, 1000n);
    const t1 = t / from;
    const t2 = t % from;
    const low = mul(t2, 1_000_000n);
    const up = low % from > 0n ? 1n : 0n;
    g = add(add(add(mul(q, INDEX_SCALE), mul(t1, 1_000_000n)), low / from), up);
    if (g > MAX_GROWTH) g = MAX_GROWTH;
  }
  return g;
}

/** Zins auf die Schuld d vom Index `from` bis `to`: ⌈d·growth/1e9⌉ – stable_vault.sil accrual */
export function accrual(d: bigint, from: bigint, to: bigint): bigint {
  if (d > 0n && to > from && from > 0n) return mulDivUp(d, growth(from, to), INDEX_SCALE);
  return 0n;
}

// -------------------------------------------------------------- MATH-END ----

export interface VaultState {
  /** KAS-Betrag der Vault-UTXO in sompi */
  collateral: bigint;
  /** geprägte, noch nicht getilgte GHOST (Einheiten) */
  debt: bigint;
  /** aufgelaufener Zins in USD × 1e8, abgerechnet bis indexAt */
  interest: bigint;
  /** Orakelindex × 1e9, bis zu dem der Zins abgerechnet ist */
  indexAt: bigint;
}

export interface OracleView {
  kasUsd: bigint;
  stableIndex: bigint;
}

/** Zins bis zum Orakelindex `index` (USD × 1e8): Schuld und offener Zins wachsen mit – stable_vault.sil accrued */
export function accrued(v: VaultState, index: bigint): bigint {
  return add(v.interest, accrual(add(v.debt, v.interest), v.indexAt, index));
}

/** Was für die Quoten zählt: Schuld + Zins bis `index` (USD × 1e8) */
export function owedOf(v: VaultState, index: bigint): bigint {
  return add(v.debt, accrued(v, index));
}

/** Sicherheit·Preis ≥ (Schuld + Zins)·Quote (nichts offen ist immer gesund) – stable_vault.sil healthy */
export function healthy(coll: bigint, owed: bigint, kasUsd: bigint, ratioBps: bigint): boolean {
  // Wie im Vertrag werden beide Seiten berechnet (ein Überlauf bricht also
  // auch bei owed == 0 ab), erst dann greift das ||.
  const value = mulDivDown(coll, kasUsd, PRICE_SCALE);
  const need = mulDivUp(owed, ratioBps, BPS);
  return owed === 0n || value >= need;
}

/** Index-Fortschreibung wie risk_oracle.sil update(): für Δ gilt der ALTE Satz, abgerundet. */
export function oracleIndexAfter(index: bigint, stableRate: bigint, deltaDaa: bigint): bigint {
  const g = mul(stableRate, deltaDaa) / RATE_SCALE;
  return add(index, mul(index, g) / RATE_SCALE);
}

/** Rücknahme: KAS (sompi) für `amount` GHOST – 1 USD je GHOST abzüglich feeBps, beide Schritte abgerundet */
export function redeemPayout(amount: bigint, kasUsd: bigint, feeBps: bigint = REDEEM_FEE_BPS): bigint {
  const usd = mulDivDown(amount, BPS - feeBps, BPS);
  return mulDivDown(usd, PRICE_SCALE, kasUsd);
}

/**
 * Zinsgebühr beim Schließen: ⌈Zins·1e8/kasUsd⌉ sompi, höchstens die Sicherheit.
 * Unter 0,2 KAS wird sie erlassen (waived); `due` ist, was tatsächlich an die Zinsadresse geht.
 */
export function closeFee(interest: bigint, kasUsd: bigint, collateral?: bigint): { fee: bigint; waived: boolean; due: bigint } {
  let fee = mulDivUp(interest, PRICE_SCALE, kasUsd);
  if (collateral !== undefined && fee > collateral) fee = collateral;
  const waived = fee < DUST;
  return { fee, waived, due: waived ? 0n : fee };
}

/** Zinsgebühr in sompi zum Orakelpreis (aufgerundet, höchstens die Sicherheit) – stable_vault.sil interestFee */
export function interestFee(v: VaultState, o: OracleView): bigint {
  const fee = mulDivUp(accrued(v, o.stableIndex), PRICE_SCALE, o.kasUsd);
  return fee > v.collateral ? v.collateral : fee;
}

/**
 * Lässt stable_vault.sil sweep den Vault zu? Schuld 0 und der Zins zehrt die ganze
 * Sicherheit auf. Läuft interestFee über 64 Bit (nur bei extremem Zins zum
 * Tiefstpreis), bricht auch der Vertrag ab: nicht auflösbar (Audit 12 A12-14).
 */
export function sweepAllowed(v: VaultState, o: OracleView): boolean {
  try {
    return v.debt === 0n && interestFee(v, o) === v.collateral;
  } catch (e) {
    if (e instanceof OverflowError) return false;
    throw e;
  }
}

/** Wie sweepAllowed, und die Auflösung lässt sich bauen: nach SWEEP_FEE bleiben mindestens SWEEP_MIN_TREASURY für die Zinsadresse (A12-2) */
export function sweepable(v: VaultState, o: OracleView): boolean {
  return v.collateral >= SWEEP_FEE + SWEEP_MIN_TREASURY && sweepAllowed(v, o);
}

// ----------------------------------------------------- Einträge (Simulation)

export type EntryResult =
  | {
      ok: true;
      state: VaultState | null;
      note?: string;
      /** Liquidation: KAS an den Liquidator */
      seized?: bigint;
      burned?: bigint;
      /** Liquidation: ausgebuchte Restschuld (GHOST) */
      writtenOff?: bigint;
      /** Rücknahme: KAS an den Rücknehmer */
      paid?: bigint;
      /** Schließen/Auflösen: Zinsgebühr an die Zinsadresse (0 = erlassen) und Auszahlung an den Besitzer */
      fee?: bigint;
      payout?: bigint;
    }
  | { ok: false; error: string };

const fail = (error: string): EntryResult => ({ ok: false, error });
const belowMcr = () =>
  fail(tr("Danach läge die Quote unter der Mindestquote – der Vertrag lehnt das ab.", "This would put the ratio below the minimum ratio – the contract rejects it."));

/** continueWith(): Grenzen, die der Vertrag an jede Fortsetzung stellt. */
function continueWith(value: bigint, newDebt: bigint, newInterest: bigint, newIndexAt: bigint): EntryResult {
  if (!(value > 0n))
    return fail(tr("Die Sicherheit muss größer als 0 bleiben. Zum Auflösen „Schließen“ verwenden.", "Collateral must stay above 0. Use “Close” to unwind."));
  if (value > MAX_COLLATERAL)
    return fail(tr("Mehr als 100 Mio. KAS je Vault sind nicht vorgesehen (MAX_COLLATERAL).", "More than 100 million KAS per vault is not supported (MAX_COLLATERAL)."));
  if (newDebt < 0n) return fail(tr("Negative Schuld ist unmöglich.", "Negative debt is impossible."));
  if (newDebt > MAX_DEBT) return fail(tr("Obergrenze für die Schuld überschritten (MAX_DEBT).", "Debt limit exceeded (MAX_DEBT)."));
  if (newInterest < 0n) return fail(tr("Negativer Zins ist unmöglich.", "Negative interest is impossible."));
  if (newInterest > MAX_INTEREST) return fail(tr("Obergrenze für den Zins überschritten (MAX_INTEREST).", "Interest limit exceeded (MAX_INTEREST)."));
  return { ok: true, state: { collateral: value, debt: newDebt, interest: newInterest, indexAt: newIndexAt } };
}

function guard(fn: () => EntryResult): EntryResult {
  try {
    return fn();
  } catch (e) {
    if (e instanceof OverflowError) return fail(e.message);
    throw e;
  }
}

/** deposit: nur echte Erhöhung; Zins und indexAt bleiben unverändert */
export function simDeposit(v: VaultState, amount: bigint): EntryResult {
  return guard(() => {
    const newColl = v.collateral + amount;
    if (!(newColl > v.collateral)) return fail(tr("Einzahlung muss größer als 0 sein.", "Deposit must be greater than 0."));
    return continueWith(newColl, v.debt, v.interest, v.indexAt);
  });
}

export function simWithdraw(v: VaultState, amount: bigint, o: OracleView, mcrBps: bigint): EntryResult {
  return guard(() => {
    const newColl = v.collateral - amount;
    if (!(newColl < v.collateral)) return fail(tr("Auszahlung muss größer als 0 sein.", "Withdrawal must be greater than 0."));
    if (newColl < 0n) return fail(tr("So viel KAS liegt nicht im Vault.", "The vault does not hold that much KAS."));
    const owedInterest = accrued(v, o.stableIndex);
    if (!healthy(newColl, add(v.debt, owedInterest), o.kasUsd, mcrBps)) return belowMcr();
    return continueWith(newColl, v.debt, owedInterest, o.stableIndex);
  });
}

/** close: nur ohne Schuld; der Zins geht in KAS an die Zinsadresse (unter 0,2 KAS erlassen), der Rest an den Besitzer */
export function simClose(v: VaultState, o: OracleView): EntryResult {
  return guard(() => {
    if (v.debt !== 0n) return fail(tr("Schließen geht nur ohne Schuld. Erst vollständig tilgen.", "Closing only works without debt. Repay it fully first."));
    const f = closeFee(accrued(v, o.stableIndex), o.kasUsd, v.collateral);
    const payout = v.collateral - f.due;
    const note =
      f.fee === 0n
        ? tr(`${v.collateral} sompi gehen an den Besitzer zurück.`, `${v.collateral} sompi go back to the owner.`)
        : f.waived
          ? tr(
              `Der Zins (${f.fee} sompi) liegt unter 0,2 KAS und wird erlassen. ${payout} sompi gehen an den Besitzer zurück.`,
              `The interest (${f.fee} sompi) is below 0.2 KAS and is waived. ${payout} sompi go back to the owner.`,
            )
          : tr(
              `${f.due} sompi Zins gehen an die Zinsadresse, ${payout} sompi an den Besitzer.`,
              `${f.due} sompi of interest go to the interest address, ${payout} sompi to the owner.`,
            );
    return { ok: true, state: null, fee: f.due, payout, note };
  });
}

/**
 * sweep: jeder löst einen Vault ohne Schuld auf, dessen Zinsgebühr die ganze
 * Sicherheit erreicht. Alles bis auf SWEEP_FEE geht an die Zinsadresse, der
 * Besitzer bekommt nichts (er bekäme auch beim Schließen nichts). Bleibt nach
 * SWEEP_FEE weniger als SWEEP_MIN_TREASURY, lässt sich die Tx nicht bauen –
 * dann nie ein negativer oder winziger Betrag (Audit 12 A12-2).
 */
export function simSweep(v: VaultState, o: OracleView): EntryResult {
  return guard(() => {
    if (v.debt !== 0n) return fail(tr("Auflösen geht nur ohne Schuld.", "Dissolving only works without debt."));
    if (interestFee(v, o) !== v.collateral)
      return fail(
        tr(
          "Der Zins ist kleiner als die Sicherheit. Diesen Vault kann nur sein Besitzer schließen.",
          "The interest is smaller than the collateral. Only the owner can close this vault.",
        ),
      );
    if (v.collateral < SWEEP_FEE + SWEEP_MIN_TREASURY)
      return fail(
        tr(
          `Die Sicherheit (${v.collateral} sompi) ist zu klein zum Auflösen: Vorab gehen 0,1 KAS für das Auflösen ab (die Netzgebühr, den Rest bekommt, wer auflöst). Danach bliebe für die Zinsadresse zu wenig für einen eigenen Ausgang.`,
          `The collateral (${v.collateral} sompi) is too small to dissolve: 0.1 KAS go to dissolving first (the network fee, the rest goes to whoever dissolves). Too little would remain for an output to the interest address.`,
        ),
      );
    const fee = v.collateral - SWEEP_FEE;
    return {
      ok: true,
      state: null,
      fee,
      payout: 0n,
      note: tr(`${fee} sompi gehen an die Zinsadresse, der Vault endet.`, `${fee} sompi go to the interest address, the vault ends.`),
    };
  });
}

/** mint: Gesundheit mit neuer Schuld + Zins bei der Mindestquote; maxDebt (Obergrenze je Vault) gilt nur für die Schuld */
export function simMint(v: VaultState, amount: bigint, o: OracleView, mcrBps: bigint, maxDebt: bigint | null = null): EntryResult {
  return guard(() => {
    if (!(amount > 0n)) return fail(tr("Betrag muss größer als 0 sein.", "Amount must be greater than 0."));
    const owedInterest = accrued(v, o.stableIndex);
    const newDebt = add(v.debt, amount);
    if (!healthy(v.collateral, add(newDebt, owedInterest), o.kasUsd, mcrBps)) return belowMcr();
    if (maxDebt !== null && newDebt > maxDebt)
      return fail(
        tr(
          `Je Vault sind höchstens ${maxDebt / UNIT} GHOST Schuld erlaubt (der Zins zählt dabei nicht).`,
          `Each vault may owe at most ${maxDebt / UNIT} GHOST (interest does not count here).`,
        ),
      );
    return continueWith(v.collateral, newDebt, owedInterest, o.stableIndex);
  });
}

/** repay: verbrennt höchstens die Schuld; der Zins bleibt stehen */
export function simRepay(v: VaultState, burned: bigint, o: OracleView): EntryResult {
  return guard(() => {
    if (!(burned > 0n)) return fail(tr("Betrag muss größer als 0 sein.", "Amount must be greater than 0."));
    if (burned > v.debt) return fail(tr("Überzahlung: Es darf höchstens die Schuld verbrannt werden.", "Overpayment: at most the debt may be burned."));
    return continueWith(v.collateral, v.debt - burned, accrued(v, o.stableIndex), o.stableIndex);
  });
}

/**
 * Rücknahme wie stable_vault.sil `redeem`: Jeder gibt `amount` GHOST zurück
 * (verbrannt) und erhält KAS im Wert von 1 USD je GHOST minus 1 %. Mindestens
 * 1 GHOST oder die ganze Schuld. Nur an Vaults ab der Liquidationsschwelle; im
 * Vault bleiben mindestens 0,2 KAS.
 */
export function simRedeem(v: VaultState, amount: bigint, o: OracleView, liqBps: bigint, feeBps: bigint = REDEEM_FEE_BPS): EntryResult {
  return guard(() => {
    if (!(amount > 0n)) return fail(tr("Betrag muss größer als 0 sein.", "Amount must be greater than 0."));
    if (amount > v.debt) return fail(tr("Zurückgeben lässt sich höchstens die Schuld dieses Vaults.", "At most this vault's debt can be redeemed."));
    if (amount < MIN_REDEEM && amount !== v.debt)
      return fail(
        tr(
          "Zurückgeben lässt sich mindestens 1 GHOST oder die ganze Schuld des Vaults.",
          "You can redeem at least 1 GHOST or the vault's whole debt.",
        ),
      );
    const owedInterest = accrued(v, o.stableIndex);
    if (!healthy(v.collateral, add(v.debt, owedInterest), o.kasUsd, liqBps))
      return fail(
        tr(
          "Der Vault liegt unter der Liquidationsschwelle. Dort ist keine Rücknahme möglich, nur Liquidation.",
          "The vault is below the liquidation threshold. Redemption is not possible there, only liquidation.",
        ),
      );
    const paid = redeemPayout(amount, o.kasUsd, feeBps);
    if (!(paid > 0n)) return fail(tr("Der Betrag ist zu klein für eine Auszahlung.", "The amount is too small for a payout."));
    if (v.collateral - paid < DUST)
      return fail(tr("Im Vault müssen mindestens 0,2 KAS bleiben. Bitte einen kleineren Betrag wählen.", "At least 0.2 KAS must remain in the vault. Please choose a smaller amount."));
    const r = continueWith(v.collateral - paid, v.debt - amount, owedInterest, o.stableIndex);
    return r.ok ? { ...r, paid, burned: amount } : r;
  });
}

/**
 * Liquidation wie stable_vault.sil `liquidate(oracleIdx, burn, …)`:
 * Erlaubt, wenn der Vault mit Schuld + Zins unter der Schwelle liegt. `burn`
 * GHOST (höchstens die Schuld, Standard = ganze Schuld) werden verbrannt, der
 * Liquidator erhält KAS im Wert von burn + Bonus. Der Zins bleibt stehen.
 * - Reicht die Sicherheit dafür nicht (seize == coll): er bekommt alles, der
 *   Vault endet, Restschuld und Zins sind ausgebucht.
 * - Bliebe weniger als 0,2 KAS übrig: nur bei voller Tilgung erlaubt, dann
 *   endet der Vault.
 */
export function simLiquidate(v: VaultState, o: OracleView, liqBps: bigint, bonusBps: bigint, burn?: bigint): EntryResult {
  return guard(() => {
    const coll = v.collateral;
    const owedInterest = accrued(v, o.stableIndex);
    if (healthy(coll, add(v.debt, owedInterest), o.kasUsd, liqBps))
      return fail(tr("Der Vault liegt über der Liquidationsschwelle und kann nicht liquidiert werden.", "The vault is above the liquidation threshold and cannot be liquidated."));
    const b = burn ?? v.debt;
    if (!(b > 0n)) return fail(tr("Es muss mehr als 0 GHOST verbrannt werden.", "More than 0 GHOST must be burned."));
    if (b > v.debt) return fail(tr("Es darf höchstens die ganze Schuld verbrannt werden.", "At most the whole debt may be burned."));
    const claim = mulDivUp(b, BPS + bonusBps, BPS); // USD × 1e8
    let seize = coll;
    if (mulDivDown(coll, o.kasUsd, PRICE_SCALE) > claim) {
      seize = mulDivUp(claim, PRICE_SCALE, o.kasUsd); // sompi, < coll
    }
    const rest = coll - seize;
    if (seize === coll) {
      // Sicherheit deckt den Anspruch nicht: Vault endet, Restschuld uneinbringlich
      const writtenOff = v.debt - b;
      return {
        ok: true,
        state: null,
        seized: coll,
        burned: b,
        writtenOff,
        note:
          writtenOff > 0n
            ? tr(
                "Die Sicherheit ist erschöpft: Der Liquidator bekommt alles, der Vault endet, die Restschuld wird ausgebucht.",
                "Collateral is exhausted: the liquidator gets everything, the vault ends, the remaining debt is written off.",
              )
            : tr("Die ganze Sicherheit geht an den Liquidator, der Vault endet.", "All collateral goes to the liquidator, the vault ends."),
      };
    }
    if (rest < DUST) {
      if (b !== v.debt)
        return fail(tr("Der Rest wäre kleiner als 0,2 KAS – dann muss die ganze Schuld verbrannt werden.", "The remainder would be less than 0.2 KAS – in that case the whole debt must be burned."));
      return {
        ok: true,
        state: null,
        seized: coll,
        burned: b,
        writtenOff: 0n,
        note: tr("Rest unter 0,2 KAS geht mit an den Liquidator, der Vault endet.", "A remainder under 0.2 KAS goes to the liquidator too, the vault ends."),
      };
    }
    // Der Zins bleibt stehen (gehört der Zinsadresse, wird beim Schließen bezahlt)
    const r = continueWith(rest, v.debt - b, owedInterest, o.stableIndex);
    return r.ok ? { ...r, seized: seize, burned: b, writtenOff: 0n } : r;
  });
}

/**
 * Vorschau einer Liquidation nur aus Sicherheit, Preis, Schuld und burn,
 * so wie die Oberfläche es aus ghostctl status kennt. Gleiche Formeln wie
 * simLiquidate (claim, seize, rest, DUST); der Zins ändert daran nichts.
 */
export function liquidationPreview(coll: bigint, kasUsd: bigint, debt: bigint, burn: bigint, bonusBps: bigint) {
  const claim = mulDivUp(burn, BPS + bonusBps, BPS);
  let seize = coll;
  if (mulDivDown(coll, kasUsd, PRICE_SCALE) > claim) seize = mulDivUp(claim, PRICE_SCALE, kasUsd);
  const rest = coll - seize;
  const exhausted = seize === coll;
  // Rest < 0,2 KAS ohne volle Tilgung lehnt der Vertrag ab (allowed = false)
  const smallRest = !exhausted && rest < DUST;
  const allowed = !smallRest || burn === debt;
  const ends = exhausted || smallRest;
  return { claim, seize: ends ? coll : seize, rest: ends ? 0n : rest, ends, allowed, writtenOff: exhausted ? debt - burn : 0n };
}

// ------------------------------------------------ Kennzahlen für die Anzeige

/** Sicherheitswert in USD × 1e8 (entspricht GHOST-Einheiten bei 1 GHOST = 1 USD) */
export function collateralValue(coll: bigint, kasUsd: bigint): bigint {
  return mulDivDown(coll, kasUsd, PRICE_SCALE);
}

/** Besicherungsquote in bps (abgerundet) auf Schuld + Zins (`owed`), null wenn nichts offen ist. */
export function ratioBps(coll: bigint, owed: bigint, kasUsd: bigint): bigint | null {
  if (owed === 0n) return null;
  return (collateralValue(coll, kasUsd) * BPS) / owed;
}

/**
 * Gesundheitsfaktor nach Aave-Art: Quote / Liquidationsschwelle (150 %).
 * Unter 1,0 ist der Vault liquidierbar. Als ×1e4 (4 Nachkommastellen).
 */
export function healthFactorE4(coll: bigint, owed: bigint, kasUsd: bigint, liqBps: bigint): bigint | null {
  if (owed === 0n) return null;
  return (collateralValue(coll, kasUsd) * BPS * BPS) / (owed * liqBps);
}

/**
 * Kleinster KAS-Preis (× 1e8), bei dem healthy(coll, owed, …, ratioBps) noch gilt.
 * Exakt: floor(coll·p/1e8) ≥ need ⇔ coll·p ≥ need·1e8 ⇔ p ≥ ceil(need·1e8/coll).
 * Mit der Liquidationsschwelle ist das der Liquidationspreis.
 */
export function minHealthyPrice(coll: bigint, owed: bigint, ratioBpsArg: bigint): bigint | null {
  if (owed === 0n || coll === 0n) return null;
  const need = mulDivUp(owed, ratioBpsArg, BPS);
  return (need * PRICE_SCALE + coll - 1n) / coll;
}

/** Höchster zusätzlich prägbarer Betrag (GHOST-Einheiten), exakt per Binärsuche über simMint. */
export function maxMintable(v: VaultState, o: OracleView, mcrBps: bigint, maxDebt: bigint | null = null): bigint {
  const ok = (m: bigint) => m === 0n || simMint(v, m, o, mcrBps, maxDebt).ok;
  let lo = 0n;
  let hi = (collateralValue(v.collateral, o.kasUsd) * BPS) / mcrBps + 1n;
  if (hi > MAX_DEBT) hi = MAX_DEBT;
  while (lo < hi) {
    const mid = (lo + hi + 1n) / 2n;
    if (ok(mid)) lo = mid;
    else hi = mid - 1n;
  }
  return lo;
}

/** Höchster abhebbarer KAS-Betrag (sompi), exakt per Binärsuche (mindestens 1 sompi bleibt; alles gibt nur „Schließen“ frei). */
export function maxWithdrawable(v: VaultState, o: OracleView, mcrBps: bigint): bigint {
  let lo = 0n;
  let hi = v.collateral - 1n; // Fortsetzung braucht > 0
  if (hi < 0n) return 0n;
  while (lo < hi) {
    const mid = (lo + hi + 1n) / 2n;
    if (simWithdraw(v, mid, o, mcrBps).ok) lo = mid;
    else hi = mid - 1n;
  }
  return lo;
}

/**
 * Höchster Rücknahmebetrag (GHOST-Einheiten): höchstens die Schuld, im Vault
 * bleiben ≥ 0,2 KAS, und mindestens 1 GHOST oder die ganze Schuld (sonst 0).
 */
export function maxRedeemable(coll: bigint, debt: bigint, kasUsd: bigint, feeBps: bigint = REDEEM_FEE_BPS): bigint {
  const ok = (m: bigint) => coll - redeemPayout(m, kasUsd, feeBps) >= DUST;
  let lo = 0n;
  let hi = debt;
  while (lo < hi) {
    const mid = (lo + hi + 1n) / 2n;
    if (ok(mid)) lo = mid;
    else hi = mid - 1n;
  }
  return lo < MIN_REDEEM && lo !== debt ? 0n : lo;
}

/** Jahreszins in bps → stableRate (Zins je DAA × 1e18), wie im Orakel-Kommentar (365 Tage). */
export function rateFromAprBps(aprBps: bigint, daaPerSecond: bigint = 10n): bigint {
  const daaPerYear = 365n * 86_400n * daaPerSecond;
  // aprBps/1e4 · 1e18 / daaPerYear, abgerundet (5 % → 158_548_959 wie im Orakel-Kommentar)
  return (aprBps * 100_000_000_000_000n) / daaPerYear;
}

export interface InterestPoint {
  day: number;
  index: bigint;
  /** Schuld in GHOST-Einheiten – bleibt ohne Tilgen gleich */
  debt: bigint;
  /** aufgelaufener Zins in USD × 1e8 */
  interest: bigint;
}

/**
 * Verlauf bei konstantem Satz, ohne dass der Vault angefasst wird: Das Orakel
 * schreibt den Index alle intervalDaa fort (oracleIndexAfter), der Zins folgt
 * über accrued(); die Schuld bleibt gleich.
 */
export function projectInterest(
  v: VaultState,
  index: bigint,
  stableRate: bigint,
  days: number[],
  intervalDaa: bigint,
  daaPerSecond: bigint = 10n,
): InterestPoint[] {
  const sorted = [...days].sort((a, b) => a - b);
  const out: InterestPoint[] = [];
  let idx = index;
  let elapsed = 0n; // DAA seit Start
  for (const day of sorted) {
    const target = BigInt(day) * 86_400n * daaPerSecond;
    while (elapsed + intervalDaa <= target) {
      idx = oracleIndexAfter(idx, stableRate, intervalDaa);
      elapsed += intervalDaa;
    }
    if (target > elapsed) {
      idx = oracleIndexAfter(idx, stableRate, target - elapsed);
      elapsed = target;
    }
    out.push({ day, index: idx, debt: v.debt, interest: accrued(v, idx) });
  }
  return out;
}
