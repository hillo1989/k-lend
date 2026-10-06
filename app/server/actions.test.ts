import { describe, expect, it } from "vitest";
import {
  aboNeedsRun,
  buildAboListArgs,
  buildAboRunArgs,
  buildActionArgs,
  buildKeygenArgs,
  buildMessagesArgs,
  checkAmount,
  checkDate,
  checkMessage,
  checkRequest,
  buildTresorListArgs,
  buildTresorPayArgs,
  isNodeError,
  localDate,
  TRESOR_PMT_LAG_MS,
  tresorNeedsRun,
  ValidationError,
} from "./actions.ts";

const files = new Set(["keys/tn10-deployer.json", "keys/tn10-user.json", "keys/tn10-committee.json", "keys/mainnet-owner.json"]);
const exists = (p: string) => files.has(p);
const base = { network: "testnet-10", dryRun: true };

const build = (r: Record<string, unknown>) => buildActionArgs({ ...base, ...r } as never, exists);
const fails = (r: Record<string, unknown>, msg?: RegExp) => {
  let err: unknown;
  try {
    build(r);
  } catch (e) {
    err = e;
  }
  expect(err).toBeInstanceOf(ValidationError);
  if (msg) expect((err as Error).message).toMatch(msg);
};

describe("buildActionArgs", () => {
  it("mint als Probelauf", () => {
    expect(build({ action: "mint", params: { key: "keys/tn10-deployer.json", vault: 0, ghost: "2" } }).args).toEqual([
      "--network", "testnet-10", "--json", "--ja", "--dry-run", "mint", "--key", "keys/tn10-deployer.json", "--vault", "0", "--ghost", "2",
    ]);
  });

  it("ohne dryRun kein --dry-run", () => {
    const a = build({ action: "deposit", dryRun: false, params: { key: "keys/tn10-deployer.json", vault: "0", kas: "1.5" } }).args;
    expect(a).not.toContain("--dry-run");
    expect(a.slice(-6)).toEqual(["--key", "keys/tn10-deployer.json", "--vault", "0", "--kas", "1.5"]);
  });

  it("repay ohne Betrag = ganze Schuld", () => {
    expect(build({ action: "repay", params: { key: "keys/tn10-deployer.json", vault: 0 } }).args.slice(-4)).toEqual([
      "--key", "keys/tn10-deployer.json", "--vault", "0",
    ]);
  });

  it("liquidate: ganz oder als Teil-Liquidation (Version 2)", () => {
    expect(build({ action: "liquidate", params: { key: "keys/tn10-deployer.json", vault: 0 } }).args.slice(-4)).toEqual([
      "--key", "keys/tn10-deployer.json", "--vault", "0",
    ]);
    expect(build({ action: "liquidate", params: { key: "keys/tn10-deployer.json", vault: 0, ghost: "2.5" } }).args.slice(-2)).toEqual([
      "--ghost", "2.5",
    ]);
    fails({ action: "liquidate", params: { key: "keys/tn10-deployer.json", vault: 0, ghost: "-1" } });
  });

  it("redeem: Vault und Betrag sind Pflicht (Version 3)", () => {
    expect(build({ action: "redeem", params: { key: "keys/tn10-user.json", vault: 2, ghost: "1.5" } }).args.slice(-7)).toEqual([
      "redeem", "--key", "keys/tn10-user.json", "--vault", "2", "--ghost", "1.5",
    ]);
    fails({ action: "redeem", params: { key: "keys/tn10-user.json", vault: 2 } });
    fails({ action: "redeem", params: { key: "keys/tn10-user.json", vault: 2, ghost: "0" } });
    fails({ action: "redeem", params: { key: "keys/tn10-user.json", ghost: "1" } });
    fails({ action: "redeem", params: { key: "keys/tn10-user.json", vault: 2, ghost: "1", keep: "1" } }, /Unerwarteter Parameter/);
  });

  it("sweep: nur Schlüssel und Vault (Audit 11 A11-V-4)", () => {
    expect(build({ action: "sweep", params: { key: "keys/tn10-user.json", vault: 3 } }).args.slice(-5)).toEqual(["sweep", "--key", "keys/tn10-user.json", "--vault", "3"]);
    fails({ action: "sweep", params: { key: "keys/tn10-user.json" } });
    fails({ action: "sweep", params: { key: "keys/tn10-user.json", vault: 3, ghost: "1" } }, /Unerwarteter Parameter/);
  });

  it("withdraw nutzt --keep", () => {
    expect(build({ action: "withdraw", params: { key: "keys/tn10-deployer.json", vault: 0, keep: "2999" } }).args).toContain("--keep");
  });

  it("send: Adresse muss zum Netz passen", () => {
    const addr = "kaspatest:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz";
    expect(build({ action: "send", params: { key: "keys/tn10-user.json", to: addr, kas: "1" } }).args).toContain(addr);
    fails({ action: "send", params: { key: "keys/tn10-user.json", to: addr.replace("kaspatest:", "kaspa:"), kas: "1" } }, /passt nicht zum Netz/);
    fails({ action: "send", params: { key: "keys/tn10-user.json", to: "kaspatest:x; rm -rf /", kas: "1" } });
    expect(build({ action: "send", params: { key: "keys/tn10-user.json", to: "keys/tn10-deployer.json", kas: "1" } }).args).toContain(
      "keys/tn10-deployer.json",
    );
  });

  it("transfer: x-only oder Schlüsseldatei", () => {
    const x = "d64cbe281ed8bc5d22e521ce4534de0a176b6f5dd1d43762faf0a9e2fe90af6a";
    expect(build({ action: "transfer", params: { key: "keys/tn10-deployer.json", to: x, ghost: "0.5" } }).args).toContain(x);
    fails({ action: "transfer", params: { key: "keys/tn10-deployer.json", to: x.toUpperCase(), ghost: "0.5" } });
  });

  it("transfer: Kaspa-Adresse nur im passenden Netz", () => {
    const a = "kaspatest:qptlzrcs9eeazs7m2e50llcl686sl2yfzsw8543vg3gymc20ywmr7cqwspkhd";
    expect(build({ action: "transfer", params: { key: "keys/tn10-deployer.json", to: a, ghost: "0.5" } }).args).toContain(a);
    fails({ action: "transfer", params: { key: "keys/tn10-deployer.json", to: a.replace("kaspatest:", "kaspa:"), ghost: "0.5" } }, /passt nicht zum Netz/);
  });

  it("Pool: Anlegen, Tauschen mit Mindestbetrag", () => {
    const k = "keys/tn10-deployer.json";
    expect(build({ action: "pool-open", params: { key: k, kas: "100", ghost: "2" } }).args.slice(-7)).toEqual(["pool-open", "--key", k, "--kas", "100", "--ghost", "2"]);
    expect(build({ action: "swap", params: { key: k, kas: "10", min: "0.4" } }).args.slice(-6)).toEqual(["--key", k, "--kas", "10", "--min-ghost", "0.4"]);
    expect(build({ action: "swap", params: { key: k, ghost: "1", min: "20" } }).args.slice(-4)).toEqual(["--ghost", "1", "--min-kas", "20"]);
    fails({ action: "swap", params: { key: k, kas: "10", ghost: "1", min: "1" } }, /entweder/);
    fails({ action: "swap", params: { key: k, kas: "10" } }, /Mindestbetrag/);
    expect(build({ action: "pool-add", params: { key: k, kas: "10", ghost: "0.5" } }).args.slice(-4)).toEqual(["--kas", "10", "--ghost", "0.5"]);
    expect(build({ action: "pool-remove", params: { key: k, percent: "50" } }).args.slice(-2)).toEqual(["--percent", "50"]);
    fails({ action: "pool-remove", params: { key: k, percent: "101" } }, /höchstens 100/);
    fails({ action: "pool-remove", params: { key: k, kas: "1" } }, /Unerwarteter Parameter/);
  });

  it("oracle-update mit optionalem Preis und Zins", () => {
    const a = build({ action: "oracle-update", params: { key: "keys/tn10-deployer.json", committee: "keys/tn10-committee.json", usd: "0.05", rate: "0" } }).args;
    expect(a.slice(-6)).toEqual(["--committee", "keys/tn10-committee.json", "--usd", "0.05", "--rate", "0"]);
    fails({ action: "oracle-update", params: { key: "keys/tn10-deployer.json", committee: "keys/tn10-committee.json", rate: "101" } });
  });

  it("Mainnet ohne Bestätigung nur als Probelauf", () => {
    const p = { key: "keys/mainnet-owner.json", vault: 0, ghost: "1" };
    expect(buildActionArgs({ network: "mainnet", action: "mint", params: p, dryRun: true }, exists).args).toContain("--dry-run");
    fails({ network: "mainnet", action: "mint", dryRun: false, params: p }, /Bestätigung/);
    fails({ network: "mainnet", action: "mint", dryRun: false, confirmMainnet: "true", params: p }, /Bestätigung/);
    expect(buildActionArgs({ network: "mainnet", action: "mint", params: p, dryRun: false, confirmMainnet: true }, exists).args).not.toContain("--dry-run");
  });

  it("lehnt Unsinn ab", () => {
    fails({ network: "devnet", action: "mint", params: {} }, /Netz/);
    fails({ action: "deploy", params: {} }, /Aktion/);
    fails({ action: "mint", dryRun: "ja", params: {} });
    fails({ action: "mint", params: { key: "../keys/tn10-deployer.json", vault: 0, ghost: "1" } }, /keys/);
    fails({ action: "mint", params: { key: "keys/fehlt.json", vault: 0, ghost: "1" } }, /gibt es nicht/);
    fails({ action: "mint", params: { key: "keys/tn10-deployer.json", vault: -1, ghost: "1" } }, /Vault/);
    fails({ action: "mint", params: { key: "keys/tn10-deployer.json", vault: 1.5, ghost: "1" } }, /Vault/);
    fails({ action: "mint", params: { key: "keys/tn10-deployer.json", vault: 0, ghost: "0" } }, /größer als 0/);
    fails({ action: "mint", params: { key: "keys/tn10-deployer.json", vault: 0, ghost: "1", extra: "--ja" } }, /Unerwarteter/);
    fails({ action: "close", params: null });
  });
});

