//! `cicada mcp` (v0.1; docs/11 §Read tools, DECISIONS.md documentation-
//! pipeline row revised 2026-08-19): a Model Context Protocol server over
//! stdio that serves the node catalog and the checker to agents — from the
//! SAME data as `/api/catalog` (the server's catalog renderer) and through
//! the SAME checker as `cicada run` and the live session
//! (`cicada_server::compile`). Never a second copy of either.
//!
//! Tools: `catalog_search`, `node_doc`, `list_categories`, `check`. With
//! `--project <dir-or-pipeline>` the catalog includes the project's script
//! nodes (the server's own discovery, re-run whenever `scripts/*.py`
//! change on disk) and `check` resolves relative paths against the project
//! directory.
//!
//! `check` is the checker AND the dry lowering: after `compile::check_source`
//! it runs the session's `lower_partial` (closures are built, nothing is
//! solved) so the refusals only lowering sees — an integer literal at or
//! beyond 2^53, a literal that refuses construction — reach the agent as
//! `excluded` bindings with the very text the canvas would show, instead
//! of an `ok: true` that `cicada run` then contradicts.
//!
//! Transport discipline: stdout carries JSON-RPC frames and nothing else;
//! every note goes to stderr. The server is read-only by construction —
//! edits reach a pipeline through the running app's atomic
//! `POST /api/edit/apply_text` (docs/13), never through this process, and
//! no node — script or stdlib — ever runs here.
//!
//! Built on `rmcp` (the official MCP Rust SDK, Apache-2.0) without its
//! macros: the tools are plain functions routed by hand so the workspace
//! lints see every line.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use cicada_core::catalog::category_rank;
use cicada_core::config::ProjectConfig;
use cicada_core::spec::NodeSpec;
use cicada_lang::check::Resolution;
use cicada_lang::diag::{Diagnostic, DiagnosticKind};
use cicada_lang::document::Document;
use cicada_server::compile;
use cicada_server::lower::{Exclusion, lower_partial};
use cicada_server::scripts::{ScriptCancel, ScriptNode};
use rmcp::handler::server::common::schema_for_input;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ResultType, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt as _};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Arguments of `cicada mcp`.
pub struct McpArgs {
    /// A project directory or a `.cic` pipeline (its directory becomes the
    /// project). `None` = the stdlib catalog alone; `check` paths resolve
    /// against the working directory.
    pub project: Option<PathBuf>,
}

/// The `initialize` instructions — what a model reads once per session
/// before it sees the tool list.
const INSTRUCTIONS: &str = "\
Cicada is a code-first parametric design tool: a pipeline is a `.cic` text file \
where every line `name = node(port=value, …)` is one node of a dataflow graph \
(first line `# cicada 1`; kwargs only, no positional arguments, no nested calls — \
name the intermediate; forward references are legal; `a, b = node(…)` unpacks a \
multi-output node; `each(list)` maps a node over a list; `x.port` selects an output). \
This server is READ-ONLY: use `catalog_search` to find nodes by name, title, \
Grasshopper component name or description; `list_categories` to see the catalog's \
shape; `node_doc` for one node's full contract (ports, defaults, when it goes red, a \
runnable example) before wiring it; and `check` to typecheck and dry-lower pipeline \
text or a file in milliseconds — the inner loop: iterate on `check` until `ok` is true \
(no diagnostics, no binding excluded by a lowering refusal), then apply the text \
through the running app's atomic `POST /api/edit/apply_text` route (docs/13) or \
`cicada run <file>` headless. Diagnostics carry a `fix` with a `replacement` when one \
is machine-applicable.";

/// Run the MCP server over stdin/stdout until the client closes the pipe.
///
/// # Errors
///
/// A bad `--project` path, script discovery failing at startup (the
/// catalog must be complete before the first tool call), or a transport
/// failure during the handshake.
pub fn mcp_command(args: &McpArgs) -> anyhow::Result<()> {
    let project = match &args.project {
        Some(path) => Some(Project::open(path)?),
        None => None,
    };
    let server = McpServer::new(project)?;
    // The catalog is discovered eagerly so a broken `scripts/` directory
    // refuses at startup, on stderr, instead of inside the first tool call.
    let specs = server
        .specs()
        .map_err(|refusal| anyhow::anyhow!("{}", refusal.message()))?;
    eprintln!(
        "cicada mcp — {} node(s) in the catalog{}; tools: catalog_search, node_doc, \
         list_categories, check; stdio JSON-RPC (Ctrl-C or EOF stops it)",
        specs.len(),
        match &server.project {
            Some(project) => format!(", project {}", project.dir.display()),
            None => " (stdlib only — pass --project <dir-or-pipeline> for script nodes)".to_owned(),
        }
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the tokio runtime")?;
    let outcome = runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .context("MCP handshake over stdio")?;
        service.waiting().await.context("MCP service task")?;
        Ok::<(), anyhow::Error>(())
    });
    // The stdin reader is a blocking thread; never wait on it past the
    // service's end (a dropped runtime would block forever on it).
    runtime.shutdown_background();
    outcome
}

// ------------------------------------------------------------ project --

/// A `scripts/*.py` snapshot — sorted paths with their bytes — compared
/// whole to decide when discovery re-runs. Exact by construction (an mtime
/// would miss a same-size edit inside one filesystem tick); the scripts of
/// a project are kilobytes, reading them per call is nothing next to the
/// Python describe they gate.
type Fingerprint = Vec<(PathBuf, Vec<u8>)>;

/// The catalog every tool reads and `check` lowers against: the specs
/// (stdlib first, then the project's script nodes by name — `compile`'s
/// order) and the discovered script nodes themselves. The script nodes
/// are kept, not dropped, because the dry lowering inside `check` needs
/// their run functions to EXIST (`lower_call` refuses a script spec it has
/// no node for); they are never called — this process solves nothing.
struct ProjectCatalog {
    specs: Vec<&'static NodeSpec>,
    scripts: HashMap<String, ScriptNode>,
}

