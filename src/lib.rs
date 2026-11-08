#![warn(clippy::unwrap_used)]
// De testharness compileert deze crate met `cfg(test)`; unwrap hoort daar wel.
#![cfg_attr(test, allow(clippy::unwrap_used))]

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
pub mod redact;
pub mod startup;
pub mod time;
pub mod tui;

/// Crate-versie uit `Cargo.toml`. `--version` print deze tekst. Een debug-build
/// zet er in de binary de git-sha achter; deze functie blijft de crate-versie.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(version(), "1.0.0");
    }
}
