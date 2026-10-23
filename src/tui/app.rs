//! TUI-toestand: selectie, scroll, modus en statusbalk (task.md T8; blueprint.md
//! `tui::app`).
//!
//! [`App`] bevat geen eigen I/O of ratatui-rendering: het is de pure toestandsmachine
//! die [`crate::tui::keys`] muteert en [`crate::tui::view`] leest, zodat beide
//! onafhankelijk van een echte terminal getest kunnen worden (task.md T8 "Klaar als":
//! snapshot- en toetsenreeks-tests zonder een echte tty).
//!
//! De lijst van records (`records`) staat vast voor de levensduur van deze `App`; de
//! kanaal- en niveaufilters uit het kiezermenu ("c"/"l") werken op een `visible`-lijst
//! van indexen in `records` die aan het huidige [`FilterSpec`] voldoet. Dat is bewust
//! een synchrone herberekening ([`App::recompute_visible`]) in plaats van de
//! achtergrond-filterthread uit blueprint.md ("generatienummer"): die thread bestaat om
//! grote bestanden responsief te houden (task.md T12), een taak die nog niet gebouwd
//! is; T8 zelf vraagt in "Klaar als" alleen snapshot-, resize- en
//! toetsenreeks-correctheid, niet schaal. Een latere taak kan `recompute_visible`
//! vervangen door een aanroep naar die thread zonder dat [`crate::tui::keys`] of
//! [`crate::tui::view`] hoeven te veranderen.
//!
//! De interactieve `/zoek` (task.md: "`/zoek`, `n/N`") is navigatie, geen filter: hij
//! verbergt geen records (dat doet `--grep`/[`FilterSpec::search`] op het niveau van de
//! CLI, nog niet aangesloten), maar verplaatst de selectie naar de volgende/vorige
//! match, net als vim's `/`. Voor de matchregel wordt [`FilterSpec::matches_parsed`]
//! hergebruikt (zelfde semantiek als de CLI-zoekfilter) in plaats van een eigen
//! implementatie.

use crate::filter::{ChannelFilter, FilterSpec, LevelFilter};
use crate::model::{self, Level};
use crate::parse::raw::ParsedRecord;
use regex::Regex;

/// Welke invoermodus de TUI nu in staat (task.md T8 "Doet"): bepaalt welke toetsen
/// [`crate::tui::keys::handle_key`] herkent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Lijst-/detailweergave, de normale vim-achtige navigatie.
    Normal,
    /// Zoekprompt open (`/`); getypte tekens gaan in `search_input`.
    Search,
    /// Kanaalkiezer open (`c`).
    ChannelPicker,
    /// Niveaukiezer open (`l`).
    LevelPicker,
    /// Helpoverlay open (`?`).
    Help,
}

/// Meerkeuzelijst voor de kanaal- en niveaukiezer (task.md T8 "Doet"): `cursor` is de
/// gemarkeerde regel, `selected` de regels die aangevinkt zijn. Beide kiezers delen
/// deze structuur; alleen de herkomst van `items` en het toepassen bij
/// [`App::commit_picker`] verschilt.
#[derive(Debug, Clone, Default)]
pub struct Picker {
    pub items: Vec<String>,
    pub cursor: usize,
    pub selected: Vec<bool>,
}