impl ProjectCatalog {
    /// The stdlib alone — what a server without `--project` reads.
    fn stdlib() -> Self {
        Self {
            specs: cicada_stdlib::registry().to_vec(),
            scripts: HashMap::new(),
        }
    }
}

/// The project whose script nodes join the catalog.
struct Project {
    /// The project directory — `scripts/` lives here and `check` paths
    /// resolve against it.
    dir: PathBuf,
    /// The cancel bridge the discovered run functions take their switch
    /// from (never killed here: this process never solves).
    cancel: Arc<ScriptCancel>,
    /// The last discovered catalog and the fingerprint it matched.
    cache: Mutex<Option<(Fingerprint, Arc<ProjectCatalog>)>>,
}

impl Project {
    /// Resolve `--project`: a directory, or a `.cic` file whose directory
    /// is the project.
    fn open(path: &Path) -> anyhow::Result<Self> {
        let canonical =
            std::fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))?;
        let (dir, _pipeline) = crate::serve::split_target(&crate::serve::plain(&canonical))?;
        Ok(Self {
            dir,
            cancel: ScriptCancel::new(),
            cache: Mutex::new(None),
        })
    }

    fn fingerprint(&self) -> Result<Fingerprint, Refusal> {
        let scripts = self.dir.join("scripts");
        if !scripts.is_dir() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&scripts).map_err(|error| Refusal::ScriptDiscovery {
            message: format!("reading {}: {error}", scripts.display()),
        })?;
        let mut files: Fingerprint = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| Refusal::ScriptDiscovery {
                message: format!("reading {}: {error}", scripts.display()),
            })?;
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "py") {
                continue;
            }
            let bytes = std::fs::read(&path).map_err(|error| Refusal::ScriptDiscovery {
                message: format!("reading {}: {error}", path.display()),
            })?;
            files.push((path, bytes));
        }
        files.sort();
        Ok(files)
    }

    /// The project catalog: stdlib + this project's script nodes, re-run
    /// through the server's discovery whenever `scripts/` changed.
    fn catalog(&self) -> Result<Arc<ProjectCatalog>, Refusal> {
        let fingerprint = self.fingerprint()?;
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached, catalog)) = cache.as_ref()
            && *cached == fingerprint
        {
            return Ok(Arc::clone(catalog));
        }
        // A changed `scripts/` replaces the whole catalog; the previous
        // script nodes (and the worker pool behind their run functions)
        // drop with it.
        let (specs, scripts) =
            compile::catalog_specs_in(&self.dir, &self.cancel).map_err(|error| {
                Refusal::ScriptDiscovery {
                    message: error.to_string(),
                }
            })?;
        let catalog = Arc::new(ProjectCatalog { specs, scripts });
        *cache = Some((fingerprint, Arc::clone(&catalog)));
        Ok(catalog)
    }
}

// ------------------------------------------------------------- server --

/// The MCP service: the tool router plus the project it reads from.
struct McpServer {
    project: Option<Project>,
    /// The catalog of a server without `--project`.
    stdlib: Arc<ProjectCatalog>,
    router: ToolRouter<Self>,
}

impl McpServer {
    fn new(project: Option<Project>) -> anyhow::Result<Self> {
        Ok(Self {
            project,
            stdlib: Arc::new(ProjectCatalog::stdlib()),
            router: build_router()?,
        })
    }

    /// The catalog every tool reads: the project's (stdlib + scripts) or
    /// the stdlib alone.
    fn catalog(&self) -> Result<Arc<ProjectCatalog>, Refusal> {
        match &self.project {
            Some(project) => project.catalog(),
            None => Ok(Arc::clone(&self.stdlib)),
        }
    }

    /// The specs of [`Self::catalog`].
    fn specs(&self) -> Result<Vec<&'static NodeSpec>, Refusal> {
        self.catalog().map(|catalog| catalog.specs.clone())
    }

    /// Where `check {path}` resolves a relative path.
    fn base_dir(&self) -> Option<&Path> {
        self.project.as_ref().map(|project| project.dir.as_path())
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("cicada", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            result_type: Some(ResultType::COMPLETE),
            tools: self.router.list_all(),
            meta: None,
            next_cursor: None,
            ttl_ms: None,
            cache_scope: None,
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.router.get(name).cloned()
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        self.router
            .call(ToolCallContext::new(self, request, context))
    }
}

/// One tool's wiring: the MCP `Tool` (name, description, input schema from
/// the argument type, output schema from the result type, read-only
/// annotations) and its handler function.
fn tool<A: JsonSchema + 'static, O: JsonSchema + 'static>(
    name: &'static str,
    description: &'static str,
) -> anyhow::Result<Tool> {
    let input = schema_for_input::<Parameters<A>>()
        .map_err(|error| anyhow::anyhow!("tool `{name}` input schema: {error}"))?;
    Ok(Tool::new(name, description, input)
        .with_output_schema::<O>()
        .with_annotations(
            ToolAnnotations::new()
                .read_only(true)
                .idempotent(true)
                .open_world(false),
        ))
}

