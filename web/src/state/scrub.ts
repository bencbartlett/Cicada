/**
 * Scrub caching, the client's pure half (docs/12 §Speculative warming,
 * docs/13 §Scrub caching, docs/16 §Sliders; v0.1 item 5 S2). The SERVER
 * computes everything that means anything — eligibility off the slider's
 * literals, the warm set off its queue — and the client renders: the buffer
 * bar under both slider widgets and the toggle (the inspector's actions,
 * the params row, the node menu). Nothing here decides whether a slider may
 * scrub-cache; `ineligible` is the server's own words, shown as the reason
 * the toggle is greyed with, and the server refuses `set_scrub` with the
 * same words if a client sends it anyway.
 *
 * Two things are computed here, both display-only: the MERGE of a slider's
 * `param.scrub` (every snapshot / delta) with the `scrub_progress` overlay
 * the store keeps between deltas, and the CURRENT position's index — which
 * notch of the bar the thumb is on — from the widget's own snap rule
 * (`grid.ts::snapToStep`, the same lattice the server warms: `min + k ×
 * step`). No React, no store — unit-tested.
 */
import type { NodeView, ScrubProgressPayload, ScrubView } from "../protocol/messages";

/**
 * The slider's scrub view as the app shows it: the view the last snapshot /
 * delta carried, with the fields a later `scrub_progress` moved (`warmed`,
 * `warming`, `bytes`, `capped`) laid over it. `on`, `positions` and
 * `ineligible` move only with the text, which the delta carries, so they
 * are always the view's. Undefined when the param carries no scrub view
 * (not a slider — toggles, constants).
 */
export function mergeScrub(
  view: ScrubView | undefined,
  progress: ScrubProgressPayload | undefined,
): ScrubView | undefined {
  if (view === undefined) return undefined;
  if (progress === undefined) return view;
  const merged: ScrubView = {
    ...view,
    warmed: progress.warmed,
    warming: progress.warming,
    bytes: progress.bytes,
  };
  if (progress.capped === true) merged.capped = true;
  else delete merged.capped;
  return merged;
}

/** Does the bar have anything to draw? Opted in, and eligible (positions > 0). */
export function showsScrubBar(scrub: ScrubView | undefined): scrub is ScrubView {
  return scrub !== undefined && scrub.on && scrub.positions > 0;
}

/**
 * The position index the thumb sits on: `round((value − min) / step)`
 * clamped into `0 … positions − 1` — the nearest notch, as the widget
 * snaps a drag (`snapToStep`) and as the server picks the warming's start
 * (`Positions::nearest`). A value off the grid (a hand-written literal
 * between notches) marks its nearest notch.
 */
export function currentPosition(value: number, min: number, step: number, positions: number): number {
  if (positions <= 0) return 0;
  if (!(step > 0) || !Number.isFinite(value)) return 0;
  const k = Math.round((value - min) / step);
  if (!Number.isFinite(k) || k <= 0) return 0;
  return Math.min(k, positions - 1);
}

/** Bytes → `1.2 KB` / `3.4 MB` (the bar's tooltip; the same spelling as the inspector's cache line). */
function bytesText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

/**
 * The bar's tooltip: `scrub cache · 7 / 19 positions warm · warming…`,
 * then what the warming stored; `capped at the 256 MiB budget` when the
 * server stopped it (the warm positions stay).
 */
export function scrubBarTitle(scrub: ScrubView): string {
  const parts = [`scrub cache · ${scrub.warmed.length} / ${scrub.positions} positions warm`];
  if (scrub.capped === true) parts.push("capped at the 256 MiB budget — the warm positions stay");
  else if (scrub.warming) parts.push("warming while the app is idle…");
  else if (scrub.warmed.length === scrub.positions) parts.push("every position is a cache read");
  if (scrub.bytes > 0) parts.push(`${bytesText(scrub.bytes)} stored`);
  return parts.join(" · ");
}

/** What the toggle shows for one slider — the same on every surface. */
export interface ScrubToggleState {
  /** The text says `scrub=True`. */
  on: boolean;
  /** The click is refused beforehand: the server says the slider cannot scrub-cache and it is not on. */
  disabled: boolean;
  /** The server's reason (`scrub.ineligible`), when there is one — shown greyed, whether on or off. */
  reason: string | null;
  /** The menu / action label: `scrub-cache this slider` or `stop scrub-caching`. */
  label: string;
  /** The short hint beside the label: the reason, else the position count. */
  hint: string;
  /** The full tooltip. */
  title: string;
  /** The intent the click sends (`set_scrub {node, on}`'s `on`). */
  next: boolean;
}

/**
 * The toggle's state for a node, or null when the node is no slider with a
 * scrub view (the toggle is not offered). The rule is the server's and
 * arrives on the view: `ineligible` greys the toggle with its reason while
 * the slider is OFF (turning it on would be refused with the same words);
 * a slider that is ON and ineligible — a hand-written `scrub=True` the
 * server warms nothing for — keeps the toggle live, since turning it off
 * is always allowed (docs/13), and the hint says nothing is warmed.
 *
 * `progress` is the slider's `scrub_progress` overlay (`store.scrubProgress`,
 * read with `scrubProgressFor`) and is REQUIRED, undefined spelled out: the
 * state is computed off the MERGED view — the view the bar draws — so the
 * warm count in the hint and in the tooltip is the bar's. The review of
 * 2026-08-24 found the first cut reading the raw graph view on two
 * surfaces: the overlay never writes the graph (docs/13), so the node menu
 * said `0 / 19 positions warm` under a full bar. An optional parameter
 * would let the next surface forget again.
 */
export function scrubToggle(
  view: Pick<NodeView, "func" | "param"> | undefined,
  progress: ScrubProgressPayload | undefined,
): ScrubToggleState | null {
  if (view === undefined || view.func !== "slider") return null;
  const scrub = mergeScrub(view.param?.scrub, progress);
  if (scrub === undefined) return null;
  const reason = scrub.ineligible ?? null;
  const on = scrub.on;
  const disabled = !on && reason !== null;
  const label = on ? "stop scrub-caching" : "scrub-cache this slider";
  const positions = `${scrub.positions} position${scrub.positions === 1 ? "" : "s"}`;
  let hint: string;
  let title: string;
  if (reason !== null) {
    hint = reason;
    title = on
      ? `${reason} — the text says scrub=True but nothing is warmed; turning it off removes the kwarg`
      : `${reason} — scrub caching needs literal min, max and step and a bounded position count`;
  } else if (on) {
    // The tooltip leads with the count, so the switch and the pill (which
    // spell no hint) show the warm set on hover — the bar's number.
    hint = `${scrub.warmed.length} / ${positions} warm`;
    title = `${hint} — pre-solved while the app is idle (scrub=True in the text); turning it off removes the kwarg`;
  } else {
    hint = positions;
    title = `pre-solve the ${positions} of this slider while the app is idle, so dragging it is a cache read (writes scrub=True into the text)`;
  }
  return { on, disabled, reason, label, hint, title, next: !on };
}
