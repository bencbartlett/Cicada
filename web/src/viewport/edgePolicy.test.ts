import { describe, expect, it } from "vitest";
import {
  EDGE_QUIET_MS,
  EDGE_SYNC_TRIANGLES,
  EDGE_TRIANGLE_LIMIT,
  edgeBuildAllowed,
  edgePolicy,
} from "./edgePolicy";

describe("edgePolicy", () => {
  it("builds small overlays inline, defers large ones, skips above the cap", () => {
    expect(edgePolicy(0)).toBe("inline");
    expect(edgePolicy(EDGE_SYNC_TRIANGLES - 1)).toBe("inline");
    expect(edgePolicy(EDGE_SYNC_TRIANGLES)).toBe("deferred");
    expect(edgePolicy(80_000)).toBe("deferred");
    expect(edgePolicy(EDGE_TRIANGLE_LIMIT - 1)).toBe("deferred");
    expect(edgePolicy(EDGE_TRIANGLE_LIMIT)).toBe("skip");
  });
  it("takes explicit budgets", () => {
    expect(edgePolicy(10, 5, 100)).toBe("deferred");
    expect(edgePolicy(10, 50, 100)).toBe("inline");
    expect(edgePolicy(100, 50, 100)).toBe("skip");
  });
});

describe("edgeBuildAllowed", () => {
  it("runs freely when no solve is running", () => {
    expect(edgeBuildAllowed(false, 0)).toBe(true);
  });
  it("waits for the output's frames to pause while a solve runs", () => {
    expect(edgeBuildAllowed(true, 0)).toBe(false);
    expect(edgeBuildAllowed(true, EDGE_QUIET_MS - 1)).toBe(false);
    expect(edgeBuildAllowed(true, EDGE_QUIET_MS)).toBe(true);
    expect(edgeBuildAllowed(true, 10, 5)).toBe(true);
  });
});
