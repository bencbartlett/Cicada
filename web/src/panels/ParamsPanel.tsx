/**
 * Params tab (docs/16 §Application layout): every param node (sliders,
 * toggles, constants) grouped by category, with the canvas widget
 * semantics — drag → `param_preview` (throttled to one per animation
 * frame), release/commit → `set_param`. The server writes the text; the
 * thumb keeps the dragged value only until the next delta overrides it.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { CATEGORY_ORDER } from "../kinds";
import type { NodeView, ParamView } from "../protocol/messages";
import { canWrite, useCicada } from "../state/store";
import { paramValueText, snapSlider } from "./format";

export function ParamsPanel() {
  const graph = useCicada((s) => s.graph);
  const writer = useCicada(canWrite);
  const selected = useCicada((s) => s.selection.nodes);
  const selectNodes = useCicada((s) => s.selectNodes);

  const groups = useMemo(() => {
    const byCategory = new Map<string, NodeView[]>();
    for (const node of graph.nodes) {
      if (node.param === undefined) continue;
      const list = byCategory.get(node.category) ?? [];
      list.push(node);
      byCategory.set(node.category, list);
    }
    const order = [...CATEGORY_ORDER, ...[...byCategory.keys()].filter((c) => !CATEGORY_ORDER.includes(c)).sort()];
    return order
      .filter((c) => byCategory.has(c))
      .map((category) => ({ category, nodes: byCategory.get(category) ?? [] }));
  }, [graph]);

  if (groups.length === 0) {
    return (
      <div className="faint" data-testid="params-empty">
        no param nodes yet — place a slider, toggle, or constant
      </div>
    );
  }
  return (
    <div data-testid="params-panel">
      {!writer && (
        <div className="faint" style={{ marginBottom: 8 }}>
          read-only — widgets are disabled until you hold the lease on an open connection
        </div>
      )}
      {groups.map((group) => (
        <section className="params-group" key={group.category}>
          <h3 className="insp-h">{group.category}</h3>
          {group.nodes.map((node) => (
            <ParamRow
              key={node.name}
              node={node}
              param={node.param!}
              writer={writer}
              selected={selected.includes(node.name)}
              onSelect={() => selectNodes([node.name])}
            />
          ))}
        </section>
      ))}
    </div>
  );
}

function ParamRow({
  node,
  param,
  writer,
  selected,
  onSelect,
}: {
  node: NodeView;
  param: ParamView;
  writer: boolean;
  selected: boolean;
  onSelect: () => void;
}) {
  // A `#off` ghost keeps its widget for the eye but takes no edits (the
  // writer refuses them by name): the row is dimmed and says so.
  const off = node.kind === "disabled";
  return (
    <div
      className={`param-row${selected ? " selected" : ""}${off ? " param-off" : ""}`}
      data-testid={`param-${node.name}`}
    >
      <span
        className="param-name"
        title={off ? `${node.text}\ndisabled (#off) — enable it to edit` : node.text}
        onClick={onSelect}
      >
        {node.name}
        <small>
          {off ? "disabled (#off)" : (node.func ?? node.kind)}
          {!off && param.port !== undefined ? `.${param.port}` : ""}
        </small>
      </span>
      <span className="param-widget">
        <ParamWidget node={node} param={param} disabled={!writer || off} />
      </span>
    </div>
  );
}

/** Send `set_param` with the dialect literal for this widget's value. */
function commitValue(node: string, param: ParamView, value: number | boolean | string) {
  const store = useCicada.getState();
  let text: string;
  try {
    text = paramValueText(param.kind, value);
  } catch (error: unknown) {
    store.addNotice("error", String(error));
    return;
  }
  store.send({ type: "set_param", payload: { node, port: param.port ?? null, value: text } });
}

export function ParamWidget({ node, param, disabled }: { node: NodeView; param: ParamView; disabled: boolean }) {
  switch (param.kind) {
    case "slider":
      return <SliderWidget node={node} param={param} disabled={disabled} />;
    case "number":
    case "integer":
      return <NumberWidget node={node} param={param} disabled={disabled} />;
    case "toggle":
    case "boolean":
      return (
        <input
          type="checkbox"
          checked={param.value === true}
          disabled={disabled}
          data-testid={`widget-${node.name}`}
          onChange={(e) => commitValue(node.name, param, e.target.checked)}
        />
      );
    case "text":
      return <TextWidget node={node} param={param} disabled={disabled} />;
    case "list":
      return (
        <span className="lit" title="list constants are edited on the canvas / in text">
          {typeof param.value === "string" ? param.value : JSON.stringify(param.value)}
        </span>
      );
  }
}

