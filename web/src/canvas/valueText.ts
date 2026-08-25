/**
 * The node face's value text (docs/16 LOD table; finding U23, 2026-08-25):
 * every decimal number in a value summary is shown to FOUR significant
 * figures on the canvas — `6.283185307179586` reads `6.283`, a point
 * `(0.1234567, 2.5, -3)` reads `(0.1235, 2.5, -3)` — while the inspector
 * keeps the server's full rendering (`ValueSummaryView`). Integers — counts,
 * indices, `×1000` — are never touched: only tokens with a fractional part
 * or an exponent are numbers the face rounds.
 */

/** Significant figures on the node face. */
export const FACE_SIG_FIGS = 4;

/** A decimal with a fractional part or an exponent (never a bare integer). */
const DECIMAL = /-?\d+\.\d+(?:[eE][+-]?\d+)?|-?\d+[eE][+-]?\d+/g;

/** One number to `sigFigs` significant figures, trailing zeros dropped (`3.000` → `3`). */
export function compactNumber(x: number, sigFigs = FACE_SIG_FIGS): string {
  return Number.isFinite(x) ? String(Number(x.toPrecision(sigFigs))) : String(x);
}

/** Every decimal in `text` compacted; everything else verbatim. */
export function compactValueText(text: string): string {
  return text.replace(DECIMAL, (token) => compactNumber(Number(token)));
}
