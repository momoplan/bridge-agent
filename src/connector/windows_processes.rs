use anyhow::{Context, Result};
use std::ffi::{OsStr, OsString};
use std::os::windows::{
    ffi::{OsStrExt, OsStringExt},
    io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use std::path::Path;
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ACCESS_DENIED, ERROR_GEN_FAILURE, ERROR_INVALID_PARAMETER, ERROR_NO_MORE_FILES,
        INVALID_HANDLE_VALUE, STILL_ACTIVE,
    },
    Globalization::{CompareStringOrdinal, CSTR_EQUAL},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, PROCESS_BASIC_INFORMATION,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};

// Query the native process snapshot directly. Starting PowerShell and its CIM
// provider can exceed the old five-second deadline before any process is read.
pub(super) fn package_processes(package: &Path) -> Result<Vec<u32>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .context("failed to snapshot Windows processes");
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.cast()) };
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle().cast(), &mut entry) };
    let mut pids = Vec::new();
    loop {
        if found == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                break;
            }
            return Err(error).context("failed to enumerate Windows processes");
        }
        if let Some(executable) = process_image(entry.th32ProcessID)? {
            if executable_belongs_to_package(&executable, package.as_os_str())? {
                pids.push(entry.th32ProcessID);
            }
        }
        found = unsafe { Process32NextW(snapshot.as_raw_handle().cast(), &mut entry) };
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

fn process_image(pid: u32) -> Result<Option<OsString>> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        let error = std::io::Error::last_os_error();
        // System/protected processes expose no image to this user, as with CIM.
        // Invalid PIDs include the idle process and processes that just exited.
        if matches!(
            error.raw_os_error(),
            Some(code) if code == ERROR_ACCESS_DENIED as i32 || code == ERROR_INVALID_PARAMETER as i32
        ) {
            return Ok(None);
        }
        return Err(error).with_context(|| format!("failed to open Windows process {pid}"));
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process.cast()) };
    // Windows paths contain at most 32,767 UTF-16 code units plus the terminator.
    let mut image = vec![0u16; 32_768];
    let mut length = image.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle().cast(),
            0,
            image.as_mut_ptr(),
            &mut length,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        let mut exit_code = 0;
        let exited = unsafe { GetExitCodeProcess(process.as_raw_handle().cast(), &mut exit_code) }
            != 0
            && exit_code != STILL_ACTIVE as u32;
        if exited || error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
            return Ok(None);
        }
        // System and other kernel processes expose no Win32 executable and
        // return ERROR_GEN_FAILURE. Confirm the absence of a user-mode PEB
        // instead of assuming every such error is harmless or naming PIDs.
        if error.raw_os_error() == Some(ERROR_GEN_FAILURE as i32)
            && !has_user_mode_environment(&process)
                .with_context(|| format!("failed to classify Windows process {pid}"))?
        {
            return Ok(None);
        }
        return Err(error).with_context(|| format!("failed to read Windows process {pid} image"));
    }
    Ok(Some(OsString::from_wide(&image[..length as usize])))
}

fn has_user_mode_environment(process: &OwnedHandle) -> Result<bool> {
    let mut information = PROCESS_BASIC_INFORMATION::default();
    let status = unsafe {
        NtQueryInformationProcess(
            process.as_raw_handle().cast(),
            ProcessBasicInformation,
            (&mut information as *mut PROCESS_BASIC_INFORMATION).cast(),
            std::mem::size_of_val(&information) as u32,
            std::ptr::null_mut(),
        )
    };
    if status < 0 {
        anyhow::bail!("failed to query native process information (NTSTATUS 0x{status:08x})");
    }
    Ok(!information.PebBaseAddress.is_null())
}

fn normalized_path(path: &OsStr) -> Vec<u16> {
    let path: Vec<u16> = path
        .encode_wide()
        .map(|unit| {
            if unit == b'/' as u16 {
                b'\\' as u16
            } else {
                unit
            }
        })
        .collect();
    let verbatim_unc: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    if path.starts_with(&verbatim_unc) {
        r"\\"
            .encode_utf16()
            .chain(path[verbatim_unc.len()..].iter().copied())
            .collect()
    } else if path.starts_with(&verbatim) {
        path[verbatim.len()..].to_vec()
    } else {
        path
    }
}

fn executable_belongs_to_package(executable: &OsStr, package: &OsStr) -> Result<bool> {
    let executable = normalized_path(executable);
    let mut prefix = normalized_path(package);
    if prefix.last() != Some(&(b'\\' as u16)) {
        prefix.push(b'\\' as u16);
    }
    if executable.len() <= prefix.len() {
        return Ok(false);
    }
    let comparison = unsafe {
        CompareStringOrdinal(
            executable.as_ptr(),
            prefix.len() as i32,
            prefix.as_ptr(),
            prefix.len() as i32,
            1,
        )
    };
    if comparison == 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to compare Windows package paths");
    }
    Ok(comparison == CSTR_EQUAL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn kernel_processes_have_no_connector_executable() {
        assert_eq!(process_image(0).unwrap(), None);
        assert_eq!(process_image(4).unwrap(), None);
        assert!(process_image(std::process::id()).unwrap().is_some());
    }

    #[test]
    fn package_paths_preserve_unicode_case_and_directory_boundaries() {
        let package = OsStr::new(r"\\?\C:\Apps\PaÇkAge");
        assert!(
            executable_belongs_to_package(OsStr::new(r"c:\apps\paçkage\工具.exe"), package)
                .unwrap()
        );
        assert!(!executable_belongs_to_package(
            OsStr::new(r"C:\Apps\PaÇkAge-extra\工具.exe"),
            package
        )
        .unwrap());
        assert!(!executable_belongs_to_package(OsStr::new(r"C:\Other\工具.exe"), package).unwrap());
        assert!(executable_belongs_to_package(
            OsStr::new(r"\\server\share\package\工具.exe"),
            OsStr::new(r"\\?\UNC\SERVER\share\package\"),
        )
        .unwrap());
    }

    struct Fixture(Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    #[ignore = "child-process fixture invoked by the native process test"]
    fn wait_for_parent() {
        assert_eq!(
            std::env::var("BAIJIMU_NATIVE_QUERY_FIXTURE").as_deref(),
            Ok("1")
        );
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
    }

    #[test]
    fn native_query_finds_and_releases_only_the_package_process() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("包路径");
        std::fs::create_dir(&package).unwrap();
        let executable = package.join("native-query-fixture.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        let mut child = Fixture(
            Command::new(&executable)
                .args([
                    "--exact",
                    "connector::windows_processes::tests::wait_for_parent",
                    "--ignored",
                ])
                .env("BAIJIMU_NATIVE_QUERY_FIXTURE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let pids = package_processes(&package).unwrap();
            if pids.contains(&child.0.id()) {
                assert_eq!(pids, vec![child.0.id()]);
                break;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "fixture exited before inspection"
            );
            assert!(
                Instant::now() < deadline,
                "native query did not find the package process"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            package_processes(&directory.path().join("unrelated-package"))
                .unwrap()
                .is_empty()
        );
        super::super::release_connector_package_processes(&package).unwrap();
        child.0.wait().unwrap();
        assert!(package_processes(&package).unwrap().is_empty());
    }
}