describe("isNodeError", () => {
  it.each([
    "Verbindung fehlgeschlagen (3 Versuche): wRPC -> WebSocket -> Connection timeout",
    "HTTP 502 Bad Gateway",
    "ghostctl status fehlgeschlagen: Fehler: Verbindung fehlgeschlagen",
  ])("erkennt %s", (m) => expect(isNodeError(m)).toBe(true));
  it.each(["Mindestquote verletzt: höchstens 70 GHOST prägbar", "Input 0: VerifyError", "zu wenig GHOST: 1 vorhanden, 2 nötig"])(
    "kein Node-Fehler: %s",
    (m) => expect(isNodeError(m)).toBe(false),
  );
});

describe("checkAmount", () => {
  it.each([
    ["1", "1"],
    ["001.500", "1.5"],
    ["0.00000001", "0.00000001"],
    [2.25, "2.25"],
  ])("%s → %s", (i, o) => expect(checkAmount(i, "x")).toBe(o));
  it.each(["1,5", "-1", "1e5", "0.000000001", "abc", "", " 1 2", "Infinity"])("lehnt %s ab", (i) =>
    expect(() => checkAmount(i, "x")).toThrow(ValidationError),
  );
  it("NaN und Infinity als Zahl", () => {
    expect(() => checkAmount(Number.NaN, "x")).toThrow(ValidationError);
    expect(() => checkAmount(Number.POSITIVE_INFINITY, "x")).toThrow(ValidationError);
  });
});

