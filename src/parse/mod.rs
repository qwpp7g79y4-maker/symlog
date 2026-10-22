//! Parsers voor Monolog-logregels (task.md T3; blueprint.md `parse/`).
//!
//! T3 voegt alleen [`line`] toe (LineFormatter, Monolog 1–3). De dispatch tussen
//! formaten (`detect`/`parse_record` uit blueprint.md) komt pas zinvol tot zijn recht
//! zodra [`json`](self) (T5) en de Raw-terugval (T6) er ook staan; tot die tijd roept
//! de aanroeper [`line::parse_line`] rechtstreeks aan. Een regel die niet aan het
//! standaardformaat voldoet, levert `None` op en blijft onbehandeld tot T6 ("Blijft"
//! in task.md T3: eigen LineFormatter-strings vallen terug op "onbekend formaat").

pub mod line;
