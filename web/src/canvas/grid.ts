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

/**
 * Plain substring match on `name` / `title` (docs/16: v1 is substring). With
 * a probe catalog only funcs that have an accepting port are listed, carrying
 * those ports. Sorted: prefix matches first, then alphabetical.
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
  const hits: SearchHit[] = [];
  for (const node of catalog.nodes) {
    const ports = accepting?.get(node.name);
    if (accepting !== null && (ports === undefined || ports.length === 0)) continue;
    const name = node.name.toLowerCase();
    const title = node.title.toLowerCase();
    if (q !== "" && !name.includes(q) && !title.includes(q)) continue;
    hits.push({ node, ports: ports ?? [] });
  }
  const rank = (hit: SearchHit) => {
    const name = hit.node.name.toLowerCase();
    if (q === "") return 1;
    if (name === q) return 0;
    if (name.startsWith(q)) return 1;
    if (hit.node.title.toLowerCase().startsWith(q)) return 2;
    return 3;
  };
  hits.sort((a, b) => rank(a) - rank(b) || a.node.name.localeCompare(b.node.name));
  return hits.slice(0, limit);
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
