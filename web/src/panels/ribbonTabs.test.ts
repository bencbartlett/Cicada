/**
 * The menu bar's model (docs/17 wave 5 M1): tabs in docs/08 order, each
 * with its columns — the category's sub-groups in the catalog's table
 * order, by title inside, empty ones absent, unlisted ones trailing under
 * their own name. The committed `catalog.json` is the real table.
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Catalog, CatalogNode, CatalogSubgroups } from "../protocol/messages";
import { ribbonTabs } from "./ribbonTabs";

function node(name: string, category: string, title = name, sub = "Util"): CatalogNode {
  return {
    name,
    title,
    description: "",
    category,
    tier: "S",
    version: 1,
    pure: true,
    uses_tolerance: false,
    gh: null,
    sub,
    examples: [],
    inputs: [],
    outputs: [],
  };
}

const TABLE: CatalogSubgroups[] = [
  { category: "Params & input", subgroups: ["Input", "Time"] },
  { category: "Maths & logic", subgroups: ["Operators", "Trig", "Util", "Domain", "Logic"] },
  { category: "Script", subgroups: ["Script"] },
];

describe("ribbonTabs", () => {
  it("orders tabs by the docs/08 category order and nodes by title", () => {
    const tabs = ribbonTabs(
      [
        node("box", "Surface & solid", "Box"),
        node("add", "Maths & logic", "Add"),
        node("slider", "Params & input", "Number Slider"),
        node("abs", "Maths & logic", "Absolute"),
        node("my_script", "Script"),
        node("odd", "Zeta category"),
      ],
      TABLE,
    );
    expect(tabs.map((t) => t.label)).toEqual(["Params", "Maths", "Surface", "Project", "Zeta category"]);
    expect(tabs[1]!.nodes.map((n) => n.name)).toEqual(["abs", "add"]);
  });

  it("lays a tab's columns out in the table's order, skipping sub-groups no node fills, nodes by title inside", () => {
    const tabs = ribbonTabs(
      [
        node("sin", "Maths & logic", "Sine", "Trig"),
        node("add", "Maths & logic", "Add", "Operators"),
        node("and", "Maths & logic", "And", "Logic"),
        node("abs", "Maths & logic", "Absolute", "Operators"),
        // no Util, no Domain: no column for them
      ],
      TABLE,
    );
    const maths = tabs[0]!;
    expect(maths.columns.map((c) => c.sub)).toEqual(["Operators", "Trig", "Logic"]);
    expect(maths.columns[0]!.nodes.map((n) => n.name)).toEqual(["abs", "add"]);
    // The count is every node of the category, columns or not.
    expect(maths.nodes).toHaveLength(4);
  });

  it("trails a sub-group the table does not list for the category, under its own name — shown, never folded into a listed column", () => {
    const tabs = ribbonTabs(
      [
        node("add", "Maths & logic", "Add", "Operators"),
        // A project script node that declared a stdlib category keeps its `Script` sub-group.
        node("my_fn", "Maths & logic", "My Fn", "Script"),
        node("zz", "Maths & logic", "Zz", "Bespoke"),
      ],
      TABLE,
    );
    expect(tabs[0]!.columns.map((c) => c.sub)).toEqual(["Operators", "Bespoke", "Script"]);
    expect(tabs[0]!.columns[2]!.nodes.map((n) => n.name)).toEqual(["my_fn"]);
  });

  it("a category without a table row gets its columns alphabetically — no table, no order to follow", () => {
    const tabs = ribbonTabs([node("b", "Zeta", "B", "Two"), node("a", "Zeta", "A", "One")], TABLE);
    expect(tabs[0]!.columns.map((c) => c.sub)).toEqual(["One", "Two"]);
    // Without any table at all (the argument's default), the same.
    expect(ribbonTabs([node("b", "Zeta", "B", "Two"), node("a", "Zeta", "A", "One")])[0]!.columns.map((c) => c.sub)).toEqual([
      "One",
      "Two",
    ]);
  });

  it("is empty for an empty catalog", () => {
    expect(ribbonTabs([], TABLE)).toEqual([]);
  });

  it("over the committed catalog: every tab's columns are exactly its table row's filled sub-groups, in row order; Maths has five", () => {
    const here = dirname(fileURLToPath(import.meta.url));
    const catalog = JSON.parse(readFileSync(resolve(here, "../../../docs/generated/catalog.json"), "utf8")) as Catalog;
    const tabs = ribbonTabs(catalog.nodes, catalog.subgroups);
    expect(tabs.length).toBeGreaterThanOrEqual(11);
    const rows = new Map(catalog.subgroups.map((row) => [row.category, row.subgroups]));
    for (const tab of tabs) {
      const row = rows.get(tab.category);
      expect(row, tab.category).toBeDefined();
      const filled = new Set(tab.nodes.map((n) => n.sub));
      expect(tab.columns.map((c) => c.sub), tab.category).toEqual(row!.filter((sub) => filled.has(sub)));
      expect(tab.columns.flatMap((c) => c.nodes).length, `${tab.category}: every node is in one column`).toBe(tab.nodes.length);
    }
    const maths = tabs.find((t) => t.label === "Maths")!;
    expect(maths.columns.map((c) => c.sub)).toEqual(["Operators", "Trig", "Util", "Domain", "Logic"]);
    expect(maths.columns[0]!.nodes.map((n) => n.name)).toContain("add");
  });
});
