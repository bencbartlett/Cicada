//! End-to-end `cicada mcp` (v0.1, docs/11 read tools): the real binary as
//! a Model Context Protocol server over stdio, driven with real JSON-RPC
//! framing — `initialize` → `notifications/initialized` → `tools/list` →
//! every tool once → `ping` — asserting the shapes an agent client reads:
//! tool schemas (`node_doc`'s real output schema), `node_doc`'s ports and
//! GH name, GH-only search matches, `check`'s did-you-mean diagnostic and
//! its `excluded` bindings (the dry lowering's refusals), and (with
//! `--project`) a script node in the catalog plus the three `path` cases —
//! in the project, outside it, in a scripted subdirectory.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

/// The protocol revision the test client speaks — a released one every MCP
/// client uses; the server answers with a version it supports.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// One MCP client session over the binary's stdio.
struct Client {
    child: Child,
    /// `None` once closed (EOF to the server).
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn spawn(args: &[&str], cwd: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cicada"))
            .arg("mcp")
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("cicada binary runs");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            next_id: 1,
        }
    }

    /// Send one JSON-RPC request and return its `result` (panics on an
    /// `error` response — every call in these tests is expected to route).
    fn request(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let response = self.read();
        assert_eq!(response["jsonrpc"], "2.0", "{response}");
        assert_eq!(response["id"], id, "responses arrive in order: {response}");
        assert!(
            response.get("error").is_none(),
            "{method} returned a protocol error: {response}"
        );
        response["result"].clone()
    }

    /// Send one JSON-RPC request and return the raw response (for the
    /// protocol-error assertions).
    fn request_raw(&mut self, method: &str, params: &Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.read()
    }

    fn notify(&mut self, method: &str) {
        self.write(&json!({"jsonrpc": "2.0", "method": method}));
    }

    fn write(&mut self, message: &Value) {
        let mut line = serde_json::to_string(message).unwrap();
        line.push('\n');
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).unwrap();
        assert!(read > 0, "the server closed stdout before answering");
        serde_json::from_str(line.trim_end()).unwrap_or_else(|error| {
            panic!("stdout must carry JSON-RPC only — got {line:?}: {error}")
        })
    }

    /// The MCP handshake; returns the `initialize` result.
    fn initialize(&mut self) -> Value {
        let result = self.request(
            "initialize",
            &json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "cicada-mcp-test", "version": "0"}
            }),
        );
        self.notify("notifications/initialized");
        result
    }

    /// `tools/call`; returns the result object (`content`,
    /// `structuredContent`, `isError`).
    fn call(&mut self, tool: &str, arguments: &Value) -> Value {
        self.request("tools/call", &json!({"name": tool, "arguments": arguments}))
    }

    /// Close stdin (EOF = the client went away) and wait for a clean exit.
    /// A watchdog kills a server that ignores EOF, so a regression there
    /// fails instead of hanging CI.
    fn finish(mut self) -> String {
        drop(self.stdin.take());
        let mut stderr = self.child.stderr.take().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_mins(1);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the server did not exit after EOF on stdin; killing it"
            );
            std::thread::yield_now();
        };
        let mut text = String::new();
        std::io::Read::read_to_string(&mut stderr, &mut text).unwrap();
        assert!(status.success(), "server exit {status}; stderr:\n{text}");
        text
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// The structured result of a tool call that must have succeeded.
fn structured(result: &Value) -> &Value {
    assert_ne!(result["isError"], true, "tool error: {result}");
    let content = result["content"].as_array().expect("content array");
    assert_eq!(content[0]["type"], "text");
    // The text block carries the same JSON as the structured content, for
    // clients that read only `content`.
    let from_text: Value = serde_json::from_str(content[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&from_text, &result["structuredContent"]);
    &result["structuredContent"]
}

#[test]
fn handshake_tool_list_and_every_tool_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = Client::spawn(&[], dir.path());

    let init = client.initialize();
    assert_eq!(init["serverInfo"]["name"], "cicada");
    assert!(init["capabilities"]["tools"].is_object(), "{init}");
    assert!(
        init["instructions"]
            .as_str()
            .is_some_and(|text| text.contains("catalog_search") && text.contains("check")),
        "the instructions tell the model how to use the server: {init}"
    );
    let version = init["protocolVersion"].as_str().unwrap();
    assert!(
        version <= PROTOCOL_VERSION,
        "the server never answers with a newer revision than the client asked for: {version}"
    );

    let listed = client.request("tools/list", &json!({}));
    assert_tool_list(listed["tools"].as_array().unwrap());
    assert_catalog_search(&mut client);
    assert_list_categories(&mut client);
    assert_node_doc(&mut client);
    assert_check(&mut client);

    // An unknown tool is a JSON-RPC error (there is nothing to route to).
    let unknown = client.request_raw(
        "tools/call",
        &json!({"name": "what_feeds", "arguments": {}}),
    );
    assert!(unknown["error"].is_object(), "{unknown}");

    // ping answers with an empty result.
    let pong = client.request("ping", &json!({}));
    assert_eq!(pong, json!({}));

    let stderr = client.finish();
    assert!(
        stderr.contains("cicada mcp") && stderr.contains("node(s) in the catalog"),
        "the startup note goes to stderr, never stdout:\n{stderr}"
    );
}

