//! symlog: terminal-logviewer voor Symfony/Monolog-logs (read-only, geen netwerk).
//!
//! T1 legde het repo-skelet vast, T2 voegde [`reader`] toe (offset-index en lazy
//! records). T3 voegt [`model`] (gedeelde recordtypes) en [`parse`] (de
//! LineFormatter-parser) toe, zie `blueprint.md`. T4/T5/T6 breidden [`parse`] uit met
//! multiline-records, de JsonFormatter en de Raw-terugval. T7 voegt [`time`]
//! (tijdzone-weergave) en [`filter`] (kanaal/niveau/zoeken) toe. T8 voegt [`tui`] toe
//! (ratatui + crossterm, vim-toetsen). T9 voegt [`editor`] toe (Neovim-sprong,
//! `$EDITOR`/`vi`-terugval, padmapping). Follow volgt in een latere taak.

pub mod editor;
pub mod filter;
pub mod model;
pub mod parse;
pub mod reader;
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
