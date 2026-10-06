// Wallet-Routen /api/wallet/build und /api/wallet/submit: Prüfung der
// Eingaben (nur Adressen und Pläne, nie Schlüssel oder Pfade), Größen,
// Mainnet-Bestätigung, Ratenbegrenzung und der öffentliche Modus mit einem
// Ersatz für ghostctl.
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import http from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createGhostApi, redactPaths } from "./api.ts";
import { ValidationError } from "./actions.ts";
import {
  buildWalletBuildArgs,
  buildWalletSubmitCall,
  clientKey,
  createRateLimiter,
  ipInNets,
  ipKey,
  MAX_PLAN,
  trustedProxies,
  PLAN_KIND,
  WALLET_ACTIONS,
  WALLET_PARAMS,
} from "./walletActions.ts";

const A = "kaspa:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const T = "kaspatest:q" + "qpzry9x8gf2tvdw0s3jn54khce6mua7l".repeat(2).slice(0, 60);
const X = "ab".repeat(32);

const plan = (extra: Record<string, unknown> = {}) => ({ kind: PLAN_KIND, network: "mainnet", address: A, action: { action: "mint", vault: 0, ghost: 1 }, ...extra });

function bad(f: () => unknown, re: RegExp) {
  try {
    f();
  } catch (e) {
    expect(e).toBeInstanceOf(ValidationError);
    expect((e as Error).message).toMatch(re);
    return;
  }
  throw new Error("keine Ablehnung");
}

describe("wallet build: Argumente", () => {
  it("baut für jede Aktion eine feste Argumentliste ohne Schlüssel", () => {
    const ex: Record<string, Record<string, unknown>> = {
      "open-vault": { kas: "100" },
      mint: { vault: 0, ghost: "1.5" },
      repay: { vault: "2" },
      deposit: { vault: 1, kas: "3" },
      withdraw: { vault: 1, keep: "50" },
      close: { vault: 0 },
      redeem: { vault: 0, ghost: "2" },
      liquidate: { vault: 0, ghost: "1" },
      sweep: { vault: 3 },
      send: { to: A, kas: "1", message: "Hallo", onchain: true },
      transfer: { to: X, ghost: "1" },
      swap: { kas: "10", min: "0.3" },
      "pool-add": { kas: "10", ghost: "0.4", minShares: "1000" },
      "pool-remove": { percent: "50", minKas: "0", minGhost: "0.1" },
    };
    expect(Object.keys(ex).sort()).toEqual([...WALLET_ACTIONS].sort());
    for (const a of WALLET_ACTIONS) {
      const { args } = buildWalletBuildArgs({ network: "mainnet", action: a, address: A, params: ex[a] });
      expect(args.slice(0, 8)).toEqual(["--network", "mainnet", "--json", "wallet", "build", a, "--address", A]);
      expect(args.join(" ")).not.toMatch(/keys\/|--key\b|--ja|--send/);
    }
    expect(buildWalletBuildArgs({ action: "send", address: A, params: ex.send }).args).toContain("--message=Hallo");
    expect(buildWalletBuildArgs({ action: "mint", address: A, params: { vault: "007", ghost: "1.50" } }).args.slice(8)).toEqual(["--vault", "7", "--ghost", "1.5"]);
  });

  it("lehnt Schlüsselpfade, fremde Parameter und falsche Netze ab", () => {
    bad(() => buildWalletBuildArgs({ action: "send", address: A, params: { to: "keys/owner.json", kas: "1" } }), /Empfänger/);
    bad(() => buildWalletBuildArgs({ action: "transfer", address: A, params: { to: "keys/owner.json", ghost: "1" } }), /Empfänger/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: A, params: { vault: 0, ghost: "1", key: "keys/a.json" } }), /Unerwarteter Parameter „key“/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: A, params: { vault: 0, ghost: "1" }, key: "keys/a.json" } as never), /Unbekannter Parameter/);
    bad(() => buildWalletBuildArgs({ action: "oracle-update", address: A, params: {} }), /Unbekannte Aktion/);
    bad(() => buildWalletBuildArgs({ action: "pool-open", address: A, params: {} }), /Unbekannte Aktion/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: T, params: { vault: 0, ghost: "1" } }), /Netz/);
    bad(() => buildWalletBuildArgs({ network: "testnet-10", action: "send", address: T, params: { to: A, kas: "1" } }), /Netz/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: "kaspa:pqqqq", params: {} }), /Schnorr-Adresse/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: A, params: { vault: -1, ghost: "1" } }), /Vault-Nummer/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: A, params: { vault: 0, ghost: "1e9" } }), /Zahl mit Punkt/);
    bad(() => buildWalletBuildArgs({ action: "mint", address: A, params: { vault: 0, ghost: "0" } }), /größer als 0/);
    bad(() => buildWalletBuildArgs({ action: "swap", address: A, params: { kas: "1", ghost: "1" } }), /entweder/);
    bad(() => buildWalletBuildArgs({ action: "pool-remove", address: A, params: { percent: "101" } }), /höchstens 100/);
    bad(() => buildWalletBuildArgs({ action: "send", address: A, params: { to: A, kas: "1", message: "a‮b" } }), /Nachricht/);
    bad(() => buildWalletBuildArgs({ action: "send", address: A, params: { to: A, kas: "1", onchain: true } }), /keine Nachricht/);
  });

  it("eine Nachricht mit Bindestrich bleibt ein Wert", () => {
    const { args } = buildWalletBuildArgs({ action: "send", address: A, params: { to: A, kas: "1", message: "--send --ja" } });
    expect(args).toContain("--message=--send --ja");
    expect(args).not.toContain("--send");
  });

  it("jede Aktion kennt nur ihre Parameter", () => {
    for (const a of WALLET_ACTIONS) for (const k of WALLET_PARAMS[a]) expect(["vault", "kas", "ghost", "keep", "to", "message", "onchain", "min", "minShares", "percent", "minKas", "minGhost"]).toContain(k);
  });
});

