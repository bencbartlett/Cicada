/**
 * Edge-overlay scheduling policy (pure, unit-tested). `EdgesGeometry` is a
 * synchronous CPU pass over every triangle; on wall-scale meshes it blocked
 * the UI thread for hundreds of milliseconds per preview generation. So the
 * shaded mesh always attaches immediately and the overlay is:
 *
 * - built inline for small pieces (under `EDGE_SYNC_TRIANGLES`),
 * - deferred to an idle slot for larger ones (up to the memory cap
 *   `EDGE_TRIANGLE_LIMIT`), running only when the solve is quiet — no solve
 *   running, or at least `EDGE_QUIET_MS` since that output's last frame — and
 *   cancelled if the piece is dropped or replaced first,
 * - skipped above the cap.
 */

/** Crease angle (degrees) for the edge overlay. */
export const EDGE_THRESHOLD_DEG = 30;
/** Edge overlays are skipped above this many triangles per draw (memory cap). */
export const EDGE_TRIANGLE_LIMIT = 200_000;
/** Pieces under this many triangles build their edges synchronously with the mesh. */
export const EDGE_SYNC_TRIANGLES = 4_000;
/** While a solve runs, a deferred edge build waits this long after the output's last frame. */
export const EDGE_QUIET_MS = 150;

export type EdgePolicy = "inline" | "deferred" | "skip";

export function edgePolicy(
  triangles: number,
  syncBudget: number = EDGE_SYNC_TRIANGLES,
  limit: number = EDGE_TRIANGLE_LIMIT,
): EdgePolicy {
  if (triangles >= limit) return "skip";
  return triangles < syncBudget ? "inline" : "deferred";
}

/** May a deferred edge build run now? Quiet = no solve running, or the output's frames have paused. */
export function edgeBuildAllowed(
  solveRunning: boolean,
  msSinceLastFrame: number,
  quietMs: number = EDGE_QUIET_MS,
): boolean {
  return !solveRunning || msSinceLastFrame >= quietMs;
}
