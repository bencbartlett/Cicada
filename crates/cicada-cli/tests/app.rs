//! `cicada app` at the process level (v0.1 wave 4, docs/17 L1): the real
//! binary over a scratch project with `--no-browser` and a stand-in SPA (a
//! `--web-dir` holding an `index.html`). The server comes up, the console's
//! first line carries the URL (token and pipeline included), `/health`
//! answers over that address, `/` serves the SPA — never the server's
//! "API only" page — the console says what `--no-browser` did, and the
//! process dies on kill (Ctrl-C's job here). Without a SPA to open — no
//! `--web-dir` and no embedded build, or a `--web-dir` without an
//! `index.html` — `app` refuses before binding. The path argument goes
//! through `serve`'s resolution — the same function — so a path `serve`
//! refuses, `app` refuses with the same words. Browser discovery itself is
//! the pure function unit-tested in `cicada_cli::app`; nothing here opens a
//! window.
//!
//! The server child is a kill-on-drop guard with BOTH its pipes ours: a
//! failed assertion can never leave an orphan holding cargo's stderr (the
//! first review's stall — 13 minutes on a panic before `kill()`), and every
//! console line is read through a bounded wait, so a line that never comes
//! is a red test, not a hang. The bound is never waited for on the pass
//! path.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DEMO: &str = "# cicada 1\nnums = series(count=3)\n";
const INDEX: &str = "<!doctype html><title>cicada test spa</title>";
/// The most one console line may take before the test fails — a bound on a
/// hang, never a wait the pass path takes.
const LINE_TIMEOUT: Duration = Duration::from_secs(60);

fn cicada() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cicada"))
}

/// A scratch project with `demo.cic` and a stand-in SPA under `dist/`;
/// returns the directory (kept alive by the caller).
fn scratch() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("demo.cic"), DEMO).unwrap();
    std::fs::create_dir(dir.path().join("dist")).unwrap();
    std::fs::write(dir.path().join("dist").join("index.html"), INDEX).unwrap();
    dir
}

/// A running `cicada` child: killed and reaped when dropped, whatever
/// happened before, with its stdout read line by line on a thread and its
/// stderr drained on another — no pipe of cargo's is ever inherited.
struct Server {
    child: Child,
    lines: mpsc::Receiver<String>,
    stderr: Option<std::thread::JoinHandle<String>>,
}

