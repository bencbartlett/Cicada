import { beforeEach, describe, expect, it, test } from "vitest";
import type { ClientMessage, GraphView, HistoryView, NodeView, ServerEnvelope } from "../protocol/messages";
import {
  EMPTY_CACHES,
  EMPTY_HISTORY,
  canWrite,
  dragStandsAfter,
  errorNoticeLevel,
  lastErrorOf,
  modalOpen,
  pendingFor,
  pruneKeys,
  roleChangeNotice,
  settingsFrom,
  useCicada,
  writeBlockReason,
} from "./store";
import { TRANSPORT_AT_REST } from "./transport";

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

describe("the profile read's lifecycle (docs/13 §The profiler; v0.1 wave 5 P1)", () => {
  let sent: ClientMessage[];
  beforeEach(() => {
    sent = [];
    useCicada.setState({ connection: "open", profile: null, profileAsk: null, profileRefusal: null, notices: [], lastError: null });
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return `id-${sent.length}`;
    });
  });

  it("askProfile sends ONE read and keeps it outstanding until the answer; a second ask meanwhile sends nothing", () => {
    useCicada.getState().askProfile();
    useCicada.getState().askProfile();
    expect(sent).toEqual([{ type: "profile", payload: {} }]);
    expect(useCicada.getState().profileAsk).toBe("id-1");
    const view = { generation: 4, kind: "structural", phases: { queued_ms: 0, solve_ms: 1, tessellate_ms: 2, encode_ms: 0.5, bytes: 10 }, nodes: [], display: [], caches: EMPTY_CACHES };
    useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "profile_view", payload: view });
    expect(useCicada.getState().profileAsk).toBeNull();
    expect(useCicada.getState().profile).toEqual(view);
    useCicada.getState().askProfile();
    expect(sent).toHaveLength(2);
    // The same generation answered again is not a new profile object (nothing re-renders on it).
    const held = useCicada.getState().profile;
    useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "profile_view", payload: { ...view } });
    expect(useCicada.getState().profile).toBe(held);
    expect(useCicada.getState().profileAsk).toBeNull();
    // A newer generation — or the same one now marked cut — replaces it.
    useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "profile_view", payload: { ...view, generation: 5 } });
    expect(useCicada.getState().profile?.generation).toBe(5);
    useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "profile_view", payload: { ...view, generation: 5, cancelled: true, cut_by: "esc" } });
    expect(useCicada.getState().profile?.cut_by).toBe("esc");
  });

  it("a refusal of the outstanding read is the panel's placeholder, never a notice; any other error still is one", () => {
    useCicada.getState().askProfile();
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 0,
      type: "error",
      payload: { intent_id: "id-1", kind: "invalid", message: "profile: no generation has completed yet" },
    });
    const s = useCicada.getState();
    expect(s.profileAsk).toBeNull();
    expect(s.profileRefusal).toBe("profile: no generation has completed yet");
    expect(s.notices).toEqual([]);
    expect(s.lastError?.message, "still the last error, for anyone who asks").toBe("profile: no generation has completed yet");
    // The next answer clears the refusal.
    useCicada.getState().askProfile();
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 0,
      type: "profile_view",
      payload: { generation: 1, kind: "structural", phases: { queued_ms: 0, solve_ms: 1, tessellate_ms: 2, encode_ms: 0.5, bytes: 10 }, nodes: [], display: [], caches: EMPTY_CACHES },
    });
    expect(useCicada.getState().profileRefusal).toBeNull();
    // An error that is not our read's answer is a notice as before.
    useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "error", payload: { intent_id: "other", kind: "invalid", message: "nope" } });
    expect(useCicada.getState().notices.map((n) => [n.level, n.message])).toEqual([["error", "nope"]]);
  });

  it("no sender yet: nothing is outstanding; a disconnect or a session reset forgets an outstanding read and its refusal", () => {
    useCicada.getState().installSender(() => "");
    useCicada.getState().askProfile();
    expect(useCicada.getState().profileAsk).toBeNull();
    useCicada.setState({ profileAsk: "id-9", profileRefusal: "x" });
    useCicada.getState().markDisconnected("gone", { attempt: 1, nextAt: null });
    expect(useCicada.getState().profileAsk).toBeNull();
    expect(useCicada.getState().profileRefusal).toBeNull();
    useCicada.setState({ profileAsk: "id-9", profileRefusal: "x" });
    useCicada.getState().resetSession("t", "b.cic");
    expect(useCicada.getState().profileAsk).toBeNull();
    expect(useCicada.getState().profileRefusal).toBeNull();
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
        version: null,
        threads: null,
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

  it("hello carries the build and the threads into HelloInfo (wave 5 R1); an engine that reports neither leaves them null", () => {
    const base = { client_id: 7, role: "writer" as const, protocol: 1, engine: "x", project: "p", pipeline: "a.cic", unit_px: 24 };
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 0,
      type: "hello",
      payload: { ...base, version: { semver: "0.1.0-alpha.1", commit: "a82eb39d1c2e", built: "2026-08-25" }, threads: 6 },
    });
    expect(useCicada.getState().hello?.version).toEqual({ semver: "0.1.0-alpha.1", commit: "a82eb39d1c2e", built: "2026-08-25" });
    expect(useCicada.getState().hello?.threads).toBe(6);
    useCicada.getState().applyServerMessage({ v: 1, seq: 0, type: "hello", payload: base });
    expect(useCicada.getState().hello?.version).toBeNull();
    expect(useCicada.getState().hello?.threads).toBeNull();
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


function node(name: string): NodeView {
  return {
    ref: name.length,
    name,
    targets: [name],
    line: 1,
    text: `${name} = 1.0`,
    kind: "literal",
    title: "Constant",
    category: "Params & input",
    inputs: [],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [8, 2],
    manual: false,
  };
}

function graph(...names: string[]): GraphView {
  return { nodes: names.map(node), wires: [], diagnostics: [] };
}

const hello: ServerEnvelope = {
  v: 1,
  seq: 0,
  type: "hello",
  payload: {
    client_id: 7,
    role: "writer",
    protocol: 1,
    engine: "cicada",
    project: "p",
    pipeline: "p.cic",
    unit_px: 24,
  },
};

/** The history the server reports right after pushing `label` as op number `depth`. */
function after(label: string, depth: number): HistoryView {
  return { can_undo: true, can_redo: false, undo_label: label, redo_label: null, depth };
}

test("pruneKeys keeps the record identity when nothing is pruned", () => {
  const record = { a: 1, b: 2 };
  expect(pruneKeys(record, new Set(["a", "b"]))).toBe(record);
  expect(pruneKeys(record, new Set(["a"]))).toEqual({ a: 1 });
});

test("a delta prunes dead bindings from statuses and follows renames / my placements", () => {
  const apply = useCicada.getState().applyServerMessage;
  apply(hello);
  apply({
    v: 1,
    seq: 1,
    type: "snapshot",
    payload: {
      graph: graph("a", "b"),
      text: "",
      statuses: {
        a: { state: "done", generation: 1 },
        b: { state: "red", generation: 1 },
      },
      summary: {
        generation: 1,
        running: false,
        cancelled: false,
        computed: 0,
        cached: 0,
        pending: 0,
        red: 1,
        blocked: 0,
        elapsed_ms: 0,
        eta_rough: false,
      },
      lease: { writer: 7, clients: [[7, "writer"]] },
      barrier: false,
      reason: "initial",
      history: EMPTY_HISTORY,
      transport: TRANSPORT_AT_REST,
      caches: EMPTY_CACHES,
    },
  });
  useCicada.getState().selectNodes(["b"]);
  // Rename b → c: the selection follows, b's status is gone.
  apply({
    v: 1,
    seq: 2,
    type: "delta",
    payload: {
      source: { client: 7, intent_id: "r", label: "rename b → c" },
      graph: graph("a", "c"),
      text: "",
      dirty: ["c"],
      history: after("rename b → c", 1),
    },
  });
  let state = useCicada.getState();
  expect(state.selection.nodes).toEqual(["c"]);
  expect(Object.keys(state.statuses)).toEqual(["a"]);
  // Delete c: nothing selected, nothing phantom.
  apply({
    v: 1,
    seq: 3,
    type: "delta",
    payload: {
      source: { client: 7, intent_id: "d", label: "delete c" },
      graph: graph("a"),
      text: "",
      dirty: [],
      history: after("delete c", 2),
    },
  });
  state = useCicada.getState();
  expect(state.selection.nodes).toEqual([]);
  expect(Object.keys(state.statuses)).toEqual(["a"]);
  // My own placement selects what I placed; someone else's does not.
  apply({
    v: 1,
    seq: 4,
    type: "delta",
    payload: {
      source: { client: 7, intent_id: "p", label: "place add" },
      graph: graph("a", "add_1"),
      text: "",
      dirty: ["add_1"],
      history: after("place add", 3),
    },
  });
  expect(useCicada.getState().selection.nodes).toEqual(["add_1"]);
  apply({
    v: 1,
    seq: 5,
    type: "delta",
    payload: {
      source: { client: 9, intent_id: "p", label: "place add" },
      graph: graph("a", "add_1", "add_2"),
      text: "",
      dirty: ["add_2"],
      history: after("place add", 4),
    },
  });
  expect(useCicada.getState().selection.nodes).toEqual(["add_1"]);
});

describe("node_values (the inspect answer; wave 5 N1 carries inputs beside outputs)", () => {
  const number = { kind: "Number", hash: "ab".repeat(32), samples: ["2.5"] };
  it("stores outputs AND inputs per node, and an older engine's answer without inputs as none (review L2-5, C-6)", () => {
    useCicada.setState({ nodeValues: {} });
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 4,
      type: "node_values",
      payload: { node: "span", generation: 4, outputs: [["out", null]], inputs: [["start", null], ["end", number]] },
    });
    expect(useCicada.getState().nodeValues.span).toEqual({
      generation: 4,
      outputs: [["out", null]],
      inputs: [["start", null], ["end", number]],
    });
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 5,
      type: "node_values",
      payload: { node: "old", generation: 4, outputs: [["out", number]] },
    });
    expect(useCicada.getState().nodeValues.old).toEqual({ generation: 4, outputs: [["out", number]], inputs: [] });
    expect(useCicada.getState().nodeValues.span, "other nodes' answers stay").toBeDefined();
  });
  it("a snapshot clears every answer (the values are re-asked)", () => {
    useCicada.setState({
      nodeValues: { span: { generation: 4, outputs: [], inputs: [["end", number]] } },
      selection: { nodes: [], wire: null, element: null },
      snapshots: 0,
    });
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 6,
      type: "snapshot",
      payload: {
        graph: graph("span"),
        text: "# cicada 1\nspan = 1.0\n",
        statuses: {},
        summary: useCicada.getState().summary,
        lease: { writer: null, clients: [] },
        barrier: false,
        reason: "initial",
        history: EMPTY_HISTORY,
        transport: TRANSPORT_AT_REST,
        caches: EMPTY_CACHES,
      },
    });
    expect(useCicada.getState().nodeValues).toEqual({});
  });
});

