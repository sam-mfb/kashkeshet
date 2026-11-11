# Checks 2 to 5 of check-crates.sh, over `cargo metadata --no-deps` output.
# Prints one line per problem, and nothing when the manifests follow the
# rules. No package is named here except kkt-scratch, the subject of check 4.

# The package's own features that its `default` turns on, at any depth,
# `default` included. An entry that names a dependency (`dep:x`, `x/f`,
# `x?/f`) is not followed.
def default_reach($features):
    def grow:
        . as $seen
        | ($seen + [$seen[] | ($features[.] // [])[] | select(test("[:/]") | not)] | unique) as $all
        | if $all == $seen then $seen else $all | grow end;
    ["default"] | grow;

[.packages[].name] as $workspace
| .packages[]
| . as $package
| any(.targets[]; any(.kind[]; . == "bin")) as $program
| any(.targets[]; any(.kind[]; . == "example")) as $examples
| [.dependencies[] | select(.name as $name | any($workspace[]; . == $name))] as $local
| (
    $local[]
    | . as $dependency
    | (.kind // "normal") as $kind
    | ({normal: "[dependencies]", dev: "[dev-dependencies]", build: "[build-dependencies]"}[$kind]
        + (if .target then " for \(.target)" else "" end)) as $section
    # Check 2: only programs turn on another package's real.
    | (
        if any(.features[]; . == "real")
            and ((($kind == "normal" and $program) or ($kind == "dev" and $examples)) | not)
        then "\($package.name): turns on \($dependency.name)'s real in \($section); only a package with a binary target may (in [dependencies]), or one with an example target (in [dev-dependencies])"
        else empty end
    ),
    # Check 3: another package's testing only in [dev-dependencies].
    (
        if any(.features[]; . == "testing") and $kind != "dev"
        then "\($package.name): turns on \($dependency.name)'s testing in \($section); only [dev-dependencies] may"
        else empty end
    ),
    # Check 4: kkt-scratch only in [dev-dependencies].
    (
        if .name == "kkt-scratch" and $kind != "dev"
        then "\($package.name): lists kkt-scratch in \($section); only [dev-dependencies] may"
        else empty end
    )
),
(
    # Checks 2 and 3 for a package's own features that turn on a workspace
    # dependency's real or testing. Anyone may turn on a package's features,
    # so these count as outside [dev-dependencies]. The one exception: a
    # package's own `testing` may turn on a dependency's `testing`, since
    # it is itself on only from [dev-dependencies] (Sam, 2026-11-11).
    .features
    | to_entries[]
    | .key as $feature
    | .value[]
    | capture("^(?<dependency>[^/?]+)\\??/(?<enables>real|testing)$")
    | select(.dependency as $key | any($local[]; (.rename // .name) == $key))
    | select(.enables == "testing" or ($program | not))
    | select(($feature == "testing" and .enables == "testing") | not)
    | "\($package.name): its feature \($feature) turns on \(.dependency)'s \(.enables); "
        + (if .enables == "real" then "only a package with a binary target may, in [dependencies]" else "only [dev-dependencies] may" end)
),
(
    # Checks 2 and 3 for a library's `default` feature, which every build of
    # the library gets unless it asks otherwise: it may not turn on real or
    # testing, its own or a workspace dependency's, whether it lists them
    # itself or through its other features at any depth.
    if $program then empty else
        .features as $features
        | default_reach($features)[] as $feature
        | ($features[$feature] // [])[]
        | if . == "real" or . == "testing"
            then {whose: "its own", enables: .}
            else
                capture("^(?<dependency>[^/?]+)\\??/(?<enables>real|testing)$")
                | select(.dependency as $key | any($local[]; (.rename // .name) == $key))
                | {whose: "\(.dependency)'s", enables}
            end
        | "\($package.name): its default feature turns on \(.whose) \(.enables)"
            + (if $feature == "default" then "" else " (through its feature \($feature))" end)
            + "; a library's default feature may not"
    end
),
(
    # Check 5: every package's name starts with kkt-, so its code logs under
    # a kkt_ path and the log filter keeps the lines at kkt's own level
    # (LOG_OWN_TARGETS in config.rs). A package with a binary target is a
    # program, whose name is the one people install, and is not held to it.
    if ($program | not) and (.name | startswith("kkt-") | not)
    then "\(.name): its name does not start with kkt-; every package without a binary target must"
    else empty end
)
