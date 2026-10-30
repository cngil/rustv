#!/bin/sh
# Runs the same checks as CI (.github/workflows/ci.yml) on this machine.
#
#   scripts/check.sh           shellcheck, fmt, clippy, tests
#   scripts/check.sh --remote  the above, then the web remote's browser tests
#
# The browser tests start a real tvd, so they need write access to /dev/uinput.

set -eu

cd "$(dirname "$0")/.."

remote=false
for arg in "$@"; do
  case "$arg" in
    --remote) remote=true ;;
    *) echo "usage: $0 [--remote]" >&2; exit 2 ;;
  esac
done

step() { printf '\n==> %s\n' "$*"; }

step shellcheck
shellcheck scripts/*.sh

step "cargo fmt"
cargo fmt --all --check

step "cargo clippy"
cargo clippy --workspace --all-targets -- -D warnings

step "cargo test"
cargo test --workspace

if [ "$remote" = true ]; then
  if [ ! -w /dev/uinput ]; then
    echo "error: /dev/uinput is not writable; tvd cannot start (see the README)" >&2
    exit 1
  fi
  step "web remote (browser tests)"
  cargo build -p tvd
  cd tests/remote
  [ -d node_modules ] || npm ci
  npx playwright install chromium
  npx playwright test
fi

printf '\nAll checks passed.\n'
