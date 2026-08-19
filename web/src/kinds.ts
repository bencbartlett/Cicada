/**
 * Kind families → hues (docs/16 §Canvas conventions: one hue per kind
 * family, stable everywhere — ports, wires, badges, inspector). Refinements
 * share their base kind's hue; type variables and `Any` are neutral.
 */

const FAMILY: Record<string, string> = {
  Number: "number",
  Integer: "number",
  Boolean: "boolean",
  Text: "text",
  Color: "color",
  Domain: "domain",
  IndexMap: "domain",
  Point: "point",
  Vector: "vector",
  Plane: "plane",
  Xform: "plane",
  Curve: "curve",
  "Closed<Curve>": "curve",
  Mesh: "mesh",
  "Watertight<Mesh>": "mesh",
  Geometry: "geometry",
};

/** CSS custom-property name carrying the family hue. */
export function kindFamily(base: string): string {
  return FAMILY[base] ?? "neutral";
}

/** The kind color as a CSS `var(--kind-…)` expression. */
export function kindColor(base: string): string {
  return `var(--kind-${kindFamily(base)})`;
}

/** Strip list brackets / optional marks from a rendered type: `[Point?]` → `Point`. */
export function baseOfType(type: string): string {
  let t = type.trim();
  while (t.startsWith("[") && t.endsWith("]")) t = t.slice(1, -1);
  if (t.endsWith("?")) t = t.slice(0, -1);
  return t;
}

/** List depth of a rendered type. */
export function depthOfType(type: string): number {
  let depth = 0;
  let t = type.trim();
  while (t.startsWith("[") && t.endsWith("]")) {
    depth += 1;
    t = t.slice(1, -1);
  }
  return depth;
}

/** The docs/08 category order, for the ribbon (mirrors core::catalog::CATEGORY_ORDER). */
export const CATEGORY_ORDER = [
  "Params & input",
  "Sequences & random",
  "Maths & logic",
  "List & axis",
  "Point · Vector · Plane",
  "Curve",
  "Surface & solid",
  "Mesh & field",
  "Intersect & regions",
  "Transform",
  "Output, display & export",
  "Script",
];

/** Short ribbon tab label per category. */
export function categoryLabel(category: string): string {
  const short: Record<string, string> = {
    "Params & input": "Params",
    "Sequences & random": "Sets",
    "Maths & logic": "Maths",
    "List & axis": "List",
    "Point · Vector · Plane": "Vector",
    Curve: "Curve",
    "Surface & solid": "Surface",
    "Mesh & field": "Mesh",
    "Intersect & regions": "Intersect",
    Transform: "Transform",
    "Output, display & export": "Display",
    Script: "Project",
  };
  return short[category] ?? category;
}
