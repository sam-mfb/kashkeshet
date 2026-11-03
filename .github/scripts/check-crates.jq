# Checks 2 to 4 of check-crates.sh, over `cargo metadata --no-deps` output.
# Prints one line per problem, and nothing when the manifests follow the
# rules. No package is named here except kkt-scratch, the subject of check 4.
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
    # so these count as outside [dev-dependencies].
    .features
    | to_entries[]
    | .key as $feature
    | .value[]
    | capture("^(?<dependency>[^/?]+)\\??/(?<enables>real|testing)$")
    | select(.dependency as $key | any($local[]; (.rename // .name) == $key))
    | select(.enables == "testing" or ($program | not))
    | "\($package.name): its feature \($feature) turns on \(.dependency)'s \(.enables); "
        + (if .enables == "real" then "only a package with a binary target may, in [dependencies]" else "only [dev-dependencies] may" end)
)
