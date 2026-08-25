/**
 * Wires (docs/16 + docs/09): kind-colored, depth-styled in GH's convention
 * (depth 0 a single line · depth 1 a double line · depth ≥ 2 a thick dashed
 * line — finding U26), lit by the selection glow at a lower opacity on
 * hover and whenever a node they attach to is selected (findings U21, U22;
 * the glow path is always drawn, the stylesheet sets its opacity), red
 * with the reason on hover,
 * spline (bezier) or trace (the PCB-style router of `trace.ts`: orthogonal
 * runs, 45° corner cuts of one unit, laned so parallel runs never coincide
 * — the lanes assigned once per canvas render and read here from the
 * `TraceLanesContext`) per the user's wire-mode setting, and a `map` chip
 * on the wire's middle run when the kwarg is lifted. Stroke width and colour
 * are the same in both modes.
 */
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from "@xyflow/react";
import { memo } from "react";
import { baseOfType, kindColor } from "../kinds";
import { useCicada } from "../state/store";
import type { CanvasEdge } from "./flow";
import { wireStrokeWidth, wireStyle } from "./grid";
import { tracePath } from "./trace";
import { useTraceRoute } from "./traceLanes";

function CicadaEdgeImpl({
  id,
  source,
  target,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  selected,
}: EdgeProps<CanvasEdge>) {
  const wireMode = useCicada((s) => s.settings.wireMode);
  const unit = useCicada((s) => s.hello?.unitPx ?? 24);
  // A selected node lights every wire attached to it (U22): one boolean
  // per edge, so a selection change re-renders only the edges it flips.
  const attached = useCicada((s) => s.selection.nodes.includes(source) || s.selection.nodes.includes(target));
  const wire = data?.wire;
  const depth = wire?.depth ?? 0;
  const red = wire?.red ?? false;
  const color = red ? "var(--error)" : kindColor(baseOfType(wire?.type ?? "?"));
  const width = wireStrokeWidth(depth);
  const style = wireStyle(depth);

  // The lane this wire was assigned with every other wire in view (the
  // canvas's per-render memo); the endpoints are React Flow's measured
  // handles, so the trace meets them exactly.
  const route = useTraceRoute(id);
  const trace = wireMode === "trace" ? tracePath({ sx: sourceX, sy: sourceY, tx: targetX, ty: targetY }, unit, route) : null;
  const [path, labelX, labelY] = trace ?? getBezierPath({ sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition });
  // The router's one drawing fallback — the assigned route's kind disagreed
  // with the measured endpoints and the natural route was drawn — is marked
  // on the edge, never silent (the traces spec asserts it never happens).
  const yielded = trace !== null && trace[3];

  const shape = style === "single" ? "" : style === "double" ? " (list)" : ` (tree, depth ${depth})`;
  const title = red
    ? `${wire?.id ?? id}: ${wire?.reason ?? "red"}`
    : `${wire?.id ?? id}: ${wire?.type ?? "?"}${shape}${
        wire && wire.lift > 0 ? ` · each()${wire.lift > 1 ? ` ×${wire.lift}` : ""}` : ""
      }`;

  const classes = ["cicada-edge", `depth-${Math.min(depth, 2)}`, `wire-${style}`];
  if (red) classes.push("red");
  if (data?.ghost) classes.push("ghost");
  if (selected) classes.push("selected");
  if (attached) classes.push("attached");

  return (
    <g
      className={classes.join(" ")}
      data-wire={wire?.id ?? id}
      data-style={style}
      data-attached={attached ? "" : undefined}
      data-trace-yield={yielded ? "" : undefined}
    >
      <title>{title}</title>
      <path d={path} className="cicada-edge-glow" style={{ stroke: color, strokeWidth: width + 6 }} />
      <BaseEdge
        id={id}
        path={path}
        style={{
          stroke: color,
          strokeWidth: width,
          opacity: red ? 0.9 : 1,
          // The tree: one thick dashed stroke (U26).
          strokeDasharray: style === "dashed" ? `${unit / 3} ${unit / 5}` : undefined,
        }}
        interactionWidth={16}
      />
      {style === "double" && (
        // The list: a core line in the canvas background over the wide
        // stroke leaves two parallel lines (GH's double wire, docs/09).
        <path d={path} className="cicada-edge-core" style={{ strokeWidth: Math.max(1, width - 2.5) }} />
      )}
      {red && <path d={path} className="cicada-edge-red-dash" style={{ strokeWidth: width }} />}
      {wire && wire.lift > 0 && (
        <EdgeLabelRenderer>
          <div
            className="cicada-edge-chip nodrag nopan"
            style={{ transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)` }}
            title={`each()${wire.lift > 1 ? ` ×${wire.lift}` : ""} — the target kwarg is mapped over this list`}
          >
            map{wire.lift > 1 ? ` ×${wire.lift}` : ""}
          </div>
        </EdgeLabelRenderer>
      )}
    </g>
  );
}

export const CicadaEdge = memo(CicadaEdgeImpl);
