/**
 * The git badges and markers are colored by the THEME's semantic tokens
 * (docs/16 §Theme: "these tokens are the contract"; docs/17 item 2: "color-
 * coded by the existing theme tokens") — never by a literal rgba, which
 * would be one theme's tint under the other theme's glyph (the `added`
 * green was). Read the stylesheets as text: every `.cn-git-*` /
 * `.git-mark-*` / `.git-status-*` rule's colors are `var(--…)`, and every
 * token they name is defined in BOTH theme blocks of `styles.css`.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel: string) => readFileSync(join(src, rel), "utf8");

/** `{selector: declarations}` for every rule whose selector list matches `pick`. */
function rules(css: string, pick: RegExp): Map<string, string> {
  const out = new Map<string, string>();
  const re = /([^{}]+)\{([^{}]*)\}/g;
  for (const match of css.matchAll(re)) {
    const selector = match[1]!.trim();
    if (pick.test(selector)) out.set(selector, match[2]!);
  }
  return out;
}

/** The custom properties defined in one theme block (`:root, [data-theme="dark"]` or `[data-theme="light"]`). */
function tokensOf(css: string, block: RegExp): Set<string> {
  const match = block.exec(css);
  if (match === null) throw new Error(`theme block not found: ${block}`);
  return new Set(Array.from(match[1]!.matchAll(/--([\w-]+)\s*:/g), (m) => m[1]!));
}

describe("git badge / marker colors come from the theme tokens", () => {
  const styles = read("styles.css");
  const dark = tokensOf(styles, /:root,\s*\[data-theme="dark"\]\s*\{([^}]*)\}/);
  const light = tokensOf(styles, /\[data-theme="light"\]\s*\{([^}]*)\}/);

  const GIT_RULES: [string, RegExp][] = [
    ["canvas/canvas.css", /^\.cn-git-(added|modified|renamed|removed)$/],
    ["panels/panels.css", /\.git-(mark|status)-/],
  ];
  for (const [file, pick] of GIT_RULES) {
    it(`${file}: every color/background is a var(--token) defined in both themes`, () => {
      const found = rules(read(file), pick);
      expect(found.size, `no git rules matched ${pick} in ${file}`).toBeGreaterThan(0);
      for (const [selector, body] of found) {
        for (const decl of body.split(";")) {
          const [prop, value] = decl.split(":").map((s) => s.trim());
          if (prop !== "color" && prop !== "background" && prop !== "background-color") continue;
          expect(value, `${selector} { ${prop} }`).toMatch(/^var\(--[\w-]+\)$/);
          const token = /^var\(--([\w-]+)\)$/.exec(value!)![1]!;
          expect(dark.has(token), `--${token} in the dark theme`).toBe(true);
          expect(light.has(token), `--${token} in the light theme`).toBe(true);
        }
      }
    });
  }

  it("the four change kinds each have a foreground AND a background token (`--ok-bg` included)", () => {
    for (const token of ["ok", "ok-bg", "warn", "warn-bg", "accent", "accent-bg", "error", "error-bg"]) {
      expect(dark.has(token), `--${token} (dark)`).toBe(true);
      expect(light.has(token), `--${token} (light)`).toBe(true);
    }
  });

  it("no stylesheet hard-codes the dark theme's `--ok` green as a tint", () => {
    for (const file of ["styles.css", "canvas/canvas.css", "panels/panels.css"]) {
      const css = read(file);
      // The token definitions themselves are allowed to spell the rgba once.
      const outsideTokens = css.replace(/--ok-bg:\s*rgba\([^)]*\);/g, "");
      expect(outsideTokens, file).not.toMatch(/rgba\(\s*122\s*,\s*211\s*,\s*138/);
    }
  });
});
