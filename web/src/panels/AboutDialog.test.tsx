// @vitest-environment jsdom
/**
 * The About dialog (docs/16 §Settings; docs/17 wave 5 R1) against a seeded
 * store: every field comes from `hello` — version, commit, built, protocol,
 * the engine's threads — the two links point at the repository and this
 * version's release page, a click on the commit copies it (and a refused
 * clipboard says so), an engine that reported no build reads "not reported"
 * rather than an invented value, Esc / × close it, and the settings menu's
 * last entry opens it while closing the menu.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useCicada, type HelloInfo } from "../state/store";
import { NOT_REPORTED, REPOSITORY_URL, releaseNotesUrl } from "./about";
import { AboutDialog } from "./AboutDialog";
import { TopBar } from "./TopBar";

const stamped: HelloInfo = {
  clientId: 1,
  role: "writer",
  protocol: 1,
  engine: "cicada 0.1.0-alpha.1",
  project: "p",
  pipeline: "p.cic",
  unitPx: 24,
  version: { semver: "0.1.0-alpha.1", commit: "a82eb39d1c2e-dirty", built: "2026-08-25" },
  threads: 6,
};

function seed(hello: HelloInfo | null) {
  useCicada.setState({ connection: "open", role: "writer", hello, notices: [] });
}

function installClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

const text = (testId: string) => screen.getByTestId(testId).textContent;

describe("the About dialog", () => {
  afterEach(cleanup);

  it("shows the build's fields from hello and links to the repository and this version's release notes", () => {
    seed(stamped);
    render(<AboutDialog onClose={() => {}} />);
    expect(screen.getByRole("dialog", { name: "about" })).toBeTruthy();
    expect(text("about-version")).toBe("0.1.0-alpha.1");
    expect(text("about-commit")).toBe("a82eb39d1c2e-dirty");
    expect(text("about-built")).toBe("2026-08-25");
    expect(text("about-protocol")).toBe("1");
    expect(text("about-threads")).toBe("6");
    expect(text("about-engine")).toBe("cicada 0.1.0-alpha.1");
    expect(screen.getByTestId("about-repo").getAttribute("href")).toBe(REPOSITORY_URL);
    expect(screen.getByTestId("about-notes").getAttribute("href")).toBe(`${REPOSITORY_URL}/releases/tag/v0.1.0-alpha.1`);
    expect(releaseNotesUrl(null)).toBe(`${REPOSITORY_URL}/releases`);
  });

  it("a click on the commit copies it and says so; a refused clipboard says why", async () => {
    seed(stamped);
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    installClipboard(writeText);
    render(<AboutDialog onClose={() => {}} />);
    expect(screen.queryByTestId("about-copied")).toBeNull();
    await act(async () => {
      fireEvent.click(screen.getByTestId("about-commit"));
    });
    expect(writeText).toHaveBeenCalledWith("a82eb39d1c2e-dirty");
    expect(text("about-copied")).toBe("copied");

    writeText.mockRejectedValueOnce(new Error("clipboard denied"));
    await act(async () => {
      fireEvent.click(screen.getByTestId("about-commit"));
    });
    expect(text("about-copied")).toBe("copy failed: clipboard denied");
  });

  it("an engine that reported no build says so instead of inventing one, and the notes link falls back to the releases list", () => {
    seed({ ...stamped, version: null, threads: null });
    render(<AboutDialog onClose={() => {}} />);
    expect(text("about-version")).toBe(NOT_REPORTED);
    expect(text("about-commit")).toBe(NOT_REPORTED);
    expect(text("about-built")).toBe(NOT_REPORTED);
    expect(text("about-threads")).toBe(NOT_REPORTED);
    expect(text("about-protocol")).toBe("1");
    // Nothing to copy: the commit is plain text, not a button.
    expect(screen.getByTestId("about-commit").tagName).toBe("SPAN");
    expect(screen.getByTestId("about-notes").getAttribute("href")).toBe(`${REPOSITORY_URL}/releases`);
  });

  it("before the engine says hello every field is a dash and the dialog says it is not connected", () => {
    seed(null);
    render(<AboutDialog onClose={() => {}} />);
    expect(text("about-version")).toBe("—");
    expect(text("about-threads")).toBe("—");
    expect(screen.getByText(/not connected/)).toBeTruthy();
  });

  it("Esc, the × and a click on the backdrop close it", () => {
    seed(stamped);
    const onClose = vi.fn();
    render(<AboutDialog onClose={onClose} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId("about-close"));
    expect(onClose).toHaveBeenCalledTimes(2);
    fireEvent.pointerDown(screen.getByTestId("about-backdrop"));
    expect(onClose).toHaveBeenCalledTimes(3);
    // A pointer inside the dialog is not a close.
    fireEvent.pointerDown(screen.getByTestId("about-dialog"));
    expect(onClose).toHaveBeenCalledTimes(3);
  });

  it("opens from the settings menu's last entry, which closes the menu; × takes it down again", () => {
    seed(stamped);
    render(<TopBar />);
    expect(screen.queryByTestId("about-dialog")).toBeNull();
    fireEvent.click(screen.getByTestId("tb-settings"));
    expect(screen.getByRole("dialog", { name: "settings" })).toBeTruthy();
    fireEvent.click(screen.getByTestId("tb-about"));
    expect(screen.getByTestId("about-dialog")).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "settings" })).toBeNull();
    expect(text("about-commit")).toBe("a82eb39d1c2e-dirty");
    fireEvent.click(screen.getByTestId("about-close"));
    expect(screen.queryByTestId("about-dialog")).toBeNull();
  });
});