impl Picker {
    fn new(items: Vec<String>) -> Picker {
        let selected = vec![false; items.len()];
        Picker {
            items,
            cursor: 0,
            selected,
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let len = self.items.len() as isize;
        let next = (self.cursor as isize + delta).clamp(0, len - 1);
        self.cursor = next as usize;
    }

    fn toggle_current(&mut self) {
        if let Some(flag) = self.selected.get_mut(self.cursor) {
            *flag = !*flag;
        }
    }

    fn selected_items(&self) -> Vec<&str> {
        self.items
            .iter()
            .zip(self.selected.iter())
            .filter(|(_, &selected)| selected)
            .map(|(item, _)| item.as_str())
            .collect()
    }
}

/// De acht Monolog-niveaus in ernstvolgorde, voor de niveaukiezer (task.md T8 "Doet":
/// "niveaukiezer").
const ALL_LEVELS: [Level; 8] = [
    Level::Debug,
    Level::Info,
    Level::Notice,
    Level::Warning,
    Level::Error,
    Level::Critical,
    Level::Alert,
    Level::Emergency,
];

/// TUI-toestand (task.md T8; blueprint.md `tui::app`). Zie de moduledoc hierboven voor
/// het ontwerp.
pub struct App {
    records: Vec<ParsedRecord>,
    visible: Vec<usize>,
    filter: FilterSpec,
    selected: usize,
    top: usize,
    viewport_height: usize,
    mode: Mode,
    search_input: String,
    last_search: Option<Regex>,
    status: String,
    follow: bool,
    detail_open: bool,
    detail_frame: usize,
    channel_picker: Picker,
    level_picker: Picker,
    pending_g: bool,
    pending_open: Option<model::Frame>,
    copy_on_exit: Option<String>,
    should_quit: bool,
}

impl App {
    /// Bouwt een nieuwe `App` over `records` (al geparst, inclusief Raw-terugval,
    /// task.md T6/T7). Alles is zichtbaar totdat de kanaal- of niveaukiezer een filter
    /// instelt.
    pub fn new(records: Vec<ParsedRecord>) -> App {
        let visible = (0..records.len()).collect();
        App {
            records,
            visible,
            filter: FilterSpec::default(),
            selected: 0,
            top: 0,
            viewport_height: 1,
            mode: Mode::Normal,
            search_input: String::new(),
            last_search: None,
            status: String::new(),
            follow: false,
            detail_open: false,
            detail_frame: 0,
            channel_picker: Picker::default(),
            level_picker: Picker::default(),
            pending_g: false,
            pending_open: None,
            copy_on_exit: None,
            should_quit: false,
        }
    }

    // --- lezen (gebruikt door crate::tui::view) ---

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn follow(&self) -> bool {
        self.follow
    }

    pub fn detail_open(&self) -> bool {
        self.detail_open
    }

    pub fn detail_frame_index(&self) -> usize {
        self.detail_frame
    }

    pub fn search_input(&self) -> &str {
        &self.search_input
    }

    pub fn channel_picker(&self) -> &Picker {
        &self.channel_picker
    }

    pub fn level_picker(&self) -> &Picker {
        &self.level_picker
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn pending_open(&self) -> Option<&model::Frame> {
        self.pending_open.as_ref()
    }

    pub fn copy_on_exit(&self) -> Option<&str> {
        self.copy_on_exit.as_deref()
    }

    /// Aantal records dat (na filters) zichtbaar is.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// 0-gebaseerde positie van de selectie binnen de zichtbare lijst.
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// Eerste zichtbare rij-offset (voor scrollen in [`crate::tui::view`]).
    pub fn top(&self) -> usize {
        self.top
    }

    /// Het geselecteerde record, of `None` als er (na filtering) niets zichtbaar is.
    pub fn selected_record(&self) -> Option<&ParsedRecord> {
        self.visible.get(self.selected).map(|&i| &self.records[i])
    }

    /// Het record op zichtbare positie `row` (voor het tekenen van het venster
    /// [`App::top`]..[`App::top`]+hoogte in [`crate::tui::view`]).
    pub fn record_at(&self, row: usize) -> Option<&ParsedRecord> {
        self.visible.get(row).map(|&i| &self.records[i])
    }

    /// Het momenteel bekeken frame in het detailpaneel (task.md T8 "Doet": `o`/`y` op
    /// een stacktrace-frame), `None` als het detailpaneel dicht is, het record geen
    /// `Record` is, of het record geen frames heeft.
    pub fn selected_frame(&self) -> Option<&model::Frame> {
        if !self.detail_open {
            return None;
        }
        let ParsedRecord::Record(record) = self.selected_record()? else {
            return None;
        };
        if record.frames.is_empty() {
            return None;
        }
        let index = self.detail_frame.min(record.frames.len() - 1);
        record.frames.get(index)
    }

    // --- navigatie ---

    /// Zet de viewport-hoogte die [`crate::tui::view::render`] voor de lijst
    /// gebruikte (task.md T8 "Doet": `Ctrl-d`/`Ctrl-u` is een halve pagina van *die*
    /// hoogte, en resize moet de selectie op hetzelfde record houden). Minimaal 1, zodat
    /// een venster van hoogte 0 nooit tot een deling door nul leidt.
    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height.max(1);
        self.ensure_selected_visible();
    }

