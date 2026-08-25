/**
 * The on-canvas param widget row (docs/10 §3): slider / number / integer /
 * toggle / text / choice (a select over the `choice` node's options —
 * catalog C2b; picking one commits ONE `set_param` spelling the option as
 * a Text literal; a value the text carries that is not among the options
 * stays selectable as "(not an option)" so the red node shows what it
 * says; wired options leave a plain text field). While a slider drags, `param_preview` streams at most one
 * value per animation frame (latest wins); release commits `set_param` with
 * the same shortest-repr text. The scalar editors (number / toggle / text)
 * are the shared `LiteralWidgets` — the same ones the inline kwarg literals
 * and the inspector use. Every widget carries `nodrag` so React Flow never
 * starts a node drag from it.
 *
 * Compute-on-release (docs/13 §Slider drags, DECISIONS.md row 39): when
 * the server answers a drag's tick with `preview_policy`, the store holds
 * the pending param and the slider says so — the thumb keeps following the
 * pointer, the value label shows the pending value in the warn color, and
 * a `pending · N s` chip hangs under the row (`~` when the estimate is a
 * floor); the viewport is NOT expected to move until release. The chip is
 * positioned absolutely so the track never changes width under a held
 * pointer (a narrower track would jump the thumb away from the pointer).
 * A release on the committed value writes nothing and sends `end_drag`
 * instead (the server's `drag_ended` then clears the twin widget and the
 * observers). A cheap cone never hears of the policy and previews live, as
 * before.
 */
import { useEffect, useRef, useState } from "react";
import { isCommitChord } from "../keyboard";
import type { NodeView, ParamView } from "../protocol/messages";
import { pendingHint, pendingTitle } from "../panels/format";
import { pendingFor, useCicada } from "../state/store";
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
  // This widget is mid-edit (first change → release): its own thumb is
  // what it shows, whatever the pending entry says.
  const [engaged, setEngaged] = useState(false);
  const dragging = useRef(false);
  const dirty = useRef(false);
  const port = param.port ?? null;
  const { preview, commit, cancel } = useParamSender(view.name, port);
  const pending = useCicada((s) => pendingFor(s, view.name, port));
  const trackPendingValue = useCicada((s) => s.trackPendingValue);
  const endDrag = useCicada((s) => s.endDrag);

  // The server's value wins whenever we are not mid-drag.
  useEffect(() => {
    if (!dragging.current) setLocal(authoritative);
  }, [authoritative]);

  const text = (x: number) => paramValueText("slider", x);

  // What the thumb and the label show: my own edit as it happens;
  // otherwise the pending value of a compute-on-release drag driven
  // elsewhere (the params panel's twin of this slider, or — as an observer
  // — the writer's); else the committed value.
  const pendingNumber = pending === undefined ? NaN : Number(pending.value);
  const shown = !engaged && Number.isFinite(pendingNumber) ? pendingNumber : local;

  const onChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const x = snapToStep(Number(event.target.value), min, step);
    setLocal(x);
    setEngaged(true);
    dirty.current = true;
    if (writer) {
      const literal = text(x);
      preview(literal);
      // Only the FIRST withheld tick's value is in the policy message — the
      // pending entry follows the thumb from here (a no-op while live).
      trackPendingValue(view.name, port, literal);
    }
  };
  const release = () => {
    dragging.current = false;
    if (!dirty.current) return;
    dirty.current = false;
    setEngaged(false);
    if (local !== authoritative) {
      // The write ends the drag server-side; its delta (or the error that
      // refuses it) takes the pending badge down together with the value.
      commit(text(local));
    } else {
      // Released on the committed value: no write goes out, so nothing
      // else would take the badge down — the store clears it and tells the
      // server the drag is over (`end_drag`), so a re-grab inside the gap
      // is a fresh drag and every other client hears `drag_ended`. The
      // queued tick (if any) is dropped first: sent after the `end_drag`
      // it would be a fresh drag on the committed value.
      cancel();
      endDrag(view.name, port);
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
    <div
      className={`cn-widget cn-slider nodrag nopan nowheel${pending === undefined ? "" : " pending"}`}
      title={`slider ${min} … ${max}`}
    >
      <input
        type="range"
        className="nodrag"
        min={min}
        max={max}
        step={step}
        value={shown}
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
      <span
        className="cn-widget-value mono"
        title={pending === undefined ? undefined : pendingTitle(pending)}
        data-testid={`slider-value-${view.name}`}
      >
        {text(shown)}
      </span>
      {pending !== undefined && (
        <span
          className="cn-pending mono"
          title={pendingTitle(pending)}
          data-testid={`pending-${view.name}`}
          role="status"
        >
          {pendingHint(pending)}
        </span>
      )}
    </div>
  );
}

/**
 * The `choice` node's dropdown. The options come from the view-model (the
 * text's literal list, in order); the current value is the text's, whether
 * or not it is one of them — a stray value (the node is red) is shown as
 * an extra option marked so, never silently replaced by the first option.
 * A wired `options` list is a solve result the view cannot read: the value
 * is typed as text then.
 */
function ChoiceWidget({ view, param, writer }: Props) {
  const port = param.port ?? null;
  const { commit } = useParamSender(view.name, port);
  const current = String(param.value);
  const options = param.options;
  if (options === undefined) {
    return (
      <div className="cn-widget cn-text nodrag nopan nowheel" title="options are wired — type the value">
        <LiteralWidget
          node={view.name}
          port={port}
          kind="text"
          value={param.value}
          writable={writer}
          testId={`choice-${view.name}`}
          label={`${view.name} value`}
        />
      </div>
    );
  }
  const stray = !options.includes(current);
  return (
    <div
      className={`cn-widget cn-choice nodrag nopan nowheel${stray ? " stray" : ""}`}
      title={stray ? `"${current}" is not one of the options` : `${options.length} option${options.length === 1 ? "" : "s"}`}
    >
      <select
        className="nodrag"
        value={current}
        disabled={!writer}
        aria-label={`${view.name} value`}
        data-testid={`choice-${view.name}`}
        data-stray={stray ? "true" : undefined}
        onChange={(event) => {
          if (event.target.value !== current) commit(paramValueText("choice", event.target.value));
        }}
        onKeyDown={(event) => {
          // Arrows and letters pick options; nothing reaches the hotkey map
          // or React Flow — except Ctrl+S, the commit dialog (docs/16).
          if (!isCommitChord(event)) event.stopPropagation();
        }}
      >
        {stray && <option value={current}>{current} (not an option)</option>}
        {options.map((option, index) => (
          <option key={`${index}-${option}`} value={option}>
            {option}
          </option>
        ))}
      </select>
    </div>
  );
}

export function ParamWidget(props: Props) {
  const { view, param, writer } = props;
  const port = param.port ?? null;
  switch (param.kind) {
    case "slider":
      return <SliderWidget {...props} />;
    case "choice":
      return <ChoiceWidget {...props} />;
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
