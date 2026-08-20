/**
 * Inspector (docs/16 §Inspector contents): tabs Inspect · Params · Git ·
 * Text. Inspect shows the selected node (ports + cached values, status,
 * diagnostics, contract, actions), the selected wire (type, depth, pairing,
 * values), or — nothing selected — the pipeline overview. Git is the git
 * panel (slice 1: status, per-node markers, commit, revert-to-HEAD).
 */
import { useEffect, useRef, useState } from "react";
import { baseOfType, kindColor } from "../kinds";
import type {
  Diagnostic,
  InputView,
  OutputView,
  ValueSummary,
  WireView,
} from "../protocol/messages";
import { LiteralWidget } from "../canvas/LiteralWidgets";
import { literalKindOf } from "../state/literals";
import { canWrite, nodeByName, useCicada } from "../state/store";
import { viewportApi } from "../viewport/api";
import { readFrameCounters } from "./debugHandle";
import { formatBytes, formatMs, statusText, summaryText } from "./format";
import { GitPanel } from "./GitPanel";
import { useInspectorTab, type InspectorTab } from "./inspectorTab";
import { ParamsPanel } from "./ParamsPanel";
import { TextPanel } from "./TextPanel";
import { ValueSummaryView } from "./ValueSummaryView";
import "./panels.css";

const TABS: [InspectorTab, string][] = [
  ["inspect", "Inspect"],
  ["params", "Params"],
  ["git", "Git"],
  ["text", "Text"],
];

export function Inspector() {
  const tab = useInspectorTab((s) => s.tab);
  const setTab = useInspectorTab((s) => s.setTab);
  const textPanel = useCicada((s) => s.settings.textPanel);
  // The Git tab wears the dirty-file count of this pipeline's commit scope
  // (dimmed while the cached status is stale — an edit landed, the re-read is on its way).
  const dirty = useCicada((s) => s.git.status?.scope.length ?? 0);
  const stale = useCicada((s) => s.git.stale);

  // The settings toggle "text panel" defaults the inspector to the Text tab.
  useEffect(() => {
    if (textPanel) setTab("text");
  }, [textPanel, setTab]);

  return (
    <aside className="panel inspector" data-testid="inspector">
      <div className="insp-tabs" role="tablist">
        {TABS.map(([id, label]) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={`insp-tab${tab === id ? " active" : ""}`}
            data-testid={`insp-tab-${id}`}
            onClick={() => setTab(id)}
          >
            {label}
            {id === "git" && dirty > 0 && (
              <span
                className={`insp-tab-count${stale ? " stale" : ""}`}
                title={`${dirty} dirty ${dirty === 1 ? "file" : "files"} to commit${stale ? " (last read — an edit landed since, re-reading)" : ""}`}
                data-testid="insp-tab-git-count"
              >
                {dirty}
              </span>
            )}
          </button>
        ))}
      </div>
      <div className="insp-body" data-testid={`insp-body-${tab}`}>
        {tab === "inspect" && <InspectTab />}
        {tab === "params" && <ParamsPanel />}
        {tab === "git" && <GitPanel />}
        {tab === "text" && <TextPanel />}
      </div>
    </aside>
  );
}

function InspectTab() {
  const selection = useCicada((s) => s.selection);
  if (selection.wire !== null) return <WireInspect id={selection.wire} />;
  const first = selection.nodes[0];
  if (first !== undefined) return <NodeInspect name={first} extra={selection.nodes.length - 1} />;
  return <Overview />;
}

// ---------------------------------------------------------------- node --

