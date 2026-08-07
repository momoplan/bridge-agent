#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <connector-version> <connector-manifest>" >&2
  exit 2
fi

version="$1"
connector_manifest_path="$2"
publication_status_file="${MARKET_PUBLICATION_STATUS_FILE:-$PWD/.codex-local-app-publication-status}"
rm -f "$publication_status_file"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid connector version: $version" >&2
  exit 2
fi
if [ ! -f "$connector_manifest_path" ]; then
  echo "connector manifest does not exist: $connector_manifest_path" >&2
  exit 2
fi

connector_manifest="$(jq -ce \
  --arg version "$version" \
  '
    select(.schemaVersion == "2.0")
    | select(.id == "com.baijimu.connector.codex")
    | select(.version == $version)
    | select(.source.revision == ("v" + $version))
    | select(.runtime.type == "process")
    | select((.runtime.command | type) == "string" and (.runtime.command | length) > 0)
    | select(.runtime.processOwnership == "host")
    | select(.runtime.args == ["start"])
    | select(.runtime.stopArgs == ["stop"])
    | select(.hostRequirements.minimumVersion == "0.2.40")
    | select((.hostRequirements.capabilities // []) | index("connector.process.host-managed.v1") != null)
  ' "$connector_manifest_path")" || {
  echo "connector manifest identity, version, source revision, or runtime contract is invalid" >&2
  exit 2
}

: "${LOCAL_APP_MARKET_PUBLISH_TOKEN:?LOCAL_APP_MARKET_PUBLISH_TOKEN is required}"

for dependency in curl jq sha256sum awk grep seq; do
  if ! command -v "$dependency" >/dev/null 2>&1; then
    echo "required market publisher dependency is unavailable: ${dependency}" >&2
    exit 127
  fi
done

BAIJIMU_CLI="${BAIJIMU_CLI:-$(command -v baijimu || true)}"
if [ -z "$BAIJIMU_CLI" ] || [ ! -x "$BAIJIMU_CLI" ]; then
  echo "Baijimu CLI is required; set BAIJIMU_CLI to the pinned release binary" >&2
  exit 127
fi

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/codex-local-app-publish.XXXXXX")"
cleanup() {
  rm -rf "$work_dir"
}
trap cleanup EXIT

auth_file="$work_dir/baijimu-auth.json"
BAIJIMU_AUTH_FILE="$auth_file" "$BAIJIMU_CLI" auth login \
  --token "$LOCAL_APP_MARKET_PUBLISH_TOKEN" \
  --workspace-id 1211 \
  --no-browser \
  --json \
  >/dev/null
BAIJIMU_AUTH_FILE="$auth_file" "$BAIJIMU_CLI" local-app get codex --json \
  | jq -e '(.data // .).id == "codex" and (.data // .).connectorId == "com.baijimu.connector.codex"' \
  >/dev/null

if [ "${VALIDATE_ONLY:-false}" = "true" ]; then
  echo "validated Codex local app publisher identity and pinned CLI"
  exit 0
fi

: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"

release_tag="codex-local-app-v${version}"
release_json="$(curl -fsS \
  --retry 3 \
  --retry-all-errors \
  --retry-delay 2 \
  --connect-timeout 10 \
  --max-time 60 \
  -H "Authorization: Bearer ${GITHUB_TOKEN}" \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2022-11-28' \
  "https://api.github.com/repos/momoplan/bridge-agent/releases/tags/${release_tag}")"
printf '%s' "$release_json" | jq -e \
  --arg tag "$release_tag" \
  '.tag_name == $tag and .draft == false and .prerelease == false' \
  >/dev/null

declare -A checksums
declare -A sources

download_release_asset() {
  local asset_name="$1"
  local destination="$2"
  local asset_url
  asset_url="$(printf '%s' "$release_json" | jq -er \
    --arg name "$asset_name" \
    '.assets[] | select(.name == $name and .state == "uploaded" and .size > 0) | .browser_download_url')"
  case "$asset_url" in
    "https://github.com/momoplan/bridge-agent/releases/download/${release_tag}/${asset_name}") ;;
    *) echo "invalid immutable GitHub release URL for ${asset_name}: ${asset_url}" >&2; exit 1 ;;
  esac
  curl -fsSL \
    --retry 6 \
    --retry-all-errors \
    --retry-delay 3 \
    --connect-timeout 15 \
    --max-time 900 \
    -H "Authorization: Bearer ${GITHUB_TOKEN}" \
    "$asset_url" \
    -o "$destination"
}