describe("buildKeygenArgs", () => {
  it("baut keys/<name>.json", () => {
    expect(buildKeygenArgs("testnet-10", "mein-key", exists)).toEqual({
      file: "keys/mein-key.json",
      args: ["--network", "testnet-10", "--json", "keygen", "keys/mein-key.json"],
    });
  });
  it("vorhandene Datei und schlechte Namen", () => {
    expect(() => buildKeygenArgs("testnet-10", "tn10-user", exists)).toThrow(/gibt es schon/);
    expect(() => buildKeygenArgs("testnet-10", "../x", exists)).toThrow(ValidationError);
    expect(() => buildKeygenArgs("testnet-10", "AB", exists)).toThrow(ValidationError);
  });
});

describe("checkRequest", () => {
  const ok = { method: "POST", host: "localhost:5180", origin: "http://localhost:5180", contentType: "application/json", clientHeader: "1", port: 5180 };
  it("eigene Seite ist erlaubt", () => {
    expect(checkRequest(ok)).toBe(null);
    expect(checkRequest({ ...ok, host: "127.0.0.1:5180", origin: "http://127.0.0.1:5180" })).toBe(null);
    expect(checkRequest({ ...ok, method: "GET", origin: undefined, clientHeader: undefined, contentType: undefined })).toBe(null);
  });
  it("fehlende Kopfzeile → 403", () => expect(checkRequest({ ...ok, clientHeader: undefined })?.status).toBe(403));
  it("fremder Origin → 403", () => expect(checkRequest({ ...ok, origin: "http://evil.example" })?.status).toBe(403));
  it("Origin null → 403", () => expect(checkRequest({ ...ok, origin: "null" })?.status).toBe(403));
  it("falscher Host (DNS-Rebinding) → 403", () => {
    expect(checkRequest({ ...ok, host: "evil.example:5180", origin: undefined })?.status).toBe(403);
    expect(checkRequest({ ...ok, host: "localhost:9999", origin: undefined })?.status).toBe(403);
  });
  it("falscher Content-Type", () => expect(checkRequest({ ...ok, contentType: "text/plain" })?.status).toBe(415));
});