describe("wallet submit: Prüfung", () => {
  it("Plan und Antwort als Dateien, Pfade wählt der Server", () => {
    const c = buildWalletSubmitCall({ network: "mainnet", plan: plan(), signed: '"{}"' });
    expect(c.args).toEqual(["--network", "mainnet", "--json", "wallet", "submit", "--plan", "{plan}", "--signed", "{signed}"]);
    expect(c.sends).toBe(false);
    expect(JSON.parse(c.files.plan).kind).toBe(PLAN_KIND);
  });

  it("Senden im Mainnet nur mit Bestätigung, dann mit --ja und --send", () => {
    bad(() => buildWalletSubmitCall({ plan: plan(), signed: "x", send: true }), /Bestätigung/);
    bad(() => buildWalletSubmitCall({ plan: plan(), signed: "x", confirmMainnet: true }), /ohne Senden/);
    bad(() => buildWalletSubmitCall({ plan: plan(), signed: "x", send: "yes" }), /true oder false/);
    const c = buildWalletSubmitCall({ plan: plan(), signed: "x", send: true, confirmMainnet: true });
    expect(c.args).toEqual(["--network", "mainnet", "--json", "--ja", "wallet", "submit", "--plan", "{plan}", "--signed", "{signed}", "--send"]);
    expect(buildWalletSubmitCall({ network: "testnet-10", plan: plan({ network: "testnet-10" }), signed: "x", send: true }).sends).toBe(true);
  });

  it("falsche Art, falsches Netz, zu groß, fremde Felder → abgelehnt", () => {
    bad(() => buildWalletSubmitCall({ plan: { kind: "ghost-wallet-plan:1", network: "mainnet" }, signed: "x" }), /falsche Art/);
    bad(() => buildWalletSubmitCall({ plan: plan({ network: "testnet-10" }), signed: "x" }), /anderen Netz/);
    bad(() => buildWalletSubmitCall({ plan: plan({ pad: "x".repeat(MAX_PLAN) }), signed: "x" }), /zu groß/);
    bad(() => buildWalletSubmitCall({ plan: plan(), signed: "x".repeat(400 * 1024) }), /zu groß/);
    bad(() => buildWalletSubmitCall({ plan: plan(), signed: "" }), /Antwort der Wallet fehlt/);
    bad(() => buildWalletSubmitCall({ plan: plan(), signed: "x", key: "keys/a.json" } as never), /Unbekannter Parameter/);
    bad(() => buildWalletSubmitCall({ plan: [plan()], signed: "x" }), /Signierplan fehlt/);
  });
});