describe("history (docs/13 §Undo/redo)", () => {
  const summary = {
    generation: 1,
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
  const snapshot = (history: HistoryView, barrier = false): ServerEnvelope => ({
    v: 1,
    seq: 10,
    type: "snapshot",
    payload: {
      graph: graph("a"),
      text: "a = 1.0\n",
      statuses: {},
      summary,
      lease: { writer: 7, clients: [[7, "writer"]] },
      barrier,
      reason: barrier ? "external change" : "initial",
      history,
      transport: TRANSPORT_AT_REST,
      caches: EMPTY_CACHES,
    },
  });
  const delta = (label: string, history: HistoryView): ServerEnvelope => ({
    v: 1,
    seq: 11,
    type: "delta",
    payload: {
      source: { client: 7, intent_id: "u", label },
      graph: graph("a"),
      text: "a = 1.0\n",
      dirty: [],
      history,
    },
  });

  beforeEach(() => {
    useCicada.setState({ history: EMPTY_HISTORY, notices: [], lastError: null, lastDeltaLabel: "" });
    useCicada.getState().applyServerMessage(hello);
  });

  it("starts empty and mirrors the snapshot's history field for field", () => {
    expect(useCicada.getState().history).toEqual({
      can_undo: false,
      can_redo: false,
      undo_label: null,
      redo_label: null,
      depth: 0,
    });
    const h: HistoryView = { can_undo: true, can_redo: true, undo_label: "move a", redo_label: "delete b", depth: 3 };
    useCicada.getState().applyServerMessage(snapshot(h));
    expect(useCicada.getState().history).toEqual(h);
  });

  it("a delta replaces the history wholesale (undo: cursor back, redo available) and keeps the label", () => {
    useCicada.getState().applyServerMessage(delta("delete a", after("delete a", 1)));
    expect(useCicada.getState().history).toEqual(after("delete a", 1));
    const undone: HistoryView = { can_undo: false, can_redo: true, undo_label: null, redo_label: "delete a", depth: 0 };
    useCicada.getState().applyServerMessage(delta("undo: delete a", undone));
    const s = useCicada.getState();
    expect(s.history).toEqual(undone);
    expect(s.lastDeltaLabel).toBe("undo: delete a");
  });

  it("a reload barrier snapshot carries the cleared log", () => {
    useCicada.getState().applyServerMessage(delta("delete a", after("delete a", 1)));
    useCicada.getState().applyServerMessage(snapshot(EMPTY_HISTORY, true));
    expect(useCicada.getState().history).toEqual(EMPTY_HISTORY);
    expect(useCicada.getState().notices.at(-1)?.message).toMatch(/reloaded from disk/);
  });

  it("nothing_to_undo / nothing_to_redo are info notices that carry the server's reason; other errors stay errors", () => {
    expect(errorNoticeLevel("nothing_to_undo")).toBe("info");
    expect(errorNoticeLevel("nothing_to_redo")).toBe("info");
    expect(errorNoticeLevel("refused")).toBe("error");
    expect(errorNoticeLevel("lease")).toBe("error");
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 12,
      type: "error",
      payload: {
        intent_id: "9",
        kind: "nothing_to_undo",
        message:
          "nothing to undo — the op log was cleared by a reload barrier (an external file change — git, an editor)",
      },
    });
    const s = useCicada.getState();
    expect(s.notices.at(-1)).toMatchObject({ level: "info", message: expect.stringMatching(/reload barrier/) });
    expect(s.lastError).toEqual({
      intentId: "9",
      kind: "nothing_to_undo",
      message: expect.stringMatching(/^nothing to undo/),
    });
  });

  it("a failed batch's error keeps the failing op's kind and index; apply_text details are named", () => {
    useCicada.getState().applyServerMessage({
      v: 1,
      seq: 13,
      type: "error",
      payload: {
        intent_id: "b",
        kind: "refused",
        message: "batch `delete 3 nodes` failed at op 2 (delete_node): no node named `c`",
        index: 2,
      },
    });
    expect(useCicada.getState().lastError).toEqual({
      intentId: "b",
      kind: "refused",
      message: expect.stringMatching(/failed at op 2/),
      index: 2,
    });
    expect(useCicada.getState().notices.at(-1)?.level).toBe("error");
    expect(lastErrorOf({ kind: "stale_base", message: "stale", current_text_hash: "ff" })).toEqual({
      kind: "stale_base",
      message: "stale",
      currentTextHash: "ff",
    });
    const diagnostics = [{ kind: "parse", span: { line: 1, col_start: 0, col_end: 1 }, message: "bad" }];
    expect(lastErrorOf({ kind: "parse_error", message: "p", diagnostics })).toEqual({
      kind: "parse_error",
      message: "p",
      diagnostics,
    });
  });
});

