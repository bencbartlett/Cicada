import { describe, expect, it } from "vitest";
import type { CatalogNode } from "../protocol/messages";
import { ribbonTabs } from "./ribbonTabs";

function node(name: string, category: string, title = name): CatalogNode {
  return {
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
  };
}

describe("ribbonTabs", () => {
  it("orders tabs by the docs/08 category order and nodes by title", () => {
    const tabs = ribbonTabs([
      node("box", "Surface & solid", "Box"),
      node("add", "Maths & logic", "Add"),
      node("slider", "Params & input", "Number Slider"),
      node("abs", "Maths & logic", "Absolute"),
      node("my_script", "Script"),
      node("odd", "Zeta category"),
    ]);
    expect(tabs.map((t) => t.label)).toEqual(["Params", "Maths", "Surface", "Project", "Zeta category"]);
    expect(tabs[1]!.nodes.map((n) => n.name)).toEqual(["abs", "add"]);
  });
  it("is empty for an empty catalog", () => {
    expect(ribbonTabs([])).toEqual([]);
  });
});