    fn ensure_selected_visible(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        }
        let bottom = self.top + self.viewport_height.saturating_sub(1);
        if self.selected > bottom {
            self.top = self
                .selected
                .saturating_sub(self.viewport_height.saturating_sub(1));
        }
        if self.top > 0 && self.top + self.viewport_height > self.visible.len() {
            self.top = self.visible.len().saturating_sub(self.viewport_height);
        }
    }

    fn set_selected(&mut self, index: usize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = index.min(self.visible.len() - 1);
        if self.detail_open {
            self.detail_frame = 0;
        }
        self.ensure_selected_visible();
    }

    /// `j`/`k`: één regel op/neer (task.md T8 "Doet"). Als het detailpaneel open is op
    /// een record met frames, beweegt dit in plaats daarvan door de frames, net als in
    /// de lijst (consistente toetsen in beide weergaves).
    pub fn move_selection(&mut self, delta: isize) {
        if self.detail_open {
            if let Some(ParsedRecord::Record(record)) = self.selected_record() {
                if !record.frames.is_empty() {
                    let len = record.frames.len() as isize;
                    let next = (self.detail_frame as isize + delta).clamp(0, len - 1);
                    self.detail_frame = next as usize;
                    return;
                }
            }
            return;
        }
        let next = (self.selected as isize + delta).clamp(0, self.visible.len() as isize - 1);
        self.set_selected(next.max(0) as usize);
    }

    /// `gg`: naar het eerste record (task.md T8 "Doet"/"Klaar als").
    pub fn go_top(&mut self) {
        self.set_selected(0);
    }

    /// `G`: naar het laatste record (task.md T8 "Doet"/"Klaar als").
    pub fn go_bottom(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        self.set_selected(self.visible.len() - 1);
    }

    /// `Ctrl-d`/`Ctrl-u`: een halve pagina op/neer (task.md T8 "Doet"), gebaseerd op de
    /// laatst bekende viewport-hoogte ([`App::set_viewport_height`]).
    pub fn half_page(&mut self, delta: isize) {
        let half = (self.viewport_height / 2).max(1) as isize;
        self.move_selection(delta * half);
    }

    /// Multi-toets `gg` (task.md T8 "Klaar als": toetsenreeks `gg`, `G`, ...): de
    /// eerste `g` wacht op een tweede. Geeft terug of deze `g` de eerste van het paar
    /// was (dan heeft de aanroeper nog niets te doen) of de afsluitende tweede (dan is
    /// [`App::go_top`] al aangeroepen).
    pub fn handle_g_key(&mut self) {
        if self.pending_g {
            self.pending_g = false;
            self.go_top();
        } else {
            self.pending_g = true;
        }
    }

    /// Of er op een losse `g` gewacht wordt (voor [`crate::tui::keys`): een andere toets
    /// dan de tweede `g` annuleert het wachten zonder zelf genegeerd te worden.
    pub fn pending_g(&self) -> bool {
        self.pending_g
    }

    pub fn clear_pending_g(&mut self) {
        self.pending_g = false;
    }

    // --- zoeken (navigatie, task.md T8 "Doet": "`/`", "`n/N`") ---

    pub fn enter_search_mode(&mut self) {
        self.mode = Mode::Search;
        self.search_input.clear();
    }

    pub fn cancel_search_input(&mut self) {
        self.mode = Mode::Normal;
        self.search_input.clear();
    }

    pub fn push_search_char(&mut self, c: char) {
        self.search_input.push(c);
    }