manifest_asset="baijimu-codex-local-app-${version}-oss-manifest.json"
download_release_asset "$manifest_asset" "$work_dir/$manifest_asset"
manifest_json="$(cat "$work_dir/$manifest_asset")"
printf '%s' "$manifest_json" | jq -e \
  --arg version "$version" \
  '.schemaVersion == "1.0" and
   .applicationId == "codex" and
   .connectorId == "com.baijimu.connector.codex" and
   .version == $version and
   (.artifacts | length) == 3 and
   all(.artifacts[]; .source | startswith("https://lowcode-common.oss-cn-beijing.aliyuncs.com/local-app-artifacts/codex/releases/v" + $version + "/"))' \
  >/dev/null

for platform in macos windows linux; do
  checksum="$(printf '%s' "$manifest_json" | jq -er \
    --arg platform "$platform" \
    '.artifacts[] | select(.platform == $platform) | .checksum | sub("^sha256:"; "")')"
  source_url="$(printf '%s' "$manifest_json" | jq -er \
    --arg platform "$platform" \
    '.artifacts[] | select(.platform == $platform) | .source')"
  if ! [[ "$checksum" =~ ^[0-9a-fA-F]{64}$ ]]; then
    echo "OSS manifest did not return a valid SHA-256 for ${platform}" >&2
    exit 1
  fi
  checksum="$(printf '%s' "$checksum" | tr '[:upper:]' '[:lower:]')"
  case "$source_url" in
    https://lowcode-common.oss-cn-beijing.aliyuncs.com/local-app-artifacts/codex/releases/v${version}/*) ;;
    *) echo "invalid immutable public OSS URL for ${platform}: ${source_url}" >&2; exit 1 ;;
  esac
  checksums[$platform]="$checksum"
  curl -fsSL \
    --retry 6 \
    --retry-all-errors \
    --retry-delay 3 \
    --connect-timeout 15 \
    --max-time 900 \
    "$source_url" \
    -o "$work_dir/oss-${platform}.zip"
  oss_checksum="$(sha256sum "$work_dir/oss-${platform}.zip" | awk '{print $1}')"
  if [ "$oss_checksum" != "$checksum" ]; then
    echo "anonymous OSS download checksum mismatch for ${platform}" >&2
    exit 1
  fi
  sources[$platform]="$source_url"
done

manifest="$(jq -nc \
  --argjson connector "$connector_manifest" \
  --arg mac_source "${sources[macos]}" \
  --arg win_source "${sources[windows]}" \
  --arg linux_source "${sources[linux]}" \
  --arg mac_sha "sha256:${checksums[macos]}" \
  --arg win_sha "sha256:${checksums[windows]}" \
  --arg linux_sha "sha256:${checksums[linux]}" \
  '($connector + {
    applicationType: "connector",
    artifacts: [
      {platform: "macos", arch: "universal", source: $mac_source, checksum: $mac_sha},
      {platform: "windows", arch: "x86_64", source: $win_source, checksum: $win_sha},
      {platform: "linux", arch: "x86_64", source: $linux_source, checksum: $linux_sha}
    ]
  })')"
capabilities="$(printf '%s' "$connector_manifest" | jq -c '[.remoteCapabilities[]?.name]')"

for document in "$manifest" "$capabilities"; do
  printf '%s' "$document" | jq -e . >/dev/null
