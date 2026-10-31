//! The wall clock in the user's own time zone, for labels such as the
//! `done 10:03 PM` on a finished turn. Nothing here measures a duration.

/// `h:mm AM/PM` for a 24-hour `hour` and a `minute`: `0:05` is `12:05 AM`,
/// `13:30` is `1:30 PM`.
pub fn clock_label(hour: u32, minute: u32) -> String {
    let twelve = match hour % 12 {
        0 => 12,
        hour => hour,
    };
    let half = if hour % 24 < 12 { "AM" } else { "PM" };
    format!("{twelve}:{:02} {half}", minute % 60)
}

/// The local time of day as `(hour, minute)`, or `None` when the system
/// will not say.
pub fn local_time_of_day() -> Option<(u32, u32)> {
    imp::local_time_of_day()
}

/// [`local_time_of_day`] as [`clock_label`] writes it.
pub fn local_clock_label() -> Option<String> {
    local_time_of_day().map(|(hour, minute)| clock_label(hour, minute))
}

#[cfg(unix)]
mod imp {
    pub fn local_time_of_day() -> Option<(u32, u32)> {
        // SAFETY: `time` accepts a null pointer, and `localtime_r` writes only
        // into the `tm` it is given.
        unsafe {
            let now = libc::time(std::ptr::null_mut());
            let mut parts: libc::tm = std::mem::zeroed();
            if libc::localtime_r(&now, &mut parts).is_null() {
                return None;
            }
            Some((
                u32::try_from(parts.tm_hour).ok()?,
                u32::try_from(parts.tm_min).ok()?,
            ))
        }
    }
}

#[cfg(windows)]
mod imp {
    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }

    pub fn local_time_of_day() -> Option<(u32, u32)> {
        let mut time = SystemTime {
            year: 0,
            month: 0,
            day_of_week: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            milliseconds: 0,
        };
        // SAFETY: `GetLocalTime` fills the struct it is given and cannot fail.
        unsafe { GetLocalTime(&mut time) };
        Some((u32::from(time.hour), u32::from(time.minute)))
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn local_time_of_day() -> Option<(u32, u32)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_uses_a_twelve_hour_clock() {
        assert_eq!(clock_label(0, 5), "12:05 AM");
        assert_eq!(clock_label(9, 0), "9:00 AM");
        assert_eq!(clock_label(12, 0), "12:00 PM");
        assert_eq!(clock_label(13, 30), "1:30 PM");
        assert_eq!(clock_label(22, 3), "10:03 PM");
        assert_eq!(clock_label(23, 59), "11:59 PM");
    }

    #[test]
    fn the_local_clock_reports_a_real_time_of_day() {
        let (hour, minute) = local_time_of_day().expect("a local clock");
        assert!(hour < 24 && minute < 60);
        let label = local_clock_label().expect("a label");
        assert!(label.ends_with("AM") || label.ends_with("PM"), "{label}");
    }
}
