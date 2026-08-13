//! Node and port specifications — the registry's currency.
//!
//! One `NodeSpec` per registered node; the same named ports appear in Rust
//! call sites, the JSON catalog, the canvas labels, and dialect kwargs
//! (DECISIONS.md, struct-in/struct-out ABI). From stage 1, specs are
//! assembled by `#[node]` + `#[derive(Ports)]` (cicada-macros) and
//! registered at compile time through `inventory`.

use std::collections::BTreeSet;

/// Compile-time registration plumbing for the `#[node]` macro. Not public
/// API — subject to change without a version bump.
#[doc(hidden)]
pub use inventory;

/// A port's wire type in catalog notation: a base kind name plus list
/// nesting and element optionality. `{base: "Point", list_depth: 1,
/// optional: false}` renders `[Point]`; `optional` renders `T?` (docs/08
/// notation, docs/09 slot semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortType {
    /// Base kind name, e.g. `Number`, `Point`.
    pub base: &'static str,
    /// List nesting depth: 0 = scalar, 1 = `[T]`, 2 = `[[T]]`.
    pub list_depth: u8,
    /// Element-level optionality (`T?` — slot-preserving nulls).
    pub optional: bool,
}

impl PortType {
    /// A plain scalar type.
    #[must_use]
    pub const fn named(base: &'static str) -> Self {
        Self {
            base,
            list_depth: 0,
            optional: false,
        }
    }

    /// Catalog notation: `Number`, `Point?`, `[Point]`, `[[Number]]`.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for _ in 0..self.list_depth {
            out.push('[');
        }
        out.push_str(self.base);
        if self.optional {
            out.push('?');
        }
        for _ in 0..self.list_depth {
            out.push(']');
        }
        out
    }
}

/// Physical dimension tag on a port (DECISIONS.md units row): lets unit
/// conversion know a `radius` from a `count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    /// Lengths in document units — rescaled by unit conversion.
    Length,
    /// Angles in radians — never rescaled.
    Angle,
}

/// Catalog tier (docs/08): spike set, v0.1, v0.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Vertical-slice spike set.
    S,
    /// v0.1 (full mesh-tier catalog).
    V01,
    /// v0.2 (B-rep tier and stragglers).
    V02,
}

/// One named, typed port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortSpec {
    /// Field name — identical in Rust, catalog, canvas label, and dialect
    /// kwarg (one nomenclature end to end).
    pub name: &'static str,
    /// Wire type.
    pub ty: PortType,
    /// Default-value literal for optional ports (`None` = required).
    pub default: Option<&'static str>,
    /// One-line port doc, from the field's doc comment.
    pub doc: &'static str,
    /// Physical dimension, when declared (`#[port(dimension = length)]`).
    pub dimension: Option<Dimension>,
}

/// Specification of one node: what the palette, the checker, the catalog,
/// and the AI all read.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    /// Dialect name (`snake_case`) used in `.cic` bindings. The Rust fn may
    /// carry a keyword-dodging trailing underscore (`move_`); this name
    /// never does (`move`).
    pub name: &'static str,
    /// Human title — the doc comment's first line before ` — `.
    pub title: &'static str,
    /// One-line description — the first line after ` — `.
    pub description: &'static str,
    /// Palette category — one of [`crate::catalog::CATEGORY_ORDER`]
    /// (docs/08 §Catalog).
    pub category: &'static str,
    /// Catalog tier.
    pub tier: Tier,
    /// Explicit semantic node version (doc 12 cache keys): bumped on any
    /// behavior change; recompiling never invalidates caches, changing
    /// meaning does.
    pub version: u32,
    /// False for effectful nodes (exporters — run only on explicit action).
    pub pure: bool,
    /// True when the node consults `ProjectConfig` tolerances — the
    /// tolerance hash joins the `NodeKey` (DECISIONS.md tolerance row).
    pub uses_tolerance: bool,
    /// Input ports in declaration order. A port with a default is optional.
    pub inputs: &'static [PortSpec],
    /// Output ports in declaration order. Single-output nodes use one port
    /// named `out`.
    pub outputs: &'static [PortSpec],
    /// Defining module (`module_path!`) — catalog sort key within category.
    pub module: &'static str,
    /// Defining line (`line!`) — catalog sort key within module.
    pub line: u32,
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
            [single] if single.name == "out" => single.ty.render(),
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
            Some(default) => format!("{}: {} = {default}", self.name, self.ty.render()),
            None => format!("{}: {}", self.name, self.ty.render()),
        }
    }
}

