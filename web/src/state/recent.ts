/**
 * Recent pipelines (docs/16 §Application layout, File → Recent): the last
 * `RECENT_LIMIT` root-relative pipelines THIS ORIGIN opened, most recent
 * first, in `localStorage` — per origin by construction (`localStorage` is),
 * which is per served root as long as one server owns the port. A pipeline
 * is remembered when its session's `hello` arrives (the server confirmed it
 * exists and named it root-relative), not when it was asked for. Pure over
 * a `Storage`-like so tests need no browser; a missing or refusing storage
 * (a private window, a quota) reads as empty and remembers nothing — the
 * menu then says "nothing yet", never throws.
 */

export const RECENT_KEY = "cicada.recent.v1";
export const RECENT_LIMIT = 10;

export type StorageLike = Pick<Storage, "getItem" | "setItem">;

/** `window.localStorage` when the page has one it may use, else null. */
export function browserStorage(): StorageLike | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

/** The remembered list, most recent first; malformed or missing = empty. */
export function readRecent(storage: StorageLike | null): string[] {
  if (storage === null) return [];
  try {
    const raw = storage.getItem(RECENT_KEY);
    if (raw === null) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((entry): entry is string => typeof entry === "string" && entry !== "").slice(0, RECENT_LIMIT);
  } catch {
    return [];
  }
}

/** Put `pipeline` at the front (once), keep the last `RECENT_LIMIT`, write; returns the new list. */
export function rememberRecent(storage: StorageLike | null, pipeline: string): string[] {
  const next = [pipeline, ...readRecent(storage).filter((entry) => entry !== pipeline)].slice(0, RECENT_LIMIT);
  write(storage, next);
  return next;
}

/** Drop `pipeline` (a file the server no longer has); returns the new list. */
export function forgetRecent(storage: StorageLike | null, pipeline: string): string[] {
  const next = readRecent(storage).filter((entry) => entry !== pipeline);
  write(storage, next);
  return next;
}

function write(storage: StorageLike | null, list: string[]): void {
  if (storage === null) return;
  try {
    storage.setItem(RECENT_KEY, JSON.stringify(list));
  } catch {
    // storage refused (quota, a private window) — the list stays in memory for this call's caller only
  }
}
