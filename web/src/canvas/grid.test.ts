import { describe, expect, it } from "vitest";
import type { Catalog, CatalogNode } from "../protocol/messages";
import {
  cellToPx,
  filterCatalog,
  firstLine,
  isRefinement,
  lodTier,
  paramValueText,
  pxToCell,
  sliderStep,
  snapToStep,
  statusBadge,
  stepDecimals,
  wireStrokeWidth,
} from "./grid";

describe("grid maths", () => {
  it("maps cells to pixels and back", () => {
    expect(cellToPx([3, -2], 24)).toEqual({ x: 72, y: -48 });
    expect(pxToCell(72, -48, 24)).toEqual([3, -2]);
    expect(pxToCell(83, 37, 24)).toEqual([3, 2]);
    expect(pxToCell(84, 36, 24)).toEqual([4, 2]);
  });
});

describe("paramValueText", () => {
  it("keeps a decimal point on Number-typed values", () => {
    expect(paramValueText("slider", 3)).toBe("3.0");
    expect(paramValueText("number", 3)).toBe("3.0");
    expect(paramValueText("slider", 2.75)).toBe("2.75");
    expect(paramValueText("slider", 0.1 + 0.2)).toBe("0.30000000000000004");
  });
  it("writes integers bare", () => {
    expect(paramValueText("integer", 24)).toBe("24");
    expect(() => paramValueText("integer", 2.5)).toThrow();
  });
  it("capitalizes dialect booleans", () => {
    expect(paramValueText("toggle", true)).toBe("True");
    expect(paramValueText("boolean", false)).toBe("False");
  });
  it("quotes text JSON-style", () => {
    expect(paramValueText("text", 'a "b"')).toBe('"a \\"b\\""');
  });
  it("refuses non-finite numbers loudly", () => {
    expect(() => paramValueText("slider", Number.NaN)).toThrow();
    expect(() => paramValueText("list", "x")).toThrow();
  });
});

describe("sliderStep", () => {
  it("uses the given step or a fine power-of-ten one", () => {
    expect(sliderStep(0, 10, 0.5)).toBe(0.5);
    expect(sliderStep(0.5, 5, 0)).toBe(0.001);
    expect(sliderStep(0, 30, 0)).toBe(0.01);
    expect(sliderStep(0, 0.5, undefined)).toBe(0.0001);
    expect(sliderStep(1, 1, undefined)).toBe(0.001);
  });
  it("snaps onto the lattice without float noise", () => {
    expect(snapToStep(3.5004, 0.5, 0.001)).toBe(3.5);
    expect(snapToStep(0.5 + 3000 * 0.001, 0.5, 0.001)).toBe(3.5);
    expect(snapToStep(2.62, 0, 0.25)).toBe(2.5);
    expect(snapToStep(7.3, 0, 1)).toBe(7);
    expect(stepDecimals(0.25)).toBe(2);
    expect(stepDecimals(1)).toBe(0);
    expect(stepDecimals(1e-7)).toBe(7);
  });
});

describe("lodTier", () => {
  it("has four monotone tiers", () => {
    expect(lodTier(0.2)).toBe("far");
    expect(lodTier(0.5)).toBe("mid");
    expect(lodTier(1)).toBe("near");
    expect(lodTier(2)).toBe("closest");
  });
});

const node = (name: string, title: string, category = "Maths & logic"): CatalogNode => ({
  name,
  title,
  description: "",
  category,
  tier: "S",
  version: 1,
  pure: true,
  uses_tolerance: false,
  inputs: [],
  outputs: [],
});

const catalog: Catalog = {
  format: 1,
  nodes: [node("sphere", "Sphere"), node("box", "Box"), node("mesh_sphere", "Mesh sphere"), node("add", "Add")],
};

describe("filterCatalog", () => {
  it("matches by substring on name or title, prefix first", () => {
    const hits = filterCatalog(catalog, "sph", null).map((h) => h.node.name);
    expect(hits).toEqual(["sphere", "mesh_sphere"]);
  });
  it("lists everything on an empty query", () => {
    expect(filterCatalog(catalog, "", null)).toHaveLength(4);
    expect(filterCatalog(null, "", null)).toEqual([]);
  });
  it("restricts to probe-accepting funcs and carries the ports", () => {
    const hits = filterCatalog(catalog, "", [
      { func: "box", ports: [["x", "ok"]] },
      { func: "sphere", ports: [] },
      { func: "add", ports: [["a", "lift"], ["b", "ok"]] },
    ]);
    expect(hits.map((h) => h.node.name)).toEqual(["add", "box"]);
    expect(hits[0]?.ports).toEqual([["a", "lift"], ["b", "ok"]]);
  });
});

describe("statusBadge", () => {
  it("speaks the docs/16 vocabulary", () => {
    expect(statusBadge(undefined, 0).label).toBe("idle");
    expect(statusBadge({ state: "done", generation: 1, nanos: 1_237_600 }, 0).label).toBe("1.2ms");
    expect(statusBadge({ state: "done", generation: 1, nanos: 4_800 }, 0).label).toBe("5µs");
    expect(statusBadge({ state: "running", generation: 1, elements_done: 3, elements: 12 }, 0).label).toBe(
      "25%",
    );
    const red = statusBadge({ state: "red", generation: 1, message: "boom" }, 2);
    expect(red.label).toBe("● 2");
    expect(red.title).toBe("boom");
    expect(statusBadge({ state: "blocked", generation: 1 }, 0).className).toBe("state-blocked");
  });
});

describe("misc", () => {
  it("first line truncates", () => {
    expect(firstLine("a\nb")).toBe("a");
    expect(firstLine("x".repeat(80))).toHaveLength(58);
  });
  it("wire widths grow with depth", () => {
    expect(wireStrokeWidth(0)).toBeLessThan(wireStrokeWidth(1));
    expect(wireStrokeWidth(1)).toBeLessThan(wireStrokeWidth(2));
  });
  it("detects refinements", () => {
    expect(isRefinement("Watertight<Mesh>")).toBe(true);
    expect(isRefinement("Mesh")).toBe(false);
  });
});
