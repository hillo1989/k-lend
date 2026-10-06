import { describe, expect, it } from "vitest";
import type { KeyEntry } from "./api";
import { minKeepSompi, precheck, ratioAfterKeep, vaultSweepable } from "./precheck";
import type { DeployedStatus } from "./status";

const owner = "d64cbe281ed8bc5d22e521ce4534de0a176b6f5dd1d43762faf0a9e2fe90af6a";
const status: DeployedStatus = {
  network: "testnet-10",
  deployed: true,
  daa: 1,
  oracle: { covenantId: "", kasUsd: 0.05, seq: 1, ageMinutes: 5, ratePctYear: 5, index: 1, fresh: true, freshError: null },
  params: { mcrPct: 200, liqPct: 150, bonusPct: 10 },
  factoryCovenantId: "",
  ghostCovenantId: "",
  totals: { vaults: 2, collateralKas: 3100, debtGhost: 11 },
  tokens: [],
  vaults: [
    { index: 0, owner, covenantId: "a", collateralKas: 3000, debtGhost: 1, ratioPct: 15000, liquidationPriceUsd: 0.0005, maxMintGhost: 74 },
    { index: 1, owner: "f".repeat(64), covenantId: "b", collateralKas: 100, debtGhost: 10, ratioPct: 50, liquidationPriceUsd: 0.15, maxMintGhost: 0 },
  ],
};
const key: KeyEntry = { file: "keys/tn10-deployer.json", type: "key", xonly: owner, address: "kaspatest:q", kas: 100, ghost: 2, vaults: [0] };
const E8 = 100_000_000n;
const levels = (h: ReturnType<typeof precheck>) => h.map((x) => x.level);

