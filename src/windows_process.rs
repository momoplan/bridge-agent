use anyhow::{bail, Context, Result};
use std::path::Path;
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WindowsProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) parent_pid: Option<u32>,
    pub(crate) image_name: String,
    pub(crate) executable_path: Option<String>,
}

pub(crate) fn inspect_windows_process(pid: u32) -> Result<Option<WindowsProcessIdentity>> {
    if pid == 0 {
        return Ok(None);
    }

    let executable_path = query_process_image_path(pid);
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        if let Some(path) = executable_path {
            return Ok(Some(WindowsProcessIdentity {
                pid,
                parent_pid: None,
                image_name: process_file_name(&path).to_string(),
                executable_path: Some(path),
            }));
        }
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!("failed to snapshot Windows processes while inspecting pid {pid}")
        });
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while ok {
        if entry.th32ProcessID == pid {
            let image_name = wide_null_terminated_to_string(&entry.szExeFile).or_else(|| {
                executable_path
                    .as_deref()
                    .map(process_file_name)
                    .map(str::to_string)
            });
            if let Some(image_name) = image_name {
                found = Some(WindowsProcessIdentity {
                    pid,
                    parent_pid: Some(entry.th32ParentProcessID),
                    image_name,
                    executable_path,
                });
            }
            break;
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }

    unsafe {
        CloseHandle(snapshot);
    }
    Ok(found)
}

pub(crate) fn windows_process_is_running(pid: u32) -> bool {
    match inspect_windows_process(pid) {
        Ok(process) => process.is_some(),
        // Process inspection failing must not authorize a duplicate runtime.
        Err(_) => true,
    }
}

pub(crate) fn terminate_windows_process(expected: &WindowsProcessIdentity) -> Result<()> {
    let Some(current) = inspect_windows_process(expected.pid)? else {
        return Ok(());
    };
    if !same_process_identity(expected, &current) {
        bail!(
            "pid {} changed owner from {} to {}; refusing to terminate a reused pid",
            expected.pid,
            expected.image_name,
            current.image_name
        );
    }

    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, expected.pid) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to open pid {} for termination", expected.pid));
    }
    let terminated = unsafe { TerminateProcess(handle, 1) } != 0;
    let terminate_error = if terminated {
        None
    } else {
        Some(std::io::Error::last_os_error())
    };
    unsafe {
        CloseHandle(handle);
    }
    if let Some(err) = terminate_error {
        return Err(err).with_context(|| format!("failed to terminate pid {}", expected.pid));
    }
    Ok(())
}

fn same_process_identity(left: &WindowsProcessIdentity, right: &WindowsProcessIdentity) -> bool {
    if left.pid != right.pid {
        return false;
    }
    match (&left.executable_path, &right.executable_path) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => left.image_name.eq_ignore_ascii_case(&right.image_name),
    }
}

fn query_process_image_path(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }

    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) } != 0;
    unsafe {
        CloseHandle(handle);
    }
    if !ok || size == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..size as usize]))
}

fn wide_null_terminated_to_string(value: &[u16]) -> Option<String> {
    let end = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    (end != 0).then(|| String::from_utf16_lossy(&value[..end]))
}

fn process_file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}
