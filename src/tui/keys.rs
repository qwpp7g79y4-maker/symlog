//! Vim-toetsenbord: vertaalt crossterm-toetsgebeurtenissen naar [`App`]-mutaties
//! (task.md T8; blueprint.md `tui::keys`).
//!
//! [`handle_key`] is een pure functie van `(App, KeyEvent) -> ()`: hij roept geen
//! terminal-API's aan, dus een gescripte reeks toetsen kan zonder een echte tty getest
//! worden (task.md T8 "Klaar als": "een test die de toetsenreeks `gg`, `G`, `/foo`,
//! `n`, `N` op een gescripte event-stroom toetst", zie [`App`]'s eigen tests voor de
//! navigatielogica zelf — dit bestand test vooral dat de juiste toets de juiste
//! `App`-methode aanroept).

use super::app::{App, Mode};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Verwerkt één toetsgebeurtenis. Welke toetsen herkend worden, hangt af van
/// [`App::mode`]: de zoekprompt en de kiezers hebben hun eigen kleine toetsenbord.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    match app.mode() {
        Mode::Search => handle_search_key(app, key),
        Mode::ChannelPicker | Mode::LevelPicker => handle_picker_key(app, key),
        Mode::Help => handle_help_key(app, key),
        Mode::Normal => handle_normal_key(app, key),
    }
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

fn handle_normal_key(app: &mut App, key: KeyEvent) {
    // Een losse `g` wacht op een tweede (task.md T8 "Klaar als": `gg`); elke andere
    // toets annuleert dat wachten en wordt daarna normaal verwerkt, niet genegeerd.
    if app.pending_g() {
        app.clear_pending_g();
        if key.code == KeyCode::Char('g') {
            app.go_top();
            return;
        }
    }

    if is_ctrl(&key, 'c') {
        app.quit();
        return;
    }
    if is_ctrl(&key, 'd') {
        app.half_page(1);
        return;
    }
    if is_ctrl(&key, 'u') {
        app.half_page(-1);
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Char('j') | KeyCode::Down => app.move_selection(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_selection(-1),
        KeyCode::Char('g') => app.handle_g_key(),
        KeyCode::Char('G') => app.go_bottom(),
        KeyCode::Char('/') => app.enter_search_mode(),
        KeyCode::Char('n') => app.search_next(),
        KeyCode::Char('N') => app.search_prev(),
        KeyCode::Char('c') => app.open_channel_picker(),
        KeyCode::Char('l') => app.open_level_picker(),
        KeyCode::Char('F') => app.toggle_follow(),
        KeyCode::Char('o') => app.request_open(),
        KeyCode::Char('y') => app.mark_copy_on_exit(),
        KeyCode::Char('?') => app.toggle_help(),
        KeyCode::Enter | KeyCode::Tab => app.toggle_detail(),
        _ => {}
    }
}

fn handle_search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.commit_search(),
        KeyCode::Esc => app.cancel_search_input(),
        KeyCode::Backspace => app.pop_search_char(),
        KeyCode::Char(c) => app.push_search_char(c),
        _ => {}
    }
}

fn handle_picker_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.picker_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.picker_move(-1),
        KeyCode::Char(' ') => app.picker_toggle_current(),
        KeyCode::Enter => app.commit_picker(),
        KeyCode::Esc => app.cancel_picker(),
        _ => {}
    }
}

