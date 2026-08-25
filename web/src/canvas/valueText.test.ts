import { describe, expect, it } from "vitest";
import { compactNumber, compactValueText } from "./valueText";

describe("the node face's value text (U23)", () => {
  it("rounds a decimal to four significant figures and drops trailing zeros", () => {
    expect(compactNumber(6.283185307179586)).toBe("6.283");
    expect(compactNumber(0.1234567)).toBe("0.1235");
    expect(compactNumber(3.0)).toBe("3");
    expect(compactNumber(-12.345678)).toBe("-12.35");
    expect(compactNumber(123456.789)).toBe("123500");
    expect(compactNumber(0.000001)).toBe("0.000001");
    expect(compactNumber(1e-7)).toBe("1e-7");
    expect(compactNumber(Number.NaN)).toBe("NaN");
  });
  it("compacts every decimal in a summary and leaves integers and words alone", () => {
    expect(compactValueText("Number 6.283185307179586")).toBe("Number 6.283");
    expect(compactValueText("Point ×1000 · (0.1234567, 2.5, -3.0)")).toBe("Point ×1000 · (0.1235, 2.5, -3)");
    expect(compactValueText("Solid(4494 bytes)")).toBe("Solid(4494 bytes)");
    expect(compactValueText("Domain 0.0..6.283185307")).toBe("Domain 0..6.283");
    expect(compactValueText("1.5e-9 · 2E+3")).toBe("1.5e-9 · 2000");
    expect(compactValueText("—")).toBe("—");
  });
});