describe("Ratenbegrenzung", () => {
  it("höchstens N je Minute und Absender, danach Wartezeit", () => {
    const l = createRateLimiter(3);
    expect([l.take("a", 0), l.take("a", 1), l.take("a", 2)]).toEqual([null, null, null]);
    expect(l.take("a", 3)).toBe(60);
    expect(l.take("b", 3)).toBe(null);
    expect(l.take("a", 60_001)).toBe(null);
  });
  it("hinter dem eigenen Webserver zählt der letzte X-Forwarded-For-Eintrag", () => {
    expect(clientKey("127.0.0.1", "1.2.3.4, 5.6.7.8")).toBe("5.6.7.8");
    expect(clientKey("::ffff:127.0.0.1", "5.6.7.8")).toBe("5.6.7.8");
    expect(clientKey("9.9.9.9", "1.2.3.4")).toBe("9.9.9.9");
    expect(clientKey("::1", "<script>")).toBe("0:0:0:0::/64");
  });

  it("A17-3: Webserver im Docker-Netz (GHOST_HOST) gilt als Proxy, fremde Absender nicht", () => {
    const t = trustedProxies(undefined, "127.0.0.1,172.18.0.1");
    expect(t).toEqual(["127.0.0.0/8", "::1/128", "172.18.0.0/16"]);
    // Caddy-Container 172.18.0.5: jeder Besucher eigener Schlüssel
    expect(clientKey("172.18.0.5", "203.0.113.7", t)).toBe("203.0.113.7");
    expect(clientKey("172.18.0.5", "203.0.113.8", t)).toBe("203.0.113.8");
    // der letzte (vom Webserver gesetzte) Eintrag zählt, nicht der erste (fälschbare)
    expect(clientKey("172.18.0.5", "1.1.1.1, 203.0.113.7", t)).toBe("203.0.113.7");
    // eigene Proxys rechts werden übersprungen
    expect(clientKey("172.18.0.5", "203.0.113.7, 172.18.0.9", t)).toBe("203.0.113.7");
    // Unsinn im letzten Eintrag: zählt die Gegenstelle, nie ein fälschbarer Eintrag weiter links
    expect(clientKey("172.18.0.5", "203.0.113.7, <script>", t)).toBe("172.18.0.5");
    // ohne die Docker-Angabe: früheres Verhalten, alle Besucher teilen den Container-Schlüssel
    expect(clientKey("172.18.0.5", "203.0.113.7")).toBe("172.18.0.5");
    // ein fremder Absender kann X-Forwarded-For nicht setzen
    expect(clientKey("198.51.100.1", "203.0.113.7", t)).toBe("198.51.100.1");
    expect(clientKey("172.19.0.5", "203.0.113.7", t)).toBe("172.19.0.5");
  });

  it("A17-3: IPv6 zählt je /64", () => {
    const t = trustedProxies(undefined, "172.18.0.1");
    const a = clientKey("172.18.0.5", "2001:db8:1:2:aaaa::1", t);
    expect(a).toBe("2001:db8:1:2::/64");
    expect(clientKey("172.18.0.5", "2001:db8:1:2:ffff:1:2:3", t)).toBe(a);
    expect(clientKey("172.18.0.5", "2001:db8:1:3::1", t)).not.toBe(a);
    expect(clientKey("2001:db8:1:2::99", undefined)).toBe(a);
    expect(ipKey("::ffff:1.2.3.4".replace("::ffff:", ""))).toBe("1.2.3.4");
    // 64 Adressen eines /64 teilen sich das Kontingent
    const l = createRateLimiter(3);
    const hits = Array.from({ length: 5 }, (_, i) => l.take(clientKey("172.18.0.5", `2001:db8:1:2::${i + 1}`, t), i));
    expect(hits.filter((x) => x === null)).toHaveLength(3);
  });

  it("A17-3: GHOST_TRUSTED_PROXY ersetzt den Standard, Unsinn wird abgelehnt", () => {
    expect(trustedProxies("10.0.0.0/24, 192.168.1.2", "172.18.0.1")).toEqual(["127.0.0.0/8", "::1/128", "10.0.0.0/24", "192.168.1.2"]);
    expect(() => trustedProxies("0.0.0.0/0", undefined)).toThrow(/GHOST_TRUSTED_PROXY/);
    expect(() => trustedProxies("irgendwas", undefined)).toThrow(/GHOST_TRUSTED_PROXY/);
    expect(ipInNets("10.0.0.200", ["10.0.0.0/24"])).toBe(true);
    expect(ipInNets("10.0.1.1", ["10.0.0.0/24"])).toBe(false);
    expect(ipInNets("::1", ["127.0.0.0/8"])).toBe(false);
  });
});

