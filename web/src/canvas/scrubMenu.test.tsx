// @vitest-environment jsdom
/**
 * The scrub-cache toggle as a node-menu item (docs/16 §Sliders §Scrub-cached;
 * v0.1 item 5 S2), rendered through the real `ContextMenu`: the label, the
 * greying with the SERVER's reason (a `disabled` the review of 2026-08-24
 * found covered by the e2e alone), the hint — read off the MERGED view, so a
 * `scrub_progress` overlay moves the warm count the raw graph view freezes
 * at the last delta (the review's `0 / 19 positions warm` under a full bar)
 * — and the intent the click sends. Nothing for a node that is no slider.
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ClientMessage, NodeView, ScrubProgressPayload, ScrubView } from "../protocol/messages";
import { ContextMenu } from "./ContextMenu";
import { scrubMenuItems } from "./scrubMenu";

const eligibleOff: ScrubView = { on: false, positions: 19, warmed: [], warming: false, bytes: 0 };
const eligibleOn: ScrubView = { on: true, positions: 19, warmed: [6], warming: true, bytes: 0 };
const tooMany: ScrubView = {
  on: false,
  positions: 0,
  warmed: [],
  warming: false,
  bytes: 0,
  ineligible: "too many positions (51 > 32)",
};
const allWarm: ScrubProgressPayload = {
  node: "size",
  port: "value",
  warmed: Array.from({ length: 19 }, (_, i) => i),
  warming: false,
  bytes: 1_970_000,
};

function slider(name: string, scrub: ScrubView | undefined, func = "slider"): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text: `${name} = slider(value=2.0, min=0.5, max=5.0, step=0.25)`,
    kind: "call",
    func,
    title: "Number Slider",
    category: "Params & input",
    inputs: [],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    param: { kind: "slider", port: "value", value: 2, min: 0.5, max: 5, step: 0.25, scrub },
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [8, 7],
    manual: false,
  };
}

/** The items through the real menu; what the click sends is collected. */
function renderMenu(view: NodeView, progress: ScrubProgressPayload | undefined): ClientMessage[] {
  const sent: ClientMessage[] = [];
  const items = scrubMenuItems(view, progress, (message) => sent.push(message));
  render(<ContextMenu left={0} top={0} title={view.name} items={items} onClose={() => {}} />);
  return sent;
}
const item = () => screen.getByRole("menuitem") as HTMLButtonElement;
const hint = () => item().querySelector(".cv-menu-hint")?.textContent ?? null;

describe("the node menu's scrub-cache item", () => {
  afterEach(cleanup);

  it("reads the MERGED view: the hint is the last delta's count without the overlay, the broadcast's with it", () => {
    renderMenu(slider("size", eligibleOn), undefined);
    expect(item().textContent).toBe("stop scrub-caching1 / 19 positions warm");
    expect(hint()).toBe("1 / 19 positions warm");
    cleanup();
    const sent = renderMenu(slider("size", eligibleOn), allWarm);
    expect(item().textContent).toBe("stop scrub-caching19 / 19 positions warm");
    expect(hint()).toBe("19 / 19 positions warm");
    expect(item().disabled).toBe(false);
    fireEvent.click(item());
    expect(sent).toEqual([{ type: "set_scrub", payload: { node: "size", on: false } }]);
  });

  it("an eligible slider that is off: live, the position count as the hint, the click sends set_scrub on", () => {
    const sent = renderMenu(slider("size", eligibleOff), undefined);
    expect(item().textContent).toBe("scrub-cache this slider19 positions");
    expect(item().disabled).toBe(false);
    fireEvent.click(item());
    expect(sent).toEqual([{ type: "set_scrub", payload: { node: "size", on: true } }]);
  });

  it("an ineligible slider that is off is greyed with the SERVER's reason, verbatim, and the click sends nothing", () => {
    const sent = renderMenu(slider("fine", tooMany), undefined);
    expect(item().textContent).toBe("scrub-cache this slidertoo many positions (51 > 32)");
    expect(item().disabled).toBe(true);
    expect(hint()).toBe("too many positions (51 > 32)");
    expect(item().title).toBe("too many positions (51 > 32)");
    fireEvent.click(item());
    expect(sent).toEqual([]);
  });

  it("an ineligible slider that is ON (a hand-written kwarg) stays live: off is always allowed, the reason is the hint", () => {
    const sent = renderMenu(slider("fine", { ...tooMany, on: true }), undefined);
    expect(item().textContent).toBe("stop scrub-cachingtoo many positions (51 > 32)");
    expect(item().disabled).toBe(false);
    fireEvent.click(item());
    expect(sent).toEqual([{ type: "set_scrub", payload: { node: "fine", on: false } }]);
  });

  it("offers nothing for a node that is no slider, or a slider view without scrub", () => {
    const send = () => undefined;
    expect(scrubMenuItems(slider("span", eligibleOff, "construct_domain"), undefined, send)).toEqual([]);
    expect(scrubMenuItems(slider("s", undefined), undefined, send)).toEqual([]);
    expect(scrubMenuItems(slider("s", undefined), allWarm, send)).toEqual([]);
  });
});
