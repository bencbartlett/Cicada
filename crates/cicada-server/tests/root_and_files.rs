//! The root and the file list (docs/17 wave 4 O1; docs/13 §Projects,
//! pipelines, sessions, §HTTP surface `GET /api/files`, §External changes
//! and file watching) over a REAL served root in a temp dir: a root with no
//! default pipeline opens nothing and serves its listing one directory at
//! a time in the documented shape; every escape is refused with the typed
//! body and nothing above the root is ever named; an unreadable directory
//! is `io_error`; a pipeline in a SUBDIRECTORY of the root opens by its
//! root-relative name and its external changes still reload — the watcher
//! follows sessions, not the root (a root may be a home directory). No
//! network beyond loopback; the store lives in the temp dir.
//!
//! Two fixtures need the OS's co-operation and SKIP LOUDLY (a printed
//! reason, those assertions left out — never `#[ignore]`, never a silent
//! pass) when it is refused: a directory link (Windows without Developer
//! Mode refuses symlinks; a junction is tried then), and a directory the
//! process may not read (root on Unix reads everything).

// Tests are exempt from the expect/unwrap denial (clippy.toml), but the
// exemption recognizes #[test] fns only — not helpers in integration tests.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cicada_server::protocol::PROTOCOL_VERSION;
use cicada_server::{ServeConfig, ServerHandle, serve};
use futures_util::{SinkExt as _, StreamExt as _};
use tokio_tungstenite::tungstenite::Message;

const PIPELINE: &str = "# cicada 1\n\
                        size = slider(value=2.0, min=0.5, max=5.0)\n\
                        span = construct_domain(start=0.0, end=size)\n\
                        block = box(x=span, y=span, z=span)\n";

const PIPELINE_CHANGED: &str = "# cicada 1\n\
                                size = slider(value=3.0, min=0.5, max=5.0)\n\
                                span = construct_domain(start=0.0, end=size)\n\
                                block = box(x=span, y=span, z=span)\n";

const PIPELINE_CHANGED_AGAIN: &str = "# cicada 1\n\
                                      size = slider(value=4.0, min=0.5, max=5.0)\n\
                                      span = construct_domain(start=0.0, end=size)\n\
                                      block = box(x=span, y=span, z=span)\n";

/// A script node (discovery refuses a `.py` with none) — nothing in the
/// pipeline uses it; its ARRIVAL beside the pipeline is what must be seen.
const SCRIPT: &str = "import cicada\n\n\
                      @cicada.node(title=\"Triple\", description=\"x times three.\")\n\
                      def triple(x: \"Number\") -> \"Number\":\n    return x * 3.0\n";

/// The same node edited in place — the SAME file name, so the pipeline's
/// directory watch sees no entry change; only a watch ON `scripts/` can
/// see this rewrite — with a title the catalog shows once a rescan has
/// read it.
const SCRIPT_EDITED: &str = "import cicada\n\n\
                             @cicada.node(title=\"Quadruple\", description=\"x times four.\")\n\
                             def triple(x: \"Number\") -> \"Number\":\n    return x * 4.0\n";

/// Minimal HTTP/1.1 GET (loopback only): `(status, body)`.
fn http_get(addr: SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let auth = token.map_or(String::new(), |t| format!("X-Cicada-Token: {t}\r\n"));
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {addr}\r\n{auth}Connection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("http response");
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    assert!(
        !head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "unexpected chunked body:\n{head}"
    );
    (status, body.to_owned())
}

/// `GET` with the token; the body parsed when it is JSON, the text otherwise.
async fn get(addr: SocketAddr, path: &str) -> (u16, serde_json::Value) {
    let path = path.to_owned();
    let (status, body) = tokio::task::spawn_blocking(move || http_get(addr, &path, Some("t")))
        .await
        .unwrap();
    let value = serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body));
    (status, value)
}

/// Percent-encode a query value (everything but the unreserved set).
fn urlencode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02X}").unwrap();
        }
    }
    out
}

