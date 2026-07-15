// Unit tests for the pure, side-effect-free helpers in main.js. No DOM or Tauri
// bridge is touched here — these are the frontend equivalents of the Rust
// rule-engine unit tests: small, fast, exhaustive on edge cases.
import { describe, it, expect } from "vitest";
import { toCents, fmt, escapeHtml, closingClass, yearElapsedFraction } from "../main.js";

describe("toCents", () => {
  it("converts whole and fractional dollars to integer cents", () => {
    expect(toCents("50000")).toBe(5_000_000);
    expect(toCents("50.50")).toBe(5050);
    expect(toCents(0)).toBe(0);
    expect(toCents("1234.56")).toBe(123456);
  });

  it("returns 0 for non-numeric or empty input rather than NaN", () => {
    expect(toCents("abc")).toBe(0);
    expect(toCents(undefined)).toBe(0);
    expect(toCents(NaN)).toBe(0);
  });
});

describe("fmt", () => {
  it("formats cents as CAD currency", () => {
    // Assert on structure, not exact glyphs, to stay locale/ICU robust.
    const s = fmt(5_000_000);
    expect(s).toContain("50,000");
    expect(s).toContain("$");
    expect(fmt(0)).toContain("0.00");
  });

  it("formats negative amounts", () => {
    expect(fmt(-150_00)).toContain("150");
    expect(fmt(-150_00)).toMatch(/-|\(/); // minus sign or accounting parens
  });
});

describe("escapeHtml", () => {
  it("escapes the five HTML-significant characters", () => {
    expect(escapeHtml(`<script>alert("x")&'`)).toBe("&lt;script&gt;alert(&quot;x&quot;)&amp;&#39;");
  });

  it("coerces non-strings", () => {
    expect(escapeHtml(2024)).toBe("2024");
  });
});

describe("closingClass", () => {
  it("flags negative balances as a warning", () => {
    expect(closingClass(-1)).toBe("num warn");
    expect(closingClass(0)).toBe("num");
    expect(closingClass(100)).toBe("num");
  });
});

describe("yearElapsedFraction", () => {
  it("is ~0 at the start of the year and ~1 at the end", () => {
    expect(yearElapsedFraction(new Date(2024, 0, 1))).toBeCloseTo(0, 5);
    expect(yearElapsedFraction(new Date(2024, 11, 31))).toBeGreaterThan(0.99);
    expect(yearElapsedFraction(new Date(2024, 11, 31))).toBeLessThanOrEqual(1);
  });

  it("is ~0.5 at mid-year", () => {
    expect(yearElapsedFraction(new Date(2024, 6, 1))).toBeCloseTo(0.5, 2);
  });
});
