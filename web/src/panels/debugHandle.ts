/**
 * Read-side access to the `window.__cicada` debug handle (installed by the
 * connection module) without widening the global `Window` type — other
 * folders cast the same way, and duplicate global declarations would clash.
 */

export interface FrameCounters {
  received: number;
  bytes: number;
  /** `performance.now()` of the last frame received (0 before any). */
  lastAt: number;
  /** Highest generation any frame carried (0 before any). */
  lastGeneration: number;
}

export function readFrameCounters(): FrameCounters | null {
  const handle = (window as unknown as { __cicada?: { frames?: () => FrameCounters } }).__cicada;
  if (handle === undefined || typeof handle.frames !== "function") return null;
  return handle.frames();
}