/// `tools/list`: the four read tools, each with an object input schema, an
/// output schema, read-only annotations, and a description that says when
/// to use it.
fn assert_tool_list(tools: &[Value]) {
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["catalog_search", "check", "list_categories", "node_doc"]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
        assert!(tool["outputSchema"].is_object(), "{tool}");
        assert!(
            tool["description"].as_str().unwrap().len() > 80,
            "{}",
            tool["name"]
        );
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
    }
    let search_tool = tools
        .iter()
        .find(|t| t["name"] == "catalog_search")
        .unwrap();
    assert!(
        search_tool["inputSchema"]["properties"]["query"].is_object()
            && search_tool["inputSchema"]["properties"]["category"].is_object()
            && search_tool["inputSchema"]["properties"]["limit"].is_object(),
        "{search_tool}"
    );
    let check_tool = tools.iter().find(|t| t["name"] == "check").unwrap();
    assert!(
        check_tool["inputSchema"]["properties"]["text"].is_object()
            && check_tool["inputSchema"]["properties"]["path"].is_object(),
        "{check_tool}"
    );
    assert!(
        check_tool["outputSchema"]["properties"]["excluded"].is_object()
            && check_tool["outputSchema"]["properties"]["diagnostics"].is_object(),
        "{check_tool}"
    );
    // node_doc's output schema is the real shape, not an open object: a
    // client validating structuredContent learns the fields.
    let doc_tool = tools.iter().find(|t| t["name"] == "node_doc").unwrap();
    let doc_properties = doc_tool["outputSchema"]["properties"].as_object().unwrap();
    for key in [
        "inputs",
        "outputs",
        "panics",
        "gh",
        "signature",
        "effectful",
        "examples",
    ] {
        assert!(
            doc_properties.contains_key(key),
            "node_doc schema lacks `{key}`: {doc_tool}"
        );
    }
    assert_ne!(
        doc_tool["outputSchema"]["additionalProperties"], true,
        "{doc_tool}"
    );
    let port_properties = doc_tool["outputSchema"]["$defs"]["PortDoc"]["properties"]
        .as_object()
        .unwrap();
    for key in ["type", "default", "doc", "optional", "dimension"] {
        assert!(
            port_properties.contains_key(key),
            "PortDoc schema lacks `{key}`"
        );
    }
}

/// `catalog_search`: the exact name ranks first; GH names match too — on
/// their own, by a word no other field carries; limit applies after
/// ranking.
fn assert_catalog_search(client: &mut Client) {
    let hits = client.call("catalog_search", &json!({"query": "slider", "limit": 5}));
    let hits = structured(&hits);
    assert_eq!(hits["nodes"][0]["name"], "slider");
    assert_eq!(hits["nodes"][0]["gh"], "Number Slider");
    assert_eq!(hits["nodes"][0]["category"], "Params & input");
    assert!(
        hits["nodes"][0]["signature"]
            .as_str()
            .unwrap()
            .starts_with("slider(value: Number, min: Number = 0.0")
    );
    assert!(hits["nodes"].as_array().unwrap().len() <= 5);
    let by_gh = client.call(
        "catalog_search",
        &json!({"query": "Number Slider", "limit": 1}),
    );
    assert_eq!(structured(&by_gh)["nodes"][0]["name"], "slider");
    // A GH name that appears in no other field: `pick` is "Pick'n'Choose"
    // in Grasshopper; `add` is "Addition" (exact, 80) and must beat
    // `mass_addition`'s name prefix (60).
    let gh_only = client.call("catalog_search", &json!({"query": "Pick'n'Choose"}));
    let gh_only = structured(&gh_only);
    assert_eq!(gh_only["total_matches"], 1, "{gh_only}");
    assert_eq!(gh_only["nodes"][0]["name"], "pick");
    assert_eq!(gh_only["nodes"][0]["score"], 80);
    let addition = client.call("catalog_search", &json!({"query": "addition"}));
    let addition = structured(&addition);
    assert_eq!(addition["nodes"][0]["name"], "add", "{addition}");
    assert_eq!(addition["nodes"][0]["score"], 80);
    assert_eq!(addition["nodes"][1]["name"], "mass_addition");
}

