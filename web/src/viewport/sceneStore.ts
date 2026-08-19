/**
 * The viewport's frame ledger — pure TypeScript, no three.js, so the
 * generation rules are unit-testable with synthetic frames.
 *
 * Keyed by `nodeRef:output`. Each entry holds the drawables of the NEWEST
 * generation applied for that output (docs/13 §Binary frame format,
 * docs/04 §partial results):
 *
 * - a frame OLDER than the newest applied generation for its (node, output)
 *   is dropped — stale frames are unpaintable by construction;
 * - a frame with a NEWER generation replaces every kind previously held;
 * - frames of the SAME generation accumulate (one generation may send a
 *   mesh + a curve + a point batch for one output; instances arrive one
 *   frame per mesh hash) — re-applying the same frame is idempotent, so a
 *   server re-stream after `display_reset` paints exactly once;
 * - `clear` removes the output's drawables (also at an equal generation:
 *   the server clears a vanished node with the generation it last drew);
 * - `mesh_blob` is content-addressed: cached by hash, never rebuilt.
 *
 * Pick ids are session-stable on the server, so the pick table only grows.
 */
import type {
  BatchFrame,
  Frame,
  FrameKind,
  InstanceEntry,
  InstancesFrame,
  MeshBlobFrame,
} from "../protocol/frames";

export type Bounds = [[number, number, number], [number, number, number]];

export type BatchKind = "mesh" | "curve" | "point";

export interface OutputEntry {
  nodeRef: number;
  output: number;
  generation: number;
  /** One batch per kind (the server sends at most one per generation). */
  batches: Map<BatchKind, BatchFrame>;
  /** Instances keyed by mesh hash. */
  instances: Map<string, InstancesFrame>;
}

export interface PickTarget {
  nodeRef: number;
  output: number;
  element: number;
}

export type ApplyResult =
  "dropped" | "replaced" | "accumulated" | "cleared" | "blob" | "blob-cached";

export interface OutputStats {
  generation: number;
  kinds: string[];
  elements: number;
  vertices: number;
  triangles: number;
  segments: number;
  points: number;
  instanced: number;
  bounds: Bounds | null;
}

export function outputKey(nodeRef: number, output: number): string {
  return `${nodeRef}:${output}`;
}

export function isBatchKind(kind: FrameKind): kind is BatchKind {
  return kind === "mesh" || kind === "curve" || kind === "point";
}

/** Axis-aligned bounds of packed xyz positions (null when empty). */
export function boundsOfPositions(positions: Float32Array): Bounds | null {
  if (positions.length < 3) return null;
  let minX = Infinity;
  let minY = Infinity;
  let minZ = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  let maxZ = -Infinity;
  for (let i = 0; i + 2 < positions.length; i += 3) {
    const x = positions[i] as number;
    const y = positions[i + 1] as number;
    const z = positions[i + 2] as number;
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
    if (z < minZ) minZ = z;
    if (z > maxZ) maxZ = z;
  }
  return [
    [minX, minY, minZ],
    [maxX, maxY, maxZ],
  ];
}

/** Union of two bounds (either may be null). */
export function unionBounds(a: Bounds | null, b: Bounds | null): Bounds | null {
  if (a === null) return b === null ? null : [[...b[0]], [...b[1]]];
  if (b === null) return [[...a[0]], [...a[1]]];
  return [
    [Math.min(a[0][0], b[0][0]), Math.min(a[0][1], b[0][1]), Math.min(a[0][2], b[0][2])],
    [Math.max(a[1][0], b[1][0]), Math.max(a[1][1], b[1][1]), Math.max(a[1][2], b[1][2])],
  ];
}

/** Apply a 3×4 row-major affine to a point. */
export function transformPoint(
  m: Float32Array,
  x: number,
  y: number,
  z: number,
): [number, number, number] {
  const r = (i: number) => m[i] as number;
  return [
    r(0) * x + r(1) * y + r(2) * z + r(3),
    r(4) * x + r(5) * y + r(6) * z + r(7),
    r(8) * x + r(9) * y + r(10) * z + r(11),
  ];
}

/** Bounds of a blob's bounds pushed through every instance transform. */
export function instancedBounds(blob: Bounds | null, instances: InstanceEntry[]): Bounds | null {
  if (blob === null || instances.length === 0) return null;
  let out: Bounds | null = null;
  const corners: [number, number, number][] = [];
  for (const x of [blob[0][0], blob[1][0]]) {
    for (const y of [blob[0][1], blob[1][1]]) {
      for (const z of [blob[0][2], blob[1][2]]) corners.push([x, y, z]);
    }
  }
  for (const inst of instances) {
    for (const [x, y, z] of corners) {
      const p = transformPoint(inst.transform, x, y, z);
      out = unionBounds(out, [p, p]);
    }
  }
  return out;
}

export interface SceneStoreEvents {
  /** An output's drawables changed (replaced / accumulated / cleared). */
  onOutput?: (key: string, entry: OutputEntry | null) => void;
  /** A blob arrived (already cached blobs do not fire). */
  onBlob?: (hash: string, blob: MeshBlobFrame) => void;
  /** Everything was dropped (display reset). */
  onReset?: () => void;
}

