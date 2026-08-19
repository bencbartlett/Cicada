/**
 * The one live `SceneStore` behind every mounted viewport. It outlives the
 * React component: a viewport that (re)mounts later — StrictMode's double
 * mount, the canvas/viewport swap, a pane rebuild — repaints from the store
 * instead of waiting for the next generation (frames are only sent when a
 * value hash changes, so a lost frame would otherwise stay lost).
 *
 * The frame-bus subscription and the `display_reset` watch start on first
 * use (not at import), so unit tests can import the pure store freely.
 */
import { frameBus } from "../state/frameBus";
import { useCicada } from "../state/store";
import { SceneStore } from "./sceneStore";

const store = new SceneStore();
let started = false;

export function liveSceneStore(): SceneStore {
  if (!started) {
    started = true;
    frameBus.subscribe((frame) => store.apply(frame));
    useCicada.subscribe((state, prev) => {
      // The server re-streams every displayed output after a reset.
      if (state.displayGeneration !== prev.displayGeneration) store.reset();
    });
  }
  return store;
}
