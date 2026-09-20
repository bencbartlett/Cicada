/**
 * The profiler's pure half (docs/16 §Inspector contents; v0.1 wave 5 P1):
 * the ring's arcs, the phases it draws, the client's phases, the node
 * table's sort and filter, and the formatting.
 */
import { describe, expect, it } from "vitest";
import type { ProfileNode, ProfileView } from "../protocol/messages";
import type { GenerationFrames } from "../state/frameBus";
import { EMPTY_CACHES, type DisplayPass } from "../state/store";
import {
  DEFAULT_NODE_SORT,
  RING_GAP_PX,
  RING_RADIUS,
  RING_TOP_NODES,
  clientPhases,
  displayCaption,
  filterProfileNodes,
  nextSort,
  nodeShares,
  nodeTimeNanos,
  profileTitle,
  rateText,
  ringArcs,
  ringPhases,
  shareText,
  sortProfileNodes,
  type RingPhase,
} from "./profile";

const node = (name: string, extra: Partial<ProfileNode> = {}): ProfileNode => ({ name, state: "done", ...extra });

const view = (nodes: ProfileNode[], phases: Partial<ProfileView["phases"]> = {}): ProfileView => ({
  generation: 12,
  kind: "structural",
  phases: { queued_ms: 1, solve_ms: 5, tessellate_ms: 40, encode_ms: 2, bytes: 4096, ...phases },
  nodes,
  display: [],
  caches: EMPTY_CACHES,
});

const phase = (id: string, ms: number): RingPhase => ({ id, label: id, ms, color: `var(--prof-${id})`, kind: "server" });

describe("ringArcs", () => {
  it("shares the turn by milliseconds, contiguous from 12 o'clock, in the order given", () => {
    const arcs = ringArcs([phase("a", 30), phase("b", 10), phase("c", 60)]);
    expect(arcs.map((a) => a.id)).toEqual(["a", "b", "c"]);
    expect(arcs.map((a) => a.share)).toEqual([0.3, 0.1, 0.6]);
    expect(arcs.reduce((s, a) => s + a.share, 0)).toBeCloseTo(1, 12);
    expect(arcs[0]!.start).toBeCloseTo(-Math.PI / 2, 12);
    for (let i = 1; i < arcs.length; i += 1) expect(arcs[i]!.start).toBeCloseTo(arcs[i - 1]!.end, 12);
    expect(arcs[arcs.length - 1]!.end).toBeCloseTo(-Math.PI / 2 + 2 * Math.PI, 12);
    for (const arc of arcs) {
      expect(arc.d.startsWith("M ")).toBe(true);
      expect(arc.d).toContain(` A ${RING_RADIUS} ${RING_RADIUS} `);
    }
    // The 60 % arc sweeps more than half a turn: the large-arc flag is set; the 10 % one not.
    expect(arcs[2]!.d).toMatch(/ 0 1 1 /);
    expect(arcs[1]!.d).toMatch(/ 0 0 1 /);
  });

  it("draws nothing for a phase without time and nothing at all when nothing took time", () => {
    expect(ringArcs([phase("a", 0), phase("b", -1), phase("c", Number.NaN)])).toEqual([]);
    expect(ringArcs([])).toEqual([]);
    const arcs = ringArcs([phase("a", 0), phase("b", 5)]);
    expect(arcs.map((a) => a.id)).toEqual(["b"]);
  });

  it("a lone arc is the whole ring — two half arcs, no gap", () => {
    const [only] = ringArcs([phase("a", 7)]);
    expect(only!.share).toBe(1);
    expect(only!.d.match(/ A /g)?.length).toBe(2);
    // Starts at the top and returns to it.
    const top = `60 ${60 - RING_RADIUS}`;
    expect(only!.d.startsWith(`M ${top} `)).toBe(true);
    expect(only!.d.endsWith(` ${top}`)).toBe(true);
  });

  it("shortens adjacent arcs by half the surface gap at each end (the mark spec's 2 px)", () => {
    const [half] = ringArcs([phase("a", 1), phase("b", 1)]);
    const gap = RING_GAP_PX / RING_RADIUS;
    // The path starts a half-gap past the top: x is just right of centre.
    const x = Number(half!.d.split(" ")[1]);
    expect(x).toBeGreaterThan(60);
    expect(x).toBeCloseTo(60 + RING_RADIUS * Math.cos(-Math.PI / 2 + gap / 2), 3);
    // Angles themselves stay contiguous; only the drawn path is trimmed.
    const [a, b] = ringArcs([phase("a", 1), phase("b", 1)]);
    expect(a!.end).toBeCloseTo(b!.start, 12);
  });

  it("a sliver narrower than the gap still gets a hairline", () => {
    const arcs = ringArcs([phase("a", 10_000), phase("b", 0.001)]);
    expect(arcs).toHaveLength(2);
    const sliver = arcs[1]!;
    const [, x0, y0, , , , , , , x1, y1] = sliver.d.split(" ");
    expect(x0 === x1 && y0 === y1, "not a zero-length path").toBe(false);
    // Its drawn length is the minimum, not its true (sub-pixel) share.
    const drawn = Math.hypot(Number(x1) - Number(x0), Number(y1) - Number(y0));
    expect(drawn).toBeGreaterThan(0.7);
    expect(drawn).toBeLessThan(0.8);
    expect(sliver.share).toBeLessThan(1e-6);
  });
});

