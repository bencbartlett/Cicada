/**
 * The socket client's handshake: the `hello` it opens with carries the
 * protocol version and — only when asked for — the join hint `role:
 * "observer"` (docs/13 §Projects, pipelines, sessions; the pop-out
 * viewport). The main window's hello has NO `role` key at all (additive:
 * an older server ignores nothing it is not sent), and the hello is the
 * FIRST message on the wire.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { CicadaClient, wsUrl } from "./client";
import { PROTOCOL_VERSION } from "./version";

class FakeSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static last: FakeSocket | null = null;
  readyState = FakeSocket.CONNECTING;
  binaryType = "blob";
  onopen: (() => void) | null = null;
  onmessage: unknown = null;
  onclose: unknown = null;
  onerror: unknown = null;
  sent: string[] = [];
  constructor(readonly url: string) {
    FakeSocket.last = this;
  }
  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.();
  }
  send(data: string): void {
    this.sent.push(data);
  }
  close(): void {
    this.readyState = FakeSocket.CLOSED;
  }
}

const handlers = { onMessage: () => {}, onFrame: () => {} };

describe("CicadaClient's hello", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("carries the version and no role unless asked — the main window's join", () => {
    const client = new CicadaClient("ws://x/ws", handlers);
    expect(client.hello()).toEqual({ type: "hello", payload: { v: PROTOCOL_VERSION } });
    expect("role" in client.hello().payload, "no `role` key at all, not `undefined`").toBe(false);
  });

  it("carries role: observer for a declared observer — the pop-out viewport's join", () => {
    const client = new CicadaClient("ws://x/ws", handlers, { role: "observer" });
    expect(client.hello()).toEqual({ type: "hello", payload: { v: PROTOCOL_VERSION, role: "observer" } });
  });

  it("is the first message on the wire, as the envelope the server parses", () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    const client = new CicadaClient("ws://x/ws", handlers, { role: "observer" });
    client.connect();
    const socket = FakeSocket.last!;
    socket.open();
    expect(socket.sent).toHaveLength(1);
    expect(JSON.parse(socket.sent[0]!)).toEqual({
      v: PROTOCOL_VERSION,
      id: "1",
      type: "hello",
      payload: { v: PROTOCOL_VERSION, role: "observer" },
    });
    client.close();
  });

  it("close() detaches the socket's message and error handlers — a closed socket reports nothing but its close", () => {
    vi.stubGlobal("WebSocket", FakeSocket);
    const onMessage = vi.fn();
    const onError = vi.fn();
    const onClose = vi.fn();
    const client = new CicadaClient("ws://x/ws", { onMessage, onFrame: () => {}, onError, onClose });
    client.connect();
    const socket = FakeSocket.last!;
    socket.open();
    expect(socket.onmessage).not.toBeNull();
    client.close();
    expect(socket.readyState).toBe(FakeSocket.CLOSED);
    expect(client.isOpen).toBe(false);
    expect(socket.onmessage, "no message handler after close").toBeNull();
    expect(socket.onerror, "no error handler after close").toBeNull();
    // The close event still reports `closedByUs` — the connection module's
    // guard reads it for the socket it closed itself.
    (socket.onclose as (event: { code: number; reason: string }) => void)({ code: 1000, reason: "" });
    expect(onClose).toHaveBeenCalledWith("closed", true);
    expect(onMessage).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });
});

describe("wsUrl", () => {
  it("names the pipeline root-relative, URL-encoded, with the token", () => {
    const base = { protocol: "http:", host: "127.0.0.1:8420" } as Location;
    expect(wsUrl("t", "sub dir/p.cic", base)).toBe("ws://127.0.0.1:8420/ws?token=t&pipeline=sub+dir%2Fp.cic");
    expect(wsUrl("t", "p.cic", { protocol: "https:", host: "h" } as Location)).toBe("wss://h/ws?token=t&pipeline=p.cic");
  });
});
