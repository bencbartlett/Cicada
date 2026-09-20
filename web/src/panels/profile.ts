/**
 * The profiler's pure half (docs/13 §The profiler; docs/16 §Inspector
 * contents; v0.1 wave 5 P1): the client's own phases of a generation's
 * display, the ring's arcs (no chart dependency — plain SVG paths), and the
 * node table's sort and filter. No React, no store — unit-tested.
 */
import type { ProfileNode, ProfileView } from "../protocol/messages";
import type { GenerationFrames } from "../state/frameBus";
import type { DisplayPass } from "../state/store";

// ------------------------------------------------------- client phases --

/**
 * What the client itself did with a generation's frames, beside the server's
 * phases (`ProfileView.phases`). Measured per generation by the frame bus
 * as the frames arrive (`GenerationFrames`) and by the viewport's paint
 * stamp (`DisplayPass.paintedMs`); nothing here is sent to the server.
 */
export interface ClientPhases {
  /** Wall milliseconds decoding the frames (the socket measures it around `decodeFrame`). */
  decode_ms: number;
  /** Wall milliseconds applying them — the scene building its geometry for the GPU. */
  upload_ms: number;
  /**
   * `display_begin` to the first render after the last frame applied — the
   * viewport indicator's `painted in`; null until that render, and for a
   * pass other than the profiled generation's.
   */
  first_paint_ms: number | null;
  /**
   * The socket's share: what remains of the client's wall from
   * `display_begin` to the last frame applied once the server's
   * tessellation and encode and the client's decode and upload are taken
   * out — transfer and queueing (clamped at 0: `display_begin` itself
   * crosses the wire). Null when the pass sent no frame the client saw, or
   * the pass at hand is another generation's.
   */
  socket_ms: number | null;
  /** The pass's bytes over `socket_ms`, bytes per millisecond; null when the socket time is null or 0. */
  rate_bytes_per_ms: number | null;
  /** Frames the client saw for the generation. */
  frames: number;
}

/**
 * The client's phases of `view`'s generation from the frame bus's record of
 * its frames and the display pass the store holds (the same generation, or
 * not — then only what the frames alone say).
 */
export function clientPhases(view: ProfileView, frames: GenerationFrames | null, pass: DisplayPass | null): ClientPhases {
  const same = pass !== null && pass.generation === view.generation ? pass : null;
  const first_paint_ms = same?.paintedMs ?? null;
  if (frames === null || frames.frames === 0) {
    return { decode_ms: 0, upload_ms: 0, first_paint_ms, socket_ms: null, rate_bytes_per_ms: null, frames: 0 };
  }
  const decode_ms = frames.decodeMs;
  const upload_ms = frames.applyMs;
  let socket_ms: number | null = null;
  if (same !== null) {
    const wall = frames.lastAt - same.beganAt;
    socket_ms = Math.max(0, wall - view.phases.tessellate_ms - view.phases.encode_ms - decode_ms - upload_ms);
  }
  const rate_bytes_per_ms = socket_ms !== null && socket_ms > 0 ? view.phases.bytes / socket_ms : null;
  return { decode_ms, upload_ms, first_paint_ms, socket_ms, rate_bytes_per_ms, frames: frames.frames };
}

// ----------------------------------------------------------------- ring --

/** What an arc of the ring stands for: a node's compute, the rest of the nodes, a server phase, a client phase. */
export type RingKind = "node" | "other" | "server" | "client";

/** One phase the ring may draw — its wall (or, for a node, its measured work) in milliseconds and its colour token. */
export interface RingPhase {
  id: string;
  label: string;
  ms: number;
  /** A CSS colour — the profiler's `--prof-*` tokens (`panels.css`), themed for both surfaces. */
  color: string;
  kind: RingKind;
}

/** A drawn arc: the phase, its share of the ring, its angles (radians from 12 o'clock, clockwise) and its SVG path. */
export interface RingArc extends RingPhase {
  share: number;
  start: number;
  end: number;
  /** A stroked arc (`fill: none`, `stroke-width` = the ring's width). */
  d: string;
}

/** The ring's geometry in a 120 × 120 view box: the stroke's centre-line radius and its width. */
export const RING_SIZE = 120;
export const RING_RADIUS = 46;
export const RING_WIDTH = 14;
/** The surface gap between adjacent arcs, in view-box pixels (the mark spec's 2 px). */
export const RING_GAP_PX = 2;
/** The least an arc is drawn, in view-box pixels along the ring: a phase that took time is never invisible. */
export const RING_MIN_ARC_PX = 0.75;

/** How many nodes the ring draws on their own before the rest fold into "other nodes" (the contract's eight). */
export const RING_TOP_NODES = 8;

const TWO_PI = Math.PI * 2;
const TOP = -Math.PI / 2;

function point(radius: number, angle: number): [number, number] {
  const c = RING_SIZE / 2;
  return [c + radius * Math.cos(angle), c + radius * Math.sin(angle)];
}

