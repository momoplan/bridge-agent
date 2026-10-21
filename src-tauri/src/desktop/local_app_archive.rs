use super::*;

#[derive(Clone, Copy)]
pub(super) enum ConnectorArchiveKind {
    Zip,
    TarGz,
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

const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy)]
struct ArchiveLimits {
    max_entries: usize,
    max_uncompressed_bytes: u64,
}

fn default_archive_limits() -> ArchiveLimits {
    ArchiveLimits {
        max_entries: MAX_ARCHIVE_ENTRIES,
        max_uncompressed_bytes: MAX_ARCHIVE_UNCOMPRESSED_BYTES,
    }
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
    extract_zip_reader(Cursor::new(bytes), destination, default_archive_limits())
}

fn extract_zip_reader<R: Read + std::io::Seek>(
    reader: R,
    destination: &Path,
    limits: ArchiveLimits,
) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(reader).map_err(|err| format!("解析本地应用 zip 失败: {err}"))?;
    if archive.len() > limits.max_entries {
        return Err("本地应用压缩包条目过多。".to_string());
    }
    let mut written = 0u64;
    for index in 0..archive.len() {
        extract_zip_entry(&mut archive, index, destination, limits, &mut written)?;
    }
    Ok(())
}

fn extract_zip_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
    destination: &Path,
    limits: ArchiveLimits,
    written: &mut u64,
) -> Result<(), String> {
    let mut entry = archive
        .by_index(index)
        .map_err(|err| format!("读取本地应用 zip 条目失败: {err}"))?;
    let Some(relative_path) = entry.enclosed_name().map(|path| path.to_path_buf()) else {
        return Err("本地应用 zip 包含不安全路径。".to_string());
    };
    let mode = entry.unix_mode();
    let target = destination.join(relative_path);
    if entry.is_dir() {
        fs::create_dir_all(&target).map_err(|err| format!("创建解压目录失败: {err}"))?;
        return apply_archive_mode(&target, mode);
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建解压目录失败: {err}"))?;
    }
    let mut file = fs::File::create(&target).map_err(|err| format!("写入解压文件失败: {err}"))?;
    copy_limited(
        &mut entry,
        &mut file,
        written,
        limits.max_uncompressed_bytes,
    )?;
    apply_archive_mode(&target, mode)
}

pub(super) fn extract_connector_tar_gz(bytes: &[u8], destination: &Path) -> Result<(), String> {
    extract_tar_reader(Cursor::new(bytes), destination, default_archive_limits())
}

fn extract_tar_reader<R: Read>(
    reader: R,
    destination: &Path,
    limits: ArchiveLimits,
) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|err| format!("解析本地应用 tar.gz 失败: {err}"))?;
    let mut written = 0u64;
    let mut count = 0usize;
    for entry in entries {
        count += 1;
        if count > limits.max_entries {
            return Err("本地应用压缩包条目过多。".to_string());
        }
        let mut entry = entry.map_err(|err| format!("读取本地应用 tar.gz 条目失败: {err}"))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err("本地应用 tar.gz 包含不支持的链接文件。".to_string());
        }
        let mode = entry.header().mode().ok();
        let relative_path = sanitize_archive_path(
            &entry
                .path()
                .map_err(|err| format!("读取 tar.gz 路径失败: {err}"))?,
        )?;
        let target = destination.join(relative_path);
        if entry_type.is_dir() {
            fs::create_dir_all(&target).map_err(|err| format!("创建解压目录失败: {err}"))?;
            apply_archive_mode(&target, mode)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("创建解压目录失败: {err}"))?;
        }
        let mut file =
            fs::File::create(&target).map_err(|err| format!("写入解压文件失败: {err}"))?;
        copy_limited(
            &mut entry,
            &mut file,
            &mut written,
            limits.max_uncompressed_bytes,
        )?;
        apply_archive_mode(&target, mode)?;
    }
    Ok(())
}

