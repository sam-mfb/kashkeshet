#!/usr/bin/env bash
# Runs the crate map's headless page test, page-test.js, and exits non-zero
# when any check fails. Playwright, at the version
# page-test/package-lock.json pins, and its Chromium are installed under
# target/tmp/page-test the first time, again whenever the lock file
# changes, and again after an install that did not finish.
#
# Usage, from anywhere in the repository:
#   docs/architecture/run-page-test.sh
#   docs/architecture/run-page-test.sh --with-system-deps
# The second also installs the system libraries Chromium needs, with the
# system's package manager (CI uses it; on Linux it asks for sudo).
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
pins="$root/docs/architecture/page-test"
dir="$root/target/tmp/page-test"

case "${1-}" in
  "") with_deps=() ;;
  --with-system-deps) with_deps=(--with-deps) ;;
  *) echo "usage: $0 [--with-system-deps]" >&2; exit 2 ;;
esac

# installed.lock is the lock file of the last install that finished, so an
# install that stops partway is tried again on the next run.
mkdir -p "$dir"
if ! cmp -s "$pins/package-lock.json" "$dir/installed.lock"; then
  rm -f "$dir/installed.lock"
  cp "$pins/package.json" "$pins/package-lock.json" "$dir/"
  npm ci --prefix "$dir" --no-audit --no-fund
  cp "$pins/package-lock.json" "$dir/installed.lock"
fi
"$dir/node_modules/.bin/playwright" install ${with_deps[@]+"${with_deps[@]}"} chromium
NODE_PATH="$dir/node_modules" node "$root/docs/architecture/page-test.js"
