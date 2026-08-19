/**
 * The imperative three.js scene behind the `Viewport` component.
 *
 * Owns the renderer, camera, controls, helpers and one `Group` per drawn
 * output (`nodeRef:output`), each holding one draw call per (kind | mesh
 * hash): meshes and instanced blobs use `SurfaceMaterial`, curves / points
 * / edge overlays use `FlatMaterial`. Geometry is built straight over the
 * frame's typed-array views (zero copy for positions and indices; pick ids
 * are converted to a float attribute once — they fit exactly below 2^24).
 *
 * Backward picking is an ID-buffer pass: the scene is rendered with the
 * pick override material into a 1×1 target scissored to the cursor via
 * `camera.setViewOffset`, and the RGB8 pixel decodes to a pick id.
 *
 * Rendering is on demand (frames, camera, hover, selection, resize); a
 * `requestAnimationFrame` loop runs only while the controls are moving.
 */
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { BatchFrame, InstancesFrame } from "../protocol/frames";
import type { ViewportStats } from "./api";
import {
  createFlatMaterial,
  createPickMaterial,
  createSharedUniforms,
  createSurfaceMaterial,
  nodeColor,
  type CicadaMaterial,
  type SharedUniforms,
} from "./materials";
import { EdgeBuilder, type EdgeJobHandle } from "./edgeBuilder";
import { EDGE_QUIET_MS, EDGE_THRESHOLD_DEG, EDGE_TRIANGLE_LIMIT, edgeBuildAllowed, edgePolicy } from "./edgePolicy";
import { decodePickPixel } from "./picking";
import {
  type SceneStore,
  outputKey,
  unionBounds,
  type BatchKind,
  type Bounds,
  type OutputEntry,
  type PickTarget,
} from "./sceneStore";

export type DisplayMode = "shaded_edges" | "shaded" | "wireframe";
export type NavigationPreset = "rhino" | "blender";

/** Theme colors sampled from the CSS custom properties (already parsed). */
export interface ThemeColors {
  theme: "dark" | "light";
  background: THREE.Color;
  accent: THREE.Color;
  grid: THREE.Color;
  gridStrong: THREE.Color;
  edge: THREE.Color;
  curve: THREE.Color;
  point: THREE.Color;
}

export interface ScenePick extends PickTarget {
  pickId: number;
}

export interface SceneCallbacks {
  /** Binding name for a node ref (null when the graph does not know it yet). */
  nameOf(nodeRef: number): string | null;
  onHover(pick: ScenePick | null): void;
  onClick(pick: ScenePick | null): void;
  /** Called after each render (stats readout). */
  onRendered(): void;
  notice(level: "info" | "warning" | "error", message: string): void;
  /** Is a solve running right now? Deferred edge overlays wait for quiet (edgePolicy.ts). */
  solveRunning(): boolean;
}

export { EDGE_THRESHOLD_DEG, EDGE_TRIANGLE_LIMIT } from "./edgePolicy";
const POINT_SIZE_PX = 6;
const CLICK_SLOP_PX = 4;
const HOVER_INTERVAL_MS = 33;
/** The WebGL context-lost notice waits this long for a restore before it is raised. */
export const CONTEXT_LOST_GRACE_MS = 1000;

/** Every drawable of one (kind | `inst:` + hash) inside an output. */
interface Piece {
  objects: THREE.Object3D[];
  disposables: { dispose(): void }[];
  materials: CicadaMaterial[];
  triangles: number;
  edgesSkipped: boolean;
  /** A main-thread deferred edge build not run yet (worker unavailable; null = none). */
  edgeJob: EdgeJob | null;
  /** An off-thread edge build in flight for this piece (null = none). */
  edgeHandle: EdgeJobHandle | null;
  /** Set by `dropPiece`: pending work for this piece must not run. */
  dropped: boolean;
}

/** A deferred edge-overlay build: attaches the overlay to its (still live) piece. */
interface EdgeJob {
  key: string;
  piece: Piece;
  build: () => void;
}

interface OutputDrawables {
  entry: OutputEntry;
  group: THREE.Group;
  pieces: Map<string, Piece>;
}

const LAYER_PICKABLE = 0;
const LAYER_DECOR = 1;

/** The last camera pose, so a remounted viewport (pane swap) keeps its view. */
let savedView: { position: THREE.Vector3; target: THREE.Vector3; radius: number } | null = null;

export interface ExtendedStats extends ViewportStats {
  /** Draws whose edge overlay was skipped (over `EDGE_TRIANGLE_LIMIT`). */
  edgesSkipped: number;
  /** Draws whose edge overlay is scheduled but not built yet (deferred to idle time). */
  edgesPending: number;
  /** Deferred edge overlays built so far. */
  edgesDeferredBuilt: number;
  /** ID-buffer passes run so far (hover + click picks). */
  pickPasses: number;
  /** Main renders so far. */
  renders: number;
}

/** A "nice" grid step (1·2·5 × 10^k) so ~`divisions` cells span `extent`. */
export function niceGridStep(extent: number, divisions: number): number {
  const raw = Math.max(extent / divisions, 1e-6);
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const nice = norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10;
  return nice * mag;
}

/** Bounds → (center, radius); degenerate bounds get a unit radius. */
export function sphereOfBounds(bounds: Bounds): { center: THREE.Vector3; radius: number } {
  const [min, max] = bounds;
  const center = new THREE.Vector3(
    (min[0] + max[0]) / 2,
    (min[1] + max[1]) / 2,
    (min[2] + max[2]) / 2,
  );
  const radius = new THREE.Vector3(max[0] - min[0], max[1] - min[1], max[2] - min[2]).length() / 2;
  return { center, radius: radius > 1e-6 ? radius : 1 };
}

