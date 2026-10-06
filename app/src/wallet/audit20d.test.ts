// Audit 20 d, Signierablauf mit der Browser-Wallet: A20d-1 (Unklar-Sperre),
// A20d-2 (Vault über die Covenant-ID, Vault im Plan geprüft und genannt),
// A20d-8 (Empfänger aus dem Plan mit der Eingabe vergleichen), A20d-9
// (Ausgänge nach Art statt nach deutschem Text, englische Texte).
import { beforeEach, describe, expect, it } from "vitest";
import { setLangGlobal } from "../lib/i18n";
import { outputKind, outputWhat, payeeOutputs, planMismatch, planRecipient, unclearLocked, type BuildResult } from "./actions";

const ME = "kaspa:qme";
const TO = "kaspa:qempfaenger";
const OTHER = "kaspa:qandere";
const ID0 = "aa".repeat(32);
const ID1 = "bb".repeat(32);
const ids = [ID0, ID1];
const vaultIdOf = (i: number) => ids[i];

function plan(action: Record<string, unknown>, outputs: BuildResult["outputs"] = []): BuildResult {
  return { ok: true, outputs, plan: { kind: "ghost-wallet-action:1", network: "mainnet", address: ME, action } };
}
const out = (index: number, address: string, what: string, kas = 1) => ({ index, sompi: kas * 1e8, kas, address, what });

beforeEach(() => setLangGlobal("de"));

describe("A20d-1: Unklar-Sperre im Wallet-Ablauf", () => {
  it("gesperrt bis ein Status nach dem Vorfall geladen ist", () => {
    expect(unclearLocked(null, null)).toBe(false);
    expect(unclearLocked(1000, null)).toBe(true);
    expect(unclearLocked(1000, 900)).toBe(true);
    expect(unclearLocked(1000, 1000)).toBe(true);
    expect(unclearLocked(1000, 1001)).toBe(false);
  });
  it("WalletSignFlow setzt die Sperre bei unklarem Ausgang und sperrt alle drei Knöpfe", async () => {
    const src = (await import("../components/WalletSignFlow.tsx?raw")).default as string;
    expect(src).toMatch(/if \(sendOutcome\(r\) === "unclear"\) setUnclearAt\(Date\.now\(\)\)/);
    expect(src).toMatch(/setUnclearAt\(Date\.now\(\)\);\n\s*onDone\(true\)/);
    expect(src.match(/isLocked \|\| phase !== "idle"/g)).toHaveLength(3);
  });
});

describe("A20d-8: Empfänger laut Plan muss der Eingabe entsprechen", () => {
  it("KAS senden: plan.action.to und eine Zahlung an genau diese Adresse", () => {
    const ok = plan({ action: "send", to: TO, kas: 1 }, [out(0, TO, "andere Adresse"), out(1, ME, "Wechselgeld an die Wallet")]);
    expect(planRecipient(ok)).toBe(TO);
    expect(planMismatch(ok, "send", { to: TO, kas: "1" }, ME)).toBeNull();
    expect(planMismatch(ok, "send", { to: OTHER, kas: "1" }, ME)).toMatch(/weicht von deiner Eingabe ab/);
    const wrongOut = plan({ action: "send", to: TO, kas: 1 }, [out(0, OTHER, "andere Adresse")]);
    expect(planMismatch(wrongOut, "send", { to: TO, kas: "1" }, ME)).toMatch(/Keine Zahlung an den Empfänger/);
  });
  it("GHOST senden: Empfänger steht nur im Token-Covenant – Vergleich über plan.action.to", () => {
    const p = plan({ action: "transfer", to: TO, ghost: 1 }, [out(0, "kaspa:pcov", "GHOST-Token – 0,3 KAS stecken darin und kommen beim Weitergeben bzw. Tilgen zurück", 0.3)]);
    expect(planMismatch(p, "transfer", { to: TO, ghost: "1" }, ME)).toBeNull();
    expect(planMismatch(p, "transfer", { to: OTHER, ghost: "1" }, ME)).not.toBeNull();
    expect(planMismatch(plan({ action: "transfer", ghost: 1 }), "transfer", { to: TO, ghost: "1" }, ME)).toMatch(/keinen Empfänger/);
  });
  it("Tresor anlegen: der neue Tresor nennt den Empfänger", () => {
    const good = plan({ action: "tresor-open", to: TO }, [out(0, "kaspa:ptresor", `Dein neuer Tresor – 10 KAS, zahlt 1 KAS monatlich am 1. an ${TO}`, 10)]);
    expect(planMismatch(good, "tresor-open", { to: TO }, ME)).toBeNull();
    const bad = plan({ action: "tresor-open", to: TO }, [out(0, "kaspa:ptresor", `Dein neuer Tresor – 10 KAS, zahlt 1 KAS monatlich am 1. an ${OTHER}`, 10)]);
    expect(planMismatch(bad, "tresor-open", { to: TO }, ME)).toMatch(/anderen Empfänger/);
  });
});

