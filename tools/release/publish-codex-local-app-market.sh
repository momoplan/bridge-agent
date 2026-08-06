#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <connector-version> <connector-manifest>" >&2
  exit 2
fi

version="$1"
connector_manifest_path="$2"
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
  ' "$connector_manifest_path")" || {
  echo "connector manifest identity, version, source revision, or runtime contract is invalid" >&2
  exit 2
}

: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"
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

OSS_BUCKET="${OSS_BUCKET:-lowcode-common}"
OSS_PREFIX="${OSS_PREFIX:-local-app-artifacts/codex}"
OSS_ENDPOINT="${OSS_ENDPOINT:-oss-cn-beijing.aliyuncs.com}"
OSS_CONFIG_FILE="${OSS_CONFIG_FILE:-$HOME/.ossutilconfig}"
OSS_PUBLIC_BASE_URL="${OSS_PUBLIC_BASE_URL:-https://${OSS_BUCKET}.${OSS_ENDPOINT}}"
if command -v ossutil >/dev/null 2>&1; then
  OSS_CLIENT="$(command -v ossutil)"
  OSS_CLIENT_MODE="native"
elif command -v aliyun >/dev/null 2>&1; then
  OSS_CLIENT="$(command -v aliyun)"
  OSS_CLIENT_MODE="aliyun"
else
  echo "ossutil or aliyun oss is required for local app publication" >&2
  exit 127
fi
OSS_AUTH_ARGS=()
if [ -n "${OSS_ACCESS_KEY_ID:-}" ] && [ -n "${OSS_ACCESS_KEY_SECRET:-}" ]; then
  OSS_AUTH_ARGS=(
    --access-key-id "$OSS_ACCESS_KEY_ID"
    --access-key-secret "$OSS_ACCESS_KEY_SECRET"
  )
elif [ -f "$OSS_CONFIG_FILE" ]; then
  OSS_AUTH_ARGS=(--config-file "$OSS_CONFIG_FILE")
else
  echo "OSS credentials are unavailable: provide OSS_CONFIG_FILE or OSS_ACCESS_KEY_ID/OSS_ACCESS_KEY_SECRET" >&2
  exit 1
fi

case "$OSS_BUCKET" in
  *[!a-z0-9-]*|'') echo "invalid OSS bucket: $OSS_BUCKET" >&2; exit 2 ;;
esac
case "$OSS_PREFIX" in
  /*|*'..'*|*'//'*) echo "invalid OSS prefix: $OSS_PREFIX" >&2; exit 2 ;;
esac
case "$OSS_PUBLIC_BASE_URL" in
  "https://${OSS_BUCKET}."*'.aliyuncs.com') ;;
  *) echo "OSS public base URL must be the canonical public Aliyun OSS bucket URL" >&2; exit 2 ;;
esac

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

oss_validate_access() {
  local destination="oss://${OSS_BUCKET}/${OSS_PREFIX}/"
  if [ "$OSS_CLIENT_MODE" = "native" ]; then
    "$OSS_CLIENT" ls "$destination" \
      "${OSS_AUTH_ARGS[@]}" \
      --endpoint "$OSS_ENDPOINT" \
      --limited-num 1 \
      >/dev/null
  else
    "$OSS_CLIENT" oss ls "$destination" \
      "${OSS_AUTH_ARGS[@]}" \
      --endpoint "$OSS_ENDPOINT" \
      --limited-num 1 \
      >/dev/null
  fi
}

oss_validate_access
if [ "${VALIDATE_ONLY:-false}" = "true" ]; then
  echo "validated Codex local app publisher identity, pinned CLI, and OSS read access"
  exit 0
fi

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

declare -A assets=(
  [macos]="baijimu-codex-local-app-${version}-macos-universal.zip"
  [windows]="baijimu-codex-local-app-${version}-windows-x64.zip"
  [linux]="baijimu-codex-local-app-${version}-linux-x64.zip"
)
declare -A checksums
declare -A sources

oss_upload() {
  local source_file="$1"
  local object_key="$2"
  local content_type="$3"
  local destination="oss://${OSS_BUCKET}/${object_key}"
  local metadata="Content-Type:${content_type}#Cache-Control:public,max-age=31536000,immutable"
  if [ "$OSS_CLIENT_MODE" = "native" ]; then
    "$OSS_CLIENT" cp "$source_file" "$destination" \
      "${OSS_AUTH_ARGS[@]}" \
      --endpoint "$OSS_ENDPOINT" \
      --force \
      --no-progress \
      --meta "$metadata"
  else
    "$OSS_CLIENT" oss cp "$source_file" "$destination" \
      "${OSS_AUTH_ARGS[@]}" \
      --endpoint "$OSS_ENDPOINT" \
      --force \
      --meta "$metadata"
  fi
}

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

for platform in macos windows linux; do
  asset="${assets[$platform]}"
  digest="$(printf '%s' "$release_json" | jq -er \
    --arg name "$asset" \
    '.assets[] | select(.name == $name and .state == "uploaded" and .size > 0) | .digest')"
  if ! [[ "$digest" =~ ^sha256:[0-9a-fA-F]{64}$ ]]; then
    echo "GitHub did not return a valid server-computed digest for ${asset}" >&2
    exit 1
  fi
  checksum="$(printf '%s' "${digest#sha256:}" | tr '[:upper:]' '[:lower:]')"
  checksums[$platform]="$checksum"

  download_release_asset "$asset" "$work_dir/$asset"
  download_release_asset "${asset}.sha256" "$work_dir/${asset}.sha256"
  actual_checksum="$(sha256sum "$work_dir/$asset" | awk '{print $1}')"
  if [ "$actual_checksum" != "$checksum" ]; then
    echo "downloaded ${asset} differs from the immutable GitHub release digest" >&2
    exit 1
  fi
  if ! grep -Fxq "${checksum}  ${asset}" "$work_dir/${asset}.sha256"; then
    echo "release checksum file does not match ${asset}" >&2
    exit 1
  fi

  object_prefix="${OSS_PREFIX}/releases/v${version}/${checksum}"
  object_key="${object_prefix}/${asset}"
  checksum_object_key="${object_prefix}/${asset}.sha256"
  source_url="${OSS_PUBLIC_BASE_URL%/}/${object_key}"
  checksum_url="${OSS_PUBLIC_BASE_URL%/}/${checksum_object_key}"

  oss_upload "$work_dir/$asset" "$object_key" 'application/zip'
  oss_upload "$work_dir/${asset}.sha256" "$checksum_object_key" 'text/plain; charset=utf-8'

  curl -fsSL \
    --retry 6 \
    --retry-all-errors \
    --retry-delay 3 \
    --connect-timeout 15 \
    --max-time 900 \
    "$source_url" \
    -o "$work_dir/oss-${asset}"
  oss_checksum="$(sha256sum "$work_dir/oss-${asset}" | awk '{print $1}')"
  if [ "$oss_checksum" != "$checksum" ]; then
    echo "anonymous OSS download checksum mismatch for ${asset}" >&2
    exit 1
  fi
  curl -fsSL \
    --retry 6 \
    --retry-all-errors \
    --retry-delay 3 \
    --connect-timeout 15 \
    --max-time 120 \
    "$checksum_url" \
    -o "$work_dir/oss-${asset}.sha256"
  if ! grep -Fxq "${checksum}  ${asset}" "$work_dir/oss-${asset}.sha256"; then
    echo "anonymous OSS checksum document mismatch for ${asset}" >&2
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
  '({
    schemaVersion: $connector.schemaVersion,
    applicationType: "connector",
    runtime: $connector.runtime.type,
    command: $connector.runtime.command,
    args: ($connector.runtime.args // []),
    management: ($connector.management != null),
    artifacts: [
      {platform: "macos", arch: "universal", source: $mac_source, checksum: $mac_sha},
      {platform: "windows", arch: "x86_64", source: $win_source, checksum: $win_sha},
      {platform: "linux", arch: "x86_64", source: $linux_source, checksum: $linux_sha}
    ]
  }
  + if $connector.setup == null then {} else {
      setup: true,
      setupTimeoutSecs: $connector.setup.timeoutSecs
    } end
  + if $connector.hostRequirements == null then {} else {
      hostRequirements: $connector.hostRequirements
    } end)')"
capabilities='["codex.project.read","codex.thread.read","codex.app.read","codex.turn.write","codex.raw.request","codex.turn.interrupt"]'

for document in "$manifest" "$capabilities"; do
  printf '%s' "$document" | jq -e . >/dev/null
done
printf '%s' "$manifest" | jq -e \
  --arg prefix "${OSS_PUBLIC_BASE_URL%/}/${OSS_PREFIX}/releases/v${version}/" \
  '.applicationType == "connector" and
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
echo "submitted Codex local app ${version}; publication=${publication_id:-existing}; status=${publication_status}"

verified=false
for attempt in $(seq 1 400); do
  all_targets_verified=true
  for target in 'macos&arch=aarch64' 'windows&arch=x86_64' 'linux&arch=x86_64'; do
    payload="$(curl -fsS --retry 2 --connect-timeout 5 --max-time 15 \
      "https://api.baijimu.com/lowcode3/api/local-app-market/apps/codex?platform=${target}&hostVersion=0.2.21&hostCapabilities=connector.setup.v1")"
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
  echo "market verification failed or publication was not approved before the 20 minute deadline" >&2
  exit 1
fi

echo "published Codex local app ${version} from anonymous Baijimu OSS artifacts"
