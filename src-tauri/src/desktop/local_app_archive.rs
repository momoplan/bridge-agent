use super::*;

#[derive(Clone, Copy)]
pub(super) enum ConnectorArchiveKind {
    Zip,
    TarGz,
}

pub(super) async fn resolve_connector_archive_source(
    archive_url: &str,
    expected_checksum: Option<&str>,
    progress: Option<&LocalAppInstallProgressReporter>,
) -> Result<ResolvedConnectorSource, String> {
    let kind = connector_archive_kind(archive_url)
        .ok_or_else(|| "本地应用下载源必须是 .zip、.tar.gz 或 .tgz 文件。".to_string())?;
    let mut response = Client::new()
        .get(archive_url)
        .header(reqwest::header::USER_AGENT, CONNECTOR_DOWNLOAD_USER_AGENT)
        .send()
        .await
        .map_err(|err| format!("下载本地应用失败: {err}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let payload = response.text().await.unwrap_or_default();
        return Err(format!("下载本地应用失败 ({status}): {payload}"));
    }
    let total_bytes = response.content_length();
    let mut bytes = Vec::with_capacity(
        total_bytes
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or_default(),
    );
    if let Some(progress) = progress {
        progress.download(0, total_bytes);
    }
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| format!("读取本地应用下载包失败: {err}"))?
    {
        bytes.extend_from_slice(&chunk);
        if let Some(progress) = progress {
            progress.download(bytes.len() as u64, total_bytes);
        }
    }
    if let Some(progress) = progress {
        progress.report(
            LocalAppInstallTaskPhase::Verifying,
            Some(58),
            "下载完成，正在校验应用包",
        );
    }
    verify_connector_archive_checksum(bytes.as_ref(), expected_checksum)?;
    let temp_dir = tempfile::tempdir().map_err(|err| err.to_string())?;
    let extract_dir = temp_dir.path().join("connector-archive");
    fs::create_dir_all(&extract_dir).map_err(|err| format!("创建本地应用解压目录失败: {err}"))?;
    extract_connector_archive(bytes.as_ref(), kind, &extract_dir)?;
    let path = find_extracted_connector_root(&extract_dir)?;
    Ok(ResolvedConnectorSource::Archive {
        path,
        _temp_dir: temp_dir,
    })
}

pub(super) fn verify_connector_archive_checksum(
    bytes: &[u8],
    expected_checksum: Option<&str>,
) -> Result<(), String> {
    let Some(expected) = expected_checksum
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let expected = expected
        .strip_prefix("sha256:")
        .unwrap_or(expected)
        .to_ascii_lowercase();
    if expected.len() != 64
        || !expected
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("本地应用 SHA-256 checksum 格式无效".to_string());
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(format!(
            "本地应用下载包 SHA-256 校验失败：期望 {expected}，实际 {actual}"
        ));
    }
    Ok(())
}

pub(super) fn connector_archive_download_url(
    source: &str,
    revision: Option<&str>,
    allow_git: bool,
) -> Result<Option<String>, String> {
    if connector_archive_kind(source).is_some() {
        return Ok(Some(source.trim().to_string()));
    }
    if !is_http_connector_source(source) {
        return Ok(None);
    }
    if !is_git_connector_source(source) {
        return Err(
            "HTTP(S) 本地应用安装源必须是 .zip、.tar.gz、.tgz，或可转换为源码包的 GitHub/Gitee 仓库 URL。".to_string(),
        );
    }
    if allow_git {
        return Ok(None);
    }
    github_archive_url(source, revision)
        .or_else(|| gitee_archive_url(source, revision))
        .map(Some)
        .ok_or_else(|| {
            "市场本地应用不能依赖本机 git，请将安装源发布为 .zip 或 .tar.gz 下载包。".to_string()
        })
}

pub(super) fn connector_archive_kind(source: &str) -> Option<ConnectorArchiveKind> {
    let source = source
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .to_ascii_lowercase();
    if source.ends_with(".zip") {
        Some(ConnectorArchiveKind::Zip)
    } else if source.ends_with(".tar.gz") || source.ends_with(".tgz") {
        Some(ConnectorArchiveKind::TarGz)
    } else {
        None
    }
}

pub(super) fn github_archive_url(source: &str, revision: Option<&str>) -> Option<String> {
    let (owner, repo) = parse_https_git_repo(source, "github.com")?;
    let revision = revision?.trim();
    if revision.is_empty() {
        return None;
    }
    Some(format!(
        "https://github.com/{owner}/{repo}/archive/{revision}.zip"
    ))
}

