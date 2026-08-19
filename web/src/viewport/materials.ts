/**
 * Viewport materials — small custom `ShaderMaterial`s, no
 * `MeshStandardMaterial` (docs/16 §Viewport conventions):
 *
 * - `SurfaceMaterial`: flat shading from screen-space derivatives (no
 *   normals are transmitted — docs/13), a key light + headlight +
 *   hemisphere ambient, per-node base color, node/element highlight tints;
 * - `FlatMaterial`: unlit lines / points (curves, points, edge overlays),
 *   same highlight uniforms; optionally instanced (edge overlays of
 *   instanced blobs carry their own `instanceMatrix` attribute);
 * - `PickMaterial`: the ID-buffer override — writes the per-vertex (or
 *   per-instance) pick id as RGB8.
 *
 * Highlight uniforms shared by every material live in one `SharedUniforms`
 * object so a selection change is one assignment, not a scene walk.
 */
import * as THREE from "three";
import { GLSL_ENCODE_PICK } from "./picking";

export interface SharedUniforms {
  uAccent: { value: THREE.Color };
  uPickSelected: { value: number };
  uPickHover: { value: number };
}

export function createSharedUniforms(accent: THREE.Color): SharedUniforms {
  return {
    uAccent: { value: accent },
    uPickSelected: { value: -1 },
    uPickHover: { value: -1 },
  };
}

const PICK_VARYING_VERTEX = /* glsl */ `
attribute float pickId;
#ifdef USE_INSTANCING
attribute float instancePick;
#endif
varying float vPick;
`;

const SURFACE_VERTEX = /* glsl */ `
${PICK_VARYING_VERTEX}
varying vec3 vViewPos;
void main() {
  vec4 p = vec4(position, 1.0);
  #ifdef USE_INSTANCING
    p = instanceMatrix * p;
    vPick = instancePick;
  #else
    vPick = pickId;
  #endif
  vec4 mv = modelViewMatrix * p;
  vViewPos = mv.xyz;
  gl_Position = projectionMatrix * mv;
}
`;

const HIGHLIGHT_FRAGMENT = /* glsl */ `
uniform vec3 uAccent;
uniform float uNodeHighlight;
uniform float uPickSelected;
uniform float uPickHover;
varying float vPick;
// Highlight strength: the strongest of node-selected / element-hovered /
// element-selected (not additive), applied to the BASE color so shading
// survives on a highlighted solid.
vec3 cicadaHighlight(vec3 base) {
  float t = 0.0;
  if (uNodeHighlight > 0.5) t = 0.35;
  if (abs(vPick - uPickHover) < 0.5) t = max(t, 0.3);
  if (abs(vPick - uPickSelected) < 0.5) t = max(t, 0.55);
  return mix(base, uAccent, t);
}
`;

const SURFACE_FRAGMENT = /* glsl */ `
uniform vec3 uColor;
uniform float uOpacity;
varying vec3 vViewPos;
${HIGHLIGHT_FRAGMENT}
void main() {
  vec3 base = cicadaHighlight(uColor);
  #ifdef WIREFRAME
    vec3 shaded = base * 0.9;
  #else
    // Flat normal facing the viewer, from the screen-space derivatives of
    // the view-space position (CAD-correct hard edges, no vertex normals).
    vec3 n = normalize(cross(dFdx(vViewPos), dFdy(vViewPos)));
    // World normal (transpose of the rigid view rotation) for the sky term.
    vec3 wn = vec3(dot(viewMatrix[0].xyz, n), dot(viewMatrix[1].xyz, n), dot(viewMatrix[2].xyz, n));
    float sky = 0.5 + 0.5 * wn.z;
    vec3 key = normalize(vec3(-0.35, 0.55, 1.0));
    float diffuse = max(dot(n, key), 0.0);
    float head = max(n.z, 0.0);
    float light = 0.30 + 0.20 * sky + 0.40 * diffuse + 0.14 * head;
    vec3 shaded = base * light;
    vec3 h = normalize(key + vec3(0.0, 0.0, 1.0));
    shaded += vec3(0.07) * pow(max(dot(n, h), 0.0), 28.0);
  #endif
  gl_FragColor = vec4(shaded, uOpacity);
  #include <colorspace_fragment>
}
`;

const FLAT_VERTEX = /* glsl */ `
attribute float pickId;
#ifdef CICADA_INSTANCED
attribute mat4 instanceMatrix;
#endif
uniform float uPointSize;
varying float vPick;
void main() {
  vec4 p = vec4(position, 1.0);
  #ifdef CICADA_INSTANCED
    p = instanceMatrix * p;
  #endif
  vPick = pickId;
  gl_Position = projectionMatrix * modelViewMatrix * p;
  gl_PointSize = uPointSize;
}
`;

const FLAT_FRAGMENT = /* glsl */ `
uniform vec3 uColor;
uniform float uOpacity;
${HIGHLIGHT_FRAGMENT}
void main() {
  #ifdef ROUND_POINTS
    vec2 c = gl_PointCoord - vec2(0.5);
    if (dot(c, c) > 0.25) discard;
  #endif
  gl_FragColor = vec4(cicadaHighlight(uColor), uOpacity);
  #include <colorspace_fragment>
}
`;

