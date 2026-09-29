//! Bridge-owned offline/startup migration, versioned and shipped with the host release.
mod source;
mod startup;
#[cfg(test)]
mod tests;

use anyhow::{ensure, Context, Result};
use clap::Parser;
use fs2::FileExt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use sysinfo::{Pid, System};

#[derive(Parser)]
#[command(version)]
struct Args {
    /// Bridge configuration directory. The host and applications must be stopped.
    #[arg(long)]
    config_dir: PathBuf,
    /// Installed local application directory from the device configuration.
    #[arg(long)]
    local_apps_dir: PathBuf,
    /// Managed application directory from the device's configured data location.
    #[arg(long)]
    managed_apps_dir: PathBuf,
    /// Explicit acknowledgement that host and application writers have been stopped.
    #[arg(long, required_unless_present = "prepare_startup")]
    host_already_stopped: bool,
    /// Stop configured applications before migration, while desktop business startup is gated.
    #[arg(long, conflicts_with = "host_already_stopped")]
    prepare_startup: bool,
    #[arg(
        long,
        requires = "prepare_startup",
        required_if_eq("prepare_startup", "true")
    )]
    config: Option<PathBuf>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Discovery {
    pid: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.host_already_stopped || args.prepare_startup,
        "stop the host before migration"
    );
    ensure!(
        args.config_dir.is_absolute()
            && args.local_apps_dir.is_absolute()
            && args.managed_apps_dir.is_absolute(),
        "migration directories must be absolute"
    );
    fs::create_dir_all(&args.config_dir)?;
    let lock_path = args.config_dir.join("environment-identity-migration.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .context("another identity migration is running")?;
    let files = discover(&args.local_apps_dir, &args.managed_apps_dir)?;
    let changes = plan(&files)?;
    if !changes.is_empty() {
        verify_host_stopped(&args.config_dir, &System::new_all())?;
        if args.prepare_startup {
            startup::stop_applications(args.config.as_deref().context("missing startup config")?)?;
        }
        verify_stopped(
            &args.config_dir,
            &[&args.local_apps_dir, &args.managed_apps_dir],
        )?;
        apply(changes)?;
    }
    FileExt::unlock(&lock)?;
    println!("Environment identity migration completed; source-less records remain unclaimed");
    Ok(())
}

fn verify_stopped(config: &Path, roots: &[&Path]) -> Result<()> {
    ensure!(
        config.is_absolute() && roots.iter().all(|path| path.is_absolute()),
        "migration directories must be absolute"
    );
    let system = System::new_all();
    verify_host_stopped(config, &system)?;
    let mut canonical_roots = Vec::new();
    for root in roots {
        if root.try_exists()? {
            canonical_roots.push(root.canonicalize()?);
        }
    }
    let uses_installation = |path: &Path| {
        roots.iter().any(|root| path.starts_with(root))
            || path
                .canonicalize()
                .is_ok_and(|path| canonical_roots.iter().any(|root| path.starts_with(root)))
    };
    let own_pid = Pid::from_u32(std::process::id());
    let own_threads = system.process(own_pid).and_then(|process| process.tasks());
    for process in system.processes().values() {
        // Our required directory arguments identify the migration targets, not a writer.
        // Linux also exposes the process's worker threads as individual task records.
        if process.pid() == own_pid
            || own_threads.is_some_and(|threads| threads.contains(&process.pid()))
        {
            continue;
        }
        ensure!(
            !(process.exe().is_some_and(uses_installation)
                || process.cwd().is_some_and(uses_installation)
                || process
                    .cmd()
                    .iter()
                    .any(|arg| uses_installation(Path::new(arg)))),
            "an application is still using its installation"
        );
    }
    Ok(())
}

fn verify_host_stopped(config: &Path, system: &System) -> Result<()> {
    let discovery = config.join("local-app-control.json");
    if discovery.is_file() {
        let record: Discovery = serde_json::from_slice(&fs::read(discovery)?)?;
        ensure!(
            system.process(Pid::from_u32(record.pid)).is_none(),
            "Bridge is still running"
        );
    }
    Ok(())
}

fn discover(local_apps: &Path, managed: &Path) -> Result<Vec<(PathBuf, bool)>> {
    let mut files = Vec::new();
    for (root, name) in [
        (local_apps.to_path_buf(), "install.json"),
        (managed.to_path_buf(), "state.json"),
    ] {
        if !root.exists() {
            continue;
        }
        for child in fs::read_dir(root)? {
            let child = child?;
            if child.file_type()?.is_dir() {
                let path = child.path().join(name);
                if path.is_file() {
                    files.push((path, false));
                }
            }
        }
    }
    collect_sidecars(managed, &mut files)?;
    Ok(files)
}

fn collect_sidecars(root: &Path, files: &mut Vec<(PathBuf, bool)>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_sidecars(&entry.path(), files)?;
        } else if kind.is_file()
            && entry
                .file_name()
                .to_string_lossy()
                .ends_with(".market.json")
        {
            files.push((entry.path(), true));
        }
    }
    Ok(())
}

type Change = (PathBuf, Vec<u8>, Vec<u8>);

fn plan(files: &[(PathBuf, bool)]) -> Result<Vec<Change>> {
    // Parse every candidate first: invalid provenance cannot cause a partially applied plan.
    let mut changes = Vec::new();
    for (path, sidecar) in files {
        let original = fs::read(path)?;
        if let Some(next) = source::convert(&original, *sidecar)
            .with_context(|| format!("invalid provenance in {}", path.display()))?
        {
            changes.push((path.clone(), original, next));
        }
    }
    Ok(changes)
}

#[cfg(test)]
fn migrate(files: &[(PathBuf, bool)]) -> Result<()> {
    apply(plan(files)?)
}

fn apply(changes: Vec<Change>) -> Result<()> {
    for (path, original, next) in changes {
        ensure!(
            fs::read(&path)? == original,
            "installation changed during migration"
        );
        let backup = path.with_extension("json.before-environment-identity-3");
        if backup.exists() {
            ensure!(
                fs::read(&backup)? == original,
                "conflicting migration backup"
            );
        } else {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)?;
            file.write_all(&original)?;
            file.sync_all()?;
        }
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("missing parent")?)?;
        temp.write_all(&next)?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
    }
    Ok(())
}
