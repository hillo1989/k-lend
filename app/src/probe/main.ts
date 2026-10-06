// Oberfläche der lokalen Seite /wallet-probe.html (Logik in probe.ts).
// Läuft nur über den lokalen Vite-Server (localhost), der ghostctl aufruft.
import { tr } from "../lib/i18n";
import { installedWallets, WALLET_NAMES, type WalletKind } from "../wallet/providers";
import {
  addressProblem,
  attachBody,
  callProbeApi,
  canSend,
  exportBody,
  forgetProbe,
  kasText,
  loadProbe,
  networkProblem,
  parseProbe,
  payBody,
  probeSigner,
  reportLines,
  saveProbe,
  stages,
  type AttachResult,
  type ExportResult,
  type ProbeFile,
  type ProbeNetwork,
  type ProbeSigner,
  type ProbeStage,
} from "./probe";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const fetchFn = (u: string, i: RequestInit) => fetch(u, i);
const store = (() => {
  try {
    return window.localStorage;
  } catch {
    const m = new Map<string, string>();
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), removeItem: (k: string) => void m.delete(k) };
  }
})();

const st: {
  network: ProbeNetwork;
  signer: ProbeSigner | null;
  address: string | null;
  stage: ProbeStage;
  exp: ExportResult | null;
  signed: string | null;
  result: AttachResult | null;
  pay: Record<string, unknown> | null;
} = { network: "mainnet", signer: null, address: null, stage: "dry-cancel", exp: null, signed: null, result: null, pay: null };

function el(tag: string, attrs: Record<string, string> = {}, text?: string): HTMLElement {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text !== undefined) e.textContent = text;
  return e;
}

function log(lines: string[] | string, kind: "info" | "ok" | "bad" = "info") {
  const box = $("log");
  const p = el("div", { class: `line ${kind}` });
  p.textContent = (Array.isArray(lines) ? lines : [lines]).join("\n");
  box.prepend(p);
}

function busy(on: boolean) {
  document.querySelectorAll<HTMLButtonElement>("button").forEach((b) => (b.disabled = on || b.dataset.off === "1"));
}

async function guarded(f: () => Promise<void>) {
  busy(true);
  try {
    await f();
  } catch (e) {
    log(tr(`Fehler: ${(e as Error).message ?? String(e)}`, `Error: ${(e as Error).message ?? String(e)}`), "bad");
  } finally {
    busy(false);
    render();
  }
}

function resetFlow() {
  st.exp = null;
  st.signed = null;
  st.result = null;
}

function probeOfAccount(): ProbeFile | null {
  return st.address ? loadProbe(store, st.network, st.address) : null;
}

function render() {
  $("acct").textContent = st.address
    ? tr(`Verbunden: ${WALLET_NAMES[st.signer!.kind]} – ${st.address}`, `Connected: ${WALLET_NAMES[st.signer!.kind]} – ${st.address}`)
    : tr("Keine Wallet verbunden.", "No wallet connected.");
  const info = stages().find((s) => s.id === st.stage)!;
  $("stage-what").textContent = `${info.what} ${info.cost}`;
  $("open-opts").hidden = st.stage !== "tresor-open";
  $("cancel-opts").hidden = st.stage !== "tresor-cancel";
  const pr = probeOfAccount();
  $("probe-show").textContent = pr
    ? tr(
        `Gemerkter Probe-Tresor: ${pr.outpoint}, ${kasText(pr.value)}, Termin ${new Date(pr.state.nextDue).toLocaleString("de-DE")}, Zahlungen offen: ${pr.state.left}`,
        `Remembered test vault: ${pr.outpoint}, ${kasText(pr.value)}, due ${new Date(pr.state.nextDue).toLocaleString("en-US")}, payments left: ${pr.state.left}`,
      )
    : tr("Kein Probe-Tresor gemerkt (Stufe 1 zuerst, oder Datei unten einfügen).", "No test vault remembered (stage 1 first, or paste the file below).");
  const set = (id: string, off: boolean) => {
    const b = $<HTMLButtonElement>(id);
    b.dataset.off = off ? "1" : "0";
    b.disabled = off;
  };
  set("btn-export", !st.address);
  set("btn-sign", !st.exp);
  set("btn-send", !canSend(st.stage, st.result));
  $("btn-send").hidden = !canSend(st.stage, st.result);
  set("btn-pay", !pr);
  set("btn-pay-send", !(st.pay && st.pay.ok === true && st.pay.sent !== true));
  set("btn-dl", !pr);
  const s = $("summary");
  s.textContent = "";
  if (st.exp) {
    s.append(el("div", {}, tr(`Tx (${st.exp.stage}) geholt: Gebühr ${kasText(st.exp.feeSompi)}, ${st.exp.signInputs.length} Eingang/Eingänge zu signieren.`, `Tx (${st.exp.stage}) fetched: fee ${kasText(st.exp.feeSompi)}, ${st.exp.signInputs.length} input(s) to sign.`)));
    for (const o of st.exp.outputs) s.append(el("div", {}, `• ${kasText(o.sompi)} → ${o.what} (${o.address})`));
    if (st.exp.dryOnly) s.append(el("div", { class: "warn" }, tr("Trockenprobe: wird nie gesendet.", "Dry run: never sent.")));
  }
  const v = $("verdict");
  v.className = "verdict";
  v.textContent = "";
  if (st.result) {
    v.classList.add(st.result.ok && st.result.valid ? "ok" : "bad");
    v.textContent = reportLines(st.result).join("\n");
  }
}

