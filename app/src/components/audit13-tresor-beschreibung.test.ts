// Audit 13, Gruppe Tresor-Code: Die Tresor-Liste darf nie mehr behaupten, als
// der Vertrag erzwingt. Bei einer verschlüsselten Nachricht bindet der Vertrag
// nur die verschlüsselte Fassung, nicht die Beschreibung im Tresor-Code
// (A13-tresor-1). Dazu die Untergrenze der Höchstgebühr (A13-tresor-4).
//
// Gerendert wird wie in audit12a-tresor.test.ts mit react-dom/server; useState
// wird je Aufrufnummer überschrieben (0 = Liste).
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { KeyEntry } from "../lib/api";
import { setLangGlobal } from "../lib/i18n";
import { maxFeeTooLow, messageSure, TRESOR_MIN_MAX_FEE, type Tresor } from "../lib/tresor";

const g = globalThis as unknown as { __over: Record<number, unknown>; __n: number };

vi.mock("react", async (orig) => {
  const R = await orig<typeof import("react")>();
  const useState = ((init: unknown) => {
    const i = g.__n++;
    return i in g.__over ? [g.__over[i], () => {}] : R.useState(init);
  }) as typeof R.useState;
  return { ...R, default: R, useState };
});

const me: KeyEntry = { file: "keys/tn10-user.json", type: "key", xonly: "aa".repeat(32), address: "kaspatest:qown", kas: 500, ghost: 0, vaults: [] };
const { TresorList } = await import("./TresorList");

beforeEach(() => {
  setLangGlobal("de");
  g.__over = {};
  g.__n = 0;
});

const t0: Tresor = {
  id: "abababab",
  covenantId: "ab".repeat(32),
  owner: "cc".repeat(32),
  recipient: me.xonly,
  ownerAddress: "kaspatest:qqabsender000000000000000000000000000000000000000000",
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
  message: "Ab jetzt an kaspa:qneu zahlen",
  onchain: false,
  encrypted: true,
  messageCheck: "unchecked",
  key: null,
  created: "",
  ended: null,
  missing: null,
  lastError: null,
  due: false,
  history: [],
  code: "ghost-tresor:2:x",
};

function renderList(t: Tresor, account: KeyEntry | null = me) {
  g.__over = { 0: { ok: true, tresore: [t] } };
  g.__n = 0;
  return renderToStaticMarkup(createElement(TresorList, { network: "testnet-10", account, isMain: false, refresh: 0 }));
}

const BOUND = "Jede Zahlung trägt genau sie";

describe("A13-tresor-1: „Jede Zahlung trägt genau sie“ nur, wenn der Vertrag es erzwingt", () => {
  it("übernommen, verschlüsselt, nicht geprüft: kein „genau sie“, gekennzeichnet als nicht geprüft", () => {
    const h = renderList(t0);
    expect(h).not.toContain(BOUND);
    expect(h).toContain("laut Tresor-Code, nicht geprüft");
    expect(h).toContain("Im Vertrag gebunden ist die verschlüsselte Nachricht, die jede Zahlung trägt, nicht diese Beschreibung.");
  });

  it("mit dem Schlüssel des Empfängers als abweichend erkannt: Warnung statt „genau sie“", () => {
    const h = renderList({ ...t0, messageCheck: "mismatch" });
    expect(h).not.toContain(BOUND);
    expect(h).toContain("weicht von der Nachricht in den Zahlungen ab");
    expect(h).toContain("der Tresor-Code wurde verändert");
  });

  it("geprüft (Import mit Empfängerschlüssel), öffentlich oder vom Absender hier angelegt: „genau sie“", () => {
    for (const t of [{ ...t0, messageCheck: "checked" as const }, { ...t0, onchain: true, encrypted: false, messageCheck: "bound" as const }]) {
      const h = renderList(t);
      expect(h).toContain(BOUND);
      expect(h).not.toContain("nicht geprüft");
      expect(h).toContain("laut Tresor-Code"); // Herkunft bleibt genannt
    }
    const owner = renderList({ ...t0, owner: me.xonly, recipient: "dd".repeat(32), key: me.file, messageCheck: "checked" });
    expect(owner).toContain(BOUND);
    expect(owner).not.toContain("laut Tresor-Code");
  });

  it("älteres ghostctl ohne messageCheck: nur öffentlich oder hier angelegt gilt als gebunden", () => {
    const { messageCheck: _, ...old } = t0;
    expect(messageSure(old)).toBe(false);
    expect(renderList(old)).not.toContain(BOUND);
    expect(messageSure({ ...old, onchain: true })).toBe(true);
    expect(messageSure({ ...old, key: me.file })).toBe(true);
    expect(messageSure({ ...old, message: "", key: me.file })).toBe(false);
  });

  it("ohne Nachricht: weder „genau sie“ noch Kennzeichen", () => {
    const h = renderList({ ...t0, message: "", encrypted: false, messageCheck: "none" });
    expect(h).not.toContain(BOUND);
    expect(h).not.toContain("nicht geprüft");
  });

  it("englisch: dieselben Kennzeichen", () => {
    setLangGlobal("en");
    const h = renderList(t0);
    expect(h).toContain("as stated in the vault code, not verified");
    expect(h).not.toContain("every payment carries exactly this message");
  });
});

describe("A13-tresor-4: Höchstgebühr unter der Mindestgebühr", () => {
  it("Untergrenze wie tresor::MIN_MAX_FEE (0,004 KAS)", () => {
    expect(TRESOR_MIN_MAX_FEE).toBe(400_000n);
    expect(maxFeeTooLow({ maxFee: "0.001" })).toBe(true);
    expect(maxFeeTooLow({ maxFee: "0.00399999" })).toBe(true);
    expect(maxFeeTooLow({ maxFee: "0.004" })).toBe(false);
    expect(maxFeeTooLow({ maxFee: "0.01" })).toBe(false);
  });

  it("die Liste warnt bei einem Tresor mit zu kleiner Höchstgebühr", () => {
    expect(renderList({ ...t0, maxFee: "0.001" })).toContain("unter der Mindestgebühr einer Zahlung");
    expect(renderList(t0)).not.toContain("unter der Mindestgebühr einer Zahlung");
  });
});
