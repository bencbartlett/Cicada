import { describe, expect, it } from "vitest";
import { literalKindOf, paramValueText } from "./literals";

describe("literalKindOf (inline kwarg widgets)", () => {
  it("lets the port's base type decide for the scalar kinds", () => {
    expect(literalKindOf({ base: "Number", literal: "0.75", literal_value: 0.75 })).toBe("number");
    // `24` on an Integer port writes bare integers; `2` on a Number port keeps the point.
    expect(literalKindOf({ base: "Integer", literal: "24", literal_value: 24 })).toBe("integer");
    expect(literalKindOf({ base: "Number", literal: "2.0", literal_value: 2 })).toBe("number");
    expect(literalKindOf({ base: "Boolean", literal: "True", literal_value: true })).toBe("boolean");
    expect(literalKindOf({ base: "Text", literal: '"a"', literal_value: "a" })).toBe("text");
  });
  it("falls back to the literal's own spelling on other bases", () => {
    expect(literalKindOf({ base: "T", literal: "3", literal_value: 3 })).toBe("integer");
    expect(literalKindOf({ base: "T", literal: "3.0", literal_value: 3 })).toBe("number");
    expect(literalKindOf({ base: "Any", literal: "1e3", literal_value: 1000 })).toBe("number");
    expect(literalKindOf({ base: "Any", literal: "True", literal_value: true })).toBe("boolean");
  });
  it("offers no widget without a scalar value or on a type mismatch", () => {
    expect(literalKindOf({ base: "Number", literal: undefined, literal_value: undefined })).toBeNull();
    expect(literalKindOf({ base: "Integer", literal: '"x"', literal_value: "x" })).toBeNull();
    expect(literalKindOf({ base: "Boolean", literal: "1", literal_value: 1 })).toBeNull();
  });
  it("round-trips through the one spelling rule", () => {
    const radius = { base: "Number", literal: "0.75", literal_value: 0.75 };
    const segments = { base: "Integer", literal: "24", literal_value: 24 };
    expect(paramValueText(literalKindOf(radius)!, 1.25)).toBe("1.25");
    expect(paramValueText(literalKindOf(radius)!, 1)).toBe("1.0");
    expect(paramValueText(literalKindOf(segments)!, 16)).toBe("16");
  });
});
