import { beforeEach, describe, expect, it } from "vitest";
import { canWrite, roleChangeNotice, useCicada, writeBlockReason } from "./store";

describe("canWrite", () => {
  it("needs the lease AND an open socket", () => {
    expect(canWrite({ role: "writer", connection: "open" })).toBe(true);
    expect(canWrite({ role: "observer", connection: "open" })).toBe(false);
    expect(canWrite({ role: "writer", connection: "reconnecting" })).toBe(false);
    expect(canWrite({ role: "writer", connection: "closed" })).toBe(false);
    expect(writeBlockReason({ role: "writer", connection: "reconnecting" })).toBe("not connected");
    expect(writeBlockReason({ role: "observer", connection: "open" })).toBe("read-only observer");
    expect(writeBlockReason({ role: "writer", connection: "open" })).toBeNull();
  });
});

describe("disconnect / reconnect bookkeeping", () => {
  beforeEach(() => {
    useCicada.setState({
      connection: "open",
      role: "writer",
      lease: { writer: 1, clients: [[1, "writer"]] },
      hello: {
        clientId: 1,
        role: "writer",
        protocol: 1,
        engine: "x",
        project: "p",
        pipeline: "a.cic",
        unitPx: 24,
      },
      notices: [],
      probe: null,
    });
  });

  it("markDisconnected clears the session identity but keeps the mirror and hello", () => {
    useCicada.setState({ text: "a = 1" });
    useCicada.getState().markDisconnected("socket closed (1006)", { attempt: 1, nextAt: 123 });
    const s = useCicada.getState();
    expect(s.connection).toBe("reconnecting");
    expect(s.role).toBe("observer");
    expect(s.lease).toEqual({ writer: null, clients: [] });
    expect(s.reconnect).toEqual({ attempt: 1, nextAt: 123 });
    expect(s.hello?.clientId).toBe(1);
    expect(s.text).toBe("a = 1");
    expect(canWrite(s)).toBe(false);
  });

  it("a re-hello re-establishes identity, clears reconnect state on open, and says so", () => {
    useCicada.getState().markDisconnected("gone", { attempt: 2, nextAt: null });
    useCicada.getState().setConnection("open");
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 0,
      type: "hello",
      payload: { client_id: 7, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "a.cic", unit_px: 24 },
    });
    const s = useCicada.getState();
    expect(s.connection).toBe("open");
    expect(s.reconnect).toBeNull();
    expect(s.role).toBe("writer");
    expect(s.hello?.clientId).toBe(7);
    expect(canWrite(s)).toBe(true);
    expect(s.notices.at(-1)?.message).toMatch(/reconnected as client #7/);
  });
});

describe("lease change notices", () => {
  it("is loud when the lease is lost, informative when gained, silent otherwise", () => {
    const lease = { writer: 3, clients: [[1, "observer"], [3, "writer"]] as [number, "observer" | "writer"][] };
    expect(roleChangeNotice("writer", "observer", lease)).toEqual({
      level: "warning",
      message: "write lease taken by client #3 — you are read-only now",
    });
    expect(roleChangeNotice("observer", "writer", lease)?.level).toBe("info");
    expect(roleChangeNotice("writer", "writer", lease)).toBeNull();
  });

  it("the store raises them from `lease` messages", () => {
    useCicada.setState({ role: "writer", notices: [] });
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 1,
      type: "lease",
      payload: { lease: { writer: 2, clients: [[1, "observer"], [2, "writer"]] }, role: "observer" },
    });
    expect(useCicada.getState().role).toBe("observer");
    expect(useCicada.getState().notices.at(-1)).toMatchObject({
      level: "warning",
      message: "write lease taken by client #2 — you are read-only now",
    });
  });
});
