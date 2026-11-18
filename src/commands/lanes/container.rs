//! The lanes of kkt in a container: `kkt lanes export` and `import` run
//! in the container, with the lane files carried out or in, and the lane
//! limit read from the container's own config file. The host's config
//! file plays no part; the container never reads it.

use kkt_api_agent::LaneSource;
use kkt_container::Folder;
use kkt_containerized_service::{Arg, Carry, ConfigReader, InServer, ServiceContainer};
use kkt_operator_commands::lanes::{LaneFuture, LaneImport, LaneServer};
use kkt_operator_commands::{Output, Paths};
use std::path::Path;

/// The start of the name of the folder in the kkt container that `kkt lanes
/// import` copies the lane files into, to import them there. Each run makes
/// its own, which adds random characters, and removes only that one
/// afterwards, so runs at the same time do not share a folder.
const LANES_IMPORT_FOLDER: &str = "/tmp/kkt-lanes-import";
/// The start of the name of the folder in the kkt container that `kkt lanes
/// export` writes the lane files to, to copy them out; one per run, as for
/// [`LANES_IMPORT_FOLDER`].
const LANES_EXPORT_FOLDER: &str = "/tmp/kkt-lanes-export";

/// The lanes of kkt installed in a container in `folder`.
pub(crate) struct ContainerLanes<'a> {
    pub container: &'a dyn ServiceContainer,
    /// Reads the container's config file.
    pub configs: &'a dyn ConfigReader,
    /// Makes an export's folder absolute, as the person typed it here.
    pub paths: &'a dyn Paths,
    pub output: &'a dyn Output,
    pub folder: Folder,
}

impl LaneServer for ContainerLanes<'_> {
    fn export<'a>(&'a self, dest: &'a Path) -> LaneFuture<'a, ()> {
        Box::pin(async move {
            let dest = self
                .paths
                .absolute(dest)
                .map_err(|e| format!("could not resolve {}: {e}", dest.display()))?;
            let said = self.container.run_in_server(
                &self.folder,
                &InServer {
                    args: &[text("lanes"), text("export"), Arg::Folder],
                    folder_prefix: LANES_EXPORT_FOLDER,
                    carry: Carry::Out(dest.clone()),
                },
            )?;
            self.output.output(format_args!("{said}"));
            self.output
                .errors(format_args!("Exported to {}\n", dest.display()));
            Ok(())
        })
    }

    /// The import in the container records the folder on the host as its
    /// source, so no other source can be given.
    fn check_source(&self, source: Option<&LaneSource>) -> Result<(), String> {
        match source {
            Some(_) => {
                Err("--source-folder is for `kkt lanes import` in the container itself".into())
            }
            None => Ok(()),
        }
    }

    /// The files were read here first, so a mistake is named by its path on
    /// the host; [`kkt_lane_files::Read::files_to_copy`] says which go in.
    fn import(&self, lanes: LaneImport) -> LaneFuture<'_, String> {
        Box::pin(async move {
            self.check_source(lanes.source.as_ref())?;
            let files = lanes.read.files_to_copy(lanes.retire_missing);
            let mut args = vec![
                text("lanes"),
                text("import"),
                Arg::Folder,
                text("--source-folder"),
                Arg::Text(lanes.read.folder.display().to_string()),
            ];
            if let Some(commit) = lanes.read.commit {
                args.extend([text("--source-commit"), Arg::Text(commit)]);
            }
            if lanes.retire_missing {
                args.push(text("--retire-missing"));
            }
            self.container
                .run_in_server(
                    &self.folder,
                    &InServer {
                        args: &args,
                        folder_prefix: LANES_IMPORT_FOLDER,
                        carry: Carry::In(files),
                    },
                )
                .map_err(String::from)
        })
    }

    /// The container's own setting when the folder holds an install, and
    /// the default for a new one, whose container starts with no config
    /// file. `kkt lanes import` in the container is run with no
    /// `--config`, so it reads only the default config file there.
    fn lane_limit(&self) -> Result<usize, String> {
        let default = kkt_rules::Settings::DEFAULT.max_lane_chars;
        kkt_setup::lane_reports::docker_limit(
            self.container.has_settings(&self.folder),
            default,
            || {
                let said = self
                    .container
                    .config_home_file(&self.folder, &kkt_config_file::config_file_in_home())?;
                // Only the lane limit is used, so the paths are not, and a
                // `~/` path need only be read as one: the home given is a
                // stand-in for the container's.
                let settings = self
                    .configs
                    .settings(&said, Path::new("/"), Some(Path::new("/")))
                    // Names the setting that is wrong.
                    .map_err(|detail| {
                        format!("unexpected output from the kkt container's config file: {detail}")
                    })?;
                Ok(settings.max_lane_chars.unwrap_or(default))
            },
        )
    }
}

