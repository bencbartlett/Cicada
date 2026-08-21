import { describe, expect, it } from "vitest";
import { baseOfType, kindColor, kindFamily } from "./kinds";

describe("kind families", () => {
  it("gives the B-rep Solid its own hue, distinct from the mesh tier", () => {
    expect(kindFamily("Solid")).toBe("solid");
    expect(kindColor("Solid")).toBe("var(--kind-solid)");
    expect(kindFamily("Mesh")).toBe("mesh");
    expect(kindFamily("Watertight<Mesh>")).toBe("mesh");
    expect(kindFamily("Solid")).not.toBe(kindFamily("Watertight<Mesh>"));
    // Lists and optionals of solids share the hue through the base type.
    expect(kindFamily(baseOfType("[Solid?]"))).toBe("solid");
  });
  it("keeps unknown kinds and type variables neutral", () => {
    expect(kindFamily("T")).toBe("neutral");
    expect(kindFamily("Nonesuch")).toBe("neutral");
  });
});
