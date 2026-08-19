/**
 * Wires (docs/16 + docs/09): kind-colored, depth-styled (depth 0 single ·
 * depth 1 heavier · depth ≥ 2 double/hatched), red with the reason on hover,
 * spline (bezier) or trace (orthogonal, snapped to the grid — a step-edge
 * approximation of the 45° PCB look) per the user's wire-mode setting, and
 * a `map` chip near the target when the kwarg is lifted.
 */
import {
  BaseEdge,
  EdgeLabelRenderer,
  getBezierPath,
  getSmoothStepPath,
  type EdgeProps,
} from "@xyflow/react";
import { memo } from "react";
import { baseOfType, kindColor } from "../kinds";
import { useCicada } from "../state/store";
import type { CanvasEdge } from "./flow";
import { wireStrokeWidth } from "./grid";

function CicadaEdgeImpl({
  id,
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
  const wire = data?.wire;
  const depth = wire?.depth ?? 0;
  const red = wire?.red ?? false;
  const color = red ? "var(--error)" : kindColor(baseOfType(wire?.type ?? "?"));
  const width = wireStrokeWidth(depth);

  const params = { sourceX, sourceY, targetX, targetY, sourcePosition, targetPosition };
  const [path, labelX, labelY] =
    wireMode === "trace"
      ? getSmoothStepPath({ ...params, borderRadius: unit / 2, offset: unit })
      : getBezierPath(params);

  const title = red
    ? `${wire?.id ?? id}: ${wire?.reason ?? "red"}`
    : `${wire?.id ?? id}: ${wire?.type ?? "?"}${depth > 0 ? ` (depth ${depth})` : ""}${
        wire && wire.lift > 0 ? ` · each()${wire.lift > 1 ? ` ×${wire.lift}` : ""}` : ""
      }`;

  const classes = ["cicada-edge", `depth-${Math.min(depth, 2)}`];
  if (red) classes.push("red");
  if (selected) classes.push("selected");

  return (
    <g className={classes.join(" ")} data-wire={wire?.id ?? id}>
      <title>{title}</title>
      {selected && (
        <path d={path} className="cicada-edge-glow" style={{ stroke: color, strokeWidth: width + 6 }} />
      )}
      <BaseEdge
        id={id}
        path={path}
        style={{ stroke: color, strokeWidth: width, opacity: red ? 0.9 : 1 }}
        interactionWidth={16}
      />
      {depth >= 2 && (
        // The hatched core: a dashed line in the canvas background over the
        // wide stroke reads as a double/hatched wire (docs/09).
        <path
          d={path}
          className="cicada-edge-hatch"
          style={{ strokeWidth: Math.max(1, width - 2.5), strokeDasharray: `${unit / 6} ${unit / 6}` }}
        />
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
