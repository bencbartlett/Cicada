// @vitest-environment jsdom
/**
 * The scrub-cache toggle on its two surfaces (docs/16 §Sliders, §Inspector
 * contents; v0.1 item 5 S2) — the inspector's actions row and the params
 * panel's row — rendered for real against a seeded store: checked = the
 * text's `scrub=True`; the click sends `set_scrub {node, on}` and nothing
 * else; greyed with the SERVER's reason (verbatim, in `data-blocked` and the
 * title) while an ineligible slider is off; an ineligible slider that is on
 * can be turned off; observers and `#off` ghosts see it disabled; nothing is
 * offered for a node that is not a slider.
 */
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { ClientMessage, InputView, NodeView, ScrubView } from "../protocol/messages";
import { useCicada } from "../state/store";
import { Inspector } from "./Inspector";
import { ParamsPanel } from "./ParamsPanel";

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

function port(name: string, literal: string): InputView {
  return {
    name,
    type: "Number",
    base: "Number",
    depth: 0,
    optional: false,
    required: name === "value",
    lift: 0,
    literal,
    literal_value: Number(literal),
  };
}

function sliderView(name: string, scrub: ScrubView | undefined, extra: Partial<NodeView> = {}): NodeView {
  return {
    ref: 1,
    name,
    targets: [name],
    line: 1,
    text: `${name} = slider(value=2.0, min=0.5, max=5.0, step=0.25)`,
    kind: "call",
    func: "slider",
    title: "Number Slider",
    category: "Params & input",
    inputs: [port("value", "2.0"), port("min", "0.5"), port("max", "5.0"), port("step", "0.25")],
    outputs: [{ name: "out", type: "Number", base: "Number", displayable: false }],
    param: { kind: "slider", port: "value", value: 2, min: 0.5, max: 5, step: 0.25, scrub },
    diagnostics: [],
    effectful: false,
    preview: false,
    cell: [0, 0],
    size: [8, 7],
    manual: false,
    ...extra,
  };
}

let sent: ClientMessage[];
function seed(role: "writer" | "observer", nodes: NodeView[], selected: string) {
  sent = [];
  useCicada.setState({
    connection: "open",
    role,
    catalog: null,
    graph: { nodes, wires: [], diagnostics: [] },
    selection: { nodes: [selected], wire: null, element: null },
    transport: null,
    statuses: {},
    nodeValues: {},
    notices: [],
    pending: null,
    scrubProgress: {},
    hello: { clientId: 1, role, protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unitPx: 24 },
  });
  useCicada.getState().installSender((message) => {
    sent.push(message);
    return "";
  });
}
/** The inspector asks for the selected node's values on mount (`inspect`, a read); the writes are what matters here. */
const writes = () => sent.filter((message) => message.type !== "inspect");

