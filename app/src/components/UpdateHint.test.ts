import { describe, expect, it } from "vitest";
import { mainScript } from "./UpdateHint";

describe("mainScript", () => {
  it("liest das gebaute Haupt-Skript aus index.html", () => {
    expect(mainScript('<script type="module" crossorigin src="/assets/main-CMFmnNC5.js"></script>')).toBe("/assets/main-CMFmnNC5.js");
    expect(mainScript("<html></html>")).toBeNull();
  });
});