    pub fn pop_search_char(&mut self) {
        self.search_input.pop();
    }

    /// `Enter` in de zoekprompt: compileert `search_input` als regex (net als
    /// `--grep`/[`FilterSpec::search`]) en springt naar de eerste match ná de huidige
    /// positie. Een ongeldige regex blijft genegeerd met een statusmelding, geen
    /// paniek.
    pub fn commit_search(&mut self) {
        self.mode = Mode::Normal;
        if self.search_input.is_empty() {
            return;
        }
        match Regex::new(&self.search_input) {
            Ok(re) => {
                self.last_search = Some(re);
                self.search_next();
            }
            Err(_) => {
                self.status = format!("ongeldige regex: {}", self.search_input);
            }
        }
    }

    fn matches_search(&self, re: &Regex, visible_index: usize) -> bool {
        let Some(&record_index) = self.visible.get(visible_index) else {
            return false;
        };
        let spec = FilterSpec {
            channels: ChannelFilter::default(),
            levels: None,
            search: Some(re.clone()),
        };
        spec.matches_parsed(&self.records[record_index])
    }

    /// `n`: volgende match van de laatst uitgevoerde zoekopdracht, wrap-around (task.md
    /// T8 "Klaar als").
    pub fn search_next(&mut self) {
        self.search_step(1);
    }

    /// `N`: vorige match van de laatst uitgevoerde zoekopdracht, wrap-around (task.md
    /// T8 "Klaar als").
    pub fn search_prev(&mut self) {
        self.search_step(-1);
    }

    fn search_step(&mut self, direction: isize) {
        let Some(re) = self.last_search.clone() else {
            self.status = "geen eerdere zoekopdracht".to_string();
            return;
        };
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len() as isize;
        let mut candidate = self.selected as isize;
        for _ in 0..len {
            candidate = (candidate + direction).rem_euclid(len);
            if self.matches_search(&re, candidate as usize) {
                self.set_selected(candidate as usize);
                return;
            }
        }
        self.status = format!("geen match voor {:?}", re.as_str());
    }

    // --- kanaal-/niveaukiezer (task.md T8 "Doet") ---

    pub fn open_channel_picker(&mut self) {
        let mut channels: Vec<String> = self
            .records
            .iter()
            .filter_map(|r| match r {
                ParsedRecord::Record(record) => Some(record.channel.clone()),
                ParsedRecord::Raw(_) => None,
            })
            .collect();
        channels.sort();
        channels.dedup();
        let mut picker = Picker::new(channels);
        // Herstel de vinkjes naar de huidige expliciete inclusielijst, zodat heropenen
        // de kiezer niet terugzet naar "niets aangevinkt" (dat zou "alles tonen"
        // betekenen, zie App::commit_picker).
        let included = self.filter.channels.included_channels();
        for (item, flag) in picker.items.iter().zip(picker.selected.iter_mut()) {
            *flag = included.iter().any(|c| c == item);
        }
        self.channel_picker = picker;
        self.mode = Mode::ChannelPicker;
    }

    pub fn open_level_picker(&mut self) {
        let items: Vec<String> = ALL_LEVELS.iter().map(|l| l.as_str().to_string()).collect();
        let mut picker = Picker::new(items);
        if let Some(LevelFilter::OneOf(levels)) = &self.filter.levels {
            for (level, flag) in ALL_LEVELS.iter().zip(picker.selected.iter_mut()) {
                *flag = levels.contains(level);
            }
        }
        self.level_picker = picker;
        self.mode = Mode::LevelPicker;
    }

    pub fn picker_move(&mut self, delta: isize) {
        match self.mode {
            Mode::ChannelPicker => self.channel_picker.move_cursor(delta),
            Mode::LevelPicker => self.level_picker.move_cursor(delta),
            _ => {}
        }
    }

    pub fn picker_toggle_current(&mut self) {
        match self.mode {
            Mode::ChannelPicker => self.channel_picker.toggle_current(),
            Mode::LevelPicker => self.level_picker.toggle_current(),
            _ => {}
        }
    }

