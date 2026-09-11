#!/usr/bin/env bash

set -euo pipefail

readonly command_timeout_seconds="${APT_COMMAND_TIMEOUT_SECONDS:-180}"
readonly acquire_retries="${APT_ACQUIRE_RETRIES:-5}"
readonly sudo_command="${APT_SUDO_COMMAND-sudo}"
readonly timeout_command="${APT_TIMEOUT_COMMAND:-timeout}"
readonly apt_get_command="${APT_GET_COMMAND:-apt-get}"

run_privileged() {
  if [ -n "$sudo_command" ]; then
    "$sudo_command" "$@"
  else
    "$@"
  fi
}

run_apt_get() {
  run_privileged "$timeout_command" --signal=TERM "${command_timeout_seconds}s" \
    "$apt_get_command" \
    -o "Acquire::Retries=${acquire_retries}" \
    -o "Acquire::http::Timeout=15" \
    -o "Acquire::https::Timeout=15" \
    "$@"
}

# APT sources and mirror priorities belong to the runner image. Preserve them
# so apt can select among its configured mirrors without collapsing the list.
run_apt_get update
run_apt_get install -y --no-install-recommends \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf
