// Browser-Wallet-Probe: Logik der lokalen Seite /wallet-probe.html (ohne DOM,
// mit Vitest geprüft). Ablauf und Hintergrund: docs/wallet-probe.md.
//
// Diese Seite ist die EINZIGE Stelle, an der die App eine Wallet etwas
// signieren lässt – und nur zur Probe. Aufgerufen werden ausschließlich
//   KasWare: requestAccounts, getNetwork, signPskt
//   Kastle:  connect, getAccount, getNetwork, signTx
// signPskt und signTx geben die signierte Tx zurück und senden NICHT (Belege:
// kasware-wallet/extension src/background/controller/wallet.ts signPskt →
// serializeToSafeJSON; docs.kastle.cc „Sign Transaction“: signTx gibt den
// signierten JSON-Text zurück, signAndBroadcastTx wäre die sendende Variante).
// pushTx, sendKaspa, signAndBroadcastTx sind hier nicht einmal deklariert.
// Gesendet wird nur über ghostctl (lokaler Node), nur nach dem Knopf
// „Jetzt senden“ und nie in Stufe 0.
import { tr } from "../lib/i18n";
import { describeNetwork, type WalletKind } from "../wallet/providers";

export type ProbeNetwork = "mainnet" | "testnet-10";
export type ProbeStage = "dry-cancel" | "tresor-open" | "tresor-cancel";

export interface StageInfo {
  id: ProbeStage;
  title: string;
  what: string;
  cost: string;
  /** darf überhaupt gesendet werden */
  sendable: boolean;
}

export function stages(): StageInfo[] {
  return [
    {
      id: "dry-cancel",
      title: tr("Stufe 0 – Trockenprobe", "Stage 0 – dry run"),
      what: tr(
        "Die Wallet signiert die Kündigung eines ERFUNDENEN Tresors. ghostctl prüft die Signatur lokal gegen den Vertrag. Diese Tx gibt eine UTXO aus, die es nicht gibt – sie wird nie gesendet.",
        "The wallet signs the cancellation of an INVENTED vault. ghostctl checks the signature locally against the contract. It spends a UTXO that does not exist – it is never sent.",
      ),
      cost: tr("Kosten: 0 KAS. Nichts verlässt den Rechner.", "Cost: 0 KAS. Nothing leaves this computer."),
      sendable: false,
    },
    {
      id: "tresor-open",
      title: tr("Stufe 1 – Probe-Tresor anlegen", "Stage 1 – open a test vault"),
      what: tr(
        "Aus deiner Wallet gehen etwa 1,5 KAS in einen Tresor-Vertrag. Absender und Empfänger bist du selbst; eine Zahlung zu 1 KAS, Termin in etwa einer Stunde.",
        "About 1.5 KAS go from your wallet into a vault contract. You are sender and recipient; one payment of 1 KAS, due in about one hour.",
      ),
      cost: tr(
        "Kosten: Netzgebühr ≈ 0,002–0,003 KAS. Die 1,5 KAS bleiben deine und kommen mit Stufe 2 zurück.",
        "Cost: network fee ≈ 0.002–0.003 KAS. The 1.5 KAS stay yours and come back with stage 2.",
      ),
      sendable: true,
    },
    {
      id: "tresor-cancel",
      title: tr("Stufe 2 – Probe-Tresor kündigen", "Stage 2 – cancel the test vault"),
      what: tr(
        "Die Wallet signiert den Covenant-Eingang des Tresors (die eigentliche Probe). Alles abzüglich Gebühr geht an deine Adresse zurück.",
        "The wallet signs the vault's covenant input (the actual test). Everything minus the fee goes back to your address.",
      ),
      cost: tr("Kosten: Netzgebühr ≈ 0,003 KAS.", "Cost: network fee ≈ 0.003 KAS."),
      sendable: true,
    },
  ];
}

// ------------------------------------------------------------- Wallets ----