fn build_router() -> anyhow::Result<ToolRouter<McpServer>> {
    Ok(ToolRouter::new()
        .with_route((
            tool::<SearchArgs, SearchResult>(
                "catalog_search",
                "Find Cicada nodes. Use this FIRST whenever you need a node and do not know \
                 its exact name: it ranks the catalog (stdlib plus the project's script \
                 nodes) by how well each word of `query` matches a node's dialect name, \
                 title, Grasshopper component name (`gh` — Grasshopper users search by \
                 the component they know, e.g. `Number Slider`, `Move`), port names and \
                 description. Returns one line per node — name, title, gh, category, and \
                 the signature `name(port: Type = default, …) → Type` — enough to write \
                 the binding; call `node_doc` for port docs and the red-when contract. An \
                 empty `query` lists the catalog (optionally one `category`).",
            )?,
            catalog_search,
        ))
        .with_route((
            tool::<NodeDocArgs, NodeDoc>(
                "node_doc",
                "The full specification of one node by its dialect name — the same object \
                 `/api/catalog` serves: `signature`; `title`; `description`; `category`; \
                 `tier` (S = spike set, 1 = v0.1, 2 = v0.2); `version` (semantic node \
                 version); `pure` and `effectful` (effectful nodes — exporters — never run \
                 unless a human or `cicada run --node` names them); `volatile` (never memoized — recomputed every generation); `uses_tolerance`; \
                 `panics` (the runtime contract: the conditions under which the node goes \
                 red); `gh` (the Grasshopper component it replaces, null for Cicada-only \
                 nodes); `examples` (runnable `.cic` snippets CI solves); `inputs` and \
                 `outputs` — every port with `name`, `type` (catalog notation: `Number`, \
                 `[Point]`, `Mesh?`; `T` = any transformable kind, `E` = any element kind) \
                 and its parts `base` / `list_depth` / `optional` (`optional` = the type \
                 carries `?`, a value that may be absent — NOT whether the kwarg may be \
                 omitted), `default` (present = the kwarg may be omitted; absent = you \
                 must pass it), `doc`, and `dimension` (`length` ports rescale with units, \
                 `angle` ports are radians). A single output is the port `out`. Use it \
                 before wiring a node you have not used in this session. Unknown names \
                 return an error with a did-you-mean.",
            )?,
            node_doc,
        ))
        .with_route((
            tool::<NoArgs, CategoriesResult>(
                "list_categories",
                "The catalog's categories (the app's ribbon tabs, in ribbon order) with \
                 the number of nodes in each — the shape of what exists. Use it to scope \
                 a `catalog_search` by `category` or to learn what the catalog covers \
                 before planning a pipeline.",
            )?,
            list_categories,
        ))
        .with_route((
            tool::<CheckArgs, CheckResult>(
                "check",
                "Parse, typecheck and dry-lower a Cicada pipeline WITHOUT solving any \
                 geometry — milliseconds, the inner loop. Pass exactly one of `text` (the \
                 whole pipeline source, `# cicada 1` header included) or `path` (a `.cic` \
                 file; relative paths resolve against the project directory, and the \
                 file's own `scripts/` directory joins its catalog). Returns `ok`, the \
                 doc-11 diagnostics and `excluded`. Each diagnostic has `kind` \
                 (unknown_node, unknown_name, missing_kwarg, unknown_kwarg, type_mismatch, \
                 needs_lift, needs_adapter, zip_length_mismatch, unpack_arity, \
                 unknown_port, rebinding, cycle, parse_error, …), the red `node` (binding \
                 name), a `span` (1-based line, byte columns), a domain-quality `message`, \
                 `expected`/`actual` types in catalog notation where relevant, and a `fix` \
                 with a `label` and — when the fix is a pure splice of the span — a \
                 `replacement`. `excluded` lists every binding the app would NOT solve, \
                 with `status` `red` (its own diagnostics, `#off`, or a lowering refusal \
                 such as an integer literal at or beyond 2^53 — refusals the checker \
                 alone never sees) or `blocked` (fed by an excluded binding) and the \
                 `reason` the canvas shows. `ok` is true when there are no diagnostics \
                 and no lowering refusal (`#off` and its blocked cone are a deliberate \
                 state, not an error). Iterate until `ok` is true before applying text to \
                 a project; a pipeline with diagnostics still opens (red nodes are a valid \
                 state) but its excluded bindings will not solve.",
            )?,
            check,
        )))
}

// ------------------------------------------------------------ refusals --

/// Why a tool call refused — returned as a tool-level error result
/// (`isError: true`, this object as the structured content) so the model
/// reads the reason; never an opaque JSON-RPC protocol error.
#[derive(Debug, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
enum Refusal {
    /// `node_doc` for a name the catalog lacks.
    UnknownNode {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        did_you_mean: Option<String>,
        message: String,
    },
    /// `catalog_search` with a category that is not one of the catalog's.
    UnknownCategory {
        category: String,
        categories: Vec<String>,
        message: String,
    },
    /// `check` with neither `text` nor `path`, or both.
    CheckSource { message: String },
    /// `check {path}` could not be read.
    UnreadablePath { path: String, message: String },
    /// The project's `scripts/` directory failed discovery.
    ScriptDiscovery { message: String },
    /// The dry lowering could not assemble the checked pipeline into a
    /// graph — an engine bug, never a pipeline problem (per-binding
    /// refusals are `excluded` entries, not this); the message is the
    /// evidence to report.
    GraphAssembly { message: String },
}

impl Refusal {
    fn message(&self) -> &str {
        match self {
            Self::UnknownNode { message, .. }
            | Self::UnknownCategory { message, .. }
            | Self::CheckSource { message }
            | Self::UnreadablePath { message, .. }
            | Self::ScriptDiscovery { message }
            | Self::GraphAssembly { message } => message,
        }
    }

    fn into_result(self) -> CallToolResult {
        // A tagged enum of strings serializes; Null would still be a loud
        // `isError` result.
        CallToolResult::structured_error(
            serde_json::to_value(&self).unwrap_or(serde_json::Value::Null),
        )
    }
}

/// Structured success: the value as `structuredContent` plus its JSON text
/// as the `content` block (clients that ignore structured output still
/// see it).
fn structured<T: Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| {
            ErrorData::internal_error(format!("serializing the result: {error}"), None)
        })
}

fn outcome<T: Serialize>(result: Result<T, Refusal>) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(value) => structured(&value),
        Err(refusal) => Ok(refusal.into_result()),
    }
}

// ------------------------------------------------------ catalog_search --

/// Arguments of `catalog_search`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    /// Words to match against node names, titles, Grasshopper component
    /// names, port names and descriptions (case-insensitive; every word
    /// scores independently, the best-matching nodes come first). Empty =
    /// list the catalog in ribbon order.
    #[serde(default)]
    query: String,
    /// Restrict to one category — a name from `list_categories`.
    #[serde(default)]
    category: Option<String>,
    /// Maximum number of nodes to return (default 20).
    #[serde(default)]
    limit: Option<u32>,
}

