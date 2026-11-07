//! Golden-test voor `examples/`: elk invoerbestand draait door de echte binary
//! (`--plain --no-color`) en moet exact het bijbehorende `.expected`-bestand geven.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn check(input: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let output = Command::cargo_bin("symlog")
        .expect("binary vinden")
        .arg(root.join(input))
        .args(["--plain", "--no-color"])
        .output()
        .expect("symlog draait");
    assert!(output.status.success());
    let expected = fs::read_to_string(root.join(format!("{input}.expected"))).expect("expected");
    assert_eq!(String::from_utf8(output.stdout).expect("utf8"), expected);
}

#[test]
fn symfony_prod_example_matches_expected() {
    check("symfony-prod.log");
}

#[test]
fn api_json_example_matches_expected() {
    check("api-json.ndjson");
}

#[test]
fn examples_never_leak_their_demo_secrets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    for name in ["symfony-prod.log.expected", "api-json.ndjson.expected"] {
        let text = fs::read_to_string(root.join(name)).expect("expected");
        assert!(!text.contains("hunter2") && !text.contains("demo-not-a-real-key"));
    }
}
