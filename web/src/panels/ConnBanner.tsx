/**
 * The connection banner: a full-width strip under the top edge whenever the
 * socket is not open after a session was once established (a dead server
 * must never leave the app LOOKING live). It names the state — retrying in
 * N s / reconnecting — and offers "retry now"; every write affordance is
 * already disabled through `canWrite`.
 */
import { useEffect, useState } from "react";
import { retryConnectionNow } from "../state/connection";
import { useCicada } from "../state/store";
import "./panels.css";

export function ConnBanner() {
  const connection = useCicada((s) => s.connection);
  const message = useCicada((s) => s.connectionMessage);
  const reconnect = useCicada((s) => s.reconnect);
  const everConnected = useCicada((s) => s.hello !== null);
  const [now, setNow] = useState(Date.now());

  const visible = everConnected && connection !== "open";
  const waiting = visible && reconnect !== null && reconnect.nextAt !== null;

  // Count down to the next attempt (4 Hz is plenty for a whole-second readout).
  useEffect(() => {
    if (!waiting) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(timer);
  }, [waiting, reconnect?.nextAt]);

  // The slot always exists (a zero-height grid row) so the app layout never shifts.
  if (!visible) return <div className="conn-banner-slot" />;

  let text: string;
  if (connection === "closed") {
    text = "connection closed";
  } else if (reconnect !== null && reconnect.nextAt !== null) {
    const seconds = Math.max(0, Math.ceil((reconnect.nextAt - now) / 1000));
    text = `connection lost — retrying in ${seconds}s… (attempt ${reconnect.attempt})`;
  } else {
    text = "reconnecting…";
  }

  return (
    <div className="conn-banner-slot">
      <div className="conn-banner" role="alert" data-testid="conn-banner" data-connection={connection}>
        <span className="conn-banner-dot" aria-hidden />
        <strong>{text}</strong>
        {message && <span className="conn-banner-why">{message}</span>}
        <span className="conn-banner-note">read-only until the session is back — edits are disabled</span>
        {reconnect !== null && reconnect.nextAt !== null && (
          <button type="button" className="conn-banner-retry" onClick={retryConnectionNow}>
            retry now
          </button>
        )}
      </div>
    </div>
  );
}