impl Server {
    fn start(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("cicada binary runs");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let err = child.stderr.take().unwrap();
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(err).read_to_string(&mut text);
            text
        });
        Self {
            child,
            lines,
            stderr: Some(stderr),
        }
    }

    /// Kill, reap, and return everything the child wrote to stderr.
    fn finish(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.stderr
            .take()
            .map(|handle| handle.join().unwrap())
            .unwrap_or_default()
    }

    /// The next console line — or a failure naming what was expected, with
    /// the child's stderr when it exited instead. Never an unbounded wait.
    fn line(&mut self, expected: &str) -> String {
        match self.lines.recv_timeout(LINE_TIMEOUT) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let text = self.finish();
                panic!("stdout closed before {expected}; stderr:\n{text}")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let text = self.finish();
                panic!("no {expected} within {LINE_TIMEOUT:?}; stderr:\n{text}")
            }
        }
    }

    /// The child must NOT come up: stdout closes without a line, the exit is
    /// non-zero, and stderr — the refusal — is returned.
    fn expect_refusal(mut self) -> String {
        match self.lines.recv_timeout(LINE_TIMEOUT) {
            Ok(line) => {
                let text = self.finish();
                panic!("the server came up instead of refusing: {line}\nstderr:\n{text}")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let text = self.finish();
                panic!(
                    "neither a refusal nor a console line within {LINE_TIMEOUT:?}; stderr:\n{text}"
                )
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
        let status = self.child.wait().unwrap();
        let text = self.finish();
        assert!(!status.success(), "exit {status} with stderr:\n{text}");
        text
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One plain HTTP/1.1 GET over TCP; the whole response as text.
fn get(addr: &str, target: &str) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn app_without_a_browser_serves_the_spa_and_prints_the_url() {
    let dir = scratch();
    let mut server = Server::start(
        cicada()
            .args([
                "app",
                "--no-browser",
                "--port",
                "0",
                "--token",
                "t",
                "--threads",
                "2",
                "--web-dir",
            ])
            .arg(dir.path().join("dist"))
            .arg("--cache-dir")
            .arg(dir.path().join("cache"))
            .arg(dir.path().join("demo.cic")),
    );

    // `cicada app — http://127.0.0.1:<port>/?token=t&pipeline=demo.cic`
    let header = server.line("the URL line");
    assert!(
        header.starts_with("cicada app — http://127.0.0.1:"),
        "{header}"
    );
    let url = header.split(" — ").nth(1).unwrap().to_owned();
    assert!(url.ends_with("/?token=t&pipeline=demo.cic"), "{url}");
    // `  root <dir> — demo.cic open`: the root model (wave 4 O1) names what is
    // served and what opened, right under the URL.
    let root_line = server.line("the root line");
    assert!(
        root_line.trim_start().starts_with("root ") && root_line.ends_with("— demo.cic open"),
        "{root_line}"
    );
    let second = server.line("the Ctrl-C line");
    assert!(second.contains("Ctrl-C stops the server"), "{second}");
    let third = server.line("the --no-browser line");
    assert!(
        third.contains("--no-browser") && third.contains("open the URL above"),
        "{third}"
    );

    // The printed address serves: /health over plain TCP …
    let addr = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let health = get(&addr, "/health?token=t");
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.ends_with("ok"), "{health}");
    // … and `/` is the SPA the window would load — never the API-only page.
    let root = get(&addr, "/?token=t");
    assert!(root.starts_with("HTTP/1.1 200"), "{root}");
    assert!(root.contains(INDEX), "{root}");
    assert!(!root.contains("API only"), "{root}");

    server.finish();
}

#[test]
fn app_refuses_before_binding_when_it_has_no_spa_to_open() {
    let dir = scratch();
    // A `--web-dir` without an index.html: refused whatever the build — the
    // server would serve that directory, 404 where the app should be.
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let text = Server::start(
        cicada()
            .args(["app", "--no-browser", "--port", "0", "--web-dir"])
            .arg(&empty)
            .arg(dir.path().join("demo.cic")),
    )
    .expect_refusal();
    assert!(
        text.contains("has no index.html") && text.contains("npm run build"),
        "{text}"
    );

    // No `--web-dir`: only a build with the SPA embedded has anything to
    // open. This test binary shares the features of the `cicada` it drives.
    let bare = Server::start(
        cicada()
            .args(["app", "--no-browser", "--port", "0", "--cache-dir"])
            .arg(dir.path().join("cache"))
            .arg(dir.path().join("demo.cic")),
    );
    if cfg!(feature = "embed") {
        let mut bare = bare;
        let header = bare.line("the URL line");
        assert!(header.starts_with("cicada app — http://"), "{header}");
        bare.finish();
    } else {
        let text = bare.expect_refusal();
        for needle in [
            "nothing to open",
            "--web-dir web/dist",
            "--features embed",
            "cicada serve",
        ] {
            assert!(text.contains(needle), "{text}");
        }
    }
}

/// `serve` / `app` on a path, with the stand-in SPA so `app` gets as far as
/// the path: the refusal's words.
fn refusal(subcommand: &str, path: &Path, dist: &Path) -> String {
    Server::start(
        cicada()
            .arg(subcommand)
            .arg(path)
            .args(["--port", "0", "--web-dir"])
            .arg(dist),
    )
    .expect_refusal()
}

#[test]
fn app_resolves_its_path_exactly_as_serve_does() {
    let dir = scratch();
    let dist = dir.path().join("dist");
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "not a pipeline").unwrap();
    for (path, expected) in [
        (dir.path().join("nowhere"), "resolving"),
        (notes, "neither a directory nor a .cic file"),
    ] {
        let serve = refusal("serve", &path, &dist);
        let app = refusal("app", &path, &dist);
        assert!(serve.contains(expected), "{serve}");
        // Same function, same words — the proof `app` duplicates nothing.
        assert_eq!(serve, app);
    }
}

#[test]
fn help_carries_serve_flags_and_the_browser_switch() {
    let output = cicada().args(["app", "--help"]).output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for flag in [
        "--no-browser",
        "--host",
        "--port",
        "--token",
        "--cache-dir",
        "--threads",
        "--web-dir",
    ] {
        assert!(text.contains(flag), "`app --help` lacks {flag}:\n{text}");
    }
    assert!(
        text.contains("embed") && text.contains("API-only"),
        "`app --help` says what the window needs:\n{text}"
    );
    let serve = cicada().args(["serve", "--help"]).output().unwrap();
    let serve_text = String::from_utf8_lossy(&serve.stdout);
    assert!(
        !serve_text.contains("--no-browser"),
        "`serve` has no browser to switch off:\n{serve_text}"
    );
}
