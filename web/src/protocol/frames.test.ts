import { expect, test } from "vitest";
import { FrameError, decodeFrame, encodeBatchForTest } from "./frames";

test("a batch frame decodes to aligned typed-array views", () => {
  const buffer = encodeBatchForTest(
    { kind: "mesh", generation: 7, node: 3, output: 1, elementStart: 0, elementCount: 2 },
    [
      { elementIndex: 0, pickId: 11, vertexStart: 0, vertexCount: 3, indexStart: 0, indexCount: 3 },
      { elementIndex: 1, pickId: 12, vertexStart: 3, vertexCount: 3, indexStart: 3, indexCount: 3 },
    ],
    [0, 0, 0, 1, 0, 0, 0, 1, 0, 5, 5, 5, 6, 5, 5, 5, 6, 5],
    [0, 1, 2, 3, 4, 5],
    [11, 11, 11, 12, 12, 12],
  );
  const frame = decodeFrame(buffer);
  expect(frame.header).toEqual({
    kind: "mesh",
    generation: 7,
    node: 3,
    output: 1,
    elementStart: 0,
    elementCount: 2,
  });
  if (!("elements" in frame)) throw new Error("mesh batch");
  expect(frame.elements[1]?.pickId).toBe(12);
  expect(frame.positions.byteOffset % 4).toBe(0);
  expect(frame.positions.buffer).toBe(buffer); // a view, not a copy
  expect(Array.from(frame.indices)).toEqual([0, 1, 2, 3, 4, 5]);
  expect(Array.from(frame.pickIds)).toEqual([11, 11, 11, 12, 12, 12]);
});

test("malformed frames are refused loudly", () => {
  expect(() => decodeFrame(new ArrayBuffer(8))).toThrow(FrameError);
  const buffer = encodeBatchForTest(
    { kind: "point", generation: 1, node: 1, output: 0, elementStart: 0, elementCount: 1 },
    [{ elementIndex: 0, pickId: 1, vertexStart: 0, vertexCount: 1, indexStart: 0, indexCount: 0 }],
    [1, 2, 3],
    [],
    [1],
  );
  const truncated = buffer.slice(0, buffer.byteLength - 2);
  expect(() => decodeFrame(truncated)).toThrow(/truncated/);
  new DataView(buffer).setUint32(0, 0xdeadbeef, true);
  expect(() => decodeFrame(buffer)).toThrow(/magic/);
});
