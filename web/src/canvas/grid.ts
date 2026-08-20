/**
 * Pure canvas logic (unit-tested): grid ⇄ pixel maths (docs/10 §sidecar),
 * slider maths, zoom LOD tiers (docs/16), catalog search filtering, and
 * the status-badge vocabulary. The literal-spelling rule (`paramValueText`)
 * lives in `state/literals.ts` — ONE rule for canvas and panels — and is
 * re-exported here for the canvas modules.
 */
import type { Catalog, CatalogNode, NodeStatus, ProbeCatalogEntry } from "../protocol/messages";

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

/** Zoom LOD tiers (docs/16 §Canvas conventions: far · mid · near · closest). */
export type LodTier = "far" | "mid" | "near" | "closest";

export function lodTier(zoom: number): LodTier {
  if (zoom < 0.35) return "far";
  if (zoom < 0.65) return "mid";
  if (zoom < 1.6) return "near";
  return "closest";
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
 * `null` otherwise (Cicada-only nodes included).
 */
export function ghHint(node: CatalogNode): string | null {
  if (node.gh === null) return null;
  return node.gh.toLowerCase() === node.title.toLowerCase() ? null : node.gh;
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

function ms(nanos: number): string {
  const v = nanos / 1e6;
  if (v < 0.1) return `${(nanos / 1e3).toFixed(0)}µs`;
  if (v < 10) return `${v.toFixed(1)}ms`;
  if (v < 1000) return `${v.toFixed(0)}ms`;
  return `${(v / 1000).toFixed(1)}s`;
}

export function statusBadge(status: NodeStatus | undefined, diagnostics: number): Badge {
  if (status === undefined) {
    return { label: "idle", className: "state-idle", title: "idle — not solved yet" };
  }
  switch (status.state) {
    case "cached":
      return { label: "cached", className: "state-cached", title: "cached — result reused" };
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
      const time = status.nanos !== undefined ? ms(status.nanos) : "";
      return { label: time || "done", className: "state-done", title: `done${time ? ` in ${time}` : ""}` };
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

/** Wire stroke width by list depth (docs/09: single / double / hatched). */
export function wireStrokeWidth(depth: number): number {
  if (depth <= 0) return 1.5;
  if (depth === 1) return 2.5;
  return 4;
}

/** Whether a rendered base type expects a refinement (`Closed<Curve>`, `Watertight<Mesh>`). */
export function isRefinement(base: string): boolean {
  return base.includes("<");
}
