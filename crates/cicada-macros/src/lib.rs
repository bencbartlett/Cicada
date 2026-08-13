//! Proc macros for the node ABI (DECISIONS.md struct-in/struct-out row;
//! docs/08 §The node registry):
//!
//! - `#[derive(Ports)]` reflects a struct's named fields into typed ports —
//!   a field with `#[port(default = …)]` is an optional port; field doc
//!   comments become port docs.
//! - `#[node(category = "…", tier = "S", version = 1)]` assembles the
//!   `NodeSpec` from the function — name (trailing keyword-dodging `_`
//!   stripped), title/description from the doc comment's first line
//!   (`Title — description.`), ports from the input struct and return type —
//!   and registers it at compile time.
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
/// semantic node version in cache keys, doc 12), `name = "…"` (optional
/// dialect-name override), `effectful` (marks impure), `uses_tolerance`
/// (folds `ProjectConfig` into the `NodeKey`, doc 49).
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
        let doc = doc_first_line(&field.attrs);
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
                <#ty as cicada_core::marshal::FromValue>::from_value(value).map_err(
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
    /// inlines it) and its rendered catalog literal.
    default: Option<(Expr, String)>,
    dimension: Option<String>,
}

fn parse_port_attrs(attrs: &[Attribute]) -> syn::Result<PortAttrs> {
    let mut out = PortAttrs {
        default: None,
        dimension: None,
    };
    for attr in attrs {
        if !attr.path().is_ident("port") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("default") {
                if out.default.is_some() {
                    return Err(meta.error("duplicate `default` — the second would silently win"));
                }
                let expr: Expr = meta.value()?.parse()?;
                let rendered = render_default(&expr)?;
                out.default = Some((expr, rendered));
                Ok(())
            } else if meta.path.is_ident("dimension") {
                if out.dimension.is_some() {
                    return Err(meta.error("duplicate `dimension` — the second would silently win"));
                }
                let ident: syn::Ident = meta.value()?.parse()?;
                out.dimension = Some(ident.to_string());
                Ok(())
            } else {
                Err(meta.error("unknown #[port(...)] key — expected `default` or `dimension`"))
            }
        })?;
    }
    Ok(out)
}

/// Render a default expression as the catalog literal. Plain and negated
/// literals only — non-literal defaults (`Point::new(…)`, docs/08's
/// `origin: Point = origin`) are refused until their catalog rendering is
/// designed with the stage-4 nodes that need them.
fn render_default(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => Ok(lit_text(lit)),
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: inner,
            ..
        }) => {
            if let Expr::Lit(ExprLit { lit, .. }) = inner.as_ref() {
                Ok(format!("-{}", lit_text(lit)))
            } else {
                Err(non_literal_default(expr))
            }
        }
        other => Err(non_literal_default(other)),
    }
}

fn non_literal_default(expr: &Expr) -> syn::Error {
    syn::Error::new(
        expr.span(),
        "#[port(default = …)] takes a literal (or negated literal) for now — \
         non-literal defaults arrive with the stage-4 nodes that need them \
         (their catalog rendering is undesigned)",
    )
}

fn lit_text(lit: &Lit) -> String {
    match lit {
        Lit::Str(s) => format!("{:?}", s.value()),
        other => quote!(#other).to_string(),
    }
}

/// First non-empty line of the doc comment, trimmed.
fn doc_first_line(attrs: &[Attribute]) -> String {
    doc_lines(attrs)
        .into_iter()
        .find(|line| !line.is_empty())
        .unwrap_or_default()
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
            for line in text.value().lines() {
                lines.push(line.trim().to_owned());
            }
        }
    }
    lines
}

struct NodeArgs {
    category: Option<LitStr>,
    tier: Option<LitStr>,
    version: Option<LitInt>,
    name: Option<LitStr>,
    effectful: bool,
    uses_tolerance: bool,
}

fn parse_node_args(args: &TokenStream2) -> syn::Result<NodeArgs> {
    let mut parsed = NodeArgs {
        category: None,
        tier: None,
        version: None,
        name: None,
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
        } else if meta.path.is_ident("effectful") {
            parsed.effectful = true;
            Ok(())
        } else if meta.path.is_ident("uses_tolerance") {
            parsed.uses_tolerance = true;
            Ok(())
        } else {
            Err(meta.error(
                "unknown #[node(...)] key — expected category, tier, version, name, \
                 effectful, or uses_tolerance",
            ))
        }
    });
    syn::parse::Parser::parse2(parser, args.clone())?;
    Ok(parsed)
}

/// Title/description from the doc comment's first line: `Title — description.`
fn parse_title_line(function: &ItemFn) -> syn::Result<(String, String)> {
    let span = function.sig.span();
    // Loudly refuse anything else — this line IS the catalog (docs/08:
    // "docstring first line = node title").
    let first_line = doc_first_line(&function.attrs);
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

    let (title, description) = parse_title_line(function)?;

    // Exactly one typed argument: the input struct (struct-in ABI).
    let inputs: Vec<&FnArg> = function.sig.inputs.iter().collect();
    let input_ty: &Type = match inputs.as_slice() {
        [FnArg::Typed(pat)] => &pat.ty,
        _ => {
            return Err(syn::Error::new(
                span,
                "a node takes exactly one argument: its input struct \
                 (struct-in/struct-out ABI, DECISIONS.md) — e.g. `fn add(input: AddIn) -> f64`",
            ));
        }
    };
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
    let invoke_shim = invoke_shim(&invoke_ident, fn_ident, input_ty);

    Ok(quote! {
        #function

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
            inputs: <#input_ty as cicada_core::spec::Ports>::PORTS,
            outputs: <#output_ty as cicada_core::spec::AsOutputs>::OUTPUTS,
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

/// The type-erased invocation shim (stage 3): marshal in, call the real fn,
/// marshal out. Panics are NOT caught here — the scheduler `catch_unwind`s
/// and turns them into red nodes (docs/12).
fn invoke_shim(
    invoke_ident: &proc_macro2::Ident,
    fn_ident: &proc_macro2::Ident,
    input_ty: &Type,
) -> TokenStream2 {
    quote! {
        #[doc(hidden)]
        #[allow(missing_docs)]
        fn #invoke_ident(
            inputs: &[Option<::std::sync::Arc<cicada_core::value::HashedValue>>],
        ) -> Result<
            Vec<::std::sync::Arc<cicada_core::value::HashedValue>>,
            cicada_core::marshal::InvokeError,
        > {
            let input = <#input_ty as cicada_core::marshal::FromValues>::from_values(inputs)?;
            cicada_core::marshal::IntoValues::into_values(#fn_ident(input))
        }
    }
}
