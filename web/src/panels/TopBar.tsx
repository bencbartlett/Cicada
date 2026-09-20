/**
 * Top bar (docs/16 §Application layout): project · pipeline · git chip
 * (branch / detached / no repo · dirty count; click → the Git tab) ·
 * engine · lease/role badge · undo/redo · solve chip (`Solving gen N` →
 * `painting…` → `gen N · solve · display`, a spinner while either runs,
 * the counts in the hover) · the `profile` button (→ the profiler tab) ·
 * the caches indicator (display cache bytes / budget · meshes; warn tone
 * while over budget or thrashing; click → the profiler's caches section)
 * · connection · settings menu (with the viewport mode, the second-monitor
 * pop-out and the display cache size). Everything here reads the store
 * mirror; the intents it sends are `undo`, `redo`, `cancel`, `take_lease`
 * and `set_display_cache`.
 */
import { useEffect, useRef, useState } from "react";
import {
  canWrite,
  useCicada,
  type DisplayMode,
  type Settings,
  type SplitPreset,
  type WireMode,
} from "../state/store";
import { VIEWPORT_MODES } from "../viewport/modes";
import { popOutViewport } from "../viewport/popout";
import { chooseViewportMode } from "../viewport/windowMode";
import { DisplayCachePicker } from "./DisplayCachePicker";
import { FileMenu } from "./FileMenu";
import { basename, cachesText, cachesTitle, currentPass, summaryText, summaryTitle, withStatusCounts } from "./format";
import { gitChip } from "./gitFormat";
import { useInspectorTab } from "./inspectorTab";
import "./panels.css";

export function TopBar() {
  const hello = useCicada((s) => s.hello);
  const pipeline = useCicada((s) => s.pipeline);
  const role = useCicada((s) => s.role);
  const lease = useCicada((s) => s.lease);
  const summary = useCicada((s) => s.summary);
  const statuses = useCicada((s) => s.statuses);
  const connection = useCicada((s) => s.connection);
  const connectionMessage = useCicada((s) => s.connectionMessage);
  const display = useCicada((s) => s.display);
  const send = useCicada((s) => s.send);

  const project = hello === null ? "…" : basename(hello.project);
  const clients = lease.clients.length;
  // Nodes excluded by diagnostics never enter the solve, so the summary's
  // red/blocked counts miss them; the per-node statuses do not.
  const shown = withStatusCounts(summary, statuses);
  // This generation's display pass is in flight (docs/13 §The display
  // edge): between its `display_begin` and its `display_end` (a pass for a
  // newer generation than the summary names counts — its begin can
  // precede the generation's first status).
  const painting = !summary.running && currentPass(summary, display)?.phase === "painting";
  const solveClass = shown.running
    ? "running"
    : painting
      ? "painting"
      : shown.cancelled
        ? "cancelled"
        : shown.red > 0
          ? "red"
          : shown.blocked > 0
            ? "blocked"
            : "";
  // Cost-weighted progress from what the summary carries: done / (done + pending).
  const done = summary.computed + summary.cached;
  const fraction = done + summary.pending > 0 ? done / (done + summary.pending) : 0;

  return (
    <header className="topbar" data-testid="topbar">
      <span className="tb-brand">Cicada</span>
      <FileMenu />
      <span className="tb-item" title={hello?.project ?? "project"}>
        <span className="faint">project</span>
        <span className="mono" data-testid="tb-project">
          {project}
        </span>
      </span>
      <span className="tb-sep">·</span>
      <span className="tb-item" title="pipeline">
        <span className="mono" data-testid="tb-pipeline">
          {hello?.pipeline ?? pipeline}
        </span>
      </span>
      <span className="tb-sep">·</span>
      <GitChip />
      <span className="tb-sep tb-engine">·</span>
      <span className="tb-item faint tb-engine" title="engine">
        {hello?.engine ?? "engine…"}
        {hello !== null && <span className="faint">· protocol {hello.protocol}</span>}
      </span>
      <span className="tb-sep">·</span>
      <span className="tb-item" data-testid="tb-role">
        <span
          className={`badge ${role === "writer" && connection === "open" ? "accent" : "warn"}`}
          title={
            connection !== "open"
              ? "not connected — read-only until the session is back"
              : role === "writer"
                ? "you hold the write lease"
                : "another client holds the write lease — you observe"
          }
        >
          {connection !== "open" ? "read-only (offline)" : role === "writer" ? "writer" : "read-only observer"}
        </span>
        <span className="faint" title="connected clients">
          {clients} connected
        </span>
        {role === "observer" && connection === "open" && (
          <button
            className="tb-esc"
            title="take the write lease"
            onClick={() => send({ type: "take_lease", payload: {} })}
          >
            take lease
          </button>
        )}
      </span>

      <span className="tb-sep">·</span>
      <HistoryButtons />

      <span className="tb-spacer" />

      <span
        className={`tb-solve ${solveClass}`}
        data-testid="tb-solve"
        data-phase={summary.running ? "solving" : painting ? "painting" : "idle"}
        title={summaryTitle(shown, display)}
      >
        {(summary.running || painting) && <i className="tb-spin" aria-hidden data-testid="tb-solve-spinner" />}
        {summary.running && (
          <span className={`tb-progress${summary.eta_rough ? " rough" : ""}`} aria-hidden>
            <i style={{ width: `${Math.round(fraction * 100)}%` }} />
          </span>
        )}
        <span className="tb-chip-text" data-testid="tb-solve-text">
          {summaryText(shown, display)}
        </span>
        {summary.running && (
          <button
            className="tb-esc"
            title="cancel the running solve (Esc)"
            onClick={() => send({ type: "cancel", payload: {} })}
          >
            Esc · cancel
          </button>
        )}
      </span>

      <ProfileButton />
      <CachesChip />

      <span className={`tb-conn ${connection}`} data-testid="tb-conn" title={connectionMessage}>
        <i />
        <span>{connection}</span>
        {connectionMessage && connection !== "open" && (
          <span className="faint">— {connectionMessage}</span>
        )}
      </span>

      <SettingsMenu />
    </header>
  );
}

