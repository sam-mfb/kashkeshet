//! The files of kkt's containerized service: the Compose files that come
//! with this program, which are the repository's own (so the manual route
//! in `docs/docker.md` and setup use the same files), and the settings file
//! beside them.

/// The Compose file of a local install.
const COMPOSE: &str = include_str!("../../../compose.yaml");
/// The Compose file of an install shared over Tailscale.
const COMPOSE_TAILSCALE: &str = include_str!("../../../compose.tailscale.yaml");
/// Tailscale's serve settings, for an install shared over Tailscale.
const SERVE: &str = include_str!("../../../tailscale/serve.json");

/// The file Compose reads its settings from, in the managed folder.
pub(crate) const ENV_FILE: &str = ".env";

/// The `COMPOSE_FILE` line of the settings file of an install shared over
/// Tailscale. A local install's has none.
pub(crate) const TAILSCALE_COMPOSE_FILE: &str = "compose.tailscale.yaml";

/// What `TS_TAILNET` is set to until the Tailscale container has logged in
/// and the real name is known.
pub(crate) const PENDING_TAILNET: &str = "pending.invalid";

/// The variables the service's files read, which every command runs
/// without, so the managed folder's settings file wins over the caller's
/// environment.
pub(crate) const SHIELDED: &[&str] = &[
    "KKT_VERSION",
    "KKT_PORT",
    "TS_HOSTNAME",
    "TS_TAILNET",
    "TS_AUTHKEY",
];

/// The files beside the settings file: where, relative to the folder, and
/// what.
pub(crate) fn embedded(tailscale: bool) -> Vec<(&'static str, &'static str)> {
    if tailscale {
        vec![
            (TAILSCALE_COMPOSE_FILE, COMPOSE_TAILSCALE),
            ("tailscale/serve.json", SERVE),
        ]
    } else {
        vec![("compose.yaml", COMPOSE)]
    }
}

/// The kkt version the settings file `env` selects, from its `KKT_VERSION`
/// line.
pub(crate) fn version_in_env(env: &str) -> Option<String> {
    env.lines()
        .find_map(|line| line.strip_prefix("KKT_VERSION="))
        .map(|v| v.trim().to_string())
}
