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

shopt -s nullglob
windows=(release-assets/*.msi)
macos_tar=(release-assets/*.app.tar.gz)
macos_dmg=(release-assets/*.dmg)
linux_deb=(release-assets/*.deb)
linux_appimage=(release-assets/*.AppImage)
if [ "${#windows[@]}" -ne 1 ] ||
   [ "${#macos_tar[@]}" -ne 1 ] ||
   [ "${#macos_dmg[@]}" -ne 1 ] ||
   [ "${#linux_deb[@]}" -ne 1 ] ||
   [ "${#linux_appimage[@]}" -ne 1 ]; then
  echo "Expected exactly one release bundle for each published format" >&2
  find release-assets -maxdepth 1 -type f -print | sort
  exit 1
fi

node .github/scripts/register-release-manifest.mjs \
  "$api" "$RELEASE_TAG" \
  "Windows x64::${windows[0]}::true" \
  "macOS Universal::${macos_tar[0]}::true" \
  "macOS Universal::${macos_dmg[0]}::false" \
  "Linux x64::${linux_deb[0]}::false" \
  "Linux x64::${linux_appimage[0]}::true"

upload_asset "Windows x64" "${windows[0]}"
upload_asset "macOS Universal" "${macos_tar[0]}"
upload_asset "macOS Universal" "${macos_dmg[0]}"
upload_asset "Linux x64" "${linux_deb[0]}"
upload_asset "Linux x64" "${linux_appimage[0]}"
