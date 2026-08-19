/**
 * THE literal-spelling rule for `set_param` / `param_preview` values
 * (docs/10 §3): one function, imported by the canvas widgets, the inline
 * kwarg editors, and the panels — never re-derived. Pure, unit-tested.
 *
 * - Number-typed values keep a decimal point (`3` → `3.0`: a bare `3` would
 *   parse as an Integer literal); otherwise the shortest round-trip repr.
 * - Integer-typed values are bare integers; a non-integer is refused loudly.
 * - Booleans are the capitalised keywords `True` / `False`.
 * - Text is a JSON-quoted string (the dialect's escapes are JSON's).
 * - Lists have no widget (edited in text).
 */
import type { InputView, ParamView } from "../protocol/messages";

export type LiteralKind = ParamView["kind"];

export function paramValueText(kind: LiteralKind, value: number | boolean | string): string {
  switch (kind) {
    case "slider":
    case "number": {
      const x = typeof value === "number" ? value : Number(value);
      if (!Number.isFinite(x)) throw new Error(`param value is not a finite number: ${String(value)}`);
      return Number.isInteger(x) ? x.toFixed(1) : String(x);
    }
    case "integer": {
      const x = typeof value === "number" ? value : Number(value);
      if (!Number.isInteger(x)) throw new Error(`param value is not an integer: ${String(value)}`);
      return String(x);
    }
    case "toggle":
    case "boolean":
      return value === true || value === "true" || value === "True" ? "True" : "False";
    case "text":
      return JSON.stringify(String(value));
    case "list":
      throw new Error("list literals have no widget — edit them in text");
  }
}

/**
 * The widget kind for an inline literal kwarg: the PORT's base type decides
 * (an Integer port writes bare integers even when the value is `24`; a
 * Number port keeps the point). For other bases (type variables, `Any`) the
 * literal's own spelling decides — `3` stays an Integer literal, `3.0` a
 * Number. `null` = no inline widget (lists, unknown shapes).
 */
export function literalKindOf(
  input: Pick<InputView, "base" | "literal" | "literal_value">,
): Exclude<LiteralKind, "slider" | "toggle" | "list"> | null {
  const value = input.literal_value;
  if (value === undefined) return null;
  switch (input.base) {
    case "Integer":
      return typeof value === "number" ? "integer" : null;
    case "Number":
      return typeof value === "number" ? "number" : null;
    case "Boolean":
      return typeof value === "boolean" ? "boolean" : null;
    case "Text":
      return typeof value === "string" ? "text" : null;
    default:
      break;
  }
  if (typeof value === "number") {
    const spelled = input.literal ?? "";
    return /[.eE]/.test(spelled) || !Number.isInteger(value) ? "number" : "integer";
  }
  if (typeof value === "boolean") return "boolean";
  if (typeof value === "string") return "text";
  return null;
}
