//! The program's prober: kkt-probe's `HttpProber`, with the doctor's
//! timeout, response cap, protocol version and check names from
//! kkt-diagnostics, adapted to kkt-diagnostics' `Prober` (`Probe`). The
//! program injects it; its HTTP integration tests use it too.

use kkt_diagnostics::{self as diagnostics, Check, KeyFile, Prober, Probing};
use kkt_probe::{FileRead, Finding, HttpProber, Settings};
use std::path::Path;

/// The prober `kkt doctor` and `kkt setup` use.
pub fn prober() -> Probe {
    Probe(HttpProber::new(Settings {
        timeout: diagnostics::Settings::DEFAULT.doctor_timeout,
        max_response_bytes: diagnostics::DOCTOR_MAX_RESPONSE_BYTES,
        protocol_version: diagnostics::DOCTOR_PROTOCOL_VERSION,
        server_check: diagnostics::SERVER_CHECK,
        key_check: diagnostics::KEY_CHECK,
    }))
}

/// kkt-probe's `HttpProber`, adapted to kkt-diagnostics' `Prober`: each
/// record it gives becomes kkt-diagnostics' record of the same fields.
pub struct Probe(HttpProber);

impl Prober for Probe {
    fn key_file(&self, path: &Path) -> Result<KeyFile, String> {
        self.0.key_file(path).map(key_file)
    }

    fn probe<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> Probing<'a> {
        Box::pin(async move {
            let findings = self.0.probe(url, key).await?;
            Ok(findings.into_iter().map(check).collect())
        })
    }
}

fn key_file(read: FileRead) -> KeyFile {
    let FileRead { mode, text } = read;
    KeyFile { mode, text }
}

fn check(finding: Finding) -> Check {
    let Finding {
        name,
        ok,
        detail,
        fix,
    } = finding;
    Check {
        name,
        ok,
        detail,
        fix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_becomes_the_check_with_each_field_in_its_place() {
        let finding = Finding {
            name: "the name",
            ok: false,
            detail: "the detail".into(),
            fix: Some("the fix".into()),
        };
        assert_eq!(
            check(finding),
            Check {
                name: "the name",
                ok: false,
                detail: "the detail".into(),
                fix: Some("the fix".into()),
            }
        );
        let passed = Finding {
            name: "n",
            ok: true,
            detail: "d".into(),
            fix: None,
        };
        assert_eq!(check(passed), Check::ok("n", "d"));
    }

    #[test]
    fn a_file_read_becomes_the_key_file_with_each_field_in_its_place() {
        let read = key_file(FileRead {
            mode: Some(0o600),
            text: Ok("the key".into()),
        });
        assert_eq!((read.mode, read.text), (Some(0o600), Ok("the key".into())));
        let unreadable = key_file(FileRead {
            mode: None,
            text: Err("why not".into()),
        });
        assert_eq!(
            (unreadable.mode, unreadable.text),
            (None, Err("why not".into()))
        );
    }
}
