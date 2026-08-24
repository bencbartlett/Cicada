/**
 * The theme tokens (`web/src/styles.css`) as the contract they are: the
 * grid tokens are pinned to the values docs/16 §Theme records — halfway
 * from their previous values to `--bg` (U4, 2026-08-24: "grid lines ~50 %
 * less visible, closer to the background") — and the viewport's fallback
 * table agrees with the stylesheet for every token it carries.
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { FALLBACK } from "./theme";

const css = readFileSync(new URL("../styles.css", import.meta.url), "utf8");

/** The `--name: value;` declarations of one selector block. */
function declarationsOf(selector: string): Record<string, string> {
  const start = css.indexOf(selector);
  if (start < 0) throw new Error(`no ${JSON.stringify(selector)} block in styles.css`);
  const end = css.indexOf("\n}", start);
  const block = css.slice(start, end);
  const tokens: Record<string, string> = {};
  for (const match of block.matchAll(/(--[a-z0-9-]+):\s*([^;]+);/g)) tokens[match[1]!] = match[2]!.trim();
  return tokens;
}

/**
 * A theme's tokens as the cascade resolves them: `:root` declares every
 * token (the dark theme); `[data-theme="light"]` overrides the ones it
 * names and inherits the rest (the kind hues, the fonts).
 */
function tokensOf(theme: "dark" | "light"): Record<string, string> {
  const root = declarationsOf(':root,\n[data-theme="dark"] {');
  return theme === "dark" ? root : { ...root, ...declarationsOf('[data-theme="light"] {') };
}

function rgb(hex: string): [number, number, number] {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (m === null) throw new Error(`not a 6-digit hex color: ${hex}`);
  const n = parseInt(m[1]!, 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/** The values before B1 — the reference the "halfway" rule is measured against. */
const BEFORE = {
  dark: { "--grid": "#20242a", "--grid-strong": "#2a2f37" },
  light: { "--grid": "#e8ebf0", "--grid-strong": "#dde1e8" },
} as const;

/** What docs/16 §Theme records (2026-08-24). */
const DOCUMENTED = {
  dark: { "--grid": "#1b1e23", "--grid-strong": "#202329" },
  light: { "--grid": "#eef0f4", "--grid-strong": "#e9ebf0" },
} as const;

describe("grid tokens — halfway to the background, as docs/16 §Theme records", () => {
  for (const theme of ["dark", "light"] as const) {
    const tokens = tokensOf(theme);
    const bg = rgb(tokens["--bg"]!);
    for (const name of ["--grid", "--grid-strong"] as const) {
      it(`${theme} ${name} is the documented value`, () => {
        expect(tokens[name]).toBe(DOCUMENTED[theme][name]);
      });
      it(`${theme} ${name} lies halfway between its previous value and --bg, per channel`, () => {
        const before = rgb(BEFORE[theme][name]);
        const now = rgb(tokens[name]!);
        for (let c = 0; c < 3; c += 1) {
          const midpoint = (before[c]! + bg[c]!) / 2;
          // Rounded to a whole channel value: within half a step of the midpoint.
          expect(Math.abs(now[c]! - midpoint)).toBeLessThanOrEqual(0.5);
        }
      });
    }
    it(`${theme} --grid is fainter than --grid-strong, and both are distinct from --bg`, () => {
      const distance = (a: readonly number[], b: readonly number[]) =>
        Math.hypot(a[0]! - b[0]!, a[1]! - b[1]!, a[2]! - b[2]!);
      const grid = rgb(tokens["--grid"]!);
      const strong = rgb(tokens["--grid-strong"]!);
      expect(distance(grid, bg)).toBeGreaterThan(0);
      expect(distance(strong, bg)).toBeGreaterThan(distance(grid, bg));
    });
  }
});

describe("axis tokens — X red, Y green, Z blue in both themes", () => {
  for (const theme of ["dark", "light"] as const) {
    it(`${theme}: each axis hue is dominated by its own channel`, () => {
      const tokens = tokensOf(theme);
      const [xr, xg, xb] = rgb(tokens["--axis-x"]!);
      const [yr, yg, yb] = rgb(tokens["--axis-y"]!);
      const [zr, zg, zb] = rgb(tokens["--axis-z"]!);
      expect(xr).toBeGreaterThan(Math.max(xg, xb));
      expect(yg).toBeGreaterThan(Math.max(yr, yb));
      expect(zb).toBeGreaterThan(Math.max(zr, zg));
    });
  }
});

describe("the viewport's fallback table agrees with the stylesheet", () => {
  for (const theme of ["dark", "light"] as const) {
    it(`${theme}: every fallback token is the stylesheet's value`, () => {
      const tokens = tokensOf(theme);
      for (const [name, value] of Object.entries(FALLBACK[theme])) {
        expect(tokens[name], name).toBe(value);
      }
    });
  }
});
