/**
 * The connection module's WIRING, against a fake socket: `startConnection`
 * hands every envelope the socket delivers to the store, the catalog policy
 * and the git policy. The policies' own tests (`catalog.test.ts`,
 * `git.test.ts`) hold each in isolation and cannot see a feed that was
 * never called — the 2026-08-21 review removed `feedCatalogPolicy` from
 * `onMessage` and 242 tests stayed green, with the Playwright search spec
 * as the only net. This file is the unit-level net: a `snapshot` on the
 * socket must become exactly one `GET /api/catalog` with the token, a
 * `delta` none, and `window.__cicada.catalog()` must report it.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ServerEnvelope } from "../protocol/messages";
import { catalogPolicy, stopCatalogRefresh } from "./catalog";
import { getClient, optionsForRoute, startConnection, stopConnection, syncConnection } from "./connection";
import { stopGitStatus } from "./git";
import { readRecent } from "./recent";
import type { Route } from "./route";
import { useCicada } from "./store";

const HISTORY = { can_undo: false, can_redo: false, undo_label: null, redo_label: null, depth: 0 };
/** No time params: the bar is hidden and playback moves nothing (`TransportView`). */
const IDLE_TRANSPORT = { playing: false, speed: 1, t_ms: 0, frame: 0, frames: 120, period_ms: 4000, driven: [] };
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

function snapshot(seq: number, barrier: boolean, reason: string): ServerEnvelope {
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
      transport: IDLE_TRANSPORT,
    },
  };
}

const HELLO: ServerEnvelope = {
  v: 1,
  seq: 0,
  type: "hello",
  payload: { client_id: 1, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unit_px: 24 },
};

const DELTA: ServerEnvelope = {
  v: 1,
  seq: 3,
  type: "delta",
  payload: { source: { client: 1, label: "set size" }, graph: GRAPH, text: "# cicada 1\n", dirty: [], history: HISTORY },
};

