use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SKILL_NAME: &str = "baijimu-platform";
const SKILL_FILE_NAME: &str = "SKILL.md";
const MAX_SKILL_BYTES: usize = 256 * 1024;
const BUNDLED_SKILL: &[u8] = include_bytes!("../resources/skills/baijimu-platform/SKILL.md");
const BUNDLED_PROVENANCE: &str =
    include_str!("../resources/skills/baijimu-platform/PROVENANCE.json");
const LEGACY_SKILL_NAMES: &[&str] = &["baijimu-docs"];

pub fn install_bundled() -> Result<PathBuf> {
    validate_skill(BUNDLED_SKILL)?;
    let skills_root = skills_root()?;
    let installed = install(
        BUNDLED_SKILL,
        &skills_root.join(SKILL_NAME).join(SKILL_FILE_NAME),
    )?;
    migrate_legacy_skills(&skills_root)?;
    Ok(installed)
}

fn install(contents: &[u8], target: &Path) -> Result<PathBuf> {
    if fs::read(target).is_ok_and(|existing| existing == contents) {
        set_skill_permissions(target)?;
        return Ok(target.to_path_buf());
    }

    let parent = target
        .parent()
        .context("Codex skill target has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create Codex skill directory {}",
            parent.display()
        )
    })?;

    let temporary = parent.join(format!(
        ".{SKILL_FILE_NAME}-{}-{}.tmp",
        std::process::id(),
        now_nanos()
    ));
    let mut file = fs::File::create(&temporary).with_context(|| {
        format!(
            "failed to create temporary Codex skill {}",
            temporary.display()
        )
    })?;
    file.write_all(contents)?;
    file.sync_all()?;
    set_skill_permissions(&temporary)?;

    #[cfg(windows)]
    {
        let backup = parent.join(format!(
            ".{SKILL_FILE_NAME}-{}-{}.bak",
            std::process::id(),
            now_nanos()
        ));
        if target.exists() {
            fs::rename(target, &backup).with_context(|| {
                format!(
                    "failed to prepare Codex skill replacement at {}",
                    target.display()
                )
            })?;
        }
        if let Err(error) = fs::rename(&temporary, target) {
            if backup.exists() {
                let _ = fs::rename(&backup, target);
            }
            return Err(error.into());
        }
        let _ = fs::remove_file(backup);
    }
    #[cfg(not(windows))]
    fs::rename(&temporary, target)
        .with_context(|| format!("failed to install Codex skill at {}", target.display()))?;

    set_skill_permissions(target)?;
    Ok(target.to_path_buf())
}

fn validate_skill(contents: &[u8]) -> Result<()> {
    if contents.is_empty() || contents.len() > MAX_SKILL_BYTES {
        bail!("bundled Codex skill has an invalid size");
    }
    let contents = std::str::from_utf8(contents).context("bundled Codex skill is not UTF-8")?;
    if !contents.starts_with("---\n")
        || !contents.contains("\nname: baijimu-platform\n")
        || !contents.contains("\ndescription:")
        || !contents.contains("https://docs.baijimu.com/")
    {
        bail!("bundled Codex skill is missing required frontmatter");
    }
    let provenance: Value =
        serde_json::from_str(BUNDLED_PROVENANCE).context("bundled skill provenance is invalid")?;
    if provenance.get("repository").and_then(Value::as_str)
        != Some("https://github.com/momoplan/baijimu-platform-skill")
        || !provenance
            .get("release")
            .and_then(Value::as_str)
            .is_some_and(|release| release.starts_with('v'))
        || !provenance
            .get("archiveSha256")
            .and_then(Value::as_str)
            .is_some_and(|digest| digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()))
    {
        bail!("bundled skill provenance does not identify an immutable upstream release");
    }
    let expected = provenance
        .get("skillSha256")
        .and_then(Value::as_str)
        .context("bundled skill provenance is missing skillSha256")?;
    let actual = format!("{:x}", Sha256::digest(contents.as_bytes()));
    if actual != expected {
        bail!("bundled Codex skill does not match its pinned release provenance");
    }
    Ok(())
}

fn skills_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("BAIJIMU_AGENT_SKILLS_DIR") {
        return Ok(PathBuf::from(root));
    }
    if let Some(root) = std::env::var_os("BAIJIMU_CODEX_SKILLS_DIR") {
        return Ok(PathBuf::from(root));
    }
    Ok(dirs::home_dir()
        .context("current user home directory is unavailable")?
        .join(".agents")
        .join("skills"))
}

