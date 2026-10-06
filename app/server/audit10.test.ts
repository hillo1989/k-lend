// Regressionstests zu Audit 10 (audit/10-opus-app.md): die Befunde
// A10-W-6, -8 und -9 dürfen nicht zurückkommen.
import { describe, expect, it } from "vitest";
import { buildActionArgs, checkRequest, isNodeError } from "./actions.ts";

const ok = () => true;

describe("A10-W-6: Mindestwerte beim Einlegen und Abziehen", () => {
  it("Senden ohne Mindestwerte wird abgelehnt, der Probelauf braucht keine", () => {
    expect(() =>
      buildActionArgs({ network: "mainnet", action: "pool-add", params: { key: "keys/a.json", kas: "10", ghost: "1" }, dryRun: false, confirmMainnet: true }, ok),
    ).toThrow(/Mindestanteile/);
    expect(() =>
      buildActionArgs({ network: "mainnet", action: "pool-remove", params: { key: "keys/a.json", percent: "50" }, dryRun: false, confirmMainnet: true }, ok),
    ).toThrow(/Mindestbeträge/);
    const dry = buildActionArgs({ network: "mainnet", action: "pool-add", params: { key: "keys/a.json", kas: "10", ghost: "1" }, dryRun: true }, ok);
    expect(dry.args.join(" ")).not.toMatch(/--min/);
  });
  it("Mindestwerte gehen als Optionen an ghostctl", () => {
    const add = buildActionArgs(
      { network: "mainnet", action: "pool-add", params: { key: "keys/a.json", kas: "10", ghost: "1", minShares: "990" }, dryRun: false, confirmMainnet: true },
      ok,
    );
    expect(add.args.slice(-2)).toEqual(["--min-shares", "990"]);
    const rm = buildActionArgs(
      { network: "mainnet", action: "pool-remove", params: { key: "keys/a.json", percent: "50", minKas: "4.95", minGhost: "0.2475" }, dryRun: false, confirmMainnet: true },
      ok,
    );
    expect(rm.args.slice(-4)).toEqual(["--min-kas", "4.95", "--min-ghost", "0.2475"]);
  });
  it("unsinnige Mindestanteile werden abgelehnt", () => {
    for (const m of ["0", "-1", "1.5", "abc", "1e3"])
      expect(() =>
        buildActionArgs({ network: "mainnet", action: "pool-add", params: { key: "keys/a.json", kas: "1", ghost: "1", minShares: m }, dryRun: true }, ok),
      ).toThrow();
  });
});

describe("A10-W-8: fremde Seiten dürfen auch per GET nichts anstoßen", () => {
  const base = { method: "GET", host: "localhost:5180", origin: undefined, contentType: undefined, clientHeader: undefined, port: 5180 };
  it("cross-site und same-site werden abgelehnt", () => {
    expect(checkRequest({ ...base, fetchSite: "cross-site" })?.status).toBe(403);
    expect(checkRequest({ ...base, fetchSite: "same-site" })?.status).toBe(403);
  });
  it("eigene Seite, Adresszeile und Werkzeuge ohne Kopfzeile bleiben erlaubt", () => {
    expect(checkRequest({ ...base, fetchSite: "same-origin" })).toBeNull();
    expect(checkRequest({ ...base, fetchSite: "none" })).toBeNull();
    expect(checkRequest({ ...base })).toBeNull();
  });
});

describe("A10-W-9: isNodeError erkennt HTTP-Codes nur im Kontext", () => {
  it("Beträge und Vault-Nummern sind keine Node-Ausfälle", () => {
    expect(isNodeError("zu wenig GHOST: 502.00000000 vorhanden")).toBe(false);
    expect(isNodeError("Vault 503 gibt es nicht")).toBe(false);
  });
  it("echte Node-Fehler werden weiter erkannt", () => {
    expect(isNodeError("kein Node erreichbar (Resolver: Zeitlimit 8 s)")).toBe(true);
    expect(isNodeError("Verbindung fehlgeschlagen (3 Versuch(e)): WebSocket closed")).toBe(true);
    expect(isNodeError("HTTP 502 Bad Gateway")).toBe(true);
    expect(isNodeError("status code: 503")).toBe(true);
  });
});
