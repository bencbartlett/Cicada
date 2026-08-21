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
      // EVERY `display_reset` empties the ledger — the server re-streams
      // every displayed output after it. Counted, not keyed to the reset's
      // generation: that generation is the MAX of the server's display table,
      // and an output that vanished meanwhile (its `clear` lost with the
      // socket) can leave the max unchanged — a reconnect or a
      // `resync_display` then re-announces the same generation, and a
      // generation-keyed reset would keep the vanished output painted
      // (review 2026-08-21; `state/frameBus.test.ts`).
      if (state.displayResets !== prev.displayResets) store.reset();
    });
  }
  return store;
}