// ------------------------------------------------- über den Server ----

function project() {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-wallet-api-"));
  mkdirSync(path.join(dir, "deployments"));
  // Ersatz für ghostctl: Argumente und den Inhalt der Plan-Datei mitschreiben
  writeFileSync(
    path.join(dir, "ghostctl"),
    `#!/bin/sh
echo "$*" >> "${dir}/calls.txt"
for a in "$@"; do case "$a" in */plan.json) cat "$a" > "${dir}/plan-copy.json";; esac; done
echo '{"ok":true,"valid":true,"plan":{"kind":"${PLAN_KIND}"}}'
`,
  );
  chmodSync(path.join(dir, "ghostctl"), 0o755);
  const calls = () => {
    try {
      return readFileSync(path.join(dir, "calls.txt"), "utf8").trim().split("\n");
    } catch {
      return [];
    }
  };
  return { dir, calls };
}

function request(port: number, p: string, body: unknown, headers: Record<string, string> = {}): Promise<{ status: number; body: string; headers: http.IncomingHttpHeaders }> {
  return new Promise((resolve, reject) => {
    const data = typeof body === "string" ? body : JSON.stringify(body);
    const req = http.request(
      { host: "127.0.0.1", port, method: "POST", path: p, headers: { host: `localhost:${port}`, "content-type": "application/json", "x-ghost-client": "1", ...headers } },
      (res) => {
        let b = "";
        res.setEncoding("utf8");
        res.on("data", (c) => (b += c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body: b, headers: res.headers }));
      },
    );
    req.on("error", reject);
    req.write(data);
    req.end();
  });
}

const servers: http.Server[] = [];
afterEach(() => {
  for (const s of servers.splice(0)) s.close();
});

async function start(dir: string, opts: Parameters<typeof createGhostApi>[1] = {}) {
  const api = createGhostApi(dir, { public: true, ghostctl: path.join(dir, "ghostctl"), ...opts });
  const server = http.createServer((req, res) => api(req, res, () => {
    res.statusCode = 404;
    res.end();
  }));
  servers.push(server);
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", r));
  return (server.address() as AddressInfo).port;
}

