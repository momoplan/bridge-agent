set -euo pipefail
manifest="migration-artifacts/unified-app-id/Cargo.toml"
target_dir="$RUNNER_TEMP/unified-app-id-migration"
output_dir="src-tauri/resources/bin"
mkdir -p "$output_dir"
case "$RUNNER_OS" in
  macOS)
    CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --manifest-path "$manifest" --target x86_64-apple-darwin
    CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --manifest-path "$manifest" --target aarch64-apple-darwin
    lipo -create \
      "$target_dir/x86_64-apple-darwin/release/bridge-agent-unified-app-id-migration" \
      "$target_dir/aarch64-apple-darwin/release/bridge-agent-unified-app-id-migration" \
      -output "$output_dir/bridge-agent-unified-app-id-migration"
    chmod 755 "$output_dir/bridge-agent-unified-app-id-migration"
    ;;
  Windows)
    CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --manifest-path "$manifest"
    cp "$target_dir/release/bridge-agent-unified-app-id-migration.exe" \
      "$output_dir/bridge-agent-unified-app-id-migration.exe"
    ;;
  Linux)
    CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --manifest-path "$manifest"
    install -m 755 \
      "$target_dir/release/bridge-agent-unified-app-id-migration" \
      "$output_dir/bridge-agent-unified-app-id-migration"
    ;;
  *)
    echo "Unsupported runner OS: $RUNNER_OS" >&2
    exit 1
    ;;
esac
