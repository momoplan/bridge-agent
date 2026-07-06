fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src-tauri/Cargo.toml");
    println!("cargo:rerun-if-changed=src-tauri/tauri.conf.json");

    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
    );

    let root_manifest = read_to_string(manifest_dir.join("Cargo.toml"));
    let cli_binary_name = package_name_from_toml(&root_manifest)
        .or_else(|| std::env::var("CARGO_PKG_NAME").ok())
        .unwrap_or_else(|| "bridge-agent".to_string());
    set_rustc_env("BRIDGE_AGENT_CLI_BIN_NAME", &cli_binary_name);

    let desktop_manifest = read_to_string(manifest_dir.join("src-tauri").join("Cargo.toml"));
    let desktop_binary_name = package_name_from_toml(&desktop_manifest)
        .unwrap_or_else(|| "bridge-agent-desktop".to_string());
    set_rustc_env("BRIDGE_AGENT_DESKTOP_BIN_NAME", &desktop_binary_name);

    let tauri_config = read_to_string(manifest_dir.join("src-tauri").join("tauri.conf.json"));
    let product_name =
        tauri_product_name_from_json(&tauri_config).unwrap_or_else(|| "百积木".to_string());
    set_rustc_env("BRIDGE_AGENT_PRODUCT_NAME", &product_name);

    set_rustc_env("BRIDGE_AGENT_SERVICE_BIN_NAME", "bridge-agent-service");
}

fn read_to_string(path: impl AsRef<std::path::Path>) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn package_name_from_toml(contents: &str) -> Option<String> {
    toml::from_str::<toml::Value>(contents)
        .ok()?
        .get("package")?
        .get("name")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn tauri_product_name_from_json(contents: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(contents)
        .ok()?
        .get("productName")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn set_rustc_env(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}
