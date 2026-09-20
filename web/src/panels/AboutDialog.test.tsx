// @vitest-environment jsdom
/**
 * The About dialog (docs/16 §Settings; docs/17 wave 5 R1) against a seeded
 * store: every field comes from `hello` — version, commit, built, protocol,
 * the engine's threads — the two links point at the repository and this
 * version's release page, a click on the commit copies it (and a refused
 * clipboard says so), an engine that reported no build reads "not reported"
 * rather than an invented value, Esc / × close it, and the settings menu's
 * last entry opens it while closing the menu. With the REAL key router on
 * the window (App's order — the router first, the dialog's own listener
 * after; keydown AND keyup, the path Space takes — and a paused transport
 * with a time param, so Space would otherwise send `transport_play`: fix
 * round 2 2026-09-20, L2-R1-2 / L3A-4 — without both the Space assertion
 * could not fail), Esc from the page behind the dialog closes it and does
 * nothing else, Del and Space behind it send nothing, and focus moves into
 * the dialog on open and back to the gear on close (fix round 2026-09-20,
 * L5-1 / R1-C1 / R1-C5).
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createKeyRouter } from "../keyboard";
import type { ClientMessage } from "../protocol/messages";
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

function seed(hello: HelloInfo | null, aboutDialog = true) {
  useCicada.setState({ connection: "open", role: "writer", hello, notices: [], aboutDialog });
}

function installClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

const text = (testId: string) => screen.getByTestId(testId).textContent;
const isOpen = () => useCicada.getState().aboutDialog;

// A paused transport with a time param: from the canvas, a Space tap sends
// `transport_play`; with no transport the map only posts a notice.
const paused = {
  view: {
    playing: false,
    speed: 1,
    t_ms: 0,
    frame: 0,
    frames: 120,
    period_ms: 4000,
    driven: [{ node: "spin", port: "frame", signal: "frame" as const, loop: { frames: 120, period_ms: 4000 } }],
  },
  receivedAt: 0,
};

describe("the About dialog", () => {
  afterEach(() => {
    cleanup();
    useCicada.setState({ aboutDialog: false });
  });

  it("shows the build's fields from hello and links to the repository and this version's release notes", () => {
    seed(stamped);
    render(<AboutDialog />);
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
    render(<AboutDialog />);
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
    render(<AboutDialog />);
    expect(text("about-version")).toBe(NOT_REPORTED);
    expect(text("about-commit")).toBe(NOT_REPORTED);
    expect(text("about-built")).toBe(NOT_REPORTED);
    expect(text("about-threads")).toBe(NOT_REPORTED);
    expect(text("about-protocol")).toBe("1");
    // Nothing to copy: the commit is plain text, not a button.
    expect(screen.getByTestId("about-commit").tagName).toBe("SPAN");
    expect(screen.getByTestId("about-notes").getAttribute("href")).toBe(`${REPOSITORY_URL}/releases`);
  });

  it("a build git could not name shows `unknown` as text, not as a hash to copy", () => {
    seed({ ...stamped, version: { semver: "0.1.0-alpha.1", commit: "unknown", built: "2026-09-20" } });
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    installClipboard(writeText);
    render(<AboutDialog />);
    const commit = screen.getByTestId("about-commit");
    expect(commit.textContent).toBe("unknown");
    expect(commit.tagName, "plain text: nothing to copy").toBe("SPAN");
    fireEvent.click(commit);
    expect(writeText).not.toHaveBeenCalled();
    expect(screen.queryByTestId("about-copied")).toBeNull();
    // The release-notes link still stands: the version is known.
    expect(screen.getByTestId("about-notes").getAttribute("href")).toBe(`${REPOSITORY_URL}/releases/tag/v0.1.0-alpha.1`);
  });

  it("before the engine says hello every field is a dash and the dialog says it is not connected", () => {
    seed(null);
    render(<AboutDialog />);
    expect(text("about-version")).toBe("—");
    expect(text("about-threads")).toBe("—");
    expect(screen.getByText(/not connected/)).toBeTruthy();
  });

  it("Esc, the × and a click on the backdrop close it (the store flag)", () => {
    seed(stamped);
    render(<AboutDialog />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(isOpen()).toBe(false);
    useCicada.setState({ aboutDialog: true });
    fireEvent.click(screen.getByTestId("about-close"));
    expect(isOpen()).toBe(false);
    useCicada.setState({ aboutDialog: true });
    fireEvent.pointerDown(screen.getByTestId("about-backdrop"));
    expect(isOpen()).toBe(false);
    // A pointer inside the dialog is not a close.
    useCicada.setState({ aboutDialog: true });
    fireEvent.pointerDown(screen.getByTestId("about-dialog"));
    expect(isOpen()).toBe(true);
  });

  it("opens from the settings menu's last entry, which closes the menu; × takes it down again", () => {
    seed(stamped, false);
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

  it("Esc on the gear while its menu is open closes the menu alone — nothing is cancelled", () => {
    seed(stamped, false);
    const sent: ClientMessage[] = [];
    useCicada.getState().installSender((m) => {
      sent.push(m);
      return "id";
    });
    useCicada.setState({ summary: { ...useCicada.getState().summary, running: true } });
    const router = createKeyRouter();
    window.addEventListener("keydown", router.onKeyDown);
    try {
      render(<TopBar />);
      const gear = screen.getByTestId("tb-settings");
      fireEvent.click(gear);
      gear.focus();
      expect(screen.getByRole("dialog", { name: "settings" })).toBeTruthy();
      fireEvent.keyDown(gear, { key: "Escape" });
      expect(screen.queryByRole("dialog", { name: "settings" }), "the menu closed").toBeNull();
      expect(sent, "the running solve was not cancelled").toEqual([]);
      // Closed: the next Esc from the gear is the map's.
      fireEvent.keyDown(gear, { key: "Escape" });
      expect(sent).toEqual([{ type: "cancel", payload: {} }]);
    } finally {
      window.removeEventListener("keydown", router.onKeyDown);
      useCicada.setState({ summary: { ...useCicada.getState().summary, running: false } });
    }
  });

  it("never opens over another modal: with the commit dialog open the menu's About entry does nothing (one modal at a time)", () => {
    seed(stamped, false);
    useCicada.setState({ commitDialog: true });
    try {
      render(<TopBar />);
      fireEvent.click(screen.getByTestId("tb-settings"));
      fireEvent.click(screen.getByTestId("tb-about"));
      expect(screen.queryByTestId("about-dialog")).toBeNull();
      expect(isOpen()).toBe(false);
      expect(useCicada.getState().commitDialog, "the commit dialog stands").toBe(true);
    } finally {
      useCicada.setState({ commitDialog: false });
    }
  });

  it("with the real key router on the window: Esc behind About closes it and nothing else, Del behind it deletes nothing, focus lands in the dialog and returns to the gear", () => {
    seed(stamped, false);
    const sent: ClientMessage[] = [];
    useCicada.getState().installSender((m) => {
      sent.push(m);
      return "id";
    });
    useCicada.setState({
      summary: { ...useCicada.getState().summary, running: true },
      selection: { nodes: ["a"], wire: null, element: null },
      transport: paused,
    });
    const router = createKeyRouter();
    window.addEventListener("keydown", router.onKeyDown);
    window.addEventListener("keyup", router.onKeyUp);
    try {
      render(<TopBar />);
      fireEvent.click(screen.getByTestId("tb-settings"));
      fireEvent.click(screen.getByTestId("tb-about"));
      const dialog = screen.getByTestId("about-dialog");
      expect(document.activeElement, "the modal takes focus on open").toBe(dialog);

      // Focus on the page behind it (the review's reproduction): Del must not
      // reach the canvas, the dialog stays.
      fireEvent.keyDown(document.body, { key: "Delete" });
      expect(sent, "no delete_node behind the modal").toEqual([]);
      expect(isOpen()).toBe(true);
      fireEvent.keyDown(document.body, { key: " ", code: "Space" });
      fireEvent.keyUp(document.body, { key: " ", code: "Space" });
      expect(sent, "no transport intent behind the modal").toEqual([]);
      expect(useCicada.getState().notices, "and no notice either").toEqual([]);
      expect(isOpen(), "Space is not the modal's close key either").toBe(true);

      // Esc behind it: closes About — the running solve is NOT cancelled and
      // the selection stands.
      fireEvent.keyDown(document.body, { key: "Escape" });
      expect(isOpen()).toBe(false);
      expect(screen.queryByTestId("about-dialog")).toBeNull();
      expect(sent, "no cancel: Esc did one thing").toEqual([]);
      expect(useCicada.getState().selection.nodes).toEqual(["a"]);
      const gear = screen.getByTestId("tb-settings");
      expect(document.activeElement, "focus returns to the gear that opened it").toBe(gear);

      // Esc again, from the gear the focus sits on: the map's (a running
      // solve → cancel) — a focused button keeps its plain keys from the
      // map, so the gear hands Esc over itself (fix round 2, L3A-2: "Esc to
      // close About, Esc to cancel" cancelled nothing until a click). Del
      // on the gear stays the button's: the gear forwards Esc alone.
      fireEvent.keyDown(gear, { key: "Delete" });
      expect(sent, "Del on a focused button is the button's").toEqual([]);
      fireEvent.keyDown(gear, { key: "Escape" });
      expect(sent).toEqual([{ type: "cancel", payload: {} }]);
      expect(useCicada.getState().selection.nodes, "the cancel was the one thing this Esc did").toEqual(["a"]);
    } finally {
      window.removeEventListener("keydown", router.onKeyDown);
      window.removeEventListener("keyup", router.onKeyUp);
      useCicada.setState({ summary: { ...useCicada.getState().summary, running: false }, selection: { nodes: [], wire: null, element: null }, transport: null });
    }
  });
});
