/**
 * The viewport's imperative surface for the rest of the app (keyboard map,
 * inspector buttons, debug hooks). The Viewport component installs the real
 * implementation on mount; before that every call is a loud no-op notice
 * rather than a silent nothing.
 */
import { useCicada } from "../state/store";

export interface ViewportApi {
  /** Frame the selected geometry (`F`), else everything. */
  frameSelection(): void;
  /** Frame everything (`Home`). */
  frameAll(): void;
  /** Render the current view to a PNG blob (same path as /debug/screenshot). */
  screenshot(): Promise<Blob>;
  /** Scene statistics for assertions (`window.__cicada.scene()`). */
  stats(): ViewportStats;
}

export interface ViewportStats {
  /** Outputs currently drawn, keyed `nodeRef:output`. */
  outputs: Record<
    string,
    {
      generation: number;
      kinds: string[];
      elements: number;
      vertices: number;
      triangles: number;
      segments: number;
      points: number;
      instanced: number;
      bounds: [[number, number, number], [number, number, number]] | null;
    }
  >;
  /** Union bounds of everything drawn. */
  bounds: [[number, number, number], [number, number, number]] | null;
  drawCalls: number;
  framesReceived: number;
  lastGeneration: number;
  highlighted: { nodes: number[]; pickId: number | null };
}

const notMounted = (what: string) => {
  useCicada.getState().addNotice("warning", `viewport not mounted — ${what} ignored`);
};

let current: ViewportApi = {
  frameSelection: () => notMounted("frame selection"),
  frameAll: () => notMounted("frame all"),
  screenshot: () => Promise.reject(new Error("viewport not mounted")),
  stats: () => ({
    outputs: {},
    bounds: null,
    drawCalls: 0,
    framesReceived: 0,
    lastGeneration: 0,
    highlighted: { nodes: [], pickId: null },
  }),
};

export function installViewportApi(api: ViewportApi | null): void {
  if (api !== null) current = api;
}

export const viewportApi: ViewportApi = {
  frameSelection: () => current.frameSelection(),
  frameAll: () => current.frameAll(),
  screenshot: () => current.screenshot(),
  stats: () => current.stats(),
};
