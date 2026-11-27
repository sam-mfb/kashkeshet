//! The program's prober (`Probe`): kkt-probe's `HttpProber` with the
//! doctor's timeout, response cap, protocol version and client name, and the
//! key files through kkt-keys' `KeyFiles` on this machine's disk, adapted to
//! kkt-diagnostics' `Prober`. The program injects it; its HTTP integration
//! tests use it too.

use kkt_diagnostics::{
    self as diagnostics, Answered, Initialized, KeyFile, KeyFileUnread, Prober, Probing, Reply,
    ReplyError, Status, Tls, Unprobed,
};
use kkt_keys::KeyFiles;
use kkt_keys_os::OsKeyFiles;
use kkt_probe::{HttpProber, Settings};
use std::path::Path;

/// The name and version `kkt doctor` announces to a server it probes.
const CLIENT_NAME: &str = "kkt-doctor";
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The prober `kkt doctor` and `kkt setup` use.
pub fn prober() -> Probe {
    Probe(HttpProber::new(Settings {
        timeout: diagnostics::Settings::DEFAULT.doctor_timeout,
        max_response_bytes: diagnostics::DOCTOR_MAX_RESPONSE_BYTES,
        protocol_version: diagnostics::DOCTOR_PROTOCOL_VERSION,
        client_name: CLIENT_NAME,
        client_version: CLIENT_VERSION,
    }))
}

/// kkt-probe's `HttpProber` and this machine's key files, adapted to
/// kkt-diagnostics' `Prober`: each record kkt-probe gives becomes
/// kkt-diagnostics' record of the same fields.
pub struct Probe(HttpProber);

impl Prober for Probe {
    fn key_file(&self, path: &Path) -> Result<KeyFile, KeyFileUnread> {
        let mode = OsKeyFiles.mode(path).map_err(KeyFileUnread::new)?;
        Ok(KeyFile {
            mode,
            text: OsKeyFiles.read(path).map_err(KeyFileUnread::new),
        })
    }

    fn answer<'a>(&'a self, url: &'a str) -> Probing<'a, Answered> {
        Box::pin(async move {
            let answered = self.0.answer(url).await.map_err(unprobed)?;
            Ok(answered.map(status))
        })
    }

    fn initialize<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a, Initialized> {
        Box::pin(async move {
            let initialized = self.0.initialize(url, key).await.map_err(unprobed)?;
            Ok(self::initialized(initialized))
        })
    }

    fn tls<'a>(&'a self, url: &'a str) -> Probing<'a, Tls> {
        Box::pin(async move {
            Ok(match self.0.tls(url).await.map_err(unprobed)? {
                kkt_probe::Tls::Answered => Tls::Answered,
                kkt_probe::Tls::NoListener => Tls::NoListener,
                kkt_probe::Tls::NotTls => Tls::NotTls,
                kkt_probe::Tls::Failed(e) => Tls::Failed(e),
            })
        })
    }
}

fn status(status: kkt_probe::Status) -> Status {
    let kkt_probe::Status { code, reason } = status;
    Status { code, reason }
}

fn unprobed(why: kkt_probe::Unprobed) -> Unprobed {
    match why {
        kkt_probe::Unprobed::BadUrl => Unprobed::BadUrl,
        kkt_probe::Unprobed::NoClient(e) => Unprobed::NoClient(e),
    }
}

fn initialized(initialized: kkt_probe::Initialized) -> Initialized {
    match initialized {
        kkt_probe::Initialized::Failed(e) => Initialized::Failed(e),
        kkt_probe::Initialized::Refused(refused) => Initialized::Refused(status(refused)),
        kkt_probe::Initialized::Replied { reply, cleanup } => Initialized::Replied {
            reply: reply.map(self::reply).map_err(reply_error),
            cleanup: cleanup.map(|closed| closed.map(status)),
        },
    }
}

fn reply(reply: kkt_probe::Reply) -> Reply {
    match reply {
        kkt_probe::Reply::ServerInfo { name, version } => Reply::ServerInfo { name, version },
        kkt_probe::Reply::NoServerInfo => Reply::NoServerInfo,
    }
}

fn reply_error(error: kkt_probe::ReplyError) -> ReplyError {
    match error {
        kkt_probe::ReplyError::TooLarge => ReplyError::TooLarge,
        kkt_probe::ReplyError::NoResult => ReplyError::NoResult,
        kkt_probe::ReplyError::Read(e) => ReplyError::Read(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probed(code: u16, reason: &str) -> kkt_probe::Status {
        kkt_probe::Status {
            code,
            reason: Some(reason.into()),
        }
    }

    fn ours(code: u16, reason: &str) -> Status {
        Status {
            code,
            reason: Some(reason.into()),
        }
    }

    #[test]
    fn each_initialize_outcome_becomes_the_same_outcome() {
        assert_eq!(
            initialized(kkt_probe::Initialized::Failed("e".into())),
            Initialized::Failed("e".into())
        );
        assert_eq!(
            initialized(kkt_probe::Initialized::Refused(probed(401, "Unauthorized"))),
            Initialized::Refused(ours(401, "Unauthorized"))
        );
        assert_eq!(
            initialized(kkt_probe::Initialized::Replied {
                reply: Ok(kkt_probe::Reply::ServerInfo {
                    name: Some("n".into()),
                    version: Some("v".into()),
                }),
                cleanup: Some(Ok(probed(200, "OK"))),
            }),
            Initialized::Replied {
                reply: Ok(Reply::ServerInfo {
                    name: Some("n".into()),
                    version: Some("v".into()),
                }),
                cleanup: Some(Ok(ours(200, "OK"))),
            }
        );
        assert_eq!(
            initialized(kkt_probe::Initialized::Replied {
                reply: Ok(kkt_probe::Reply::NoServerInfo),
                cleanup: Some(Err("closed".into())),
            }),
            Initialized::Replied {
                reply: Ok(Reply::NoServerInfo),
                cleanup: Some(Err("closed".into())),
            }
        );
        for (error, expected) in [
            (kkt_probe::ReplyError::TooLarge, ReplyError::TooLarge),
            (kkt_probe::ReplyError::NoResult, ReplyError::NoResult),
            (
                kkt_probe::ReplyError::Read("r".into()),
                ReplyError::Read("r".into()),
            ),
        ] {
            assert_eq!(
                initialized(kkt_probe::Initialized::Replied {
                    reply: Err(error),
                    cleanup: None,
                }),
                Initialized::Replied {
                    reply: Err(expected),
                    cleanup: None,
                }
            );
        }
    }

    #[test]
    fn each_reason_a_probe_cannot_be_made_becomes_the_same_reason() {
        assert_eq!(unprobed(kkt_probe::Unprobed::BadUrl), Unprobed::BadUrl);
        assert_eq!(
            unprobed(kkt_probe::Unprobed::NoClient("c".into())),
            Unprobed::NoClient("c".into())
        );
    }
}
