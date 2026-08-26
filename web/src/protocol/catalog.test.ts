/**
 * The catalog mirror against the real bytes: `docs/generated/catalog.json`
 * is rendered by the same `catalog.rs` that serves `GET /api/catalog`
 * (CI keeps it fresh), so reading it here pins `CatalogNode` / `CatalogPort`
 * to what the server actually writes — the format-2 fields search-to-place
 * and the port tooltips rely on (`gh`, `examples`, per-port `doc`) and the
 * format-3 ones the menu bar reads (`sub` per node, the `subgroups` table).
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Catalog } from "./messages";

const here = dirname(fileURLToPath(import.meta.url));
const committed = resolve(here, "../../../docs/generated/catalog.json");
const catalog = JSON.parse(readFileSync(committed, "utf8")) as Catalog;

describe("CatalogNode mirrors docs/generated/catalog.json (format 3)", () => {
  it("is format 3 with a non-empty node list", () => {
    expect(catalog.format).toBe(3);
    expect(catalog.nodes.length).toBeGreaterThan(50);
  });

  it("carries the sub-group table in menu order — one row per category, Title Case names, no row empty", () => {
    expect(Array.isArray(catalog.subgroups)).toBe(true);
    expect(catalog.subgroups.length).toBeGreaterThanOrEqual(12);
    expect(catalog.subgroups[0]).toEqual({ category: "Params & input", subgroups: ["Input", "Time"] });
    const categories = catalog.subgroups.map((row) => row.category);
    expect(new Set(categories).size).toBe(categories.length);
    expect(categories).toContain("Script");
    for (const row of catalog.subgroups) {
      expect(row.subgroups.length, row.category).toBeGreaterThan(0);
      for (const sub of row.subgroups) expect(sub, `${row.category}/${sub}`).toMatch(/^[A-Z][a-z]+( [A-Z][a-z]+)?$/);
    }
  });

  it("gives every node a sub-group that is a column of its category, and fills every column a shipped category lists", () => {
    const columns = new Map(catalog.subgroups.map((row) => [row.category, row.subgroups]));
    const filled = new Set<string>();
    for (const node of catalog.nodes) {
      expect(typeof node.sub, `${node.name}.sub`).toBe("string");
      expect(columns.get(node.category), `${node.name}: ${node.category}/${node.sub}`).toContain(node.sub);
      filled.add(`${node.category}/${node.sub}`);
    }
    for (const row of catalog.subgroups) {
      if (row.category === "Script") continue; // the project's scripts fill it, not the stdlib
      for (const sub of row.subgroups) expect(filled.has(`${row.category}/${sub}`), `${row.category}/${sub} has a node`).toBe(true);
    }
    // The menu's canonical probes.
    const subOf = (name: string) => catalog.nodes.find((n) => n.name === name)?.sub;
    expect(subOf("slider")).toBe("Input");
    expect(subOf("add")).toBe("Operators");
    expect(subOf("sin")).toBe("Trig");
    expect(subOf("move")).toBe("Euclidean");
    expect(subOf("solid_union")).toBe("Boolean");
  });

  it("always carries gh (string or null) and examples (strings) — never absent", () => {
    for (const node of catalog.nodes) {
      expect(Object.hasOwn(node, "gh"), `${node.name}.gh`).toBe(true);
      expect(node.gh === null || typeof node.gh === "string", `${node.name}.gh`).toBe(true);
      expect(Array.isArray(node.examples), `${node.name}.examples`).toBe(true);
      for (const example of node.examples) expect(typeof example).toBe("string");
    }
    // Both branches exist in the stdlib: GH replacements and Cicada-only nodes.
    expect(catalog.nodes.some((n) => typeof n.gh === "string")).toBe(true);
    expect(catalog.nodes.some((n) => n.gh === null)).toBe(true);
    // The migrant's canonical probes resolve to the nodes that replace them.
    const byGh = (gh: string) => catalog.nodes.filter((n) => n.gh === gh).map((n) => n.name);
    expect(byGh("Series")).toEqual(["series"]);
    expect(byGh("Move")).toEqual(["move"]);
    expect(byGh("Merge")).toEqual(["concat"]);
  });

  it("documents every port — a bare `out` carries the node's # Returns line", () => {
    for (const node of catalog.nodes) {
      for (const port of [...node.inputs, ...node.outputs]) {
        expect(typeof port.doc, `${node.name}.${port.name}.doc`).toBe("string");
        expect(port.doc, `${node.name}.${port.name}.doc`).not.toBe("");
        expect(port.doc, `${node.name}.${port.name}.doc is one line`).not.toMatch(/\n/);
      }
    }
    const sphere = catalog.nodes.find((n) => n.name === "sphere");
    expect(sphere?.outputs.map((o) => o.name)).toEqual(["out"]);
    expect(sphere?.outputs[0]?.doc).toMatch(/sphere/i);
  });

  it("keeps the structured port fields the palette reads", () => {
    for (const node of catalog.nodes) {
      for (const port of [...node.inputs, ...node.outputs]) {
        expect(typeof port.type).toBe("string");
        expect(typeof port.base).toBe("string");
        expect(Number.isInteger(port.list_depth)).toBe(true);
        expect(typeof port.optional).toBe("boolean");
        if (port.dimension !== undefined) expect(["length", "angle"]).toContain(port.dimension);
      }
    }
  });

  it("marks exactly the two transport-driven ports — cycle.frame (frame) and clock.t (time), inputs only, each with a default", () => {
    const driven: string[] = [];
    for (const node of catalog.nodes) {
      for (const port of node.outputs) {
        expect(port.transport_driven, `${node.name}.${port.name} is an output`).toBeUndefined();
      }
      for (const port of node.inputs) {
        if (port.transport_driven === undefined) continue;
        expect(["frame", "time"], `${node.name}.${port.name}`).toContain(port.transport_driven);
        // The macro refuses a transport-driven port without a default: the
        // default IS the headless value (frame 0, t 0).
        expect(port.default, `${node.name}.${port.name} has a headless default`).toBeDefined();
        driven.push(`${node.name}.${port.name}=${port.transport_driven}`);
      }
    }
    expect(driven.sort()).toEqual(["clock.t=time", "cycle.frame=frame"]);
  });
});
