/**
 * The route (docs/16 §Application layout, wave 4 O2/O3): the URL's three
 * parameters read and written, the pop-out's URL, and the routing over a
 * fake window — opening pushes ONE history entry and tells the connection,
 * Back (`popstate`) follows the URL, closing returns to the picker, the
 * same pipeline again pushes nothing.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  NO_ROUTE,
  closePipeline,
  installRouting,
  openPipeline,
  parseRoute,
  popoutUrl,
  routeSearch,
  sameRoute,
  titleFor,
  useRoute,
  type Route,
  type RoutingWindow,
} from "./route";

describe("parseRoute / routeSearch", () => {
  it("reads the three parameters; anything but view=viewport is the app", () => {
    expect(parseRoute("")).toEqual(NO_ROUTE);
    expect(parseRoute("?token=t")).toEqual({ token: "t", pipeline: undefined, view: "app" });
    expect(parseRoute("?token=t&pipeline=sub%2Fp.cic")).toEqual({ token: "t", pipeline: "sub/p.cic", view: "app" });
    expect(parseRoute("?token=t&pipeline=p.cic&view=viewport")).toEqual({ token: "t", pipeline: "p.cic", view: "viewport" });
    expect(parseRoute("?token=t&pipeline=p.cic&view=canvas").view).toBe("app");
    expect(parseRoute("?pipeline=p.cic").token).toBeUndefined();
  });

  it("writes them back, encoded, and nothing else", () => {
    expect(routeSearch(NO_ROUTE)).toBe("");
    expect(routeSearch({ token: "t", pipeline: undefined, view: "app" })).toBe("?token=t");
    expect(routeSearch({ token: "t", pipeline: "sub dir/p.cic", view: "app" })).toBe("?token=t&pipeline=sub+dir%2Fp.cic");
    expect(routeSearch({ token: "t", pipeline: "p.cic", view: "viewport" })).toBe("?token=t&pipeline=p.cic&view=viewport");
    const round: Route = { token: "a b", pipeline: "x/y z.cic", view: "viewport" };
    expect(parseRoute(routeSearch(round))).toEqual(round);
    expect(parseRoute(routeSearch(parseRoute("?token=t&pipeline=p.cic&theme=dark")))).toEqual({
      token: "t",
      pipeline: "p.cic",
      view: "app",
    });
  });

  it("popoutUrl is the same page with view=viewport added (idempotent)", () => {
    const location = { origin: "http://127.0.0.1:8420", pathname: "/", search: "?token=t&pipeline=sub%2Fp.cic" };
    expect(popoutUrl(location)).toBe("http://127.0.0.1:8420/?token=t&pipeline=sub%2Fp.cic&view=viewport");
    expect(popoutUrl({ ...location, search: "?token=t&pipeline=p.cic&view=viewport" })).toBe(
      "http://127.0.0.1:8420/?token=t&pipeline=p.cic&view=viewport",
    );
  });

  it("sameRoute compares the three fields", () => {
    const a: Route = { token: "t", pipeline: "p.cic", view: "app" };
    expect(sameRoute(a, { ...a })).toBe(true);
    expect(sameRoute(a, { ...a, view: "viewport" })).toBe(false);
    expect(sameRoute(a, { ...a, pipeline: "q.cic" })).toBe(false);
  });

  it("titleFor names the pipeline, marks the pop-out, and is plain for the picker", () => {
    expect(titleFor(undefined, "app")).toBe("Cicada");
    expect(titleFor("sub/p.cic", "app")).toBe("sub/p.cic · Cicada");
    expect(titleFor("p.cic", "viewport")).toBe("p.cic — viewport · Cicada");
  });
});

/** A window with a history the test can read: `pushState` rewrites `location.search`, `back()` pops it and fires `popstate`. */
function fakeWindow(search: string) {
  const entries = [search];
  const listeners = new Set<() => void>();
  const win: RoutingWindow & { back(): void; pushed: string[]; listeners: Set<() => void> } = {
    location: { pathname: "/", search },
    history: {
      pushState: (_data: unknown, _unused: string, url?: string | URL | null) => {
        const text = String(url);
        const next = text.includes("?") ? text.slice(text.indexOf("?")) : "";
        win.pushed.push(text);
        entries.push(next);
        win.location.search = next;
      },
    },
    addEventListener: (_type, listener) => listeners.add(listener),
    removeEventListener: (_type, listener) => listeners.delete(listener),
    back: () => {
      entries.pop();
      win.location.search = entries[entries.length - 1] ?? "";
      for (const listener of listeners) listener();
    },
    pushed: [],
    listeners,
  };
  return win;
}

describe("installRouting / openPipeline / closePipeline", () => {
  let uninstall: (() => void) | null = null;
  afterEach(() => {
    uninstall?.();
    uninstall = null;
    useRoute.setState({ route: NO_ROUTE });
  });

  it("reads the URL at install, pushes one entry per open/close, and Back follows the URL", () => {
    const win = fakeWindow("?token=t");
    const onRoute = vi.fn<(route: Route) => void>();
    uninstall = installRouting(win, onRoute);
    expect(useRoute.getState().route).toEqual({ token: "t", pipeline: undefined, view: "app" });
    expect(onRoute).toHaveBeenCalledTimes(1);

    openPipeline("02-solids.cic");
    expect(win.pushed).toEqual(["/?token=t&pipeline=02-solids.cic"]);
    expect(useRoute.getState().route.pipeline).toBe("02-solids.cic");
    expect(onRoute).toHaveBeenLastCalledWith({ token: "t", pipeline: "02-solids.cic", view: "app" });

    openPipeline("02-solids.cic");
    expect(win.pushed, "the file already open: no entry, no reconnect").toHaveLength(1);
    expect(onRoute).toHaveBeenCalledTimes(2);

    openPipeline("06-lists.cic");
    expect(win.pushed).toHaveLength(2);
    expect(win.location.search).toBe("?token=t&pipeline=06-lists.cic");

    win.back();
    expect(useRoute.getState().route.pipeline, "Back returns to the previous file").toBe("02-solids.cic");
    expect(onRoute).toHaveBeenLastCalledWith({ token: "t", pipeline: "02-solids.cic", view: "app" });

    closePipeline();
    expect(win.pushed[2]).toBe("/?token=t");
    expect(useRoute.getState().route.pipeline).toBeUndefined();
    expect(onRoute).toHaveBeenLastCalledWith({ token: "t", pipeline: undefined, view: "app" });

    win.back();
    expect(useRoute.getState().route.pipeline, "Back from the picker reopens the file").toBe("02-solids.cic");
  });

  it("keeps the view: opening from a pop-out stays a pop-out route", () => {
    const win = fakeWindow("?token=t&pipeline=p.cic&view=viewport");
    uninstall = installRouting(win, () => {});
    openPipeline("q.cic");
    expect(win.location.search).toBe("?token=t&pipeline=q.cic&view=viewport");
  });

  it("uninstall stops following popstate; opening without routing is loud", () => {
    const win = fakeWindow("?token=t&pipeline=p.cic");
    const onRoute = vi.fn<(route: Route) => void>();
    uninstall = installRouting(win, onRoute);
    uninstall();
    uninstall = null;
    expect(win.listeners.size).toBe(0);
    expect(() => openPipeline("q.cic")).toThrow(/routing not installed/);
  });
});
