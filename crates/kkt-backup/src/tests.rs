use super::*;
use kkt_clock::Span;

fn at(seconds: u32) -> Time {
    Time::from_parts(2026, 10, 28, 13, 15, seconds).expect("time")
}

#[test]
fn the_about_file_records_the_highest_message_id_only_when_read() {
    let contents = |highest_message_id| Contents {
        database: Path::new("/data/kashkeshet.db"),
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
        highest_message_id,
    };
    let with = layout(at(0), "1.0.0", &contents(Some(41)));
    assert!(
        with.about().contains("highest message ID: 41\n"),
        "{}",
        with.about()
    );
    let without = layout(at(0), "1.0.0", &contents(None));
    assert!(
        !without.about().contains("highest message ID"),
        "{}",
        without.about()
    );
}

#[test]
fn names_say_the_version_and_sort_by_time() {
    assert_eq!(name(at(0), "0.3.0"), "kkt-20261028T131500.000Z-v0.3.0");
    let later = at(0) + Span::milliseconds(1);
    assert!(name(at(0), "0.9.0") < name(later, "0.3.0"));
    assert!(is_backup(&name(at(0), "0.3.0")));
    for other in [
        "kkt-20261028T131500.000Z-v0.3.0.partial",
        "kkt-yesterday-v0.3.0",
        "kkt-20261028T131500.000Z",
        "notes.txt",
    ] {
        assert!(!is_backup(other), "{other}");
    }
}
