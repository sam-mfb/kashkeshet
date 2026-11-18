//! Where this machine's access, web and admin keys are, as kkt-deployment's
//! key finders decide over what is installed here and the serve config.

use crate::OperatorParts;
use kkt_deployment::{
    self, RecordedServerRead, ServeConfigReads, ServerKeys, UnfinishedSetup, UnrecordedReads,
};
use std::path::PathBuf;

pub use kkt_deployment::{KeyKind, admin_key_collision};

/// The reads finding a key makes, over the operator's parts.
struct Reads<'a, 'p> {
    parts: &'a OperatorParts<'p>,
}

impl RecordedServerRead for Reads<'_, '_> {
    fn recorded(&self) -> Result<Option<ServerKeys>, kkt_deployment::KeyReadError> {
        self.parts
            .installed
            .recorded()
            .map_err(|said| kkt_deployment::KeyReadError(said.into()))
    }
}

impl ServeConfigReads for Reads<'_, '_> {
    fn configured_access(&self) -> Result<Option<PathBuf>, kkt_deployment::KeyReadError> {
        self.parts
            .serve
            .configured_access_key()
            .map_err(|said| kkt_deployment::KeyReadError(said.into()))
    }
    fn config_dir(&self) -> Option<PathBuf> {
        self.parts.serve.config_dir()
    }
}

impl UnrecordedReads for Reads<'_, '_> {
    fn unfinished(&self) -> Result<Option<UnfinishedSetup>, kkt_deployment::KeyReadError> {
        self.parts
            .installed
            .unfinished()
            .map_err(|said| kkt_deployment::KeyReadError(said.into()))
    }
    /// A server kkt installed here with no record. One that cannot be made
    /// out is said on standard error, as `kkt status` says it, and the keys
    /// are then looked for as the serve config says.
    fn unrecorded(&self) -> Option<ServerKeys> {
        match self.parts.installed.unrecorded() {
            Ok(found) => found,
            Err(e) => {
                self.parts
                    .output
                    .errors(format_args!("Unrecorded install: {e}\n"));
                None
            }
        }
    }
}

/// Resolve a key without reading its contents or creating a file. An explicit
/// path names the selected key itself; otherwise use this machine's setup.
pub fn selected_key_path(
    parts: &OperatorParts<'_>,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<PathBuf, String> {
    optional_key_path(parts, path, kind)?.ok_or_else(|| "there is no admin key".into())
}

/// None means the access key occupies the implicit admin key's place.
pub fn optional_key_path(
    parts: &OperatorParts<'_>,
    path: Option<PathBuf>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    kkt_deployment::find_key(&Reads { parts }, path, kind).map_err(|e| e.to_string())
}

/// Whether this machine's server runs in Docker, as recorded or, with no
/// record and no unfinished setup, as found.
pub(crate) fn runs_in_docker(parts: &OperatorParts<'_>) -> Result<bool, String> {
    Ok(kkt_deployment::find_here(&Reads { parts })
        .map_err(|e| e.to_string())?
        .runs_in_docker())
}

