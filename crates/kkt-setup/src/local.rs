//! The advice when no record has been saved.

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again.
pub fn recorded_setup(
    record: Option<crate::Record>,
    unfinished: impl FnOnce() -> Result<Option<crate::Unfinished>, String>,
    unrecorded: impl FnOnce() -> Result<Option<crate::Unrecorded>, (crate::Purpose, String)>,
) -> Result<crate::Record, String> {
    if let Some(record) = record {
        return Ok(record);
    }
    let no_record = "this machine has no setup record";
    // An unfinished setup is described by `kkt status`.
    match unfinished() {
        Ok(Some(_)) => return Err(format!("{no_record}; run `kkt setup` to finish it")),
        Ok(None) => {}
        Err(e) => return Err(format!("{no_record}; run `kkt setup` first ({e})")),
    }
    Err(match unrecorded() {
        Ok(Some(found)) => format!(
            "{no_record}. {} Run `kkt setup` to record it (`kkt setup --adopt` with no \
             terminal); it installs nothing. Then run this again.",
            found.summary()
        ),
        Ok(None) => format!("{no_record}; run `kkt setup` first"),
        Err((_, e)) => format!("{no_record}; run `kkt setup` first. {e}"),
    })
}

#[cfg(test)]
mod tests {
    //! What a command that needs the setup record says without one, pinned
    //! exactly.

    use super::*;
    use crate::{Mode, Purpose, Unfinished, Unrecorded};
    use std::path::Path;

    fn note() -> Unfinished {
        Unfinished::parse(
            include_str!("../tests/v0_10_0/unfinished-docker-tailscale.toml"),
            Path::new("/s/unfinished.toml"),
        )
        .expect("a note")
    }

    fn said(
        unfinished: Result<Option<Unfinished>, String>,
        unrecorded: Result<Option<Unrecorded>, (Purpose, String)>,
    ) -> String {
        recorded_setup(None, || unfinished, || unrecorded)
            .err()
            .expect("no record")
            .to_string()
    }

    #[test]
    fn every_no_record_sentence_says_what_it_always_said() {
        assert_eq!(
            said(Ok(Some(note())), Ok(None)),
            "this machine has no setup record; run `kkt setup` to finish it"
        );
        assert_eq!(
            said(Err("the note is unreadable".into()), Ok(None)),
            "this machine has no setup record; run `kkt setup` first (the note is unreadable)"
        );
        assert_eq!(
            said(Ok(None), Ok(None)),
            "this machine has no setup record; run `kkt setup` first"
        );
        let found = Unrecorded {
            mode: Mode::Service { tailscale: false },
            url: "http://127.0.0.1:7420/mcp".into(),
            key_file: None,
        };
        let summary = found.summary();
        assert_eq!(
            said(Ok(None), Ok(Some(found))),
            format!(
                "this machine has no setup record. {summary} Run `kkt setup` to record it (`kkt setup --adopt` with no terminal); it installs nothing. Then run this again."
            )
        );
        assert_eq!(
            said(
                Ok(None),
                Err((Purpose::Service, "the service is unreadable".into()))
            ),
            "this machine has no setup record; run `kkt setup` first. the service is unreadable"
        );
    }
}
