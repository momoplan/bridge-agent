set -euo pipefail

retry_gh() {
  local attempt delay
  for attempt in $(seq 1 20); do
    if "$@"; then
      return 0
    fi
    if [ "$attempt" -eq 20 ]; then
      echo "GitHub CLI command failed after ${attempt} attempts: $*" >&2
      return 1
    fi
    delay=$((attempt * 5))
    if [ "$delay" -gt 60 ]; then
      delay=60
    fi
    echo "GitHub CLI command failed; retrying in ${delay} seconds (${attempt}/20): $*" >&2
    sleep "$delay"
  done
}

release_notes="$(mktemp)"
cat > "$release_notes" <<'NOTES'
百积木桌面端发布。

Download the installer for your platform from the Assets section below:
- macOS: universal `.dmg` (works on both Intel and Apple Silicon)
- Windows: `.msi` installer that downloads WebView2 only when it is missing
- Linux: `.AppImage` / `.deb` / archive

This release is built automatically by GitHub Actions from the tagged source.
NOTES

release_create_args=(
  "$RELEASE_TAG"
  --verify-tag
  --title "百积木 $RELEASE_TAG"
  --notes-file "$release_notes"
)
case "$RELEASE_TAG" in
  *-alpha*|*-beta*|*-rc*) release_create_args+=(--prerelease) ;;
esac

if ! gh release view "$RELEASE_TAG" >/dev/null 2>&1; then
  retry_gh gh release create "${release_create_args[@]}" \
    || retry_gh gh release view "$RELEASE_TAG" >/dev/null
fi

shopt -s nullglob
case "$RELEASE_TARGET" in
  "macOS Universal")
    bundle_files=(
      src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
      src-tauri/target/universal-apple-darwin/release/bundle/macos/*.app.tar.gz
      src-tauri/target/universal-apple-darwin/release/bundle/macos/*.app.tar.gz.sig
    )
    ;;
  "Windows x64")
    bundle_files=(
      src-tauri/target/release/bundle/msi/*.msi
      src-tauri/target/release/bundle/msi/*.msi.sig
    )
    ;;
  "Linux x64")
    bundle_files=(
      src-tauri/target/release/bundle/appimage/*.AppImage
      src-tauri/target/release/bundle/appimage/*.AppImage.sig
      src-tauri/target/release/bundle/deb/*.deb
    )
    ;;
  *)
    echo "Unsupported release target: $RELEASE_TARGET" >&2
    exit 1
    ;;
esac

if [ "${#bundle_files[@]}" -eq 0 ]; then
  echo "No release bundles found for $RELEASE_TARGET" >&2
  exit 1
fi

for asset in "${bundle_files[@]}"; do
  retry_gh gh release upload "$RELEASE_TAG" "$asset" --clobber
done
