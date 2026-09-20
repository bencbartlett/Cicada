// @vitest-environment jsdom
/**
 * The commit dialog as a MODAL (docs/16 §Keyboard map's Esc row; fix round
 * 2026-09-20, finding L2-R1-1): with the REAL key router on the window —
 * keydown AND keyup, the path Space takes — and the focus on the page
 * behind the dialog (its root takes no focus, so a click on its header
 * lands the focus on the body), Del, Space, P, D, the arrows and Ctrl+Z
 * send nothing and post no notice, and Esc closes the dialog and does
 * nothing else: a running solve is not cancelled, the selection stands.
 * `keyboard.test.ts` holds the rule at the map's level; this holds the
 * routing over the rendered dialog, as `AboutDialog.test.tsx` does for
 * About. A rule narrowed back to About + File → Open (with the old
 * commit-dialog Esc branch kept) passed every other test and sent
 * `delete_node`, `transport_play`, `toggle_disable`, `move_node` and `undo`
 * from behind the open dialog.
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createKeyRouter } from "../keyboard";
import type { ClientMessage, NodeView } from "../protocol/messages";
import { useCicada } from "../state/store";
import { CommitDialog } from "./CommitDialog";

function fakeNode(name: string): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text: `${name} = box()`,
    kind: "call",
    func: "box",
    title: "Box",
    category: "Surface & solid",
    inputs: [],
    outputs: [{ name: "out", type: "Mesh", base: "Mesh", displayable: true }],
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [3, 4],
    size: [8, 3],
    manual: false,
  };
}

// A paused transport with a time param: Space would send `transport_play`
// from the canvas — behind the modal it must not.
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

describe("the commit dialog owns the keyboard", () => {
  afterEach(() => {
    cleanup();
    useCicada.setState({
      commitDialog: false,
      transport: null,
      summary: { ...useCicada.getState().summary, running: false },
      selection: { nodes: [], wire: null, element: null },
    });
  });

  it("with the real key router: Del, Space, P, D, the arrows and Ctrl+Z behind it send nothing; Esc closes it and nothing else", () => {
    const sent: ClientMessage[] = [];
    useCicada.getState().installSender((m) => {
      sent.push(m);
      return "id";
    });
    useCicada.setState({
      connection: "open",
      role: "writer",
      commitDialog: true,
      aboutDialog: false,
      fileDialog: false,
      notices: [],
      search: null,
      transport: paused,
      summary: { ...useCicada.getState().summary, running: true },
      graph: { nodes: [fakeNode("a")], wires: [], diagnostics: [] },
      selection: { nodes: ["a"], wire: null, element: null },
    });
    const router = createKeyRouter();
    window.addEventListener("keydown", router.onKeyDown);
    window.addEventListener("keyup", router.onKeyUp);
    try {
      render(<CommitDialog />);
      expect(screen.getByRole("dialog", { name: "commit" })).toBeTruthy();
      expect(document.activeElement, "the dialog's root takes no focus: the body is where a click on its header leaves it").toBe(document.body);

      fireEvent.keyDown(document.body, { key: "Delete" });
      fireEvent.keyDown(document.body, { key: " ", code: "Space" });
      fireEvent.keyUp(document.body, { key: " ", code: "Space" });
      fireEvent.keyDown(document.body, { key: "p" });
      fireEvent.keyDown(document.body, { key: "d" });
      fireEvent.keyDown(document.body, { key: "ArrowRight" });
      fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
      expect(sent, "nothing reached the canvas from behind the open dialog").toEqual([]);
      expect(useCicada.getState().notices).toEqual([]);
      expect(useCicada.getState().commitDialog).toBe(true);
      expect(screen.getByTestId("commit-dialog")).toBeTruthy();
      expect(useCicada.getState().selection.nodes).toEqual(["a"]);

      // Esc behind it: the dialog closes — the running solve is NOT
      // cancelled and the selection stands.
      fireEvent.keyDown(document.body, { key: "Escape" });
      expect(useCicada.getState().commitDialog).toBe(false);
      expect(screen.queryByTestId("commit-dialog")).toBeNull();
      expect(sent, "no cancel: Esc did one thing").toEqual([]);
      expect(useCicada.getState().selection.nodes).toEqual(["a"]);

      // With the dialog closed the same keys are the map's again.
      fireEvent.keyDown(document.body, { key: "Escape" });
      expect(sent).toEqual([{ type: "cancel", payload: {} }]);
    } finally {
      window.removeEventListener("keydown", router.onKeyDown);
      window.removeEventListener("keyup", router.onKeyUp);
    }
  });
});