function fmt(n: number): string {
  return Number(n.toFixed(3)).toString();
}

/** A stroked arc from `start` to `end` (radians, clockwise); a full turn is two half arcs (one SVG arc cannot close on itself). */
function arcPath(radius: number, start: number, end: number): string {
  const sweep = end - start;
  if (sweep >= TWO_PI - 1e-9) {
    const [x0, y0] = point(radius, start);
    const [x1, y1] = point(radius, start + Math.PI);
    return `M ${fmt(x0)} ${fmt(y0)} A ${radius} ${radius} 0 1 1 ${fmt(x1)} ${fmt(y1)} A ${radius} ${radius} 0 1 1 ${fmt(x0)} ${fmt(y0)}`;
  }
  const [x0, y0] = point(radius, start);
  const [x1, y1] = point(radius, end);
  const large = sweep > Math.PI ? 1 : 0;
  return `M ${fmt(x0)} ${fmt(y0)} A ${radius} ${radius} 0 ${large} 1 ${fmt(x1)} ${fmt(y1)}`;
}

/**
 * The ring's arcs for `phases`, in the order given: a phase with no time
 * (zero, negative, not finite) draws nothing; the rest share the turn by
 * their milliseconds — contiguous from 12 o'clock, clockwise — and every
 * arc is shortened by half the surface gap at each end (the mark spec's
 * 2 px between adjacent fills), except a lone arc, which is the whole
 * ring; a sliver is still drawn at least `RING_MIN_ARC_PX` long (a phase
 * that took time is never invisible — the path's coordinates are rounded
 * to a thousandth, and a true sub-pixel sweep would round to a point).
 * Empty when nothing took time.
 */
export function ringArcs(phases: readonly RingPhase[], radius = RING_RADIUS, gapPx = RING_GAP_PX): RingArc[] {
  const drawn = phases.filter((p) => Number.isFinite(p.ms) && p.ms > 0);
  const total = drawn.reduce((sum, p) => sum + p.ms, 0);
  if (total <= 0) return [];
  const gap = drawn.length > 1 ? gapPx / radius : 0;
  const minSweep = RING_MIN_ARC_PX / radius;
  let angle = TOP;
  return drawn.map((phase) => {
    const share = phase.ms / total;
    const start = angle;
    const end = start + TWO_PI * share;
    angle = end;
    const half = Math.min(gap / 2, Math.max(0, (end - start) / 2 - minSweep / 2));
    const from = start + half;
    const to = Math.max(end - half, from + minSweep);
    return { ...phase, share, start, end, d: arcPath(radius, from, Math.min(to, start + TWO_PI)) };
  });
}

/**
 * The phases the ring shows for a profile (docs/16 §Inspector contents):
 * the top `RING_TOP_NODES` nodes by THIS generation's measured work, plus
 * "other nodes" for the rest, then the server's tessellation and encode,
 * the socket, and the client's decode and upload. Node times are measured
 * work (CPU, summed across chunks — the memo's `nanos`), the rest wall
 * time: the ring is the glance, the table the comparison. A cached node
 * cost this generation a cache read and is not an arc.
 *
 * Colours: a top node's colour follows its ORDER IN THE PIPELINE among the
 * top nodes (the nodes list is the lowering's order), not its rank — so a
 * node keeps its colour across profiles while the same nodes are on top
 * (the dataviz rule: colour follows the entity, never its rank); the other
 * arcs wear neutral steps, told apart by the legend.
 */
export function ringPhases(view: ProfileView, client: ClientPhases): RingPhase[] {
  const computed = view.nodes
    .map((node, index) => ({ node, index, nanos: node.nanos ?? 0 }))
    .filter((entry) => entry.node.state === "done" && entry.nanos > 0);
  const byCost = [...computed].sort((a, b) => b.nanos - a.nanos || a.index - b.index);
  const top = byCost.slice(0, RING_TOP_NODES).sort((a, b) => a.index - b.index);
  const rest = byCost.slice(RING_TOP_NODES);
  const phases: RingPhase[] = top.map((entry, slot) => ({
    id: `node:${entry.node.name}`,
    label: entry.node.name,
    ms: entry.nanos / 1e6,
    color: `var(--prof-${slot + 1})`,
    kind: "node",
  }));
  phases.push({
    id: "other",
    label: rest.length === 1 ? "1 other node" : `${rest.length} other nodes`,
    ms: rest.reduce((sum, entry) => sum + entry.nanos, 0) / 1e6,
    color: "var(--prof-other)",
    kind: "other",
  });
  phases.push(
    { id: "tessellate", label: "tessellation", ms: view.phases.tessellate_ms, color: "var(--prof-tessellate)", kind: "server" },
    { id: "encode", label: "encode", ms: view.phases.encode_ms, color: "var(--prof-encode)", kind: "server" },
    { id: "socket", label: "socket", ms: client.socket_ms ?? 0, color: "var(--prof-socket)", kind: "client" },
    { id: "decode", label: "decode", ms: client.decode_ms, color: "var(--prof-decode)", kind: "client" },
    { id: "upload", label: "upload", ms: client.upload_ms, color: "var(--prof-upload)", kind: "client" },
  );
  return phases;
}

