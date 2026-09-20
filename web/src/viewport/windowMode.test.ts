// @vitest-environment jsdom
/**
 * The viewport-mode controller (docs/16 §Viewport conventions; wave 5 V1)
 * against a FAKE `documentPictureInPicture` and a fake pop-out `open`:
 * choosing `window` asks for a PiP window of the floating size, moves the
 * registered element into its document with the app's styles and theme,
 * re-homes the scene there and only then flips the mode; the window's own
 * `pagehide` brings the element home and lands on `split`; leaving by the
 * control brings it home FIRST, closes the window and lands on the chosen
 * mode (a `pagehide` the close raises is ignored); the host unmounting
 * reclaims and closes; a refused request is an error notice and no change;
 * without the API the observer pop-out opens with a warning and the mode
 * stays. `adoptStyles` copies linked sheets by href and inline ones by rule.
 */
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { useCicada } from "../state/store";
import { DEFAULT_WINDOW_SIZE, type DocumentPictureInPicture, type PipWindow } from "./modes";
import { POPOUT_NAME } from "./popout";
import { adoptStyles, chooseViewportMode, registerViewportHost, viewportWindowOpen, type ModeWindow } from "./windowMode";

class FakeResizeObserver {
  observe(): void {}
  disconnect(): void {}
  unobserve(): void {}
}

interface FakePip {
  win: PipWindow;
  document: Document;
  close: ReturnType<typeof vi.fn>;
  fire(type: string): void;
}

function fakePip(): FakePip {
  const doc = document.implementation.createHTMLDocument("pip");
  const target = new EventTarget();
  const close = vi.fn();
  const win = {
    document: doc,
    addEventListener: target.addEventListener.bind(target),
    removeEventListener: target.removeEventListener.bind(target),
    close,
    ResizeObserver: FakeResizeObserver,
    devicePixelRatio: 2,
    requestAnimationFrame: (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0) as unknown as number,
  } as unknown as PipWindow;
  return { win, document: doc, close, fire: (type) => target.dispatchEvent(new Event(type)) };
}

type OpenMock = Mock<() => Window | null>;

function mainWindow(api: unknown, open: OpenMock = vi.fn<() => Window | null>(() => ({}) as Window)): ModeWindow & { open: OpenMock } {
  return {
    open,
    location: { origin: "http://127.0.0.1:8420", pathname: "/", search: "?token=t&pipeline=02-solids.cic" },
    documentPictureInPicture: api,
    document,
    ResizeObserver: FakeResizeObserver as unknown as typeof ResizeObserver,
    devicePixelRatio: 1,
    requestAnimationFrame: (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0) as unknown as number,
  };
}

