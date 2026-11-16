# Checks 2 to 6 of check-crates.sh, over `cargo metadata --no-deps` output,
# with `$effects`, the names of the crates docs/architecture/concerns.json
# marks "effect", and `$effect_exceptions`, the documented exceptions to
# check 2 (effect-exceptions.json: package, dependency, reason).
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

# Check 2's exceptions: a listed exception that no longer applies fails, so
# the list says only what is true.
(
    . as $root
    | $effect_exceptions[]
    | . as $exception
    | select(
        any($root.packages[]; .name == $exception.package
            and any(.dependencies[]; .name == $exception.dependency))
        | not)
    | "effect-exceptions.json lists \($exception.package) depending on \($exception.dependency), which it no longer does; remove the entry"
),
(
[.packages[].name] as $workspace
| .packages[]
| . as $package
| any(.targets[]; any(.kind[]; . == "bin")) as $program
# A dev-only program: a binary never released, which counts as test code.
| ($program and (.metadata.kkt["dev-only"] // false) == true) as $dev_program
| [.dependencies[] | select(.name as $name | any($workspace[]; . == $name))] as $local
| (
    $local[]
    | . as $dependency
    | (.kind // "normal") as $kind
    | ({normal: "[dependencies]", dev: "[dev-dependencies]", build: "[build-dependencies]"}[$kind]
        + (if .target then " for \(.target)" else "" end)) as $section
    # Check 2: only programs depend on an effect crate. A program's tests
    # count too: an effect crate it names only in [dev-dependencies], and
    # not in [dependencies] or [build-dependencies], is one only its tests
    # use. Anything else that must is a documented exception, in
    # $effect_exceptions.
    | (
        if any($effects[]; . == $dependency.name)
            and (($program | not)
                or ($kind == "dev" and (any($local[]; .name == $dependency.name and (.kind // "normal") != "dev") | not)))
            and (any($effect_exceptions[];
                .package == $package.name and .dependency == $dependency.name) | not)
        then if $program
            then "\($package.name): its tests depend on the effect crate \($dependency.name), which the program does not; list it in effect-exceptions.json with the reason, or do without it"
            else "\($package.name): depends on the effect crate \($dependency.name) in \($section); only a program (a package with a binary target) may, unless listed in effect-exceptions.json"
            end
        else empty end
    ),
    # Check 3: another package's testing only in [dev-dependencies], or in
    # a dev-only program's [dependencies].
    (
        if any(.features[]; . == "testing") and $kind != "dev"
            and (($kind == "normal" and $dev_program) | not)
        then "\($package.name): turns on \($dependency.name)'s testing in \($section); only [dev-dependencies] may, or a dev-only program's [dependencies]"
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
    # Check 3 for a package's own features that turn on a workspace
    # dependency's testing. Anyone may turn on a package's features, so
    # these count as outside [dev-dependencies]. The one exception: a
    # package's own `testing` may turn on a dependency's `testing`, since
    # it is itself on only from [dev-dependencies] (Sam, 2026-11-11).
    .features
    | to_entries[]
    | .key as $feature
    | .value[]
    | capture("^(?<dependency>[^/?]+)\\??/testing$")
    | select(.dependency as $key | any($local[]; (.rename // .name) == $key))
    | select($feature != "testing")
    | "\($package.name): its feature \($feature) turns on \(.dependency)'s testing; only [dev-dependencies] may"
),
(
    # Check 3 for a library's `default` feature, which every build of the
    # library gets unless it asks otherwise: it may not turn on testing, its
    # own or a workspace dependency's, whether it lists it itself or through
    # its other features at any depth.
    if $program then empty else
        .features as $features
        | default_reach($features)[] as $feature
        | ($features[$feature] // [])[]
        | if . == "testing"
            then {whose: "its own"}
            else
                capture("^(?<dependency>[^/?]+)\\??/testing$")
                | select(.dependency as $key | any($local[]; (.rename // .name) == $key))
                | {whose: "\(.dependency)'s"}
            end
        | "\($package.name): its default feature turns on \(.whose) testing"
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
),
(
    # Check 6: no package has a feature named `real`. Code that touches the
    # machine is in an effect crate of its own, not behind a feature
    # (phase 16).
    if .features | has("real")
    then "\(.name): it has a `real` feature; code that touches the machine belongs in an effect crate, not behind a feature"
    else empty end
)
)
