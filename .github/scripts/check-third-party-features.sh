#!/bin/bash
# Compare the output of third-party-features.sh, read on standard input, with
# the list kept in the repository (the file named as the only argument,
# .github/third-party-features.txt in CI). Exit 0 if they are the same.
# Otherwise print the difference and how to update the list, and exit 1.
#
# CI runs:
#   LC_ALL=C bash .github/scripts/third-party-features.sh \
#       | bash .github/scripts/check-third-party-features.sh .github/third-party-features.txt
# Tested by test_check_third_party_features.py.
set -euo pipefail
if [ "$#" -ne 1 ]; then
    echo "usage: third-party-features.sh | check-third-party-features.sh <list file>" >&2
    exit 2
fi
list=$1
if [ ! -f "$list" ]; then
    echo "check-third-party-features: no list file at $list" >&2
    exit 2
fi
if ! diff -u --label "$list (kept in the repository)" --label "third-party-features.sh (this build)" "$list" -; then
    cat >&2 <<EOF

check-third-party-features: the third-party packages or features built into
kkt's programs differ from $list (the difference is above).
If this pull request does not mean to change them, find what changed them.
If it does, for example after a dependency update, update the list in the
same pull request and say in its description that the change is intended:
    LC_ALL=C bash .github/scripts/third-party-features.sh > $list
EOF
    exit 1
fi
