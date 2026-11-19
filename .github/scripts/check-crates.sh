#!/bin/bash
# Check each package in the workspace on its own, which packages may depend
# on an effect or a composition crate, and the rules for the `testing` feature (CLAUDE.md,
# "Where code lives"). The package list and manifests come from
# `cargo metadata`, so a new package is never missed. No package is exempt.
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
# 2. Only programs depend on an effect crate (a crate concerns.json marks
#    "effect"), in [dependencies], [dev-dependencies] or
#    [build-dependencies]. A program is a package with a binary target. Its
#    tests count too: an effect crate a program names only in
#    [dev-dependencies], not in [dependencies] or [build-dependencies], is
#    one only its tests use. Anything else that must is a documented
#    exception, listed with its reason in effect-exceptions.json beside this
#    script; a listed exception that no longer applies fails.
# 3. No package turns on another workspace package's `testing` outside
#    [dev-dependencies], with two exceptions. A package's own `testing`
#    feature may turn on a workspace dependency's `testing`: it is itself on
#    only from [dev-dependencies]. And a dev-only program
#    (a binary target and `[package.metadata.kkt] dev-only = true`) may turn
#    on `testing` in [dependencies]: dev tooling counts as test code, and
#    check-release-packages.sh keeps it out of every release. A library's `default` feature also may not
#    reach its own `testing`, or a workspace dependency's, directly or
#    through its other features at any depth.
# 4. `kkt-scratch` (test scratch folders) appears only in
#    [dev-dependencies].
# 5. Every package's name starts with `kkt-`, so its code logs under a
#    `kkt_` path, which the log filter keeps at kkt's own level. A package
#    with a binary target is a program and is not held to this.
# 6. No package has a `real` feature, and no source names one: code that
#    touches the machine is in an effect crate of its own, not behind a
#    feature.
# 7. Only programs depend on a composition crate (a crate concerns.json
#    marks "composition") in [dependencies]: a composition is put together
#    by the program that runs it, and nothing under it names it.
# 8. A platform crate (one concerns.json puts in the `platform` layer)
#    depends in [dependencies] only on other platform crates: a platform
#    crate knows nothing of kkt, so it cannot rest on a crate that does.
#    Its tests and build script may reach further.
#
# Checks 2 to 8 are in check-crates.jq, tested by test_check_crates.py,
# except check 6's search of the sources, which is below.
set -euo pipefail
scripts=$(dirname "$0")

metadata=$(cargo metadata --locked --no-deps --format-version 1)

root=$(dirname "$(cargo locate-project --workspace --message-format plain)")
effects=$(jq -c '[.crates | to_entries[] | select(.value.kind == "effect") | .key]' \
    "$root/docs/architecture/concerns.json")
compositions=$(jq -c '[.crates | to_entries[] | select(.value.kind == "composition") | .key]' \
    "$root/docs/architecture/concerns.json")
platform=$(jq -c '[.crates | to_entries[] | select(.value.layer == "platform") | .key]' \
    "$root/docs/architecture/concerns.json")
problems=$(jq -r --argjson effects "$effects" --argjson compositions "$compositions" \
    --argjson platform "$platform" \
    --argjson effect_exceptions "$(cat "$scripts/effect-exceptions.json")" \
    -f "$scripts/check-crates.jq" <<< "$metadata")
# Check 6, the sources: no code behind a `real` feature, anywhere in the
# workspace.
while read -r found; do
    [ -n "$found" ] && problems+=$'\n'"$found: names a \`real\` feature; code that touches the machine belongs in an effect crate, not behind a feature"
done < <(grep -rl --include='*.rs' --exclude-dir=target --exclude-dir=.git 'feature *= *"real"' "$root" || true)
problems=$(sed '/^$/d' <<< "$problems")
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
