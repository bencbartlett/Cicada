/**
 * Where the page is (docs/16 §Application layout; docs/17 wave 4 O2 + O3):
 * the URL's `?token=…&pipeline=…&view=viewport`.
 *
 *   - `token` alone → the landing picker (the root's file list);
 *   - `token` + `pipeline` → the app on that pipeline's session;
 *   - `… &view=viewport` → the pop-out: the viewport alone, joined as a
 *     declared observer (docs/13 — the join hint).
 *
 * Opening a pipeline (File → Open / Recent, the picker) or closing it
 * (File → Close) is a `history.pushState` of the new search plus a route
 * change here, so the browser's Back returns to the previous file (or the
 * picker) through `popstate`. The route store is UI state; the connection
 * module follows it (`syncConnection`): the pipeline in the URL is the one
 * the socket is joined to, never the other way round. Pure functions over
 * the search string (`parseRoute`, `routeSearch`, `popoutUrl`) carry the
 * shape; `installRouting` wires a window.
 */
import { create } from "zustand";

export type View = "app" | "viewport";

export interface Route {
  /** The session token; undefined = the page cannot talk to the server (the landing explains). */
  token: string | undefined;
  /** The pipeline open in this tab, root-relative; undefined = the picker. */
  pipeline: string | undefined;
  view: View;
}

export const NO_ROUTE: Route = { token: undefined, pipeline: undefined, view: "app" };

/** Read a route off a search string (`?token=…&pipeline=…&view=viewport`). Anything but `view=viewport` is the app. */
export function parseRoute(search: string): Route {
  const params = new URLSearchParams(search);
  const token = params.get("token") ?? undefined;
  const pipeline = params.get("pipeline") ?? undefined;
  const view: View = params.get("view") === "viewport" ? "viewport" : "app";
  return { token, pipeline, view };
}

/**
 * The search string of a route — `?token=…[&pipeline=…][&view=viewport]`,
 * `""` for an empty one. Only the three parameters: whatever else the page
 * arrived with is not carried along.
 */
export function routeSearch(route: Route): string {
  const params = new URLSearchParams();
  if (route.token !== undefined) params.set("token", route.token);
  if (route.pipeline !== undefined) params.set("pipeline", route.pipeline);
  if (route.view === "viewport") params.set("view", "viewport");
  const text = params.toString();
  return text === "" ? "" : `?${text}`;
}

/** The pop-out's URL for the page at `location`: the same page, `view=viewport` added (docs/16 §Viewport conventions). */
export function popoutUrl(location: Pick<Location, "origin" | "pathname" | "search">): string {
  const route = parseRoute(location.search);
  return `${location.origin}${location.pathname}${routeSearch({ ...route, view: "viewport" })}`;
}

/** The document title for a route: the pipeline's name, "viewport" for the pop-out, plain "Cicada" for the picker. */
export function titleFor(pipeline: string | undefined, view: View): string {
  if (pipeline === undefined) return "Cicada";
  return view === "viewport" ? `${pipeline} — viewport · Cicada` : `${pipeline} · Cicada`;
}

/** Same token, pipeline and view? */
export function sameRoute(a: Route, b: Route): boolean {
  return a.token === b.token && a.pipeline === b.pipeline && a.view === b.view;
}

interface RouteState {
  route: Route;
  setRoute: (route: Route) => void;
}

/** The current route — what `Root` renders from. Written only by the routing below. */
export const useRoute = create<RouteState>((set) => ({
  route: NO_ROUTE,
  setRoute: (route) => set({ route }),
}));

/** What the routing needs of a window: the search to read, the history to push or replace, `popstate` to hear. */
export interface RoutingWindow {
  location: Pick<Location, "pathname" | "search">;
  history: Pick<History, "pushState" | "replaceState">;
  addEventListener(type: "popstate", listener: () => void): void;
  removeEventListener(type: "popstate", listener: () => void): void;
}

let installed: { win: RoutingWindow; onRoute: (route: Route) => void } | null = null;

/**
 * Wire a window: read its URL into the route store (and tell `onRoute`),
 * follow `popstate` the same way, and let `openPipeline` / `closePipeline`
 * push. Called once from `main.tsx`; returns the uninstaller (tests).
 */
export function installRouting(win: RoutingWindow, onRoute: (route: Route) => void): () => void {
  const apply = () => {
    const route = parseRoute(win.location.search);
    useRoute.getState().setRoute(route);
    onRoute(route);
  };
  installed = { win, onRoute };
  win.addEventListener("popstate", apply);
  apply();
  return () => {
    win.removeEventListener("popstate", apply);
    if (installed?.win === win) installed = null;
  };
}

/**
 * Go to a route that differs from the current one in `pipeline`: the URL
 * (`push` = one history entry, so Back returns here; `replace` = the current
 * entry rewritten, for a URL nobody should be returned to), the store, the
 * connection. The same pipeline again is a no-op — no entry is pushed for a
 * file already open.
 */
function navigate(pipeline: string | undefined, how: "push" | "replace" = "push"): void {
  if (installed === null) {
    throw new Error("routing not installed — installRouting(window) must run before a pipeline can be opened");
  }
  const current = useRoute.getState().route;
  if (current.pipeline === pipeline) return;
  const next: Route = { ...current, pipeline };
  const { win, onRoute } = installed;
  const url = `${win.location.pathname}${routeSearch(next)}`;
  if (how === "push") win.history.pushState(null, "", url);
  else win.history.replaceState(null, "", url);
  useRoute.getState().setRoute(next);
  onRoute(next);
}

/** Open a pipeline (root-relative) in this tab: the URL, the store, the socket. */
export function openPipeline(pipeline: string): void {
  navigate(pipeline);
}

/** Close the open pipeline: back to the picker (the socket closes; the server's session lives on for whoever else has it). */
export function closePipeline(): void {
  navigate(undefined);
}

/**
 * Leave a pipeline the server refused to open (the handshake's `pipeline`
 * error — docs/13 §Projects, pipelines, sessions): back to the picker with
 * the dead URL REPLACED in the history, so Back skips it instead of asking
 * for it again. The one case in which the connection writes the route
 * instead of following it (`connection.ts`).
 */
export function leaveRefusedPipeline(): void {
  navigate(undefined, "replace");
}