/// One catalog hit.
#[derive(Debug, Serialize, JsonSchema)]
struct SearchHit {
    /// Dialect name — what you write before `(` in a binding.
    name: String,
    /// Human title.
    title: String,
    /// The Grasshopper component this node replaces; null for Cicada-only nodes.
    gh: Option<String>,
    /// Catalog category (ribbon tab).
    category: String,
    /// `name(port: Type = default, …) → Type` — the signature to write a binding from.
    signature: String,
    /// One-line description.
    description: String,
    /// Relevance (higher = better); 0 when `query` is empty.
    score: u32,
}

/// Result of `catalog_search`.
#[derive(Debug, Serialize, JsonSchema)]
struct SearchResult {
    /// The query as matched (lowercased words).
    query: String,
    /// How many nodes matched before `limit`.
    total_matches: usize,
    /// The best `limit` hits, highest score first.
    nodes: Vec<SearchHit>,
}

const DEFAULT_LIMIT: usize = 20;

fn catalog_search(
    server: &McpServer,
    Parameters(args): Parameters<SearchArgs>,
) -> Result<CallToolResult, ErrorData> {
    outcome(search(server, &args))
}

fn search(server: &McpServer, args: &SearchArgs) -> Result<SearchResult, Refusal> {
    let specs = server.specs()?;
    if let Some(category) = &args.category
        && !specs.iter().any(|spec| spec.category == category)
    {
        let categories = categories_of(&specs)
            .into_iter()
            .map(|(name, _)| name.to_owned())
            .collect::<Vec<_>>();
        return Err(Refusal::UnknownCategory {
            message: format!(
                "no category named `{category}` — the catalog's categories are: {}",
                categories.join(", ")
            ),
            category: category.clone(),
            categories,
        });
    }
    let words: Vec<String> = args
        .query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let phrase = words.join(" ");
    let mut hits: Vec<(u32, usize, &NodeSpec)> = specs
        .iter()
        .enumerate()
        .filter(|(_, spec)| {
            args.category
                .as_deref()
                .is_none_or(|category| spec.category == category)
        })
        .filter_map(|(index, spec)| {
            let score = score(spec, &words, &phrase);
            (words.is_empty() || score > 0).then_some((score, index, *spec))
        })
        .collect();
    // Best first; ties keep catalog order (the registry's deterministic
    // category-then-name order).
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let total_matches = hits.len();
    let limit = args.limit.map_or(DEFAULT_LIMIT, |limit| {
        usize::try_from(limit).unwrap_or(usize::MAX)
    });
    let nodes = hits
        .into_iter()
        .take(limit)
        .map(|(score, _, spec)| SearchHit {
            name: spec.name.to_owned(),
            title: spec.title.to_owned(),
            gh: spec.gh.map(str::to_owned),
            category: spec.category.to_owned(),
            signature: spec.signature(),
            description: spec.description.to_owned(),
            score,
        })
        .collect();
    Ok(SearchResult {
        query: phrase,
        total_matches,
        nodes,
    })
}

/// Relevance of `spec` for the query words: the best field match per word,
/// summed, plus a bonus when the whole phrase IS a name, title or GH name.
fn score(spec: &NodeSpec, words: &[String], phrase: &str) -> u32 {
    if words.is_empty() {
        return 0;
    }
    let name = spec.name.to_lowercase();
    let title = spec.title.to_lowercase();
    let gh = spec.gh.map(str::to_lowercase);
    let description = spec.description.to_lowercase();
    let category = spec.category.to_lowercase();
    let ports: Vec<String> = spec
        .inputs
        .iter()
        .chain(spec.outputs)
        .map(|port| port.name.to_lowercase())
        .collect();

    let mut total = 0_u32;
    for word in words {
        let mut best = 0_u32;
        best = best.max(match_rank(&name, word, 100, 60, 40));
        best = best.max(match_rank(&title, word, 80, 50, 30));
        if let Some(gh) = &gh {
            best = best.max(match_rank(gh, word, 80, 50, 30));
        }
        if ports.iter().any(|port| port == word) {
            best = best.max(20);
        }
        // Prose matches at word starts only: "move" must not surface every
        // node whose description says "removed".
        if word_rank(&description, word) {
            best = best.max(15);
        }
        if word_rank(&category, word) {
            best = best.max(10);
        }
        total += best;
    }
    if words.len() > 1 && (name == phrase || title == phrase || gh.as_deref() == Some(phrase)) {
        total += 100;
    }
    total
}

/// Whether some word of `prose` (split at anything but letters, digits and
/// `_`) starts with `word`.
fn word_rank(prose: &str, word: &str) -> bool {
    prose
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|part| part.starts_with(word))
}

/// `exact` when the field equals the word, `prefix` when the field or one
/// of its `_`/space-separated words starts with it, `contains` otherwise
/// (0 when absent).
fn match_rank(field: &str, word: &str, exact: u32, prefix: u32, contains: u32) -> u32 {
    if field == word {
        exact
    } else if field.split(['_', ' ']).any(|part| part.starts_with(word)) {
        prefix
    } else if field.contains(word) {
        contains
    } else {
        0
    }
}

// ------------------------------------------------------------ node_doc --

/// Arguments of `node_doc`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NodeDocArgs {
    /// The node's dialect name, e.g. `slider`, `mesh_difference`.
    name: String,
}

