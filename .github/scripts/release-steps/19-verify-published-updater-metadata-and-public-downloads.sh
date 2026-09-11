set -euo pipefail
if [ -z "$BRIDGE_AGENT_UPDATE_API_URL" ]; then
  echo "Missing required secret: BRIDGE_AGENT_UPDATE_API_URL" >&2
  exit 1
fi
latest_api="$(node .github/scripts/release-service-url.mjs update "$BRIDGE_AGENT_UPDATE_API_URL")"
node .github/scripts/verify-platform-release.mjs verify "$latest_api"
