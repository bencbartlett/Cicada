import { describe, expect, it } from "vitest";
import type { SolveSummary } from "../protocol/messages";
import {
  basename,
  boundsText,
  factsList,
  formatNanos,
  highlightedLines,
  lineOwners,
  nodeLineRange,
  paramValueText,
  pendingHint,
  pendingTitle,
  shortHash,
  snapSlider,
  statusText,
  summaryText,
  valueHeadline,
  withStatusCounts,
} from "./format";

describe("paramValueText", () => {
  it("keeps a decimal point on slider / number literals", () => {
    expect(paramValueText("slider", 3)).toBe("3.0");
    expect(paramValueText("slider", 2.5)).toBe("2.5");
    expect(paramValueText("number", 0)).toBe("0.0");
    expect(paramValueText("number", -4)).toBe("-4.0");
    expect(paramValueText("slider", 0.30000000000000004)).toBe("0.30000000000000004");
  });
  it("writes integers without a point and refuses non-integers (one rule with the canvas: no silent rounding)", () => {
    expect(paramValueText("integer", 7)).toBe("7");
    expect(() => paramValueText("integer", 7.4)).toThrow(/integer/);
  });
  it("writes capitalised dialect booleans", () => {
    expect(paramValueText("toggle", true)).toBe("True");
    expect(paramValueText("boolean", false)).toBe("False");
  });
  it("JSON-quotes text", () => {
    expect(paramValueText("text", 'a "b"')).toBe('"a \\"b\\""');
  });
  it("refuses lists and non-finite numbers loudly", () => {
    expect(() => paramValueText("list", "x")).toThrow(/no widget/);
    expect(() => paramValueText("slider", "abc")).toThrow(/finite/);
  });
  it("is the same function the canvas uses (one literal rule)", async () => {
    const canvas = await import("../canvas/grid");
    const shared = await import("../state/literals");
    expect(canvas.paramValueText).toBe(shared.paramValueText);
    expect(paramValueText).toBe(shared.paramValueText);
  });
});

describe("snapSlider", () => {
  it("snaps to the step and clamps", () => {
    expect(snapSlider(2.26, 0, 5, 0.1)).toBe(2.3);
    expect(snapSlider(9, 0, 5, 0)).toBe(5);
    expect(snapSlider(-1, 0.5, 5, 0)).toBe(0.5);
    expect(snapSlider(2.71828, 0, 5, 0)).toBe(2.71828);
  });
});

describe("durations", () => {
  it("formats nanos across scales", () => {
    expect(formatNanos(44_000)).toBe("0.04 ms");
    expect(formatNanos(2_124_300)).toBe("2.1 ms");
    expect(formatNanos(150_000_000)).toBe("150 ms");
    expect(formatNanos(1_234_000_000)).toBe("1.23 s");
  });
});

const idle: SolveSummary = {
  generation: 3,
  running: false,
  cancelled: false,
  computed: 4,
  cached: 6,
  pending: 0,
  red: 0,
  blocked: 0,
  elapsed_ms: 12.4,
  eta_rough: false,
};

describe("summaryText", () => {
  it("describes an idle solve with the doc-16 vocabulary", () => {
    expect(summaryText(idle)).toBe("solved gen 3 · 4 computed / 6 cached · 12.4 ms");
    expect(summaryText({ ...idle, red: 1, blocked: 2 })).toBe(
      "solved gen 3 · 4 computed / 6 cached / 1 red / 2 blocked · 12.4 ms",
    );
  });
  it("shows pending + ETA while running, with ~ when rough", () => {
    expect(summaryText({ ...idle, running: true, pending: 5, eta_ms: 2500, eta_rough: true })).toBe(
      "solving… pending 5 · ETA ~2.50 s",
    );
    expect(summaryText({ ...idle, running: true, pending: 1, eta_ms: 80, eta_rough: false })).toBe(
      "solving… pending 1 · ETA 80 ms",
    );
    expect(summaryText({ ...idle, running: true, pending: 1 })).toBe("solving… pending 1");
  });
  it("lifts red/blocked to the status counts (diagnostic-excluded nodes never solve)", () => {
    const lifted = withStatusCounts(idle, {
      a: { state: "red", generation: 1 },
      b: { state: "blocked", generation: 1 },
      c: { state: "blocked", generation: 1 },
      d: { state: "done", generation: 1 },
    });
    expect(summaryText(lifted)).toBe("solved gen 3 · 4 computed / 6 cached / 1 red / 2 blocked · 12.4 ms");
    expect(withStatusCounts({ ...idle, red: 3 }, {}).red).toBe(3);
  });
  it("says cancelled", () => {
    expect(summaryText({ ...idle, cancelled: true })).toBe("cancelled gen 3 · 4 computed / 6 cached");
  });
});

