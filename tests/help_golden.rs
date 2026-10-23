//! `--help` als golden test (task.md T15 "Doet"): de `--help`-uitvoer is wat de
//! README als flaglijst overneemt, dus een ongemerkte wijziging daaraan (een nieuwe
//! flag, een andere omschrijving) moet deze test laten falen in plaats van de README
//! stil te laten verlopen.

use assert_cmd::Command;
use std::fs;
use std::path::Path;

fn golden_path(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name),
    )
    .expect("golden-bestand lezen")
}

#[test]
fn help_output_matches_golden() {
    let output = Command::cargo_bin("symlog")
        .expect("binary vinden")
        .arg("--help")
        .output()
        .expect("symlog --help draait");

    assert!(output.status.success());
    let actual = String::from_utf8(output.stdout).expect("utf8");
    assert_eq!(actual, golden_path("help.txt"));
}
