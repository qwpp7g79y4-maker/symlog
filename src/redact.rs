//! Maskeert geheimen in tekst die naar de gebruiker gaat.
//!
//! Het logbestand zelf blijft onaangeroerd: dit draait alleen over tekst die plain
//! of de TUI al gaat tonen. `--no-redact` slaat deze stap over.
//!
//! Vervangen door `***`:
//! - de waarde na `password`, `passwd`, `token`, `secret`, `api_key` of `apikey`
//!   (hoofdletterongevoelig, met `=` of `:`, ook als JSON-veld);
//! - het token na `Authorization: Bearer`;
//! - een sleutel `sk_live_…` of `sk_test_…`;
//! - een AWS-toegangssleutel `AKIA` plus precies 16 hoofdletters of cijfers.

use crate::parse::raw::sanitize_for_display;

const KEYS: [&str; 6] = ["password", "api_key", "apikey", "passwd", "secret", "token"];

struct Span {
    /// Einde (exclusief) van het stuk dat blijft staan, vóór `***`.
    keep_end: usize,
    /// Einde (exclusief) van de waarde die door `***` vervangen wordt.
    value_end: usize,
    /// Index waar het scannen verdergaat (na een sluitend aanhalingsteken).
    resume: usize,
}

/// Saniteert `text` voor weergave en maskeert geheimen als `redact` aan staat.
/// Saniteren komt ná het maskeren, zodat een stuurteken midden in een waarde de
/// waarde niet in tweeën knipt.
pub fn for_display(text: &str, redact: bool) -> String {
    let masked = if redact {
        redact_secrets(text)
    } else {
        text.to_string()
    };
    sanitize_for_display(&masked)
}

/// Vervangt herkende geheimen in `input` door `***`. Sleutelnamen blijven staan.
pub fn redact_secrets(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(span) = match_at(bytes, i) {
            if span_is_usable(i, bytes.len(), &span) {
                out.push_str(&input[i..span.keep_end]);
                out.push_str("***");
                out.push_str(&input[span.value_end..span.resume]);
                i = span.resume;
                continue;
            }
        }
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn span_is_usable(start: usize, len: usize, span: &Span) -> bool {
    span.resume > start
        && span.keep_end >= start
        && span.keep_end <= span.value_end
        && span.value_end <= span.resume
        && span.resume <= len
}

fn match_at(bytes: &[u8], i: usize) -> Option<Span> {
    match_assignment(bytes, i)
        .or_else(|| match_bearer(bytes, i))
        .or_else(|| match_stripe(bytes, i))
        .or_else(|| match_akia(bytes, i))
}

fn match_assignment(bytes: &[u8], i: usize) -> Option<Span> {
    if !at_boundary(bytes, i) {
        return None;
    }
    for key in KEYS {
        if !starts_with_ignore_ascii(&bytes[i..], key) {
            continue;
        }
        let mut j = i + key.len();
        if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
            j += 1;
        }
        j = skip_inline_ws(bytes, j);
        if j >= bytes.len() || (bytes[j] != b'=' && bytes[j] != b':') {
            continue;
        }
        j += 1;
        j = skip_inline_ws(bytes, j);
        if let Some(span) = scan_value(bytes, j) {
            return Some(span);
        }
    }
    None
}

fn match_bearer(bytes: &[u8], i: usize) -> Option<Span> {
    if !at_boundary(bytes, i) {
        return None;
    }
    const WORD: &str = "authorization";
    if !starts_with_ignore_ascii(&bytes[i..], WORD) {
        return None;
    }
    let mut j = i + WORD.len();
    j = skip_inline_ws(bytes, j);
    if j >= bytes.len() || bytes[j] != b':' {
        return None;
    }
    j += 1;
    j = skip_inline_ws(bytes, j);
    const BEARER: &str = "bearer";
    if !starts_with_ignore_ascii(&bytes[j..], BEARER) {
        return None;
    }
    j += BEARER.len();
    if j >= bytes.len() || !bytes[j].is_ascii_whitespace() {
        return None;
    }
    let value_start = skip_inline_ws(bytes, j);
    if value_start >= bytes.len() || bytes[value_start].is_ascii_whitespace() {
        return None;
    }
    let mut value_end = value_start;
    while value_end < bytes.len()
        && !bytes[value_end].is_ascii_whitespace()
        && bytes[value_end] != b'"'
        && bytes[value_end] != b'\''
    {
        value_end += 1;
    }
    if value_end == value_start {
        return None;
    }
    Some(Span {
        keep_end: value_start,
        value_end,
        resume: value_end,
    })
}

fn match_stripe(bytes: &[u8], i: usize) -> Option<Span> {
    for prefix in ["sk_live_", "sk_test_"] {
        let prefix_bytes = prefix.as_bytes();
        if !bytes[i..].starts_with(prefix_bytes) {
            continue;
        }
        let mut j = i + prefix_bytes.len();
        let alnum_start = j;
        while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if j > alnum_start {
            return Some(Span {
                keep_end: i,
                value_end: j,
                resume: j,
            });
        }
    }
    None
}

fn match_akia(bytes: &[u8], i: usize) -> Option<Span> {
    const PREFIX: &[u8] = b"AKIA";
    if !bytes[i..].starts_with(PREFIX) {
        return None;
    }
    let start = i + PREFIX.len();
    let end = start + 16;
    if end > bytes.len() {
        return None;
    }
    if !bytes[start..end]
        .iter()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return None;
    }
    if end < bytes.len() && (bytes[end].is_ascii_uppercase() || bytes[end].is_ascii_digit()) {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: end,
        resume: end,
    })
}