function NodeInspect({ name, extra }: { name: string; extra: number }) {
  const graph = useCicada((s) => s.graph);
  const status = useCicada((s) => s.statuses[name]);
  const values = useCicada((s) => s.nodeValues[name]);
  const generation = useCicada((s) => s.summary.generation);
  const running = useCicada((s) => s.summary.running);
  const element = useCicada((s) => s.selection.element);
  const send = useCicada((s) => s.send);
  const connection = useCicada((s) => s.connection);
  const snapshots = useCicada((s) => s.snapshots);
  const writer = useCicada(canWrite);
  const pipeline = useCicada((s) => s.pipeline);
  const token = useCicada((s) => s.token);
  const selectNodes = useCicada((s) => s.selectNodes);
  const setTab = useInspectorTab((s) => s.setTab);
  const [runBusy, setRunBusy] = useState(false);
  const inspected = useRef<string>("");

  const node = nodeByName(graph, name);

  // Ask for the node's cached values once per (node, generation, running
  // flag): a solve that finishes re-asks so the final values replace the
  // partial ones seen while running.
  useEffect(() => {
    if (node === undefined || connection !== "open") return;
    // A re-hydration snapshot (reconnect, barrier reload) clears the cached
    // values, so it re-asks too.
    const key = `${name}:${generation}:${running ? 1 : 0}:${snapshots}`;
    if (inspected.current === key) return;
    inspected.current = key;
    send({ type: "inspect", payload: { node: name } });
  }, [name, generation, running, node, connection, send, snapshots]);

  if (node === undefined) {
    return (
      <div className="insp-section faint">
        node <code>{name}</code> is no longer in the graph
      </div>
    );
  }

  const displayable = node.outputs.some((o) => o.displayable);
  const off = node.kind === "disabled";
  const stale = values !== undefined && values.generation < generation;

  const rename = () => {
    const next = window.prompt(`rename \`${name}\` to:`, name);
    if (next === null || next.trim() === "" || next === name) return;
    send({ type: "rename", payload: { node: name, new: next.trim() } });
  };
  const remove = () => send({ type: "delete_node", payload: { node: name } });
  const togglePreview = () => send({ type: "set_preview", payload: { node: name, on: !node.preview } });
  const toggleDisable = () => send({ type: "toggle_disable", payload: { node: name } });
  const run = async () => {
    setRunBusy(true);
    useCicada.getState().clearRunNotice();
    try {
      const response = await fetch(
        `/api/run/${encodeURIComponent(name)}?pipeline=${encodeURIComponent(pipeline)}`,
        {
          method: "POST",
          headers: {
            "X-Cicada-Token": token,
            // The write lease is required for effectful runs (they write
            // files); the server checks this id holds it.
            "X-Cicada-Client": String(useCicada.getState().hello?.clientId ?? ""),
          },
        },
      );
      const text = await response.text();
      if (!response.ok) {
        useCicada.getState().addNotice("error", `run ${name}: HTTP ${response.status} — ${text}`);
        return;
      }
      const json = JSON.parse(text) as { message?: string };
      // The server broadcasts `run_finished` before answering the POST and
      // the store already turned that into a notice — only speak when it
      // did not reach us (observer sockets, races).
      const state = useCicada.getState();
      if (state.runNotice?.node !== name) {
        state.addNotice("info", json.message ?? `\`${name}\` ran`);
      }
    } catch (error: unknown) {
      useCicada.getState().addNotice("error", `run ${name}: ${String(error)}`);
    } finally {
      setRunBusy(false);
    }
  };

  return (
    <div data-testid="node-inspect" data-node={name}>
      {element !== null && element.node === name && (
        <div className="pick-line" data-testid="pick-line">
          {name}.{node.outputs[element.output]?.name ?? `#${element.output}`}[{element.element}] · pick #
          {element.pickId}
        </div>
      )}
      <div className="insp-title">
        <span className="name">{name}</span>
        <span className="func">
          {node.func !== undefined ? <code>{node.func}</code> : null} {node.title}
        </span>
        {extra > 0 && <span className="faint">+{extra} more selected</span>}
      </div>
      <div className="insp-badges">
        <span className="badge">{node.category || "—"}</span>
        <span className="badge">{node.kind}</span>
        {node.outputs[0] !== undefined && (
          <span className="kind-badge" style={{ color: kindColor(node.outputs[0].base) }}>
            {node.outputs[0].resolved ?? node.outputs[0].type}
          </span>
        )}
        {node.effectful && (
          <span className="badge warn" title="effectful: runs only when you press Run">
            effectful
          </span>
        )}
        {node.preview && displayable && <span className="badge accent">preview</span>}
        {node.manual && <span className="badge" title="manually placed (sidecar)">manual</span>}
        <span className="faint">line {node.line + 1}</span>
      </div>
      {node.excluded !== undefined && (
        <div className={`excluded ${node.excluded.status}`} data-testid="excluded">
          {node.excluded.status}: {node.excluded.reason}
        </div>
      )}

      <section className="insp-section">
        <h3 className="insp-h">status</h3>
        <div className={`status-line state-${status?.state ?? "idle"}`} data-testid="node-status">
          <span className="state-dot" />
          <span>{statusText(status)}</span>
        </div>
        {status?.element_ids !== undefined && status.element_ids.length > 0 && (
          <div className="mono dim" title="element ids">
            elements: {status.element_ids.slice(0, 32).join(", ")}
            {status.element_ids.length > 32 ? " …" : ""}
          </div>
        )}
      </section>

      <section className="insp-section">
        <h3 className="insp-h">inputs</h3>
        {node.inputs.length === 0 && <div className="faint">no inputs</div>}
        {node.inputs.map((input) => (
          <InputRow
            key={input.name}
            node={name}
            input={input}
            // A `#off` ghost takes no in-place edits (enable it first).
            writer={writer && !off}
            onSelect={(n) => selectNodes([n])}
          />
        ))}
      </section>

      <section className="insp-section">
        <h3 className="insp-h">
          outputs
          {values !== undefined && (
            <span className="right faint">values gen {values.generation}</span>
          )}
        </h3>
        {node.outputs.length === 0 && <div className="faint">no outputs</div>}
        {node.outputs.map((output) => (
          <OutputRow
            key={output.name}
            output={output}
            value={values?.outputs.find(([n]) => n === output.name)}
            stale={stale}
          />
        ))}
      </section>

      {node.diagnostics.length > 0 && (
        <section className="insp-section">
          <h3 className="insp-h">diagnostics</h3>
          {node.diagnostics.map((d, i) => (
            <DiagnosticRow key={i} diagnostic={d} />
          ))}
        </section>
      )}

      <section className="insp-section">
        <h3 className="insp-h">contract</h3>
        {node.description && <div className="insp-desc">{node.description}</div>}
        {node.panics && (
          <div className="insp-desc" style={{ marginTop: 4 }}>
            <span style={{ color: "var(--error)" }}>Red when:</span> {node.panics}
          </div>
        )}
        {!node.description && !node.panics && <div className="faint">no contract text</div>}
        {node.comment && (
          <div className="insp-desc mono" style={{ marginTop: 4 }} title="attached comment">
            # {node.comment}
          </div>
        )}
      </section>

      <section className="insp-section">
        <h3 className="insp-h">actions</h3>
        <div className="actions" data-testid="node-actions">
          {node.effectful && (
            <button
              className="run"
              disabled={runBusy || !canWrite(useCicada.getState())}
              title="solve this node's cone and run it (exporters never auto-run)"
              onClick={() => void run()}
              data-testid="action-run"
            >
              {runBusy ? "running…" : "▶ Run"}
            </button>
          )}
          <button
            disabled={!writer || !displayable || off}
            title={off ? "disabled (#off) — enable it first" : displayable ? "toggle viewport preview (P)" : "no displayable output"}
            onClick={togglePreview}
            data-testid="action-preview"
          >
            {node.preview ? "hide preview" : "show preview"}
          </button>
          <button
            disabled={!writer || node.kind === "broken"}
            title={off ? "remove the #off prefix (D)" : "prefix the statement with #off (D)"}
            onClick={toggleDisable}
            data-testid="action-disable"
          >
            {off ? "enable" : "disable"}
          </button>
          <button disabled={!writer} onClick={rename} data-testid="action-rename">
            rename
          </button>
          <button className="danger" disabled={!writer} onClick={remove} data-testid="action-delete">
            delete
          </button>
          <button onClick={() => setTab("text")} data-testid="action-show-text">
            show in text
          </button>
        </div>
        {!writer && (
          <div className="faint" style={{ marginTop: 4 }}>
            {connection === "open"
              ? "read-only observer — take the lease to edit"
              : "not connected — edits are disabled until the session is back"}
          </div>
        )}
      </section>
    </div>
  );
}

