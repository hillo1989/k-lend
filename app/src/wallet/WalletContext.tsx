import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import {
  describeNetwork,
  describeWalletError,
  getAdapter,
  installedWallets,
  type NetworkInfo,
  type WalletAdapter,
  type WalletKind,
} from "./providers";
import { tr } from "../lib/i18n";

const STORE_KEY = "kl-demo-wallet"; // merkt nur, WELCHE Wallet zuletzt verbunden war

export interface WalletState {
  installed: WalletKind[];
  status: "idle" | "connecting" | "connected";
  kind: WalletKind | null;
  address: string | null;
  network: NetworkInfo | null;
  balance: bigint | null;
  publicKey: string | null;
  error: string | null;
  connect(kind: WalletKind): Promise<void>;
  disconnect(): Promise<void>;
  refresh(): Promise<void>;
  clearError(): void;
  /** Wallets erneut suchen (Erweiterungen melden sich teils spät an) */
  rescan(): void;
}

const Ctx = createContext<WalletState | null>(null);

function remember(kind: WalletKind | null) {
  try {
    if (kind) localStorage.setItem(STORE_KEY, kind);
    else localStorage.removeItem(STORE_KEY);
  } catch {
    /* Speicher gesperrt – dann eben ohne Wiederherstellung */
  }
}
function recall(): WalletKind | null {
  try {
    const v = localStorage.getItem(STORE_KEY);
    return v === "kasware" || v === "kastle" ? v : null;
  } catch {
    return null;
  }
}

export function WalletProvider({ children }: { children: ReactNode }) {
  const [installed, setInstalled] = useState<WalletKind[]>([]);
  const [status, setStatus] = useState<WalletState["status"]>("idle");
  const [kind, setKind] = useState<WalletKind | null>(null);
  const [address, setAddress] = useState<string | null>(null);
  const [network, setNetwork] = useState<NetworkInfo | null>(null);
  const [balance, setBalance] = useState<bigint | null>(null);
  const [publicKey, setPublicKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const adapterRef = useRef<WalletAdapter | null>(null);

  const reset = useCallback(() => {
    adapterRef.current = null;
    setStatus("idle");
    setKind(null);
    setAddress(null);
    setNetwork(null);
    setBalance(null);
    setPublicKey(null);
  }, []);

  const loadDetails = useCallback(async (a: WalletAdapter) => {
    try {
      setNetwork(describeNetwork(await a.network()));
    } catch {
      setNetwork(null);
    }
    try {
      setBalance(await a.balanceSompi());
    } catch {
      setBalance(null);
    }
    try {
      setPublicKey(await a.publicKey());
    } catch {
      setPublicKey(null);
    }
  }, []);

  // Erweiterungen injizieren ihr Objekt teils erst nach dem Laden der Seite:
  // bis etwa 5 s weitersuchen (Wallet-Probe 05.10.2026, KasWare in Brave),
  // danach bietet die Seite „Erneut suchen“ an (rescan)
  useEffect(() => {
    const check = () => setInstalled(installedWallets());
    check();
    const timers = [300, 1000, 2000, 3000, 4000, 5000].map((ms) => window.setTimeout(check, ms));
    window.addEventListener("load", check);
    return () => {
      timers.forEach(clearTimeout);
      window.removeEventListener("load", check);
    };
  }, []);

  // Wiederherstellen ohne Dialog, aber nur wenn der Nutzer früher selbst verbunden hat.
  const restoredRef = useRef(false);
  useEffect(() => {
    if (restoredRef.current || status !== "idle") return;
    const last = recall();
    if (!last || !installed.includes(last)) return;
    restoredRef.current = true;
    const a = getAdapter(last);
    if (!a) return;
    a.currentAccount()
      .then(async (addr) => {
        if (!addr) return remember(null);
        adapterRef.current = a;
        setKind(last);
        setAddress(addr);
        setStatus("connected");
        await loadDetails(a);
      })
      .catch(() => remember(null));
  }, [installed, status, loadDetails]);

  // Ereignisse der verbundenen Wallet
  useEffect(() => {
    const a = adapterRef.current;
    if (!a || status !== "connected") return;
    return a.subscribe({
      accounts(list) {
        if (list.length === 0) {
          remember(null);
          reset();
          setError(tr("Die Wallet hat die Verbindung getrennt.", "The wallet disconnected."));
        } else {
          setAddress(list[0]);
          void loadDetails(a);
        }
      },
      network(n) {
        setNetwork(describeNetwork(n));
        a.balanceSompi().then(setBalance, () => setBalance(null));
      },
      balance() {
        a.balanceSompi().then(setBalance, () => setBalance(null));
      },
    });
  }, [status, kind, loadDetails, reset]);

  const connect = useCallback(
    async (k: WalletKind) => {
      const a = getAdapter(k);
      if (!a) {
        setError(tr("Diese Wallet wurde im Browser nicht gefunden.", "This wallet was not found in the browser."));
        return;
      }
      setError(null);
      setStatus("connecting");
      try {
        const addr = await a.connect();
        if (!addr) throw new Error(tr("Die Wallet hat keine Adresse geliefert.", "The wallet did not return an address."));
        adapterRef.current = a;
        setKind(k);
        setAddress(addr);
        setStatus("connected");
        remember(k);
        await loadDetails(a);
      } catch (e) {
        reset();
        setError(describeWalletError(e));
      }
    },
    [loadDetails, reset],
  );

  const disconnect = useCallback(async () => {
    const a = adapterRef.current;
    remember(null);
    reset();
    try {
      await a?.disconnect();
    } catch {
      /* lokal ist die Verbindung ohnehin vergessen */
    }
  }, [reset]);

  const refresh = useCallback(async () => {
    if (adapterRef.current) await loadDetails(adapterRef.current);
  }, [loadDetails]);

  const value: WalletState = {
    installed,
    status,
    kind,
    address,
    network,
    balance,
    publicKey,
    error,
    connect,
    disconnect,
    refresh,
    clearError: () => setError(null),
    rescan: () => setInstalled(installedWallets()),
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useWallet(): WalletState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useWallet außerhalb von WalletProvider");
  return v;
}
