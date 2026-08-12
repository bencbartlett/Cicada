import { PROTOCOL_VERSION } from "./protocol/version";

/**
 * Stage-0 placeholder shell. The real app — React Flow canvas, three.js
 * viewport, inspector, ribbon (docs/13, docs/16) — lands in stage 5.
 */
export function App() {
  return (
    <main>
      <h1>Cicada</h1>
      <p>
        Engine not connected — the spike UI lands in stage 5 (protocol v
        {PROTOCOL_VERSION}).
      </p>
    </main>
  );
}
