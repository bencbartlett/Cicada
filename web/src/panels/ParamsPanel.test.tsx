// @vitest-environment jsdom
/**
 * The params panel's slider under compute-on-release (docs/13 §Slider
 * drags, docs/17 item 3b), rendered from a seeded store: the hint and the
 * number while pending, the drag that returns to its start (review finding
 * 2026-08-20: the native `change` event never fires then, and the badge
 * stood for ever — the release is decided on the pointer/key release now),
 * the release that writes, the keyboard path without a duplicate
 * `end_drag`, and the observer / twin view that only `drag_ended` takes
 * down.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, GraphView, NodeView, ServerEnvelope } from "../protocol/messages";
import { useCicada } from "../state/store";
import { ParamsPanel } from "./ParamsPanel";

const deboss: NodeView = {
  ref: 3,
  name: "deboss",
  targets: ["deboss"],
  line: 4,
  text: "deboss = slider(value=1.0, min=0.5, max=2.0, step=0.1)",
  kind: "call",
  func: "slider",
  title: "Slider",
  category: "Params & input",
  inputs: [],
  outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
  diagnostics: [],
  effectful: false,
  preview: false,
  cell: [0, 0],
  size: [8, 2],
  manual: false,
  param: { kind: "slider", port: "value", value: 1.0, min: 0.5, max: 2.0, step: 0.1 },
};
const graph: GraphView = { nodes: [deboss], wires: [], diagnostics: [] };

const policy = (pending_value: string, estimate_ms = 6665.9, rough = false): ServerEnvelope => ({
  v: 1,
  seq: 4,
  type: "preview_policy",
  payload: { node: "deboss", port: "value", mode: "compute_on_release", estimate_ms, rough, pending_value },
});
const dragEnded: ServerEnvelope = { v: 1, seq: 4, type: "drag_ended", payload: { node: "deboss", port: "value" } };

/** One animation frame: the preview sender flushes its queued tick. */
async function frame(): Promise<void> {
  await act(async () => {
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => resolve());
    });
  });
}

/** A drag tick: the `input` event (React's onChange), NOT the native `change`. */
function tick(range: HTMLInputElement, value: string) {
  fireEvent.input(range, { target: { value } });
}
/** The native `change` the browser fires on release when the value differs from the start. */
function nativeChange(range: HTMLInputElement) {
  fireEvent(range, new Event("change", { bubbles: true }));
}

