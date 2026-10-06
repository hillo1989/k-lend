import { beforeEach, describe, expect, it } from "vitest";
import { setLangGlobal } from "./i18n";
import { vaultLabel } from "./status";

beforeEach(() => setLangGlobal("de"));
const me = "aa".repeat(32);
const other = "bb".repeat(32);
const all = [
  { index: 0, owner: other, covenantId: "11".repeat(32) },
  { index: 1, owner: me, covenantId: "22".repeat(32) },
  { index: 2, owner: other, covenantId: "33".repeat(32) },
  { index: 3, owner: me, covenantId: "44".repeat(32) },
];

describe("vaultLabel: jeder Nutzer zählt seine Vaults ab 1", () => {
  it("eigene Vaults 1, 2 …, fremde mit Kurz-ID", () => {
    expect(vaultLabel(all[1], all, me)).toBe("Vault 1");
    expect(vaultLabel(all[3], all, me)).toBe("Vault 2");
    expect(vaultLabel(all[0], all, me)).toBe("Fremder Vault 11111111");
    expect(vaultLabel(all[0], all, other)).toBe("Vault 1");
    expect(vaultLabel(all[2], all, other)).toBe("Vault 2");
  });
  it("ohne Schlüssel ist jeder Vault fremd", () => {
    expect(vaultLabel(all[1], all, null)).toBe("Fremder Vault 22222222");
  });
});
