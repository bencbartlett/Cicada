/**
 * The transport's client side (docs/13 §Animation transport; docs/17 item
 * 4): the playhead extrapolation the play bar shows between broadcasts —
 * the server's quantization in the same arithmetic — the speed menu, and
 * the store slice: every snapshot and `transport` message REPLACES the
 * view (never merges), a delta leaves it alone, a dead socket forgets it.
 */
import { beforeEach, describe, expect, it } from "vitest";
import type { DrivenView, GraphView, ServerEnvelope, TransportView } from "../protocol/messages";
import { EMPTY_HISTORY, errorNoticeLevel, useCicada } from "./store";
import {
  DISPLAY_TICK_MS,
  SPEED_CHOICES,
  TRANSPORT_AT_REST,
  drivenEntry,
  fedValue,
  formatPlayhead,
  formatSpeed,
  frameAt,
  hasTimeParams,
  playheadAt,
  speedChoices,
} from "./transport";

const orbit: TransportView = {
  playing: true,
  speed: 1,
  t_ms: 1000,
  frame: 30,
  frames: 120,
  period_ms: 4000,
  driven: [{ node: "spin", port: "frame", signal: "frame", loop: { frames: 120, period_ms: 4000 } }],
};

describe("frameAt — the server's quantization, floor(t × frames / period) mod frames", () => {
  it("walks the default loop: 0 → 0, 1 s → 30, the end wraps to 0", () => {
    expect(frameAt(0, 120, 4000)).toBe(0);
    expect(frameAt(1000, 120, 4000)).toBe(30);
    expect(frameAt(3999.99, 120, 4000)).toBe(119);
    expect(frameAt(4000, 120, 4000)).toBe(0);
    expect(frameAt(9000, 120, 4000)).toBe(30);
  });
  it("quantizes exactly as lower.rs does on the frames a nominal seek rounds short of (31, 62, 65)", () => {
    // The server now lands a seek INSIDE the frame (`Playhead::at_frame`);
    // the view's `t_ms` read back through this must name the same frame.
    for (const frame of [31, 62, 65, 0, 119]) {
      let t = (frame * 4000) / 120;
      // The first playhead inside the frame: nudge like the server does.
      for (let i = 0; i < 8 && frameAt(t, 120, 4000) !== frame; i += 1) t = nextUp(t);
      expect(frameAt(t, 120, 4000), `frame ${frame}`).toBe(frame);
    }
    // …and the nominal start of 31 really does read 30 (why the nudge exists).
    expect(frameAt((31 * 4000) / 120, 120, 4000)).toBe(30);
  });
  it("refuses a loop that is not positive — a protocol fault, never frame 0", () => {
    expect(() => frameAt(0, 0, 4000)).toThrow(RangeError);
    expect(() => frameAt(0, 120, 0)).toThrow(RangeError);
    expect(() => frameAt(0, 1.5, 4000)).toThrow(RangeError);
  });
});

/** The next representable double above `x` (the server's `f64::next_up`). */
function nextUp(x: number): number {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, x);
  view.setBigUint64(0, view.getBigUint64(0) + 1n);
  return view.getFloat64(0);
}

describe("playheadAt — the view is a position at the moment of the message", () => {
  it("while playing, advances t_ms by the wall time since receipt at the speed, and quantizes the frame", () => {
    const state = { view: orbit, receivedAt: 5000 };
    expect(playheadAt(state, 5000)).toEqual({ tMs: 1000, frame: 30 });
    expect(playheadAt(state, 6000)).toEqual({ tMs: 2000, frame: 60 });
    expect(playheadAt({ view: { ...orbit, speed: 2 }, receivedAt: 5000 }, 6000)).toEqual({ tMs: 3000, frame: 90 });
    expect(playheadAt({ view: { ...orbit, speed: 0.25 }, receivedAt: 5000 }, 6000)).toEqual({ tMs: 1250, frame: 37 });
    // Past the loop's end the frame wraps; t_ms does not (unbounded, like the server's).
    expect(playheadAt(state, 9000)).toEqual({ tMs: 5000, frame: 30 });
  });
  it("while paused, is the server's own t_ms and frame whatever the clock says", () => {
    const paused = { view: { ...orbit, playing: false, t_ms: 1033.3333333333333, frame: 31 }, receivedAt: 0 };
    expect(playheadAt(paused, 1e9)).toEqual({ tMs: 1033.3333333333333, frame: 31 });
  });
  it("a clock that reads before the receipt stamp counts as no time elapsed", () => {
    expect(playheadAt({ view: orbit, receivedAt: 5000 }, 4000)).toEqual({ tMs: 1000, frame: 30 });
  });
});

