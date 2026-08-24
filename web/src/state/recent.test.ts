/**
 * File → Recent's list (docs/16 §Application layout): most recent first,
 * deduplicated, capped at ten, tolerant of a missing, refusing or
 * corrupted storage — never a throw on the way to a menu.
 */
import { describe, expect, it } from "vitest";
import { RECENT_KEY, RECENT_LIMIT, forgetRecent, readRecent, rememberRecent, type StorageLike } from "./recent";

function memoryStorage(initial: Record<string, string> = {}): StorageLike & { data: Map<string, string> } {
  const data = new Map(Object.entries(initial));
  return {
    data,
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => {
      data.set(key, value);
    },
  };
}

describe("recent pipelines", () => {
  it("starts empty, remembers most-recent-first, moves a repeat to the front", () => {
    const storage = memoryStorage();
    expect(readRecent(storage)).toEqual([]);
    expect(rememberRecent(storage, "a.cic")).toEqual(["a.cic"]);
    expect(rememberRecent(storage, "sub/b.cic")).toEqual(["sub/b.cic", "a.cic"]);
    expect(rememberRecent(storage, "a.cic")).toEqual(["a.cic", "sub/b.cic"]);
    expect(readRecent(storage), "persisted under the key").toEqual(["a.cic", "sub/b.cic"]);
    expect(JSON.parse(storage.data.get(RECENT_KEY)!)).toEqual(["a.cic", "sub/b.cic"]);
  });

  it("keeps the last ten", () => {
    const storage = memoryStorage();
    for (let i = 0; i < RECENT_LIMIT + 3; i += 1) rememberRecent(storage, `p${i}.cic`);
    const list = readRecent(storage);
    expect(list).toHaveLength(RECENT_LIMIT);
    expect(list[0]).toBe(`p${RECENT_LIMIT + 2}.cic`);
    expect(list).not.toContain("p0.cic");
  });

  it("forgets an entry", () => {
    const storage = memoryStorage();
    rememberRecent(storage, "a.cic");
    rememberRecent(storage, "b.cic");
    expect(forgetRecent(storage, "a.cic")).toEqual(["b.cic"]);
    expect(forgetRecent(storage, "zzz.cic"), "forgetting what is not there changes nothing").toEqual(["b.cic"]);
  });

  it("reads a corrupted or foreign value as empty and never throws", () => {
    expect(readRecent(memoryStorage({ [RECENT_KEY]: "not json" }))).toEqual([]);
    expect(readRecent(memoryStorage({ [RECENT_KEY]: JSON.stringify({ a: 1 }) }))).toEqual([]);
    expect(readRecent(memoryStorage({ [RECENT_KEY]: JSON.stringify(["ok.cic", 3, null, "", "two.cic"]) }))).toEqual([
      "ok.cic",
      "two.cic",
    ]);
    const oversized = memoryStorage({ [RECENT_KEY]: JSON.stringify(Array.from({ length: 30 }, (_, i) => `p${i}.cic`)) });
    expect(readRecent(oversized)).toHaveLength(RECENT_LIMIT);
  });

  it("a missing storage reads empty and remembers nothing; a refusing one is survived", () => {
    expect(readRecent(null)).toEqual([]);
    expect(rememberRecent(null, "a.cic")).toEqual(["a.cic"]);
    const refusing: StorageLike = {
      getItem: () => {
        throw new Error("SecurityError");
      },
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
    };
    expect(readRecent(refusing)).toEqual([]);
    expect(rememberRecent(refusing, "a.cic")).toEqual(["a.cic"]);
  });
});
