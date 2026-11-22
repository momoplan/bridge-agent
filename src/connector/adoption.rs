/// The marker ties a pending data-directory rename to the exact previously
/// installed source. A retry can distinguish its own move from a conflict.
struct ConnectorAdoption {
    legacy: local_app_contract::SourceVersion,
    target: String,
    old_record: PathBuf,
    marker: PathBuf,
    old_data: PathBuf,
    new_data: PathBuf,
}

impl ConnectorAdoption {
    fn start(
        legacy: &local_app_contract::SourceVersion,
        target: &str,
        config: &AgentConfig,
    ) -> Result<Self> {
        let old_id = legacy.application.app_id.as_str();
        let record = load_install_record(old_id)?;
        if crate::market_distribution::installed_source_version(record.install_source.as_ref())?
            != legacy
            || record.bundle_source.is_some()
        {
            bail!("the installed Connector does not match the recorded adoption");
        }
        record.manifest.verify_source_identity(legacy)?;
        let old_record = install_record_path(old_id)?;
        let marker = old_record.with_file_name("locator-adoption.json");
        let old_data = connector_data_dir(old_id)?;
        let new_data = connector_data_dir(target)?;
        let expected = serde_json::to_string(&(legacy, target))?;
        match fs::read_to_string(&marker) {
            Ok(prior) if prior == expected => {}
            Ok(_) => bail!("an adoption of this installation is already in progress"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if new_data.try_exists()? {
                    bail!("the target Connector already has local data");
                }
                if !config.local_apps.iter().any(|app| app.app_id == old_id) {
                    bail!(
                        "the legacy Connector is not present in the current device configuration"
                    );
                }
                let app = config.local_apps.iter().find(|app| app.app_id == old_id)
                    .context("the legacy Connector is not present in the current device configuration")?;
                if app.stop_command.is_some() {
                    stop_connector_for_package_change(old_id, config)?;
                }
                release_connector_package_processes(Path::new(&record.package_path))?;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)?;
                use std::io::Write;
                file.write_all(expected.as_bytes())?;
                file.sync_all()?;
            }
            Err(error) => return Err(error.into()),
        }
        if old_data.try_exists()? {
            if new_data.try_exists()? {
                bail!("both Connector data directories exist");
            }
            fs::create_dir_all(new_data.parent().context("missing Connector data parent")?)?;
            rename_connector_path_after_handles_close(&old_data, &new_data)?;
        }
        Ok(Self {
            legacy: legacy.clone(),
            target: target.into(),
            old_record,
            marker,
            old_data,
            new_data,
        })
    }

    fn finish(self, config_path: &Path, original: &AgentConfig) -> Result<()> {
        crate::event_delivery::adopt_installation(
            config_path,
            original,
            &self.legacy,
            &self.target,
        )?;
        fs::rename(
            &self.old_record,
            self.old_record
                .with_file_name("connector.install.adopted.json"),
        )?;
        fs::remove_file(self.marker)?;
        Ok(())
    }

    fn rollback(self) -> Result<()> {
        if self.new_data.try_exists()? {
            rename_connector_path_after_handles_close(&self.new_data, &self.old_data)?;
        }
        fs::remove_file(self.marker)?;
        Ok(())
    }
}