/// Maps a Rust type to its wire [`PortType`]. Implemented for the core leaf
/// kinds here and composed structurally: `Vec<T>` adds a list level,
/// `Option<T>` marks element optionality. Downstream crates implement it for
/// their kinds via [`impl_port_leaf!`](crate::impl_port_leaf).
pub trait PortTyped {
    /// The wire type of this Rust type.
    const TYPE: PortType;
}

/// Reflects a struct's fields as ports. Derived with `#[derive(Ports)]`
/// (cicada-macros); input structs of `#[node]` fns must implement it.
pub trait Ports {
    /// The ports, in field declaration order.
    const PORTS: &'static [PortSpec];
}

/// The output ports of a node, from its return type: a `Ports` struct
/// contributes its fields; a bare [`PortTyped`] value is one port named
/// `out`; `()` is a sink with no outputs.
pub trait AsOutputs {
    /// The output ports.
    const OUTPUTS: &'static [PortSpec];
}

impl<T: PortTyped> AsOutputs for Vec<T> {
    const OUTPUTS: &'static [PortSpec] = &[PortSpec {
        name: "out",
        ty: <Vec<T> as PortTyped>::TYPE,
        default: None,
        doc: "",
        dimension: None,
    }];
}

impl<T: PortTyped> AsOutputs for Option<T> {
    const OUTPUTS: &'static [PortSpec] = &[PortSpec {
        name: "out",
        ty: <Option<T> as PortTyped>::TYPE,
        default: None,
        doc: "",
        dimension: None,
    }];
}

impl AsOutputs for () {
    const OUTPUTS: &'static [PortSpec] = &[];
}

impl<T: PortTyped> PortTyped for Vec<T> {
    const TYPE: PortType = PortType {
        base: T::TYPE.base,
        list_depth: T::TYPE.list_depth + 1,
        optional: T::TYPE.optional,
    };
}

impl<T: PortTyped> PortTyped for Option<T> {
    // `Option` models ELEMENT-level optionality (`T?`, docs/09). A whole
    // optional list (`[T]?`) has no PortType representation yet, so
    // `Option<Vec<T>>` / `Option<Option<T>>` must refuse at compile time —
    // silently rendering them as `[T?]` would ship a wrong catalog type.
    const TYPE: PortType = {
        assert!(
            T::TYPE.list_depth == 0 && !T::TYPE.optional,
            "Option<T> ports support element-level optionality only (T?); an optional \
             list ([T]?) needs a PortType representation change first (docs/09)"
        );
        PortType {
            base: T::TYPE.base,
            list_depth: 0,
            optional: true,
        }
    };
}

/// Implements [`PortTyped`] (and single-`out` [`AsOutputs`]) for a leaf
/// kind. Used by core below and by downstream crates for their kinds:
/// `cicada_core::impl_port_leaf!(Mesh, "Mesh");`
#[macro_export]
macro_rules! impl_port_leaf {
    ($ty:ty, $name:literal) => {
        impl $crate::spec::PortTyped for $ty {
            const TYPE: $crate::spec::PortType = $crate::spec::PortType::named($name);
        }
        impl $crate::spec::AsOutputs for $ty {
            const OUTPUTS: &'static [$crate::spec::PortSpec] = &[$crate::spec::PortSpec {
                name: "out",
                ty: $crate::spec::PortType::named($name),
                default: None,
                doc: "",
                dimension: None,
            }];
        }
    };
}

impl_port_leaf!(f64, "Number");
impl_port_leaf!(i64, "Integer");
impl_port_leaf!(bool, "Boolean");
impl_port_leaf!(String, "Text");
impl_port_leaf!(crate::scalar::Color, "Color");
impl_port_leaf!(crate::scalar::Domain, "Domain");
impl_port_leaf!(crate::scalar::IndexMap, "IndexMap");
impl_port_leaf!(crate::spatial::Point, "Point");
impl_port_leaf!(crate::spatial::Vector, "Vector");
impl_port_leaf!(crate::spatial::Plane, "Plane");
impl_port_leaf!(crate::spatial::Xform, "Xform");

