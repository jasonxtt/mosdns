//! The local calendar date, as `time.Now().Format("2006-01-02")` prints it.
//!
//! `time`'s local-offset lookup refuses to run once the process has more than
//! one thread, so the C library's `localtime_r` is called directly.
#![allow(unsafe_code)]

/// `YYYY-MM-DD` in the process's local time zone (UTC where there is no
/// `localtime_r`).
#[must_use]
pub(crate) fn local_date() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0));
    let (year, month, day) = civil(seconds);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(unix)]
fn civil(seconds: i64) -> (i32, u32, u32) {
    use std::ffi::{c_char, c_int, c_long};

    #[repr(C)]
    struct Tm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
    }
    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }
    let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
    // SAFETY: `time` and `tm` are valid for the call; `localtime_r` is
    // thread-safe and fills `tm` when it returns non-null.
    let filled = unsafe { localtime_r(&raw const seconds, tm.as_mut_ptr()) };
    if filled.is_null() {
        return utc(seconds);
    }
    // SAFETY: `localtime_r` initialised the structure.
    let tm = unsafe { tm.assume_init() };
    (
        tm.tm_year + 1900,
        u32::try_from(tm.tm_mon + 1).unwrap_or(1),
        u32::try_from(tm.tm_mday).unwrap_or(1),
    )
}

#[cfg(not(unix))]
fn civil(seconds: i64) -> (i32, u32, u32) {
    utc(seconds)
}

fn utc(seconds: i64) -> (i32, u32, u32) {
    time::OffsetDateTime::from_unix_timestamp(seconds).map_or((1970, 1, 1), |moment| {
        (
            moment.year(),
            u32::from(u8::from(moment.month())),
            u32::from(moment.day()),
        )
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_date_has_the_reference_layout() {
        let date = super::local_date();
        let parts: Vec<&str> = date.split('-').collect();
        assert_eq!(parts.len(), 3, "{date}");
        assert_eq!(parts[0].len(), 4);
        assert!(
            parts
                .iter()
                .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
        );
    }

    #[test]
    fn utc_conversion_is_exact() {
        assert_eq!(super::utc(0), (1970, 1, 1));
        assert_eq!(super::utc(1_791_417_600), (2026, 10, 8));
    }
}
