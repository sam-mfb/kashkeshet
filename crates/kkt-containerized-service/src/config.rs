//! The settings of the server's config file that the containerized
//! service reads, and the reader the caller gives it.

use std::path::Path;

/// The settings of the server's config file the containerized service uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerConfig {
    /// The name pack the file names, if it names one.
    pub name_pack: Option<String>,
    /// The longest lane instructions, in characters, if the file says.
    pub max_lane_chars: Option<usize>,
}

/// Parses a config file's text into the settings the containerized service
/// uses.
pub trait ConfigReader {
    /// The settings in a config file's `text`. `dir` is the directory the
    /// file is in, for relative paths; `home` is the home folder, for `~/`
    /// paths, if there is one. The error is a message that starts with the
    /// name of the setting that is wrong.
    fn settings(
        &self,
        text: &str,
        dir: &Path,
        home: Option<&Path>,
    ) -> Result<ContainerConfig, String>;
}