/// `<tmp>/root` — every kind of entry the listing rules speak about — plus
/// `<tmp>/elsewhere/secret.cic` BEYOND the root and the store beside it,
/// so neither can appear in a listing by accident.
struct Fixture {
    /// Owns the whole temp tree (the root, the outside, the store); its
    /// name is what no listing may ever contain.
    dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    cache: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    for sub in [
        "sub/inner",
        "Zeta",
        "alpha",
        ".hidden",
        "node_modules",
        "target",
    ] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    for (file, text) in [
        ("top.cic", PIPELINE),
        ("Beta.cic", PIPELINE),
        ("alpha.cic", PIPELINE),
        ("README.md", "not a pipeline\n"),
        ("sub/p.cic", PIPELINE),
        ("sub/inner/q.cic", PIPELINE),
        (".hidden/h.cic", PIPELINE),
    ] {
        std::fs::write(root.join(file), text).unwrap();
    }
    // Named so that no server prose can contain it by accident (the sweep
    // below greps responses for it).
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.cic"), PIPELINE).unwrap();
    let cache = dir.path().join("cache");
    Fixture {
        dir,
        root,
        outside,
        cache,
    }
}

/// Serve the fixture's root with NO default pipeline on an ephemeral port.
async fn start(fx: &Fixture) -> ServerHandle {
    let mut config = ServeConfig::new(fx.root.clone());
    config.port = 0;
    config.token = Some("t".to_owned());
    config.cache_dir = Some(fx.cache.clone());
    config.threads = 2;
    serve(config).await.expect("serve")
}

fn entries(listing: &serde_json::Value) -> Vec<(String, String)> {
    listing["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("no entries in {listing}"))
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap().to_owned(),
                e["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, k)| ((*n).to_owned(), (*k).to_owned()))
        .collect()
}

/// A directory link: a symlink, or on Windows — where symlinks need
/// Developer Mode — a junction (`mklink /J`, no privilege). `Err` with the
/// reason when the OS refuses both.
fn link_dir(target: &Path, link: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).map_err(|e| format!("symlink: {e}"))
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => Ok(()),
            Err(first) => {
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(link)
                    .arg(target)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                match status {
                    Ok(status) if status.success() => Ok(()),
                    other => Err(format!("symlink_dir: {first}; mklink /J: {other:?}")),
                }
            }
        }
    }
}

/// A directory the process may not list, restored on drop. `None` when
/// the denial cannot be arranged or does not take (root on Unix).
struct Denied(PathBuf);

fn deny_listing(dir: &Path) -> Option<Denied> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).ok()?;
    }
    #[cfg(windows)]
    {
        // `*S-1-1-0` is Everyone by SID (locale-independent); `RD` is
        // "read data / list directory".
        let status = std::process::Command::new("icacls")
            .arg(dir)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
    }
    let denied = Denied(dir.to_owned());
    if std::fs::read_dir(dir).is_ok() {
        // The denial did not take (a privileged user): the guard restores.
        drop(denied);
        return None;
    }
    Some(denied)
}

impl Drop for Denied {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("icacls")
                .arg(&self.0)
                .args(["/remove:d", "*S-1-1-0"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The next text message of `kind` (others — deltas, statuses — are
/// skipped), within a deadline; what WAS seen is the failure's report.
async fn next_of(socket: &mut Socket, kind: &str) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == kind {
                    return value;
                }
                seen.push(value["type"].as_str().unwrap_or("?").to_owned());
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(error))) => panic!("socket error while waiting for {kind}: {error}"),
            Ok(None) => panic!("socket closed while waiting for {kind}; saw {seen:?}"),
            Err(elapsed) => panic!("no `{kind}` within 20 s ({elapsed}); saw {seen:?}"),
        }
    }
}

async fn join(addr: SocketAddr, pipeline: &str) -> Socket {
    let url = format!("ws://{addr}/ws?token=t&pipeline={pipeline}");
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.expect("ws");
    socket
        .send(Message::Text(
            format!(
                r#"{{"v":{PROTOCOL_VERSION},"type":"hello","payload":{{"v":{PROTOCOL_VERSION}}}}}"#
            )
            .into(),
        ))
        .await
        .unwrap();
    let hello = next_of(&mut socket, "hello").await;
    assert_eq!(hello["payload"]["role"], "writer", "{hello}");
    let _snapshot = next_of(&mut socket, "snapshot").await;
    socket
}

