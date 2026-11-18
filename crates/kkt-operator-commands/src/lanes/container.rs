//! The lanes of kkt in a container: `kkt lanes export` and `import` run
//! in the container, with the lane files carried out or in, and the lane
//! limit read from the container's own config file. The host's config
//! file plays no part; the container never reads it.

use super::{LaneError, LaneFuture, LaneImport, LaneServer};
use crate::{Output, PartError, Paths};
use kkt_api_agent::LaneSource;
use std::path::{Path, PathBuf};

/// The start of the name of the folder in the kkt container that `kkt lanes
/// import` copies the lane files into, to import them there. Each run makes
/// its own, which adds random characters, and removes only that one
/// afterwards, so runs at the same time do not share a folder.
const LANES_IMPORT_FOLDER: &str = "/tmp/kkt-lanes-import";
/// The start of the name of the folder in the kkt container that `kkt lanes
/// export` writes the lane files to, to copy them out; one per run, as for
/// [`LANES_IMPORT_FOLDER`].
const LANES_EXPORT_FOLDER: &str = "/tmp/kkt-lanes-export";

/// kkt installed in a container, as the lane commands use it. The program
/// adapts its container, the folder kkt is installed in and its reader of
/// kkt's config file to it.
pub trait ServerInContainer {
    /// Runs kkt's own program in the container with `args`, in a work
    /// folder of its own whose name starts with `folder_prefix`, carrying
    /// files in or out as `carry` says; what it printed.
    fn run_kkt(
        &self,
        args: &[RunArg],
        folder_prefix: &str,
        carry: Carried,
    ) -> Result<String, PartError>;
    /// Whether kkt is installed in the folder.
    fn installed(&self) -> bool;
    /// The text of kkt's config file in the container, where `kkt` there
    /// finds it.
    fn config_file(&self) -> Result<String, PartError>;
    /// The lane text limit that the config file `text` sets, if it sets
    /// one. The error names the setting that is wrong.
    fn lane_limit_in(&self, text: &str) -> Result<Option<usize>, PartError>;
}

/// One argument to kkt's program in the container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunArg {
    Text(String),
    /// The run's own work folder.
    WorkFolder,
}

/// The files a run in the container carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Carried {
    /// These files on the host are copied into the work folder first.
    In(Vec<PathBuf>),
    /// The work folder is copied out to this folder on the host after.
    Out(PathBuf),
}

/// The lanes of kkt installed in a container.
pub struct ContainerLanes<'a> {
    pub container: Box<dyn ServerInContainer + 'a>,
    /// Makes an export's folder absolute, as the person typed it here.
    pub paths: &'a dyn Paths,
    pub output: &'a dyn Output,
}

impl LaneServer for ContainerLanes<'_> {
    fn export<'a>(&'a self, dest: &'a Path) -> LaneFuture<'a, ()> {
        Box::pin(async move {
            let dest = self
                .paths
                .absolute(dest)
                .map_err(|source| LaneError::Resolve {
                    path: dest.to_path_buf(),
                    source,
                })?;
            let said = self
                .container
                .run_kkt(
                    &[text("lanes"), text("export"), RunArg::WorkFolder],
                    LANES_EXPORT_FOLDER,
                    Carried::Out(dest.clone()),
                )
                .map_err(LaneError::Run)?;
            self.output.output(format_args!("{said}"));
            self.output
                .errors(format_args!("Exported to {}\n", dest.display()));
            Ok(())
        })
    }

    /// The import in the container records the folder on the host as its
    /// source, so no other source can be given.
    fn check_source(&self, source: Option<&LaneSource>) -> Result<(), LaneError> {
        match source {
            Some(_) => Err(LaneError::SourceInContainer),
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
                RunArg::WorkFolder,
                text("--source-folder"),
                RunArg::Text(lanes.read.folder.display().to_string()),
            ];
            if let Some(commit) = lanes.read.commit {
                args.extend([text("--source-commit"), RunArg::Text(commit)]);
            }
            if lanes.retire_missing {
                args.push(text("--retire-missing"));
            }
            self.container
                .run_kkt(&args, LANES_IMPORT_FOLDER, Carried::In(files))
                .map_err(LaneError::Run)
        })
    }

    /// The container's own setting when the folder holds an install, and
    /// the default for a new one, whose container starts with no config
    /// file. `kkt lanes import` in the container is run with no
    /// `--config`, so it reads only the default config file there.
    fn lane_limit(&self) -> Result<usize, LaneError> {
        let default = kkt_rules::Settings::DEFAULT.max_lane_chars;
        if !self.container.installed() {
            return Ok(default);
        }
        let said = self
            .container
            .config_file()
            .map_err(LaneError::LimitUnread)?;
        let limit = self
            .container
            .lane_limit_in(&said)
            // Names the setting that is wrong.
            .map_err(LaneError::LimitSetting)?;
        Ok(limit.unwrap_or(default))
    }
}

