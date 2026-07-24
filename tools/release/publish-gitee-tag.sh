#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <tag> <expected-commit>" >&2
  exit 2
fi

tag="$1"
expected_commit="$2"
remote="${GITEE_REMOTE_URL:-https://gitee.com/zxflimit_admin/bridge-agent.git}"
: "${GITEE_USER:?GITEE_USER is required}"
: "${GITEE_ACCESS_TOKEN:?GITEE_ACCESS_TOKEN is required}"

if ! [[ "$tag" =~ ^(bridge-agent|baijimu-cli)-v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid Bridge Agent or Baijimu CLI release tag: $tag" >&2
  exit 2
fi
if ! [[ "$expected_commit" =~ ^[0-9a-f]{40}$ ]]; then
  echo "expected commit must be an exact 40-character SHA" >&2
  exit 2
fi

local_tag_commit="$(git rev-parse -q --verify "refs/tags/${tag}^{commit}" || true)"
if [ "$local_tag_commit" != "$expected_commit" ]; then
  echo "local tag $tag points to ${local_tag_commit:-nothing}, expected $expected_commit" >&2
  exit 1
fi

askpass="$(mktemp)"
cleanup() {
  rm -f "$askpass"
}
trap cleanup EXIT
chmod 700 "$askpass"
cat > "$askpass" <<'ASKPASS'
#!/bin/sh
case "$1" in
  *Username*) printf '%s\n' "$GITEE_USER" ;;
  *Password*) printf '%s\n' "$GITEE_ACCESS_TOKEN" ;;
  *) exit 1 ;;
esac
ASKPASS

export GIT_ASKPASS="$askpass"
export GIT_TERMINAL_PROMPT=0

remote_refs="$(git ls-remote "$remote" "refs/tags/$tag" "refs/tags/$tag^{}")"
remote_tag_commit="$(printf '%s\n' "$remote_refs" | awk -v peeled="refs/tags/${tag}^{}" '$2 == peeled {print $1; exit}')"
if [ -z "$remote_tag_commit" ]; then
  remote_tag_commit="$(printf '%s\n' "$remote_refs" | awk -v direct="refs/tags/${tag}" '$2 == direct {print $1; exit}')"
fi

if [ -n "$remote_tag_commit" ]; then
  if [ "$remote_tag_commit" != "$expected_commit" ]; then
    echo "Gitee tag $tag already points to $remote_tag_commit, expected $expected_commit" >&2
    exit 1
  fi
  echo "Gitee tag $tag already points to $expected_commit"
  exit 0
fi

git push "$remote" "refs/tags/$tag:refs/tags/$tag"

published_refs="$(git ls-remote "$remote" "refs/tags/$tag" "refs/tags/$tag^{}")"
published_commit="$(printf '%s\n' "$published_refs" | awk -v peeled="refs/tags/${tag}^{}" '$2 == peeled {print $1; exit}')"
if [ -z "$published_commit" ]; then
  published_commit="$(printf '%s\n' "$published_refs" | awk -v direct="refs/tags/${tag}" '$2 == direct {print $1; exit}')"
fi
if [ "$published_commit" != "$expected_commit" ]; then
  echo "Gitee tag verification failed for $tag: ${published_commit:-missing}" >&2
  exit 1
fi

echo "Published immutable Gitee tag $tag at $expected_commit"
