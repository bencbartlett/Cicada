// @vitest-environment jsdom
/**
 * The menu bar (docs/16 §Application layout; docs/17 wave 5 M1) rendered
 * for real against a seeded store and the COMMITTED catalog (its
 * `subgroups` table is the server's): the bar is tabs alone until a click
 * opens a panel whose columns are the category's filled sub-groups in table
 * order; hovering another tab while open switches; a re-click, Esc, an
 * outside pointerdown and a placement close it; a placement is ONE
 * `place_node` at the store's `canvasCenter`; observers get disabled node
 * buttons with the reason in the hover, and can still browse.
 */
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import type { Catalog, ClientMessage } from "../protocol/messages";
import { useCicada } from "../state/store";
import { MenuBar } from "./MenuBar";

const here = dirname(fileURLToPath(import.meta.url));
const catalog = JSON.parse(readFileSync(resolve(here, "../../../docs/generated/catalog.json"), "utf8")) as Catalog;
const mathsRow = catalog.subgroups.find((row) => row.category === "Maths & logic")!.subgroups;
const mathsCount = catalog.nodes.filter((n) => n.category === "Maths & logic").length;

let sent: ClientMessage[];
function seed(role: "writer" | "observer", center: [number, number] | null = [12, 7]) {
  sent = [];
  useCicada.setState({
    connection: "open",
    role,
    catalog,
    canvasCenter: center,
    notices: [],
    hello: { clientId: 1, role, protocol: 1, engine: "x", project: "p", pipeline: "p.cic", unitPx: 24 },
  });
  useCicada.getState().installSender((message) => {
    sent.push(message);
    return "";
  });
}

const panel = () => screen.queryByTestId("menu-panel");
const columns = () =>
  Array.from(panel()?.querySelectorAll<HTMLElement>("[data-testid^='menu-col-']") ?? []).map((el) => el.getAttribute("aria-label"));
const click = (testId: string) => fireEvent.click(screen.getByTestId(testId));

