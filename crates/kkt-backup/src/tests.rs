use super::*;
use kkt_clock::Span;

fn at(seconds: u32) -> Time {
    Time::from_parts(2026, 10, 28, 13, 15, seconds).expect("time")
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
