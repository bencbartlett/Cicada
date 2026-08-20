// @vitest-environment jsdom
/**
 * The on-canvas slider's compute-on-release rendering and its drag/release
 * protocol (docs/13 §Slider drags, docs/17 item 3b), against a seeded
 * store: the chip and the value while pending, the pending value following
 * the dragging thumb, the release that writes and the release that does
 * not (`end_drag`, the badge down at once), the twin/observer view that
 * only `drag_ended` can take down. Review finding (2026-08-20): nothing
 * rendered either widget in a test — a mutation blanking the chip passed.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, NodeView, ServerEnvelope } from "../protocol/messages";
import { useCicada } from "../state/store";
import { ParamWidget } from "./ParamWidget";

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

const policy = (pending_value: string, estimate_ms = 3942.3, rough = false): ServerEnvelope => ({
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

describe("the canvas slider under compute-on-release", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    useCicada.setState({ connection: "open", role: "writer", pending: null, notices: [] });
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "";
    });
  });
  afterEach(cleanup);

  const renderSlider = () => {
    render(<ParamWidget view={deboss} param={deboss.param!} writer />);
    return {
      range: screen.getByTestId("slider-deboss") as HTMLInputElement,
      label: screen.getByTestId("slider-value-deboss"),
      row: screen.getByTestId("slider-deboss").parentElement!,
    };
  };

  it("a cheap cone: the committed value, no chip, no pending class", () => {
    const { range, label, row } = renderSlider();
    expect(range.value).toBe("1");
    expect(label.textContent).toBe("1.0");
    expect(screen.queryByTestId("pending-deboss")).toBeNull();
    expect(row.className).not.toMatch(/pending/);
  });

  it("the policy shows the pending value in the thumb and the label, and the `pending · N s` chip (`~` when rough)", () => {
    const { range, label, row } = renderSlider();
    act(() => useCicada.getState().applyServerMessage(policy("1.3")));
    expect(range.value).toBe("1.3");
    expect(label.textContent).toBe("1.3");
    expect(label.title).toMatch(/about 3\.94 s/);
    expect(screen.getByTestId("pending-deboss").textContent).toBe("pending · 3.94 s");
    expect(row.className).toMatch(/\bpending\b/);
    act(() => useCicada.getState().applyServerMessage(policy("1.4", 1000, true)));
    expect(screen.getByTestId("pending-deboss").textContent).toBe("pending · ~1.00 s");
    expect(range.value).toBe("1.4");
  });

  it("my own drag: the thumb is mine, each tick moves the pending value, the release that writes is one set_param", async () => {
    const { range, label } = renderSlider();
    fireEvent.pointerDown(range);
    fireEvent.input(range, { target: { value: "1.2" } });
    await frame();
    expect(sent).toEqual([{ type: "param_preview", payload: { node: "deboss", port: "value", value: "1.2" } }]);
    // The server's verdict on that tick arrives: the chip appears, the
    // thumb stays where my pointer is (not the policy's pending_value).
    act(() => useCicada.getState().applyServerMessage(policy("1.2")));
    expect(screen.getByTestId("pending-deboss").textContent).toBe("pending · 3.94 s");
    fireEvent.input(range, { target: { value: "1.5" } });
    expect(range.value).toBe("1.5");
    expect(label.textContent).toBe("1.5");
    await frame();
    expect(sent.at(-1)).toEqual({ type: "param_preview", payload: { node: "deboss", port: "value", value: "1.5" } });
    expect(useCicada.getState().pending?.value, "the entry follows the thumb for the twin and the observers").toBe(
      "1.5",
    );
    fireEvent.pointerUp(range);
    expect(sent.at(-1)).toEqual({ type: "set_param", payload: { node: "deboss", port: "value", value: "1.5" } });
    expect(sent.filter((m) => m.type === "end_drag")).toEqual([]);
    // The badge stands until the delta: value and badge change together.
    expect(useCicada.getState().pending?.value).toBe("1.5");
    expect(screen.getByTestId("pending-deboss")).toBeTruthy();
  });

  it("a release on the committed value writes nothing: `end_drag` goes out, the chip is down at once, the queued tick is dropped", async () => {
    const { range, label } = renderSlider();
    fireEvent.pointerDown(range);
    fireEvent.input(range, { target: { value: "1.5" } });
    await frame();
    act(() => useCicada.getState().applyServerMessage(policy("1.5")));
    expect(screen.getByTestId("pending-deboss")).toBeTruthy();
    // Back to the start — and release before the frame that would send
    // the 1.0 tick.
    fireEvent.input(range, { target: { value: "1.0" } });
    fireEvent.pointerUp(range);
    expect(sent.at(-1)).toEqual({ type: "end_drag", payload: { node: "deboss", port: "value" } });
    expect(sent.filter((m) => m.type === "set_param")).toEqual([]);
    expect(useCicada.getState().pending).toBeNull();
    expect(screen.queryByTestId("pending-deboss")).toBeNull();
    expect(range.value).toBe("1");
    expect(label.textContent).toBe("1.0");
    await frame();
    expect(sent.at(-1), "no tick after the end_drag — it would be a fresh drag on the committed value").toEqual({
      type: "end_drag",
      payload: { node: "deboss", port: "value" },
    });
    // The drag_ended the intent earns finds nothing to do.
    act(() => useCicada.getState().applyServerMessage(dragEnded));
    expect(useCicada.getState().pending).toBeNull();
  });

  it("the twin / observer view: the pending value shows from the broadcast alone, and only `drag_ended` takes it down", () => {
    const { range, label, row } = renderSlider();
    // Driven elsewhere (the params panel, or the writer as seen by an
    // observer): this widget never engaged.
    act(() => useCicada.getState().applyServerMessage(policy("1.6", 5978.9)));
    expect(range.value).toBe("1.6");
    expect(label.textContent).toBe("1.6");
    expect(screen.getByTestId("pending-deboss").textContent).toBe("pending · 5.98 s");
    // A memo-warm tick painted live: a status is not the end.
    act(() =>
      useCicada.getState().applyServerMessage({
        v: 1,
        seq: 4,
        type: "status",
        payload: {
          generation: 9,
          nodes: {},
          summary: {
            generation: 9,
            running: false,
            cancelled: false,
            computed: 0,
            cached: 24,
            pending: 0,
            red: 0,
            blocked: 0,
            elapsed_ms: 0.3,
            eta_rough: false,
          },
        },
      }),
    );
    expect(screen.getByTestId("pending-deboss")).toBeTruthy();
    // The writer released on the committed value: the server's drag_ended
    // is the one signal this view gets.
    act(() => useCicada.getState().applyServerMessage(dragEnded));
    expect(screen.queryByTestId("pending-deboss")).toBeNull();
    expect(range.value).toBe("1");
    expect(label.textContent).toBe("1.0");
    expect(row.className).not.toMatch(/pending/);
    expect(sent, "an observer's view sends nothing").toEqual([]);
  });

  it("disabled for a non-writer, still showing the pending value", () => {
    render(<ParamWidget view={deboss} param={deboss.param!} writer={false} />);
    const range = screen.getByTestId("slider-deboss") as HTMLInputElement;
    expect(range.disabled).toBe(true);
    act(() => useCicada.getState().applyServerMessage(policy("0.7")));
    expect(range.value).toBe("0.7");
    expect(screen.getByTestId("pending-deboss")).toBeTruthy();
  });
});
