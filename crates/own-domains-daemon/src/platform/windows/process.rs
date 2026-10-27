//! Windows child-process supervision primitives.
//!
//! Processes start with `CREATE_NO_WINDOW` (no console flash from GUI
//! launches) and are terminated explicitly; Job Object supervision lands with
//! Gate D hardware. Handles close on drop.

use std::{io, time::Duration};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0},
        System::Threading::{
            CREATE_NO_WINDOW, CreateProcessW, GetExitCodeProcess, GetProcessId,
            PROCESS_INFORMATION, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
        },
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible},
    },
    core::{PCWSTR, PWSTR},
};

/// A spawned child process. Closing the handle does not terminate the child;
/// use [`WindowsChild::terminate`].
pub struct WindowsChild {
    handle: HANDLE,
    thread: HANDLE,
    pid: u32,
}

unsafe impl Send for WindowsChild {}

impl Drop for WindowsChild {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.thread);
            let _ = CloseHandle(self.handle);
        }
    }
}

impl WindowsChild {
    /// Spawn `command_line` with no console window.
    pub fn spawn_no_window(command_line: &str) -> io::Result<Self> {
        let mut wide: Vec<u16> = command_line.encode_utf16().chain([0]).collect();
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let startup = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(wide.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW,
                None,
                PCWSTR::null(),
                &startup,
                &mut info,
            )
            .map_err(|error| io::Error::other(format!("cannot launch child process: {error}")))?;
        }
        let pid = unsafe { GetProcessId(info.hProcess) };
        Ok(Self {
            handle: info.hProcess,
            thread: info.hThread,
            pid,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Exit code if the child already finished, `None` while it runs.
    pub fn try_wait(&self) -> io::Result<Option<u32>> {
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code) }
            .map_err(|error| io::Error::other(format!("cannot read child exit code: {error}")))?;
        if code == 259 {
            Ok(None)
        } else {
            Ok(Some(code))
        }
    }

    /// Wait up to `timeout` for the child to exit.
    pub fn wait_timeout(&self, timeout: Duration) -> io::Result<u32> {
        let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
        if unsafe { WaitForSingleObject(self.handle, millis) } != WAIT_OBJECT_0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "child did not exit in time",
            ));
        }
        self.try_wait()?
            .ok_or_else(|| io::Error::other("child exit code missing"))
    }

    /// Force-terminate the child. Returns once the kill is issued.
    pub fn terminate(&self) -> io::Result<()> {
        unsafe { TerminateProcess(self.handle, 1) }
            .map_err(|error| io::Error::other(format!("cannot terminate child process: {error}")))
    }
}

unsafe extern "system" fn find_visible_window(hwnd: HWND, ctx: LPARAM) -> windows::core::BOOL {
    use windows::Win32::Foundation::{FALSE, TRUE};
    let found = unsafe { &mut *(ctx.0 as *mut (u32, bool)) };
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == found.0 && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        found.1 = true;
        FALSE
    } else {
        TRUE
    }
}

/// Whether the process currently owns any visible top-level window.
pub fn any_visible_top_window(pid: u32) -> bool {
    let mut found = (pid, false);
    let ctx = &mut found as *mut (u32, bool);
    unsafe {
        let _ = EnumWindows(Some(find_visible_window), LPARAM(ctx as isize));
    }
    found.1
}
