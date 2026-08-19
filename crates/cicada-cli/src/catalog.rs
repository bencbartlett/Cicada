//! `cicada catalog`: regenerate (or verify) `docs/generated/CATALOG.md` +
//! `catalog.json` from the node registry (doc 14 §Documentation pipeline).

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, bail};
use cicada_core::spec::{Dimension, NodeSpec, PortSpec, Tier};

/// Repo-root generated-docs dir, resolved from this crate's manifest
/// directory so `cargo run -p cicada-cli` works from any working directory.
fn default_generated_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/generated")
}

/// Write (or `--check`) the generated catalog files.
///
/// # Errors
///
/// I/O problems, or staleness in check mode.
pub fn catalog(check: bool, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let dir = dir.unwrap_or_else(default_generated_dir);
    let specs = cicada_stdlib::registry();
    let markdown = cicada_core::catalog::render_markdown(specs);
    let json = catalog_json(specs)?;

    let outputs: [(&str, &str); 2] = [("CATALOG.md", &markdown), ("catalog.json", &json)];
    if check {
        for (file, rendered) in outputs {
            let path = dir.join(file);
            // Byte-exact on purpose: .gitattributes pins LF everywhere, so
            // any difference — including line endings — is real staleness.
            let committed =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            if committed != rendered {
                bail!(
                    "{} is stale — regenerate with `cargo run -p cicada-cli -- catalog` \
                     and commit the diff",
                    path.display()
                );
            }
            println!("catalog fresh: {}", path.display());
        }
    } else {
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for (file, rendered) in outputs {
            let path = dir.join(file);
            fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
            println!("catalog written: {}", path.display());
        }
    }
    Ok(())
}

/// The machine-readable catalog (DECISIONS.md: consumed by the palette, the
/// checker, and the AI). Format field bumps on breaking shape changes.
fn catalog_json(specs: &[&NodeSpec]) -> anyhow::Result<String> {
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
        /// The runtime contract (rustdoc `# Panics`, one line): when the
        /// node goes red. Format 2.
        #[serde(skip_serializing_if = "Option::is_none")]
        panics: Option<&'a str>,
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

    let catalog = Catalog {
        // Format 2: adds per-node `panics` (the runtime contract).
        format: 2,
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
                inputs: spec.inputs.iter().map(port).collect(),
                outputs: spec.outputs.iter().map(port).collect(),
            })
            .collect(),
    };
    let mut json = serde_json::to_string_pretty(&catalog).context("serializing catalog.json")?;
    json.push('\n');
    Ok(json)
}
