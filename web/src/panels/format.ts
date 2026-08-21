/**
 * Pure formatting helpers for the panels (docs/16 §Status and progress
 * language, §Inspector contents). No React, no store — unit-tested.
 */
import type { NodeStatus, NodeView, SolveSummary, ValueSummary } from "../protocol/messages";

// -------------------------------------------------------- param values --

// The literal-spelling rule is shared with the canvas (`state/literals.ts`):
// ONE rule for `set_param` text everywhere, re-exported for the panels.
export { paramValueText } from "../state/literals";

/** Snap a slider value to its step (step 0 = free) and clamp to bounds. */
export function snapSlider(value: number, min: number, max: number, step: number): number {
  let v = value;
  if (step > 0) v = min + Math.round((v - min) / step) * step;
  v = Math.min(max, Math.max(min, v));
  // Kill float noise from the multiply (0.1 * 3 = 0.30000000000000004).
  return Number(v.toPrecision(12));
}

// ------------------------------------------------------------ durations --

/** Nanoseconds → a short human duration (`0.04 ms`, `2.1 ms`, `1.23 s`). */
export function formatNanos(nanos: number): string {
  const ms = nanos / 1e6;
  if (ms >= 1000) return `${(ms / 1000).toFixed(2)} s`;
  if (ms >= 100) return `${ms.toFixed(0)} ms`;
  if (ms >= 1) return `${ms.toFixed(1).replace(/\.0$/, "")} ms`;
  return `${ms.toFixed(2)} ms`;
}

/** Milliseconds → short human duration. */
export function formatMs(ms: number): string {
  return formatNanos(ms * 1e6);
}

// ---------------------------------------------------- compute-on-release --

/**
 * The slider's hint while its drag is compute-on-release (docs/13 §Slider
 * drags): `pending · 3.94 s` — the predicted cost of the live preview the
 * server withheld, `~`-prefixed when the estimate is a floor (some node in
 * the cone has no cost evidence yet), the same spelling as the ETA
 * (docs/12 §Cost prediction).
 */
export function pendingHint(pending: { estimateMs: number; rough: boolean }): string {
  return `pending · ${pending.rough ? "~" : ""}${formatMs(pending.estimateMs)}`;
}

/** The tooltip behind the hint: what pending means and what happens on release. */
export function pendingTitle(pending: { estimateMs: number; rough: boolean }): string {
  const estimate = `${pending.rough ? "at least ~" : "about "}${formatMs(pending.estimateMs)}`;
  return `compute-on-release: a live preview would take ${estimate}, so the viewport waits — the value solves once, when you release`;
}

// ------------------------------------------------------- solve summary --

/** The top-bar solve-state text (docs/16 §Status and progress language). */
export function summaryText(summary: SolveSummary): string {
  if (summary.running) {
    const eta =
      summary.eta_ms === undefined
        ? ""
        : ` · ETA ${summary.eta_rough ? "~" : ""}${formatMs(summary.eta_ms)}`;
    return `solving… pending ${summary.pending}${eta}`;
  }
  if (summary.cancelled) {
    return `cancelled gen ${summary.generation} · ${summary.computed} computed / ${summary.cached} cached`;
  }
  const parts = [`${summary.computed} computed`, `${summary.cached} cached`];
  if (summary.red > 0) parts.push(`${summary.red} red`);
  if (summary.blocked > 0) parts.push(`${summary.blocked} blocked`);
  return `solved gen ${summary.generation} · ${parts.join(" / ")} · ${formatMs(summary.elapsed_ms)}`;
}

/**
 * The summary with red/blocked lifted to the per-node status counts when
 * those are higher (nodes excluded by diagnostics never enter the solve).
 */
export function withStatusCounts(
  summary: SolveSummary,
  statuses: Record<string, NodeStatus>,
): SolveSummary {
  let red = 0;
  let blocked = 0;
  for (const s of Object.values(statuses)) {
    if (s.state === "red") red += 1;
    else if (s.state === "blocked") blocked += 1;
  }
  return { ...summary, red: Math.max(summary.red, red), blocked: Math.max(summary.blocked, blocked) };
}

/**
 * The one-line status readout of a node (state word · time · elements ·
 * message). A `cached` node's time (and element count) is its LAST
 * compute's, recorded in its memo entry — never this generation's, which
 * paid a cache read — so it reads `cached · last 43.9 s` (docs/13 §Solve
 * streaming).
 */
