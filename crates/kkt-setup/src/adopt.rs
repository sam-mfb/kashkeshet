//! A server kkt installed on this machine with no setup record, as an older
//! kkt can leave one: before 0.5.0, setup wrote the record only at the end,
//! so a setup that stopped after installing left the server running and
//! unrecorded. Setup records such a server as it is ("adopts" it), without
//! installing it again, once the person agrees.

use super::SETUP_UNKNOWN_VERSION;
use super::ask::{Ask, Question};
use super::flow::{Answers, Purpose};
use super::record::{Mode, Record};
use crate::{AskError, StepError};
use std::path::PathBuf;

/// Why `--adopt` cannot record a server here.
#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    /// There is no server kkt installed with no record.
    #[error("--adopt: no kkt server without a setup record was found here")]
    NothingFound,
    /// What was found could not be read.
    #[error("--adopt: {0}")]
    Unreadable(#[source] StepError),
    /// What was found differs from the answers given now.
    #[error("--adopt: {summary} The answers given now are different.")]
    Differs { summary: String },
    /// The question whether to record it got no answer.
    #[error(transparent)]
    Ask(AskError),
}

/// Both a server in Docker and a kkt service are installed here with no
/// record: which one clients use is not known.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "{summary} A kkt service is installed here too, so which one clients use is not known; remove one (`kkt service uninstall`, or `docker compose down` in the Docker folder) and run `kkt setup` again"
)]
pub struct BothInstalled {
    /// What was found in Docker.
    pub summary: String,
}

/// An installed kkt service with no record that runs with settings other
/// than the default config file's, which a record could not keep.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", not_defaults(args))]
pub struct NotDefaults {
    /// What the service file runs `kkt` with.
    pub args: Vec<String>,
}

fn not_defaults(args: &[String]) -> String {
    unreadable_service(&format!(
        "it runs `kkt {}`, with settings other than the default config file's, which \
         `kkt setup` and `kkt upgrade` install; a record could not keep them. Keep \
         managing it with `kkt service`, or put its settings in the default config \
         file, install it again with `kkt service install` and no options, and run \
         `kkt setup`",
        args.join(" ")
    ))
}

/// Whether Enter records a found, unrecorded install during interactive
/// setup.
const DEFAULT_SETUP_ADOPT: bool = true;

/// How setup finds a server to adopt: the part of [`super::Steps`] it uses.
pub trait AdoptSteps {
    /// A server kkt installed here that no record or note covers, as an
    /// older kkt can leave one; `None` when there is a record, or a note of
    /// a server still installed. An error says what was found and what could
    /// not be read from it.
    fn unrecorded(&self) -> Result<Option<Unrecorded>, StepError>;
}

/// A server kkt installed here that neither a record nor a note of an
/// unfinished setup covers. Everything in it was read from the install
/// itself, never guessed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unrecorded {
    /// How the server runs: [`Mode::Docker`] or [`Mode::Service`].
    pub mode: Mode,
    /// The MCP endpoint its clients use.
    pub url: String,
    /// The access key its clients send, if it has one.
    pub key_file: Option<PathBuf>,
}

impl Unrecorded {
    /// One sentence saying what was found, for setup, `kkt status` and
    /// `kkt doctor`.
    pub fn summary(&self) -> String {
        let how = match &self.mode {
            Mode::Docker(docker) => format!("in Docker from {}", docker.folder.display()),
            _ => "as a service".to_string(),
        };
        format!(
            "Found a kkt server {how}, for clients at {}, not recorded.",
            self.url
        )
    }

    /// The record that adopts it, written by kkt `version`. A Docker
    /// install's version is in its settings file, but nothing says which
    /// version a service runs, so its record says it is not known, and
    /// `kkt upgrade` installs the service again.
    pub fn record(&self, version: &str) -> Record {
        let written_by = match self.mode {
            Mode::Service { .. } => SETUP_UNKNOWN_VERSION,
            _ => version,
        };
        Record {
            mode: self.mode.clone(),
            written_by: written_by.to_string(),
            url: self.url.clone(),
            key_file: self.key_file.clone(),
            clients: None,
        }
    }

    pub(super) fn purpose(&self) -> Purpose {
        match self.mode {
            Mode::Docker(_) => Purpose::Docker,
            _ => Purpose::Service,
        }
    }

    fn tailscale(&self) -> bool {
        match &self.mode {
            Mode::Docker(docker) => docker.tailscale,
            Mode::Service { tailscale } => *tailscale,
            Mode::Client => false,
        }
    }
}

/// The server kkt installed here with no record, from the one found in
/// Docker and the kkt service found. With both, which one clients use is not
/// known, so neither is taken. An error names how the server it is about
/// runs.
pub fn unrecorded_here(
    docker: Option<Unrecorded>,
    service: Option<Unrecorded>,
) -> Result<Option<Unrecorded>, (Purpose, BothInstalled)> {
    match (docker, service) {
        (Some(docker), Some(_)) => Err((
            Purpose::Docker,
            BothInstalled {
                summary: docker.summary(),
            },
        )),
        (found, None) | (None, found) => Ok(found),
    }
}