describe("statusText", () => {
  it("joins state, time, elements and message", () => {
    expect(statusText({ state: "done", generation: 1, nanos: 2_124_300, elements: 1 })).toBe(
      "done · 2.1 ms · 1 element",
    );
    // A cached node's time is its memo entry's LAST compute, not this
    // generation's cache read (docs/13 §Solve streaming).
    expect(statusText({ state: "cached", generation: 4, nanos: 43_900_000_000, elements: 1200 })).toBe(
      "cached · last 43.90 s · 1200 elements",
    );
    expect(statusText({ state: "cached", generation: 4 })).toBe("cached");
    expect(
      statusText({ state: "running", generation: 1, elements: 10, elements_done: 3 }),
    ).toBe("running · 3/10 elements");
    expect(statusText({ state: "red", generation: 1, message: "boom" })).toBe("red · boom");
    expect(statusText(undefined)).toBe("no status yet");
  });
});

describe("compute-on-release hint (docs/13 §Slider drags)", () => {
  it("spells the estimate like the ETA: plain when measured, ~ when it is a floor", () => {
    expect(pendingHint({ estimateMs: 3990.9, rough: false })).toBe("pending · 3.99 s");
    expect(pendingHint({ estimateMs: 3990.9, rough: true })).toBe("pending · ~3.99 s");
    expect(pendingHint({ estimateMs: 1000, rough: false })).toBe("pending · 1.00 s");
    expect(pendingHint({ estimateMs: 640, rough: true })).toBe("pending · ~640 ms");
  });
  it("the tooltip says what pending means and what release does", () => {
    expect(pendingTitle({ estimateMs: 3990.9, rough: false })).toBe(
      "compute-on-release: a live preview would take about 3.99 s, so the viewport waits — the value solves once, when you release",
    );
    expect(pendingTitle({ estimateMs: 1000, rough: true })).toMatch(/at least ~1\.00 s/);
  });
});

describe("values", () => {
  it("truncates hashes to 12 chars", () => {
    expect(shortHash("6d7978fb5616dffab9e3f748159e32514a22a15d")).toBe("6d7978fb5616");
  });
  it("renders bounds and headlines", () => {
    expect(
      boundsText([
        [0, 0, 0],
        [2, 2.5, 1.23456],
      ]),
    ).toBe("[0 0 0] … [2 2.50 1.23]");
    expect(valueHeadline({ kind: "List", hash: "", count: 3, absent: 1, axis: "parts" })).toBe(
      "List · 3 elements · 1 absent · axis parts",
    );
    expect(valueHeadline({ kind: "Mesh", hash: "" })).toBe("Mesh");
  });
  it("orders facts geometry-first", () => {
    expect(factsList({ watertight: true, zeta: "z", triangles: 302, vertices: 153 })).toEqual([
      ["vertices", "153"],
      ["triangles", "302"],
      ["watertight", "true"],
      ["zeta", "z"],
    ]);
    expect(factsList(undefined)).toEqual([]);
    // A Solid's summary ("Solid, N faces, bbox" — docs/03): the error, if
    // any, first; faces before the display tessellation's triangles; the
    // byte count last.
    expect(factsList({ triangles: 12, bytes: 4494, faces: 6 })).toEqual([
      ["faces", "6"],
      ["triangles", "12"],
      ["bytes", "4494"],
    ]);
    expect(factsList({ bytes: 25, error: "tessellate needs the OCCT kernel" })[0]).toEqual([
      "error",
      "tessellate needs the OCCT kernel",
    ]);
  });
});

describe("text panel line mapping", () => {
  // `line` is the server's 0-based index: node `a` sits on 1-based line 3.
  const nodes = [
    { name: "a", line: 2, text: "a = slider(value=1.0)", targets: ["a"] },
    { name: "b", line: 4, text: "b = box(\n  x=a,\n)", targets: ["b"] },
  ];
  it("computes 1-based line ranges from the 0-based index + continuation lines", () => {
    expect(nodeLineRange(nodes[0]!)).toEqual([3, 3]);
    expect(nodeLineRange(nodes[1]!)).toEqual([5, 7]);
  });
  it("maps lines to owners", () => {
    const owners = lineOwners(nodes);
    expect(owners.get(3)).toBe("a");
    expect(owners.get(6)).toBe("b");
    expect(owners.get(4)).toBeUndefined();
  });
  it("highlights the selected nodes' lines (targets count too)", () => {
    expect([...highlightedLines(nodes, ["b"])]).toEqual([5, 6, 7]);
    expect([...highlightedLines(nodes, ["nope"])]).toEqual([]);
  });
});

describe("basename", () => {
  it("handles verbatim windows paths and posix", () => {
    expect(basename("//?/C:/Users/x/examples")).toBe("examples");
    expect(basename("C:\\proj\\wall\\")).toBe("wall");
    expect(basename("wall")).toBe("wall");
  });
});
