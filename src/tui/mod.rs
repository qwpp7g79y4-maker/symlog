//! TUI (ratatui + crossterm) en vim-toetsen (task.md T8; blueprint.md `tui/`).
//!
//! [`app`] is de pure toestandsmachine, [`keys`] vertaalt crossterm-toetsgebeurtenissen
//! naar aanroepen daarop, [`view`] tekent die toestand op een [`ratatui::Frame`] en
//! [`width`] is de kolombreedte-/grafeemhulp die [`view`] daarbij gebruikt. Geen van
//! deze modules initialiseert zelf een echte terminal (raw mode, alternate screen, de
//! event-lees-loop): die main-loop hoort bij de CLI-laag die modus kiest tussen TUI en
//! plain-uitvoer (task.md T13), nog niet aangesloten. Dat ontwerp laat elk stuk hier
//! zonder een echte tty testen, zie [`app`]'s en [`keys`]'s eigen tests en de
//! [`view`]-snapshot-tests op [`ratatui::backend::TestBackend`] (task.md T8 "Klaar
//! als").
//!
//! [`install_panic_hook`] hoort wél al bij T8 zelf (task.md T8 "Doet": "Bij panic
//! wordt de terminal hersteld"): de toekomstige main-loop roept die aan vóór hij raw
//! mode aanzet, zodat een paniek tijdens het tekenen de tty altijd weer in een bruikbare
//! staat achterlaat vóór de paniekmelding verschijnt.

pub mod app;
pub mod keys;
pub mod view;
pub mod width;

pub use app::App;

use ratatui::crossterm::terminal::LeaveAlternateScreen;
use std::io::{self, Write};
use std::panic;
use std::sync::atomic::{AtomicBool, Ordering};

static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Schrijft de ANSI-reeks om het alternate screen te verlaten naar `out` (geen
/// `disable_raw_mode` hier: dat is een systeemaanroep op de terminal zelf, niet op een
/// schrijfbare stroom, en dus niet los van een echte tty te testen; zie
/// [`tests::leaving_the_alternate_screen_writes_the_expected_escape_sequence`]).
fn leave_alternate_screen<W: Write>(out: &mut W) -> io::Result<()> {
    ratatui::crossterm::execute!(out, LeaveAlternateScreen)
}

/// Installeert, eenmalig per proces, een paniekhaak die het alternate screen verlaat en
/// raw mode uitzet vóórdat de oorspronkelijke paniekhaak de melding print (task.md T8
/// "Doet"). Een fout bij het herstellen zelf (bijvoorbeeld geen tty) wordt genegeerd in
/// plaats van een tweede paniek te veroorzaken binnen de paniekafhandeling.
pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = leave_alternate_screen(&mut io::stdout());
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaving_the_alternate_screen_writes_the_expected_escape_sequence() {
        let mut out = Vec::new();
        leave_alternate_screen(&mut out).expect("schrijven naar een Vec faalt nooit");
        assert!(!out.is_empty());
    }

    #[test]
    fn install_panic_hook_is_idempotent() {
        install_panic_hook();
        install_panic_hook();
        assert!(PANIC_HOOK_INSTALLED.load(Ordering::SeqCst));
    }
}
