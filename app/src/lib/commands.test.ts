import { describe, expect, it } from "vitest";
import { ALLOWED } from "../../server/actions";
import { cliDecimal, commandFor, FLAG_ORDER, shellArg } from "./commands";

describe("commandFor", () => {
  it("kennt dieselben Parameter wie der Server (A10-W-3)", () => {
    // Daueraufträge (abo-*, tresor-*) haben ein eigenes Formular ohne „Als Befehl“
    for (const a of Object.keys(ALLOWED) as (keyof typeof ALLOWED)[]) {
      if (a.startsWith("abo-") || a.startsWith("tresor-")) continue;
      expect(FLAG_ORDER[a as keyof typeof FLAG_ORDER]).toEqual(ALLOWED[a]);
    }
    for (const a of Object.keys(FLAG_ORDER)) expect(ALLOWED).toHaveProperty(a);
  });
  it("Nachricht mit = und öffentliche Nachricht als Schalter", () => {
    expect(commandFor("mainnet", "send", { key: "keys/a.json", to: "kaspa:qabc", kas: "3", message: "Miete Oktober", onchain: true })).toBe(
      "./ghostctl --network mainnet send --key keys/a.json --to kaspa:qabc --kas 3 --message='Miete Oktober' --onchain-message",
    );
    expect(commandFor("mainnet", "transfer", { key: "keys/a.json", to: "kaspa:qabc", ghost: "1", message: "--ja", onchain: false })).toBe(
      "./ghostctl --network mainnet transfer --key keys/a.json --to kaspa:qabc --ghost 1 --message=--ja",
    );
  });
  it("Teil-Liquidation behält den Betrag", () => {
    expect(commandFor("mainnet", "liquidate", { key: "keys/a.json", vault: 3, ghost: "0.5" })).toBe(
      "./ghostctl --network mainnet liquidate --key keys/a.json --vault 3 --ghost 0.5",
    );
  });
  it("Rücknahme mit Vault und Betrag", () => {
    expect(commandFor("mainnet", "redeem", { key: "keys/a.json", vault: 4, ghost: "2" })).toBe(
      "./ghostctl --network mainnet redeem --key keys/a.json --vault 4 --ghost 2",
    );
  });
  it("Auflösen zugunsten der Zinsadresse", () => {
    expect(commandFor("mainnet", "sweep", { key: "keys/a.json", vault: 2 })).toBe("./ghostctl --network mainnet sweep --key keys/a.json --vault 2");
  });
  it("Mindestwerte im Pool als --min-…", () => {
    expect(commandFor("mainnet", "pool-add", { key: "keys/a.json", kas: "2", ghost: "0.1", minShares: 99 })).toBe(
      "./ghostctl --network mainnet pool-add --key keys/a.json --kas 2 --ghost 0.1 --min-shares 99",
    );
    expect(commandFor("mainnet", "pool-remove", { key: "keys/a.json", percent: "50", minKas: "1", minGhost: "0" })).toBe(
      "./ghostctl --network mainnet pool-remove --key keys/a.json --percent 50 --min-kas 1 --min-ghost 0",
    );
  });
  it("prägen wie in MAINNET.md", () => {
    expect(commandFor("mainnet", "mint", { key: "keys/mainnet-owner.json", vault: 0, ghost: "1" })).toBe(
      "./ghostctl --network mainnet mint --key keys/mainnet-owner.json --vault 0 --ghost 1",
    );
  });
  it("tilgen ohne Betrag = ganze Schuld", () => {
    expect(commandFor("mainnet", "repay", { key: "keys/mainnet-owner.json", vault: 0 })).toBe(
      "./ghostctl --network mainnet repay --key keys/mainnet-owner.json --vault 0",
    );
  });
  it("Probelauf und Empfänger", () => {
    expect(
      commandFor("testnet-10", "send", { key: "keys/tn10-user.json", to: "kaspatest:qabc", kas: "1.5" }, true),
    ).toBe("./ghostctl --network testnet-10 --dry-run send --key keys/tn10-user.json --to kaspatest:qabc --kas 1.5");
  });
  it("Orakel-Update mit Komitee", () => {
    expect(commandFor("mainnet", "oracle-update", { key: "keys/mainnet-owner.json", committee: "keys/mainnet-committee.json" })).toBe(
      "./ghostctl --network mainnet oracle-update --key keys/mainnet-owner.json --committee keys/mainnet-committee.json",
    );
  });
});

describe("Hilfen", () => {
  it("cliDecimal", () => {
    expect(cliDecimal(150_000_000n)).toBe("1.5");
    expect(cliDecimal(1n)).toBe("0.00000001");
    expect(cliDecimal(500n, 2)).toBe("5");
  });
  it("shellArg quotet nur bei Bedarf", () => {
    expect(shellArg("keys/a.json")).toBe("keys/a.json");
    expect(shellArg("mein key.json")).toBe("'mein key.json'");
    expect(shellArg("a'b; rm")).toBe("'a'\\''b; rm'");
  });
});