/**
 * The profiler's button (docs/16 §Inspector contents; v0.1 wave 5 P1):
 * beside the solve chip, opens the Profile tab — the last complete
 * generation itemised. The tab is what the chip's hover summarises.
 */
function ProfileButton() {
  const tab = useInspectorTab((s) => s.tab);
  const openProfile = useInspectorTab((s) => s.openProfile);
  return (
    <button
      className={`tb-esc tb-profile${tab === "profile" ? " active" : ""}`}
      title="the profiler: the last complete generation's phases, every node's cost, what the pass drew, the caches (Esc closes)"
      aria-label="open the profiler"
      aria-pressed={tab === "profile"}
      onClick={() => openProfile()}
      data-testid="tb-profile"
    >
      profile
    </button>
  );
}

/**
 * The caches indicator (docs/16 §Status and progress language; the D1
 * contract): `cache 612M / 1G · 1,397 meshes` from the session's `caches`
 * view, in the warn tone while the display cache is over budget or
 * thrashing; the full breakdown in the hover, and a click opens the
 * profiler's caches section (P1 re-targeted it from D1's breakdown panel).
 */
function CachesChip() {
  const caches = useCicada((s) => s.caches);
  const openProfile = useInspectorTab((s) => s.openProfile);
  if (caches === null) return null;
  const warn = caches.display.over_budget || caches.display.thrash;
  return (
    <span className="tb-caches-wrap">
      <button
        className={`tb-caches${warn ? " warn" : ""}`}
        title={`${cachesTitle(caches)}\nclick: the profiler's caches section`}
        aria-label={`caches: ${cachesText(caches)}${warn ? " — attention" : ""} — open the profiler's caches section`}
        onClick={() => openProfile("caches")}
        data-testid="tb-caches"
        data-warn={warn}
        data-over-budget={caches.display.over_budget}
        data-thrash={caches.display.thrash}
      >
        <span className="mono tb-chip-text" data-testid="tb-caches-text">
          {cachesText(caches)}
        </span>
      </button>
    </span>
  );
}