describe("precheck", () => {
  it("v4: eingefrorenes Orakel sperrt Prägen, Einlösen, Tausch und Abheben mit Schuld", () => {
    const frozen = { ...status, oracle: { ...status.oracle, frozen: true } };
    const blocked = (h: ReturnType<typeof precheck>) => h.some((x) => x.level === "error" && x.text.includes("eingefroren"));
    expect(blocked(precheck({ action: "mint", amount: 1n * E8, vault: 0, key, status: frozen }))).toBe(true);
    expect(blocked(precheck({ action: "redeem", amount: 1n * E8, vault: 0, key, status: frozen }))).toBe(true);
    expect(blocked(precheck({ action: "swap", amount: 1n * E8, vault: null, key, status: frozen }))).toBe(true);
    expect(blocked(precheck({ action: "withdraw", amount: 100n * E8, vault: 0, key, status: frozen }))).toBe(true);
    // Schuld 0: Abheben geht; Einzahlen und Tilgen gehen immer
    const noDebt = { ...frozen, vaults: frozen.vaults.map((v) => (v.index === 0 ? { ...v, debtGhost: 0 } : v)) };
    expect(blocked(precheck({ action: "withdraw", amount: 100n * E8, vault: 0, key, status: noDebt }))).toBe(false);
    expect(blocked(precheck({ action: "deposit", amount: 1n * E8, vault: 0, key, status: frozen }))).toBe(false);
    expect(blocked(precheck({ action: "repay", amount: 1n * E8, vault: 0, key, status: frozen }))).toBe(false);
    // nicht eingefroren: kein Sperr-Hinweis
    expect(blocked(precheck({ action: "mint", amount: 1n * E8, vault: 0, key, status }))).toBe(false);
  });
  it("Vault eröffnen: Warnung zu den 3 KAS und Mindestsicherheit", () => {
    const texts = (amt: bigint) => precheck({ action: "open-vault", amount: amt, vault: null, key, status }).map((h) => h.text);
    // Kurs im Test 0,05 USD, Quote 200 %: 1 KAS → 0,025 GHOST, für 1 GHOST etwa 40 KAS
    const small = texts(1n * E8);
    expect(small.some((t) => t.includes("bindet zusätzlich 3 KAS für immer"))).toBe(true);
    expect(small.some((t) => t.includes("höchstens 0,025 GHOST") && t.includes("mindestens etwa 40 KAS"))).toBe(true);
    const big = texts(100n * E8);
    expect(big.some((t) => t.includes("höchstens etwa 2,5 GHOST"))).toBe(true);
    expect(big.some((t) => t.includes("mindestens etwa"))).toBe(false);
  });
  it("mint über dem Maximum ist ein Fehler", () => {
    expect(levels(precheck({ action: "mint", amount: 75n * E8, vault: 0, key, status }))).toContain("error");
    expect(levels(precheck({ action: "mint", amount: 1n * E8, vault: 0, key, status }))).not.toContain("error");
  });
  it("Obergrenze je Vault beim Prägen", () => {
    const capped = { ...status, params: { ...status.params, maxDebtGhost: 50 }, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, debtGhost: 45, maxMintGhost: 5 } : v)) };
    const h = precheck({ action: "mint", amount: 6n * E8, vault: 0, key, status: capped });
    expect(h.some((x) => x.level === "error" && /höchstens 50 GHOST Schuld/.test(x.text))).toBe(true);
    expect(levels(precheck({ action: "mint", amount: 5n * E8, vault: 0, key, status: capped }))).not.toContain("error");
  });
  it("Pool anlegen: Startkurs in USD, gesperrte Mindestliquidität", () => {
    // 11,12 KAS und 0,5 GHOST bei 0,05 USD je KAS: 1 GHOST = 22,24 KAS = 1,112 USD
    const h = precheck({ action: "pool-open", amount: 1_112_000_000n, amount2: 50_000_000n, vault: null, key, status });
    const t = h.find((x) => x.level === "info")?.text ?? "";
    expect(t).toMatch(/1 GHOST = 22,24 KAS, also 1,112 USD je GHOST/);
    expect(t).toMatch(/1 KAS und 0,04496402 GHOST bleiben für immer im Pool/);
    expect(levels(precheck({ action: "pool-open", amount: E8 / 2n, amount2: E8, vault: null, key, status }))).toContain("error");
  });
  it("Pool anlegen: Startkurs muss im Kursband 1 USD ± 3 % liegen", () => {
    // 1,112 USD: außerhalb, mit Vorschlag
    const out = precheck({ action: "pool-open", amount: 1_112_000_000n, amount2: 50_000_000n, vault: null, key, status });
    expect(out.some((x) => x.level === "error" && /1 USD ± 3 %/.test(x.text) && /0,556 GHOST/.test(x.text))).toBe(true);
    // 20 KAS und 1 GHOST bei 0,05 USD je KAS = 1,00 USD: im Band
    expect(levels(precheck({ action: "pool-open", amount: 20n * E8, amount2: E8, vault: null, key, status }))).not.toContain("error");
    // 1,029 USD geht, 1,031 USD nicht
    expect(levels(precheck({ action: "pool-open", amount: 2_058_000_000n, amount2: E8, vault: null, key, status }))).not.toContain("error");
    expect(levels(precheck({ action: "pool-open", amount: 2_062_000_000n, amount2: E8, vault: null, key, status }))).toContain("error");
  });
  it("Pool: Anteile beim Einlegen, Auszahlung beim Abziehen", () => {
    const pool = { covenantId: "", lpCovenantId: "", kasSompi: String(1_000n * E8), ghostUnits: String(50n * E8), shares: String(1_000n * E8), feeBps: 30 };
    const s2 = { ...status, pool };
    const add = precheck({ action: "pool-add", amount: 100n * E8, amount2: 5n * E8, vault: null, key: { ...key, ghost: 10 }, status: s2 });
    expect(add.some((x) => /10000000000 Anteile/.test(x.text))).toBe(true);
    const lopsided = precheck({ action: "pool-add", amount: 100n * E8, amount2: 9n * E8, vault: null, key: { ...key, ghost: 10 }, status: s2 });
    // mehr als 1 % daneben: ghostctl bricht ab (A10-P-1)
    expect(lopsided.some((x) => x.level === "error" && /bricht ghostctl ab/.test(x.text))).toBe(true);
    const slight = precheck({ action: "pool-add", amount: 100n * E8, amount2: 502_000_000n, vault: null, key: { ...key, ghost: 10 }, status: s2 });
    expect(levels(slight)).not.toContain("error");
    expect(slight.some((x) => /Rest bleibt bei dir/.test(x.text))).toBe(true);
    const rem = precheck({ action: "pool-remove", amount: 50n * E8, vault: null, key: { ...key, lpShares: String(100n * E8) }, status: s2 });
    expect(rem.some((x) => /50 KAS und 2,5 GHOST/.test(x.text))).toBe(true);
    expect(levels(precheck({ action: "pool-remove", amount: 50n * E8, vault: null, key, status: s2 }))).toContain("error");
  });
  it("fremder Vault beim Prägen", () => {
    expect(precheck({ action: "mint", amount: E8, vault: 1, key, status }).some((h) => /gehört nicht/.test(h.text))).toBe(true);
  });
  it("volle Tilgung braucht genug GHOST", () => {
    expect(levels(precheck({ action: "repay", amount: null, repayAll: true, vault: 0, key, status }))).not.toContain("error");
    expect(levels(precheck({ action: "repay", amount: null, repayAll: true, vault: 0, key: { ...key, ghost: 0.5 }, status }))).toContain("error");
  });
  it("Liquidation: nur unter 150 % und mit genug GHOST", () => {
    expect(levels(precheck({ action: "liquidate", amount: null, vault: 0, key, status }))).toContain("error");
    const h = precheck({ action: "liquidate", amount: null, vault: 1, key: { ...key, ghost: 20 }, status });
    expect(levels(h)).not.toContain("error");
  });
  it("Liquidation v2: Unterdeckung → ganze Sicherheit, Restschuld ausgebucht", () => {
    // Vault 1: 100 KAS à 0,05 USD = 5 USD Sicherheit, 10 GHOST Schuld
    const h = precheck({ action: "liquidate", amount: null, liquidateAll: true, vault: 1, key: { ...key, ghost: 20 }, status });
    expect(h.some((x) => /ausgebucht/.test(x.text))).toBe(false); // volle Schuld verbrannt → nichts ausgebucht
    const part = precheck({ action: "liquidate", amount: 4n * E8, liquidateAll: false, vault: 1, key: { ...key, ghost: 20 }, status });
    // 4 GHOST + 10 % = 4,4 USD < 5 USD → Teil-Liquidation, 12 KAS bleiben
    expect(part.some((x) => /88 KAS/.test(x.text))).toBe(true);
    const deep = precheck({ action: "liquidate", amount: 5n * E8, liquidateAll: false, vault: 1, key: { ...key, ghost: 20 }, status });
    expect(deep.some((x) => x.level === "warn" && /5 GHOST Restschuld werden ausgebucht/.test(x.text))).toBe(true);
    expect(levels(precheck({ action: "liquidate", amount: 11n * E8, liquidateAll: false, vault: 1, key: { ...key, ghost: 20 }, status }))).toContain("error");
  });
  it("Einzahlen nur durch den Besitzer (v2)", () => {
    expect(precheck({ action: "deposit", amount: E8, vault: 1, key, status }).some((h) => /gehört nicht/.test(h.text))).toBe(true);
    expect(levels(precheck({ action: "deposit", amount: E8, vault: 0, key, status }))).not.toContain("error");
  });
  it("Tilgen nur durch den Besitzer (v2.1)", () => {
    expect(precheck({ action: "repay", amount: E8, repayAll: false, vault: 1, key: { ...key, ghost: 20 }, status }).some((h) => /gehört nicht/.test(h.text))).toBe(true);
  });
  it("Tilgen v3: jeder Betrag bis zur Schuld, der Zins bleibt stehen", () => {
    expect(levels(precheck({ action: "repay", amount: 1n, repayAll: false, vault: 0, key, status }))).not.toContain("error");
    expect(levels(precheck({ action: "repay", amount: E8, repayAll: false, vault: 0, key, status }))).not.toContain("error");
    expect(levels(precheck({ action: "repay", amount: E8 + 1n, repayAll: false, vault: 0, key, status }))).toContain("error");
    const s2 = { ...status, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, interestUsd: 0.25 } : v)) };
    expect(precheck({ action: "repay", amount: null, repayAll: true, vault: 0, key, status: s2 }).some((h) => /offene Zins \(0,25 USD\) bleibt stehen/.test(h.text))).toBe(true);
  });
  it("Rücknahme: Auszahlung zu 1 USD minus 1 %", () => {
    // 1 GHOST · 0,99 USD / 0,05 USD = 19,8 KAS
    const h = precheck({ action: "redeem", amount: E8, vault: 0, key, status });
    expect(levels(h)).not.toContain("error");
    expect(h.some((x) => x.level === "info" && /etwa 19,8 KAS/.test(x.text) && /minus 1 %/.test(x.text))).toBe(true);
    // fremde Vaults sind erlaubt: keine Besitzerprüfung
    expect(h.some((x) => /gehört nicht/.test(x.text))).toBe(false);
    // Abschlag aus ghostctl hat Vorrang
    const s2 = { ...status, params: { ...status.params, redeemFeePct: 0.5 } };
    expect(precheck({ action: "redeem", amount: E8, vault: 0, key, status: s2 }).some((x) => /etwa 19,9 KAS/.test(x.text))).toBe(true);
  });
  it("Rücknahme: mindestens 1 GHOST oder die ganze Schuld", () => {
    const e = precheck({ action: "redeem", amount: E8 - 1n, vault: 0, key, status });
    expect(e.some((x) => x.level === "error" && /mindestens 1 GHOST oder die ganze Schuld/.test(x.text))).toBe(true);
    // ganze Schuld unter 1 GHOST geht
    const half = { ...status, vaults: [{ ...status.vaults[0], debtGhost: 0.5 }] };
    expect(levels(precheck({ action: "redeem", amount: E8 / 2n, vault: 0, key, status: half }))).not.toContain("error");
    expect(levels(precheck({ action: "redeem", amount: E8 / 4n, vault: 0, key, status: half }))).toContain("error");
  });
  it("Rücknahme: Grenzen", () => {
    expect(levels(precheck({ action: "redeem", amount: E8 + 1n, vault: 0, key, status }))).toContain("error"); // mehr als die Schuld
    expect(levels(precheck({ action: "redeem", amount: E8, vault: 1, key, status }))).toContain("error"); // unter 150 %
    expect(levels(precheck({ action: "redeem", amount: E8, vault: 0, key: { ...key, ghost: 0.5 }, status }))).toContain("error"); // zu wenig GHOST
    // 0,3 KAS Sicherheit, 0,008 GHOST Schuld: die ganze Schuld ließe nur 0,102 KAS,
    // Teilbeträge unter 1 GHOST gehen nicht – also gar keine Rücknahme
    const tiny = { ...status, vaults: [{ ...status.vaults[0], collateralKas: 0.3, debtGhost: 0.008, ratioPct: 187.5 }] };
    const h = precheck({ action: "redeem", amount: 800_000n, vault: 0, key, status: tiny });
    expect(h.some((x) => x.level === "error" && /mindestens 0,2 KAS/.test(x.text) && /keine Rücknahme möglich/.test(x.text))).toBe(true);
    expect(precheck({ action: "redeem", amount: 502_513n, vault: 0, key, status: tiny }).some((x) => /mindestens 1 GHOST/.test(x.text))).toBe(true);
    // 20 KAS Sicherheit, 2 GHOST Schuld: höchstens so viel, dass 0,2 KAS bleiben
    const small = { ...status, vaults: [{ ...status.vaults[0], collateralKas: 20, debtGhost: 2, ratioPct: 150 }] };
    const h2 = precheck({ action: "redeem", amount: 2n * E8, vault: 0, key, status: small });
    expect(h2.some((x) => x.level === "error" && /höchstens 1,00000001 GHOST/.test(x.text))).toBe(true);
    expect(levels(precheck({ action: "redeem", amount: 100_000_001n, vault: 0, key, status: small }))).not.toContain("error");
    expect(levels(precheck({ action: "redeem", amount: 100_000_002n, vault: 0, key, status: small }))).toContain("error");
  });
  it("Schließen: Zinsgebühr in KAS, unter 0,2 KAS erlassen", () => {
    const withI = (interestUsd: number) => ({ ...status, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, debtGhost: 0, ratioPct: null, interestUsd } : v)) });
    // 2 USD bei 0,05 USD/KAS = 40 KAS
    const h = precheck({ action: "close", amount: null, vault: 0, key, status: withI(2) });
    expect(levels(h)).not.toContain("error");
    expect(h.some((x) => /Zinsgebühr: 40 KAS an die Zinsadresse/.test(x.text))).toBe(true);
    // 0,009 USD = 0,18 KAS → erlassen
    expect(precheck({ action: "close", amount: null, vault: 0, key, status: withI(0.009) }).some((x) => /erlassen/.test(x.text))).toBe(true);
    // genau 0,2 KAS wird bezahlt
    expect(precheck({ action: "close", amount: null, vault: 0, key, status: withI(0.01) }).some((x) => /Zinsgebühr: 0,2 KAS/.test(x.text))).toBe(true);
  });
  it("Schließen/Abheben: Warnung bei einem Ausgang unter etwa 0,02 KAS (A11-O-12)", () => {
    // 149,9995 USD Zins bei 0,05 USD/KAS = 2 999,99 KAS von 3 000 → 0,01 KAS an dich
    const s2 = { ...status, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, debtGhost: 0, ratioPct: null, interestUsd: 149.9995 } : v)) };
    const c = precheck({ action: "close", amount: null, vault: 0, key, status: s2 });
    expect(c.some((x) => x.level === "warn" && /nur 0,01 KAS/.test(x.text) && /nicht bauen/.test(x.text))).toBe(true);
    expect(precheck({ action: "close", amount: null, vault: 0, key, status: { ...status, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, debtGhost: 0, ratioPct: null, interestUsd: 2 } : v)) } }).some((x) => x.level === "warn")).toBe(false);
    // Abheben: 0,01 KAS Auszahlung
    const w = precheck({ action: "withdraw", amount: 299_999_000_000n, vault: 0, key, status });
    expect(w.some((x) => x.level === "warn" && /Ausgezahlt wären nur 0,01 KAS/.test(x.text))).toBe(true);
    expect(precheck({ action: "withdraw", amount: 2_000n * E8, vault: 0, key, status }).some((x) => x.level === "warn")).toBe(false);
  });
  it("Auflösen (sweep): nur ohne Schuld und mit Zins ≥ Sicherheit", () => {
    // 2 250 KAS à 0,05 USD = 112,5 USD, Zins 150 USD
    const z = { ...status.vaults[1], collateralKas: 2250, debtGhost: 0, ratioPct: null, interestUsd: 150 };
    const s2 = { ...status, vaults: [status.vaults[0], z] };
    expect(vaultSweepable(z, s2)).toBe(true);
    const h = precheck({ action: "sweep", amount: null, vault: 1, key, status: s2 });
    expect(levels(h)).not.toContain("error");
    expect(h.some((x) => /2\.249,9 KAS gehen an die Zinsadresse/.test(x.text))).toBe(true);
    // jeder darf: keine Besitzerprüfung
    expect(h.some((x) => /gehört nicht/.test(x.text))).toBe(false);
    // Zins kleiner als die Sicherheit: nur der Besitzer schließt
    const low = { ...z, interestUsd: 100 };
    expect(vaultSweepable(low, s2)).toBe(false);
    expect(levels(precheck({ action: "sweep", amount: null, vault: 1, key, status: { ...s2, vaults: [status.vaults[0], low] } }))).toContain("error");
    // mit Schuld nie; ghostctl-Angabe hat Vorrang
    expect(levels(precheck({ action: "sweep", amount: null, vault: 0, key, status }))).toContain("error");
    expect(vaultSweepable({ ...z, sweepable: false }, s2)).toBe(false);
    expect(vaultSweepable({ ...low, sweepable: true }, s2)).toBe(true);
    expect(vaultSweepable({ ...z, stale: true }, s2)).toBe(false);
  });
  it("Abheben: Zins zählt für die Mindestquote", () => {
    // 1 GHOST + 1 USD Zins bei 0,05 USD/KAS und 200 % → mindestens 80 KAS
    const s2 = { ...status, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, interestUsd: 1 } : v)) };
    expect(minKeepSompi(1, 0.05, 200, 1)).toBe(80n * E8);
    expect(levels(precheck({ action: "withdraw", amount: 79n * E8, vault: 0, key, status: s2 }))).toContain("error");
    expect(levels(precheck({ action: "withdraw", amount: 80n * E8, vault: 0, key, status: s2 }))).not.toContain("error");
    expect(ratioAfterKeep(80n * E8, 1, 0.05, 1)).toBe(200);
  });
  it("von Dritten veränderter Vault ist gesperrt", () => {
    const s3 = { ...status, vaults: status.vaults.map((v) => (v.index === 0 ? { ...v, stale: true } : v)) };
    expect(levels(precheck({ action: "mint", amount: E8, vault: 0, key, status: s3 }))).toContain("error");
  });
  it("Orakel: mindestens 1 Minute Abstand (v2.1)", () => {
    const s4 = { ...status, oracle: { ...status.oracle, ageMinutes: 0.5 } };
    expect(levels(precheck({ action: "oracle-update", amount: null, vault: null, key, status: s4 }))).toContain("error");
    const s5 = { ...status, oracle: { ...status.oracle, ageMinutes: 1.01 } };
    expect(levels(precheck({ action: "oracle-update", amount: null, vault: null, key, status: s5 }))).toContain("error");
    const s6 = { ...status, daa: 0, oracle: { ...status.oracle, ageMinutes: -5 } };
    expect(levels(precheck({ action: "oracle-update", amount: null, vault: null, key, status: s6 }))).not.toContain("error");
  });
  it("Orakel: Grenzen und höchstens ×2/÷2", () => {
    const o = (usd: bigint) => levels(precheck({ action: "oracle-update", amount: null, usd, vault: null, key, status }));
    expect(o(10_000_000n)).not.toContain("error"); // 0,1 = ×2
    expect(o(10_000_001n)).toContain("error");
    expect(o(2_500_000n)).not.toContain("error"); // 0,025 = ÷2
    expect(o(2_499_999n)).toContain("error");
    expect(o(999n)).toContain("error"); // unter 0,00001
    expect(precheck({ action: "oracle-update", amount: null, rateBps: 500n, vault: null, key, status }).some((h) => h.level === "warn")).toBe(false);
    expect(precheck({ action: "oracle-update", amount: null, rateBps: 2_500n, vault: null, key, status }).some((h) => h.level === "warn")).toBe(true);
  });
  it("Schließen mit Schuld", () => {
    expect(levels(precheck({ action: "close", amount: null, vault: 0, key, status }))).toContain("error");
  });
  it("Abheben unter die Mindestquote", () => {
    // Schuld 1 GHOST, 0,05 USD/KAS, 200 % → mindestens 40 KAS
    expect(minKeepSompi(1, 0.05, 200)).toBe(40n * E8);
    expect(levels(precheck({ action: "withdraw", amount: 39n * E8, vault: 0, key, status }))).toContain("error");
    expect(levels(precheck({ action: "withdraw", amount: 40n * E8, vault: 0, key, status }))).not.toContain("error");
    expect(ratioAfterKeep(40n * E8, 1, 0.05)).toBe(200);
  });
  it("ohne Schlüssel", () => {
    expect(levels(precheck({ action: "send", amount: E8, vault: null, key: null, status }))).toContain("error");
  });
});