/// What is said of an installed kkt service with no record that cannot be
/// recorded as it is; `what` says why.
pub fn unreadable_service(what: &str) -> String {
    format!("Found a kkt service, not recorded, but {what}.")
}

/// Refuses an installed kkt service with no record unless its service file,
/// which runs `kkt <args>`, runs it with the default config file's settings
/// (`runs_defaults`): those are what `kkt setup` and `kkt upgrade` install,
/// so a record could not keep any others.
pub fn service_runs_defaults(args: &[String], runs_defaults: bool) -> Result<(), NotDefaults> {
    if runs_defaults {
        return Ok(());
    }
    Err(NotDefaults {
        args: args.to_vec(),
    })
}

/// The installed kkt service with no record: shared over Tailscale as the
/// node `shared` names (its DNS name), at `mcp_path`, else reached at `url`,
/// with the access key in `key_file`.
pub fn unrecorded_service(
    shared: Option<&str>,
    url: String,
    key_file: Option<PathBuf>,
    mcp_path: &str,
) -> Unrecorded {
    Unrecorded {
        mode: Mode::Service {
            tailscale: shared.is_some(),
        },
        url: match shared {
            Some(dns_name) => format!("https://{dns_name}{mcp_path}"),
            None => url,
        },
        key_file,
    }
}

/// The server to adopt, if there is one and adopting is wanted: with
/// `--adopt`, or at a terminal when the person agrees. Without a terminal and
/// without `--adopt`, it is only named. With `--adopt`, finding nothing to
/// adopt, or answers that disagree with what was found, is an error.
pub(super) fn adoptable(
    answers: &Answers,
    ask: &dyn Ask,
    steps: &dyn AdoptSteps,
) -> Result<Option<Unrecorded>, AdoptError> {
    let found = match steps.unrecorded() {
        Ok(Some(found)) => found,
        Ok(None) if answers.adopt => return Err(AdoptError::NothingFound),
        Ok(None) => return Ok(None),
        Err(e) if answers.adopt => return Err(AdoptError::Unreadable(e)),
        Err(e) => {
            ask.tell(&format!("{e}\nSo it cannot be recorded as it is."));
            return Ok(None);
        }
    };
    let differs = answers.purpose.is_some_and(|p| p != found.purpose())
        || answers.tailscale.is_some_and(|t| t != found.tailscale());
    if differs {
        if answers.adopt {
            return Err(AdoptError::Differs {
                summary: found.summary(),
            });
        }
        ask.tell(&format!(
            "{} The answers given now are different, so setup does not record it.",
            found.summary()
        ));
        return Ok(None);
    }
    if answers.adopt {
        ask.tell(&found.summary());
        return Ok(Some(found));
    }
    if !ask.interactive() {
        ask.tell(&format!(
            "{} To record it instead of setting this machine up anew, run `kkt setup --adopt`.",
            found.summary()
        ));
        return Ok(None);
    }
    let question = Question {
        text: &format!("{} Record it and continue?", found.summary()),
        topic: "whether to adopt the existing server",
        flag: "--adopt",
        default: Some(DEFAULT_SETUP_ADOPT),
    };
    Ok(ask
        .confirm(question)
        .map_err(AdoptError::Ask)?
        .then_some(found))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DockerSetup;

    fn docker() -> Unrecorded {
        Unrecorded {
            mode: Mode::Docker(DockerSetup {
                folder: PathBuf::from("/data/docker"),
                tailscale: false,
                hostname: None,
                tailnet: None,
            }),
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: Some(PathBuf::from("/k/docker-access-key")),
        }
    }

    fn service() -> Unrecorded {
        Unrecorded {
            mode: Mode::Service { tailscale: false },
            url: "http://127.0.0.1:7880/mcp".into(),
            key_file: None,
        }
    }

    #[test]
    fn one_unrecorded_install_or_none_is_taken() {
        assert_eq!(unrecorded_here(None, None), Ok(None));
        assert_eq!(unrecorded_here(Some(docker()), None), Ok(Some(docker())));
        assert_eq!(unrecorded_here(None, Some(service())), Ok(Some(service())));
    }

    #[test]
    fn docker_and_a_service_both_found_are_refused_as_dockers() {
        assert_eq!(
            unrecorded_here(Some(docker()), Some(service())).map_err(|(p, e)| (p, e.to_string())),
            Err((
                Purpose::Docker,
                "Found a kkt server in Docker from /data/docker, for clients at \
                 http://127.0.0.1:8080/mcp, not recorded. A kkt service is installed here too, \
                 so which one clients use is not known; remove one (`kkt service uninstall`, \
                 or `docker compose down` in the Docker folder) and run `kkt setup` again"
                    .to_string()
            ))
        );
    }

    #[test]
    fn a_service_must_run_the_default_settings() {
        assert_eq!(service_runs_defaults(&["serve".into()], true), Ok(()));
        assert_eq!(
            service_runs_defaults(&["serve".into(), "--port".into(), "9".into()], false)
                .map_err(|e| e.to_string()),
            Err(
                "Found a kkt service, not recorded, but it runs `kkt serve --port 9`, with \
                 settings other than the default config file's, which `kkt setup` and `kkt \
                 upgrade` install; a record could not keep them. Keep managing it with `kkt \
                 service`, or put its settings in the default config file, install it again \
                 with `kkt service install` and no options, and run `kkt setup`."
                    .to_string()
            )
        );
    }

    #[test]
    fn a_service_that_cannot_be_read_says_why() {
        assert_eq!(
            unreadable_service("its service file went away while it was read"),
            "Found a kkt service, not recorded, but its service file went away while it was read."
        );
    }

    #[test]
    fn a_service_is_reached_over_tailscale_if_shared_else_as_configured() {
        let key = Some(PathBuf::from("/k/access-key"));
        assert_eq!(
            unrecorded_service(
                None,
                "http://127.0.0.1:7880/mcp".into(),
                key.clone(),
                "/mcp"
            ),
            Unrecorded {
                mode: Mode::Service { tailscale: false },
                url: "http://127.0.0.1:7880/mcp".into(),
                key_file: key.clone(),
            }
        );
        assert_eq!(
            unrecorded_service(
                Some("box.tail1234.ts.net"),
                "http://127.0.0.1:7880/mcp".into(),
                None,
                "/mcp"
            ),
            Unrecorded {
                mode: Mode::Service { tailscale: true },
                url: "https://box.tail1234.ts.net/mcp".into(),
                key_file: None,
            }
        );
    }
}

