// Audit 12, Gruppe Vault: Auflösen kleiner Vaults (A12-2), Rechengrenze der
// Zinsgebühr (A12-14) und winzige Auszahlung bei der Rücknahme (A12-18).
import { beforeEach, describe, expect, it } from "vitest";
import type { KeyEntry } from "./api";
import { setLangGlobal } from "./i18n";
import { precheck, vaultSweepable } from "./precheck";
import type { DeployedStatus, VaultStatus } from "./status";
import {
  INDEX_SCALE,
  OverflowError,
  SWEEP_FEE,
  SWEEP_MIN_TREASURY,
  interestFee,
  redeemPayout,
  simClose,
  simSweep,
  sweepAllowed,
  sweepable,
  type VaultState,
} from "./vaultMath";

const E8 = 100_000_000n;
beforeEach(() => setLangGlobal("de"));

const key: KeyEntry = { file: "keys/tn10-user.json", type: "key", xonly: "aa".repeat(32), address: "kaspatest:qown", kas: 10, ghost: 1, vaults: [] };
const vst = (collateral: bigint, debt: bigint, interest: bigint): VaultState => ({ collateral, debt, interest, indexAt: INDEX_SCALE });
const O = { kasUsd: 5_000_000n, stableIndex: INDEX_SCALE }; // 0,05 USD

function statusWith(v: Partial<VaultStatus>, kasUsd = 0.05): DeployedStatus {
  const vault: VaultStatus = { index: 0, owner: "f".repeat(64), covenantId: "a", collateralKas: 100, debtGhost: 0, ratioPct: null, liquidationPriceUsd: null, maxMintGhost: 0, ...v };
  return {
    network: "testnet-10",
    deployed: true,
    daa: 1,
    oracle: { covenantId: "", kasUsd, seq: 1, ageMinutes: 5, ratePctYear: 5, index: 1, fresh: true, freshError: null },
    params: { mcrPct: 200, liqPct: 150, bonusPct: 10 },
    factoryCovenantId: "",
    ghostCovenantId: "",
    totals: { vaults: 1, collateralKas: vault.collateralKas, debtGhost: vault.debtGhost },
    tokens: [],
    vaults: [vault],
  };
}
const texts = (h: ReturnType<typeof precheck>) => h.map((x) => x.text).join(" | ");

describe("A12-2: Auflösen nur, wenn der Kassen-Ausgang baubar ist", () => {
  it("SWEEP_FEE ist 0,1 KAS wie stable_vault.sil, die Mindestzahlung an die Kasse 0,025 KAS wie ghostctl", () => {
    expect(SWEEP_FEE).toBe(10_000_000n);
    expect(SWEEP_MIN_TREASURY).toBe(2_500_000n);
  });
  it("0,05 KAS Sicherheit: der Vertrag ließe es zu, auflösbar ist es nicht, und nie ein negativer Betrag", () => {
    const v = vst(5_000_000n, 0n, 1n * E8); // 1 USD Zins > 0,05 KAS · 0,05 USD
    expect(sweepAllowed(v, O)).toBe(true);
    expect(sweepable(v, O)).toBe(false);
    const r = simSweep(v, O);
    expect(r.ok).toBe(false);
    expect(!r.ok && r.error).toMatch(/zu klein zum Auflösen/);
  });
  it("Grenze auf den sompi genau: SWEEP_FEE + SWEEP_MIN_TREASURY geht, 1 sompi weniger nicht", () => {
    const edge = SWEEP_FEE + SWEEP_MIN_TREASURY;
    const v = vst(edge, 0n, 1n * E8);
    expect(sweepable(v, O)).toBe(true);
    const r = simSweep(v, O);
    expect(r.ok && r.fee).toBe(SWEEP_MIN_TREASURY);
    expect(sweepable({ ...v, collateral: edge - 1n }, O)).toBe(false);
    expect(simSweep({ ...v, collateral: edge - 1n }, O).ok).toBe(false);
  });
  it("Seite: ein älteres ghostctl meldet den kleinen Vault als auflösbar – weder Knopf noch negative Zahl", () => {
    const s = statusWith({ collateralKas: 0.05, interestUsd: 1, sweepable: true });
    expect(vaultSweepable(s.vaults[0], s)).toBe(false);
    const h = precheck({ action: "sweep", amount: null, vault: 0, key, status: s });
    expect(h.some((x) => x.level === "error" && /zu klein zum Auflösen/.test(x.text))).toBe(true);
    expect(texts(h)).not.toMatch(/-\d|−\d/);
    expect(texts(h)).not.toMatch(/gehen an die Zinsadresse/);
    // aus den Anzeigewerten gerechnet (ohne sweepable) ebenso
    const s2 = statusWith({ collateralKas: 0.12, interestUsd: 1 });
    expect(vaultSweepable(s2.vaults[0], s2)).toBe(false);
    const s3 = statusWith({ collateralKas: 0.125, interestUsd: 1 });
    expect(vaultSweepable(s3.vaults[0], s3)).toBe(true);
    expect(texts(precheck({ action: "sweep", amount: null, vault: 0, key, status: s3 }))).toMatch(/0,025 KAS gehen an die Zinsadresse/);
  });
  it("0,1 KAS heißen nicht Netzgebühr: die kostet ≈ 0,055 KAS, den Rest bekommt, wer auflöst (Nachprüfung)", () => {
    const s = statusWith({ collateralKas: 0.05, interestUsd: 1 });
    const h = texts(precheck({ action: "sweep", amount: null, vault: 0, key, status: s }));
    const r = simSweep(vst(5_000_000n, 0n, 1n * E8), O);
    for (const t of [h, !r.ok ? r.error : ""]) {
      expect(t).toMatch(/zu klein zum Auflösen/);
      expect(t).not.toMatch(/Netzgebühr von 0,1/);
      expect(t).toMatch(/0,1 KAS für das Auflösen ab \(die Netzgebühr, den Rest bekommt, wer auflöst\)/);
    }
    setLangGlobal("en");
    const en = texts(precheck({ action: "sweep", amount: null, vault: 0, key, status: s }));
    expect(en).not.toMatch(/0\.1 KAS network fee/);
    expect(en).toMatch(/the rest goes to whoever dissolves/);
  });
});

