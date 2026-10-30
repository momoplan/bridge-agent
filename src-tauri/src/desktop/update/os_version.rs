fn current_update_os_version() -> Result<Option<String>, String> {
    bridge_agent::current_os_version()
}