/**
 * The git chip (doc 10 §Git integration's status strip, slice 1): the
 * branch — or `detached @short`, `no repo`, `git not found` — with the
 * dirty-file count of this pipeline's commit scope, plus the facts worth a
 * glance (ahead/behind, locked, an operation in progress). The tooltip
 * carries the rest; a click opens the Git tab. Wording: `gitFormat.ts`.
 */
function GitChip() {
  const git = useCicada((s) => s.git);
  const setTab = useInspectorTab((s) => s.setTab);
  const chip = gitChip(git);
  const dirtyText = chip.dirty === null ? null : chip.dirty === 0 ? "clean" : `${chip.dirty} dirty`;
  return (
    <button
      className={`tb-git${chip.tone ? ` ${chip.tone}` : ""}${chip.dirty !== null && chip.dirty > 0 ? " dirty" : ""}`}
      title={chip.title}
      aria-label={`git: ${chip.label}${dirtyText ? `, ${dirtyText}` : ""} — open the Git tab`}
      onClick={() => setTab("git")}
      data-testid="tb-git"
      data-kind={git.status?.state.kind ?? (git.error !== null ? "error" : "loading")}
      data-stale={chip.stale}
    >
      <span className="tb-git-glyph" aria-hidden>
        ⎇
      </span>
      <span className="mono" data-testid="tb-git-branch">
        {chip.label}
      </span>
      {chip.notes.map((note) => (
        <span className="tb-git-note" key={note}>
          {note}
        </span>
      ))}
      {dirtyText !== null && (
        <span
          className={`tb-git-dirty${chip.stale ? " stale" : ""}`}
          data-testid="tb-git-dirty"
          title={chip.stale ? "the last read's count — an edit landed since, re-reading now" : undefined}
        >
          {dirtyText}
        </span>
      )}
    </button>
  );
}

/**
 * Undo / redo (docs/13 §Undo/redo): the mirror's `history` says what each
 * button would do (its tooltip is the op's label) and whether there is
 * anything to do; both are writes, so they also need `canWrite`. The
 * server stays the authority — a click sends the intent, the delta (or a
 * `nothing_to_*` refusal) answers.
 */
function HistoryButtons() {
  const history = useCicada((s) => s.history);
  const writer = useCicada(canWrite);
  const send = useCicada((s) => s.send);
  const gate = writer ? "" : " — read-only";
  const undoTitle = history.can_undo
    ? `undo: ${history.undo_label ?? "last op"} (Ctrl+Z)${gate}`
    : "nothing to undo (Ctrl+Z)";
  const redoTitle = history.can_redo
    ? `redo: ${history.redo_label ?? "last undone op"} (Ctrl+Shift+Z / Ctrl+Y)${gate}`
    : "nothing to redo (Ctrl+Shift+Z / Ctrl+Y)";
  return (
    <span className="tb-item tb-history" data-testid="tb-history" title={`${history.depth} undoable`}>
      <button
        className="tb-esc"
        title={undoTitle}
        aria-label={undoTitle}
        disabled={!writer || !history.can_undo}
        onClick={() => send({ type: "undo", payload: {} })}
        data-testid="tb-undo"
      >
        ↶ undo
      </button>
      <button
        className="tb-esc"
        title={redoTitle}
        aria-label={redoTitle}
        disabled={!writer || !history.can_redo}
        onClick={() => send({ type: "redo", payload: {} })}
        data-testid="tb-redo"
      >
        ↷ redo
      </button>
    </span>
  );
}