describe("A12-14: Rechengrenze der Zinsgebühr (Vertrag bricht ab, die Seite sagt es)", () => {
  const low = { kasUsd: 1_000n, stableIndex: INDEX_SCALE }; // 0,00001 USD, Tiefstpreis des Orakels
  it("wie der Vertrag: 1 Mio USD Zins läuft über, 900 000 USD nicht – nie ein negativer Wert", () => {
    const big = vst(10_000n * E8, 0n, 1_000_000n * E8);
    expect(() => interestFee(big, low)).toThrow(OverflowError);
    expect(sweepAllowed(big, low)).toBe(false);
    const c = simClose(big, low);
    expect(c.ok).toBe(false);
    expect(!c.ok && c.error).toMatch(/64-Bit-Überlauf/);
    const ok = vst(10_000n * E8, 0n, 900_000n * E8);
    expect(interestFee(ok, low)).toBe(ok.collateral);
    expect(sweepable(ok, low)).toBe(true);
    // genau an der Grenze: Zins·1e5 ≤ i64::MAX
    const edge = 9_223_372_036_854_775_807n / 100_000n;
    expect(sweepAllowed(vst(10_000n * E8, 0n, edge), low)).toBe(true);
    expect(sweepAllowed(vst(10_000n * E8, 0n, edge + 1n), low)).toBe(false);
  });
  it("Vorprüfung: Schließen und Auflösen nennen die Rechengrenze statt still zu bleiben", () => {
    const s = statusWith({ collateralKas: 10_000, interestUsd: 1_000_000 }, 0.00001);
    expect(vaultSweepable(s.vaults[0], s)).toBe(false);
    const own = { ...key, xonly: s.vaults[0].owner };
    const close = precheck({ action: "close", amount: null, vault: 0, key: own, status: s });
    expect(close.some((x) => x.level === "error" && /zu groß für die Rechnung des Vertrags/.test(x.text))).toBe(true);
    const sweep = precheck({ action: "sweep", amount: null, vault: 0, key, status: s });
    expect(sweep.some((x) => x.level === "error" && /zu groß für die Rechnung des Vertrags/.test(x.text))).toBe(true);
    expect(texts(sweep)).not.toMatch(/Zins ist kleiner als die Sicherheit/);
  });
});

describe("A12-18 (Vault-Teil): winzige Auszahlung bei der Rücknahme", () => {
  it("0,001 GHOST bei 0,05 USD/KAS → 0,0198 KAS, weniger als die Netzgebühr: Warnung", () => {
    expect(redeemPayout(100_000n, 5_000_000n)).toBe(1_980_000n);
    const s = statusWith({ collateralKas: 100, debtGhost: 0.001, ratioPct: 500_000, liquidationPriceUsd: 0 });
    const h = precheck({ action: "redeem", amount: 100_000n, vault: 0, key, status: s });
    expect(h.some((x) => x.level === "info" && /0,0198 KAS/.test(x.text))).toBe(true);
    expect(h.some((x) => x.level === "warn" && /zahlt nur 0,0198 KAS aus/.test(x.text) && /Netzgebühr/.test(x.text))).toBe(true);
  });
  it("Gegenprobe: 10 GHOST (198 KAS) ohne diese Warnung", () => {
    const s = statusWith({ collateralKas: 10_000, debtGhost: 10, ratioPct: 5_000, liquidationPriceUsd: 0 });
    const h = precheck({ action: "redeem", amount: 10n * E8, vault: 0, key, status: s });
    expect(h.some((x) => /198 KAS/.test(x.text))).toBe(true);
    expect(h.some((x) => /zahlt nur/.test(x.text))).toBe(false);
  });
});
