/**
 * Status bar (docs/16 §Application layout): diagnostics count · red/blocked
 * node counts · generation + last delta · connection · frames received ·
 * Esc hint while running · notices count.
 */
import { useEffect, useState } from "react";
import { useCicada } from "../state/store";
import { readFrameCounters } from "./debugHandle";
import { formatBytes } from "./format";
import "./panels.css";

export function StatusBar() {
  const diagnostics = useCicada((s) => s.graph.diagnostics.length);
  const statuses = useCicada((s) => s.statuses);
  const summary = useCicada((s) => s.summary);
  const lastDeltaLabel = useCicada((s) => s.lastDeltaLabel);
  const connection = useCicada((s) => s.connection);
  const notices = useCicada((s) => s.notices.length);
  const clearSelection = useCicada((s) => s.clearSelection);
  const [frames, setFrames] = useState(readFrameCounters());

  useEffect(() => {
    const tick = () => setFrames(readFrameCounters());
    tick();
    const timer = window.setInterval(tick, 500);
    return () => window.clearInterval(timer);
  }, [summary.generation, summary.running]);

  let red = 0;
  let blocked = 0;
  let running = 0;
  for (const s of Object.values(statuses)) {
    if (s.state === "red") red += 1;
    else if (s.state === "blocked") blocked += 1;
    else if (s.state === "running" || s.state === "queued") running += 1;
  }

  return (
    <footer className="statusbar" data-testid="statusbar">
      <span
        className="sb-item clickable"
        title="diagnostics — click to show the pipeline overview"
        onClick={clearSelection}
        data-testid="sb-diagnostics"
      >
        <span className={`badge${diagnostics > 0 ? " red" : ""}`}>{diagnostics}</span> diagnostics
      </span>
      <span className="sb-item" title="node states" data-testid="sb-nodes">
        {red > 0 && <span className="badge red">{red} red</span>}
        {blocked > 0 && <span className="badge warn">{blocked} blocked</span>}
        {running > 0 && <span className="badge accent">{running} running</span>}
        {red === 0 && blocked === 0 && running === 0 && <span className="faint">all nodes green</span>}
      </span>
      <span className="sb-item" title="solve generation · last delta" data-testid="sb-generation">
        gen {summary.generation}
        {lastDeltaLabel && <span className="faint">· {lastDeltaLabel}</span>}
      </span>
      <span className="sb-spacer" />
      {summary.running && (
        <span className="sb-item" style={{ color: "var(--running)" }}>
          solving… <span className="sb-kbd">Esc</span> cancels
        </span>
      )}
      <span className="sb-item" title="binary frames received" data-testid="sb-frames">
        {frames === null ? "frames —" : `${frames.received} frames · ${formatBytes(frames.bytes)}`}
      </span>
      <span className="sb-item" title="connection" data-testid="sb-connection">
        <span
          className="state-dot"
          style={{
            background:
              connection === "open"
                ? "var(--ok)"
                : connection === "connecting"
                  ? "var(--warn)"
                  : connection === "idle"
                    ? "var(--fg-faint)"
                    : "var(--error)",
          }}
        />
        {connection}
      </span>
      <span className="sb-item" title="notices">
        <span className={`badge${notices > 0 ? " accent" : ""}`}>{notices}</span> notices
      </span>
    </footer>
  );
}
