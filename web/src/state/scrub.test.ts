/**
 * Scrub caching, the client's pure half and the store's overlay (docs/13
 * §Scrub caching, docs/16 §Sliders; v0.1 item 5 S2): the merge of a
 * slider's view with a later `scrub_progress`, the current-position index
 * off the widget's snap rule, the bar's tooltip, the toggle's state from the
 * SERVER's words alone — and the store keeping the progress beside the
 * graph, replaced per slider, cleared by every snapshot / delta /
 * disconnect / reset.
 */
import { beforeEach, describe, expect, it } from "vitest";
import type { GraphView, NodeView, ScrubProgressPayload, ScrubView, ServerEnvelope } from "../protocol/messages";
import { currentPosition, mergeScrub, scrubBarTitle, scrubToggle, showsScrubBar } from "./scrub";
import { scrubProgressFor, useCicada } from "./store";

const eligibleOff: ScrubView = { on: false, positions: 19, warmed: [], warming: false, bytes: 0 };
const eligibleOn: ScrubView = { on: true, positions: 19, warmed: [5, 6, 7], warming: true, bytes: 1_970_000 };
const tooMany: ScrubView = {
  on: false,
  positions: 0,
  warmed: [],
  warming: false,
  bytes: 0,
  ineligible: "too many positions (51 > 32)",
};
const handWrittenOnIneligible: ScrubView = { ...tooMany, on: true };

function slider(name: string, scrub: ScrubView | undefined, func = "slider"): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text: `${name} = slider(value=2.0, min=0.5, max=5.0, step=0.25)`,
    kind: "call",
    func,
    title: "Number Slider",
    category: "Params & input",
    inputs: [],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    param: { kind: "slider", port: "value", value: 2, min: 0.5, max: 5, step: 0.25, scrub },
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [8, 7],
    manual: false,
  };
}

describe("mergeScrub — the view with the progress overlay laid over it", () => {
  it("no view (not a slider) stays undefined; no progress keeps the view", () => {
    expect(mergeScrub(undefined, undefined)).toBeUndefined();
    expect(mergeScrub(undefined, { node: "x", port: "value", warmed: [1], warming: false, bytes: 0 })).toBeUndefined();
    expect(mergeScrub(eligibleOn, undefined)).toBe(eligibleOn);
  });

  it("the progress moves warmed / warming / bytes / capped and nothing else", () => {
    const progress: ScrubProgressPayload = { node: "size", port: "value", warmed: [4, 5, 6, 7, 8], warming: false, bytes: 4_000_000, capped: true };
    expect(mergeScrub(eligibleOn, progress)).toEqual({
      on: true,
      positions: 19,
      warmed: [4, 5, 6, 7, 8],
      warming: false,
      bytes: 4_000_000,
      capped: true,
    });
    // `capped` absent in the progress clears a view's stale `capped`.
    const capped: ScrubView = { ...eligibleOn, capped: true };
    const merged = mergeScrub(capped, { node: "size", port: "value", warmed: [1], warming: true, bytes: 1 });
    expect(merged).not.toHaveProperty("capped");
    // `on`, `positions`, `ineligible` are the view's — they move with the text only.
    expect(mergeScrub(tooMany, progress)).toMatchObject({ on: false, positions: 0, ineligible: "too many positions (51 > 32)" });
  });
});

describe("showsScrubBar", () => {
  it("draws only an opted-in, eligible slider", () => {
    expect(showsScrubBar(undefined)).toBe(false);
    expect(showsScrubBar(eligibleOff)).toBe(false);
    expect(showsScrubBar(eligibleOn)).toBe(true);
    expect(showsScrubBar(tooMany)).toBe(false);
    expect(showsScrubBar(handWrittenOnIneligible), "positions 0: nothing to draw").toBe(false);
  });
});

