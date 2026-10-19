use anyhow::{bail, Result};
use std::path::PathBuf;
use std::sync::OnceLock;

// The framework owns the installed resource root for this process. In particular,
// Linux resources are not adjacent to /usr/bin and must not depend on the cwd.
static RESOURCE_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub fn initialize(root: PathBuf) -> Result<()> {
    if RESOURCE_ROOT.set(root).is_err() {
        bail!("bundled resource root was already initialized");
    }
    Ok(())
}

pub fn binary_path(name: &str) -> Option<PathBuf> {
    RESOURCE_ROOT
        .get()
        .map(|root| root.join("resources").join("bin").join(name))
}