describe("the inspector's scrub-cache action", () => {
  afterEach(cleanup);

  it("an eligible slider that is off: `scrub-cache this slider`, unchecked, live — the click sends set_scrub on", () => {
    seed("writer", [sliderView("size", eligibleOff)], "size");
    render(<Inspector />);
    const toggle = screen.getByTestId("scrub-toggle-size") as HTMLButtonElement;
    expect(toggle.textContent).toBe("scrub-cache this slider");
    expect(toggle.getAttribute("role")).toBe("switch");
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(toggle.disabled).toBe(false);
    expect(toggle.getAttribute("data-blocked")).toBeNull();
    expect(toggle.getAttribute("data-surface")).toBe("inspector");
    fireEvent.click(toggle);
    expect(writes()).toEqual([{ type: "set_scrub", payload: { node: "size", on: true } }]);
  });

  it("an eligible slider that is on: `stop scrub-caching`, checked — the click sends set_scrub off", () => {
    seed("writer", [sliderView("size", eligibleOn)], "size");
    render(<Inspector />);
    const toggle = screen.getByTestId("scrub-toggle-size") as HTMLButtonElement;
    expect(toggle.textContent).toBe("stop scrub-caching");
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.title).toMatch(/pre-solved while the app is idle/);
    fireEvent.click(toggle);
    expect(writes()).toEqual([{ type: "set_scrub", payload: { node: "size", on: false } }]);
  });

  it("an ineligible slider that is off is greyed with the server's reason, verbatim, and the click sends nothing", () => {
    seed("writer", [sliderView("fine", tooMany)], "fine");
    render(<Inspector />);
    const toggle = screen.getByTestId("scrub-toggle-fine") as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
    expect(toggle.getAttribute("data-blocked")).toBe("too many positions (51 > 32)");
    expect(toggle.title).toMatch(/^too many positions \(51 > 32\) — /);
    fireEvent.click(toggle);
    expect(writes()).toEqual([]);
  });

  it("an ineligible slider that is ON (a hand-written kwarg) can be turned off; the reason still shows", () => {
    seed("writer", [sliderView("fine", { ...tooMany, on: true })], "fine");
    render(<Inspector />);
    const toggle = screen.getByTestId("scrub-toggle-fine") as HTMLButtonElement;
    expect(toggle.disabled).toBe(false);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(toggle.getAttribute("data-blocked")).toBe("too many positions (51 > 32)");
    expect(toggle.title).toMatch(/nothing is warmed/);
    fireEvent.click(toggle);
    expect(writes()).toEqual([{ type: "set_scrub", payload: { node: "fine", on: false } }]);
  });

  it("is disabled for an observer and for a #off ghost, and absent for a non-slider", () => {
    seed("observer", [sliderView("size", eligibleOff)], "size");
    render(<Inspector />);
    expect((screen.getByTestId("scrub-toggle-size") as HTMLButtonElement).disabled).toBe(true);
    cleanup();
    seed("writer", [sliderView("size", eligibleOff, { kind: "disabled" })], "size");
    render(<Inspector />);
    expect((screen.getByTestId("scrub-toggle-size") as HTMLButtonElement).disabled).toBe(true);
    cleanup();
    const domain = sliderView("span", undefined, {
      func: "construct_domain",
      text: "span = construct_domain(start=0.0, end=size)",
      param: undefined,
    });
    seed("writer", [domain], "span");
    render(<Inspector />);
    expect(screen.queryByTestId("scrub-toggle-span")).toBeNull();
  });
});

describe("the params row's compact toggle", () => {
  afterEach(cleanup);

  it("sits in the slider's row, reads `scrub`, carries the state and the reason, and sends set_scrub", () => {
    seed("writer", [sliderView("size", eligibleOn), sliderView("fine", tooMany)], "size");
    render(<ParamsPanel />);
    const on = screen.getByTestId("scrub-toggle-size") as HTMLButtonElement;
    expect(on.textContent).toBe("scrub");
    expect(on.className).toBe("scrub-toggle compact");
    expect(on.getAttribute("data-surface")).toBe("params");
    expect(on.getAttribute("aria-checked")).toBe("true");
    expect(screen.getByTestId("param-size").contains(on)).toBe(true);
    fireEvent.click(on);
    expect(sent).toEqual([{ type: "set_scrub", payload: { node: "size", on: false } }]);
    const blocked = screen.getByTestId("scrub-toggle-fine") as HTMLButtonElement;
    expect(blocked.disabled).toBe(true);
    expect(blocked.getAttribute("data-blocked")).toBe("too many positions (51 > 32)");
  });

  it("an observer sees the state, disabled; a toggle param has no scrub toggle", () => {
    const toggleParam: NodeView = {
      ...sliderView("flag", undefined),
      func: "toggle",
      text: "flag = toggle(value=True)",
      param: { kind: "toggle", port: "value", value: true },
    };
    seed("observer", [sliderView("size", eligibleOn), toggleParam], "size");
    render(<ParamsPanel />);
    const toggle = screen.getByTestId("scrub-toggle-size") as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(screen.queryByTestId("scrub-toggle-flag")).toBeNull();
  });
});