describe("the menu bar", () => {
  afterEach(cleanup);

  it("is tabs alone until a click: label · count per category in docs/08 order, no panel", () => {
    seed("writer");
    render(<MenuBar />);
    const bar = screen.getByTestId("menubar");
    const tabs = Array.from(bar.querySelectorAll<HTMLElement>("[data-testid^='menu-tab-']"));
    expect(tabs.map((t) => t.getAttribute("data-testid"))).toEqual([
      "menu-tab-Params",
      "menu-tab-Sets",
      "menu-tab-Maths",
      "menu-tab-List",
      "menu-tab-Vector",
      "menu-tab-Curve",
      "menu-tab-Surface",
      "menu-tab-Mesh",
      "menu-tab-Intersect",
      "menu-tab-Transform",
      "menu-tab-Display",
    ]);
    expect(screen.getByTestId("menu-tab-Maths").textContent).toBe(`Maths${mathsCount}`);
    expect(screen.getByTestId("menu-tab-Maths").getAttribute("aria-expanded")).toBe("false");
    expect(panel()).toBeNull();
    // Nothing to collapse: the wave-4 control is gone with the ribbon.
    expect(screen.queryByText(/collapse/)).toBeNull();
  });

  it("a click opens the tab's panel: one column per filled sub-group in the catalog's table order, nodes under their column", () => {
    seed("writer");
    render(<MenuBar />);
    click("menu-tab-Maths");
    expect(panel()).not.toBeNull();
    expect(panel()!.getAttribute("aria-label")).toBe("Maths & logic");
    expect(screen.getByTestId("menu-tab-Maths").getAttribute("aria-expanded")).toBe("true");
    expect(columns()).toEqual(mathsRow); // every Maths column is filled in the stdlib
    expect(columns().length).toBeGreaterThanOrEqual(2);
    const operators = screen.getByTestId("menu-col-Operators");
    expect(operators.querySelector("[data-testid='menu-node-add']")).not.toBeNull();
    expect(screen.getByTestId("menu-col-Trig").querySelector("[data-testid='menu-node-sin']")).not.toBeNull();
    // Title and name on the button; the hover carries the description.
    const add = screen.getByTestId("menu-node-add");
    expect(add.querySelector(".mb-node-title")!.textContent).toBe(catalog.nodes.find((n) => n.name === "add")!.title);
    expect(add.querySelector(".mb-node-name")!.textContent).toBe("add");
    expect(add.getAttribute("title")).toMatch(/\S/);
    expect(add.getAttribute("title")).not.toMatch(/read-only/);
  });

  it("the hover carries the GH name when it says something the title does not, and the node's contract", () => {
    seed("writer");
    render(<MenuBar />);
    click("menu-tab-List");
    // concat replaces GH's Merge: the hint is in the hover.
    expect(screen.getByTestId("menu-node-concat").getAttribute("title")).toMatch(/^.*\nGH: Merge(\n|$)/);
    // A node with a `# Panics` contract shows it as "Red when".
    const red = catalog.nodes.find((n) => n.category === "List & axis" && n.panics !== undefined);
    expect(red, "a List node with a contract").toBeDefined();
    expect(screen.getByTestId(`menu-node-${red!.name}`).getAttribute("title")).toContain(`Red when: ${red!.panics!.trim()}`);
  });

  it("hovering another tab while a panel is open switches to it; hovering while closed opens nothing", () => {
    seed("writer");
    render(<MenuBar />);
    fireEvent.pointerOver(screen.getByTestId("menu-tab-Vector"));
    expect(panel()).toBeNull();
    click("menu-tab-Maths");
    fireEvent.pointerOver(screen.getByTestId("menu-tab-Vector"));
    expect(panel()!.getAttribute("aria-label")).toBe("Point · Vector · Plane");
    expect(columns()).toEqual(["Point", "Vector", "Plane"]);
    expect(screen.getByTestId("menu-tab-Maths").getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByTestId("menu-tab-Vector").getAttribute("aria-expanded")).toBe("true");
  });

  it("a re-click of the open tab, Esc, and a pointerdown outside the bar each close the panel; a pointerdown on the bar does not", () => {
    seed("writer");
    render(<MenuBar />);
    click("menu-tab-Maths");
    click("menu-tab-Maths");
    expect(panel()).toBeNull();

    click("menu-tab-Maths");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(panel()).toBeNull();

    click("menu-tab-Maths");
    fireEvent.pointerDown(screen.getByTestId("menu-col-Trig"));
    expect(panel(), "inside the bar: still open").not.toBeNull();
    fireEvent.pointerDown(document.body);
    expect(panel()).toBeNull();

    // Closed: the listeners are gone — a stray Esc changes nothing and throws nothing.
    fireEvent.keyDown(window, { key: "Escape" });
    expect(panel()).toBeNull();
  });

  it("a node click is ONE place_node at the canvas's centre cell, and it closes the panel", () => {
    seed("writer", [12, 7]);
    render(<MenuBar />);
    click("menu-tab-Maths");
    click("menu-node-add");
    expect(sent).toEqual([{ type: "place_node", payload: { func: "add", cell: [12, 7] } }]);
    expect(panel()).toBeNull();
    expect(useCicada.getState().notices).toEqual([]);
  });

  it("without a reported centre (no canvas yet) the placement carries null and the server lays the node out", () => {
    seed("writer", null);
    render(<MenuBar />);
    click("menu-tab-Params");
    click("menu-node-slider");
    expect(sent).toEqual([{ type: "place_node", payload: { func: "slider", cell: null } }]);
  });

  it("an observer can browse — tabs open, columns shown — but every node button is disabled with the reason in the hover, and nothing is sent", () => {
    seed("observer");
    render(<MenuBar />);
    click("menu-tab-Maths");
    expect(columns()).toEqual(mathsRow);
    const buttons = Array.from(panel()!.querySelectorAll<HTMLButtonElement>("button.mb-node"));
    expect(buttons.length).toBe(mathsCount);
    for (const button of buttons) {
      expect(button.disabled).toBe(true);
      expect(button.title).toMatch(/read-only — take the lease/);
    }
    fireEvent.click(screen.getByTestId("menu-node-add"));
    expect(sent).toEqual([]);
  });

  it("says the catalog is loading while there is none", () => {
    seed("writer");
    act(() => useCicada.setState({ catalog: null }));
    render(<MenuBar />);
    expect(screen.getByText("catalog loading…")).not.toBeNull();
    expect(screen.getByTestId("menubar").querySelectorAll("[data-testid^='menu-tab-']").length).toBe(0);
  });
});
