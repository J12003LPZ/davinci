//! Darwin's packed TCP PCB snapshot. All reads are bounded and unaligned-safe.
//!
//! Layout reference: Apple XNU bsd/netinet/in_pcb.h and bsd/sys/socketvar.h.
//! Both xsocket_n.xso_so and socket_info.soi_so use VM_KERNEL_ADDRHASH(so).
//! Never use so_last_pid: a historical owner does not prove a live descriptor.

use std::collections::HashSet;
use std::mem::{offset_of, size_of};

#[repr(C, packed(4))]
struct Generation {
    len: u32,
    count: u32,
    generation: u64,
    sockets: u64,
}

#[repr(C, packed(4))]
struct Inpcb {
    len: u32,
    kind: u32,
    handle: u64,
    foreign_port: u16,
    local_port: u16,
    pcb: u64,
    generation: u64,
    flags: i32,
    flow: u32,
    version: u8,
    ttl: u8,
    protocol: u8,
    foreign_address: [u32; 4],
    local_address: [u32; 4],
}

#[repr(C, packed(4))]
struct Socket {
    len: u32,
    kind: u32,
    handle: u64,
    socket_type: i16,
    options: u32,
    linger: i16,
    state: i16,
    pcb: u64,
    protocol: i32,
    family: i32,
}

fn value<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], ()> {
    bytes
        .get(offset..offset.checked_add(N).ok_or(())?)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())
}
fn word(bytes: &[u8], offset: usize) -> Result<u32, ()> {
    Ok(u32::from_ne_bytes(value(bytes, offset)?))
}
fn wide(bytes: &[u8], offset: usize) -> Result<u64, ()> {
    Ok(u64::from_ne_bytes(value(bytes, offset)?))
}
fn record<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    kind: u32,
    minimum: usize,
) -> Result<&'a [u8], ()> {
    let length = word(bytes, *cursor)? as usize;
    if length < minimum || word(bytes, *cursor + 4)? != kind {
        return Err(());
    }
    let end = cursor.checked_add(length).ok_or(())?;
    let result = bytes.get(*cursor..end).ok_or(())?;
    *cursor = cursor
        .checked_add(length.checked_add(7).ok_or(())? & !7)
        .ok_or(())?;
    if *cursor > bytes.len() {
        return Err(());
    }
    Ok(result)
}
fn matching(inp: &[u8], socket: &[u8], port: u16) -> Result<bool, ()> {
    let address = value::<4>(inp, offset_of!(Inpcb, local_address) + 12)?;
    Ok(
        u16::from_be_bytes(value(inp, offset_of!(Inpcb, local_port))?) == port
            && inp[offset_of!(Inpcb, version)] & 1 != 0
            && (address == [0; 4] || address == [127, 0, 0, 1])
            && i16::from_ne_bytes(value(socket, offset_of!(Socket, socket_type))?) == 1
            && word(socket, offset_of!(Socket, options))? & 2 != 0
            && word(socket, offset_of!(Socket, protocol))? == 6
            && word(socket, offset_of!(Socket, family))? == 2,
    )
}
fn listeners(bytes: &[u8], port: u16) -> Result<HashSet<u64>, ()> {
    let header_size = size_of::<Generation>();
    if port == 0 || bytes.len() > 1024 * 1024 || word(bytes, 0)? as usize != header_size {
        return Err(());
    }
    let footer = bytes.len().checked_sub(header_size).ok_or(())?;
    // Exact stable generation/count framing; incomplete or changing snapshots
    // must not leave an unobserved foreign listener out of the proof.
    if footer < header_size || bytes.get(..header_size) != bytes.get(footer..) {
        return Err(());
    }
    let generation = wide(bytes, offset_of!(Generation, generation))?;
    let count = word(bytes, offset_of!(Generation, count))? as usize;
    if count > 4096 {
        return Err(());
    }
    let mut cursor = header_size;
    let mut result = HashSet::new();
    let mut groups = 0;
    while cursor < footer {
        groups += 1;
        if groups > count {
            return Err(());
        }
        // XNU emits one complete group in this order, rounding each record up
        // to eight bytes. Strict grouping avoids joining different sockets.
        let inp = record(bytes, &mut cursor, 16, size_of::<Inpcb>())?;
        let socket = record(bytes, &mut cursor, 1, size_of::<Socket>())?;
        for kind in [2, 4, 8, 32] {
            record(bytes, &mut cursor, kind, 8)?;
        }
        if wide(inp, offset_of!(Inpcb, generation))? > generation {
            return Err(());
        }
        if matching(inp, socket, port)? {
            let handle = wide(socket, offset_of!(Socket, handle))?;
            if handle == 0 || !result.insert(handle) {
                return Err(());
            }
        }
    }
    if cursor != footer || result.is_empty() {
        return Err(());
    }
    Ok(result)
}