/// The SHAPE of a `node_doc` result — `/api/catalog`'s node object (the
/// server's renderer, `catalog::node_value`, produces the value; this type
/// only documents it for the tool's `outputSchema`) plus `signature` and
/// `effectful`. The unit test `node_doc_schema_matches_every_catalog_entry`
/// holds the two together: a field added to the catalog renderer must be
/// described here, or the test says so.
#[derive(JsonSchema)]
#[allow(dead_code)]
// schema-only: never constructed, the value comes from the renderer
// The flags mirror `/api/catalog`'s JSON object field for field; they are
// booleans on the wire, so they are booleans here (a state enum would
// misdescribe the contract this type exists to describe).
#[allow(clippy::struct_excessive_bools)]
struct NodeDoc {
    /// Dialect name — what you write before `(` in a binding.
    name: String,
    /// Human title.
    title: String,
    /// What the node does (one sentence, lowercase, ends with a period).
    description: String,
    /// Catalog category (ribbon tab).
    category: String,
    /// `S` = spike set, `1` = v0.1, `2` = v0.2.
    tier: String,
    /// Semantic node version (part of the cache key).
    version: u32,
    /// Pure nodes are memoized and run whenever their inputs change.
    pure: bool,
    /// `!pure`: exporters and other side-effecting nodes — never run unless
    /// a human or `cicada run --node` names them.
    effectful: bool,
    /// Volatile nodes (item 3b) are never memoized: the node recomputes
    /// every generation and everything downstream sees its fresh value
    /// (the `clock` time param is the intended user; no stdlib node sets it
    /// yet).
    volatile: bool,
    /// Whether the node's result depends on the project tolerance.
    uses_tolerance: bool,
    /// The runtime contract: the conditions under which the node goes red
    /// (absent when the node cannot fail).
    #[serde(skip_serializing_if = "Option::is_none")]
    panics: Option<String>,
    /// The Grasshopper component this node replaces; always present — null
    /// for Cicada-only nodes.
    #[schemars(required)]
    gh: Option<String>,
    /// Runnable `.cic` snippets (no `# cicada 1` header) CI solves; empty
    /// for script nodes.
    examples: Vec<String>,
    /// `name(port: Type = default, …) → Type` — the signature to write a binding from.
    signature: String,
    /// Input ports in signature order.
    inputs: Vec<PortDoc>,
    /// Output ports; a single output is the port `out`.
    outputs: Vec<PortDoc>,
}

/// One port of a [`NodeDoc`] — the `/api/catalog` port object.
#[derive(JsonSchema)]
#[allow(dead_code)] // schema-only, see NodeDoc
struct PortDoc {
    /// Port name — the kwarg (inputs) or the `.port` selector (outputs).
    name: String,
    /// Catalog notation: `Number`, `[Point]`, `Mesh?`; `T` = any
    /// transformable kind, `E` = any element kind, `Any` = a display sink.
    #[serde(rename = "type")]
    ty: String,
    /// The base kind of `type` without list brackets or `?`.
    base: String,
    /// How many `[…]` list levels `type` carries.
    list_depth: u8,
    /// True when `type` carries `?` — a value that may be absent. NOT
    /// whether the kwarg may be omitted; that is `default`.
    optional: bool,
    /// The default as dialect text; present = the kwarg may be omitted,
    /// absent = you must pass it.
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    /// The port's documentation paragraph (absent when the port has none).
    #[serde(skip_serializing_if = "Option::is_none")]
    doc: Option<String>,
    /// `length` ports rescale with the project unit; `angle` ports are
    /// radians (absent for dimensionless ports).
    #[serde(skip_serializing_if = "Option::is_none")]
    dimension: Option<String>,
}

fn node_doc(
    server: &McpServer,
    Parameters(args): Parameters<NodeDocArgs>,
) -> Result<CallToolResult, ErrorData> {
    outcome(doc_of(server, &args.name))
}

