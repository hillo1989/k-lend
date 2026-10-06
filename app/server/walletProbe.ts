// Browser-Wallet-Probe (docs/wallet-probe.md): streng geprüfte Anfrage →
// Argumente für `ghostctl wallet …`. Reine Logik ohne Ein-/Ausgabe (Vitest).
//
// Die Seite /wallet-probe.html ruft POST /api/wallet-probe mit genau einer
// Operation auf:
//   export  → ghostctl wallet export-unsigned <stufe> --address … (sendet nichts)
//   attach  → ghostctl wallet attach-sigs --plan P --signed S [--send]
//   pay     → ghostctl wallet probe-pay --probe P [--send]   (Sicherheitsnetz)
// Plan, Wallet-Antwort und Probe-Datei kommen als JSON vom Browser und gehen
// als Dateien in ein frisches Temp-Verzeichnis (api.ts legt es an und löscht
// es danach). Gesendet wird nur mit send === true, im Mainnet zusätzlich nur
// mit confirmMainnet === true (gesetzt allein vom Knopf „Jetzt senden“), und
// nie für die Trockenprobe (ghostctl lehnt das selbst noch einmal ab).
import { checkAmount, isNetwork, ValidationError, type Network } from "./actions.ts";

export const PROBE_STAGES = ["dry-cancel", "tresor-open", "tresor-cancel"] as const;
export type ProbeStage = (typeof PROBE_STAGES)[number];

/** Größte Anfrage dieser Route (Plan + Wallet-Antwort, je einige kB) */
export const PROBE_BODY_LIMIT = 128 * 1024;
const MAX_PLAN = 48 * 1024;
const MAX_SIGNED = 48 * 1024;
const MAX_PROBE = 8 * 1024;
/** Startguthaben höchstens (wie wallet::PROBE_MAX_FUND) */
const MAX_FUND_SOMPI = 1_000_000_000n;

// Schnorr-P2PK-Adresse: Präfix, Version „q“, 60 Zeichen bech32
const ADDRESS_RE = /^(kaspa|kaspatest):q[qpzry9x8gf2tvdw0s3jn54khce6mua7l]{60}$/;

export interface ProbeRequest {
  op?: unknown;
  network?: unknown;
  stage?: unknown;
  address?: unknown;
  fund?: unknown;
  dueMinutes?: unknown;
  probe?: unknown;
  plan?: unknown;
  signed?: unknown;
  send?: unknown;
  confirmMainnet?: unknown;
}

export interface ProbeCall {
  /** Argumente; Platzhalter {plan} {signed} {probe} ersetzt api.ts durch Dateipfade */
  args: string[];
  files: Partial<Record<"plan" | "signed" | "probe", string>>;
  sends: boolean;
}

const ALLOWED: Record<string, string[]> = {
  export: ["op", "network", "stage", "address", "fund", "dueMinutes", "probe"],
  attach: ["op", "network", "plan", "signed", "send", "confirmMainnet"],
  pay: ["op", "network", "probe", "send", "confirmMainnet"],
};

export function checkProbeAddress(v: unknown, network: Network): string {
  if (typeof v !== "string") throw new ValidationError("Adresse der Wallet fehlt.");
  const s = v.trim();
  const m = s.match(ADDRESS_RE);
  if (!m) throw new ValidationError("Adresse: Schnorr-Adresse kaspa:q… (bzw. kaspatest:q…) erwartet.");
  const want = network === "mainnet" ? "kaspa" : "kaspatest";
  if (m[1] !== want) throw new ValidationError(`Adresse passt nicht zum Netz ${network}.`);
  return s;
}

function jsonOf(v: unknown, max: number, what: string, kind?: string): string {
  if (!v || typeof v !== "object" || Array.isArray(v)) throw new ValidationError(`${what} fehlt.`);
  if (kind !== undefined && (v as { kind?: unknown }).kind !== kind) throw new ValidationError(`${what}: falsche Art (erwartet ${kind}).`);
  const s = JSON.stringify(v);
  if (s.length > max) throw new ValidationError(`${what} zu groß.`);
  return s;
}

function flag(v: unknown, what: string): boolean {
  if (v === undefined || v === null || v === false) return false;
  if (v === true) return true;
  throw new ValidationError(`${what}: true oder false erwartet.`);
}