/** Nur die Methoden, die die Probe braucht (keine sendenden) */
export interface KaswareSignApi {
  requestAccounts(): Promise<string[]>;
  getNetwork(): Promise<string>;
  signPskt(p: { txJsonString: string; options?: { signInputs: { index: number; sighashType: number }[] } }): Promise<unknown>;
}
export interface KastleSignApi {
  connect(): Promise<boolean>;
  getAccount(): Promise<{ address: string; publicKey?: string } | null>;
  getNetwork(): Promise<string>;
  signTx(networkId: string, txJson: string, scripts?: { inputIndex: number; scriptHex: string; signType: string }[]): Promise<unknown>;
}

export interface ExportResult {
  ok: true;
  stage: ProbeStage;
  network: ProbeNetwork;
  address: string;
  dryOnly: boolean;
  feeSompi: number;
  outputs: { index: number; sompi: number; kas: number; address: string; what: string }[];
  signInputs: { index: number; kind: string; entry?: string | null; argPos: number; redeemHex?: string | null }[];
  kastle: { networkId: string; txJson: string; scripts: { inputIndex: number; scriptHex: string; signType: string }[] };
  kasware: { txJsonString: string; options: { signInputs: { index: number; sighashType: number }[] } };
  probe: ProbeFile;
  plan: { kind: string; dryOnly: boolean; [k: string]: unknown };
}

export interface ProbeFile {
  kind: "ghost-wallet-probe:1";
  network: ProbeNetwork;
  outpoint: string;
  value: number;
  fictional?: boolean;
  state: { nextDue: number; left: number };
  [k: string]: unknown;
}

export interface ProbeSigner {
  kind: WalletKind;
  /** Freigabe-Dialog; liefert die Adresse */
  connect(): Promise<string>;
  network(): Promise<string>;
  /** signierte Tx als JSON-Text; sendet nicht */
  sign(e: ExportResult): Promise<string>;
}

/** Rückgabe der Wallet als JSON-Text (beide liefern laut Doku einen String) */
export function signedToString(x: unknown): string {
  if (typeof x === "string" && x.trim()) return x;
  if (x && typeof x === "object") return JSON.stringify(x);
  throw new Error(tr("Die Wallet hat keine signierte Transaktion zurückgegeben.", "The wallet returned no signed transaction."));
}

export function kaswareSigner(p: KaswareSignApi): ProbeSigner {
  return {
    kind: "kasware",
    async connect() {
      const a = await p.requestAccounts();
      if (!a?.[0]) throw new Error(tr("KasWare: kein Konto freigegeben.", "KasWare: no account shared."));
      return a[0];
    },
    network: () => p.getNetwork(),
    async sign(e) {
      return signedToString(await p.signPskt({ txJsonString: e.kasware.txJsonString, options: e.kasware.options }));
    },
  };
}

export function kastleSigner(p: KastleSignApi): ProbeSigner {
  return {
    kind: "kastle",
    async connect() {
      if (!(await p.connect())) throw new Error(tr("Kastle: Verbindung abgelehnt.", "Kastle: connection rejected."));
      const a = await p.getAccount();
      if (!a?.address) throw new Error(tr("Kastle: kein Konto.", "Kastle: no account."));
      return a.address;
    },
    network: () => p.getNetwork(),
    async sign(e) {
      return signedToString(await p.signTx(e.kastle.networkId, e.kastle.txJson, e.kastle.scripts));
    },
  };
}

export function probeSigner(kind: WalletKind, win: { kasware?: unknown; kastle?: unknown }): ProbeSigner | null {
  if (kind === "kasware" && win.kasware) return kaswareSigner(win.kasware as KaswareSignApi);
  if (kind === "kastle" && win.kastle) return kastleSigner(win.kastle as KastleSignApi);
  return null;
}

/** Netz der Wallet passt zum gewählten? null = ja, sonst Meldung */
export function networkProblem(raw: string, network: ProbeNetwork): string | null {
  const n = describeNetwork(raw);
  if (n.id === network) return null;
  return tr(`Die Wallet ist im Netz „${n.label}“, gewählt ist ${network}. Bitte in der Wallet umstellen.`, `The wallet is on "${n.label}", selected is ${network}. Please switch in the wallet.`);
}

