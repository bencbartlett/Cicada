/**
 * The docked window (docs/16 §Application layout): top bar · menu bar ·
 * canvas/viewport split (resizable, presets, swap) · inspector · transport
 * bar (only with time params) · status bar.
 * Regions are components owned by their folders; this file only arranges
 * them and applies per-user settings (theme, split, the viewport mode).
 */
import { useRef } from "react";
import { Canvas } from "./canvas/Canvas";
import { useKeyboard } from "./keyboard";
import { CommitDialog } from "./panels/CommitDialog";
import { ConnBanner } from "./panels/ConnBanner";
import { Inspector } from "./panels/Inspector";
import { MenuBar } from "./panels/MenuBar";
import { Notices } from "./panels/Notices";
import { OpenDialog } from "./panels/OpenDialog";
import { StatusBar } from "./panels/StatusBar";
import { TopBar } from "./panels/TopBar";
import { TransportBar } from "./panels/TransportBar";
import { useCicada } from "./state/store";
import { ViewportFrame, ViewportPlaceholder } from "./viewport/ViewportFrame";

const SPLITS: Record<string, [string, string]> = {
  canvas: ["3fr", "2fr"],
  even: ["1fr", "1fr"],
  viewport: ["2fr", "3fr"],
};

export function App() {
  const settings = useCicada((s) => s.settings);
  const updateSettings = useCicada((s) => s.updateSettings);
  const pipeline = useCicada((s) => s.pipeline);
  const workRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  useKeyboard();

  // The theme on the document is `Root`'s (every screen shares it).
  const [a, b] = SPLITS[settings.split] ?? SPLITS.canvas!;
  const style = { "--split-a": a, "--split-b": b } as React.CSSProperties;

  // Drag the splitter: converts to a custom fr pair (kept until a preset
  // is chosen again).
  const onSplitterDown = (event: React.PointerEvent) => {
    dragging.current = true;
    (event.target as HTMLElement).setPointerCapture(event.pointerId);
  };
  const onSplitterMove = (event: React.PointerEvent) => {
    if (!dragging.current || workRef.current === null) return;
    const rect = workRef.current.getBoundingClientRect();
    const t = Math.min(0.9, Math.max(0.1, (event.clientY - rect.top) / rect.height));
    workRef.current.style.setProperty("--split-a", `${t}fr`);
    workRef.current.style.setProperty("--split-b", `${1 - t}fr`);
  };
  const onSplitterUp = () => {
    dragging.current = false;
  };

  // A new file is a new canvas: keyed by the pipeline, the canvas remounts
  // on a switch (File → Open / Recent, Back) and frames the new graph
  // itself — `fitView` runs once per canvas — instead of showing it at the
  // previous file's zoom and offset (docs/16 §Application layout). The
  // viewport stays mounted: its camera is the user's. The work area's
  // children are KEYED so that a swap or a mode change reorders or
  // restyles the viewport's wrapper instead of remounting it — the three.js
  // scene and its WebGL context are the same across the three viewport
  // modes (docs/16 §Viewport conventions; wave 5 V1).
  const mode = settings.viewportMode;
  const split = mode === "split";
  const canvasPane = (
    <div key="canvas" className="pane" data-testid="canvas-pane">
      <Canvas key={pipeline} />
    </div>
  );
  const splitter = split ? (
    <div
      key="splitter"
      className="splitter"
      role="separator"
      aria-orientation="horizontal"
      title="drag to resize · double-click for presets"
      onPointerDown={onSplitterDown}
      onPointerMove={onSplitterMove}
      onPointerUp={onSplitterUp}
      onDoubleClick={() =>
        updateSettings({
          split: settings.split === "canvas" ? "even" : settings.split === "even" ? "viewport" : "canvas",
        })
      }
    />
  ) : null;
  const viewport = <ViewportFrame key="viewport" mode={mode} areaRef={workRef} />;
  const placeholder = mode === "window" ? <ViewportPlaceholder key="placeholder" /> : null;
  const work = split && settings.swap ? [viewport, splitter, canvasPane] : [canvasPane, splitter, viewport, placeholder];

  return (
    <div className="app" data-testid="app">
      <ConnBanner />
      <TopBar />
      <MenuBar />
      <div className="app-main">
        <div className="app-work" ref={workRef} style={style} data-viewport-mode={mode}>
          {work}
        </div>
        <Inspector />
      </div>
      <TransportBar />
      <StatusBar />
      <Notices />
      <CommitDialog />
      <OpenDialog />
    </div>
  );
}
