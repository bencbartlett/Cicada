// @vitest-environment jsdom
/**
 * The profiler tab (docs/16 §Inspector contents; docs/13 §The profiler;
 * v0.1 wave 5 P1), rendered from a seeded store: the `profile` read goes
 * out when the tab shows and again when a pass lands (never while one
 * paints); the ring's arcs and legend, the phases, every node as a row with
 * a cached one marked and showing its last compute, the header sort and the
 * filter, the display rows, the caches section with its budget control, and
 * the caches indicator's focus landing on that section.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ClientMessage, ProfileView } from "../protocol/messages";
import { EMPTY_CACHES, useCicada, type DisplayPass } from "../state/store";
import { useInspectorTab } from "./inspectorTab";
import { ProfilePanel } from "./ProfilePanel";

const profile: ProfileView = {
  generation: 12,
  kind: "structural",
  phases: { queued_ms: 0.4, solve_ms: 5.5, tessellate_ms: 40, encode_ms: 2, bytes: 160_100 },
  nodes: [
    { name: "ball", state: "done", nanos: 3_000_000, elements: 1 },
    { name: "block", state: "cached", last_nanos: 9_000_000, elements: 1 },
    { name: "arms", state: "done", nanos: 1_000_000, elements: 40 },
    { name: "bad", state: "red" },
    { name: "vol", state: "blocked" },
    { name: "k", state: "done" },
  ],
  display: [
    { node: "ball", output: "out", triangles: 8000, bytes: 160_100, tier: "fine", solids: 1, cache_hits: 0, cache_misses: 1 },
    { node: "pts", output: "out", triangles: 0, bytes: 48, solids: 0, cache_hits: 0, cache_misses: 0 },
  ],
  caches: EMPTY_CACHES,
};

const pass = (generation: number, phase: DisplayPass["phase"]): DisplayPass => ({
  generation,
  phase,
  outputs: 1,
  frames: 1,
  bytes: 160_100,
  tessellateMs: 40,
  encodeMs: 2,
  cancelled: false,
  cutBy: null,
  beganAt: 0,
  paintedMs: phase === "painted" ? 130 : null,
});

const rowNames = () => screen.getAllByTestId("profile-node-row").map((row) => row.getAttribute("data-node"));

describe("the profiler tab", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    useCicada.setState({
      connection: "open",
      role: "writer",
      profile: null,
      display: pass(12, "painted"),
      summary: { ...useCicada.getState().summary, generation: 12, running: false },
      snapshots: 1,
      caches: { ...EMPTY_CACHES, display: { ...EMPTY_CACHES.display, bytes: 4096, entries: 3, working_set: 4096 } },
      notices: [],
      selection: { nodes: [], wire: null, element: null },
    });
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "";
    });
    useInspectorTab.setState({ tab: "profile", profileFocus: null });
  });
  afterEach(cleanup);

  it("asks for the profile when it shows, again when a pass lands, and not while one paints", () => {
    render(<ProfilePanel />);
    expect(sent).toEqual([{ type: "profile", payload: {} }]);
    expect(screen.getByTestId("profile-view").getAttribute("data-generation")).toBe("none");
    act(() => useCicada.setState({ display: pass(13, "painting") }));
    expect(sent, "a pass in flight is not asked for").toHaveLength(1);
    act(() => useCicada.setState({ display: pass(13, "painted"), summary: { ...useCicada.getState().summary, generation: 13 } }));
    expect(sent).toHaveLength(2);
    // A re-hydration asks again for the same generation.
    act(() => useCicada.setState({ snapshots: 2 }));
    expect(sent).toHaveLength(3);
  });

  it("renders the headline, the ring, the legend, the phases and every node; a cached row is marked with its last compute", () => {
    render(<ProfilePanel />);
    act(() => useCicada.setState({ profile }));
    const view = screen.getByTestId("profile-view");
    expect(view.getAttribute("data-generation")).toBe("12");
    expect(screen.getByTestId("profile-title").textContent).toBe("gen 12 · structural");
    // The arcs: ball, arms (nodes), tessellation, encode — the socket / decode /
    // upload are 0 (no frame reached this page) and draw nothing.
    const arcs = screen.getAllByTestId("profile-arc").map((a) => a.getAttribute("data-phase"));
    expect(arcs).toEqual(["node:ball", "node:arms", "tessellate", "encode"]);
    expect(screen.getByTestId("profile-legend").textContent).toContain("tessellation");
    expect(screen.getByTestId("profile-ring-total").textContent).toBe("46 ms");
    expect(screen.getByTestId("profile-phases").textContent).toContain("solve5.5 ms");
    expect(screen.getByTestId("profile-first-paint").textContent).toBe("130 ms");
    expect(screen.getByTestId("profile-decode").textContent).toBe("—");
    // Every node is a row, costliest first, rows without a time last.
    expect(rowNames()).toEqual(["block", "ball", "arms", "bad", "k", "vol"]);
    const cached = screen.getAllByTestId("profile-node-row").find((r) => r.getAttribute("data-node") === "block")!;
    expect(cached.className).toContain("cached");
    expect(cached.textContent).toContain("last 9 ms");
    // No share: a cache read is not this generation's work.
    expect(cached.textContent).toContain("—");
    const ball = screen.getAllByTestId("profile-node-row").find((r) => r.getAttribute("data-node") === "ball")!;
    expect(ball.textContent).toContain("75.0 %");
    // The display rows: the sphere with its tier and lookups, the points without a tier.
    const rows = screen.getAllByTestId("profile-display-row");
    expect(rows.map((r) => r.getAttribute("data-output"))).toEqual(["ball.out", "pts.out"]);
    expect(rows[0]!.textContent).toContain("fine");
    expect(rows[0]!.textContent).toContain("0 / 1");
    expect(rows[1]!.textContent).toContain("—");
  });

  it("a header click sorts, a second flips, and the filter narrows the rows; a name selects the node", () => {
    render(<ProfilePanel />);
    act(() => useCicada.setState({ profile }));
    fireEvent.click(screen.getByTestId("profile-sort-name"));
    expect(rowNames()).toEqual(["arms", "bad", "ball", "block", "k", "vol"]);
    expect(screen.getByTestId("profile-nodes").getAttribute("data-descending")).toBe("false");
    fireEvent.click(screen.getByTestId("profile-sort-name"));
    expect(rowNames()[0]).toBe("vol");
    fireEvent.click(screen.getByTestId("profile-sort-elements"));
    expect(rowNames().slice(0, 2)).toEqual(["arms", "ball"]);
    // The filter reads names AND states: `bl` is `block` and the blocked `vol`; `ball` is the one node.
    fireEvent.change(screen.getByTestId("profile-filter"), { target: { value: "bl" } });
    expect(rowNames()).toEqual(["block", "vol"]);
    fireEvent.change(screen.getByTestId("profile-filter"), { target: { value: "ball" } });
    expect(rowNames()).toEqual(["ball"]);
    fireEvent.change(screen.getByTestId("profile-filter"), { target: { value: "zzz" } });
    expect(screen.queryAllByTestId("profile-node-row")).toEqual([]);
    expect(screen.getByTestId("profile-nodes").textContent).toContain("no node matches");
    fireEvent.change(screen.getByTestId("profile-filter"), { target: { value: "" } });
    fireEvent.click(screen.getAllByTestId("profile-node-row")[0]!.querySelector("button")!);
    expect(useCicada.getState().selection.nodes).toEqual(["arms"]);
  });

  it("the caches section shows the session's view with its budget control, before and after a profile, and takes the indicator's focus", () => {
    useInspectorTab.setState({ tab: "profile", profileFocus: "caches" });
    render(<ProfilePanel />);
    expect(screen.getByTestId("profile-caches")).toBeTruthy();
    expect(screen.getByTestId("profile-cache-held").textContent).toBe("4.0 KB of 1024.00 MB");
    expect((screen.getByTestId("profile-display-cache") as HTMLSelectElement).value).toBe("");
    expect(screen.getByTestId("profile-display-cache-now").textContent).toBe("session: 1G");
    expect(useInspectorTab.getState().profileFocus, "consumed once the section rendered").toBeNull();
    act(() => useCicada.setState({ profile }));
    expect(screen.getByTestId("profile-caches").getAttribute("data-warn")).toBe("false");
    fireEvent.change(screen.getByTestId("profile-display-cache"), { target: { value: "512" } });
    expect(sent.at(-1)).toEqual({ type: "set_display_cache", payload: { mib: 512 } });
    expect(useCicada.getState().settings.displayCacheMib).toBe(512);
    act(() => useCicada.getState().updateSettings({ displayCacheMib: null }));
  });

  it("Esc closes the tab from a focused button (where the keyboard map is gated), never from a text field, never twice", () => {
    render(<ProfilePanel />);
    act(() => useCicada.setState({ profile }));
    // The filter box keeps its Esc (a text field).
    const filter = screen.getByTestId("profile-filter");
    filter.focus();
    fireEvent.keyDown(filter, { key: "Escape" });
    expect(useInspectorTab.getState().tab).toBe("profile");
    // A press the keyboard map already consumed changes nothing more
    // (`defaultPrevented` is not an init field: cancel the event by hand).
    const consumed = new KeyboardEvent("keydown", { key: "Escape", cancelable: true });
    consumed.preventDefault();
    window.dispatchEvent(consumed);
    expect(useInspectorTab.getState().tab).toBe("profile");
    // From a focused button — the sort header the user just clicked — the tab closes.
    const header = screen.getByTestId("profile-sort-name");
    header.focus();
    fireEvent.keyDown(header, { key: "Escape" });
    expect(useInspectorTab.getState().tab).toBe("inspect");
  });

  it("an observer reads the profile too; a dead socket asks for nothing", () => {
    useCicada.setState({ role: "observer" });
    render(<ProfilePanel />);
    expect(sent).toEqual([{ type: "profile", payload: {} }]);
    cleanup();
    sent = [];
    useCicada.setState({ connection: "reconnecting" });
    render(<ProfilePanel />);
    expect(sent).toEqual([]);
    expect(screen.getByTestId("profile-view").textContent).toContain("not connected");
  });
});