describe("the params panel's slider under compute-on-release", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    useCicada.setState({ connection: "open", role: "writer", pending: null, notices: [], graph });
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "";
    });
  });
  afterEach(cleanup);

  const renderPanel = () => {
    render(<ParamsPanel />);
    return {
      row: screen.getByTestId("param-deboss"),
      range: screen.getByTestId("widget-deboss") as HTMLInputElement,
      number: screen.getByTestId("number-deboss") as HTMLInputElement,
    };
  };

  it("a cheap cone: committed value, no hint, no pending class", () => {
    const { row, range, number } = renderPanel();
    expect(range.value).toBe("1");
    expect(number.value).toBe("1");
    expect(screen.queryByTestId("param-pending-deboss")).toBeNull();
    expect(row.className).not.toMatch(/pending/);
    expect(range.className).not.toMatch(/pending/);
  });

  it("the policy puts the hint under the name and the pending value in the thumb and the number, in the pending class", () => {
    const { row, range, number } = renderPanel();
    act(() => useCicada.getState().applyServerMessage(policy("1.6")));
    expect(screen.getByTestId("param-pending-deboss").textContent).toBe("pending · 6.67 s");
    expect(row.className).toMatch(/\bpending\b/);
    expect(range.value).toBe("1.6");
    expect(range.className).toMatch(/\bpending\b/);
    expect(number.value).toBe("1.6");
    expect(number.className).toMatch(/\bpending\b/);
    expect(number.title).toMatch(/about 6\.67 s/);
    act(() => useCicada.getState().applyServerMessage(policy("1.7", 2000, true)));
    expect(screen.getByTestId("param-pending-deboss").textContent).toBe("pending · ~2.00 s");
  });

  it("a drag that returns to its start: no `change` ever fires — the pointer's release clears the badge and sends `end_drag`", async () => {
    const { row, range, number } = renderPanel();
    fireEvent.pointerDown(range);
    tick(range, "1.2");
    await frame();
    expect(sent).toEqual([{ type: "param_preview", payload: { node: "deboss", port: "value", value: "1.2" } }]);
    act(() => useCicada.getState().applyServerMessage(policy("1.2")));
    expect(screen.getByTestId("param-pending-deboss")).toBeTruthy();
    tick(range, "1.6");
    await frame();
    expect(useCicada.getState().pending?.value, "the entry follows my thumb").toBe("1.6");
    expect(number.value).toBe("1.6");
    tick(range, "1.0");
    await frame();
    expect(sent.at(-1)).toEqual({ type: "param_preview", payload: { node: "deboss", port: "value", value: "1.0" } });
    // Release on the committed value: Chrome fires no `change` here.
    fireEvent.pointerUp(range);
    expect(sent.at(-1)).toEqual({ type: "end_drag", payload: { node: "deboss", port: "value" } });
    expect(sent.filter((m) => m.type === "set_param")).toEqual([]);
    expect(useCicada.getState().pending).toBeNull();
    expect(screen.queryByTestId("param-pending-deboss")).toBeNull();
    expect(row.className).not.toMatch(/pending/);
    expect(range.value).toBe("1");
    expect(number.value).toBe("1");
    expect(number.className).not.toMatch(/pending/);
    // A pointer release with nothing dragged sends nothing.
    fireEvent.pointerUp(range);
    expect(sent.filter((m) => m.type === "end_drag")).toHaveLength(1);
  });

  it("a release that writes: `change` commits one set_param, the pointer's release adds nothing, the badge waits for the delta", async () => {
    const { range } = renderPanel();
    fireEvent.pointerDown(range);
    tick(range, "1.5");
    await frame();
    act(() => useCicada.getState().applyServerMessage(policy("1.5")));
    nativeChange(range);
    fireEvent.pointerUp(range);
    expect(sent.filter((m) => m.type !== "param_preview")).toEqual([
      { type: "set_param", payload: { node: "deboss", port: "value", value: "1.5" } },
    ]);
    expect(useCicada.getState().pending?.value).toBe("1.5");
    expect(screen.getByTestId("param-pending-deboss")).toBeTruthy();
    // The other order (pointerup first, then change) commits exactly once too.
    cleanup();
    sent.length = 0;
    useCicada.setState({ pending: null });
    const second = renderPanel();
    tick(second.range, "1.4");
    fireEvent.pointerUp(second.range);
    nativeChange(second.range);
    await frame();
    expect(sent.filter((m) => m.type !== "param_preview")).toEqual([
      { type: "set_param", payload: { node: "deboss", port: "value", value: "1.4" } },
    ]);
  });

  it("the keyboard: a step away and back fires `change` on the committed value — one `end_drag`, not two", async () => {
    const { range } = renderPanel();
    tick(range, "1.1");
    nativeChange(range);
    fireEvent.keyUp(range, { key: "ArrowRight" });
    await frame();
    expect(sent.filter((m) => m.type !== "param_preview")).toEqual([
      { type: "set_param", payload: { node: "deboss", port: "value", value: "1.1" } },
    ]);
    // The delta lands the value.
    act(() =>
      useCicada.getState().applyServerMessage({
        v: 1,
        seq: 5,
        type: "delta",
        payload: {
          source: { client: 7, intent_id: "k", label: "set deboss = 1.1" },
          graph: {
            ...graph,
            nodes: [{ ...deboss, param: { ...deboss.param!, value: 1.1 } }],
          },
          text: "",
          dirty: ["deboss"],
          history: { can_undo: true, can_redo: false, undo_label: "set deboss = 1.1", redo_label: null, depth: 1 },
        },
      }),
    );
    sent.length = 0;
    // Away and back within one key-hold: `change` fires with 1.1 (committed).
    tick(range, "1.2");
    tick(range, "1.1");
    nativeChange(range);
    expect(sent.filter((m) => m.type === "end_drag")).toHaveLength(1);
    fireEvent.keyUp(range, { key: "ArrowLeft" });
    expect(sent.filter((m) => m.type === "end_drag"), "the key's release finds no draft").toHaveLength(1);
    expect(sent.filter((m) => m.type === "set_param")).toEqual([]);
  });

  it("the observer / twin view: the pending value from the broadcast alone; `drag_ended` is what takes it down", () => {
    useCicada.setState({ role: "observer" });
    const { row, range, number } = renderPanel();
    expect(range.disabled).toBe(true);
    act(() => useCicada.getState().applyServerMessage(policy("1.6", 5978.9)));
    expect(screen.getByTestId("param-pending-deboss").textContent).toBe("pending · 5.98 s");
    expect(number.value).toBe("1.6");
    act(() => useCicada.getState().applyServerMessage(dragEnded));
    expect(screen.queryByTestId("param-pending-deboss")).toBeNull();
    expect(number.value).toBe("1");
    expect(range.value).toBe("1");
    expect(row.className).not.toMatch(/pending/);
    expect(sent).toEqual([]);
  });

  // Scrub caching (docs/16 §Sliders; item 5 S2): the same buffer bar as the
  // canvas widget's, in the track column under the range — the view's warm
  // set moved by `scrub_progress`, the current notch following the thumb —
  // for the writer and the observer alike.
  it("the scrub buffer bar sits under the range, reads the view + scrub_progress, and follows the thumb", () => {
    const scrubbed: NodeView = {
      ...deboss,
      param: { ...deboss.param!, scrub: { on: true, positions: 16, warmed: [5], warming: true, bytes: 0 } },
    };
    useCicada.setState({ role: "observer", graph: { ...graph, nodes: [scrubbed] }, scrubProgress: {} });
    const { range } = renderPanel();
    const bar = screen.getByTestId("scrub-bar-deboss");
    expect(bar.parentElement).toBe(range.parentElement);
    expect(bar.parentElement?.className).toBe("param-track");
    expect(bar.dataset).toMatchObject({ positions: "16", warmed: "1", warming: "true", current: "5" });
    act(() =>
      useCicada.getState().applyServerMessage({
        v: 1,
        seq: 4,
        type: "scrub_progress",
        payload: { node: "deboss", port: "value", warmed: [2, 3, 4, 5, 6, 7, 8], warming: false, bytes: 2048 },
      }),
    );
    expect(bar.dataset).toMatchObject({ warmed: "7", warming: "false" });
    expect(bar.querySelectorAll(".scrub-seg.warm")).toHaveLength(7);
    // The writer's compute-on-release drag seen from here: the pending value's notch is the current one.
    act(() => useCicada.getState().applyServerMessage(policy("1.4")));
    expect(bar.dataset.current).toBe("9");
    act(() => useCicada.getState().applyServerMessage(dragEnded));
    expect(bar.dataset.current).toBe("5");
    expect(sent).toEqual([]);
  });
});
