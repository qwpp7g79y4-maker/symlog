//! Runs the commands of the "Aan de slag in 5 minuten" section of README.md and
//! README.nl.md literally: every `symlog ...` line in the section's `sh` block is
//! executed against the repository fixtures in plain mode.

use assert_cmd::Command;
use std::path::Path;

fn quickstart_commands(readme: &str) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(readme)).unwrap();
    let section = text
        .split("## Aan de slag in 5 minuten")
        .nth(1)
        .unwrap_or_else(|| panic!("{readme}: section missing"));
    let section = section.split("\n## ").next().unwrap();
    let mut in_block = false;
    let mut out = Vec::new();
    for line in section.lines() {
        if line.starts_with("```") {
            in_block = !in_block;
        } else if in_block && !line.trim().is_empty() {
            out.push(line.trim().to_string());
        }
    }
    out
}

fn run_all(readme: &str) {
    let cmds = quickstart_commands(readme);
    assert!(cmds.len() >= 3, "{readme}: too few commands");
    for cmd in cmds {
        let mut words = cmd.split_whitespace();
        assert_eq!(
            words.next(),
            Some("symlog"),
            "{readme}: not a symlog line: {cmd}"
        );
        let args: Vec<&str> = words.collect();
        assert!(args.contains(&"--plain"), "{cmd}: plain mode required");
        let out = Command::cargo_bin("symlog")
            .unwrap()
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{cmd}");
        assert!(!out.stdout.is_empty(), "{cmd}: no output");
    }
}

#[test]
fn quickstart_commands_in_readme_run() {
    run_all("README.md");
}

#[test]
fn quickstart_commands_in_readme_nl_run() {
    run_all("README.nl.md");
}
