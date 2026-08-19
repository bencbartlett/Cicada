/** Transient notices (server notices, intent errors, watcher reloads). */
import { useCicada } from "../state/store";

export function Notices() {
  const notices = useCicada((s) => s.notices);
  const dismiss = useCicada((s) => s.dismissNotice);
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
