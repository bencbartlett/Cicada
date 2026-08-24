/**
 * The gimbal (docs/16 §Viewport conventions; docs/17 wave 4 B1): the X/Y/Z
 * triad in the viewport's upper-left, below the toolbar overlay. It is a
 * second, tiny scene drawn by the SAME renderer into a corner viewport at
 * the end of every main render — the scene renders on demand, so the
 * gimbal costs nothing at idle — with its orthographic camera on the main
 * camera's bearing, so the triad turns with the view. Non-interactive in
 * this slice (no click-to-snap): it never sees a pointer event.
 *
 * Axis hues are the theme's `--axis-x` / `--axis-y` / `--axis-z` (X red,
 * Y green, Z blue — the CAD convention), shared with the ground-origin
 * axes. The pure parts (`gimbalRect`, `axisDirections`) are unit-tested;
 * the drawing is verified in the app (`web/e2e/visuals.spec.ts`).
 */
import * as THREE from "three";

/** The gimbal's square, in CSS pixels. */
export const GIMBAL_SIZE_PX = 72;
/**
 * Where it sits: the viewport's upper-left, under the toolbar + readout
 * overlay (`.viewport-overlay`: 6 px in, two ~20 px rows and a gap).
 */
export const GIMBAL_MARGIN_PX = { left: 6, top: 56 } as const;

export interface GimbalRect {
  /** Left edge, CSS px from the canvas's left. */
  x: number;
  /** Bottom edge, CSS px from the canvas's BOTTOM (WebGL's viewport origin). */
  y: number;
  /** Side of the square, CSS px. */
  size: number;
}

/**
 * The corner viewport for a canvas of `width` × `height` CSS px: a
 * `size` square `margin.left` in from the left and `margin.top` down from
 * the top, expressed with WebGL's bottom-left origin. Shrinks to fit a
 * canvas smaller than the margins + square (a collapsed pane), never to
 * nothing.
 */
export function gimbalRect(
  width: number,
  height: number,
  size: number = GIMBAL_SIZE_PX,
  margin: { left: number; top: number } = GIMBAL_MARGIN_PX,
): GimbalRect {
  const side = Math.max(1, Math.min(size, width - margin.left, height - margin.top));
  return { x: margin.left, y: Math.max(0, height - margin.top - side), size: side };
}

/** One world axis as the camera sees it: `[right, up, towardViewer]`, a unit vector. */
export type AxisDirection = [number, number, number];

export interface AxisDirections {
  x: AxisDirection;
  y: AxisDirection;
  z: AxisDirection;
}

/**
 * The screen-space direction of each world axis under a camera orientation
 * (the camera's world quaternion): components along the camera's right, up
 * and toward-the-viewer directions. A camera at rest (identity) sees X to
 * the right, Y up and Z pointing at it; the gimbal draws exactly these
 * directions, so `stats().gimbal` lets a test assert it follows the camera.
 */
export function axisDirections(quaternion: THREE.Quaternion): AxisDirections {
  const toCamera = quaternion.clone().invert();
  const v = new THREE.Vector3();
  const of = (x: number, y: number, z: number): AxisDirection => {
    v.set(x, y, z).applyQuaternion(toCamera);
    return [v.x, v.y, v.z];
  };
  return { x: of(1, 0, 0), y: of(0, 1, 0), z: of(0, 0, 1) };
}

export interface AxisColors {
  x: THREE.Color;
  y: THREE.Color;
  z: THREE.Color;
}

/** The unit length of an axis bar in the gimbal's own space; the camera frames ±`EXTENT`. */
const BAR_LENGTH = 0.78;
const BAR_THICKNESS = 0.07;
const TIP_RADIUS = 0.2;
const EXTENT = 1.12;
const LABEL_TEXTURE_PX = 64;

interface Axis {
  bar: THREE.Mesh<THREE.BoxGeometry, THREE.MeshBasicMaterial>;
  tip: THREE.Sprite;
}

export class Gimbal {
  readonly scene = new THREE.Scene();
  readonly camera = new THREE.OrthographicCamera(-EXTENT, EXTENT, EXTENT, -EXTENT, 0.1, 10);
  private axes: Record<"x" | "y" | "z", Axis>;
  private readonly barGeometry = new THREE.BoxGeometry(BAR_LENGTH, BAR_THICKNESS, BAR_THICKNESS);
  /** The label sprites' ink: the theme's background (a dark letter on the light disc, and vice versa). */
  private ink: THREE.Color;

  constructor(colors: AxisColors, ink: THREE.Color) {
    this.ink = ink.clone();
    this.axes = {
      x: this.makeAxis("X", colors.x, new THREE.Vector3(1, 0, 0)),
      y: this.makeAxis("Y", colors.y, new THREE.Vector3(0, 1, 0)),
      z: this.makeAxis("Z", colors.z, new THREE.Vector3(0, 0, 1)),
    };
    this.camera.position.set(0, 0, 3);
    this.camera.lookAt(0, 0, 0);
  }

