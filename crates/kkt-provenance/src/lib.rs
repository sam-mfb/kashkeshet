//! Provenance: what produced a server run. Each run records the program
//! version, the git commit it was built from, the settings in effect, and a
//! hash of the instructions agents were given, so anything the run creates
//! can be traced back to them.

#[cfg(test)]
mod commit;

/// The build the server is running.
#[derive(Clone, Copy, Debug)]
pub struct Build {
    pub version: &'static str,
    /// The git commit, from `build.rs`. See `commit::commit_text`.
    pub commit: &'static str,
}

impl Build {
    /// This binary.
    pub const CURRENT: Build = Build {
        version: env!("CARGO_PKG_VERSION"),
        commit: env!("KASHKESHET_GIT_COMMIT"),
    };
}

/// One run of the server, from start to stop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRun {
    pub version: String,
    pub commit: String,
    /// The settings in effect: one `name=value` line for each of
    /// [`RunSettings::settings`], in order.
    pub settings: String,
    /// SHA-256 of the instructions text, in lowercase hex.
    pub instructions_sha256: String,
}

/// The settings a server run was given, as lines of a setting's name and
/// value, which the program words: no other crate's settings type. Keys are
/// never here: only the files they came from.
#[derive(Clone, Debug)]
pub struct RunSettings {
    /// Each setting's name and value, in the order the record shows them.
    pub settings: Vec<(String, String)>,
    /// The instructions sent to every agent that connects.
    pub instructions: String,
}

impl ServerRun {
    pub fn new(build: Build, settings: &RunSettings) -> ServerRun {
        let RunSettings {
            settings,
            instructions,
        } = settings;
        ServerRun {
            version: build.version.to_string(),
            commit: build.commit.to_string(),
            settings: settings
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("\n"),
            instructions_sha256: sha256_hex(instructions),
        }
    }
}

fn sha256_hex(text: &str) -> String {
    kkt_hash::sha256(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BUILD: Build = Build {
        version: "9.9.9",
        commit: "abc123",
    };

    fn settings(lines: &[(&str, &str)], instructions: &str) -> RunSettings {
        RunSettings {
            settings: lines
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            instructions: instructions.into(),
        }
    }

    #[test]
    fn a_run_records_the_build_settings_and_instructions() {
        let run = ServerRun::new(
            TEST_BUILD,
            &settings(&[("allow_any_bind", "false"), ("max_limit", "100")], ""),
        );
        assert_eq!(run.version, "9.9.9");
        assert_eq!(run.commit, "abc123");
        assert_eq!(run.settings, "allow_any_bind=false\nmax_limit=100");
        // SHA-256 of the empty string.
        assert_eq!(
            run.instructions_sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_run_keeps_the_lines_in_the_order_given() {
        let run = ServerRun::new(TEST_BUILD, &settings(&[("b", "2"), ("a", "1")], ""));
        assert_eq!(run.settings, "b=2\na=1");
        let none = ServerRun::new(TEST_BUILD, &settings(&[], ""));
        assert_eq!(none.settings, "");
    }

    #[test]
    fn different_settings_or_instructions_give_a_different_run() {
        let base = ServerRun::new(TEST_BUILD, &settings(&[("max_limit", "100")], "hello"));
        let changed = ServerRun::new(TEST_BUILD, &settings(&[("max_limit", "99")], "hello"));
        assert_ne!(base.settings, changed.settings);
        let changed = ServerRun::new(TEST_BUILD, &settings(&[("max_limit", "100")], "hello!"));
        assert_eq!(base.settings, changed.settings);
        assert_ne!(base.instructions_sha256, changed.instructions_sha256);
    }

    #[test]
    fn this_build_names_a_commit_or_says_unknown() {
        let commit = Build::CURRENT.commit;
        let hash = commit.strip_suffix("-dirty").unwrap_or(commit);
        assert!(
            commit == "unknown"
                || (hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit())),
            "unexpected commit text {commit:?}"
        );
    }
}