/** Camera distance so a sphere of `radius` fits the vertical and horizontal fov. */
export function fitDistance(radius: number, fovDeg: number, aspect: number): number {
  const vFov = (fovDeg * Math.PI) / 180;
  const hFov = 2 * Math.atan(Math.tan(vFov / 2) * aspect);
  const fov = Math.min(vFov, hFov);
  return (radius / Math.sin(fov / 2)) * 1.1;
}

export class ViewportScene {
  readonly store: SceneStore;
  readonly renderer: THREE.WebGLRenderer;
  readonly canvas: HTMLCanvasElement;
  readonly scene = new THREE.Scene();
  readonly camera: THREE.PerspectiveCamera;
  readonly controls: OrbitControls;
  private shared: SharedUniforms;
  private pickMaterial: THREE.ShaderMaterial;
  private pickTarget: THREE.WebGLRenderTarget;
  private pickPixel = new Uint8Array(4);
  private drawables = new Map<string, OutputDrawables>();
  private blobGeometries = new Map<string, THREE.BufferGeometry>();
  private blobEdges = new Map<string, THREE.BufferGeometry | null>();
  private helpers = new THREE.Group();
  private theme: ThemeColors;
  private displayMode: DisplayMode = "shaded_edges";
  private highlightedNodes = new Set<number>();
  private highlightedPick: number | null = null;
  private hoverPick: number | null = null;
  private lastDrawCalls = 0;
  private pickPasses = 0;
  private renders = 0;
  private framedOnce = false;
  private renderScheduled = false;
  private interacting = false;
  private disposed = false;
  private sceneRadius = 5;
  private hover = { x: 0, y: 0, pending: false, inside: false, lastId: 0 };
  private hoverTimer: ReturnType<typeof setTimeout> | null = null;
  private down: { x: number; y: number; button: number } | null = null;
  private resizeObserver: ResizeObserver | null = null;
  private pixelRatio = 1;
  private onFirstGeometry: (() => void) | null = null;
  private unsubscribeStore: () => void;
  /** Deferred edge-overlay builds, oldest first; drained one per idle slot. */
  private edgeQueue: EdgeJob[] = [];
  private edgePump: { kind: "idle" | "timer"; id: number } | null = null;
  private edgesDeferredBuilt = 0;
  /** Off-thread edge builds (edgeWorker.ts); falls back to the main thread when no worker can be made. */
  private edgeBuilder = new EdgeBuilder((level, message) => this.callbacks.notice(level, message));
  /** Instanced pieces waiting for one blob's off-thread edge build, by hash. */
  private blobEdgeWaiters = new Map<
    string,
    { piece: Piece; onReady: (edges: THREE.BufferGeometry) => void }[]
  >();
  /** When each output last received a frame (quiet detection for deferred edges). */
  private lastFrameAt = new Map<string, number>();
  private contextLostTimer: ReturnType<typeof setTimeout> | null = null;
  private contextLostNoticed = false;

  constructor(
    private container: HTMLElement,
    theme: ThemeColors,
    private callbacks: SceneCallbacks,
    store: SceneStore,
  ) {
    this.theme = theme;
    this.store = store;
    this.unsubscribeStore = store.subscribe({
      onOutput: (key, entry) => this.onOutputChanged(key, entry),
      onReset: () => this.onReset(),
    });

    this.canvas = document.createElement("canvas");
    this.canvas.className = "viewport-canvas";
    this.canvas.dataset.testid = "viewport-canvas";
    container.appendChild(this.canvas);
    this.renderer = new THREE.WebGLRenderer({
      canvas: this.canvas,
      antialias: true,
      alpha: false,
      preserveDrawingBuffer: false,
      powerPreference: "high-performance",
    });
    this.pixelRatio = Math.min(window.devicePixelRatio || 1, 2);
    this.renderer.setPixelRatio(this.pixelRatio);
    this.renderer.setClearColor(theme.background, 1);
    this.renderer.autoClear = true;

    this.camera = new THREE.PerspectiveCamera(45, 1, 0.01, 1000);
    this.camera.up.set(0, 0, 1);
    this.camera.position.set(7, -9, 6);
    this.camera.layers.enable(LAYER_DECOR);
    this.camera.lookAt(0, 0, 0);

    this.controls = new OrbitControls(this.camera, this.canvas);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.12;
    this.controls.zoomToCursor = true;
    this.controls.screenSpacePanning = true;
    this.controls.target.set(0, 0, 0);
    this.controls.addEventListener("start", () => {
      this.interacting = true;
      this.requestRender();
    });
    this.controls.addEventListener("end", () => {
      this.interacting = false;
      this.requestRender();
    });
    this.controls.addEventListener("change", () => this.requestRender());
    this.setNavigation("rhino");

    this.shared = createSharedUniforms(theme.accent.clone());
    // The pick target is one texel per CSS pixel, so point sprites there
    // are NOT scaled by the device pixel ratio.
    this.pickMaterial = createPickMaterial(POINT_SIZE_PX);
    this.pickTarget = new THREE.WebGLRenderTarget(1, 1, {
      format: THREE.RGBAFormat,
      type: THREE.UnsignedByteType,
      depthBuffer: true,
      stencilBuffer: false,
      generateMipmaps: false,
      minFilter: THREE.NearestFilter,
      magFilter: THREE.NearestFilter,
    });

    this.helpers.layers.set(LAYER_DECOR);
    this.scene.add(this.helpers);
    this.rebuildHelpers(1, new THREE.Vector3());

    this.canvas.addEventListener("webglcontextlost", this.onContextLost);
    this.canvas.addEventListener("webglcontextrestored", this.onContextRestored);
    this.canvas.addEventListener("pointermove", this.onPointerMove);
    this.canvas.addEventListener("pointerleave", this.onPointerLeave);
    this.canvas.addEventListener("pointerdown", this.onPointerDown);
    this.canvas.addEventListener("pointerup", this.onPointerUp);

    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(container);
    this.resize();

    // A remount keeps the previous view instead of re-framing.
    if (savedView !== null) {
      this.camera.position.copy(savedView.position);
      this.controls.target.copy(savedView.target);
      this.sceneRadius = savedView.radius;
      this.controls.update();
      this.rebuildHelpers(niceGridStep(savedView.radius * 4, 20), savedView.target);
      this.framedOnce = true;
    }

    // Paint whatever the live store already holds (late mount / remount).
    for (const [key, entry] of store.outputs) this.onOutputChanged(key, entry);
  }