export class SceneStore {
  readonly outputs = new Map<string, OutputEntry>();
  readonly blobs = new Map<string, MeshBlobFrame>();
  readonly picks = new Map<number, PickTarget>();
  private blobBounds = new Map<string, Bounds | null>();
  private listeners = new Set<SceneStoreEvents>();
  framesReceived = 0;
  lastGeneration = 0;

  constructor(events?: SceneStoreEvents) {
    if (events !== undefined) this.listeners.add(events);
  }

  /** Listen for changes (a scene mounting later rebuilds from `outputs` first). */
  subscribe(events: SceneStoreEvents): () => void {
    this.listeners.add(events);
    return () => {
      this.listeners.delete(events);
    };
  }

  private emitOutput(key: string, entry: OutputEntry | null): void {
    for (const l of this.listeners) l.onOutput?.(key, entry);
  }

  apply(frame: Frame): ApplyResult {
    this.framesReceived += 1;
    const h = frame.header;
    if (h.generation > this.lastGeneration) this.lastGeneration = h.generation;

    if (h.kind === "mesh_blob") {
      const blob = frame as MeshBlobFrame;
      if (this.blobs.has(blob.hash)) return "blob-cached";
      this.blobs.set(blob.hash, blob);
      this.blobBounds.set(blob.hash, boundsOfPositions(blob.positions));
      for (const l of this.listeners) l.onBlob?.(blob.hash, blob);
      return "blob";
    }

    const key = outputKey(h.node, h.output);
    const existing = this.outputs.get(key);
    if (existing !== undefined && h.generation < existing.generation) return "dropped";

    if (h.kind === "clear") {
      if (existing === undefined) return "cleared";
      this.outputs.delete(key);
      this.emitOutput(key, null);
      return "cleared";
    }

    let entry: OutputEntry;
    let result: ApplyResult;
    if (existing === undefined || h.generation > existing.generation) {
      entry = {
        nodeRef: h.node,
        output: h.output,
        generation: h.generation,
        batches: new Map(),
        instances: new Map(),
      };
      this.outputs.set(key, entry);
      result = "replaced";
    } else {
      entry = existing;
      result = "accumulated";
    }

    if (isBatchKind(h.kind)) {
      const batch = frame as BatchFrame;
      entry.batches.set(h.kind, batch);
      for (const e of batch.elements) {
        this.picks.set(e.pickId, { nodeRef: h.node, output: h.output, element: e.elementIndex });
      }
    } else {
      const inst = frame as InstancesFrame;
      entry.instances.set(inst.hash, inst);
      for (const e of inst.instances) {
        this.picks.set(e.pickId, { nodeRef: h.node, output: h.output, element: e.elementIndex });
      }
    }
    this.emitOutput(key, entry);
    return result;
  }

  /** Drop every drawable (blobs stay cached: content-addressed). */
  reset(): void {
    this.outputs.clear();
    for (const l of this.listeners) l.onReset?.();
  }

  resolvePick(pickId: number): PickTarget | null {
    return this.picks.get(pickId) ?? null;
  }

  blobBoundsOf(hash: string): Bounds | null {
    return this.blobBounds.get(hash) ?? null;
  }

  statsOf(entry: OutputEntry): OutputStats {
    const stats: OutputStats = {
      generation: entry.generation,
      kinds: [],
      elements: 0,
      vertices: 0,
      triangles: 0,
      segments: 0,
      points: 0,
      instanced: 0,
      bounds: null,
    };
    for (const [kind, batch] of entry.batches) {
      stats.kinds.push(kind);
      stats.elements += batch.elements.length;
      const v = batch.positions.length / 3;
      stats.vertices += v;
      if (kind === "mesh") stats.triangles += Math.floor(batch.indices.length / 3);
      else if (kind === "curve") stats.segments += Math.floor(batch.indices.length / 2);
      else stats.points += v;
      stats.bounds = unionBounds(stats.bounds, boundsOfPositions(batch.positions));
    }
    for (const [hash, inst] of entry.instances) {
      if (!stats.kinds.includes("instances")) stats.kinds.push("instances");
      const blob = this.blobs.get(hash);
      const n = inst.instances.length;
      stats.elements += n;
      stats.instanced += n;
      if (blob !== undefined) {
        stats.vertices += blob.positions.length / 3;
        stats.triangles += Math.floor(blob.indices.length / 3) * n;
        stats.bounds = unionBounds(
          stats.bounds,
          instancedBounds(this.blobBoundsOf(hash), inst.instances),
        );
      }
    }
    return stats;
  }

  /** Union bounds of every drawn output (optionally only some node refs). */
  bounds(nodeRefs?: Set<number>): Bounds | null {
    let out: Bounds | null = null;
    for (const entry of this.outputs.values()) {
      if (nodeRefs !== undefined && !nodeRefs.has(entry.nodeRef)) continue;
      out = unionBounds(out, this.statsOf(entry).bounds);
    }
    return out;
  }
}