describe("A20d-2: Vault im Plan gehört zur gewählten Covenant-ID", () => {
  it("Nummer im Plan → Covenant-ID laut Status; Abweichung sperrt", () => {
    expect(planMismatch(plan({ action: "mint", vault: 1, ghost: 1 }), "mint", { vault: ID1, ghost: "1" }, ME, vaultIdOf)).toBeNull();
    expect(planMismatch(plan({ action: "mint", vault: 0, ghost: 1 }), "mint", { vault: ID1, ghost: "1" }, ME, vaultIdOf)).toMatch(/anderen Vault/);
    expect(planMismatch(plan({ action: "mint", vault: 7, ghost: 1 }), "mint", { vault: ID1, ghost: "1" }, ME, vaultIdOf)).toMatch(/Vault-Stand hat sich geändert/);
    expect(planMismatch(plan({ action: "mint", ghost: 1 }), "mint", { vault: ID1, ghost: "1" }, ME, vaultIdOf)).toMatch(/keinen Vault/);
  });
  it("Auswahl und Vorbelegung über die Covenant-ID; Zusammenfassung nennt den Vault", async () => {
    const af = (await import("../components/ActionForms.tsx?raw")).default as string;
    expect(af).toMatch(/useState<string \| null>\(null\)/);
    expect(af).toContain("vaultId?: string;");
    expect(af).toContain('if (meta.vault && selV) parts.push(vaultDisplay(selV));');
    expect(af).not.toContain('typeof built.params.vault === "number") parts.push');
    expect(af.match(/<option key=\{x\.covenantId\} value=\{x\.covenantId\}>/g)).toHaveLength(2);
    const vl = (await import("../components/VaultList.tsx?raw")).default as string;
    expect(vl).not.toMatch(/onAction\([^)]*v\.index\)/);
  });
});

describe("A20d-9: Ausgänge nach Art, englische Texte", () => {
  it("kind von ghostctl hat Vorrang, sonst der deutsche Text", () => {
    expect(outputKind({ ...out(0, OTHER, "other address"), kind: "other" }, ME)).toBe("other");
    expect(outputKind(out(0, OTHER, "andere Adresse"), ME)).toBe("other");
    expect(outputKind(out(0, ME, "Wechselgeld an die Wallet"), ME)).toBe("self");
    expect(outputKind(out(0, "kaspa:pcov", "Vertrag (Covenant)"), ME)).toBe("covenant");
    // übersetzt der Server später, bleibt die Empfängerliste über `kind` gefüllt
    const b: BuildResult = { ok: true, outputs: [{ ...out(0, OTHER, "other address"), kind: "other" }, out(1, ME, "change")] };
    expect(payeeOutputs(b, ME).map((o) => o.address)).toEqual([OTHER]);
  });
  it("englische Oberfläche übersetzt die bekannten Texte", () => {
    setLangGlobal("en");
    expect(outputWhat("Wechselgeld an die Wallet")).toBe("change to the wallet");
    expect(outputWhat("andere Adresse")).toBe("other address");
    expect(outputWhat("Minter-Zweig deines Vaults 2 (läuft weiter)")).toBe("minter branch of your vault 2 (continues)");
    expect(outputWhat(`Dein neuer Tresor – 10 KAS, zahlt 1 KAS monatlich am 1. an ${TO}`)).toBe(`your new vault – 10 KAS, pays 1 KAS monatlich am 1. to ${TO}`);
    expect(outputWhat("unbekannt")).toBe("unbekannt");
    setLangGlobal("de");
    expect(outputWhat("andere Adresse")).toBe("andere Adresse");
  });
});
