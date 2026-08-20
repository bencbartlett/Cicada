//! Proc macros for the node ABI (DECISIONS.md struct-in/struct-out row;
//! docs/08 §The node registry):
//!
//! - `#[derive(Ports)]` reflects a struct's named fields into typed ports —
//!   a field with `#[port(default = …)]` is an optional port; a field's doc
//!   comment (its first paragraph, source lines joined) becomes the port doc.
//! - `#[node(category = "…", tier = "S", version = 1, gh = "…" | none)]`
//!   assembles the `NodeSpec` from the function — name (trailing
//!   keyword-dodging `_` stripped), title/description from the doc comment's
//!   first line (`Title — description.`), the runtime contract from its
//!   `# Panics` section, the doc of a bare single `out` port from its
//!   `# Returns` section, the runnable `.cic` snippets from its `# Examples`
//!   section (```` ```cic ```` fences), the Grasshopper component it replaces
//!   from `gh`, ports from the input struct and return type — and registers
//!   it at compile time. `gh` is required: a node either names the GH
//!   component it replaces or says `none` (DECISIONS.md stdlib row).
//!   `# Returns` is required exactly when the node returns one bare value
//!   (one doc line per port — the output struct's fields carry their own
//!   docs, a sink has no output to document).
//!
//! The macros emit paths into `cicada_core` but do not link against it;
//! consuming crates must depend on `cicada-core`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{
    Attribute, Data, DeriveInput, Expr, ExprLit, ExprUnary, Fields, FnArg, ItemFn, Lit, LitInt,
    LitStr, Meta, ReturnType, Type, UnOp, parse_macro_input,
};

/// Derives `cicada_core::spec::Ports` (and `AsOutputs`) from a struct with
/// named fields. See the crate docs for the field attribute grammar.
#[proc_macro_derive(Ports, attributes(port))]
pub fn derive_ports(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_ports(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Declares a node function: assembles and registers its `NodeSpec`.
///
/// Arguments: `category = "…"` (required, a docs/08 category),
/// `tier = "S" | "1" | "2"` (required), `version = N` (required — the
/// semantic node version in cache keys, doc 12), `gh = "Component Name"`
/// or `gh = none` (required — the Grasshopper component this node
/// replaces, or none for a Cicada-only node), `name = "…"` (optional
/// dialect-name override), `effectful` (marks impure), `uses_tolerance`
/// (folds `ProjectConfig` into the `NodeKey`, doc 49).
///
/// Doc sections: `# Panics` becomes the catalog's "Red when" contract;
/// `# Returns` (one line) becomes the doc of the single `out` port and is
/// required for — and only for — a node returning one bare value; a
/// multi-output node documents each field of its output struct instead;
/// `# Examples` must hold its `.cic` snippets in fences tagged `cic`
/// (```` ```cic ````) — a bare fence is refused, because rustdoc would
/// compile it as a Rust doctest.
#[proc_macro_attribute]
pub fn node(args: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as ItemFn);
    expand_node(&args.into(), &function)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_ports(input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "generic Ports structs arrive with the generic-node work (stage 4); \
             stage-1 port structs must be concrete",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new(
            input.span(),
            "#[derive(Ports)] requires a struct with named fields",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(
            input.span(),
            "#[derive(Ports)] requires NAMED fields — field names are the port names",
        ));
    };

    let mut port_specs = Vec::new();
    let mut from_fields = Vec::new();
    let mut into_fields = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "unnamed field in Ports struct"))?;
        // unraw: a keyword port declared `r#true` is port `true` everywhere
        // (docs/08 Pick needs literally-named `true`/`false` ports).
        let name = ident.unraw().to_string();
        let ty = &field.ty;
        let doc = doc_first_paragraph(&field.attrs);
        if doc.is_empty() {
            return Err(syn::Error::new(
                field.span(),
                format!(
                    "port `{name}` needs a doc comment — it becomes the catalog/canvas \
                     port doc (doc comments are the single documentation source, doc 14)"
                ),
            ));
        }
        let PortAttrs { default, dimension } = parse_port_attrs(&field.attrs)?;

        let (from_field, into_field) =
            marshal_field_tokens(ident, &name, ty, index, default.as_ref());
        from_fields.push(from_field);
        into_fields.push(into_field);

        let default_tokens = default.map_or_else(|| quote!(None), |(_, text)| quote!(Some(#text)));
        let dimension_tokens = match dimension.as_deref() {
            Some("length") => quote!(Some(cicada_core::spec::Dimension::Length)),
            Some("angle") => quote!(Some(cicada_core::spec::Dimension::Angle)),
            Some(other) => {
                return Err(syn::Error::new(
                    field.span(),
                    format!("unknown port dimension `{other}` — expected `length` or `angle`"),
                ));
            }
            None => quote!(None),
        };
        port_specs.push(quote! {
            cicada_core::spec::PortSpec {
                name: #name,
                ty: <#ty as cicada_core::spec::PortTyped>::TYPE,
                default: #default_tokens,
                doc: #doc,
                dimension: #dimension_tokens,
            }
        });
    }

    let ident = &input.ident;
    let ports = quote! {
        #[automatically_derived]
        impl cicada_core::spec::Ports for #ident {
            const PORTS: &'static [cicada_core::spec::PortSpec] = &[ #(#port_specs),* ];
        }
        #[automatically_derived]
        impl cicada_core::spec::AsOutputs for #ident {
            const OUTPUTS: &'static [cicada_core::spec::PortSpec] =
                <#ident as cicada_core::spec::Ports>::PORTS;
        }
    };
    let marshal = marshal_impls(input, fields.named.len(), &from_fields, &into_fields);
    Ok(quote! {
        #ports
        #marshal
    })
}

