/**
 * The playhead as the UI displays it (docs/13 §Animation transport): the
 * store's last `TransportView`, extrapolated to now while playing — the
 * server's position advanced by the wall time since it was heard, at the
 * playback speed — and the server's own numbers while paused. One
 * interval at the display tick runs only while playing; every broadcast
 * re-anchors the extrapolation through the store. Shared by the play bar
 * (the counter, the scrubber's thumb) and the inspector's transport rows.
 */
import { useEffect, useState } from "react";
import { useCicada } from "../state/store";
import { DISPLAY_TICK_MS, nowMs, playheadAt, type Playhead, type TransportState } from "../state/transport";

export interface DisplayedTransport {
  /** The store's slice (null before the first snapshot / while the socket is down). */
  transport: TransportState | null;
  /** The playhead to show — null exactly when `transport` is. */
  playhead: Playhead | null;
}

export function usePlayhead(): DisplayedTransport {
  const transport = useCicada((s) => s.transport);
  const playing = transport?.view.playing ?? false;
  const [now, setNow] = useState(nowMs);

  useEffect(() => {
    // Re-read the clock at once for the new view (a seek while paused moves
    // the thumb without a tick), then tick only while playing.
    setNow(nowMs());
    if (!playing) return;
    const timer = window.setInterval(() => setNow(nowMs()), DISPLAY_TICK_MS);
    return () => window.clearInterval(timer);
  }, [transport, playing]);

  return { transport, playhead: transport === null ? null : playheadAt(transport, now) };
}