  private makeAxis(letter: string, color: THREE.Color, direction: THREE.Vector3): Axis {
    const bar = new THREE.Mesh(this.barGeometry, new THREE.MeshBasicMaterial({ color }));
    // The box spans ±BAR_LENGTH/2 along its local x: slide it out so it
    // starts at the origin, then turn local x onto the axis.
    bar.position.copy(direction).multiplyScalar(BAR_LENGTH / 2);
    bar.quaternion.setFromUnitVectors(new THREE.Vector3(1, 0, 0), direction);
    this.scene.add(bar);
    const tip = new THREE.Sprite(
      new THREE.SpriteMaterial({ map: this.labelTexture(letter, color), depthTest: true, transparent: true }),
    );
    tip.position.copy(direction).multiplyScalar(BAR_LENGTH + TIP_RADIUS * 0.6);
    tip.scale.setScalar(TIP_RADIUS * 2);
    this.scene.add(tip);
    return { bar, tip };
  }

  /** A disc in the axis color with its letter in the ink color. */
  private labelTexture(letter: string, color: THREE.Color): THREE.CanvasTexture {
    const canvas = document.createElement("canvas");
    canvas.width = LABEL_TEXTURE_PX;
    canvas.height = LABEL_TEXTURE_PX;
    const context = canvas.getContext("2d");
    if (context === null) throw new Error("gimbal: no 2D canvas context for the axis labels");
    const half = LABEL_TEXTURE_PX / 2;
    context.beginPath();
    context.arc(half, half, half - 1, 0, Math.PI * 2);
    context.fillStyle = `#${color.getHexString()}`;
    context.fill();
    context.fillStyle = `#${this.ink.getHexString()}`;
    context.font = `bold ${Math.round(LABEL_TEXTURE_PX * 0.62)}px ui-sans-serif, system-ui, sans-serif`;
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText(letter, half, half + LABEL_TEXTURE_PX * 0.04);
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    return texture;
  }

  /** Recolor for a theme change (the bars' materials, the label discs). */
  setColors(colors: AxisColors, ink: THREE.Color): void {
    this.ink = ink.clone();
    const letters = { x: "X", y: "Y", z: "Z" } as const;
    for (const key of ["x", "y", "z"] as const) {
      const axis = this.axes[key];
      axis.bar.material.color.copy(colors[key]);
      axis.tip.material.map?.dispose();
      axis.tip.material.map = this.labelTexture(letters[key], colors[key]);
      axis.tip.material.needsUpdate = true;
    }
  }

  /**
   * Put the gimbal's camera on the main camera's bearing: same orientation,
   * looking at the gimbal's origin from a fixed distance.
   */
  follow(camera: THREE.Camera): void {
    this.camera.quaternion.copy(camera.quaternion);
    this.camera.position.set(0, 0, 3).applyQuaternion(camera.quaternion);
    this.camera.updateMatrixWorld();
  }

  /**
   * What the gimbal DRAWS: each world axis as the gimbal's OWN camera sees
   * it. This — never the main camera's pose — is what `stats().gimbal`
   * reports, so a gimbal that stopped following the view (a `follow()`
   * that does nothing) reads as frozen to a test while the main camera
   * turns (review finding, 2026-08-24).
   */
  directions(): AxisDirections {
    return axisDirections(this.camera.quaternion);
  }

  /**
   * Draw over the finished main render, into the corner rectangle of a
   * `width` × `height` CSS-px canvas. The color buffer is kept and the
   * depth buffer cleared, so the triad sits on top of the geometry; the
   * renderer's viewport and scissor are restored to the full canvas.
   */
  render(renderer: THREE.WebGLRenderer, width: number, height: number): void {
    const rect = gimbalRect(width, height);
    renderer.autoClear = false;
    renderer.clearDepth();
    renderer.setViewport(rect.x, rect.y, rect.size, rect.size);
    renderer.setScissor(rect.x, rect.y, rect.size, rect.size);
    renderer.setScissorTest(true);
    renderer.render(this.scene, this.camera);
    renderer.setScissorTest(false);
    renderer.setViewport(0, 0, width, height);
    renderer.setScissor(0, 0, width, height);
    renderer.autoClear = true;
  }

  dispose(): void {
    for (const axis of Object.values(this.axes)) {
      axis.bar.material.dispose();
      axis.tip.material.map?.dispose();
      axis.tip.material.dispose();
      this.scene.remove(axis.bar, axis.tip);
    }
    this.barGeometry.dispose();
  }
}