describe("ringPhases", () => {
  const client = { decode_ms: 3, upload_ms: 4, first_paint_ms: 50, socket_ms: 6, rate_bytes_per_ms: 100, frames: 2 };

  it("lists the top nodes by this generation's work, the rest as one arc, then the server's and the client's phases", () => {
    const nodes = Array.from({ length: 11 }, (_, i) => node(`n${i}`, { nanos: (i + 1) * 1e6, elements: 1 }));
    const phases = ringPhases(view(nodes), client);
    const ids = phases.map((p) => p.id);
    // n10..n3 are the eight costliest; n0..n2 fold into "other" (in pipeline order).
    expect(ids.slice(0, RING_TOP_NODES)).toEqual(["node:n3", "node:n4", "node:n5", "node:n6", "node:n7", "node:n8", "node:n9", "node:n10"]);
    expect(ids.slice(RING_TOP_NODES)).toEqual(["other", "tessellate", "encode", "socket", "decode", "upload"]);
    const other = phases.find((p) => p.id === "other")!;
    expect(other.label).toBe("3 other nodes");
    expect(other.ms).toBeCloseTo(1 + 2 + 3, 9);
    expect(phases.find((p) => p.id === "tessellate")!.ms).toBe(40);
    expect(phases.find((p) => p.id === "encode")!.ms).toBe(2);
    expect(phases.find((p) => p.id === "socket")!.ms).toBe(6);
    expect(phases.find((p) => p.id === "decode")!.ms).toBe(3);
    expect(phases.find((p) => p.id === "upload")!.ms).toBe(4);
    expect(phases.find((p) => p.id === "node:n10")!.ms).toBe(11);
  });

  it("a node's colour follows its order in the pipeline among the top nodes, not its rank", () => {
    const cheapFirst = [node("a", { nanos: 1e6 }), node("b", { nanos: 9e6 })];
    const phases = ringPhases(view(cheapFirst), client);
    expect(phases.find((p) => p.id === "node:a")!.color).toBe("var(--prof-1)");
    expect(phases.find((p) => p.id === "node:b")!.color).toBe("var(--prof-2)");
    // Their costs swap: the colours hold.
    const swapped = [node("a", { nanos: 9e6 }), node("b", { nanos: 1e6 })];
    const again = ringPhases(view(swapped), client);
    expect(again.find((p) => p.id === "node:a")!.color).toBe("var(--prof-1)");
    expect(again.find((p) => p.id === "node:b")!.color).toBe("var(--prof-2)");
  });

  it("a cached node cost this generation a cache read: not an arc; a red one neither; a socket not measured is 0", () => {
    const nodes = [node("hit", { state: "cached", last_nanos: 5e9 }), node("bad", { state: "red" }), node("done", { nanos: 2e6 })];
    const phases = ringPhases(view(nodes), { ...client, socket_ms: null });
    expect(phases.filter((p) => p.kind === "node").map((p) => p.id)).toEqual(["node:done"]);
    expect(phases.find((p) => p.id === "other")!.ms).toBe(0);
    expect(phases.find((p) => p.id === "other")!.label).toBe("0 other nodes");
    expect(phases.find((p) => p.id === "socket")!.ms).toBe(0);
    expect(ringArcs(phases).map((a) => a.id)).toEqual(["node:done", "tessellate", "encode", "decode", "upload"]);
  });
});

