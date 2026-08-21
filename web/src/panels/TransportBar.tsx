/**
 * The transport bar (docs/16 §Application layout; docs/13 §Animation
 * transport; docs/17 item 4): play / pause · reset · the frame counter ·
 * a frame scrubber over the primary loop · the speed menu, docked above
 * the status bar and shown only while the pipeline has a time param the
 * transport drives (`driven` non-empty). Every control is an intent the
 * server answers with a `transport` broadcast — the bar never flips its
 * own state; what it shows is the last view heard, extrapolated between
 * broadcasts (`usePlayhead`). Controls are writer-only (the lease is the
 * one arbiter of shared state): observers see the same bar, live, with
 * the controls disabled and the reason on hover.
 *
 * The scrubber: every change is a `transport_seek` — a scrub stream is a
 * stream of generations on the latest-wins loop, each painting the frame
 * it names (paused or playing). The thumb shows the sought frame at once
 * and hands back to the server's view on the broadcast that answers, so
 * a release never snaps back to where the drag began.
 */
import { useEffect, useRef, useState } from "react";
import { canWrite, useCicada, writeBlockReason } from "../state/store";
import { formatPlayhead, formatSpeed, hasTimeParams, speedChoices } from "../state/transport";
import { usePlayhead } from "./usePlayhead";
import "./panels.css";

export function TransportBar() {
  const { transport, playhead } = usePlayhead();
  const writer = useCicada(canWrite);
  const blockReason = useCicada(writeBlockReason);
  const send = useCicada((s) => s.send);
  // The frame the thumb shows while a scrub is under way (and until the
  // server's answer lands), else null = follow the view.
  const [scrub, setScrub] = useState<number | null>(null);
  const scrubbing = useRef(false);

  // A new view from the server (the seek's own broadcast, Esc, a reload)
  // takes the thumb back unless the pointer is still down.
  useEffect(() => {
    if (!scrubbing.current) setScrub(null);
  }, [transport]);

  if (transport === null || playhead === null || !hasTimeParams(transport)) return null;
  const view = transport.view;
  const frame = scrub ?? playhead.frame;
  const why = blockReason === null ? null : `${blockReason} — take the lease to drive the transport`;
  const title = (action: string) => (why === null ? action : `${action} (${why})`);

  const play = () => send({ type: view.playing ? "transport_pause" : "transport_play", payload: {} });
  const seek = (next: number) => {
    setScrub(next);
    send({ type: "transport_seek", payload: { frame: next } });
  };
  // The pointer came up: hand the thumb back now if the server's answer to
  // the last seek already landed while the pointer was down (the view
  // names the sought frame — nothing later would clear it); otherwise the
  // answer clears it when it arrives.
  const released = () => {
    scrubbing.current = false;
    setScrub((held) => (held !== null && held === view.frame ? null : held));
  };
  const drivenLabel =
    view.driven.length === 1
      ? `${view.driven[0]!.node}.${view.driven[0]!.port}`
      : `${view.driven[0]!.node}.${view.driven[0]!.port} +${view.driven.length - 1}`;
  const drivenTitle = `drives ${view.driven.map((d) => `${d.node}.${d.port} (${d.signal})`).join(", ")}`;

  return (
    <div
      className={`transportbar${view.playing ? " playing" : ""}`}
      data-testid="transport"
      data-playing={view.playing}
      data-frame={frame}
      data-frames={view.frames}
      data-speed={view.speed}
      role="toolbar"
      aria-label="transport"
    >
      <button
        type="button"
        className="tr-btn tr-play"
        data-testid="tr-play"
        aria-label={view.playing ? "pause" : "play"}
        aria-pressed={view.playing}
        title={title(view.playing ? "pause (Space)" : "play (Space)")}
        disabled={!writer}
        onClick={play}
      >
        {view.playing ? "❚❚" : "▶"}
      </button>
      <button
        type="button"
        className="tr-btn"
        data-testid="tr-reset"
        aria-label="reset"
        title={title("reset — pause and rewind to frame 0 (the headless values)")}
        disabled={!writer}
        onClick={() => send({ type: "transport_reset", payload: {} })}
      >
        ⏮
      </button>
      <span className="tr-counter mono" data-testid="tr-frame" title={`frame ${frame} of ${view.frames} · ${formatPlayhead(playhead.tMs)} of playhead`}>
        {frame} / {view.frames}
      </span>
      <input
        type="range"
        className="tr-scrub"
        data-testid="tr-scrub"
        aria-label="frame"
        min={0}
        max={Math.max(0, view.frames - 1)}
        step={1}
        value={frame}
        disabled={!writer}
        title={title("scrub — each frame is a seek")}
        onPointerDown={() => {
          scrubbing.current = true;
        }}
        onPointerUp={released}
        onPointerCancel={released}
        onChange={(event) => seek(Number(event.currentTarget.value))}
        onKeyDown={(event) => {
          // Space on the focused scrubber is the transport hotkey, not the
          // control's (a range input does nothing with it).
          if (event.key === " " || event.code === "Space") event.preventDefault();
        }}
        onKeyUp={(event) => {
          if (event.key === " " || event.code === "Space") play();
        }}
      />
      <span className="tr-time mono faint" data-testid="tr-time">
        {formatPlayhead(playhead.tMs)}
      </span>
      <select
        className="tr-speed"
        data-testid="tr-speed"
        aria-label="speed"
        title={title("playback speed")}
        value={String(view.speed)}
        disabled={!writer}
        onChange={(event) => {
          send({ type: "transport_speed", payload: { factor: Number(event.currentTarget.value) } });
          // Give the keyboard back to the transport: Space on a focused
          // select would open it.
          event.currentTarget.blur();
        }}
      >
        {speedChoices(view.speed).map((speed) => (
          <option key={speed} value={String(speed)}>
            {formatSpeed(speed)}
          </option>
        ))}
      </select>
      <span className="tr-spacer" />
      <span className="tr-driven faint" data-testid="tr-driven" title={drivenTitle}>
        drives <span className="mono">{drivenLabel}</span>
      </span>
    </div>
  );
}
