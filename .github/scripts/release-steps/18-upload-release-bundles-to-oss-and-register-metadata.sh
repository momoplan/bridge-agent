set -euo pipefail

for name in BRIDGE_AGENT_RELEASE_API_URL BRIDGE_AGENT_RELEASE_API_TOKEN; do
  if [ -z "${!name}" ]; then
    echo "Missing required secret: $name" >&2
    exit 1
  fi
done

api="$(node .github/scripts/release-service-url.mjs release "$BRIDGE_AGENT_RELEASE_API_URL")"
node .github/scripts/publish-platform-assets.mjs "$api"