// ---------------------------------------------------------------- table --

export type NodeSortKey = "name" | "state" | "time" | "share" | "elements";

export interface NodeSort {
  key: NodeSortKey;
  descending: boolean;
}

/** The table's opening order: the costliest node first. */
export const DEFAULT_NODE_SORT: NodeSort = { key: "time", descending: true };

/** The time a row shows: this generation's `nanos` for a computed node, the memo's `last_nanos` for a cached one, null otherwise. */
export function nodeTimeNanos(node: ProfileNode): number | null {
  return node.nanos ?? node.last_nanos ?? null;
}

/** Each computed node's share of THIS generation's measured work, by name (cached and idle rows have none). */
export function nodeShares(nodes: readonly ProfileNode[]): Map<string, number> {
  const total = nodes.reduce((sum, node) => sum + (node.state === "done" ? (node.nanos ?? 0) : 0), 0);
  const shares = new Map<string, number>();
  if (total <= 0) return shares;
  for (const node of nodes) {
    if (node.state === "done" && node.nanos !== undefined) shares.set(node.name, node.nanos / total);
  }
  return shares;
}

/** `12.3 %` — one decimal; `< 0.1 %` below it; `—` for a row without a share. */
export function shareText(share: number | null | undefined): string {
  if (share === null || share === undefined) return "—";
  const percent = share * 100;
  if (percent > 0 && percent < 0.1) return "< 0.1 %";
  return `${percent.toFixed(1)} %`;
}

/** The order the state column sorts in: what cost this generation first, then what waits, then what went wrong, then idle. */
const STATE_RANK: Record<string, number> = {
  done: 0,
  running: 1,
  queued: 2,
  cached: 3,
  red: 4,
  blocked: 5,
  cancelled: 6,
  idle: 7,
};

/**
 * The rows sorted by `sort` — stable, ties by name, rows without a value for
 * the key (no time, no share, no element count) last whichever way the sort
 * runs. `shares` is `nodeShares(nodes)` (passed in so a filtered table keeps
 * the whole generation's shares).
 */
export function sortProfileNodes(nodes: readonly ProfileNode[], sort: NodeSort, shares: ReadonlyMap<string, number>): ProfileNode[] {
  const value = (node: ProfileNode): number | string | null => {
    switch (sort.key) {
      case "name":
        return node.name;
      case "state":
        return STATE_RANK[node.state] ?? 99;
      case "time":
        return nodeTimeNanos(node);
      case "share":
        return shares.get(node.name) ?? null;
      case "elements":
        return node.elements ?? null;
    }
  };
  const direction = sort.descending ? -1 : 1;
  return [...nodes].sort((a, b) => {
    const va = value(a);
    const vb = value(b);
    if (va === null && vb === null) return a.name.localeCompare(b.name);
    if (va === null) return 1;
    if (vb === null) return -1;
    const cmp = typeof va === "string" && typeof vb === "string" ? va.localeCompare(vb) : Number(va) - Number(vb);
    return cmp !== 0 ? cmp * direction : a.name.localeCompare(b.name);
  });
}

/** A header click: the same key flips the direction; a new key opens the way it reads best (numbers biggest first, words A → Z). */
export function nextSort(current: NodeSort, key: NodeSortKey): NodeSort {
  if (current.key === key) return { key, descending: !current.descending };
  return { key, descending: key === "time" || key === "share" || key === "elements" };
}

/** The rows whose name or state contains `text` (case-insensitive; blank keeps every row). */
export function filterProfileNodes(nodes: readonly ProfileNode[], text: string): ProfileNode[] {
  const needle = text.trim().toLowerCase();
  if (needle === "") return [...nodes];
  return nodes.filter((node) => node.name.toLowerCase().includes(needle) || node.state.includes(needle));
}

/** `gen 12 · structural` — the panel's headline. */
export function profileTitle(view: ProfileView): string {
  return `gen ${view.generation} · ${view.kind}`;
}

/** `2.33 MB at 41.2 MB/s` — the socket legend's hover; null when the rate is not measurable. */
export function rateText(bytes: number, ratePerMs: number | null): string | null {
  if (ratePerMs === null) return null;
  const perSecond = ratePerMs * 1000;
  const unit = perSecond >= 1024 * 1024 ? `${(perSecond / (1024 * 1024)).toFixed(1)} MB/s` : `${(perSecond / 1024).toFixed(1)} KB/s`;
  return `${bytesText(bytes)} at ${unit}`;
}

/** Bytes → `1.2 KB` / `3.4 MB` (the same spelling as `format.ts`'s, without the import cycle). */
function bytesText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
