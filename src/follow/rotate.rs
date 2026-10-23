//! Detectie van groei / rotatie / afkappen / verdwijnen op basis van `(dev, ino,
//! size)` (task.md T10; blueprint.md `follow::rotate`).
//!
//! [`detect`] is de pure beslisfunctie uit blueprint.md "Follow en rotatie (T10)":
//! gegeven de identiteit en grootte die bij de *vorige* waarneming hoorden, en wat
//! `stat(pad)` nu teruggeeft, welke van de vier gevallen is dit? De functie raakt
//! zelf geen bestandssysteem aan (dat doet [`FileId::of_metadata`]/
//! [`crate::follow::Follower`]), zodat elke combinatie van dev/ino/size
//! deterministisch getest kan worden zonder echte bestanden.

use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

/// Identiteit van een bestand op het filesystem: `(dev, ino)`. Twee open handles met
/// hetzelfde `FileId` wijzen naar hetzelfde onderliggende bestand, ook als het pad
/// ertussen hernoemd is (blueprint.md: "dev+inode+size").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileId {
    pub dev: u64,
    pub ino: u64,
}

impl FileId {
    pub fn of_metadata(meta: &Metadata) -> FileId {
        FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        }
    }
}

/// Wat er sinds de vorige waarneming veranderd is (blueprint.md "Follow en rotatie
/// (T10)", stappen 1 t/m 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Stap 4: zelfde bestand, `size > indexed_end`. Nieuwe bytes zitten tussen
    /// `from` (= de vorige `indexed_end`) en `to` (= de nieuwe grootte).
    Grown { from: u64, to: u64 },
    /// Stap 2: `(dev, ino)` is veranderd (bijvoorbeeld rename-rotatie door logrotate,
    /// of het bestand is verwijderd en onder hetzelfde pad opnieuw aangemaakt).
    Rotated,
    /// Stap 3: zelfde `(dev, ino)`, maar `size < indexed_end` (bijvoorbeeld
    /// `copytruncate` of `> file`).
    Truncated,
    /// Stap 1: `stat(pad)` vindt niets (meer).
    Missing,
    /// Niets veranderd sinds de vorige waarneming: zelfde identiteit, zelfde grootte.
    Unchanged,
}

/// Bepaalt [`Change`] op basis van de identiteit/grootte bij de vorige waarneming
/// (`prev_id`, `indexed_end`) en wat `stat(pad)` nu teruggeeft (`current`, `None` als
/// het pad niet bestaat). Volgt de volgorde uit blueprint.md precies: eerst
/// verdwijnen, dan identiteitswissel (rotatie gaat voor afkappen/groei, ook als de
/// nieuwe grootte toevallig kleiner is dan de oude), dan afkappen, dan groei.
pub fn detect(prev_id: FileId, indexed_end: u64, current: Option<(FileId, u64)>) -> Change {
    let Some((id, size)) = current else {
        return Change::Missing;
    };
    if id != prev_id {
        return Change::Rotated;
    }
    if size < indexed_end {
        return Change::Truncated;
    }
    if size > indexed_end {
        return Change::Grown {
            from: indexed_end,
            to: size,
        };
    }
    Change::Unchanged
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: FileId = FileId { dev: 1, ino: 100 };
    const B: FileId = FileId { dev: 1, ino: 200 };

    #[test]
    fn missing_when_stat_finds_nothing() {
        assert_eq!(detect(A, 50, None), Change::Missing);
    }

    #[test]
    fn rotated_when_the_inode_changes_even_if_the_new_file_is_bigger() {
        assert_eq!(detect(A, 50, Some((B, 1_000))), Change::Rotated);
    }

    #[test]
    fn rotated_when_the_device_changes_but_the_inode_number_happens_to_match() {
        let same_ino_other_dev = FileId { dev: 2, ino: 100 };
        assert_eq!(
            detect(A, 50, Some((same_ino_other_dev, 50))),
            Change::Rotated
        );
    }

    #[test]
    fn truncated_when_same_file_shrinks() {
        assert_eq!(detect(A, 50, Some((A, 10))), Change::Truncated);
    }

    #[test]
    fn truncated_to_exactly_zero_is_still_truncated_not_unchanged() {
        assert_eq!(detect(A, 50, Some((A, 0))), Change::Truncated);
    }

    #[test]
    fn grown_when_same_file_gets_bigger() {
        assert_eq!(
            detect(A, 50, Some((A, 80))),
            Change::Grown { from: 50, to: 80 }
        );
    }

    #[test]
    fn unchanged_when_nothing_moved() {
        assert_eq!(detect(A, 50, Some((A, 50))), Change::Unchanged);
    }

    #[test]
    fn unchanged_on_an_empty_file_observed_twice() {
        assert_eq!(detect(A, 0, Some((A, 0))), Change::Unchanged);
    }
}