/** What `CicadaClient` asks of a WebSocket, and nothing more; the test delivers messages by calling the handlers. */
class FakeSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeSocket[] = [];

  readyState = FakeSocket.CONNECTING;
  binaryType = "blob";
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string | ArrayBuffer }) => void) | null = null;
  onclose: ((event: { code: number; reason: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  sent: string[] = [];

  constructor(readonly url: string) {
    FakeSocket.instances.push(this);
  }

  /** The server accepted the socket. */
  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.();
  }

  /** A control-plane text from the server. */
  deliver(envelope: ServerEnvelope): void {
    this.onmessage?.({ data: JSON.stringify(envelope) });
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(): void {
    this.readyState = FakeSocket.CLOSED;
  }
}

const catalogText = (...names: string[]) =>
  JSON.stringify({
    format: 2,
    nodes: names.map((name) => ({
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
    })),
  });

/**
 * Let every settled promise callback run: the reader is promise-only (no
 * timers), but `Response.text()` and the policy's `.finally` span several
 * microtask turns — a zero-delay macrotask drains them all.
 */
const settle = () => new Promise<void>((res) => setTimeout(res, 0));

describe("startConnection wires the socket to the catalog policy", () => {
  const fakeWindow = {
    location: { protocol: "http:", host: "127.0.0.1:8420" },
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  /** Every fetch the page made, by URL; the catalog route answers with `answer`, anything else (the git status) is refused. */
  const fetches: string[] = [];
  let answer = catalogText("series", "my_script");

  beforeEach(() => {
    FakeSocket.instances = [];
    fetches.length = 0;
    useCicada.setState({ catalog: null, notices: [] });
    vi.stubGlobal("window", fakeWindow);
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) => {
        fetches.push(url);
        if (url.startsWith("/api/catalog?")) return Promise.resolve(new Response(answer, { status: 200 }));
        return Promise.resolve(new Response("not in this test", { status: 503 }));
      }),
    );
  });

  afterEach(() => {
    getClient()?.close();
    stopCatalogRefresh();
    stopGitStatus(fakeWindow);
    vi.unstubAllGlobals();
  });

  it("a snapshot on the socket is one GET /api/catalog with the token; hello and delta are none; the debug handle counts", async () => {
    startConnection({ token: "tok", pipeline: "sub/p.cic" });
    expect(FakeSocket.instances).toHaveLength(1);
    const socket = FakeSocket.instances[0]!;
    expect(socket.url).toBe("ws://127.0.0.1:8420/ws?token=tok&pipeline=sub%2Fp.cic");
    const policy = catalogPolicy();
    expect(policy, "startConnection installs the catalog policy").not.toBeNull();
    const catalogFetches = () => fetches.filter((url) => url.startsWith("/api/catalog?"));

    socket.open();
    socket.deliver(HELLO);
    await settle();
    expect(policy!.reads, "hello reads no catalog — the join's snapshot will").toBe(0);
    expect(catalogFetches()).toEqual([]);

    socket.deliver(snapshot(1, false, "initial"));
    expect(policy!.reads, "the join's snapshot reads").toBe(1);
    expect(catalogFetches()).toEqual(["/api/catalog?pipeline=sub%2Fp.cic"]);
    const fetchMock = vi.mocked(fetch);
    const catalogCall = fetchMock.mock.calls.find(([url]) => String(url).startsWith("/api/catalog?"));
    expect(((catalogCall?.[1] as RequestInit | undefined)?.headers as Record<string, string>)["X-Cicada-Token"]).toBe("tok");
    await settle();
    expect(useCicada.getState().catalog?.nodes.map((n) => n.name), "the answer landed in the store").toEqual(["series", "my_script"]);

    socket.deliver(DELTA);
    await settle();
    expect(policy!.reads, "a delta is this session's own write — no read").toBe(1);
    expect(catalogFetches()).toHaveLength(1);

    answer = catalogText("series", "my_script", "added");
    socket.deliver(snapshot(4, true, "external file change"));
    expect(policy!.reads, "the watcher's barrier reads again").toBe(2);
    await settle();
    expect(useCicada.getState().catalog?.nodes.map((n) => n.name)).toEqual(["series", "my_script", "added"]);

    const handle = (window as unknown as { __cicada: { catalog: () => { reads: number; busy: boolean; nodes: number } } }).__cicada;
    expect(handle.catalog(), "what Playwright reads").toEqual({ reads: 2, busy: false, nodes: 3 });
  });
});

/**
 * The route drives the socket (docs/16 §Application layout; docs/13 — the
 * join hint): a pipeline route opens ONE socket to that pipeline, the
 * pop-out route's hello declares the observer, another pipeline closes the
 * first socket and resets the store before the next join, the same route
 * again (a `popstate` back to the open file) touches nothing, and the
 * picker leaves no socket. The hello remembers the pipeline under Recent.
 */
