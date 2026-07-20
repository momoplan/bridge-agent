#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 4 ] || [ "$#" -gt 6 ]; then
  echo "usage: $0 <repository> <workflow-file> <tag> <expected-sha> [event] [head-branch]" >&2
  exit 2
fi

repository="$1"
workflow="$2"
tag="$3"
expected_sha="$4"
event="${5:-push}"
head_branch="${6:-$tag}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"

case "$event" in push|workflow_dispatch) ;; *) echo "unsupported workflow event: $event" >&2; exit 2 ;; esac

api="https://api.github.com/repos/${repository}/actions/workflows/${workflow}/runs?event=${event}&per_page=100"
for attempt in $(seq 1 120); do
  payload="$(curl -fsS \
    --connect-timeout 20 \
    --retry 6 \
    --retry-all-errors \
    --retry-delay 5 \
    -H "Authorization: Bearer ${GITHUB_TOKEN}" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "$api")"
  run="$(printf '%s' "$payload" | jq -c \
    --arg head_branch "$head_branch" \
    --arg sha "$expected_sha" \
    '[.workflow_runs[] | select(.head_branch == $head_branch and .head_sha == $sha)] | sort_by(.run_number) | last // empty')"
  if [ -n "$run" ]; then
    status="$(printf '%s' "$run" | jq -r '.status')"
    conclusion="$(printf '%s' "$run" | jq -r '.conclusion // ""')"
    url="$(printf '%s' "$run" | jq -r '.html_url')"
    echo "workflow=${workflow} event=${event} tag=${tag} status=${status} conclusion=${conclusion} url=${url}"
    if [ "$status" = "completed" ]; then
      if [ "$conclusion" = "success" ]; then
        exit 0
      fi
      echo "GitHub workflow failed: ${url}" >&2
      exit 1
    fi
  else
    echo "waiting for workflow=${workflow} event=${event} tag=${tag} attempt=${attempt}/120"
  fi
  sleep 30
done

echo "timed out waiting for workflow=${workflow} tag=${tag}" >&2
exit 1
