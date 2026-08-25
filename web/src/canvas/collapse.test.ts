import { describe, expect, it } from "vitest";
import type { InputView, ParamView } from "../protocol/messages";
import { collapseHint } from "./collapse";

const port = (name: string, wired?: { node: string; port: string }): InputView => ({
  name,
  type: "Number",
  base: "Number",
  depth: 0,
  optional: false,
  required: false,
  lift: 0,
  ...(wired === undefined ? {} : { wired }),
});
const slider: ParamView = { kind: "slider", port: "value", value: 1, min: 0, max: 10, step: 0 };

describe("collapseHint (the mirror of viewmodel::collapse_refusal)", () => {
  it("a slider with literal bounds collapses", () => {
    expect(collapseHint({ param: slider, inputs: [port("value"), port("min"), port("max"), port("step")] })).toBeNull();
  });
  it("a wired min / max / step is the hint — the bounds named, `is` or `are`", () => {
    const wire = { node: "size", port: "out" };
    expect(collapseHint({ param: slider, inputs: [port("value"), port("min"), port("max", wire), port("step")] })).toBe(
      "max is wired",
    );
    expect(collapseHint({ param: slider, inputs: [port("min", wire), port("max"), port("step", wire)] })).toBe(
      "min and step are wired",
    );
  });
  it("a wired value is the widget's business, not a collapse refusal", () => {
    expect(collapseHint({ param: slider, inputs: [port("value", { node: "n", port: "out" }), port("min"), port("max")] })).toBeNull();
  });
  it("nothing but a slider collapses", () => {
    expect(collapseHint({ inputs: [port("x")] })).toBe("not a slider");
    expect(collapseHint({ param: { kind: "toggle", port: "value", value: true }, inputs: [] })).toBe("not a slider");
  });
});
