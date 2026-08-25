#!/bin/bash
# Cicada dev launcher (macOS; docs/17 wave 4 L3). Double-click: Terminal
# opens on this script. It finds Python 3 and hands over to launch.py, which
# builds cicada in release with the app embedded when it is missing or
# stale, puts the kernel's run-time libraries beside it (the binary's rpath
# rewritten to @executable_path/lib), and runs `cicada app` (arguments given
# here go to it). Every failure is printed and this window stays open on one.
# bash 3.2 (the system's), no GNU-only flags.
here="$(cd "$(dirname "$0")" && pwd)"
py=""
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1 &&
    "$candidate" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' >/dev/null 2>&1; then
    py="$candidate"
    break
  fi
done
if [ -z "$py" ]; then
  echo "error: Python 3.9 or newer is required and neither python3 nor python is one."
  echo "       macOS ships python3 with the Xcode command line tools (xcode-select --install),"
  echo "       which the Rust build needs anyway; or install it from https://www.python.org/downloads/."
  read -r -p "Press Return to close this window. "
  exit 1
fi
"$py" "$here/launch.py" "$@"
code=$?
if [ "$code" -ne 0 ]; then
  echo
  echo "Cicada's launcher stopped with exit code $code -- see the messages above."
  read -r -p "Press Return to close this window. "
fi
exit "$code"
