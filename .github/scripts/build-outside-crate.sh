#!/bin/bash
# Build a throwaway crate outside the workspace that depends on kashkeshet's
# shared crates (kkt-api-agent, kkt-rules, kkt-server and kkt-server-listen)
# the way kkt-pro will: by git, at one commit. It fails when a shared crate
# cannot be used from outside the workspace, for example when it reaches a
# file or a setting that only exists inside it.
#
# Usage: build-outside-crate.sh <repository URL> <commit> <empty folder>
#
# The folder must be outside the workspace. The workspace's Cargo.lock and
# rust-toolchain.toml are copied in, so the crate builds with the same
# versions and compiler; Cargo adds what the git source needs.
set -euo pipefail
if [ $# -ne 3 ]; then
    echo "usage: $0 <repository URL> <commit> <empty folder>" >&2
    exit 2
fi
url=$1
rev=$2
dir=$3
root=$(dirname "$(cargo locate-project --workspace --message-format plain)")

mkdir -p "$dir/src"
if [ -n "$(ls -A "$dir/src")" ] || [ -e "$dir/Cargo.toml" ]; then
    echo "build-outside-crate: $dir is not empty" >&2
    exit 1
fi
case "$(cd "$dir" && pwd -P)/" in
    "$(cd "$root" && pwd -P)/"*)
        echo "build-outside-crate: $dir is inside the workspace" >&2
        exit 1
        ;;
esac

cp "$root/Cargo.lock" "$root/rust-toolchain.toml" "$dir/"
cat > "$dir/Cargo.toml" <<TOML
[package]
name = "outside-kkt"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
# kkt-api-agent holds the records the shared operations take, SwarmId among
# them.
kkt-api-agent = { git = "$url", rev = "$rev" }
kkt-rules = { git = "$url", rev = "$rev" }
# kkt-server brings in the shared crates with build scripts
# (kkt-provenance, kkt-mcp, kkt-names): the likeliest to break outside the
# workspace.
kkt-server = { git = "$url", rev = "$rev" }
# Socket binding lives in its own effect crate.
kkt-server-listen = { git = "$url", rev = "$rev", features = ["real"] }
TOML
cat > "$dir/src/lib.rs" <<'RUST'
//! Uses kashkeshet's shared crates from outside its workspace.

use kkt_api_agent::SwarmId;
use kkt_rules::Rules;
use kkt_server::Parts;


/// `rules` acting in the swarm named `id`.
pub fn in_swarm(rules: &Rules, id: &str) -> Rules {
    rules.in_swarm(SwarmId::new(id))
}

/// The server run these rules were started for, before they are passed as an API.
pub fn run_of(rules: &Rules) -> i64 {
    rules.run_id()
}

/// The server exposes the admin contract, including its agent API limits.
pub fn limits(parts: &Parts) -> kkt_api_agent::Limits {
    parts.rules.limits()
}

/// The shared constructor and settings are usable without workspace-only values.
pub fn settings() -> kkt_server::ServerSettings {
    kkt_server::ServerSettings::DEFAULT
}
RUST
cd "$dir"
cargo build
echo "build-outside-crate: built against kkt-api-agent, kkt-rules, kkt-server and kkt-server-listen at $rev"