describe("Wallet-Routen im öffentlichen Modus", () => {
  it("build läuft öffentlich; ghostctl bekommt nur geprüfte Argumente", async () => {
    const p = project();
    const port = await start(p.dir);
    const r = await request(port, "/api/wallet/build", { network: "mainnet", action: "mint", address: A, params: { vault: 0, ghost: "1" } });
    expect(r.status).toBe(200);
    expect(JSON.parse(r.body).ok).toBe(true);
    expect(p.calls()).toEqual([`--network mainnet --json wallet build mint --address ${A} --vault 0 --ghost 1`]);
  });

  it("ungültige Anfragen erreichen ghostctl nie", async () => {
    const p = project();
    const port = await start(p.dir);
    for (const body of [
      { action: "send", address: A, params: { to: "keys/owner.json", kas: "1" } },
      { action: "mint", address: A, params: { vault: 0, ghost: "1", key: "keys/x.json" } },
      { action: "mint", address: "../../etc/passwd", params: {} },
    ]) {
      const r = await request(port, "/api/wallet/build", body);
      expect(r.status).toBe(400);
    }
    const r = await request(port, "/api/wallet/submit", { plan: plan(), signed: "x", send: true });
    expect(r.status).toBe(400);
    expect((await request(port, "/api/wallet/build", "{kaputt")).status).toBe(400);
    expect(p.calls()).toEqual([]);
  });

  it("submit: Plan geht als Datei im Temp-Verzeichnis an ghostctl, danach ist sie weg", async () => {
    const p = project();
    const port = await start(p.dir);
    const r = await request(port, "/api/wallet/submit", { plan: plan(), signed: '"{}"', send: true, confirmMainnet: true });
    expect(r.status).toBe(200);
    const [call] = p.calls();
    expect(call).toMatch(/^--network mainnet --json --ja wallet submit --plan \S+\/ghost-wallet-\w+\/plan\.json --signed \S+\/signed\.json --send$/);
    expect(JSON.parse(readFileSync(path.join(p.dir, "plan-copy.json"), "utf8")).kind).toBe(PLAN_KIND);
    const planPath = call.split(" ")[7];
    expect(() => readFileSync(planPath)).toThrow();
  });

  it("zu große Anfrage → 413, ohne ghostctl", async () => {
    const p = project();
    const port = await start(p.dir);
    const r = await request(port, "/api/wallet/submit", { plan: plan(), signed: "x".repeat(900 * 1024) });
    expect(r.status).toBe(413);
    expect(p.calls()).toEqual([]);
  });

  it("Ratenbegrenzung je Absender mit Retry-After", async () => {
    const p = project();
    const port = await start(p.dir, { walletPerMinute: 2 });
    const body = { action: "close", address: A, params: { vault: 0 } };
    expect((await request(port, "/api/wallet/build", body)).status).toBe(200);
    expect((await request(port, "/api/wallet/build", body)).status).toBe(200);
    const r = await request(port, "/api/wallet/build", body);
    expect(r.status).toBe(429);
    expect(Number(r.headers["retry-after"])).toBeGreaterThan(0);
    expect(p.calls().length).toBe(2);
  });

  it("eigene Domain hinter dem Webserver (https-Origin) wird angenommen, fremde nicht", async () => {
    const p = project();
    const port = await start(p.dir, { publicHosts: ["ghost.example"] });
    const body = { action: "close", address: A, params: { vault: 0 } };
    const ok = await request(port, "/api/wallet/build", body, { host: "ghost.example", origin: "https://ghost.example", "sec-fetch-site": "same-origin" });
    expect(ok.status).toBe(200);
    const no = await request(port, "/api/wallet/build", body, { host: "ghost.example", origin: "https://boese.example" });
    expect(no.status).toBe(403);
    const no2 = await request(port, "/api/wallet/build", body, { host: "boese.example" });
    expect(no2.status).toBe(403);
  });

  it("Schlüssel-Routen bleiben öffentlich gesperrt", async () => {
    const p = project();
    const port = await start(p.dir);
    const r = await request(port, "/api/action", { network: "mainnet", action: "mint", params: { key: "keys/a.json", vault: 0, ghost: "1" }, dryRun: true });
    expect(r.status).toBe(403);
    expect(p.calls()).toEqual([]);
  });
});

// ------------------------------------------------------------ Audit 17 ----

/** Ersatz für ghostctl mit eigenem Verhalten (Shell-Rumpf nach dem Mitschreiben) */
function stub(body: string) {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-a17-"));
  mkdirSync(path.join(dir, "deployments"));
  writeFileSync(path.join(dir, "ghostctl"), `#!/bin/sh\necho "$*" >> "${dir}/calls.txt"\n${body}\n`);
  chmodSync(path.join(dir, "ghostctl"), 0o755);
  return dir;
}

function get(port: number, p: string): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    http
      .get({ host: "127.0.0.1", port, path: p, headers: { host: `localhost:${port}` } }, (res) => {
        let b = "";
        res.on("data", (c) => (b += c));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body: b }));
      })
      .on("error", reject);
  });
}

const SEND = { plan: plan(), signed: '"{}"', send: true, confirmMainnet: true };
const CHECK = { plan: plan(), signed: '"{}"' };

