import { describe, expect, it } from "vitest";
import type { Catalog, CatalogNode } from "../protocol/messages";
import {
  catalogEntry,
  cellToPx,
  filterCatalog,
  firstLine,
  ghHint,
  isRefinement,
  lodTier,
  outputDoc,
  paramValueText,
  portTitle,
  pxToCell,
  searchRank,
  showsPortValues,
  sliderStep,
  snapToStep,
  statusBadge,
  stepDecimals,
  transportDrivenSignal,
  wireStrokeWidth,
  wireStyle,
  durationLabel,
  durationTitle,
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
  it("has three monotone tiers — and no in-between state (U18)", () => {
    expect(lodTier(0.2)).toBe("far");
    expect(lodTier(0.5)).toBe("near");
    expect(lodTier(1)).toBe("near");
    expect(lodTier(2)).toBe("closest");
  });
  it("flips at the docs/16 table's thresholds — 0.35 · 1.6, each inclusive upward", () => {
    expect(lodTier(0.3499)).toBe("far");
    expect(lodTier(0.35)).toBe("near");
    expect(lodTier(1.5999)).toBe("near");
    expect(lodTier(1.6)).toBe("closest");
  });
  it("shows the output value summaries on every tier that shows the face (U7, then U18)", () => {
    expect(showsPortValues("far")).toBe(false);
    expect(showsPortValues("near")).toBe(true);
    expect(showsPortValues("closest")).toBe(true);
    // The zoom that first shows them is the face's floor: title-only below it.
    expect(showsPortValues(lodTier(0.35))).toBe(true);
    expect(showsPortValues(lodTier(0.34))).toBe(false);
  });
});

const node = (
  name: string,
  title: string,
  gh: string | null = null,
  outputs: CatalogNode["outputs"] = [],
  category = "Maths & logic",
): CatalogNode => ({
  name,
  title,
  description: "",
  category,
  tier: "S",
  version: 1,
  pure: true,
  uses_tolerance: false,
  gh,
  examples: [],
  inputs: [],
  outputs,
});

const port = (name: string, doc?: string): CatalogNode["outputs"][number] => ({
  name,
  type: "Number",
  base: "Number",
  list_depth: 0,
  optional: false,
  ...(doc === undefined ? {} : { doc }),
});

// A slice of the real catalog's shape: dialect names, titles and the
// Grasshopper names the nodes replace (docs/generated/catalog.json).
const catalog: Catalog = {
  format: 2,
  nodes: [
    node("sphere", "Sphere", "Sphere", [port("out", "The watertight UV-sphere mesh.")]),
    node("box", "Box", "Domain Box"),
    node("mesh_sphere", "Mesh sphere", null),
    node("add", "Add", "Addition", [port("out", "The sum `a + b`.")]),
    node("mass_addition", "Mass Addition", "Mass Addition", [port("result", "The sum of the list."), port("partial")]),
    node("concat", "Concat", "Merge"),
    node("pick", "Pick", "Pick'n'Choose"),
    node("round", "Round", "Round"),
    node("floor", "Floor", "Round"),
    node("series", "Series", "Series"),
    node("ln", "Natural Logarithm", "Natural logarithm"),
    node("as_closed", "As Closed", null),
    {
      ...node("cycle", "Cycle", null, [port("out", "The loop position in `0..1`.")], "Params & input"),
      inputs: [
        { name: "period", type: "Number", base: "Number", list_depth: 0, optional: false, default: "4.0", doc: "Seconds per loop." },
        { name: "frames", type: "Integer", base: "Integer", list_depth: 0, optional: false, default: "120", doc: "Frames per loop." },
        {
          name: "frame",
          type: "Integer",
          base: "Integer",
          list_depth: 0,
          optional: false,
          default: "0",
          doc: "The current frame.",
          transport_driven: "frame",
        },
      ],
    },
  ],
};

