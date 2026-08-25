/**
 * The file browser's pure half: paths joined and split into breadcrumbs,
 * the keyboard's cursor rule, the row's date.
 */
import { describe, expect, it } from "vitest";
import { crumbsOf, dirnameOf, joinPath, modifiedText, moveCursor } from "./filePaths";

describe("paths", () => {
  it("joins with the root as the empty string", () => {
    expect(joinPath("", "a")).toBe("a");
    expect(joinPath("a", "b.cic")).toBe("a/b.cic");
    expect(joinPath("a/b", "c")).toBe("a/b/c");
  });

  it("dirnameOf", () => {
    expect(dirnameOf("p.cic")).toBe("");
    expect(dirnameOf("a/p.cic")).toBe("a");
    expect(dirnameOf("a/b/p.cic")).toBe("a/b");
    expect(dirnameOf("")).toBe("");
  });

  it("crumbs: each segment opens its prefix; the root has none", () => {
    expect(crumbsOf("")).toEqual([]);
    expect(crumbsOf("a")).toEqual([{ label: "a", dir: "a" }]);
    expect(crumbsOf("a/b/c")).toEqual([
      { label: "a", dir: "a" },
      { label: "b", dir: "a/b" },
      { label: "c", dir: "a/b/c" },
    ]);
  });
});

describe("moveCursor", () => {
  it("arrows step and clamp, Home/End jump, other keys leave it, an empty list has none", () => {
    expect(moveCursor(0, "ArrowDown", 3)).toBe(1);
    expect(moveCursor(2, "ArrowDown", 3)).toBe(2);
    expect(moveCursor(1, "ArrowUp", 3)).toBe(0);
    expect(moveCursor(0, "ArrowUp", 3)).toBe(0);
    expect(moveCursor(-1, "ArrowDown", 3), "no cursor yet: the first row").toBe(0);
    expect(moveCursor(-1, "ArrowUp", 3)).toBe(0);
    expect(moveCursor(1, "Home", 3)).toBe(0);
    expect(moveCursor(0, "End", 3)).toBe(2);
    expect(moveCursor(1, "a", 3)).toBe(1);
    expect(moveCursor(7, "ArrowDown", 3), "a cursor past the end (the list shrank) clamps").toBe(2);
    expect(moveCursor(0, "ArrowDown", 0)).toBe(-1);
  });
});

describe("modifiedText", () => {
  it("today is a time, another day a date, garbage nothing", () => {
    const now = new Date(2026, 7, 24, 15, 0, 0);
    const today = new Date(2026, 7, 24, 9, 5, 0).getTime();
    const yesterday = new Date(2026, 7, 23, 9, 5, 0).getTime();
    expect(modifiedText({ modified_ms: today }, now)).toBe(new Date(today).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }));
    expect(modifiedText({ modified_ms: yesterday }, now)).toBe(new Date(yesterday).toLocaleDateString());
    expect(modifiedText({ modified_ms: Number.NaN }, now)).toBe("");
  });
});
