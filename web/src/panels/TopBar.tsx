/**
 * Top bar (docs/16 §Application layout): project · pipeline · git chip
 * (branch / detached / no repo · dirty count; click → the Git tab) ·
 * engine · lease/role badge · undo/redo · solve state + ETA + Esc ·
 * connection · settings menu. Everything here reads the store mirror; the
 * only intents it sends are `undo`, `redo`, `cancel` and `take_lease`.
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
import { basename, summaryText, withStatusCounts } from "./format";
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
  const send = useCicada((s) => s.send);

  const project = hello === null ? "…" : basename(hello.project);
  const clients = lease.clients.length;
  // Nodes excluded by diagnostics never enter the solve, so the summary's
  // red/blocked counts miss them; the per-node statuses do not.
  const shown = withStatusCounts(summary, statuses);
  const solveClass = shown.running
    ? "running"
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
      <span className="tb-sep">·</span>
      <span className="tb-item faint" title="engine">
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

      <span className={`tb-solve ${solveClass}`} data-testid="tb-solve" title="solve state">
        {summary.running && (
          <span className={`tb-progress${summary.eta_rough ? " rough" : ""}`} aria-hidden>
            <i style={{ width: `${Math.round(fraction * 100)}%` }} />
          </span>
        )}
        <span>{summaryText(shown)}</span>
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
          <label>ribbon collapsed</label>
          <input
            type="checkbox"
            checked={settings.ribbonCollapsed}
            onChange={(e) => updateSettings({ ribbonCollapsed: e.target.checked })}
          />
          <span className="menu-h">canvas</span>
          <label>wires</label>
          {seg("wireMode", WIRE_MODES)}
          <span className="menu-h">viewport</span>
          <label>display</label>
          {seg("displayMode", DISPLAY_MODES)}
          <label>navigation</label>
          {seg("navigation", [
            ["rhino", "rhino"],
            ["blender", "blender"],
          ])}
        </div>
      )}
    </span>
  );
}
