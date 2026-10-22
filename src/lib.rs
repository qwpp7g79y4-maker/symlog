//! symlog: terminal-logviewer voor Symfony/Monolog-logs (read-only, geen netwerk).
//!
//! T1 legde het repo-skelet vast, T2 voegde [`reader`] toe (offset-index en lazy
//! records). T3 voegt [`model`] (gedeelde recordtypes) en [`parse`] (de
//! LineFormatter-parser) toe, zie `blueprint.md`. Multiline/stacktraces, de
//! JsonFormatter, de Raw-terugval, tijdzones/filters, TUI, editor en follow volgen in
//! latere taken.

pub mod model;
pub mod parse;
pub mod reader;

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
