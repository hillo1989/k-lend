import { describe, expect, it } from "vitest";
import { fetchHistory, historyCache, parseCoinGecko, parseDays, parseKraken } from "./history.ts";

const NOW = Date.UTC(2026, 8, 29);
const DAY = 86_400_000;

function fakeFetch(routes: Record<string, unknown | Error>, calls: string[] = []): typeof fetch {
  return (async (input: string | URL | Request) => {
    const url = String(input);
    calls.push(url);
    const key = Object.keys(routes).find((k) => url.includes(k));
    const r = key === undefined ? new Error("unbekannt") : routes[key];
    if (r instanceof Error) throw r;
    return new Response(JSON.stringify(r), { status: 200 });
  }) as typeof fetch;
}

describe("Kursverlauf", () => {
  it("nimmt nur 7, 30 und 365 Tage an", () => {
    expect(parseDays("7")).toBe(7);
    expect(parseDays("30")).toBe(30);
    expect(parseDays("365")).toBe(365);
    for (const bad of [null, "", "1", "90", "7; rm", "abc"]) expect(parseDays(bad)).toBeNull();
  });

  it("CoinGecko: sortiert, verwirft ungültige Werte", () => {
    const pts = parseCoinGecko({ prices: [[3, 0.05], [1, 0.04], [2, null], [4, -1], "x", [5, 0.06]] });
    expect(pts).toEqual([[1, 0.04], [3, 0.05], [5, 0.06]]);
    expect(() => parseCoinGecko({ error: "rate limit" })).toThrow();
  });

  it("Kraken: Schlusskurs, nur der gewünschte Zeitraum", () => {
    const row = (daysAgo: number, close: string) => [(NOW - daysAgo * DAY) / 1000, "0", "0", "0", close, "0", "0", 1];
    const pts = parseKraken({ error: [], result: { KASUSD: [row(40, "0.07"), row(20, "0.05"), row(1, "0.046")], last: 1 } }, 30, NOW);
    expect(pts).toEqual([
      [NOW - 20 * DAY, 0.05],
      [NOW - DAY, 0.046],
    ]);
    expect(() => parseKraken({ error: ["EQuery:Unknown asset pair"] }, 7, NOW)).toThrow(/Unknown/);
  });

  it("fällt auf Kraken zurück, wenn CoinGecko ausfällt", async () => {
    const h = await fetchHistory(
      7,
      fakeFetch({
        coingecko: new Error("429"),
        kraken: { error: [], result: { KASUSD: [[(NOW - 2 * DAY) / 1000, 0, 0, 0, "0.05"], [(NOW - DAY) / 1000, 0, 0, 0, "0.047"]] } },
      }),
      NOW,
    );
    expect(h.source).toBe("Kraken");
    expect(h.points).toHaveLength(2);
  });

  it("meldet einen Fehler, wenn beide Quellen ausfallen, und merkt ihn sich nicht", async () => {
    const calls: string[] = [];
    const get = historyCache(fakeFetch({ coingecko: new Error("down"), kraken: new Error("down") }, calls));
    const a = JSON.parse(await get(30));
    expect(a.ok).toBe(false);
    expect(a.error).toMatch(/CoinGecko.*Kraken/);
    await get(30);
    expect(calls.length).toBe(4); // zweiter Aufruf fragt erneut
  });

  it("cacht erfolgreiche Abrufe je Zeitraum", async () => {
    const calls: string[] = [];
    const get = historyCache(fakeFetch({ coingecko: { prices: [[1, 0.04], [2, 0.05]] } }, calls));
    const [a, b] = await Promise.all([get(7), get(7)]);
    expect(a).toBe(b);
    await get(7);
    expect(calls.length).toBe(1);
    await get(365);
    expect(calls.length).toBe(2);
    expect(JSON.parse(a)).toMatchObject({ ok: true, days: 7, source: "CoinGecko" });
  });
});
