//! Tekent [`App`] op een [`ratatui::Frame`] (task.md T8; blueprint.md `tui::view`):
//! lijstweergave, detailpaneel, kiezers en help-overlay.
//!
//! Elke regel tekst gaat eerst door [`sanitize_for_display`] (task.md T6: "Dit geldt
//! voor elke weergegeven tekst") en wordt daarna met [`crate::tui::width`] grafeemveilig
//! afgekapt tot de kolombreedte die er nog over is (task.md T8 "Doet": emoji/CJK = 2
//! kolommen, combining = 0, nooit midden in een grafeem afkappen); tabs en andere
//! besturingstekens worden door die sanering al vóór de breedteberekening vervangen
//! door hun zichtbare, enkel-kolombrede tegenhanger, dus `width` hoeft tabs zelf niet
//! apart te behandelen.
//!
//! [`render`] zet aan het eind ook [`App::set_viewport_height`]: de echte hoogte van de
//! lijst (na aftrek van de statusbalk, en van het detailpaneel als dat open is) is pas
//! na de lay-out bekend, en `Ctrl-d`/`Ctrl-u` en de scroll-klem in [`crate::tui::app`]
//! hebben die hoogte nodig. Dat maakt resize vanzelf correct (task.md T8 "Klaar als":
//! de selectie blijft op hetzelfde record): een nieuwe framegrootte roept gewoon
//! opnieuw [`render`] aan, zonder dat de selectie zelf hier aangeraakt wordt.

use super::app::{App, Mode, Picker};
use super::width::{pad_to_width, truncate_to_width};
use crate::model::{self, Level};
use crate::parse::raw::{sanitize_for_display, ParsedRecord};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

const TIME_WIDTH: usize = 8;
const LEVEL_WIDTH: usize = 9;
const CHANNEL_WIDTH: usize = 8;
/// Aantal spaties tussen de vier kolommen (tijd, niveau, kanaal, message).
const COLUMN_GAPS: usize = 3;

/// Of kleur getoond mag worden (task.md T8 "Blijft": "geen kleur bij `NO_COLOR`"),
/// zie <https://no-color.org/>: elke, ook lege, waarde van `NO_COLOR` schakelt kleur
/// uit. Deze functie leest de omgeving maar één keer per [`render`]-aanroep; de rest
/// van deze module neemt het resultaat als parameter aan in plaats van zelf opnieuw te
/// lezen, zodat de kleurlogica zelf ([`level_style`]) puur en zonder gedeelde,
/// proces-globale toestand te testen is (zie de tests onderaan dit bestand).
fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

