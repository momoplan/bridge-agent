set -euo pipefail

for name in BRIDGE_AGENT_RELEASE_API_URL BRIDGE_AGENT_RELEASE_API_TOKEN; do
  if [ -z "${!name}" ]; then
    echo "Missing required secret: $name" >&2
    exit 1
  fi
done

api="$(node .github/scripts/release-service-url.mjs release "$BRIDGE_AGENT_RELEASE_API_URL")"
version="${RELEASE_TAG#bridge-agent-v}"

upload_asset() {
  local target="$1"
  local asset="$2"
  local signature="${asset}.sig"
  if [ ! -s "$asset" ]; then
    echo "Missing release bundle: $asset" >&2
    exit 1
  fi
  if [ -s "$signature" ]; then
    node .github/scripts/upload-oss-release-asset.mjs \
      "$api" "$RELEASE_TAG" "$version" "$target" "$asset" "$signature"
  else
    node .github/scripts/upload-oss-release-asset.mjs \
      "$api" "$RELEASE_TAG" "$version" "$target" "$asset"
  fi
}

plan="$(node .github/scripts/release-platforms.mjs release-assets)"
entries=()
while IFS=$'\t' read -r target name signature_required; do
  entries+=("${target}::release-assets/${name}::${signature_required}")
done < <(jq -r '.assets[] | [.target, .name, (.signatureRequired | tostring)] | @tsv' <<<"$plan")

node .github/scripts/register-release-manifest.mjs "$api" "$RELEASE_TAG" "${entries[@]}"

while IFS=$'\t' read -r target name; do
  upload_asset "$target" "release-assets/$name"
done < <(jq -r '.assets[] | [.target, .name] | @tsv' <<<"$plan")