/// Waarde na `=` of `:`. `keep_end` ligt op de grens tussen scheidingsteken en waarde,
/// zodat aanhalingstekens blijven staan en alleen de inhoud `***` wordt.
fn scan_value(bytes: &[u8], i: usize) -> Option<Span> {
    if i >= bytes.len() {
        return None;
    }
    match bytes[i] {
        b'"' | b'\'' => scan_quoted(bytes, i),
        b'{' | b'[' => scan_balanced(bytes, i),
        b if !is_unquoted_delimiter(b) => scan_unquoted(bytes, i),
        _ => None,
    }
}

fn scan_quoted(bytes: &[u8], i: usize) -> Option<Span> {
    let quote = bytes[i];
    let mut j = i + 1;
    while j < bytes.len() {
        if bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        if bytes[j] == quote {
            return Some(Span {
                keep_end: i + 1,
                value_end: j,
                resume: j + 1,
            });
        }
        j += 1;
    }
    if j > i + 1 {
        return Some(Span {
            keep_end: i + 1,
            value_end: j,
            resume: j,
        });
    }
    None
}

fn scan_unquoted(bytes: &[u8], i: usize) -> Option<Span> {
    let mut j = i;
    while j < bytes.len() && !is_unquoted_delimiter(bytes[j]) {
        j += 1;
    }
    if j == i {
        return None;
    }
    Some(Span {
        keep_end: i,
        value_end: j,
        resume: j,
    })
}

fn scan_balanced(bytes: &[u8], i: usize) -> Option<Span> {
    let open = bytes[i];
    let close = match open {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };
    let mut depth = 0usize;
    let mut j = i;
    while j < bytes.len() {
        let b = bytes[j];
        if b == b'"' || b == b'\'' {
            let quote = b;
            let mut k = j + 1;
            let mut closed = false;
            while k < bytes.len() {
                if bytes[k] == b'\\' && k + 1 < bytes.len() {
                    k += 2;
                    continue;
                }
                if bytes[k] == quote {
                    closed = true;
                    k += 1;
                    break;
                }
                k += 1;
            }
            if !closed {
                return None;
            }
            j = k;
            continue;
        }
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(Span {
                    keep_end: i,
                    value_end: j + 1,
                    resume: j + 1,
                });
            }
        }
        j += 1;
    }
    None
}

fn is_unquoted_delimiter(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b',' | b';' | b'"' | b'\'' | b'{' | b'}' | b'[' | b']' | b'<' | b'>' | b'&' | b'|'
        )
}

fn at_boundary(bytes: &[u8], i: usize) -> bool {
    i == 0 || !is_ident(bytes[i - 1])
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn starts_with_ignore_ascii(haystack: &[u8], key: &str) -> bool {
    let key_bytes = key.as_bytes();
    haystack.len() >= key_bytes.len() && haystack[..key_bytes.len()].eq_ignore_ascii_case(key_bytes)
}

fn skip_inline_ws(bytes: &[u8], mut j: usize) -> usize {
    while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t') {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_line_hides_password_and_stripe_key() {
        let input = "password=hunter2 leaked token=sk_live_abcdef123456";
        let out = redact_secrets(input);
        assert_eq!(out, "password=*** leaked token=***");
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("sk_live_abcdef123456"));
    }

    #[test]
    fn json_fields_keep_the_key_and_mask_the_value() {
        let input = r#"{"password":"hunter2","api_key":"abcd","user":"ada"}"#;
        assert_eq!(
            redact_secrets(input),
            r#"{"password":"***","api_key":"***","user":"ada"}"#
        );
        let spaced = r#""token" : "sk_test_zzzz9999""#;
        assert_eq!(redact_secrets(spaced), r#""token" : "***""#);
    }

    #[test]
    fn json_string_escapes_do_not_end_the_value_early() {
        let input = r#"{"password":"hun\"ter2","user":"ada"}"#;
        assert_eq!(redact_secrets(input), r#"{"password":"***","user":"ada"}"#);
    }

    #[test]
    fn bearer_and_akia_are_masked() {
        assert_eq!(
            redact_secrets("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig"),
            "Authorization: Bearer ***"
        );
        assert_eq!(
            redact_secrets("aws AKIAIOSFODNN7EXAMPLE done"),
            "aws *** done"
        );
    }

    #[test]
    fn key_names_are_case_insensitive_and_accept_a_colon() {
        assert_eq!(redact_secrets("PASSWORD=Hunter2"), "PASSWORD=***");
        assert_eq!(redact_secrets("API_KEY=UpperKey"), "API_KEY=***");
        assert_eq!(redact_secrets("APIKEY=apk"), "APIKEY=***");
        assert_eq!(redact_secrets("PASSWD=pwtoken"), "PASSWD=***");
        assert_eq!(redact_secrets("SECRET: secval"), "SECRET: ***");
        assert_eq!(
            redact_secrets("AUTHORIZATION: BEARER S3cretToken"),
            "AUTHORIZATION: BEARER ***"
        );
    }

    #[test]
    fn ordinary_log_text_is_unchanged() {
        let input = "[2026-01-15T10:30:45.123456+01:00] app.DEBUG: User cache warmed";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn lowercase_akia_lookalike_is_not_a_key() {
        let input = "AKIAiosfodnn7example";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn for_display_can_leave_the_original_value() {
        let input = "password=hunter2";
        assert_eq!(for_display(input, true), "password=***");
        assert_eq!(for_display(input, false), input);
    }
}
