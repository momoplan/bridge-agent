#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <connector-version> <connector-manifest>" >&2
  exit 2
fi

version="$1"
connector_manifest_path="$2"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid Connector version: $version" >&2
  exit 2
fi
if [ ! -f "$connector_manifest_path" ]; then
  echo "Connector manifest does not exist: $connector_manifest_path" >&2
  exit 2
fi

connector_manifest="$(jq -ce \
  --arg version "$version" \
  '
    select(.schemaVersion == "2.0")
    | select(.id == "com.baijimu.connector.wecom")
    | select(.version == $version)
    | select(.source.type == "git")
    | select(.source.repo == "zxflimit_admin/wecom-bridge-collector")
    | select(.source.revision == ("v" + $version))
    | select(.runtime.type == "process")
    | select((.runtime.command | type) == "string" and (.runtime.command | length) > 0)
    | select((.methods | type) == "array" and (.methods | length) > 0)
    | select((.events | type) == "array" and (.events | length) > 0)
  ' "$connector_manifest_path")" || {
  echo "Connector manifest identity, version, source, or runtime contract is invalid" >&2
  exit 2
}

release_tag="wecom-local-app-v${version}"
release_base="https://github.com/momoplan/bridge-agent/releases/download/${release_tag}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"

declare -A assets=(
  [macos]="baijimu-wecom-local-app-${version}-macos-universal.zip"
  [windows]="baijimu-wecom-local-app-${version}-windows-universal.zip"
  [linux]="baijimu-wecom-local-app-${version}-linux-universal.zip"
)
declare -A checksums

release_json="$(curl -fsS \
  --retry 3 \
  --retry-delay 2 \
  --connect-timeout 10 \
  --max-time 30 \
  -H "Authorization: Bearer ${GITHUB_TOKEN}" \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2022-11-28' \
  "https://api.github.com/repos/momoplan/bridge-agent/releases/tags/${release_tag}")"
printf '%s' "$release_json" | jq -e \
  --arg tag "$release_tag" \
  '.tag_name == $tag and .draft == false and .prerelease == false' \
  >/dev/null

for platform in macos windows linux; do
  asset="${assets[$platform]}"
  digest="$(printf '%s' "$release_json" | jq -er \
    --arg name "$asset" \
    '.assets[] | select(.name == $name and .state == "uploaded" and .size > 0) | .digest')"
  if ! [[ "$digest" =~ ^sha256:[0-9a-fA-F]{64}$ ]]; then
    echo "GitHub did not return a valid server-computed digest for ${asset}" >&2
    exit 1
  fi
  printf '%s' "$release_json" | jq -e \
    --arg name "${asset}.sha256" \
    'any(.assets[]; .name == $name and .state == "uploaded" and .size > 0)' \
    >/dev/null
  checksums[$platform]="$(printf '%s' "${digest#sha256:}" | tr '[:upper:]' '[:lower:]')"
done

manifest="$(printf '%s' "$connector_manifest" | jq -c \
  --arg base "$release_base" \
  --arg mac_asset "${assets[macos]}" \
  --arg win_asset "${assets[windows]}" \
  --arg linux_asset "${assets[linux]}" \
  --arg mac_sha "sha256:${checksums[macos]}" \
  --arg win_sha "sha256:${checksums[windows]}" \
  --arg linux_sha "sha256:${checksums[linux]}" \
  '. + {
    applicationType: "connector",
    artifacts: [
      {platform: "macos", arch: "universal", source: ($base + "/" + $mac_asset), checksum: $mac_sha},
      {platform: "windows", arch: "universal", source: ($base + "/" + $win_asset), checksum: $win_sha},
      {platform: "linux", arch: "universal", source: ($base + "/" + $linux_asset), checksum: $linux_sha}
    ]
  }')"
capabilities="$(printf '%s' "$connector_manifest" | jq -c \
  '[.methods[]?.name, .events[]?.name] | map(select(type == "string" and length > 0)) | unique')"

for document in "$manifest" "$capabilities"; do
  printf '%s' "$document" | jq -e . >/dev/null
done

nacos_content="$(timeout 30s aliyun mse GetNacosConfig \
  --profile baijimu \
  --RegionId cn-beijing \
  --InstanceId mse_regserverless_cn-cy74qcvrg01 \
  --NamespaceId 6ef6a8f2-8682-422b-9627-6fadf27f2b3e \
  --DataId lowcode \
  --Group DEFAULT_GROUP 2>/dev/null \
  | jq -r '.Configuration.Content // .Content // empty')"
db_password="$(printf '%s\n' "$nacos_content" | sed -n 's/^spring.datasource.password=//p' | head -1)"
if [ -z "$db_password" ]; then
  echo "failed to resolve production database password from MSE" >&2
  exit 1
fi

