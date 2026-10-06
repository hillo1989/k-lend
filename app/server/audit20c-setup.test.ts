// Audit 20 A20c-2/3: setup.sh führt Skript und Vorlagen nur aus einem
// root-eigenen Ordner aus, npm ohne Install-Skripte, getrennte Benutzer für
// Seite und Agent, systemd-Härtung. Die Skripte laufen hier nicht als root;
// geprüft werden die Regeln und die Prüffunktion (mit ERWARTE_UID = eigener
// Benutzer statt root).
import { execFileSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir, userInfo } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const H = path.join(ROOT, "deploy", "hetzner");
const read = (p: string) => readFileSync(path.join(H, p), "utf8");

/** pruefe_root_eigen aus setup.sh ausführen */
function pruefe(dir: string): { ok: boolean; out: string } {
  const src = read("setup.sh");
  const fn = src.slice(src.indexOf("pruefe_root_eigen() {"), src.indexOf("\n}\n", src.indexOf("pruefe_root_eigen() {")) + 3);
  try {
    const out = execFileSync("bash", ["-c", `${fn}\npruefe_root_eigen "$1"`, "x", dir], { encoding: "utf8", env: { ...process.env, ERWARTE_UID: String(userInfo().uid) } });
    return { ok: true, out };
  } catch (e) {
    return { ok: false, out: String((e as { stdout?: string }).stdout ?? "") };
  }
}

function deployDir() {
  const base = mkdtempSync(path.join(tmpdir(), "ghost-deploy-"));
  chmodSync(base, 0o700);
  const d = path.join(base, "ghost-deploy");
  mkdirSync(path.join(d, "templates"), { recursive: true, mode: 0o700 });
  chmodSync(d, 0o700);
  chmodSync(path.join(d, "templates"), 0o700);
  writeFileSync(path.join(d, "setup.sh"), "echo hi\n", { mode: 0o600 });
  writeFileSync(path.join(d, "templates", "ghost-web.service"), "[Unit]\n", { mode: 0o600 });
  return d;
}

describe("A20c-2: setup.sh nur aus einem Ordner, den niemand sonst beschreiben kann", () => {
  it("eigener, nur für den Besitzer beschreibbarer Ordner geht durch", () => {
    expect(pruefe(deployDir()).ok).toBe(true);
  });
  it("gruppen- oder fremd-beschreibbare Vorlage, Verknüpfung oder Ordner → Abbruch", () => {
    const a = deployDir();
    chmodSync(path.join(a, "templates", "ghost-web.service"), 0o664);
    expect(pruefe(a)).toMatchObject({ ok: false });
    expect(pruefe(a).out).toMatch(/ghost-web\.service/);
    const b = deployDir();
    chmodSync(path.join(b, "templates"), 0o777);
    expect(pruefe(b).ok).toBe(false);
    const c = deployDir();
    symlinkSync("/etc/passwd", path.join(c, "templates", "x.service"));
    expect(pruefe(c).ok).toBe(false);
    // Elternordner für andere beschreibbar (wie der Code-Ordner des Dienstbenutzers aus Sicht von root)
    const d = deployDir();
    chmodSync(path.dirname(d), 0o777);
    expect(pruefe(d).out).toMatch(/für andere beschreibbar/);
  });
  it("setup.sh prüft sich vor allem anderen und nimmt Vorlagen nur von dort", () => {
    const s = read("setup.sh");
    expect(s.indexOf('pruefe_root_eigen "$SCRIPT_DIR"')).toBeGreaterThan(0);
    expect(s.indexOf('pruefe_root_eigen "$SCRIPT_DIR"')).toBeLessThan(s.indexOf('step "1. Benutzer'));
    expect(s).toContain('TEMPLATES="$SCRIPT_DIR/templates"');
    expect(s).not.toContain('TEMPLATES="$GHOST_DIR/deploy/hetzner/templates"');
    expect(s).toContain("GENERATED=/etc/ghost/generated");
  });
  it("npm ci ohne Install-Skripte; das Lockfile braucht keine (nur fsevents, macOS)", () => {
    expect(read("setup.sh")).toMatch(/npm ci --ignore-scripts/);
    const lock = JSON.parse(readFileSync(path.join(ROOT, "app", "package-lock.json"), "utf8")) as { packages: Record<string, { hasInstallScript?: boolean; os?: string[] }> };
    const withScripts = Object.entries(lock.packages).filter(([, p]) => p.hasInstallScript);
    expect(withScripts.map(([k]) => k)).toEqual(["node_modules/fsevents"]);
    expect(withScripts[0][1].os).toEqual(["darwin"]);
  });
  it("push-from-mac.sh legt deploy/hetzner root-eigen unter /root/ghost-deploy ab", () => {
    const s = read("push-from-mac.sh");
    expect(s).toMatch(/\/root\/ghost-deploy/);
    expect(s).toMatch(/chown -R root:root/);
    expect(s).toMatch(/bash \/root\/ghost-deploy\/setup\.sh/);
  });
  it("alle Skripte: bash -n", () => {
    for (const f of ["setup.sh", "push-from-mac.sh", "keys-upload.sh", "haertung.sh", "migration-a20c.sh"]) expect(() => execFileSync("bash", ["-n", path.join(H, f)]), f).not.toThrow();
  });
});

