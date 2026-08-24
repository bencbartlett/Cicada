/**
 * The trace lanes' React side (docs/16 §Canvas conventions; `trace.ts` is
 * the router). The lane assignment needs every wire's geometry at once, so
 * it is computed ONCE per render of the canvas — a memo over the store's
 * graph and the canvas's live node positions — and handed to the edges
 * through a context, never per edge in isolation. The result's identity is
 * kept while no wire's route changed, so a selection tick or a status tick
 * re-renders no edge; a drag moves only the wires whose lanes moved.
 *
 * The model's endpoints are corrected by the handle geometry React Flow
 * measured (`HandleGeometry`: the handle's overhang past the node edge and
 * its centre's shift below the row model's row), read from the first
 * mounted node with a source handle once the nodes are initialized — so
 * the lanes are assigned on the same endpoints the edges are drawn from,
 * and a tight gap never sees a lane the live endpoints cannot hold.
 *
 * The assignment's one fallback — a saturated column whose runs share a
 * line — comes back as `TraceLanes.collapsed`, which the canvas shows as
 * `data-trace-collapsed` (its count), so it is never silent.
 */
import { useNodesInitialized, useReactFlow } from "@xyflow/react";
import { createContext, useContext, useMemo, useRef } from "react";
import type { GraphView } from "../protocol/messages";
import type { CanvasEdge, CanvasNode } from "./flow";
import {
  UNMEASURED_HANDLES,
  assignTraceLanes,
  rowLatticeOrigin,
  sameRoute,
  wireEnds,
  type HandleGeometry,
  type TraceLanes,
  type TraceRoute,
} from "./trace";

const NO_LANES: TraceLanes = { routes: new Map(), collapsed: [] };

/** Wire id → its laned route; empty in spline mode. */
export const TraceLanesContext = createContext<ReadonlyMap<string, TraceRoute>>(NO_LANES.routes);

/**
 * The handle geometry of the mounted nodes, from React Flow's measured
 * handle bounds — the first node with a measured width and a source
 * handle — or the unmeasured zeros while none has been laid out.
 */
export function measureHandles(
  nodes: readonly CanvasNode[],
  unit: number,
  internal: (id: string) => { measured: { width?: number }; internals: { handleBounds?: HandleBounds | null } } | undefined,
): HandleGeometry {
  for (const node of nodes) {
    const found = internal(node.id);
    const width = found?.measured.width;
    const handle = found?.internals.handleBounds?.source?.[0];
    if (width === undefined || handle === undefined) continue;
    return {
      overhang: handle.x + handle.width - width,
      rowShift: handle.y + handle.height / 2 - 1.5 * unit,
    };
  }
  return UNMEASURED_HANDLES;
}

/** The slice of React Flow's `NodeHandleBounds` the measurement reads. */
interface HandleBounds {
  source?: { x: number; y: number; width: number; height: number }[] | null;
}

/** Two assignments with the same routes for the same wires and the same collapses. */
function sameLanes(a: TraceLanes, b: TraceLanes): boolean {
  if (a.routes.size !== b.routes.size) return false;
  for (const [id, route] of b.routes) if (!sameRoute(a.routes.get(id), route)) return false;
  return a.collapsed.length === b.collapsed.length && a.collapsed.every((id, i) => b.collapsed[i] === id);
}

/**
 * The lane assignment for the canvas's wires, recomputed when the graph, a
 * node position, the unit or the nodes' measurement changes while trace
 * mode is on; the same object while every route is unchanged.
 */
export function useTraceLanes(graph: GraphView, nodes: readonly CanvasNode[], unit: number, enabled: boolean): TraceLanes {
  const rf = useReactFlow<CanvasNode, CanvasEdge>();
  const initialized = useNodesInitialized();
  const held = useRef<TraceLanes>(NO_LANES);
  return useMemo(() => {
    if (!enabled) {
      held.current = NO_LANES;
      return NO_LANES;
    }
    const handles = initialized ? measureHandles(nodes, unit, (id) => rf.getInternalNode(id)) : UNMEASURED_HANDLES;
    const positions = new Map(nodes.map((node) => [node.id, node.position]));
    const next = assignTraceLanes(wireEnds(graph, positions, unit, handles), unit, rowLatticeOrigin(unit, handles));
    if (sameLanes(held.current, next)) return held.current;
    held.current = next;
    return next;
  }, [graph, nodes, unit, enabled, initialized, rf]);
}

/** This wire's laned route (`undefined` in spline mode, or for a wire the model could not place). */
export function useTraceRoute(id: string): TraceRoute | undefined {
  return useContext(TraceLanesContext).get(id);
}