const PICK_VERTEX = /* glsl */ `
${PICK_VARYING_VERTEX}
uniform float uPointSize;
void main() {
  vec4 p = vec4(position, 1.0);
  #ifdef USE_INSTANCING
    p = instanceMatrix * p;
    vPick = instancePick;
  #else
    vPick = pickId;
  #endif
  gl_Position = projectionMatrix * modelViewMatrix * p;
  gl_PointSize = uPointSize;
}
`;

const PICK_FRAGMENT = /* glsl */ `
varying float vPick;
${GLSL_ENCODE_PICK}
void main() {
  gl_FragColor = cicadaEncodePick(vPick);
}
`;

export type SurfaceMode = "shaded" | "wireframe";

export interface NodeUniforms {
  uColor: { value: THREE.Color };
  uNodeHighlight: { value: number };
}

/** A per-drawable material carrying its node color + node highlight. */
export interface CicadaMaterial extends THREE.ShaderMaterial {
  cicada: NodeUniforms;
}

function withNode(material: THREE.ShaderMaterial, node: NodeUniforms): CicadaMaterial {
  const m = material as CicadaMaterial;
  m.cicada = node;
  return m;
}

export function createSurfaceMaterial(
  shared: SharedUniforms,
  color: THREE.Color,
  mode: SurfaceMode,
): CicadaMaterial {
  const node: NodeUniforms = { uColor: { value: color.clone() }, uNodeHighlight: { value: 0 } };
  const material = new THREE.ShaderMaterial({
    uniforms: { ...shared, ...node, uOpacity: { value: 1 } },
    vertexShader: SURFACE_VERTEX,
    fragmentShader: SURFACE_FRAGMENT,
    defines: mode === "wireframe" ? { WIREFRAME: "" } : {},
    wireframe: mode === "wireframe",
    side: THREE.DoubleSide,
    polygonOffset: mode !== "wireframe",
    polygonOffsetFactor: 1,
    polygonOffsetUnits: 1,
  });
  return withNode(material, node);
}

export interface FlatOptions {
  color: THREE.Color;
  /** Points only. */
  pointSize?: number;
  opacity?: number;
  /** Edge overlays of instanced blobs (own `instanceMatrix` attribute). */
  instanced?: boolean;
  /** Round point sprites (discard outside the disc). */
  roundPoints?: boolean;
  depthTest?: boolean;
}

export function createFlatMaterial(shared: SharedUniforms, options: FlatOptions): CicadaMaterial {
  const node: NodeUniforms = {
    uColor: { value: options.color.clone() },
    uNodeHighlight: { value: 0 },
  };
  const defines: Record<string, string> = {};
  if (options.instanced) defines.CICADA_INSTANCED = "";
  if (options.roundPoints) defines.ROUND_POINTS = "";
  const opacity = options.opacity ?? 1;
  const material = new THREE.ShaderMaterial({
    uniforms: {
      ...shared,
      ...node,
      uOpacity: { value: opacity },
      uPointSize: { value: options.pointSize ?? 1 },
    },
    vertexShader: FLAT_VERTEX,
    fragmentShader: FLAT_FRAGMENT,
    defines,
    transparent: opacity < 1,
    depthTest: options.depthTest ?? true,
    depthWrite: opacity >= 1,
  });
  return withNode(material, node);
}

/** The ID-buffer override material (one per renderer). */
export function createPickMaterial(pointSize: number): THREE.ShaderMaterial {
  return new THREE.ShaderMaterial({
    uniforms: { uPointSize: { value: pointSize } },
    vertexShader: PICK_VERTEX,
    fragmentShader: PICK_FRAGMENT,
    side: THREE.DoubleSide,
    blending: THREE.NoBlending,
  });
}

/** Set the point size on any material that has the uniform (pick + points). */
export function setPointSize(material: THREE.ShaderMaterial, size: number): void {
  const u = material.uniforms.uPointSize;
  if (u !== undefined) u.value = size;
}

// ---------------------------------------------------------------- colors --

/** FNV-1a over the name → a stable hue in [0, 1). */
export function stableHue(name: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < name.length; i++) {
    h ^= name.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  // Golden-ratio scatter so neighbouring names land far apart.
  return ((h / 0x100000000) * 0.618033988749895 + 0.11) % 1;
}

/**
 * Per-node surface color: a neutral CAD grey with a subtle stable hue,
 * readable against both themes.
 */
export function nodeColor(name: string, theme: "dark" | "light"): THREE.Color {
  const hue = stableHue(name);
  return theme === "dark"
    ? new THREE.Color().setHSL(hue, 0.3, 0.64)
    : new THREE.Color().setHSL(hue, 0.32, 0.6);
}

/** Parse a CSS color (`#rgb`, `#rrggbb`, `rgb(a)(…)`) into a three Color. */
export function cssColor(value: string, fallback: string): THREE.Color {
  const v = value.trim();
  const color = new THREE.Color();
  try {
    if (v === "") return color.set(fallback);
    if (v.startsWith("rgb")) {
      const parts = v
        .slice(v.indexOf("(") + 1, v.lastIndexOf(")"))
        .split(/[\s,/]+/)
        .filter((s) => s !== "")
        .map(Number);
      const [r = 0, g = 0, b = 0] = parts;
      return color.setRGB(r / 255, g / 255, b / 255, THREE.SRGBColorSpace);
    }
    return color.set(v);
  } catch {
    return color.set(fallback);
  }
}
