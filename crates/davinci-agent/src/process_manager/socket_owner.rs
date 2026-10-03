//! Request-time OS proof for managed loopback listeners. No shell commands.

#[cfg(any(target_os = "macos", test))]
mod macos;

#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(test)]
pub(super) fn verify(pid: u32, port: u16) -> Result<(), String> {
    verify_for(pid, port, false)
}

pub(super) fn verify_for(pid: u32, port: u16, ipv6: bool) -> Result<(), String> {
    if pid == 0 || port == 0 {
        return Err("invalid managed socket binding".into());
    }
    platform::verify(pid, port, ipv6)
        .map_err(|_| "managed listener ownership could not be proven".into())
}

pub(super) fn identity(pid: u32) -> Result<u64, String> {
    platform::birth(pid).map_err(|_| "managed process identity unavailable".into())
}

#[cfg(any(target_os = "linux", test))]
fn proc_address_matches(address: &str, ipv6: bool) -> Result<bool, ()> {
    let mut bytes = [0_u8; 16];
    let words = if ipv6 { 4 } else { 1 };
    if address.len() != words * 8 {
        return Err(());
    }
    for i in 0..words {
        let word =
            u32::from_str_radix(address.get(i * 8..i * 8 + 8).ok_or(())?, 16).map_err(|_| ())?;
        bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_ne_bytes());
    }
    Ok(if ipv6 {
        let address = std::net::Ipv6Addr::from(bytes);
        address.is_unspecified() || address.is_loopback()
    } else {
        bytes[..4] == [0; 4] || bytes[..4] == [127, 0, 0, 1]
    })
}

