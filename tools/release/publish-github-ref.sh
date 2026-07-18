#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <repository> <branch|tag> <name> <sha>" >&2
  exit 2
fi

repository="$1"
kind="$2"
name="$3"
sha="$4"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"

case "$kind" in
  branch) namespace="heads" ;;
  tag) namespace="tags" ;;
  *) echo "kind must be branch or tag" >&2; exit 2 ;;
esac
if ! [[ "$name" =~ ^[0-9A-Za-z._/-]+$ ]] || ! [[ "$sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "invalid GitHub ref input" >&2
  exit 2
fi

api="https://api.github.com/repos/${repository}"
headers=(
  -H "Authorization: Bearer ${GITHUB_TOKEN}"
  -H "Accept: application/vnd.github+json"
  -H "X-GitHub-Api-Version: 2022-11-28"
)
response="$(curl -sS --connect-timeout 10 --max-time 60 \
  "${headers[@]}" -w $'\n%{http_code}' \
  "${api}/git/ref/${namespace}/${name}")"
status="${response##*$'\n'}"
body="${response%$'\n'*}"

if [ "$status" = 200 ]; then
  current="$(printf '%s' "$body" | jq -r '.object.sha')"
  if [ "$current" = "$sha" ]; then
    echo "GitHub ${kind} ${name} already points to ${sha}"
    exit 0
  fi
  if [ "$kind" = tag ]; then
    echo "GitHub tag ${name} already points to ${current}, expected ${sha}" >&2
    exit 1
  fi
  curl -fsS --connect-timeout 10 --max-time 60 \
    "${headers[@]}" \
    -X PATCH "${api}/git/refs/heads/${name}" \
    -d "$(jq -nc --arg sha "$sha" '{sha: $sha, force: false}')" \
    | jq -e --arg sha "$sha" '.object.sha == $sha' >/dev/null
  echo "updated GitHub branch ${name} to ${sha}"
  exit 0
fi

if [ "$status" != 404 ]; then
  printf '%s\n' "$body" >&2
  echo "GitHub ref lookup failed with HTTP ${status}" >&2
  exit 1
fi
if [ "$kind" = branch ]; then
  echo "GitHub branch ${name} does not exist" >&2
  exit 1
fi

curl -fsS --connect-timeout 10 --max-time 60 \
  "${headers[@]}" \
  -X POST "${api}/git/refs" \
  -d "$(jq -nc --arg ref "refs/tags/${name}" --arg sha "$sha" '{ref: $ref, sha: $sha}')" \
  | jq -e --arg sha "$sha" '.object.sha == $sha' >/dev/null
echo "created GitHub tag ${name} at ${sha}"
