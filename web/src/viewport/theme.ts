/**
 * Sample the theme tokens (`web/src/styles.css`) into the viewport's colors.
 * A probe element carrying `data-theme` reads the wanted theme's custom
 * properties directly, so the viewport recolors in the same tick the
 * setting changes (before React has re-stamped `<html data-theme>`).
 */
import type { ThemeColors } from "./scene";
import { cssColor } from "./materials";

/**
 * The tokens as `styles.css` spells them — the answer when a property is
 * missing from the computed style (a foreign stylesheet, a test DOM).
 * `theme.test.ts` holds this table to the stylesheet.
 */
export const FALLBACK: Record<"dark" | "light", Record<string, string>> = {
  dark: {
    "--bg": "#15171b",
    "--accent": "#6cb4ff",
    "--grid-strong": "#202329",
    "--border-strong": "#4a515e",
    "--kind-curve": "#7ee081",
    "--kind-point": "#ff9e6d",
    "--axis-x": "#ff6b6b",
    "--axis-y": "#6bd36b",
    "--axis-z": "#5c9dff",
  },
  light: {
    "--bg": "#f4f5f7",
    "--accent": "#1f6fd0",
    "--grid-strong": "#e9ebf0",
    "--border-strong": "#b7bec9",
    "--fg": "#1c1f24",
    "--kind-curve": "#7ee081",
    "--kind-point": "#ff9e6d",
    "--axis-x": "#d12b2b",
    "--axis-y": "#1f8f3a",
    "--axis-z": "#1f5fd0",
  },
};

export function sampleTheme(theme: "dark" | "light"): ThemeColors {
  const probe = document.createElement("div");
  probe.dataset.theme = theme;
  probe.style.display = "none";
  document.body.appendChild(probe);
  const style = getComputedStyle(probe);
  const fallback = FALLBACK[theme];
  const read = (name: string) =>
    cssColor(style.getPropertyValue(name), fallback[name] ?? "#808080");
  const colors: ThemeColors = {
    theme,
    background: read("--bg"),
    accent: read("--accent"),
    grid: read("--grid-strong"),
    gridStrong: read("--border-strong"),
    // Edges are ink on the (mid-light) surfaces, so they are dark in both
    // themes: the dark theme's page background, the light theme's text ink.
    edge: theme === "dark" ? read("--bg") : read("--fg"),
    curve: read("--kind-curve"),
    point: read("--kind-point"),
    // The axes' hues (docs/16 §Viewport conventions): the ground-origin
    // triad and the gimbal share them.
    axisX: read("--axis-x"),
    axisY: read("--axis-y"),
    axisZ: read("--axis-z"),
  };
  probe.remove();
  return colors;
}