/// The server's watched set as `/debug/state` reports it: root-relative,
/// sorted (docs/13 §External changes).
async fn watched(addr: SocketAddr, pipeline: &str) -> Vec<String> {
    let (status, state) = get(addr, &format!("/debug/state?pipeline={pipeline}")).await;
    assert_eq!(status, 200, "{state}");
    state["watched"]
        .as_array()
        .unwrap_or_else(|| panic!("no `watched` in /debug/state: {state}"))
        .iter()
        .map(|dir| dir.as_str().unwrap().to_owned())
        .collect()
}

/// The title the session's catalog gives script node `name` — `None`
/// until a (re)scan has discovered it.
async fn script_title(addr: SocketAddr, pipeline: &str, name: &str) -> Option<String> {
    let (status, catalog) = get(addr, &format!("/api/catalog?pipeline={pipeline}")).await;
    assert_eq!(status, 200, "{catalog}");
    catalog["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("no nodes in {catalog}"))
        .iter()
        .find(|node| node["name"] == name)
        .map(|node| node["title"].as_str().unwrap().to_owned())
}

/// Wait until the session's text contains `needle`: read `/debug/state`;
/// while it does not, take the next barrier snapshot (its reason must be
/// "external file change") and read again. See [`wait_for_script_title`]
/// for why the waits here read STATE instead of counting snapshots.
async fn wait_for_text(socket: &mut Socket, addr: SocketAddr, pipeline: &str, needle: &str) {
    let mut snapshots = 0;
    loop {
        let (status, state) =
            get(addr, &format!("/debug/state?pipeline={pipeline}&wait=true")).await;
        assert_eq!(status, 200, "{state}");
        let text = state["text"].as_str().unwrap().to_owned();
        if text.contains(needle) {
            return;
        }
        assert!(
            snapshots < 20,
            "{snapshots} snapshots later the text still lacks {needle:?}: {text}"
        );
        let barrier = next_of(socket, "snapshot").await;
        snapshots += 1;
        assert_eq!(
            barrier["payload"]["reason"], "external file change",
            "{barrier}"
        );
    }
}

/// Wait until the session's catalog gives script node `name` the title
/// `expected`: read the catalog; while it does not, take the next barrier
/// snapshot (its reason must be "external file change") and read again.
///
/// STATE-based on purpose. How many snapshots one burst of file events
/// yields is timing-dependent — a directory created and a file written
/// into it have landed in one 80 ms coalescing window and in two — so a
/// test that consumed "the next snapshot" per step could take a burst's
/// second snapshot for the next step's and pass without that step's change
/// ever being seen (the false PASS the review of 2026-08-24 found: with
/// the late re-watch made a no-op the old step 3 still passed). The title
/// changes only when a rescan has READ the edited file. One thing can still
/// fake it: a rescan owed to an EARLIER event, still in the coalescer when
/// the edit lands, reads the edited file (seen, 2026-08-24: the same
/// mutation passed this wait in 0.5 s once the watched-set assertion was
/// removed) — so a step whose proof is this wait first quiesces the watcher
/// with an ordered barrier ([`PIPELINE_CHANGED_AGAIN`] in the late-scripts
/// test: an event on the same directory handle is delivered after every
/// earlier one of that handle, and once its effect shows, nothing earlier
/// is pending).
async fn wait_for_script_title(
    socket: &mut Socket,
    addr: SocketAddr,
    pipeline: &str,
    name: &str,
    expected: &str,
) {
    let mut snapshots = 0;
    loop {
        let title = script_title(addr, pipeline, name).await;
        if title.as_deref() == Some(expected) {
            return;
        }
        assert!(
            snapshots < 20,
            "{snapshots} snapshots later `{name}` is still {title:?}, not {expected:?}"
        );
        let barrier = next_of(socket, "snapshot").await;
        snapshots += 1;
        assert_eq!(
            barrier["payload"]["reason"], "external file change",
            "{barrier}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_root_without_a_pipeline_opens_nothing_and_lists_one_directory_at_a_time() {
    let fx = fixture();
    let handle = start(&fx).await;
    let addr = handle.addr;
    assert_eq!(
        handle.url(),
        format!("http://{addr}/?token=t"),
        "no pipeline → no `&pipeline=` in the URL: the page with only a token is the picker"
    );
    let (status, project) = get(addr, "/api/project").await;
    assert_eq!(status, 200, "{project}");
    assert_eq!(project["default"], serde_json::Value::Null);
    assert_eq!(project["open"], serde_json::json!([]), "nothing opened");
    let (status, body) = get(addr, "/debug/state").await;
    assert_eq!(status, 400, "a pipeline-less read says so: {body}");

    // Token-gated like every /api route.
    let (status, _) = tokio::task::spawn_blocking(move || http_get(addr, "/api/files", None))
        .await
        .unwrap();
    assert_eq!(status, 401);

    // ---- The root: the documented shape, exactly.
    let (status, listing) = get(addr, "/api/files").await;
    assert_eq!(status, 200, "{listing}");
    assert_eq!(listing["root"], "root", "the root's own name, not its path");
    assert_eq!(listing["dir"], "");
    assert_eq!(listing["parent"], serde_json::Value::Null);
    assert_eq!(
        listing.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["dir", "entries", "parent", "root"],
        "exactly root, dir, parent, entries"
    );
    assert_eq!(
        entries(&listing),
        pairs(&[
            ("alpha", "dir"),
            ("sub", "dir"),
            ("Zeta", "dir"),
            ("alpha.cic", "pipeline"),
            ("Beta.cic", "pipeline"),
            ("top.cic", "pipeline"),
        ]),
        "directories first, case-insensitive order; .hidden, node_modules, target, README.md absent"
    );
    for entry in listing["entries"].as_array().unwrap() {
        assert_eq!(
            entry.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["kind", "modified_ms", "name"],
            "exactly name, kind, modified_ms: {entry}"
        );
        assert!(
            entry["modified_ms"].as_i64().unwrap() > 1_600_000_000_000,
            "a real file time: {entry}"
        );
    }
    // The same request spelled with an explicit empty dir.
    let (status, again) = get(addr, "/api/files?dir=").await;
    assert_eq!(status, 200);
    assert_eq!(again, listing);

    // ---- One directory down, and two, under every spelling the
    // normaliser accepts.
    let (status, sub) = get(addr, "/api/files?dir=sub").await;
    assert_eq!(status, 200, "{sub}");
    assert_eq!(
        (sub["dir"].as_str(), sub["parent"].as_str()),
        (Some("sub"), Some(""))
    );
    assert_eq!(
        entries(&sub),
        pairs(&[("inner", "dir"), ("p.cic", "pipeline")])
    );
    for spelling in ["sub/inner", "sub//inner/", "./sub/./inner"] {
        let (status, inner) = get(addr, &format!("/api/files?dir={}", urlencode(spelling))).await;
        assert_eq!(status, 200, "{spelling}: {inner}");
        assert_eq!(
            (inner["dir"].as_str(), inner["parent"].as_str()),
            (Some("sub/inner"), Some("sub")),
            "{spelling}"
        );
        assert_eq!(entries(&inner), pairs(&[("q.cic", "pipeline")]));
    }

    // ---- Unlisted is not unenterable: a dot-directory or a build tree
    // named in `dir` lists like any other directory under the root (the
    // root is the boundary; the skip list is about what the picker shows).
    let (status, hidden) = get(addr, "/api/files?dir=.hidden").await;
    assert_eq!(status, 200, "{hidden}");
    assert_eq!(
        (hidden["dir"].as_str(), hidden["parent"].as_str()),
        (Some(".hidden"), Some(""))
    );
    assert_eq!(entries(&hidden), pairs(&[("h.cic", "pipeline")]));
    let (status, skipped) = get(addr, "/api/files?dir=node_modules").await;
    assert_eq!(status, 200, "{skipped}");
    assert_eq!(entries(&skipped), pairs(&[]));

    // ---- `/api/project`'s walk skips the same directories the listing
    // leaves out (one predicate): the hidden pipeline is in neither.
    let pipelines = project["pipelines"].as_array().unwrap();
    assert!(
        pipelines.iter().any(|p| p == "sub/inner/q.cic")
            && pipelines
                .iter()
                .all(|p| !p.as_str().unwrap().starts_with(".hidden")),
        "{pipelines:?}"
    );

    // ---- Listing opened nothing; the subdirectory pipeline opens by its
    // root-relative name and is keyed by it.
    assert_eq!(
        get(addr, "/api/project").await.1["open"],
        serde_json::json!([])
    );
    let (status, state) = get(addr, "/debug/state?pipeline=sub/p.cic&wait=true").await;
    assert_eq!(status, 200, "{state}");
    assert_eq!(
        get(addr, "/api/project").await.1["open"],
        serde_json::json!(["sub/p.cic"])
    );
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_file_list_refuses_every_escape_and_names_nothing_above_the_root() {
    let fx = fixture();
    // A link that stays under the root, one that leaves it, and one that
    // dangles — when the OS lets a test make them.
    let links = match (
        link_dir(&fx.root.join("sub"), &fx.root.join("alias")),
        link_dir(&fx.outside, &fx.root.join("escape")),
        link_dir(&fx.root.join("never-made"), &fx.root.join("dangling")),
    ) {
        (Ok(()), Ok(()), Ok(())) => true,
        (inside, outside, dangling) => {
            eprintln!(
                "SKIPPING the directory-link assertions: this OS/user cannot create directory \
                 links here (inside: {inside:?}; outside: {outside:?}; dangling: {dangling:?})"
            );
            false
        }
    };
    let handle = start(&fx).await;
    let addr = handle.addr;
    let temp_name = fx
        .dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut bodies = Vec::new();

    // ---- Lexical escapes: refused before the file system is consulted.
    for dir in [
        "..",
        "../",
        "a/..",
        "a/../..",
        "../elsewhere",
        "../elsewhere/secret.cic",
        "/etc",
        "/",
        "//server/share",
        "C:/Windows",
        "C:\\Windows",
        "a\\b",
        "a:b",
        "a\0b",
    ] {
        let (status, body) = get(addr, &format!("/api/files?dir={}", urlencode(dir))).await;
        assert_eq!(status, 400, "{dir:?}: {body}");
        assert_eq!(body["kind"], "path_not_allowed", "{dir:?}: {body}");
        assert_eq!(body["path"], dir, "{dir:?}: {body}");
        assert!(
            body["message"].as_str().is_some_and(|m| !m.is_empty()),
            "{dir:?}: {body}"
        );
        // A refusal echoes the REQUEST (`path`, and the message quotes
        // it): the client's own words are not a disclosure, so the sweep
        // below skips the requests that named the outside themselves.
        if !dir.contains("elsewhere") {
            bodies.push(body.to_string());
        }
    }

    // ---- Not found: a missing directory, a file, a path THROUGH a file,
    // and names Windows' file systems cannot hold (`a?b`: on Windows the
    // OS refuses the name, os error 123; on Unix nothing by that name is
    // here) — nothing exists at any of them, and 403 `io_error` means
    // "exists but unreadable", so every one is 404 on every OS.
    for (dir, words) in [
        ("nowhere", "no directory"),
        ("sub/p.cic", "not a directory"),
        ("sub/p.cic/deeper", "no directory"),
        ("a?b", "no directory"),
        ("a*b", "no directory"),
        ("a<b", "no directory"),
        ("a>b", "no directory"),
        ("a|b", "no directory"),
        ("a\"b", "no directory"),
    ] {
        let (status, body) = get(addr, &format!("/api/files?dir={}", urlencode(dir))).await;
        assert_eq!(status, 404, "{dir:?}: {body}");
        assert_eq!(body["kind"], "not_found", "{dir:?}: {body}");
        assert_eq!(body["path"], dir);
        assert!(
            body["message"].as_str().unwrap().contains(words),
            "{dir:?}: {body}"
        );
        bodies.push(body.to_string());
    }

    // ---- Links.
    let (status, listing) = get(addr, "/api/files").await;
    assert_eq!(status, 200);
    bodies.push(listing.to_string());
    if links {
        let names: Vec<String> = entries(&listing).into_iter().map(|(n, _)| n).collect();
        assert!(
            names.contains(&"alias".to_owned()),
            "a link staying under the root is a directory of it: {names:?}"
        );
        assert!(
            !names.contains(&"escape".to_owned()),
            "a link leaving the root is not listed: {names:?}"
        );
        assert!(
            !names.contains(&"dangling".to_owned()),
            "a dangling link is not listed: {names:?}"
        );
        // Entering the dangling link: nothing is there.
        let (status, body) = get(addr, "/api/files?dir=dangling").await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["kind"], "not_found", "{body}");
        bodies.push(body.to_string());
        let (status, alias) = get(addr, "/api/files?dir=alias").await;
        assert_eq!(status, 200, "{alias}");
        assert_eq!(
            (alias["dir"].as_str(), alias["parent"].as_str()),
            (Some("alias"), Some("")),
            "the listed dir is the request, not the link's target"
        );
        assert_eq!(
            entries(&alias),
            pairs(&[("inner", "dir"), ("p.cic", "pipeline")])
        );
        let (status, body) = get(addr, "/api/files?dir=escape").await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(body["kind"], "path_not_allowed");
        assert_eq!(body["path"], "escape");
        bodies.push(body.to_string());
        let (status, body) = get(addr, "/api/files?dir=escape/").await;
        assert_eq!(status, 400, "{body}");
        bodies.push(body.to_string());
        // A pipeline reference through the link is refused too (the
        // pipeline routes' own, pre-existing text — not the listing's).
        let (status, body) = get(addr, "/debug/state?pipeline=escape/secret.cic").await;
        assert_eq!(status, 400, "{body}");
    }

    // ---- No LISTING response ever named anything above the root: not
    // the temp dir, not the directory beyond the root, not the file in it
    // — not even the refusal of the link that points there.
    assert!(
        bodies.len() >= 13,
        "the sweep covers every listing response"
    );
    for body in &bodies {
        assert!(
            !body.contains(&temp_name) && !body.contains("elsewhere") && !body.contains("secret"),
            "a response named something above the root: {body}"
        );
    }
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unreadable_directory_is_io_error_and_still_a_directory_of_its_parent() {
    let fx = fixture();
    let locked = fx.root.join("locked");
    std::fs::create_dir(&locked).unwrap();
    let Some(denied) = deny_listing(&locked) else {
        eprintln!(
            "SKIPPING: this OS/user cannot make a directory unreadable to itself here \
             (root on Unix reads everything)"
        );
        return;
    };
    let handle = start(&fx).await;
    let addr = handle.addr;
    let (status, body) = get(addr, "/api/files?dir=locked").await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["kind"], "io_error", "{body}");
    assert_eq!(body["path"], "locked");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .starts_with("reading `locked`:"),
        "{body}"
    );
    // Its parent still lists it: a directory you may not read is still a
    // directory of the root (the refusal comes when it is entered).
    let (status, listing) = get(addr, "/api/files").await;
    assert_eq!(status, 200, "{listing}");
    assert!(
        entries(&listing).contains(&("locked".to_owned(), "dir".to_owned())),
        "{listing}"
    );
    handle.shutdown().await;
    drop(denied);
}

/// The watcher follows the session into a subdirectory of the root, and a
/// `scripts/` that appears AFTER the session opened is watched from its
/// arrival. Two independent proofs of the late watch, neither counting
/// snapshots (see [`wait_for_script_title`]): the watched set `/debug/state`
/// reports — which a re-watch that never ran cannot contain — and a
/// content-only rewrite inside the directory, which only a watch ON it can
/// see, shown by the state it changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pipeline_in_a_subdirectory_reloads_on_external_changes_including_a_late_scripts_dir() {
    let fx = fixture();
    let handle = start(&fx).await;
    let addr = handle.addr;
    let mut socket = join(addr, "sub/p.cic").await;
    let (_, before) = get(addr, "/debug/state?pipeline=sub/p.cic&wait=true").await;
    assert!(before["text"].as_str().unwrap().contains("value=2.0"));
    assert_eq!(
        watched(addr, "sub/p.cic").await,
        ["sub"],
        "the pipeline's own directory is watched, no scripts/ exists yet, the root is not watched"
    );
    assert_eq!(script_title(addr, "sub/p.cic", "triple").await, None);

    // ---- 1. The `.cic` changes under the session (git checkout, an
    // editor): the watch on the pipeline's OWN directory sees it — the
    // root is not watched as a tree.
    std::fs::write(fx.root.join("sub").join("p.cic"), PIPELINE_CHANGED).unwrap();
    let barrier = next_of(&mut socket, "snapshot").await;
    assert_eq!(
        barrier["payload"]["reason"], "external file change",
        "{barrier}"
    );
    let (_, after) = get(addr, "/debug/state?pipeline=sub/p.cic&wait=true").await;
    assert!(
        after["text"].as_str().unwrap().contains("value=3.0"),
        "{}",
        after["text"]
    );

    // ---- 2. A `scripts/` directory appears beside it AFTER the session
    // opened, with a script inside: no watch existed for either event when
    // the session opened — the directory's arrival is seen on the parent's
    // watch, rescans (the script is discovered), and puts the new directory
    // under watch. The re-watch precedes the rescan in the watcher thread's
    // one batch, so once the catalog shows the script the set is settled.
    let scripts = fx.root.join("sub").join("scripts");
    std::fs::create_dir(&scripts).unwrap();
    std::fs::write(scripts.join("helper.py"), SCRIPT).unwrap();
    wait_for_script_title(&mut socket, addr, "sub/p.cic", "triple", "Triple").await;
    assert_eq!(
        watched(addr, "sub/p.cic").await,
        ["sub", "sub/scripts"],
        "the late scripts/ is under watch"
    );

    // ---- 2b. Quiesce the watcher before step 3, without a sleep: step 2's
    // burst may still owe a batch (its directory-modified event landing in
    // a later coalescing window), and that batch's rescan would read
    // whatever step 3 has written by then — a rescan that needs no watch on
    // `scripts/` at all, which is exactly what step 3 must prove. So an
    // ORDERED barrier: a `.cic` rewrite is an event on the same directory
    // handle (`sub`) as every event step 2 raised there, delivered after
    // them; once its text shows, no earlier event of that handle is pending,
    // and the only watch that can see step 3 is the one on `scripts/`.
    std::fs::write(fx.root.join("sub").join("p.cic"), PIPELINE_CHANGED_AGAIN).unwrap();
    wait_for_text(&mut socket, addr, "sub/p.cic", "value=4.0").await;
    assert_eq!(
        script_title(addr, "sub/p.cic", "triple").await.as_deref(),
        Some("Triple"),
        "the barrier changed the text, not the script"
    );

    // ---- 3. A content-only rewrite INSIDE it: no directory entry changes,
    // so the parent's watch cannot see this — only the watch on `scripts/`
    // can; the catalog shows the new title only once a rescan has read it.
    std::fs::write(scripts.join("helper.py"), SCRIPT_EDITED).unwrap();
    wait_for_script_title(&mut socket, addr, "sub/p.cic", "triple", "Quadruple").await;
    handle.shutdown().await;
}

/// The watcher's primary path — the shape of `examples/wall` and
/// `05-script-geometry`: a `scripts/` directory that EXISTS when the session
/// opens is watched from the start (before the session exists), and a
/// content-only edit of a script — which the pipeline's directory watch
/// cannot see — reloads with a rescan.
///
/// The watched-set assertion right after the join is the proof of the
/// at-open path; the edit alone is not. Observed 2026-08-24 with the at-open
/// watch removed: on Windows the PARENT's watch reported `scripts/` modified
/// during the open although no entry of it changed (NTFS flushes a file's
/// directory-entry info when the file is next opened and closed — the
/// session's read of `helper.py`), the re-watch path put `scripts/` under
/// watch, and the edit was seen through that healed watch. The healing is
/// the product working as designed (an idempotent re-watch and one
/// fingerprint); what it cannot fake is the set at the moment of the join.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_scripts_dir_present_at_open_is_watched_from_the_start() {
    let fx = fixture();
    let scripts = fx.root.join("sub").join("scripts");
    std::fs::create_dir(&scripts).unwrap();
    std::fs::write(scripts.join("helper.py"), SCRIPT).unwrap();
    let handle = start(&fx).await;
    let addr = handle.addr;
    let mut socket = join(addr, "sub/p.cic").await;
    assert_eq!(
        watched(addr, "sub/p.cic").await,
        ["sub", "sub/scripts"],
        "both directories are watched as the session opens"
    );
    assert_eq!(
        script_title(addr, "sub/p.cic", "triple").await.as_deref(),
        Some("Triple"),
        "discovered at open"
    );
    std::fs::write(scripts.join("helper.py"), SCRIPT_EDITED).unwrap();
    wait_for_script_title(&mut socket, addr, "sub/p.cic", "triple", "Quadruple").await;
    handle.shutdown().await;
}
