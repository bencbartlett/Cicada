/**
 * The node face (docs/16 §Canvas conventions): header row (name bold ·
 * function dim · state badge · eye · effectful hint), one port row per unit
 * (inputs left, outputs right, handles colored by kind family; required
 * filled, optional hollow, refinement double-ringed; lift badges; inline
 * literals — a transport-driven port shows the transport in its row
 * instead: no handle, no literal), an optional param widget row, comment note above, excluded
 * outline, a git change badge when the binding differs from HEAD (docs/16
 * canvas badges; doc 10's status strip markers — added / modified /
 * renamed; removed nodes live only in the Git tab), and — at the closest
 * zoom tier — output value summaries below.
 *
 * Everything dynamic (status, probe verdicts, values, dirty flash, picks) is
 * read from the store per node so a status tick never rebuilds the graph.
 */
import { Handle, Position, useConnection, type NodeProps } from "@xyflow/react";
import { memo, useEffect, useState } from "react";
import { kindColor } from "../kinds";
import { markerBadge } from "../panels/gitFormat";
import type { DrivenSignal, InputView, NodeView, OutputView, ProbeVerdict, ValueSummary } from "../protocol/messages";
import { literalKindOf } from "../state/literals";
import { canWrite, useCicada } from "../state/store";
import { sendWrite, type CanvasNode } from "./flow";
import { firstLine, isRefinement, outputDoc, portTitle, statusBadge, transportDrivenSignal } from "./grid";
import { LiteralWidget } from "./LiteralWidgets";
import { useLodTier } from "./lod";
import { ParamWidget } from "./ParamWidget";

/** `node.port` of the output a wire is currently being dragged from (null = no drag). */
function useDragSource(): string | null {
  return useConnection((c) =>
    c.inProgress && c.fromHandle.type === "source" ? `${c.fromHandle.nodeId}.${c.fromHandle.id ?? ""}` : null,
  );
}

function handleClass(base: string, required: boolean, unknown: boolean | undefined): string {
  const parts = ["cn-handle"];
  parts.push(required ? "required" : "optional");
  if (isRefinement(base)) parts.push("refined");
  if (unknown) parts.push("unknown");
  return parts.join(" ");
}

function InputRow({
  node,
  input,
  verdict,
  probing,
  awaiting,
  writer,
}: {
  node: NodeView;
  input: InputView;
  verdict: ProbeVerdict | undefined;
  /** A wire is being dragged toward this node and its probe has answered. */
  probing: boolean;
  /** A wire is being dragged toward this node but its probe has not answered yet. */
  awaiting: boolean;
  writer: boolean;
}) {
  const color = kindColor(input.base);
  const cls = ["cn-port", "cn-in"];
  // Hover: `name: type — doc` (the catalog's one-line port doc rides on the view-model).
  let title = portTitle(input.name, input.type, input.doc);
  if (input.unknown) {
    cls.push("unknown");
    title = `${input.name}: unknown kwarg for this node`;
  }
  // Expression inputs are free variables (docs/10 §4): the wire is the name
  // itself, so it cannot be redrawn — edit the expression instead.
  const freeVar = node.kind === "expression";
  if (freeVar) title = `${input.name}: free variable of the expression — edit the text to change it`;
  // The gate fails closed (docs/09): no verdict = no wire, and the hover says why.
  if (awaiting && !freeVar) {
    cls.push("probe-none");
    title = `${input.name}: checking…`;
  } else if (probing && !freeVar) {
    if (verdict === undefined) {
      cls.push("probe-none");
      title = `${input.name}: no type verdict for this port — cannot connect`;
    } else {
      cls.push(`probe-${verdict.verdict}`);
      if (verdict.reason) title = `${input.name}: ${verdict.reason}`;
    }
  }
  const blocked = verdict?.verdict === "blocked";
  // Inline literal editor (docs/16): an unwired scalar kwarg is edited in
  // place when this client may write — never for the param node's own
  // widget port (the widget row has it), never for non-scalars; observers
  // (and a dropped connection) see the literal text.
  const literalKind =
    writer && input.wired === undefined && node.param?.port !== input.name ? literalKindOf(input) : null;
  // During a wire drag the browser shows no title tooltips (a button is
  // held), so the verdict reason is rendered as a label via CSS.
  const reason = probing && !freeVar && verdict?.verdict === "blocked" ? (verdict.reason ?? "blocked") : undefined;
  return (
    <div className={cls.join(" ")} title={title} data-reason={reason}>
      <Handle
        type="target"
        position={Position.Left}
        id={input.name}
        className={handleClass(input.base, input.required, input.unknown)}
        style={{ ["--port-color" as string]: color }}
        isConnectable={writer && !blocked && !freeVar}
        data-port={`${node.name}.${input.name}`}
        data-verdict={verdict?.verdict}
      />
      <span className="cn-port-label">{input.name}</span>
      {input.lift > 0 && (
        <span className="cn-lift" title={`each() — mapped${input.lift > 1 ? ` ×${input.lift}` : ""}`}>
          map{input.lift > 1 ? ` ×${input.lift}` : ""}
        </span>
      )}
      {probing && verdict?.verdict === "lift" && input.lift === 0 && (
        <span className="cn-lift cn-lift-offer" title={verdict.reason ?? "drop here to connect with each()"}>
          map
        </span>
      )}
      {literalKind !== null && input.literal_value !== undefined ? (
        <span className="cn-literal-edit" title={`${input.name} = ${input.literal ?? ""}`}>
          <LiteralWidget
            node={node.name}
            port={input.name}
            kind={literalKind}
            value={input.literal_value}
            writable={writer}
            compact
            testId={`lit-${node.name}-${input.name}`}
            label={`${node.name}.${input.name}`}
          />
        </span>
      ) : (
        input.literal !== undefined &&
        input.wired === undefined && (
          <span className="cn-literal mono" title={input.literal}>
            {input.literal}
          </span>
        )
      )}
    </div>
  );
}

