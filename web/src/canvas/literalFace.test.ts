/**
 * The typed-literal chip's face (wave 4 B3, finding U9): what an unwired
 * literal-typed port shows, where its editor starts, and what a commit is
 * compared against — the literal as written, the catalog default spelled
 * as the dialect would (`False`, not the macro's `false`), or an empty
 * required slot — plus the one spelling rule an edit goes through.
 */
import { describe, expect, it } from "vitest";
import { chipFace, chipTitle, ofKind, spellEdit } from "./literalFace";

describe("chipFace", () => {
  it("a kwarg the text carries: shown as written, the editor starts on it, an unchanged commit is no edit", () => {
    const face = chipFace({ kind: "number", literal: "40.0", value: 40, defaultText: undefined });
    expect(face).toMatchObject({ state: "literal", text: "40.0", start: 40, startText: "40.0", spelled: "40.0" });
    // The number editor keeps the text's own spelling (`1e3`, not `1000`).
    expect(chipFace({ kind: "number", literal: "1e3", value: 1000 }).startText).toBe("1e3");
    // A Text literal is shown quoted, edited bare.
    const text = chipFace({ kind: "text", literal: '"hi there"', value: "hi there" });
    expect(text).toMatchObject({ state: "literal", text: '"hi there"', startText: "hi there", spelled: '"hi there"' });
    // A Boolean literal drives the checkbox.
    expect(chipFace({ kind: "boolean", literal: "True", value: true })).toMatchObject({
      state: "literal",
      text: "True",
      startChecked: true,
      startText: "",
    });
  });

  it("a literal of the wrong kind for the port (`\"x\"` on a Number port) is shown but the editor starts empty", () => {
    const face = chipFace({ kind: "number", literal: '"x"', value: "x" });
    expect(face).toMatchObject({ state: "literal", text: '"x"', start: null, startText: "", spelled: '"x"' });
  });

  it("a default the text omits: greyed, spelled as the dialect writes it, the editor starts on it", () => {
    // The macro renders a Rust `true`; the chip says `True`, and an
    // unchanged Enter compares against `True`.
    expect(chipFace({ kind: "boolean", defaultText: "true", defaultValue: true })).toMatchObject({
      state: "default",
      text: "True",
      startChecked: true,
      spelled: "True",
    });
    expect(chipFace({ kind: "integer", defaultText: "8", defaultValue: 8 })).toMatchObject({
      state: "default",
      text: "8",
      startText: "8",
      spelled: "8",
    });
    expect(chipFace({ kind: "number", defaultText: "4.0", defaultValue: 4 })).toMatchObject({
      state: "default",
      text: "4.0",
      startText: "4",
      spelled: "4.0",
    });
    expect(chipFace({ kind: "text", defaultText: '"DejaVu Sans Bold"', defaultValue: "DejaVu Sans Bold" })).toMatchObject({
      state: "default",
      text: '"DejaVu Sans Bold"',
      startText: "DejaVu Sans Bold",
      spelled: '"DejaVu Sans Bold"',
    });
  });

  it("a default the server could not parse shows its rendering and starts empty; a required port with nothing is an empty slot", () => {
    expect(chipFace({ kind: "number", defaultText: "some_expr" })).toMatchObject({
      state: "default",
      text: "some_expr",
      start: null,
      startText: "",
      spelled: null,
    });
    expect(chipFace({ kind: "number" })).toMatchObject({ state: "unset", text: "…", start: null, spelled: null });
  });
});

describe("ofKind", () => {
  it("admits only a scalar of the port's kind", () => {
    expect(ofKind("number", 1.5)).toBe(1.5);
    expect(ofKind("number", Number.NaN)).toBeNull();
    expect(ofKind("integer", 3)).toBe(3);
    expect(ofKind("integer", 2.5)).toBeNull();
    expect(ofKind("boolean", false)).toBe(false);
    expect(ofKind("boolean", "True")).toBeNull();
    expect(ofKind("text", "a")).toBe("a");
    expect(ofKind("text", 1)).toBeNull();
    expect(ofKind("text", undefined)).toBeNull();
  });
});

describe("spellEdit", () => {
  it("spells what the editor holds through the one literal rule", () => {
    expect(spellEdit("number", "40")).toEqual({ spelled: "40.0" });
    expect(spellEdit("number", " 2.5 ")).toEqual({ spelled: "2.5" });
    expect(spellEdit("integer", "3")).toEqual({ spelled: "3" });
    expect(spellEdit("boolean", true)).toEqual({ spelled: "True" });
    expect(spellEdit("boolean", false)).toEqual({ spelled: "False" });
    expect(spellEdit("text", "hi")).toEqual({ spelled: '"hi"' });
    expect(spellEdit("text", "")).toEqual({ spelled: '""' });
  });
  it("an empty number field is nothing to write; an unspellable value is a refusal, never a guess", () => {
    expect(spellEdit("number", "")).toEqual({ skip: true });
    expect(spellEdit("integer", "   ")).toEqual({ skip: true });
    expect(spellEdit("number", "abc")).toEqual({ error: '"abc" is not a valid number' });
    expect(spellEdit("integer", "2.5")).toEqual({ error: '"2.5" is not an integer' });
  });
});

describe("chipTitle", () => {
  it("names the state and the gesture", () => {
    const literal = chipFace({ kind: "number", literal: "40.0", value: 40 });
    expect(chipTitle("d.end", literal, true, "double-click")).toBe("d.end = 40.0 — double-click to edit");
    const dflt = chipFace({ kind: "boolean", defaultText: "true", defaultValue: true });
    expect(chipTitle("s.wrap", dflt, true, "click")).toBe("s.wrap: default True (not in the text) — click to type a value");
    expect(chipTitle("d.end", chipFace({ kind: "number" }), true, "double-click")).toBe(
      "d.end: required, nothing typed yet — double-click to type a value",
    );
    expect(chipTitle("d.end", literal, false, "click")).toBe("d.end = 40.0");
  });
});