  // ------------------------------------------------------------- lifecycle --

  dispose(): void {
    if (this.framedOnce) {
      savedView = {
        position: this.camera.position.clone(),
        target: this.controls.target.clone(),
        radius: this.sceneRadius,
      };
    }
    this.disposed = true;
    this.unsubscribeStore();
    this.resizeObserver?.disconnect();
    if (this.hoverTimer !== null) clearTimeout(this.hoverTimer);
    if (this.contextLostTimer !== null) clearTimeout(this.contextLostTimer);
    this.cancelEdgePump();
    this.edgeQueue = [];
    this.edgeBuilder.dispose();
    this.blobEdgeWaiters.clear();
    if (this.hover.lastId !== 0) {
      this.hover.lastId = 0;
      this.callbacks.onHover(null);
    }
    this.canvas.removeEventListener("webglcontextlost", this.onContextLost);
    this.canvas.removeEventListener("webglcontextrestored", this.onContextRestored);
    this.canvas.removeEventListener("pointermove", this.onPointerMove);
    this.canvas.removeEventListener("pointerleave", this.onPointerLeave);
    this.canvas.removeEventListener("pointerdown", this.onPointerDown);
    this.canvas.removeEventListener("pointerup", this.onPointerUp);
    this.controls.dispose();
    for (const key of Array.from(this.drawables.keys())) this.dropOutput(key);
    for (const g of this.blobGeometries.values()) g.dispose();
    for (const g of this.blobEdges.values()) g?.dispose();
    this.blobGeometries.clear();
    this.blobEdges.clear();
    this.disposeHelpers();
    this.pickTarget.dispose();
    this.pickMaterial.dispose();
    this.renderer.dispose();
    this.canvas.remove();
  }

  /** Called once the first geometry arrives (auto frame-all). */
  setFirstGeometryHook(hook: () => void): void {
    this.onFirstGeometry = hook;
  }

  // A context lost that the browser restores within the grace period (some
  // headless / software-GL configurations drop it once at start-up) is not
  // worth an error; one that stays lost is.
  private onContextLost = (event: Event) => {
    event.preventDefault();
    if (this.contextLostTimer !== null) return;
    this.contextLostTimer = setTimeout(() => {
      this.contextLostTimer = null;
      if (this.disposed) return;
      this.contextLostNoticed = true;
      this.callbacks.notice(
        "error",
        "viewport: WebGL context lost — waiting for the browser to restore it",
      );
    }, CONTEXT_LOST_GRACE_MS);
  };

  private onContextRestored = () => {
    if (this.contextLostTimer !== null) {
      clearTimeout(this.contextLostTimer);
      this.contextLostTimer = null;
    }
    if (this.contextLostNoticed) {
      this.contextLostNoticed = false;
      this.callbacks.notice("info", "viewport: WebGL context restored");
    }
    this.requestRender();
  };

