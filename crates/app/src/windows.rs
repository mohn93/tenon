//! The console on Windows. Every `unsafe` Windows call in this crate lives here; owner-only
//! files and folders come from `session::windows`.

use std::ptr;

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Console::{
    FreeConsole, GetConsoleMode, GetConsoleProcessList, GetStdHandle, STD_ERROR_HANDLE,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
};

/// Lets go of the console when this process is the only one on it: the app is a console
/// program, so started from Explorer it gets a console window of its own, which would stay
/// open beside the app's window. Started from a terminal the console is shared with the shell
/// and kept, so `--headless --stdio` and `--mcp` (which never call this) and the window's log
/// lines still reach it. Standard handles that were the console are cleared, so later writes
/// to them go nowhere; ones redirected elsewhere are kept.
pub fn hide_own_console() {
    let mut ids = [0u32; 2];
    // SAFETY: `ids` has room for the count passed.
    let count = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
    if count != 1 {
        return;
    }
    let console: Vec<_> = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
        .into_iter()
        .filter(|&which| {
            // SAFETY: asks for a standard handle; null or invalid when there is none.
            let handle = unsafe { GetStdHandle(which) };
            let mut mode = 0;
            // SAFETY: `handle` is checked first; `mode` receives the console mode, and the call
            // fails for a handle that is not a console.
            !handle.is_null()
                && handle != INVALID_HANDLE_VALUE
                && unsafe { GetConsoleMode(handle, &mut mode) } != 0
        })
        .collect();
    // SAFETY: detaches this process from its console; nothing else here holds it.
    if unsafe { FreeConsole() } == 0 {
        return;
    }
    for which in console {
        // SAFETY: a null handle means none; the standard library then drops what is written.
        unsafe { SetStdHandle(which, ptr::null_mut()) };
    }
}
