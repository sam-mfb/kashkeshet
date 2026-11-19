#!/bin/bash
# Check that no dev-only package is part of a release, and that no
# workspace package's `testing` feature is on in one. A package is dev-only
# when its manifest says `[package.metadata.kkt] dev-only = true`, as
# kkt-scratch (tests' scratch folders) and kkt-keymode (the second program)
# do. A released program is a package with a binary target that dist builds:
# one without `[package.metadata.dist] dist = false`. A released program
# may not be dev-only, and nothing in its normal or build dependencies,
# on any target platform, may be either. Dev-dependencies are not built
# into a release and are not looked at.
set -euo pipefail

# KKT_MANIFEST names another workspace's Cargo.toml, for this script's tests.
manifest=${KKT_MANIFEST:-$(cargo locate-project --workspace --message-format plain)}
metadata=$(cargo metadata --locked --no-deps --format-version 1 --manifest-path "$manifest")
dev_only=$(jq -r '.packages[] | select(.metadata.kkt."dev-only" == true) | .name' <<< "$metadata")
released=$(jq -r '.packages[]
    | select(any(.targets[]; any(.kind[]; . == "bin")))
    | select(.metadata.dist.dist != false)
    | .name' <<< "$metadata")
if [ -z "$released" ]; then
    echo "check-release-packages: the workspace has no released program" >&2
    exit 1
fi

root=$(dirname "$manifest")
problems=""
for program in $released; do
    tree=$(cargo tree --locked --manifest-path "$manifest" -p "$program" \
        --edges normal,build --target all --prefix none --no-dedupe --format '{p} [{f}]')
    in_release=$(awk '{print $1}' <<< "$tree" | sort -u)
    for package in $dev_only; do
        if grep -qx -- "$package" <<< "$in_release"; then
            problems+="  $package is dev-only but is part of $program"$'\n'
        fi
    done
    # A workspace package's `testing` holds stand-ins, such as kkt-db's
    # SQLite-backed FastPlace: it is on only in tests, never in a release.
    testing=$(grep -F "($root" <<< "$tree" \
        | grep -E '\[([^]]*,)?testing(,[^]]*)?\]$' | awk '{print $1}' | sort -u || true)
    for package in $testing; do
        problems+="  $package has its testing feature on in $program"$'\n'
    done
done
if [ -n "$problems" ]; then
    echo "check-release-packages: a dev-only package or a testing feature would be released:" >&2
    printf '%s' "$problems" >&2
    exit 1
fi
echo "check-release-packages: no dev-only package and no testing feature in: $(echo $released)"