describe("Nachricht bei send/transfer", () => {
  const addr = "kaspatest:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz";
  const k = "keys/tn10-user.json";
  it("als --message=… und optional --onchain-message", () => {
    expect(build({ action: "send", params: { key: k, to: addr, kas: "1", message: " Miete Oktober ", onchain: true } }).args.slice(-2)).toEqual([
      "--message=Miete Oktober",
      "--onchain-message",
    ]);
    // ein Text wie eine Option bleibt Text
    expect(build({ action: "transfer", params: { key: k, to: addr, ghost: "1", message: "--ja" } }).args.slice(-1)).toEqual(["--message=--ja"]);
    expect(build({ action: "send", params: { key: k, to: addr, kas: "1", message: "", onchain: false } }).args.slice(-1)).toEqual(["1"]);
  });
  it("lehnt Unzulässiges ab", () => {
    fails({ action: "send", params: { key: k, to: addr, kas: "1", onchain: true } }, /keine Nachricht/);
    fails({ action: "send", params: { key: k, to: addr, kas: "1", message: "a\nb" } }, /Steuerzeichen/);
    fails({ action: "send", params: { key: k, to: addr, kas: "1", message: "\u202eabc" } }, /Steuerzeichen/);
    fails({ action: "send", params: { key: k, to: addr, kas: "1", message: "x".repeat(101) } }, /100 Zeichen/);
    fails({ action: "send", params: { key: k, to: addr, kas: "1", message: 5 } }, /Text/);
    fails({ action: "send", params: { key: k, to: addr, kas: "1", message: "a", onchain: "ja" } }, /true oder false/);
  });
  it("checkMessage zählt Zeichen, nicht Bytes", () => {
    expect(checkMessage("🏠".repeat(100))).toHaveLength(200);
    expect(checkMessage(undefined)).toBe("");
  });
});

