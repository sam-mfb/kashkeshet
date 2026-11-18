//! The report of a lane import: a line for each lane it touched, then the
//! totals.

/// What an import did to one lane.
pub(crate) enum Outcome {
    Created,
    Updated(i64),
    Unchanged(i64),
    RetiredSkipped,
    Retired,
    FileSkipped,
}

pub(crate) fn import_report<'a>(
    done: impl IntoIterator<Item = (&'a str, Outcome)>,
    folder: &str,
    commit: Option<&str>,
) -> String {
    let mut lines = String::new();
    let (mut created, mut updated, mut unchanged, mut retired, mut skipped, mut kept) =
        (0, 0, 0, 0, 0, 0);
    for (name, outcome) in done {
        lines.push_str(&match outcome {
            Outcome::Created => {
                created += 1;
                format!("created {name} (version 1)\n")
            }
            Outcome::Updated(version) => {
                updated += 1;
                format!("updated {name} (version {version})\n")
            }
            Outcome::Unchanged(version) => {
                unchanged += 1;
                format!("unchanged {name} (version {version})\n")
            }
            Outcome::RetiredSkipped => {
                skipped += 1;
                format!("left {name} alone: it is retired\n")
            }
            Outcome::Retired => {
                retired += 1;
                format!("retired {name}: it has no file in the folder\n")
            }
            Outcome::FileSkipped => {
                kept += 1;
                format!("left {name} alone: its file was skipped, so it is not retired\n")
            }
        });
    }
    let commit = match commit {
        Some(commit) => format!(" (git commit {commit})"),
        None => String::new(),
    };
    lines.push_str(&format!(
        "Imported from {}{commit}: {created} created, {updated} updated, {unchanged} unchanged",
        folder
    ));
    if skipped > 0 {
        lines.push_str(&format!(", {skipped} retired left alone"));
    }
    if retired > 0 {
        lines.push_str(&format!(", {retired} retired"));
    }
    if kept > 0 {
        lines.push_str(&format!(", {kept} left alone for a skipped file"));
    }
    lines.push('\n');
    lines
}

#[cfg(test)]
mod tests {
    use super::Outcome;

    #[test]
    fn the_import_report_names_each_lane_then_the_totals() {
        let done = [
            ("orchestrator", Outcome::Created),
            ("reviewer", Outcome::Updated(3)),
            ("builder", Outcome::Unchanged(2)),
            ("old", Outcome::RetiredSkipped),
            ("gone", Outcome::Retired),
            ("writer", Outcome::FileSkipped),
        ];
        assert_eq!(
            super::import_report(done, "/home/me/lanes", Some("abc123")),
            "created orchestrator (version 1)\n\
             updated reviewer (version 3)\n\
             unchanged builder (version 2)\n\
             left old alone: it is retired\n\
             retired gone: it has no file in the folder\n\
             left writer alone: its file was skipped, so it is not retired\n\
             Imported from /home/me/lanes (git commit abc123): 1 created, 1 updated, \
             1 unchanged, 1 retired left alone, 1 retired, 1 left alone for a skipped file\n"
        );
        assert_eq!(
            super::import_report([("a", Outcome::Created)], "/l", None),
            "created a (version 1)\nImported from /l: 1 created, 0 updated, 0 unchanged\n"
        );
        assert_eq!(
            super::import_report([], "/l", None),
            "Imported from /l: 0 created, 0 updated, 0 unchanged\n"
        );
    }
}