fn handle_help_key(app: &mut App, _key: KeyEvent) {
    // Elke toets sluit de help weer (task.md T8 "Doet": "`?` (help)").
    app.toggle_help();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Level, Record};
    use crate::parse::raw::ParsedRecord;

    fn record(message: &str) -> ParsedRecord {
        ParsedRecord::Record(Record {
            datetime: "2026-01-15T10:00:00.000000+01:00".to_string(),
            ts: None,
            channel: "app".to_string(),
            level: Level::Info,
            message: message.to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames: Vec::new(),
        })
    }

    fn sample_app() -> App {
        App::new(vec![
            record("alpha"),
            record("foo one"),
            record("beta"),
            record("foo two"),
            record("gamma"),
        ])
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            handle_key(app, key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn full_keystroke_scenario_from_task_md() {
        // task.md T8 "Klaar als": de toetsenreeks `gg`, `G`, `/foo`, `n`, `N`.
        let mut app = sample_app();

        handle_key(&mut app, key(KeyCode::Char('g')));
        handle_key(&mut app, key(KeyCode::Char('g')));
        assert_eq!(app.selected_index(), 0);

        handle_key(&mut app, key(KeyCode::Char('G')));
        assert_eq!(app.selected_index(), 4);

        handle_key(&mut app, key(KeyCode::Char('/')));
        assert_eq!(app.mode(), Mode::Search);
        type_text(&mut app, "foo");
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.selected_index(), 1);

        handle_key(&mut app, key(KeyCode::Char('n')));
        assert_eq!(app.selected_index(), 3);

        handle_key(&mut app, key(KeyCode::Char('N')));
        assert_eq!(app.selected_index(), 1);
    }

    #[test]
    fn lone_g_does_not_consume_the_following_unrelated_key() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('g')));
        assert!(app.pending_g());
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert!(!app.pending_g());
        assert_eq!(app.selected_index(), 1, "de j moet nog gewoon bewegen");
    }

    #[test]
    fn j_and_k_move_the_selection() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('j')));
        assert_eq!(app.selected_index(), 1);
        handle_key(&mut app, key(KeyCode::Char('k')));
        assert_eq!(app.selected_index(), 0);
    }

    #[test]
    fn arrow_keys_are_equivalent_to_j_and_k() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Down));
        assert_eq!(app.selected_index(), 1);
        handle_key(&mut app, key(KeyCode::Up));
        assert_eq!(app.selected_index(), 0);
    }

    #[test]
    fn ctrl_d_and_ctrl_u_move_by_half_a_page() {
        let mut app = sample_app();
        app.set_viewport_height(4);
        handle_key(&mut app, ctrl_key('d'));
        assert_eq!(app.selected_index(), 2);
        handle_key(&mut app, ctrl_key('u'));
        assert_eq!(app.selected_index(), 0);
    }

    #[test]
    fn q_and_ctrl_c_both_quit() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('q')));
        assert!(app.should_quit());

        let mut app = sample_app();
        handle_key(&mut app, ctrl_key('c'));
        assert!(app.should_quit());
    }

    #[test]
    fn enter_and_tab_toggle_the_detail_panel() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Enter));
        assert!(app.detail_open());
        handle_key(&mut app, key(KeyCode::Tab));
        assert!(!app.detail_open());
    }

    #[test]
    fn question_mark_opens_and_any_key_closes_help() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('?')));
        assert_eq!(app.mode(), Mode::Help);
        handle_key(&mut app, key(KeyCode::Char('x')));
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn search_prompt_supports_backspace_and_escape() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('/')));
        type_text(&mut app, "fx");
        handle_key(&mut app, key(KeyCode::Backspace));
        type_text(&mut app, "o");
        assert_eq!(app.search_input(), "fo");
        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.search_input(), "");
    }

    #[test]
    fn channel_picker_keys_move_toggle_and_commit() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('c')));
        assert_eq!(app.mode(), Mode::ChannelPicker);
        handle_key(&mut app, key(KeyCode::Char('j')));
        handle_key(&mut app, key(KeyCode::Char(' ')));
        handle_key(&mut app, key(KeyCode::Enter));
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn picker_escape_cancels_without_applying() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('l')));
        handle_key(&mut app, key(KeyCode::Char(' ')));
        handle_key(&mut app, key(KeyCode::Esc));
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.visible_len(), 5, "niets toegepast");
    }

    #[test]
    fn f_toggles_follow() {
        let mut app = sample_app();
        handle_key(&mut app, key(KeyCode::Char('F')));
        assert!(app.follow());
    }
}