describe("Daueraufträge", () => {
  const k = "keys/tn10-user.json";
  const addr = "kaspatest:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz";
  const add = (p: Record<string, unknown>) => build({ action: "abo-add", dryRun: false, params: { key: k, asset: "KAS", to: addr, amount: "1.5", interval: "monthly", start: "2027-01-31", ...p } });
  it("abo add mit genau definierten Parametern", () => {
    expect(add({ count: 3, message: "Miete", onchain: true }).args).toEqual([
      "--network", "testnet-10", "--json", "--ja", "abo", "add", "--key", k, "--asset", "KAS", "--to", addr, "--amount", "1.5",
      "--interval", "monthly", "--start", "2027-01-31", "--count", "3", "--message=Miete", "--onchain-message",
    ]);
    expect(add({ asset: "GHOST", to: "keys/tn10-deployer.json", interval: "14", end: "2027-12-31" }).args.slice(8, 20)).toEqual([
      "--asset", "GHOST", "--to", "keys/tn10-deployer.json", "--amount", "1.5", "--interval", "14", "--start", "2027-01-31", "--end", "2027-12-31",
    ]);
  });
  it("prüft Eingaben", () => {
    const bad = (p: Record<string, unknown>, m: RegExp) => fails({ action: "abo-add", dryRun: false, params: { key: k, asset: "KAS", to: addr, amount: "1", interval: "monthly", start: "2027-01-01", ...p } }, m);
    bad({ asset: "BTC" }, /KAS oder GHOST/);
    bad({ amount: "0" }, /größer als 0/);
    bad({ amount: "-1" }, /Zahl/);
    bad({ to: "kaspa:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz" }, /passt nicht zum Netz/);
    bad({ to: k }, /Absender selbst/);
    bad({ interval: "yearly" }, /Intervall/);
    bad({ interval: "0" }, /Intervall/);
    bad({ interval: "3651" }, /Intervall/);
    bad({ start: "2027-02-30" }, /kein gültiges Datum/);
    bad({ start: "morgen" }, /JJJJ-MM-TT/);
    bad({ end: "2026-12-31" }, /vor dem Start/);
    bad({ end: "2027-12-31", count: 3 }, /nicht beides/);
    bad({ count: 0 }, /Anzahl/);
    bad({ count: "3; rm" }, /Anzahl/);
    bad({ message: "a\u0000b" }, /Steuerzeichen/);
    bad({ onchain: true }, /keine Nachricht/);
    bad({ extra: 1 }, /Unerwarteter Parameter/);
  });
  it("pausieren, fortsetzen, beenden nur mit ID, ohne Schlüssel und ohne Mainnet-Bestätigung", () => {
    expect(build({ action: "abo-pause", dryRun: false, params: { id: "0a1b2c3d" } }).args).toEqual(["--network", "testnet-10", "--json", "--ja", "abo", "pause", "0a1b2c3d"]);
    expect(buildActionArgs({ network: "mainnet", action: "abo-remove", params: { id: "0a1b2c3d" }, dryRun: false }, exists).args.slice(-3)).toEqual(["abo", "remove", "0a1b2c3d"]);
    fails({ action: "abo-resume", params: { id: "../x" } }, /ID/);
    fails({ action: "abo-resume", params: { id: "0A1B2C3D" } }, /ID/);
    fails({ action: "abo-pause", params: { id: "0a1b2c3d", key: k } }, /Unerwarteter/);
    // Anlegen im Mainnet braucht die Bestätigung wie jede Aktion, die später sendet
    fails({ network: "mainnet", action: "abo-add", dryRun: false, params: { key: "keys/mainnet-owner.json", asset: "KAS", to: "keys/tn10-user.json", amount: "1", interval: "weekly", start: "2027-01-01" } }, /Bestätigung/);
  });
  it("Liste und Lauf", () => {
    expect(buildAboListArgs("mainnet")).toEqual(["--network", "mainnet", "--json", "abo", "list"]);
    expect(() => buildAboListArgs("devnet")).toThrow(ValidationError);
    expect(buildAboRunArgs("testnet-10")).toEqual(["--network", "testnet-10", "--json", "--ja", "abo", "run"]);
    expect(checkDate("2028-02-29", "x")).toBe("2028-02-29");
    expect(() => checkDate("2027-02-29", "x")).toThrow(/kein gültiges/);
    expect(localDate(new Date(2027, 0, 5, 23, 30))).toBe("2027-01-05");
  });
  it("aboNeedsRun wie abo::needs_run", () => {
    const a = { nextDue: "2027-02-01", paused: false, ended: null, inflight: null, retry: null };
    const f = (x: Record<string, unknown>) => ({ abos: [{ ...a, ...x }], archive: [] });
    expect(aboNeedsRun(f({}), "2027-01-31", 0)).toBe(false);
    expect(aboNeedsRun(f({}), "2027-02-01", 0)).toBe(true);
    expect(aboNeedsRun(f({ paused: true }), "2027-03-01", 0)).toBe(false);
    expect(aboNeedsRun(f({ nextDue: null }), "2027-03-01", 0)).toBe(false);
    expect(aboNeedsRun(f({ nextDue: null, retry: { after: 100 } }), "2027-03-01", 99)).toBe(false);
    expect(aboNeedsRun(f({ nextDue: null, retry: { after: 100 } }), "2027-03-01", 100)).toBe(true);
    // offene Zahlung wird immer geklärt, auch pausiert oder im Archiv
    expect(aboNeedsRun({ abos: [], archive: [{ ...a, paused: true, ended: "x", inflight: { txid: "ab" } }] }, "2020-01-01", 0)).toBe(true);
    expect(aboNeedsRun(null, "2027-01-01", 0)).toBe(false);
    expect(aboNeedsRun({ abos: "kaputt" }, "2027-01-01", 0)).toBe(false);
  });
});

