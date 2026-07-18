#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <connector-version>" >&2
  exit 2
fi

version="$1"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid connector version: $version" >&2
  exit 2
fi

release_tag="codex-local-app-v${version}"
release_base="https://github.com/momoplan/bridge-agent/releases/download/${release_tag}"
: "${GITHUB_TOKEN:?GITHUB_TOKEN is required}"

declare -A assets=(
  [macos]="baijimu-codex-local-app-${version}-macos-universal.zip"
  [windows]="baijimu-codex-local-app-${version}-windows-x64.zip"
  [linux]="baijimu-codex-local-app-${version}-linux-x64.zip"
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

manifest="$(jq -nc \
  --arg version "$version" \
  --arg base "$release_base" \
  --arg mac_asset "${assets[macos]}" \
  --arg win_asset "${assets[windows]}" \
  --arg linux_asset "${assets[linux]}" \
  --arg mac_sha "sha256:${checksums[macos]}" \
  --arg win_sha "sha256:${checksums[windows]}" \
  --arg linux_sha "sha256:${checksums[linux]}" \
  '{
    applicationType: "connector",
    runtime: "process",
    command: "baijimu-connector-codex",
    args: ["start", "--daemon"],
    management: true,
    artifacts: [
      {platform: "macos", arch: "universal", source: ($base + "/" + $mac_asset), checksum: $mac_sha},
      {platform: "windows", arch: "x86_64", source: ($base + "/" + $win_asset), checksum: $win_sha},
      {platform: "linux", arch: "x86_64", source: ($base + "/" + $linux_asset), checksum: $linux_sha}
    ]
  }')"
capabilities='["codex.project.read","codex.thread.read","codex.app.read","codex.turn.write","codex.raw.request","codex.turn.interrupt"]'

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
backup_file="${WORKSPACE:-$PWD}/codex-market-before-${BUILD_NUMBER:-manual}.tsv"
MYSQL_PWD="$db_password" mysql "${mysql_args[@]}" \
  -e "SELECT app.*, version.* FROM local_app app LEFT JOIN local_app_version version ON version.app_id=app.id WHERE app.id='codex' ORDER BY version.id" \
  > "$backup_file"

b64() {
  printf '%s' "$1" | base64 | tr -d '\n'
}

name_b64="$(b64 'Codex')"
description_b64="$(b64 '统一管理本机 Codex 会话、工作区、项目和百积木 LLM credential。')"
risk_b64="$(b64 '需要访问本机 Codex CLI、Codex 私有配置和用户授权的工作区。')"
capability_b64="$(b64 '读取和管理本机 Codex 会话，并在本机安全切换限定到工作区和项目的 LLM credential。')"
platforms_b64="$(b64 '["macos","windows","linux"]')"
source_b64="$(b64 "${release_base}/${assets[macos]}")"
repo_b64="$(b64 'zxflimit_admin/baijimu-connector-codex')"
revision_b64="$(b64 "v${version}")"
capabilities_b64="$(b64 "$capabilities")"
manifest_b64="$(b64 "$manifest")"
published_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

MYSQL_PWD="$db_password" mysql "${mysql_args[@]}" <<SQL
START TRANSACTION;
INSERT INTO local_app (
  id, connector_id, name, status, publisher, description, risk, risk_level,
  capability, platforms_json, rank_order
) VALUES (
  'codex', 'com.baijimu.connector.codex',
  CONVERT(FROM_BASE64('${name_b64}') USING utf8mb4), 'PUBLISHED', 'Baijimu',
  CONVERT(FROM_BASE64('${description_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${risk_b64}') USING utf8mb4), 'medium',
  CONVERT(FROM_BASE64('${capability_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${platforms_b64}') USING utf8mb4), 30
) ON DUPLICATE KEY UPDATE
  connector_id=VALUES(connector_id), name=VALUES(name), status='PUBLISHED',
  publisher=VALUES(publisher), description=VALUES(description), risk=VALUES(risk),
  risk_level=VALUES(risk_level), capability=VALUES(capability), platforms_json=VALUES(platforms_json);

INSERT INTO local_app_version (
  app_id, version, status, source_type, source, repo, revision, checksum,
  capabilities_json, manifest_json, rank_order, published_at
) VALUES (
  'codex', '${version}', 'PUBLISHED', 'archive',
  CONVERT(FROM_BASE64('${source_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${repo_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${revision_b64}') USING utf8mb4),
  '${checksums[macos]}',
  CONVERT(FROM_BASE64('${capabilities_b64}') USING utf8mb4),
  CONVERT(FROM_BASE64('${manifest_b64}') USING utf8mb4),
  400, '${published_at}'
) ON DUPLICATE KEY UPDATE
  status='PUBLISHED', source_type=VALUES(source_type), source=VALUES(source),
  repo=VALUES(repo), revision=VALUES(revision), checksum=VALUES(checksum),
  capabilities_json=VALUES(capabilities_json), manifest_json=VALUES(manifest_json),
  rank_order=VALUES(rank_order), published_at=VALUES(published_at);
COMMIT;
SQL

for target in 'macos&arch=aarch64' 'windows&arch=x86_64' 'linux&arch=x86_64'; do
  verified=false
  for attempt in $(seq 1 20); do
    payload="$(curl -fsS --retry 2 --connect-timeout 5 --max-time 15 \
      "https://www.baijimu.com/lowcode3/api/local-app-market/apps?platform=${target}")"
    if printf '%s' "$payload" | jq -e --arg version "$version" \
      '(.data // .) | any(.connectorId == "com.baijimu.connector.codex" and .latestVersion.version == $version)' \
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
done

echo "published Codex local app ${version}; backup=${backup_file}"