const SPLIT_LABELS: [SplitPreset, string][] = [
  ["canvas", "canvas"],
  ["even", "50 · 50"],
  ["viewport", "viewport"],
];
const WIRE_MODES: [WireMode, string][] = [
  ["spline", "spline"],
  ["trace", "trace"],
];
const DISPLAY_MODES: [DisplayMode, string][] = [
  ["shaded_edges", "shaded + edges"],
  ["shaded", "shaded"],
  ["wireframe", "wireframe"],
];

function SettingsMenu() {
  const settings = useCicada((s) => s.settings);
  const updateSettings = useCicada((s) => s.updateSettings);
  const setTab = useInspectorTab((s) => s.setTab);
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: PointerEvent) => {
      if (wrapRef.current !== null && !wrapRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const seg = <K extends keyof Settings>(key: K, options: [Settings[K], string][]) => (
    <span className="seg" role="radiogroup">
      {options.map(([value, label]) => (
        <button
          key={String(value)}
          className={settings[key] === value ? "active" : ""}
          role="radio"
          aria-checked={settings[key] === value}
          onClick={() => updateSettings({ [key]: value } as Partial<Settings>)}
        >
          {label}
        </button>
      ))}
    </span>
  );

  return (
    <span className="tb-menu-wrap" ref={wrapRef}>
      <button
        className={`tb-esc${open ? " active" : ""}`}
        title="settings (per-user, never project state)"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        data-testid="tb-settings"
      >
        ⚙ settings
      </button>
      {open && (
        <div className="tb-menu" role="dialog" aria-label="settings" data-no-hotkeys>
          <span className="menu-h">appearance</span>
          <label>theme</label>
          {seg("theme", [
            ["dark", "dark"],
            ["light", "light"],
          ])}
          <span className="menu-h">layout</span>
          <label>split</label>
          {seg("split", SPLIT_LABELS)}
          <label>swap panes</label>
          <input
            type="checkbox"
            data-testid="settings-swap"
            checked={settings.swap}
            onChange={(e) => updateSettings({ swap: e.target.checked })}
          />
          <label>text panel</label>
          <input
            type="checkbox"
            checked={settings.textPanel}
            onChange={(e) => {
              updateSettings({ textPanel: e.target.checked });
              setTab(e.target.checked ? "text" : "inspect");
            }}
          />
          <span className="menu-h">canvas</span>
          <label>wires</label>
          {seg("wireMode", WIRE_MODES)}
          <span className="menu-h">viewport</span>
          <label title="split: its pane · floating: a panel over the canvas · window: a picture-in-picture window (docs/16 §Viewport conventions)">
            mode
          </label>
          <span className="seg" role="radiogroup" aria-label="viewport mode" data-testid="settings-viewport-mode">
            {VIEWPORT_MODES.map((mode) => (
              <button
                key={mode}
                className={settings.viewportMode === mode ? "active" : ""}
                role="radio"
                aria-checked={settings.viewportMode === mode}
                data-testid={`settings-viewport-mode-${mode}`}
                onClick={() => chooseViewportMode(mode)}
              >
                {mode}
              </button>
            ))}
          </span>
          <label title="a second window on this pipeline's display set for another monitor — a read-only observer with its own camera; the fallback of the window mode where the browser has no picture-in-picture window">
            second monitor
          </label>
          <button
            className="tb-esc"
            data-testid="viewport-popout"
            title="pop the viewport out into a separate read-only window (a declared observer of this pipeline, its own camera)"
            onClick={() => popOutViewport(window)}
          >
            pop out
          </button>
          <label>display</label>
          {seg("displayMode", DISPLAY_MODES)}
          <label>navigation</label>
          {seg("navigation", [
            ["rhino", "rhino"],
            ["blender", "blender"],
          ])}
          <span className="menu-h">display cache</span>
          <label
            title="the solid display meshes the viewport redraws from (docs/12 §Display cache) — a per-user choice the lease holder applies to the session on every connect"
          >
            solid meshes
          </label>
          <DisplayCachePicker />
        </div>
      )}
    </span>
  );
}
