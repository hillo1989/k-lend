import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { type NetworkId } from "../config";
import { fetchStatus, StatusError, type ProtocolStatus } from "./status";

const STORE_KEY = "gh-network";
const REFRESH_MS = 30_000;

export interface StatusState {
  network: NetworkId;
  setNetwork(n: NetworkId): void;
  status: ProtocolStatus | null;
  loading: boolean;
  error: string | null;
  /** öffentliche Kaspa-Nodes nicht erreichbar (letzter Abruf oder Status mit Fehler) */
  nodeDown: boolean;
  updatedAt: number | null;
  refresh(): void;
}

const Ctx = createContext<StatusState | null>(null);

export function StatusProvider({ children }: { children: ReactNode }) {
  // Die Seite läuft nur noch im Mainnet (Nutzer, 28.09.2026); die Netzauswahl ist entfernt.
  const [network, setNetworkState] = useState<NetworkId>("mainnet");
  const [status, setStatus] = useState<ProtocolStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nodeDown, setNodeDown] = useState(false);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [tick, setTick] = useState(0);
  const netRef = useRef(network);

  const setNetwork = useCallback((n: NetworkId) => {
    try {
      localStorage.setItem(STORE_KEY, n);
    } catch {
      /* egal */
    }
    netRef.current = n;
    setNetworkState(n);
    setStatus(null); // keine Zahlen des anderen Netzes stehen lassen
    setError(null);
    setNodeDown(false);
    setUpdatedAt(null);
  }, []);

  useEffect(() => {
    const ctl = new AbortController();
    let t: number | undefined;
    setLoading(true);
    fetchStatus(network, ctl.signal)
      .then((s) => {
        if (netRef.current !== network) return;
        setStatus(s);
        const errText = !s.deployed && s.error ? s.error : null;
        setError(errText);
        setNodeDown(!s.deployed && !!s.nodeDown);
        setUpdatedAt(Date.now());
      })
      .catch((e: Error) => {
        if (ctl.signal.aborted) return;
        setError(e.message);
        setNodeDown(e instanceof StatusError && e.nodeDown);
      })
      .finally(() => {
        if (ctl.signal.aborted) return;
        setLoading(false);
        // nächste Abfrage erst nach Abschluss dieser (ghostctl braucht teils 20 s)
        t = window.setTimeout(() => setTick((x) => x + 1), REFRESH_MS);
      });
    return () => {
      ctl.abort();
      clearTimeout(t);
    };
  }, [network, tick]);

  const refresh = useCallback(() => setTick((x) => x + 1), []);

  return (
    <Ctx.Provider value={{ network, setNetwork, status, loading, error, nodeDown, updatedAt, refresh }}>{children}</Ctx.Provider>
  );
}

export function useStatus(): StatusState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useStatus außerhalb von StatusProvider");
  return v;
}
