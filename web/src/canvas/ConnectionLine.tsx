/**
 * The in-flight wire while dragging a connection: colored by the source
 * port's kind family, dashed while no valid target is under the pointer.
 */
import { getBezierPath, getSmoothStepPath, type ConnectionLineComponentProps } from "@xyflow/react";
import { kindColor } from "../kinds";
import { useCicada } from "../state/store";
import type { CanvasNode } from "./flow";

export function ConnectionLine({
  fromNode,
  fromHandle,
  fromX,
  fromY,
  toX,
  toY,
  fromPosition,
  toPosition,
  connectionStatus,
}: ConnectionLineComponentProps<CanvasNode>) {
  const wireMode = useCicada((s) => s.settings.wireMode);
  const view = fromNode.data.view;
  const port =
    fromHandle.type === "source"
      ? view.outputs.find((o) => o.name === fromHandle.id)
      : view.inputs.find((i) => i.name === fromHandle.id);
  const color = kindColor(port?.base ?? "?");
  const params = { sourceX: fromX, sourceY: fromY, targetX: toX, targetY: toY, sourcePosition: fromPosition, targetPosition: toPosition };
  const [path] = wireMode === "trace" ? getSmoothStepPath(params) : getBezierPath(params);
  const cls = ["cicada-connection", connectionStatus ?? "free"];
  return (
    <g className={cls.join(" ")}>
      <path d={path} fill="none" style={{ stroke: color }} className="cicada-connection-path" />
      <circle cx={toX} cy={toY} r={4} style={{ fill: color }} className="cicada-connection-tip" />
    </g>
  );
}
