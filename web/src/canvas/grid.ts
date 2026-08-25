/**
 * Pure canvas logic (unit-tested): grid ⇄ pixel maths (docs/10 §sidecar),
 * slider maths, zoom LOD tiers (docs/16), catalog search filtering, and
 * the status-badge vocabulary. The literal-spelling rule (`paramValueText`)
 * lives in `state/literals.ts` — ONE rule for canvas and panels — and is
 * re-exported here for the canvas modules.
 */
import type { Catalog, CatalogNode, DrivenSignal, DrivenView, NodeStatus, ProbeCatalogEntry } from "../protocol/messages";

export { paramValueText } from "../state/literals";

/** Grid cell → canvas pixels (`cell * unit`). */
export function cellToPx(cell: [number, number], unit: number): { x: number; y: number } {
  return { x: cell[0] * unit, y: cell[1] * unit };
}

/** Canvas pixels → nearest grid cell (`move_node` rounds; docs/10). */
export function pxToCell(x: number, y: number, unit: number): [number, number] {
  return [Math.round(x / unit), Math.round(y / unit)];
}

/**
 * Slider step: `0` means continuous → a fine power-of-ten step over the range
 * (~1000 positions), so dragged values stay short (`3.5`, never `3.5015`).
 */
export function sliderStep(min: number, max: number, step: number | undefined): number {
  if (step !== undefined && step > 0) return step;
  const span = max - min;
  if (!(span > 0)) return 0.001;
  // `Number("1e-4")` is exact where `10 ** -4` is 0.0000999…
  return Number(`1e${Math.floor(Math.log10(span)) - 3}`);
}

/** Decimal places a step needs (`0.25` → 2, `1` → 0). */
export function stepDecimals(step: number): number {
  const text = String(step);
  const exp = text.match(/e-(\d+)$/);
  if (exp) return Number(exp[1]);
  const dot = text.indexOf(".");
  return dot < 0 ? 0 : text.length - dot - 1;
}

/**
 * Snap a raw slider value onto the `min + k·step` lattice and round away the
 * float noise (`0.5 + 3000 × 0.001` prints as `3.5`, not `3.4999999999999996`).
 */
export function snapToStep(x: number, min: number, step: number): number {
  if (!(step > 0)) return x;
  const k = Math.round((x - min) / step);
  const decimals = Math.max(stepDecimals(step), stepDecimals(min));
  return Number((min + k * step).toFixed(Math.min(20, decimals)));
}

/**
 * Zoom LOD tiers (docs/16 §Canvas conventions: far · near · closest). Two
 * VISIBLE states only — `far` is the title alone over the box, everything
 * else is the full face (labels, literals, values); a node never shows its
 * header bar with blank port rows (Ben's second user test, U18, 2026-08-25:
 * the old `mid` tier was exactly that in-between state). `closest` draws
 * the same face and stays reserved for the thumbnails and deeper widgets.
 */
export type LodTier = "far" | "near" | "closest";

/** The tier of a canvas zoom factor; the thresholds are the docs/16 LOD table's. */
export function lodTier(zoom: number): LodTier {
  if (zoom < 0.35) return "far";
  if (zoom < 1.6) return "near";
  return "closest";
}

/**
 * Whether a tier shows the output value summaries (docs/16 LOD table): on
 * every tier that shows the face — the title-only `far` tier is the one
 * that does not (U7 moved them from `closest` to `near`, 2026-08-24; U18
 * folded `mid` into `near`, 2026-08-25). ONE rule for the node face (which
 * renders them) and the canvas (which fetches them with `inspect`).
 */
export function showsPortValues(tier: LodTier): boolean {
  return tier !== "far";
}

/** One search-to-place hit: the catalog node plus the ports a probed wire could land on. */
export interface SearchHit {
  node: CatalogNode;
  /** `[port, verdict]` when filtering by a probe; empty otherwise. */
  ports: [string, "ok" | "lift"][];
}

/** The weakest (substring-anywhere) rank; also every node's rank on an empty query. */
export const SEARCH_RANK_SUBSTRING = 5;

