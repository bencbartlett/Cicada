/**
 * Transient notices (server notices, intent errors, watcher reloads).
 * Info fades after a few seconds, warnings a little later; errors stay
 * until dismissed (probe friction: toasts stacked over the viewport forever).
 */
import { useEffect } from "react";
import { useCicada } from "../state/store";

export const NOTICE_TTL_MS = { info: 6000, warning: 15000 } as const;

export function Notices() {
  const notices = useCicada((s) => s.notices);
  const dismiss = useCicada((s) => s.dismissNotice);
  useEffect(() => {
    const timers = notices
      .filter((n) => n.level !== "error")
      .map((n) => {
        const ttl = NOTICE_TTL_MS[n.level === "info" ? "info" : "warning"];
        const left = Math.max(0, n.at + ttl - Date.now());
        return setTimeout(() => dismiss(n.id), left);
      });
    return () => {
      for (const t of timers) clearTimeout(t);
    };
  }, [notices, dismiss]);
  if (notices.length === 0) return null;
  return (
    <div className="notices" data-testid="notices">
      {notices.slice(-4).map((n) => (
        <div key={n.id} className={`notice ${n.level}`} role="status">
          <span style={{ flex: 1 }}>{n.message}</span>
          <button onClick={() => dismiss(n.id)} aria-label="dismiss">
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
