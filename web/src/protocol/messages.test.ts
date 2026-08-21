/**
 * The control-plane mirror's predicates and the one-op helper. The shapes
 * themselves are types (the server's serde tests pin the JSON); what runs
 * here is what the client computes from them: `isWrite` / `isGesture`
 * (mirrors of `protocol::is_write` / `is_gesture`) and `asOneOp`, which
 * turns N canvas gestures into the ONE intent that makes them one undo step.
 */
import { describe, expect, it } from "vitest";
import {
  asOneOp,
  isGesture,
  isTransport,
  isWrite,
  type ApplyTextRequest,
  type ClientMessage,
  type GestureMessage,
  type TransportMessage,
} from "./messages";

const gestures: GestureMessage[] = [
  { type: "place_node", payload: { func: "box" } },
  { type: "connect", payload: { from: { node: "a", port: "out" }, to: { node: "b", port: "x" } } },
  { type: "disconnect", payload: { to: { node: "b", port: "x" } } },
  { type: "accept_lift", payload: { node: "b", port: "x" } },
  { type: "set_param", payload: { node: "s", port: "value", value: "2.0" } },
  { type: "rename", payload: { node: "a", new: "c" } },
  { type: "delete_node", payload: { node: "a" } },
  { type: "toggle_disable", payload: { node: "a" } },
  { type: "move_node", payload: { node: "a", cell: [1, 2] } },
  { type: "set_preview", payload: { node: "a", on: true } },
];

const applyText: ApplyTextRequest = {
  base_text_hash: "ab",
  files: [{ path: "p.cic", text: "# cicada 1\n" }],
  label: "agent edit",
  actor: { kind: "agent", prompt: "add a sphere" },
};

const writesNotGestures: ClientMessage[] = [
  { type: "param_preview", payload: { node: "s", port: "value", value: "2.0" } },
  { type: "cancel", payload: {} },
  { type: "undo", payload: {} },
  { type: "redo", payload: {} },
  { type: "batch", payload: { ops: gestures.slice(0, 2), label: "two" } },
  { type: "apply_text", payload: applyText },
];

/**
 * The five transport controls, spelled exactly as `protocol.rs`'s
 * `transport_messages_have_the_documented_shapes` decodes them (the
 * envelope adds `v` and `id`): empty payloads are `{}`, a seek carries
 * `frame`, a speed carries `factor`.
 */
const transportControls: TransportMessage[] = [
  { type: "transport_play", payload: {} },
  { type: "transport_pause", payload: {} },
  { type: "transport_seek", payload: { frame: 57 } },
  { type: "transport_speed", payload: { factor: 0.5 } },
  { type: "transport_reset", payload: {} },
];

const reads: ClientMessage[] = [
  { type: "hello", payload: { v: 1 } },
  { type: "inspect", payload: { node: "a" } },
  { type: "inspect_wire", payload: { to: { node: "b", port: "x" } } },
  { type: "probe_wire", payload: { from: { node: "a", port: "out" } } },
  { type: "resync_display", payload: {} },
  { type: "take_lease", payload: {} },
  { type: "screenshot", payload: { id: 1, png_base64: null } },
];

describe("isWrite / isGesture (mirrors of protocol::is_write / is_gesture)", () => {
  it("every gesture is a write and a gesture", () => {
    for (const g of gestures) {
      expect(isWrite(g), g.type).toBe(true);
      expect(isGesture(g), g.type).toBe(true);
    }
  });
  it("previews, cancel, undo, redo, batch and apply_text are writes but not gestures", () => {
    for (const m of writesNotGestures) {
      expect(isWrite(m), m.type).toBe(true);
      expect(isGesture(m), m.type).toBe(false);
    }
  });
  it("reads are neither", () => {
    for (const m of reads) {
      expect(isWrite(m), m.type).toBe(false);
      expect(isGesture(m), m.type).toBe(false);
      expect(isTransport(m), m.type).toBe(false);
    }
  });
});

describe("isTransport (mirror of protocol::is_transport)", () => {
  it("the five controls are writes (writer-only) and transport, never gestures — so never batch elements", () => {
    for (const m of transportControls) {
      expect(isTransport(m), m.type).toBe(true);
      expect(isWrite(m), m.type).toBe(true);
      expect(isGesture(m), m.type).toBe(false);
    }
    expect(JSON.stringify(transportControls.map((m) => m.type))).toBe(
      JSON.stringify(["transport_play", "transport_pause", "transport_seek", "transport_speed", "transport_reset"]),
    );
  });
  it("cancel pauses the transport server-side but is not a transport control", () => {
    expect(isTransport({ type: "cancel", payload: {} })).toBe(false);
    for (const g of gestures) expect(isTransport(g), g.type).toBe(false);
    for (const m of writesNotGestures) expect(isTransport(m), m.type).toBe(false);
  });
});

describe("asOneOp", () => {
  it("sends a single gesture as itself (the server labels it)", () => {
    const op = gestures[6]!;
    expect(asOneOp([op], "delete 1 nodes")).toBe(op);
  });
  it("sends two or more as one batch under the label, in order", () => {
    const ops = [gestures[6]!, gestures[7]!];
    expect(asOneOp(ops, "two things")).toEqual({
      type: "batch",
      payload: { ops: [gestures[6], gestures[7]], label: "two things" },
    });
  });
  it("refuses an empty list loudly — a gesture site with nothing to send must not send", () => {
    expect(() => asOneOp([], "nothing")).toThrow(/no gestures/);
  });
});