/**
 * Search-to-place rank of one catalog node for a lowercased, trimmed query
 * (lower is better; `null` = no match). docs/16: v1 is prefix/substring
 * matching over the dialect `name`, the `title` AND the Grasshopper name
 * `gh` (docs/14 §node file format: `gh` is "fed to search-to-place" — a
 * migrant types `Series`, `Merge` or `Pick'n'Choose` and lands on the node
 * that replaces it). Exact hits beat prefix hits beat substring hits; among
 * exact hits the dialect name wins (it is what a Cicada user types), then
 * the curated `gh` mapping (the declared replacement — `Addition` must
 * reach `add`, never `mass_addition`), then the title. An empty query ranks
 * everything equal.
 */
export function searchRank(node: CatalogNode, q: string): number | null {
  if (q === "") return SEARCH_RANK_SUBSTRING;
  const name = node.name.toLowerCase();
  const title = node.title.toLowerCase();
  const gh = node.gh?.toLowerCase() ?? null;
  if (name === q) return 0;
  if (gh === q) return 1;
  if (title === q) return 2;
  if (name.startsWith(q)) return 3;
  if (title.startsWith(q) || gh?.startsWith(q)) return 4;
  if (name.includes(q) || title.includes(q) || gh?.includes(q)) return SEARCH_RANK_SUBSTRING;
  return null;
}

/**
 * Search-to-place over the catalog: `name` / `title` / `gh` matching ranked
 * by `searchRank` (exact > prefix > substring; ties alphabetical by name).
 * With a probe catalog only funcs that have an accepting port are listed,
 * carrying those ports.
 */
export function filterCatalog(
  catalog: Catalog | null,
  query: string,
  probe: ProbeCatalogEntry[] | null,
  limit = 40,
): SearchHit[] {
  if (catalog === null) return [];
  const q = query.trim().toLowerCase();
  const accepting = probe === null ? null : new Map(probe.map((e) => [e.func, e.ports]));
  const hits: { hit: SearchHit; rank: number }[] = [];
  for (const node of catalog.nodes) {
    // The accepting ports are the server's verdict, whole: a transport-
    // driven port (`cycle.frame`) is never among them — the server's
    // `wire_verdict` blocks it — and nothing is second-guessed here.
    const ports = accepting?.get(node.name);
    if (accepting !== null && (ports === undefined || ports.length === 0)) continue;
    const rank = searchRank(node, q);
    if (rank === null) continue;
    hits.push({ hit: { node, ports: ports ?? [] }, rank });
  }
  hits.sort((a, b) => a.rank - b.rank || a.hit.node.name.localeCompare(b.hit.node.name));
  return hits.slice(0, limit).map((h) => h.hit);
}

/**
 * The Grasshopper name to show beside a search hit: the node's `gh` when it
 * tells the migrant something the title does not (case-insensitively
 * different — `Natural logarithm` under `Natural Logarithm` is noise),
 * `null` otherwise (Cicada-only nodes included). Tolerates an ABSENT `gh`
 * the same way `searchRank` does: the server always writes the key, but a
 * hint helper that throws inside the search-box render on a catalog it did
 * not expect would take the whole canvas down with it.
 */
export function ghHint(node: CatalogNode): string | null {
  const gh = node.gh ?? null;
  if (gh === null) return null;
  return gh.toLowerCase() === node.title.toLowerCase() ? null : gh;
}

// One index per catalog object (the store replaces the whole catalog, never
// mutates it), so a port-doc lookup per rendered port row is a map hit.
const catalogIndex = new WeakMap<Catalog, Map<string, CatalogNode>>();

/** The catalog entry of a func (`NodeView.func`), or `undefined` for an unknown/absent func. */
export function catalogEntry(catalog: Catalog | null, func: string | undefined): CatalogNode | undefined {
  if (catalog === null || func === undefined) return undefined;
  let index = catalogIndex.get(catalog);
  if (index === undefined) {
    index = new Map(catalog.nodes.map((node) => [node.name, node]));
    catalogIndex.set(catalog, index);
  }
  return index.get(func);
}

/**
 * The one-line doc of an OUTPUT port, from the catalog (the view-model's
 * `OutputView` carries no doc; inputs get theirs on `InputView.doc`). A bare
 * single `out`'s doc is the node's `# Returns` line. `undefined` when the
 * func is unknown to the catalog or the port has no doc.
 */
