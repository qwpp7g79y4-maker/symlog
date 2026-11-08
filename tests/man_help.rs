//! De man-pagina `man/symlog.1` hoort bij `--help`: elke lange optie uit de hulptekst
//! staat in de man-pagina en andersom. `--completions` geeft een script voor bash en zsh.

use assert_cmd::Command;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn long_options(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-'));
        let Some(rest) = token.strip_prefix("--") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if !name.is_empty() {
            found.insert(format!("--{name}"));
        }
    }
    found
}

fn symlog(args: &[&str]) -> std::process::Output {
    Command::cargo_bin("symlog")
        .expect("binary vinden")
        .args(args)
        .output()
        .expect("symlog draait")
}

#[test]
fn man_page_lists_the_same_long_options_as_help() {
    let help = String::from_utf8(symlog(&["--help"]).stdout).expect("utf8");
    let man = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("man/symlog.1"))
        .expect("man/symlog.1 lezen");
    for heading in [".SH NAME", ".SH SYNOPSIS", ".SH OPTIONS", ".SH EXIT STATUS"] {
        assert!(man.contains(heading), "man-pagina mist {heading}");
    }
    let help_opts = long_options(&help);
    let man_opts = long_options(&man);
    assert_eq!(help_opts, man_opts, "help: {help}");
}

#[test]
fn completions_print_a_script_without_a_path() {
    for (shell, marker) in [("bash", "complete -F _symlog"), ("zsh", "#compdef symlog")] {
        let out = symlog(&["--completions", shell]);
        assert_eq!(out.status.code(), Some(0), "{shell}");
        let text = String::from_utf8(out.stdout).expect("utf8");
        assert!(text.contains(marker), "{shell}: {text}");
        assert!(text.contains("--no-redact"), "{shell}");
    }
    let bad = symlog(&["--completions", "fish"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(bad.stdout.is_empty());
    let none = symlog(&[]);
    assert_eq!(none.status.code(), Some(2));
}
