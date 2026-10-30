/// Release version reported to Relay in `Capabilities`.
const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Platform reported to Relay, the same value the release update check sends.
fn client_platform() -> &'static str {
    std::env::consts::OS
}

/// Architecture reported to Relay, the same value the release update check sends.
fn client_arch() -> &'static str {
    std::env::consts::ARCH
}

/// OS version reported to Relay. Read once; an unreadable version is reported
/// as unknown, as the release service then selects without OS compatibility.
fn client_os_version() -> Option<String> {
    static OS_VERSION: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    OS_VERSION
        .get_or_init(|| match crate::current_os_version() {
            Ok(version) => version,
            Err(error) => {
                tracing::warn!(%error, "cannot read the OS version for relay capabilities");
                None
            }
        })
        .clone()
}

/// Relay's terminal notice: this client is below the minimum supported version
/// of its platform. Reconnecting cannot succeed until the client is upgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RelayUpdateRequired {
    minimum_supported_version: String,
    message: String,
}

impl std::fmt::Display for RelayUpdateRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "relay requires client {} or later",
            self.minimum_supported_version
        )
    }
}

impl std::error::Error for RelayUpdateRequired {}

impl RelayUpdateRequired {
    fn status_detail(&self) -> String {
        format!(
            "{}（最低支持版本 {}，当前版本 {CLIENT_VERSION}）",
            self.message, self.minimum_supported_version
        )
    }
}

impl RuntimeRunner {
    /// Stops reconnecting: Relay will reject this client until it is upgraded.
    async fn enter_update_required(
        &self,
        notice: &RelayUpdateRequired,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) {
        if let Some(queue) = &self.event_queue {
            if let Err(error) = queue.subscriptions(None).await {
                tracing::error!(%error, "cannot invalidate device subscriptions");
            }
        }
        self.update_snapshot(
            RuntimeStatus::UpdateRequired,
            Some(notice.status_detail()),
            self.config.relay.agent_id.clone(),
            self.config.relay.url.clone(),
            self.config_path.clone(),
        )
        .await;
        self.push_log(
            "error",
            &format!("{notice}; reconnect stopped until the client is upgraded"),
        )
        .await;
        wait_until_shutdown(shutdown_rx).await;
    }
}
