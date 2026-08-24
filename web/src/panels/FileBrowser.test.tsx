// @vitest-environment jsdom
/**
 * The file browser (docs/16 §Application layout — the picker and File →
 * Open…) against a stubbed `GET /api/files`: the root lists directories
 * then pipelines; the keyboard moves the cursor and Enter descends into a
 * directory (the breadcrumbs follow, a new listing is fetched for it) or
 * opens a pipeline as its root-relative path; Backspace goes up; a
 * double-click is Enter; a refused listing shows the server's reason and
 * keeps the breadcrumbs; a stale answer never lands under the wrong
 * directory.
 */
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FilesResponse } from "../protocol/messages";
import { FileBrowser } from "./FileBrowser";

const ROOT: FilesResponse = {
  root: "examples",
  dir: "",
  parent: null,
  entries: [
    { name: "wall", kind: "dir", modified_ms: 1 },
    { name: "01-curves.cic", kind: "pipeline", modified_ms: 2 },
    { name: "02-solids.cic", kind: "pipeline", modified_ms: 3 },
  ],
};
const WALL: FilesResponse = {
  root: "examples",
  dir: "wall",
  parent: "",
  entries: [
    { name: "scripts", kind: "dir", modified_ms: 1 },
    { name: "wall.cic", kind: "pipeline", modified_ms: 2 },
  ],
};

type Answer = { status: number; body: unknown } | (() => Promise<Response>);

/** A `fetch` over a table of answers by `dir`; every request is recorded. */
function fakeFetch(table: Record<string, Answer>) {
  const requests: string[] = [];
  const impl = vi.fn(async (input: string): Promise<Response> => {
    requests.push(input);
    const dir = decodeURIComponent(new URL(input, "http://x").searchParams.get("dir") ?? "");
    const answer = table[dir];
    if (answer === undefined) return new Response(JSON.stringify({ kind: "not_found", message: "no such directory", path: dir }), { status: 404 });
    if (typeof answer === "function") return answer();
    return new Response(JSON.stringify(answer.body), { status: answer.status });
  });
  return { impl, requests };
}