export function buildWalletProbeCall(r: ProbeRequest): ProbeCall {
  if (!r || typeof r !== "object") throw new ValidationError("Leere Anfrage.");
  const op = r.op;
  if (typeof op !== "string" || !ALLOWED[op]) throw new ValidationError("op: export, attach oder pay erwartet.");
  for (const k of Object.keys(r)) if (!ALLOWED[op].includes(k)) throw new ValidationError(`Unbekannter Parameter „${k}“.`);
  const network = r.network ?? "mainnet";
  if (!isNetwork(network)) throw new ValidationError("Unbekanntes Netz.");
  const base = ["--network", network, "--json"];

  if (op === "export") {
    const stage = r.stage;
    if (typeof stage !== "string" || !(PROBE_STAGES as readonly string[]).includes(stage))
      throw new ValidationError("Stufe: dry-cancel, tresor-open oder tresor-cancel.");
    const address = checkProbeAddress(r.address, network);
    const args = [...base, "wallet", "export-unsigned", stage, "--address", address];
    const files: ProbeCall["files"] = {};
    if (stage === "tresor-open") {
      if (r.fund !== undefined && r.fund !== null && r.fund !== "") {
        const f = checkAmount(r.fund, "Startguthaben");
        const [w, fr = ""] = f.split(".");
        if (BigInt(w) * 100_000_000n + BigInt((fr + "00000000").slice(0, 8)) > MAX_FUND_SOMPI)
          throw new ValidationError("Startguthaben: höchstens 10 KAS (Probe mit Kleinstbeträgen).");
        args.push("--fund", f);
      }
      if (r.dueMinutes !== undefined && r.dueMinutes !== null && r.dueMinutes !== "") {
        const m = typeof r.dueMinutes === "string" && /^\d{1,5}$/.test(r.dueMinutes) ? Number(r.dueMinutes) : r.dueMinutes;
        if (typeof m !== "number" || !Number.isInteger(m) || m < 10 || m > 10080)
          throw new ValidationError("Termin: 10 bis 10080 Minuten nach jetzt.");
        args.push("--due-minutes", String(m));
      }
    } else if (r.fund !== undefined || r.dueMinutes !== undefined) {
      throw new ValidationError("Startguthaben und Termin gibt es nur beim Anlegen.");
    }
    if (stage === "tresor-cancel") {
      files.probe = jsonOf(r.probe, MAX_PROBE, "Probe-Tresor", "ghost-wallet-probe:1");
      args.push("--probe", "{probe}");
    } else if (r.probe !== undefined) {
      throw new ValidationError("Probe-Tresor nur beim Kündigen.");
    }
    return { args, files, sends: false };
  }

  const send = flag(r.send, "Senden");
  const confirm = flag(r.confirmMainnet, "Mainnet-Bestätigung");
  if (send && network === "mainnet" && !confirm)
    throw new ValidationError("Senden im Mainnet nur nach ausdrücklicher Bestätigung (Knopf „Jetzt senden“).");
  if (!send && confirm) throw new ValidationError("Mainnet-Bestätigung ohne Senden ergibt keinen Sinn.");
  // --ja ersetzt die Rückfrage am Terminal; die Bestätigung kam von der Seite
  const head = send ? [...base, "--ja"] : base;

  if (op === "attach") {
    const files: ProbeCall["files"] = { plan: jsonOf(r.plan, MAX_PLAN, "Signierplan", "ghost-wallet-plan:1") };
    if ((r.plan as { dryOnly?: unknown }).dryOnly === true && send)
      throw new ValidationError("Die Trockenprobe wird nie gesendet.");
    if (typeof r.signed !== "string" || !r.signed.trim()) throw new ValidationError("Antwort der Wallet fehlt.");
    if (r.signed.length > MAX_SIGNED) throw new ValidationError("Antwort der Wallet zu groß.");
    files.signed = r.signed;
    const args = [...head, "wallet", "attach-sigs", "--plan", "{plan}", "--signed", "{signed}"];
    if (send) args.push("--send");
    return { args, files, sends: send };
  }

  // pay
  const files: ProbeCall["files"] = { probe: jsonOf(r.probe, MAX_PROBE, "Probe-Tresor", "ghost-wallet-probe:1") };
  const args = [...head, "wallet", "probe-pay", "--probe", "{probe}"];
  if (send) args.push("--send");
  return { args, files, sends: send };
}
