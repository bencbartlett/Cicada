/**
 * Binary geometry frames — the byte-exact layout is documented in
 * `crates/cicada-server/src/frames.rs` (docs/13 §Binary frame format).
 * Little-endian; every array starts 4-byte aligned, so this decoder returns
 * typed-array VIEWS over the received ArrayBuffer (zero copy, GPU-ready).
 */

export const FRAME_MAGIC = 0x46434943; // "CICF"
export const FRAME_VERSION = 1;
export const HEADER_LEN = 32;

export type FrameKind = "mesh" | "curve" | "point" | "clear" | "mesh_blob" | "instances";

const KINDS: Record<number, FrameKind> = {
  1: "mesh",
  2: "curve",
  3: "point",
  4: "clear",
  5: "mesh_blob",
  6: "instances",
};

export interface FrameHeader {
  kind: FrameKind;
  generation: number;
  node: number;
  output: number;
  elementStart: number;
  elementCount: number;
}

export interface ElementEntry {
  elementIndex: number;
  pickId: number;
  vertexStart: number;
  vertexCount: number;
  indexStart: number;
  indexCount: number;
}

export interface BatchFrame {
  header: FrameHeader; // kind: mesh | curve | point
  elements: ElementEntry[];
  positions: Float32Array;
  indices: Uint32Array;
  pickIds: Uint32Array;
}

export interface ClearFrame {
  header: FrameHeader; // kind: clear
}

export interface MeshBlobFrame {
  header: FrameHeader; // kind: mesh_blob
  hash: string; // 64 hex chars
  positions: Float32Array;
  indices: Uint32Array;
}

export interface InstanceEntry {
  elementIndex: number;
  pickId: number;
  /** 3×4 row-major affine. */
  transform: Float32Array;
}

export interface InstancesFrame {
  header: FrameHeader; // kind: instances
  hash: string;
  instances: InstanceEntry[];
}

export type Frame = BatchFrame | ClearFrame | MeshBlobFrame | InstancesFrame;

export class FrameError extends Error {}

function hex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

/** Decode one frame. Throws `FrameError` on malformed bytes. */
export function decodeFrame(buffer: ArrayBuffer): Frame {
  if (buffer.byteLength < HEADER_LEN) {
    throw new FrameError(`frame shorter than its header (${buffer.byteLength} bytes)`);
  }
  const view = new DataView(buffer);
  const magic = view.getUint32(0, true);
  if (magic !== FRAME_MAGIC) throw new FrameError(`bad frame magic 0x${magic.toString(16)}`);
  const version = view.getUint16(4, true);
  if (version !== FRAME_VERSION) throw new FrameError(`unsupported frame version ${version}`);
  const kindRaw = view.getUint16(6, true);
  const kind = KINDS[kindRaw];
  if (kind === undefined) throw new FrameError(`unknown frame kind ${kindRaw}`);
  const header: FrameHeader = {
    kind,
    generation: Number(view.getBigUint64(8, true)),
    node: view.getUint32(16, true),
    output: view.getUint32(20, true),
    elementStart: view.getUint32(24, true),
    elementCount: view.getUint32(28, true),
  };
  const need = (bytes: number) => {
    if (bytes > buffer.byteLength) {
      throw new FrameError(`truncated frame body: needed ${bytes} bytes, had ${buffer.byteLength}`);
    }
  };
  switch (kind) {
    case "mesh":
    case "curve":
    case "point": {
      need(HEADER_LEN + 12);
      const e = view.getUint32(32, true);
      const v = view.getUint32(36, true);
      const i = view.getUint32(40, true);
      let at = 44;
      need(at + 24 * e + 12 * v + 4 * i + 4 * v);
      const elements: ElementEntry[] = new Array(e);
      for (let k = 0; k < e; k++) {
        elements[k] = {
          elementIndex: view.getUint32(at, true),
          pickId: view.getUint32(at + 4, true),
          vertexStart: view.getUint32(at + 8, true),
          vertexCount: view.getUint32(at + 12, true),
          indexStart: view.getUint32(at + 16, true),
          indexCount: view.getUint32(at + 20, true),
        };
        at += 24;
      }
      const positions = new Float32Array(buffer, at, 3 * v);
      at += 12 * v;
      const indices = new Uint32Array(buffer, at, i);
      at += 4 * i;
      const pickIds = new Uint32Array(buffer, at, v);
      return { header, elements, positions, indices, pickIds };
    }
    case "clear":
      return { header };
    case "mesh_blob": {
      need(HEADER_LEN + 40);
      const hash = hex(new Uint8Array(buffer, 32, 32));
      const v = view.getUint32(64, true);
      const i = view.getUint32(68, true);
      need(72 + 12 * v + 4 * i);
      const positions = new Float32Array(buffer, 72, 3 * v);
      const indices = new Uint32Array(buffer, 72 + 12 * v, i);
      return { header, hash, positions, indices };
    }
    case "instances": {
      need(HEADER_LEN + 36);
      const hash = hex(new Uint8Array(buffer, 32, 32));
      const n = view.getUint32(64, true);
      need(68 + 56 * n);
      const instances: InstanceEntry[] = new Array(n);
      let at = 68;
      for (let k = 0; k < n; k++) {
        instances[k] = {
          elementIndex: view.getUint32(at, true),
          pickId: view.getUint32(at + 4, true),
          transform: new Float32Array(buffer, at + 8, 12),
        };
        at += 56;
      }
      return { header, hash, instances };
    }
  }
}

/** Test helper: encode a batch frame the way the server does. */
export function encodeBatchForTest(
  header: FrameHeader,
  elements: ElementEntry[],
  positions: number[],
  indices: number[],
  pickIds: number[],
): ArrayBuffer {
  const v = positions.length / 3;
  const size = HEADER_LEN + 12 + 24 * elements.length + 12 * v + 4 * indices.length + 4 * v;
  const buffer = new ArrayBuffer(size);
  const view = new DataView(buffer);
  view.setUint32(0, FRAME_MAGIC, true);
  view.setUint16(4, FRAME_VERSION, true);
  const kindCode = { mesh: 1, curve: 2, point: 3, clear: 4, mesh_blob: 5, instances: 6 }[
    header.kind
  ];
  view.setUint16(6, kindCode, true);
  view.setBigUint64(8, BigInt(header.generation), true);
  view.setUint32(16, header.node, true);
  view.setUint32(20, header.output, true);
  view.setUint32(24, header.elementStart, true);
  view.setUint32(28, header.elementCount, true);
  view.setUint32(32, elements.length, true);
  view.setUint32(36, v, true);
  view.setUint32(40, indices.length, true);
  let at = 44;
  for (const e of elements) {
    view.setUint32(at, e.elementIndex, true);
    view.setUint32(at + 4, e.pickId, true);
    view.setUint32(at + 8, e.vertexStart, true);
    view.setUint32(at + 12, e.vertexCount, true);
    view.setUint32(at + 16, e.indexStart, true);
    view.setUint32(at + 20, e.indexCount, true);
    at += 24;
  }
  new Float32Array(buffer, at, 3 * v).set(positions);
  at += 12 * v;
  new Uint32Array(buffer, at, indices.length).set(indices);
  at += 4 * indices.length;
  new Uint32Array(buffer, at, v).set(pickIds);
  return buffer;
}