function InputRow({
  node,
  input,
  writer,
  onSelect,
}: {
  node: string;
  input: InputView;
  writer: boolean;
  onSelect: (node: string) => void;
}) {
  const color = kindColor(input.base === "?" ? "" : baseOfType(input.base));
  // Unwired scalar literals are edited in place (the same widgets as the
  // canvas); observers see the literal text.
  const literalKind = writer && input.wired === undefined ? literalKindOf(input) : null;
  return (
    <div className="port-row" data-testid={`in-${input.name}`}>
      <span
        className={`port-dot${input.required ? " filled" : ""}`}
        style={{ color }}
        title={input.required ? "required" : "optional"}
      />
      <span>
        <span className="port-name">{input.name}</span>
        <span className="port-type" style={{ color }}>
          {input.type}
        </span>
        {input.lift > 0 && (
          <span className="lift-badge" title="lifted with each()">
            map ×{input.lift}
          </span>
        )}
        {input.unknown && (
          <span className="badge red" title="unknown port">
            ?
          </span>
        )}
      </span>
      <span className="port-src">
        {input.wired !== undefined ? (
          <>
            ←{" "}
            <button className="link" onClick={() => onSelect(input.wired!.node)} title="select the source node">
              {input.wired.node}.{input.wired.port}
            </button>
          </>
        ) : literalKind !== null && input.literal_value !== undefined ? (
          <LiteralWidget
            node={node}
            port={input.name}
            kind={literalKind}
            value={input.literal_value}
            writable={writer}
            compact
            testId={`insp-lit-${node}-${input.name}`}
            label={`${node}.${input.name}`}
          />
        ) : input.literal !== undefined ? (
          <code>{input.literal}</code>
        ) : input.default !== undefined ? (
          <span className="faint">
            default <code>{input.default}</code>
          </span>
        ) : (
          <span className="faint">unset</span>
        )}
        {input.dimension !== undefined && <span className="faint"> · {input.dimension}</span>}
      </span>
    </div>
  );
}

