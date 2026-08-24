// @vitest-environment jsdom
import * as THREE from "three";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GIMBAL_MARGIN_PX, GIMBAL_SIZE_PX, Gimbal, axisDirections, gimbalRect } from "./gimbal";

const close = (actual: readonly number[], expected: readonly number[]) => {
  expect(actual).toHaveLength(expected.length);
  actual.forEach((v, i) => expect(v).toBeCloseTo(expected[i]!, 9));
};

describe("gimbalRect — the upper-left corner with WebGL's bottom-left origin", () => {
  it("sits GIMBAL_MARGIN_PX in from the left and down from the top", () => {
    const rect = gimbalRect(800, 600);
    expect(rect).toEqual({
      x: GIMBAL_MARGIN_PX.left,
      y: 600 - GIMBAL_MARGIN_PX.top - GIMBAL_SIZE_PX,
      size: GIMBAL_SIZE_PX,
    });
    // The top edge in CSS px from the top is the margin.
    expect(600 - (rect.y + rect.size)).toBe(GIMBAL_MARGIN_PX.top);
  });
  it("takes explicit size and margins", () => {
    expect(gimbalRect(400, 300, 50, { left: 10, top: 20 })).toEqual({ x: 10, y: 230, size: 50 });
  });
  it("shrinks to fit a collapsed pane and never vanishes", () => {
    expect(gimbalRect(40, 300)).toEqual({ x: 6, y: 300 - 56 - 34, size: 34 });
    expect(gimbalRect(300, 60)).toEqual({ x: 6, y: 0, size: 4 });
    expect(gimbalRect(1, 1).size).toBe(1);
  });
});

describe("axisDirections — what the gimbal draws, as the camera sees the world axes", () => {
  it("at rest: X right, Y up, Z toward the viewer", () => {
    const d = axisDirections(new THREE.Quaternion());
    close(d.x, [1, 0, 0]);
    close(d.y, [0, 1, 0]);
    close(d.z, [0, 0, 1]);
  });
  it("a camera turned a quarter about Y sees X coming at it and Z pointing left", () => {
    const q = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), Math.PI / 2);
    const d = axisDirections(q);
    close(d.x, [0, 0, 1]);
    close(d.y, [0, 1, 0]);
    close(d.z, [-1, 0, 0]);
  });
  it("the viewport's default pose (Z up, looking at the origin from +x/-y/+z) puts Z straight up on screen", () => {
    const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 1000);
    camera.up.set(0, 0, 1);
    camera.position.set(7, -9, 6);
    camera.lookAt(0, 0, 0);
    const d = axisDirections(camera.quaternion);
    expect(d.z[0]).toBeCloseTo(0, 9); // no sideways lean
    expect(d.z[1]).toBeGreaterThan(0.7); // up
    expect(d.z[2]).toBeGreaterThan(0); // and slightly toward the viewer (the camera is above the ground)
    // X points right-and-away, Y left-and-away: the camera stands in the
    // +x / -y quadrant looking back at the origin.
    expect(d.x[0]).toBeGreaterThan(0.5);
    expect(d.y[0]).toBeGreaterThan(0.5);
    expect(d.x[1]).toBeLessThan(0);
    expect(d.y[1]).toBeGreaterThan(0);
    for (const axis of [d.x, d.y, d.z]) expect(Math.hypot(...axis)).toBeCloseTo(1, 9);
  });
  it("is the inverse rotation: orbiting the camera turns the triad the other way", () => {
    const quarter = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), Math.PI / 2);
    const d = axisDirections(quarter);
    // The camera rolled +90° about its view axis; the world's X now reads as screen-down.
    close(d.x, [0, -1, 0]);
    close(d.y, [1, 0, 0]);
  });
});

/** The viewport's default pose (`scene.ts`): Z up, looking at the origin from +x/-y/+z. */
function defaultCamera(): THREE.PerspectiveCamera {
  const camera = new THREE.PerspectiveCamera(45, 1, 0.01, 1000);
  camera.up.set(0, 0, 1);
  camera.position.set(7, -9, 6);
  camera.lookAt(0, 0, 0);
  return camera;
}

describe("Gimbal — directions() reports the pose follow() put on the gimbal's OWN camera", () => {
  // jsdom has no 2D canvas (the `canvas` package is not installed): the
  // label textures only need a context that accepts the drawing calls.
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
      () =>
        ({
          beginPath() {},
          arc() {},
          fill() {},
          fillText() {},
          fillStyle: "",
          font: "",
          textAlign: "",
          textBaseline: "",
        }) as unknown as CanvasRenderingContext2D,
    );
  });
  afterEach(() => vi.restoreAllMocks());

  const colors = { x: new THREE.Color("#ff0000"), y: new THREE.Color("#00ff00"), z: new THREE.Color("#0000ff") };
  const ink = new THREE.Color("#000000");

  it("at rest it draws the identity pose", () => {
    const gimbal = new Gimbal(colors, ink);
    const d = gimbal.directions();
    close(d.x, [1, 0, 0]);
    close(d.y, [0, 1, 0]);
    close(d.z, [0, 0, 1]);
    gimbal.dispose();
  });

  it("after follow(camera) it draws the main camera's view of the axes, from 3 units out looking at its origin", () => {
    const camera = defaultCamera();
    const gimbal = new Gimbal(colors, ink);
    gimbal.follow(camera);
    const drawn = gimbal.directions();
    const expected = axisDirections(camera.quaternion);
    close(drawn.x, expected.x);
    close(drawn.y, expected.y);
    close(drawn.z, expected.z);
    expect(drawn.z[1]).toBeGreaterThan(0.7); // and it is not the identity: Z reads up
    // The gimbal's camera stands 3 units from the triad's origin on the main
    // camera's bearing, looking back at it.
    expect(gimbal.camera.position.length()).toBeCloseTo(3, 9);
    const forward = new THREE.Vector3(0, 0, -1).applyQuaternion(gimbal.camera.quaternion);
    close(forward.toArray(), gimbal.camera.position.clone().negate().normalize().toArray());
    gimbal.dispose();
  });

  it("follows every call, not only the first: an orbit about Z turns X and Y and keeps Z where it was", () => {
    // The review's mutation (2026-08-24): a `follow()` that never moves the
    // gimbal's camera. `stats().gimbal` used to read the MAIN camera, so the
    // app-level test could not see it; `directions()` reads what is drawn.
    const camera = defaultCamera();
    const gimbal = new Gimbal(colors, ink);
    gimbal.follow(camera);
    const before = gimbal.directions();
    camera.position.applyAxisAngle(new THREE.Vector3(0, 0, 1), Math.PI / 2);
    camera.lookAt(0, 0, 0);
    gimbal.follow(camera);
    const after = gimbal.directions();
    expect(Math.hypot(after.x[0]! - before.x[0]!, after.x[1]! - before.x[1]!)).toBeGreaterThan(0.5);
    expect(Math.hypot(after.y[0]! - before.y[0]!, after.y[1]! - before.y[1]!)).toBeGreaterThan(0.5);
    close(after.z, before.z);
    gimbal.dispose();
  });
});
