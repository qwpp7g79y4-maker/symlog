//! Draaiende Neovim-server: socketherkenning en de twee remote-aanroepen (task.md T9;
//! blueprint.md `editor::nvim`).
//!
//! Een draaiende server wordt alleen *herkend*, nooit geprobeerd te bereiken vóórdat
//! het echte commando loopt: er is geen aparte "is deze socket open?"-stap. Dat houdt
//! de herkenning puur en deterministisch testbaar (task.md T9 "Klaar als": exacte argv
//! voor "server via `$NVIM`" en "server via `--server`"); of de server daadwerkelijk
//! antwoordt, blijkt uit de exitcode van de twee aanroepen die
//! [`crate::editor::execute`] doet. Antwoordt de server niet (stale socket, Neovim
//! dicht), dan faalt de aanroep en komt dat terug als [`crate::editor::JumpOutcome::Failed`]
//! (blueprint.md "Foutafhandeling": "Editor faalt ... → statusbalk, symlog blijft
//! draaien"), niet als een stille val-door naar de volgende stap: de volgorde in
//! task.md T9 "Doet" is een beslisboom op *configuratie* (is er een serverreferentie?),
//! geen retry-keten op *runtime-gedrag*.
//!
//! Twee losse `nvim`-aanroepen in plaats van één (task.md T9 "Doet"): eerst het
//! bestand openen (`--remote FILE`), dan de cursor verplaatsen (`--remote-expr`). Het
//! alternatief uit task.md's open punt 2, `--remote-send ':<line><CR>'`, stuurt
//! toetsaanslagen de lopende Neovim-sessie in en stoort dus als de gebruiker daar op
//! dat moment in insert- of terminal-mode zit; `--remote-expr` met `execute(':line')`
//! roept een Vim-functie aan buiten de toetsenbordinvoer om en heeft dat probleem niet
//! (de voorkeur die task.md's open punt 2 en blueprint.md's datastroomdiagram beide al
//! aangeven).

/// Bepaalt welke socketreferentie geldt, in task.md T9 "Doet"'s volgorde: een
/// expliciete `--server`-vlag wint van `config.toml`'s `server`-key, die wint van de
/// `$NVIM`-omgevingsvariabele (die Neovim zelf zet binnen een `:terminal`).
pub fn resolve_server<'a>(
    flag: Option<&'a str>,
    config: Option<&'a str>,
    env_nvim: Option<&'a str>,
) -> Option<&'a str> {
    [flag, config, env_nvim]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
}

/// Argv voor de eerste remote-aanroep: `FILE` openen op de server (zonder de cursor
/// nog te verplaatsen).
pub fn remote_open_argv(socket: &str, file: &str) -> Vec<String> {
    vec![
        "--server".to_string(),
        socket.to_string(),
        "--remote".to_string(),
        file.to_string(),
    ]
}

/// Argv voor de tweede remote-aanroep: de cursor naar `line` zetten in het net
/// geopende bestand (zie de moduledoc hierboven voor de keuze voor `--remote-expr`
/// boven `--remote-send`).
pub fn remote_goto_line_argv(socket: &str, line: u32) -> Vec<String> {
    vec![
        "--server".to_string(),
        socket.to_string(),
        "--remote-expr".to_string(),
        format!("execute(':{line}')"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_server_prefers_the_flag_over_config_and_env() {
        assert_eq!(
            resolve_server(
                Some("/tmp/flag.sock"),
                Some("/tmp/config.sock"),
                Some("/tmp/env.sock")
            ),
            Some("/tmp/flag.sock")
        );
    }

    #[test]
    fn resolve_server_falls_back_to_config_then_env() {
        assert_eq!(
            resolve_server(None, Some("/tmp/config.sock"), Some("/tmp/env.sock")),
            Some("/tmp/config.sock")
        );
        assert_eq!(
            resolve_server(None, None, Some("/tmp/env.sock")),
            Some("/tmp/env.sock")
        );
    }

    #[test]
    fn resolve_server_is_none_when_nothing_is_set() {
        assert_eq!(resolve_server(None, None, None), None);
    }

    #[test]
    fn resolve_server_skips_an_empty_env_value() {
        // Een lege $NVIM (gezet maar leeg) telt niet als server, net als onbestaand.
        assert_eq!(resolve_server(None, None, Some("")), None);
    }

    #[test]
    fn remote_open_argv_has_the_expected_shape() {
        assert_eq!(
            remote_open_argv("/tmp/nvim.sock", "/home/u/proj/src/Foo.php"),
            vec![
                "--server",
                "/tmp/nvim.sock",
                "--remote",
                "/home/u/proj/src/Foo.php"
            ]
        );
    }

    #[test]
    fn remote_goto_line_argv_uses_remote_expr_execute() {
        assert_eq!(
            remote_goto_line_argv("/tmp/nvim.sock", 42),
            vec![
                "--server",
                "/tmp/nvim.sock",
                "--remote-expr",
                "execute(':42')"
            ]
        );
    }
}
