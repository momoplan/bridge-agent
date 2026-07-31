fn main() {
    println!("cargo:rerun-if-env-changed=BRIDGE_AGENT_UPDATE_API_URL");
    tauri_build::build()
}