export function outputDoc(catalog: Catalog | null, func: string | undefined, port: string): string | undefined {
  const doc = catalogEntry(catalog, func)?.outputs.find((o) => o.name === port)?.doc;
  return doc === undefined || doc === "" ? undefined : doc;
}

/**
 * The transport signal an INPUT port of `func` is driven by (`cycle.frame`
 * → `frame`, `clock.t` → `time`; the catalog's `transport_driven` flag),
 * `undefined` for every other port and for a func the catalog does not
 * know (the project's script nodes — none are driven). Such a port is the
 * session's, not the user's (docs/13 §Animation transport): the canvas
 * and the inspector HIDE it as a port — no connectable handle, no literal
 * editor — and show the transport in its place. The server decides it is
 * never a wire target (`probe_wire` answers `blocked`, `connect` refuses);
 * a kwarg or wire a human wrote for it by hand stays in the text as the
 * headless value / source, shown and removable, never edited here.
 *
 * Until the catalog has arrived — it is fetched over HTTP beside the
 * socket's snapshot, so the first paint can precede it — the snapshot's
 * own `driven` set stands in: `driving` is this port's entry in the last
 * `TransportView` heard, and its `signal` is the answer, so a port the
 * transport is feeding never paints as an ordinary input (handle, literal
 * editor) for even one frame. Once the catalog is here it decides alone:
 * a port of a red `cycle` is driven by nature, in the driven set or not.
 */
export function transportDrivenSignal(
  catalog: Catalog | null,
  func: string | undefined,
  port: string,
  driving?: DrivenView,
): DrivenSignal | undefined {
  if (catalog === null) return driving?.signal;
  const entry = catalogEntry(catalog, func);
  return entry === undefined ? undefined : drivenSignalOf(entry, port);
}

function drivenSignalOf(node: CatalogNode, port: string): DrivenSignal | undefined {
  return node.inputs.find((input) => input.name === port)?.transport_driven;
}

/**
 * The hover text of a transport-driven port's row (canvas node and
 * inspector alike): what drives it, whether it is driving now, and what
 * the text says for the headless run — a literal kwarg or a wire, named
 * as the headless value / source, never edited here.
 */
export function drivenTitle(
  name: string,
  type: string,
  signal: DrivenSignal,
  driving: boolean,
  literal: string | undefined,
  wired: { node: string; port: string } | undefined,
): string {
  const what = signal === "frame" ? "the loop frame" : "the playhead in seconds";
  // The transport owns the port in the app (it fills it from the playhead);
  // it is never edited here. What the text says is the headless value.
  const owned = "the session's — never edited here";
  const state = driving
    ? `driven by the transport (${what}), ${owned}.`
    : `the transport's port (${what}); not driving while this node is not solvable; ${owned}.`;
  let written = "";
  if (wired !== undefined) {
    written = ` The text wires \`${name}=${wired.node}\` — the headless source (cicada run); the transport overrides it in the app. Unwire it to drop the kwarg.`;
  } else if (literal !== undefined) {
    written = ` The text's \`${name}=${literal}\` is the headless value (cicada run).`;
  }
  return `${name}: ${type} — ${state}${written}`;
}

/** The hover text of a port row: `name: type — doc` (docs/16: one line, the type, the doc). */
export function portTitle(name: string, type: string, doc: string | undefined): string {
  return doc === undefined || doc === "" ? `${name}: ${type}` : `${name}: ${type} — ${doc}`;
}

/** The status badge: docs/16 vocabulary, one label everywhere. */
export interface Badge {
  /** Short text on the node face. */
  label: string;
  /** CSS modifier (`state-<state>`). */
  className: string;
  /** Hover text. */
  title: string;
}

/**
 * The badge's compact duration: `640ns` · `5µs` · `1.2ms` · `43ms` · `2.3s`
 * (finding U25, 2026-08-25: below a microsecond the unit is nanoseconds,
 * never `0µs`).
 */
export function durationLabel(nanos: number): string {
  if (nanos < 1e3) return `${Math.round(nanos)}ns`;
  const v = nanos / 1e6;
  if (v < 0.1) return `${(nanos / 1e3).toFixed(0)}µs`;
  if (v < 10) return `${v.toFixed(1)}ms`;
  if (v < 1000) return `${v.toFixed(0)}ms`;
  return `${(v / 1000).toFixed(1)}s`;
}

