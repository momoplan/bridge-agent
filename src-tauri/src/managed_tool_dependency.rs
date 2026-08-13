use crate::managed_tool;
use anyhow::{Context, Result};
use bridge_agent::{ConnectorManagedToolDependencyPhase, ConnectorManifest};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub async fn ensure_ready(
    manifest: &ConnectorManifest,
    phase: ConnectorManagedToolDependencyPhase,
    bundled_baijimu_cli: Option<&Path>,
) -> Result<BTreeMap<String, String>> {
    let manifest = manifest.clone();
    let bundled_baijimu_cli = bundled_baijimu_cli.map(Path::to_path_buf);
    tauri::async_runtime::spawn_blocking(move || {
        ensure_ready_blocking(&manifest, phase, bundled_baijimu_cli.as_deref())
    })
    .await
    .context("managed tool dependency task failed")?
}

fn ensure_ready_blocking(
    manifest: &ConnectorManifest,
    phase: ConnectorManagedToolDependencyPhase,
    bundled_baijimu_cli: Option<&Path>,
) -> Result<BTreeMap<String, String>> {
    let mut environment = BTreeMap::new();
    for dependency in manifest
        .managed_tool_dependencies
        .iter()
        .filter(|dependency| dependency.required_for.contains(&phase))
    {
        let status = managed_tool::ensure_bundled_dependency_ready(
            &dependency.id,
            &dependency.minimum_version,
            bundled_baijimu_cli,
        )
        .with_context(|| {
            format!(
                "本地应用 `{}` 所需工具 `{}` 尚未就绪",
                manifest.name, dependency.id
            )
        })?;
        let launcher = absolute_launcher_path(&status.launcher_path).with_context(|| {
            format!(
                "本地应用 `{}` 所需工具 `{}` 的启动路径无效",
                manifest.name, dependency.id
            )
        })?;
        environment.insert(
            dependency.executable_path_env.clone(),
            launcher.display().to_string(),
        );
    }
    Ok(environment)
}

fn absolute_launcher_path(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        anyhow::bail!(
            "managed tool launcher path must be absolute: {}",
            path.display()
        );
    }
    if !path.is_file() {
        anyhow::bail!("managed tool launcher does not exist: {}", path.display());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::absolute_launcher_path;

    #[test]
    fn dependency_launcher_must_be_absolute_and_existing() {
        assert!(absolute_launcher_path("baijimu").is_err());
        let temporary = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            absolute_launcher_path(temporary.path().to_str().unwrap()).unwrap(),
            temporary.path()
        );
    }
}
