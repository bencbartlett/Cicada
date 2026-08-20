/**
 * One-per-frame `param_preview` sender + immediate `set_param` commit, per
 * widget (docs/10 §3: previews stream at most once per animation frame,
 * latest wins; a commit cancels any pending preview — so does `cancel`,
 * for a release that writes nothing: a tick sent AFTER the `end_drag` would
 * start a fresh server-side drag on the committed value, which is cold
 * while its own solve is still running, and earn a pending badge nothing
 * would take down). Shared by the slider, the scalar literal editors, and
 * therefore every place a literal is edited.
 */
import { useCallback, useEffect, useRef } from "react";
import { sendWrite } from "./flow";

export function useParamSender(node: string, port: string | null) {
  const pending = useRef<string | null>(null);
  const raf = useRef(0);
  const lastSent = useRef<string | null>(null);

  useEffect(
    () => () => {
      if (raf.current !== 0) cancelAnimationFrame(raf.current);
    },
    [],
  );

  const preview = useCallback(
    (value: string) => {
      pending.current = value;
      if (raf.current !== 0) return;
      raf.current = requestAnimationFrame(() => {
        raf.current = 0;
        const text = pending.current;
        pending.current = null;
        if (text === null || text === lastSent.current) return;
        lastSent.current = text;
        sendWrite({ type: "param_preview", payload: { node, port, value: text } });
      });
    },
    [node, port],
  );

  /** Drop the queued preview tick, if any, and forget the last sent value. */
  const cancel = useCallback(() => {
    if (raf.current !== 0) {
      cancelAnimationFrame(raf.current);
      raf.current = 0;
    }
    pending.current = null;
    lastSent.current = null;
  }, []);

  const commit = useCallback(
    (value: string) => {
      cancel();
      sendWrite({ type: "set_param", payload: { node, port, value } });
    },
    [cancel, node, port],
  );

  return { preview, commit, cancel };
}
