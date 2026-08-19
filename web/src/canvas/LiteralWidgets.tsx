/**
 * The scalar literal editors shared by the param widget row, the inline
 * kwarg literals on ordinary calls (`sphere(radius=0.75, segments=24)`), and
 * the inspector's input rows: number / integer (Enter/blur commit, typing
 * previews via `param_preview` at most once per animation frame), boolean
 * (checkbox), text (Enter/blur commit). Every commit goes through the ONE
 * literal-spelling rule (`paramValueText`) as `set_param {node, port,
 * value}`; every widget carries `nodrag` so React Flow never starts a node
 * drag from it, and Escape reverts to the authoritative value.
 */
import { useEffect, useRef, useState } from "react";
import { paramValueText, type LiteralKind } from "../state/literals";
import { useCicada } from "../state/store";
import { useParamSender } from "./useParamSender";
import "./canvas.css";

export type ScalarKind = Exclude<LiteralKind, "slider" | "toggle" | "list">;

export interface LiteralWidgetProps {
  /** Binding name. */
  node: string;
  /** Kwarg name; `null` = the literal binding itself (`x = 3.0`). */
  port: string | null;
  kind: ScalarKind;
  /** The authoritative value (from the view-model). */
  value: number | boolean | string;
  /** `canWrite` — disabled otherwise. */
  writable: boolean;
  /** Compact inline variant for a node's port row / the inspector. */
  compact?: boolean;
  testId: string;
  label: string;
}

/** Keyboard inside a widget never reaches the hotkey map or React Flow. */
function keyGuard(onEnter: () => void, onEscape: () => void) {
  return (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      onEnter();
      (event.target as HTMLInputElement).blur();
    } else if (event.key === "Escape") {
      onEscape();
      (event.target as HTMLInputElement).blur();
    }
    event.stopPropagation();
  };
}

export function NumberLiteralInput({ node, port, kind, value, writable, compact, testId, label }: LiteralWidgetProps) {
  const numKind: "number" | "integer" = kind === "integer" ? "integer" : "number";
  const authoritative = typeof value === "number" ? value : Number(value);
  const [local, setLocal] = useState(String(authoritative));
  const [invalid, setInvalid] = useState(false);
  const editing = useRef(false);
  const { preview, commit } = useParamSender(node, port);

  // The server's value wins whenever we are not mid-edit.
  useEffect(() => {
    if (!editing.current) setLocal(String(authoritative));
  }, [authoritative]);

  const parse = (raw: string): string | null => {
    const x = Number(raw);
    if (raw.trim() === "" || !Number.isFinite(x)) return null;
    if (numKind === "integer" && !Number.isInteger(x)) return null;
    return paramValueText(numKind, x);
  };

  const onChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    editing.current = true;
    setLocal(event.target.value);
    const text = parse(event.target.value);
    setInvalid(text === null);
    if (text !== null && writable) preview(text);
  };
  const revert = () => {
    editing.current = false;
    setInvalid(false);
    setLocal(String(authoritative));
  };
  const finish = () => {
    if (!editing.current) return;
    editing.current = false;
    const text = parse(local);
    if (text === null) {
      // Refused loudly; the field reverts to the authoritative value.
      useCicada.getState().addNotice(
        "warning",
        `${node}${port !== null ? `.${port}` : ""}: "${local}" is not a valid ${numKind}`,
      );
      setInvalid(false);
      setLocal(String(authoritative));
      return;
    }
    setInvalid(false);
    if (text !== paramValueText(numKind, authoritative)) commit(text);
    else setLocal(String(authoritative));
  };

  return (
    <input
      type="number"
      className={`lit-input nodrag nopan nowheel mono${compact ? " compact" : ""}${invalid ? " invalid" : ""}`}
      step={numKind === "integer" ? 1 : "any"}
      value={local}
      disabled={!writable}
      onChange={onChange}
      onBlur={finish}
      onKeyDown={keyGuard(finish, revert)}
      aria-label={label}
      title={writable ? `${label} (${numKind}) — Enter/blur commits, Esc reverts` : `${label} (${numKind})`}
      data-testid={testId}
    />
  );
}

export function BooleanLiteralInput({ node, port, value, writable, compact, testId, label }: LiteralWidgetProps) {
  const on = value === true || value === "True" || value === "true";
  const { commit } = useParamSender(node, port);
  return (
    <input
      type="checkbox"
      className={`lit-check nodrag nopan${compact ? " compact" : ""}`}
      checked={on}
      disabled={!writable}
      onChange={(event) => commit(paramValueText("boolean", event.target.checked))}
      aria-label={label}
      data-testid={testId}
    />
  );
}

export function TextLiteralInput({ node, port, value, writable, compact, testId, label }: LiteralWidgetProps) {
  const authoritative = String(value);
  const [local, setLocal] = useState(authoritative);
  const editing = useRef(false);
  const { commit } = useParamSender(node, port);
  useEffect(() => {
    if (!editing.current) setLocal(authoritative);
  }, [authoritative]);
  const revert = () => {
    editing.current = false;
    setLocal(authoritative);
  };
  const finish = () => {
    if (!editing.current) return;
    editing.current = false;
    if (local !== authoritative) commit(paramValueText("text", local));
  };
  return (
    <input
      type="text"
      className={`lit-input nodrag nopan nowheel mono${compact ? " compact" : ""}`}
      value={local}
      disabled={!writable}
      onChange={(event) => {
        editing.current = true;
        setLocal(event.target.value);
      }}
      onBlur={finish}
      onKeyDown={keyGuard(finish, revert)}
      aria-label={label}
      title={writable ? `${label} (text) — Enter/blur commits, Esc reverts` : `${label} (text)`}
      data-testid={testId}
    />
  );
}

/** Dispatch on the literal kind. */
export function LiteralWidget(props: LiteralWidgetProps) {
  switch (props.kind) {
    case "number":
    case "integer":
      return <NumberLiteralInput {...props} />;
    case "boolean":
      return <BooleanLiteralInput {...props} />;
    case "text":
      return <TextLiteralInput {...props} />;
  }
}
