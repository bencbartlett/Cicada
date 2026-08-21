/**
 * The transport's client side (docs/13 §Animation transport; docs/17 item
 * 4): what the store keeps of the last `TransportView` it heard, the
 * playhead extrapolation the play bar shows between broadcasts, and the
 * speed choices. Nothing here decides semantics the server owns — the view
 * is replaced whole by every snapshot and `transport` message, and every
 * control goes out as an intent whose refusal says why.
 */
import type { DrivenView, TransportView } from "../protocol/messages";

/** The store's transport slice: the last view, whole, and when it arrived. */
export interface TransportState {
  /** The last `TransportView` (a snapshot's `transport` or a `transport` broadcast) — never merged, always replaced. */
  view: TransportView;
  /** `nowMs()` when it arrived — the anchor the display extrapolates from while playing. */
  receivedAt: number;
}

/** The display clock (monotonic milliseconds) — `receivedAt` and the bar's ticker read the same one. */
export function nowMs(): number {
  return performance.now();
}

/** The view at rest with nothing to drive — what the store holds before the first snapshot says otherwise. */
export const TRANSPORT_AT_REST: TransportView = {
  playing: false,
  speed: 1,
  t_ms: 0,
  frame: 0,
  frames: 120,
  period_ms: 4000,
  driven: [],
};

/**
 * Does the pipeline have a time param the transport drives? `driven` empty
 * = no `cycle` / `clock` lowered (none in the text, or red): playback
 * would move nothing, so the bar is not shown and Space says why.
 */
export function hasTimeParams(transport: Pick<TransportState, "view"> | null): boolean {
  return transport !== null && transport.view.driven.length > 0;
}

/**
 * The primary loop's frame at playhead `tMs`: `floor(t × frames / period)
 * mod frames` — the server's quantization (`lower.rs` `Playhead::frame`),
 * in the same double arithmetic (`period_ms` on the wire IS the server's
 * `period × 1000`). The loop is the server's and always positive; anything
 * else is a protocol fault, thrown rather than rendered as frame 0.
 */
export function frameAt(tMs: number, frames: number, periodMs: number): number {
  if (!(Number.isInteger(frames) && frames > 0) || !(periodMs > 0)) {
    throw new RangeError(`transport loop must be positive: frames ${frames}, period_ms ${periodMs}`);
  }
  const raw = Math.floor((tMs * frames) / periodMs);
  return ((raw % frames) + frames) % frames;
}

/**
 * The driven entry of `node.port` in the view — the port is in the current
 * graph's driven set (the transport is feeding it) — or `undefined` when
 * the node is red / not in the graph / the view is gone.
 */
export function drivenEntry(view: TransportView, node: string, port: string): DrivenView | undefined {
  return view.driven.find((d) => d.node === node && d.port === port);
}

/**
 * What the transport feeds one driven port at playhead `tMs`, as the
 * inspector shows it: a `frame` port's frame of ITS OWN loop — `frame 3 of
 * 60`, `frameAt` on the `loop` the server sent with the entry, the numbers
 * the lowering quantized the injected frame from — never the primary
 * loop's frame (a second `cycle` loops inside the primary at its own rate:
 * at the primary's frame 10 of 40 over 8 s, a 60-frame / 2 s `cycle` is at
 * frame 0 of 60); a `time` port's playhead in seconds (`2.00 s`).
 */
export function fedValue(driven: DrivenView, tMs: number): string {
  if (driven.signal === "frame") {
    return `frame ${frameAt(tMs, driven.loop.frames, driven.loop.period_ms)} of ${driven.loop.frames}`;
  }
  return formatPlayhead(tMs);
}

/** The playhead the bar displays: the server's position advanced by the wall time since it was heard, at `speed`. */
export interface Playhead {
  tMs: number;
  frame: number;
}

/**
 * Extrapolate the playhead to `nowMs`: `t_ms + (now − receivedAt) × speed`
 * while playing (docs/13: the view is a position at the moment of the
 * message; the client trusts the next broadcast), the server's own `t_ms`
 * and `frame` while paused — a paused view is exact, nothing to compute.
 * A clock that reads earlier than `receivedAt` (never, for one monotonic
 * clock) counts as no time elapsed.
 */
export function playheadAt(transport: TransportState, nowMs: number): Playhead {
  const { view, receivedAt } = transport;
  if (!view.playing) return { tMs: view.t_ms, frame: view.frame };
  const tMs = view.t_ms + Math.max(0, nowMs - receivedAt) * view.speed;
  return { tMs, frame: frameAt(tMs, view.frames, view.period_ms) };
}

/** The play bar's speed menu (docs/17 item 4): quarter to four times. */
export const SPEED_CHOICES: readonly number[] = [0.25, 0.5, 1, 2, 4];

/**
 * The speeds the menu offers: the fixed choices plus the server's current
 * speed when it is none of them (another client, an agent on the socket —
 * the menu must show what IS, never a choice that is not). Ascending.
 */
export function speedChoices(current: number): number[] {
  const choices = SPEED_CHOICES.includes(current) ? [...SPEED_CHOICES] : [...SPEED_CHOICES, current];
  return choices.sort((a, b) => a - b);
}

/** `0.25×`, `1×`, `2×` — a non-menu speed keeps up to three decimals (`1.333×`). */
export function formatSpeed(speed: number): string {
  const text = Number.isInteger(speed) ? String(speed) : speed.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
  return `${text}×`;
}

/** The playhead in seconds for the counter: `1.25 s` (two decimals — frames are tens of ms apart). */
export function formatPlayhead(tMs: number): string {
  return `${(tMs / 1000).toFixed(2)} s`;
}

/** How often the bar re-reads the clock while playing: 30 Hz — the counter and the thumb, not the geometry (that is the server's frame rate). */
export const DISPLAY_TICK_MS = 33;
