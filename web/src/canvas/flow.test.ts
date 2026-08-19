import { describe, expect, it } from "vitest";
import type { GraphView, NodeView, WireView } from "../protocol/messages";
import { buildEdges, buildNodes, sameNames, syncSelected, type CanvasNode } from "./flow";

const node = (name: string, cell: [number, number], size: [number, number] = [8, 3]): NodeView => ({
  ref: 1,
  name,
  targets: [name],
  line: 0,
  text: `${name} = f()`,
  kind: "call",
  func: "f",
  title: "F",
  category: "Maths & logic",
  inputs: [],
  outputs: [],
  diagnostics: [],
  effectful: false,
  preview: false,
  cell,
  size,
  manual: false,
});

const wire = (from: string, to: string, port: string): WireView => ({
  id: `${from}.out->${to}.${port}`,
  from: { node: from, port: "out" },
  to: { node: to, port },
  lift: 0,
  type: "Number",
  depth: 0,
  red: false,
});

const graph: GraphView = {
  nodes: [node("a", [0, 0]), node("b", [11, 2], [8, 4])],
  wires: [wire("a", "b", "x")],
  diagnostics: [],
};

describe("buildNodes", () => {
  it("places nodes at cell × unit with size × unit and mirrors selection", () => {
    const nodes = buildNodes(graph, 24, [], ["b"]);
    expect(nodes.map((n) => n.id)).toEqual(["a", "b"]);
    expect(nodes[0]?.position).toEqual({ x: 0, y: 0 });
    expect(nodes[1]?.position).toEqual({ x: 264, y: 48 });
    expect(nodes[1]?.width).toBe(192);
    expect(nodes[1]?.height).toBe(96);
    expect(nodes.map((n) => n.selected)).toEqual([false, true]);
    expect(nodes[0]?.data.view.name).toBe("a");
  });
  it("keeps the optimistic position of a node mid-drag", () => {
    const prev: CanvasNode[] = buildNodes(graph, 24, [], []).map((n) =>
      n.id === "a" ? { ...n, position: { x: 99, y: 77 }, dragging: true } : n,
    );
    const next = buildNodes(graph, 24, prev, []);
    expect(next[0]?.position).toEqual({ x: 99, y: 77 });
    expect(next[1]?.position).toEqual({ x: 264, y: 48 });
  });
  it("makes broken lines undraggable and leaves the rest to the canvas flag", () => {
    const g: GraphView = { ...graph, nodes: [{ ...node("z", [0, 0]), kind: "broken" }, node("a", [0, 4])] };
    const nodes = buildNodes(g, 24, [], []);
    expect(nodes[0]?.draggable).toBe(false);
    expect(nodes[1]?.draggable).toBeUndefined();
  });
});

describe("buildEdges", () => {
  it("maps wires to handles by port name and marks the selected one", () => {
    const edges = buildEdges(graph, "a.out->b.x");
    expect(edges).toHaveLength(1);
    const e = edges[0]!;
    expect(e.source).toBe("a");
    expect(e.sourceHandle).toBe("out");
    expect(e.target).toBe("b");
    expect(e.targetHandle).toBe("x");
    expect(e.selected).toBe(true);
    expect(e.deletable).toBe(false);
    expect(e.data?.wire.id).toBe("a.out->b.x");
  });
});

describe("syncSelected / sameNames", () => {
  it("returns the same array when nothing changes", () => {
    const items = [{ id: "a", selected: true }, { id: "b" }];
    expect(syncSelected(items, new Set(["a"]))).toBe(items);
    const next = syncSelected(items, new Set(["b"]));
    expect(next).not.toBe(items);
    expect(next.map((i) => i.selected)).toEqual([false, true]);
  });
  it("compares name sets order-insensitively", () => {
    expect(sameNames(["a", "b"], ["b", "a"])).toBe(true);
    expect(sameNames(["a"], ["a", "b"])).toBe(false);
    expect(sameNames([], [])).toBe(true);
  });
});
