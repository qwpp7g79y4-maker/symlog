//! symlog: terminal-logviewer voor Symfony/Monolog-logs (read-only, geen netwerk).
//!
//! Dit is het repo-skelet uit taak T1 van `task.md`. De modules uit `blueprint.md`
//! (reader, parse, filter, tui, editor, follow) volgen in latere taken.

/// Placeholder tot T2 het echte leeswerk toevoegt; bestaat zodat de crate een lib-test
/// heeft om CI tegen te draaien.
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