fn doc_of(
    server: &McpServer,
    name: &str,
) -> Result<serde_json::Map<String, serde_json::Value>, Refusal> {
    let specs = server.specs()?;
    let Some(spec) = specs.iter().find(|spec| spec.name == name) else {
        let did_you_mean = did_you_mean(name, &specs);
        return Err(Refusal::UnknownNode {
            message: match &did_you_mean {
                Some(suggestion) => {
                    format!("no node named `{name}` in the catalog — did you mean `{suggestion}`?")
                }
                None => format!(
                    "no node named `{name}` in the catalog — search for it with catalog_search"
                ),
            },
            name: name.to_owned(),
            did_you_mean,
        });
    };
    // The catalog entry itself — the object `/api/catalog` serves — plus
    // the derived conveniences agents ask for first.
    let mut entry = match cicada_server::catalog::node_value(spec) {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    entry.insert("signature".to_owned(), spec.signature().into());
    entry.insert("effectful".to_owned(), (!spec.pure).into());
    Ok(entry)
}

/// The checker's own did-you-mean for an unknown node: check a one-binding
/// probe pipeline and read the `unknown_node` diagnostic's fix — one
/// suggestion algorithm in the repo, not two.
fn did_you_mean(name: &str, specs: &[&'static NodeSpec]) -> Option<String> {
    let probe = format!("# cicada 1\nprobe = {name}()\n");
    let (_, resolution) = compile::check_source(&probe, specs);
    resolution
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.kind == DiagnosticKind::UnknownNode)
        .and_then(|diagnostic| diagnostic.fix.as_ref())
        .and_then(|fix| fix.replacement.clone())
}

// ----------------------------------------------------- list_categories --

/// `list_categories` takes no arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// One category.
#[derive(Debug, Serialize, JsonSchema)]
struct CategoryCount {
    /// The category name — pass it to `catalog_search` as `category`.
    name: String,
    /// Nodes in it.
    count: usize,
}

/// Result of `list_categories`.
#[derive(Debug, Serialize, JsonSchema)]
struct CategoriesResult {
    /// Categories in ribbon order.
    categories: Vec<CategoryCount>,
    /// Nodes in the whole catalog.
    total: usize,
}

fn list_categories(
    server: &McpServer,
    Parameters(NoArgs {}): Parameters<NoArgs>,
) -> Result<CallToolResult, ErrorData> {
    outcome(server.specs().map(|specs| {
        CategoriesResult {
            categories: categories_of(&specs)
                .into_iter()
                .map(|(name, count)| CategoryCount {
                    name: name.to_owned(),
                    count,
                })
                .collect(),
            total: specs.len(),
        }
    }))
}

/// Categories with counts, in the catalog's order (docs/08 ribbon order,
/// unknown categories after, alphabetically).
fn categories_of(specs: &[&'static NodeSpec]) -> Vec<(&'static str, usize)> {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for spec in specs {
        *counts.entry(spec.category).or_default() += 1;
    }
    let mut categories: Vec<(&'static str, usize)> = counts.into_iter().collect();
    categories.sort_by_key(|(name, _)| category_rank(name));
    categories
}

// --------------------------------------------------------------- check --

/// Arguments of `check` — exactly one of the two.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CheckArgs {
    /// The whole pipeline text, `# cicada 1` header included.
    #[serde(default)]
    text: Option<String>,
    /// A `.cic` file; relative paths resolve against the project directory
    /// (`--project`), else the server's working directory.
    #[serde(default)]
    path: Option<String>,
}

/// What `check` checked.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CheckedSource {
    /// Inline text, checked against the project catalog.
    Text,
    /// A file, checked against the catalog of its own directory.
    Path {
        /// The resolved absolute path.
        path: String,
    },
}

/// One binding the app would not solve (the canvas's `excluded` view,
/// status word and reason from the same renderer).
#[derive(Debug, Serialize, JsonSchema)]
struct ExcludedBinding {
    /// The binding name.
    node: String,
    /// `red` — its own diagnostics, `#off`, or a lowering refusal — or
    /// `blocked` — fed by an excluded binding.
    status: String,
    /// The reason as the canvas shows it on the node.
    reason: String,
}

/// Result of `check`.
#[derive(Debug, Serialize, JsonSchema)]
struct CheckResult {
    /// What was checked.
    source: CheckedSource,
    /// True when there are no diagnostics and no binding is excluded by a
    /// lowering refusal (`#off` and its blocked cone are a deliberate
    /// state, not an error).
    ok: bool,
    /// Number of diagnostics.
    diagnostic_count: usize,
    /// The doc-11 diagnostics: `kind`, `node`, `span {line, col_start,
    /// col_end}`, `message`, `expected`, `actual`, `fix {label, replacement}`.
    diagnostics: Vec<serde_json::Value>,
    /// Every binding the app would not solve, by name: red by its own
    /// diagnostics / `#off` / a lowering refusal the checker alone never
    /// sees (an integer literal at or beyond 2^53, a literal that refuses
    /// construction), or blocked by an excluded upstream.
    excluded: Vec<ExcludedBinding>,
    /// Binding names the pipeline introduces, in file order.
    bindings: Vec<String>,
}

fn check(
    server: &McpServer,
    Parameters(args): Parameters<CheckArgs>,
) -> Result<CallToolResult, ErrorData> {
    outcome(check_pipeline(server, args))
}

/// A parsed + checked pipeline with the catalog it was checked against —
/// what the dry lowering consumes.
struct Checked {
    document: Document,
    resolution: Resolution,
    catalog: Arc<ProjectCatalog>,
}

impl Checked {
    /// Check `text` against `catalog` — THE checker.
    fn against(text: &str, catalog: Arc<ProjectCatalog>) -> Self {
        let (document, resolution) = compile::check_source(text, &catalog.specs);
        Self {
            document,
            resolution,
            catalog,
        }
    }

    /// The dry lowering: the session's `lower_partial` over the checked
    /// document (closures are built, nothing runs, the graph is dropped),
    /// reduced to the bindings it excluded and why — the canvas's view.
    fn excluded(&self) -> Result<Vec<(String, Exclusion)>, Refusal> {
        let lowered = lower_partial(
            &self.document,
            &self.resolution,
            &self.catalog.specs,
            &ProjectConfig::default(),
            &self.catalog.scripts,
        )
        .map_err(|error| Refusal::GraphAssembly {
            message: error.to_string(),
        })?;
        Ok(lowered.excluded.into_iter().collect())
    }
}

fn check_pipeline(server: &McpServer, args: CheckArgs) -> Result<CheckResult, Refusal> {
    let (checked, source) = match (args.text, args.path) {
        (Some(text), None) => (
            Checked::against(&text, server.catalog()?),
            CheckedSource::Text,
        ),
        (None, Some(path)) => {
            let relative = PathBuf::from(&path);
            let joined = match server.base_dir() {
                Some(base) if relative.is_relative() => base.join(&relative),
                _ => relative,
            };
            let pipeline = std::fs::canonicalize(&joined)
                .map(|canonical| crate::serve::plain(&canonical))
                .map_err(|error| Refusal::UnreadablePath {
                    path: joined.display().to_string(),
                    message: format!("resolving {}: {error}", joined.display()),
                })?;
            let text =
                std::fs::read_to_string(&pipeline).map_err(|error| Refusal::UnreadablePath {
                    path: pipeline.display().to_string(),
                    message: format!("reading {}: {error}", pipeline.display()),
                })?;
            // The file's own `scripts/` joins its catalog, exactly as in
            // `cicada run`: a file inside the project reads the cached
            // project catalog (kept current by the fingerprint); a file
            // elsewhere goes through the server's `load` (discovery next
            // to it; the headless run never cancels either).
            let in_project = server
                .base_dir()
                .is_some_and(|base| pipeline.parent() == Some(base));
            let checked = if in_project {
                Checked::against(&text, server.catalog()?)
            } else {
                let loaded =
                    compile::load(&pipeline, &text, &ScriptCancel::new()).map_err(|error| {
                        Refusal::ScriptDiscovery {
                            message: error.to_string(),
                        }
                    })?;
                Checked {
                    document: loaded.document,
                    resolution: loaded.resolution,
                    catalog: Arc::new(ProjectCatalog {
                        specs: loaded.specs,
                        scripts: loaded.scripts,
                    }),
                }
            };
            (
                checked,
                CheckedSource::Path {
                    path: pipeline.display().to_string(),
                },
            )
        }
        (None, None) => {
            return Err(Refusal::CheckSource {
                message: "pass `text` (the pipeline source) or `path` (a .cic file)".to_owned(),
            });
        }
        (Some(_), Some(_)) => {
            return Err(Refusal::CheckSource {
                message: "pass either `text` or `path`, not both".to_owned(),
            });
        }
    };
    let excluded = checked.excluded()?;
    let diagnostics = &checked.resolution.diagnostics;
    let bindings = checked
        .document
        .statements()
        .flat_map(|(_, statement, _)| statement.targets.iter().map(|target| target.name.clone()))
        .collect();
    Ok(CheckResult {
        source,
        // `#off` and the cone it blocks are a deliberate state; a lowering
        // refusal is a problem the checker could not see.
        ok: diagnostics.is_empty()
            && !excluded
                .iter()
                .any(|(_, exclusion)| matches!(exclusion, Exclusion::Lowering(_))),
        diagnostic_count: diagnostics.len(),
        diagnostics: diagnostics.iter().map(diagnostic_json).collect(),
        excluded: excluded
            .iter()
            .map(|(node, exclusion)| ExcludedBinding {
                node: node.clone(),
                status: exclusion.status().to_owned(),
                reason: exclusion.reason(),
            })
            .collect(),
        bindings,
    })
}

/// The doc-11 JSON of one diagnostic (its `Serialize` impl IS the shape).
fn diagnostic_json(diagnostic: &Diagnostic) -> serde_json::Value {
    serde_json::to_value(diagnostic).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdlib_server() -> McpServer {
        McpServer::new(None).unwrap()
    }

    #[test]
    fn router_lists_the_four_read_tools_with_schemas() {
        let server = stdlib_server();
        let names: Vec<String> = server
            .router
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        // The router lists by name.
        assert_eq!(
            names,
            ["catalog_search", "check", "list_categories", "node_doc"]
        );
        for tool in server.router.list_all() {
            assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
            assert!(
                tool.output_schema.is_some(),
                "{} has an output schema",
                tool.name
            );
            assert!(
                tool.description.as_ref().is_some_and(|d| d.len() > 80),
                "{} tells the model when to use it",
                tool.name
            );
        }
    }

    #[test]
    fn search_ranks_exact_name_first_and_matches_gh_names() {
        let server = stdlib_server();
        let result = search(
            &server,
            &SearchArgs {
                query: "slider".to_owned(),
                category: None,
                limit: None,
            },
        )
        .unwrap();
        assert_eq!(result.nodes[0].name, "slider");
        assert_eq!(result.nodes[0].gh.as_deref(), Some("Number Slider"));
        assert!(
            result.nodes[0]
                .signature
                .starts_with("slider(value: Number")
        );

        // A Grasshopper migrant searches by the component they know.
        let result = search(
            &server,
            &SearchArgs {
                query: "Number Slider".to_owned(),
                category: None,
                limit: Some(3),
            },
        )
        .unwrap();
        assert_eq!(result.nodes[0].name, "slider");
        assert_eq!(result.nodes.len(), 3, "limit applies after ranking");
        assert!(result.total_matches > 3);
    }

    /// The GH-name field must carry weight on its own: `slider`'s GH name
    /// equals its title, so only a query that matches NOTHING but `gh`
    /// proves the branch (a deleted `gh` scoring line once left every test
    /// green).
    #[test]
    fn search_matches_gh_names_that_appear_nowhere_else() {
        let server = stdlib_server();
        let query = |text: &str| {
            search(
                &server,
                &SearchArgs {
                    query: text.to_owned(),
                    category: None,
                    limit: None,
                },
            )
            .unwrap()
        };
        // `pick`: name "pick", title "Pick", GH "Pick'n'Choose" — the
        // apostrophes keep the word out of every prose field.
        let result = query("Pick'n'Choose");
        assert_eq!(result.total_matches, 1, "{result:?}");
        assert_eq!(result.nodes[0].name, "pick");
        assert_eq!(result.nodes[0].score, 80, "an exact GH match ranks 80");
        // `add`'s GH name "Addition" (exact, 80) outranks `mass_addition`'s
        // name prefix (60): without the GH branch `add` would not match.
        let result = query("addition");
        assert_eq!(result.nodes[0].name, "add", "{result:?}");
        assert_eq!(result.nodes[0].score, 80);
        assert_eq!(result.nodes[1].name, "mass_addition");
        assert_eq!(result.nodes[1].score, 60);
    }

    #[test]
    fn empty_query_lists_a_category_and_unknown_category_refuses() {
        let server = stdlib_server();
        let all = search(
            &server,
            &SearchArgs {
                query: String::new(),
                category: Some("Params & input".to_owned()),
                limit: Some(1000),
            },
        )
        .unwrap();
        assert!(all.nodes.iter().all(|hit| hit.category == "Params & input"));
        assert!(all.nodes.iter().any(|hit| hit.name == "slider"));
        let refused = search(
            &server,
            &SearchArgs {
                query: String::new(),
                category: Some("Widgets".to_owned()),
                limit: None,
            },
        )
        .unwrap_err();
        assert!(matches!(refused, Refusal::UnknownCategory { .. }));
        assert!(refused.message().contains("Params & input"));
    }

    #[test]
    fn node_doc_is_the_catalog_entry_plus_signature() {
        let server = stdlib_server();
        let doc = doc_of(&server, "slider").unwrap();
        assert_eq!(doc["gh"], "Number Slider");
        assert_eq!(
            doc["signature"],
            "slider(value: Number, min: Number = 0.0, max: Number = 10.0, step: Number = 0.0) → Number"
        );
        assert_eq!(doc["effectful"], false);
        assert_eq!(doc["pure"], true);
        let inputs = doc["inputs"].as_array().unwrap();
        assert_eq!(inputs[0]["name"], "value");
        assert_eq!(inputs[1]["default"], "0.0");
        assert!(doc["panics"].is_string());
        assert_eq!(doc["outputs"][0]["name"], "out");

        // An exporter: the flags are derived, not hardcoded.
        let exporter = doc_of(&server, "export_obj").unwrap();
        assert_eq!(exporter["pure"], false);
        assert_eq!(exporter["effectful"], true);
        assert_eq!(exporter["outputs"], serde_json::json!([]));
        assert!(exporter["signature"].as_str().unwrap().ends_with("→ ()"));

        let refused = doc_of(&server, "slidr").unwrap_err();
        match refused {
            Refusal::UnknownNode { did_you_mean, .. } => {
                assert_eq!(did_you_mean.as_deref(), Some("slider"));
            }
            other => panic!("expected an unknown-node refusal, got {other:?}"),
        }
    }

    /// The `node_doc` output schema describes the value the renderer
    /// produces — for EVERY node: each key of each entry (and of each port)
    /// is a described property, and each required property is present. A
    /// field added to `catalog::node()` (e.g. `volatile`, item 3b) fails
    /// here until `NodeDoc` documents it.
    #[test]
    fn node_doc_schema_matches_every_catalog_entry() {
        let server = stdlib_server();
        let tool = server.router.get("node_doc").unwrap();
        let schema = tool.output_schema.as_ref().unwrap();
        let properties = schema["properties"].as_object().unwrap();
        for key in [
            "inputs",
            "outputs",
            "panics",
            "gh",
            "signature",
            "effectful",
        ] {
            assert!(properties.contains_key(key), "schema describes `{key}`");
        }
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert!(
            required.contains(&"gh"),
            "gh is always present (null = Cicada-only)"
        );
        let port_schema = &schema["$defs"]["PortDoc"];
        let port_properties = port_schema["properties"].as_object().unwrap();
        let port_required: Vec<&str> = port_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();

        for spec in cicada_stdlib::registry() {
            let doc = doc_of(&server, spec.name).unwrap();
            for key in doc.keys() {
                assert!(
                    properties.contains_key(key),
                    "`{}` has `{key}` which the node_doc schema does not describe",
                    spec.name
                );
            }
            for key in &required {
                assert!(
                    doc.contains_key(*key),
                    "`{}` lacks required `{key}`",
                    spec.name
                );
            }
            for port in doc["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .chain(doc["outputs"].as_array().unwrap())
            {
                let port = port.as_object().unwrap();
                for key in port.keys() {
                    assert!(
                        port_properties.contains_key(key),
                        "port `{}` of `{}` has `{key}` which the PortDoc schema does not describe",
                        port["name"],
                        spec.name
                    );
                }
                for key in &port_required {
                    assert!(
                        port.contains_key(*key),
                        "port `{}` of `{}` lacks required `{key}`",
                        port["name"],
                        spec.name
                    );
                }
            }
        }
    }

    #[test]
    fn categories_cover_the_registry_in_ribbon_order() {
        let specs = cicada_stdlib::registry().to_vec();
        let categories = categories_of(&specs);
        assert_eq!(categories[0].0, "Params & input");
        assert_eq!(
            categories.iter().map(|(_, count)| count).sum::<usize>(),
            specs.len()
        );
    }

    #[test]
    fn check_text_reports_the_did_you_mean_and_clean_text_is_ok() {
        let server = stdlib_server();
        let result = check_pipeline(
            &server,
            CheckArgs {
                text: Some("# cicada 1\nx = slidr(value=1.0)\n".to_owned()),
                path: None,
            },
        )
        .unwrap();
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["kind"], "unknown_node");
        assert_eq!(result.diagnostics[0]["fix"]["replacement"], "slider");
        assert_eq!(result.bindings, ["x"]);
        assert_eq!(result.excluded.len(), 1);
        assert_eq!(
            (
                result.excluded[0].node.as_str(),
                result.excluded[0].status.as_str(),
                result.excluded[0].reason.as_str()
            ),
            ("x", "red", "has diagnostics")
        );

        let result = check_pipeline(
            &server,
            CheckArgs {
                text: Some("# cicada 1\nx = slider(value=1.0)\n".to_owned()),
                path: None,
            },
        )
        .unwrap();
        assert!(result.ok, "{:?}", result.diagnostics);
        assert!(result.excluded.is_empty());
        assert!(matches!(result.source, CheckedSource::Text));

        for (text, path) in [(None, None), (Some(String::new()), Some(String::new()))] {
            let refused = check_pipeline(&server, CheckArgs { text, path }).unwrap_err();
            assert!(matches!(refused, Refusal::CheckSource { .. }));
        }
        let refused = check_pipeline(
            &server,
            CheckArgs {
                text: None,
                path: Some("definitely-missing.cic".to_owned()),
            },
        )
        .unwrap_err();
        assert!(matches!(refused, Refusal::UnreadablePath { .. }));
    }

    /// What the checker alone never sees: an integer literal at 2^53 is
    /// refused by lowering (`cicada run` exits 1, the canvas shows the node
    /// red) — `check` must not say `ok` for it. `#off` is a deliberate
    /// state: excluded, reported, but still `ok`.
    #[test]
    fn check_reports_lowering_refusals_as_excluded_and_off_as_deliberate() {
        let server = stdlib_server();
        let text = |source: &str| {
            check_pipeline(
                &server,
                CheckArgs {
                    text: Some(source.to_owned()),
                    path: None,
                },
            )
            .unwrap()
        };
        let result = text(
            "# cicada 1\nn = duplicate(item=1.0, count=9007199254740993)\nm = reverse(list=n)\n",
        );
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.ok, "a lowering refusal is not ok");
        let by_name: HashMap<&str, (&str, &str)> = result
            .excluded
            .iter()
            .map(|e| (e.node.as_str(), (e.status.as_str(), e.reason.as_str())))
            .collect();
        let (status, reason) = by_name["n"];
        assert_eq!(status, "red");
        assert!(reason.contains("2^53"), "{reason}");
        assert_eq!(by_name["m"], ("blocked", "fed by red `n`"));

        let result = text(
            "# cicada 1\na = slider(value=1.0, min=0.0, max=2.0)\n#off b = multiply(a=a, b=2.0)\nd = add(a=a, b=1.0)\n",
        );
        assert!(result.ok, "{:?}", result.excluded);
        assert_eq!(result.excluded.len(), 1);
        assert_eq!(result.excluded[0].node, "b");
        assert_eq!(result.excluded[0].status, "red");
        assert_eq!(result.excluded[0].reason, "disabled (`#off`)");
        assert_eq!(result.bindings, ["a", "d"], "a disabled line binds nothing");
    }

    #[test]
    fn refusals_are_structured_error_results() {
        let result = Refusal::CheckSource {
            message: "m".to_owned(),
        }
        .into_result();
        assert_eq!(result.is_error, Some(true));
        let content = result.structured_content.unwrap();
        assert_eq!(content["error"], "check_source");
        assert_eq!(content["message"], "m");
    }
}
