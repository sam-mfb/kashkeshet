//! The rule for an agent credential file, from facts the caller inspected.

use crate::ForeignCredentialFile;
use std::path::Path;

/// The change needed to take group and other access off a credential file.
pub struct CredentialFileFix {
    /// The old mode with only group and other access removed.
    pub mode: u32,
    /// Advice appended if applying this mode fails.
    pub advice: String,
    /// What to say after this mode is successfully applied.
    pub warning: String,
}

/// Refuses another user's file; otherwise leaves a private mode alone or
/// removes group/other access without adding owner permissions. `quoted`
/// is the caller's shell-quoted path for the existing fix advice.
pub fn credential_file_check(
    path: &Path,
    quoted: &str,
    owner: u32,
    mode: u32,
    uid: u32,
) -> Result<Option<CredentialFileFix>, ForeignCredentialFile> {
    if owner != uid {
        return Err(ForeignCredentialFile {
            path: path.to_path_buf(),
            quoted: quoted.to_string(),
            owner,
            uid,
        });
    }
    if mode & 0o077 == 0 {
        return Ok(None);
    }
    Ok(Some(CredentialFileFix {
        mode: mode & !0o077,
        advice: format!("run chmod {:o} {quoted}", mode & 0o700),
        warning: format!(
            "credential file {} was readable by others and is now private; if anyone else could have read it, get a new token with `reclaim`",
            path.display()
        ),
    }))
}
