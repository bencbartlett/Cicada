// @vitest-environment jsdom
/**
 * The real `Viewport` registers its host with the mode controller and, on
 * unmount (File → Close, the picker), the layout effect's cleanup brings
 * the element home and closes the picture-in-picture window — with the
 * mode back at `split` — before React removes anything (docs/16 §Viewport
 * conventions; wave 5 V1). jsdom has no WebGL, so the scene's creation
 * fails on its own path (the failure text, a notice); the registration
 * does not depend on it. The observer pop-out page (`view=viewport`)
 * registers nothing: its viewport is never the window mode's. And the
 * toolbar that moves WITH the element keeps working there: its handlers
 * are reached through the portal's listeners on the host, not through the
 * main document's root (a click in the PiP document never bubbles to it).
 */
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useRoute } from "../state/route";
import { useCicada } from "../state/store";
import type { PipWindow } from "./modes";
import { Viewport } from "./Viewport";
import { chooseViewportMode, viewportWindowOpen, type ModeWindow } from "./windowMode";

class FakeResizeObserver {
  observe(): void {}
  disconnect(): void {}
  unobserve(): void {}
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function fakePip() {
  const doc = document.implementation.createHTMLDocument("pip");
  const target = new EventTarget();
  const close = vi.fn();
  const win = {
    document: doc,
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    close,
    ResizeObserver: FakeResizeObserver,
    devicePixelRatio: 1,
    requestAnimationFrame: (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0) as unknown as number,
  } as unknown as PipWindow;
  return { win, document: doc, close };
}

function mainWindow(pip: PipWindow): ModeWindow {
  return {
    open: vi.fn<() => Window | null>(() => null),
    location: { origin: "http://127.0.0.1:8420", pathname: "/", search: "?token=t&pipeline=02-solids.cic" },
    documentPictureInPicture: { requestWindow: () => Promise.resolve(pip), window: null },
    document,
    ResizeObserver: FakeResizeObserver as unknown as typeof ResizeObserver,
    devicePixelRatio: 1,
    requestAnimationFrame: (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0) as unknown as number,
  };
}

describe("Viewport + the window mode on unmount", () => {
  beforeEach(() => {
    vi.stubGlobal("ResizeObserver", FakeResizeObserver);
    vi.spyOn(console, "error").mockImplementation(() => {});
    useCicada.setState({ notices: [], pipeline: "02-solids.cic" });
    useCicada.getState().updateSettings({ viewportMode: "split", floatingViewport: null });
    useRoute.getState().setRoute({ token: "t", pipeline: "02-solids.cic", view: "app" });
  });
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("the app's viewport moves into the window and, on unmount, comes home, the window closes, the mode is split", async () => {
    const pip = fakePip();
    const win = mainWindow(pip.win);
    const { container, unmount } = render(<Viewport />);
    const host = container.querySelector<HTMLElement>("[data-testid='viewport']");
    expect(host).not.toBeNull();
    chooseViewportMode("window", win);
    await flush();
    expect(useCicada.getState().settings.viewportMode).toBe("window");
    expect(host!.ownerDocument).toBe(pip.document);
    expect(container.querySelector("[data-testid='viewport']")).toBeNull();

    unmount();
    expect(host!.ownerDocument).toBe(document);
    expect(pip.close).toHaveBeenCalledTimes(1);
    expect(useCicada.getState().settings.viewportMode).toBe("split");
    expect(viewportWindowOpen()).toBe(false);
    // Unmounted for good: the element is out of every tree, not parked in the PiP body.
    expect(pip.document.body.contains(host)).toBe(false);
    expect(document.body.contains(host)).toBe(false);
  });

  it("the toolbar that moved into the PiP document still reaches React: a display-mode click and the control's `split` work from there", async () => {
    const pip = fakePip();
    const win = mainWindow(pip.win);
    useCicada.getState().updateSettings({ displayMode: "shaded_edges" });
    const { container } = render(<Viewport />);
    const host = container.querySelector<HTMLElement>("[data-testid='viewport']")!;
    chooseViewportMode("window", win);
    await flush();
    expect(host.ownerDocument).toBe(pip.document);
    // The PiP document's own events (a real click bubbles in the document the
    // node is in — `document.implementation`'s has no window of its own, so
    // the event is constructed here and dispatched there; fireEvent needs a
    // window and cannot).
    const click = (testIdOrTitle: string) => {
      const target = host.querySelector<HTMLElement>(testIdOrTitle);
      expect(target, testIdOrTitle).not.toBeNull();
      act(() => {
        target!.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      });
    };
    click("button[title='wireframe']");
    expect(useCicada.getState().settings.displayMode).toBe("wireframe");
    expect(host.querySelector("button[title='wireframe']")?.className).toBe("active");
    click("[data-testid='viewport-mode-split']");
    expect(useCicada.getState().settings.viewportMode).toBe("split");
    expect(host.ownerDocument).toBe(document);
    expect(container.contains(host)).toBe(true);
    expect(pip.close).toHaveBeenCalledTimes(1);
    expect(viewportWindowOpen()).toBe(false);
    // Home again, the same buttons still work once (no doubled listeners).
    const before = useCicada.getState().settings.displayMode;
    click("button[title='shaded']");
    expect(before).toBe("wireframe");
    expect(useCicada.getState().settings.displayMode).toBe("shaded");
  });

  it("the observer pop-out page's viewport registers nothing: `window` there opens no PiP and moves nothing", async () => {
    useRoute.getState().setRoute({ token: "t", pipeline: "02-solids.cic", view: "viewport" });
    const pip = fakePip();
    const win = mainWindow(pip.win);
    const { container } = render(<Viewport />);
    expect(container.querySelector("[data-testid='viewport-modes']")).toBeNull();
    chooseViewportMode("window", win);
    await flush();
    // The request resolved with no host registered: the window is closed again, unused; the mode stays.
    expect(pip.close).toHaveBeenCalledTimes(1);
    expect(useCicada.getState().settings.viewportMode).toBe("split");
    expect(container.querySelector("[data-testid='viewport']")?.ownerDocument).toBe(document);
  });
});
