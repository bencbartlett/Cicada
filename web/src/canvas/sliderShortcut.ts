/**
 * Grasshopper's slider shortcut in search-to-place (wave 4 B4, finding U10
 * — "slider shortcuts like Grasshopper's `1<20` and `0.0<0.5<1.0`"): a
 * query of the form `A<B` or `A<B<C` is not a node name but a slider to
 * make — min `A`, max `B` (or `C`), value `A` (or `B`); negatives allowed;
 * the step is the precision typed, `10^-(the most decimals typed)`, so
 * whole numbers make GH's integer slider (step 1 on whole values — the
 * `slider` node's ports are `Number`, so the literals are spelled `1.0`
 * like every Number literal the canvas writes; there is no Integer-typed
 * slider node). `min` must be below `max` and `value` within them, else
 * the shortcut carries its `problem` and placing it is a notice. Pure;
 * `SearchBox` renders the row and sends ONE `place_node` whose `params`
 * are `sliderShortcutParams` — one op, so one undo removes the slider.
 */
import type { ParamSpec } from "../protocol/messages";
import { paramValueText } from "../state/literals";

/** One number of the grammar: an optional sign, digits, an optional fraction (`.5` allowed). */
const NUMBER = String.raw`-?(?:\d+(?:\.\d+)?|\.\d+)`;
const SHORTCUT = new RegExp(String.raw`^\s*(${NUMBER})\s*<\s*(${NUMBER})(?:\s*<\s*(${NUMBER}))?\s*$`);

export interface SliderShortcut {
  min: number;
  max: number;
  value: number;
  /** The most decimals typed across the numbers. */
  decimals: number;
  /** `10^-decimals`: the step the typed precision implies; `1` for whole numbers. */
  step: number;
  /** Every number typed whole — GH's integer slider (step 1 on whole values). */
  integer: boolean;
  /** Why the slider cannot be placed as typed (`min` not below `max`, `value` outside), or null. */
  problem: string | null;
}

/** Decimal places a typed number carries (`0.50` → 2, `7` → 0, `.5` → 1). */
function decimalsTyped(text: string): number {
  const dot = text.indexOf(".");
  return dot < 0 ? 0 : text.length - dot - 1;
}

/**
 * Parse a search query as a slider shortcut; `null` when it is not one (an
 * ordinary node search). A partial `1<` is not one either.
 */
export function parseSliderShortcut(query: string): SliderShortcut | null {
  const match = SHORTCUT.exec(query);
  if (match === null) return null;
  const a = match[1]!;
  const b = match[2]!;
  const c = match[3];
  const typed = c === undefined ? [a, b] : [a, b, c];
  const decimals = Math.max(...typed.map(decimalsTyped));
  const min = Number(a);
  const max = Number(c ?? b);
  const value = c === undefined ? min : Number(b);
  // `Number("1e-3")` is exact where `10 ** -3` is 0.0010000000000000002.
  const step = Number(`1e-${decimals}`);
  let problem: string | null = null;
  if (!(min < max)) problem = `min ${a} must be below max ${c ?? b}`;
  else if (value < min || value > max) problem = `value ${b} is outside ${a} … ${c}`;
  return { min, max, value, decimals, step, integer: decimals === 0, problem };
}

/** The slider's literals as the canvas spells Number literals (`1` → `1.0`), in the node's port order. */
export function sliderShortcutParams(shortcut: SliderShortcut): ParamSpec[] {
  const spell = (x: number) => paramValueText("slider", x);
  return [
    { port: "value", value: spell(shortcut.value) },
    { port: "min", value: spell(shortcut.min) },
    { port: "max", value: spell(shortcut.max) },
    { port: "step", value: spell(shortcut.step) },
  ];
}

/** What the search row previews: the slider this shortcut makes. */
export function sliderShortcutSummary(shortcut: SliderShortcut): { title: string; detail: string } {
  const spell = (x: number) => paramValueText("slider", x);
  const kind = shortcut.integer ? "integer slider" : "slider";
  return {
    title: `${kind} ${spell(shortcut.min)} … ${spell(shortcut.max)}`,
    detail: `value ${spell(shortcut.value)} · step ${spell(shortcut.step)}`,
  };
}
