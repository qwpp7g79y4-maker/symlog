//! Kolombreedte en grafeemveilig afkappen voor de TUI (task.md T8; blueprint.md
//! `tui::width`).
//!
//! Terminals tekenen breed-Oost-Aziatische tekens en de meeste emoji twee kolommen
//! breed, en combinerende tekens (accenten) nul kolommen: het aantal `char`s alleen
//! geeft dus de verkeerde breedte (task.md T8 "Doet": "emoji en CJK = 2 kolommen,
//! combining = 0"). `unicode-width` levert die kolombreedte per grafeemcluster;
//! `unicode-segmentation` levert de grafeemclusters zelf, zodat afkappen nooit een
//! cluster middendoor snijdt (task.md T8 "Doet": "afkappen gebeurt nooit midden in een
//! grafeem") — bijvoorbeeld een meerdere-codepunt ZWJ-emoji-reeks (F10) of een letter
//! met een losse combinerende accent (ook F10).
//!
//! Besturingstekens (tab, NUL, ANSI-escapes) worden hier niet apart behandeld: de
//! aanroeper saneert tekst eerst met [`crate::parse::raw::sanitize_for_display`] (T6),
//! die ze al vervangt door zichtbare, enkel-kolombrede "control picture"-tekens
//! vóórdat de breedte hier berekend wordt.
//!
//! Bekende beperking: deze module kent geen aparte regel voor ZWJ-emoji-reeksen (zie
//! F10) — de breedte van zo'n cluster is de som van de breedtes van de afzonderlijke
//! codepunten (de ZWJ zelf telt als 0), niet de ene of twee kolommen die een terminal
//! er werkelijk voor reserveert. Dat is een bekende, gedeelde beperking van de
//! `unicode-width`-crate (nog geen Unicode-brede consensus over ZWJ-breedte); deze
//! module geeft dat getal consistent door, en de F10-tests in [`crate::tui::view`]
//! leggen vast wat dat concreet oplevert, niet wat een specifieke terminal-emulator
//! zou tekenen.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Weergavebreedte van `text` in terminalkolommen (som van de grafeemclusterbreedtes).
pub fn display_width(text: &str) -> usize {
    text.graphemes(true).map(grapheme_width).sum()
}

/// Breedte van één grafeemcluster (meerdere codepunten bij een combinerend teken of een
/// ZWJ-reeks, zie de moduledoc hierboven).
fn grapheme_width(grapheme: &str) -> usize {
    grapheme.width()
}

/// Kapt `text` af tot hoogstens `max_width` kolommen, zonder een grafeemcluster
/// middendoor te snijden (task.md T8 "Doet"). Geeft de hele tekst terug als die al
/// past; geeft een lege string terug als zelfs het eerste cluster niet past.
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0usize;
    for grapheme in text.graphemes(true) {
        let gw = grapheme_width(grapheme);
        if width + gw > max_width {
            break;
        }
        result.push_str(grapheme);
        width += gw;
    }
    result
}

/// Kapt `text` af tot hoogstens `width` kolommen en vult daarna aan met spaties tot
/// precies `width` kolommen, voor kolomuitlijning in de lijstweergave
/// ([`crate::tui::view`]).
pub fn pad_to_width(text: &str, width: usize) -> String {
    let truncated = truncate_to_width(text, width);
    let current = display_width(&truncated);
    let mut out = truncated;
    out.push_str(&" ".repeat(width.saturating_sub(current)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_width_equals_char_count() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn cjk_characters_are_two_columns_wide() {
        // task.md T8 "Doet": CJK = 2 kolommen.
        assert_eq!(display_width("你好"), 4);
    }

    #[test]
    fn combining_accent_adds_no_width() {
        // 'e' (1 kolom) + combinerende acute accent U+0301 (0 kolommen) = 1 grafeem,
        // 1 kolom breed in totaal (task.md T8 "Doet": "combining = 0").
        let decomposed = "e\u{0301}";
        assert_eq!(display_width(decomposed), 1);
        // Hetzelfde teken, vooraf samengesteld (NFC): ook 1 kolom.
        assert_eq!(display_width("\u{00e9}"), 1);
    }

    #[test]
    fn truncate_never_splits_a_combining_grapheme_cluster() {
        // "e" + combinerende accent is één grafeem van 1 kolom: bij max_width 0 moet
        // het hele cluster wegvallen, niet alleen de combinerende accent.
        let decomposed = "e\u{0301}bc";
        assert_eq!(truncate_to_width(decomposed, 0), "");
        assert_eq!(truncate_to_width(decomposed, 1), "e\u{0301}");
    }

    #[test]
    fn truncate_never_splits_a_zwj_emoji_sequence() {
        // Familie-emoji als ZWJ-reeks (F10): man + ZWJ + vrouw + ZWJ + meisje + ZWJ +
        // jongen is in Unicode-segmentatie één grafeemcluster. Een max_width die niet
        // het hele cluster toelaat, levert dus niets van dat cluster op, nooit een
        // kapotte helft.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        assert_eq!(truncate_to_width(family, 0), "");
        let full_width = display_width(family);
        assert_eq!(truncate_to_width(family, full_width), family);
        assert_eq!(truncate_to_width(family, full_width - 1), "");
    }

    #[test]
    fn truncate_keeps_whole_text_when_it_already_fits() {
        assert_eq!(truncate_to_width("hallo", 10), "hallo");
        assert_eq!(truncate_to_width("hallo", 5), "hallo");
    }

    #[test]
    fn truncate_stops_exactly_at_the_boundary() {
        assert_eq!(truncate_to_width("abcdef", 3), "abc");
    }

    #[test]
    fn pad_extends_short_text_with_spaces() {
        assert_eq!(pad_to_width("ab", 5), "ab   ");
        assert_eq!(display_width(&pad_to_width("ab", 5)), 5);
    }

    #[test]
    fn pad_truncates_text_that_is_too_wide() {
        // "你好" is 4 kolommen breed maar bestaat uit 2 grafemen van elk 2 kolommen:
        // bij width 3 past alleen "你" (2 kolommen), plus 1 spatie opvulling.
        assert_eq!(pad_to_width("你好", 3), "你 ");
        assert_eq!(display_width(&pad_to_width("你好", 3)), 3);
    }

    #[test]
    fn pad_on_exact_width_adds_no_spaces() {
        assert_eq!(pad_to_width("abc", 3), "abc");
    }
}