export function statusText(status: NodeStatus | undefined): string {
  if (status === undefined) return "no status yet";
  const parts: string[] = [status.state];
  if (status.nanos !== undefined) {
    parts.push(status.state === "cached" ? `last ${formatNanos(status.nanos)}` : formatNanos(status.nanos));
  }
  if (status.elements !== undefined) {
    parts.push(
      status.elements_done !== undefined && status.state === "running"
        ? `${status.elements_done}/${status.elements} elements`
        : `${status.elements} element${status.elements === 1 ? "" : "s"}`,
    );
  }
  if (status.message) parts.push(status.message);
  return parts.join(" · ");
}

// -------------------------------------------------------------- values --

/** First 12 hex chars of a content hash. */
export function shortHash(hash: string): string {
  return hash.slice(0, 12);
}

/** Bounds → `[x0 y0 z0] … [x1 y1 z1]` with 3 significant digits. */
export function boundsText(bounds: [[number, number, number], [number, number, number]]): string {
  const f = (v: number) => (Number.isInteger(v) ? String(v) : v.toPrecision(3));
  const [lo, hi] = bounds;
  return `[${lo.map(f).join(" ")}] … [${hi.map(f).join(" ")}]`;
}

/** The compact one-line summary (kind + count/absent/axis). */
export function valueHeadline(summary: ValueSummary): string {
  const parts = [summary.kind];
  if (summary.count !== undefined) parts.push(`${summary.count} element${summary.count === 1 ? "" : "s"}`);
  if (summary.absent !== undefined && summary.absent > 0) parts.push(`${summary.absent} absent`);
  if (summary.axis !== undefined) parts.push(`axis ${summary.axis}`);
  return parts.join(" · ");
}

/** Facts → `key value` pairs in a stable order (known geometry facts first). */
export function factsList(facts: Record<string, unknown> | undefined): [string, string][] {
  if (facts === undefined) return [];
  const order = [
    "element_kind",
    "error",
    "faces",
    "solids",
    "vertices",
    "triangles",
    "segments",
    "points",
    "watertight",
    "unclosed",
    "closed",
    "bytes",
  ];
  const keys = Object.keys(facts).sort((a, b) => {
    const ia = order.indexOf(a);
    const ib = order.indexOf(b);
    if (ia === -1 && ib === -1) return a.localeCompare(b);
    if (ia === -1) return 1;
    if (ib === -1) return -1;
    return ia - ib;
  });
  return keys.map((k) => [k, factValueText(facts[k])]);
}

function factValueText(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return JSON.stringify(value);
}

// ------------------------------------------------------------ text panel --

/** Number of source lines a node's binding text spans (continuations). */
export function nodeLineCount(node: Pick<NodeView, "text">): number {
  if (node.text.length === 0) return 1;
  return node.text.split("\n").length;
}

/**
 * The 1-based inclusive line range `[first, last]` of a node's binding.
 * `NodeView.line` is the server's 0-based line index (viewmodel.rs), while
 * `Diagnostic.span.line` is 1-based — this is the one place that converts.
 */
export function nodeLineRange(node: Pick<NodeView, "line" | "text">): [number, number] {
  const first = node.line + 1;
  return [first, first + nodeLineCount(node) - 1];
}

/**
 * Line (1-based) → binding name, for every node's range. A line owned by two
 * nodes (a multi-target binding rendered as several nodes) keeps the first
 * in graph order.
 */
export function lineOwners(nodes: Pick<NodeView, "name" | "line" | "text">[]): Map<number, string> {
  const owners = new Map<number, string>();
  for (const node of nodes) {
    const [first, last] = nodeLineRange(node);
    for (let line = first; line <= last; line += 1) {
      if (!owners.has(line)) owners.set(line, node.name);
    }
  }
  return owners;
}

/** The set of highlighted lines for a selection of node names. */
export function highlightedLines(
  nodes: Pick<NodeView, "name" | "line" | "text" | "targets">[],
  selected: readonly string[],
): Set<number> {
  const lines = new Set<number>();
  const chosen = new Set(selected);
  for (const node of nodes) {
    if (!chosen.has(node.name) && !node.targets.some((t) => chosen.has(t))) continue;
    const [first, last] = nodeLineRange(node);
    for (let line = first; line <= last; line += 1) lines.add(line);
  }
  return lines;
}

// --------------------------------------------------------------- misc --

/** Basename of a project path (`//?/C:/x/examples` → `examples`). */
export function basename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const name = idx === -1 ? trimmed : trimmed.slice(idx + 1);
  return name.length === 0 ? path : name;
}

/** Bytes → `1.2 KB` / `3.4 MB`. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
