//! End-to-end `cicada run` (stage 3): the real binary, a real `.cic` file,
//! the real registry, a real disk store — the repo's first full surface.
//! Cold solve → warm rerun computing nothing; diagnostics gate scoped to
//! the target cone; stable `--hashes` output as the verification currency
//! (doc 14).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::{Command, Output};

const DEMO: &str = "\
# cicada 1
nums = series(count=5)
bumped = add(a=each(nums), b=10)
span = construct_domain(start=2, end=7)
lo, hi = deconstruct_domain(domain=span)
twice = 2 * lo
";

fn cicada(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cicada"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("cicada binary runs")
}

fn write_demo(dir: &Path, source: &str) {
    std::fs::write(dir.join("demo.cic"), source).unwrap();
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn hash_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| line.contains('\t'))
        .map(str::to_owned)
        .collect()
}

#[test]
fn cold_solve_then_warm_rerun_computes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write_demo(dir.path(), DEMO);
    let args = [
        "run",
        "demo.cic",
        "--cache-dir",
        "cache",
        "--hashes",
        "--time",
    ];

    let cold = cicada(dir.path(), &args);
    assert!(
        cold.status.success(),
        "cold run failed:\n{}\n{}",
        stdout(&cold),
        stderr(&cold)
    );
    let cold_out = stdout(&cold);
    let cold_hashes = hash_lines(&cold_out);
    assert_eq!(
        cold_hashes.len(),
        3,
        "three leaves (bumped, hi, twice): {cold_out}"
    );
    assert!(
        cold_out.contains("5 computed, 0 from cache"),
        "cold solve computes all five nodes: {cold_out}"
    );

    let warm = cicada(dir.path(), &args);
    assert!(warm.status.success(), "warm run failed: {}", stderr(&warm));
    let warm_out = stdout(&warm);
    assert_eq!(
        hash_lines(&warm_out),
        cold_hashes,
        "hashes are byte-stable across runs"
    );
    assert!(
        warm_out.contains("0 computed, 5 from cache"),
        "warm rerun computes NOTHING: {warm_out}"
    );

    // The cache must live where we pointed it, and never appear in-repo
    // by accident.
    assert!(dir.path().join("cache").join("memo.log").exists());
}

#[test]
fn values_render_for_humans() {
    let dir = tempfile::tempdir().unwrap();
    write_demo(dir.path(), DEMO);
    let output = cicada(
        dir.path(),
        &[
            "run",
            "demo.cic",
            "--cache-dir",
            "cache",
            "--node",
            "twice",
            "--node",
            "bumped",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("twice.out = 4"), "{text}");
    assert!(
        text.contains("bumped.out = [10, 11, 12, 13, 14] ×5"),
        "{text}"
    );
}

#[test]
fn multi_output_target_prints_every_port() {
    let dir = tempfile::tempdir().unwrap();
    // Bind the whole multi-output node (no unpack) and target it.
    write_demo(
        dir.path(),
        "# cicada 1\nspan = construct_domain(start=2, end=7)\nd = deconstruct_domain(domain=span)\n",
    );
    let output = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "d"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("d.start = 2"), "{text}");
    assert!(text.contains("d.end = 7"), "{text}");
}

#[test]
fn diagnostics_in_the_cone_refuse_with_doc11_json() {
    let dir = tempfile::tempdir().unwrap();
    write_demo(dir.path(), "# cicada 1\nbad = add(a=1.0)\n");
    let output = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "bad"],
    );
    assert!(!output.status.success(), "missing kwarg must refuse");
    let err = stderr(&output);
    assert!(
        err.contains("missing_kwarg"),
        "doc-11 JSON on stderr: {err}"
    );
    assert!(err.contains("\"node\": \"bad\""), "{err}");
}

#[test]
fn the_gate_is_scoped_to_the_target_cone() {
    let dir = tempfile::tempdir().unwrap();
    // A broken statement UNRELATED to the requested cone: the run
    // proceeds and warns (docs/12: a red statement reds one node, not the
    // file).
    let source = "# cicada 1\nnums = series(count=3)\nbroken = add(a=\n";
    write_demo(dir.path(), source);

    let scoped = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "nums"],
    );
    assert!(
        scoped.status.success(),
        "an unrelated broken statement must not block: {}",
        stderr(&scoped)
    );
    assert!(
        stderr(&scoped).contains("outside the requested cone"),
        "but it IS warned about: {}",
        stderr(&scoped)
    );

    // A broken statement binds nothing, so it can never be a target —
    // asking for it by name is a loud refusal…
    let direct = cicada(
        dir.path(),
        &[
            "run",
            "demo.cic",
            "--cache-dir",
            "cache",
            "--node",
            "broken",
        ],
    );
    assert!(!direct.status.success());
    assert!(
        stderr(&direct).contains("no binding named `broken`"),
        "{}",
        stderr(&direct)
    );

    // …and a healthy statement that REFERENCES it is in the blast radius:
    // the checker's "failed to parse — fix that statement first" lands on
    // the consumer, and the gate refuses.
    write_demo(
        dir.path(),
        "# cicada 1\nnums = series(count=3)\nbroken = add(a=\nuses = add(a=broken, b=1)\n",
    );
    let downstream = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "uses"],
    );
    assert!(!downstream.status.success());
    assert!(
        stderr(&downstream).contains("failed to parse"),
        "{}",
        stderr(&downstream)
    );
}

