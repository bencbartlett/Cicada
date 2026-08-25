/**
 * The typed-literal chip (docs/16 §Canvas conventions, wave 4 B3 / finding
 * U9): every UNWIRED input whose type takes a literal — Number, Integer,
 * Text, Boolean and their `?` forms — wears its value as a chip on the
 * canvas node row and in the inspector's Node tab: the kwarg's literal as
 * the text spells it, else the catalog default greyed, else an empty
 * required slot (`literalFace.ts` is the rule). Double-click (canvas) /
 * click (inspector) opens an input in place; Enter — or leaving the field —
 * commits ONE `set_param {node, port, value}` spelled by `paramValueText`
 * (the server adds the kwarg when the call lacks it, in spec order); Esc
 * cancels. Nothing streams from the typed editor: Enter is the one write,
 * so a cancelled edit leaves no preview behind, and a value equal to what
 * the chip showed writes nothing (`isNoEdit`: by value, so `0` over a
 * Number port's `start=0` is no spelling-only op). An unspellable value
 * (`2.5` on an Integer port, `3,5` or `1/2` anywhere) is refused with a
 * notice, never written — the number editor is a plain text field so the
 * rule sees every keystroke; a browser number input would have dropped the
 * offending characters and written `35`. A literal inside `each(…)` is
 * edited inside it (the writer rewrites the inner token; the lift stays).
 * Observers and `#off` ghosts get no chip — the callers show the text.
 *
 * Keys inside the editor never reach the hotkey map or React Flow (the
 * keydown is stopped) — except `Ctrl+S`, the commit dialog, which must
 * bubble from every text field (docs/16 §Keyboard map).
 */
import { useEffect, useRef, useState } from "react";
import { isCommitChord } from "../keyboard";
import { useCicada } from "../state/store";
import { sendWrite } from "./flow";
import { chipFace, chipTitle, isNoEdit, spellEdit, type ChipKind, type ChipSource } from "./literalFace";
import "./canvas.css";

export interface LiteralChipProps extends ChipSource {
  /** Binding name. */
  node: string;
  /** Kwarg name. */
  port: string;
  /** The canvas opens on double-click (a click selects the node); the inspector on click. */
  surface: "canvas" | "inspector";
  testId: string;
  /** `node.port`, for the tooltip, the notice and the accessible name. */
  label: string;
}

interface EditorProps {
  kind: ChipKind;
  startText: string;
  startChecked: boolean;
  label: string;
  testId: string;
  onCommit: (held: string | boolean) => void;
  onCancel: () => void;
}

/** The opened input: focused on mount, Enter/blur commit, Esc cancels, one outcome only. */
function LiteralEditor({ kind, startText, startChecked, label, testId, onCommit, onCancel }: EditorProps) {
  const ref = useRef<HTMLInputElement>(null);
  const [text, setText] = useState(startText);
  const [checked, setChecked] = useState(startChecked);
  // Enter blurs the field and a removed field may blur too: the first
  // outcome (commit or cancel) is the only one.
  const settled = useRef(false);

  useEffect(() => {
    const input = ref.current;
    if (input === null) return;
    input.focus();
    if (kind !== "boolean") input.select();
  }, [kind]);

  const finish = (commit: boolean) => {
    if (settled.current) return;
    settled.current = true;
    if (commit) onCommit(kind === "boolean" ? checked : text);
    else onCancel();
  };
  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (isCommitChord(event)) return;
    if (event.key === "Enter") {
      event.preventDefault();
      finish(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      finish(false);
    }
    event.stopPropagation();
  };
  const hint = `${label} (${kind}) — Enter commits, Esc cancels`;

  if (kind === "boolean") {
    return (
      <input
        ref={ref}
        type="checkbox"
        className="lit-check compact nodrag nopan"
        checked={checked}
        onChange={(event) => setChecked(event.target.checked)}
        onBlur={() => finish(true)}
        onKeyDown={onKeyDown}
        aria-label={label}
        title={hint}
        data-testid={testId}
      />
    );
  }
  // A plain text field for numbers too — never `type="number"`: the browser
  // sanitises that one's value before React sees it (`3,5` → `35`, `1/2` →
  // `12`, `abc` → empty), so the refusal in `spellEdit` would never fire and
  // a different number than typed would be written. `inputMode` keeps the
  // numeric keyboard where there is one.
  return (
    <input
      ref={ref}
      type="text"
      inputMode={kind === "text" ? undefined : "decimal"}
      className="lit-input compact nodrag nopan nowheel mono"
      value={text}
      onChange={(event) => setText(event.target.value)}
      onBlur={() => finish(true)}
      onKeyDown={onKeyDown}
      aria-label={label}
      title={hint}
      data-testid={testId}
    />
  );
}

export function LiteralChip(props: LiteralChipProps) {
  const { node, port, kind, surface, testId, label } = props;
  const [editing, setEditing] = useState(false);
  const face = chipFace(props);
  const gesture = surface === "canvas" ? "double-click" : "click";

  const commit = (held: string | boolean) => {
    setEditing(false);
    const spelling = spellEdit(kind, held);
    if ("skip" in spelling) return;
    if ("error" in spelling) {
      useCicada.getState().addNotice("warning", `${label}: ${spelling.error} — nothing written`);
      return;
    }
    // What the chip already showed — the literal as written, or the
    // default the text omits — is no edit; compared by value, so a
    // re-spelling alone (`0` → `0.0`) is no op either.
    if (isNoEdit(face, spelling)) return;
    sendWrite({ type: "set_param", payload: { node, port, value: spelling.spelled } });
  };

  if (editing) {
    return (
      <span className={surface === "canvas" ? "cn-literal-edit" : "insp-literal-edit"}>
        <LiteralEditor
          kind={kind}
          startText={face.startText}
          startChecked={face.startChecked}
          label={label}
          testId={`${testId}-input`}
          onCommit={commit}
          onCancel={() => setEditing(false)}
        />
      </span>
    );
  }
  const open = (event: React.SyntheticEvent) => {
    event.stopPropagation();
    setEditing(true);
  };
  return (
    <button
      type="button"
      className={`lit-chip mono nodrag ${face.state}${surface === "canvas" ? " cn-literal-chip" : ""}`}
      title={chipTitle(label, face, true, gesture)}
      aria-label={`${label}: ${face.state === "unset" ? "required, nothing typed yet" : face.text}`}
      data-testid={testId}
      data-state={face.state}
      onClick={surface === "inspector" ? open : undefined}
      onDoubleClick={surface === "canvas" ? open : undefined}
      onKeyDown={surface === "canvas" ? (event) => event.key === "Enter" && open(event) : undefined}
    >
      {face.text}
    </button>
  );
}