export function addressProblem(address: string, network: ProbeNetwork): string | null {
  const want = network === "mainnet" ? "kaspa:q" : "kaspatest:q";
  if (address.startsWith(want)) return null;
  return tr(
    `Die Adresse ${address} ist keine Schnorr-Adresse (${want}…) dieses Netzes. Die Probe braucht ein Schnorr-Konto.`,
    `Address ${address} is not a Schnorr address (${want}…) of this network. The test needs a Schnorr account.`,
  );
}

// ------------------------------------------------------------- Server ----

export type Fetch = (url: string, init: { method: string; headers: Record<string, string>; body: string }) => Promise<{ json(): Promise<unknown> }>;

export async function callProbeApi(fetchFn: Fetch, body: Record<string, unknown>): Promise<Record<string, unknown>> {
  const r = await fetchFn("/api/wallet-probe", {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Ghost-Client": "1" },
    body: JSON.stringify(body),
  });
  const j = (await r.json()) as Record<string, unknown>;
  if (!j || typeof j !== "object") throw new Error(tr("Ungültige Antwort des Servers.", "Invalid server response."));
  return j;
}

export function exportBody(stage: ProbeStage, network: ProbeNetwork, address: string, o: { fund?: string; dueMinutes?: number; probe?: ProbeFile | null } = {}) {
  const b: Record<string, unknown> = { op: "export", network, stage, address };
  if (stage === "tresor-open") {
    if (o.fund) b.fund = o.fund;
    if (o.dueMinutes) b.dueMinutes = o.dueMinutes;
  }
  if (stage === "tresor-cancel") {
    if (!o.probe) throw new Error(tr("Für Stufe 2 fehlt der Probe-Tresor aus Stufe 1.", "Stage 2 needs the test vault from stage 1."));
    b.probe = o.probe;
  }
  return b;
}

/** Prüfen (send=false) oder senden (send=true, nur vom Knopf „Jetzt senden“) */
export function attachBody(network: ProbeNetwork, plan: ExportResult["plan"], signed: string, send: boolean) {
  if (send && plan.dryOnly) throw new Error(tr("Die Trockenprobe wird nie gesendet.", "The dry run is never sent."));
  const b: Record<string, unknown> = { op: "attach", network, plan, signed };
  if (send) {
    b.send = true;
    if (network === "mainnet") b.confirmMainnet = true;
  }
  return b;
}

export function payBody(network: ProbeNetwork, probe: ProbeFile, send: boolean) {
  const b: Record<string, unknown> = { op: "pay", network, probe };
  if (send) {
    b.send = true;
    if (network === "mainnet") b.confirmMainnet = true;
  }
  return b;
}

export interface AttachResult {
  ok: boolean;
  valid?: boolean;
  stage?: ProbeStage;
  dryOnly?: boolean;
  sent?: boolean;
  confirmed?: boolean;
  txid?: string;
  feeSompi?: number;
  probe?: ProbeFile;
  error?: string;
  report?: {
    valid: boolean;
    changed: string[];
    ignored: string[];
    inputs: { index: number; kind: string; signed: boolean; scriptLen: number; hashType: number | null; sigValid: boolean; note: string }[];
    plannedUnits: number[];
    usedUnits: number[];
    budgets: number[];
    budgetsRaised: boolean;
    fee: number;
    minFee: number;
    error: string | null;
  };
}

/** Darf „Jetzt senden“ erscheinen? */
export function canSend(stage: ProbeStage | null, r: AttachResult | null): boolean {
  return !!stage && stage !== "dry-cancel" && !!r && r.ok === true && r.valid === true && r.dryOnly === false && r.sent !== true;
}

export function kasText(sompi: number): string {
  return `${(sompi / 1e8).toLocaleString("de-DE", { minimumFractionDigits: 2, maximumFractionDigits: 8 })} KAS`;
}

