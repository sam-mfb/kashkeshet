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