function OutputRow({
  output,
  value,
  stale,
}: {
  output: OutputView;
  value: [string, ValueSummary | null] | undefined;
  stale: boolean;
}) {
  const color = kindColor(output.base);
  return (
    <div className="port-row" data-testid={`out-${output.name}`}>
      <span className="port-dot filled" style={{ color }} />
      <span>
        <span className="port-name">{output.name}</span>
        <span className="port-type" style={{ color }}>
          {output.resolved ?? output.type}
        </span>
        {output.displayable && (
          <span className="faint" title="displayable in the viewport">
            {" "}
            👁
          </span>
        )}
      </span>
      <span className="port-src faint">{value === undefined ? "values pending…" : ""}</span>
      {value !== undefined && <ValueSummaryView summary={value[1]} stale={stale} />}
    </div>
  );
}

function DiagnosticRow({ diagnostic, onClick }: { diagnostic: Diagnostic; onClick?: () => void }) {
  const warning = /warn/i.test(diagnostic.kind);
  return (
    <div
      className={`diag${warning ? " warning" : ""}${onClick ? " clickable" : ""}`}
      onClick={onClick}
      data-testid="diagnostic"
    >
      <span className="kind">{diagnostic.kind}</span>
      {diagnostic.node && <code>{diagnostic.node}</code>}{" "}
      <span className="faint">L{diagnostic.span.line}</span> {diagnostic.message}
      {(diagnostic.expected || diagnostic.actual) && (
        <div className="dim">
          {diagnostic.expected && (
            <>
              expected <code>{diagnostic.expected}</code>{" "}
            </>
          )}
          {diagnostic.actual && (
            <>
              actual <code>{diagnostic.actual}</code>
            </>
          )}
        </div>
      )}
      {diagnostic.fix && <div className="fix">fix: {diagnostic.fix.label}</div>}
    </div>
  );
}

