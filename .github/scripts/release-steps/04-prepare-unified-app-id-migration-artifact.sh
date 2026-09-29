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

# The source-identity converter is an internal binary of the same Bridge release.
identity_binary="bridge-agent-environment-identity-migration"
case "$RUNNER_OS" in
  macOS)
    for target in x86_64-apple-darwin aarch64-apple-darwin; do
      CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --bin "$identity_binary" --target "$target"
    done
    lipo -create "$target_dir/x86_64-apple-darwin/release/$identity_binary" "$target_dir/aarch64-apple-darwin/release/$identity_binary" -output "$output_dir/$identity_binary"
    chmod 755 "$output_dir/$identity_binary"
    ;;
  Windows)
    CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --bin "$identity_binary"
    cp "$target_dir/release/$identity_binary.exe" "$output_dir/$identity_binary.exe"
    ;;
  Linux)
    CARGO_TARGET_DIR="$target_dir" cargo build --locked --release --bin "$identity_binary"
    install -m 755 "$target_dir/release/$identity_binary" "$output_dir/$identity_binary"
    ;;
esac
