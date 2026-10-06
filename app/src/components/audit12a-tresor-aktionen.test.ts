// Audit 12, Nachprüfung (Gruppe a): Welche Parameter gehen beim Klick in der
// Tresor-Liste wirklich an ghostctl? Die Tests in audit12a-tresor.test.ts
// prüfen payParams und topupKas nur als Funktionen und den Text der Knöpfe;
// ein Knopf, der wieder {id, key} statt payParams sendet (A12-10: stille
// Zahlung vom eigenen Schlüssel) oder beim Auffüllen den Rohtext (A12-18),
// blieb grün.
//
// Die Komponente wird ohne DOM als Funktion aufgerufen: Hooks sind ersetzt
// (useState liefert je Aufrufnummer einen festen Wert und merkt sich, was
// gesetzt wird; useEffect läuft nicht), dann wird im Elementbaum der Knopf
// gesucht und sein onClick ausgeführt.
//
// Zweite Nachprüfung: Der erste Knopf „Fällige Zahlung abholen“ darf nur die
// Rückfrage öffnen. Sendete er wieder direkt, blieben alle Tests grün (Rückbau
// T6b des Prüfers).
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { KeyEntry } from "../lib/api";
import { setLangGlobal } from "../lib/i18n";
import type { Tresor } from "../lib/tresor";

const g = globalThis as unknown as { __over: Record<number, unknown>; __n: number; __set: [number, unknown][] };

vi.mock("react", async (orig) => {
  const R = await orig<typeof import("react")>();
  const useState = ((init: unknown) => {
    const i = g.__n++;
    return [i in g.__over ? g.__over[i] : typeof init === "function" ? (init as () => unknown)() : init, (v: unknown) => g.__set.push([i, v])];
  }) as unknown as typeof R.useState;
  return { ...R, default: R, useState, useEffect: () => {}, useId: () => "id" };
});
const runAction = vi.fn(async (_req: { network: string; action: string; params: Record<string, unknown>; dryRun: boolean; confirmMainnet?: boolean }) => ({ ok: true, back: "1" }));
vi.mock("../lib/api", async (orig) => ({ ...(await orig<typeof import("../lib/api")>()), runAction }));

const { TresorList } = await import("./TresorList");

type El = { type: unknown; props: Record<string, unknown> };
function* walk(n: unknown): Generator<El> {
  if (Array.isArray(n)) {
    for (const c of n) yield* walk(c);
  } else if (n && typeof n === "object" && "props" in n) {
    const e = n as El;
    yield e;
    yield* walk(e.props.children);
  }
}
const text = (n: unknown): string =>
  typeof n === "string" || typeof n === "number" ? String(n) : Array.isArray(n) ? n.map(text).join("") : n && typeof n === "object" && "props" in n ? text((n as El).props.children) : "";

const me: KeyEntry = { file: "keys/main-user.json", type: "key", xonly: "aa".repeat(32), address: "kaspa:qown", kas: 500, ghost: 0, vaults: [] };
const t0: Tresor = {
  id: "abababab",
  covenantId: "ab".repeat(32),
  owner: "cc".repeat(32),
  recipient: me.xonly,
  ownerAddress: "kaspa:qqabsender",
  recipientAddress: me.address,
  amount: "10",
  maxFee: "0.01",
  anchorDay: 31,
  periodMs: 0,
  nextDue: Date.UTC(2027, 0, 31),
  left: 3,
  value: "50",
  covered: 4,
  outpoint: "x:1",
  message: "",
  onchain: false,
  key: null,
  created: "",
  ended: null,
  missing: null,
  lastError: null,
  due: true,
  history: [],
  code: "ghost-tresor:2:x",
};

/** Elementbaum mit diesem Tresor; useState-Reihenfolge: 0 list, 5 confirm, 6 topup */
function render(t: Tresor, over: Record<number, unknown>) {
  g.__over = { 0: { ok: true, tresore: [t] }, ...over };
  g.__n = 0;
  g.__set = [];
  return TresorList({ network: "mainnet", account: me, isMain: true, refresh: 0 });
}
const button = (tree: unknown, label: string) => [...walk(tree)].find((e) => e.type === "button" && text(e.props.children).includes(label));
async function press(tree: unknown, label: string) {
  const b = button(tree, label);
  expect(b, `Knopf „${label}“`).toBeDefined();
  await (b!.props.onClick as () => unknown)();
  await Promise.resolve();
}

/** Klick auf den Knopf mit dieser Aufschrift; genau ein Aufruf an ghostctl */
async function click(t: Tresor, label: string, over: Record<number, unknown>) {
  await press(render(t, over), label);
  expect(runAction).toHaveBeenCalledTimes(1);
  return runAction.mock.calls[0][0];
}

beforeEach(() => {
  setLangGlobal("de");
  runAction.mockClear();
});

describe("A12-10: „Fällige Zahlung abholen“ öffnet nur die Rückfrage", () => {
  const cases: [string, Tresor][] = [
    ["Tresor trägt die Gebühr", t0],
    ["nur mit eigener Gebühr", { ...t0, value: "11.005", covered: 0 }],
  ];
  for (const [name, t] of cases) {
    it(`${name}: erster Klick sendet nichts`, async () => {
      const tree = render(t, {});
      expect(button(tree, "Wirklich abholen"), "ohne Rückfrage kein Sendeknopf").toBeUndefined();
      await press(tree, "Fällige Zahlung abholen");
      expect(runAction).not.toHaveBeenCalled();
      expect(g.__set).toEqual([[5, "pay-abababab"]]);
    });
  }
  it("in der Rückfrage steht der erste Knopf nicht mehr da", () => {
    const tree = render(t0, { 5: "pay-abababab" });
    expect(button(tree, "Fällige Zahlung abholen")).toBeUndefined();
    expect(button(tree, "Wirklich abholen")).toBeDefined();
  });
});

describe("A12-10: „Wirklich abholen“ sendet payParams, nicht immer den eigenen Schlüssel", () => {
  it("Tresor trägt die Gebühr: ohne Schlüssel", async () => {
    const req = await click(t0, "Wirklich abholen", { 5: "pay-abababab" });
    expect(req.action).toBe("tresor-pay");
    expect(req.params).toEqual({ id: "abababab" });
    expect(req.confirmMainnet).toBe(true);
    expect(req.dryRun).toBe(false);
  });
  it("nur mit eigener Gebühr: mit Schlüssel (und das stand in der Rückfrage)", async () => {
    const req = await click({ ...t0, value: "11.005", covered: 0 }, "Wirklich abholen", { 5: "pay-abababab" });
    expect(req.params).toEqual({ id: "abababab", key: me.file });
  });
});

describe("A12-18: Auffüllen sendet den geprüften Betrag, nicht den Rohtext", () => {
  const own = { ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file, due: false };
  it("„1.000,5“ → 1000.5", async () => {
    const req = await click(own, "Wirklich 1000,5 KAS nachlegen", { 5: "topup-abababab", 6: { abababab: "1.000,5" } });
    expect(req.action).toBe("tresor-topup");
    expect(req.params).toEqual({ id: "abababab", key: me.file, kas: "1000.5" });
  });
  it("„12,5“ → 12.5", async () => {
    const req = await click(own, "Wirklich 12,5 KAS nachlegen", { 5: "topup-abababab", 6: { abababab: " 12,5 " } });
    expect(req.params.kas).toBe("12.5");
  });
});
