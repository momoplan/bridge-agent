set -euo pipefail
for name in BRIDGE_AGENT_UPDATE_API_URL BRIDGE_AGENT_RELEASE_API_URL BRIDGE_AGENT_RELEASE_API_TOKEN; do
  if [ -z "${!name}" ]; then
    echo "Missing required secret: $name" >&2
    exit 1
  fi
done
node .github/scripts/release-service-url.mjs update "$BRIDGE_AGENT_UPDATE_API_URL" >/dev/null
node .github/scripts/release-service-url.mjs release "$BRIDGE_AGENT_RELEASE_API_URL" >/dev/null