describe("clientPhases", () => {
  const frames: GenerationFrames = { frames: 3, bytes: 4096, decodeMs: 2, applyMs: 5, firstAt: 1000, lastAt: 1100 };
  const pass = (generation: number, beganAt: number, paintedMs: number | null): DisplayPass => ({
    generation,
    phase: "painted",
    outputs: 1,
    frames: 3,
    bytes: 4096,
    tessellateMs: 40,
    encodeMs: 2,
    cancelled: false,
    cutBy: null,
    beganAt,
    paintedMs,
  });

  it("no frames seen: nothing decoded or uploaded, no socket, the paint from the pass when it is this generation's", () => {
    expect(clientPhases(view([]), null, pass(12, 900, 130))).toEqual({
      decode_ms: 0,
      upload_ms: 0,
      first_paint_ms: 130,
      socket_ms: null,
      rate_bytes_per_ms: null,
      frames: 0,
    });
    expect(clientPhases(view([]), { ...frames, frames: 0 }, pass(11, 900, 130)).first_paint_ms).toBeNull();
  });

  it("the socket is what remains of the client's wall to the last frame once the server's phases and its own work are out", () => {
    // 1100 − 1000 = 100 ms wall; minus tessellate 40, encode 2, decode 2, upload 5 → 51.
    const phases = clientPhases(view([]), frames, pass(12, 1000, 130));
    expect(phases).toEqual({
      decode_ms: 2,
      upload_ms: 5,
      first_paint_ms: 130,
      socket_ms: 51,
      rate_bytes_per_ms: 4096 / 51,
      frames: 3,
    });
  });

  it("never negative: a pass whose begin arrived late clamps the socket at 0 with no rate", () => {
    const phases = clientPhases(view([]), frames, pass(12, 1090, null));
    expect(phases.socket_ms).toBe(0);
    expect(phases.rate_bytes_per_ms).toBeNull();
    expect(phases.first_paint_ms).toBeNull();
  });

  it("another generation's pass says nothing about the socket or the paint", () => {
    const phases = clientPhases(view([]), frames, pass(13, 1000, 130));
    expect(phases.socket_ms).toBeNull();
    expect(phases.first_paint_ms).toBeNull();
    expect(phases.decode_ms).toBe(2);
    expect(clientPhases(view([]), frames, null).socket_ms).toBeNull();
  });

  it("a cut pass that sent no frame painted nothing: no first paint, whatever the store stamped at its end", () => {
    // The store stamps a pass with no frames at `display_end` (nothing to
    // render); for a CUT pass that stamp is the time until Esc landed.
    const cut: DisplayPass = { ...pass(12, 1000, 787), frames: 0, outputs: 0, cancelled: true, cutBy: "esc" };
    const phases = clientPhases({ ...view([]), cancelled: true, cut_by: "esc" }, null, cut);
    expect(phases.first_paint_ms).toBeNull();
    expect(phases.frames).toBe(0);
    // A pass that drew nothing NEW (not cut) did paint: nothing changed, at once.
    const empty: DisplayPass = { ...pass(12, 1000, 3), frames: 0, outputs: 0 };
    expect(clientPhases(view([]), null, empty).first_paint_ms).toBe(3);
    // A cut pass that did send frames before the cut painted those.
    const partial: DisplayPass = { ...pass(12, 1000, 400), cancelled: true, cutBy: "edit" };
    expect(clientPhases({ ...view([]), cut_by: "edit" }, frames, partial).first_paint_ms).toBe(400);
  });
});

