/**
 * Edge-overlay worker: `EdgesGeometry` is a synchronous CPU pass over every
 * triangle (≈120 ms for 80k triangles, 400+ ms at wall scale), so large
 * pieces compute their crease edges HERE, off the UI thread. In: a copy of
 * the mesh's positions + indices (transferred); out: the edge segments'
 * packed xyz positions (transferred back). No DOM, no store — three.js
 * maths only.
 */
import { BufferAttribute, BufferGeometry, EdgesGeometry } from "three";

export interface EdgeRequest {
  id: number;
  positions: Float32Array;
  indices: Uint32Array;
  thresholdDeg: number;
}

export interface EdgeResponse {
  id: number;
  /** Packed xyz pairs (2 vertices per segment). */
  positions: Float32Array;
  ms: number;
}

/** Pure: the edge segments of an indexed triangle mesh (used by the worker and testable without one). */
export function edgePositions(positions: Float32Array, indices: Uint32Array, thresholdDeg: number): Float32Array {
  const geometry = new BufferGeometry();
  geometry.setAttribute("position", new BufferAttribute(positions, 3));
  geometry.setIndex(new BufferAttribute(indices, 1));
  const edges = new EdgesGeometry(geometry, thresholdDeg);
  const attribute = edges.getAttribute("position");
  const out = attribute.array instanceof Float32Array ? attribute.array : Float32Array.from(attribute.array);
  edges.dispose();
  geometry.dispose();
  return out;
}

interface WorkerScope {
  addEventListener(type: "message", listener: (event: MessageEvent<EdgeRequest>) => void): void;
  postMessage(message: EdgeResponse, transfer: Transferable[]): void;
}

// Only wire the message loop when actually running as a worker (the module
// is also imported by unit tests for `edgePositions`).
const scope =
  typeof self === "undefined" ? null : (self as unknown as Partial<WorkerScope> & { document?: unknown });
if (scope !== null && typeof scope.addEventListener === "function" && scope.document === undefined) {
  scope.addEventListener("message", (event) => {
    const { id, positions, indices, thresholdDeg } = event.data;
    const t0 = performance.now();
    const out = edgePositions(positions, indices, thresholdDeg);
    scope.postMessage?.({ id, positions: out, ms: performance.now() - t0 }, [out.buffer]);
  });
}
