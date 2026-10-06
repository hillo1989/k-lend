// Audit 20 c, Betrieb: A20c-5 (haertung.sh prüft vor dem Ersetzen), A20c-6
// (.gitignore und GitHub-Sync halten Betriebsdateien zurück). Die Skripte
// laufen hier nicht gegen Server oder GitHub; geprüft werden ihre Regeln.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (p: string) => readFileSync(path.join(ROOT, p), "utf8");

/** Variable aus github-sync.sh (zsh) auslesen */
function syncVar(name: string): string {
  const src = read("deploy/github-sync.sh");
  const lines = src.split("\n").filter((l) => /^(OEFFENTLICH_DEPLOYMENTS|VERBOTEN|ERLAUBT)=/.test(l));
  return execFileSync("zsh", ["-c", `${lines.join("\n")}\nprint -r -- "$${name}"`], { encoding: "utf8" }).trim();
}
const grepE = (re: string, lines: string[]) => lines.filter((l) => new RegExp(re).test(l));

describe("A20c-6: Betriebsdateien gehen nie ins öffentliche Repo", () => {
  it(".gitignore ignoriert Zustand, Tresore, Zinsregel, Journale – außer den v1-Kettendaten", () => {
    const ignored = (f: string) => {
      try {
        execFileSync("git", ["-C", ROOT, "check-ignore", "-q", "--no-index", f]);
        return true;
      } catch {
        return false;
      }
    };
    for (const f of [
      "deployments/mainnet.json",
      "deployments/mainnet-tresore.json",
      "deployments/mainnet-zins.json",
      "deployments/mainnet.pending.json",
      "deployments/mainnet-txlog.jsonl",
      "deployments/mainnet-abos.json",
      "deployments/mainnet-v4probe.stufe",
      "deployments/mainnet.lock",
      "deployments/mainnet-v2.json",
    ])
      expect(ignored(f), f).toBe(true);
    for (const f of ["deployments/mainnet-v1.json", "deployments/testnet-10-v1.json", "protocol/Cargo.lock", "app/package-lock.json"]) expect(ignored(f), f).toBe(false);
  });

  it("Sperr- und Freigabeliste des Syncs: heutiger Stand geht durch, Betriebsdateien nicht", () => {
    const VERBOTEN = syncVar("VERBOTEN");
    const ERLAUBT = syncVar("ERLAUBT");
    const AUSLASSEN = ["RECHT_PRUEFUNG.md", "bin/ghostctl-v1", "audit/6-aussagen-und-doku.md"];
    const tracked = execFileSync("git", ["-C", ROOT, "-c", "core.quotepath=off", "ls-files"], { encoding: "utf8" })
      .trim()
      .split("\n")
      .filter((f) => !AUSLASSEN.includes(f));
    expect(grepE(VERBOTEN, tracked)).toEqual([]);
    expect(tracked.filter((f) => !new RegExp(ERLAUBT).test(f))).toEqual([]);
    const bad = [
      "deployments/mainnet.json",
      "deployments/mainnet-tresore.json",
      "deployments/mainnet-zins.json",
      "deployments/mainnet.pending.json",
      "deployments/mainnet-txlog.jsonl",
      "deployments/mainnet.lock",
      "keys/mainnet-keeper.json",
    ];
    expect(grepE(VERBOTEN, bad)).toEqual(bad);
    // Unbekanntes außerhalb der Freigabeliste bricht ab
    expect(["geheim/notiz.txt", "deployments/mainnet-v3.json", "bin/ghostctl-v4"].filter((f) => new RegExp(ERLAUBT).test(f))).toEqual([]);
  });

  it("Sync entfernt deployments/ außer den v1-Dateien vor dem Commit", () => {
    const src = read("deploy/github-sync.sh");
    expect(src).toMatch(/git -c core\.quotepath=off ls-files deployments \| grep -vE/);
    expect(src.indexOf("BETRIEB=")).toBeLessThan(src.indexOf("git add -A"));
    expect(execFileSync("zsh", ["-n", path.join(ROOT, "deploy/github-sync.sh")]).toString()).toBe("");
  });
});

describe("A20c-5: haertung.sh prüft die neue Caddyfile, bevor es sie schreibt", () => {
  const src = read("deploy/hetzner/haertung.sh");
  it("Temp-Datei per mktemp, Prüfung im Container, erst danach in die echte Datei", () => {
    expect(src).not.toMatch(/\/tmp\/Caddyfile\.neu/);
    expect(src).toMatch(/NEU=\$\(mktemp /);
    const validate = src.indexOf('caddy validate --config "$PRUEF"');
    const write = src.indexOf('cat "$NEU" > "$F"');
    expect(validate).toBeGreaterThan(0);
    expect(write).toBeGreaterThan(validate);
    expect(src).toMatch(/docker cp "\$NEU" "\$C:\$PRUEF"/);
    expect(src).toMatch(/trap aufraeumen EXIT/);
  });
  it("bash -n", () => {
    expect(() => execFileSync("bash", ["-n", path.join(ROOT, "deploy/hetzner/haertung.sh")])).not.toThrow();
  });
});
