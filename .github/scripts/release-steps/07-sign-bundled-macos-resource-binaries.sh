set -euo pipefail
for name in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do
  if [ -z "${!name}" ]; then
    echo "Missing required secret: $name" >&2
    exit 1
  fi
done

resource_binaries=(
  "src-tauri/resources/bin/baijimu"
  "src-tauri/resources/bin/bridge-agent-unified-app-id-migration"
)
for binary in "${resource_binaries[@]}"; do
  if [ ! -x "$binary" ]; then
    echo "Bundled resource is missing or not executable: $binary" >&2
    exit 1
  fi
done

certificate_path="$RUNNER_TEMP/baijimu-cli-signing-certificate.p12"
keychain_path="$RUNNER_TEMP/baijimu-cli-signing.keychain-db"
keychain_password="$(uuidgen)"

printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$certificate_path"
security create-keychain -p "$keychain_password" "$keychain_path"
security set-keychain-settings -lut 21600 "$keychain_path"
security unlock-keychain -p "$keychain_password" "$keychain_path"
security import "$certificate_path" \
  -k "$keychain_path" \
  -P "$APPLE_CERTIFICATE_PASSWORD" \
  -T /usr/bin/codesign
security list-keychains -d user -s "$keychain_path" $(security list-keychains -d user | tr -d '"')
security set-key-partition-list \
  -S apple-tool:,apple:,codesign: \
  -s \
  -k "$keychain_password" \
  "$keychain_path"

for binary in "${resource_binaries[@]}"; do
  bash .github/scripts/sign-macos-with-retry.sh \
    "$binary" \
    "$APPLE_SIGNING_IDENTITY"
  lipo -info "$binary"
done