describe("compute-on-release (docs/13 §Slider drags — the frozen client contract)", () => {
  const summary = {
    generation: 1,
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
  /** The wire shape the server's `preview_policy_encodes_the_documented_shape` test pins. */
  const policy = (
    seq: number,
    payload: { node: string; port?: string; estimate_ms: number; rough: boolean; pending_value: string },
  ): ServerEnvelope => ({
    v: 1,
    seq,
    type: "preview_policy",
    payload: { mode: "compute_on_release", ...payload },
  });
  const delta = (seq: number, label: string): ServerEnvelope => ({
    v: 1,
    seq,
    type: "delta",
    payload: {
      source: { client: 7, intent_id: "r", label },
      graph: graph("deboss", "x"),
      text: "",
      dirty: ["deboss"],
      history: after(label, 1),
    },
  });
  const status = (seq: number): ServerEnvelope => ({
    v: 1,
    seq,
    type: "status",
    payload: { generation: 3, nodes: { carved: { state: "cached", generation: 3, nanos: 4.39e10 } }, summary },
  });

  beforeEach(() => {
    useCicada.setState({ pending: null, notices: [], lastError: null, statuses: {} });
    useCicada.getState().applyServerMessage(hello);
  });

  it("a cheap cone never hears of the policy: no message, no pending state", () => {
    const s = useCicada.getState();
    expect(s.pending).toBeNull();
    expect(pendingFor(s, "size", "value")).toBeUndefined();
    // Previews, frames and statuses on their own leave it that way.
    s.applyServerMessage(status(1));
    expect(useCicada.getState().pending).toBeNull();
  });

  it("the message sets the pending param field for field (an absent port is a bare literal: null)", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    const s = useCicada.getState();
    expect(s.pending).toEqual({
      node: "deboss",
      port: "value",
      mode: "compute_on_release",
      value: "0.875",
      estimateMs: 3990.9,
      rough: false,
      seq: 5,
    });
    expect(pendingFor(s, "deboss", "value")).toBe(s.pending);
    expect(pendingFor(s, "deboss", null), "another port of the same binding is not pending").toBeUndefined();
    expect(pendingFor(s, "amps", "value")).toBeUndefined();
    apply(policy(6, { node: "x", estimate_ms: 1000, rough: true, pending_value: "2.0" }));
    expect(useCicada.getState().pending).toMatchObject({ node: "x", port: null, rough: true, estimateMs: 1000 });
    expect(pendingFor(useCicada.getState(), "x", undefined)).toBeDefined();
  });

  it("every arrival REPLACES the pending state, never stacks it — the server holds one drag at a time", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    // The same param, announced again after a pause / an Esc: the new verdict.
    apply(policy(8, { node: "deboss", port: "value", estimate_ms: 4100.0, rough: true, pending_value: "1.3" }));
    expect(useCicada.getState().pending).toMatchObject({ value: "1.3", estimateMs: 4100, rough: true, seq: 8 });
    // Another param: the deboss drag has ended (silently, by the gap rule).
    apply(policy(9, { node: "amps", port: "value", estimate_ms: 2000.0, rough: false, pending_value: "2.5" }));
    const s = useCicada.getState();
    expect(s.pending?.node).toBe("amps");
    expect(pendingFor(s, "deboss", "value")).toBeUndefined();
  });

  it("the dragging widget's later ticks move the pending value; other params and absent entries are no-ops", () => {
    const apply = useCicada.getState().applyServerMessage;
    const untouched = useCicada.getState();
    useCicada.getState().trackPendingValue("deboss", "value", "0.9");
    expect(useCicada.getState(), "no pending entry: the state object is unchanged").toBe(untouched);
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    useCicada.getState().trackPendingValue("deboss", "value", "0.9");
    expect(useCicada.getState().pending).toMatchObject({ value: "0.9", estimateMs: 3990.9, seq: 5 });
    const before = useCicada.getState();
    useCicada.getState().trackPendingValue("deboss", "value", "0.9");
    expect(useCicada.getState(), "same value: no state change").toBe(before);
    useCicada.getState().trackPendingValue("amps", "value", "3.0");
    expect(useCicada.getState(), "another param: no state change").toBe(before);
    expect(useCicada.getState().pending?.value).toBe("0.9");
  });

  it("frames and statuses are NOT the end of the drag; the release's delta is", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    // A memo-warm tick painted live mid-drag: a status arrives. Still pending.
    apply(status(6));
    expect(useCicada.getState().pending?.node).toBe("deboss");
    // The release's set_param -> its delta: the pending value is now the committed one.
    apply(delta(7, "set deboss = 0.9"));
    expect(useCicada.getState().pending).toBeNull();
  });

  it("any write's delta clears it (undo, another client's edit): every write ends the drag server-side", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    apply({
      v: 1,
      seq: 6,
      type: "delta",
      payload: {
        source: { client: 9, intent_id: "p", label: "place add" },
        graph: graph("deboss", "add_1"),
        text: "",
        dirty: ["add_1"],
        history: after("place add", 2),
      },
    });
    expect(useCicada.getState().pending).toBeNull();
  });

  it("a refused write ends the drag like a landed one — except a lease refusal, decided before the door", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    apply({ v: 1, seq: 6, type: "error", payload: { intent_id: "9", kind: "lease", message: "read-only observer" } });
    expect(useCicada.getState().pending?.node, "lease: the drag stands").toBe("deboss");
    apply({
      v: 1,
      seq: 7,
      type: "error",
      payload: { intent_id: "10", kind: "refused", message: "no node named deboss" },
    });
    expect(useCicada.getState().pending).toBeNull();
    apply(policy(8, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    apply({ v: 1, seq: 9, type: "error", payload: { intent_id: "11", kind: "nothing_to_undo", message: "nothing" } });
    expect(useCicada.getState().pending, "undo mid-drag ends it, landed or refused").toBeNull();
  });

  it("a refused transport control leaves it standing: a write for the lease, never a drag-ender", () => {
    // `transport_seek` outside the loop / `transport_speed` out of bounds
    // mid-drag (a script can; the play bar cannot) are refused with kind
    // `transport`, and the server's drag stands (docs/13 §Animation
    // transport) — so must the badge (review 2026-08-21).
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    apply({
      v: 1,
      seq: 6,
      type: "error",
      payload: { intent_id: "12", kind: "transport", message: "frame 500 is outside the loop (frames 0..120)" },
    });
    expect(useCicada.getState().pending?.node, "transport: the drag stands").toBe("deboss");
    expect(useCicada.getState().lastError?.kind).toBe("transport");
    expect(dragStandsAfter("transport")).toBe(true);
    expect(dragStandsAfter("lease")).toBe(true);
    expect(dragStandsAfter("refused")).toBe(false);
    expect(dragStandsAfter("nothing_to_undo")).toBe(false);
  });

  it("the widget's release without a write clears it at once AND tells the server (`end_drag`) — when it can write", () => {
    const apply = useCicada.getState().applyServerMessage;
    const sent: ClientMessage[] = [];
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "";
    });
    useCicada.setState({ connection: "open", role: "writer" });
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    // Another param's release: this entry stands, but the server still hears
    // of THAT release (its drag — cheap, live — ends now, not by the gap).
    useCicada.getState().endDrag("amps", "value");
    expect(useCicada.getState().pending?.node, "another param: the entry stands").toBe("deboss");
    expect(sent).toEqual([{ type: "end_drag", payload: { node: "amps", port: "value" } }]);
    useCicada.getState().endDrag("deboss", "value");
    expect(useCicada.getState().pending, "cleared optimistically, ahead of the drag_ended").toBeNull();
    expect(sent.at(-1)).toEqual({ type: "end_drag", payload: { node: "deboss", port: "value" } });
    // A bare literal's port travels as null.
    useCicada.getState().endDrag("x", null);
    expect(sent.at(-1)).toEqual({ type: "end_drag", payload: { node: "x", port: null } });
    // The drag_ended the intent earns finds nothing to do.
    apply({ v: 1, seq: 5, type: "drag_ended", payload: { node: "deboss", port: "value" } });
    expect(useCicada.getState().pending).toBeNull();

    // Not the writer (the lease moved mid-drag) or no socket: the entry is
    // still cleared here — the release happened — but nothing is sent; the
    // server's drag is not this client's to end (the handover ended it).
    sent.length = 0;
    apply(policy(6, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    useCicada.setState({ role: "observer" });
    useCicada.getState().endDrag("deboss", "value");
    expect(useCicada.getState().pending).toBeNull();
    expect(sent, "no lease: nothing sent").toEqual([]);
    useCicada.setState({ role: "writer", connection: "reconnecting" });
    useCicada.getState().endDrag("deboss", "value");
    expect(sent, "no socket: nothing sent").toEqual([]);
    useCicada.setState({ connection: "open" });
  });

  it("`drag_ended` clears the entry it names — the observer's and the twin widget's only signal for a release that wrote nothing, an Esc, a refused write, a handover", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(5, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    const before = useCicada.getState();
    apply({ v: 1, seq: 5, type: "drag_ended", payload: { node: "amps", port: "value" } });
    expect(useCicada.getState(), "another param: no state change").toBe(before);
    apply({ v: 1, seq: 5, type: "drag_ended", payload: { node: "deboss" } });
    expect(useCicada.getState(), "deboss's bare literal is not deboss.value").toBe(before);
    apply({ v: 1, seq: 5, type: "drag_ended", payload: { node: "deboss", port: "value" } });
    expect(useCicada.getState().pending).toBeNull();
    // A bare literal's end has no port key; it names the null-port entry.
    apply(policy(6, { node: "x", estimate_ms: 1000, rough: true, pending_value: "2.0" }));
    apply({ v: 1, seq: 6, type: "drag_ended", payload: { node: "x" } });
    expect(useCicada.getState().pending).toBeNull();
    // After a landed write the delta has already cleared it: a no-op.
    apply(policy(7, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.9" }));
    apply(delta(8, "set deboss = 0.9"));
    const cleared = useCicada.getState();
    apply({ v: 1, seq: 8, type: "drag_ended", payload: { node: "deboss", port: "value" } });
    expect(useCicada.getState()).toBe(cleared);
    // A newer policy for another param has replaced the entry: the late end
    // of the old drag leaves the new one standing.
    apply(policy(9, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.9" }));
    apply(policy(9, { node: "amps", port: "value", estimate_ms: 2000, rough: false, pending_value: "2.5" }));
    apply({ v: 1, seq: 9, type: "drag_ended", payload: { node: "deboss", port: "value" } });
    expect(useCicada.getState().pending?.node).toBe("amps");
  });

  it("a reload barrier and a disconnect clear it too", () => {
    const apply = useCicada.getState().applyServerMessage;
    apply(policy(6, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    apply({
      v: 1,
      seq: 7,
      type: "snapshot",
      payload: {
        graph: graph("deboss"),
        text: "",
        statuses: {},
        summary,
        lease: { writer: 7, clients: [[7, "writer"]] },
        barrier: true,
        reason: "external change",
        history: EMPTY_HISTORY,
        transport: TRANSPORT_AT_REST,
        caches: EMPTY_CACHES,
      },
    });
    expect(useCicada.getState().pending, "the watcher's reload ends the drag").toBeNull();

    apply(policy(8, { node: "deboss", port: "value", estimate_ms: 3990.9, rough: false, pending_value: "0.875" }));
    useCicada.getState().markDisconnected("socket closed", { attempt: 1, nextAt: null });
    expect(useCicada.getState().pending, "the drag died with the socket").toBeNull();
  });
});

describe("resetSession (File → Open / Recent / Close, Back)", () => {
  it("sets the identity and clears every pipeline-bound slice; settings, notices and the catalog survive; the viewport's ledger is told", () => {
    const catalog = { format: 3 as const, subgroups: [], nodes: [] };
    useCicada.setState({
      connection: "open",
      connectionMessage: "",
      reconnect: { attempt: 2, nextAt: 5 },
      hello: { clientId: 3, role: "writer", protocol: 1, engine: "x", project: "p", pipeline: "a.cic", unitPx: 24, version: null, threads: null },
      role: "writer",
      lease: { writer: 3, clients: [[3, "writer"]] },
      token: "t",
      pipeline: "a.cic",
      seq: 12,
      text: "# cicada 1\ndeboss = slider()\n",
      statuses: { deboss: { state: "done", generation: 4 } },
      summary: {
        generation: 4,
        running: false,
        cancelled: false,
        computed: 1,
        cached: 0,
        pending: 0,
        red: 0,
        blocked: 0,
        elapsed_ms: 3,
        eta_rough: false,
      },
      dirty: ["deboss"],
      lastDeltaLabel: "set deboss",
      history: { ...EMPTY_HISTORY, can_undo: true, depth: 2 },
      lastError: { kind: "lease", message: "x" },
      snapshots: 3,
      displayGeneration: 4,
      displayResets: 2,
      catalog,
      catalogError: "catalog: HTTP 503",
      nodeValues: { deboss: { generation: 4, outputs: [], inputs: [] } },
      wireValues: { "a.out->b.x": { from: { node: "a", port: "out" }, to: { node: "b", port: "x" }, summary: null, pairing: "" } },
      probe: { from: { node: "a", port: "out" }, targets: {}, catalog: [], intentId: null },
      transport: { view: TRANSPORT_AT_REST, receivedAt: 1 },
      pending: { node: "deboss", port: "value", mode: "compute_on_release", value: "0.5", estimateMs: 1000, rough: false, seq: 12 },
      selection: { nodes: ["deboss"], wire: null, element: null },
      notices: [{ id: 1, level: "info", message: "kept", at: 0 }],
      search: { x: 1, y: 2, cell: null, from: null },
      commitDialog: true,
      fileDialog: true,
      aboutDialog: true,
    });
    useCicada.getState().setGitStatus({
      state: { kind: "not_a_repo" },
      pipeline: { path: "a.cic", tracked: false, ignored: false, dirty: false, nodes: [{ name: "deboss", change: "added" }], removed: [] },
      scope: [],
      text_hash: "00",
    });
    const settings = useCicada.getState().settings;

    useCicada.getState().resetSession("t", "sub/b.cic");
    const s = useCicada.getState();
    expect([s.token, s.pipeline]).toEqual(["t", "sub/b.cic"]);
    expect(s.connection).toBe("idle");
    expect(s.reconnect).toBeNull();
    expect(s.hello).toBeNull();
    expect(s.role).toBe("observer");
    expect(canWrite(s)).toBe(false);
    expect(s.lease).toEqual({ writer: null, clients: [] });
    expect([s.seq, s.text, s.dirty, s.lastDeltaLabel, s.snapshots, s.displayGeneration]).toEqual([0, "", [], "", 0, 0]);
    expect(s.graph.nodes).toEqual([]);
    expect(s.statuses).toEqual({});
    expect(s.summary.generation).toBe(0);
    expect(s.history).toEqual(EMPTY_HISTORY);
    expect(s.lastError).toBeNull();
    expect(s.displayResets, "bumped: the scene's ledger empties on the change").toBe(3);
    expect(s.nodeValues).toEqual({});
    expect(s.wireValues).toEqual({});
    expect(s.probe).toBeNull();
    expect(s.git.status, "the git cache is the old pipeline's").toBeNull();
    expect(s.gitMarkers).toEqual({});
    expect(s.transport).toBeNull();
    expect(s.pending).toBeNull();
    expect(s.selection).toEqual({ nodes: [], wire: null, element: null });
    expect(s.search).toBeNull();
    expect([s.commitDialog, s.fileDialog, s.aboutDialog]).toEqual([false, false, false]);
    expect(s.catalog, "the catalog stays until the join's snapshot re-reads it").toBe(catalog);
    expect(s.catalogError, "and its failure record with it — the same re-read clears or renews it").toBe("catalog: HTTP 503");
    expect(s.notices.map((n) => n.message)).toEqual(["kept"]);
    expect(s.settings).toBe(settings);
  });

  it("the picker's reset leaves no pipeline", () => {
    useCicada.getState().resetSession("t", "");
    expect(useCicada.getState().pipeline).toBe("");
    expect(useCicada.getState().hello).toBeNull();
  });
});

// One modal at a time (docs/16 §Keyboard map's Esc row; fix round 2
// 2026-09-20, finding L3A-3): by Tab the gear behind the commit dialog's
// backdrop opened About over it, and one Esc then closed both.
describe("the modals open one at a time", () => {
  const flags = () => {
    const s = useCicada.getState();
    return [s.commitDialog, s.fileDialog, s.aboutDialog];
  };

  it("an open action is a no-op while another modal is open; once that one closes, it opens", () => {
    useCicada.setState({ commitDialog: false, fileDialog: false, aboutDialog: false });
    const s = () => useCicada.getState();
    expect(modalOpen(s())).toBe(false);
    s().openCommitDialog();
    expect(modalOpen(s())).toBe(true);
    s().openAboutDialog();
    s().openFileDialog();
    expect(flags(), "About and Open refused over the commit dialog").toEqual([true, false, false]);
    s().closeCommitDialog();
    s().openAboutDialog();
    expect(flags()).toEqual([false, false, true]);
    s().openCommitDialog();
    s().openFileDialog();
    expect(flags(), "the commit dialog and Open refused over About").toEqual([false, false, true]);
    s().closeAboutDialog();
    s().openFileDialog();
    expect(flags()).toEqual([false, true, false]);
    s().openAboutDialog();
    s().openCommitDialog();
    expect(flags(), "About and the commit dialog refused over Open").toEqual([false, true, false]);
    s().closeFileDialog();
    expect(modalOpen(s())).toBe(false);
    s().openCommitDialog();
    expect(flags()).toEqual([true, false, false]);
    s().closeCommitDialog();
  });
});

describe("the canvas centre (U29)", () => {
  it("is recorded by the canvas, unchanged by an equal cell, cleared with null", () => {
    useCicada.setState({ canvasCenter: null });
    useCicada.getState().setCanvasCenter([3, -2]);
    expect(useCicada.getState().canvasCenter).toEqual([3, -2]);
    const held = useCicada.getState().canvasCenter;
    useCicada.getState().setCanvasCenter([3, -2]);
    expect(useCicada.getState().canvasCenter).toBe(held);
    useCicada.getState().setCanvasCenter([4, -2]);
    expect(useCicada.getState().canvasCenter).toEqual([4, -2]);
    useCicada.getState().setCanvasCenter(null);
    expect(useCicada.getState().canvasCenter).toBeNull();
  });
});

describe("the display edge (wave 5 D1: display_begin / display_end / caches / set_display_cache)", () => {
  const begin = (seq: number, generation: number, outputs: number): ServerEnvelope => ({
    v: 1,
    seq,
    type: "display_begin",
    payload: { generation, outputs },
  });
  const end = (
    seq: number,
    generation: number,
    extra: Partial<{ outputs: number; frames: number; bytes: number; cancelled: boolean; cut_by: "edit" | "esc" }> = {},
  ): ServerEnvelope => ({
    v: 1,
    seq,
    type: "display_end",
    payload: {
      generation,
      outputs: extra.outputs ?? 2,
      frames: extra.frames ?? 3,
      tessellate_ms: 2301.5,
      encode_ms: 114.25,
      bytes: extra.bytes ?? 160_100_000,
      ...(extra.cancelled === undefined ? {} : { cancelled: extra.cancelled }),
      ...(extra.cut_by === undefined ? {} : { cut_by: extra.cut_by }),
    },
  });
  const apply = (envelope: ServerEnvelope) => useCicada.getState().applyServerMessage(envelope);

  beforeEach(() => {
    useCicada.setState({
      display: null,
      caches: null,
      connection: "open",
      role: "writer",
      lastError: null,
      notices: [],
      settings: { ...useCicada.getState().settings, displayCacheMib: null },
    });
  });

  it("begin starts a painting pass; end finishes it with the server's numbers; a stale end and an older begin are ignored", () => {
    apply(begin(1, 12, 2));
    const painting = useCicada.getState().display;
    expect(painting).toMatchObject({ generation: 12, phase: "painting", outputs: 2, frames: 0, bytes: 0, cancelled: false, cutBy: null, paintedMs: null });
    expect(painting!.beganAt).toBeGreaterThan(0);
    // An older generation's end (its begin was overtaken) changes nothing.
    apply(end(2, 11));
    expect(useCicada.getState().display).toBe(painting);
    apply(end(3, 12, { outputs: 1, frames: 3 }));
    const done = useCicada.getState().display!;
    expect(done).toMatchObject({
      generation: 12,
      phase: "painted",
      outputs: 1,
      frames: 3,
      bytes: 160_100_000,
      tessellateMs: 2301.5,
      encodeMs: 114.25,
      cancelled: false,
      cutBy: null,
      paintedMs: null,
      beganAt: painting!.beganAt,
    });
    // A newer begin replaces it whatever its phase; an older begin does not.
    apply(begin(4, 13, 1));
    expect(useCicada.getState().display).toMatchObject({ generation: 13, phase: "painting", outputs: 1 });
    apply(begin(5, 12, 9));
    expect(useCicada.getState().display!.generation).toBe(13);
    // A cut pass says so, and by what.
    apply(end(6, 13, { outputs: 0, frames: 0, bytes: 0, cancelled: true, cut_by: "esc" }));
    expect(useCicada.getState().display).toMatchObject({ generation: 13, phase: "painted", cancelled: true, cutBy: "esc", outputs: 0 });
    apply(begin(7, 14, 1));
    apply(end(8, 14, { outputs: 0, frames: 0, bytes: 0, cancelled: true, cut_by: "edit" }));
    expect(useCicada.getState().display).toMatchObject({ generation: 14, cancelled: true, cutBy: "edit" });
    // A cancelled solve's empty pass: cancelled, nothing cut it.
    apply(begin(9, 15, 0));
    apply(end(10, 15, { outputs: 0, frames: 0, bytes: 0, cancelled: true }));
    expect(useCicada.getState().display).toMatchObject({ generation: 15, cancelled: true, cutBy: null });
  });

  it("a pass that sent no frame is painted at its end; markPainted stamps the client's wall once, for the standing pass only", () => {
    apply(begin(1, 20, 0));
    apply(end(2, 20, { outputs: 0, frames: 0, bytes: 0 }));
    const empty = useCicada.getState().display!;
    expect(empty.paintedMs).not.toBeNull();
    expect(empty.paintedMs!).toBeGreaterThanOrEqual(0);
    // A pass with frames waits for the viewport's render.
    apply(begin(3, 21, 2));
    useCicada.getState().markPainted(21, 99_999);
    expect(useCicada.getState().display!.paintedMs, "not before its end").toBeNull();
    apply(end(4, 21));
    const began = useCicada.getState().display!.beganAt;
    useCicada.getState().markPainted(20, began + 5);
    expect(useCicada.getState().display!.paintedMs, "another generation's render is not this pass's").toBeNull();
    useCicada.getState().markPainted(21, began + 2900);
    expect(useCicada.getState().display!.paintedMs).toBe(2900);
    useCicada.getState().markPainted(21, began + 9000);
    expect(useCicada.getState().display!.paintedMs, "stamped once").toBe(2900);
  });

  it("a disconnect and a session reset forget the pass and the caches", () => {
    apply(begin(1, 30, 2));
    apply({ v: 1, seq: 2, type: "caches", payload: EMPTY_CACHES });
    useCicada.getState().markDisconnected("socket closed", { attempt: 1, nextAt: null });
    expect(useCicada.getState().display).toBeNull();
    expect(useCicada.getState().caches).toBeNull();
    apply(begin(3, 31, 2));
    useCicada.getState().resetSession("t", "other.cic");
    expect(useCicada.getState().display).toBeNull();
  });

  it("the caches arrive whole — the broadcast's and the snapshot's — and replace", () => {
    const loud = { ...EMPTY_CACHES, display: { ...EMPTY_CACHES.display, bytes: 7, entries: 1, over_budget: true } };
    apply({ v: 1, seq: 1, type: "caches", payload: loud });
    expect(useCicada.getState().caches).toBe(loud);
    apply({
      v: 1,
      seq: 2,
      type: "snapshot",
      payload: {
        graph: { nodes: [], wires: [], diagnostics: [] },
        text: "",
        statuses: {},
        summary: useCicada.getState().summary,
        lease: { writer: 7, clients: [[7, "writer"]] },
        barrier: false,
        reason: "initial",
        history: EMPTY_HISTORY,
        transport: TRANSPORT_AT_REST,
        caches: EMPTY_CACHES,
      },
    });
    expect(useCicada.getState().caches).toBe(EMPTY_CACHES);
  });

  it("chooseDisplayCache remembers the choice and sends set_display_cache only while writing; the writer's hello re-applies it", () => {
    const sent: ClientMessage[] = [];
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "1";
    });
    useCicada.getState().chooseDisplayCache(2048);
    expect(useCicada.getState().settings.displayCacheMib).toBe(2048);
    expect(sent).toEqual([{ type: "set_display_cache", payload: { mib: 2048 } }]);
    // An observer's choice is kept, not sent.
    useCicada.setState({ role: "observer" });
    useCicada.getState().chooseDisplayCache(512);
    expect(useCicada.getState().settings.displayCacheMib).toBe(512);
    expect(sent).toHaveLength(1);
    // Forgetting the preference sends nothing (the server's default stands from the next session).
    useCicada.setState({ role: "writer" });
    useCicada.getState().chooseDisplayCache(null);
    expect(useCicada.getState().settings.displayCacheMib).toBeNull();
    expect(sent).toHaveLength(1);
    // The hello as writer re-applies a standing preference; as observer, never.
    useCicada.getState().updateSettings({ displayCacheMib: 256 });
    const hello = (role: "writer" | "observer"): ServerEnvelope => ({
      v: 1,
      seq: 0,
      type: "hello",
      payload: { client_id: 3, role, protocol: 1, engine: "x", project: "p", pipeline: "a.cic", unit_px: 24 },
    });
    useCicada.setState({ hello: null });
    apply(hello("observer"));
    expect(sent).toHaveLength(1);
    useCicada.setState({ hello: null });
    apply(hello("writer"));
    expect(sent.at(-1)).toEqual({ type: "set_display_cache", payload: { mib: 256 } });
    // Without a preference the writer's hello sends nothing.
    useCicada.getState().updateSettings({ displayCacheMib: null });
    useCicada.setState({ hello: null });
    apply(hello("writer"));
    expect(sent).toHaveLength(2);
  });

  it("a client that becomes the writer by a lease change applies its preference too (the lease holder's wins)", () => {
    const sent: ClientMessage[] = [];
    useCicada.getState().installSender((message) => {
      sent.push(message);
      return "1";
    });
    useCicada.getState().updateSettings({ displayCacheMib: 512 });
    const lease = (role: "writer" | "observer", writer: number | null): ServerEnvelope => ({
      v: 1,
      seq: 0,
      type: "lease",
      payload: { role, lease: { writer, clients: [[3, role]] } },
    });
    // An observer stays an observer: nothing sent.
    useCicada.setState({ role: "observer" });
    apply(lease("observer", 7));
    expect(sent).toHaveLength(0);
    // Becoming the writer (take_lease, or the writer left) applies it.
    apply(lease("writer", 3));
    expect(sent).toEqual([{ type: "set_display_cache", payload: { mib: 512 } }]);
    // A lease broadcast that leaves the client the writer sends nothing more.
    apply(lease("writer", 3));
    expect(sent).toHaveLength(1);
    // Without a preference, becoming the writer sends nothing.
    useCicada.getState().updateSettings({ displayCacheMib: null });
    useCicada.setState({ role: "observer" });
    apply(lease("writer", 3));
    expect(sent).toHaveLength(1);
  });
});

describe("the profile read cache (v0.1 wave 5 P1)", () => {
  const view = {
    generation: 12,
    kind: "structural",
    phases: { queued_ms: 1, solve_ms: 5, tessellate_ms: 40, encode_ms: 2, bytes: 4096 },
    nodes: [{ name: "ball", state: "done" as const, nanos: 3_000_000, elements: 1 }],
    display: [],
    caches: EMPTY_CACHES,
  };

  it("a profile_view replaces the held profile; a disconnect and a session reset forget it", () => {
    useCicada.setState({ profile: null, connection: "open", role: "writer" });
    useCicada.getState().applyServerMessage({ v: 1, seq: 9, type: "profile_view", payload: view });
    expect(useCicada.getState().profile).toEqual(view);
    useCicada.getState().applyServerMessage({ v: 1, seq: 10, type: "profile_view", payload: { ...view, generation: 13 } });
    expect(useCicada.getState().profile?.generation).toBe(13);
    useCicada.getState().markDisconnected("gone", { attempt: 1, nextAt: null });
    expect(useCicada.getState().profile, "a dead socket's profile is not shown as live").toBeNull();
    useCicada.getState().applyServerMessage({ v: 1, seq: 11, type: "profile_view", payload: view });
    useCicada.getState().resetSession("t", "other.cic");
    expect(useCicada.getState().profile, "the old pipeline's profile goes with it").toBeNull();
  });
});

describe("settingsFrom (the stored per-user settings → this build's)", () => {
  it("keeps the keys this build has, fills the missing ones with defaults, and DROPS a removed key — a stored `ribbonCollapsed` is ignored", () => {
    const settings = settingsFrom({ theme: "light", ribbonCollapsed: true, split: "even" });
    expect(settings.theme).toBe("light");
    expect(settings.split).toBe("even");
    expect(settings.swap).toBe(false);
    expect(settings.navigation).toBe("rhino");
    expect(Object.hasOwn(settings, "ribbonCollapsed")).toBe(false);
    // And what `updateSettings` would write back carries no removed key.
    expect(Object.keys(settings).sort()).toEqual(["displayCacheMib", "displayMode", "navigation", "split", "swap", "textPanel", "theme", "wireMode"]);
  });
  it("anything that is not a settings object is the defaults", () => {
    for (const raw of [null, undefined, 3, "x", [], true]) {
      expect(settingsFrom(raw), String(raw)).toEqual(settingsFrom({}));
    }
    expect(settingsFrom({}).theme).toBe("dark");
  });
});
