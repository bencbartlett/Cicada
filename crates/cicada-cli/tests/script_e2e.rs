//! End-to-end script-node pipeline through the real binary (doc 15 stage
//! 4): scripts/*.py next to the pipeline join the catalog, solve, cache
//! by SOURCE hash, and recompute when the source changes. Pure-stdlib
//! Python fixture — CI needs an interpreter, no packages.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

const PIPELINE: &str = "# cicada 1\n\
                        base = slider(value=3.0, min=0.0, max=10.0)\n\
                        tripled = triple_up(x=base)\n";

const SCRIPT: &str = "import cicada\n\
                      @cicada.node(title=\"Triple Up\", description=\"x times three.\")\n\
                      def triple_up(x: \"Number\") -> \"Number\":\n    return x * 3.0\n";

fn run(project: &Path, cache: &Path, extra: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_cicada"))
        .arg("run")
        .arg(project.join("pipeline.cic"))
        .arg("--cache-dir")
        .arg(cache)
        .args(extra)
        .output()
        .expect("binary runs");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

#[test]
fn script_node_solves_caches_and_recomputes_on_source_change() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("pipeline.cic"), PIPELINE).unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    let script_path = project.path().join("scripts").join("triple.py");
    std::fs::write(&script_path, SCRIPT).unwrap();

    // Cold: the script node computes.
    let (stdout, stderr, ok) = run(project.path(), cache.path(), &["--time"]);
    assert!(ok, "cold run failed:\n{stderr}");
    assert!(stdout.contains("tripled.out = 9"), "{stdout}");
    assert!(stdout.contains("time: tripled"), "computed cold: {stdout}");

    // Warm: pure script nodes hit the memo (source hash unchanged).
    let (stdout, stderr, ok) = run(project.path(), cache.path(), &["--time"]);
    assert!(ok, "warm run failed:\n{stderr}");
    assert!(
        !stdout.contains("time: tripled"),
        "warm run must cache-hit the script node: {stdout}"
    );

    // Edit the source: the body hash changes, the node recomputes.
    std::fs::write(&script_path, SCRIPT.replace("x * 3.0", "x * 4.0")).unwrap();
    let (stdout, stderr, ok) = run(project.path(), cache.path(), &["--time"]);
    assert!(ok, "edited run failed:\n{stderr}");
    assert!(stdout.contains("tripled.out = 12"), "{stdout}");
    assert!(
        stdout.contains("time: tripled"),
        "edited source must recompute: {stdout}"
    );
}

#[test]
fn script_python_error_reds_the_node_with_the_traceback() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("pipeline.cic"), PIPELINE).unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    std::fs::write(
        project.path().join("scripts").join("triple.py"),
        SCRIPT.replace("return x * 3.0", "raise ValueError(\"kaput\")"),
    )
    .unwrap();
    let (_stdout, stderr, ok) = run(project.path(), cache.path(), &[]);
    assert!(!ok, "a raising script must fail the run");
    assert!(
        stderr.contains("red: `tripled`") && stderr.contains("kaput"),
        "red node carries the Python error:\n{stderr}"
    );
}

// Regression (adversarial review, stage 4): a script whose RETURN lies
// about its declared annotation must red AT THE BOUNDARY, not surface as
// a marshalling error three nodes downstream.
#[test]
fn output_type_lie_reds_at_the_boundary() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("pipeline.cic"), PIPELINE).unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    std::fs::write(
        project.path().join("scripts").join("triple.py"),
        SCRIPT.replace("return x * 3.0", "return \"not a number\""),
    )
    .unwrap();
    let (_stdout, stderr, ok) = run(project.path(), cache.path(), &[]);
    assert!(!ok, "a type-lying script must fail the run");
    assert!(
        stderr.contains("declared `Number`") && stderr.contains("a Text"),
        "boundary validation names the mismatch:\n{stderr}"
    );
}

#[test]
fn stdlib_collision_is_a_loud_refusal() {
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("pipeline.cic"), PIPELINE).unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    std::fs::write(
        project.path().join("scripts").join("clash.py"),
        SCRIPT.replace("triple_up", "slider"),
    )
    .unwrap();
    let (_stdout, stderr, ok) = run(project.path(), cache.path(), &[]);
    assert!(!ok);
    assert!(
        stderr.contains("collides with a stdlib node"),
        "collision named:\n{stderr}"
    );
}

// Regression (adversarial review, v0.1 C1): the checker's "`compact`
// removes the holes" advice must be FOLLOWABLE. A script node is the one
// hole producer reachable from a pipeline (`-> "[Number?]"`); compacting
// its output must type `[Number]` so a present-wanting port downstream is
// green — it once typed `[Number?]` (the `?` rode through `E`) and the
// advice fired again on compact's own output.
#[test]
fn compact_makes_a_script_nodes_holes_present_for_downstream_ports() {
    const HOLEY: &str = "import cicada
@cicada.node(title=\"Holey\", description=\"every other slot absent.\")
def holey(count: \"Integer\") -> \"[Number?]\":
    return [float(i) if i % 2 == 0 else None for i in range(count)]
";
    let project = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("pipeline.cic"),
        "# cicada 1\n\
         h = holey(count=5)\n\
         present, sources = compact(list=h)\n\
         bumped = add(a=each(present), b=1.0)\n\
         n = length(list=bumped)\n\
         total, running = mass_addition(list=bumped)\n",
    )
    .unwrap();
    std::fs::create_dir(project.path().join("scripts")).unwrap();
    std::fs::write(project.path().join("scripts").join("holey.py"), HOLEY).unwrap();
    let (stdout, stderr, ok) = run(project.path(), cache.path(), &[]);
    assert!(
        ok,
        "compact's output must fit `add`'s present-wanting port:\n{stderr}"
    );
    // [0, None, 2, None, 4] → compact → [0, 2, 4] → +1 → [1, 3, 5].
    assert!(stdout.contains("n.out = 3"), "{stdout}");
    assert!(stdout.contains("total.result = 9"), "{stdout}");
}
