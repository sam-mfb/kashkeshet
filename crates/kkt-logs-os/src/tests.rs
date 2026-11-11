use super::*;
use kkt_clock::{Span, testing::FixedClock};
use std::time::Duration;

const POLICY: LogPolicy = LogPolicy {
    max_bytes: 100,
    rotated_files: 3,
    max_age: Duration::from_secs(24 * 60 * 60),
};

/// A fresh folder, a clock, and the log path in it. The clock is set in
/// 2000, so files written now count as new; a test makes a file old by
/// setting its modification time before the clock (see `make_old`). The
/// folder is removed when dropped, so a test keeps it to the end.
fn setup(name: &str) -> (crate::scratch::Dir, Arc<FixedClock>, PathBuf) {
    let dir = crate::scratch::Dir::new(&format!("kashkeshet-logs-{name}"));
    let start = Time::from_parts(2000, 1, 1, 0, 0, 0).unwrap();
    let clock = Arc::new(FixedClock::new(start));
    let path = dir.join("logs/kkt.log");
    (dir, clock, path)
}

/// Sets `path` as last written `hours` before the clock's time.
fn make_old(path: &Path, clock: &FixedClock, hours: i64) {
    let when = clock.now() - Span::hours(hours);
    let file = OpenOptions::new().write(true).open(path).expect("open");
    file.set_modified(when.into()).expect("set modified");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read")
}

fn line(n: usize) -> Vec<u8> {
    // 40 bytes, so two fit in 100 and a third rolls the file.
    format!("{n:038}\n\n").into_bytes()[..40].to_vec()
}

#[test]
fn the_file_rolls_over_at_the_size_limit() {
    let (_dir, clock, path) = setup("size");
    let mut log = RotatingFile::open(&path, POLICY, clock).expect("open");
    for n in 0..3 {
        log.write_all(&line(n)).expect("write");
    }
    let rolled = log.rolled(1);
    assert_eq!(std::fs::metadata(&rolled).expect("rolled").len(), 80);
    assert_eq!(read(&path).len(), 40);
    assert!(read(&path).starts_with(&format!("{:038}", 2)));
}

#[test]
fn only_so_many_rolled_files_are_kept() {
    let (_dir, clock, path) = setup("count");
    let mut log = RotatingFile::open(&path, POLICY, clock).expect("open");
    // Each pair of lines fills a file; twelve lines make five rollovers.
    for n in 0..12 {
        log.write_all(&line(n)).expect("write");
    }
    for n in 1..=POLICY.rotated_files {
        assert!(log.rolled(n).exists(), "{n}");
    }
    assert!(!log.rolled(POLICY.rotated_files + 1).exists());
    // The newest rolled file holds the lines just before the current ones.
    assert!(read(&log.rolled(1)).starts_with(&format!("{:038}", 8)));
}

#[test]
fn rolled_files_past_the_age_limit_are_deleted() {
    let (_dir, clock, path) = setup("age");
    let mut log = RotatingFile::open(&path, POLICY, clock.clone()).expect("open");
    for n in 0..3 {
        log.write_all(&line(n)).expect("write");
    }
    // The rolled file was last written a day and an hour ago. At the
    // next rollover it moves to .2 and, being too old, is deleted; the
    // file just rolled stays.
    make_old(&log.rolled(1), &clock, 25);
    for n in 3..5 {
        log.write_all(&line(n)).expect("write");
    }
    assert!(!log.rolled(2).exists());
    assert!(log.rolled(1).exists());
    // A rolled file just under the age stays.
    make_old(&log.rolled(1), &clock, 23);
    drop(log);
    let log = RotatingFile::open(&path, POLICY, clock.clone()).expect("reopen");
    assert!(log.rolled(1).exists());
    // Opening deletes one past the age.
    make_old(&log.rolled(1), &clock, 25);
    drop(log);
    let log = RotatingFile::open(&path, POLICY, clock).expect("reopen");
    assert!(!log.rolled(1).exists());
}

#[test]
fn a_running_log_deletes_old_files_without_rolling_over() {
    let (_dir, clock, path) = setup("running");
    let mut log = RotatingFile::open(&path, POLICY, clock.clone()).expect("open");
    for n in 0..3 {
        log.write_all(&line(n)).expect("write");
    }
    let rolled = log.rolled(1);
    // The rolled file goes past its age while the server keeps running.
    // A small write within the hour leaves it; one after the hour, still
    // well under the size limit, deletes it.
    make_old(&rolled, &clock, 25);
    clock.advance(Span::minutes(59));
    log.write_all(b"x\n").expect("write");
    assert!(rolled.exists());
    clock.advance(Span::minutes(1));
    log.write_all(b"x\n").expect("write");
    assert!(!rolled.exists());
}

#[test]
fn reopening_appends_and_counts_what_is_there() {
    let (_dir, clock, path) = setup("reopen");
    let mut log = RotatingFile::open(&path, POLICY, clock.clone()).expect("open");
    log.write_all(&line(0)).expect("write");
    log.write_all(&line(1)).expect("write");
    drop(log);
    let mut log = RotatingFile::open(&path, POLICY, clock).expect("reopen");
    log.write_all(&line(2)).expect("write");
    // The two earlier lines count toward the limit, so the third rolls.
    assert_eq!(std::fs::metadata(log.rolled(1)).expect("rolled").len(), 80);
}

#[test]
fn a_write_larger_than_the_limit_lands_whole() {
    let (_dir, clock, path) = setup("large");
    let mut log = RotatingFile::open(&path, POLICY, clock).expect("open");
    let big = vec![b'x'; 150];
    log.write_all(&big).expect("write");
    assert_eq!(read(&path).len(), 150);
    assert!(!log.rolled(1).exists());
}
