#!/bin/bash
# Check each package in the workspace on its own, and the rules for its
# `real` and `testing` features (CLAUDE.md, "Where code lives"). The package list and
# manifests come from `cargo metadata`, so a new package is never missed. No
# package is exempt.
#
# 1. Each package builds alone (`cargo check -p`), with `--all-targets`, and
#    once more with each of its own features turned on. With only `-p`,
#    Cargo unifies features within that one package's graph, so a package
#    that uses another's gated item without asking for the feature fails.
#    A package with a library target is also checked with each of its own
#    features without `--all-targets`. Test targets turn on the features
#    their dev-dependencies ask for, so with test targets in the build a
#    library that uses a gated item without asking for the feature in
#    [dependencies] still passes. Without them it fails.
# 2. Only programs turn on another workspace package's `real`. A package
#    with a binary target may do so in [dependencies]; a package with an
#    example target may do so in [dev-dependencies], for its examples. Any
#    other way, including through a feature of the package's own, fails.
#    A library's `default` feature also may not reach its own `real` or
#    `testing`, or a workspace dependency's, directly or through its other
#    features at any depth.
# 3. No package turns on another workspace package's `testing` outside
#    [dev-dependencies], except that a package's own `testing` feature may
#    turn on a workspace dependency's `testing`: it is itself on only from
#    [dev-dependencies] (Sam, 2026-11-11).
# 4. `kkt-scratch` (test scratch folders) appears only in
#    [dev-dependencies].
# 5. Every package's name starts with `kkt-`, so its code logs under a
#    `kkt_` path, which the log filter keeps at kkt's own level. A package
#    with a binary target is a program and is not held to this.
#
# Checks 2 to 5 are in check-crates.jq, tested by test_check_crates.py.
set -euo pipefail
scripts=$(dirname "$0")

metadata=$(cargo metadata --locked --no-deps --format-version 1)

problems=$(jq -r -f "$scripts/check-crates.jq" <<< "$metadata")
if [ -n "$problems" ]; then
    echo "check-crates: the workspace's manifests break the feature or name rules:" >&2
    echo "$problems" >&2
    exit 1
fi

# Check 1: each package builds alone, then with each of its own features:
# with all targets, and, for a package with a library target, without them.
# Each line is the package's name, "library" or "no-library", then its
# features.
jq -r '.packages[]
    | [.name,
       (if any(.targets[].kind[]; . == "lib" or . == "rlib" or . == "dylib" or . == "proc-macro")
        then "library" else "no-library" end),
       (.features | keys[] | select(. != "default"))]
    | join(" ")' <<< "$metadata" \
    | while read -r package library features; do
        echo "check-crates: $package" >&2
        cargo check --locked -p "$package" < /dev/null
        cargo check --locked -p "$package" --all-targets < /dev/null
        for feature in $features; do
            echo "check-crates: $package with feature $feature" >&2
            cargo check --locked -p "$package" --all-targets --features "$feature" < /dev/null
            if [ "$library" = "library" ]; then
                echo "check-crates: $package with feature $feature, without test targets" >&2
                cargo check --locked -p "$package" --features "$feature" < /dev/null
            fi
        done
    done
echo "check-crates: every package passed" >&2
