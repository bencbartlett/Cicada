/**
 * The GH slider shortcut grammar (wave 4 B4, finding U10): `A<B` and
 * `A<B<C`, negatives allowed, the typed precision as the step, whole numbers
 * as the integer slider; the range rules carried as a `problem`; the
 * literals spelled by the one rule in port order; what the row previews.
 */
import { describe, expect, it } from "vitest";
import { parseSliderShortcut, sliderShortcutParams, sliderShortcutSummary } from "./sliderShortcut";

describe("parseSliderShortcut", () => {
  it("`A<B`: min A, max B, value A; whole numbers → step 1, the integer slider", () => {
    expect(parseSliderShortcut("1<20")).toEqual({
      min: 1,
      max: 20,
      value: 1,
      decimals: 0,
      step: 1,
      integer: true,
      problem: null,
    });
  });

  it("`A<B<C`: min A, value B, max C; the most decimals typed set the step", () => {
    expect(parseSliderShortcut("0.0<0.5<1.0")).toMatchObject({ min: 0, value: 0.5, max: 1, decimals: 1, step: 0.1, integer: false });
    expect(parseSliderShortcut("0<0.25<1")).toMatchObject({ decimals: 2, step: 0.01, integer: false });
    expect(parseSliderShortcut("0.000<5")).toMatchObject({ decimals: 3, step: 0.001 });
    expect(parseSliderShortcut(".5<1")).toMatchObject({ min: 0.5, max: 1, value: 0.5, decimals: 1, step: 0.1 });
  });

  it("negatives and whitespace are allowed", () => {
    expect(parseSliderShortcut(" -1.5 < 0 < 1.5 ")).toMatchObject({ min: -1.5, value: 0, max: 1.5, step: 0.1, problem: null });
    expect(parseSliderShortcut("-20<-1")).toMatchObject({ min: -20, max: -1, value: -20, integer: true, problem: null });
  });

  it("anything else is an ordinary search — including a partial shortcut", () => {
    for (const q of ["", "series", "1<", "<20", "1<20<", "1<<20", "a<b", "1<2<3<4", "1.<2", "1e3<5", "+1<2"]) {
      expect(parseSliderShortcut(q), q).toBeNull();
    }
  });

  it("min must be below max and value within them — the problem names the numbers as typed", () => {
    expect(parseSliderShortcut("5<2")?.problem).toBe("min 5 must be below max 2");
    expect(parseSliderShortcut("3<3")?.problem).toBe("min 3 must be below max 3");
    expect(parseSliderShortcut("0<7<5")?.problem).toBe("value 7 is outside 0 … 5");
    expect(parseSliderShortcut("0.0<-0.1<1.0")?.problem).toBe("value -0.1 is outside 0.0 … 1.0");
    expect(parseSliderShortcut("1<1<5")?.problem).toBeNull();
    expect(parseSliderShortcut("1<5<5")?.problem).toBeNull();
  });
});

describe("sliderShortcutParams / sliderShortcutSummary", () => {
  it("spells every literal as a Number (the slider's ports are Number) in port order", () => {
    expect(sliderShortcutParams(parseSliderShortcut("1<20")!)).toEqual([
      { port: "value", value: "1.0" },
      { port: "min", value: "1.0" },
      { port: "max", value: "20.0" },
      { port: "step", value: "1.0" },
    ]);
    expect(sliderShortcutParams(parseSliderShortcut("-1.5<0<1.5")!)).toEqual([
      { port: "value", value: "0.0" },
      { port: "min", value: "-1.5" },
      { port: "max", value: "1.5" },
      { port: "step", value: "0.1" },
    ]);
    expect(sliderShortcutParams(parseSliderShortcut("0<0.25<1")!).at(-1)).toEqual({ port: "step", value: "0.01" });
  });

  it("previews the slider it will make", () => {
    expect(sliderShortcutSummary(parseSliderShortcut("1<20")!)).toEqual({
      title: "integer slider 1.0 … 20.0",
      detail: "value 1.0 · step 1.0",
    });
    expect(sliderShortcutSummary(parseSliderShortcut("0.0<0.5<1.0")!)).toEqual({
      title: "slider 0.0 … 1.0",
      detail: "value 0.5 · step 0.1",
    });
  });
});
