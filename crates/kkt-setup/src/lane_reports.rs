//! The report of a lane import, and the Docker limit when no install is present.

/// The plain import outcome the program adapts from the operator API.
pub enum Outcome {
    Created,
    Updated(i64),
    Unchanged(i64),
    RetiredSkipped,
    Retired,
    FileSkipped,
}

pub fn docker_limit(
    present: bool,
    default: usize,
    read: impl FnOnce() -> Result<usize, String>,
) -> Result<usize, String> {
    if !present {
        return Ok(default);
    }
    read().map_err(|e| format!("could not read the lane text limit of kkt in Docker: {e}"))
}

pub fn import_report<'a>(
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
    #[test]
    fn a_new_docker_install_uses_the_default_without_reading_container_settings() {
        assert_eq!(
            super::docker_limit(false, 123, || panic!("no install to read")),
            Ok(123)
        );
        assert_eq!(
            super::docker_limit(true, 123, || Err("unreadable".into())),
            Err("could not read the lane text limit of kkt in Docker: unreadable".into())
        );
    }
}
