//! `cicada app` at the process level (v0.1 wave 4, docs/17 L1): the real
//! binary over a scratch project with `--no-browser`. The server comes up,
//! the console's first line carries the URL (token and pipeline included),
//! `/health` answers over that address, the console says what
//! `--no-browser` did, and the process dies on kill (Ctrl-C's job here).
//! The path argument goes through `serve`'s resolution — the same function
//! — so a path `serve` refuses, `app` refuses with the same words.
//! Browser discovery itself is the pure function unit-tested in
//! `cicada_cli::app`; nothing here opens a window.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const DEMO: &str = "# cicada 1\nnums = series(count=3)\n";

fn cicada() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cicada"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn app_without_a_browser_serves_and_prints_the_url() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("demo.cic"), DEMO).unwrap();
    let mut child = cicada()
        .args([
            "app",
            "--no-browser",
            "--port",
            "0",
            "--token",
            "t",
            "--threads",
            "2",
            "--cache-dir",
        ])
        .arg(dir.path().join("cache"))
        .arg(dir.path().join("demo.cic"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("cicada binary runs");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    // `cicada app — http://127.0.0.1:<port>/?token=t&pipeline=demo.cic`
    let header = lines
        .next()
        .expect("the server prints its URL before anything else")
        .unwrap();
    assert!(
        header.starts_with("cicada app — http://127.0.0.1:"),
        "{header}"
    );
    let url = header.split(" — ").nth(1).unwrap();
    assert!(url.ends_with("/?token=t&pipeline=demo.cic"), "{url}");
    let second = lines.next().unwrap().unwrap();
    assert!(second.contains("Ctrl-C stops the server"), "{second}");
    let third = lines.next().unwrap().unwrap();
    assert!(
        third.contains("--no-browser") && third.contains("open the URL above"),
        "{third}"
    );

    // The printed address serves: /health over plain TCP.
    let addr = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut stream = TcpStream::connect(&addr).unwrap();
    write!(
        stream,
        "GET /health?token=t HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("ok"), "{response}");

    child.kill().unwrap();
    child.wait().unwrap();
}

fn refusal(subcommand: &str, path: &Path) -> Output {
    cicada()
        .arg(subcommand)
        .arg(path)
        .args(["--port", "0"])
        .output()
        .expect("cicada binary runs")
}

#[test]
fn app_resolves_its_path_exactly_as_serve_does() {
    let dir = tempfile::tempdir().unwrap();
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "not a pipeline").unwrap();
    for (path, expected) in [
        (dir.path().join("nowhere"), "resolving"),
        (notes, "neither a directory nor a .cic file"),
    ] {
        let serve = refusal("serve", &path);
        let app = refusal("app", &path);
        assert!(!serve.status.success(), "serve accepted {}", path.display());
        assert!(!app.status.success(), "app accepted {}", path.display());
        assert!(stderr(&serve).contains(expected), "{}", stderr(&serve));
        // Same function, same words — the proof `app` duplicates nothing.
        assert_eq!(stderr(&serve), stderr(&app));
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
    let serve = cicada().args(["serve", "--help"]).output().unwrap();
    let serve_text = String::from_utf8_lossy(&serve.stdout);
    assert!(
        !serve_text.contains("--no-browser"),
        "`serve` has no browser to switch off:\n{serve_text}"
    );
}