/**
 * The row of a transport-driven input (docs/13 §Animation transport; the
 * catalog's `transport_driven`): the port is the session's, so it is HIDDEN
 * as a port — no handle (nothing to wire into, nothing to drop on), no
 * literal editor — and the row shows the transport driving it instead,
 * lit while this port is in the current graph's driven set. A kwarg a
 * human wrote by hand (`frame=5`) is the headless value; the tooltip says
 * so rather than offering to edit it.
 */
function DrivenRow({ node, input, signal }: { node: NodeView; input: InputView; signal: DrivenSignal }) {
  const driven = useCicada((s) =>
    s.transport?.view.driven.some((d) => d.node === node.name && d.port === input.name) ?? false,
  );
  const what = signal === "frame" ? "the loop frame" : "the playhead in seconds";
  const written = input.literal !== undefined ? ` The text's \`${input.name}=${input.literal}\` is the headless value (cicada run).` : "";
  const title = driven
    ? `${input.name}: ${input.type} — driven by the transport (${what}); not wired or edited here.${written}`
    : `${input.name}: ${input.type} — the transport's port (${what}); not driving while this node is not solvable.${written}`;
  return (
    <div
      className={`cn-port cn-in cn-driven${driven ? " on" : ""}`}
      title={title}
      data-testid={`driven-${node.name}-${input.name}`}
      data-signal={signal}
      data-driven={driven}
    >
      <span className="cn-port-label">{input.name}</span>
      <span className="cn-transport-chip mono" aria-label={`${input.name} is driven by the transport`}>
        {driven ? "▶" : "▷"} transport
      </span>
    </div>
  );
}

function summaryText(summary: ValueSummary | null): string {
  if (summary === null) return "—";
  const parts: string[] = [];
  if (summary.count !== undefined) parts.push(`${summary.kind} ×${summary.count}`);
  const sample = summary.samples?.[0];
  if (sample !== undefined) parts.push(sample);
  else if (summary.count === undefined) parts.push(summary.kind);
  return parts.join(" · ");
}

function OutputRow({
  output,
  doc,
  value,
}: {
  output: OutputView;
  /** The catalog's one-line doc of this port (a bare `out`'s `# Returns` line), if any. */
  doc: string | undefined;
  /** Closest-zoom preview: `undefined` = not shown, `null` = no value yet. */
  value: ValueSummary | null | undefined;
}) {
  const color = kindColor(output.base);
  const type = output.resolved ?? output.type;
  const shown = value !== undefined;
  // Hover: `name: type — doc`, the displayable tag, then the value line at the closest zoom.
  const tag = output.displayable ? " (displayable)" : "";
  const valueLine = shown ? `\n${summaryText(value)}` : "";
  const title = portTitle(output.name, type, doc) + tag + valueLine;
  return (
    <div className={`cn-port cn-out${shown ? " with-value" : ""}`} title={title}>
      <span className="cn-port-label">{output.name}</span>
      {shown && <span className="cn-port-value mono">{summaryText(value)}</span>}
      <Handle
        type="source"
        position={Position.Right}
        id={output.name}
        className={handleClass(output.base, true, false)}
        style={{ ["--port-color" as string]: color }}
      />
    </div>
  );
}

/**
 * The git change badge: one glyph on the header when this binding's line
 * differs from HEAD (`+` added · `~` modified · `→` renamed), colored by
 * the theme's semantic tokens, the reason in its tooltip. Read per node
 * from the store's marker index so a status answer never rebuilds the graph.
 */