describe("fedValue — what the transport feeds one driven port, on THAT port's own loop", () => {
  // `slow = cycle(period=8.0, frames=40)`, `fast = cycle(period=2.0, frames=60)`,
  // `tick = clock()`: the primary loop is `slow` (the longest period); the
  // view's `frame` / `frames` are its. `fast` loops inside it four times.
  const slow: DrivenView = { node: "slow", port: "frame", signal: "frame", loop: { frames: 40, period_ms: 8000 } };
  const fast: DrivenView = { node: "fast", port: "frame", signal: "frame", loop: { frames: 60, period_ms: 2000 } };
  const tick: DrivenView = { node: "tick", port: "t", signal: "time" };
  const view: TransportView = { ...orbit, playing: false, t_ms: 2000, frame: 10, frames: 40, period_ms: 8000, driven: [slow, fast, tick] };

  it("a frame port shows the frame of its own loop — never the primary loop's frame", () => {
    // At the primary's frame 10 of 40 (2 s) the 2 s loop has come round: frame 0 of 60.
    expect(fedValue(slow, 2000)).toBe("frame 10 of 40");
    expect(fedValue(fast, 2000)).toBe("frame 0 of 60");
    expect(fedValue(fast, 2100)).toBe("frame 3 of 60");
    expect(fedValue(slow, 2100)).toBe("frame 10 of 40");
    // The same server arithmetic as the primary loop: floor(t × frames / period_ms) mod frames.
    expect(fedValue(fast, 7999.99)).toBe(`frame ${frameAt(7999.99, 60, 2000)} of 60`);
  });
  it("a time port shows the playhead in seconds", () => {
    expect(fedValue(tick, 2000)).toBe("2.00 s");
    expect(fedValue(tick, 0)).toBe("0.00 s");
  });
  it("drivenEntry finds the port in the driven set, or nothing when the node is not driven", () => {
    expect(drivenEntry(view, "fast", "frame")).toBe(fast);
    expect(drivenEntry(view, "tick", "t")).toBe(tick);
    expect(drivenEntry(view, "fast", "frames"), "the loop ports are not driven").toBeUndefined();
    expect(drivenEntry(view, "gone", "frame")).toBeUndefined();
    expect(drivenEntry({ ...view, driven: [] }, "slow", "frame")).toBeUndefined();
  });
  it("a frame entry without a positive loop is a protocol fault — thrown, never frame 0", () => {
    const bad = { ...fast, loop: { frames: 0, period_ms: 2000 } };
    expect(() => fedValue(bad, 1000)).toThrow(RangeError);
  });
});

describe("the speed menu", () => {
  it("offers quarter to four times, plus the server's speed when it is none of them", () => {
    expect(SPEED_CHOICES).toEqual([0.25, 0.5, 1, 2, 4]);
    expect(speedChoices(1)).toEqual([0.25, 0.5, 1, 2, 4]);
    expect(speedChoices(1.5)).toEqual([0.25, 0.5, 1, 1.5, 2, 4]);
    expect(speedChoices(8)).toEqual([0.25, 0.5, 1, 2, 4, 8]);
  });
  it("formats speeds and the playhead", () => {
    expect(formatSpeed(0.25)).toBe("0.25×");
    expect(formatSpeed(1)).toBe("1×");
    expect(formatSpeed(4)).toBe("4×");
    expect(formatSpeed(1.5)).toBe("1.5×");
    expect(formatSpeed(4 / 3)).toBe("1.333×");
    expect(formatPlayhead(1250)).toBe("1.25 s");
    expect(formatPlayhead(0)).toBe("0.00 s");
    expect(DISPLAY_TICK_MS).toBeGreaterThanOrEqual(16);
  });
});

