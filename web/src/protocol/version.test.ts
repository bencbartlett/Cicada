import { expect, test } from "vitest";
import { CATALOG_FORMAT, PROTOCOL_VERSION } from "./version";

test("protocol version is a non-negative integer", () => {
  expect(Number.isInteger(PROTOCOL_VERSION)).toBe(true);
  expect(PROTOCOL_VERSION).toBeGreaterThanOrEqual(0);
});

test("catalog format is a positive integer (the committed catalog.json carries the same number — catalog.test.ts)", () => {
  expect(Number.isInteger(CATALOG_FORMAT)).toBe(true);
  expect(CATALOG_FORMAT).toBeGreaterThanOrEqual(1);
});
