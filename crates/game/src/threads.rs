//! Background threads give way to the main thread.
//!
//! The chunk and far-tree workers and the single threads beside them (far view, crows,
//! scanner, collision, map) can keep every core busy while the camera moves. At the
//! same priority as the main thread, Windows hands them equal turns, and a frame that
//! is pre-empted half built misses its vsync. Below normal, they still have all the
//! time the main thread does not want, but it never waits behind them.

/// Lowers the calling thread to below-normal priority. Call it first thing on every
/// background thread. Does nothing outside Windows.
pub fn background() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        // SAFETY: GetCurrentThread returns a pseudo-handle for this thread that is always
        // valid and needs no closing, and SetThreadPriority only reads it.
        let ok = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) };
        if ok == 0 {
            log::debug!("could not lower the priority of thread {:?}", std::thread::current().name());
        }
    }
}
