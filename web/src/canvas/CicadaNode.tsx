/**
 * The node face (docs/16 §Canvas conventions): header row (name bold ·
 * function dim · state badge · eye · effectful hint), one port row per unit
 * (inputs left, outputs right, handles colored by kind family; required
 * filled, optional hollow, refinement double-ringed; lift badges; the
 * typed-literal chip on every unwired literal-typed port — a
 * transport-driven port shows the transport in its row instead: no handle,
 * no chip), an optional param widget row, comment note above, excluded
 * outline, a git change badge when the binding differs from HEAD (docs/16
 * canvas badges; doc 10's status strip markers — added / modified /
 * renamed; removed nodes live only in the Git tab), and — from the near
 * zoom tier up (`showsPortValues`) — output value summaries below. A
 * slider the sidecar collapses (`NodeView.collapsed`, wave 4 B4) is ONE
 * row instead — name · track · value · its output handle, GH-like
 * (`CollapsedSlider`): no header, no port rows, since the server collapses
 * only a slider whose bounds are literals.
 *
 * Everything dynamic (status, probe verdicts, values, dirty flash, picks) is
 * read from the store per node so a status tick never rebuilds the graph.
 */
import { Handle, Position, useConnection, type NodeProps } from "@xyflow/react";
import { memo, useEffect, useState } from "react";
import { kindColor } from "../kinds";
import { markerBadge } from "../panels/gitFormat";
import type {
  DrivenSignal,
  InputView,
  NodeView,
  OutputView,
  ParamView,
  ProbeVerdict,
  ValueSummary,
} from "../protocol/messages";
import { literalPortKind } from "../state/literals";
import { canWrite, useCicada } from "../state/store";
import { sendWrite, type CanvasNode } from "./flow";
import {
  drivenTitle,
  firstLine,
  isRefinement,
  outputDoc,
  portTitle,
  showsPortValues,
  statusBadge,
  transportDrivenSignal,
} from "./grid";
import { LiteralChip } from "./LiteralChip";
import { chipFace } from "./literalFace";
import { useLodTier } from "./lod";
import { ParamWidget } from "./ParamWidget";
import { compactValueText } from "./valueText";

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
  // The typed-literal chip (docs/16 §Canvas conventions, wave 4 B3): an
  // unwired port whose type takes a literal wears its value — the kwarg's
  // literal, else the catalog default greyed, else an empty required slot
  // — and double-click opens an editor when this client may write; never
  // for the param node's own widget port (the widget row has it), never
  // for an expression's free variables (the wire IS the name). Observers
  // (and a dropped connection, and a `#off` ghost) see the text, no chip.
  const chipKind =
    input.wired === undefined && node.param?.port !== input.name && !freeVar ? literalPortKind(input) : null;
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
      {chipKind !== null && writer ? (
        <LiteralChip
          node={node.name}
          port={input.name}
          kind={chipKind}
          literal={input.literal}
          value={input.literal_value}
          defaultText={input.default}
          defaultValue={input.default_value}
          surface="canvas"
          testId={`lit-${node.name}-${input.name}`}
          label={`${node.name}.${input.name}`}
        />
      ) : input.literal !== undefined && input.wired === undefined ? (
        <span className="cn-literal mono" title={input.literal}>
          {input.literal}
        </span>
      ) : (
        chipKind !== null &&
        input.default !== undefined && (
          <span className="cn-literal mono faint" title={`${input.name}: default (not in the text)`}>
            {chipFace({ kind: chipKind, defaultText: input.default, defaultValue: input.default_value }).text}
          </span>
        )
      )}
    </div>
  );
}

/**
 * The row of a transport-driven input (docs/13 §Animation transport; the
 * catalog's `transport_driven`): the port is the session's, so it is HIDDEN
 * as a port — no connectable handle (nothing to drop on: the server's
 * probe answers `blocked` and `connect` refuses), no literal editor — and
 * the row shows the transport driving it instead, lit while this port is
 * in the current graph's driven set. What a human wrote by hand in the
 * text is the headless value and is never hidden: a kwarg (`frame=5`) is
 * named in the tooltip; a WIRE (`frame=n`) keeps a target handle — not
 * connectable, but React Flow draws an edge only between two handles, and
 * a wire the text carries and `cicada run` evaluates must be visible and
 * removable (drag it off, or the wire menu's disconnect), never silently
 * dropped.
 */
