/**
 * The wrapper the viewport lives in, whatever the mode (docs/16 §Viewport
 * conventions; wave 5 V1): ONE element keyed into the work area so the
 * `Viewport` inside it — and its three.js scene — never remounts when the
 * mode changes. `split`: a grid pane like the canvas's. `floating`: an
 * absolutely placed panel over the canvas — a title strip drags it, a
 * corner resizes it (pointer capture; the rect written to the DOM while
 * the pointer moves and persisted to the settings on release), the
 * stored rect clamped into the work area every time the area is measured
 * (a window resize re-clamps). `window`: the element itself is in the PiP
 * document (`windowMode.ts`), so the wrapper is parked out of layout and
 * the `ViewportPlaceholder` says where the viewport went.
 */
import { useEffect, useRef, useState, type RefObject } from "react";
import { useCicada } from "../state/store";
import { clampFloating, defaultFloating, moveFloating, resizeFloating, type FloatingRect, type Size, type ViewportMode } from "./modes";
import { Viewport } from "./Viewport";
import { chooseViewportMode } from "./windowMode";

interface Drag {
  kind: "move" | "resize";
  startX: number;
  startY: number;
  from: FloatingRect;
  /** The last rect written to the DOM during this drag (persisted on release). */
  live: FloatingRect;
}

export function ViewportFrame({ mode, areaRef }: { mode: ViewportMode; areaRef: RefObject<HTMLDivElement> }) {
  const stored = useCicada((s) => s.settings.floatingViewport);
  const updateSettings = useCicada((s) => s.updateSettings);
  const frameRef = useRef<HTMLDivElement>(null);
  const drag = useRef<Drag | null>(null);
  const [area, setArea] = useState<Size | null>(null);

  // Measure the work area while floating: the stored rect is clamped into
  // it at every size, so a shrunken window never hides the panel.
  useEffect(() => {
    if (mode !== "floating") return;
    const el = areaRef.current;
    if (el === null) return;
    const measure = () => setArea({ width: el.clientWidth, height: el.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [mode, areaRef]);

  const rect = mode === "floating" && area !== null ? clampFloating(stored ?? defaultFloating(area), area) : null;

  const apply = (next: FloatingRect) => {
    const el = frameRef.current;
    if (el === null) return;
    el.style.left = `${next.x}px`;
    el.style.top = `${next.y}px`;
    el.style.width = `${next.width}px`;
    el.style.height = `${next.height}px`;
  };
  const onDown = (kind: Drag["kind"]) => (event: React.PointerEvent<HTMLDivElement>) => {
    if (rect === null || event.button !== 0) return;
    drag.current = { kind, startX: event.clientX, startY: event.clientY, from: rect, live: rect };
    const target = event.currentTarget;
    if (typeof target.setPointerCapture === "function") target.setPointerCapture(event.pointerId);
    event.preventDefault();
  };
  const onMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (d === null || area === null) return;
    const dx = event.clientX - d.startX;
    const dy = event.clientY - d.startY;
    d.live = d.kind === "move" ? moveFloating(d.from, dx, dy, area) : resizeFloating(d.from, dx, dy, area);
    apply(d.live);
  };
  const onUp = () => {
    const d = drag.current;
    if (d === null) return;
    drag.current = null;
    updateSettings({ floatingViewport: d.live });
  };

  const className = mode === "split" ? "pane" : mode === "floating" ? "viewport-float" : "viewport-parked";
  const style = rect === null ? undefined : { left: rect.x, top: rect.y, width: rect.width, height: rect.height };
  return (
    <div className={className} data-testid="viewport-pane" data-mode={mode} ref={frameRef} style={style}>
      {mode === "floating" && (
        <div
          className="viewport-float-title"
          data-testid="viewport-float-title"
          title="drag to move the viewport · the lower-right corner resizes it"
          onPointerDown={onDown("move")}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onUp}
        >
          viewport
        </div>
      )}
      <Viewport />
      {mode === "floating" && (
        <div
          className="viewport-float-corner"
          data-testid="viewport-float-corner"
          title="drag to resize the viewport (at least 240 × 160)"
          onPointerDown={onDown("resize")}
          onPointerMove={onMove}
          onPointerUp={onUp}
          onPointerCancel={onUp}
        />
      )}
    </div>
  );
}

/** Shown in the work area while the viewport is in its own window: one click brings it back (to `split`). */
export function ViewportPlaceholder() {
  return (
    <button
      type="button"
      className="viewport-placeholder"
      data-testid="viewport-placeholder"
      title="the viewport is in its picture-in-picture window; click to return it to its pane (closing that window does the same)"
      onClick={() => chooseViewportMode("split")}
    >
      the viewport is in its own window — click to bring it back
    </button>
  );
}