#[cfg(test)]
mod refusals {
    //! Every sentence adopting says when it refuses, pinned exactly.

    use super::*;
    use crate::DockerSetup;

    /// Finds what it was given, or says it could not read it.
    struct Found(Result<Option<Unrecorded>, String>);
    impl AdoptSteps for Found {
        fn unrecorded(&self) -> Result<Option<Unrecorded>, StepError> {
            self.0.clone().map_err(|said| StepError(said.into()))
        }
    }

    /// Never asked: every case here refuses before a question.
    struct Silent;
    impl Ask for Silent {
        fn interactive(&self) -> bool {
            false
        }
        fn choose(&self, _: Question, _: &[&str], _: usize) -> Result<usize, crate::AskError> {
            panic!("no question expected")
        }
        fn confirm(&self, _: Question) -> Result<bool, crate::AskError> {
            panic!("no question expected")
        }
        fn text(&self, _: Question, _: &str) -> Result<String, crate::AskError> {
            panic!("no question expected")
        }
        fn tell(&self, _: &str) {}
    }

    fn docker() -> Unrecorded {
        Unrecorded {
            mode: Mode::Docker(DockerSetup {
                folder: PathBuf::from("/data/docker"),
                tailscale: false,
                hostname: None,
                tailnet: None,
            }),
            url: "http://127.0.0.1:8080/mcp".into(),
            key_file: None,
        }
    }

    fn refused(answers: &Answers, found: Found) -> String {
        adoptable(answers, &Silent, &found)
            .map(|_| ())
            .expect_err("refused")
            .to_string()
    }

    #[test]
    fn every_adopt_refusal_says_what_it_always_said() {
        let adopt = Answers {
            adopt: true,
            ..Answers::default()
        };
        assert_eq!(
            refused(&adopt, Found(Ok(None))),
            "--adopt: no kkt server without a setup record was found here"
        );
        assert_eq!(
            refused(
                &adopt,
                Found(Err("its service file could not be read".into()))
            ),
            "--adopt: its service file could not be read"
        );
        let summary = docker().summary();
        let differs = Answers {
            purpose: Some(Purpose::Service),
            ..adopt
        };
        assert_eq!(
            refused(&differs, Found(Ok(Some(docker())))),
            format!("--adopt: {summary} The answers given now are different.")
        );
        let service = Unrecorded {
            mode: Mode::Service { tailscale: false },
            url: "http://127.0.0.1:7420/mcp".into(),
            key_file: None,
        };
        let (_, both) = unrecorded_here(Some(docker()), Some(service)).expect_err("both");
        assert_eq!(
            both.to_string(),
            format!(
                "{summary} A kkt service is installed here too, so which one clients use is not known; remove one (`kkt service uninstall`, or `docker compose down` in the Docker folder) and run `kkt setup` again"
            )
        );
        assert_eq!(
            service_runs_defaults(&["serve".into(), "--bind".into(), "0.0.0.0:7420".into()], false)
                .map_err(|e| e.to_string()),
            Err("Found a kkt service, not recorded, but it runs `kkt serve --bind 0.0.0.0:7420`, with settings other than the default config file's, which `kkt setup` and `kkt upgrade` install; a record could not keep them. Keep managing it with `kkt service`, or put its settings in the default config file, install it again with `kkt service install` and no options, and run `kkt setup`.".to_string())
        );
    }
}