function DrivenRow({ node, input, signal }: { node: NodeView; input: InputView; signal: DrivenSignal }) {
  const driven = useCicada((s) =>
    s.transport?.view.driven.some((d) => d.node === node.name && d.port === input.name) ?? false,
  );
  const title = drivenTitle(input.name, input.type, signal, driven, input.literal, input.wired);
  return (
    <div
      className={`cn-port cn-in cn-driven${driven ? " on" : ""}${input.wired !== undefined ? " wired" : ""}`}
      title={title}
      data-testid={`driven-${node.name}-${input.name}`}
      data-signal={signal}
      data-driven={driven}
      data-wired={input.wired === undefined ? undefined : `${input.wired.node}.${input.wired.port}`}
    >
      {input.wired !== undefined && (
        <Handle
          type="target"
          position={Position.Left}
          id={input.name}
          className={`${handleClass(input.base, input.required, input.unknown)} driven`}
          style={{ ["--port-color" as string]: kindColor(input.base) }}
          isConnectable={false}
          isConnectableStart={false}
          isConnectableEnd={false}
          data-port={`${node.name}.${input.name}`}
          data-verdict="blocked"
        />
      )}
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
  /** The value preview (near tier and up): `undefined` = not shown, `null` = no value yet. */
  value: ValueSummary | null | undefined;
}) {
  const color = kindColor(output.base);
  const type = output.resolved ?? output.type;
  const shown = value !== undefined;
  // Hover: `name: type — doc`, the displayable tag, then the value line when the tier shows values.
  const tag = output.displayable ? " (displayable)" : "";
  const valueLine = shown ? `\n${summaryText(value)}` : "";
  const title = portTitle(output.name, type, doc) + tag + valueLine;
  // The face rounds every decimal to four significant figures (U23); the
  // hover above and the inspector keep the full value.
  return (
    <div className={`cn-port cn-out${shown ? " with-value" : ""}`} title={title}>
      <span className="cn-port-label">{output.name}</span>
      {shown && <span className="cn-port-value mono">{compactValueText(summaryText(value))}</span>}
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

/**
 * The collapsed slider (docs/16 §Canvas conventions, wave 4 B4 — finding
 * U11): one grid unit tall, GH-like — the name, the same slider widget as
 * the expanded face (drag protocol, pending chip and all), and the output
 * handle at the right edge. No header and no port rows: the server
 * collapses only a slider whose `min` / `max` / `step` are literals, so
 * nothing is wired INTO it and no input handle is owed. The state badge is
 * shown only for a problem (red, blocked, off) — the value says the rest —
 * and `data-state` carries it always.
 */
function CollapsedSlider({
  view,
  param,
  classes,
  badge,
  writer,
  unit,
  gitChange,
  status,
  title,
}: {
  view: NodeView;
  param: ParamView;
  classes: string[];
  badge: { label: string; className: string; title: string };
  writer: boolean;
  unit: number;
  gitChange: string | undefined;
  status: string;
  title: string;
}) {
  const out = view.outputs[0];
  const problem = badge.className === "state-red" || badge.className === "state-blocked" || badge.className === "state-off";
  return (
    <div
      className={[...classes, "cn-collapsed"].join(" ")}
      style={{ width: view.size[0] * unit, height: view.size[1] * unit, ["--unit" as string]: `${unit}px` }}
      title={title}
      data-node={view.name}
      data-state={status}
      data-git={gitChange}
      data-collapsed="true"
    >
      <div className="cn-collapsed-row">
        <span className="cn-collapsed-name" data-testid={`collapsed-${view.name}`}>
          {view.name}
        </span>
        <ParamWidget view={view} param={param} writer={writer} />
        <span className="cn-badges">
          <GitBadge name={view.name} />
          {problem && (
            <span className={`cn-state ${badge.className}`} title={badge.title} data-testid={`state-${view.name}`}>
              {badge.label}
            </span>
          )}
        </span>
        {out !== undefined && (
          <Handle
            type="source"
            position={Position.Right}
            id={out.name}
            className={handleClass(out.base, true, false)}
            style={{ ["--port-color" as string]: kindColor(out.base) }}
          />
        )}
      </div>
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
  // carries none); the catalog object is replaced whole, never mutated,
  // and only when a read's answer differs from the one held (every
  // snapshot re-reads it — `state/catalog.ts` — but an identical answer
  // is not re-applied), so this subscription fires when a script node
  // appeared or changed, not per reload.
  const catalog = useCicada((s) => s.catalog);
  // Until the catalog arrives, the snapshot's driven set stands in for its
  // `transport_driven` flag (`transportDrivenSignal`). Read only while the
  // catalog is missing: afterwards the selector is a constant, so a
  // `transport` broadcast (every seek of a scrub) re-renders no node.
  const preCatalogDriven = useCicada((s) => (s.catalog === null ? s.transport?.view.driven : undefined));
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
  // Near zoom and up: every output shows what is sitting on it (docs/16 LOD table).
  const outputValues =
    showsPortValues(tier) && values !== undefined ? new Map(values.outputs) : null;
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

  // The collapsed slider: the server says so only for a slider it can
  // collapse (`NodeView.collapsed`; its `size` is already one unit).
  if (view.collapsed === true && view.param?.kind === "slider") {
    return (
      <CollapsedSlider
        view={view}
        param={view.param}
        classes={classes}
        badge={badge}
        writer={editable}
        unit={unit}
        gitChange={gitChange}
        status={disabled ? "off" : (status?.state ?? "idle")}
        title={headerTitle}
      />
    );
  }

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
              <EyeIcon off={!view.preview} />
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
          const signal = input
            ? transportDrivenSignal(
                catalog,
                view.func,
                input.name,
                preCatalogDriven?.find((d) => d.node === name && d.port === input.name),
              )
            : undefined;
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

/**
 * The preview toggle's icon (finding U20, 2026-08-25): an eye while the
 * output is drawn, the same eye with a slash across it while it is hidden —
 * drawn here as two strokes and a circle, theme-coloured through
 * `currentColor`. `data-icon` names the state for tests and tooling.
 */
function EyeIcon({ off }: { off: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="14"
      height="14"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      data-icon={off ? "eye-off" : "eye"}
    >
      <path d="M2 12c2.6-4 6-6 10-6s7.4 2 10 6c-2.6 4-6 6-10 6S4.6 16 2 12z" />
      <circle cx="12" cy="12" r="3" />
      {off && <path d="M4 4l16 16" />}
    </svg>
  );
}

/** The spike's fallback glyph (docs/16 §Icons): two letters on the node's
 * output kind hue — the generated icon set lands with the v0.1 catalog. */
function glyphOf(view: { func?: string; kind: string; name: string }): string {
  const source = view.func ?? (view.kind === "expression" ? "fx" : view.kind === "literal" ? "#" : view.name);
  const parts = source.split("_").filter((p) => p.length > 0);
  if (parts.length >= 2) return (parts[0]![0]! + parts[1]![0]!).toUpperCase();
  return source.slice(0, 2).toUpperCase();
}
