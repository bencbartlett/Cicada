/**
 * Pick-id ↔ RGBA8 encoding for the ID-buffer pass (docs/04 backward
 * picking). Ids are stable server-side integers below 2^24, so three bytes
 * carry them exactly; 0 means "nothing".
 */

export const PICK_ID_MAX = 0xffffff;

/** Encode a pick id into the three color channels the pick shader writes. */
export function encodePickId(id: number): [number, number, number] {
  if (!Number.isInteger(id) || id < 0 || id > PICK_ID_MAX) {
    throw new RangeError(`pick id ${id} does not fit RGB8`);
  }
  return [id & 0xff, (id >> 8) & 0xff, (id >> 16) & 0xff];
}

/** Decode the pixel read back from the pick target (RGBA8, alpha ignored). */
export function decodePickPixel(rgba: ArrayLike<number>): number {
  const r = rgba[0] ?? 0;
  const g = rgba[1] ?? 0;
  const b = rgba[2] ?? 0;
  return r | (g << 8) | (b << 16);
}

/**
 * The GLSL that mirrors `encodePickId` (used by the pick override
 * material). `id` is a float carrying an exact integer < 2^24.
 */
export const GLSL_ENCODE_PICK = /* glsl */ `
vec4 cicadaEncodePick(float id) {
  float r = mod(id, 256.0);
  float g = mod(floor(id / 256.0), 256.0);
  float b = floor(id / 65536.0);
  return vec4(r / 255.0, g / 255.0, b / 255.0, 1.0);
}
`;