  resize(): void {
    if (this.disposed) return;
    const width = Math.max(1, Math.floor(this.container.clientWidth));
    const height = Math.max(1, Math.floor(this.container.clientHeight));
    this.renderer.setSize(width, height, false);
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${height}px`;
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.requestRender();
  }

  // ---------------------------------------------------------------- frames --

  /** GPU geometry for a cached blob, built once per hash (content-addressed). */
  private blobGeometry(hash: string): THREE.BufferGeometry | null {
    const cached = this.blobGeometries.get(hash);
    if (cached !== undefined) return cached;
    const blob = this.store.blobs.get(hash);
    if (blob === undefined) return null;
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(blob.positions, 3));
    geometry.setIndex(new THREE.BufferAttribute(blob.indices, 1));
    this.blobGeometries.set(hash, geometry);
    return geometry;
  }

  private onReset(): void {
    for (const key of Array.from(this.drawables.keys())) this.dropOutput(key);
    this.requestRender();
  }

  private onOutputChanged(key: string, entry: OutputEntry | null): void {
    this.lastFrameAt.set(key, performance.now());
    if (entry === null) {
      this.dropOutput(key);
      this.requestRender();
      return;
    }
    let drawables = this.drawables.get(key);
    if (drawables === undefined || drawables.entry !== entry) {
      // A newer generation (or a first sight): everything held goes. (The
      // server only sends an output's frames when its value hash changed,
      // so a byte-identical batch never arrives under a new generation —
      // nothing to reuse here; `syncPieces` reuses within a generation.)
      if (drawables !== undefined) this.dropOutput(key);
      const group = new THREE.Group();
      group.name = key;
      this.scene.add(group);
      drawables = { entry, group, pieces: new Map() };
      this.drawables.set(key, drawables);
    }
    this.syncPieces(drawables);
    if (!this.framedOnce && this.store.bounds() !== null) {
      this.framedOnce = true;
      this.frameBounds(this.store.bounds());
      this.onFirstGeometry?.();
    }
    this.requestRender();
  }

  /** Make the output's pieces match its entry (rebuild only what changed). */
  private syncPieces(drawables: OutputDrawables): void {
    const { entry, group, pieces } = drawables;
    const wanted = new Set<string>();
    for (const [kind, batch] of entry.batches) {
      wanted.add(kind);
      const piece = pieces.get(kind);
      if (piece !== undefined && pieceMatchesBatch(piece, batch)) continue;
      if (piece !== undefined) this.dropPiece(group, piece);
      pieces.set(kind, this.buildBatchPiece(entry, kind, batch, group));
    }
    for (const [hash, inst] of entry.instances) {
      const id = `inst:${hash}`;
      wanted.add(id);
      const piece = pieces.get(id);
      if (piece !== undefined && pieceMatchesInstances(piece, inst)) continue;
      if (piece !== undefined) this.dropPiece(group, piece);
      const built = this.buildInstancesPiece(entry, inst, group);
      if (built !== null) pieces.set(id, built);
      else pieces.delete(id);
    }
    for (const [id, piece] of Array.from(pieces)) {
      if (!wanted.has(id)) {
        this.dropPiece(group, piece);
        pieces.delete(id);
      }
    }
    this.applyNodeHighlight(drawables);
  }

  private dropPiece(group: THREE.Group, piece: Piece): void {
    piece.dropped = true;
    // Cancelled: a dropped/replaced piece never gets its overlay built.
    if (piece.edgeJob !== null) {
      this.edgeQueue = this.edgeQueue.filter((job) => job !== piece.edgeJob);
      piece.edgeJob = null;
    }
    if (piece.edgeHandle !== null) {
      // A shared blob edge build stays alive for its other live waiters
      // (the result is cached per hash regardless).
      if (!this.hasOtherBlobWaiters(piece)) piece.edgeHandle.cancel();
      piece.edgeHandle = null;
    }
    for (const object of piece.objects) group.remove(object);
    for (const d of piece.disposables) d.dispose();
  }

  private dropOutput(key: string): void {
    const drawables = this.drawables.get(key);
    if (drawables === undefined) return;
    for (const piece of drawables.pieces.values()) this.dropPiece(drawables.group, piece);
    this.scene.remove(drawables.group);
    this.drawables.delete(key);
    this.lastFrameAt.delete(key);
  }

  // ------------------------------------------------- deferred edge overlays --
  //
  // Large pieces (edgePolicy "deferred") never block the frame handler: the
  // shaded mesh attaches at once and the crease pass runs off the UI thread
  // in `edgeWorker.ts` via `EdgeBuilder` (one job in flight; stale jobs —
  // dropped/replaced pieces — are cancelled before they start). Where no
  // worker can be made, the same builds run on the main thread from an idle
  // slot, gated by the quiet policy (`edgeBuildAllowed`).

  /** Queue a main-thread edge build (worker unavailable); drained at idle time when quiet. */
  private deferEdgesOnMainThread(key: string, piece: Piece, build: () => void): void {
    const job: EdgeJob = { key, piece, build };
    piece.edgeJob = job;
    this.edgeQueue.push(job);
    this.pumpEdges();
  }

  private cancelEdgePump(): void {
    if (this.edgePump === null) return;
    if (this.edgePump.kind === "idle") window.cancelIdleCallback(this.edgePump.id);
    else window.clearTimeout(this.edgePump.id);
    this.edgePump = null;
  }

  /** Schedule one drain step (idle callback when the browser has one, else a short timer). */
  private pumpEdges(delayMs = 0): void {
    if (this.disposed || this.edgePump !== null || this.edgeQueue.length === 0) return;
    if (delayMs === 0 && typeof window.requestIdleCallback === "function") {
      this.edgePump = {
        kind: "idle",
        id: window.requestIdleCallback(() => this.drainEdges(), { timeout: 1000 }),
      };
    } else {
      this.edgePump = { kind: "timer", id: window.setTimeout(() => this.drainEdges(), Math.max(delayMs, 16)) };
    }
  }

  /** Build ONE eligible overlay (a quiet output), then yield; wait if nothing is quiet yet. */
  private drainEdges(): void {
    this.edgePump = null;
    if (this.disposed) return;
    const now = performance.now();
    const running = this.callbacks.solveRunning();
    let waitMs = Infinity;
    for (let i = 0; i < this.edgeQueue.length; i++) {
      const job = this.edgeQueue[i] as EdgeJob;
      if (job.piece.dropped) {
        this.edgeQueue.splice(i, 1);
        i -= 1;
        continue;
      }
      const since = now - (this.lastFrameAt.get(job.key) ?? 0);
      if (!edgeBuildAllowed(running, since)) {
        waitMs = Math.min(waitMs, EDGE_QUIET_MS - since);
        continue;
      }
      this.edgeQueue.splice(i, 1);
      job.piece.edgeJob = null;
      job.build();
      this.edgesDeferredBuilt += 1;
      this.requestRender();
      break;
    }
    if (this.edgeQueue.length > 0) this.pumpEdges(Number.isFinite(waitMs) ? Math.max(waitMs, 16) : 0);
  }

  /** Attach ready edge segments to a piece as its overlay. */
  private attachEdgeLines(piece: Piece, edges: THREE.BufferGeometry, group: THREE.Group): void {
    const edgeMaterial = createFlatMaterial(this.shared, { color: this.theme.edge, opacity: 0.6 });
    const lines = new THREE.LineSegments(edges, edgeMaterial);
    lines.frustumCulled = false;
    lines.layers.set(LAYER_DECOR);
    group.add(lines);
    piece.objects.push(lines);
    piece.materials.push(edgeMaterial);
    piece.disposables.push(edges, edgeMaterial);
    // A highlight applied while the overlay was pending reaches it too.
    edgeMaterial.cicada.uNodeHighlight.value = piece.materials[0]?.cicada.uNodeHighlight.value ?? 0;
  }

  /** The edge overlay of a (non-instanced) mesh, built synchronously and attached. */
  private attachMeshEdges(piece: Piece, geometry: THREE.BufferGeometry, group: THREE.Group): void {
    this.attachEdgeLines(piece, new THREE.EdgesGeometry(geometry, EDGE_THRESHOLD_DEG), group);
  }

  /** The edge overlay of a (non-instanced) mesh, built off-thread; attached when it lands (unless dropped). */
  private attachMeshEdgesOffThread(piece: Piece, batch: BatchFrame, group: THREE.Group): void {
    piece.edgeHandle = this.edgeBuilder.submit(
      { positions: batch.positions, indices: batch.indices },
      (positions) => {
        piece.edgeHandle = null;
        if (piece.dropped || this.disposed) return;
        this.attachEdgeLines(piece, edgeGeometryFromPositions(positions), group);
        this.edgesDeferredBuilt += 1;
        this.requestRender();
      },
    );
  }

  /** The instanced edge overlay for an instances piece, from a ready edge source. */
  private attachInstancedEdges(
    piece: Piece,
    edgeSource: THREE.BufferGeometry,
    mesh: THREE.InstancedMesh,
    group: THREE.Group,
  ): void {
    const edgeGeometry = new THREE.InstancedBufferGeometry();
    edgeGeometry.setAttribute("position", edgeSource.getAttribute("position"));
    edgeGeometry.setAttribute("instanceMatrix", mesh.instanceMatrix);
    edgeGeometry.instanceCount = mesh.count;
    const edgeMaterial = createFlatMaterial(this.shared, {
      color: this.theme.edge,
      opacity: 0.6,
      instanced: true,
    });
    const lines = new THREE.LineSegments(edgeGeometry, edgeMaterial);
    lines.frustumCulled = false;
    lines.layers.set(LAYER_DECOR);
    group.add(lines);
    piece.objects.push(lines);
    piece.materials.push(edgeMaterial);
    piece.disposables.push(edgeGeometry, edgeMaterial);
    edgeMaterial.cicada.uNodeHighlight.value = piece.materials[0]?.cicada.uNodeHighlight.value ?? 0;
  }

  /**
   * Edge source of a blob (one per hash, shared by every instanced piece
   * drawing it): cached → now; small → built inline now; large → built
   * off-thread once, and every waiting piece attaches when it lands.
   */
  private withBlobEdges(
    hash: string,
    blob: THREE.BufferGeometry,
    piece: Piece,
    onReady: (edgeSource: THREE.BufferGeometry) => void,
  ): void {
    const cached = this.blobEdges.get(hash);
    if (cached !== undefined) {
      if (cached !== null) onReady(cached);
      return;
    }
    const blobTriangles = Math.floor((blob.index?.count ?? 0) / 3);
    const policy = edgePolicy(blobTriangles);
    if (policy === "skip") {
      this.blobEdges.set(hash, null);
      return;
    }
    if (policy === "inline" || !this.edgeBuilder.offThread) {
      // Inline, or the main-thread fallback (the caller scheduled us when quiet).
      const edges = new THREE.EdgesGeometry(blob, EDGE_THRESHOLD_DEG);
      this.blobEdges.set(hash, edges);
      onReady(edges);
      return;
    }
    const waiters = this.blobEdgeWaiters.get(hash);
    if (waiters !== undefined) {
      waiters.push({ piece, onReady });
      return;
    }
    this.blobEdgeWaiters.set(hash, [{ piece, onReady }]);
    const positions = blob.getAttribute("position").array;
    const index = blob.index?.array;
    if (!(positions instanceof Float32Array) || !(index instanceof Uint32Array)) {
      this.blobEdgeWaiters.delete(hash);
      this.blobEdges.set(hash, null);
      this.callbacks.notice(
        "error",
        `viewport: blob ${hash.slice(0, 12)}… has unexpected buffer types — no edge overlay`,
      );
      return;
    }
    piece.edgeHandle = this.edgeBuilder.submit({ positions, indices: index }, (edgePositions) => {
      const pending = this.blobEdgeWaiters.get(hash) ?? [];
      this.blobEdgeWaiters.delete(hash);
      if (this.disposed) return;
      const edges = edgeGeometryFromPositions(edgePositions);
      this.blobEdges.set(hash, edges);
      for (const waiter of pending) {
        waiter.piece.edgeHandle = null;
        if (!waiter.piece.dropped) {
          waiter.onReady(edges);
          this.edgesDeferredBuilt += 1;
        }
      }
      this.requestRender();
    });
  }

  /** Is this piece's in-flight blob edge build also awaited by another live piece? */
  private hasOtherBlobWaiters(piece: Piece): boolean {
    for (const waiters of this.blobEdgeWaiters.values()) {
      const mine = waiters.some((w) => w.piece === piece);
      if (mine && waiters.some((w) => w.piece !== piece && !w.piece.dropped)) return true;
    }
    return false;
  }

  private colorFor(nodeRef: number): THREE.Color {
    const name = this.callbacks.nameOf(nodeRef);
    return nodeColor(name ?? `#${nodeRef}`, this.theme.theme);
  }

