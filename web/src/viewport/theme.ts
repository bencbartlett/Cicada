/**
 * Sample the theme tokens (`web/src/styles.css`) into three colors. A probe
 * element carrying `data-theme` reads the wanted theme's custom properties
 * directly, so the viewport recolors in the same tick the setting changes
 * (before React has re-stamped `<html data-theme>`).
 */
import type { ThemeColors } from "./scene";
import { cssColor } from "./materials";

const FALLBACK: Record<"dark" | "light", Record<string, string>> = {
  dark: {
    "--bg": "#15171b",
    "--accent": "#6cb4ff",
    "--grid-strong": "#2a2f37",
    "--border-strong": "#4a515e",
    "--kind-curve": "#7ee081",
    "--kind-point": "#ff9e6d",
  },
  light: {
    "--bg": "#f4f5f7",
    "--accent": "#1f6fd0",
    "--grid-strong": "#dde1e8",
    "--border-strong": "#b7bec9",
    "--fg": "#1c1f24",
    "--kind-curve": "#7ee081",
    "--kind-point": "#ff9e6d",
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
  };
  probe.remove();
  return colors;
}