fn text(text: &str) -> Arg {
    Arg::Text(text.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future::BoxFuture;
    use kkt_clock::{Clock, Time};
    use kkt_containerized_service::testing::{Memory, Run};
    use kkt_containerized_service::{Inputs, KeyCopies, LoginLink};
    use kkt_lane_files::{LaneFile, Read, Skip, Skipped};
    use std::cell::RefCell;
    use std::fmt::Arguments;
    use std::path::PathBuf;

    struct Unused;
    impl Clock for Unused {
        fn now(&self) -> Time {
            panic!("no clock is read")
        }
        fn sleep_until(&self, _: Time) -> BoxFuture<'_, ()> {
            panic!("nothing sleeps")
        }
    }
    impl LoginLink for Unused {
        fn show(&self, _: &str) {
            panic!("no login")
        }
        fn open_requested(&self) -> bool {
            panic!("no login")
        }
    }

    /// Paths typed here are under `/home/me`.
    struct Here;
    impl Paths for Here {
        fn absolute(&self, path: &Path) -> std::io::Result<PathBuf> {
            Ok(Path::new("/home/me").join(path))
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            panic!("nothing is made canonical")
        }
    }

    /// What was said, on standard output and standard error.
    #[derive(Default)]
    struct Said {
        output: RefCell<String>,
        errors: RefCell<String>,
    }
    impl Output for Said {
        fn output(&self, text: Arguments<'_>) {
            self.output.borrow_mut().push_str(&text.to_string());
        }
        fn errors(&self, text: Arguments<'_>) {
            self.errors.borrow_mut().push_str(&text.to_string());
        }
    }

    fn folder() -> Folder {
        Folder("/data/docker".into())
    }

    /// A container installed in [`folder`], whose every run prints `said`.
    fn installed(said: &str) -> Memory {
        let memory = Memory::new(None, said.into());
        let inputs = Inputs {
            version: "0.4.0".into(),
            project: "kkt".into(),
            port: 7420,
            allow_hosts: vec![],
            sharing: None,
        };
        let keys = KeyCopies {
            access: "/k/a".into(),
            web: "/k/w".into(),
            admin: "/k/m".into(),
        };
        futures::executor::block_on(memory.provision(&Unused, &Unused, &folder(), &inputs, &keys))
            .expect("installed");
        memory
    }

    fn lanes<'a>(memory: &'a Memory, said: &'a Said) -> ContainerLanes<'a> {
        ContainerLanes {
            container: memory,
            configs: &crate::machine::settings::ContainerConfigs,
            paths: &Here,
            output: said,
            folder: folder(),
        }
    }

    fn text(text: &str) -> Arg {
        Arg::Text(text.into())
    }

    /// Two lanes and a file with no header, read from `/home/me/lanes`.
    fn read(commit: Option<&str>) -> Read {
        let lane = |name: &str| LaneFile {
            name: name.into(),
            description: String::new(),
            text: "Do the work.".into(),
        };
        Read {
            lanes: vec![lane("orchestrator"), lane("reviewer")],
            skipped: vec![Skipped {
                path: "/home/me/lanes/writer.md".into(),
                why: Skip::NoHeader,
            }],
            folder: "/home/me/lanes".into(),
            commit: commit.map(str::to_string),
        }
    }

    #[test]
    fn an_export_runs_kkt_lanes_export_in_the_container_and_carries_the_files_out() {
        let memory = installed("reviewer.md (version 2)\n");
        let said = Said::default();
        futures::executor::block_on(lanes(&memory, &said).export(Path::new("out")))
            .expect("exported");
        assert_eq!(
            memory.runs(),
            [Run {
                folder: folder(),
                args: vec![text("lanes"), text("export"), Arg::Folder],
                folder_prefix: "/tmp/kkt-lanes-export".into(),
                carry: Carry::Out("/home/me/out".into()),
            }]
        );
        assert_eq!(*said.output.borrow(), "reviewer.md (version 2)\n");
        assert_eq!(*said.errors.borrow(), "Exported to /home/me/out\n");
    }

    #[test]
    fn an_import_runs_kkt_lanes_import_in_the_container_with_the_host_folder_as_source() {
        let memory = installed("created reviewer (version 1)\n");
        let said = Said::default();
        let report = futures::executor::block_on(lanes(&memory, &said).import(LaneImport {
            read: read(Some("abc123")),
            retire_missing: true,
            source: None,
        }))
        .expect("imported");
        assert_eq!(report, "created reviewer (version 1)\n");
        futures::executor::block_on(lanes(&memory, &said).import(LaneImport {
            read: read(None),
            retire_missing: false,
            source: None,
        }))
        .expect("imported");
        assert_eq!(
            memory.runs(),
            [
                Run {
                    folder: folder(),
                    args: vec![
                        text("lanes"),
                        text("import"),
                        Arg::Folder,
                        text("--source-folder"),
                        text("/home/me/lanes"),
                        text("--source-commit"),
                        text("abc123"),
                        text("--retire-missing"),
                    ],
                    folder_prefix: "/tmp/kkt-lanes-import".into(),
                    // Retiring, the file with no header goes in too, so its
                    // lane is not retired.
                    carry: Carry::In(vec![
                        "/home/me/lanes/orchestrator.md".into(),
                        "/home/me/lanes/reviewer.md".into(),
                        "/home/me/lanes/writer.md".into(),
                    ]),
                },
                Run {
                    folder: folder(),
                    args: vec![
                        text("lanes"),
                        text("import"),
                        Arg::Folder,
                        text("--source-folder"),
                        text("/home/me/lanes"),
                    ],
                    folder_prefix: "/tmp/kkt-lanes-import".into(),
                    carry: Carry::In(vec![
                        "/home/me/lanes/orchestrator.md".into(),
                        "/home/me/lanes/reviewer.md".into(),
                    ]),
                },
            ]
        );
        assert!(said.output.borrow().is_empty() && said.errors.borrow().is_empty());
    }

    #[test]
    fn an_import_with_its_own_source_is_refused_and_runs_nothing() {
        let memory = installed("");
        let said = Said::default();
        let source = LaneSource {
            folder: "/elsewhere".into(),
            commit: None,
        };
        let refusal = "--source-folder is for `kkt lanes import` in the container itself";
        let server = lanes(&memory, &said);
        assert_eq!(server.check_source(Some(&source)), Err(refusal.into()));
        assert_eq!(server.check_source(None), Ok(()));
        let done = futures::executor::block_on(server.import(LaneImport {
            read: read(None),
            retire_missing: false,
            source: Some(source),
        }));
        assert_eq!(done, Err(refusal.into()));
        assert!(memory.runs().is_empty());
    }

    #[test]
    fn the_lane_limit_is_the_containers_own_setting_or_the_default() {
        let default = kkt_rules::Settings::DEFAULT.max_lane_chars;
        let said = Said::default();

        // No install yet: the default, and nothing is read.
        let memory = Memory::new(None, String::new());
        assert_eq!(lanes(&memory, &said).lane_limit(), Ok(default));
        assert!(memory.config_home_reads().is_empty());

        // An install with no config file, or one without the setting.
        let memory = installed("");
        assert_eq!(lanes(&memory, &said).lane_limit(), Ok(default));
        memory.set_config_home_file("kkt/config.toml".into(), "name_pack = \"animals\"\n".into());
        assert_eq!(lanes(&memory, &said).lane_limit(), Ok(default));

        // Its own setting, read from where `kkt` in the container finds
        // its config file; a path under the home folder reads too.
        memory.set_config_home_file(
            "kkt/config.toml".into(),
            "db = \"~/kkt.db\"\nmax_lane_chars = 20\n".into(),
        );
        assert_eq!(lanes(&memory, &said).lane_limit(), Ok(20));
        assert_eq!(
            memory.config_home_reads(),
            vec![PathBuf::from("kkt/config.toml"); 3]
        );
    }

    #[test]
    fn a_lane_limit_that_cannot_be_read_names_the_setting() {
        let memory = installed("");
        memory.set_config_home_file("kkt/config.toml".into(), "max_lane_chars = 0.5\n".into());
        let err = lanes(&memory, &Said::default())
            .lane_limit()
            .expect_err("refused");
        assert!(
            err.starts_with(
                "could not read the lane text limit of kkt in Docker: unexpected output from \
                 the kkt container's config file: "
            ),
            "{err}"
        );
        assert!(err.contains("max_lane_chars"), "{err}");
    }
}
