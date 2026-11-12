//! The program's prober: kkt-probe-reqwest's `HttpProber`, with the doctor's
//! timeout, response cap, protocol version and check names from
//! kkt-diagnostics. The program injects it; its HTTP integration tests use
//! it too.

use kkt_diagnostics as diagnostics;
use kkt_probe_reqwest::{HttpProber, Settings};

/// The prober `kkt doctor` and `kkt setup` use.
pub fn prober() -> HttpProber {
    HttpProber::new(Settings {
        timeout: diagnostics::Settings::DEFAULT.doctor_timeout,
        max_response_bytes: diagnostics::DOCTOR_MAX_RESPONSE_BYTES,
        protocol_version: diagnostics::DOCTOR_PROTOCOL_VERSION,
        server_check: diagnostics::SERVER_CHECK,
        key_check: diagnostics::KEY_CHECK,
    })
}