/**
 * The hover's duration: THREE significant figures in the unit that puts
 * the number in [1, 1000) — `640 ns`, `4.80 µs`, `1.24 ms`, `43.9 s`
 * (finding U25). Whole seconds from 1000 s up (no exponent on a hover).
 */
export function durationTitle(nanos: number): string {
  if (nanos >= 1e12) return `${Math.round(nanos / 1e9)} s`;
  const units: [number, string][] = [
    [1e9, "s"],
    [1e6, "ms"],
    [1e3, "µs"],
  ];
  for (const [scale, unit] of units) {
    if (nanos >= scale) return `${(nanos / scale).toPrecision(3)} ${unit}`;
  }
  return `${nanos.toPrecision(3)} ns`;
}

export function statusBadge(status: NodeStatus | undefined, diagnostics: number): Badge {
  if (status === undefined) {
    return { label: "idle", className: "state-idle", title: "idle — not solved yet" };
  }
  switch (status.state) {
    case "cached": {
      // The memo entry's recorded cost: what the LAST compute of this key
      // took, never this generation's cache read (docs/13 §Solve streaming).
      // The face shows that time in parentheses (grey, by the class) — the
      // word "cached" only when the entry recorded no cost (U25).
      if (status.nanos === undefined) {
        return { label: "cached", className: "state-cached", title: "cached — result reused" };
      }
      return {
        label: `(${durationLabel(status.nanos)})`,
        className: "state-cached",
        title: `cached — result reused; the last compute took ${durationTitle(status.nanos)}`,
      };
    }
    case "queued":
      return { label: "queued", className: "state-queued", title: "queued" };
    case "running": {
      const done = status.elements_done;
      const total = status.elements;
      const label =
        done !== undefined && total !== undefined && total > 0
          ? `${Math.round((100 * done) / total)}%`
          : "running";
      return {
        label,
        className: "state-running",
        title:
          done !== undefined && total !== undefined
            ? `running — ${done}/${total} elements`
            : "running",
      };
    }
    case "done": {
      if (status.nanos === undefined) return { label: "done", className: "state-done", title: "done" };
      return {
        label: durationLabel(status.nanos),
        className: "state-done",
        title: `done in ${durationTitle(status.nanos)}`,
      };
    }
    case "red": {
      const count = Math.max(diagnostics, 1);
      return {
        label: `● ${count}`,
        className: "state-red",
        title: status.message ?? `${count} diagnostic${count === 1 ? "" : "s"}`,
      };
    }
    case "blocked":
      return { label: "blocked", className: "state-blocked", title: status.message ?? "blocked" };
    case "cancelled":
      return { label: "cancelled", className: "state-cancelled", title: "cancelled" };
    case "idle":
      return { label: "idle", className: "state-idle", title: "idle" };
  }
}

/** First line of a comment (the visible note); the full text goes in `title`. */
export function firstLine(text: string): string {
  const line = text.split(/\r?\n/, 1)[0] ?? "";
  return line.length > 60 ? `${line.slice(0, 57)}…` : line;
}

/**
 * The GH wire convention (docs/09; finding U26, 2026-08-25): a single line
 * for one value, a DOUBLE line for a list, a thick DASHED line for a tree
 * (depth ≥ 2 — nested lists, and every deeper structure).
 */
export type WireStyle = "single" | "double" | "dashed";

export function wireStyle(depth: number): WireStyle {
  if (depth <= 0) return "single";
  if (depth === 1) return "double";
  return "dashed";
}

/**
 * Wire stroke width by list depth: the double line is drawn as one 4 px
 * stroke with a 1.5 px core in the canvas background (two 1.25 px lines);
 * the tree's dashes are the same 4 px.
 */
export function wireStrokeWidth(depth: number): number {
  return depth <= 0 ? 1.5 : 4;
}

/** Whether a rendered base type expects a refinement (`Closed<Curve>`, `Watertight<Mesh>`). */
export function isRefinement(base: string): boolean {
  return base.includes("<");
}