function GitBadge({ name }: { name: string }) {
  const marker = useCicada((s) => s.gitMarkers[name]);
  if (marker === undefined) return null;
  const badge = markerBadge(marker);
  return (
    <span className={`cn-git cn-git-${badge.kind}`} title={badge.title} data-testid={`git-${name}`} data-change={badge.kind}>
      {badge.glyph}
    </span>
  );
}

/**
 * The port-less ghost: a broken line, or a `#off` line whose body does not
 * parse. A `#off` line that parses renders as the node it is (ports,
 * literals, wires intact) with the `disabled` styling — see `CicadaNodeImpl`.
 */
function GhostNode({ view, unit }: { view: NodeView; unit: number }) {
  const gitChange = useCicada((s) => s.gitMarkers[view.name]?.change);
  const label = view.kind === "disabled" ? "disabled (#off)" : "broken line";
  const title = view.diagnostics.map((d) => d.message).join("\n") || label;
  return (
    <div
      className={`cn cn-ghost cn-kind-${view.kind}`}
      style={{ width: view.size[0] * unit, height: view.size[1] * unit }}
      title={title}
      data-node={view.name}
      data-git={gitChange}
    >
      <div className="cn-header">
        <span className="cn-name">{view.name}</span>
        <span className="cn-func">{label}</span>
        <span className="cn-badges">
          <GitBadge name={view.name} />
        </span>
      </div>
      <div className="cn-raw mono">{view.text}</div>
    </div>
  );
}