// ------------------------------------- review regressions (stage 3) --

// Literal bindings never pass through the store — the run must print the
// value in hand. An unwired param (the normal state of an in-progress
// design) used to fail the DEFAULT run with "value … is not in the store".
#[test]
fn literal_targets_print_without_the_store() {
    let dir = tempfile::tempdir().unwrap();
    write_demo(
        dir.path(),
        "# cicada 1\nunused = 42\nnums = series(count=3)\n",
    );
    // Default run (both leaves, one of them a bare literal).
    let output = cicada(dir.path(), &["run", "demo.cic", "--cache-dir", "cache"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("unused.out = 42"), "{text}");
    assert!(text.contains("nums.out = [0, 1, 2] ×3"), "{text}");
    // Direct literal target, human and hash modes.
    let direct = cicada(
        dir.path(),
        &[
            "run",
            "demo.cic",
            "--cache-dir",
            "cache",
            "--node",
            "unused",
        ],
    );
    assert!(direct.status.success(), "{}", stderr(&direct));
    assert!(stdout(&direct).contains("unused.out = 42"));
}

// The literal text `9007199254740993` (2^53+1) parses to exactly 2^53, so
// an f64 at the boundary may be a silently drifted digit — both it AND
// the boundary itself must refuse; below the boundary stays exact.
#[test]
fn integer_literals_at_the_exact_boundary_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    for bad in ["9007199254740993", "9007199254740992", "-9007199254740993"] {
        write_demo(dir.path(), &format!("# cicada 1\nx = {bad}\n"));
        let output = cicada(
            dir.path(),
            &["run", "demo.cic", "--cache-dir", "cache", "--node", "x"],
        );
        assert!(!output.status.success(), "{bad} must refuse");
        assert!(
            stderr(&output).contains("outside the exact range"),
            "{}",
            stderr(&output)
        );
    }
    write_demo(dir.path(), "# cicada 1\nx = 9007199254740991\n");
    let ok = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "x"],
    );
    assert!(ok.status.success(), "{}", stderr(&ok));
    assert!(stdout(&ok).contains("x.out = 9007199254740991"));
}

// i64::MAX is reachable through integer-mode expression arithmetic
// (checked i64), and its widening to Number is off by one — the saturating
// f64→i64 cast used to masquerade it as exact. It must be a red node.
#[test]
fn i64_max_refuses_to_widen_to_number() {
    let dir = tempfile::tempdir().unwrap();
    // 49 × 73 × 127 × 337 × 92737 × 649657 = 2^63 − 1, every factor < 2^53.
    write_demo(
        dir.path(),
        "# cicada 1\nm = 49 * 73 * 127 * 337 * 92737 * 649657\nf = m / 1\n",
    );
    // The Integer itself is fine…
    let integer = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "m"],
    );
    assert!(integer.status.success(), "{}", stderr(&integer));
    assert!(stdout(&integer).contains("m.out = 9223372036854775807"));
    // …but widening it into a float expression must refuse loudly, never
    // print 2^63.
    let widened = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "f"],
    );
    assert!(!widened.status.success(), "lossy widening must be red");
    assert!(
        stderr(&widened).contains("does not convert exactly"),
        "{}",
        stderr(&widened)
    );
}

// `x = x + 1` is a length-1 cycle: a doc-11 Cycle diagnostic, never a bare
// internal "not lowerable" string.
#[test]
fn direct_self_reference_is_a_cycle_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    write_demo(dir.path(), "# cicada 1\nx = x + 1\n");
    let output = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "x"],
    );
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("\"cycle\""), "doc-11 JSON: {err}");
    assert!(err.contains("x → x"), "{err}");
    assert!(!err.contains("not lowerable"), "{err}");
}

#[test]
fn red_nodes_exit_nonzero_with_element_ids() {
    let dir = tempfile::tempdir().unwrap();
    // series panics on count < 0 → the scheduler turns it into a red node
    // (the sequences.rs doc comment's promise, kept at stage 3).
    write_demo(dir.path(), "# cicada 1\nnums = series(count=-1)\n");
    let output = cicada(
        dir.path(),
        &["run", "demo.cic", "--cache-dir", "cache", "--node", "nums"],
    );
    assert!(!output.status.success(), "a red node fails the run");
    let err = stderr(&output);
    assert!(err.contains("red: `nums`"), "{err}");
    assert!(
        err.contains("count must be >= 0"),
        "panic message surfaces: {err}"
    );
}