fn copy_limited(
    reader: &mut impl Read,
    writer: &mut impl Write,
    written: &mut u64,
    max_bytes: u64,
) -> Result<(), String> {
    let mut buffer = [0u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|err| format!("写入解压文件失败: {err}"))?;
        if read == 0 {
            return Ok(());
        }
        let next = written
            .checked_add(read as u64)
            .filter(|size| *size <= max_bytes)
            .ok_or_else(|| "本地应用压缩包解压后超过大小限制。".to_string())?;
        writer
            .write_all(&buffer[..read])
            .map_err(|err| format!("写入解压文件失败: {err}"))?;
        *written = next;
    }
}

fn apply_archive_mode(path: &Path, mode: Option<u32>) -> Result<(), String> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        let mut permissions = fs::metadata(path)
            .map_err(|err| format!("读取解压文件权限失败: {err}"))?
            .permissions();
        permissions.set_mode(mode & 0o755);
        fs::set_permissions(path, permissions)
            .map_err(|err| format!("设置解压文件权限失败: {err}"))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tight_limits(entries: usize, bytes: u64) -> ArchiveLimits {
        ArchiveLimits {
            max_entries: entries,
            max_uncompressed_bytes: bytes,
        }
    }

    fn zip_bytes(files: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        for (name, mode, body) in files {
            let options = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            writer.start_file(*name, options).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        cursor.into_inner()
    }

    fn tar_gz_bytes(files: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, mode, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_mode(*mode);
            header.set_size(body.len() as u64);
            header.set_cksum();
            builder.append_data(&mut header, *name, *body).unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn zip_extraction_drops_setuid_and_rejects_oversized_output() {
        let dir = tempfile::tempdir().unwrap();
        let archive = zip_bytes(&[("payload.txt", 0o4755, b"hello")]);
        extract_zip_reader(Cursor::new(archive), dir.path(), tight_limits(10, 1024)).unwrap();
        let extracted = dir.path().join("payload.txt");
        assert_eq!(fs::read(&extracted).unwrap(), b"hello");
        #[cfg(unix)]
        {
            let mode = fs::metadata(&extracted).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }

        let bomb = zip_bytes(&[("big.txt", 0o644, &[b'a'; 32])]);
        let error =
            extract_zip_reader(Cursor::new(bomb), dir.path(), tight_limits(10, 8)).unwrap_err();
        assert!(error.contains("超过大小限制"));
    }

    #[test]
    fn archive_extraction_rejects_too_many_entries() {
        let dir = tempfile::tempdir().unwrap();
        let zip_archive = zip_bytes(&[
            ("a.txt", 0o644, b"a"),
            ("b.txt", 0o644, b"b"),
            ("c.txt", 0o644, b"c"),
        ]);
        let zip_error =
            extract_zip_reader(Cursor::new(zip_archive), dir.path(), tight_limits(2, 1024))
                .unwrap_err();
        assert!(zip_error.contains("条目过多"));

        let tar_archive = tar_gz_bytes(&[
            ("a.txt", 0o4755, b"a"),
            ("b.txt", 0o644, b"b"),
            ("c.txt", 0o644, b"c"),
        ]);
        let tar_error =
            extract_tar_reader(Cursor::new(tar_archive), dir.path(), tight_limits(2, 1024))
                .unwrap_err();
        assert!(tar_error.contains("条目过多"));
    }

    #[test]
    fn tar_extraction_drops_setuid_and_limits_uncompressed_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz_bytes(&[("payload.txt", 0o6755, b"hello")]);
        extract_tar_reader(Cursor::new(archive), dir.path(), tight_limits(10, 1024)).unwrap();
        let extracted = dir.path().join("payload.txt");
        assert_eq!(fs::read(&extracted).unwrap(), b"hello");
        #[cfg(unix)]
        {
            let mode = fs::metadata(&extracted).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }

        let bomb = tar_gz_bytes(&[("big.txt", 0o644, &[b'a'; 32])]);
        let error =
            extract_tar_reader(Cursor::new(bomb), dir.path(), tight_limits(10, 8)).unwrap_err();
        assert!(error.contains("超过大小限制"));
    }
}
