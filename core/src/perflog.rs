//! Minimal millisecond-resolution perf logger that writes straight to
//! Android logcat (or stderr off-Android) without pulling in `log` +
//! `android_logger` crates. Used to diagnose where time goes between an
//! FFI dispatch, the core thread processing, and the UI reconcile.

use std::time::{SystemTime, UNIX_EPOCH};

#[inline]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(target_os = "android")]
mod sink {
    use std::ffi::CString;

    // ANDROID_LOG_INFO = 4
    const ANDROID_LOG_INFO: i32 = 4;

    extern "C" {
        fn __android_log_write(prio: i32, tag: *const u8, msg: *const u8) -> i32;
    }

    pub fn write(tag: &str, msg: &str) {
        // CString allocation is cheap; instrumentation hot path is rare.
        if let (Ok(t), Ok(m)) = (CString::new(tag), CString::new(msg)) {
            unsafe {
                __android_log_write(
                    ANDROID_LOG_INFO,
                    t.as_ptr() as *const u8,
                    m.as_ptr() as *const u8,
                );
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
mod sink {
    pub fn write(tag: &str, msg: &str) {
        eprintln!("{tag}: {msg}");
    }
}

/// `IrisPerf` tag — `adb logcat IrisPerf:I *:S` shows only these.
#[macro_export]
macro_rules! perflog {
    ($($arg:tt)*) => {
        $crate::perflog::__write(format_args!($($arg)*))
    };
}

#[doc(hidden)]
pub fn __write(args: std::fmt::Arguments<'_>) {
    let msg = format!("{} {}", now_ms(), args);
    sink::write("IrisPerf", &msg);
}
