/**
 * Pure formatting helpers for the panels (docs/16 §Status and progress
 * language, §Inspector contents). No React, no store — unit-tested.
 */
import type { CachesView, NodeStatus, NodeView, SolveSummary, ValueSummary } from "../protocol/messages";
import type { DisplayPass } from "../state/store";

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

/**
 * The top-bar solve chip (docs/16 §Status and progress language; v0.1 wave
 * 5 D1): `Solving gen N` (+ the ETA) while the solve runs; `gen N ·
 * painting…` while gen N's display pass is in flight (between its
 * `display_begin` and `display_end`); then `gen N · solve 17 ms · display
 * 2.4 s` — display = the server's tessellation + encode. The counts
 * (computed / cached / red / blocked) live in the hover (`summaryTitle`).
 * `display` is the newest pass the client heard of: one for an OLDER
 * generation than the summary's says nothing about this one; one for a
 * NEWER generation is the current state (its `display_begin` rides the
 * control lane and can precede the ≤ 10 Hz status that names the
 * generation — a sub-100 ms window in which the chip would otherwise fall
 * back to the previous generation's idle text).
 */
export function summaryText(summary: SolveSummary, display: DisplayPass | null = null): string {
  if (summary.running) {
    // No ETA below a millisecond: `ETA ~0.00 ms` beside a running solve is
    // noise (the cost model has nothing yet for never-seen nodes).
    const eta =
      summary.eta_ms === undefined || summary.eta_ms < 1
        ? ""
        : ` · ETA ${summary.eta_rough ? "~" : ""}${formatMs(summary.eta_ms)}`;
    return `Solving gen ${summary.generation}${eta}`;
  }
  const pass = currentPass(summary, display);
  if (pass !== null && pass.phase === "painting") {
    return `gen ${pass.generation} · painting…`;
  }
  if (summary.cancelled) {
    return `cancelled gen ${summary.generation}`;
  }
  const solve = `solve ${formatMs(summary.elapsed_ms)}`;
  if (pass === null) return `gen ${summary.generation} · ${solve}`;
  const display_ms = pass.tessellateMs + pass.encodeMs;
  const cut = pass.cancelled ? " (cut)" : "";
  if (pass.generation > summary.generation) {
    // Painted before the generation's final status arrived: the solve
    // time is not known yet.
    return `gen ${pass.generation} · display ${formatMs(display_ms)}${cut}`;
  }
  return `gen ${summary.generation} · ${solve} · display ${formatMs(display_ms)}${cut}`;
}

/**
 * Why a cut pass stopped, for the hover and the viewport (docs/13 §The
 * display edge): an edit's newer generation redraws what it did not reach;
 * Esc leaves the previous picture until the next edit; a pass whose solve
 * was cancelled had nothing to paint.
 */
export function cutShort(pass: DisplayPass): string {
  if (!pass.cancelled) return "";
  if (pass.cutBy === "esc") return " · cut short by Esc — the previous picture stays until the next edit";
  if (pass.cutBy === "edit") return " · cut short by a newer generation";
  return " · nothing to paint: the solve was cancelled";
}

/** The display pass that describes the current generation: the summary's own, or a newer one (never an older one). */
export function currentPass(summary: SolveSummary, display: DisplayPass | null): DisplayPass | null {
  return display !== null && display.generation >= summary.generation ? display : null;
}

/**
 * The chip's hover: the counts the chip no longer shows, the solve time,
 * and the display pass itemised (tessellation, encode, frames and bytes,
 * the client's own paint time when the viewport measured it).
 */
export function summaryTitle(summary: SolveSummary, display: DisplayPass | null = null): string {
  const parts = [`${summary.computed} computed`, `${summary.cached} cached`];
  if (summary.red > 0) parts.push(`${summary.red} red`);
  if (summary.blocked > 0) parts.push(`${summary.blocked} blocked`);
  if (summary.pending > 0) parts.push(`${summary.pending} pending`);
  const lines = [`gen ${summary.generation}: ${parts.join(" / ")}`];
  if (summary.running) {
    lines.push(`solving for ${formatMs(summary.elapsed_ms)}`);
    return lines.join("\n");
  }
  lines.push(`solve ${formatMs(summary.elapsed_ms)}${summary.cancelled ? " (cancelled)" : ""}`);
  const pass = currentPass(summary, display);
  if (pass === null) return lines.join("\n");
  if (pass.phase === "painting") {
    lines.push(`painting ${pass.outputs} ${pass.outputs === 1 ? "output" : "outputs"}…`);
    return lines.join("\n");
  }
  lines.push(
    `display ${formatMs(pass.tessellateMs + pass.encodeMs)}: tessellation ${formatMs(pass.tessellateMs)} · encode ${formatMs(pass.encodeMs)}`,
  );
  lines.push(
    `${pass.outputs} ${pass.outputs === 1 ? "output" : "outputs"} · ${pass.frames} ${pass.frames === 1 ? "frame" : "frames"} · ${formatBytes(pass.bytes)}${cutShort(pass)}`,
  );
  if (pass.paintedMs !== null) lines.push(`painted here in ${formatMs(pass.paintedMs)}`);
  return lines.join("\n");
}

