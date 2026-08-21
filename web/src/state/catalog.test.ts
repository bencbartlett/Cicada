/**
 * The catalog refresh policy (docs/17 Follow-ups: the stale catalog after
 * a scripts-change reload): WHICH server messages re-read `/api/catalog`
 * (every `snapshot` — the join's, a barrier, an `apply_text` that changed
 * scripts — and nothing else), HOW reads are sequenced (one in flight, one
 * follow-up for any number of snapshots that land meanwhile, so the store
 * never keeps an older catalog on top of a newer one), and what a read
 * does to the store (replace on success, notice + keep on failure).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Catalog, CatalogNode, ServerEnvelope } from "../protocol/messages";
import { CatalogRefreshPolicy, catalogPolicy, feedCatalogPolicy, readCatalog, startCatalogRefresh, stopCatalogRefresh } from "./catalog";
import { useCicada } from "./store";

const HISTORY = { can_undo: false, can_redo: false, undo_label: null, redo_label: null, depth: 0 };
const GRAPH = { nodes: [], wires: [], diagnostics: [] };
const SUMMARY = {
  generation: 0,
  running: false,
  cancelled: false,
  computed: 0,
  cached: 0,
  pending: 0,
  red: 0,
  blocked: 0,
  elapsed_ms: 0,
  eta_rough: false,
};

/** A snapshot envelope the way the server sends each kind (`session.rs`). */
function snapshot(barrier: boolean, reason: string, seq = 1): ServerEnvelope {
  return {
    v: 1,
    seq,
    type: "snapshot",
    payload: {
      graph: GRAPH,
      text: "# cicada 1\n",
      statuses: {},
      summary: SUMMARY,
      lease: { writer: 1, clients: [[1, "writer"]] },
      barrier,
      reason,
      history: HISTORY,
    },
  };
}

function catalogOf(...names: string[]): Catalog {
  return {
    format: 2,
    nodes: names.map(
      (name): CatalogNode => ({
        name,
        title: name,
        description: "",
        category: "Test",
        tier: "S",
        version: 1,
        pure: true,
        uses_tolerance: false,
        gh: null,
        examples: [],
        inputs: [],
        outputs: [],
      }),
    ),
  };
}

/** A read whose completion the test controls. */
function gate(): { read: () => Promise<void>; release: () => void; fail: (error: Error) => void } {
  let resolve: (() => void) | null = null;
  let reject: ((error: Error) => void) | null = null;
  return {
    read: () =>
      new Promise<void>((res, rej) => {
        resolve = res;
        reject = rej;
      }),
    release: () => resolve?.(),
    fail: (error) => reject?.(error),
  };
}

/** Let settled promise callbacks run (no timers anywhere in the policy). */
const settle = () => new Promise<void>((res) => queueMicrotask(res));

describe("feedCatalogPolicy", () => {
  it("every snapshot reads — the join's, a barrier, an apply_text that changed scripts; nothing else does", () => {
    const policy = { onSnapshot: vi.fn() };
    const feed = (message: ServerEnvelope) => feedCatalogPolicy(policy, message);

    feed({
      v: 1,
      seq: 0,
      type: "hello",
      payload: { client_id: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unit_px: 24 },
    });
    expect(policy.onSnapshot, "the hello's hydration ends in a snapshot — that one reads").toHaveBeenCalledTimes(0);

    feed(snapshot(false, "initial"));
    expect(policy.onSnapshot, "the join's snapshot: first connect AND every reconnect").toHaveBeenCalledTimes(1);
    feed(snapshot(true, "external file change", 2));
    expect(policy.onSnapshot, "the watcher's barrier — text or scripts, the server does not say which").toHaveBeenCalledTimes(2);
    feed(snapshot(true, "git revert", 3));
    expect(policy.onSnapshot, "a revert may have put a script back").toHaveBeenCalledTimes(3);
    feed(snapshot(false, "apply_text: add a node", 4));
    expect(policy.onSnapshot, "apply_text answers with a snapshot only when a script changed").toHaveBeenCalledTimes(4);

    feed({
      v: 1,
      seq: 5,
      type: "delta",
      payload: { source: { client: 1, label: "move size" }, graph: GRAPH, text: "# cicada 1\n", dirty: [], history: HISTORY },
    });
    feed({ v: 1, seq: 6, type: "status", payload: { generation: 1, nodes: {}, summary: SUMMARY } });
    feed({ v: 1, seq: 7, type: "lease", payload: { lease: { writer: 1, clients: [] }, role: "writer" } });
    feed({ v: 1, seq: 8, type: "notice", payload: { level: "info", message: "x" } });
    feed({ v: 1, seq: 9, type: "display_reset", payload: { generation: 1 } });
    expect(policy.onSnapshot, "a delta is this session's own text/sidecar write — it cannot touch scripts/").toHaveBeenCalledTimes(4);
  });
});

