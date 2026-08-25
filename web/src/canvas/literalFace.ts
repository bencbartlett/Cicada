/**
 * What a typed-literal chip shows and where its editor starts (wave 4 B3,
 * finding U9) — pure, unit-tested, shared by the canvas node row and the
 * inspector's Node tab. An unwired literal-typed port is in one of three
 * states: the text carries a kwarg (`literal` — shown as written), the
 * catalog has a default the text omits (`default` — shown greyed, spelled
 * as the dialect would write it: `False`, never the macro's `false`), or
 * nothing at all (`unset` — a required port nobody typed into yet).
 * `spelled` is the `set_param` text that would leave the file as it is:
 * a commit equal to it writes nothing.
 */
import { paramValueText, type LiteralKind } from "../state/literals";

export type ChipKind = Exclude<LiteralKind, "slider" | "toggle" | "list">;
export type Scalar = number | boolean | string;
export type ChipState = "literal" | "default" | "unset";

export interface ChipSource {
  kind: ChipKind;
  /** The kwarg's literal as the text spells it, when the text carries one. */
  literal?: string;
  /** That literal parsed, when it is a scalar (`InputView.literal_value`). */
  value?: Scalar;
  /** The catalog default as rendered (`InputView.default`). */
  defaultText?: string;
  /** The catalog default parsed in the port's kind (`InputView.default_value`). */
  defaultValue?: Scalar;
}

export interface ChipFace {
  state: ChipState;
  /** The chip's text. */
  text: string;
  /** The scalar the editor starts from; `null` = start empty / unchecked. */
  start: Scalar | null;
  /** The editor's initial text for a number or text input (`""` when empty). */
  startText: string;
  /** The editor's initial state for a Boolean checkbox. */
  startChecked: boolean;
  /** The `set_param` spelling of what the chip shows; `null` when nothing is shown. */
  spelled: string | null;
}

/** `value` when it is a scalar of `kind`, else `null` (a `"x"` on a Number port). */
export function ofKind(kind: ChipKind, value: Scalar | undefined): Scalar | null {
  if (value === undefined) return null;
  switch (kind) {
    case "number":
      return typeof value === "number" && Number.isFinite(value) ? value : null;
    case "integer":
      return typeof value === "number" && Number.isInteger(value) ? value : null;
    case "boolean":
      return typeof value === "boolean" ? value : null;
    case "text":
      return typeof value === "string" ? value : null;
  }
}

/** A text input's initial content for `start`: the literal as written for numbers, the bare string for text. */
function startTextOf(kind: ChipKind, start: Scalar | null, literal: string | undefined): string {
  if (start === null || kind === "boolean") return "";
  if (kind === "text") return String(start);
  // A number keeps the spelling the text has (`40.0`, `1e3`) so an
  // unchanged Enter compares equal; a default has no spelling but its own.
  return literal ?? String(start);
}

export function chipFace(source: ChipSource): ChipFace {
  const { kind } = source;
  if (source.literal !== undefined) {
    const start = ofKind(kind, source.value);
    return {
      state: "literal",
      text: source.literal,
      start,
      startText: startTextOf(kind, start, source.literal),
      startChecked: start === true,
      spelled: source.literal,
    };
  }
  if (source.defaultText !== undefined) {
    const start = ofKind(kind, source.defaultValue);
    const spelled = start === null ? null : paramValueText(kind, start);
    return {
      state: "default",
      text: spelled ?? source.defaultText,
      start,
      startText: startTextOf(kind, start, undefined),
      startChecked: start === true,
      spelled,
    };
  }
  return { state: "unset", text: "…", start: null, startText: "", startChecked: false, spelled: null };
}

/** The hover text of a chip. */
export function chipTitle(
  label: string,
  face: ChipFace,
  writable: boolean,
  gesture: "double-click" | "click",
): string {
  const how = writable ? ` — ${gesture} to ${face.state === "literal" ? "edit" : "type a value"}` : "";
  switch (face.state) {
    case "literal":
      return `${label} = ${face.text}${how}`;
    case "default":
      return `${label}: default ${face.text} (not in the text)${how}`;
    case "unset":
      return `${label}: required, nothing typed yet${how}`;
  }
}

/** A spelled edit: the `set_param` text and the scalar it denotes. */
export interface SpelledEdit {
  spelled: string;
  value: Scalar;
}

/**
 * The number spellings the editor accepts — the dialect's own grammar
 * (`parse.rs::parse_number`: digits, an optional point, an optional
 * exponent; `.5` and `5.` included) plus a leading `+`. Nothing else is
 * reinterpreted: `0x10`, `1_000`, `Infinity` and `3,5` are refusals, not
 * the numbers JavaScript's `Number()` would quietly make of them.
 */
const NUMBER_SPELLING = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;

/**
 * The `set_param` spelling of what an editor holds, or the reason it
 * cannot be spelled (an empty number field is no value; `2.5` is no
 * integer; `3,5` and `1/2` are no numbers). `{ skip: true }` = nothing
 * typed, nothing to write. The number field is a plain text field, so
 * every keystroke reaches this rule — a browser `type="number"` input
 * drops the characters it dislikes before anyone sees them (`3,5` → `35`).
 */
export function spellEdit(kind: ChipKind, held: string | boolean): SpelledEdit | { skip: true } | { error: string } {
  switch (kind) {
    case "boolean": {
      const value = held === true;
      return { spelled: paramValueText("boolean", value), value };
    }
    case "text": {
      const value = String(held);
      return { spelled: paramValueText("text", value), value };
    }
    case "number":
    case "integer": {
      const raw = String(held).trim();
      if (raw === "") return { skip: true };
      if (!NUMBER_SPELLING.test(raw)) return { error: `"${raw}" is not a valid number` };
      const x = Number(raw);
      if (!Number.isFinite(x)) return { error: `"${raw}" is too large for the Number type` };
      if (kind === "integer" && !Number.isInteger(x)) return { error: `"${raw}" is not an integer` };
      return { spelled: paramValueText(kind, x), value: x };
    }
  }
}

/**
 * Whether a spelled edit leaves the file as it is: what the chip already
 * showed — the literal as written, or the default the text omits — is no
 * edit. Compared by VALUE as well as by spelling: `0` typed over a Number
 * port's `start=0` (the checker accepts the integer spelling) denotes the
 * same number as the `0.0` the rule would write, and a spelling-only op is
 * no edit the user made.
 */
export function isNoEdit(face: ChipFace, edit: SpelledEdit): boolean {
  return edit.spelled === face.spelled || (face.start !== null && edit.value === face.start);
}