describe("hasTimeParams", () => {
  it("is true only when the pipeline has a driven port — nothing before the first snapshot, nothing with driven []", () => {
    expect(hasTimeParams(null)).toBe(false);
    expect(hasTimeParams({ view: TRANSPORT_AT_REST })).toBe(false);
    expect(hasTimeParams({ view: orbit })).toBe(true);
  });
});

// ---------------------------------------------------------- store slice --

const GRAPH: GraphView = { nodes: [], wires: [], diagnostics: [] };
const summary = {
  generation: 1,
  running: false,
  cancelled: false,
  computed: 0,
  cached: 0,
  pending: 0,
  red: 0,
  blocked: 0,
  elapsed_ms: 0,
  eta_rough: false,
};

function snapshot(transport: TransportView, barrier = false): ServerEnvelope {
  return {
    v: 1,
    seq: 3,
    type: "snapshot",
    payload: {
      graph: GRAPH,
      text: "# cicada 1\n",
      statuses: {},
      summary,
      lease: { writer: 1, clients: [[1, "writer"]] },
      barrier,
      reason: barrier ? "external change" : "initial",
      history: EMPTY_HISTORY,
      transport,
    },
  };
}

describe("the store's transport slice", () => {
  beforeEach(() => {
    useCicada.setState({ transport: null, notices: [], connection: "open", role: "writer" });
  });

  it("is null before the first snapshot; a snapshot fills it whole with an arrival stamp", () => {
    expect(useCicada.getState().transport).toBeNull();
    const before = performance.now();
    useCicada.getState().applyServerMessage(snapshot(orbit));
    const after = performance.now();
    const transport = useCicada.getState().transport;
    expect(transport?.view).toEqual(orbit);
    expect(transport?.receivedAt).toBeGreaterThanOrEqual(before);
    expect(transport?.receivedAt).toBeLessThanOrEqual(after);
  });

  it("a `transport` broadcast REPLACES the view — a shorter driven list is not merged into the old one", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(snapshot(orbit));
    const paused: TransportView = { ...TRANSPORT_AT_REST, t_ms: 1250, frame: 37, speed: 2 };
    apply({ v: 1, seq: 3, type: "transport", payload: paused });
    expect(useCicada.getState().transport?.view).toEqual(paused);
    expect(useCicada.getState().transport?.view.driven).toEqual([]);
    // Esc / the last client leaving: playing false, nothing else inferred.
    apply({ v: 1, seq: 3, type: "transport", payload: { ...orbit, playing: false } });
    expect(useCicada.getState().transport?.view.playing).toBe(false);
    expect(useCicada.getState().transport?.view.driven).toEqual(orbit.driven);
  });

  it("a delta carries no transport and leaves the slice exactly as it was", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(snapshot(orbit));
    const kept = useCicada.getState().transport;
    apply({
      v: 1,
      seq: 4,
      type: "delta",
      payload: { source: { client: 1, label: "set size" }, graph: GRAPH, text: "", dirty: [], history: EMPTY_HISTORY },
    });
    expect(useCicada.getState().transport).toBe(kept);
  });

  it("a reload barrier's snapshot replaces it too (the loop is re-read from the new text)", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(snapshot(orbit));
    apply(snapshot({ ...orbit, frames: 50, period_ms: 2000, frame: 25 }, true));
    expect(useCicada.getState().transport?.view.frames).toBe(50);
  });

  it("a dead socket forgets it — no playhead is extrapolated over a connection nobody can confirm", () => {
    useCicada.getState().applyServerMessage(snapshot(orbit));
    useCicada.getState().markDisconnected("socket closed", { attempt: 1, nextAt: 1 });
    expect(useCicada.getState().transport).toBeNull();
  });

  it("a refused control (kind `transport`) is an error notice with the server's words", () => {
    expect(errorNoticeLevel("transport")).toBe("error");
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 3,
      type: "error",
      payload: { intent_id: "s1", kind: "transport", message: "frame 500 is outside the loop (frames 0..120)" },
    });
    expect(useCicada.getState().notices.map((n) => [n.level, n.message])).toEqual([
      ["error", "frame 500 is outside the loop (frames 0..120)"],
    ]);
    expect(useCicada.getState().lastError).toEqual({
      intentId: "s1",
      kind: "transport",
      message: "frame 500 is outside the loop (frames 0..120)",
    });
  });
});
