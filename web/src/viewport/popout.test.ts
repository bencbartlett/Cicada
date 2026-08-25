/**
 * The pop-out button's one act (docs/16 §Viewport conventions): open the
 * same page with `view=viewport` under the fixed window name, and say so
 * when the browser refuses.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useCicada } from "../state/store";
import { POPOUT_NAME, popOutViewport } from "./popout";

const location = { origin: "http://127.0.0.1:8420", pathname: "/", search: "?token=t&pipeline=02-solids.cic" };

describe("popOutViewport", () => {
  beforeEach(() => useCicada.setState({ notices: [] }));

  it("opens <same URL>&view=viewport as the named window", () => {
    const opened = {} as Window;
    const open = vi.fn(() => opened);
    expect(popOutViewport({ open, location })).toBe(opened);
    expect(open).toHaveBeenCalledWith("http://127.0.0.1:8420/?token=t&pipeline=02-solids.cic&view=viewport", POPOUT_NAME);
    expect(POPOUT_NAME).toBe("cicada-viewport");
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("a blocked pop-up is a warning notice, not silence", () => {
    expect(popOutViewport({ open: vi.fn(() => null), location })).toBeNull();
    const notices = useCicada.getState().notices;
    expect(notices).toHaveLength(1);
    expect(notices[0]!.level).toBe("warning");
    expect(notices[0]!.message).toMatch(/blocked the pop-out window/);
  });
});
