//! The advice when no record has been saved.

use std::error::Error;

/// Why there is no setup record to act on, and what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum NoRecord {
    /// A setup began here and did not finish.
    #[error("this machine has no setup record; run `kkt setup` to finish it")]
    Unfinished,
    /// The note of an unfinished setup could not be read.
    #[error("this machine has no setup record; run `kkt setup` first ({0})")]
    UnfinishedUnreadable(#[source] Box<dyn Error + Send + Sync>),
    /// Nothing kkt installed is here.
    #[error("this machine has no setup record; run `kkt setup` first")]
    Nothing,
    /// A server kkt installed is here with no record: `summary` says what.
    #[error(
        "this machine has no setup record. {summary} Run `kkt setup` to record it (`kkt setup --adopt` with no terminal); it installs nothing. Then run this again."
    )]
    Found { summary: String },
    /// Whether a server kkt installed is here could not be told.
    #[error("this machine has no setup record; run `kkt setup` first. {0}")]
    FoundUnreadable(#[source] Box<dyn Error + Send + Sync>),
}

/// This machine's setup record. Without one, the error says so, and names a
/// server kkt installed here anyway, which `kkt setup` can record without
/// installing it again. The readers' errors are their own, passed on as
/// they are.
pub fn recorded_setup<E1, E2>(
    record: Option<crate::Record>,
    unfinished: impl FnOnce() -> Result<Option<crate::Unfinished>, E1>,
    unrecorded: impl FnOnce() -> Result<Option<crate::Unrecorded>, (crate::Purpose, E2)>,
) -> Result<crate::Record, NoRecord>
where
    E1: Into<Box<dyn Error + Send + Sync>>,
    E2: Into<Box<dyn Error + Send + Sync>>,
{
    if let Some(record) = record {
        return Ok(record);
    }
    // An unfinished setup is described by `kkt status`.
    match unfinished() {
        Ok(Some(_)) => return Err(NoRecord::Unfinished),
        Ok(None) => {}
        Err(e) => return Err(NoRecord::UnfinishedUnreadable(e.into())),
    }
    Err(match unrecorded() {
        Ok(Some(found)) => NoRecord::Found {
            summary: found.summary(),
        },
        Ok(None) => NoRecord::Nothing,
        Err((_, e)) => NoRecord::FoundUnreadable(e.into()),
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
            .expect_err("no record")
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
