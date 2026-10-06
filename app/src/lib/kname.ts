// .k-Namen (dotk.name) im Empfängerfeld: der Server löst sie auf und prüft sie
// am eigenen Node (server/dotkNames.ts). Die Seite zahlt nur an eine bewiesene
// Adresse und fragt alle 60 s neu, damit nie an eine veraltete Adresse gezahlt wird.
import { useEffect, useState } from "react";
import type { NetworkId } from "../config";

/** kürzeste Eingabe, die nachgeschlagen wird (A20d-10) */
export const KNAME_MIN_CHARS = 3;
/** Wartezeit nach dem letzten Tastendruck (A20d-10; vorher 400 ms) */
export const KNAME_DEBOUNCE_MS = 700;

/**
 * Eingabe wie ein .k-Name (keine Adresse, kein x-only-Schlüssel)? Wie
 * looksLikeName im Server. Audit 20 A20d-10: erst ab 3 Zeichen und nicht,
 * solange die Eingabe der Anfang einer Adresse sein kann („k“, „kas“,
 * „kaspa“, „kaspatest“ …) – sonst ginge jedes Tippfragment einer Adresse an
 * den Server und an dotk.name.
 */
export function isKName(s: string): boolean {
  const t = s.trim().toLowerCase();
  if (t.length < KNAME_MIN_CHARS || t.length > 80 || t.includes(":")) return false;
  if ("kaspa:".startsWith(t) || "kaspatest:".startsWith(t)) return false;
  if (/^[0-9a-f]{64}$/.test(t)) return false;
  return /^[a-z0-9][a-z0-9.\-_]*$/.test(t);
}

export interface KName {
  /** Eingabe, zu der die Antwort gehört */
  input: string;
  loading: boolean;
  display?: string;
  address?: string;
  error?: string;
}

const REFRESH_MS = 60_000;

export function useKName(input: string, network: NetworkId): KName | null {
  const t = input.trim();
  const active = isKName(t);
  const [state, setState] = useState<KName | null>(null);
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!active) {
      setState(null);
      return;
    }
    const ctl = new AbortController();
    setState((s) => (s && s.input === t ? { ...s, loading: true } : { input: t, loading: true }));
    const timer = window.setTimeout(() => {
      fetch(`./api/wallet/name?network=${encodeURIComponent(network)}&name=${encodeURIComponent(t)}`, { signal: ctl.signal, cache: "no-store" })
        .then((r) => r.json() as Promise<{ ok?: boolean; display?: string; address?: string; error?: string }>)
        .then((j) =>
          setState(
            j.ok && j.address
              ? { input: t, loading: false, display: j.display, address: j.address }
              : { input: t, loading: false, display: j.display, error: j.error ?? "Name nicht auflösbar." },
          ),
        )
        .catch((e: Error) => {
          if (!ctl.signal.aborted) setState({ input: t, loading: false, error: e.message });
        });
    }, KNAME_DEBOUNCE_MS);
    const again = window.setTimeout(() => setTick((x) => x + 1), REFRESH_MS);
    return () => {
      ctl.abort();
      clearTimeout(timer);
      clearTimeout(again);
    };
  }, [t, active, network, tick]);

  return active ? (state && state.input === t ? state : { input: t, loading: true }) : null;
}