fn migrate_legacy_skills(skills_root: &Path) -> Result<()> {
    let backup_root = skills_root
        .parent()
        .context("Agent skills root has no parent directory")?
        .join("skill-backups");
    for name in LEGACY_SKILL_NAMES {
        move_to_backup(&skills_root.join(name), &backup_root, name)?;
    }

    let legacy_codex_root = if let Some(root) =
        std::env::var_os("BAIJIMU_LEGACY_CODEX_SKILLS_DIR")
    {
        PathBuf::from(root)
    } else {
        dirs::home_dir()
            .context("current user home directory is unavailable")?
            .join(".codex")
            .join("skills")
    };
    if legacy_codex_root != skills_root {
        for name in [SKILL_NAME, "baijimu-docs"] {
            move_to_backup(
                &legacy_codex_root.join(name),
                &backup_root,
                &format!("legacy-codex-{name}"),
            )?;
        }
    }
    Ok(())
}

fn move_to_backup(source: &Path, backup_root: &Path, label: &str) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    fs::create_dir_all(backup_root)
        .with_context(|| format!("failed to create skill backup root {}", backup_root.display()))?;
    let backup = backup_root.join(format!("{label}.backup-{}", now_nanos()));
    fs::rename(source, &backup).with_context(|| {
        format!(
            "failed to migrate legacy skill {} to {}",
            source.display(),
            backup.display()
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn set_skill_permissions(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_skill_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn installs_bundled_skill_in_user_skill_directory() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var("BAIJIMU_AGENT_SKILLS_DIR", temp.path().join("skills"));
        std::env::set_var(
            "BAIJIMU_LEGACY_CODEX_SKILLS_DIR",
            temp.path().join("legacy-codex-skills"),
        );

        let installed = install_bundled().unwrap();
        assert_eq!(
            installed,
            temp.path()
                .join("skills")
                .join("baijimu-platform")
                .join("SKILL.md")
        );
        assert_eq!(fs::read(&installed).unwrap(), BUNDLED_SKILL);

        std::env::remove_var("BAIJIMU_AGENT_SKILLS_DIR");
        std::env::remove_var("BAIJIMU_LEGACY_CODEX_SKILLS_DIR");
    }

    #[test]
    fn bundled_install_is_idempotent_and_repairs_modified_content() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        std::env::set_var("BAIJIMU_AGENT_SKILLS_DIR", temp.path().join("skills"));
        std::env::set_var(
            "BAIJIMU_LEGACY_CODEX_SKILLS_DIR",
            temp.path().join("legacy-codex-skills"),
        );

        let installed = install_bundled().unwrap();
        fs::write(&installed, b"locally modified").unwrap();
        let repaired = install_bundled().unwrap();

        assert_eq!(repaired, installed);
        assert_eq!(fs::read(repaired).unwrap(), BUNDLED_SKILL);

        std::env::remove_var("BAIJIMU_AGENT_SKILLS_DIR");
        std::env::remove_var("BAIJIMU_LEGACY_CODEX_SKILLS_DIR");
    }

    #[test]
    fn migrates_duplicate_legacy_skills_out_of_discovery_roots() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let skills_root = temp.path().join(".agents").join("skills");
        let legacy_codex_root = temp.path().join(".codex").join("skills");
        fs::create_dir_all(skills_root.join("baijimu-docs")).unwrap();
        fs::write(
            skills_root.join("baijimu-docs").join("SKILL.md"),
            b"legacy docs",
        )
        .unwrap();
        fs::create_dir_all(legacy_codex_root.join("baijimu-platform")).unwrap();
        fs::write(
            legacy_codex_root
                .join("baijimu-platform")
                .join("SKILL.md"),
            b"legacy platform",
        )
        .unwrap();
        std::env::set_var("BAIJIMU_AGENT_SKILLS_DIR", &skills_root);
        std::env::set_var("BAIJIMU_LEGACY_CODEX_SKILLS_DIR", &legacy_codex_root);

        install_bundled().unwrap();

        assert!(!skills_root.join("baijimu-docs").exists());
        assert!(!legacy_codex_root.join("baijimu-platform").exists());
        let backup_count = fs::read_dir(temp.path().join(".agents").join("skill-backups"))
            .unwrap()
            .count();
        assert_eq!(backup_count, 2);

        std::env::remove_var("BAIJIMU_AGENT_SKILLS_DIR");
        std::env::remove_var("BAIJIMU_LEGACY_CODEX_SKILLS_DIR");
    }

    #[test]
    fn rejects_invalid_skill_frontmatter() {
        assert!(validate_skill(b"not a skill").is_err());
    }
}