pub(super) fn gitee_archive_url(source: &str, revision: Option<&str>) -> Option<String> {
    let (owner, repo) = parse_https_git_repo(source, "gitee.com")?;
    let revision = revision?.trim();
    if revision.is_empty() {
        return None;
    }
    Some(format!(
        "https://gitee.com/{owner}/{repo}/repository/archive/{revision}.zip"
    ))
}

pub(super) fn parse_https_git_repo(source: &str, host: &str) -> Option<(String, String)> {
    let without_scheme = source
        .strip_prefix("https://")
        .or_else(|| source.strip_prefix("http://"))?;
    let path = without_scheme.strip_prefix(host)?.trim_start_matches('/');
    let mut parts = path.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim().trim_end_matches(".git");
    if parts.next().is_some() {
        return None;
    }
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

pub(super) fn extract_connector_archive(
    bytes: &[u8],
    kind: ConnectorArchiveKind,
    destination: &Path,
) -> Result<(), String> {
    match kind {
        ConnectorArchiveKind::Zip => extract_connector_zip(bytes, destination),
        ConnectorArchiveKind::TarGz => extract_connector_tar_gz(bytes, destination),
    }
}

pub(super) fn extract_connector_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|err| format!("解析本地应用 zip 失败: {err}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("读取本地应用 zip 条目失败: {err}"))?;
        let Some(relative_path) = entry.enclosed_name() else {
            return Err("本地应用 zip 包含不安全路径。".to_string());
        };
        let target = destination.join(relative_path);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|err| format!("创建解压目录失败: {err}"))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("创建解压目录失败: {err}"))?;
        }
        let mut file =
            fs::File::create(&target).map_err(|err| format!("写入解压文件失败: {err}"))?;
        std::io::copy(&mut entry, &mut file).map_err(|err| format!("写入解压文件失败: {err}"))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            let mut permissions = file
                .metadata()
                .map_err(|err| format!("读取解压文件权限失败: {err}"))?
                .permissions();
            permissions.set_mode(mode);
            fs::set_permissions(&target, permissions)
                .map_err(|err| format!("设置解压文件权限失败: {err}"))?;
        }
    }
    Ok(())
}

pub(super) fn extract_connector_tar_gz(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| format!("解析本地应用 tar.gz 失败: {err}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|err| format!("读取本地应用 tar.gz 条目失败: {err}"))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err("本地应用 tar.gz 包含不支持的链接文件。".to_string());
        }
        let relative_path = sanitize_archive_path(
            &entry
                .path()
                .map_err(|err| format!("读取 tar.gz 路径失败: {err}"))?,
        )?;
        let target = destination.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("创建解压目录失败: {err}"))?;
        }
        entry
            .unpack(&target)
            .map_err(|err| format!("解压本地应用 tar.gz 失败: {err}"))?;
    }
    Ok(())
}

pub(super) fn sanitize_archive_path(path: &Path) -> Result<PathBuf, String> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => clean.push(part),
            std::path::Component::CurDir => {}
            _ => return Err("本地应用压缩包包含不安全路径。".to_string()),
        }
    }
    if clean.as_os_str().is_empty() {
        return Err("本地应用压缩包包含空路径。".to_string());
    }
    Ok(clean)
}

pub(super) fn find_extracted_connector_root(extract_dir: &Path) -> Result<PathBuf, String> {
    let mut manifests = Vec::new();
    collect_connector_manifests(extract_dir, &mut manifests)
        .map_err(|err| format!("查找本地应用清单失败: {err}"))?;
    match manifests.len() {
        0 => Err("下载包中没有找到 connector.json。".to_string()),
        1 => Ok(manifests
            .remove(0)
            .parent()
            .unwrap_or(extract_dir)
            .to_path_buf()),
        _ => Err("下载包中包含多个 connector.json，无法确定应用根目录。".to_string()),
    }
}

pub(super) fn collect_connector_manifests(
    dir: &Path,
    manifests: &mut Vec<PathBuf>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        if file_name.to_str().is_some_and(|name| name == "__MACOSX") {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_connector_manifests(&path, manifests)?;
        } else if file_type.is_file()
            && file_name
                .to_str()
                .is_some_and(|name| name == CONNECTOR_MANIFEST_FILE)
        {
            manifests.push(path);
        }
    }
    Ok(())
}
