/**
 * The 3D viewport (docs/16 §Viewport conventions, docs/04 backward picking).
 * React owns only the shell (overlay buttons, readouts); the scene is the
 * imperative `ViewportScene`. Frames come from `frameBus`; selection,
 * hover, settings and the graph are watched on the store; the imperative
 * API (`frameSelection`/`frameAll`/`screenshot`/`stats`) is installed on
 * mount for the keyboard map, the inspector and `window.__cicada.scene`.
 */
import { useEffect, useRef, useState } from "react";
import { frameBus } from "../state/frameBus";
import { nodeByName, nodeByRef, useCicada, type ElementPick } from "../state/store";
import { installViewportApi, type ViewportApi } from "./api";
import { liveSceneStore } from "./liveStore";
import { ViewportScene, type ScenePick } from "./scene";
import { sampleTheme } from "./theme";
import "./viewport.css";

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
          onRendered: () => refreshReadout(scene),
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
        </div>
        <div className="viewport-readout mono" data-testid="viewport-readout">
          {readout.outputs} outputs · {readout.triangles} tris · {readout.drawCalls} draws · gen{" "}
          {readout.generation}
        </div>
      </div>
      {hoverLabel !== null && (
        <div className="viewport-hover mono" data-testid="viewport-hover">
          {hoverLabel}
        </div>
      )}
    </div>
  );
}
