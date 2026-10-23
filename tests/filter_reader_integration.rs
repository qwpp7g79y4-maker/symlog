//! Integratietest voor `filter::filtered_offsets` (task.md T7; blueprint.md
//! datastroom: "FilterSpec ──► Filter-thread: loopt index af, parst, levert
//! gefilterde offset-lijst incrementeel"). De losse matching-logica staat al grondig
//! getest in `src/filter.rs`; dit bestand bewijst dat die logica ook correct bedraad
//! is bovenop de echte [`symlog::reader::Reader`] (index + `pread`), op F1.

use std::path::Path;
use symlog::filter::{ChannelFilter, FilterSpec, LevelFilter};
use symlog::reader::Reader;

fn open_f1() -> Reader {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/line-basic.log");
    let mut reader = Reader::open(&path).expect("open F1");
    reader.wait_until_indexed().expect("index F1");
    reader
}

#[test]
fn filtered_offsets_returns_exactly_the_matching_record_offsets() {
    let reader = open_f1();
    let spec = FilterSpec {
        channels: ChannelFilter::from_specs(["!doctrine"]),
        levels: Some(LevelFilter::parse(">=warning").unwrap()),
        search: None,
    };
    let actual = symlog::filter::filtered_offsets(&reader, &spec).expect("filtert zonder I/O-fout");

    // F1 (zie tests/fixtures/line-basic.log): index 0 app.DEBUG, 1 request.INFO,
    // 2 doctrine.NOTICE vallen af; 3 security.WARNING, 4 php.ERROR,
    // 5 console.CRITICAL, 6 app.ALERT en 7 request.EMERGENCY matchen (doctrine.NOTICE
    // valt af op kanaal, de rest op niveau: alles onder WARNING).
    let expected: Vec<u64> = [3, 4, 5, 6, 7]
        .iter()
        .map(|&i| reader.index().offset(i).expect("F1 heeft 8 records"))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn filtered_offsets_with_default_spec_returns_every_offset() {
    let reader = open_f1();
    let offsets = symlog::filter::filtered_offsets(&reader, &FilterSpec::default())
        .expect("filtert zonder I/O-fout");
    assert_eq!(offsets.len(), reader.record_count());
}