/// One compile-time node registration. `#[node]` submits these; never
/// construct by hand.
pub struct NodeRegistration {
    /// The registered spec.
    pub spec: &'static NodeSpec,
}

inventory::collect!(NodeRegistration);

/// Every node registered in the linked binary, in canonical catalog order:
/// docs/08 category order, then (module path, line) within a category —
/// alphabetical by module, source order within a module. Computed once and
/// cached (hot callers: palette search, checker).
///
/// # Panics
///
/// Panics (once, at first query) if two registrations share a dialect name
/// — a compile-time bug surfaced loudly rather than shipping an ambiguous
/// catalog.
#[must_use]
pub fn registered() -> &'static [&'static NodeSpec] {
    static REGISTERED: std::sync::OnceLock<Vec<&'static NodeSpec>> = std::sync::OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            let mut specs: Vec<&'static NodeSpec> = inventory::iter::<NodeRegistration>
                .into_iter()
                .map(|registration| registration.spec)
                .collect();
            specs.sort_by_key(|spec| {
                (
                    crate::catalog::category_rank(spec.category),
                    spec.module,
                    spec.line,
                )
            });
            let mut seen = BTreeSet::new();
            for spec in &specs {
                assert!(
                    seen.insert(spec.name),
                    "duplicate node name `{}` registered (second at {}:{})",
                    spec.name,
                    spec.module,
                    spec.line
                );
            }
            specs
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIVIDE_IN: &[PortSpec] = &[
        PortSpec {
            name: "curve",
            ty: PortType::named("Curve"),
            default: None,
            doc: "",
            dimension: None,
        },
        PortSpec {
            name: "count",
            ty: PortType::named("Integer"),
            default: Some("10"),
            doc: "",
            dimension: None,
        },
    ];
    const DIVIDE_OUT: &[PortSpec] = &[
        PortSpec {
            name: "points",
            ty: PortType {
                base: "Point",
                list_depth: 1,
                optional: false,
            },
            default: None,
            doc: "",
            dimension: None,
        },
        PortSpec {
            name: "tangents",
            ty: PortType {
                base: "Vector",
                list_depth: 1,
                optional: false,
            },
            default: None,
            doc: "",
            dimension: None,
        },
    ];

    #[test]
    fn port_type_rendering() {
        assert_eq!(PortType::named("Number").render(), "Number");
        assert_eq!(
            PortType {
                base: "Point",
                list_depth: 1,
                optional: false
            }
            .render(),
            "[Point]"
        );
        assert_eq!(
            PortType {
                base: "Number",
                list_depth: 2,
                optional: false
            }
            .render(),
            "[[Number]]"
        );
        assert_eq!(
            PortType {
                base: "Curve",
                list_depth: 1,
                optional: true
            }
            .render(),
            "[Curve?]"
        );
    }

    #[test]
    fn structural_port_typing_composes() {
        assert_eq!(<Vec<Vec<f64>> as PortTyped>::TYPE.render(), "[[Number]]");
        assert_eq!(
            <Vec<Option<crate::spatial::Point>> as PortTyped>::TYPE.render(),
            "[Point?]"
        );
        assert_eq!(<Option<i64> as PortTyped>::TYPE.render(), "Integer?");
    }

    #[test]
    fn signature_multi_output_with_default() {
        let spec = NodeSpec {
            name: "divide_curve",
            title: "Divide Curve",
            description: "Points and tangents along a curve.",
            category: "Curve",
            tier: Tier::S,
            version: 1,
            pure: true,
            uses_tolerance: false,
            inputs: DIVIDE_IN,
            outputs: DIVIDE_OUT,
            module: "test",
            line: 0,
        };
        assert_eq!(
            spec.signature(),
            "divide_curve(curve: Curve, count: Integer = 10) → (points: [Point], tangents: [Vector])"
        );
    }
}
