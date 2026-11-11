//! Adapts the machine's key files and HTTP probe to diagnostics' contract.

use kkt_diagnostics as diagnostics;
use kkt_diagnostics_os as native;
use std::path::Path;

/// The program's real prober, also used by its HTTP integration tests.
pub struct DiagnosticProber;

fn prober() -> native::OsProber {
    native::OsProber::new(native::Settings {
        timeout: diagnostics::Settings::DEFAULT.doctor_timeout,
        max_response_bytes: diagnostics::DOCTOR_MAX_RESPONSE_BYTES,
        protocol_version: diagnostics::DOCTOR_PROTOCOL_VERSION,
        server_check: diagnostics::SERVER_CHECK,
        key_check: diagnostics::KEY_CHECK,
    })
}

impl diagnostics::Prober for DiagnosticProber {
    fn key_file(&self, path: &Path) -> Result<diagnostics::KeyFile, String> {
        prober().key_file(path).map(|file| diagnostics::KeyFile {
            mode: file.mode,
            text: file.text,
        })
    }

    fn probe<'a>(&'a self, url: &'a str, key: Option<&'a str>) -> diagnostics::Probing<'a> {
        Box::pin(async move {
            let prober = prober();
            prober.probe(url, key).await.map(|checks| {
                checks
                    .into_iter()
                    .map(|check| diagnostics::Check {
                        name: check.name,
                        ok: check.ok,
                        detail: check.detail,
                        fix: check.fix,
                    })
                    .collect()
            })
        })
    }
}