describe("currentPosition — the notch the thumb is on, by the widget's snap", () => {
  it("is round((value − min) / step), clamped into the range", () => {
    expect(currentPosition(2.0, 0.5, 0.25, 19)).toBe(6);
    expect(currentPosition(0.5, 0.5, 0.25, 19)).toBe(0);
    expect(currentPosition(5.0, 0.5, 0.25, 19)).toBe(18);
    // Off the grid: the nearest notch (2.1 → 2.0, 2.13 → 2.25).
    expect(currentPosition(2.1, 0.5, 0.25, 19)).toBe(6);
    expect(currentPosition(2.13, 0.5, 0.25, 19)).toBe(7);
    // Outside the bounds: clamped.
    expect(currentPosition(-3, 0.5, 0.25, 19)).toBe(0);
    expect(currentPosition(99, 0.5, 0.25, 19)).toBe(18);
    // 0…0.3 by 0.1 (IEEE's 2.9999999999999996): the last notch is index 3.
    expect(currentPosition(0.3, 0, 0.1, 4)).toBe(3);
  });

  it("degenerate inputs land on 0 rather than NaN", () => {
    expect(currentPosition(2, 0.5, 0, 19)).toBe(0);
    expect(currentPosition(Number.NaN, 0.5, 0.25, 19)).toBe(0);
    expect(currentPosition(2, 0.5, 0.25, 0)).toBe(0);
  });
});

describe("scrubBarTitle", () => {
  it("counts the warm positions and says what the worker is doing", () => {
    expect(scrubBarTitle(eligibleOn)).toBe("scrub cache · 3 / 19 positions warm · warming while the app is idle… · 1.88 MB stored");
    expect(scrubBarTitle({ ...eligibleOn, warmed: Array.from({ length: 19 }, (_, i) => i), warming: false })).toBe(
      "scrub cache · 19 / 19 positions warm · every position is a cache read · 1.88 MB stored",
    );
    expect(scrubBarTitle({ ...eligibleOn, warming: false, capped: true, bytes: 268_435_456 })).toBe(
      "scrub cache · 3 / 19 positions warm · capped at the 256 MiB budget — the warm positions stay · 256.00 MB stored",
    );
    expect(scrubBarTitle({ ...eligibleOn, warmed: [], bytes: 0 })).toBe("scrub cache · 0 / 19 positions warm · warming while the app is idle…");
  });
});

describe("scrubToggle — the server's words, on every surface", () => {
  it("is not offered for a non-slider, or a slider view without scrub (an older server)", () => {
    expect(scrubToggle(undefined)).toBeNull();
    expect(scrubToggle(slider("n", eligibleOff, "construct_domain"))).toBeNull();
    expect(scrubToggle(slider("s", undefined))).toBeNull();
  });

  it("an eligible slider that is off: live, `scrub-cache this slider`, the position count as the hint, next = on", () => {
    expect(scrubToggle(slider("size", eligibleOff))).toMatchObject({
      on: false,
      disabled: false,
      reason: null,
      label: "scrub-cache this slider",
      hint: "19 positions",
      next: true,
    });
  });

  it("an eligible slider that is on: live, `stop scrub-caching`, the warm count, next = off", () => {
    expect(scrubToggle(slider("size", eligibleOn))).toMatchObject({
      on: true,
      disabled: false,
      reason: null,
      label: "stop scrub-caching",
      hint: "3 / 19 positions warm",
      next: false,
    });
  });

  it("an ineligible slider that is off is greyed with the SERVER's reason, verbatim", () => {
    const state = scrubToggle(slider("fine", tooMany));
    expect(state).toMatchObject({ on: false, disabled: true, reason: "too many positions (51 > 32)", hint: "too many positions (51 > 32)" });
    expect(state?.title).toMatch(/^too many positions \(51 > 32\) — /);
    const wired = scrubToggle(
      slider("bound", { ...tooMany, ineligible: "max is wired — the positions are a function of literal min, max and step" }),
    );
    expect(wired?.disabled).toBe(true);
    expect(wired?.hint).toBe("max is wired — the positions are a function of literal min, max and step");
  });

  it("an ineligible slider that is ON (a hand-written kwarg) stays live: turning it off is always allowed", () => {
    expect(scrubToggle(slider("fine", handWrittenOnIneligible))).toMatchObject({
      on: true,
      disabled: false,
      reason: "too many positions (51 > 32)",
      label: "stop scrub-caching",
      hint: "too many positions (51 > 32)",
      next: false,
    });
  });
});

