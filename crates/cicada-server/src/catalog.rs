//! The machine-readable catalog (`catalog.json`, format 2 — DECISIONS.md:
//! consumed by the palette, the checker, and the AI). Rendered here so the
//! `GET /api/catalog` route and `cicada catalog` emit the same bytes; the
//! server's copy is project-aware (stdlib + the session's script nodes,
//! ledger revision 2026-08-18) while the CLI's committed file is the
//! stdlib alone.

use cicada_core::spec::{Dimension, NodeSpec, PortSpec, Tier};

/// The catalog format version this build emits. Bumps on breaking shape
/// changes; additive fields keep it.
pub const CATALOG_FORMAT: u32 = 2;

#[derive(serde::Serialize)]
struct Catalog<'a> {
    format: u32,
    nodes: Vec<Node<'a>>,
}

#[derive(serde::Serialize)]
struct Node<'a> {
    name: &'a str,
    title: &'a str,
    description: &'a str,
    category: &'a str,
    tier: &'a str,
    version: u32,
    pure: bool,
    uses_tolerance: bool,
    /// The runtime contract (rustdoc `# Panics`, one line): when the node
    /// goes red. Format 2.
    #[serde(skip_serializing_if = "Option::is_none")]
    panics: Option<&'a str>,
    /// The Grasshopper component this node replaces, or null for a
    /// Cicada-only node — always present (the attribute is required), so
    /// a migrant's search can match GH names. Format 2, additive.
    gh: Option<&'a str>,
    /// Runnable `.cic` snippets (no `# cicada 1` header), solved by CI;
    /// empty for script nodes. Format 2, additive.
    examples: Vec<&'a str>,
    inputs: Vec<Port<'a>>,
    outputs: Vec<Port<'a>>,
}

#[derive(serde::Serialize)]
struct Port<'a> {
    name: &'a str,
    // Both forms: the rendered notation for humans/AI, the structured
    // fields so the palette/checker never parse bracket syntax
    // (DECISIONS.md: the JSON catalog drives palette, checker, AI).
    #[serde(rename = "type")]
    ty: String,
    base: &'a str,
    list_depth: u8,
    optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<&'a str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    doc: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimension: Option<&'a str>,
}

fn port(spec: &PortSpec) -> Port<'_> {
    Port {
        name: spec.name,
        ty: spec.ty.render(),
        base: spec.ty.base,
        list_depth: spec.ty.list_depth,
        optional: spec.ty.optional,
        default: spec.default,
        doc: spec.doc,
        dimension: spec.dimension.map(|d| match d {
            Dimension::Length => "length",
            Dimension::Angle => "angle",
        }),
    }
}

fn build<'a>(specs: &[&'a NodeSpec]) -> Catalog<'a> {
    Catalog {
        format: CATALOG_FORMAT,
        nodes: specs
            .iter()
            .map(|spec| Node {
                name: spec.name,
                title: spec.title,
                description: spec.description,
                category: spec.category,
                tier: match spec.tier {
                    Tier::S => "S",
                    Tier::V01 => "1",
                    Tier::V02 => "2",
                },
                version: spec.version,
                pure: spec.pure,
                uses_tolerance: spec.uses_tolerance,
                panics: spec.panics,
                gh: spec.gh,
                examples: spec.examples.to_vec(),
                inputs: spec.inputs.iter().map(port).collect(),
                outputs: spec.outputs.iter().map(port).collect(),
            })
            .collect(),
    }
}

/// The catalog as a `serde_json::Value` (the API route serves it; key
/// order is the Value's — clients read fields by name).
#[must_use]
pub fn catalog_value(specs: &[&NodeSpec]) -> serde_json::Value {
    // Serializing a struct of strings and bools cannot fail.
    serde_json::to_value(build(specs)).unwrap_or(serde_json::Value::Null)
}

/// The pretty JSON text `cicada catalog` writes to `docs/generated/`
/// (trailing newline included). Serialized straight from the structs so
/// field order — and therefore the committed bytes — stay exactly what
/// stage 1 established (the freshness check is byte-exact).
///
/// # Errors
///
/// `serde_json` failures — practically unreachable for this shape.
pub fn render_json(specs: &[&NodeSpec]) -> Result<String, serde_json::Error> {
    let mut json = serde_json::to_string_pretty(&build(specs))?;
    json.push('\n');
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_format_2_with_structured_ports() {
        let specs = cicada_stdlib::registry();
        let value = catalog_value(specs);
        assert_eq!(value["format"], CATALOG_FORMAT);
        let nodes = value["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), specs.len());
        let slider = nodes.iter().find(|n| n["name"] == "slider").unwrap();
        assert_eq!(slider["inputs"][0]["base"], "Number");
        assert!(
            slider["panics"].is_string(),
            "format 2 carries the contract"
        );
        assert_eq!(
            slider["gh"], "Number Slider",
            "format 2 carries the GH name"
        );
        assert!(
            slider["examples"].as_array().is_some_and(|e| !e.is_empty()),
            "format 2 carries the runnable examples"
        );
        // A Cicada-only node says so explicitly: null, never absent.
        let as_closed = nodes.iter().find(|n| n["name"] == "as_closed").unwrap();
        assert!(as_closed["gh"].is_null() && as_closed.get("gh").is_some());
        assert!(render_json(specs).unwrap().ends_with('\n'));
    }
}