#[cfg(target_os = "macos")]
pub(super) fn birth(pid: u32) -> Result<u64, ()> {
    Ok(native::identity(pid)?.1)
}

#[cfg(target_os = "macos")]
pub(super) fn verify(root: u32, port: u16) -> Result<(), ()> {
    native::verify(root, port)
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::mem::size_of_val;

    #[repr(C)]
    struct FileInfo {
        open_flags: u32,
        status: u32,
        offset: i64,
        file_type: i32,
        guard_flags: u32,
    }
    #[repr(C)]
    struct Stat {
        device: u32,
        mode: u16,
        links: u16,
        inode: u64,
        uid: u32,
        gid: u32,
        times: [i64; 8],
        size: i64,
        blocks: i64,
        block_size: i32,
        flags: u32,
        generation: u32,
        rdev: u32,
        spare: [i64; 2],
    }
    #[repr(C)]
    struct DescriptorPrefix {
        file: FileInfo,
        stat: Stat,
        handle: u64,
    }
    const SOCKET_HANDLE: usize = offset_of!(DescriptorPrefix, handle);

    pub(super) fn identity(pid: u32) -> Result<(u32, u64), ()> {
        let pid = i32::try_from(pid).map_err(|_| ())?;
        // SAFETY: proc_bsdinfo consists entirely of integer/character fields.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        // SAFETY: correctly aligned/sized native output buffer, query-only API.
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut info as *mut libc::proc_bsdinfo).cast(),
                size_of::<libc::proc_bsdinfo>() as i32,
            )
        };
        if count != size_of::<libc::proc_bsdinfo>() as i32
            || info.pbi_pid != pid as u32
            || info.pbi_status == 5
            || info.pbi_start_tvusec >= 1_000_000
        {
            return Err(());
        }
        let birth = info
            .pbi_start_tvsec
            .checked_mul(1_000_000)
            .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
            .ok_or(())?;
        Ok((info.pbi_ppid, birth))
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
    fn snapshot() -> Result<Vec<u8>, ()> {
        let name = c"net.inet.tcp.pcblist_n";
        for _ in 0..2 {
            let mut size = 0;
            // SAFETY: static NUL-terminated name, valid size pointer, null output
            // requests size only; no replacement sysctl value is supplied.
            if unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            } != 0
                || !(48..=1024 * 1024).contains(&size)
            {
                return Err(());
            }
            let mut bytes = vec![0_u8; size];
            // SAFETY: byte output buffer holds size writable bytes. The kernel
            // copies packed records; our parser never dereferences typed fields.
            let status = unsafe {
                libc::sysctlbyname(
                    name.as_ptr(),
                    bytes.as_mut_ptr().cast(),
                    &mut size,
                    std::ptr::null_mut(),
                    0,
                )
            };
            if status == 0 && size <= bytes.len() {
                bytes.truncate(size);
                return Ok(bytes);
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
                return Err(());
            }
        }
        Err(())
    }
    fn descriptors(pid: i32, remaining: usize) -> Result<Option<Vec<libc::proc_fdinfo>>, ()> {
        // SAFETY: documented size-only query, no output buffer is dereferenced.
        let size =
            unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
        if size == 0 {
            // Other users' descriptors can be inaccessible. Their listener
            // hashes still occur in the global table and require ownership.
            return Ok(None);
        }
        if size < 0 || size as usize > remaining * size_of::<libc::proc_fdinfo>() {
            return Err(());
        }
        let capacity = (size as usize).div_ceil(size_of::<libc::proc_fdinfo>()) + 1;
        let mut fds: Vec<_> = (0..capacity)
            .map(|_| libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0,
            })
            .collect();
        let size = fds.len() * size_of::<libc::proc_fdinfo>();
        // SAFETY: aligned output array allocated for the advertised byte count.
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                size as i32,
            )
        };
        // A full buffer may have silently omitted descriptors. Fail closed.
        if count == 0 {
            return Ok(None);
        }
        if count < 0
            || count as usize >= size
            || count as usize % size_of::<libc::proc_fdinfo>() != 0
        {
            return Err(());
        }
        fds.truncate(count as usize / size_of::<libc::proc_fdinfo>());
        Ok(Some(fds))
    }
    fn socket(pid: i32, fd: i32) -> Result<u64, ()> {
        // Larger than the current socket_fdinfo ABI; future growth beyond the
        // cap fails closed. proc_pidfdinfo accepts a buffer at least its size.
        let mut buffer = [0_u64; 512];
        // SAFETY: live aligned 4096-byte native output storage, socket flavor 3.
        let count = unsafe {
            libc::proc_pidfdinfo(
                pid,
                fd,
                3,
                buffer.as_mut_ptr().cast(),
                size_of_val(&buffer) as i32,
            )
        };
        if count < (SOCKET_HANDLE + 8) as i32 || count as usize > size_of_val(&buffer) {
            return Err(());
        }
        // SAFETY: initialized u64 storage exposed as bytes only within the
        // returned native byte count; no native pointers are dereferenced.
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast(), count as usize) };
        wide(bytes, SOCKET_HANDLE)
    }
    pub(super) fn verify(root: u32, port: u16) -> Result<(), ()> {
        let birth = identity(root)?.1;
        let handles = listeners(&snapshot()?, port)?;
        let mut pids = [0_i32; 4097];
        // SAFETY: writable aligned PID array; libproc returns the PID count,
        // unlike proc_pidinfo, which returns bytes.
        let count =
            unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size_of_val(&pids) as i32) };
        if count <= 0 || count as usize >= pids.len() {
            return Err(());
        }
        let mut owned = HashSet::new();
        let mut total = 0;
        for &pid in &pids[..count as usize] {
            if pid <= 0 {
                continue;
            }
            let Ok(before) = identity(pid as u32) else {
                continue;
            };
            let Some(fds) = descriptors(pid, 65536 - total)? else {
                continue;
            };
            total += fds.len();
            if total > 65536 {
                return Err(());
            }
            for fd in fds {
                if fd.proc_fdtype != libc::PROX_FDTYPE_SOCKET as u32 {
                    continue;
                }
                let Ok(handle) = socket(pid, fd.proc_fd) else {
                    continue;
                };
                if handles.contains(&handle) {
                    descendant(pid as u32, root)?;
                    if identity(pid as u32)? != before {
                        return Err(());
                    }
                    owned.insert(handle);
                }
            }
        }
        if owned != handles || identity(root)?.1 != birth {
            return Err(());
        }
        // Re-read the endpoint after descriptor inspection: a distinct listener
        // must not take over the port during this bounded observation.
        if listeners(&snapshot()?, port)? != handles {
            return Err(());
        }
        Ok(())
    }

    #[cfg(test)]
    #[test]
    fn layouts_match_the_installed_darwin_sdk() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("socket-layout.c");
        std::fs::write(&source, format!(r#"
#define PRIVATE 1
#include <stddef.h>
#include <sys/socket.h>
#include <sys/socketvar.h>
#include <netinet/in.h>
#include <netinet/in_pcb.h>
#include <sys/proc_info.h>
_Static_assert(sizeof(struct xinpgen) == {generation_size}, "generation size");
_Static_assert(offsetof(struct xinpcb_n, inp_lport) == {port}, "port offset");
_Static_assert(offsetof(struct xinpcb_n, inp_gencnt) == {generation}, "generation offset");
_Static_assert(offsetof(struct xinpcb_n, inp_vflag) == {version}, "version offset");
_Static_assert(offsetof(struct xinpcb_n, inp_dependladdr) == {address}, "address offset");
_Static_assert(offsetof(struct xsocket_n, xso_so) == {handle}, "socket handle offset");
_Static_assert(offsetof(struct xsocket_n, so_type) == {socket_type}, "socket type offset");
_Static_assert(offsetof(struct xsocket_n, so_options) == {options}, "options offset");
_Static_assert(offsetof(struct xsocket_n, xso_protocol) == {protocol}, "protocol offset");
_Static_assert(offsetof(struct xsocket_n, xso_family) == {family}, "family offset");
_Static_assert(offsetof(struct socket_fdinfo, psi) + offsetof(struct socket_info, soi_so) == {descriptor}, "descriptor handle offset");
_Static_assert(AF_INET == 2 && SOCK_STREAM == 1 && IPPROTO_TCP == 6 && SO_ACCEPTCONN == 2, "listener constants");
_Static_assert(PROC_PIDFDSOCKETINFO == 3 && sizeof(struct socket_fdinfo) <= 4096, "descriptor ABI cap");
int main(void) {{ return 0; }}
"#, generation_size=size_of::<Generation>(), port=offset_of!(Inpcb,local_port), generation=offset_of!(Inpcb,generation), version=offset_of!(Inpcb,version), address=offset_of!(Inpcb,local_address), handle=offset_of!(Socket,handle), socket_type=offset_of!(Socket,socket_type), options=offset_of!(Socket,options), protocol=offset_of!(Socket,protocol), family=offset_of!(Socket,family), descriptor=SOCKET_HANDLE)).unwrap();
        let result = std::process::Command::new("/usr/bin/cc")
            .args(["-std=c11", "-fsyntax-only"])
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[cfg(test)]
    fn reuseport_listener(port: u16) -> std::net::TcpListener {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        // SAFETY: documented IPv4 TCP socket request, without borrowed pointers.
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0, "{}", std::io::Error::last_os_error());
        // SAFETY: socket returned a new, valid descriptor, transferred once.
        let socket = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: set close-on-exec on our new descriptor before spawning the
        // helper, so it cannot inherit the parent's distinct listener.
        assert_eq!(
            unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
        let enabled = 1_i32;
        // SAFETY: live socket and correctly sized integer option storage.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_REUSEPORT,
                    (&enabled as *const i32).cast(),
                    size_of_val(&enabled) as libc::socklen_t,
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        let address = libc::sockaddr_in {
            sin_len: size_of::<libc::sockaddr_in>() as u8,
            sin_family: libc::AF_INET as u8,
            sin_port: port.to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from_ne_bytes([127, 0, 0, 1]),
            },
            sin_zero: [0; 8],
        };
        // SAFETY: live socket and correctly sized native IPv4 address storage.
        assert_eq!(
            unsafe {
                libc::bind(
                    socket.as_raw_fd(),
                    (&address as *const libc::sockaddr_in).cast(),
                    size_of_val(&address) as libc::socklen_t,
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        // SAFETY: listen only operates on our live TCP socket descriptor.
        assert_eq!(unsafe { libc::listen(socket.as_raw_fd(), 8) }, 0);
        socket.into()
    }

    #[cfg(test)]
    #[test]
    fn reuseport_helper() {
        let Ok(port) = std::env::var("DAVINCI_SOCKET_REUSEPORT_TEST") else {
            return;
        };
        let socket = reuseport_listener(port.parse().unwrap());
        println!("SOCKET_PORT={}", socket.local_addr().unwrap().port());
        // Parent owns kill/wait cleanup; this deadline also bounds an orphaned
        // test fixture if the parent itself terminates unexpectedly.
        std::thread::sleep(std::time::Duration::from_secs(30));
        drop(socket);
    }

    #[cfg(test)]
    #[test]
    fn foreign_reuseport_listener_is_not_hidden_by_an_owned_listener() {
        use std::io::BufRead;
        let parent_listener = reuseport_listener(0);
        let port = parent_listener.local_addr().unwrap().port();
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut child = Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "process_manager::socket_owner::macos::native::reuseport_helper",
                    "--nocapture",
                ])
                .env("DAVINCI_SOCKET_REUSEPORT_TEST", port.to_string())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = child.0.stdout.take().unwrap();
        let (send, recv) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().take(128) {
                let Ok(line) = line else { break };
                if line == format!("SOCKET_PORT={port}") {
                    let _ = send.send(());
                    break;
                }
            }
        });
        let ready = recv.recv_timeout(std::time::Duration::from_secs(10));
        if ready.is_err() {
            drop(child);
            reader.join().unwrap();
            panic!("reuse-port child did not publish a live listener");
        }
        reader.join().unwrap();
        // Both sockets are in this process's owned tree.
        assert!(verify(std::process::id(), port).is_ok());
        // The parent socket is foreign to the child. Its distinct identity
        // cannot be hidden by the child's genuine descriptor on the same port.
        assert!(verify(child.0.id(), port).is_err());
        drop(parent_listener);
        assert!(verify(child.0.id(), port).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }
    fn record(kind: u32, size: usize) -> Vec<u8> {
        let mut bytes = vec![0; size.next_multiple_of(8)];
        put(&mut bytes, 0, (size as u32).to_ne_bytes());
        put(&mut bytes, 4, kind.to_ne_bytes());
        bytes
    }
    fn generation() -> Vec<u8> {
        let mut bytes = vec![0; size_of::<Generation>()];
        put(
            &mut bytes,
            0,
            (size_of::<Generation>() as u32).to_ne_bytes(),
        );
        put(&mut bytes, 4, 1_u32.to_ne_bytes());
        put(&mut bytes, 8, 10_u64.to_ne_bytes());
        put(&mut bytes, 16, 12_u64.to_ne_bytes());
        bytes
    }
    fn group(handle: u64, address: [u8; 4], port: u16) -> Vec<u8> {
        let mut inp = record(16, size_of::<Inpcb>());
        put(&mut inp, offset_of!(Inpcb, local_port), port.to_be_bytes());
        put(&mut inp, offset_of!(Inpcb, generation), 9_u64.to_ne_bytes());
        inp[offset_of!(Inpcb, version)] = 1;
        put(&mut inp, offset_of!(Inpcb, local_address) + 12, address);
        let mut socket = record(1, size_of::<Socket>());
        put(
            &mut socket,
            offset_of!(Socket, handle),
            handle.to_ne_bytes(),
        );
        put(
            &mut socket,
            offset_of!(Socket, socket_type),
            1_i16.to_ne_bytes(),
        );
        put(
            &mut socket,
            offset_of!(Socket, options),
            2_u32.to_ne_bytes(),
        );
        put(
            &mut socket,
            offset_of!(Socket, protocol),
            6_i32.to_ne_bytes(),
        );
        put(&mut socket, offset_of!(Socket, family), 2_i32.to_ne_bytes());
        inp.extend(socket);
        for kind in [2, 4, 8, 32] {
            inp.extend(record(kind, 32));
        }
        inp
    }
    fn snapshot(groups: &[Vec<u8>]) -> Vec<u8> {
        let mut framing = generation();
        put(&mut framing, 4, (groups.len() as u32).to_ne_bytes());
        let mut bytes = framing.clone();
        for group in groups {
            bytes.extend(group);
        }
        bytes.extend(framing);
        bytes
    }

    #[test]
    fn collects_every_matching_listener_and_filters_other_endpoints() {
        let bytes = snapshot(&[
            group(91, [127, 0, 0, 1], 4321),
            group(92, [0; 4], 4321),
            group(93, [127, 0, 0, 1], 4322),
            group(94, [192, 0, 2, 1], 4321),
        ]);
        assert_eq!(listeners(&bytes, 4321), Ok(HashSet::from([91, 92])));
        assert!(listeners(&bytes, 0).is_err());
        assert!(listeners(&snapshot(&[]), 4321).is_err());
    }

    #[test]
    fn incomplete_and_ambiguous_snapshots_never_prove_ownership() {
        let valid = snapshot(&[group(91, [127, 0, 0, 1], 4321)]);
        for length in 0..valid.len() {
            assert!(listeners(&valid[..length], 4321).is_err(), "{length}");
        }
        let mut extra = valid.clone();
        extra.push(0);
        assert!(listeners(&extra, 4321).is_err());
        let mut changed = valid.clone();
        let footer = changed.len() - size_of::<Generation>();
        put(&mut changed, footer + 8, 11_u64.to_ne_bytes());
        assert!(listeners(&changed, 4321).is_err());
        let mut impossible_count = valid.clone();
        put(&mut impossible_count, 4, 0_u32.to_ne_bytes());
        put(&mut impossible_count, footer + 4, 0_u32.to_ne_bytes());
        assert!(listeners(&impossible_count, 4321).is_err());
        let mut future = valid.clone();
        put(
            &mut future,
            size_of::<Generation>() + offset_of!(Inpcb, generation),
            11_u64.to_ne_bytes(),
        );
        assert!(listeners(&future, 4321).is_err());
        let mut unknown = valid.clone();
        put(
            &mut unknown,
            size_of::<Generation>() + 4,
            64_u32.to_ne_bytes(),
        );
        assert!(listeners(&unknown, 4321).is_err());
        let mut duplicate = valid.clone();
        let socket = size_of::<Generation>() + size_of::<Inpcb>().next_multiple_of(8);
        put(&mut duplicate, socket + 4, 16_u32.to_ne_bytes());
        assert!(listeners(&duplicate, 4321).is_err());
        assert!(listeners(&snapshot(&[group(0, [127, 0, 0, 1], 4321)]), 4321).is_err());
        assert!(listeners(
            &snapshot(&[
                group(91, [127, 0, 0, 1], 4321),
                group(91, [127, 0, 0, 1], 4321),
            ]),
            4321
        )
        .is_err());
    }

    #[test]
    fn only_ipv4_tcp_streams_in_listen_state_match() {
        let valid = snapshot(&[group(91, [127, 0, 0, 1], 4321)]);
        let socket = size_of::<Generation>() + size_of::<Inpcb>().next_multiple_of(8);
        for (offset, value) in [
            (socket + offset_of!(Socket, options), 0_u32),
            (socket + offset_of!(Socket, protocol), 17),
            (socket + offset_of!(Socket, family), 30),
        ] {
            let mut bytes = valid.clone();
            put(&mut bytes, offset, value.to_ne_bytes());
            assert!(listeners(&bytes, 4321).is_err());
        }
        let mut datagram = valid.clone();
        put(
            &mut datagram,
            socket + offset_of!(Socket, socket_type),
            2_i16.to_ne_bytes(),
        );
        assert!(listeners(&datagram, 4321).is_err());
        let mut ipv6 = valid;
        ipv6[size_of::<Generation>() + offset_of!(Inpcb, version)] = 2;
        assert!(listeners(&ipv6, 4321).is_err());
    }
}
