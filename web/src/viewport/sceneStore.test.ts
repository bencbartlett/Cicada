import { expect, test } from "vitest";
import {
  decodeFrame,
  encodeBatchForTest,
  type BatchFrame,
  type FrameHeader,
  type InstancesFrame,
  type MeshBlobFrame,
} from "../protocol/frames";
import {
  SceneStore,
  boundsOfPositions,
  instancedBounds,
  outputKey,
  unionBounds,
} from "./sceneStore";

function header(kind: FrameHeader["kind"], generation: number, node = 7, output = 0): FrameHeader {
  return { kind, generation, node, output, elementStart: 0, elementCount: 1 };
}

/** One triangle at `origin`, one element with `pickId`. */
function triangle(
  kind: "mesh" | "curve" | "point",
  generation: number,
  pickId: number,
  origin: [number, number, number] = [0, 0, 0],
  node = 7,
): BatchFrame {
  const [x, y, z] = origin;
  const buffer = encodeBatchForTest(
    header(kind, generation, node),
    [{ elementIndex: 0, pickId, vertexStart: 0, vertexCount: 3, indexStart: 0, indexCount: 3 }],
    [x, y, z, x + 1, y, z, x, y + 1, z],
    kind === "point" ? [] : [0, 1, 2],
    [pickId, pickId, pickId],
  );
  return decodeFrame(buffer) as BatchFrame;
}

const IDENTITY = new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]);

function blob(hash: string, generation: number): MeshBlobFrame {
  return {
    header: header("mesh_blob", generation),
    hash,
    positions: new Float32Array([0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 2]),
    indices: new Uint32Array([0, 1, 2, 0, 1, 3]),
  };
}

function instances(hash: string, generation: number, picks: number[], node = 7): InstancesFrame {
  return {
    header: header("instances", generation, node),
    hash,
    instances: picks.map((pickId, elementIndex) => {
      const transform = new Float32Array(IDENTITY);
      transform[3] = elementIndex * 10; // translate x by 10 per instance
      return { elementIndex, pickId, transform };
    }),
  };
}

test("frames of the same generation accumulate; a newer generation replaces every kind", () => {
  const events: string[] = [];
  const store = new SceneStore({
    onOutput: (key, entry) => events.push(`${key}:${entry === null ? "gone" : entry.generation}`),
  });
  expect(store.apply(triangle("mesh", 3, 11))).toBe("replaced");
  expect(store.apply(triangle("curve", 3, 12))).toBe("accumulated");
  expect(store.apply(triangle("point", 3, 13))).toBe("accumulated");
  const entry = store.outputs.get(outputKey(7, 0));
  expect(entry?.generation).toBe(3);
  expect(Array.from(entry?.batches.keys() ?? [])).toEqual(["mesh", "curve", "point"]);
  const stats = store.statsOf(entry!);
  expect(stats.kinds).toEqual(["mesh", "curve", "point"]);
  expect(stats.triangles).toBe(1);
  expect(stats.segments).toBe(1);
  expect(stats.points).toBe(3);
  expect(stats.elements).toBe(3);

  // Generation 5 arrives with only a mesh: curve + point from gen 3 vanish.
  expect(store.apply(triangle("mesh", 5, 11, [4, 4, 4]))).toBe("replaced");
  const next = store.outputs.get(outputKey(7, 0));
  expect(next?.generation).toBe(5);
  expect(Array.from(next?.batches.keys() ?? [])).toEqual(["mesh"]);
  expect(store.statsOf(next!).bounds).toEqual([
    [4, 4, 4],
    [5, 5, 4],
  ]);
  expect(store.lastGeneration).toBe(5);
  expect(store.framesReceived).toBe(4);
  expect(events).toEqual(["7:0:3", "7:0:3", "7:0:3", "7:0:5"]);
});

test("stale frames are dropped by construction; equal-generation frames are idempotent", () => {
  const store = new SceneStore();
  store.apply(triangle("mesh", 5, 11));
  expect(store.apply(triangle("curve", 4, 12))).toBe("dropped");
  expect(store.apply(triangle("mesh", 2, 13))).toBe("dropped");
  const entry = store.outputs.get(outputKey(7, 0));
  expect(entry?.generation).toBe(5);
  expect(entry?.batches.size).toBe(1);
  // A re-stream (after display_reset) re-sends the same generation: one batch, not two.
  expect(store.apply(triangle("mesh", 5, 11))).toBe("accumulated");
  expect(entry?.batches.size).toBe(1);
  expect(store.framesReceived).toBe(4);
});

test("clear removes the output — also at the generation it was drawn with", () => {
  const gone: string[] = [];
  const store = new SceneStore({
    onOutput: (key, entry) => {
      if (entry === null) gone.push(key);
    },
  });
  store.apply(triangle("mesh", 5, 11));
  expect(store.apply({ header: header("clear", 4) })).toBe("dropped");
  expect(store.outputs.size).toBe(1);
  expect(store.apply({ header: header("clear", 5) })).toBe("cleared");
  expect(store.outputs.size).toBe(0);
  expect(gone).toEqual(["7:0"]);
  // Clearing something never drawn is a no-op, not an error.
  expect(store.apply({ header: header("clear", 9, 42) })).toBe("cleared");
  // A later generation can draw the output again.
  expect(store.apply(triangle("mesh", 6, 11))).toBe("replaced");
});

