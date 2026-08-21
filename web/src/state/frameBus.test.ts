/**
 * The frame bus under the two-lane socket (docs/13 §Two lanes, one socket).
 *
 * The server drains a client's control lane ahead of its display lane, so
 * a `delta`, `status` or `preview_policy` may reach the page before frames
 * the server queued earlier. Two things make that harmless, and both are
 * pinned here: the bus hands frames to the viewport's ledger in the display
 * lane's own FIFO order (a control text applies nothing to the ledger), and
 * the ledger's generation rules converge on the server's display table from
 * any interleaving of a restream with the frames queued before it. The one
 * text the ledger reacts to — `display_reset` — rides the display lane, FIFO
 * with the frames it announces; the test below shows the ledger would
 * converge even if it overtook them (the second-socket transport docs/13
 * defers would make that the wire order), and why "drop every frame older
 * than the reset's generation" would be the WRONG rule: a restream carries
 * each output at the generation that last drew it, and the reset's
 * generation is the newest of those — unchanged outputs arrive below it.
 * The ledger empties on EVERY reset (counted — `displayResets`), not on a
 * change of the reset's generation: that generation is the table's max and
 * can repeat after an output vanished.
 */
import { expect, test } from "vitest";
import { decodeFrame, encodeBatchForTest, type BatchFrame, type Frame, type FrameHeader } from "../protocol/frames";
import { liveSceneStore } from "../viewport/liveStore";
import { outputKey } from "../viewport/sceneStore";
import { frameBus } from "./frameBus";
import { useCicada } from "./store";

const BYTES = 64;

function header(kind: FrameHeader["kind"], generation: number, node: number): FrameHeader {
  return { kind, generation, node, output: 0, elementStart: 0, elementCount: 1 };
}

/** One mesh triangle for `node` at `generation`, one element with `pickId`. */
function mesh(generation: number, node: number, pickId: number): BatchFrame {
  const buffer = encodeBatchForTest(
    header("mesh", generation, node),
    [{ elementIndex: 0, pickId, vertexStart: 0, vertexCount: 3, indexStart: 0, indexCount: 3 }],
    [0, 0, 0, 1, 0, 0, 0, 1, 0],
    [0, 1, 2],
    [pickId, pickId, pickId],
  );
  return decodeFrame(buffer) as BatchFrame;
}

function clear(generation: number, node: number): Frame {
  return { header: header("clear", generation, node) };
}

/** The server's `display_reset` text, as the connection applies it. */
function displayReset(generation: number): void {
  useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "display_reset", payload: { generation } });
}

function held(): Record<string, number> {
  const out: Record<string, number> = {};
  for (const [key, entry] of liveSceneStore().outputs) out[key] = entry.generation;
  return out;
}

test("the bus delivers in arrival order — replayed to a late subscriber, then live", () => {
  const seen: string[] = [];
  // Nothing is subscribed yet (the live ledger starts on first use below):
  // these queue for replay in order.
  frameBus.publish(mesh(1, 40, 400), BYTES);
  frameBus.publish(mesh(1, 41, 410), BYTES);
  const unsubscribe = frameBus.subscribe((frame) => seen.push(outputKey(frame.header.node, frame.header.output)));
  expect(seen).toEqual(["40:0", "41:0"]);
  frameBus.publish(mesh(2, 42, 420), BYTES);
  expect(seen).toEqual(["40:0", "41:0", "42:0"]);
  expect(frameBus.received).toBe(3);
  expect(frameBus.bytes).toBe(3 * BYTES);
  expect(frameBus.lastGeneration).toBe(2);
  unsubscribe();
});