    pub fn cancel_picker(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Past de aanvinkingen in de open kiezer toe op het filter en herberekent de
    /// zichtbare lijst (task.md T8 "Doet").
    pub fn commit_picker(&mut self) {
        match self.mode {
            Mode::ChannelPicker => {
                let selected = self.channel_picker.selected_items();
                self.filter.channels = if selected.is_empty() {
                    ChannelFilter::default()
                } else {
                    ChannelFilter::from_specs(selected)
                };
            }
            Mode::LevelPicker => {
                let selected: Vec<Level> = self
                    .level_picker
                    .selected_items()
                    .into_iter()
                    .filter_map(Level::parse)
                    .collect();
                self.filter.levels = if selected.is_empty() {
                    None
                } else {
                    Some(LevelFilter::OneOf(selected))
                };
            }
            _ => return,
        }
        self.mode = Mode::Normal;
        self.recompute_visible();
    }

    /// Zet het filter rechtstreeks (task.md T13: `--level`/`--channel` op de
    /// commandoregel, vóórdat de kiezers er ooit iets mee gedaan hebben). Herberekent
    /// de zichtbare lijst net als [`App::commit_picker`] zou doen.
    pub fn set_filter(&mut self, filter: FilterSpec) {
        self.filter = filter;
        self.recompute_visible();
    }

    fn recompute_visible(&mut self) {
        let selected_record_index = self.visible.get(self.selected).copied();
        self.visible = (0..self.records.len())
            .filter(|&i| self.filter.matches_parsed(&self.records[i]))
            .collect();
        // Blijf, indien mogelijk, op hetzelfde record staan in plaats van op dezelfde
        // positie: een filterwijziging mag de aandacht niet naar een ander record
        // verspringen als het oude record nog zichtbaar is.
        let new_position = selected_record_index
            .and_then(|old_index| self.visible.iter().position(|&i| i == old_index))
            .unwrap_or(0);
        self.selected = 0;
        self.set_selected(new_position);
    }

    // --- overige toetsen (task.md T8 "Doet") ---

    pub fn toggle_follow(&mut self) {
        self.follow = !self.follow;
        self.status = if self.follow {
            "follow aan".to_string()
        } else {
            "follow uit".to_string()
        };
    }

    pub fn toggle_detail(&mut self) {
        self.detail_open = !self.detail_open;
        self.detail_frame = 0;
    }

    pub fn toggle_help(&mut self) {
        self.mode = if self.mode == Mode::Help {
            Mode::Normal
        } else {
            Mode::Help
        };
    }

    /// `o`: vraagt het geselecteerde frame aan om te openen (de daadwerkelijke
    /// editor-sprong volgt in task.md T9; hier wordt alleen vastgelegd *welk* frame).
    /// Een frame zonder bestand:regel ("[internal function]", task.md T4 "Blijft") is
    /// niet springbaar en krijgt alleen een statusmelding.
    pub fn request_open(&mut self) {
        match self.selected_frame().cloned() {
            Some(frame) if frame.path.is_some() && frame.line.is_some() => {
                self.status = format!(
                    "open: {}:{}",
                    frame.path.as_deref().unwrap_or(""),
                    frame.line.unwrap_or(0)
                );
                self.pending_open = Some(frame);
            }
            Some(_) => {
                self.status = "frame niet springbaar (geen bestand:regel)".to_string();
            }
            None => {
                self.status = "geen frame geselecteerd".to_string();
            }
        }
    }

    /// `y`: legt bestand:regel van het geselecteerde frame vast om bij afsluiten naar
    /// stdout te kopiëren (task.md T8 "Doet": "optioneel"; het afsluitpad zelf hoort bij
    /// de CLI-event-loop, nog niet aangesloten).
    pub fn mark_copy_on_exit(&mut self) {
        match self.selected_frame() {
            Some(frame) if frame.path.is_some() && frame.line.is_some() => {
                self.copy_on_exit = Some(format!(
                    "{}:{}",
                    frame.path.as_deref().unwrap_or(""),
                    frame.line.unwrap_or(0)
                ));
                self.status = "bestand:regel gemarkeerd om te kopiëren".to_string();
            }
            _ => {
                self.status = "geen springbaar frame geselecteerd".to_string();
            }
        }
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Level, Record};

    fn record(channel: &str, level: Level, message: &str) -> ParsedRecord {
        ParsedRecord::Record(Record {
            datetime: "2026-01-15T10:00:00.000000+01:00".to_string(),
            ts: None,
            channel: channel.to_string(),
            level,
            message: message.to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames: Vec::new(),
        })
    }

    fn sample_app() -> App {
        App::new(vec![
            record("app", Level::Info, "alpha"),
            record("app", Level::Error, "foo one"),
            record("request", Level::Info, "beta"),
            record("app", Level::Warning, "foo two"),
            record("doctrine", Level::Debug, "gamma"),
        ])
    }

    #[test]
    fn new_app_shows_every_record() {
        let app = sample_app();
        assert_eq!(app.visible_len(), 5);
        assert_eq!(app.selected_index(), 0);
    }

    #[test]
    fn move_selection_clamps_at_the_edges() {
        let mut app = sample_app();
        app.move_selection(-1);
        assert_eq!(app.selected_index(), 0);
        app.move_selection(100);
        assert_eq!(app.selected_index(), 4);
        app.move_selection(100);
        assert_eq!(app.selected_index(), 4);
    }

    #[test]
    fn go_top_and_go_bottom() {
        let mut app = sample_app();
        app.go_bottom();
        assert_eq!(app.selected_index(), 4);
        app.go_top();
        assert_eq!(app.selected_index(), 0);
    }

    #[test]
    fn gg_requires_two_g_presses() {
        let mut app = sample_app();
        app.go_bottom();
        assert_eq!(app.selected_index(), 4);
        app.handle_g_key();
        assert!(app.pending_g());
        assert_eq!(app.selected_index(), 4, "de eerste g beweegt nog niet");
        app.handle_g_key();
        assert!(!app.pending_g());
        assert_eq!(app.selected_index(), 0, "de tweede g springt naar boven");
    }

    #[test]
    fn half_page_moves_by_half_the_viewport_height() {
        let mut app = sample_app();
        app.set_viewport_height(4); // halve pagina = 2
        app.half_page(1);
        assert_eq!(app.selected_index(), 2);
        app.half_page(1);
        assert_eq!(app.selected_index(), 4, "geklemd aan de onderkant");
        app.half_page(-1);
        assert_eq!(app.selected_index(), 2);
    }

    #[test]
    fn search_sequence_matches_task_keystroke_scenario() {
        // task.md T8 "Klaar als": `gg`, `G`, `/foo`, `n`, `N` op een gescripte
        // event-stroom. Matches op "foo" staan op visuele index 1 en 3.
        let mut app = sample_app();
        app.go_top();
        assert_eq!(app.selected_index(), 0);
        app.go_bottom();
        assert_eq!(app.selected_index(), 4);

        app.enter_search_mode();
        for c in "foo".chars() {
            app.push_search_char(c);
        }
        assert_eq!(app.search_input(), "foo");
        app.commit_search();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(
            app.selected_index(),
            1,
            "eerste match na index 4, wrap-around"
        );

        app.search_next();
        assert_eq!(app.selected_index(), 3);

        app.search_prev();
        assert_eq!(app.selected_index(), 1);
    }

    #[test]
    fn search_with_no_match_keeps_the_selection_and_sets_a_status() {
        let mut app = sample_app();
        app.enter_search_mode();
        for c in "nergens-te-vinden".chars() {
            app.push_search_char(c);
        }
        app.commit_search();
        assert_eq!(app.selected_index(), 0);
        assert!(app.status().contains("geen match"));
    }

    #[test]
    fn search_with_invalid_regex_sets_a_status_without_panicking() {
        let mut app = sample_app();
        app.enter_search_mode();
        for c in "(unclosed".chars() {
            app.push_search_char(c);
        }
        app.commit_search();
        assert!(app.status().contains("ongeldige regex"));
    }

    #[test]
    fn cancel_search_input_clears_the_prompt_without_navigating() {
        let mut app = sample_app();
        app.enter_search_mode();
        app.push_search_char('x');
        app.pop_search_char();
        app.push_search_char('y');
        app.cancel_search_input();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.search_input(), "");
        assert_eq!(app.selected_index(), 0);
    }

