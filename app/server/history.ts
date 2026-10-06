// KAS-Kursverlauf für die Statistik-Seite (GET /api/history?days=7|30|365).
// Quelle CoinGecko (7 Tage stündlich, sonst täglich), bei Ausfall Kraken.
// Abgerufen wird vom lokalen Server, nicht vom Browser: die Seite bleibt ohne
// fremde Verbindungen, und der Cache schont die kostenlosen Schnittstellen.

export const HISTORY_DAYS = [7, 30, 365] as const;
export type HistoryDays = (typeof HISTORY_DAYS)[number];
/** [Zeit in ms, USD je KAS] */
export type Point = [number, number];

const CACHE_MS = 10 * 60_000;
const FETCH_TIMEOUT_MS = 15_000;
const DAY_MS = 86_400_000;

export function parseDays(v: string | null): HistoryDays | null {
  const n = Number(v);
  return (HISTORY_DAYS as readonly number[]).includes(n) ? (n as HistoryDays) : null;
}

/** nur endliche, positive Preise in aufsteigender Zeit */
function clean(points: Point[]): Point[] {
  return points.filter(([t, p]) => Number.isFinite(t) && Number.isFinite(p) && p > 0).sort((a, b) => a[0] - b[0]);
}

export function parseCoinGecko(j: unknown): Point[] {
  const prices = (j as { prices?: unknown })?.prices;
  if (!Array.isArray(prices)) throw new Error("CoinGecko: keine Preisliste");
  return clean(prices.filter((x): x is [number, number] => Array.isArray(x) && x.length >= 2).map((x) => [Number(x[0]), Number(x[1])]));
}

/** Kraken-OHLC: [Zeit s, open, high, low, close, …]; es zählt der Schlusskurs */
export function parseKraken(j: unknown, days: HistoryDays, now: number): Point[] {
  const o = j as { error?: unknown[]; result?: Record<string, unknown> };
  if (o?.error?.length) throw new Error(`Kraken: ${String(o.error[0])}`);
  const rows = Object.entries(o?.result ?? {}).find(([k]) => k !== "last")?.[1];
  if (!Array.isArray(rows)) throw new Error("Kraken: keine Kerzen");
  const from = now - days * DAY_MS;
  return clean(rows.filter(Array.isArray).map((r) => [Number(r[0]) * 1000, Number(r[4])] as Point)).filter(([t]) => t >= from);
}

async function getJson(fetchFn: typeof fetch, url: string): Promise<unknown> {
  const r = await fetchFn(url, { signal: AbortSignal.timeout(FETCH_TIMEOUT_MS), headers: { accept: "application/json" } });
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

export async function fetchHistory(days: HistoryDays, fetchFn: typeof fetch = fetch, now = Date.now()): Promise<{ source: string; points: Point[] }> {
  const errors: string[] = [];
  try {
    const points = parseCoinGecko(await getJson(fetchFn, `https://api.coingecko.com/api/v3/coins/kaspa/market_chart?vs_currency=usd&days=${days}`));
    if (points.length >= 2) return { source: "CoinGecko", points };
    errors.push("CoinGecko: zu wenige Werte");
  } catch (e) {
    errors.push(`CoinGecko: ${(e as Error).message}`);
  }
  try {
    // 7 Tage stündlich, sonst täglich (Kraken liefert höchstens 720 Kerzen)
    const interval = days === 7 ? 60 : 1440;
    const points = parseKraken(await getJson(fetchFn, `https://api.kraken.com/0/public/OHLC?pair=KASUSD&interval=${interval}`), days, now);
    if (points.length >= 2) return { source: "Kraken", points };
    errors.push("Kraken: zu wenige Werte");
  } catch (e) {
    errors.push(`Kraken: ${(e as Error).message}`);
  }
  throw new Error(`Kursverlauf nicht abrufbar (${errors.join("; ")})`);
}

/** Cache je Zeitraum; parallele Anfragen teilen sich einen Abruf, Fehler werden nicht gemerkt */
export function historyCache(fetchFn: typeof fetch = fetch) {
  const store = new Map<HistoryDays, { at: number; body: string }>();
  const inflight = new Map<HistoryDays, Promise<string>>();
  return async (days: HistoryDays): Promise<string> => {
    const hit = store.get(days);
    if (hit && Date.now() - hit.at < CACHE_MS) return hit.body;
    let p = inflight.get(days);
    if (!p) {
      p = fetchHistory(days, fetchFn)
        .then((h) => {
          const body = JSON.stringify({ ok: true, days, ...h });
          store.set(days, { at: Date.now(), body });
          return body;
        })
        .catch((e: Error) => JSON.stringify({ ok: false, error: e.message }))
        .finally(() => inflight.delete(days));
      inflight.set(days, p);
    }
    return p;
  };
}
