/**
 * `GET /api/files`' client side (docs/13 §HTTP surface): the request (the
 * `dir` encoded, the token header), the typed answer, and the typed refusal
 * every failure becomes — the server's `{kind, message, path}`, a text 401,
 * an unreachable server — with one sentence per kind for the picker.
 */
import { describe, expect, it, vi } from "vitest";
import { FilesRouteError, describeFilesError, describeFilesFailure, fetchFiles, filesUrl, refusalOf } from "./files";
import type { FilesResponse } from "./messages";

const LISTING: FilesResponse = {
  root: "examples",
  dir: "wall",
  parent: "",
  entries: [
    { name: "golden", kind: "dir", modified_ms: 1 },
    { name: "scripts", kind: "dir", modified_ms: 2 },
    { name: "wall.cic", kind: "pipeline", modified_ms: 3 },
  ],
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

describe("fetchFiles", () => {
  it("GETs /api/files?dir=… with the token header and returns the listing", async () => {
    const fetchImpl = vi.fn(async () => json(200, LISTING));
    const listing = await fetchFiles({ token: "t0k" }, "wall", fetchImpl);
    expect(listing).toEqual(LISTING);
    expect(fetchImpl).toHaveBeenCalledWith("/api/files?dir=wall", { headers: { "X-Cicada-Token": "t0k" } });
  });

  it("encodes the directory; the root is the empty dir", () => {
    expect(filesUrl("")).toBe("/api/files?dir=");
    expect(filesUrl("sub dir/a+b")).toBe("/api/files?dir=sub%20dir%2Fa%2Bb");
  });

  it("a refused listing is a FilesRouteError with the server's body", async () => {
    const fetchImpl = vi.fn(async () =>
      json(404, { kind: "not_found", message: "no such directory", path: "gone" }),
    );
    const error = await fetchFiles({ token: "t" }, "gone", fetchImpl).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(FilesRouteError);
    const refusal = error as FilesRouteError;
    expect(refusal.status).toBe(404);
    expect(refusal.kind).toBe("not_found");
    expect(refusal.body).toEqual({ kind: "not_found", message: "no such directory", path: "gone" });
  });

  it("a text failure (the 401 middleware) and an unreachable server are `transport` refusals naming the dir", async () => {
    const text = await refusalOf(new Response("missing or wrong token", { status: 401 }), "x");
    expect(text.kind).toBe("transport");
    expect(text.status).toBe(401);
    expect(text.message).toBe("HTTP 401 — missing or wrong token");
    expect(text.body.path).toBe("x");
    const down = (await fetchFiles({ token: "t" }, "", async () => {
      throw new TypeError("Failed to fetch");
    }).catch((e: unknown) => e)) as FilesRouteError;
    expect(down).toBeInstanceOf(FilesRouteError);
    expect(down.kind).toBe("transport");
    expect(down.status).toBe(0);
    expect(down.message).toMatch(/unreachable/);
  });

  it("one sentence per kind", () => {
    expect(describeFilesError({ kind: "not_found", message: "m", path: "a/b" })).toMatch(/`a\/b` is not a directory under the served root/);
    expect(describeFilesError({ kind: "not_found", message: "m", path: "" })).toMatch(/^the root is not a directory/);
    expect(describeFilesError({ kind: "path_not_allowed", message: "m", path: "../x" })).toMatch(/outside the served root/);
    expect(describeFilesError({ kind: "io_error", message: "permission denied", path: "locked" })).toBe(
      "`locked` could not be read: permission denied",
    );
    expect(describeFilesError({ kind: "transport", message: "HTTP 401", path: "" })).toBe("the file list could not be read: HTTP 401");
    expect(describeFilesError({ kind: "new_kind", message: "m", path: "" })).toBe("new_kind: m");
    expect(describeFilesFailure(new FilesRouteError(403, { kind: "io_error", message: "busy", path: "d" }))).toBe("`d` could not be read: busy");
    expect(describeFilesFailure(new Error("boom"))).toBe("Error: boom");
  });
});
