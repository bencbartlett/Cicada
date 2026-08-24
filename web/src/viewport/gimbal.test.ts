import * as THREE from "three";
import { describe, expect, it } from "vitest";
import { GIMBAL_MARGIN_PX, GIMBAL_SIZE_PX, axisDirections, gimbalRect } from "./gimbal";

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
