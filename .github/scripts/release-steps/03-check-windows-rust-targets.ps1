$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
# Shell lifecycle tests exercise real child-process and pipe handling.
# Keep the Windows quality gate deterministic instead of competing with
# the connector installation tests for process and I/O resources.
cargo test --locked --workspace -- --test-threads=1
cargo check --locked --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml codex_skill::tests::installs_bundled_skill_in_user_skill_directory -- --exact
cargo test --locked --manifest-path src-tauri/Cargo.toml managed_tool::tests::windows_registry_path_registration_round_trips -- --exact
cargo fmt --manifest-path tools/windows-uninstaller/Cargo.toml -- --check
cargo test --locked --manifest-path tools/windows-uninstaller/Cargo.toml
cargo fmt --manifest-path migration-artifacts/unified-app-id/Cargo.toml -- --check
cargo test --locked --manifest-path migration-artifacts/unified-app-id/Cargo.toml
cargo clippy --locked --manifest-path migration-artifacts/unified-app-id/Cargo.toml --all-targets -- -D warnings
