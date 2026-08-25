#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <tauri-build-command> [args...]" >&2
  exit 2
fi

max_attempts="${TAURI_NOTARY_MAX_ATTEMPTS:-4}"
retry_delay_seconds="${TAURI_NOTARY_RETRY_DELAY_SECONDS:-15}"
sleep_bin="${SLEEP_BIN:-sleep}"
case "$max_attempts" in
  ''|*[!0-9]*|0) echo "TAURI_NOTARY_MAX_ATTEMPTS must be a positive integer" >&2; exit 2 ;;
esac
case "$retry_delay_seconds" in
  ''|*[!0-9]*) echo "TAURI_NOTARY_RETRY_DELAY_SECONDS must be a non-negative integer" >&2; exit 2 ;;
esac

log_file="$(mktemp "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/tauri-notary.XXXXXX")"
cleanup() {
  rm -f "$log_file"
}
trap cleanup EXIT

attempt=1
while [ "$attempt" -le "$max_attempts" ]; do
  : > "$log_file"
  set +e
  "$@" 2>&1 | tee "$log_file"
  status="${PIPESTATUS[0]}"
  set -e
  if [ "$status" -eq 0 ]; then
    exit 0
  fi

  if ! grep -Eiq 'failed to notarize|notarytool|appstoreconnect\.apple\.com/notary' "$log_file" \
    || ! grep -Eiq 'NSURLErrorDomain Code=-(1001|1003|1005|1009)|Internet connection appears to be offline|network connection was lost|could not resolve host|connection (reset|timed out)|temporarily unavailable|HTTPError\(statusCode: (429|5[0-9]{2})' "$log_file"; then
    echo "Tauri build failed without a transient Apple notarization error; not retrying" >&2
    exit "$status"
  fi

  if [ "$attempt" -eq "$max_attempts" ]; then
    echo "Tauri build failed after $max_attempts transient Apple notarization attempts" >&2
    exit "$status"
  fi

  echo "Transient Apple notarization failure; retrying Tauri build in ${retry_delay_seconds}s ($attempt/$max_attempts)" >&2
  "$sleep_bin" "$retry_delay_seconds"
  attempt=$((attempt + 1))
  if [ "$retry_delay_seconds" -lt 60 ]; then
    retry_delay_seconds=$((retry_delay_seconds * 2))
    if [ "$retry_delay_seconds" -gt 60 ]; then
      retry_delay_seconds=60
    fi
  fi
done