fn text(text: &str) -> RunArg {
    RunArg::Text(text.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_lane_files::{LaneFile, Read, Skip, Skipped};
    use std::cell::RefCell;
    use std::fmt::Arguments;

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

    /// One run of kkt in the container: its arguments, its work folder's
    /// prefix and what it carried.
    type RunKkt = (Vec<RunArg>, String, Carried);

    /// kkt in a container, in memory: installed or not, every run printing
    /// `said`, and a config file (none is an empty one, as a container with
    /// no config file reads).
    #[derive(Default)]
    struct Stand {
        installed: bool,
        said: String,
        config: RefCell<Option<String>>,
        runs: RefCell<Vec<RunKkt>>,
        config_reads: RefCell<usize>,
        /// Whether the config file cannot be read.
        config_unread: bool,
    }

    impl ServerInContainer for &Stand {
        fn run_kkt(
            &self,
            args: &[RunArg],
            folder_prefix: &str,
            carry: Carried,
        ) -> Result<String, PartError> {
            self.runs
                .borrow_mut()
                .push((args.to_vec(), folder_prefix.into(), carry));
            Ok(self.said.clone())
        }
        fn installed(&self) -> bool {
            self.installed
        }
        fn config_file(&self) -> Result<String, PartError> {
            *self.config_reads.borrow_mut() += 1;
            if self.config_unread {
                return Err(PartError("no such container".into()));
            }
            Ok(self.config.borrow().clone().unwrap_or_default())
        }
        /// A `max_lane_chars = N` line, refused by name when N is not a
        /// whole number, as the program's reader refuses it.
        fn lane_limit_in(&self, text: &str) -> Result<Option<usize>, PartError> {
            let mut max_lane_chars = None;
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("max_lane_chars = ") {
                    max_lane_chars = Some(value.parse::<usize>().map_err(|_| {
                        PartError(format!("max_lane_chars: {value} is not a whole number").into())
                    })?);
                }
            }
            Ok(max_lane_chars)
        }
    }

    /// kkt installed in the container, whose every run prints `said`.
    fn installed(said: &str) -> Stand {
        Stand {
            installed: true,
            said: said.into(),
            ..Stand::default()
        }
    }

    fn lanes<'a>(stand: &'a Stand, said: &'a Said) -> ContainerLanes<'a> {
        ContainerLanes {
            container: Box::new(stand),
            paths: &Here,
            output: said,
        }
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
        let stand = installed("reviewer.md (version 2)\n");
        let said = Said::default();
        futures::executor::block_on(lanes(&stand, &said).export(Path::new("out")))
            .expect("exported");
        assert_eq!(
            *stand.runs.borrow(),
            [(
                vec![text("lanes"), text("export"), RunArg::WorkFolder],
                "/tmp/kkt-lanes-export".to_string(),
                Carried::Out("/home/me/out".into()),
            )]
        );
        assert_eq!(*said.output.borrow(), "reviewer.md (version 2)\n");
        assert_eq!(*said.errors.borrow(), "Exported to /home/me/out\n");
    }

    #[test]
    fn an_import_runs_kkt_lanes_import_in_the_container_with_the_host_folder_as_source() {
        let stand = installed("created reviewer (version 1)\n");
        let said = Said::default();
        let report = futures::executor::block_on(lanes(&stand, &said).import(LaneImport {
            read: read(Some("abc123")),
            retire_missing: true,
            source: None,
        }))
        .expect("imported");
        assert_eq!(report, "created reviewer (version 1)\n");
        futures::executor::block_on(lanes(&stand, &said).import(LaneImport {
            read: read(None),
            retire_missing: false,
            source: None,
        }))
        .expect("imported");
        assert_eq!(
            *stand.runs.borrow(),
            [
                (
                    vec![
                        text("lanes"),
                        text("import"),
                        RunArg::WorkFolder,
                        text("--source-folder"),
                        text("/home/me/lanes"),
                        text("--source-commit"),
                        text("abc123"),
                        text("--retire-missing"),
                    ],
                    "/tmp/kkt-lanes-import".to_string(),
                    // Retiring, the file with no header goes in too, so its
                    // lane is not retired.
                    Carried::In(vec![
                        "/home/me/lanes/orchestrator.md".into(),
                        "/home/me/lanes/reviewer.md".into(),
                        "/home/me/lanes/writer.md".into(),
                    ]),
                ),
                (
                    vec![
                        text("lanes"),
                        text("import"),
                        RunArg::WorkFolder,
                        text("--source-folder"),
                        text("/home/me/lanes"),
                    ],
                    "/tmp/kkt-lanes-import".to_string(),
                    Carried::In(vec![
                        "/home/me/lanes/orchestrator.md".into(),
                        "/home/me/lanes/reviewer.md".into(),
                    ]),
                ),
            ]
        );
        assert!(said.output.borrow().is_empty() && said.errors.borrow().is_empty());
    }

    #[test]
    fn an_import_with_its_own_source_is_refused_and_runs_nothing() {
        let stand = installed("");
        let said = Said::default();
        let source = LaneSource {
            folder: "/elsewhere".into(),
            commit: None,
        };
        let refusal = "--source-folder is for `kkt lanes import` in the container itself";
        let server = lanes(&stand, &said);
        assert_eq!(
            server
                .check_source(Some(&source))
                .map_err(|e| e.to_string()),
            Err(refusal.to_string())
        );
        assert_eq!(server.check_source(None).map_err(|e| e.to_string()), Ok(()));
        let done = futures::executor::block_on(server.import(LaneImport {
            read: read(None),
            retire_missing: false,
            source: Some(source),
        }));
        assert_eq!(done.map_err(|e| e.to_string()), Err(refusal.to_string()));
        assert!(stand.runs.borrow().is_empty());
    }

    #[test]
    fn the_lane_limit_is_the_containers_own_setting_or_the_default() {
        let default = kkt_rules::Settings::DEFAULT.max_lane_chars;
        let said = Said::default();

        // No install yet: the default, and nothing is read.
        let stand = Stand::default();
        assert_eq!(
            lanes(&stand, &said).lane_limit().map_err(|e| e.to_string()),
            Ok(default)
        );
        assert_eq!(*stand.config_reads.borrow(), 0);

        // An install with no config file, or one without the setting.
        let stand = installed("");
        assert_eq!(
            lanes(&stand, &said).lane_limit().map_err(|e| e.to_string()),
            Ok(default)
        );
        *stand.config.borrow_mut() = Some("name_pack = \"animals\"\n".into());
        assert_eq!(
            lanes(&stand, &said).lane_limit().map_err(|e| e.to_string()),
            Ok(default)
        );

        // Its own setting.
        *stand.config.borrow_mut() = Some("db = \"~/kkt.db\"\nmax_lane_chars = 20\n".into());
        assert_eq!(
            lanes(&stand, &said).lane_limit().map_err(|e| e.to_string()),
            Ok(20)
        );
        assert_eq!(*stand.config_reads.borrow(), 3);
    }

    #[test]
    fn a_lane_limit_that_cannot_be_read_names_the_setting() {
        let stand = installed("");
        *stand.config.borrow_mut() = Some("max_lane_chars = 0.5\n".into());
        let err = lanes(&stand, &Said::default())
            .lane_limit()
            .expect_err("refused")
            .to_string();
        assert!(
            err.starts_with(
                "could not read the lane text limit of kkt in Docker: unexpected output from \
                 the kkt container's config file: "
            ),
            "{err}"
        );
        assert!(err.contains("max_lane_chars"), "{err}");
    }

    /// Paths typed here cannot be made absolute.
    struct Nowhere;
    impl Paths for Nowhere {
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("refused"))
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            panic!("nothing is made canonical")
        }
    }

    /// The refusals the lanes of kkt in a container word themselves, word
    /// for word.
    #[test]
    fn the_container_lanes_refusals_are_pinned() {
        let said = Said::default();
        let unread = Stand {
            installed: true,
            config_unread: true,
            ..Stand::default()
        };
        assert_eq!(
            lanes(&unread, &said)
                .lane_limit()
                .map_err(|e| e.to_string()),
            Err(
                "could not read the lane text limit of kkt in Docker: no such container"
                    .to_string()
            )
        );
        let stand = installed("");
        *stand.config.borrow_mut() = Some("max_lane_chars = 0.5\n".into());
        assert_eq!(
            lanes(&stand, &said).lane_limit().map_err(|e| e.to_string()),
            Err(
                "could not read the lane text limit of kkt in Docker: unexpected output from \
                 the kkt container's config file: max_lane_chars: 0.5 is not a whole number"
                    .to_string()
            )
        );
        let stand = installed("");
        let server = ContainerLanes {
            container: Box::new(&stand),
            paths: &Nowhere,
            output: &said,
        };
        assert_eq!(
            futures::executor::block_on(server.export(Path::new("out"))).map_err(|e| e.to_string()),
            Err("could not resolve out: refused".to_string())
        );
    }
}
