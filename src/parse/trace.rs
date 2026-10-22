//! Frames uit stacktrace-tekst en uit JSON-exceptiedata (task.md T4; blueprint.md
//! `parse::trace`).
//!
//! Twee bronnen, dezelfde regelsyntax voor een frame (`#<n> <pad>(<regel>): <call>`,
//! `#<n> [internal function]: <call>` of `#<n> {main}`, zoals PHP's eigen
//! `Exception::getTraceAsString()` die schrijft):
//! - [`frames_from_message`]: een (mogelijk meerregelig) LineFormatter-bericht. Regels
//!   die geen frame zijn (`Stack trace:`, `[previous exception] ...`, de eerste regel
//!   van de melding zelf) worden overgeslagen; ze blijven gewoon deel van `message`.
//! - [`frames_from_exception_json`]: `context.exception` uit een JsonFormatter-record.
//!   `file` (`"/pad:regel"`, de werplocatie) wordt de eerste frame, zonder functienaam;
//!   `trace` is een array met dezelfde regelsyntax als hierboven.

use crate::model::Frame;

/// Haalt frames uit de doorlopende tekst van een (mogelijk meerregelig) bericht: elke
/// regel die matcht met `#<n> ...` wordt een frame.
pub fn frames_from_message(text: &str) -> Vec<Frame> {
    text.lines().filter_map(parse_frame_line).collect()
}

/// Haalt frames uit `context.exception` in een JsonFormatter-record: `file`
/// (`"/pad:regel"`) als werplocatie, gevolgd door de frames in `trace`. Ontbrekende of
/// onverwacht gevormde velden leveren gewoon minder frames op (geen fout).
pub fn frames_from_exception_json(exception: &serde_json::Value) -> Vec<Frame> {
    let mut frames = Vec::new();
    if let Some(file) = exception.get("file").and_then(|v| v.as_str()) {
        frames.extend(parse_file_location(file));
    }
    if let Some(trace) = exception.get("trace").and_then(|v| v.as_array()) {
        frames.extend(
            trace
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(parse_frame_line),
        );
    }
    frames
}

/// Parseert `"/pad:regel"` (task.md T4) tot een werplocatie-frame zonder functienaam.
fn parse_file_location(file: &str) -> Option<Frame> {
    let colon = file.rfind(':')?;
    let line: u32 = file[colon + 1..].parse().ok()?;
    Some(Frame {
        path: Some(file[..colon].to_string()),
        line: Some(line),
        func: String::new(),
    })
}

/// Parseert één `#<n> ...`-regel uit een PHP-stacktrace tot een [`Frame`]. `None` als
/// de regel geen frame is (bijvoorbeeld `Stack trace:` of `[previous exception] ...`).
fn parse_frame_line(line: &str) -> Option<Frame> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let rest = line.strip_prefix('#')?;
    let digit_count = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 {
        return None;
    }
    let rest = rest[digit_count..].strip_prefix(' ')?;

    if rest == "{main}" {
        return Some(Frame {
            path: None,
            line: None,
            func: "{main}".to_string(),
        });
    }
    if let Some(after) = rest.strip_prefix("[internal function]") {
        let func = after.strip_prefix(':').unwrap_or(after).trim_start();
        return Some(Frame {
            path: None,
            line: None,
            func: func.to_string(),
        });
    }

    // "<pad>(<regel>): <call>": `call` zelf mag haakjes bevatten (bijvoorbeeld
    // `runStep(0)`), dus een simpele rfind op `(` zou daar de verkeerde `(` vinden.
    // Zoek in plaats daarvan van links de eerste `(` waarvan de inhoud tot de
    // sluitende `)` puur cijfers is én direct gevolgd wordt door `: `: dat is de
    // regelnummer-markering, niet een haakje in het pad of in de aanroep.
    let (open, digits, func) = rest.match_indices('(').find_map(|(idx, _)| {
        let after_open = &rest[idx + 1..];
        let close = after_open.find(')')?;
        let digits = &after_open[..close];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let func = after_open[close + 1..].strip_prefix(": ")?;
        Some((idx, digits, func))
    })?;
    let line_no: u32 = digits.parse().ok()?;
    Some(Frame {
        path: Some(rest[..open].to_string()),
        line: Some(line_no),
        func: func.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_file_and_line_frame() {
        let frame =
            parse_frame_line("#0 /var/www/src/Bar.php(10): App\\Foo->run()").expect("moet parsen");
        assert_eq!(frame.path.as_deref(), Some("/var/www/src/Bar.php"));
        assert_eq!(frame.line, Some(10));
        assert_eq!(frame.func, "App\\Foo->run()");
    }

    #[test]
    fn parses_a_frame_whose_call_has_parenthesised_arguments() {
        // Een rfind op `(` zou hier de `(0)` van de aanroep vinden in plaats van het
        // echte regelnummer-haakje; zie de code-commentaar in parse_frame_line.
        let frame = parse_frame_line("#0 /var/www/src/Dispatcher.php(40): App\\Worker->runStep(0)")
            .expect("moet parsen");
        assert_eq!(frame.path.as_deref(), Some("/var/www/src/Dispatcher.php"));
        assert_eq!(frame.line, Some(40));
        assert_eq!(frame.func, "App\\Worker->runStep(0)");
    }

    #[test]
    fn parses_internal_function_frame_without_jump_target() {
        let frame =
            parse_frame_line("#1 [internal function]: App\\Kernel->handle()").expect("moet parsen");
        assert_eq!(frame.path, None);
        assert_eq!(frame.line, None);
        assert_eq!(frame.func, "App\\Kernel->handle()");
    }

    #[test]
    fn parses_main_frame() {
        let frame = parse_frame_line("#2 {main}").expect("moet parsen");
        assert_eq!(frame.path, None);
        assert_eq!(frame.line, None);
        assert_eq!(frame.func, "{main}");
    }

    #[test]
    fn non_frame_lines_are_ignored() {
        assert!(parse_frame_line("Stack trace:").is_none());
        assert!(parse_frame_line(
            "[previous exception] [object] (LogicException(code: 0): oorzaak at /a.php:1)"
        )
        .is_none());
        assert!(parse_frame_line("Uncaught RuntimeException: boom in /a.php:1").is_none());
    }

    #[test]
    fn frames_from_message_skips_non_frame_lines_and_keeps_order() {
        let text = "Uncaught RuntimeException: boom in /a.php:1\nStack trace:\n#0 /a.php(1): foo()\n#1 {main}";
        let frames = frames_from_message(text);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].line, Some(1));
        assert_eq!(frames[1].func, "{main}");
    }

    #[test]
    fn frames_from_exception_json_combines_file_and_trace() {
        let exception = serde_json::json!({
            "class": "RuntimeException",
            "message": "boom",
            "file": "/var/www/src/Worker.php:88",
            "trace": [
                "#0 /var/www/src/Dispatcher.php(40): App\\Worker->run()",
                "#1 {main}",
            ],
        });
        let frames = frames_from_exception_json(&exception);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].path.as_deref(), Some("/var/www/src/Worker.php"));
        assert_eq!(frames[0].line, Some(88));
        assert_eq!(frames[0].func, "");
        assert_eq!(frames[1].line, Some(40));
        assert_eq!(frames[2].func, "{main}");
    }

    #[test]
    fn frames_from_exception_json_tolerates_missing_fields() {
        assert_eq!(frames_from_exception_json(&serde_json::json!({})), vec![]);
    }
}
