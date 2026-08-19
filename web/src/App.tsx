/**
 * The docked window (docs/16 §Application layout): top bar · ribbon ·
 * canvas/viewport split (resizable, presets, swap) · inspector · status bar.
 * Regions are components owned by their folders; this file only arranges
 * them and applies per-user settings (theme, split).
 */
import { useEffect, useRef } from "react";
import { Canvas } from "./canvas/Canvas";
import { useKeyboard } from "./keyboard";
import { ConnBanner } from "./panels/ConnBanner";
import { Inspector } from "./panels/Inspector";
import { Notices } from "./panels/Notices";
import { Ribbon } from "./panels/Ribbon";
import { StatusBar } from "./panels/StatusBar";
import { TopBar } from "./panels/TopBar";
import { useCicada } from "./state/store";
import { Viewport } from "./viewport/Viewport";

const SPLITS: Record<string, [string, string]> = {
  canvas: ["3fr", "2fr"],
  even: ["1fr", "1fr"],
  viewport: ["2fr", "3fr"],
};

export function App() {
  const settings = useCicada((s) => s.settings);
  const updateSettings = useCicada((s) => s.updateSettings);
  const workRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);
  useKeyboard();

  useEffect(() => {
    document.documentElement.dataset.theme = settings.theme;
  }, [settings.theme]);

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

  const first = settings.swap ? <Viewport /> : <Canvas />;
  const second = settings.swap ? <Canvas /> : <Viewport />;

  return (
    <div className="app" data-testid="app">
      <ConnBanner />
      <TopBar />
      <Ribbon />
      <div className="app-main">
        <div className="app-work" ref={workRef} style={style}>
          <div className="pane" data-testid={settings.swap ? "viewport-pane" : "canvas-pane"}>
            {first}
          </div>
          <div
            className="splitter"
            role="separator"
            aria-orientation="horizontal"
            title="drag to resize · double-click for presets"
            onPointerDown={onSplitterDown}
            onPointerMove={onSplitterMove}
            onPointerUp={onSplitterUp}
            onDoubleClick={() =>
              updateSettings({
                split:
                  settings.split === "canvas"
                    ? "even"
                    : settings.split === "even"
                      ? "viewport"
                      : "canvas",
              })
            }
          />
          <div className="pane" data-testid={settings.swap ? "canvas-pane" : "viewport-pane"}>
            {second}
          </div>
        </div>
        <Inspector />
      </div>
      <StatusBar />
      <Notices />
    </div>
  );
}
