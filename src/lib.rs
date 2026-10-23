//! symlog: terminal-logviewer voor Symfony/Monolog-logs (read-only, geen netwerk).
//!
//! T1 legde het repo-skelet vast, T2 voegde [`reader`] toe (offset-index en lazy
//! records). T3 voegt [`model`] (gedeelde recordtypes) en [`parse`] (de
//! LineFormatter-parser) toe, zie `blueprint.md`. T4/T5/T6 breidden [`parse`] uit met
//! multiline-records, de JsonFormatter en de Raw-terugval. T7 voegt [`time`]
//! (tijdzone-weergave) en [`filter`] (kanaal/niveau/zoeken) toe. T8 voegt [`tui`] toe
//! (ratatui + crossterm, vim-toetsen). T9 voegt [`editor`] toe (Neovim-sprong,
//! `$EDITOR`/`vi`-terugval, padmapping). T10 voegt [`follow`] toe (`tail -f`,
//! rotatie en afkappen). T11 voegt [`startup`] toe (leeg en ontbrekend bestand). T13
//! voegt [`plain`] toe (niet-TTY-uitvoer) en verbindt de CLI (`src/main.rs`) voor het
//! eerst met de volledige leespijplijn: modus kiezen tussen TUI en plain op basis van
//! of stdout een tty is.

pub mod editor;
pub mod filter;
pub mod follow;
pub mod model;
pub mod parse;
pub mod plain;
pub mod reader;
pub mod startup;
pub mod time;
pub mod tui;

/// Placeholder tot er een CLI-laag is die dit toont; bestaat zodat de crate een
/// lib-test heeft om CI tegen te draaien (T1).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }
}