// ---------------------------------------------------------------- wire --

function WireInspect({ id }: { id: string }) {
  const graph = useCicada((s) => s.graph);
  const wire = graph.wires.find((w) => w.id === id);
  const send = useCicada((s) => s.send);
  const connection = useCicada((s) => s.connection);
  const snapshots = useCicada((s) => s.snapshots);
  const generation = useCicada((s) => s.summary.generation);
  const running = useCicada((s) => s.summary.running);
  const selectNodes = useCicada((s) => s.selectNodes);
  const key = wire === undefined ? "" : `${wire.from.node}.${wire.from.port}->${wire.to.node}.${wire.to.port}`;
  const values = useCicada((s) => s.wireValues[key]);
  const asked = useRef("");

  useEffect(() => {
    if (wire === undefined || connection !== "open") return;
    const k = `${id}:${generation}:${running ? 1 : 0}:${snapshots}`;
    if (asked.current === k) return;
    asked.current = k;
    send({ type: "inspect_wire", payload: { to: wire.to } });
  }, [id, wire, generation, running, connection, send, snapshots]);

  if (wire === undefined) {
    return <div className="insp-section faint">wire {id} is no longer in the graph</div>;
  }
  return <WireBody wire={wire} pairing={values?.pairing} summary={values?.summary} onSelect={(n) => selectNodes([n])} />;
}

function WireBody({
  wire,
  pairing,
  summary,
  onSelect,
}: {
  wire: WireView;
  pairing: string | undefined;
  summary: ValueSummary | null | undefined;
  onSelect: (node: string) => void;
}) {
  const color = kindColor(wire.type === undefined ? "" : baseOfType(wire.type));
  return (
    <div data-testid="wire-inspect" data-wire={wire.id}>
      <div className="insp-title">
        <span className="name" style={{ fontSize: 13 }}>
          wire
        </span>
        {wire.type !== undefined && (
          <span className="kind-badge" style={{ color }}>
            {wire.type}
          </span>
        )}
        {wire.red && <span className="badge red">red</span>}
      </div>
      <section className="insp-section">
        <div className="insp-row">
          <span className="k">from</span>
          <span className="v">
            <button className="link" onClick={() => onSelect(wire.from.node)}>
              {wire.from.node}.{wire.from.port}
            </button>
          </span>
        </div>
        <div className="insp-row">
          <span className="k">to</span>
          <span className="v">
            <button className="link" onClick={() => onSelect(wire.to.node)}>
              {wire.to.node}.{wire.to.port}
            </button>
          </span>
        </div>
        <div className="insp-row">
          <span className="k">depth</span>
          <span className="v mono">{wire.depth}</span>
        </div>
        <div className="insp-row">
          <span className="k">lift</span>
          <span className="v">
            {wire.lift > 0 ? <span className="lift-badge">map ×{wire.lift}</span> : <span className="faint">none</span>}
          </span>
        </div>
        <div className="insp-row">
          <span className="k">pairing</span>
          <span className="v mono">{pairing ?? <span className="faint">asking…</span>}</span>
        </div>
        {wire.reason !== undefined && (
          <div className="insp-row">
            <span className="k" style={{ color: "var(--error)" }}>
              red
            </span>
            <span className="v" style={{ color: "var(--error)" }}>
              {wire.reason}
            </span>
          </div>
        )}
      </section>
      <section className="insp-section">
        <h3 className="insp-h">value on the wire</h3>
        {summary === undefined ? (
          <div className="faint">values pending…</div>
        ) : (
          <ValueSummaryView summary={summary} />
        )}
      </section>
    </div>
  );
}

// ------------------------------------------------------------ overview --

interface ViewportTotals {
  outputs: number;
  triangles: number;
  vertices: number;
  drawCalls: number;
  framesReceived: number;
}