/// Where this machine's web or admin key file is, if it runs a server: for
/// Docker, the host copy `kkt setup` saves, even while that setup is
/// `unfinished` or the server is `unrecorded`; otherwise the file the server
/// reads. The file need not exist.
pub fn local_operator_key(
    parts: &OperatorParts<'_>,
    unfinished: Option<&UnfinishedSetup>,
    unrecorded: Option<&ServerKeys>,
    kind: KeyKind,
) -> Result<Option<PathBuf>, String> {
    kkt_deployment::find_operator_key(&Reads { parts }, unfinished, unrecorded, kind)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! What the operator's key lookups give for each thing that can be
    //! installed here: a record, an unfinished setup, a server found with
    //! no record, one that cannot be made out, and nothing.

    use super::*;
    use crate::lanes::{LaneServer, LaneServers};
    use crate::{Config, Databases, Output, Paths, ServeConfig};
    use kkt_deployment::Deployment;
    use std::cell::RefCell;
    use std::fmt::Arguments;
    use std::path::Path;
    use std::sync::Arc;

    struct Serve;
    impl ServeConfig for Serve {
        fn config(&self, _: Option<PathBuf>) -> Result<Config, String> {
            unreachable!("the key lookups read no config")
        }
        fn configured_access_key(&self) -> Result<Option<PathBuf>, String> {
            Ok(Some(PathBuf::from("/home/me/.config/kkt/access-key")))
        }
        fn database(&self, _: Option<PathBuf>, _: Option<PathBuf>) -> Result<PathBuf, String> {
            unreachable!("the key lookups open no database")
        }
        fn default_access_key(&self) -> Option<PathBuf> {
            unreachable!("the key lookups make no key")
        }
        fn config_dir(&self) -> Option<PathBuf> {
            Some(PathBuf::from("/home/me/.config/kkt"))
        }
    }

    struct Unused;
    impl Paths for Unused {
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!()
        }
        fn canonicalize(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!()
        }
    }
    impl LaneServers for Unused {
        fn here(
            &self,
            _: Option<PathBuf>,
            _: Option<PathBuf>,
        ) -> Result<Box<dyn LaneServer + '_>, String> {
            unreachable!()
        }
    }
    impl kkt_lane_files::Disk for Unused {
        fn file_names(&self, _: &Path) -> std::io::Result<Vec<String>> {
            unreachable!()
        }
        fn read(&self, _: &Path) -> std::io::Result<String> {
            unreachable!()
        }
        fn create_dir_all(&self, _: &Path) -> std::io::Result<()> {
            unreachable!()
        }
        fn write(&self, _: &Path, _: &str) -> std::io::Result<()> {
            unreachable!()
        }
        fn absolute(&self, _: &Path) -> std::io::Result<PathBuf> {
            unreachable!()
        }
    }
    impl kkt_lane_files::LaneCommit for Unused {
        fn commit(&self, _: &Path) -> std::io::Result<Option<String>> {
            unreachable!()
        }
    }
    impl Databases for Unused {
        fn exists(&self, _: &Path) -> bool {
            unreachable!()
        }
        fn open(&self, _: &Path) -> Result<kkt_rules::Db, String> {
            unreachable!()
        }
    }

    #[derive(Default)]
    struct Said(RefCell<String>);
    impl Output for Said {
        fn output(&self, text: Arguments<'_>) {
            self.0.borrow_mut().push_str(&format!("out: {text}"));
        }
        fn errors(&self, text: Arguments<'_>) {
            self.0.borrow_mut().push_str(&format!("err: {text}"));
        }
    }

    /// Every lookup's answer, and what was said, for what `installed` reads.
    fn lookups(installed: &Here) -> String {
        let said = Said::default();
        let clock = kkt_clock::testing::FixedClock::new(
            kkt_clock::Time::from_parts(2026, 11, 18, 12, 0, 0).expect("a time"),
        );
        let parts = OperatorParts {
            installed,
            serve: &Serve,
            paths: &Unused,
            key_files: &kkt_keys::testing::Memory::new(),
            entropy: &kkt_entropy::testing::SeqEntropy::new(0),
            lane_servers: &Unused,
            lanes: &Unused,
            lane_commit: &Unused,
            database: &Unused,
            clock: Arc::new(clock),
            output: &said,
        };
        let mut lines = String::new();
        for kind in [KeyKind::Access, KeyKind::Web, KeyKind::Admin] {
            lines.push_str(&format!(
                "{kind:?}: {:?}\n",
                optional_key_path(&parts, None, kind)
            ));
            lines.push_str(&format!(
                "{kind:?} here: {:?}\n",
                local_operator_key(&parts, None, None, kind)
            ));
        }
        lines.push_str(&format!("docker: {:?}\n", runs_in_docker(&parts)));
        lines.push_str(&said.0.borrow());
        lines
    }

    #[test]
    fn the_key_lookups_follow_what_is_installed_here() {
        let mut all = String::new();
        for (name, installed) in states() {
            all.push_str(&format!("== {name}\n{}", lookups(&installed)));
        }
        assert_eq!(all, EXPECTED, "\n{all}");
    }

    // What is installed here, as the program reads it.
    struct Here {
        recorded: Option<ServerKeys>,
        unfinished: Option<UnfinishedSetup>,
        unrecorded: Result<Option<ServerKeys>, String>,
    }

    impl crate::InstalledHere for Here {
        fn recorded(&self) -> Result<Option<ServerKeys>, String> {
            Ok(self.recorded.clone())
        }
        fn unfinished(&self) -> Result<Option<UnfinishedSetup>, String> {
            Ok(self.unfinished.clone())
        }
        fn unrecorded(&self) -> Result<Option<ServerKeys>, String> {
            self.unrecorded.clone()
        }
    }

    fn docker(folder: &str) -> Deployment {
        Deployment::Docker {
            folder: PathBuf::from(folder),
            tailscale: false,
            hostname: None,
            tailnet: None,
        }
    }

    fn recorded(deployment: Deployment) -> ServerKeys {
        ServerKeys {
            deployment,
            key_file: Some(PathBuf::from("/home/me/.config/kkt/docker-access-key")),
        }
    }

    fn states() -> Vec<(&'static str, Here)> {
        let nothing = || Here {
            recorded: None,
            unfinished: None,
            unrecorded: Ok(None),
        };
        vec![
            ("nothing installed", nothing()),
            (
                "a service, recorded",
                Here {
                    recorded: Some(recorded(Deployment::Service { tailscale: false })),
                    ..nothing()
                },
            ),
            (
                "Docker, recorded",
                Here {
                    recorded: Some(recorded(docker("/data/docker"))),
                    ..nothing()
                },
            ),
            (
                "a client, recorded",
                Here {
                    recorded: Some(recorded(Deployment::Client)),
                    ..nothing()
                },
            ),
            (
                "a Docker setup not finished",
                Here {
                    unfinished: Some(UnfinishedSetup {
                        server: None,
                        in_docker: true,
                    }),
                    ..nothing()
                },
            ),
            (
                "a Docker server found with no record",
                Here {
                    unrecorded: Ok(Some(recorded(docker("/data/docker")))),
                    ..nothing()
                },
            ),
            (
                "a server that cannot be made out",
                Here {
                    unrecorded: Err("its settings are unreadable".into()),
                    ..nothing()
                },
            ),
        ]
    }

    const EXPECTED: &str = "\
== nothing installed
Access: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Access here: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Web: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Web here: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Admin: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
Admin here: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
docker: Ok(false)
== a service, recorded
Access: Ok(Some(\"/home/me/.config/kkt/docker-access-key\"))
Access here: Ok(Some(\"/home/me/.config/kkt/docker-access-key\"))
Web: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Web here: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Admin: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
Admin here: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
docker: Ok(false)
== Docker, recorded
Access: Ok(Some(\"/home/me/.config/kkt/docker-access-key\"))
Access here: Ok(Some(\"/home/me/.config/kkt/docker-access-key\"))
Web: Ok(Some(\"/home/me/.config/kkt/docker-web-key\"))
Web here: Ok(Some(\"/home/me/.config/kkt/docker-web-key\"))
Admin: Ok(Some(\"/home/me/.config/kkt/docker-admin-key\"))
Admin here: Ok(Some(\"/home/me/.config/kkt/docker-admin-key\"))
docker: Ok(true)
== a client, recorded
Access: Ok(Some(\"/home/me/.config/kkt/docker-access-key\"))
Access here: Ok(None)
Web: Err(\"the web key is on the server's machine; run `kkt key show --web` there\")
Web here: Ok(None)
Admin: Err(\"the admin key is on the server's machine; run `kkt key show --admin` there\")
Admin here: Ok(None)
docker: Ok(false)
== a Docker setup not finished
Access: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Access here: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Web: Ok(Some(\"/home/me/.config/kkt/docker-web-key\"))
Web here: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Admin: Ok(Some(\"/home/me/.config/kkt/docker-admin-key\"))
Admin here: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
docker: Ok(false)
== a Docker server found with no record
Access: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Access here: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Web: Ok(Some(\"/home/me/.config/kkt/docker-web-key\"))
Web here: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Admin: Ok(Some(\"/home/me/.config/kkt/docker-admin-key\"))
Admin here: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
docker: Ok(true)
== a server that cannot be made out
Access: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Access here: Ok(Some(\"/home/me/.config/kkt/access-key\"))
Web: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Web here: Ok(Some(\"/home/me/.config/kkt/web-key\"))
Admin: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
Admin here: Ok(Some(\"/home/me/.config/kkt/admin-key\"))
docker: Ok(false)
err: Unrecorded install: its settings are unreadable
err: Unrecorded install: its settings are unreadable
err: Unrecorded install: its settings are unreadable
";
}
