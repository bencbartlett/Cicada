/**
 * The file browser's pure half (docs/16 §Application layout — the picker
 * and File → Open): root-relative paths joined and split into breadcrumbs,
 * and the keyboard's cursor rule. The component (`FileBrowser.tsx`) owns
 * the fetching and the DOM.
 */
import type { FileEntry } from "../protocol/messages";

/** `dir/name` with `""` as the root (`joinPath("", "a")` = `a`). */
export function joinPath(dir: string, name: string): string {
  return dir === "" ? name : `${dir}/${name}`;
}

/** The directory part of a root-relative path (`"a/b/p.cic"` → `"a/b"`, `"p.cic"` → `""`). */
export function dirnameOf(path: string): string {
  const at = path.lastIndexOf("/");
  return at < 0 ? "" : path.slice(0, at);
}

export interface Crumb {
  /** The segment as shown. */
  label: string;
  /** The directory this crumb opens. */
  dir: string;
}

/** Breadcrumbs for `dir` after the root: `"a/b"` → `[{a, "a"}, {b, "a/b"}]`; the root has none. */
export function crumbsOf(dir: string): Crumb[] {
  if (dir === "") return [];
  const crumbs: Crumb[] = [];
  let prefix = "";
  for (const label of dir.split("/")) {
    prefix = joinPath(prefix, label);
    crumbs.push({ label, dir: prefix });
  }
  return crumbs;
}

/**
 * Where the cursor goes on a key: arrows step and clamp (no wrap — the end
 * is a stop, as in every native list), Home/End jump; any other key leaves
 * it. An empty list has no cursor (`-1`).
 */
export function moveCursor(cursor: number, key: string, length: number): number {
  if (length === 0) return -1;
  const at = cursor < 0 ? -1 : Math.min(cursor, length - 1);
  switch (key) {
    case "ArrowDown":
      return Math.min(length - 1, at + 1);
    case "ArrowUp":
      return Math.max(0, at < 0 ? 0 : at - 1);
    case "Home":
      return 0;
    case "End":
      return length - 1;
    default:
      return at;
  }
}

/** The entry's own date, for the row (`modified_ms` is Unix epoch milliseconds). */
export function modifiedText(entry: Pick<FileEntry, "modified_ms">, now: Date = new Date()): string {
  const date = new Date(entry.modified_ms);
  if (Number.isNaN(date.getTime())) return "";
  const sameDay =
    date.getFullYear() === now.getFullYear() && date.getMonth() === now.getMonth() && date.getDate() === now.getDate();
  return sameDay ? date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }) : date.toLocaleDateString();
}
