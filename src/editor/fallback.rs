//! `$VISUAL`/`$EDITOR`-terugval en de laatste terugval op `vi` (task.md T9;
//! blueprint.md `editor::fallback`).
//!
//! De waarde van `$EDITOR` (bijvoorbeeld `code --wait`) wordt met [`shell_words::split`]
//! in losse argv-delen gesplitst (task.md T9 "Doet": "`$EDITOR=\"code --wait\"`
//! (gesplitst met `shell-words`)"), **nooit** doorgegeven aan een shell (blueprint.md
//! "Altijd argv, nooit `sh -c`"): een pad met spaties of aanhalingstekens in de naam
//! kan zo geen extra commando injecteren via `$EDITOR`.
//!
//! Alleen editors waarvan bekend is dat ze `+<regel>` als vlag herkennen krijgen die
//! vlag ook (task.md T9 "Doet": "vi/vim/nvim/nano/emacs/micro/kak"); `hx` (Helix)
//! herkent in plaats daarvan `BESTAND:REGEL` als één argument. Een onbekende editor
//! (bijvoorbeeld `code`) krijgt alleen het bestand: een vlag raden die hij niet kent,
//! zou het bestand juist niet openen.

use std::path::Path;

/// Editors die `+<regel>` als los argument vóór het bestand herkennen.
const PLUS_LINE_EDITORS: [&str; 7] = ["vi", "vim", "nvim", "nano", "emacs", "micro", "kak"];

/// Helix: `BESTAND:REGEL` in plaats van een los `+<regel>`-argument.
const COLON_LINE_EDITORS: [&str; 1] = ["hx"];

/// Een kant-en-klaar te starten editoraanroep: nooit via een shell, altijd losse argv
/// (blueprint.md T9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCommand {
    pub program: String,
    pub args: Vec<String>,
}

/// Haalt de kale programmanaam uit een (mogelijk absoluut) pad, voor de
/// bekende-editor-check: `/usr/bin/vim` moet net zo herkend worden als `vim`.
fn program_name(program: &str) -> &str {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
}

/// Bouwt de argv voor `vi +<regel> <bestand>`, de laatste terugval als er geen
/// server, geen `nvim` op PATH en geen `$VISUAL`/`$EDITOR` is (task.md T9 "Doet":
/// "anders `vi`").
pub fn vi_command(file: &str, line: u32) -> EditorCommand {
    EditorCommand {
        program: "vi".to_string(),
        args: vec![format!("+{line}"), file.to_string()],
    }
}

/// Bouwt de argv voor een `$VISUAL`/`$EDITOR`-waarde (task.md T9 "Doet"). `spec` is de
/// ruwe omgevingswaarde, bijvoorbeeld `"code --wait"` of `"nano"`. `Err` als `spec`
/// geen geldige shell-woorden zijn (bijvoorbeeld een ongesloten aanhalingsteken); de
/// aanroeper toont dat als statusmelding in plaats van te crashen.
pub fn build_command(
    spec: &str,
    file: &str,
    line: u32,
) -> Result<EditorCommand, shell_words::ParseError> {
    let mut parts = shell_words::split(spec)?;
    if parts.is_empty() {
        // Lege/witruimte-only waarde: geen programma om te starten, val terug op `vi`
        // in plaats van een leeg `program`-veld door te geven aan `Command`.
        return Ok(vi_command(file, line));
    }
    let program = parts.remove(0);
    let mut args = parts; // bestaande extra vlaggen, bijvoorbeeld "--wait"
    let name = program_name(&program);
    if PLUS_LINE_EDITORS.contains(&name) {
        args.push(format!("+{line}"));
        args.push(file.to_string());
    } else if COLON_LINE_EDITORS.contains(&name) {
        args.push(format!("{file}:{line}"));
    } else {
        args.push(file.to_string());
    }
    Ok(EditorCommand { program, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vi_command_has_the_expected_shape() {
        let cmd = vi_command("/home/u/proj/src/Foo.php", 42);
        assert_eq!(cmd.program, "vi");
        assert_eq!(cmd.args, vec!["+42", "/home/u/proj/src/Foo.php"]);
    }

    #[test]
    fn build_command_for_a_bare_known_editor_adds_the_plus_line_flag() {
        // task.md T9 "Klaar als": "alleen $EDITOR=nano".
        let cmd = build_command("nano", "/home/u/proj/src/Foo.php", 42).expect("geldige spec");
        assert_eq!(cmd.program, "nano");
        assert_eq!(cmd.args, vec!["+42", "/home/u/proj/src/Foo.php"]);
    }

    #[test]
    fn build_command_splits_quoted_flags_with_shell_words() {
        // task.md T9 "Klaar als": `$EDITOR="code --wait"` (gesplitst met `shell-words`).
        let cmd =
            build_command("code --wait", "/home/u/proj/src/Foo.php", 42).expect("geldige spec");
        assert_eq!(cmd.program, "code");
        assert_eq!(
            cmd.args,
            vec!["--wait".to_string(), "/home/u/proj/src/Foo.php".to_string()],
            "onbekende editor: geen +regel geraden, alleen het bestand erbij"
        );
    }

    #[test]
    fn build_command_recognises_known_editors_by_basename_even_with_a_full_path() {
        let cmd =
            build_command("/usr/bin/vim", "/home/u/proj/src/Foo.php", 7).expect("geldige spec");
        assert_eq!(cmd.program, "/usr/bin/vim");
        assert_eq!(cmd.args, vec!["+7", "/home/u/proj/src/Foo.php"]);
    }

    #[test]
    fn build_command_uses_file_colon_line_for_helix() {
        let cmd = build_command("hx", "/home/u/proj/src/Foo.php", 7).expect("geldige spec");
        assert_eq!(cmd.program, "hx");
        assert_eq!(cmd.args, vec!["/home/u/proj/src/Foo.php:7"]);
    }

    #[test]
    fn build_command_rejects_unbalanced_quotes_without_panicking() {
        assert!(build_command("code \"unclosed", "/x.php", 1).is_err());
    }

    #[test]
    fn build_command_on_blank_spec_falls_back_to_vi() {
        assert_eq!(
            build_command("   ", "/x.php", 1).expect("leeg mag geen fout zijn"),
            vi_command("/x.php", 1)
        );
    }
}
