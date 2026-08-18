#!/usr/bin/env bash

set -euo pipefail

readonly apt_sources_root="${APT_SOURCES_ROOT:-/etc/apt}"
readonly unavailable_mirror="${UBUNTU_APT_UNAVAILABLE_MIRROR:-http://azure.archive.ubuntu.com/ubuntu}"
readonly fallback_mirror="${UBUNTU_APT_FALLBACK_MIRROR:-https://archive.ubuntu.com/ubuntu}"
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

replace_unavailable_runner_mirror() {
  local source_file
  while IFS= read -r -d '' source_file; do
    if grep -Fq "$unavailable_mirror" "$source_file"; then
      echo "Replacing unavailable GitHub runner APT mirror in $source_file"
      local replacement_file
      replacement_file="$(mktemp)"
      sed "s#${unavailable_mirror}#${fallback_mirror}#g" "$source_file" > "$replacement_file"
      run_privileged cp "$replacement_file" "$source_file"
      rm -f "$replacement_file"
    fi
  done < <(
    find "$apt_sources_root" -type f \
      \( -name 'sources.list' -o -name '*.list' -o -name '*.sources' -o -name 'apt-mirrors.txt' \) \
      -print0
  )
}

run_apt_get() {
  run_privileged "$timeout_command" --signal=TERM "${command_timeout_seconds}s" \
    "$apt_get_command" \
    -o "Acquire::Retries=${acquire_retries}" \
    -o "Acquire::http::Timeout=15" \
    -o "Acquire::https::Timeout=15" \
    "$@"
}

replace_unavailable_runner_mirror
run_apt_get update
run_apt_get install -y --no-install-recommends \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf
