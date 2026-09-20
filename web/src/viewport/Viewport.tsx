/**
 * The 3D viewport (docs/16 §Viewport conventions, docs/04 backward picking).
 * React owns only the shell (overlay buttons, readouts); the scene is the
 * imperative `ViewportScene`. Frames come from `frameBus`; selection,
 * hover, settings and the graph are watched on the store; the imperative
 * API (`frameSelection`/`frameAll`/`screenshot`/`stats`) is installed on
 * mount for the keyboard map, the inspector and `window.__cicada.scene`.
 * The toolbar's three-way control — split · floating · window — is the
 * viewport-mode controller's (`windowMode.ts`); the host element is
 * registered with it so the `window` mode can move it into the
 * picture-in-picture document and back without a remount.
 */
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { displayText } from "../panels/format";
import { frameBus } from "../state/frameBus";
import { useRoute } from "../state/route";
import { nodeByName, nodeByRef, useCicada, type ElementPick } from "../state/store";
import { installViewportApi, type ViewportApi } from "./api";
import { liveSceneStore } from "./liveStore";
import { VIEWPORT_MODES, type ViewportMode } from "./modes";
import { ViewportScene, type ScenePick } from "./scene";
import { sampleTheme } from "./theme";
import { chooseViewportMode, registerViewportHost } from "./windowMode";
import "./viewport.css";

/** The three-way control's labels and hovers (docs/16 §Viewport conventions). */
const MODE_LABELS: Record<ViewportMode, { label: string; title: string }> = {
  split: { label: "split", title: "split: the viewport in its pane beside the canvas" },
  floating: { label: "floating", title: "floating: a panel over the canvas — drag its title strip, resize its corner" },
  window: {
    label: "window",
    title:
      "window: the viewport in a picture-in-picture window of its own (Chromium); closing it returns to split — elsewhere the read-only pop-out opens instead",
  },
};

interface Readout {
  outputs: number;
  triangles: number;
  drawCalls: number;
  generation: number;
}

const EMPTY_READOUT: Readout = { outputs: 0, triangles: 0, drawCalls: 0, generation: 0 };

function toElementPick(pick: ScenePick, name: string | null): ElementPick {
  return {
    pickId: pick.pickId,
    nodeRef: pick.nodeRef,
    node: name,
    output: pick.output,
    element: pick.element,
  };
}

/** Node refs of the selected binding names (unknown names drop out). */
function selectedRefs(names: string[]): Set<number> {
  const graph = useCicada.getState().graph;
  const refs = new Set<number>();
  for (const name of names) {
    const node = nodeByName(graph, name);
    if (node !== undefined) refs.add(node.ref);
  }
  return refs;
}

