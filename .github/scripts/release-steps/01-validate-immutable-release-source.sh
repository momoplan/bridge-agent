set -euo pipefail
case "$RELEASE_TAG" in
  bridge-agent-v*) ;;
  *)
    echo "Invalid release tag: $RELEASE_TAG" >&2
    exit 1
    ;;
esac

version="${RELEASE_TAG#bridge-agent-v}"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid release version: $version" >&2
  exit 1
fi

assert_version() {
  local label="$1"
  local actual="$2"
  if [ "$actual" != "$version" ]; then
    echo "$label version $actual does not match release tag $RELEASE_TAG" >&2
    exit 1
  fi
}

if [ "$REPAIR_ASSETS_ONLY" != "true" ]; then
  assert_version package.json "$(node -p 'require("./package.json").version')"
  assert_version src-tauri/tauri.conf.json "$(node -p 'require("./src-tauri/tauri.conf.json").version')"
  assert_version Cargo.toml "$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)"
  assert_version src-tauri/Cargo.toml "$(sed -n 's/^version = "\([^"]*\)"/\1/p' src-tauri/Cargo.toml | head -1)"
  assert_version tools/windows-uninstaller/Cargo.toml "$(sed -n 's/^version = "\([^"]*\)"/\1/p' tools/windows-uninstaller/Cargo.toml | head -1)"
fi

test "$(git rev-parse HEAD)" = "$GITHUB_SHA"
tag_commit="$(git rev-parse -q --verify "refs/tags/${RELEASE_TAG}^{commit}" || true)"
if [ -z "$tag_commit" ]; then
  echo "Release tag $RELEASE_TAG is not available in the GitHub repository" >&2
  exit 1
fi

git fetch --no-tags origin refs/heads/main:refs/remotes/origin/main
if [ "$GITHUB_EVENT_NAME" = push ]; then
  if [ "$tag_commit" != "$GITHUB_SHA" ]; then
    echo "Release tag $RELEASE_TAG points to $tag_commit, expected $GITHUB_SHA" >&2
    exit 1
  fi
  if ! git merge-base --is-ancestor "$GITHUB_SHA" refs/remotes/origin/main; then
    echo "Release commit $GITHUB_SHA must already belong to GitHub main" >&2
    exit 1
  fi
else
  if ! git merge-base --is-ancestor "$tag_commit" "$GITHUB_SHA"; then
    echo "Repair source $GITHUB_SHA must descend from immutable tag $RELEASE_TAG ($tag_commit)" >&2
    exit 1
  fi
fi

cli_version="$(tr -d '[:space:]' < tools/baijimu-cli/VERSION)"
cli_app_id="$(tr -d '[:space:]' < tools/baijimu-cli/APP_ID)"
case "$cli_version" in
  *[!0-9.]*|'')
    echo "Invalid bundled Baijimu CLI version: $cli_version" >&2
    exit 1
    ;;
esac
if [ -z "$cli_app_id" ]; then
  echo "Missing bundled Baijimu CLI app ID" >&2
  exit 1
fi

if [ "$REPAIR_ASSETS_ONLY" != "true" ]; then
  node .github/scripts/prepare-bundled-market-source.mjs
fi
