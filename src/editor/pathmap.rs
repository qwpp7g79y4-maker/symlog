//! Padmapping: containerpad naar lokaal pad (task.md T9; blueprint.md
//! `editor::pathmap`).
//!
//! Een `--map`-regel bindt een containerprefix aan een lokale prefix, bijvoorbeeld
//! `/var/www=/home/u/proj`. Meerdere regels mogen overlappen (`/var/www` en
//! `/var/www/html/src` tegelijk geconfigureerd); de **langste** matchende prefix wint
//! (task.md T9 "Doet": "meerdere, langste prefix wint"). Een prefix matcht alleen op
//! een padcomponentgrens: `/var/www` matcht `/var/www/html/src/Foo.php` maar niet
//! `/var/www-other/Foo.php`, zodat een toevallig gedeeld tekstvoorvoegsel geen record
//! verkeerd mapt.

/// Eén `--map CONTAINER=LOCAL`-regel, ook geldig als `map`-key in `config.toml`
/// (task.md T9 "Doet": "ook uit config").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntry {
    pub container: String,
    pub local: String,
}

impl MapEntry {
    /// Parseert `"CONTAINER=LOCAL"` (de vorm achter `--map` op de command line).
    /// `None` als er geen `=` in staat, of als één van beide kanten leeg is: beide
    /// zijn geen geldig pad en zouden stil alles (of niets) matchen.
    pub fn parse_spec(spec: &str) -> Option<MapEntry> {
        let (container, local) = spec.split_once('=')?;
        if container.is_empty() || local.is_empty() {
            return None;
        }
        Some(MapEntry {
            container: container.to_string(),
            local: local.to_string(),
        })
    }
}

/// Een verzameling padmappingregels (task.md T9; blueprint.md `editor::pathmap`).
#[derive(Debug, Clone, Default)]
pub struct PathMap {
    entries: Vec<MapEntry>,
}

impl PathMap {
    pub fn new(entries: Vec<MapEntry>) -> PathMap {
        PathMap { entries }
    }

    /// Of `prefix` op een padcomponentgrens voorkomt in `path`: een exacte match, of
    /// gevolgd door `/`. Voorkomt dat `/var/www` ten onrechte `/var/www-other/x.php`
    /// matcht.
    fn matches_at_boundary(path: &str, prefix: &str) -> bool {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    /// Zoekt het lokale pad voor `path` via de langst matchende prefix. `None` als
    /// geen enkele regel matcht; de aanroeper houdt dan het pad ongewijzigd aan (geen
    /// mapping betekent niet per se "bestand ontbreekt": het pad kan al lokaal zijn).
    pub fn map(&self, path: &str) -> Option<String> {
        let best = self
            .entries
            .iter()
            .filter(|entry| Self::matches_at_boundary(path, &entry.container))
            .max_by_key(|entry| entry.container.len())?;
        let rest = &path[best.container.len()..];
        Some(format!("{}{}", best.local, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spec_splits_on_the_first_equals_sign() {
        let entry = MapEntry::parse_spec("/var/www=/home/u/proj").expect("geldige spec");
        assert_eq!(entry.container, "/var/www");
        assert_eq!(entry.local, "/home/u/proj");
    }

    #[test]
    fn parse_spec_rejects_missing_equals_sign_and_empty_sides() {
        assert_eq!(MapEntry::parse_spec("no-equals-sign"), None);
        assert_eq!(MapEntry::parse_spec("=/home/u/proj"), None);
        assert_eq!(MapEntry::parse_spec("/var/www="), None);
    }

    #[test]
    fn map_rewrites_the_matching_prefix() {
        let map = PathMap::new(vec![MapEntry::parse_spec("/var/www=/home/u/proj").unwrap()]);
        assert_eq!(
            map.map("/var/www/html/src/Foo.php"),
            Some("/home/u/proj/html/src/Foo.php".to_string())
        );
    }

    #[test]
    fn map_returns_none_when_nothing_matches() {
        let map = PathMap::new(vec![MapEntry::parse_spec("/var/www=/home/u/proj").unwrap()]);
        assert_eq!(map.map("/already/local/Foo.php"), None);
    }

    #[test]
    fn map_requires_a_path_component_boundary() {
        // "/var/www-other" deelt een tekstvoorvoegsel met "/var/www" maar is een ander
        // pad: de "/" na de prefix moet er staan, anders geen match.
        let map = PathMap::new(vec![MapEntry::parse_spec("/var/www=/home/u/proj").unwrap()]);
        assert_eq!(map.map("/var/www-other/Foo.php"), None);
    }

    #[test]
    fn map_matches_the_container_prefix_exactly_with_no_remainder() {
        let map = PathMap::new(vec![MapEntry::parse_spec("/var/www=/home/u/proj").unwrap()]);
        assert_eq!(map.map("/var/www"), Some("/home/u/proj".to_string()));
    }

    #[test]
    fn overlapping_prefixes_let_the_longest_one_win() {
        // task.md T9 "Klaar als": "padmappingtests met overlappende prefixen".
        let map = PathMap::new(vec![
            MapEntry::parse_spec("/var/www=/home/u/proj-root").unwrap(),
            MapEntry::parse_spec("/var/www/html/src=/home/u/proj-src").unwrap(),
        ]);
        assert_eq!(
            map.map("/var/www/html/src/Foo.php"),
            Some("/home/u/proj-src/Foo.php".to_string()),
            "de langere, specifiekere prefix wint"
        );
        assert_eq!(
            map.map("/var/www/other/Bar.php"),
            Some("/home/u/proj-root/other/Bar.php".to_string()),
            "buiten de specifiekere prefix geldt de kortere nog steeds"
        );
    }

    #[test]
    fn overlapping_prefixes_are_order_independent() {
        // Dezelfde twee regels, maar nu de specifiekere eerst geconfigureerd: de
        // uitkomst mag niet van de volgorde in de lijst afhangen.
        let map = PathMap::new(vec![
            MapEntry::parse_spec("/var/www/html/src=/home/u/proj-src").unwrap(),
            MapEntry::parse_spec("/var/www=/home/u/proj-root").unwrap(),
        ]);
        assert_eq!(
            map.map("/var/www/html/src/Foo.php"),
            Some("/home/u/proj-src/Foo.php".to_string())
        );
    }

    #[test]
    fn multiple_unrelated_prefixes_each_map_independently() {
        // task.md F17: frames onder /var/www/html/src én /app/vendor in hetzelfde
        // bestand, allebei met hun eigen mapping.
        let map = PathMap::new(vec![
            MapEntry::parse_spec("/var/www/html/src=/home/u/proj/src").unwrap(),
            MapEntry::parse_spec("/app/vendor=/home/u/proj/vendor").unwrap(),
        ]);
        assert_eq!(
            map.map("/var/www/html/src/Controller/HomeController.php"),
            Some("/home/u/proj/src/Controller/HomeController.php".to_string())
        );
        assert_eq!(
            map.map("/app/vendor/symfony/http-kernel/HttpKernel.php"),
            Some("/home/u/proj/vendor/symfony/http-kernel/HttpKernel.php".to_string())
        );
    }

    #[test]
    fn empty_pathmap_never_matches() {
        let map = PathMap::default();
        assert_eq!(map.map("/var/www/Foo.php"), None);
    }
}
