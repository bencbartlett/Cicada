#!/usr/bin/env bash
# The release workflow's stamp (docs/17 wave 5 R1; fix round 2026-09-20,
# findings L3-5 and R1-C8). Two modes:
#
#   bash tools/release_stamp.sh
#       In a checkout: HEAD must be the commit the pushed object names
#       (`$GITHUB_SHA` peeled — for an annotated tag the pushed object is the
#       tag, not its commit), else stop with both hashes. Then append
#       CICADA_GIT_SHA (HEAD's full hash) and SOURCE_DATE_EPOCH (HEAD's
#       committer time) to $GITHUB_ENV, so every binary stamps the tagged
#       commit and ITS date — a re-run of the same tag on another day
#       builds the same `built`, and the date means "the release's".
#
#   bash tools/release_stamp.sh --check "<cicada --version line>" <version>
#       The built binary's line must be exactly
#       `cicada <version> (<HEAD short-12>, <HEAD's UTC date>)` — an
#       independent witness: the expected values are read from git here,
#       never from the variables that fed the build.
#
# Outside CI (no GITHUB_ENV) the stamp mode prints the two lines instead.
set -euo pipefail

head="$(git rev-parse HEAD)"
short="$(git rev-parse --short=12 HEAD)"
epoch="$(git log -1 --format=%ct HEAD)"
date="$(python -c "import datetime, sys; print(datetime.datetime.fromtimestamp(int(sys.argv[1]), datetime.timezone.utc).strftime('%Y-%m-%d'))" "$epoch")"

if [ "${1:-}" = "--check" ]; then
  line="${2:?the --version line}"
  version="${3:?the release version}"
  expected="cicada $version ($short, $date)"
  echo "$line"
  if [ "$line" != "$expected" ]; then
    echo "error: the binary says \`$line\`, the checkout says \`$expected\` (HEAD $head, committed $epoch)" >&2
    exit 1
  fi
  echo "the binary stamps HEAD's commit and date"
  exit 0
fi

if [ -n "${GITHUB_SHA:-}" ]; then
  # Peel an annotated tag's object to its commit; a lightweight tag or a
  # branch head is already one.
  pushed="$(git rev-parse "${GITHUB_SHA}^{commit}")"
  if [ "$pushed" != "$head" ]; then
    echo "error: the checkout's HEAD is $head but the pushed object names commit $pushed — refusing to stamp a hash this checkout did not build" >&2
    exit 1
  fi
fi
if [ -n "${GITHUB_ENV:-}" ]; then
  {
    echo "CICADA_GIT_SHA=$head"
    echo "SOURCE_DATE_EPOCH=$epoch"
  } >> "$GITHUB_ENV"
  echo "stamping HEAD $short, committed $date (SOURCE_DATE_EPOCH=$epoch)"
else
  echo "CICADA_GIT_SHA=$head"
  echo "SOURCE_DATE_EPOCH=$epoch"
fi
