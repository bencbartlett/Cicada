/**
 * The catalog mirror against the real bytes: `docs/generated/catalog.json`
 * is rendered by the same `catalog.rs` that serves `GET /api/catalog`
 * (CI keeps it fresh), so reading it here pins `CatalogNode` / `CatalogPort`
 * to what the server actually writes — the format-2 fields search-to-place
 * and the port tooltips rely on (`gh`, `examples`, per-port `doc`).
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Catalog } from "./messages";

const here = dirname(fileURLToPath(import.meta.url));
const committed = resolve(here, "../../../docs/generated/catalog.json");
const catalog = JSON.parse(readFileSync(committed, "utf8")) as Catalog;

describe("CatalogNode mirrors docs/generated/catalog.json (format 2)", () => {
  it("is format 2 with a non-empty node list", () => {
    expect(catalog.format).toBe(2);
    expect(catalog.nodes.length).toBeGreaterThan(50);
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
});
