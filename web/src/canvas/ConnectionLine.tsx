/**
 * The in-flight wire while dragging a connection: colored by the source
 * port's kind family, dashed while no valid target is under the pointer;
 * in trace mode the same PCB-style path the placed wires take (`trace.ts`,
 * its natural route — a wire not yet on the canvas holds no lane).
 */
import { Position, getBezierPath, type ConnectionLineComponentProps } from "@xyflow/react";
import { kindColor } from "../kinds";
import { useCicada } from "../state/store";
import type { CanvasNode } from "./flow";
import { tracePath } from "./trace";

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
  const unit = useCicada((s) => s.hello?.unitPx ?? 24);
  const view = fromNode.data.view;
  const port =
    fromHandle.type === "source"
      ? view.outputs.find((o) => o.name === fromHandle.id)
      : view.inputs.find((i) => i.name === fromHandle.id);
  const color = kindColor(port?.base ?? "?");
  let path: string;
  if (wireMode === "trace") {
    // A drag from a TARGET handle (re-wiring a wire's source end) starts at
    // the wire's entry: the pointer is then the source the trace leaves.
    const ends =
      fromPosition === Position.Left
        ? { sx: toX, sy: toY, tx: fromX, ty: fromY }
        : { sx: fromX, sy: fromY, tx: toX, ty: toY };
    [path] = tracePath(ends, unit);
  } else {
    [path] = getBezierPath({
      sourceX: fromX,
      sourceY: fromY,
      targetX: toX,
      targetY: toY,
      sourcePosition: fromPosition,
      targetPosition: toPosition,
    });
  }
  const cls = ["cicada-connection", connectionStatus ?? "free"];
  return (
    <g className={cls.join(" ")}>
      <path d={path} fill="none" style={{ stroke: color }} className="cicada-connection-path" />
      <circle cx={toX} cy={toY} r={4} style={{ fill: color }} className="cicada-connection-tip" />
    </g>
  );
}