// ------------------------------------------------------------- caches --

/** Bytes as the caches indicator spells them: `612M`, `1G`, `2.1G`, `96K` (binary units). */
export function shortBytes(bytes: number): string {
  const KIB = 1024;
  if (bytes >= KIB * KIB * KIB) {
    const gib = bytes / (KIB * KIB * KIB);
    return `${gib >= 10 ? Math.round(gib) : Number(gib.toFixed(1))}G`;
  }
  if (bytes >= KIB * KIB) return `${Math.round(bytes / (KIB * KIB))}M`;
  if (bytes >= KIB) return `${Math.round(bytes / KIB)}K`;
  return `${bytes}B`;
}

/**
 * The top bar's caches indicator (docs/16 §Status and progress language):
 * `cache 612M / 1G · 1,397 meshes` — the display cache's bytes against its
 * budget and the display MESHES it holds (entries minus cached refusals: a
 * mesh per solid per tier, past value sets included — not the solids on
 * screen). The memo store's footprint is in the hover and the breakdown
 * (`cachesTitle`), not the pill: at the reference 1400 px the bar had no
 * room for `· memo 2.1G` beside a whole solve chip.
 */
export function cachesText(caches: CachesView): string {
  const meshes = Math.max(0, caches.display.entries - caches.display.refusals);
  return `cache ${shortBytes(caches.display.bytes)} / ${shortBytes(caches.display.budget)} · ${meshes.toLocaleString("en-US")} ${meshes === 1 ? "mesh" : "meshes"}`;
}

/** The indicator's hover: the full breakdown, one fact per line, the flags spelled out. */
export function cachesTitle(caches: CachesView): string {
  const d = caches.display;
  const lines = [
    `display cache: ${formatBytes(d.bytes)} of ${formatBytes(d.budget)} held in ${d.entries} ${d.entries === 1 ? "entry" : "entries"}${d.refusals > 0 ? ` (${d.refusals} cached ${d.refusals === 1 ? "refusal" : "refusals"})` : ""}`,
    `on screen: ${formatBytes(d.working_set)} of display meshes`,
    `hits ${d.hits.toLocaleString("en-US")} · misses ${d.misses.toLocaleString("en-US")} · evictions ${d.evictions.toLocaleString("en-US")}${d.oversized > 0 ? ` · ${d.oversized} too large to keep` : ""}`,
  ];
  if (d.over_budget) lines.push("OVER BUDGET: the picture does not fit — every redraw re-tessellates part of it");
  if (d.thrash) lines.push("THRASHING: the last pass evicted meshes the previous generation displayed");
  if (d.over_budget || d.thrash) lines.push("raise the display cache in settings, or draw fewer solids");
  lines.push(`memo store: ${formatBytes(caches.memo.bytes)} of value blobs in ${caches.memo.entries.toLocaleString("en-US")} ${caches.memo.entries === 1 ? "entry" : "entries"}`);
  return lines.join("\n");
}

/** The settings menu's label for a display cache size in MiB: `256 MiB`, `1 GiB`, `1.5 GiB`. */
export function displayCacheLabel(mib: number): string {
  return mib >= 1024 ? `${Number((mib / 1024).toFixed(1))} GiB` : `${mib} MiB`;
}

/** The viewport's display indicator: `painting 3 outputs…` while a pass is in flight, `painted 3 outputs · 160 MB in 2.9 s` after. */
export function displayText(display: DisplayPass): string {
  const outputs = `${display.outputs} ${display.outputs === 1 ? "output" : "outputs"}`;
  if (display.phase === "painting") return `painting ${outputs}…`;
  const took = display.paintedMs === null ? "" : ` in ${formatMs(display.paintedMs)}`;
  const cut = display.cancelled ? (display.cutBy === "esc" ? " · cut short by Esc" : " · cut short") : "";
  return `painted ${outputs} · ${formatBytes(display.bytes)}${took}${cut}`;
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