describe("CatalogRefreshPolicy", () => {
  it("one read in flight; any number of snapshots meanwhile collapse into exactly one follow-up", async () => {
    const g = gate();
    const read = vi.fn(g.read);
    const policy = new CatalogRefreshPolicy(read);
    expect(policy.busy).toBe(false);

    policy.onSnapshot();
    expect(read).toHaveBeenCalledTimes(1);
    expect(policy.reads).toBe(1);
    expect(policy.busy).toBe(true);

    // Three more snapshots while the first read is in flight (a burst of
    // saves): nothing starts, one follow-up is owed.
    policy.onSnapshot();
    policy.onSnapshot();
    policy.onSnapshot();
    expect(read, "no concurrent reads — they would race to the store").toHaveBeenCalledTimes(1);

    g.release();
    await settle();
    await settle();
    expect(read, "the owed follow-up ran once, not three times").toHaveBeenCalledTimes(2);
    expect(policy.reads).toBe(2);
    expect(policy.busy, "the follow-up is in flight").toBe(true);

    g.release();
    await settle();
    await settle();
    expect(read, "nothing was owed after the follow-up").toHaveBeenCalledTimes(2);
    expect(policy.busy).toBe(false);
  });

  it("a failed read neither stops the policy nor loses the owed follow-up", async () => {
    const g = gate();
    const read = vi.fn(g.read);
    const policy = new CatalogRefreshPolicy(read);
    policy.onSnapshot();
    policy.onSnapshot();
    g.fail(new Error("catalog: HTTP 503"));
    await settle();
    await settle();
    expect(read, "the snapshot that landed during the failed read still gets its read").toHaveBeenCalledTimes(2);
    g.release();
    await settle();
    await settle();
    policy.onSnapshot();
    expect(read, "and the policy keeps reading afterwards").toHaveBeenCalledTimes(3);
  });

  it("a disposed policy reads no more", async () => {
    const g = gate();
    const read = vi.fn(g.read);
    const policy = new CatalogRefreshPolicy(read);
    policy.onSnapshot();
    policy.onSnapshot();
    policy.dispose();
    g.release();
    await settle();
    await settle();
    policy.onSnapshot();
    expect(read, "neither the owed follow-up nor a later snapshot reads after dispose").toHaveBeenCalledTimes(1);
    expect(policy.busy).toBe(false);
  });
});

describe("startCatalogRefresh / stopCatalogRefresh", () => {
  afterEach(() => stopCatalogRefresh());

  it("installs the policy the debug handle reads, and the socket's snapshots drive it", () => {
    expect(catalogPolicy()).toBeNull();
    const read = vi.fn(() => Promise.resolve());
    const policy = startCatalogRefresh(read);
    expect(catalogPolicy()).toBe(policy);
    expect(read, "installing reads nothing — the first snapshot does").toHaveBeenCalledTimes(0);
    feedCatalogPolicy(policy, snapshot(false, "initial"));
    expect(read).toHaveBeenCalledTimes(1);
    stopCatalogRefresh();
    expect(catalogPolicy()).toBeNull();
  });
});

describe("readCatalog", () => {
  beforeEach(() => {
    useCicada.setState({ catalog: null, notices: [] });
    useCicada.getState().setIdentity("tok", "sub/p.cic");
  });

  it("replaces the store's catalog from GET /api/catalog?pipeline=… with the token header", async () => {
    const fetchImpl = vi.fn(() => Promise.resolve(new Response(JSON.stringify(catalogOf("series", "my_script")), { status: 200 })));
    await readCatalog(fetchImpl);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("/api/catalog?pipeline=sub%2Fp.cic");
    expect((init.headers as Record<string, string>)["X-Cicada-Token"]).toBe("tok");
    expect(useCicada.getState().catalog?.nodes.map((n) => n.name)).toEqual(["series", "my_script"]);
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("a second read swaps the whole object (the canvas indexes per object), so a script node's arrival shows", async () => {
    await readCatalog(() => Promise.resolve(new Response(JSON.stringify(catalogOf("series")), { status: 200 })));
    const before = useCicada.getState().catalog;
    await readCatalog(() => Promise.resolve(new Response(JSON.stringify(catalogOf("series", "new_script")), { status: 200 })));
    const after = useCicada.getState().catalog;
    expect(after).not.toBe(before);
    expect(after?.nodes.map((n) => n.name)).toEqual(["series", "new_script"]);
  });

  it("a failed read is an error notice and the previous catalog stays", async () => {
    await readCatalog(() => Promise.resolve(new Response(JSON.stringify(catalogOf("series")), { status: 200 })));
    await readCatalog(() => Promise.resolve(new Response("bad token", { status: 401 })));
    const state = useCicada.getState();
    expect(state.catalog?.nodes.map((n) => n.name), "better a stale search box than an empty one").toEqual(["series"]);
    expect(state.notices.map((n) => [n.level, n.message])).toEqual([["error", "Error: catalog: HTTP 401"]]);
  });
});