describe("syncConnection follows the route", () => {
  const fakeWindow = {
    location: { protocol: "http:", host: "127.0.0.1:8420" },
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  const storage = new Map<string, string>();
  const fakeStorage = {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
  };

  beforeEach(() => {
    FakeSocket.instances = [];
    storage.clear();
    vi.stubGlobal("window", fakeWindow);
    vi.stubGlobal("WebSocket", FakeSocket);
    vi.stubGlobal("localStorage", fakeStorage);
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response("not in this test", { status: 503 }))));
  });

  afterEach(() => {
    stopConnection();
    stopCatalogRefresh();
    stopGitStatus(fakeWindow);
    vi.unstubAllGlobals();
  });

  const route = (pipeline: string | undefined, view: "app" | "viewport" = "app"): Route => ({ token: "tok", pipeline, view });
  const hello = (pipeline: string): ServerEnvelope => ({ ...HELLO, payload: { ...HELLO.payload, pipeline } });
  const debug = () => (window as unknown as { __cicada: { connection: () => unknown } }).__cicada.connection();

  it("opens one socket per pipeline, switches by closing the first, ignores the same route, and closes for the picker", () => {
    expect(optionsForRoute(route(undefined))).toBeNull();
    expect(optionsForRoute({ token: undefined, pipeline: "a.cic", view: "app" })).toBeNull();
    expect(optionsForRoute(route("a.cic"))).toEqual({ token: "tok", pipeline: "a.cic" });
    expect(optionsForRoute(route("a.cic", "viewport"))).toEqual({ token: "tok", pipeline: "a.cic", role: "observer" });

    syncConnection(route("a.cic"));
    expect(FakeSocket.instances).toHaveLength(1);
    const first = FakeSocket.instances[0]!;
    expect(first.url).toBe("ws://127.0.0.1:8420/ws?token=tok&pipeline=a.cic");
    expect(useCicada.getState().pipeline).toBe("a.cic");
    expect(useCicada.getState().connection).toBe("connecting");
    first.open();
    first.deliver(hello("a.cic"));
    expect(useCicada.getState().hello?.pipeline).toBe("a.cic");
    expect(JSON.parse(first.sent[0]!).payload, "the main window's hello carries no role").toEqual({ v: 1 });
    expect(readRecent(fakeStorage), "remembered on the hello").toEqual(["a.cic"]);
    expect(debug()).toEqual({ token: "tok", pipeline: "a.cic" });

    syncConnection(route("a.cic"));
    expect(FakeSocket.instances, "the same route again: no new socket").toHaveLength(1);
    expect(first.readyState).toBe(FakeSocket.OPEN);

    useCicada.setState({ text: "old text" });
    syncConnection(route("sub/b.cic"));
    expect(first.readyState, "the first socket is closed by us").toBe(FakeSocket.CLOSED);
    expect(FakeSocket.instances).toHaveLength(2);
    const second = FakeSocket.instances[1]!;
    expect(second.url).toBe("ws://127.0.0.1:8420/ws?token=tok&pipeline=sub%2Fb.cic");
    const s = useCicada.getState();
    expect([s.pipeline, s.text, s.hello, s.connection], "the store is reset before the next join").toEqual(["sub/b.cic", "", null, "connecting"]);
    expect(getClient(), "the live client is the second").not.toBeNull();
    second.open();
    second.deliver(hello("sub/b.cic"));
    expect(readRecent(fakeStorage)).toEqual(["sub/b.cic", "a.cic"]);

    syncConnection(route(undefined));
    expect(second.readyState).toBe(FakeSocket.CLOSED);
    expect(getClient()).toBeNull();
    expect(debug()).toBeNull();
    expect(useCicada.getState().pipeline).toBe("");
    expect(useCicada.getState().hello).toBeNull();
    syncConnection(route(undefined));
    expect(FakeSocket.instances, "the picker again: nothing to do").toHaveLength(2);
  });

  it("the pop-out route joins as a declared observer: role: observer in its hello", () => {
    syncConnection(route("a.cic", "viewport"));
    const socket = FakeSocket.instances[0]!;
    socket.open();
    expect(JSON.parse(socket.sent[0]!)).toEqual({ v: 1, id: "1", type: "hello", payload: { v: 1, role: "observer" } });
    expect(debug()).toEqual({ token: "tok", pipeline: "a.cic", role: "observer" });
    // The same pipeline as the app (no role) is a different connection.
    syncConnection(route("a.cic"));
    expect(FakeSocket.instances).toHaveLength(2);
    expect(JSON.parse(FakeSocket.instances[1]!.sent[0] ?? "{}").payload ?? null, "not sent before open").toBeNull();
  });

  it("a socket we closed reports nothing over the next pipeline's store", () => {
    syncConnection(route("a.cic"));
    const first = FakeSocket.instances[0]!;
    first.open();
    syncConnection(route("b.cic"));
    // The browser fires the old socket's close AFTER the switch: the new
    // pipeline's "connecting" must stand, and no reconnect may be scheduled
    // for a socket we abandoned.
    first.onclose?.({ code: 1000, reason: "" });
    expect(useCicada.getState().connection).toBe("connecting");
    expect(useCicada.getState().pipeline).toBe("b.cic");
    expect(FakeSocket.instances).toHaveLength(2);
  });
});