describe("Audit 17: Wallet-Routen hinter dem Proxy", () => {
  it("A17-4: höchstens 1 Senden und 2 submit gleichzeitig – Überzählige sofort 503, ohne ghostctl", async () => {
    const dir = stub(`sleep 0.6\necho '{"ok":true,"valid":true,"sent":true,"confirmed":true}'`);
    const port = await start(dir);
    const sends = await Promise.all([request(port, "/api/wallet/submit", SEND), request(port, "/api/wallet/submit", SEND)]);
    expect(sends.map((r) => r.status).sort()).toEqual([200, 503]);
    const busy = sends.find((r) => r.status === 503)!;
    expect(JSON.parse(busy.body)).toMatchObject({ ok: false, busy: true });
    expect(busy.headers["retry-after"]).toBe("10");
    const checks = await Promise.all([1, 2, 3].map(() => request(port, "/api/wallet/submit", CHECK)));
    expect(checks.map((r) => r.status).sort()).toEqual([200, 200, 503]);
    expect(readFileSync(path.join(dir, "calls.txt"), "utf8").trim().split("\n")).toHaveLength(3);
  });

  it("G-3: ein abgewiesener Plan wird 5 Minuten sofort abgelehnt, ohne ghostctl", async () => {
    const dir = stub(`echo '{"ok":false,"error":"Plan passt nicht zum aktuellen Stand – nicht gesendet","transactions":[]}'; exit 1`);
    const port = await start(dir);
    const first = await request(port, "/api/wallet/submit", SEND);
    expect(JSON.parse(first.body).ok).toBe(false);
    const again = await request(port, "/api/wallet/submit", SEND);
    expect(again.status).toBe(409);
    expect(JSON.parse(again.body)).toMatchObject({ ok: false, rejected: true });
    expect(readFileSync(path.join(dir, "calls.txt"), "utf8").trim().split("\n")).toHaveLength(1);
    // ein Node-Ausfall sperrt den Plan nicht
    const down = stub(`echo '{"ok":false,"error":"Nodes nicht erreichbar (WebSocket)"}'; exit 1`);
    const p2 = await start(down);
    await request(p2, "/api/wallet/submit", SEND);
    expect((await request(p2, "/api/wallet/submit", SEND)).status).not.toBe(409);
  });

  it("G-3: Sendungen insgesamt begrenzt", async () => {
    const dir = stub(`echo '{"ok":true,"valid":true,"sent":true,"confirmed":true}'`);
    const port = await start(dir, { sendsPerMinute: 2 });
    const codes = [];
    for (let i = 0; i < 3; i++) codes.push((await request(port, "/api/wallet/submit", { ...SEND, signed: `"{}${i}"` })).status);
    expect(codes).toEqual([200, 200, 503]);
  });

  it("A17-4: hängende Wallet-Aufrufe verdrängen Status und Preis nicht (eigene Plätze)", async () => {
    const dir = stub(`case "$*" in *wallet*) sleep 2;; esac\necho '{"ok":true}'`);
    const port = await start(dir, { maxProcs: 1 });
    const slow = [1, 2].map(() => request(port, "/api/wallet/build", { action: "close", address: A, params: { vault: 0 } }));
    await new Promise((r) => setTimeout(r, 200));
    const t0 = Date.now();
    expect((await get(port, "/api/price")).status).toBe(200);
    expect(Date.now() - t0).toBeLessThan(1500);
    await Promise.all(slow);
  });

  it("A17-7: Fehler nach dem Senden und Zeitüberschreitung → unklar, nicht „nicht gesendet“", async () => {
    const after = stub(`echo '{"ok":false,"error":"Tx abc nach 45s nicht bestätigt","transactions":[{"sent":true,"txid":"abc"}]}'; exit 1`);
    let port = await start(after);
    expect(JSON.parse((await request(port, "/api/wallet/submit", SEND)).body)).toMatchObject({ ok: false, unclear: true });
    const journal = stub(`echo '{"ok":false,"error":"Node lehnt ab – ob die Transaktion trotzdem im Netz ist, ist unklar","transactions":[{"sent":false,"unclear":true}]}'; exit 1`);
    port = await start(journal);
    expect(JSON.parse((await request(port, "/api/wallet/submit", SEND)).body)).toMatchObject({ ok: false, unclear: true });
    const before = stub(`echo '{"ok":false,"error":"Signatur ungültig – nicht gesendet","transactions":[]}'; exit 1`);
    port = await start(before);
    expect(JSON.parse((await request(port, "/api/wallet/submit", SEND)).body).unclear).toBeUndefined();
    const hang = stub(`sleep 5`);
    port = await start(hang, { walletSendTimeoutMs: 300 });
    expect(JSON.parse((await request(port, "/api/wallet/submit", SEND)).body)).toMatchObject({ ok: false, timeout: true, unclear: true });
  });

  it("A17-7: Zeitlimit der Seite beim Senden liegt unter dem des Webservers", async () => {
    const { WALLET_SEND_TIMEOUT_MS } = await import("./api.ts");
    const tpl = (f: string) => readFileSync(path.join(__dirname, "..", "..", "deploy", "hetzner", "templates", f), "utf8");
    const nginx = Number(/proxy_read_timeout (\d+)s;/.exec(tpl("nginx-ghost.conf"))![1]);
    const apache = Number(/ProxyTimeout (\d+)/.exec(tpl("apache-ghost.conf"))![1]);
    expect(WALLET_SEND_TIMEOUT_MS).toBeLessThan(nginx * 1000);
    expect(WALLET_SEND_TIMEOUT_MS).toBeLessThan(apache * 1000);
    expect(tpl("ghost.caddy")).not.toMatch(/timeout/); // Caddy wartet ohne Limit
  });

  it("A17-5: Webserver-Vorlagen lassen eine ganze submit-Anfrage durch", async () => {
    const { WALLET_BODY_LIMIT } = await import("./walletActions.ts");
    const tpl = (f: string) => readFileSync(path.join(__dirname, "..", "..", "deploy", "hetzner", "templates", f), "utf8");
    const unit = (n: string, u: string) => Number(n) * ({ k: 1024, kb: 1024, m: 1024 * 1024, mb: 1024 * 1024, "": 1 } as Record<string, number>)[u.toLowerCase()];
    const caddy = /max_size (\d+)(\w*)/.exec(tpl("ghost.caddy"))!;
    const nginx = /client_max_body_size (\d+)(\w*);/.exec(tpl("nginx-ghost.conf"))!;
    const apache = /LimitRequestBody (\d+)/.exec(tpl("apache-ghost.conf"))!;
    for (const [n, u] of [caddy.slice(1), nginx.slice(1), [apache[1], ""]]) expect(unit(n, u)).toBeGreaterThanOrEqual(WALLET_BODY_LIMIT);
    // und die Doku zum Caddy im Docker-Container nennt dieselbe Grenze
    const readme = readFileSync(path.join(__dirname, "..", "..", "deploy", "hetzner", "README.md"), "utf8");
    expect(readme).toMatch(/request_body \{\n\t\tmax_size 1MB\n\t\}\n\treverse_proxy 172\.18\.0\.1:8787/);
  });

  it("A17-8: öffentliche Fehlermeldungen ohne absolute Server-Pfade, URLs bleiben", async () => {
    const dir = stub(
      `echo '{"ok":false,"error":"Eine andere ghostctl-Instanz arbeitet gerade (Sperre /opt/ghost/kaspa-lending/deployments/mainnet.lock). REST-API: https://api.kaspa.org/transactions/ab: 500"}'; exit 1`,
    );
    const port = await start(dir);
    const j = JSON.parse((await request(port, "/api/wallet/build", { action: "close", address: A, params: { vault: 0 } })).body);
    expect(j.error).toContain("(Sperre mainnet.lock)");
    expect(j.error).toContain("https://api.kaspa.org/transactions/ab");
    expect(j.error).not.toContain("/opt/");
    const err = stub(`echo "Fehler: keine Zustandsdatei /opt/ghost/kaspa-lending/deployments/mainnet.json – zuerst deploy" >&2; exit 1`);
    const p2 = await start(err);
    const k = JSON.parse((await request(p2, "/api/wallet/build", { action: "close", address: A, params: { vault: 0 } })).body);
    expect(k.error).toContain("mainnet.json");
    expect(k.error).not.toContain("/opt/");
    expect(redactPaths("Journal /home/x/deployments/mainnet.pending.json: kaputt")).toBe("Journal mainnet.pending.json: kaputt");
  });

  it("A17-3: über den Server – Absender aus X-Forwarded-For des vertrauten Proxys", async () => {
    const dir = stub(`echo '{"ok":true}'`);
    const port = await start(dir, { walletPerMinute: 1, trustedProxies: ["127.0.0.0/8"] });
    const body = { action: "close", address: A, params: { vault: 0 } };
    expect((await request(port, "/api/wallet/build", body, { "x-forwarded-for": "203.0.113.7" })).status).toBe(200);
    expect((await request(port, "/api/wallet/build", body, { "x-forwarded-for": "198.51.100.1, 203.0.113.7" })).status).toBe(429);
    expect((await request(port, "/api/wallet/build", body, { "x-forwarded-for": "203.0.113.8" })).status).toBe(200);
    // ohne vertrauten Proxy zählt nur die Gegenstelle: X-Forwarded-For bleibt wirkungslos
    const p2 = await start(dir, { walletPerMinute: 1, trustedProxies: [] });
    expect((await request(p2, "/api/wallet/build", body, { "x-forwarded-for": "203.0.113.7" })).status).toBe(200);
    expect((await request(p2, "/api/wallet/build", body, { "x-forwarded-for": "203.0.113.8" })).status).toBe(429);
  });
});