function readViewportTotals(): ViewportTotals {
  const stats = viewportApi.stats();
  let triangles = 0;
  let vertices = 0;
  for (const o of Object.values(stats.outputs)) {
    triangles += o.triangles;
    vertices += o.vertices;
  }
  return {
    outputs: Object.keys(stats.outputs).length,
    triangles,
    vertices,
    drawCalls: stats.drawCalls,
    framesReceived: stats.framesReceived,
  };
}

function Overview() {
  const graph = useCicada((s) => s.graph);
  const summary = useCicada((s) => s.summary);
  const statuses = useCicada((s) => s.statuses);
  const lease = useCicada((s) => s.lease);
  const hello = useCicada((s) => s.hello);
  const role = useCicada((s) => s.role);
  const selectNodes = useCicada((s) => s.selectNodes);
  const [frames, setFrames] = useState(readFrameCounters());
  const [totals, setTotals] = useState<ViewportTotals | null>(null);

  useEffect(() => {
    const tick = () => {
      setFrames(readFrameCounters());
      setTotals(readViewportTotals());
    };
    tick();
    const timer = window.setInterval(tick, 500);
    return () => window.clearInterval(timer);
  }, [summary.generation]);

  const counts: Record<string, number> = {};
  for (const s of Object.values(statuses)) counts[s.state] = (counts[s.state] ?? 0) + 1;

  return (
    <div data-testid="overview">
      <div className="insp-title">
        <span className="name" style={{ fontSize: 13 }}>
          pipeline overview
        </span>
        <span className="faint">nothing selected</span>
      </div>

      <section className="insp-section">
        <h3 className="insp-h">
          diagnostics
          <span className={`badge${graph.diagnostics.length > 0 ? " red" : ""}`}>{graph.diagnostics.length}</span>
        </h3>
        {graph.diagnostics.length === 0 && <div className="faint">none — the pipeline type-checks</div>}
        {graph.diagnostics.map((d, i) => (
          <DiagnosticRow
            key={i}
            diagnostic={d}
            onClick={d.node ? () => selectNodes([d.node!]) : undefined}
          />
        ))}
      </section>

      <section className="insp-section">
        <h3 className="insp-h">solve</h3>
        <div className="insp-desc" data-testid="overview-solve">
          {summaryText(summary)}
        </div>
        <div className="stat-grid" style={{ marginTop: 4 }}>
          <span className="k">nodes</span>
          <span className="v">
            {graph.nodes.length} · {graph.wires.length} wires
          </span>
          <span className="k">states</span>
          <span className="v">
            {Object.entries(counts)
              .map(([k, v]) => `${v} ${k}`)
              .join(" · ") || "—"}
          </span>
          <span className="k">elapsed</span>
          <span className="v">{formatMs(summary.elapsed_ms)}</span>
        </div>
      </section>

      <section className="insp-section">
        <h3 className="insp-h">display</h3>
        <div className="stat-grid">
          <span className="k">frames</span>
          <span className="v">
            {frames === null ? "—" : `${frames.received} · ${formatBytes(frames.bytes)}`}
          </span>
          <span className="k">outputs</span>
          <span className="v">{totals === null ? "—" : totals.outputs}</span>
          <span className="k">triangles</span>
          <span className="v">{totals === null ? "—" : `${totals.triangles} · ${totals.vertices} vertices`}</span>
          <span className="k">draw calls</span>
          <span className="v">{totals === null ? "—" : totals.drawCalls}</span>
        </div>
      </section>

      <section className="insp-section">
        <h3 className="insp-h">session</h3>
        <div className="stat-grid">
          <span className="k">role</span>
          <span className="v">{role}</span>
          <span className="k">clients</span>
          <span className="v">
            {lease.clients.length === 0
              ? "—"
              : lease.clients.map(([id, r]) => `#${id} ${r}${lease.writer === id ? " ✎" : ""}`).join(" · ")}
          </span>
          <span className="k">engine</span>
          <span className="v">{hello?.engine ?? "—"}</span>
          <span className="k">protocol</span>
          <span className="v">{hello?.protocol ?? "—"}</span>
          <span className="k">dialect</span>
          <span className="v">{graph.dialect ?? "—"}</span>
        </div>
      </section>
    </div>
  );
}
