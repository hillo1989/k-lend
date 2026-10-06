import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import type { NetworkId } from "../config";
import { fetchKeys, type CommitteeEntry, type KeyEntry, type KeyListEntry } from "./api";
import { useStatus } from "./StatusContext";

// Schlüsseldateien (keys/*.json) des lokalen ghostctl – nur öffentliche Daten.
// Die Auswahl merkt sich die Seite je Netz im localStorage.

const storeKey = (n: NetworkId) => `gh-key-${n}`;
const NET_PREFIX: Record<NetworkId, string> = { mainnet: "keys/mainnet-", "testnet-10": "keys/tn10-" };

function recall(n: NetworkId): string | null {
  try {
    return localStorage.getItem(storeKey(n));
  } catch {
    return null;
  }
}
function remember(n: NetworkId, file: string) {
  try {
    localStorage.setItem(storeKey(n), file);
  } catch {
    /* egal */
  }
}

export interface AccountState {
  keys: KeyEntry[];
  committees: CommitteeEntry[];
  selected: KeyEntry | null;
  select(file: string): void;
  loading: boolean;
  error: string | null;
  refresh(): void;
  /** Schlüssel, deren Name zum Netz passt, zuerst */
  isNetworkKey(file: string): boolean;
}

const Ctx = createContext<AccountState | null>(null);

export function AccountProvider({ children }: { children: ReactNode }) {
  const { network, updatedAt } = useStatus();
  const [list, setList] = useState<KeyListEntry[]>([]);
  const [loadedFor, setLoadedFor] = useState<NetworkId | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const [selFile, setSelFile] = useState<string | null>(() => recall(network));

  useEffect(() => setSelFile(recall(network)), [network]);

  // Mit jedem Status-Abruf (alle 30 s, „Neu laden“, nach Aktionen) auch die
  // Guthaben neu holen – sonst blieben Sendungen aus dem Terminal oder vom
  // Agenten unsichtbar, bis man „Guthaben neu laden“ drückt. Still, ohne
  // Ladeanzeige, wenn für dieses Netz schon Daten da sind.
  const [statusTick, setStatusTick] = useState(0);
  useEffect(() => {
    if (updatedAt !== null) setStatusTick((x) => x + 1);
  }, [updatedAt]);
  const shownFor = useRef<NetworkId | null>(null);

  useEffect(() => {
    const ctl = new AbortController();
    if (shownFor.current !== network) setLoading(true);
    fetchKeys(network, ctl.signal)
      .then((l) => {
        setList(l);
        setLoadedFor(network);
        shownFor.current = network;
        setError(null);
      })
      .catch((e: Error) => {
        if (!ctl.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!ctl.signal.aborted) setLoading(false);
      });
    return () => ctl.abort();
  }, [network, tick, statusTick]);

  const isNetworkKey = useCallback((f: string) => f.startsWith(NET_PREFIX[network]), [network]);

  const value = useMemo<AccountState>(() => {
    const current = loadedFor === network ? list : [];
    const keys = current
      .filter((k): k is KeyEntry => k.type === "key")
      .sort((a, b) => Number(isNetworkKey(b.file)) - Number(isNetworkKey(a.file)) || a.file.localeCompare(b.file));
    const committees = current
      .filter((k): k is CommitteeEntry => k.type === "committee")
      .sort((a, b) => Number(isNetworkKey(b.file)) - Number(isNetworkKey(a.file)) || a.file.localeCompare(b.file));
    const selected = keys.find((k) => k.file === selFile) ?? keys.find((k) => isNetworkKey(k.file)) ?? keys[0] ?? null;
    return {
      keys,
      committees,
      selected,
      select(file: string) {
        remember(network, file);
        setSelFile(file);
      },
      loading,
      error,
      refresh: () => {
        shownFor.current = null; // von Hand: mit Ladeanzeige
        setTick((x) => x + 1);
      },
      isNetworkKey,
    };
  }, [list, loadedFor, network, selFile, loading, error, isNetworkKey]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAccount(): AccountState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useAccount außerhalb von AccountProvider");
  return v;
}