test("a display_reset that overtook the frames queued before it: the ledger converges on the restream's set", () => {
  const scene = liveSceneStore();
  const counters = { applied: 0 };
  frameBus.subscribe(() => {
    counters.applied += 1;
  });

  // The join: generation 3 drew A (node ref 1) and B (node ref 2). The
  // server queued [display_reset{3}, A@3, B@3]; the client has applied
  // the reset and A@3 so far — B@3 is still on the wire.
  displayReset(3);
  frameBus.publish(mesh(3, 1, 11), BYTES);
  expect(held()).toEqual({ "1:0": 3 });

  // Meanwhile generation 5 repainted A (A@5 queued behind B@3), and the
  // client asked for a resync: the server queued display_reset{5} — the
  // newest generation in its display table — then the restream: A@5 and
  // B@3, each at the generation that last drew it.
  //
  // Suppose the reset overtook the queued frames. The ledger empties …
  displayReset(5);
  expect(useCicada.getState().displayGeneration).toBe(5);
  expect(held()).toEqual({});

  // … and the display lane lands after it, FIFO: the old B@3 (older than
  // the reset's generation — and NOT stale: the restream re-sends exactly
  // it), the A@5 the resync raced, then the restream itself.
  frameBus.publish(mesh(3, 2, 12), BYTES);
  expect(held()).toEqual({ "2:0": 3 });
  frameBus.publish(mesh(5, 1, 11), BYTES);
  frameBus.publish(mesh(5, 1, 11), BYTES); // restream: idempotent
  frameBus.publish(mesh(3, 2, 12), BYTES); // restream: idempotent
  expect(held()).toEqual({ "1:0": 5, "2:0": 3 });
  expect(scene.outputs.get("1:0")?.batches.size).toBe(1);
  expect(scene.outputs.get("2:0")?.batches.size).toBe(1);
  expect(counters.applied).toBe(5);

  // A frame genuinely older than what the ledger holds for its output is
  // dropped — the rule that makes stale frames unpaintable by construction
  // is per output, never "older than the reset".
  expect(scene.apply(mesh(4, 1, 11))).toBe("dropped");
  expect(held()).toEqual({ "1:0": 5, "2:0": 3 });

  // Generation 6 clears B: a clear lands at the generation the server
  // last drew the output with or later, so it is never dropped as stale.
  frameBus.publish(clear(6, 2), BYTES);
  expect(held()).toEqual({ "1:0": 5 });
});

test("a display_reset at an UNCHANGED generation still empties the ledger: an output whose clear was lost is not kept", () => {
  // The ledger holds A@5 from above; the server also drew B (node ref 2)
  // at generation 5.
  frameBus.publish(mesh(5, 2, 12), BYTES);
  expect(held()).toEqual({ "1:0": 5, "2:0": 5 });

  // Generation 6 removed B. Its `clear` rode a socket that dropped, so the
  // page never applied it; the server's display table is now {A@5} and its
  // MAX generation is still 5. The reconnect (or a `resync_display`)
  // announces display_reset{5} — the same generation as before. The
  // convergence argument (docs/13 §Two lanes, one socket, point 2) needs
  // the ledger to empty HERE: keyed to a change of generation it would not,
  // and B would stay painted for good (review 2026-08-21).
  const before = useCicada.getState().displayGeneration;
  expect(before).toBe(5);
  displayReset(5);
  expect(useCicada.getState().displayGeneration).toBe(before);
  expect(held()).toEqual({});

  // The restream re-sends exactly the table: A@5, and no B.
  frameBus.publish(mesh(5, 1, 11), BYTES);
  expect(held()).toEqual({ "1:0": 5 });
  expect(liveSceneStore().outputs.has("2:0")).toBe(false);
});

test("control-plane texts that overtake frames touch nothing the ledger reads", () => {
  const scene = liveSceneStore();
  const before = { held: held(), received: frameBus.received, applied: scene.framesReceived };
  const store = useCicada.getState();
  store.applyServerMessage({
    v: 1,
    seq: 9,
    type: "status",
    payload: {
      generation: 7,
      nodes: {},
      summary: {
        generation: 7,
        running: false,
        cancelled: false,
        computed: 1,
        cached: 0,
        pending: 0,
        red: 0,
        blocked: 0,
        elapsed_ms: 3,
        eta_rough: false,
      },
    },
  });
  store.applyServerMessage({
    v: 1,
    seq: 10,
    type: "preview_policy",
    payload: { node: "deboss", port: "value", mode: "compute_on_release", estimate_ms: 4000, rough: false, pending_value: "1.1" },
  });
  store.applyServerMessage({ v: 1, seq: 11, type: "drag_ended", payload: { node: "deboss", port: "value" } });
  expect(held()).toEqual(before.held);
  expect(frameBus.received).toBe(before.received);
  expect(scene.framesReceived).toBe(before.applied);
  // The texts did what they do — in the store, not the ledger.
  expect(useCicada.getState().summary.generation).toBe(7);
});
