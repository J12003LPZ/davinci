//! Membership is established in the helper before it can spawn a command.
use std::{io, process::Child};

#[cfg(unix)]
pub(super) struct Ownership;

#[cfg(unix)]
impl Ownership {
    pub fn enter() -> io::Result<Self> {
        // This is a fresh, dedicated helper, before any threads or children.
        if unsafe { libc::setsid() } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
    pub fn terminate(&self) -> ! {
        // Our still-live leader pins this group ID. Never target a stored PID
        // after the leader has been reaped.
        unsafe {
            libc::kill(-libc::getpid(), libc::SIGKILL);
            libc::_exit(1);
        }
    }
}

/// The caller exclusively owns an UNREAPED child; this is not a PID lookup API.
pub(super) fn terminate_owned(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    // On Windows the helper's last job handle closes when its process dies.
    let _ = child.kill();
}

#[cfg(windows)]
pub(super) use windows::Ownership;

#[cfg(windows)]
mod windows {
    use super::io;
    use std::{
        ffi::c_void,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_processes: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io_counters: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
    }

    pub struct Ownership(OwnedHandle);
    impl Ownership {
        pub fn enter() -> io::Result<Self> {
            // Unnamed and non-inheritable: only this helper owns the handle.
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let owned = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
            let limits = ExtendedLimits {
                basic: BasicLimits {
                    flags: 0x2000,
                    ..Default::default()
                }, // KILL_ON_JOB_CLOSE
                ..Default::default()
            };
            if unsafe {
                SetInformationJobObject(
                    raw,
                    9,
                    (&limits as *const ExtendedLimits).cast(),
                    std::mem::size_of::<ExtendedLimits>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            // Joining before spawn eliminates the child-spawn/assignment race.
            // Descendants inherit membership; no breakaway flag is enabled.
            if unsafe { AssignProcessToJobObject(raw, GetCurrentProcess()) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(owned)
        }
        pub fn terminate(&self) -> ! {
            unsafe {
                TerminateJobObject(self.0.as_raw_handle(), 1);
            }
            std::process::exit(1)
        }
    }
}