function CicadaNodeImpl({ data, selected }: NodeProps<CanvasNode>) {
  const view = data.view;
  const name = view.name;
  const unit = useCicada((s) => s.hello?.unitPx ?? 24);
  const status = useCicada((s) => s.statuses[name]);
  const probe = useCicada((s) => s.probe);
  const values = useCicada((s) => s.nodeValues[name]);
  const dirty = useCicada((s) => s.dirty);
  const picked = useCicada((s) => s.selection.element?.node === name);
  const hovered = useCicada((s) => s.hoverPick?.node === name);
  const writer = useCicada(canWrite);
  const gitChange = useCicada((s) => s.gitMarkers[name]?.change);
  // Output-port docs come from the catalog (the view-model's `OutputView`
  // carries none); the catalog object is replaced whole, never mutated, so
  // this subscription only fires once per load.
  const catalog = useCicada((s) => s.catalog);
  const dragSource = useDragSource();
  const tier = useLodTier();

  // Dirty flash: ~600 ms accent outline when a delta names this node.
  const [flash, setFlash] = useState(false);
  useEffect(() => {
    if (!dirty.includes(name)) return;
    setFlash(true);
    const timer = setTimeout(() => setFlash(false), 600);
    return () => clearTimeout(timer);
  }, [dirty, name]);

  const disabled = view.kind === "disabled";
  if (view.kind === "broken" || (disabled && view.inputs.length === 0 && view.outputs.length === 0)) {
    return <GhostNode view={view} unit={unit} />;
  }
  // A `#off` ghost keeps its ports and wiring but takes no in-place edits
  // (the writer refuses them by name — enable it first): no literal
  // editors, no slider, no preview eye; its badge says `off`.
  const editable = writer && !disabled;

  const badge = disabled
    ? { label: "off", className: "state-off", title: "disabled (#off) — D or the menu enables it" }
    : statusBadge(status, view.diagnostics.length);
  // Closest zoom: every output shows what is sitting on it (docs/16).
  const outputValues =
    tier === "closest" && values !== undefined ? new Map(values.outputs) : null;
  const displayable = !disabled && view.outputs.some((o) => o.displayable);
  // A drag is heading somewhere: `probing` once the verdicts for ITS source
  // are in, `awaiting` while they are not (the gate fails closed meanwhile).
  const probeMatchesDrag =
    probe !== null && (dragSource === null || `${probe.from.node}.${probe.from.port}` === dragSource);
  const probing = probe !== null && probeMatchesDrag && probe.from.node !== name;
  const awaiting = dragSource !== null && !probeMatchesDrag && !dragSource.startsWith(`${name}.`);
  const rows = Math.max(view.inputs.length, view.outputs.length, 1);
  const width = view.size[0] * unit;
  const height = view.size[1] * unit;

  const classes = ["cn", `cn-kind-${view.kind}`, badge.className];
  if (selected) classes.push("selected");
  if (picked) classes.push("picked");
  if (hovered) classes.push("hover-pick");
  if (flash) classes.push("flash");
  if (disabled) classes.push("cn-disabled");
  else if (view.excluded) classes.push(view.excluded.status === "red" ? "excluded-red" : "excluded-amber");
  if (view.effectful) classes.push("effectful");
  if (view.preview) classes.push("preview-on");
  if (probe !== null && probe.from.node === name) classes.push("probe-source");

  const subtitle = disabled
    ? "disabled (#off)"
    : view.kind === "literal"
      ? "Constant"
      : view.kind === "expression"
        ? "Expression"
        : (view.func ?? view.title);
  const headerTitle = [
    `${name} — ${view.title}`,
    view.kind === "expression" && view.description ? view.description : null,
    view.excluded ? `${view.excluded.status}: ${view.excluded.reason}` : null,
    view.diagnostics.length > 0 ? view.diagnostics.map((d) => d.message).join("\n") : null,
    view.effectful ? "effectful — runs only explicitly (never auto-runs)" : null,
  ]
    .filter((s): s is string => s !== null)
    .join("\n");

  const togglePreview = (event: React.MouseEvent) => {
    event.stopPropagation();
    sendWrite({ type: "set_preview", payload: { node: name, on: !view.preview } });
  };

  return (
    <div
      className={classes.join(" ")}
      style={{ width, height, ["--unit" as string]: `${unit}px` }}
      title={view.excluded ? `${view.excluded.status}: ${view.excluded.reason}` : undefined}
      data-node={name}
      data-state={disabled ? "off" : (status?.state ?? "idle")}
      data-git={gitChange}
    >
      {view.comment && (
        <div className="cn-note" title={view.comment}>
          {firstLine(view.comment)}
        </div>
      )}
      <div className="cn-header" title={headerTitle}>
        <span className="cn-glyph" aria-hidden="true" style={{ ["--glyph-color" as string]: kindColor(view.outputs[0]?.base ?? "") }}>
          {glyphOf(view)}
        </span>
        <span className="cn-name">{name}</span>
        <span className="cn-func">{subtitle}</span>
        <span className="cn-badges">
          <GitBadge name={name} />
          {view.effectful && (
            <span className="cn-fx" title="effectful — runs only explicitly (use the inspector's run)">
              run
            </span>
          )}
          {displayable && (
            <button
              type="button"
              className={`cn-eye nodrag${view.preview ? " on" : ""}`}
              title={view.preview ? "preview on — click to hide" : "preview off — click to show"}
              onClick={togglePreview}
              onDoubleClick={(event) => event.stopPropagation()}
              aria-pressed={view.preview}
              data-testid={`eye-${name}`}
            >
              {view.preview ? "◉" : "◌"}
            </button>
          )}
          <span className={`cn-state ${badge.className}`} title={badge.title} data-testid={`state-${name}`}>
            {badge.label}
          </span>
        </span>
      </div>
      <div className="cn-rows">
        {Array.from({ length: rows }, (_, i) => {
          const input = view.inputs[i];
          const output = view.outputs[i];
          const signal = input ? transportDrivenSignal(catalog, view.func, input.name) : undefined;
          return (
            <div className="cn-row" key={i}>
              {input && signal !== undefined ? (
                <DrivenRow node={view} input={input} signal={signal} />
              ) : input ? (
                <InputRow
                  node={view}
                  input={input}
                  verdict={probing ? probe?.targets[`${name}.${input.name}`] : undefined}
                  probing={probing}
                  awaiting={awaiting}
                  writer={editable}
                />
              ) : (
                <span className="cn-port cn-in cn-empty" />
              )}
              {output ? (
                <OutputRow
                  output={output}
                  doc={outputDoc(catalog, view.func, output.name)}
                  value={outputValues === null ? undefined : (outputValues.get(output.name) ?? null)}
                />
              ) : (
                <span className="cn-port cn-out cn-empty" />
              )}
            </div>
          );
        })}
      </div>
      {view.param && <ParamWidget view={view} param={view.param} writer={editable} />}
      {status?.state === "running" && status.elements !== undefined && status.elements > 0 && (
        <div className="cn-progress">
          <div
            className="cn-progress-bar"
            style={{ width: `${Math.min(100, (100 * (status.elements_done ?? 0)) / status.elements)}%` }}
          />
        </div>
      )}
    </div>
  );
}

export const CicadaNode = memo(CicadaNodeImpl);

/** The spike's fallback glyph (docs/16 §Icons): two letters on the node's
 * output kind hue — the generated icon set lands with the v0.1 catalog. */
function glyphOf(view: { func?: string; kind: string; name: string }): string {
  const source = view.func ?? (view.kind === "expression" ? "fx" : view.kind === "literal" ? "#" : view.name);
  const parts = source.split("_").filter((p) => p.length > 0);
  if (parts.length >= 2) return (parts[0]![0]! + parts[1]![0]!).toUpperCase();
  return source.slice(0, 2).toUpperCase();
}