    #[test]
    fn channel_picker_filters_by_selected_channels() {
        let mut app = sample_app();
        app.open_channel_picker();
        assert_eq!(app.mode(), Mode::ChannelPicker);
        assert_eq!(
            app.channel_picker().items,
            vec![
                "app".to_string(),
                "doctrine".to_string(),
                "request".to_string()
            ]
        );
        // "request" ligt op cursorpositie 2.
        app.picker_move(2);
        app.picker_toggle_current();
        app.commit_picker();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.visible_len(), 1);
        let ParsedRecord::Record(record) = app.selected_record().unwrap() else {
            panic!("verwacht Record")
        };
        assert_eq!(record.channel, "request");
    }

    #[test]
    fn channel_picker_with_nothing_selected_shows_everything() {
        let mut app = sample_app();
        app.open_channel_picker();
        app.commit_picker();
        assert_eq!(app.visible_len(), 5);
    }

    #[test]
    fn cancel_picker_discards_changes() {
        let mut app = sample_app();
        app.open_channel_picker();
        app.picker_toggle_current();
        app.cancel_picker();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.visible_len(), 5, "niets toegepast");
    }

    #[test]
    fn level_picker_filters_by_selected_levels() {
        let mut app = sample_app();
        app.open_level_picker();
        assert_eq!(
            app.level_picker().items,
            vec![
                "DEBUG",
                "INFO",
                "NOTICE",
                "WARNING",
                "ERROR",
                "CRITICAL",
                "ALERT",
                "EMERGENCY"
            ]
        );
        // "ERROR" staat op index 4.
        app.picker_move(4);
        app.picker_toggle_current();
        app.commit_picker();
        assert_eq!(app.visible_len(), 1);
        let ParsedRecord::Record(record) = app.selected_record().unwrap() else {
            panic!("verwacht Record")
        };
        assert_eq!(record.level, Level::Error);
    }

    #[test]
    fn reopening_channel_picker_restores_current_selection() {
        let mut app = sample_app();
        app.open_channel_picker();
        app.picker_toggle_current(); // "app" aanvinken (cursor start op 0)
        app.commit_picker();

        app.open_channel_picker();
        assert_eq!(app.channel_picker().selected, vec![true, false, false]);
    }

    #[test]
    fn toggle_follow_flips_state_and_sets_status() {
        let mut app = sample_app();
        assert!(!app.follow());
        app.toggle_follow();
        assert!(app.follow());
        assert_eq!(app.status(), "follow aan");
        app.toggle_follow();
        assert!(!app.follow());
    }

    #[test]
    fn toggle_help_switches_mode_back_and_forth() {
        let mut app = sample_app();
        app.toggle_help();
        assert_eq!(app.mode(), Mode::Help);
        app.toggle_help();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[test]
    fn toggle_detail_resets_frame_cursor() {
        let mut app = sample_app();
        assert!(!app.detail_open());
        app.toggle_detail();
        assert!(app.detail_open());
        app.toggle_detail();
        assert!(!app.detail_open());
    }

    #[test]
    fn request_open_without_detail_open_sets_a_status() {
        let mut app = sample_app();
        app.request_open();
        assert_eq!(app.status(), "geen frame geselecteerd");
        assert!(app.pending_open().is_none());
    }

    #[test]
    fn request_open_on_a_jumpable_frame_sets_pending_open() {
        let frame = model::Frame {
            path: Some("/app/src/Foo.php".to_string()),
            line: Some(42),
            func: "App\\Foo->bar()".to_string(),
        };
        let mut app = App::new(vec![ParsedRecord::Record(Record {
            datetime: "2026-01-15T10:00:00.000000+01:00".to_string(),
            ts: None,
            channel: "app".to_string(),
            level: Level::Error,
            message: "boom".to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames: vec![frame.clone()],
        })]);
        app.toggle_detail();
        app.request_open();
        assert_eq!(app.pending_open(), Some(&frame));
        assert_eq!(app.status(), "open: /app/src/Foo.php:42");
    }

    #[test]
    fn request_open_on_a_non_jumpable_frame_only_sets_a_status() {
        let frame = model::Frame {
            path: None,
            line: None,
            func: "[internal function]".to_string(),
        };
        let mut app = App::new(vec![ParsedRecord::Record(Record {
            datetime: "2026-01-15T10:00:00.000000+01:00".to_string(),
            ts: None,
            channel: "app".to_string(),
            level: Level::Error,
            message: "boom".to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames: vec![frame],
        })]);
        app.toggle_detail();
        app.request_open();
        assert!(app.pending_open().is_none());
        assert_eq!(app.status(), "frame niet springbaar (geen bestand:regel)");
    }

    #[test]
    fn mark_copy_on_exit_records_file_and_line() {
        let frame = model::Frame {
            path: Some("/app/src/Foo.php".to_string()),
            line: Some(42),
            func: "App\\Foo->bar()".to_string(),
        };
        let mut app = App::new(vec![ParsedRecord::Record(Record {
            datetime: "2026-01-15T10:00:00.000000+01:00".to_string(),
            ts: None,
            channel: "app".to_string(),
            level: Level::Error,
            message: "boom".to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames: vec![frame],
        })]);
        app.toggle_detail();
        app.mark_copy_on_exit();
        assert_eq!(app.copy_on_exit(), Some("/app/src/Foo.php:42"));
    }

    #[test]
    fn quit_sets_should_quit() {
        let mut app = sample_app();
        assert!(!app.should_quit());
        app.quit();
        assert!(app.should_quit());
    }

    #[test]
    fn set_viewport_height_keeps_selection_in_view() {
        let mut app = sample_app();
        app.set_viewport_height(2);
        app.go_bottom();
        assert_eq!(app.selected_index(), 4);
        assert!(
            app.top() <= 4 && app.top() + 2 > 4,
            "selectie moet binnen beeld blijven"
        );
    }

    #[test]
    fn resize_keeps_selection_on_the_same_record() {
        let mut app = sample_app();
        app.set_viewport_height(5);
        app.move_selection(1);
        assert_eq!(app.selected_index(), 1);
        app.set_viewport_height(2);
        assert_eq!(
            app.selected_index(),
            1,
            "resize zelf verandert de selectie niet"
        );
        app.set_viewport_height(40);
        assert_eq!(app.selected_index(), 1);
    }

    #[test]
    fn record_at_reads_the_visible_window() {
        let app = sample_app();
        let ParsedRecord::Record(record) = app.record_at(1).unwrap() else {
            panic!("verwacht Record")
        };
        assert_eq!(record.message, "foo one");
        assert!(app.record_at(100).is_none());
    }

    #[test]
    fn empty_app_has_no_selected_record() {
        let app = App::new(Vec::new());
        assert_eq!(app.visible_len(), 0);
        assert!(app.selected_record().is_none());
    }

    #[test]
    fn set_filter_applies_immediately_like_the_picker_would() {
        // task.md T13: `--level`/`--channel` moeten dezelfde filtering geven als het
        // handmatig instellen via de kiezers (T8).
        let mut app = sample_app();
        app.set_filter(FilterSpec {
            channels: ChannelFilter::from_specs(["app"]),
            levels: Some(LevelFilter::parse(">=error").unwrap()),
            search: None,
        });
        assert_eq!(app.visible_len(), 1);
        let ParsedRecord::Record(record) = app.selected_record().unwrap() else {
            panic!("verwacht Record")
        };
        assert_eq!(record.message, "foo one");
    }
}