describe("the store's scrub_progress overlay", () => {
  const size = slider("size", eligibleOn);
  const graph: GraphView = { nodes: [size], wires: [], diagnostics: [] };
  const progress = (seq: number, node: string, warmed: number[], warming: boolean): ServerEnvelope => ({
    v: 1,
    seq,
    type: "scrub_progress",
    payload: { node, port: "value", warmed, warming, bytes: warmed.length * 1000 },
  });
  const apply = (envelope: ServerEnvelope) => useCicada.getState().applyServerMessage(envelope);

  beforeEach(() => {
    useCicada.setState({ graph, scrubProgress: {}, pending: null, notices: [] });
  });

  it("records the payload by slider, replacing that slider's entry whole and leaving the others", () => {
    apply(progress(3, "size", [6], true));
    apply(progress(3, "other", [0, 1], false));
    apply(progress(3, "size", [5, 6, 7], true));
    expect(scrubProgressFor(useCicada.getState(), "size")).toEqual({
      node: "size",
      port: "value",
      warmed: [5, 6, 7],
      warming: true,
      bytes: 3000,
    });
    expect(scrubProgressFor(useCicada.getState(), "other")?.warmed).toEqual([0, 1]);
    expect(scrubProgressFor(useCicada.getState(), "none")).toBeUndefined();
    // The graph is NOT rewritten: the canvas would rebuild its nodes and
    // re-route every trace wire on each message.
    expect(useCicada.getState().graph).toBe(graph);
    expect(mergeScrub(size.param?.scrub, scrubProgressFor(useCicada.getState(), "size"))).toMatchObject({
      on: true,
      positions: 19,
      warmed: [5, 6, 7],
    });
  });

  it("a delta clears every entry — its views carry the warm sets as of the rebuild", () => {
    apply(progress(3, "size", [5, 6, 7], true));
    apply({
      v: 1,
      seq: 4,
      type: "delta",
      payload: {
        source: { client: 7, intent_id: "k", label: "set size = 2.25" },
        graph,
        text: "",
        dirty: ["size"],
        history: { can_undo: true, can_redo: false, undo_label: "set size = 2.25", redo_label: null, depth: 1 },
      },
    });
    expect(useCicada.getState().scrubProgress).toEqual({});
  });

  it("a snapshot, a disconnect and a session reset clear it too", () => {
    apply(progress(3, "size", [5], true));
    apply({
      v: 1,
      seq: 5,
      type: "snapshot",
      payload: {
        graph,
        text: "",
        statuses: {},
        summary: {
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
        },
        lease: { writer: null, clients: [] },
        history: { can_undo: false, can_redo: false, undo_label: null, redo_label: null, depth: 0 },
        barrier: false,
        reason: "",
        transport: { playing: false, speed: 1, t_ms: 0, frame: 0, frames: 0, period_ms: 0, driven: [] },
      },
    });
    expect(useCicada.getState().scrubProgress).toEqual({});
    apply(progress(6, "size", [5], true));
    useCicada.getState().markDisconnected("gone", { attempt: 1, nextAt: null });
    expect(useCicada.getState().scrubProgress).toEqual({});
    useCicada.setState({ scrubProgress: { size: { node: "size", port: "value", warmed: [1], warming: false, bytes: 0 } } });
    useCicada.getState().resetSession("t", "other.cic");
    expect(useCicada.getState().scrubProgress).toEqual({});
  });
});
