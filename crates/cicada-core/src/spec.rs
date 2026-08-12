//! Node and port specifications — the registry's currency.
//!
//! One `NodeSpec` per registered node; the same named ports appear in Rust
//! call sites, the JSON catalog, the canvas labels, and dialect kwargs
//! (DECISIONS.md, struct-in/struct-out ABI). Stage 0 keeps port types as
//! display strings; the typed kind lattice replaces them in stage 1–2.

/// Specification of one node: what the palette, the checker, the catalog,
/// and the AI all read.
///
/// Stage 0: hand-written statics. Stage 1: assembled by `#[node]` +
/// `#[derive(Ports)]` at compile time (docs/08 §The node registry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSpec {
    /// Dialect name (`snake_case`) used in `.cic` bindings, e.g. `divide_curve`.
    pub name: &'static str,
    /// Human title — the doc comment's first line, e.g. `Divide Curve`.
    pub title: &'static str,
    /// One-line description, from the doc comment body.
    pub description: &'static str,
    /// Palette category — one of [`catalog::CATEGORY_ORDER`](crate::catalog::CATEGORY_ORDER)
    /// (docs/08 §Catalog).
    pub category: &'static str,
    /// Input ports in declaration order. A port with a default is optional.
    pub inputs: &'static [PortSpec],
    /// Output ports in declaration order. Single-output nodes use one port
    /// named `out` (docs/08).
    pub outputs: &'static [PortSpec],
}

/// One named, typed port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSpec {
    /// Field name — identical in Rust, catalog, canvas label, and dialect
    /// kwarg (one nomenclature end to end).
    pub name: &'static str,
    /// Type in catalog notation, e.g. `Number`, `[Point]`, `Curve?`
    /// (docs/08 §Catalog). Stage 1 replaces this with the typed kind lattice.
    pub ty: &'static str,
    /// Default-value literal for optional ports; `None` means required.
    pub default: Option<&'static str>,
}

impl NodeSpec {
    /// Catalog signature: `name(port: Type, …) → (port: Type, …)`, defaults
    /// as `= x`; a single output named `out` renders as its bare type
    /// (docs/08 §Catalog notation).
    #[must_use]
    pub fn signature(&self) -> String {
        let inputs = self
            .inputs
            .iter()
            .map(PortSpec::render)
            .collect::<Vec<_>>()
            .join(", ");
        let outputs = match self.outputs {
            [single] if single.name == "out" => single.ty.to_owned(),
            many => {
                let list = many
                    .iter()
                    .map(PortSpec::render)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({list})")
            }
        };
        format!("{}({inputs}) → {outputs}", self.name)
    }
}

impl PortSpec {
    /// `name: Type` or `name: Type = default`.
    #[must_use]
    pub fn render(&self) -> String {
        match self.default {
            Some(default) => format!("{}: {} = {default}", self.name, self.ty),
            None => format!("{}: {}", self.name, self.ty),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTS: &[PortSpec] = &[
        PortSpec {
            name: "curve",
            ty: "Curve",
            default: None,
        },
        PortSpec {
            name: "count",
            ty: "Integer",
            default: Some("10"),
        },
    ];
    const MULTI_OUT: &[PortSpec] = &[
        PortSpec {
            name: "points",
            ty: "[Point]",
            default: None,
        },
        PortSpec {
            name: "tangents",
            ty: "[Vector]",
            default: None,
        },
    ];
    const BARE_OUT: &[PortSpec] = &[PortSpec {
        name: "out",
        ty: "Number",
        default: None,
    }];

    #[test]
    fn signature_multi_output_with_default() {
        let spec = NodeSpec {
            name: "divide_curve",
            title: "Divide Curve",
            description: "Points and tangents along a curve.",
            category: "Curve",
            inputs: POINTS,
            outputs: MULTI_OUT,
        };
        assert_eq!(
            spec.signature(),
            "divide_curve(curve: Curve, count: Integer = 10) → (points: [Point], tangents: [Vector])"
        );
    }

    #[test]
    fn signature_single_output_renders_bare_type() {
        let spec = NodeSpec {
            name: "add",
            title: "Add",
            description: "Sum.",
            category: "Maths & logic",
            inputs: &[],
            outputs: BARE_OUT,
        };
        assert_eq!(spec.signature(), "add() → Number");
    }
}