function SliderWidget({ node, param, disabled }: { node: NodeView; param: ParamView; disabled: boolean }) {
  const min = param.min ?? 0;
  const max = param.max ?? 10;
  const step = param.step ?? 0;
  const committed = Number(param.value);
  const [draft, setDraft] = useState<number | null>(null);
  const rangeRef = useRef<HTMLInputElement>(null);
  const frame = useRef<number | null>(null);
  const pendingPreview = useRef<number | null>(null);

  // A new authoritative value (delta after set_param, or someone else's
  // edit) overrides the optimistic thumb.
  useEffect(() => {
    setDraft(null);
  }, [committed]);

  const shown = draft ?? committed;

  const preview = (value: number) => {
    pendingPreview.current = value;
    if (frame.current !== null) return;
    frame.current = window.requestAnimationFrame(() => {
      frame.current = null;
      const v = pendingPreview.current;
      pendingPreview.current = null;
      if (v === null) return;
      useCicada.getState().send({
        type: "param_preview",
        payload: { node: node.name, port: param.port ?? null, value: paramValueText("slider", v) },
      });
    });
  };

  const commit = (value: number) => {
    if (frame.current !== null) {
      window.cancelAnimationFrame(frame.current);
      frame.current = null;
      pendingPreview.current = null;
    }
    if (!Number.isFinite(value)) {
      useCicada.getState().addNotice("error", `\`${node.name}\`: not a number`);
      setDraft(null);
      return;
    }
    const snapped = snapSlider(value, min, max, step);
    setDraft(snapped);
    if (snapped === committed) return;
    commitValue(node.name, param, snapped);
  };

  // Native `change` fires on release (mouse) / after each key step; React's
  // onChange fires on every `input` — so preview from React, commit from the
  // native change event (through a ref so the listener sees the latest
  // bounds without re-subscribing).
  const commitRef = useRef(commit);
  useEffect(() => {
    commitRef.current = commit;
  });
  useEffect(() => {
    const el = rangeRef.current;
    if (el === null) return;
    const onChange = () => commitRef.current(Number(el.value));
    el.addEventListener("change", onChange);
    return () => el.removeEventListener("change", onChange);
  }, []);

  useEffect(
    () => () => {
      if (frame.current !== null) window.cancelAnimationFrame(frame.current);
    },
    [],
  );

  return (
    <>
      <input
        ref={rangeRef}
        type="range"
        min={min}
        max={max}
        step={step > 0 ? step : "any"}
        value={shown}
        disabled={disabled}
        data-testid={`widget-${node.name}`}
        aria-label={`${node.name} slider`}
        onChange={(e) => {
          const v = Number(e.target.value);
          setDraft(v);
          preview(v);
        }}
      />
      <input
        type="number"
        min={min}
        max={max}
        step={step > 0 ? step : "any"}
        value={shown}
        disabled={disabled}
        aria-label={`${node.name} value`}
        data-testid={`number-${node.name}`}
        onChange={(e) => setDraft(Number(e.target.value))}
        onBlur={(e) => {
          if (draft !== null) commit(e.target.value === "" ? NaN : Number(e.target.value));
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            const raw = (e.target as HTMLInputElement).value;
            commit(raw === "" ? NaN : Number(raw));
          }
          if (e.key === "Escape") setDraft(null);
        }}
      />
    </>
  );
}

function NumberWidget({ node, param, disabled }: { node: NodeView; param: ParamView; disabled: boolean }) {
  const committed = Number(param.value);
  const [draft, setDraft] = useState<string | null>(null);
  useEffect(() => {
    setDraft(null);
  }, [committed]);
  const commit = (raw: string) => {
    const v = Number(raw);
    if (raw.trim() === "" || !Number.isFinite(v)) {
      useCicada.getState().addNotice("error", `\`${node.name}\`: "${raw}" is not a number`);
      setDraft(null);
      return;
    }
    const value = param.kind === "integer" ? Math.round(v) : v;
    if (value === committed) {
      setDraft(null);
      return;
    }
    commitValue(node.name, param, value);
  };
  return (
    <input
      type="number"
      step={param.kind === "integer" ? 1 : "any"}
      value={draft ?? String(committed)}
      disabled={disabled}
      data-testid={`widget-${node.name}`}
      aria-label={`${node.name} value`}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={(e) => {
        if (draft !== null) commit(e.target.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit((e.target as HTMLInputElement).value);
        if (e.key === "Escape") setDraft(null);
      }}
    />
  );
}

function TextWidget({ node, param, disabled }: { node: NodeView; param: ParamView; disabled: boolean }) {
  const committed = String(param.value);
  const [draft, setDraft] = useState<string | null>(null);
  useEffect(() => {
    setDraft(null);
  }, [committed]);
  const commit = (raw: string) => {
    if (raw === committed) {
      setDraft(null);
      return;
    }
    commitValue(node.name, param, raw);
  };
  return (
    <input
      type="text"
      value={draft ?? committed}
      disabled={disabled}
      data-testid={`widget-${node.name}`}
      aria-label={`${node.name} text`}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={(e) => {
        if (draft !== null) commit(e.target.value);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit((e.target as HTMLInputElement).value);
        if (e.key === "Escape") setDraft(null);
      }}
    />
  );
}
