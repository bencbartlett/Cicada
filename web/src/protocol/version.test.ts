import { expect, test } from "vitest";
import { PROTOCOL_VERSION } from "./version";

test("protocol version is a non-negative integer", () => {
  expect(Number.isInteger(PROTOCOL_VERSION)).toBe(true);
  expect(PROTOCOL_VERSION).toBeGreaterThanOrEqual(0);
});
