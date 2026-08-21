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
import { getClient, startConnection } from "./connection";
import { stopGitStatus } from "./git";
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
