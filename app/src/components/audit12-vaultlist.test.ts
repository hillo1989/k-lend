// Audit 12, Restpunkt B-P5 (A12-2, Seite): Seit A12-2 gelten Zombie-Vaults
// unter 0,125 KAS nicht mehr als auflösbar. Die Vault-Liste zeigte bei ihnen
// dann auch das Etikett „Zins zehrt Sicherheit auf“ nicht mehr, obwohl der Zins
// die Sicherheit weiter aufzehrt. Jetzt: Etikett mit Hinweis, aber ohne
// Auflöse-Knopf.
//
// Gerendert wird mit react-dom/server; Status, Schlüssel und Wallet kommen aus
// Attrappen der Kontexte.
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { setLangGlobal } from "../lib/i18n";
import type { DeployedStatus, VaultStatus } from "../lib/status";

const g = globalThis as unknown as { __status: DeployedStatus };

vi.mock("../lib/StatusContext", () => ({ useStatus: () => ({ status: g.__status, network: "mainnet" }) }));
vi.mock("../lib/AccountContext", () => ({ useAccount: () => ({ selected: { xonly: "aa".repeat(32) } }) }));
vi.mock("../wallet/WalletContext", () => ({ useWallet: () => ({ publicKey: null }) }));
vi.mock("../lib/usd", async (orig) => ({ ...(await orig<typeof import("../lib/usd")>()), useUsd: () => () => null }));

const { VaultList } = await import("./VaultList");
const { vaultInterestEatsCollateral, vaultSweepable } = await import("../lib/precheck");

function status(vaults: Partial<VaultStatus>[]): DeployedStatus {
  const vs: VaultStatus[] = vaults.map((v, index) => ({
    index,
    owner: "f".repeat(64),
    covenantId: `c${index}`,
    collateralKas: 100,
    debtGhost: 0,
    ratioPct: null,
    liquidationPriceUsd: null,
    maxMintGhost: 0,
    ...v,
  }));
  return {
    network: "mainnet",
    deployed: true,
    daa: 1,
    oracle: { covenantId: "", kasUsd: 0.05, seq: 1, ageMinutes: 5, ratePctYear: 5, index: 1, fresh: true, freshError: null },
    params: { mcrPct: 200, liqPct: 150, bonusPct: 10 },
    factoryCovenantId: "",
    ghostCovenantId: "",
    totals: { vaults: vs.length, collateralKas: 0, debtGhost: 0 },
    tokens: [],
    vaults: vs,
  };
}

/** Ausschnitt eines Listeneintrags (je Vault ein <li>) */
function item(html: string, index: number): string {
  const parts = html.split('<li class="vault-item');
  return parts.find((p) => p.includes(`<strong>Vault ${index}</strong>`)) ?? "";
}

function render(s: DeployedStatus): string {
  g.__status = s;
  return renderToStaticMarkup(createElement(VaultList, { onAction: () => {} }));
}

beforeEach(() => setLangGlobal("de"));

describe("B-P5: Etikett „Zins zehrt Sicherheit auf“ auch für zu kleine Zombie-Vaults", () => {
  // 1 USD Zins bei 0,05 USD je KAS = 20 KAS: zehrt jede dieser Sicherheiten auf
  const s = status([
    { collateralKas: 0.09, interestUsd: 1, sweepable: false }, // zu klein, ghostctl meldet nicht auflösbar
    { collateralKas: 0.12, interestUsd: 1 }, // zu klein, älteres ghostctl ohne sweepable
    { collateralKas: 0.3, interestUsd: 1, sweepable: true }, // auflösbar
    { collateralKas: 0.09, interestUsd: 0.001, sweepable: false }, // Zins kleiner als die Sicherheit
    { collateralKas: 0.09, debtGhost: 1, interestUsd: 1, ratioPct: 1 }, // mit Schuld
  ]);

  it("Rechnung: aufgezehrt heißt Schuld 0 und Zins ≥ Sicherheit, auch unter 0,125 KAS", () => {
    expect(s.vaults.map((v) => vaultInterestEatsCollateral(v, s))).toEqual([true, true, true, false, false]);
    expect(s.vaults.map((v) => vaultSweepable(v, s))).toEqual([false, false, true, false, false]);
    // ein großer Vault, den ghostctl (genau gerechnet) nicht als auflösbar meldet, zählt nicht
    const big = status([{ collateralKas: 20, interestUsd: 1, sweepable: false }]);
    expect(vaultInterestEatsCollateral(big.vaults[0], big)).toBe(false);
    // gesperrte Vaults bekommen nur ihr eigenes Etikett
    const st = status([{ collateralKas: 0.09, interestUsd: 1, stale: true }]);
    expect(vaultInterestEatsCollateral(st.vaults[0], st)).toBe(false);
  });

  it("zu kleiner Zombie-Vault: Etikett und Hinweis, aber kein Auflöse-Knopf", () => {
    const h = render(s);
    for (const i of [0, 1]) {
      const li = item(h, i);
      expect(li).toContain("Zins zehrt Sicherheit auf");
      expect(li).toContain("Zu klein zum Auflösen: Erst ab 0,125 KAS");
      expect(li).toContain("ein Zins unter 0,2 KAS wird dabei erlassen");
      expect(li).not.toContain("Auflösen (Zinsadresse)");
    }
  });

  it("auflösbarer Vault: Etikett und Knopf, kein Hinweis", () => {
    const li = item(render(s), 2);
    expect(li).toContain("Zins zehrt Sicherheit auf");
    expect(li).toContain("Auflösen (Zinsadresse)");
    expect(li).not.toContain("Zu klein zum Auflösen");
  });

  it("Zins kleiner als die Sicherheit oder Schuld offen: kein Etikett", () => {
    const h = render(s);
    for (const i of [3, 4]) {
      expect(item(h, i)).not.toContain("Zins zehrt Sicherheit auf");
      expect(item(h, i)).not.toContain("Zu klein zum Auflösen");
    }
  });

  it("englisch", () => {
    setLangGlobal("en");
    const li = item(render(s), 0);
    expect(li).toContain("interest exceeds collateral");
    expect(li).toContain("Too small to dissolve: only from 0.125 KAS");
    expect(li).not.toContain("Dissolve (interest address)");
  });
});