/** Bericht von attach-sigs in Klartext */
export function reportLines(r: AttachResult): string[] {
  const out: string[] = [];
  if (!r.ok) return [tr(`Fehler: ${r.error ?? "unbekannt"}`, `Error: ${r.error ?? "unknown"}`)];
  out.push(r.valid ? tr("Signatur GÜLTIG – die Tx besteht die lokale Skriptprüfung.", "Signature VALID – the tx passes the local script check.") : tr("Signatur UNGÜLTIG.", "Signature INVALID."));
  const rep = r.report;
  if (!rep) return out;
  for (const i of rep.inputs) {
    const ht = i.hashType === null ? "–" : `0x${i.hashType.toString(16).padStart(2, "0")}`;
    out.push(tr(`Eingang ${i.index} (${i.kind}): ${i.note} [Skript ${i.scriptLen} Byte, Hashtype ${ht}]`, `Input ${i.index} (${i.kind}): ${i.note} [script ${i.scriptLen} bytes, hashtype ${ht}]`));
  }
  if (rep.changed.length) out.push(tr(`Von der Wallet verändert: ${rep.changed.join(", ")}`, `Changed by the wallet: ${rep.changed.join(", ")}`));
  if (rep.ignored.length) out.push(tr(`Verändert, aber nicht signiert (überschrieben): ${rep.ignored.join(", ")}`, `Changed but not signed (overwritten): ${rep.ignored.join(", ")}`));
  if (rep.usedUnits.length)
    out.push(
      tr(
        `Skript-Einheiten geplant ${rep.plannedUnits.join("/")}, gemessen ${rep.usedUnits.join("/")}${rep.budgetsRaised ? " – Budget erhöht" : ""}`,
        `Script units planned ${rep.plannedUnits.join("/")}, measured ${rep.usedUnits.join("/")}${rep.budgetsRaised ? " – budget raised" : ""}`,
      ),
    );
  if (rep.fee) out.push(tr(`Gebühr ${kasText(rep.fee)} (Mindestgebühr ${kasText(rep.minFee)})`, `Fee ${kasText(rep.fee)} (minimum ${kasText(rep.minFee)})`));
  if (rep.error) out.push(tr(`Grund: ${rep.error}`, `Reason: ${rep.error}`));
  return out;
}

// --------------------------------------------------- Probe-Tresor merken ----

const probeKey = (network: ProbeNetwork, address: string) => `gh-wallet-probe:${network}:${address}`;

export interface KV {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
}

export function saveProbe(s: KV, network: ProbeNetwork, address: string, p: ProbeFile) {
  s.setItem(probeKey(network, address), JSON.stringify(p));
}

export function loadProbe(s: KV, network: ProbeNetwork, address: string): ProbeFile | null {
  try {
    const v = s.getItem(probeKey(network, address));
    return v ? parseProbe(v, network) : null;
  } catch {
    return null;
  }
}

export function forgetProbe(s: KV, network: ProbeNetwork, address: string) {
  s.removeItem(probeKey(network, address));
}

/** Probe-Datei (eingefügt oder geladen) grob prüfen; die genaue Prüfung macht ghostctl */
export function parseProbe(text: string, network: ProbeNetwork): ProbeFile {
  let p: unknown;
  try {
    p = JSON.parse(text);
  } catch {
    throw new Error(tr("Probe-Datei ist kein JSON.", "Probe file is not JSON."));
  }
  const f = p as ProbeFile;
  if (!f || f.kind !== "ghost-wallet-probe:1") throw new Error(tr("Keine Probe-Datei (ghost-wallet-probe:1).", "Not a probe file (ghost-wallet-probe:1)."));
  if (f.network !== network) throw new Error(tr(`Probe-Datei gehört zu ${f.network}.`, `Probe file belongs to ${f.network}.`));
  if (f.fictional) throw new Error(tr("Das ist der erfundene Tresor der Trockenprobe.", "This is the invented vault of the dry run."));
  return f;
}
