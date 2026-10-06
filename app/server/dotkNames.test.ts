// .k-Namen: nur am eigenen Node bewiesene Adressen werden herausgegeben.
// Die Registry-API wird hier durch ein falsches fetch ersetzt, der Node durch
// eine Liste von UTXOs – so lässt sich eine lügende API nachstellen.
import { Dotk, decodeAddress, toHex } from "@dotk/sdk";
import { describe, expect, it } from "vitest";
import { looksLikeName, resolveName, type UtxoSource } from "./dotkNames.ts";

const A = "kaspa:qzh7m3fcmyzcdfr9cpwe2qdzquhqpv2fsyvsjra00udklflgvesmcwusm546u";
const B = "kaspa:qrkjl20t34fph4wkg0fg4lxk0cc8hlzt6j3e72w67cc705rmcw442ugapc9pf";
const dotk = new Dotk({ network: "mainnet", api: null });
const REG = dotk.registryCovenantId;

/** API, die für „alice“ den Besitzer `addr` nennt */
const api = (addr: string): typeof fetch =>
  (async (input: string | URL | Request) => {
    const u = String(input);
    if (!u.includes("/names/alice")) return new Response(JSON.stringify({ error: "not found", code: "not_found" }), { status: 404, headers: { "content-type": "application/json" } });
    const body = { name: "alice", ownerType: 0, owner: toHex(decodeAddress(addr).payload), address: addr, deedAddress: dotk.deedAddress("alice", addr), registryCovenantId: REG };
    return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;

/** Node, auf dem nur die Urkunde von alice → A liegt */
const node: UtxoSource = async (_n, addresses) =>
  addresses.filter((a) => a === dotk.deedAddress("alice", A)).map((address) => ({ address, covenantId: REG, transactionId: "11".repeat(32), index: 0, daaScore: 1 }));

describe("resolveName", () => {
  it("gibt die Adresse nur heraus, wenn der Node die Urkunde bestätigt", async () => {
    const r = await resolveName("mainnet", "Alice.K", node, api(A));
    expect(r).toMatchObject({ ok: true, display: "alice.k", address: A, proven: true });
  });
  it("eine lügende Registry-API (anderer Besitzer) liefert keine Adresse", async () => {
    const r = await resolveName("mainnet", "alice.k", node, api(B));
    expect(r.ok).toBe(false);
    expect(r.address).toBeUndefined();
  });
  it("Node nicht erreichbar: keine Adresse", async () => {
    const down: UtxoSource = async () => {
      throw new Error("Node nicht erreichbar");
    };
    const r = await resolveName("mainnet", "alice.k", down, api(A));
    expect(r.ok).toBe(false);
    expect(r.address).toBeUndefined();
  });
  it("unvergebener Name und Unternamen", async () => {
    expect(await resolveName("mainnet", "bob.k", node, api(A))).toMatchObject({ ok: false, notFound: true });
    expect((await resolveName("mainnet", "bob.alice.k", node, api(A))).ok).toBe(false);
  });
  it("Adressen und Schlüssel sind keine Namen", () => {
    expect(looksLikeName("alice.k")).toBe(true);
    expect(looksLikeName(A)).toBe(false);
    expect(looksLikeName("ab".repeat(32))).toBe(false);
    expect(looksLikeName("../keys/x")).toBe(false);
  });
});
