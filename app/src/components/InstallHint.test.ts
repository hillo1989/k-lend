import { describe, expect, it } from "vitest";
import { platformOf } from "./InstallHint";

describe("platformOf", () => {
  it("erkennt iPhone-Safari, aber nicht Chrome/Firefox auf iOS", () => {
    expect(platformOf("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1")).toBe("ios");
    expect(platformOf("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) CriOS/130.0 Mobile Safari/604.1")).toBe(null);
  });
  it("erkennt Android, Desktop nicht", () => {
    expect(platformOf("Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/130.0 Mobile Safari/537.36")).toBe("android");
    expect(platformOf("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/130.0 Safari/537.36")).toBe(null);
  });
});
