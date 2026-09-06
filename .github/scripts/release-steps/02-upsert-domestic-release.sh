set -euo pipefail
for name in BRIDGE_AGENT_RELEASE_API_URL BRIDGE_AGENT_RELEASE_API_TOKEN; do
  if [ -z "${!name}" ]; then
    echo "Missing required secret: $name" >&2
    exit 1
  fi
done

api="$(node .github/scripts/release-service-url.mjs release "$BRIDGE_AGENT_RELEASE_API_URL")"
version="${RELEASE_TAG#bridge-agent-v}"

curl -fsSL --post301 --post302 --post303 --retry 3 --retry-delay 2 \
  -X POST \
  -H "Authorization: Bearer $BRIDGE_AGENT_RELEASE_API_TOKEN" \
  -H "Content-Type: application/json" \
  --data "{\"tagName\":\"$RELEASE_TAG\",\"version\":\"$version\",\"releaseName\":\"百积木 $RELEASE_TAG\",\"source\":\"github-actions\"}" \
  "$api/releases/$RELEASE_TAG"