test("outputs are independent: keyed by node ref and output index", () => {
  const store = new SceneStore();
  store.apply(triangle("mesh", 9, 1, [0, 0, 0], 1));
  store.apply(triangle("mesh", 2, 2, [0, 0, 0], 2)); // older gen but another node
  const b = decodeFrame(
    encodeBatchForTest(
      { kind: "curve", generation: 9, node: 1, output: 1, elementStart: 0, elementCount: 1 },
      [
        {
          elementIndex: 0,
          pickId: 3,
          vertexStart: 0,
          vertexCount: 2,
          indexStart: 0,
          indexCount: 2,
        },
      ],
      [0, 0, 0, 1, 1, 1],
      [0, 1],
      [3, 3],
    ),
  );
  store.apply(b);
  expect(store.outputs.size).toBe(3);
  expect(store.outputs.get("1:0")?.generation).toBe(9);
  expect(store.outputs.get("2:0")?.generation).toBe(2);
  expect(store.outputs.get("1:1")?.batches.has("curve")).toBe(true);
  expect(store.bounds(new Set([2]))).toEqual([
    [0, 0, 0],
    [1, 1, 0],
  ]);
  expect(store.bounds()).toEqual([
    [0, 0, 0],
    [1, 1, 1],
  ]);
});

test("mesh blobs are content-addressed and instances build on them", () => {
  const blobs: string[] = [];
  const store = new SceneStore({ onBlob: (hash) => blobs.push(hash) });
  expect(store.apply(blob("aa", 3))).toBe("blob");
  expect(store.apply(blob("aa", 3))).toBe("blob-cached");
  expect(store.apply(blob("aa", 8))).toBe("blob-cached"); // same hash, later gen: never rebuilt
  expect(blobs).toEqual(["aa"]);
  expect(store.outputs.size).toBe(0); // blobs are not outputs

  expect(store.apply(instances("aa", 3, [21, 22, 23]))).toBe("replaced");
  const entry = store.outputs.get("7:0")!;
  const stats = store.statsOf(entry);
  expect(stats.kinds).toEqual(["instances"]);
  expect(stats.elements).toBe(3);
  expect(stats.instanced).toBe(3);
  expect(stats.triangles).toBe(6); // 2 per blob × 3
  expect(stats.vertices).toBe(4);
  expect(stats.bounds).toEqual([
    [0, 0, 0],
    [22, 2, 2],
  ]);
  // A second hash in the same generation accumulates alongside.
  store.apply(blob("bb", 3));
  expect(store.apply(instances("bb", 3, [31]))).toBe("accumulated");
  expect(entry.instances.size).toBe(2);
  // Re-sent instances for a hash replace, not duplicate.
  expect(store.apply(instances("aa", 3, [21, 22, 23]))).toBe("accumulated");
  expect(entry.instances.size).toBe(2);
  expect(store.statsOf(entry).elements).toBe(4);
});

test("pick ids resolve to (node ref, output, element) across kinds", () => {
  const store = new SceneStore();
  store.apply(triangle("mesh", 1, 11, [0, 0, 0], 4));
  store.apply(blob("cc", 1));
  store.apply(instances("cc", 1, [50, 51], 9));
  expect(store.resolvePick(11)).toEqual({ nodeRef: 4, output: 0, element: 0 });
  expect(store.resolvePick(51)).toEqual({ nodeRef: 9, output: 0, element: 1 });
  expect(store.resolvePick(0)).toBeNull();
  expect(store.resolvePick(999)).toBeNull();
});

test("reset drops drawables but keeps the blob cache", () => {
  let resets = 0;
  const store = new SceneStore({ onReset: () => (resets += 1) });
  store.apply(blob("dd", 1));
  store.apply(instances("dd", 1, [1]));
  store.apply(triangle("mesh", 1, 2));
  store.reset();
  expect(store.outputs.size).toBe(0);
  expect(store.blobs.has("dd")).toBe(true);
  expect(resets).toBe(1);
  expect(store.apply(blob("dd", 2))).toBe("blob-cached");
});

test("bounds helpers", () => {
  expect(boundsOfPositions(new Float32Array([]))).toBeNull();
  expect(boundsOfPositions(new Float32Array([1, 2, 3]))).toEqual([
    [1, 2, 3],
    [1, 2, 3],
  ]);
  expect(
    unionBounds(
      [
        [0, 0, 0],
        [1, 1, 1],
      ],
      [
        [-1, 2, 0.5],
        [0, 3, 0.5],
      ],
    ),
  ).toEqual([
    [-1, 0, 0],
    [1, 3, 1],
  ]);
  expect(unionBounds(null, null)).toBeNull();
  const shifted = new Float32Array(IDENTITY);
  shifted[11] = 5; // z += 5
  expect(
    instancedBounds(
      [
        [0, 0, 0],
        [1, 1, 1],
      ],
      [{ elementIndex: 0, pickId: 1, transform: shifted }],
    ),
  ).toEqual([
    [0, 0, 5],
    [1, 1, 6],
  ]);
});