describe("the node table", () => {
  const rows: ProfileNode[] = [
    node("ball", { nanos: 3e6, elements: 1 }),
    node("block", { state: "cached", last_nanos: 9e6, elements: 1 }),
    node("bad", { state: "red" }),
    node("vol", { state: "blocked" }),
    node("arms", { nanos: 1e6, elements: 40 }),
    node("k", { state: "done" }),
  ];
  const shares = nodeShares(rows);

  it("shares are of this generation's measured work — computed nodes only", () => {
    expect(shares.get("ball")).toBeCloseTo(0.75, 12);
    expect(shares.get("arms")).toBeCloseTo(0.25, 12);
    expect(shares.has("block"), "a cache read is not this generation's work").toBe(false);
    expect(shares.has("k"), "a literal has no cost").toBe(false);
    expect(nodeShares([node("x", { state: "cached", last_nanos: 5 })]).size).toBe(0);
  });

  it("opens costliest first; rows without a time last either way; ties by name", () => {
    expect(DEFAULT_NODE_SORT).toEqual({ key: "time", descending: true });
    expect(sortProfileNodes(rows, DEFAULT_NODE_SORT, shares).map((n) => n.name)).toEqual(["block", "ball", "arms", "bad", "k", "vol"]);
    expect(sortProfileNodes(rows, { key: "time", descending: false }, shares).map((n) => n.name)).toEqual(["arms", "ball", "block", "bad", "k", "vol"]);
    expect(nodeTimeNanos(rows[1]!), "a cached row shows its last compute").toBe(9e6);
    expect(nodeTimeNanos(rows[2]!)).toBeNull();
  });

  it("sorts by name, by state (what cost this generation first), by share and by elements", () => {
    expect(sortProfileNodes(rows, { key: "name", descending: false }, shares).map((n) => n.name)).toEqual(["arms", "bad", "ball", "block", "k", "vol"]);
    expect(sortProfileNodes(rows, { key: "name", descending: true }, shares).map((n) => n.name)).toEqual(["vol", "k", "block", "ball", "bad", "arms"]);
    expect(sortProfileNodes(rows, { key: "state", descending: false }, shares).map((n) => n.state)).toEqual(["done", "done", "done", "cached", "red", "blocked"]);
    expect(sortProfileNodes(rows, { key: "share", descending: true }, shares).map((n) => n.name).slice(0, 2)).toEqual(["ball", "arms"]);
    expect(sortProfileNodes(rows, { key: "elements", descending: true }, shares).map((n) => n.name).slice(0, 3)).toEqual(["arms", "ball", "block"]);
  });

  it("a header click flips the same key and opens a new one the way it reads best", () => {
    expect(nextSort(DEFAULT_NODE_SORT, "time")).toEqual({ key: "time", descending: false });
    expect(nextSort(DEFAULT_NODE_SORT, "name")).toEqual({ key: "name", descending: false });
    expect(nextSort({ key: "name", descending: false }, "share")).toEqual({ key: "share", descending: true });
    expect(nextSort({ key: "name", descending: false }, "state")).toEqual({ key: "state", descending: false });
  });

  it("filters on the name or the state, case-insensitively; blank keeps every row", () => {
    expect(filterProfileNodes(rows, "b").map((n) => n.name)).toEqual(["ball", "block", "bad", "vol"]);
    expect(filterProfileNodes(rows, "CACHED").map((n) => n.name)).toEqual(["block"]);
    expect(filterProfileNodes(rows, "  ")).toHaveLength(rows.length);
    expect(filterProfileNodes(rows, "zzz")).toEqual([]);
  });
});

describe("formatting", () => {
  it("shares read as one-decimal percentages, slivers as < 0.1 %, none as a dash", () => {
    expect(shareText(0.1234)).toBe("12.3 %");
    expect(shareText(1)).toBe("100.0 %");
    expect(shareText(0.0004)).toBe("< 0.1 %");
    expect(shareText(0)).toBe("0.0 %");
    expect(shareText(null)).toBe("—");
    expect(shareText(undefined)).toBe("—");
  });

  it("the headline names a cut pass — Esc's as the chip's `cancelled`, an edit's while its record stands", () => {
    expect(profileTitle(view([]))).toBe("gen 12 · structural");
    expect(profileTitle({ ...view([]), cancelled: true, cut_by: "esc" })).toBe("gen 12 · structural · cut by Esc");
    expect(profileTitle({ ...view([]), cut_by: "edit" })).toBe("gen 12 · structural · cut by an edit");
    // The display caption says why a cut pass has no rows, or that its rows are partial.
    expect(displayCaption(view([]))).toBe("this pass drew nothing new");
    expect(displayCaption({ ...view([]), cancelled: true, cut_by: "esc" })).toBe("the pass was cut before it drew — the previous picture stays");
    const row = { node: "ball", output: "out", triangles: 8, bytes: 16, solids: 1, cache_hits: 0, cache_misses: 1 };
    expect(displayCaption({ ...view([]), display: [row] })).toBe("what this pass drew");
    expect(displayCaption({ ...view([]), cut_by: "edit", display: [row] })).toBe("what this pass drew before it was cut");
  });

  it("the headline and the socket's rate", () => {
    expect(profileTitle(view([]))).toBe("gen 12 · structural");
    expect(rateText(2_444_000, 2_444_000 / 51)).toBe("2.33 MB at 45.7 MB/s");
    expect(rateText(4096, 4096 / 100)).toBe("4.0 KB at 40.0 KB/s");
    expect(rateText(4096, null)).toBeNull();
  });
});
