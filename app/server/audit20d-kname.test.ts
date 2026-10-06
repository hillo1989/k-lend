// Audit 20 A20d-10 (Server-Seite): looksLikeName lehnt dieselben
// Tippfragmente ab wie isKName in der Seite. A20d-11: Umbruch langer Adressen
// in .tx-list.
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { looksLikeName } from "./dotkNames.ts";

describe("A20d-10: Server löst keine Adressanfänge und Kürzel auf", () => {
  it("wie isKName in der Seite", () => {
    for (const t of ["k", "ka", "kas", "kasp", "kaspa", "kaspat", "kaspatest", "KASPA", " kaspa ", "ab", "a"]) expect(looksLikeName(t), t).toBe(false);
    for (const t of ["alice.k", "bob", "a.k", "kaspalover", "kaspa.k", "kaspafan.k"]) expect(looksLikeName(t), t).toBe(true);
  });
});

describe("A20d-11: lange Adressen brechen in der Empfängerliste um", () => {
  it(".tx-list li hat overflow-wrap: anywhere", () => {
    const css = readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "src", "styles.css"), "utf8");
    expect(css).toMatch(/\.tx-list li\s*\{[^}]*overflow-wrap:\s*anywhere/);
  });
});
