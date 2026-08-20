/**
 * The on-canvas param widget row (docs/10 §3): slider / number / integer /
 * toggle / text. While a slider drags, `param_preview` streams at most one
 * value per animation frame (latest wins); release commits `set_param` with
 * the same shortest-repr text. The scalar editors (number / toggle / text)
 * are the shared `LiteralWidgets` — the same ones the inline kwarg literals
 * and the inspector use. Every widget carries `nodrag` so React Flow never
 * starts a node drag from it.
 */
import { useEffect, useRef, useState } from "react";
import type { NodeView, ParamView } from "../protocol/messages";
import { paramValueText, sliderStep, snapToStep } from "./grid";
import { LiteralWidget } from "./LiteralWidgets";
import { useParamSender } from "./useParamSender";

interface Props {
  view: NodeView;
  param: ParamView;
  /** `canWrite` — widgets are disabled otherwise. */
  writer: boolean;
}

function SliderWidget({ view, param, writer }: Props) {
  const min = param.min ?? 0;
  const max = param.max ?? 10;
  const step = sliderStep(min, max, param.step);
  const authoritative = typeof param.value === "number" ? param.value : Number(param.value);
  const [local, setLocal] = useState(authoritative);
  const dragging = useRef(false);
  const dirty = useRef(false);
  const { preview, commit } = useParamSender(view.name, param.port ?? null);

  // The server's value wins whenever we are not mid-drag.
  useEffect(() => {
    if (!dragging.current) setLocal(authoritative);
  }, [authoritative]);

  const text = (x: number) => paramValueText("slider", x);

  const onChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const x = snapToStep(Number(event.target.value), min, step);
    setLocal(x);
    dirty.current = true;
    if (writer) preview(text(x));
  };
  const release = () => {
    dragging.current = false;
    if (!dirty.current) return;
    dirty.current = false;
    if (local !== authoritative) {
      commit(text(local));
    }
  };
  // A pointer drag is over: hand the focus back to the canvas, so Del /
  // arrows / P work again at once (a focused range input would keep its
  // plain keys — docs/16 keyboard map). Keyboard-driven steps keep focus.
  const pointerRelease = (event: React.PointerEvent<HTMLInputElement>) => {
    release();
    event.currentTarget.blur();
  };

  return (
    <div className="cn-widget cn-slider nodrag nopan nowheel" title={`slider ${min} … ${max}`}>
      <input
        type="range"
        className="nodrag"
        min={min}
        max={max}
        step={step}
        value={local}
        disabled={!writer}
        onPointerDown={() => {
          dragging.current = true;
        }}
        onChange={onChange}
        onPointerUp={pointerRelease}
        onPointerCancel={pointerRelease}
        onKeyUp={release}
        onBlur={release}
        aria-label={`${view.name} value`}
        data-testid={`slider-${view.name}`}
      />
      <span className="cn-widget-value mono">{text(local)}</span>
    </div>
  );
}

export function ParamWidget(props: Props) {
  const { view, param, writer } = props;
  const port = param.port ?? null;
  switch (param.kind) {
    case "slider":
      return <SliderWidget {...props} />;
    case "number":
    case "integer": {
      const kind = param.kind === "integer" ? "integer" : "number";
      return (
        <div className="cn-widget cn-number nodrag nopan nowheel">
          <LiteralWidget
            node={view.name}
            port={port}
            kind={kind}
            value={param.value}
            writable={writer}
            testId={`number-${view.name}`}
            label={`${view.name} value`}
          />
          <span className="cn-widget-kind faint">{kind}</span>
        </div>
      );
    }
    case "toggle":
    case "boolean": {
      const on = param.value === true || param.value === "True" || param.value === "true";
      return (
        <label className="cn-widget cn-toggle nodrag nopan">
          <LiteralWidget
            node={view.name}
            port={port}
            kind="boolean"
            value={param.value}
            writable={writer}
            testId={`toggle-${view.name}`}
            label={`${view.name} value`}
          />
          <span className="mono">{on ? "True" : "False"}</span>
        </label>
      );
    }
    case "text":
      return (
        <div className="cn-widget cn-text nodrag nopan nowheel">
          <LiteralWidget
            node={view.name}
            port={port}
            kind="text"
            value={param.value}
            writable={writer}
            testId={`text-${view.name}`}
            label={`${view.name} value`}
          />
        </div>
      );
    case "list":
      return (
        <div className="cn-widget cn-list faint mono" title="list literals are edited in text (v0.1)">
          {String(param.value)}
        </div>
      );
  }
}
