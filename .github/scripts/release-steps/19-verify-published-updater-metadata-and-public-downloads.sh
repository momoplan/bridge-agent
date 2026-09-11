set -euo pipefail
if [ -z "$BRIDGE_AGENT_UPDATE_API_URL" ]; then
  echo "Missing required secret: BRIDGE_AGENT_UPDATE_API_URL" >&2
  exit 1
fi
latest_api="$(node .github/scripts/release-service-url.mjs update "$BRIDGE_AGENT_UPDATE_API_URL")"
version="${RELEASE_TAG#bridge-agent-v}"

plan="$(node .github/scripts/release-platforms.mjs)"

latest_payload="$(curl -fsSL --retry 6 --retry-delay 5 --retry-all-errors \
  --connect-timeout 20 \
  "${latest_api}?currentVersion=0.0.0")"
jq -e --arg version "$version" --argjson plan "$plan" '
  .version == $version
  and (([.assets[] | {target, name}] | sort_by(.target, .name))
    == ([$plan.assets[] | {target, name}] | sort_by(.target, .name)))
  and all(.assets[];
    .provider == "baijimu-oss"
    and (.downloadUrl
      | test("^https://download\\.baijimu\\.com/lowcode/direct-uploads/bridge-agent-release/"))
  )
' <<<"$latest_payload" >/dev/null

while IFS=$'\t' read -r target arch name; do
  updater_payload="$(curl -fsSL --retry 6 --retry-delay 5 --retry-all-errors \
    --connect-timeout 20 \
    "${latest_api}/tauri?target=${target}&arch=${arch}&currentVersion=0.0.0")"
  updater_url="$(jq -er --arg version "$version" '
    select(.version == $version)
    | select(.signature | length > 0)
    | .url
    | select(test("^https://download\\.baijimu\\.com/lowcode/direct-uploads/bridge-agent-release/"))
  ' <<<"$updater_payload")"
  expected_url="$(jq -er --arg name "$name" '.assets[] | select(.name == $name) | .downloadUrl' <<<"$latest_payload")"
  test "$updater_url" = "$expected_url"
  curl -fsSL --retry 6 --retry-delay 5 --retry-all-errors \
    --connect-timeout 20 --range 0-0 --max-time 120 \
    --output /dev/null "$updater_url"
done < <(jq -r '.updaters[] | [.target, .arch, .name] | @tsv' <<<"$plan")
