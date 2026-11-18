//! kkt installed in a container, as kkt-operator-commands' lane commands
//! take it (`KktInContainer`): this program's container service, the
//! folder kkt is installed in, and its reader of kkt's config file.

use kkt_container::Folder;
use kkt_containerized_service::{Arg, Carry, ConfigReader, InServer, ServiceContainer};
use kkt_operator_commands::lanes::{Carried, KktArg, KktInContainer};
use std::path::Path;

/// kkt in the container installed in `folder`.
pub(crate) struct LanesInContainer<'a> {
    pub(crate) container: &'a dyn ServiceContainer,
    /// Reads kkt's config file in the container.
    pub(crate) configs: &'a dyn ConfigReader,
    pub(crate) folder: Folder,
}

impl KktInContainer for LanesInContainer<'_> {
    fn run_kkt(
        &self,
        args: &[KktArg],
        folder_prefix: &str,
        carry: Carried,
    ) -> Result<String, String> {
        let args: Vec<Arg> = args
            .iter()
            .map(|arg| match arg {
                KktArg::Text(text) => Arg::Text(text.clone()),
                KktArg::WorkFolder => Arg::Folder,
            })
            .collect();
        let carry = match carry {
            Carried::In(files) => Carry::In(files),
            Carried::Out(dest) => Carry::Out(dest),
        };
        self.container
            .run_in_server(
                &self.folder,
                &InServer {
                    args: &args,
                    folder_prefix,
                    carry,
                },
            )
            .map_err(String::from)
    }

    fn installed(&self) -> bool {
        self.container.has_settings(&self.folder)
    }

    fn config_file(&self) -> Result<String, String> {
        self.container
            .config_home_file(&self.folder, &kkt_config_file::config_file_in_home())
            .map_err(String::from)
    }

    /// Only the lane limit is used, so the paths are not, and a `~/` path
    /// need only be read as one: the home given is a stand-in for the
    /// container's.
    fn lane_limit_in(&self, text: &str) -> Result<Option<usize>, String> {
        self.configs
            .settings(text, Path::new("/"), Some(Path::new("/")))
            .map(|settings| settings.max_lane_chars)
    }
}
