/**
 * Top bar (docs/16 §Application layout): project · pipeline · engine ·
 * lease/role badge · solve state + ETA + Esc · connection · settings menu.
 * Everything here reads the store mirror; the only intents it sends are
 * `cancel` and `take_lease`.
 */
import { useEffect, useRef, useState } from "react";
import { useCicada, type DisplayMode, type Settings, type SplitPreset, type WireMode } from "../state/store";
import { useInspectorTab } from "./inspectorTab";
import { basename, summaryText, withStatusCounts } from "./format";
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
