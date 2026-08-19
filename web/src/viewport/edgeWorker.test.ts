import { describe, expect, it } from "vitest";
import { EDGE_THRESHOLD_DEG } from "./edgePolicy";
import { edgePositions } from "./edgeWorker";

/** A unit cube: 8 vertices, 12 triangles. */
function cube(): { positions: Float32Array; indices: Uint32Array } {
  const positions = new Float32Array([
    0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1,
  ]);
  const indices = new Uint32Array([
    0, 2, 1, 0, 3, 2, // bottom
    4, 5, 6, 4, 6, 7, // top
    0, 1, 5, 0, 5, 4, // front
    1, 2, 6, 1, 6, 5, // right
    2, 3, 7, 2, 7, 6, // back
    3, 0, 4, 3, 4, 7, // left
  ]);
  return { positions, indices };
}

describe("edgePositions (the worker's maths, run inline)", () => {
  it("finds the 12 crease edges of a cube as 24 packed vertices", () => {
    const { positions, indices } = cube();
    const out = edgePositions(positions, indices, EDGE_THRESHOLD_DEG);
    expect(out).toBeInstanceOf(Float32Array);
    // 12 edges × 2 vertices × 3 coordinates.
    expect(out.length).toBe(12 * 2 * 3);
    // Every edge vertex is a cube corner (coordinates in {0, 1}).
    for (const v of out) expect(v === 0 || v === 1).toBe(true);
  });
  it("finds no crease edges on a flat, coplanar quad", () => {
    const positions = new Float32Array([0, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0]);
    const indices = new Uint32Array([0, 1, 2, 0, 2, 3]);
    const out = edgePositions(positions, indices, EDGE_THRESHOLD_DEG);
    // The outline (boundary edges) is kept by EdgesGeometry: 4 edges; the
    // shared diagonal (0°) is not.
    expect(out.length).toBe(4 * 2 * 3);
  });
});