function setup() {
  $("title").textContent = tr("Wallet-Probe (nur lokal)", "Wallet test (local only)");
  $("intro").textContent = tr(
    "Prüft, ob KasWare oder Kastle Transaktionen mit den GHOST-Covenants signieren können. Die Wallet signiert nur (signPskt bzw. signTx) – gesendet wird ausschließlich, wenn du unten „Jetzt senden“ drückst, und nie in Stufe 0. Jede Signatur bestätigst du selbst in der Wallet. Was du signierst, zeigt die Wallet nur als Ein- und Ausgänge an: vergleiche die Beträge mit der Übersicht hier.",
    "Checks whether KasWare or Kastle can sign transactions with the GHOST covenants. The wallet only signs (signPskt or signTx) – sending happens only when you press “Send now” below, and never in stage 0. You confirm every signature yourself in the wallet. The wallet shows only inputs and outputs: compare the amounts with the overview here.",
  );
  const net = $<HTMLSelectElement>("network");
  net.onchange = () => {
    st.network = net.value as ProbeNetwork;
    st.signer = null;
    st.address = null;
    resetFlow();
    render();
  };
  // Erweiterungen melden sich teils erst nach dem Laden der Seite an:
  // einige Sekunden weitersuchen, danach „Erneut suchen“ anbieten
  const wl = $("wallets");
  const showWallets = (final: boolean) => {
    const found = installedWallets();
    wl.replaceChildren();
    if (!found.length) {
      if (!final) {
        wl.append(el("div", { class: "muted" }, tr("Suche Wallet …", "Looking for wallet …")));
        return false;
      }
      wl.append(el("div", { class: "warn" }, tr("Keine Wallet gefunden (KasWare oder Kastle als Browser-Erweiterung). Ist die Erweiterung installiert, entsperrt und für diese Seite erlaubt?", "No wallet found (KasWare or Kastle browser extension). Is the extension installed, unlocked and allowed on this site?")));
      const again = el("button", {}, tr("Erneut suchen", "Search again")) as HTMLButtonElement;
      again.onclick = () => showWallets(true);
      wl.append(again);
      return false;
    }
    for (const k of found) {
      const b = el("button", {}, tr(`Mit ${WALLET_NAMES[k]} verbinden`, `Connect ${WALLET_NAMES[k]}`)) as HTMLButtonElement;
      b.onclick = () => guarded(() => connect(k));
      wl.append(b);
    }
    return true;
  };
  if (!showWallets(false)) {
    let tries = 0;
    const timer = window.setInterval(() => {
      tries += 1;
      if (showWallets(tries >= 10)) window.clearInterval(timer);
      else if (tries >= 10) window.clearInterval(timer);
    }, 500);
  }
  const list = $("stages");
  for (const s of stages()) {
    const id = `stage-${s.id}`;
    const r = el("input", { type: "radio", name: "stage", id, value: s.id }) as HTMLInputElement;
    r.checked = s.id === st.stage;
    r.onchange = () => {
      st.stage = s.id;
      resetFlow();
      render();
    };
    const l = el("label", { for: id });
    l.append(r, document.createTextNode(` ${s.title}`));
    list.append(el("div"), l);
  }
  $("btn-export").onclick = () => guarded(fetchTx);
  $("btn-sign").onclick = () => guarded(signAndCheck);
  $("btn-send").onclick = () => guarded(sendNow);
  $("btn-pay").onclick = () => guarded(() => pay(false));
  $("btn-pay-send").onclick = () => guarded(() => pay(true));
  $("btn-dl").onclick = () => {
    const pr = probeOfAccount();
    if (!pr) return;
    const a = el("a", { href: URL.createObjectURL(new Blob([JSON.stringify(pr, null, 2)], { type: "application/json" })), download: "probe-tresor.json" });
    a.click();
  };
  $("btn-probe-paste").onclick = () =>
    guarded(async () => {
      if (!st.address) throw new Error(tr("Erst die Wallet verbinden.", "Connect the wallet first."));
      const p = parseProbe($<HTMLTextAreaElement>("probe-text").value, st.network);
      saveProbe(store, st.network, st.address, p);
      log(tr("Probe-Tresor übernommen.", "Test vault taken over."), "ok");
    });
  render();
}