describe("A20c-3: getrennte Benutzer und Härtung", () => {
  const web = read("templates/ghost-web.service");
  const agent = read("templates/ghost-agent.service");
  const val = (unit: string, k: string) => unit.split("\n").filter((l) => l.startsWith(`${k}=`)).map((l) => l.slice(k.length + 1));
  it("Seite als ghost-web (Zusatzgruppe ghost), Agent als ghost", () => {
    expect(val(web, "User")).toEqual(["ghost-web"]);
    expect(val(web, "SupplementaryGroups")).toEqual(["ghost"]);
    expect(val(agent, "User")).toEqual(["ghost"]);
    expect(val(web, "InaccessiblePaths")).toEqual(["-@GHOST_DIR@/keys"]);
  });
  it("beide: ProtectProc=invisible, Systemaufruf-Filter, nur deployments/ beschreibbar, nichts dort ausführbar, Dateien 660", () => {
    for (const u of [web, agent]) {
      expect(val(u, "ProtectProc")).toEqual(["invisible"]);
      expect(val(u, "SystemCallFilter")).toEqual(["@system-service", "~@privileged @obsolete"]);
      expect(val(u, "SystemCallErrorNumber")).toEqual(["EPERM"]);
      expect(val(u, "ReadWritePaths")).toEqual(["@GHOST_DIR@/deployments"]);
      expect(val(u, "NoExecPaths")[0]).toContain("@GHOST_DIR@/deployments");
      expect(val(u, "UMask")).toEqual(["0007"]);
      expect(val(u, "CapabilityBoundingSet")).toEqual([""]);
      expect(val(u, "NoNewPrivileges")).toEqual(["true"]);
    }
    // Node braucht AF_UNIX (socketpair für ghostctl) und JIT (kein MemoryDenyWriteExecute)
    expect(val(web, "RestrictAddressFamilies")).toEqual(["AF_INET AF_INET6 AF_UNIX"]);
    expect(val(web, "MemoryDenyWriteExecute")).toEqual([]);
    expect(val(agent, "MemoryDenyWriteExecute")).toEqual(["true"]);
  });
  it("setup.sh: Benutzer ghost-web, deployments/ 2770 mit Gruppenrechten, keys/ 700, ptrace_scope geprüft", () => {
    const s = read("setup.sh");
    expect(s).toMatch(/useradd --system [^\n]*--groups ghost ghost-web/);
    expect(s).toMatch(/install -d -m 2770 -o ghost -g ghost "\$GHOST_DIR\/deployments"/);
    expect(s).toMatch(/install -d -m 700 -o ghost -g ghost "\$GHOST_DIR\/keys"/);
    expect(s).toMatch(/chmod g\+rw,o-rwx/);
    expect(s).toMatch(/kernel\.yama\.ptrace_scope/);
  });
  it("Migrationsskript: sichert Units, prüft nach dem Neustart, nimmt bei Fehler zurück", () => {
    const m = read("migration-a20c.sh");
    expect(m).toMatch(/pruefe_root_eigen "\$SCRIPT_DIR"/);
    expect(m).toMatch(/ghost-web/);
    expect(m).toMatch(/zuruecknehmen/);
    expect(m.indexOf("cp -a /etc/systemd/system/ghost-web.service")).toBeLessThan(m.indexOf("systemctl restart ghost-web"));
    expect(m).not.toMatch(/keys\/[a-z]/); // fasst keine Schlüsseldatei an
  });
});
