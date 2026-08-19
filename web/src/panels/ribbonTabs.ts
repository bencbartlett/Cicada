/** Catalog → ribbon tabs (pure; unit-tested). */
import { CATEGORY_ORDER, categoryLabel } from "../kinds";
import type { CatalogNode } from "../protocol/messages";

export interface RibbonTab {
  category: string;
  label: string;
  nodes: CatalogNode[];
}

/** Catalog → tabs in docs/08 order (unknown categories trail, alphabetically). */
export function ribbonTabs(nodes: CatalogNode[]): RibbonTab[] {
  const byCategory = new Map<string, CatalogNode[]>();
  for (const node of nodes) {
    const list = byCategory.get(node.category) ?? [];
    list.push(node);
    byCategory.set(node.category, list);
  }
  const known = CATEGORY_ORDER.filter((c) => byCategory.has(c));
  const unknown = [...byCategory.keys()].filter((c) => !CATEGORY_ORDER.includes(c)).sort();
  return [...known, ...unknown].map((category) => ({
    category,
    label: categoryLabel(category),
    nodes: [...(byCategory.get(category) ?? [])].sort((a, b) => a.title.localeCompare(b.title)),
  }));
}