const list = () => screen.getByTestId("files-list");
/** The rows' names, read from the name cell (the date cell's text is locale-shaped — never split on it). */
const names = () => screen.getAllByRole("option").map((row) => row.querySelector(".files-name")?.textContent);
/** Let every settled promise callback run (`Response.json()` and the component's `.then` span several microtask turns). */
const settle = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe("FileBrowser", () => {
  afterEach(cleanup);

  it("lists the root (directories first), walks into a directory with the keyboard, opens a pipeline by its root-relative path, goes up with Backspace", async () => {
    const { impl, requests } = fakeFetch({ "": { status: 200, body: ROOT }, wall: { status: 200, body: WALL } });
    const onOpen = vi.fn();
    render(<FileBrowser token="t" onOpen={onOpen} autoFocus fetchImpl={impl} />);
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(3));
    expect(requests).toEqual(["/api/files?dir="]);
    expect(screen.getAllByRole("option").map((row) => row.getAttribute("data-kind"))).toEqual(["dir", "pipeline", "pipeline"]);
    expect(names()).toEqual(["wall", "01-curves.cic", "02-solids.cic"]);
    expect(screen.getByTestId("files-crumb-root").textContent).toBe("examples");
    expect(document.activeElement, "autoFocus puts the keyboard on the list").toBe(list());
    expect(screen.getByTestId("files-entry-wall").getAttribute("aria-selected")).toBe("true");

    // Enter on the directory: descend — the crumbs and the listing follow.
    fireEvent.keyDown(list(), { key: "Enter" });
    await waitFor(() => expect(screen.getByTestId("files-crumb-wall")).toBeTruthy());
    await waitFor(() => expect(names()).toEqual(["scripts", "wall.cic"]));
    expect(requests).toEqual(["/api/files?dir=", "/api/files?dir=wall"]);
    expect(screen.getByTestId("file-browser").getAttribute("data-dir")).toBe("wall");

    // Down, Enter on the pipeline: opened as `wall/wall.cic`.
    fireEvent.keyDown(list(), { key: "ArrowDown" });
    expect(screen.getByTestId("files-entry-wall.cic").getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(list(), { key: "ArrowDown" });
    expect(screen.getByTestId("files-entry-wall.cic").getAttribute("aria-selected"), "the end is a stop").toBe("true");
    fireEvent.keyDown(list(), { key: "Enter" });
    expect(onOpen).toHaveBeenCalledWith("wall/wall.cic");

    // Backspace: up to the root.
    fireEvent.keyDown(list(), { key: "Backspace" });
    await waitFor(() => expect(names()).toEqual(["wall", "01-curves.cic", "02-solids.cic"]));
    expect(screen.queryByTestId("files-crumb-wall")).toBeNull();
    fireEvent.keyDown(list(), { key: "Backspace" });
    expect(requests, "Backspace at the root goes nowhere").toHaveLength(3);
  });

  it("a double-click opens; the breadcrumb climbs out; Home/End jump", async () => {
    const { impl } = fakeFetch({ "": { status: 200, body: ROOT }, wall: { status: 200, body: WALL } });
    const onOpen = vi.fn();
    render(<FileBrowser token="t" initialDir="wall" onOpen={onOpen} fetchImpl={impl} />);
    await waitFor(() => expect(names()).toEqual(["scripts", "wall.cic"]));
    fireEvent.doubleClick(screen.getByTestId("files-entry-wall.cic"));
    expect(onOpen).toHaveBeenCalledWith("wall/wall.cic");
    fireEvent.click(screen.getByTestId("files-crumb-root"));
    await waitFor(() => expect(names()).toEqual(["wall", "01-curves.cic", "02-solids.cic"]));
    fireEvent.keyDown(list(), { key: "End" });
    expect(screen.getByTestId("files-entry-02-solids.cic").getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(list(), { key: "Home" });
    expect(screen.getByTestId("files-entry-wall").getAttribute("aria-selected")).toBe("true");
    fireEvent.click(screen.getByTestId("files-entry-01-curves.cic"));
    expect(screen.getByTestId("files-entry-01-curves.cic").getAttribute("aria-selected"), "a click moves the cursor without opening").toBe("true");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("a refused listing shows the server's reason and keeps the breadcrumbs to climb out", async () => {
    const { impl } = fakeFetch({ "": { status: 200, body: ROOT } });
    render(<FileBrowser token="t" initialDir="gone" onOpen={() => {}} fetchImpl={impl} />);
    await waitFor(() => expect(screen.getByTestId("files-error")).toBeTruthy());
    expect(screen.getByTestId("files-error").textContent).toMatch(/`gone` is not a directory under the served root/);
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByTestId("files-crumb-gone")).toBeTruthy();
    fireEvent.click(screen.getByTestId("files-crumb-root"));
    await waitFor(() => expect(names()).toEqual(["wall", "01-curves.cic", "02-solids.cic"]));
    expect(screen.queryByTestId("files-error")).toBeNull();
  });

  it("an empty directory says so", async () => {
    const { impl } = fakeFetch({ empty: { status: 200, body: { root: "examples", dir: "empty", parent: "", entries: [] } } });
    render(<FileBrowser token="t" initialDir="empty" onOpen={() => {}} fetchImpl={impl} />);
    await waitFor(() => expect(screen.getByTestId("files-empty")).toBeTruthy());
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByTestId("file-browser").getAttribute("data-loading")).toBe("false");
  });

  it("a stale answer never lands under the wrong directory: two requests race inside ONE browser and the newest wins", async () => {
    // `wall` answers slowly; the user climbs to the root (answered at once)
    // before it lands. The crumbs come from the PATH, not the listing, so the
    // root crumb is clickable while `wall` is still loading. Without the
    // request guard the late `wall` listing would overwrite the root's under
    // the root's breadcrumbs — the 2026-08-24 review found the earlier form
    // of this test (a fresh mount) could not see that mutation.
    let releaseWall: (() => void) | null = null;
    const slowWall = new Promise<Response>((resolve) => {
      releaseWall = () => resolve(new Response(JSON.stringify(WALL), { status: 200 }));
    });
    const { impl, requests } = fakeFetch({ "": { status: 200, body: ROOT }, wall: () => slowWall });
    render(<FileBrowser token="t" initialDir="wall" onOpen={() => {}} fetchImpl={impl} />);
    expect(screen.getByTestId("file-browser").getAttribute("data-loading")).toBe("true");
    expect(requests).toEqual(["/api/files?dir=wall"]);
    fireEvent.click(screen.getByTestId("files-crumb-root"));
    await waitFor(() => expect(names()).toEqual(["wall", "01-curves.cic", "02-solids.cic"]));
    expect(requests).toEqual(["/api/files?dir=wall", "/api/files?dir="]);
    expect(screen.getByTestId("file-browser").getAttribute("data-dir")).toBe("");
    expect(screen.getByTestId("file-browser").getAttribute("data-loading")).toBe("false");
    // Now the abandoned request's answer arrives, and is dropped.
    await act(async () => {
      releaseWall?.();
      await slowWall;
      await settle();
    });
    expect(names(), "the root's listing stands — not `wall`'s").toEqual(["wall", "01-curves.cic", "02-solids.cic"]);
    expect(screen.getByTestId("file-browser").getAttribute("data-dir")).toBe("");
    expect(screen.queryByTestId("files-crumb-wall")).toBeNull();
    expect(screen.getByTestId("files-entry-wall").getAttribute("aria-selected"), "the cursor is the root's").toBe("true");
  });
});