export function Viewport() {
  const hostRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<ViewportScene | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [readout, setReadout] = useState<Readout>(EMPTY_READOUT);
  const displayMode = useCicada((s) => s.settings.displayMode);
  const hoverPick = useCicada((s) => s.hoverPick);
  const updateSettings = useCicada((s) => s.updateSettings);
  const display = useCicada((s) => s.display);
  const viewportMode = useCicada((s) => s.settings.viewportMode);
  const view = useRoute((s) => s.route.view);

  // The `window` mode moves this element into the picture-in-picture
  // document (`windowMode.ts`). A LAYOUT effect, so its cleanup — the
  // element back where React left it — runs before React removes the node
  // on unmount (a passive cleanup would run after, on a node that is no
  // longer where React looks for it).
  useLayoutEffect(() => {
    const host = hostRef.current;
    if (host === null || view === "viewport") return;
    return registerViewportHost({ element: host, rehome: (win) => sceneRef.current?.rehome(win) });
  }, [view]);

  useEffect(() => {
    const host = hostRef.current;
    if (host === null) return;
    const store = useCicada.getState;
    const nameOf = (ref: number) => nodeByRef(store().graph, ref)?.name ?? null;

    let readoutTimer: ReturnType<typeof setTimeout> | null = null;
    const refreshReadout = (scene: ViewportScene) => {
      if (readoutTimer !== null) return;
      readoutTimer = setTimeout(() => {
        readoutTimer = null;
        const stats = scene.stats();
        let triangles = 0;
        for (const output of Object.values(stats.outputs)) triangles += output.triangles;
        setReadout({
          outputs: Object.keys(stats.outputs).length,
          triangles,
          drawCalls: stats.drawCalls,
          generation: stats.lastGeneration,
        });
      }, 100);
    };

    const frames = liveSceneStore();
    let scene: ViewportScene;
    try {
      scene = new ViewportScene(
        host,
        sampleTheme(store().settings.theme),
        {
          nameOf,
          onHover: (pick) => {
            store().setHoverPick(pick === null ? null : toElementPick(pick, nameOf(pick.nodeRef)));
          },
          onClick: (pick) => {
            if (pick === null) {
              store().clearSelection();
              return;
            }
            const name = nameOf(pick.nodeRef);
            store().selectElement(toElementPick(pick, name));
            if (name !== null) store().send({ type: "inspect", payload: { node: name } });
          },
          onRendered: () => {
            refreshReadout(scene);
            // The first render after a pass ended is the one that uploaded
            // its last frames (they were applied before `display_end`, which
            // rides the display lane behind them): the client's paint time.
            const pass = store().display;
            if (pass !== null && pass.phase === "painted" && pass.paintedMs === null) {
              store().markPainted(pass.generation);
            }
          },
          notice: (level, message) => store().addNotice(level, message),
          solveRunning: () => store().summary.running,
        },
        frames,
      );
    } catch (error) {
      const message = `viewport: WebGL unavailable — ${String(error)}`;
      setFailure(message);
      store().addNotice("error", message);
      return;
    }
    sceneRef.current = scene;
    scene.setDisplayMode(store().settings.displayMode);
    scene.setNavigation(store().settings.navigation);

    const applySelection = () => {
      const state = store();
      scene.setNodeHighlight(selectedRefs(state.selection.nodes));
      scene.setPickHighlight(
        state.selection.element?.pickId ?? null,
        state.hoverPick?.pickId ?? null,
      );
    };
    applySelection();

    const unsubscribeStore = useCicada.subscribe((state, prev) => {
      if (state.settings.displayMode !== prev.settings.displayMode) {
        scene.setDisplayMode(state.settings.displayMode);
      }
      if (state.settings.navigation !== prev.settings.navigation) {
        scene.setNavigation(state.settings.navigation);
      }
      if (state.settings.theme !== prev.settings.theme) {
        scene.setTheme(sampleTheme(state.settings.theme));
      }
      // A pass just ended: render once more so `onRendered` stamps its
      // paint time even when the frames' own render already happened
      // before `display_end` arrived (a redundant render is cheap).
      if (
        state.display !== prev.display &&
        state.display !== null &&
        state.display.phase === "painted" &&
        state.display.paintedMs === null
      ) {
        scene.requestRender();
      }
      if (state.graph !== prev.graph) {
        scene.recolor();
        applySelection();
      } else if (state.selection !== prev.selection || state.hoverPick !== prev.hoverPick) {
        applySelection();
      }
    });

    const api: ViewportApi = {
      frameSelection: () => scene.frameNodes(selectedRefs(store().selection.nodes)),
      frameAll: () => scene.frameAll(),
      screenshot: () => scene.screenshot(),
      stats: () => scene.stats(),
    };
    installViewportApi(api);
    frameBus.setScreenshotHandler(() => api.screenshot());
    const debug = (window as unknown as { __cicada?: { scene: (() => unknown) | null } }).__cicada;
    if (debug !== undefined) debug.scene = () => api.stats();

    return () => {
      unsubscribeStore();
      frameBus.setScreenshotHandler(null);
      if (debug !== undefined) debug.scene = null;
      if (readoutTimer !== null) clearTimeout(readoutTimer);
      scene.dispose();
      sceneRef.current = null;
    };
  }, []);

  const hoverLabel =
    hoverPick === null
      ? null
      : `${hoverPick.node ?? `#${hoverPick.nodeRef}`}[${hoverPick.element}]`;

  return (
    <div className="viewport" data-testid="viewport" ref={hostRef}>
      {failure !== null && <div className="viewport-failure">{failure}</div>}
      <div className="viewport-overlay">
        <div className="viewport-toolbar">
          <button
            type="button"
            className={displayMode === "shaded_edges" ? "active" : ""}
            title="shaded + edges"
            onClick={() => updateSettings({ displayMode: "shaded_edges" })}
          >
            shaded+edges
          </button>
          <button
            type="button"
            className={displayMode === "shaded" ? "active" : ""}
            title="shaded"
            onClick={() => updateSettings({ displayMode: "shaded" })}
          >
            shaded
          </button>
          <button
            type="button"
            className={displayMode === "wireframe" ? "active" : ""}
            title="wireframe"
            onClick={() => updateSettings({ displayMode: "wireframe" })}
          >
            wire
          </button>
          <button
            type="button"
            title="frame all (Home)"
            data-testid="viewport-frame-all"
            onClick={() => sceneRef.current?.frameAll()}
          >
            frame all
          </button>
          {view !== "viewport" && (
            <span className="viewport-modes" role="radiogroup" aria-label="viewport mode" data-testid="viewport-modes">
              {VIEWPORT_MODES.map((mode) => (
                <button
                  key={mode}
                  type="button"
                  role="radio"
                  aria-checked={viewportMode === mode}
                  className={viewportMode === mode ? "active" : ""}
                  title={MODE_LABELS[mode].title}
                  data-testid={`viewport-mode-${mode}`}
                  onClick={() => chooseViewportMode(mode)}
                >
                  {MODE_LABELS[mode].label}
                </button>
              ))}
            </span>
          )}
        </div>
        <div className="viewport-readout mono" data-testid="viewport-readout">
          {readout.outputs} outputs · {readout.triangles} tris · {readout.drawCalls} draws · gen{" "}
          {readout.generation}
        </div>
        {display !== null && (
          <div
            className="viewport-display mono"
            data-testid="viewport-display"
            data-phase={display.phase}
            data-generation={display.generation}
            title={
              display.phase === "painting"
                ? `generation ${display.generation}'s display pass is in flight — the server is tessellating and encoding its frames`
                : `generation ${display.generation}: tessellation ${display.tessellateMs.toFixed(1)} ms · encode ${display.encodeMs.toFixed(1)} ms on the server${display.paintedMs === null ? "" : ` · ${display.paintedMs.toFixed(0)} ms here from display_begin to the first render after the last frame`}`
            }
          >
            {display.phase === "painting" && <i className="spin" aria-hidden />}
            <span>{displayText(display)}</span>
          </div>
        )}
      </div>
      {hoverLabel !== null && (
        <div className="viewport-hover mono" data-testid="viewport-hover">
          {hoverLabel}
        </div>
      )}
    </div>
  );
}