/// `list_categories`: ribbon order, counts sum to the catalog.
fn assert_list_categories(client: &mut Client) {
    let categories = client.call("list_categories", &json!({}));
    let categories = structured(&categories);
    assert_eq!(categories["categories"][0]["name"], "Params & input");
    let sum: u64 = categories["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["count"].as_u64().unwrap())
        .sum();
    assert_eq!(Some(sum), categories["total"].as_u64());
    assert!(sum >= 100, "the v0.1 catalog: {sum}");
}

/// `node_doc` slider: every port with type/default/doc, the GH name, the
/// contract, the runnable example, the signature — and a typo's
/// did-you-mean as a tool-level error the model can read.
fn assert_node_doc(client: &mut Client) {
    let doc = client.call("node_doc", &json!({"name": "slider"}));
    let doc = structured(&doc);
    assert_eq!(doc["name"], "slider");
    assert_eq!(doc["gh"], "Number Slider");
    assert_eq!(doc["title"], "Number Slider");
    assert_eq!(doc["pure"], true);
    assert_eq!(doc["effectful"], false);
    assert_eq!(doc["tier"], "S");
    assert_eq!(
        doc["signature"],
        "slider(value: Number, min: Number = 0.0, max: Number = 10.0, step: Number = 0.0) → Number"
    );
    let inputs = doc["inputs"].as_array().unwrap();
    let port_names: Vec<&str> = inputs.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(port_names, ["value", "min", "max", "step"]);
    assert_eq!(inputs[0]["type"], "Number");
    assert!(inputs[0].get("default").is_none(), "value is required");
    assert_eq!(inputs[1]["default"], "0.0");
    assert!(inputs[1]["doc"].as_str().unwrap().ends_with('.'));
    assert_eq!(doc["outputs"][0]["name"], "out");
    assert!(
        doc["outputs"][0]["doc"]
            .as_str()
            .unwrap()
            .contains("min..=max")
    );
    assert!(doc["panics"].as_str().unwrap().contains("inverted"));
    assert!(
        doc["examples"][0]
            .as_str()
            .unwrap()
            .starts_with("amps = slider(")
    );

    // An exporter: `pure` / `effectful` come from the spec, not a constant.
    let exporter = client.call("node_doc", &json!({"name": "export_obj"}));
    let exporter = structured(&exporter);
    assert_eq!(exporter["pure"], false);
    assert_eq!(exporter["effectful"], true);
    assert_eq!(exporter["tier"], "S");
    assert_eq!(exporter["version"], 1);
    assert!(exporter["gh"].is_null(), "Cicada-only: null, present");
    assert_eq!(exporter["outputs"], json!([]));

    let missing = client.call("node_doc", &json!({"name": "slidr"}));
    assert_eq!(missing["isError"], true, "{missing}");
    assert_eq!(missing["structuredContent"]["error"], "unknown_node");
    assert_eq!(missing["structuredContent"]["did_you_mean"], "slider");
}

/// `check`: the unknown-node diagnostic carries the did-you-mean fix; clean
/// text is ok; exactly one source; unknown arguments refuse loudly.
fn assert_check(client: &mut Client) {
    let checked = client.call(
        "check",
        &json!({"text": "# cicada 1\namps = slidr(value=1.0, min=0.0, max=2.0)\n"}),
    );
    let checked = structured(&checked);
    assert_eq!(checked["ok"], false);
    assert_eq!(checked["diagnostic_count"], 1);
    let diagnostic = &checked["diagnostics"][0];
    assert_eq!(diagnostic["kind"], "unknown_node");
    assert_eq!(diagnostic["node"], "amps");
    assert_eq!(diagnostic["span"]["line"], 2);
    assert!(
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("no node named `slidr`")
    );
    assert_eq!(diagnostic["fix"]["label"], "did you mean `slider`?");
    assert_eq!(diagnostic["fix"]["replacement"], "slider");
    assert_eq!(checked["bindings"], json!(["amps"]));
    assert_eq!(checked["source"]["kind"], "text");
    assert_eq!(
        checked["excluded"],
        json!([{"node": "amps", "status": "red", "reason": "has diagnostics"}])
    );

    let clean = client.call(
        "check",
        &json!({"text": "# cicada 1\namps = slider(value=1.0, min=0.0, max=2.0)\ntwice = multiply(a=amps, b=2.0)\n"}),
    );
    let clean = structured(&clean);
    assert_eq!(clean["ok"], true, "{clean}");
    assert_eq!(clean["diagnostics"], json!([]));
    assert_eq!(clean["excluded"], json!([]));
    assert_eq!(clean["bindings"], json!(["amps", "twice"]));

    // A refusal only lowering sees (the checker carries literals as f64;
    // 2^53 is refused as inexact) — `cicada run` exits 1 on this text and
    // the canvas shows `n` red, so `check` must not say ok. The downstream
    // binding is blocked, with the canvas's words.
    let lowering = client.call(
        "check",
        &json!({"text": "# cicada 1\nn = duplicate(item=1.0, count=9007199254740993)\nm = reverse(list=n)\n"}),
    );
    let lowering = structured(&lowering);
    assert_eq!(lowering["ok"], false, "{lowering}");
    assert_eq!(lowering["diagnostic_count"], 0);
    let excluded = lowering["excluded"].as_array().unwrap();
    let n = excluded.iter().find(|e| e["node"] == "n").unwrap();
    assert_eq!(n["status"], "red");
    assert!(
        n["reason"].as_str().unwrap().contains("2^53"),
        "the lowering's own message: {n}"
    );
    let m = excluded.iter().find(|e| e["node"] == "m").unwrap();
    assert_eq!(m["status"], "blocked");
    assert_eq!(m["reason"], "fed by red `n`");

    // Exactly one source; an unknown argument is refused rather than
    // ignored (deny_unknown_fields).
    let neither = client.call("check", &json!({}));
    assert_eq!(neither["isError"], true, "{neither}");
    assert_eq!(neither["structuredContent"]["error"], "check_source");
    let misspelled = client.call("check", &json!({"txt": "# cicada 1\n"}));
    assert_eq!(misspelled["isError"], true, "{misspelled}");
    assert!(
        misspelled["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("txt"),
        "{misspelled}"
    );
}

const SCRIPT: &str = "import cicada\n\
                      @cicada.node(title=\"Triple Up\", description=\"x times three.\")\n\
                      def triple_up(x: \"Number\") -> \"Number\":\n    return x * 3.0\n";

#[test]
fn project_scripts_join_the_catalog_and_check_resolves_project_paths() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    std::fs::write(project.path().join("scripts").join("triple.py"), SCRIPT).unwrap();
    std::fs::write(
        project.path().join("pipeline.cic"),
        "# cicada 1\nbase = slider(value=3.0, min=0.0, max=10.0)\ntripled = triple_up(x=base)\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("broken.cic"),
        "# cicada 1\ntripled = triple_up(y=1.0)\n",
    )
    .unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let project_arg = project.path().to_string_lossy().into_owned();
    // Launched from an unrelated directory: relative `check` paths resolve
    // against --project, not the working directory.
    let mut client = Client::spawn(&["--project", &project_arg], elsewhere.path());
    client.initialize();

    let hits = client.call("catalog_search", &json!({"query": "triple"}));
    let hits = structured(&hits);
    assert_eq!(hits["nodes"][0]["name"], "triple_up", "{hits}");
    assert_eq!(hits["nodes"][0]["title"], "Triple Up");
    assert!(hits["nodes"][0]["gh"].is_null());

    let doc = client.call("node_doc", &json!({"name": "triple_up"}));
    let doc = structured(&doc);
    assert_eq!(doc["inputs"][0]["name"], "x");
    assert_eq!(doc["inputs"][0]["type"], "Number");
    assert_eq!(doc["signature"], "triple_up(x: Number) → Number");

    let text = client.call(
        "check",
        &json!({"text": "# cicada 1\nt = triple_up(x=2.0)\n"}),
    );
    assert_eq!(structured(&text)["ok"], true, "{text}");

    let file = client.call("check", &json!({"path": "pipeline.cic"}));
    let file = structured(&file);
    // `ok` covers the dry lowering too: the script node lowers (its run
    // function exists in the kept catalog) — a missing script map would
    // exclude `tripled` as red.
    assert_eq!(file["ok"], true, "{file}");
    assert_eq!(file["excluded"], json!([]));
    assert_eq!(file["source"]["kind"], "path");
    assert!(
        file["source"]["path"]
            .as_str()
            .unwrap()
            .ends_with("pipeline.cic")
    );

    let broken = client.call("check", &json!({"path": "broken.cic"}));
    let broken = structured(&broken);
    assert_eq!(broken["ok"], false);
    let kinds: Vec<&str> = broken["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["kind"].as_str().unwrap())
        .collect();
    // `y` is unknown and `x` is missing; the checker folds the two into one
    // unknown-kwarg diagnostic whose fix names the missing port.
    assert_eq!(kinds, ["unknown_kwarg"]);
    assert_eq!(broken["diagnostics"][0]["fix"]["replacement"], "x");

    let missing = client.call("check", &json!({"path": "nope.cic"}));
    assert_eq!(missing["isError"], true, "{missing}");
    assert_eq!(missing["structuredContent"]["error"], "unreadable_path");

    assert_check_outside_the_project(&mut client, project.path());

    // A script added while the server runs joins the catalog on the next
    // call — discovery re-runs when scripts/ changes (no restart, no stale
    // catalog).
    std::fs::write(
        project.path().join("scripts").join("quad.py"),
        SCRIPT
            .replace("triple_up", "quad_up")
            .replace("Triple Up", "Quad Up")
            .replace("* 3.0", "* 4.0"),
    )
    .unwrap();
    let hits = client.call("catalog_search", &json!({"query": "quad_up"}));
    assert_eq!(structured(&hits)["nodes"][0]["name"], "quad_up");

    let stderr = client.finish();
    assert!(stderr.contains("project "), "{stderr}");
}

/// `check {path}` outside the project: a file elsewhere is checked against
/// its OWN directory's scripts, exactly as `cicada run` would — not the
/// project's (the stranger's node is unknown to the project catalog, as
/// `text` shows, yet its own pipeline is ok by absolute path); a scripted
/// subdirectory of the project (`examples/wall/` under `examples/`) is a
/// project of its own — relative to `--project` for resolution, its own
/// directory for the catalog. Forcing the in-project branch once passed
/// every test.
fn assert_check_outside_the_project(client: &mut Client, project: &Path) {
    let half_script = SCRIPT
        .replace("triple_up", "half_down")
        .replace("Triple Up", "Half Down")
        .replace("* 3.0", "* 0.5");
    let stranger = tempfile::tempdir().unwrap();
    std::fs::create_dir(stranger.path().join("scripts")).unwrap();
    std::fs::write(
        stranger.path().join("scripts").join("half.py"),
        &half_script,
    )
    .unwrap();
    let stranger_text = "# cicada 1\nh = half_down(x=4.0)\n";
    std::fs::write(stranger.path().join("other.cic"), stranger_text).unwrap();
    let as_text = client.call("check", &json!({"text": stranger_text}));
    let as_text = structured(&as_text);
    assert_eq!(as_text["ok"], false, "{as_text}");
    assert_eq!(as_text["diagnostics"][0]["kind"], "unknown_node");
    let absolute = stranger.path().join("other.cic");
    let as_file = client.call("check", &json!({"path": absolute.to_string_lossy()}));
    let as_file = structured(&as_file);
    assert_eq!(as_file["ok"], true, "{as_file}");
    assert_eq!(as_file["bindings"], json!(["h"]));
    assert!(
        as_file["source"]["path"]
            .as_str()
            .unwrap()
            .ends_with("other.cic")
    );
    // The scripted subdirectory — inside the project tree, its own catalog.
    let inner_dir = project.join("inner");
    std::fs::create_dir_all(inner_dir.join("scripts")).unwrap();
    std::fs::write(inner_dir.join("scripts").join("half.py"), &half_script).unwrap();
    std::fs::write(inner_dir.join("inner.cic"), stranger_text).unwrap();
    let inner = client.call("check", &json!({"path": "inner/inner.cic"}));
    let inner = structured(&inner);
    assert_eq!(inner["ok"], true, "{inner}");
    assert!(
        inner["source"]["path"]
            .as_str()
            .unwrap()
            .ends_with("inner.cic")
    );
}

#[test]
fn a_broken_scripts_directory_refuses_at_startup() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    // A script that collides with a stdlib node is the discovery refusal
    // `cicada run` gives; the MCP server must not start with a catalog it
    // could not build.
    std::fs::write(
        project.path().join("scripts").join("clash.py"),
        SCRIPT.replace("triple_up", "slider"),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cicada"))
        .arg("mcp")
        .arg("--project")
        .arg(project.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("slider"), "{stderr}");
    assert!(output.stdout.is_empty(), "nothing but JSON-RPC on stdout");
}