describe("filterCatalog", () => {
  it("matches by substring on name or title, prefix first", () => {
    const hits = filterCatalog(catalog, "sph", null).map((h) => h.node.name);
    expect(hits).toEqual(["sphere", "mesh_sphere"]);
  });
  it("lists everything on an empty query", () => {
    expect(filterCatalog(catalog, "", null)).toHaveLength(catalog.nodes.length);
    expect(filterCatalog(null, "", null)).toEqual([]);
  });
  it("matches the Grasshopper name a migrant types, case-insensitively", () => {
    expect(filterCatalog(catalog, "Merge", null).map((h) => h.node.name)).toEqual(["concat"]);
    expect(filterCatalog(catalog, "pick'n'choose", null).map((h) => h.node.name)).toEqual(["pick"]);
    expect(filterCatalog(catalog, "domain", null).map((h) => h.node.name)).toEqual(["box"]);
    expect(filterCatalog(catalog, "Series", null)[0]?.node.name).toBe("series");
  });
  it("ranks an exact gh hit above the substring hits it also lights up", () => {
    // `Addition` is add's GH name and a substring of mass_addition's everything.
    expect(filterCatalog(catalog, "Addition", null).map((h) => h.node.name)).toEqual(["add", "mass_addition"]);
  });
  it("ranks name exact > gh exact > title exact > name prefix > title/gh prefix > substring", () => {
    const ranked: Catalog = {
      format: 2,
      nodes: [
        node("emerge", "Emergency", null), // substring
        node("merge_tree", "Merge", null), // title exact (and name prefix)
        node("concat", "Concat", "Merge"), // gh exact
        node("merge", "Join", null), // name exact
        node("mergers", "Mergers", null), // name prefix
        node("weave", "Merge Streams", null), // title prefix
        node("join", "Join", "Merge Faces"), // gh prefix
      ],
    };
    expect(filterCatalog(ranked, "merge", null).map((h) => h.node.name)).toEqual([
      "merge",
      "concat",
      "merge_tree",
      "mergers",
      "join",
      "weave",
      "emerge",
    ]);
    expect(searchRank(ranked.nodes[3]!, "merge")).toBe(0);
    expect(searchRank(ranked.nodes[2]!, "merge")).toBe(1);
    expect(searchRank(ranked.nodes[1]!, "merge")).toBe(2);
    expect(searchRank(ranked.nodes[0]!, "merge")).toBe(5);
    expect(searchRank(ranked.nodes[0]!, "zzz")).toBeNull();
  });
  it("ties go to the node matching on its own name (Round: round before floor)", () => {
    expect(filterCatalog(catalog, "Round", null).map((h) => h.node.name)).toEqual(["round", "floor"]);
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
  it("still matches gh under a probe filter", () => {
    const hits = filterCatalog(catalog, "merge", [{ func: "concat", ports: [["a", "ok"]] }]);
    expect(hits.map((h) => h.node.name)).toEqual(["concat"]);
  });
  it("carries the server's accepting ports WHOLE — the hidden-port rule is the server's, not re-decided here", () => {
    // A transport-driven port (`cycle.frame`) is never a wire target: the
    // server's `wire_verdict` blocks it, so the probe's catalog never lists
    // it (session test `a_transport_driven_port_is_never_a_wire_target_from_the_app`).
    // The client does not second-guess the answer (the protocol-change rule:
    // never compute wire compatibility client-side) — what the server offers
    // is what the search shows, and a func with no accepting port drops out.
    const offered = filterCatalog(catalog, "", [{ func: "cycle", ports: [["frames", "ok"]] }]);
    expect(offered.map((h) => [h.node.name, h.ports])).toEqual([["cycle", [["frames", "ok"]]]]);
    expect(filterCatalog(catalog, "", [{ func: "cycle", ports: [] }])).toEqual([]);
  });
});

describe("transportDrivenSignal — the hidden-port rule", () => {
  it("names the signal of a transport-driven input and nothing else", () => {
    expect(transportDrivenSignal(catalog, "cycle", "frame")).toBe("frame");
    expect(transportDrivenSignal(catalog, "cycle", "frames")).toBeUndefined();
    expect(transportDrivenSignal(catalog, "cycle", "period")).toBeUndefined();
    expect(transportDrivenSignal(catalog, "cycle", "out")).toBeUndefined();
    expect(transportDrivenSignal(catalog, "add", "frame")).toBeUndefined();
    // Unknown func (a script node), no func (a literal / expression), no catalog yet.
    expect(transportDrivenSignal(catalog, "my_script", "frame")).toBeUndefined();
    expect(transportDrivenSignal(catalog, undefined, "frame")).toBeUndefined();
    expect(transportDrivenSignal(null, "cycle", "frame")).toBeUndefined();
  });
  it("before the catalog arrives, the snapshot's driven entry for the port is the answer; once it is here the catalog decides alone", () => {
    const frameEntry = { node: "spin", port: "frame", signal: "frame" as const, loop: { frames: 120, period_ms: 4000 } };
    const timeEntry = { node: "tick", port: "t", signal: "time" as const };
    // No catalog yet (the HTTP fetch races the socket's snapshot): the port
    // the transport is feeding is driven — never an ordinary input, not
    // even on the first paint (review 2026-08-21).
    expect(transportDrivenSignal(null, "cycle", "frame", frameEntry)).toBe("frame");
    expect(transportDrivenSignal(null, "clock", "t", timeEntry)).toBe("time");
    expect(transportDrivenSignal(null, "cycle", "frames", undefined)).toBeUndefined();
    // The catalog is the authority once here: a port of a red `cycle` is
    // driven by nature (not in the driven set), and an entry cannot make an
    // ordinary port driven.
    expect(transportDrivenSignal(catalog, "cycle", "frame", undefined)).toBe("frame");
    expect(transportDrivenSignal(catalog, "cycle", "frames", frameEntry)).toBeUndefined();
    expect(transportDrivenSignal(catalog, "my_script", "frame", frameEntry)).toBeUndefined();
  });
});

describe("ghHint", () => {
  it("shows the GH name only when it tells the user something the title does not", () => {
    expect(ghHint(node("concat", "Concat", "Merge"))).toBe("Merge");
    expect(ghHint(node("box", "Box", "Domain Box"))).toBe("Domain Box");
    expect(ghHint(node("sphere", "Sphere", "Sphere"))).toBeNull();
    expect(ghHint(node("ln", "Natural Logarithm", "Natural logarithm"))).toBeNull();
    expect(ghHint(node("as_closed", "As Closed", null))).toBeNull();
  });
  it("degrades like searchRank on a node whose gh key is absent instead of throwing in the render", () => {
    // The server always writes `gh` (catalog.test.ts pins it); a foreign
    // catalog without the key must not take the search box down.
    const absent = node("concat", "Concat", "Merge");
    delete (absent as Partial<CatalogNode>).gh;
    expect("gh" in absent).toBe(false);
    expect(ghHint(absent)).toBeNull();
    expect(searchRank(absent, "concat")).toBe(0);
    expect(searchRank(absent, "merge")).toBeNull();
  });
});

describe("port docs from the catalog", () => {
  it("looks up a func's entry and its output doc (a bare out's # Returns line)", () => {
    expect(catalogEntry(catalog, "add")?.title).toBe("Add");
    expect(catalogEntry(catalog, "nope")).toBeUndefined();
    expect(catalogEntry(catalog, undefined)).toBeUndefined();
    expect(catalogEntry(null, "add")).toBeUndefined();
    expect(outputDoc(catalog, "add", "out")).toBe("The sum `a + b`.");
    expect(outputDoc(catalog, "mass_addition", "result")).toBe("The sum of the list.");
    expect(outputDoc(catalog, "mass_addition", "partial")).toBeUndefined();
    expect(outputDoc(catalog, "mass_addition", "nope")).toBeUndefined();
    expect(outputDoc(null, "add", "out")).toBeUndefined();
  });
  it("renders the hover as name: type — doc", () => {
    expect(portTitle("out", "Number", "The sum `a + b`.")).toBe("out: Number — The sum `a + b`.");
    expect(portTitle("out", "Number", undefined)).toBe("out: Number");
    expect(portTitle("out", "Number", "")).toBe("out: Number");
  });
});

describe("durations (U25)", () => {
  it("labels compactly, in nanoseconds below a microsecond", () => {
    expect(durationLabel(640)).toBe("640ns");
    expect(durationLabel(4_800)).toBe("5µs");
    expect(durationLabel(1_237_600)).toBe("1.2ms");
    expect(durationLabel(43_000_000)).toBe("43ms");
    expect(durationLabel(2_300_000_000)).toBe("2.3s");
  });
  it("titles with three significant figures in the unit that fits", () => {
    expect(durationTitle(640)).toBe("640 ns");
    expect(durationTitle(4_800)).toBe("4.80 µs");
    expect(durationTitle(1_237_600)).toBe("1.24 ms");
    expect(durationTitle(43_900_000_000)).toBe("43.9 s");
    expect(durationTitle(1_234_000_000_000)).toBe("1234 s");
  });
});

describe("statusBadge", () => {
  it("speaks the docs/16 vocabulary", () => {
    expect(statusBadge(undefined, 0).label).toBe("idle");
    const done = statusBadge({ state: "done", generation: 1, nanos: 1_237_600 }, 0);
    expect(done.label).toBe("1.2ms");
    expect(done.title).toBe("done in 1.24 ms");
    expect(statusBadge({ state: "done", generation: 1, nanos: 4_800 }, 0).label).toBe("5µs");
    expect(statusBadge({ state: "done", generation: 1, nanos: 640 }, 0).label).toBe("640ns");
    // A cached node shows its LAST compute's time (the memo entry's cost) in
    // parentheses — the class greys it — and says so in the title (U25).
    const cached = statusBadge({ state: "cached", generation: 2, nanos: 43_900_000_000 }, 0);
    expect(cached.label).toBe("(43.9s)");
    expect(cached.className).toBe("state-cached");
    expect(cached.title).toBe("cached — result reused; the last compute took 43.9 s");
    // An entry that recorded no cost has nothing to show but the word.
    expect(statusBadge({ state: "cached", generation: 2 }, 0).label).toBe("cached");
    expect(statusBadge({ state: "cached", generation: 2 }, 0).title).toBe("cached — result reused");
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
  it("wires follow the GH convention: single, double, thick dashed (U26)", () => {
    expect(wireStyle(0)).toBe("single");
    expect(wireStyle(1)).toBe("double");
    expect(wireStyle(2)).toBe("dashed");
    expect(wireStyle(5)).toBe("dashed");
    expect(wireStrokeWidth(0)).toBeLessThan(wireStrokeWidth(1));
    expect(wireStrokeWidth(1)).toBe(wireStrokeWidth(2));
  });
  it("detects refinements", () => {
    expect(isRefinement("Watertight<Mesh>")).toBe(true);
    expect(isRefinement("Mesh")).toBe(false);
  });
});
