// Audit 12, A12-11: Nachrichtenfilter. Dieselben Fälle wie ghostctl
// (protocol/tests/payload_tests.rs, Datei protocol/tests/data/nachrichtenfilter.json).
// Vor der Behebung ließen Seite und Server u. a. U+2028, Tag-Zeichen,
// Hangul-Füller und Variation Selectors durch.
import { beforeEach, describe, expect, it } from "vitest";
import raw from "../../../protocol/tests/data/nachrichtenfilter.json?raw";
import { checkMessage, hasBadMessageChar, ValidationError } from "../../server/actions";
import { hasBadChar, messageProblem, sendableMessage } from "./abo";
import { setLangGlobal } from "./i18n";

const data = JSON.parse(raw) as { ranges: string[]; cases: { name: string; text: string; ok: boolean }[] };
const ranges = data.ranges.map((r) => r.split("-").map((x) => parseInt(x, 16)) as [number, number]);
const listed = (u: number) => ranges.some(([a, b]) => u >= a && u <= b);

beforeEach(() => setLangGlobal("de"));

describe("A12-11: gleiche Fälle wie ghostctl", () => {
  it.each(data.cases.map((c) => [c.name, c] as const))("%s", (_name, c) => {
    expect(messageProblem(c.text) === null).toBe(c.ok);
    if (c.ok) expect(checkMessage(c.text)).toBe(c.text.trim());
    else expect(() => checkMessage(c.text)).toThrow(ValidationError);
  });

  it("Fehlertext nennt Steuer- und unsichtbare Zeichen", () => {
    expect(messageProblem("a\u2028b")).toBe("Nachricht: nur eine Zeile normaler Text, ohne Steuerzeichen und unsichtbare Zeichen.");
    expect(() => checkMessage("Miete\u{E0041}")).toThrow(/unsichtbare Zeichen/);
  });
});

describe("A12-11: jeder Codepunkt wie in der gemeinsamen Liste", () => {
  it("Seite und Server lehnen genau die gelisteten Codepunkte ab", () => {
    // Ebenen 0 und 1 und die Tag-/Selector-Ebene vollständig, sonst jede
    // Bereichsgrenze und eine Stichprobe (alle Codepunkte prüft ghostctl)
    const points = new Set<number>();
    for (let u = 0; u <= 0x1ffff; u++) points.add(u);
    for (let u = 0xe0000; u <= 0xe1000; u++) points.add(u);
    for (const [a, b] of ranges) for (const u of [a - 1, a, b, b + 1]) if (u >= 0 && u <= 0x10ffff) points.add(u);
    for (let u = 0; u <= 0x10ffff; u += 251) points.add(u);
    const wrong: string[] = [];
    for (const u of points) {
      if (u >= 0xd800 && u <= 0xdfff) continue;
      const t = `a${String.fromCodePoint(u)}b`;
      if (hasBadChar(t) !== listed(u) || hasBadMessageChar(t) !== listed(u)) wrong.push(u.toString(16));
    }
    expect(wrong).toEqual([]);
  });

  it("die Liste ist genau Cc, Cf, Co, Zl, Zp, Default_Ignorable_Code_Point und Nichtzeichen dieser Unicode-Version", () => {
    const re = /[\p{Cc}\p{Cf}\p{Co}\p{Zl}\p{Zp}\p{DI}\p{NChar}]/u;
    const diff: string[] = [];
    for (let u = 0; u <= 0x10ffff; u++) {
      if (u >= 0xd800 && u <= 0xdfff) continue;
      if (re.test(String.fromCodePoint(u)) !== listed(u)) diff.push(u.toString(16));
    }
    expect(diff).toEqual([]);
  });

  it("einzelne Surrogate (gibt es nur in JavaScript-Text) werden abgelehnt", () => {
    expect(hasBadChar("a\ud800b")).toBe(true);
    expect(hasBadMessageChar("a\udc00b")).toBe(true);
    expect(hasBadChar("a\u{1F3E0}b")).toBe(false);
  });
});

// Nachprüfung zu A12-11: ältere Daueraufträge mit inzwischen verbotenen Zeichen
// scheiterten bei jeder Ausführung; ghostctl sendet sie jetzt bereinigt
// (abo::sendable_message), die Seite zeigt, was gesendet wird
describe("Nachprüfung A12-11: gespeicherte Nachricht nach heutigem Filter (wie ghostctl)", () => {
  it.each(data.cases.map((c) => [c.name, c] as const))("%s", (_name, c) => {
    const s = sendableMessage(c.text);
    expect(messageProblem(s)).toBeNull();
    if (c.ok) expect(s).toBe(c.text.trim());
  });
  it("dieselben Beispiele wie in payload_tests.rs", () => {
    const pairs: [string, string][] = [
      ["Miete\u2028Mai", "Miete Mai"],
      ["Platz 1\ufe0f\u20e3", "Platz 1\u20e3"],
      ["Danke \u2764\ufe0f", "Danke \u2764\ufe0f"],
      ["\u2764\u200b\ufe0f", "\u2764\ufe0f"],
      ["a\ue000b\u{e0041}", "ab"],
      ["\ue000", ""],
    ];
    for (const [old, now] of pairs) expect(sendableMessage(old)).toBe(now);
    // Leerraum wie Rust (char::is_whitespace), nicht wie \s in JavaScript
    expect(sendableMessage("a\u0085b")).toBe("a b");
    expect(sendableMessage("a\ufeffb")).toBe("ab");
    expect(sendableMessage("a\ud800b"), "einzelnes Surrogat").toBe("ab");
  });
});