fn level_style(level: Level, colors_enabled: bool) -> Style {
    if !colors_enabled {
        return Style::default();
    }
    let color = match level {
        Level::Debug => Color::DarkGray,
        Level::Info => Color::Blue,
        Level::Notice => Color::Cyan,
        Level::Warning => Color::Yellow,
        Level::Error => Color::Red,
        Level::Critical => Color::LightRed,
        Level::Alert => Color::Magenta,
        Level::Emergency => Color::LightMagenta,
    };
    let style = Style::default().fg(color);
    if level >= Level::Error {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// HH:MM:SS-gedeelte van de ruwe `%datetime%`-tekst (task.md T3: zowel Monolog 1
/// `Y-m-d H:i:s` als Monolog 2/3 `Y-m-d\TH:i:s...` hebben de tijd op dezelfde
/// bytepositie). De volledige datetime met offset blijft zichtbaar in het
/// detailpaneel; dit is alleen de compacte lijstkolom.
fn time_column(datetime: &str) -> &str {
    datetime.get(11..19).unwrap_or(datetime)
}

fn record_line(
    parsed: &ParsedRecord,
    selected: bool,
    width: usize,
    colors_enabled: bool,
) -> Line<'static> {
    let line = match parsed {
        ParsedRecord::Record(record) => {
            let time = pad_to_width(
                &sanitize_for_display(time_column(&record.datetime)),
                TIME_WIDTH,
            );
            let level_text = pad_to_width(record.level.as_str(), LEVEL_WIDTH);
            let channel = pad_to_width(&sanitize_for_display(&record.channel), CHANNEL_WIDTH);
            let fixed_width = TIME_WIDTH + LEVEL_WIDTH + CHANNEL_WIDTH + COLUMN_GAPS;
            let remaining = width.saturating_sub(fixed_width);
            let first_line = record.message.split('\n').next().unwrap_or("");
            let message = truncate_to_width(&sanitize_for_display(first_line), remaining);
            Line::from(vec![
                Span::raw(format!("{time} ")),
                Span::styled(level_text, level_style(record.level, colors_enabled)),
                Span::raw(format!(" {channel} ")),
                Span::raw(message),
            ])
        }
        ParsedRecord::Raw(raw) => {
            let fixed_width = TIME_WIDTH + LEVEL_WIDTH + CHANNEL_WIDTH + COLUMN_GAPS;
            let marker_style = if colors_enabled {
                Style::default().fg(Color::Magenta)
            } else {
                Style::default()
            };
            let marker = pad_to_width(&format!("RAW[{}]", raw.reason.as_str()), fixed_width);
            let remaining = width.saturating_sub(fixed_width);
            let first_line = raw.text.split('\n').next().unwrap_or("");
            let text = truncate_to_width(&sanitize_for_display(first_line), remaining);
            Line::from(vec![Span::styled(marker, marker_style), Span::raw(text)])
        }
    };
    if selected {
        line.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn render_list(frame: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    let height = area.height as usize;
    let top = app.top();
    let colors_enabled = colors_enabled();
    let lines: Vec<Line<'static>> = (top..top + height)
        .filter_map(|row| {
            app.record_at(row).map(|record| {
                record_line(record, row == app.selected_index(), width, colors_enabled)
            })
        })
        .collect();
    let title = format!(
        " symlog — {}/{} records{} ",
        app.visible_len().min(app.selected_index() + 1),
        app.visible_len(),
        if app.follow() { " — follow" } else { "" }
    );
    let block = Block::default().borders(Borders::NONE).title(title);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn frame_line(frame_ref: &model::Frame, selected: bool) -> Line<'static> {
    let location = match (&frame_ref.path, frame_ref.line) {
        (Some(path), Some(line)) => format!("{path}:{line}"),
        _ => "[internal function]".to_string(),
    };
    let text = format!("{} ({location})", sanitize_for_display(&frame_ref.func));
    let line = Line::raw(text);
    if selected {
        line.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn render_detail(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" detail ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(record) = app.selected_record() else {
        frame.render_widget(Paragraph::new("geen record geselecteerd"), inner);
        return;
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    match record {
        ParsedRecord::Record(record) => {
            lines.push(Line::raw(sanitize_for_display(&format!(
                "{} {}.{}",
                record.datetime,
                record.channel,
                record.level.as_str()
            ))));
            for message_line in record.message.split('\n') {
                lines.push(Line::raw(sanitize_for_display(message_line)));
            }
            lines.push(Line::raw(""));
            lines.push(Line::raw(format!("context: {}", record.context)));
            lines.push(Line::raw(format!("extra: {}", record.extra)));
            if !record.frames.is_empty() {
                lines.push(Line::raw(""));
                lines.push(Line::raw("frames:"));
                for (i, frame_ref) in record.frames.iter().enumerate() {
                    lines.push(frame_line(frame_ref, i == app.detail_frame_index()));
                }
            }
        }
        ParsedRecord::Raw(raw) => {
            lines.push(Line::raw(format!("RAW[{}]", raw.reason.as_str())));
            for raw_line in raw.text.split('\n') {
                lines.push(Line::raw(sanitize_for_display(raw_line)));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let text = match app.mode() {
        Mode::Search => format!("/{}", app.search_input()),
        _ if !app.status().is_empty() => app.status().to_string(),
        _ => "j/k gg/G Ctrl-d/u / n/N c l F o y ? q".to_string(),
    };
    let width = area.width as usize;
    let line = Line::raw(truncate_to_width(&text, width));
    frame.render_widget(Paragraph::new(line), area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn render_picker(frame: &mut Frame, area: Rect, title: &str, picker: &Picker) {
    let popup = centered_rect(60, 60, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines: Vec<Line<'static>> = picker
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let mark = if picker.selected.get(i).copied().unwrap_or(false) {
                "[x]"
            } else {
                "[ ]"
            };
            let text = format!("{mark} {item}");
            let line = Line::raw(text);
            if i == picker.cursor {
                line.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

const HELP_TEXT: &str = "j/k   regel op/neer\n\
gg/G  eerste/laatste record\n\
Ctrl-d/u  halve pagina op/neer\n\
/ n N  zoeken, volgende/vorige match\n\
c     kanaalkiezer\n\
l     niveaukiezer\n\
F     follow aan/uit\n\
Enter/Tab  detailpaneel\n\
o     open frame\n\
y     kopieer bestand:regel bij afsluiten\n\
?     deze hulp\n\
q / Ctrl-c  stoppen";

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" help (willekeurige toets sluit) ");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let lines: Vec<Line<'static>> = HELP_TEXT
        .lines()
        .map(|l| Line::raw(l.to_string()))
        .collect();
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Left), inner);
}

/// Tekent de volledige TUI-toestand (task.md T8 "Doet"): lijst, detailpaneel (als
/// open), statusbalk, en de kiezer- of help-overlay in de huidige modus.
pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let (main_area, status_area) = (vertical[0], vertical[1]);

    let list_area = if app.detail_open() {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_area);
        render_detail(frame, app, split[1]);
        split[0]
    } else {
        main_area
    };

    render_list(frame, app, list_area);
    render_status(frame, app, status_area);

    match app.mode() {
        Mode::ChannelPicker => render_picker(
            frame,
            area,
            " kanalen (spatie/enter/esc) ",
            app.channel_picker(),
        ),
        Mode::LevelPicker => render_picker(
            frame,
            area,
            " niveaus (spatie/enter/esc) ",
            app.level_picker(),
        ),
        Mode::Help => render_help(frame, area),
        Mode::Normal | Mode::Search => {}
    }

    app.set_viewport_height(list_area.height as usize);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Record;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;
    use std::fs;
    use std::path::Path;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn golden_path(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(name)
    }

    fn load_records(fixture: &str) -> Vec<ParsedRecord> {
        let content = fs::read_to_string(fixture_path(fixture)).expect("fixture leesbaar");
        crate::parse::split_records(&content)
            .into_iter()
            .map(|block| crate::parse::parse_record_lossy(block.as_bytes()))
            .collect()
    }

    /// Zet een [`TestBackend`]-buffer om naar platte tekstregels, voor golden-vergelijking
    /// (zelfde aanpak als de bestaande golden-tests in `tests/`, task.md T3-T7).
    fn buffer_to_lines(buffer: &Buffer) -> Vec<String> {
        let area = buffer.area;
        (0..area.height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..area.width {
                    let cell = buffer
                        .cell((area.x + x, area.y + y))
                        .expect("binnen de buffer");
                    row.push_str(cell.symbol());
                }
                row.trim_end().to_string()
            })
            .collect()
    }

    fn render_to_lines(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("TestBackend init");
        terminal.draw(|f| render(f, app)).expect("draw");
        buffer_to_lines(terminal.backend().buffer())
    }

    fn assert_matches_golden(name: &str, lines: &[String]) {
        let path = golden_path(name);
        let actual = lines.join("\n");
        if std::env::var_os("SYMLOG_BLESS_GOLDEN").is_some() {
            fs::write(&path, format!("{actual}\n")).expect("golden schrijven");
            return;
        }
        let expected = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("ontbrekend golden-bestand: {path:?}"));
        assert_eq!(
            actual,
            expected.trim_end_matches('\n'),
            "golden {name} komt niet overeen"
        );
    }

    // --- task.md T8 "Klaar als": snapshot-tests op 80x24, 40x10 en 200x50 ---

    #[test]
    fn snapshot_f1_line_basic_80x24() {
        let mut app = App::new(load_records("line-basic.log"));
        let lines = render_to_lines(&mut app, 80, 24);
        assert_matches_golden("tui_f1_80x24.txt", &lines);
    }

    #[test]
    fn snapshot_f1_line_basic_40x10() {
        let mut app = App::new(load_records("line-basic.log"));
        let lines = render_to_lines(&mut app, 40, 10);
        assert_matches_golden("tui_f1_40x10.txt", &lines);
    }

    #[test]
    fn snapshot_f1_line_basic_200x50() {
        let mut app = App::new(load_records("line-basic.log"));
        let lines = render_to_lines(&mut app, 200, 50);
        assert_matches_golden("tui_f1_200x50.txt", &lines);
    }

    #[test]
    fn snapshot_f10_wide_80x24() {
        let mut app = App::new(load_records("wide.log"));
        let lines = render_to_lines(&mut app, 80, 24);
        assert_matches_golden("tui_f10_80x24.txt", &lines);
    }

    #[test]
    fn snapshot_f10_wide_40x10() {
        let mut app = App::new(load_records("wide.log"));
        let lines = render_to_lines(&mut app, 40, 10);
        assert_matches_golden("tui_f10_40x10.txt", &lines);
    }

    #[test]
    fn snapshot_f10_wide_200x50() {
        let mut app = App::new(load_records("wide.log"));
        let lines = render_to_lines(&mut app, 200, 50);
        assert_matches_golden("tui_f10_200x50.txt", &lines);
    }

    #[test]
    fn snapshot_f3_stacktrace_with_detail_open_80x24() {
        let mut app = App::new(load_records("line-stacktrace.log"));
        app.toggle_detail();
        let lines = render_to_lines(&mut app, 80, 24);
        assert_matches_golden("tui_f3_80x24.txt", &lines);
    }

    #[test]
    fn snapshot_f3_stacktrace_with_detail_open_40x10() {
        let mut app = App::new(load_records("line-stacktrace.log"));
        app.toggle_detail();
        let lines = render_to_lines(&mut app, 40, 10);
        assert_matches_golden("tui_f3_40x10.txt", &lines);
    }

    #[test]
    fn snapshot_f3_stacktrace_with_detail_open_200x50() {
        let mut app = App::new(load_records("line-stacktrace.log"));
        app.toggle_detail();
        let lines = render_to_lines(&mut app, 200, 50);
        assert_matches_golden("tui_f3_200x50.txt", &lines);
    }

    // --- task.md T8 "Klaar als": resize 80x24 -> 30x8 -> 120x40 houdt de selectie ---

    #[test]
    fn resize_keeps_selection_on_the_same_record_and_never_panics() {
        let mut app = App::new(load_records("line-basic.log"));
        app.move_selection(3);
        let selected_before = format!("{:?}", app.selected_record());

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("init");
        terminal.draw(|f| render(f, &mut app)).expect("draw 80x24");
        assert_eq!(format!("{:?}", app.selected_record()), selected_before);

        terminal.backend_mut().resize(30, 8);
        terminal.draw(|f| render(f, &mut app)).expect("draw 30x8");
        assert_eq!(format!("{:?}", app.selected_record()), selected_before);

        terminal.backend_mut().resize(120, 40);
        terminal.draw(|f| render(f, &mut app)).expect("draw 120x40");
        assert_eq!(format!("{:?}", app.selected_record()), selected_before);
    }

    #[test]
    fn render_on_an_empty_app_does_not_panic() {
        let mut app = App::new(Vec::new());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("init");
        terminal.draw(|f| render(f, &mut app)).expect("draw");
    }

    #[test]
    fn error_level_is_rendered_in_red() {
        // Rechtstreeks op `record_line` getest (met `colors_enabled` expliciet als
        // parameter), niet via een volledige `render`-aanroep die de echte `NO_COLOR`
        // omgevingsvariabele leest: die kan in de testomgeving zelf al gezet zijn (ook
        // als lege waarde, zie <https://no-color.org/>), wat dit geval dan zou
        // overslaan in plaats van testen.
        let record = ParsedRecord::Record(Record {
            datetime: "2026-01-15T10:00:00.000000+01:00".to_string(),
            ts: None,
            channel: "app".to_string(),
            level: Level::Error,
            message: "boom".to_string(),
            context: serde_json::json!([]),
            extra: serde_json::json!([]),
            frames: Vec::new(),
        });
        let line = record_line(&record, false, 80, true);
        let level_span = &line.spans[1];
        assert_eq!(level_span.content.trim(), "ERROR");
        assert_eq!(level_span.style.fg, Some(Color::Red));
    }

    #[test]
    fn level_style_without_color_is_plain() {
        // Geen omgevingsvariabele-mutatie hier (task.md T8 "Blijft": "geen kleur bij
        // `NO_COLOR`"): `cargo test` draait standaard op meerdere threads, en
        // proces-globale env-mutatie zou dan racen met elke andere test die tekent
        // (zoals `error_level_is_rendered_in_red` hierboven). `level_style` neemt de
        // uitkomst van die afweging daarom als parameter aan; alleen [`colors_enabled`]
        // zelf leest `NO_COLOR`, en dat is een ondeelbare, triviale ja/nee-lezing die
        // geen eigen test nodig heeft.
        assert_eq!(level_style(Level::Error, false), Style::default());
        assert_eq!(level_style(Level::Debug, false), Style::default());
    }

    #[test]
    fn level_style_with_color_sets_the_expected_colors() {
        assert_eq!(level_style(Level::Error, true).fg, Some(Color::Red));
        assert_eq!(level_style(Level::Debug, true).fg, Some(Color::DarkGray));
    }
}
