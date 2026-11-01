//! Small platform helpers.

#[cfg(windows)]
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThread() -> isize;
    fn SetThreadPriority(thread: isize, priority: i32) -> i32;
}

/// Ask Windows for 1 ms timer resolution. Without this, `thread::sleep(5ms)`
/// can be rounded up to ~15 ms, which shows up as buffer jitter and underruns.
#[cfg(windows)]
pub fn begin_high_res_timer() {
    unsafe {
        timeBeginPeriod(1);
    }
}

#[cfg(not(windows))]
pub fn begin_high_res_timer() {}

/// Raise the current thread slightly above normal so a busy UI or logger does
/// not cause packet/record bursts. `THREAD_PRIORITY_ABOVE_NORMAL` is a
/// deliberately modest boost; the realtime audio callback is driven by WASAPI.
#[cfg(windows)]
pub fn boost_thread_priority() {
    unsafe {
        SetThreadPriority(GetCurrentThread(), 1);
    }
}

#[cfg(not(windows))]
pub fn boost_thread_priority() {}
