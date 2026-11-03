#!/bin/bash
# Print the third-party packages built into the workspace's programs (the
# packages with a binary target), each with the features turned on, sorted.
# Workspace packages are left out, so moving code between them does not
# change the output. A change that must not change the binary must not
# change this output either: run it before and after and compare.
#
# Dev-dependencies are left out, as in a release build. Every target
# platform is included, so a change that only affects macOS shows up on
# Linux too.
set -euo pipefail
manifest=$(cargo locate-project --workspace --message-format plain)
root=${manifest%/Cargo.toml}
programs=$(cargo metadata --locked --no-deps --format-version 1 \
    | jq -r '.packages[] | select(any(.targets[]; any(.kind[]; . == "bin"))) | .name')
if [ -z "$programs" ]; then
    echo "third-party-features: the workspace has no package with a binary target" >&2
    exit 1
fi
for program in $programs; do
    cargo tree --locked --manifest-path "$manifest" -p "$program" \
        --edges normal,build --target all --prefix none --no-dedupe \
        --format '{p} [{f}]'
done | grep -v -F "($root" | sort -u
