set -euo pipefail
intel_configured_min="10.13"
apple_silicon_configured_min="11.0"
dmg_dir="src-tauri/target/universal-apple-darwin/release/bundle/dmg"

shopt -s nullglob
dmg_files=("$dmg_dir"/*.dmg)
if [ "${#dmg_files[@]}" -ne 1 ]; then
  echo "Expected one macOS DMG, found ${#dmg_files[@]}:" >&2
  printf ' - %s\n' "${dmg_files[@]}" >&2
  exit 1
fi

mount_dir="$(mktemp -d)"
hdiutil attach "${dmg_files[0]}" -readonly -nobrowse -mountpoint "$mount_dir"
cleanup_mount() {
  hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
  rm -rf "$mount_dir"
}
trap cleanup_mount EXIT

app_path="$mount_dir/百积木.app"
if [ ! -d "$app_path" ]; then
  app_path="$(find "$mount_dir" -maxdepth 2 -name '*.app' -type d | head -n 1)"
fi
if [ -z "$app_path" ] || [ ! -d "$app_path" ]; then
  echo "Unable to locate app bundle inside ${dmg_files[0]}" >&2
  exit 1
fi
applications_link="$mount_dir/Applications"
if [ ! -L "$applications_link" ] || [ "$(readlink "$applications_link")" != "/Applications" ]; then
  echo "macOS DMG must contain an Applications symlink for drag-to-install" >&2
  find "$mount_dir" -maxdepth 1 -mindepth 1 -print >&2
  exit 1
fi
if [ ! -s "$mount_dir/.DS_Store" ]; then
  echo "macOS DMG is missing the Finder layout that applies its background and icon positions" >&2
  find "$mount_dir" -maxdepth 2 -mindepth 1 -print >&2
  exit 1
fi
if [ ! -s "$mount_dir/.background/dmg-background.png" ]; then
  echo "macOS DMG is missing the branded drag-to-install background" >&2
  find "$mount_dir" -maxdepth 2 -mindepth 1 -print >&2
  exit 1
fi
if ! cmp -s "$mount_dir/.background/dmg-background.png" "src-tauri/images/dmg-background.png"; then
  echo "macOS DMG drag-to-install background does not match the versioned source asset" >&2
  exit 1
fi
info_plist="${app_path}/Contents/Info.plist"

version_lte() {
  python3 - "$1" "$2" <<'PY'
import sys
def parts(value):
    return tuple(int(part) for part in value.split(".")[:3])
left = parts(sys.argv[1])
right = parts(sys.argv[2])
size = max(len(left), len(right))
left += (0,) * (size - len(left))
right += (0,) * (size - len(right))
sys.exit(0 if left <= right else 1)
PY
}

verify_macho_min() {
  local binary="$1"
  local label="$2"
  if [ ! -x "$binary" ]; then
    echo "${label} is missing or not executable: ${binary}" >&2
    exit 1
  fi
  for arch in $(lipo -archs "$binary"); do
    local configured_min="$intel_configured_min"
    if [ "$arch" = "arm64" ]; then
      configured_min="$apple_silicon_configured_min"
    fi
    local min_version
    min_version="$(
      otool -arch "$arch" -l "$binary" | awk '
        /LC_BUILD_VERSION/ { in_build = 1; in_min = 0; next }
        in_build && /minos/ { print $2; exit }
        /LC_VERSION_MIN_MACOSX/ { in_min = 1; in_build = 0; next }
        in_min && /version/ { print $2; exit }
      '
    )"
    if [ -z "$min_version" ]; then
      echo "Unable to read ${label} ${arch} minimum macOS version" >&2
      exit 1
    fi
    if ! version_lte "$min_version" "$configured_min"; then
      echo "${label} ${arch} requires macOS ${min_version}, expected <= ${configured_min}" >&2
      exit 1
    fi
  done
}

if plist_min="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$info_plist" 2>/dev/null)"; then
  if ! version_lte "$plist_min" "$apple_silicon_configured_min"; then
    echo "LSMinimumSystemVersion=${plist_min}, expected <= ${apple_silicon_configured_min}" >&2
    exit 1
  fi
else
  echo "LSMinimumSystemVersion not present; relying on Mach-O deployment target checks."
fi

verify_macho_min "${app_path}/Contents/MacOS/bridge-agent-desktop" "bridge-agent-desktop"
verify_macho_min "${app_path}/Contents/Resources/resources/bin/baijimu" "bundled baijimu CLI"
verify_macho_min "${app_path}/Contents/Resources/resources/bin/bridge-agent-unified-app-id-migration" "unified app ID migration"
