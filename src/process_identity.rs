const FALLBACK_DESKTOP_BINARY_NAME: &str = "bridge-agent-desktop";
const FALLBACK_PRODUCT_NAME: &str = "Bridge Agent";
const FALLBACK_SERVICE_BINARY_NAME: &str = "bridge-agent-service";
const WINDOWS_SERVICE_NAME: &str = "BridgeAgent";

pub(crate) fn is_bridge_agent_process_name(image_name: &str) -> bool {
    let Some(image_name) = normalize_process_name(image_name) else {
        return false;
    };

    bridge_agent_process_name_candidates()
        .into_iter()
        .filter_map(normalize_process_name)
        .any(|candidate| candidate == image_name)
}

pub(crate) fn process_file_name(path: &str) -> &str {
    path.trim()
        .trim_matches('"')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

fn bridge_agent_process_name_candidates() -> [&'static str; 5] {
    [
        option_env!("BRIDGE_AGENT_CLI_BIN_NAME").unwrap_or(env!("CARGO_PKG_NAME")),
        option_env!("BRIDGE_AGENT_DESKTOP_BIN_NAME").unwrap_or(FALLBACK_DESKTOP_BINARY_NAME),
        option_env!("BRIDGE_AGENT_PRODUCT_NAME").unwrap_or(FALLBACK_PRODUCT_NAME),
        option_env!("BRIDGE_AGENT_SERVICE_BIN_NAME").unwrap_or(FALLBACK_SERVICE_BINARY_NAME),
        WINDOWS_SERVICE_NAME,
    ]
}

fn normalize_process_name(image_name: &str) -> Option<String> {
    let image_name = process_file_name(image_name).trim().trim_matches('"');
    if image_name.is_empty() {
        return None;
    }

    let lower = image_name.to_ascii_lowercase();
    let without_windows_extension = lower.strip_suffix(".exe").unwrap_or(&lower);
    let normalized = without_windows_extension.replace([' ', '_', '-'], "");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::{bridge_agent_process_name_candidates, is_bridge_agent_process_name};

    #[test]
    fn generated_candidates_include_all_packaged_hosts() {
        let candidates = bridge_agent_process_name_candidates();

        assert!(candidates.contains(&"bridge-agent"));
        assert!(candidates.contains(&"bridge-agent-desktop"));
        assert!(candidates.contains(&"Bridge Agent"));
        assert!(candidates.contains(&"bridge-agent-service"));
        assert!(candidates.contains(&"BridgeAgent"));
    }

    #[test]
    fn bridge_agent_process_name_allows_only_known_hosts() {
        assert!(is_bridge_agent_process_name("Bridge Agent"));
        assert!(is_bridge_agent_process_name("Bridge Agent.exe"));
        assert!(is_bridge_agent_process_name("bridge-agent"));
        assert!(is_bridge_agent_process_name("bridge-agent.exe"));
        assert!(is_bridge_agent_process_name("bridge-agent-desktop.exe"));
        assert!(is_bridge_agent_process_name("bridge-agent-service.exe"));
        assert!(is_bridge_agent_process_name(
            r#"C:\Program Files\Bridge Agent\bridge-agent-desktop.exe"#
        ));
        assert!(!is_bridge_agent_process_name("node.exe"));
        assert!(!is_bridge_agent_process_name("my-bridge-agent-helper.exe"));
    }
}
