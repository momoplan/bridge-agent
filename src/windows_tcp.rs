#[cfg(any(windows, test))]
use std::net::{IpAddr, SocketAddr};

#[cfg(windows)]
pub(crate) fn find_windows_tcp_listener_pid(bind: SocketAddr) -> anyhow::Result<Option<u32>> {
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};

    let address_family = if bind.is_ipv4() { AF_INET } else { AF_INET6 };
    let mut byte_len = 0u32;
    let first_result = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut byte_len,
            0,
            address_family as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if first_result != 0 && first_result != ERROR_INSUFFICIENT_BUFFER {
        anyhow::bail!(
            "GetExtendedTcpTable failed while sizing the local TCP table: Windows error {first_result}"
        );
    }
    if byte_len < std::mem::size_of::<u32>() as u32 {
        return Ok(None);
    }

    for _ in 0..3 {
        // u32 storage gives the header and row structures four-byte alignment.
        let word_len = (byte_len as usize).div_ceil(std::mem::size_of::<u32>());
        let mut buffer = vec![0u32; word_len];
        let mut actual_len = (buffer.len() * std::mem::size_of::<u32>()) as u32;
        let result = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr().cast(),
                &mut actual_len,
                0,
                address_family as u32,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if result == ERROR_INSUFFICIENT_BUFFER {
            byte_len = actual_len;
            continue;
        }
        if result != 0 {
            anyhow::bail!(
                "GetExtendedTcpTable failed while reading the local TCP table: Windows error {result}"
            );
        }
        if actual_len < std::mem::size_of::<u32>() as u32
            || actual_len as usize > buffer.len() * std::mem::size_of::<u32>()
        {
            anyhow::bail!("Windows TCP table returned an invalid buffer length");
        }

        let entry_count = buffer[0] as usize;
        let row_bytes = actual_len as usize - std::mem::size_of::<u32>();
        if bind.is_ipv4() {
            let available_rows = row_bytes / std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
            if entry_count > available_rows {
                anyhow::bail!("Windows IPv4 TCP table returned an invalid row count");
            }
            let rows = unsafe {
                std::slice::from_raw_parts(
                    buffer.as_ptr().add(1).cast::<MIB_TCPROW_OWNER_PID>(),
                    entry_count,
                )
            };
            return Ok(rows.iter().find_map(|row| {
                let local_ip = std::net::Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes());
                windows_listener_matches(bind, local_ip.into(), row.dwLocalPort)
                    .then_some(row.dwOwningPid)
            }));
        }

        let available_rows = row_bytes / std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
        if entry_count > available_rows {
            anyhow::bail!("Windows IPv6 TCP table returned an invalid row count");
        }
        let rows = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().add(1).cast::<MIB_TCP6ROW_OWNER_PID>(),
                entry_count,
            )
        };
        return Ok(rows.iter().find_map(|row| {
            let local_ip = std::net::Ipv6Addr::from(row.ucLocalAddr);
            windows_listener_matches(bind, local_ip.into(), row.dwLocalPort)
                .then_some(row.dwOwningPid)
        }));
    }

    anyhow::bail!("Windows TCP table changed repeatedly while inspecting local listeners")
}

#[cfg(any(windows, test))]
pub(crate) fn windows_listener_matches(
    bind: SocketAddr,
    listener_ip: IpAddr,
    raw_port: u32,
) -> bool {
    let listener_port = u16::from_be(raw_port as u16);
    listener_port == bind.port()
        && listener_ip.is_ipv4() == bind.is_ipv4()
        && (listener_ip.is_unspecified() || listener_ip == bind.ip())
}
