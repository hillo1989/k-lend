// Der lokale Server stößt Daueraufträge jede Minute an. Doppelt anstoßen
// (zwei Takte gleichzeitig, oder Server und GHOST-Agent) muss harmlos sein:
// hier die Seite des Servers (nie zwei Läufe gleichzeitig, nur bei Fälligem);
// die Seite von ghostctl (Sperre + Journal) prüfen die Rust-Tests in abo.rs.
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { createGhostApi } from "./api.ts";
import { localDate } from "./actions.ts";

function project(due: boolean) {
  const dir = mkdtempSync(path.join(tmpdir(), "ghost-abotimer-"));
  mkdirSync(path.join(dir, "deployments"));
  // Ersatz für ghostctl: schreibt jeden Aufruf mit und braucht etwas Zeit
  writeFileSync(
    path.join(dir, "ghostctl"),
    `#!/bin/sh\necho "$*" >> "${dir}/calls.txt"\nsleep 0.3\necho '{"ok":true,"reports":[{"id":"0a1b2c3d","ok":true,"paid":true,"text":"gesendet"}]}'\n`,
  );
  chmodSync(path.join(dir, "ghostctl"), 0o755);
  const nextDue = due ? localDate(new Date()) : "2999-01-01";
  writeFileSync(
    path.join(dir, "deployments", "testnet-10-abos.json"),
    JSON.stringify({ version: 1, network: "testnet-10", abos: [{ id: "0a1b2c3d", nextDue, paused: false, ended: null, inflight: null, retry: null }], archive: [] }),
  );
  const calls = () => {
    try {
      return readFileSync(path.join(dir, "calls.txt"), "utf8").trim().split("\n");
    } catch {
      return [];
    }
  };
  return { api: createGhostApi(dir), calls };
}

describe("Takt für Daueraufträge", () => {
  it("zwei gleichzeitige Takte starten ghostctl nur einmal", async () => {
    const { api, calls } = project(true);
    await Promise.all([api.aboTick(), api.aboTick(), api.aboTick()]);
    expect(calls()).toEqual(["--network testnet-10 --json --ja abo run"]);
    // nächster Takt darf wieder (ghostctl selbst findet dann nichts Fälliges mehr)
    await api.aboTick();
    expect(calls()).toHaveLength(2);
  });
  it("ohne Fälliges wird ghostctl gar nicht gestartet", async () => {
    const { api, calls } = project(false);
    await api.aboTick();
    expect(calls()).toEqual([]);
  });
});
