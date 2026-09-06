set -euo pipefail
if [ -z "$BRIDGE_AGENT_UPDATE_API_URL" ]; then
  echo "Missing required secret: BRIDGE_AGENT_UPDATE_API_URL" >&2
  exit 1
fi
latest_api="$(node .github/scripts/release-service-url.mjs update "$BRIDGE_AGENT_UPDATE_API_URL")"
version="${RELEASE_TAG#bridge-agent-v}"

latest_payload="$(curl -fsSL --retry 6 --retry-delay 5 --retry-all-errors \
  --connect-timeout 20 \
  "${latest_api}?currentVersion=0.0.0")"
jq -e --arg version "$version" '
  .version == $version
  and (.assets | length == 5)
  and all(.assets[];
    .provider == "baijimu-oss"
    and (.downloadUrl
      | test("^https://download\\.baijimu\\.com/lowcode/direct-uploads/bridge-agent-release/"))
  )
' <<<"$latest_payload" >/dev/null

for target_arch in darwin:aarch64 windows:x86_64 linux:x86_64; do
  target="${target_arch%%:*}"
  arch="${target_arch#*:}"
  updater_payload="$(curl -fsSL --retry 6 --retry-delay 5 --retry-all-errors \
    --connect-timeout 20 \
    "${latest_api}/tauri?target=${target}&arch=${arch}&currentVersion=0.0.0")"
  updater_url="$(jq -er --arg version "$version" '
    select(.version == $version)
    | select(.signature | length > 0)
    | .url
    | select(test("^https://download\\.baijimu\\.com/lowcode/direct-uploads/bridge-agent-release/"))
  ' <<<"$updater_payload")"
  curl -fsSL --retry 6 --retry-delay 5 --retry-all-errors \
    --connect-timeout 20 --range 0-0 --max-time 120 \
    --output /dev/null "$updater_url"
done