done
printf '%s' "$manifest" | jq -e \
  --arg version "$version" \
  --arg prefix "https://lowcode-common.oss-cn-beijing.aliyuncs.com/local-app-artifacts/codex/releases/v${version}/" \
  '.schemaVersion == "2.0" and
   .applicationType == "connector" and
   .id == "com.baijimu.connector.codex" and
   .version == $version and
   (.runtime | type) == "object" and
   (.transport | type) == "object" and
   (((.methods // []) | length) + ((.events // []) | length) > 0) and
   (has("services") | not) and
   (has("serviceRegistrationFiles") | not) and
   (.artifacts | length) == 3 and
   all(.artifacts[]; (.source | startswith($prefix)) and (.checksum | test("^sha256:[0-9a-f]{64}$")))' \
  >/dev/null

publish_body="$work_dir/publish.json"
jq -n \
  --arg version "$version" \
  --arg source "${sources[macos]}" \
  --arg repo 'zxflimit_admin/baijimu-connector-codex' \
  --arg revision "v${version}" \
  --arg checksum "${checksums[macos]}" \
  --argjson capabilities "$capabilities" \
  --argjson manifest "$manifest" \
  '{version:$version,sourceType:"https",source:$source,repo:$repo,revision:$revision,
    checksum:$checksum,capabilities:$capabilities,manifest:$manifest}' \
  > "$publish_body"

publish_response="$(BAIJIMU_AUTH_FILE="$auth_file" "$BAIJIMU_CLI" local-app publish codex \
  --data "@${publish_body}" \
  --json)"
printf '%s' "$publish_response" | jq -e \
  --arg version "$version" \
  '(.errorCode == "0") and
   (.data.appId == "codex") and
   (.data.version == $version) and
   (.data.status == "PENDING_REVIEW" or .data.status == "PUBLISHED")' \
  >/dev/null
publication_id="$(printf '%s' "$publish_response" | jq -r '.data.publicationId // empty')"
publication_status="$(printf '%s' "$publish_response" | jq -r '.data.status')"
printf '%s\n' "$publication_status" > "$publication_status_file"
echo "submitted Codex local app ${version}; publication=${publication_id:-existing}; status=${publication_status}"

# Human review belongs to local-app-market, not to the Jenkins executor.  A
# successful submission is a complete release-pipeline outcome even though the
# public catalog cannot expose the version until an independent reviewer acts.
# Re-running the same immutable release after approval returns PUBLISHED and
# performs the public propagation checks below.
if [ "$publication_status" = "PENDING_REVIEW" ]; then
  echo "Codex local app ${version} is pending independent market review; public verification is deferred"
  exit 0
fi

verified=false
for attempt in $(seq 1 40); do
  all_targets_verified=true
  for target in 'macos&arch=aarch64' 'windows&arch=x86_64' 'linux&arch=x86_64'; do
    payload="$(curl -fsS --retry 2 --connect-timeout 5 --max-time 15 \
      "https://api.baijimu.com/lowcode3/api/local-app-market/apps/codex?platform=${target}&hostVersion=0.2.40&hostCapabilities=connector.setup.v1,connector.process.host-managed.v1")"
    if ! printf '%s' "$payload" | jq -e \
      --arg version "$version" \
      --argjson expected_manifest "$manifest" \
      --arg mac_source "${sources[macos]}" \
      --arg win_source "${sources[windows]}" \
      --arg linux_source "${sources[linux]}" \
      '(.data // .) |
       .connectorId == "com.baijimu.connector.codex" and
       .latestVersion.version == $version and
       .latestVersion.compatibility.compatible == true and
       .latestVersion.source == $mac_source and
       .latestVersion.manifest == $expected_manifest and
       .latestVersion.manifest.runtime.processOwnership == "host" and
       .latestVersion.manifest.runtime.args == ["start"] and
       .latestVersion.manifest.hostRequirements.minimumVersion == "0.2.40" and
       ((.latestVersion.manifest.hostRequirements.capabilities // []) | index("connector.process.host-managed.v1") != null) and
       any(.latestVersion.manifest.artifacts[]; .platform == "macos" and .source == $mac_source) and
       any(.latestVersion.manifest.artifacts[]; .platform == "windows" and .source == $win_source) and
       any(.latestVersion.manifest.artifacts[]; .platform == "linux" and .source == $linux_source)' \
      >/dev/null; then
      all_targets_verified=false
    fi
  done
  if [ "$all_targets_verified" = true ]; then
    verified=true
    break
  fi
  sleep 3
done

if [ "$verified" != true ]; then
  echo "published market record did not propagate to every public target before the 2 minute deadline" >&2
  exit 1
fi

echo "published Codex local app ${version} from anonymous Baijimu OSS artifacts"