/** A registered host in a wrapper, like the `Viewport` in its `ViewportFrame`. */
function mountHost(win: ModeWindow) {
  const home = document.createElement("div");
  home.className = "pane";
  const element = document.createElement("div");
  element.className = "viewport";
  home.append(element);
  document.body.append(home);
  const rehome = vi.fn();
  const unregister = registerViewportHost({ element, rehome }, win);
  return { home, element, rehome, unregister };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const mode = () => useCicada.getState().settings.viewportMode;

describe("chooseViewportMode", () => {
  let cleanup: (() => void)[] = [];
  beforeEach(() => {
    useCicada.setState({ notices: [], pipeline: "02-solids.cic" });
    useCicada.getState().updateSettings({ viewportMode: "split", floatingViewport: null, theme: "dark" });
    document.body.innerHTML = "";
    document.head.innerHTML = "";
  });
  afterEach(() => {
    for (const fn of cleanup) fn();
    cleanup = [];
  });

  it("split ↔ floating is the setting alone", () => {
    const win = mainWindow(undefined);
    const { unregister } = mountHost(win);
    cleanup.push(unregister);
    chooseViewportMode("floating", win);
    expect(mode()).toBe("floating");
    chooseViewportMode("split", win);
    expect(mode()).toBe("split");
    expect(win.open).not.toHaveBeenCalled();
    expect(viewportWindowOpen()).toBe(false);
  });

  it("without the API, window = the observer pop-out + a warning, and the mode stays", () => {
    const win = mainWindow(undefined);
    const { unregister, element, home } = mountHost(win);
    cleanup.push(unregister);
    chooseViewportMode("floating", win);
    chooseViewportMode("window", win);
    expect(win.open).toHaveBeenCalledWith("http://127.0.0.1:8420/?token=t&pipeline=02-solids.cic&view=viewport", POPOUT_NAME);
    expect(mode()).toBe("floating");
    expect(element.parentElement).toBe(home);
    const notices = useCicada.getState().notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]!.level).toBe("warning");
    expect(notices[0]!.message).toMatch(/no picture-in-picture window/);
    expect(notices[0]!.message).toMatch(/read-only window instead/);
  });

  it("a further `window` click with the pop-out open re-targets it and adds no second warning", () => {
    const existing = {} as Window;
    const win = mainWindow(
      undefined,
      vi.fn<() => Window | null>(() => existing),
    );
    cleanup.push(mountHost(win).unregister);
    chooseViewportMode("window", win);
    chooseViewportMode("window", win);
    chooseViewportMode("window", win);
    expect(win.open).toHaveBeenCalledTimes(3);
    const notices = useCicada.getState().notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]!.message).toMatch(/read-only window instead/);
  });

  it("a blocked pop-out is the pop-out's own notice, not a second one", () => {
    const win = mainWindow(
      undefined,
      vi.fn<() => Window | null>(() => null),
    );
    cleanup.push(mountHost(win).unregister);
    chooseViewportMode("window", win);
    const notices = useCicada.getState().notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]!.message).toMatch(/blocked the pop-out window/);
  });

  it("with the API: the window is asked for at the floating size, the element moves in with styles + theme, the scene re-homes, THEN the mode flips", async () => {
    const pip = fakePip();
    const requestWindow = vi.fn(() => Promise.resolve(pip.win));
    const api: DocumentPictureInPicture = { requestWindow, window: null };
    const win = mainWindow(api);
    const { unregister, element, home, rehome } = mountHost(win);
    cleanup.push(unregister);
    const style = document.createElement("style");
    style.textContent = ".viewport { position: absolute; }";
    document.head.append(style);
    useCicada.getState().updateSettings({ theme: "light", floatingViewport: { x: 1, y: 2, width: 500, height: 320 } });

    chooseViewportMode("window", win);
    expect(requestWindow).toHaveBeenCalledWith({ width: 500, height: 320 });
    // Not yet: the mode changes when the element is in the PiP document.
    expect(mode()).toBe("split");
    expect(element.parentElement).toBe(home);
    await flush();

    expect(mode()).toBe("window");
    expect(viewportWindowOpen()).toBe(true);
    expect(element.ownerDocument).toBe(pip.document);
    expect(element.parentElement).toBe(pip.document.body);
    expect(pip.document.documentElement.dataset.theme).toBe("light");
    expect(pip.document.title).toBe("02-solids.cic — viewport · Cicada");
    const copied = Array.from(pip.document.head.querySelectorAll("style")).map((s) => s.textContent?.replace(/\s+/g, ""));
    expect(copied).toEqual([".viewport{position:absolute;}"]);
    expect(rehome).toHaveBeenCalledTimes(1);
    expect(rehome).toHaveBeenLastCalledWith(pip.win);
    // The theme follows while the window is open.
    useCicada.getState().updateSettings({ theme: "dark" });
    expect(pip.document.documentElement.dataset.theme).toBe("dark");
    // A second `window` while open opens nothing more.
    chooseViewportMode("window", win);
    expect(requestWindow).toHaveBeenCalledTimes(1);

    // ---- the window closes on its own: the element comes home, the mode is split, the scene is back on the main window.
    pip.fire("pagehide");
    expect(mode()).toBe("split");
    expect(viewportWindowOpen()).toBe(false);
    expect(element.parentElement).toBe(home);
    expect(element.ownerDocument).toBe(document);
    expect(rehome).toHaveBeenCalledTimes(2);
    expect(rehome).toHaveBeenLastCalledWith(win);
    expect(pip.close).not.toHaveBeenCalled();
    // The theme no longer follows a closed window.
    useCicada.getState().updateSettings({ theme: "light" });
    expect(pip.document.documentElement.dataset.theme).toBe("dark");
  });

  it("a request without a floating size asks for the default size", async () => {
    const pip = fakePip();
    const requestWindow = vi.fn(() => Promise.resolve(pip.win));
    const win = mainWindow({ requestWindow, window: null });
    cleanup.push(mountHost(win).unregister);
    chooseViewportMode("window", win);
    expect(requestWindow).toHaveBeenCalledWith(DEFAULT_WINDOW_SIZE);
    await flush();
    chooseViewportMode("split", win);
  });

  it("leaving by the control: home first, then close, landing on the CHOSEN mode; the close's pagehide is ignored", async () => {
    const pip = fakePip();
    const win = mainWindow({ requestWindow: () => Promise.resolve(pip.win), window: null });
    const { unregister, element, home } = mountHost(win);
    cleanup.push(unregister);
    chooseViewportMode("window", win);
    await flush();
    expect(mode()).toBe("window");
    let parentAtClose: HTMLElement | null = null;
    pip.close.mockImplementation(() => {
      parentAtClose = element.parentElement;
      pip.fire("pagehide");
    });
    // The element must be home BEFORE the store's mode changes: React
    // re-renders on that change and looks for the node in its wrapper.
    let parentAtModeChange: HTMLElement | null = null;
    const unsubscribe = useCicada.subscribe((state, prev) => {
      if (state.settings.viewportMode !== prev.settings.viewportMode) parentAtModeChange = element.parentElement;
    });
    chooseViewportMode("floating", win);
    unsubscribe();
    expect(pip.close).toHaveBeenCalledTimes(1);
    expect(parentAtClose).toBe(home);
    expect(parentAtModeChange).toBe(home);
    expect(mode()).toBe("floating");
    expect(viewportWindowOpen()).toBe(false);
    // A late pagehide from the closed window changes nothing either.
    pip.fire("pagehide");
    expect(mode()).toBe("floating");
  });

  it("the host unmounting while the window is open reclaims, closes and lands on split", async () => {
    const pip = fakePip();
    const win = mainWindow({ requestWindow: () => Promise.resolve(pip.win), window: null });
    const { unregister, element, home } = mountHost(win);
    chooseViewportMode("window", win);
    await flush();
    expect(element.parentElement).toBe(pip.document.body);
    unregister();
    expect(element.parentElement).toBe(home);
    expect(pip.close).toHaveBeenCalledTimes(1);
    expect(mode()).toBe("split");
    expect(viewportWindowOpen()).toBe(false);
  });

  it("a window that resolves after the host left is closed again, unused", async () => {
    const pip = fakePip();
    let resolve: (w: PipWindow) => void = () => {};
    const win = mainWindow({ requestWindow: () => new Promise<PipWindow>((r) => (resolve = r)), window: null });
    const { unregister, element, home } = mountHost(win);
    chooseViewportMode("window", win);
    unregister();
    resolve(pip.win);
    await flush();
    expect(pip.close).toHaveBeenCalledTimes(1);
    expect(element.parentElement).toBe(home);
    expect(mode()).toBe("split");
  });

  it("a refused request is an error notice and no change of mode", async () => {
    const win = mainWindow({ requestWindow: () => Promise.reject(new Error("needs a user gesture")), window: null });
    const { unregister, element, home } = mountHost(win);
    cleanup.push(unregister);
    chooseViewportMode("floating", win);
    chooseViewportMode("window", win);
    await flush();
    expect(mode()).toBe("floating");
    expect(element.parentElement).toBe(home);
    const notices = useCicada.getState().notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]!.level).toBe("error");
    expect(notices[0]!.message).toMatch(/refused — Error: needs a user gesture/);
    expect(win.open).not.toHaveBeenCalled();
  });
});

describe("adoptStyles", () => {
  it("copies a linked sheet by href and an inline one by its rules; an unreadable sheet is skipped", () => {
    const into = document.implementation.createHTMLDocument("pip");
    const unreadable = {
      href: null,
      get cssRules(): CSSRuleList {
        throw new DOMException("cross-origin", "SecurityError");
      },
    };
    const from = {
      styleSheets: [
        { href: "http://127.0.0.1:8420/assets/index-abc.css" },
        { href: null, cssRules: [{ cssText: ".a { color: red; }" }, { cssText: ".b { top: 22px; }" }] },
        unreadable,
      ],
    } as unknown as Document;
    adoptStyles(from, into);
    const links = Array.from(into.head.querySelectorAll("link")).map((l) => [l.rel, l.href]);
    expect(links).toEqual([["stylesheet", "http://127.0.0.1:8420/assets/index-abc.css"]]);
    const styles = Array.from(into.head.querySelectorAll("style")).map((s) => s.textContent);
    expect(styles).toEqual([".a { color: red; }\n.b { top: 22px; }"]);
  });
});