  private buildBatchPiece(
    entry: OutputEntry,
    kind: BatchKind,
    batch: BatchFrame,
    group: THREE.Group,
  ): Piece {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(batch.positions, 3));
    geometry.setAttribute("pickId", new THREE.BufferAttribute(Float32Array.from(batch.pickIds), 1));
    if (kind !== "point") geometry.setIndex(new THREE.BufferAttribute(batch.indices, 1));
    const piece: Piece = {
      objects: [],
      disposables: [geometry],
      materials: [],
      triangles: 0,
      edgesSkipped: false,
      edgeJob: null,
      edgeHandle: null,
      dropped: false,
    };
    const tag = (object: THREE.Object3D) => {
      object.frustumCulled = false;
      object.layers.set(LAYER_PICKABLE);
      object.userData.batch = batch;
      group.add(object);
      piece.objects.push(object);
    };
    if (kind === "mesh") {
      piece.triangles = Math.floor(batch.indices.length / 3);
      const material = createSurfaceMaterial(
        this.shared,
        this.colorFor(entry.nodeRef),
        this.displayMode === "wireframe" ? "wireframe" : "shaded",
      );
      piece.materials.push(material);
      piece.disposables.push(material);
      // The shaded mesh attaches NOW; the edge overlay follows the policy:
      // inline when small, off-thread (or idle-time) when large, skipped
      // above the cap (edgePolicy.ts).
      tag(new THREE.Mesh(geometry, material));
      if (this.displayMode === "shaded_edges") {
        switch (edgePolicy(piece.triangles)) {
          case "inline":
            this.attachMeshEdges(piece, geometry, group);
            break;
          case "deferred":
            if (this.edgeBuilder.offThread) {
              this.attachMeshEdgesOffThread(piece, batch, group);
            } else {
              this.deferEdgesOnMainThread(outputKey(entry.nodeRef, entry.output), piece, () =>
                this.attachMeshEdges(piece, geometry, group),
              );
            }
            break;
          case "skip":
            piece.edgesSkipped = true;
            break;
        }
      }
    } else if (kind === "curve") {
      const material = createFlatMaterial(this.shared, { color: this.theme.curve });
      piece.materials.push(material);
      piece.disposables.push(material);
      tag(new THREE.LineSegments(geometry, material));
    } else {
      const material = createFlatMaterial(this.shared, {
        color: this.theme.point,
        pointSize: POINT_SIZE_PX * this.pixelRatio,
        roundPoints: true,
      });
      piece.materials.push(material);
      piece.disposables.push(material);
      tag(new THREE.Points(geometry, material));
    }
    return piece;
  }

  private buildInstancesPiece(
    entry: OutputEntry,
    inst: InstancesFrame,
    group: THREE.Group,
  ): Piece | null {
    const blob = this.blobGeometry(inst.hash);
    if (blob === null) {
      this.callbacks.notice(
        "error",
        `viewport: instances for ${outputKey(entry.nodeRef, entry.output)} reference mesh blob ${inst.hash.slice(0, 12)}… that never arrived`,
      );
      return null;
    }
    const count = inst.instances.length;
    // A per-output geometry SHARING the blob's attribute buffers, plus the
    // per-instance pick ids (the blob itself stays untouched and cached).
    const geometry = new THREE.BufferGeometry();
    const position = blob.getAttribute("position");
    geometry.setAttribute("position", position);
    if (blob.index !== null) geometry.setIndex(blob.index);
    const picks = new Float32Array(count);
    for (let i = 0; i < count; i++) picks[i] = inst.instances[i]?.pickId ?? 0;
    geometry.setAttribute("instancePick", new THREE.InstancedBufferAttribute(picks, 1));
    const material = createSurfaceMaterial(
      this.shared,
      this.colorFor(entry.nodeRef),
      this.displayMode === "wireframe" ? "wireframe" : "shaded",
    );
    const mesh = new THREE.InstancedMesh(geometry, material, count);
    const m = new THREE.Matrix4();
    for (let i = 0; i < count; i++) {
      const t = inst.instances[i]?.transform;
      if (t === undefined) continue;
      m.set(
        t[0] ?? 1,
        t[1] ?? 0,
        t[2] ?? 0,
        t[3] ?? 0,
        t[4] ?? 0,
        t[5] ?? 1,
        t[6] ?? 0,
        t[7] ?? 0,
        t[8] ?? 0,
        t[9] ?? 0,
        t[10] ?? 1,
        t[11] ?? 0,
        0,
        0,
        0,
        1,
      );
      mesh.setMatrixAt(i, m);
    }
    mesh.instanceMatrix.needsUpdate = true;
    mesh.frustumCulled = false;
    mesh.layers.set(LAYER_PICKABLE);
    mesh.userData.instances = inst;
    group.add(mesh);
    const blobTriangles = Math.floor((blob.index?.count ?? 0) / 3);
    const triangles = blobTriangles * count;
    const piece: Piece = {
      objects: [mesh],
      disposables: [geometry, material, mesh],
      materials: [material],
      triangles,
      edgesSkipped: false,
      edgeJob: null,
      edgeHandle: null,
      dropped: false,
    };
    if (this.displayMode === "shaded_edges") {
      if (triangles >= EDGE_TRIANGLE_LIMIT) {
        piece.edgesSkipped = true;
      } else {
        // The CPU cost is the BLOB's edge pass (cached per hash); the
        // instanced draw itself is free — so the blob size picks the policy
        // (inside withBlobEdges). The main-thread fallback for a large blob
        // waits for quiet like any other deferred build.
        const attach = (edgeSource: THREE.BufferGeometry) =>
          this.attachInstancedEdges(piece, edgeSource, mesh, group);
        const large = !this.blobEdges.has(inst.hash) && edgePolicy(blobTriangles) === "deferred";
        if (large && !this.edgeBuilder.offThread) {
          this.deferEdgesOnMainThread(outputKey(entry.nodeRef, entry.output), piece, () =>
            this.withBlobEdges(inst.hash, blob, piece, attach),
          );
        } else {
          this.withBlobEdges(inst.hash, blob, piece, attach);
        }
      }
    }
    return piece;
  }

  /** Rebuild every drawable (display mode / theme / graph rename). */
  rebuildAll(): void {
    for (const drawables of this.drawables.values()) {
      for (const piece of drawables.pieces.values()) this.dropPiece(drawables.group, piece);
      drawables.pieces.clear();
      this.syncPieces(drawables);
    }
    this.requestRender();
  }

  // -------------------------------------------------------------- settings --

  setDisplayMode(mode: DisplayMode): void {
    if (mode === this.displayMode) return;
    this.displayMode = mode;
    this.rebuildAll();
  }

  setNavigation(preset: NavigationPreset): void {
    // Left button is always picking. Rhino: RMB orbit, Shift+RMB pan
    // (OrbitControls pans when a modifier is held with the rotate button),
    // wheel zoom-to-cursor. Blender: MMB orbit, Shift+MMB pan.
    if (preset === "blender") {
      this.controls.mouseButtons = { LEFT: null, MIDDLE: THREE.MOUSE.ROTATE, RIGHT: null };
    } else {
      this.controls.mouseButtons = {
        LEFT: null,
        MIDDLE: THREE.MOUSE.DOLLY,
        RIGHT: THREE.MOUSE.ROTATE,
      };
    }
  }

  setTheme(theme: ThemeColors): void {
    this.theme = theme;
    this.renderer.setClearColor(theme.background, 1);
    this.shared.uAccent.value.copy(theme.accent);
    this.rebuildHelpers(this.gridStep, this.gridCenter);
    this.rebuildAll();
  }

  /** Node names changed (graph update): refresh per-node colors. */
  recolor(): void {
    for (const drawables of this.drawables.values()) {
      const color = this.colorFor(drawables.entry.nodeRef);
      for (const piece of drawables.pieces.values()) {
        for (const object of piece.objects) {
          if (object instanceof THREE.Mesh) {
            const material = object.material as CicadaMaterial;
            material.cicada.uColor.value.copy(color);
          }
        }
      }
    }
    this.requestRender();
  }

  // ------------------------------------------------------------ highlights --

  setNodeHighlight(nodeRefs: Set<number>): void {
    this.highlightedNodes = nodeRefs;
    for (const drawables of this.drawables.values()) this.applyNodeHighlight(drawables);
    this.requestRender();
  }

  private applyNodeHighlight(drawables: OutputDrawables): void {
    const on = this.highlightedNodes.has(drawables.entry.nodeRef) ? 1 : 0;
    for (const piece of drawables.pieces.values()) {
      for (const material of piece.materials) material.cicada.uNodeHighlight.value = on;
    }
  }

  setPickHighlight(selected: number | null, hover: number | null): void {
    this.highlightedPick = selected;
    this.hoverPick = hover;
    this.shared.uPickSelected.value = selected ?? -1;
    this.shared.uPickHover.value = hover ?? -1;
    this.requestRender();
  }

  // --------------------------------------------------------------- picking --

  /** The pick id under canvas-relative CSS pixel (x, y); 0 = nothing. */
  pickAt(x: number, y: number): number {
    if (this.disposed) return 0;
    const width = this.container.clientWidth;
    const height = this.container.clientHeight;
    if (width < 1 || height < 1) return 0;
    this.pickPasses += 1;
    const px = Math.min(Math.max(Math.floor(x), 0), width - 1);
    const py = Math.min(Math.max(Math.floor(y), 0), height - 1);
    this.camera.setViewOffset(width, height, px, py, 1, 1);
    this.camera.layers.set(LAYER_PICKABLE);
    const clear = this.renderer.getClearColor(new THREE.Color());
    const clearAlpha = this.renderer.getClearAlpha();
    this.renderer.setClearColor(0x000000, 0);
    this.scene.overrideMaterial = this.pickMaterial;
    this.renderer.setRenderTarget(this.pickTarget);
    this.renderer.render(this.scene, this.camera);
    this.renderer.readRenderTargetPixels(this.pickTarget, 0, 0, 1, 1, this.pickPixel);
    this.renderer.setRenderTarget(null);
    this.scene.overrideMaterial = null;
    this.renderer.setClearColor(clear, clearAlpha);
    this.camera.layers.enableAll();
    this.camera.clearViewOffset();
    return decodePickPixel(this.pickPixel);
  }

  private resolve(pickId: number): ScenePick | null {
    if (pickId === 0) return null;
    const target = this.store.resolvePick(pickId);
    if (target === null) return null;
    return { pickId, ...target };
  }

  private onPointerMove = (event: PointerEvent) => {
    const rect = this.canvas.getBoundingClientRect();
    this.hover.x = event.clientX - rect.left;
    this.hover.y = event.clientY - rect.top;
    this.hover.inside = true;
    if (event.buttons !== 0) return; // dragging: no hover picks
    this.scheduleHover();
  };

  private onPointerLeave = () => {
    this.hover.inside = false;
    if (this.hover.lastId !== 0) {
      this.hover.lastId = 0;
      this.callbacks.onHover(null);
    }
  };

  private scheduleHover(): void {
    if (this.hover.pending) return;
    this.hover.pending = true;
    this.hoverTimer = setTimeout(() => {
      this.hoverTimer = null;
      this.hover.pending = false;
      if (this.disposed || !this.hover.inside) return;
      const id = this.pickAt(this.hover.x, this.hover.y);
      if (id !== this.hover.lastId) {
        this.hover.lastId = id;
        this.callbacks.onHover(this.resolve(id));
      }
    }, HOVER_INTERVAL_MS);
  }

  private onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) {
      this.down = null;
      return;
    }
    const rect = this.canvas.getBoundingClientRect();
    this.down = { x: event.clientX - rect.left, y: event.clientY - rect.top, button: 0 };
  };

  private onPointerUp = (event: PointerEvent) => {
    if (event.button !== 0 || this.down === null) return;
    const rect = this.canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const moved = Math.hypot(x - this.down.x, y - this.down.y);
    this.down = null;
    if (moved > CLICK_SLOP_PX) return;
    const id = this.pickAt(x, y);
    this.callbacks.onClick(this.resolve(id));
  };

  // --------------------------------------------------------------- framing --

  private gridStep = 1;
  private gridCenter = new THREE.Vector3();

  frameBounds(bounds: Bounds | null): void {
    if (bounds === null) {
      this.callbacks.notice("info", "viewport: nothing to frame");
      return;
    }
    const { center, radius } = sphereOfBounds(bounds);
    this.sceneRadius = radius;
    const direction = new THREE.Vector3().subVectors(this.camera.position, this.controls.target);
    if (direction.lengthSq() < 1e-12) direction.set(0.6, -0.75, 0.5);
    direction.normalize();
    const distance = fitDistance(radius, this.camera.fov, this.camera.aspect);
    this.controls.target.copy(center);
    this.camera.position.copy(center).addScaledVector(direction, distance);
    this.camera.lookAt(center);
    this.controls.update();
    this.rebuildHelpers(niceGridStep(radius * 4, 20), center);
    this.requestRender();
  }

  frameAll(): void {
    this.frameBounds(this.store.bounds());
  }

  frameNodes(nodeRefs: Set<number>): void {
    const bounds = nodeRefs.size > 0 ? this.store.bounds(nodeRefs) : null;
    this.frameBounds(bounds ?? this.store.bounds());
  }

  private disposeHelpers(): void {
    for (const child of Array.from(this.helpers.children)) {
      this.helpers.remove(child);
      if (child instanceof THREE.GridHelper || child instanceof THREE.AxesHelper) child.dispose();
    }
  }

  /** Ground grid in the XY plane (Z up) + origin axes, sized to the scene. */
  private rebuildHelpers(step: number, center: THREE.Vector3): void {
    this.disposeHelpers();
    this.gridStep = step;
    this.gridCenter.copy(center);
    const divisions = 20;
    const grid = new THREE.GridHelper(
      step * divisions,
      divisions,
      this.theme.gridStrong,
      this.theme.grid,
    );
    grid.rotation.x = Math.PI / 2; // GridHelper spans XZ; the engine's ground is XY
    grid.position.set(Math.round(center.x / step) * step, Math.round(center.y / step) * step, 0);
    grid.layers.set(LAYER_DECOR);
    const gridMaterial = grid.material as THREE.Material;
    gridMaterial.transparent = true;
    gridMaterial.opacity = 0.9;
    gridMaterial.depthWrite = false;
    this.helpers.add(grid);
    const axes = new THREE.AxesHelper(step * 2);
    axes.layers.set(LAYER_DECOR);
    this.helpers.add(axes);
  }

  // ------------------------------------------------------------- rendering --

  requestRender(): void {
    if (this.disposed || this.renderScheduled) return;
    this.renderScheduled = true;
    requestAnimationFrame(this.tick);
  }

  private tick = () => {
    this.renderScheduled = false;
    if (this.disposed) return;
    const moving = this.controls.update();
    this.render();
    if (moving || this.interacting) this.requestRender();
  };

  /** One synchronous render of the main view. */
  render(): void {
    if (this.disposed) return;
    const distance = this.camera.position.distanceTo(this.controls.target);
    this.camera.near = Math.max(1e-4, Math.min(distance, this.sceneRadius) * 0.002);
    this.camera.far = (distance + this.sceneRadius * 4) * 8;
    this.camera.updateProjectionMatrix();
    this.renderer.setRenderTarget(null);
    this.renderer.render(this.scene, this.camera);
    this.lastDrawCalls = this.renderer.info.render.calls;
    this.renders += 1;
    this.callbacks.onRendered();
  }

  /** Render now and encode the canvas as PNG (no preserveDrawingBuffer needed). */
  screenshot(): Promise<Blob> {
    return new Promise((resolve, reject) => {
      if (this.disposed) {
        reject(new Error("viewport disposed"));
        return;
      }
      this.render();
      this.canvas.toBlob((blob) => {
        if (blob === null) reject(new Error("canvas.toBlob returned null"));
        else resolve(blob);
      }, "image/png");
    });
  }

  // ----------------------------------------------------------------- stats --

  stats(): ExtendedStats {
    const outputs: ViewportStats["outputs"] = {};
    let bounds: Bounds | null = null;
    let edgesSkipped = 0;
    let edgesPending = 0;
    for (const [key, entry] of this.store.outputs) {
      const s = this.store.statsOf(entry);
      outputs[key] = s;
      bounds = unionBounds(bounds, s.bounds);
      const drawables = this.drawables.get(key);
      if (drawables !== undefined) {
        for (const piece of drawables.pieces.values()) {
          if (piece.edgesSkipped) edgesSkipped += 1;
          if (piece.edgeJob !== null || piece.edgeHandle !== null) edgesPending += 1;
        }
      }
    }
    return {
      outputs,
      bounds,
      drawCalls: this.lastDrawCalls,
      framesReceived: this.store.framesReceived,
      lastGeneration: this.store.lastGeneration,
      highlighted: {
        nodes: Array.from(this.highlightedNodes),
        pickId: this.highlightedPick ?? this.hoverPick,
      },
      edgesSkipped,
      edgesPending,
      edgesDeferredBuilt: this.edgesDeferredBuilt,
      pickPasses: this.pickPasses,
      renders: this.renders,
    };
  }
}

/** A BufferGeometry over packed edge-segment positions (from the worker). */
function edgeGeometryFromPositions(positions: Float32Array): THREE.BufferGeometry {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  return geometry;
}

function pieceMatchesBatch(piece: Piece, batch: BatchFrame): boolean {
  return piece.objects[0]?.userData.batch === batch;
}

function pieceMatchesInstances(piece: Piece, inst: InstancesFrame): boolean {
  return piece.objects[0]?.userData.instances === inst;
}