/// The `FromValues`/`IntoValues` conversion of ONE port field (stage 3's
/// marshalling layer): an absent slot takes the default TYPED — the
/// original Rust expression (via `Into`, so `&str` defaults fill `String`
/// ports), never a re-parse of the catalog string; a required port with no
/// value refuses loudly.
fn marshal_field_tokens(
    ident: &syn::Ident,
    name: &str,
    ty: &Type,
    index: usize,
    default: Option<&(Expr, String)>,
) -> (TokenStream2, TokenStream2) {
    let absent = default.map_or_else(
        || {
            quote! {
                return Err(cicada_core::marshal::InvokeError::Missing { port: #name })
            }
        },
        |(expr, _)| quote!(::std::convert::Into::into(#expr)),
    );
    let from_field = quote! {
        #ident: match &values[#index] {
            Some(value) => {
                // from_arc, not from_value: pass-through port types (Any,
                // E) clone the Arc instead of re-hashing the payload.
                <#ty as cicada_core::marshal::FromValue>::from_arc(value).map_err(
                    |source| cicada_core::marshal::InvokeError::Input { port: #name, source },
                )?
            }
            None => #absent,
        }
    };
    let into_field = quote! {
        cicada_core::marshal::IntoValue::into_value(self.#ident).map_err(
            |source| cicada_core::marshal::InvokeError::Output { port: #name, source },
        )?
    };
    (from_field, into_field)
}

/// The `FromValues`/`IntoValues` impls of one Ports struct (stage 3's
/// marshalling layer — see the crate docs).
fn marshal_impls(
    input: &DeriveInput,
    port_count: usize,
    from_fields: &[TokenStream2],
    into_fields: &[TokenStream2],
) -> TokenStream2 {
    let ident = &input.ident;
    quote! {
        #[automatically_derived]
        impl cicada_core::marshal::FromValues for #ident {
            fn from_values(
                values: &[Option<::std::sync::Arc<cicada_core::value::HashedValue>>],
            ) -> Result<Self, cicada_core::marshal::InvokeError> {
                if values.len() != #port_count {
                    return Err(cicada_core::marshal::InvokeError::Arity {
                        want: #port_count,
                        got: values.len(),
                    });
                }
                Ok(Self { #(#from_fields),* })
            }
        }
        #[automatically_derived]
        impl cicada_core::marshal::IntoValues for #ident {
            fn into_values(
                self,
            ) -> Result<
                Vec<::std::sync::Arc<cicada_core::value::HashedValue>>,
                cicada_core::marshal::InvokeError,
            > {
                Ok(vec![ #(#into_fields),* ])
            }
        }
    }
}

struct PortAttrs {
    /// The default, both as the original typed expression (marshalling
    /// inlines it) and its rendered catalog text.
    default: Option<(Expr, String)>,
    dimension: Option<String>,
}

fn parse_port_attrs(attrs: &[Attribute]) -> syn::Result<PortAttrs> {
    let mut default_expr: Option<Expr> = None;
    let mut default_doc: Option<LitStr> = None;
    let mut dimension: Option<String> = None;
    for attr in attrs {
        if !attr.path().is_ident("port") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                if default_expr.is_some() {
                    return Err(meta.error("duplicate `default` — the second would silently win"));
                }
                default_expr = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("default_doc") {
                if default_doc.is_some() {
                    return Err(
                        meta.error("duplicate `default_doc` — the second would silently win")
                    );
                }
                default_doc = Some(meta.value()?.parse()?);
                Ok(())
            } else if meta.path.is_ident("dimension") {
                if dimension.is_some() {
                    return Err(meta.error("duplicate `dimension` — the second would silently win"));
                }
                let ident: syn::Ident = meta.value()?.parse()?;
                dimension = Some(ident.to_string());
                Ok(())
            } else {
                Err(meta.error(
                    "unknown #[port(...)] key — expected `default`, `default_doc`, \
                     or `dimension`",
                ))
            }
        })?;
    }
    // A literal default renders itself; a non-literal default (docs/08's
    // `origin: Point = origin`) must carry its catalog rendering in
    // `default_doc` — one honest source, never a guessed pretty-print.
    let default = match (default_expr, default_doc) {
        (None, None) => None,
        (None, Some(doc)) => {
            return Err(syn::Error::new(
                doc.span(),
                "`default_doc` without `default` — it names a default's catalog \
                 rendering; there is no default here",
            ));
        }
        (Some(expr), doc) => {
            let rendered = match (render_literal(&expr), doc) {
                (Some(_), Some(doc)) => {
                    return Err(syn::Error::new(
                        doc.span(),
                        "`default_doc` on a LITERAL default — the literal renders \
                         itself; two sources would drift",
                    ));
                }
                (Some(text), None) => text,
                (None, Some(doc)) => doc.value(),
                (None, None) => {
                    return Err(syn::Error::new(
                        expr.span(),
                        "non-literal default needs #[port(default_doc = \"…\")] — its \
                         catalog rendering (e.g. default_doc = \"origin\")",
                    ));
                }
            };
            Some((expr, rendered))
        }
    };
    Ok(PortAttrs { default, dimension })
}

/// The catalog rendering of a plain or negated literal, or `None` for
/// non-literal expressions (which must carry `default_doc`).
fn render_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => Some(lit_text(lit)),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: inner,
            ..
        }) => {
            if let Expr::Lit(ExprLit { lit, .. }) = inner.as_ref() {
                Some(format!("-{}", lit_text(lit)))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn lit_text(lit: &Lit) -> String {
    match lit {
        Lit::Str(s) => format!("{:?}", s.value()),
        other => quote!(#other).to_string(),
    }
}

/// First paragraph of the doc comment — its source lines joined with single
/// spaces ("" when there is none). A port doc wraps at rustdoc's 80 columns
/// like any prose; taking only the first physical line truncated 28 port
/// docs mid-sentence in `catalog.json` (regression: adversarial review, C1
/// — the same defect the node title line had in stage 4).
fn doc_first_paragraph(attrs: &[Attribute]) -> String {
    doc_lines(attrs)
        .into_iter()
        .skip_while(String::is_empty)
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The body of a `# <name>` rustdoc section, joined to one line ("" if the
/// section is absent or empty). This is how a node's `# Panics` contract
/// travels into the catalog (doc 14: doc comments are the single source).
fn doc_section(attrs: &[Attribute], name: &str) -> String {
    let heading = format!("# {name}");
    let mut collected: Vec<String> = Vec::new();
    let mut inside = false;
    for line in doc_lines(attrs) {
        if inside {
            if line.starts_with("# ") {
                break; // next section
            }
            if !line.is_empty() {
                collected.push(line);
            }
        } else if line == heading {
            inside = true;
        }
    }
    collected.join(" ")
}

fn doc_lines(attrs: &[Attribute]) -> Vec<String> {
    let mut lines = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            }) = &nv.value
        {
            let value = text.value();
            // An empty `///` line is a PARAGRAPH BREAK and must survive:
            // `"".lines()` yields nothing, which silently glued adjacent
            // paragraphs together (regression: adversarial review, stage
            // 4 — `# Panics` sections leaked into catalog descriptions).
            let mut any = false;
            for line in value.lines() {
                lines.push(line.trim().to_owned());
                any = true;
            }
            if !any {
                lines.push(String::new());
            }
        }
    }
    lines
}

/// The `gh = …` value: a quoted Grasshopper component name, or the bare
/// word `none` for a Cicada-only node.
enum Gh {
    Named(LitStr),
    None,
}

struct NodeArgs {
    category: Option<LitStr>,
    tier: Option<LitStr>,
    version: Option<LitInt>,
    name: Option<LitStr>,
    gh: Option<Gh>,
    effectful: bool,
    uses_tolerance: bool,
}

fn parse_gh(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Gh> {
    let value = meta.value()?;
    if value.peek(LitStr) {
        let name: LitStr = value.parse()?;
        if name.value().trim().is_empty() || name.value().trim() != name.value() {
            return Err(syn::Error::new(
                name.span(),
                "gh = \"…\" names the Grasshopper component this node replaces — a non-empty \
                 name without surrounding whitespace, or `gh = none` for a Cicada-only node",
            ));
        }
        return Ok(Gh::Named(name));
    }
    let word: syn::Ident = value.parse()?;
    if word == "none" {
        Ok(Gh::None)
    } else {
        Err(syn::Error::new(
            word.span(),
            "gh takes a quoted Grasshopper component name (gh = \"Move\") or the bare word \
             `none` for a Cicada-only node",
        ))
    }
}

fn parse_node_args(args: &TokenStream2) -> syn::Result<NodeArgs> {
    let mut parsed = NodeArgs {
        category: None,
        tier: None,
        version: None,
        name: None,
        gh: None,
        effectful: false,
        uses_tolerance: false,
    };
    // Duplicate keys are errors — a silently-last-winning `version = 1,
    // version = 2` would corrupt cache-key semantics.
    let parser = syn::meta::parser(|meta| {
        fn set_once<T>(
            slot: &mut Option<T>,
            value: T,
            meta: &syn::meta::ParseNestedMeta<'_>,
        ) -> syn::Result<()> {
            if slot.is_some() {
                return Err(meta.error("duplicate key — the second would silently win"));
            }
            *slot = Some(value);
            Ok(())
        }
        if meta.path.is_ident("category") {
            let value = meta.value()?.parse()?;
            set_once(&mut parsed.category, value, &meta)
        } else if meta.path.is_ident("tier") {
            let value = meta.value()?.parse()?;
            set_once(&mut parsed.tier, value, &meta)
        } else if meta.path.is_ident("version") {
            let value = meta.value()?.parse()?;
            set_once(&mut parsed.version, value, &meta)
        } else if meta.path.is_ident("name") {
            let value = meta.value()?.parse()?;
            set_once(&mut parsed.name, value, &meta)
        } else if meta.path.is_ident("gh") {
            let value = parse_gh(&meta)?;
            set_once(&mut parsed.gh, value, &meta)
        } else if meta.path.is_ident("effectful") {
            parsed.effectful = true;
            Ok(())
        } else if meta.path.is_ident("uses_tolerance") {
            parsed.uses_tolerance = true;
            Ok(())
        } else {
            Err(meta.error(
                "unknown #[node(...)] key — expected category, tier, version, gh, name, \
                 effectful, or uses_tolerance",
            ))
        }
    });
    syn::parse::Parser::parse2(parser, args.clone())?;
    Ok(parsed)
}

/// Title/description from the doc comment's FIRST PARAGRAPH:
/// `Title — description…` — the paragraph may wrap across source lines
/// (rustdoc's 80-column discipline), and every line of it belongs to the
/// description; truncating at the first physical line garbled half the
/// catalog (regression: adversarial review, stage 4).
fn parse_title_line(function: &ItemFn) -> syn::Result<(String, String)> {
    let span = function.sig.span();
    // Loudly refuse anything else — this paragraph IS the catalog
    // (docs/08: "docstring first line = node title").
    let lines = doc_lines(&function.attrs);
    let first_line = lines
        .iter()
        .skip_while(|line| line.is_empty())
        .take_while(|line| !line.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let Some((title, description)) = first_line.split_once(" — ") else {
        return Err(syn::Error::new(
            span,
            "node doc comment must start with `Title — description.` \
             (em dash, spaces) — it feeds the catalog, canvas, and AI",
        ));
    };
    let (title, description) = (title.trim(), description.trim());
    if title.is_empty() || description.is_empty() {
        return Err(syn::Error::new(
            span,
            "node doc comment first line has an empty title or description",
        ));
    }
    Ok((title.to_owned(), description.to_owned()))
}

/// The `panics: …` spec-field tokens from the fn's `# Panics` doc section
/// (the runtime contract carried into the catalog — doc 14's "catalog
/// carries contracts from stage 4"). The conventional "Panics when/if "
/// opener is stripped: the catalog renders `Red when: <condition>`, and
/// a stuttered "Red when: Panics when …" would read like a bug.
fn panics_tokens(function: &ItemFn) -> TokenStream2 {
    let text = doc_section(&function.attrs, "Panics");
    let text = text
        .strip_prefix("Panics when ")
        .or_else(|| text.strip_prefix("Panics if "))
        .unwrap_or(&text);
    if text.is_empty() {
        quote!(None)
    } else {
        quote!(Some(#text))
    }
}

/// The text of the fn's `# Returns` doc section ("" when absent), joined
/// to one line: the doc of a bare single `out` port (one doc line per
/// port, DECISIONS.md stdlib row), written as a noun phrase the way every
/// input port's doc is ("The sum of `a` and `b`.").
fn returns_text(function: &ItemFn) -> String {
    doc_section(&function.attrs, "Returns")
}

/// The `examples: &[…]` spec-field tokens from the fn's `# Examples` doc
/// section: the body of every ```` ```cic ```` fence, lines joined by `\n`.
/// Prose between fences is documentation, not data. A fence with any other
/// tag (or none) is refused: rustdoc compiles a bare fence as a Rust
/// doctest — a guaranteed, confusing failure for a `.cic` snippet — and a
/// non-`cic` tag would ship an example the runner never solves.
fn examples_tokens(function: &ItemFn) -> syn::Result<TokenStream2> {
    let span = function.sig.span();
    let mut examples: Vec<String> = Vec::new();
    let mut inside_section = false;
    let mut fence: Option<Vec<String>> = None;
    for line in doc_lines(&function.attrs) {
        if let Some(body) = fence.as_mut() {
            if line.starts_with("```") {
                let snippet = body.join("\n");
                if snippet.trim().is_empty() {
                    return Err(syn::Error::new(
                        span,
                        "`# Examples` has an empty ```cic fence — write the snippet or drop \
                         the fence",
                    ));
                }
                examples.push(snippet);
                fence = None;
            } else {
                body.push(line);
            }
            continue;
        }
        if inside_section {
            if line.starts_with("# ") {
                break; // next section
            }
            if let Some(tag) = line.strip_prefix("```") {
                if tag.trim() != "cic" {
                    return Err(syn::Error::new(
                        span,
                        format!(
                            "`# Examples` fences must be tagged `cic` (```cic), got `{}` — \
                             a bare fence is compiled by rustdoc as a Rust doctest and fails; \
                             any other tag would ship an example CI never solves",
                            tag.trim()
                        ),
                    ));
                }
                fence = Some(Vec::new());
            }
        } else if line == "# Examples" {
            inside_section = true;
        }
    }
    if fence.is_some() {
        return Err(syn::Error::new(
            span,
            "`# Examples` has an unterminated ```cic fence",
        ));
    }
    Ok(quote!(&[#(#examples),*]))
}

/// The `gh: …` spec-field tokens. Required: a node either names the
/// Grasshopper component it replaces or says `none` — silence would leave
/// the catalog unable to tell "no counterpart" from "nobody looked".
fn gh_tokens(gh: Option<Gh>, span: proc_macro2::Span) -> syn::Result<TokenStream2> {
    match gh {
        Some(Gh::Named(name)) => Ok(quote!(Some(#name))),
        Some(Gh::None) => Ok(quote!(None)),
        None => Err(syn::Error::new(
            span,
            "#[node] requires gh = \"Grasshopper Component Name\" (the component this \
             node replaces, e.g. gh = \"Move\") or gh = none for a Cicada-only node — \
             the catalog and search-to-place show GH migrants the name they know",
        )),
    }
}

/// The input-struct type of a node fn (struct-in ABI): exactly one typed
/// argument, with `uses_tolerance` nodes taking the project config as an
/// explicit FIRST argument (tolerance is explicit state, never ambient).
fn input_struct_type(function: &ItemFn, uses_tolerance: bool) -> syn::Result<&Type> {
    let span = function.sig.span();
    let inputs: Vec<&FnArg> = function.sig.inputs.iter().collect();
    match (uses_tolerance, inputs.as_slice()) {
        (false, [FnArg::Typed(pat)]) => Ok(&pat.ty),
        (true, [FnArg::Typed(_config), FnArg::Typed(pat)]) => Ok(&pat.ty),
        (false, _) => Err(syn::Error::new(
            span,
            "a node takes exactly one argument: its input struct \
             (struct-in/struct-out ABI, DECISIONS.md) — e.g. `fn add(input: AddIn) -> f64`",
        )),
        (true, _) => Err(syn::Error::new(
            span,
            "a uses_tolerance node takes the config then its input struct — \
             e.g. `fn as_closed(config: &ProjectConfig, input: AsClosedIn) -> …`",
        )),
    }
}

fn expand_node(args: &TokenStream2, function: &ItemFn) -> syn::Result<TokenStream2> {
    let parsed = parse_node_args(args)?;
    let span = function.sig.span();
    if !function.sig.generics.params.is_empty() {
        // Mirror the derive's guard — asymmetric acceptance would let a
        // generic node register a spec with no bounds recorded (docs/08
        // says NodeSpec carries generic bounds; that arrives stage 4).
        return Err(syn::Error::new(
            function.sig.generics.span(),
            "generic node fns arrive with the generic-node work (stage 4); \
             stage-1 nodes must be concrete",
        ));
    }
    let category = parsed.category.ok_or_else(|| {
        syn::Error::new(
            span,
            "#[node] requires category = \"…\" (a docs/08 category)",
        )
    })?;
    let tier_lit = parsed
        .tier
        .ok_or_else(|| syn::Error::new(span, "#[node] requires tier = \"S\" | \"1\" | \"2\""))?;
    let version = parsed.version.ok_or_else(|| {
        syn::Error::new(
            span,
            "#[node] requires version = N — the semantic node version in cache keys (doc 12); \
             bump it on any behavior change",
        )
    })?;
    let tier = match tier_lit.value().as_str() {
        "S" => quote!(cicada_core::spec::Tier::S),
        "1" => quote!(cicada_core::spec::Tier::V01),
        "2" => quote!(cicada_core::spec::Tier::V02),
        other => {
            return Err(syn::Error::new(
                tier_lit.span(),
                format!("unknown tier `{other}` — expected \"S\", \"1\", or \"2\""),
            ));
        }
    };

    let gh_tokens = gh_tokens(parsed.gh, span)?;

    let (title, description) = parse_title_line(function)?;
    let panics_tokens = panics_tokens(function);
    let returns = returns_text(function);
    let examples_tokens = examples_tokens(function)?;

    let input_ty = input_struct_type(function, parsed.uses_tolerance)?;
    let output_ty: TokenStream2 = match &function.sig.output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, ty) => quote!(#ty),
    };

    // Dialect name: override, or fn ident — unrawed (`r#move` → `move`)
    // and with one keyword-dodging trailing underscore stripped (`move_`
    // registers as `move`, docs/10).
    let fn_ident = &function.sig.ident;
    let dialect_name = parsed.name.map_or_else(
        || {
            let raw = fn_ident.unraw().to_string();
            raw.strip_suffix('_').map_or(raw.clone(), str::to_owned)
        },
        |lit| lit.value(),
    );

    let pure = !parsed.effectful;
    let uses_tolerance = parsed.uses_tolerance;
    let spec_ident = format_ident!(
        "__CICADA_NODE_SPEC_{}",
        fn_ident.unraw().to_string().to_uppercase()
    );
    let invoke_ident = format_ident!("__cicada_invoke_{}", fn_ident.unraw());
    let invoke_shim = invoke_shim(&invoke_ident, fn_ident, input_ty, uses_tolerance);
    let outputs_ident = format_ident!(
        "__CICADA_NODE_OUTPUTS_{}",
        fn_ident.unraw().to_string().to_uppercase()
    );
    let outputs_static = outputs_static(&outputs_ident, &output_ty, &returns);

    Ok(quote! {
        // The struct-in ABI is by-value by design (DECISIONS.md): a node
        // that only borrows its input must not be pushed to change its
        // signature — the shim owns the calling convention.
        #[allow(clippy::needless_pass_by_value)]
        #function

        #outputs_static

        #[doc(hidden)]
        #[allow(missing_docs, non_upper_case_globals)]
        pub static #spec_ident: cicada_core::spec::NodeSpec = cicada_core::spec::NodeSpec {
            name: #dialect_name,
            title: #title,
            description: #description,
            category: #category,
            tier: #tier,
            version: #version,
            pure: #pure,
            uses_tolerance: #uses_tolerance,
            panics: #panics_tokens,
            gh: #gh_tokens,
            examples: #examples_tokens,
            inputs: <#input_ty as cicada_core::spec::Ports>::PORTS,
            outputs: &#outputs_ident,
            module: module_path!(),
            line: line!(),
        };

        #invoke_shim

        cicada_core::spec::inventory::submit! {
            cicada_core::spec::NodeRegistration {
                spec: &#spec_ident,
                invoke: #invoke_ident,
            }
        }
    })
}

/// The node's output-port static: the return type's `AsOutputs` ports with
/// the `# Returns` line attached to a bare single `out` (whose `AsOutputs`
/// impl is a per-TYPE const and cannot carry a per-node doc). Evaluated at
/// compile time; the two assertions are the format's "one doc line per
/// port" rule for outputs — a failure points at this node's `#[node]` line.
fn outputs_static(
    outputs_ident: &proc_macro2::Ident,
    output_ty: &TokenStream2,
    returns: &str,
) -> TokenStream2 {
    quote! {
        #[doc(hidden)]
        #[allow(missing_docs, non_upper_case_globals)]
        pub static #outputs_ident: [
            cicada_core::spec::PortSpec;
            <#output_ty as cicada_core::spec::AsOutputs>::OUTPUTS.len()
        ] = {
            const RAW: &[cicada_core::spec::PortSpec] =
                <#output_ty as cicada_core::spec::AsOutputs>::OUTPUTS;
            const RETURNS: &str = #returns;
            let single_out = cicada_core::spec::is_single_out(RAW);
            assert!(
                !single_out || !RETURNS.is_empty(),
                "a node returning one value needs a `# Returns` doc section (one line) — \
                 it becomes the `out` port's doc: one doc line per port, DECISIONS.md \
                 stdlib row"
            );
            assert!(
                single_out || RETURNS.is_empty(),
                "`# Returns` is for a node returning ONE bare value — a multi-output node \
                 documents each field of its output struct, and a sink returns nothing"
            );
            cicada_core::spec::documented_outputs(RAW, RETURNS)
        };
    }
}

/// The type-erased invocation shim (stage 3): marshal in, call the real fn,
/// marshal out. Panics are NOT caught here — the scheduler `catch_unwind`s
/// and turns them into red nodes (docs/12).
///
/// `uses_tolerance` nodes take the project config as an explicit first
/// argument (`fn f(config: &ProjectConfig, input: FIn)`) — tolerance is
/// explicit state, never ambient; other nodes never see it, keeping the
/// declaration and the capability in lockstep with the `NodeKey` folding.
fn invoke_shim(
    invoke_ident: &proc_macro2::Ident,
    fn_ident: &proc_macro2::Ident,
    input_ty: &Type,
    uses_tolerance: bool,
) -> TokenStream2 {
    let call = if uses_tolerance {
        quote!(#fn_ident(config, input))
    } else {
        quote!(#fn_ident(input))
    };
    quote! {
        #[doc(hidden)]
        #[allow(missing_docs)]
        fn #invoke_ident(
            config: &cicada_core::config::ProjectConfig,
            inputs: &[Option<::std::sync::Arc<cicada_core::value::HashedValue>>],
        ) -> Result<
            Vec<::std::sync::Arc<cicada_core::value::HashedValue>>,
            cicada_core::marshal::InvokeError,
        > {
            let _ = config; // tolerance-free nodes ignore it by design
            let input = <#input_ty as cicada_core::marshal::FromValues>::from_values(inputs)?;
            cicada_core::marshal::IntoValues::into_values(#call)
        }
    }
}
