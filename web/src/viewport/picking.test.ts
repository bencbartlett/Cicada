import { expect, test } from "vitest";
import { PICK_ID_MAX, decodePickPixel, encodePickId } from "./picking";

test("pick ids round-trip through RGB8", () => {
  for (const id of [0, 1, 2, 255, 256, 65535, 65536, 0x123456, PICK_ID_MAX]) {
    const [r, g, b] = encodePickId(id);
    expect(decodePickPixel([r, g, b, 255])).toBe(id);
  }
});

test("the encoding matches what the shader writes (channel = value / 255)", () => {
  // Shader: r = mod(id,256), g = mod(floor(id/256),256), b = floor(id/65536).
  const id = 0x0a0b0c;
  expect(encodePickId(id)).toEqual([0x0c, 0x0b, 0x0a]);
  // Alpha is ignored; a cleared pixel is "nothing".
  expect(decodePickPixel([0, 0, 0, 0])).toBe(0);
});

test("ids outside RGB8 are refused loudly", () => {
  expect(() => encodePickId(-1)).toThrow(RangeError);
  expect(() => encodePickId(PICK_ID_MAX + 1)).toThrow(RangeError);
  expect(() => encodePickId(1.5)).toThrow(RangeError);
});
