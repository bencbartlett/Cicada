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
    useCicada.setState({ profileAsk: null, profileRefusal: null });
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return String(sent.length);
    });
    useInspectorTab.setState({ tab: "profile", profileFocus: null });
  });
  afterEach(cleanup);

  /** The server's answer to the outstanding read (the sender's ids are the send count; an answer clears whichever is in flight). */
  const answer = (generation: number) =>
    act(() => useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "profile_view", payload: { ...profile, generation } }));

  it("asks for the profile when it shows, once per LANDED pass, never while one paints and never on a status bump", () => {
    render(<ProfilePanel />);
    expect(sent).toEqual([{ type: "profile", payload: {} }]);
    expect(screen.getByTestId("profile-view").getAttribute("data-generation")).toBe("none");
    answer(12);
    act(() => useCicada.setState({ display: pass(13, "painting") }));
    expect(sent, "a pass in flight is not asked for").toHaveLength(1);
    // The next solve starts (the status flips `running`, then names the
    // generation) while the previous pass stands painted: NOT a landed pass
    // — the first build asked here too, twice per generation (L5-4 / C3).
    act(() => useCicada.setState({ display: pass(12, "painted"), summary: { ...useCicada.getState().summary, generation: 13, running: true } }));
    expect(sent, "a status bump is not a landed pass").toHaveLength(1);
    act(() => useCicada.setState({ summary: { ...useCicada.getState().summary, running: false } }));
    expect(sent).toHaveLength(1);
    act(() => useCicada.setState({ display: pass(13, "painted") }));
    expect(sent, "the landed pass").toHaveLength(2);
    answer(13);
    // A re-hydration asks again for the same generation.
    act(() => useCicada.setState({ snapshots: 2 }));
    expect(sent).toHaveLength(3);
  });

  it("keeps at most one read outstanding: passes landing while an answer is awaited coalesce into ONE read after it", () => {
    render(<ProfilePanel />);
    expect(sent).toHaveLength(1);
    expect(useCicada.getState().profileAsk).toBe("1");
    // Three passes land before the answer to the first read arrives (a drag
    // on a heavy pipeline: the answers are O(nodes) and lag the frames).
    for (const generation of [13, 14, 15]) {
      act(() => useCicada.setState({ display: pass(generation, "painted") }));
    }
    expect(sent, "nothing more while the read is outstanding").toHaveLength(1);
    answer(12);
    expect(useCicada.getState().profileAsk, "the answer arrived: the latest landed pass is asked for, once").toBe("2");
    expect(sent).toHaveLength(2);
    answer(15);
    expect(useCicada.getState().profileAsk).toBeNull();
    expect(sent, "two reads for four landed passes, never two in flight").toHaveLength(2);
    expect(screen.getByTestId("profile-view").getAttribute("data-generation")).toBe("15");
    // The same generation answered again (a re-hydration's read) re-renders nothing.
    const before = useCicada.getState().profile;
    act(() => useCicada.setState({ snapshots: 2 }));
    answer(15);
    expect(useCicada.getState().profile).toBe(before);
  });

  it("a refusal of its own read is the placeholder, not a notice: the session's first generation is still solving", () => {
    useCicada.setState({ display: null, summary: { ...useCicada.getState().summary, generation: 1, running: true } });
    render(<ProfilePanel />);
    expect(sent).toHaveLength(1);
    act(() =>
      useCicada.getState().applyServerMessage({
        v: 1,
        seq: 0,
        type: "error",
        payload: { intent_id: "1", kind: "invalid", message: "profile: no generation has completed yet" },
      }),
    );
    expect(useCicada.getState().notices, "no toast for a normal state").toEqual([]);
    expect(useCicada.getState().profileAsk).toBeNull();
    expect(screen.getByTestId("profile-waiting").textContent).toBe("profile: no generation has completed yet");
    expect(sent, "not asked again until a pass lands").toHaveLength(1);
    // The first pass lands: asked again, answered, the placeholder gone.
    act(() => useCicada.setState({ display: pass(1, "painted") }));
    expect(sent).toHaveLength(2);
    answer(1);
    expect(useCicada.getState().profileRefusal).toBeNull();
    expect(screen.getByTestId("profile-view").getAttribute("data-generation")).toBe("1");
    // Another client's-worth of refusal (an intent id that is not our read's) is still a notice.
    act(() => useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "error", payload: { intent_id: "zz", kind: "invalid", message: "something else" } }));
    expect(useCicada.getState().notices.map((n) => n.message)).toEqual(["something else"]);
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

  it("a pass Esc cut is presented as cut, never as a complete pass that drew nothing", () => {
    // The store's pass for the cut generation: no frame, stamped at its end.
    useCicada.setState({ display: { ...pass(12, "painted"), frames: 0, outputs: 0, bytes: 0, cancelled: true, cutBy: "esc", paintedMs: 787 } });
    render(<ProfilePanel />);
    act(() => useCicada.setState({ profile: { ...profile, cancelled: true, cut_by: "esc", display: [], phases: { ...profile.phases, encode_ms: 0.02, bytes: 0 } } }));
    const view = screen.getByTestId("profile-view");
    expect(view.getAttribute("data-cancelled")).toBe("true");
    expect(view.getAttribute("data-cut-by")).toBe("esc");
    expect(screen.getByTestId("profile-title").textContent).toBe("gen 12 · structural · cut by Esc");
    expect(screen.getByTestId("profile-title").getAttribute("title")).toMatch(/Esc cut this generation's display pass/);
    expect(screen.getByTestId("profile-display-caption").textContent).toBe("the pass was cut before it drew — the previous picture stays");
    expect(screen.queryByTestId("profile-display")).toBeNull();
    // Nothing was painted: the first paint is not the time until Esc landed.
    expect(screen.getByTestId("profile-first-paint").textContent).toBe("—");
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

  it("the caches section shows the session's view with its budget control, before and after a profile, and takes the indicator's focus once the full view has scrolled", () => {
    // jsdom has no `scrollIntoView`: a spy stands in, so the scroll is
    // asserted to happen on the render that HAS the profile — the focus
    // consumed on the placeholder render left the section below the fold
    // once the ring and the table rendered above it (L2-P1-1).
    const scrolled: string[] = [];
    const proto = HTMLElement.prototype as unknown as { scrollIntoView?: (this: HTMLElement, options?: unknown) => void };
    const before = proto.scrollIntoView;
    proto.scrollIntoView = function scrollIntoView(this: HTMLElement) {
      scrolled.push(`${this.id}:${screen.getByTestId("profile-view").getAttribute("data-generation")}`);
    };
    try {
      useInspectorTab.setState({ tab: "profile", profileFocus: "caches" });
      render(<ProfilePanel />);
      expect(screen.getByTestId("profile-caches")).toBeTruthy();
      expect(screen.getByTestId("profile-cache-held").textContent).toBe("4.0 KB of 1024.00 MB");
      expect((screen.getByTestId("profile-display-cache") as HTMLSelectElement).value).toBe("");
      expect(screen.getByTestId("profile-display-cache-now").textContent).toBe("session: 1G");
      expect(scrolled, "scrolled on the placeholder render (harmless)").toEqual(["profile-caches:none"]);
      expect(useInspectorTab.getState().profileFocus, "NOT consumed before the full view rendered").toBe("caches");
      act(() => useCicada.setState({ profile }));
      expect(scrolled, "scrolled again once the ring and the table rendered above the section").toEqual(["profile-caches:none", "profile-caches:12"]);
      expect(useInspectorTab.getState().profileFocus, "consumed after that scroll").toBeNull();
      // A later profile does not scroll again: the focus is spent.
      act(() => useCicada.setState({ profile: { ...profile, generation: 13 } }));
      expect(scrolled).toHaveLength(2);
    } finally {
      if (before === undefined) delete proto.scrollIntoView;
      else proto.scrollIntoView = before;
    }
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
