import { describe, expect, it } from "vitest";
import type { InputView } from "../protocol/messages";
import { COLLAPSED_ROW_PORTS, collapseHint } from "./collapse";

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
const wire = { node: "size", port: "out" };

describe("collapseHint (the mirror of viewmodel::collapse_refusal)", () => {
  it("a slider with literal bounds collapses", () => {
    expect(collapseHint({ func: "slider", inputs: [port("value"), port("min"), port("max"), port("step")] })).toBeNull();
  });
  it("a wired min / max / step is the hint — the ports named in spec order, `is` or `are`", () => {
    expect(collapseHint({ func: "slider", inputs: [port("value"), port("min"), port("max", wire), port("step")] })).toBe(
      "max is wired",
    );
    expect(collapseHint({ func: "slider", inputs: [port("step", wire), port("max"), port("min", wire)] })).toBe(
      "min and step are wired",
    );
  });
  it("a wired value is refused too — the collapsed row IS the track (the server says the same)", () => {
    expect(collapseHint({ func: "slider", inputs: [port("value", { node: "n", port: "out" }), port("min"), port("max")] })).toBe(
      "value is wired",
    );
    expect(COLLAPSED_ROW_PORTS).toEqual(["value", "min", "max", "step"]);
  });
  it("the rule keys on the called node, not on the widget: a slider without a widget is still a slider", () => {
    // `driven = slider(value=n)` has no `param` (no literal value to drag) but is a slider.
    expect(collapseHint({ func: "slider", inputs: [port("value", wire)] })).toBe("value is wired");
  });
  it("nothing but a slider collapses", () => {
    expect(collapseHint({ inputs: [port("x")] })).toBe("not a slider");
    expect(collapseHint({ func: "toggle", inputs: [] })).toBe("not a slider");
    expect(collapseHint({ func: "construct_domain", inputs: [port("start"), port("end", wire)] })).toBe("not a slider");
  });
});