#[cfg(windows)]
mod platform {
    use std::collections::HashMap;
    use std::ffi::c_void;

    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn GetExtendedTcpTable(
            table: *mut c_void,
            size: *mut u32,
            order: i32,
            family: u32,
            class: u32,
            reserved: u32,
        ) -> u32;
    }
    #[repr(C)]
    struct Entry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        executable: [u16; 260],
    }
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Process32FirstW(handle: isize, entry: *mut Entry) -> i32;
        fn Process32NextW(handle: isize, entry: *mut Entry) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetLastError() -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn GetProcessTimes(
            handle: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    struct Handle(isize);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: this wrapper owns a successfully opened Windows handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    fn started(pid: u32) -> Result<u64, ()> {
        // SAFETY: scalar documented query-only access, no inherited handle.
        let handle = unsafe { OpenProcess(0x1000, 0, pid) };
        if handle == 0 {
            return Err(());
        }
        let handle = Handle(handle);
        let (mut creation, mut exit, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        // SAFETY: four valid, distinct FILETIME output buffers.
        if unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
            || exit.low != 0
            || exit.high != 0
        {
            return Err(());
        }
        Ok((u64::from(creation.high) << 32) | u64::from(creation.low))
    }
    pub(super) fn birth(pid: u32) -> Result<u64, ()> {
        started(pid)
    }
    fn parents() -> Result<HashMap<u32, u32>, ()> {
        // SAFETY: documented process snapshot flag; the PID argument is ignored.
        let raw = unsafe { CreateToolhelp32Snapshot(2, 0) };
        if raw == -1 {
            return Err(());
        }
        let handle = Handle(raw);
        // SAFETY: all fields accept zero initialization; size is set before use.
        let mut entry: Entry = unsafe { std::mem::zeroed() };
        entry.size = std::mem::size_of::<Entry>() as u32;
        let mut result = HashMap::new();
        // SAFETY: correctly sized PROCESSENTRY32W output buffer and live snapshot.
        let mut found = unsafe { Process32FirstW(handle.0, &mut entry) } != 0;
        while found {
            if result.len() >= 4096 {
                return Err(());
            }
            result.insert(entry.pid, entry.parent);
            // SAFETY: same live snapshot and correctly sized buffer.
            found = unsafe { Process32NextW(handle.0, &mut entry) } != 0;
        }
        // SAFETY: GetLastError reads this thread's immediately preceding
        // enumeration failure. Only normal end-of-snapshot is accepted.
        if unsafe { GetLastError() } != 18 {
            return Err(());
        }
        Ok(result)
    }
    fn descendant(mut pid: u32, root: u32, parents: &HashMap<u32, u32>) -> Result<(), ()> {
        for _ in 0..32 {
            if pid == root {
                return Ok(());
            }
            let parent = *parents.get(&pid).ok_or(())?;
            if parent == 0 || parent == pid || started(pid)? < started(parent)? {
                return Err(());
            }
            pid = parent;
        }
        Err(())
    }
    pub(super) fn verify(root: u32, port: u16, ipv6: bool) -> Result<(), ()> {
        let birth = started(root)?;
        let family = if ipv6 { 23 } else { 2 };
        let columns = if ipv6 { 14 } else { 6 };
        let mut size = 0;
        // SAFETY: null table requests the required size; other inputs are fixed
        // AF_INET/AF_INET6 and TCP_TABLE_OWNER_PID_LISTENER, reserved zero.
        let status =
            unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut size, 0, family, 3, 0) };
        if status != 122 {
            return Err(());
        }
        let mut table = Vec::<u32>::new();
        let mut complete = false;
        for _ in 0..2 {
            if !(4..=1024 * 1024).contains(&size) {
                return Err(());
            }
            table.resize((size as usize).div_ceil(4), 0);
            // SAFETY: u32 storage is aligned, allocated for at least size bytes.
            let status = unsafe {
                GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut size, 0, family, 3, 0)
            };
            if status == 0 {
                complete = true;
                break;
            }
            if status != 122 {
                return Err(());
            }
        }
        if !complete || size as usize > table.len() * 4 {
            return Err(());
        }
        let count = table[0] as usize;
        if count > (size as usize / 4).saturating_sub(1) / columns {
            return Err(());
        }
        let mut owners = Vec::new();
        for row in table[1..1 + count * columns].chunks_exact(columns) {
            let (state, candidate, address, owner) = if ipv6 {
                // MIB_TCP6ROW_OWNER_PID: 16-byte addresses, scope and port
                // DWORDs, then state and owner. Address bytes are network order.
                let bytes: Vec<_> = row[..4]
                    .iter()
                    .flat_map(|word| word.to_ne_bytes())
                    .collect();
                let address =
                    std::net::Ipv6Addr::from(<[u8; 16]>::try_from(bytes).map_err(|_| ())?);
                (
                    row[12],
                    row[5],
                    row[4] == 0 && (address.is_unspecified() || address.is_loopback()),
                    row[13],
                )
            } else {
                (
                    row[0],
                    row[2],
                    row[1] == 0 || row[1].to_ne_bytes() == [127, 0, 0, 1],
                    row[5],
                )
            };
            if state == 2 && u16::from_be(candidate as u16) == port && address {
                owners.push(owner);
            }
        }
        if owners.is_empty() {
            return Err(());
        }
        let parents = parents()?;
        for owner in owners {
            descendant(owner, root, &parents)?;
        }
        if started(root)? != birth {
            return Err(());
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{collections::HashSet, fs, io::Read, path::Path};

    fn read(path: impl AsRef<Path>, cap: usize) -> Result<String, ()> {
        let mut bytes = Vec::new();
        fs::File::open(path)
            .map_err(|_| ())?
            .take(cap as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() > cap {
            return Err(());
        }
        String::from_utf8(bytes).map_err(|_| ())
    }
    fn identity(pid: u32) -> Result<(u32, u64), ()> {
        let raw = read(format!("/proc/{pid}/stat"), 16 * 1024)?;
        let (_, fields) = raw.rsplit_once(')').ok_or(())?;
        let fields: Vec<_> = fields.split_whitespace().collect();
        Ok((
            fields.get(1).ok_or(())?.parse().map_err(|_| ())?,
            fields.get(19).ok_or(())?.parse().map_err(|_| ())?,
        ))
    }
    pub(super) fn birth(pid: u32) -> Result<u64, ()> {
        Ok(identity(pid)?.1)
    }
    fn descendant(mut pid: u32, root: u32) -> Result<(), ()> {
        for _ in 0..32 {
            if pid == root {
                return Ok(());
            }
            let (parent, birth) = identity(pid)?;
            if parent == 0 || parent == pid || birth < identity(parent)?.1 {
                return Err(());
            }
            pid = parent;
        }
        Err(())
    }
    pub(super) fn verify(root: u32, port: u16, ipv6: bool) -> Result<(), ()> {
        let birth = identity(root)?.1;
        let tcp = read(
            if ipv6 {
                "/proc/net/tcp6"
            } else {
                "/proc/net/tcp"
            },
            1024 * 1024,
        )?;
        let mut inodes = HashSet::new();
        for line in tcp.lines().skip(1) {
            let row: Vec<_> = line.split_whitespace().collect();
            if row.len() < 10 {
                return Err(());
            }
            let (address, candidate) = row[1].split_once(':').ok_or(())?;
            if row[3] == "0A"
                && u16::from_str_radix(candidate, 16).map_err(|_| ())? == port
                && super::proc_address_matches(address, ipv6)?
            {
                inodes.insert(format!("socket:[{}]", row[9]));
            }
        }
        if inodes.is_empty() {
            return Err(());
        }
        let mut owned = HashSet::new();
        let mut processes = 0;
        let mut descriptors = 0;
        for entry in fs::read_dir("/proc").map_err(|_| ())? {
            let entry = entry.map_err(|_| ())?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            processes += 1;
            if processes > 4096 {
                return Err(());
            }
            let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            for fd in fds {
                descriptors += 1;
                if descriptors > 65536 {
                    return Err(());
                }
                let Ok(fd) = fd else {
                    continue;
                };
                let Ok(target) = fs::read_link(fd.path()) else {
                    continue;
                };
                let Some(target) = target.to_str() else {
                    continue;
                };
                if inodes.contains(target) {
                    descendant(pid, root)?;
                    owned.insert(target.to_owned());
                }
            }
        }
        if owned != inodes || identity(root)?.1 != birth {
            return Err(());
        }
        Ok(())
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    pub(super) fn birth(_pid: u32) -> Result<u64, ()> {
        Err(())
    }
    pub(super) fn verify(_pid: u32, _port: u16, _ipv6: bool) -> Result<(), ()> {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn proc_addresses_require_exact_loopback_or_wildcard() {
        fn encoded(bytes: &[u8]) -> String {
            use std::fmt::Write;
            let mut encoded = String::new();
            for chunk in bytes.chunks_exact(4) {
                write!(
                    &mut encoded,
                    "{:08X}",
                    u32::from_ne_bytes(chunk.try_into().unwrap())
                )
                .unwrap();
            }
            encoded
        }
        for bytes in [
            std::net::Ipv6Addr::LOCALHOST.octets(),
            std::net::Ipv6Addr::UNSPECIFIED.octets(),
        ] {
            assert_eq!(
                super::proc_address_matches(&encoded(&bytes), true),
                Ok(true)
            );
        }
        assert_eq!(
            super::proc_address_matches(
                &encoded(
                    &"2001:db8::1"
                        .parse::<std::net::Ipv6Addr>()
                        .unwrap()
                        .octets()
                ),
                true
            ),
            Ok(false)
        );
        assert_eq!(
            super::proc_address_matches(&encoded(&[127, 0, 0, 1]), false),
            Ok(true)
        );
        for invalid in [
            "",
            "00000000",
            "Z0000000000000000000000000000000",
            "é0000000000000000000000000000000",
        ] {
            assert!(super::proc_address_matches(invalid, true).is_err());
        }
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[test]
    fn ipv6_listener_requires_the_selected_family_and_live_owner() {
        let socket = std::net::TcpListener::bind("[::1]:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(super::verify_for(std::process::id(), port, true).is_ok());
        assert!(super::verify_for(u32::MAX, port, true).is_err());
        assert!(super::verify_for(std::process::id(), port, false).is_err());
        drop(socket);
        assert!(super::verify_for(std::process::id(), port, true).is_err());
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    #[test]
    fn listener_requires_the_actual_live_owner() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        assert!(super::verify(std::process::id(), port).is_ok());
        assert!(super::verify(u32::MAX, port).is_err());
        drop(socket);
        assert!(super::verify(std::process::id(), port).is_err());
        assert!(super::verify(std::process::id(), 0).is_err());
    }
}