mysql_args=(
  --protocol=TCP
  --host=rm-2zen9i892pqpan6at.mysql.rds.aliyuncs.com
  --user=baijimu
  --database=local_app_market
  --connect-timeout=10
  --default-character-set=utf8mb4
  --batch
  --raw
)
backup_file="${WORKSPACE:-$PWD}/wecom-market-before-${BUILD_NUMBER:-manual}.tsv"
MYSQL_PWD="$db_password" mysql "${mysql_args[@]}" \
  -e "SELECT app.*, version.* FROM local_app app LEFT JOIN local_app_version version ON version.app_id=app.id WHERE app.id='wecom' ORDER BY version.id" \
  > "$backup_file"

identity="$(MYSQL_PWD="$db_password" mysql "${mysql_args[@]}" --skip-column-names \
  -e "SELECT CONCAT(id, ':', connector_id, ':', status) FROM local_app WHERE id='wecom'")"
if [ "$identity" != "wecom:com.baijimu.connector.wecom:PUBLISHED" ]; then
  echo "market app identity is not the registered published WeCom app: $identity" >&2
  exit 1
fi

b64() {
  printf '%s' "$1" | base64 | tr -d '\n'
}

source_url="${release_base}/${assets[macos]}"
repo="zxflimit_admin/wecom-bridge-collector"
revision="v${version}"
checksum="${checksums[macos]}"
source_b64="$(b64 "$source_url")"
repo_b64="$(b64 "$repo")"
revision_b64="$(b64 "$revision")"
capabilities_b64="$(b64 "$capabilities")"
manifest_b64="$(b64 "$manifest")"
published_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

expected_fingerprint="$(printf '%s\n' \
  PUBLISHED archive "$source_url" "$repo" "$revision" "$checksum" "$capabilities" "$manifest" \
  | sha256sum | awk '{print $1}')"
existing_fingerprint="$(MYSQL_PWD="$db_password" mysql "${mysql_args[@]}" --skip-column-names -e "
SELECT SHA2(CONCAT_WS(CHAR(10), status, source_type, source, repo, revision, checksum,
  capabilities_json, manifest_json, ''), 256)
FROM local_app_version WHERE app_id='wecom' AND version='${version}'
" | tr '[:upper:]' '[:lower:]')"

if [ -n "$existing_fingerprint" ]; then
  if [ "$existing_fingerprint" != "$expected_fingerprint" ]; then
    echo "该版本已经存在且内容与当前流水线不一致；不可覆盖不可变市场版本" >&2
    exit 1
  fi
  echo "market version wecom ${version} already matches this immutable release"
else
  MYSQL_PWD="$db_password" mysql "${mysql_args[@]}" <<SQL
START TRANSACTION;
INSERT INTO local_app_version (
  app_id, version, status, source_type, source, repo, revision, checksum,
  capabilities_json, manifest_json, rank_order, published_at
) VALUES (
  'wecom', '${version}', 'PUBLISHED', 'archive',
  CONVERT(FROM_BASE64('${source_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${repo_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${revision_b64}') USING utf8mb4),
  '${checksum}',
  CONVERT(FROM_BASE64('${capabilities_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${manifest_b64}') USING utf8mb4),
  (
    SELECT next_rank FROM (
      SELECT COALESCE(MAX(rank_order), 0) + 1 AS next_rank
      FROM local_app_version WHERE app_id='wecom'
    ) AS ranks
  ), '${published_at}'
);
COMMIT;
SQL
fi

for platform in macos windows linux; do
  case "$platform" in
    macos) target='macos&arch=aarch64' ;;
    windows) target='windows&arch=x86_64' ;;
    linux) target='linux&arch=x86_64' ;;
  esac
  verified=false
  for attempt in $(seq 1 20); do
    payload="$(curl -fsS --retry 2 --connect-timeout 5 --max-time 15 \
      "https://api.baijimu.com/lowcode3/api/local-app-market/apps/wecom?platform=${target}&hostVersion=0.2.39")"
    if printf '%s' "$payload" | jq -e --arg version "$version" \
      --argjson expected_manifest "$manifest" \
      '((.data // .).connectorId == "com.baijimu.connector.wecom")
       and ((.data // .).latestVersion.version == $version)
       and ((.data // .).latestVersion.compatibility.compatible == true)
       and ((.data // .).latestVersion.manifest == $expected_manifest)' \
      >/dev/null; then
      verified=true
      break
    fi
    sleep 3
  done
  if [ "$verified" != true ]; then
    echo "market verification failed for ${target}" >&2
    exit 1
  fi

  artifact_source="$(printf '%s' "$manifest" | jq -er --arg platform "$platform" \
    '.artifacts[] | select(.platform == $platform) | .source')"
  artifact_checksum="$(printf '%s' "$manifest" | jq -er --arg platform "$platform" \
    '.artifacts[] | select(.platform == $platform) | .checksum | sub("^sha256:"; "")')"
  actual_checksum="$(curl -fsSL --retry 3 --connect-timeout 10 --max-time 120 "$artifact_source" | sha256sum | awk '{print $1}')"
  test "$actual_checksum" = "$artifact_checksum"
done

echo "published WeCom local app ${version}; backup=${backup_file}"