async function connect(k: WalletKind) {
  const s = probeSigner(k, window as unknown as { kasware?: unknown; kastle?: unknown });
  if (!s) throw new Error(tr("Wallet nicht gefunden.", "Wallet not found."));
  const address = await s.connect();
  const np = networkProblem(await s.network(), st.network);
  if (np) throw new Error(np);
  const ap = addressProblem(address, st.network);
  if (ap) throw new Error(ap);
  st.signer = s;
  st.address = address;
  resetFlow();
  log(tr(`${WALLET_NAMES[k]} verbunden: ${address}`, `${WALLET_NAMES[k]} connected: ${address}`), "ok");
}

async function fetchTx() {
  if (!st.address) return;
  resetFlow();
  const body = exportBody(st.stage, st.network, st.address, {
    fund: $<HTMLInputElement>("fund").value.trim() || undefined,
    dueMinutes: Number($<HTMLInputElement>("due").value) || undefined,
    probe: st.stage === "tresor-cancel" ? probeOfAccount() : null,
  });
  const j = await callProbeApi(fetchFn, body);
  if (j.ok !== true) throw new Error(String(j.error ?? "ghostctl"));
  st.exp = j as unknown as ExportResult;
  log(tr(`Unsignierte Tx geholt (${st.exp.stage}), Gebühr ${kasText(st.exp.feeSompi)}.`, `Unsigned tx fetched (${st.exp.stage}), fee ${kasText(st.exp.feeSompi)}.`));
}

async function signAndCheck() {
  if (!st.exp || !st.signer) return;
  const np = networkProblem(await st.signer.network(), st.network);
  if (np) throw new Error(np);
  log(tr(`Bitte in ${WALLET_NAMES[st.signer.kind]} prüfen und signieren …`, `Please review and sign in ${WALLET_NAMES[st.signer.kind]} …`));
  st.signed = await st.signer.sign(st.exp);
  const r = (await callProbeApi(fetchFn, attachBody(st.network, st.exp.plan, st.signed, false))) as unknown as AttachResult;
  st.result = r;
  log(reportLines(r), r.ok && r.valid ? "ok" : "bad");
}

async function sendNow() {
  if (!st.exp || !st.signed || !st.address || !canSend(st.stage, st.result)) return;
  const what = st.stage === "tresor-open" ? tr("Probe-Tresor anlegen", "open the test vault") : tr("Probe-Tresor kündigen", "cancel the test vault");
  if (!window.confirm(tr(`${st.network.toUpperCase()}: jetzt wirklich senden (${what})?`, `${st.network.toUpperCase()}: really send now (${what})?`))) return;
  const r = (await callProbeApi(fetchFn, attachBody(st.network, st.exp.plan, st.signed, true))) as unknown as AttachResult;
  if (r.ok !== true) {
    st.result = { ...(st.result as AttachResult), sent: (r as { unclear?: boolean }).unclear ? true : st.result?.sent };
    throw new Error(String(r.error ?? "Senden fehlgeschlagen"));
  }
  st.result = r;
  log(tr(`Gesendet: ${r.txid}${r.confirmed ? " – bestätigt" : ""}`, `Sent: ${r.txid}${r.confirmed ? " – confirmed" : ""}`), "ok");
  if (st.stage === "tresor-open" && r.probe) saveProbe(store, st.network, st.address, r.probe);
  if (st.stage === "tresor-cancel") forgetProbe(store, st.network, st.address);
}

async function pay(send: boolean) {
  const pr = probeOfAccount();
  if (!pr || !st.address) return;
  if (send && !window.confirm(tr(`${st.network.toUpperCase()}: Zahlung jetzt auslösen und senden?`, `${st.network.toUpperCase()}: trigger and send the payment now?`))) return;
  const j = await callProbeApi(fetchFn, payBody(st.network, pr, send));
  st.pay = j;
  if (j.ok !== true) throw new Error(String(j.error ?? "probe-pay"));
  log(
    send
      ? tr(`Zahlung gesendet: ${String(j.txid)}`, `Payment sent: ${String(j.txid)}`)
      : tr(`Zahlung möglich: Gebühr ${kasText(Number(j.feeSompi))}. Zum Senden „Zahlung senden“ drücken.`, `Payment possible: fee ${kasText(Number(j.feeSompi))}. Press “Send payment” to send.`),
    "ok",
  );
  if (send && j.probe) saveProbe(store, st.network, st.address, j.probe as ProbeFile);
}

setup();