describe("Tresore", () => {
  const k = "keys/tn10-user.json";
  const addr = "kaspatest:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz";
  const code = "ghost-tresor:2:eyJuZXR3b3JrIjoidGVzdG5ldC0xMCJ9_-Ab";
  const open = (p: Record<string, unknown>, extra: Record<string, unknown> = {}) =>
    build({ action: "tresor-open", dryRun: false, ...extra, params: { key: k, to: addr, amount: "10", interval: "monthly", start: "2027-01-31", ...p } });
  it("anlegen mit genau definierten Parametern", () => {
    expect(open({ count: 12, message: "Miete", onchain: true }).args).toEqual([
      "--network", "testnet-10", "--json", "--ja", "tresor", "open", "--key", k, "--to", addr, "--amount", "10",
      "--interval", "monthly", "--start", "2027-01-31", "--count", "12", "--message=Miete", "--onchain-message",
    ]);
    expect(open({ fund: "120.5", interval: "7" }, { dryRun: true }).args).toEqual([
      "--network", "testnet-10", "--json", "--ja", "--dry-run", "tresor", "open", "--key", k, "--to", addr, "--amount", "10",
      "--interval", "7", "--start", "2027-01-31", "--fund", "120.5",
    ]);
  });
  it("anlegen prüft Eingaben", () => {
    const bad = (p: Record<string, unknown>, m: RegExp) => fails({ action: "tresor-open", dryRun: false, params: { key: k, to: addr, amount: "10", interval: "monthly", start: "2027-01-01", count: 3, ...p } }, m);
    bad({ amount: "0.99999999" }, /mindestens 1 KAS/);
    bad({ amount: "abc" }, /Zahl/);
    bad({ to: k }, /Absender selbst/);
    bad({ to: "kaspa:qrtye03grmvtchfzu5suu3f5mc9pw6m0thgagdmzltc2nch7jzhk5yczhlmrz" }, /passt nicht zum Netz/);
    bad({ interval: "yearly" }, /Intervall/);
    bad({ start: "2027-02-30" }, /kein gültiges Datum/);
    bad({ count: 0 }, /Anzahl/);
    bad({ count: undefined }, /Startguthaben/);
    bad({ fund: "-5" }, /Zahl/);
    bad({ asset: "KAS" }, /Unerwarteter Parameter/);
    bad({ end: "2027-12-31" }, /Unerwarteter Parameter/);
    bad({ maxFee: "1" }, /Unerwarteter Parameter/);
    bad({ key: "../keys/x.json" }, /keys\/<name>\.json/);
    bad({ message: "a\nb" }, /Steuerzeichen/);
  });
  it("auslösen, auffüllen, kündigen nur mit Tresor-ID", () => {
    expect(build({ action: "tresor-pay", dryRun: false, params: { id: "0a1b2c3d" } }).args).toEqual(["--network", "testnet-10", "--json", "--ja", "tresor", "pay", "0a1b2c3d"]);
    expect(build({ action: "tresor-pay", params: { id: "0a1b2c3d", key: k } }).args.slice(-5)).toEqual(["tresor", "pay", "0a1b2c3d", "--key", k]);
    expect(build({ action: "tresor-topup", dryRun: false, params: { id: "0a1b2c3d", key: k, kas: "25" } }).args.slice(4)).toEqual(["tresor", "topup", "0a1b2c3d", "--key", k, "--kas", "25"]);
    expect(build({ action: "tresor-cancel", dryRun: false, params: { id: "0a1b2c3d", key: k } }).args.slice(4)).toEqual(["tresor", "cancel", "0a1b2c3d", "--key", k]);
    fails({ action: "tresor-pay", params: { id: "0A1B2C3D" } }, /8 Hex/);
    fails({ action: "tresor-pay", params: { id: "--all" } }, /8 Hex/);
    fails({ action: "tresor-cancel", params: { id: "0a1b2c3d" } }, /Schlüsseldatei/);
    fails({ action: "tresor-topup", params: { id: "0a1b2c3d", key: k, kas: "0" } }, /größer als 0/);
    fails({ action: "tresor-cancel", params: { id: "0a1b2c3d", key: k, to: addr } }, /Unerwarteter/);
  });
  it("Mainnet: Senden nur mit Bestätigung, Übernehmen und Abgleichen ohne", () => {
    const main = (action: string, params: Record<string, unknown>, confirmMainnet?: boolean) => buildActionArgs({ network: "mainnet", action, params, dryRun: false, confirmMainnet }, exists);
    expect(() => main("tresor-pay", { id: "0a1b2c3d" })).toThrow(/Bestätigung/);
    expect(() => main("tresor-cancel", { id: "0a1b2c3d", key: "keys/mainnet-owner.json" })).toThrow(/Bestätigung/);
    expect(main("tresor-pay", { id: "0a1b2c3d" }, true).args.slice(-3)).toEqual(["tresor", "pay", "0a1b2c3d"]);
    expect(main("tresor-import", { code }).args).toEqual(["--network", "mainnet", "--json", "--ja", "tresor", "import", code]);
    expect(main("tresor-sync", {}).args).toEqual(["--network", "mainnet", "--json", "--ja", "tresor", "sync"]);
  });
  it("Tresor-Code wird streng geprüft", () => {
    expect(build({ action: "tresor-import", dryRun: false, params: { code: `  ${code}\n` } }).args.at(-1)).toBe(code);
    fails({ action: "tresor-import", dryRun: false, params: { code: "ghost-tresor:2:abc def ghi jkl mno" } }, /Tresor-Code/);
    fails({ action: "tresor-import", dryRun: false, params: { code: "ghost-tresor:2:--network=mainnet-xxxxxx" } }, /Tresor-Code/);
    fails({ action: "tresor-import", dryRun: false, params: { code: "--help" } }, /Tresor-Code/);
    fails({ action: "tresor-import", dryRun: false, params: { code: `ghost-tresor:2:${"A".repeat(4001)}` } }, /Tresor-Code/);
    fails({ action: "tresor-import", dryRun: false, params: { code: 42 } }, /Tresor-Code/);
    fails({ action: "tresor-import", dryRun: true, params: { code } }, /ohne Probelauf/);
    // A12-1 im Vertrag: alte Codes (Vertrag ohne gebundene Nachricht) klar abgelehnt
    fails({ action: "tresor-import", dryRun: false, params: { code: code.replace("ghost-tresor:2:", "ghost-tresor:1:") } }, /Alter Tresor-Code.*neuen Tresor/);
    fails({ action: "tresor-sync", dryRun: false, params: { id: "0a1b2c3d" } }, /Unerwarteter/);
  });
  it("Liste und automatischer Lauf", () => {
    expect(buildTresorListArgs("testnet-10")).toEqual(["--network", "testnet-10", "--json", "tresor", "list"]);
    expect(() => buildTresorListArgs("x")).toThrow(ValidationError);
    expect(buildTresorPayArgs("mainnet")).toEqual(["--network", "mainnet", "--json", "--ja", "tresor", "pay"]);
  });
  it("tresorNeedsRun wie TresorRec::looks_due ohne Schlüssel", () => {
    const due = 1_801_353_600_000;
    const r = { params: { amount: 1_000_000_000, maxFee: 1_000_000 }, utxo: { value: 5_000_000_000, state: { nextDue: due, left: 3 } }, ended: null, missing: null, retryAfter: null };
    const f = (x: Record<string, unknown> = {}, u: Record<string, unknown> = {}) => ({ tresore: [{ ...r, ...x, utxo: { ...r.utxo, ...u } }] });
    const at = due + TRESOR_PMT_LAG_MS;
    expect(tresorNeedsRun(f(), at - 1)).toBe(false);
    expect(tresorNeedsRun(f(), at)).toBe(true);
    expect(tresorNeedsRun(f({ ended: "x" }), at)).toBe(false);
    // nicht auffindbar: eine Woche lang stündlich erneut nachsehen (A12-16)
    expect(tresorNeedsRun(f({ missing: "x", missingMs: at - 3_600_000, retryAfter: at + 1 }), at)).toBe(false);
    expect(tresorNeedsRun(f({ missing: "x", missingMs: at - 3_600_000, retryAfter: at }), at)).toBe(true);
    expect(tresorNeedsRun(f({ missing: "x", missingMs: at - 7 * 86_400_000 - 1, retryAfter: null }), at)).toBe(false);
    expect(tresorNeedsRun(f({ retryAfter: at + 1 }), at)).toBe(false);
    expect(tresorNeedsRun(f({ retryAfter: at }), at)).toBe(true);
    expect(tresorNeedsRun(f({}, { state: { nextDue: due, left: 0 } }), at)).toBe(false);
    expect(tresorNeedsRun(f({}, { state: { nextDue: due, left: -1 } }), at)).toBe(true);
    // nach Betrag und Höchstgebühr bliebe weniger als 1 KAS: nur mit eigenem Schlüssel
    expect(tresorNeedsRun(f({}, { value: 1_100_999_999 }), at)).toBe(false);
    expect(tresorNeedsRun(f({}, { value: 1_101_000_000 }), at)).toBe(true);
    expect(tresorNeedsRun({ tresore: [{ utxo: { value: "5" } }] }, at)).toBe(false);
    expect(tresorNeedsRun(null, at)).toBe(false);
  });
});

describe("Eingegangene Nachrichten", () => {
  it("baut ghostctl messages nur für eigene Schlüsseldateien", () => {
    expect(buildMessagesArgs("testnet-10", "keys/tn10-user.json", exists).args).toEqual([
      "--network",
      "testnet-10",
      "--json",
      "messages",
      "--key",
      "keys/tn10-user.json",
    ]);
    for (const bad of [null, "", "keys/../x.json", "/etc/passwd", "keys/fehlt.json", "keys/a b.json", "--key=keys/tn10-user.json", ["keys/tn10-user.json"]])
      expect(() => buildMessagesArgs("testnet-10", bad, exists)).toThrow(ValidationError);
    expect(() => buildMessagesArgs("devnet", "keys/tn10-user.json", exists)).toThrow(/Netz/);
  });
});
