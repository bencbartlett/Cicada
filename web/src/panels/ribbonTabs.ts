/**
 * Catalog → the menu bar's model (pure; unit-tested). One tab per docs/08
 * category; a tab's panel is laid out in COLUMNS, one per sub-group the
 * category's nodes fill, in the order of the catalog's `subgroups` table
 * (`cicada_core::spec::SUBGROUPS` served as format-3 `catalog.subgroups` —
 * the client keeps no copy of the order). An empty sub-group gets no column
 * (a heading over nothing is a promise the menu cannot keep — docs/17 C2c);
 * a sub-group the table does not list for the category — a project script
 * node declaring a stdlib category keeps its `Script` sub-group — trails
 * the listed ones, alphabetically, under its own name: shown, never
 * silently folded into a listed column. The function keeps the name the
 * ribbon gave it (the contract's word for the model — docs/17 M1).
 */
import { CATEGORY_ORDER, categoryLabel } from "../kinds";
import type { CatalogNode, CatalogSubgroups } from "../protocol/messages";

/** One column of a tab's panel: a sub-group and its nodes, by title. */
export interface MenuColumn {
  sub: string;
  nodes: CatalogNode[];
}

/** One tab of the menu bar: the category, its label, every node (the count), and the panel's columns. */
export interface RibbonTab {
  category: string;
  label: string;
  nodes: CatalogNode[];
  columns: MenuColumn[];
}

const byTitle = (a: CatalogNode, b: CatalogNode) => a.title.localeCompare(b.title);

/**
 * Catalog → tabs in docs/08 order (unknown categories trail,
 * alphabetically), each with its columns in `subgroups` table order
 * (unlisted sub-groups trail, alphabetically; empty ones are absent).
 */
export function ribbonTabs(nodes: CatalogNode[], subgroups: CatalogSubgroups[] = []): RibbonTab[] {
  const byCategory = new Map<string, CatalogNode[]>();
  for (const node of nodes) {
    const list = byCategory.get(node.category) ?? [];
    list.push(node);
    byCategory.set(node.category, list);
  }
  const order = new Map(subgroups.map((row) => [row.category, row.subgroups]));
  const known = CATEGORY_ORDER.filter((c) => byCategory.has(c));
  const unknown = [...byCategory.keys()].filter((c) => !CATEGORY_ORDER.includes(c)).sort();
  return [...known, ...unknown].map((category) => {
    const all = [...(byCategory.get(category) ?? [])].sort(byTitle);
    return {
      category,
      label: categoryLabel(category),
      nodes: all,
      columns: columnsOf(all, order.get(category) ?? []),
    };
  });
}

/** `nodes` (already by title) → columns: the listed sub-groups that have nodes, in table order, then the unlisted ones alphabetically. */
function columnsOf(nodes: CatalogNode[], listed: string[]): MenuColumn[] {
  const bySub = new Map<string, CatalogNode[]>();
  for (const node of nodes) {
    const list = bySub.get(node.sub) ?? [];
    list.push(node);
    bySub.set(node.sub, list);
  }
  const trailing = [...bySub.keys()].filter((sub) => !listed.includes(sub)).sort();
  return [...listed.filter((sub) => bySub.has(sub)), ...trailing].map((sub) => ({
    sub,
    nodes: bySub.get(sub) ?? [],
  }));
}
