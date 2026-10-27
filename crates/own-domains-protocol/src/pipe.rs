//! Windows named-pipe byte stream and endpoint helpers.
//!
//! The pipe is always byte mode (`PIPE_TYPE_BYTE`), so the shared JSON-line
//! codec in `transport` works unchanged. Every operation runs overlapped with
//! an explicit wait: there is no documented pipe read/write timeout, and
//! `SetCommTimeouts` does not apply to named pipes.

use std::{
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, HANDLE, HLOCAL,
            LPARAM, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        Storage::FileSystem::{ReadFile, WriteFile},
        System::{
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
            Pipes::WaitNamedPipeW,
            Threading::{
                CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent, WaitForSingleObject,
            },
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};

fn win_error(error: windows::core::Error) -> io::Error {
    error.into()
}

/// Current user SID as a string (`S-1-5-...`), for endpoint derivation and DACLs.
pub fn current_user_sid() -> Result<String, String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|error| format!("pipe: OpenProcessToken failed: {error}"))?;
        let sid = (|| {
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            if needed == 0 {
                return Err("pipe: token has no user".to_string());
            }
            // TOKEN_USER requires 8-byte alignment; a u8 buffer does not
            // guarantee it.
            let mut raw = vec![0u64; needed as usize / 8 + 1];
            GetTokenInformation(
                token,
                TokenUser,
                Some(raw.as_mut_ptr() as *mut _),
                needed,
                &mut needed,
            )
            .map_err(|error| format!("pipe: GetTokenInformation failed: {error}"))?;
            let user = &*(raw.as_ptr() as *const TOKEN_USER);
            let mut wide = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut wide)
                .map_err(|error| format!("pipe: SID string failed: {error}"))?;
            let text = String::from_utf16_lossy(wide.as_wide());
            LocalFree(Some(HLOCAL(wide.as_ptr() as _)));
            Ok(text)
        })();
        let _ = CloseHandle(token);
        sid
    }
}

/// Connected pipe handle with per-operation waits.
///
/// Each `Read`/`Write` runs overlapped and waits up to the configured
/// timeout, then cancels. A timed-out read surfaces `TimedOut`, which the
/// event loop treats like the Unix read-timeout retry.
pub struct PipeStream {
    handle: HANDLE,
    event: HANDLE,
    timeout_ms: Option<u32>,
    cancel: Option<Arc<AtomicBool>>,
}

// The handles are owned kernel objects, valid in every thread of the process.
// `PipeStream` is only ever transferred by value (never shared by reference),
// so sending ownership across threads is sound.
unsafe impl Send for PipeStream {}

impl PipeStream {
    pub fn new(handle: HANDLE, timeout: Duration) -> io::Result<Self> {
        let event = unsafe { CreateEventW(None, true, false, None).map_err(win_error)? };
        Ok(Self {
            handle,
            event,
            timeout_ms: Some(timeout.as_millis().min(u32::MAX as u128) as u32),
            cancel: None,
        })
    }

    pub fn handle(&self) -> HANDLE {
        self.handle
    }
}

impl Drop for PipeStream {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.event);
            let _ = CloseHandle(self.handle);
        }
    }
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        unsafe {
            ResetEvent(self.event).map_err(win_error)?;
            let mut overlapped = OVERLAPPED {
                hEvent: self.event,
                ..Default::default()
            };
            let mut done = 0u32;
            match ReadFile(
                self.handle,
                Some(buf),
                Some(&mut done),
                Some(std::ptr::addr_of_mut!(overlapped)),
            ) {
                Ok(()) => Ok(done as usize),
                Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                    self.complete(&overlapped, &mut done)?;
                    Ok(done as usize)
                }
                Err(error) if error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0) => Err(
                    io::Error::new(io::ErrorKind::UnexpectedEof, "peer disconnected"),
                ),
                Err(error) => Err(win_error(error)),
            }
        }
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        unsafe {
            ResetEvent(self.event).map_err(win_error)?;
            let mut overlapped = OVERLAPPED {
                hEvent: self.event,
                ..Default::default()
            };
            let mut done = 0u32;
            match WriteFile(
                self.handle,
                Some(buf),
                Some(&mut done),
                Some(std::ptr::addr_of_mut!(overlapped)),
            ) {
                Ok(()) => Ok(done as usize),
                Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                    self.complete(&overlapped, &mut done)?;
                    Ok(done as usize)
                }
                Err(error) if error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0) => Err(
                    io::Error::new(io::ErrorKind::UnexpectedEof, "peer disconnected"),
                ),
                Err(error) => Err(win_error(error)),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl PipeStream {
    /// Signal this stream's in-flight read to stop at the next 50 ms slice.
    pub fn set_cancel(&mut self, cancel: Arc<AtomicBool>) {
        self.cancel = Some(cancel);
    }

    /// Wait without a deadline, as the Unix blocking stream does. Used for
    /// server-side connections, where an idle subscription must survive.
    /// Client operations keep their own timeouts.
    pub fn set_no_timeout(&mut self) {
        self.timeout_ms = None;
    }

    /// Wait for a pending overlapped operation, polling a cancellation flag.
    ///
    /// The cancelling thread never holds the in-flight `OVERLAPPED` (it is
    /// stack-local to the waiting `read`/`write`), so instead of naming the
    /// operation to `CancelIoEx` across threads, the waiter slices at 50 ms
    /// and checks the shared flag; cancellation lands within one slice.
    fn complete(&self, overlapped: &OVERLAPPED, done: &mut u32) -> io::Result<()> {
        let deadline = self
            .timeout_ms
            .map(|ms| Instant::now() + Duration::from_millis(ms as u64));
        loop {
            if self
                .cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Acquire))
            {
                unsafe {
                    let _ = CancelIoEx(self.handle, Some(overlapped));
                    let _ = GetOverlappedResult(self.handle, overlapped, done, true);
                }
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "pipe operation cancelled",
                ));
            }
            let remaining =
                deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
            if remaining.is_some_and(|remaining| remaining.is_zero()) {
                // The operation may have completed between the deadline check
                // and the cancel: only report a timeout when the drain fails.
                let drained = unsafe {
                    let _ = CancelIoEx(self.handle, Some(overlapped));
                    GetOverlappedResult(self.handle, overlapped, done, true)
                };
                if drained.is_ok() {
                    return Ok(());
                }
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "pipe operation timed out",
                ));
            }
            let slice = remaining
                .map(|remaining| remaining.as_millis().clamp(1, 50) as u32)
                .unwrap_or(50);
            let waited = unsafe { WaitForSingleObject(self.event, slice) };
            if waited == WAIT_OBJECT_0 {
                unsafe {
                    GetOverlappedResult(self.handle, overlapped, done, false).map_err(win_error)?;
                }
                return Ok(());
            } else if waited != WAIT_TIMEOUT {
                unsafe {
                    let _ = CancelIoEx(self.handle, Some(overlapped));
                    let _ = GetOverlappedResult(self.handle, overlapped, done, true);
                }
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "pipe wait failed",
                ));
            }
        }
    }
}

/// Security descriptor of a live pipe as an SDDL string.
///
/// Opens the pipe and reads back what `CreateNamedPipeW` installed, so the
/// DACL audit asserts on the kernel object itself rather than on the string
/// that was requested. The `\\.\pipe\` name form does not resolve for
/// `GetNamedSecurityInfoW` (`ERROR_BAD_PATHNAME`), hence the handle query.
/// The connection itself is never used; the server-side handler it spawns
/// idles until the handle closes.
pub fn pipe_security_sddl(full_name: &str) -> Result<String, String> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_SUCCESS, HLOCAL, LocalFree},
        Security::{
            Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo,
                SDDL_REVISION_1, SE_KERNEL_OBJECT,
            },
            DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR,
        },
        Storage::FileSystem::{CreateFileW, FILE_SHARE_MODE, OPEN_EXISTING, READ_CONTROL},
    };

    let wide: Vec<u16> = full_name.encode_utf16().chain([0]).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            READ_CONTROL.0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
    }
    .map_err(|error| format!("pipe open for audit failed: {error}"))?;
    let info = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut descriptor = PSECURITY_DESCRIPTOR(std::ptr::null_mut());
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            info,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status != ERROR_SUCCESS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err(format!("pipe security query failed: {status:?}"));
    }
    let text = unsafe {
        let mut out = PWSTR::null();
        let result = ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            info,
            &mut out,
            None,
        );
        LocalFree(Some(HLOCAL(descriptor.0 as _)));
        result
            .map(|()| {
                let text = String::from_utf16_lossy(out.as_wide());
                LocalFree(Some(HLOCAL(out.as_ptr() as _)));
                text
            })
            .map_err(|error| format!("pipe SDDL conversion failed: {error}"))?
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    Ok(text)
}

/// Whether a pipe server currently owns `full_name` (`\\.\pipe\...`).
///
/// `WaitNamedPipeW` with a zero timeout waits up to the server default (50 ms: instances use zero): success
/// and pipe-busy/access-denied all mean present; only file-not-found means absent.
pub fn server_present(full_name: &str) -> bool {
    let wide: Vec<u16> = full_name.encode_utf16().chain([0]).collect();
    unsafe {
        if WaitNamedPipeW(PCWSTR(wide.as_ptr()), 0).as_bool() {
            return true;
        }
        windows::core::Error::from_thread().code() != HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0)
    }
}

/// Spec endpoint derivation: `own-domains-pipe-v1` + user SID + canonical
/// state directory, SHA-256, first 16 bytes as lowercase hex.
pub fn default_pipe_name(state_dir: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{
        Globalization::{LCMAP_LOWERCASE, LCMapStringEx, LOCALE_NAME_INVARIANT},
        Storage::FileSystem::GetFullPathNameW,
        UI::Shell::{PATHCCH_ALLOW_LONG_PATHS, PathCchCanonicalizeEx},
    };

    let sid = current_user_sid()?;

    let wide: Vec<u16> = state_dir.as_os_str().encode_wide().chain([0]).collect();
    let mut full = vec![0u16; 32768];
    let full_len =
        unsafe { GetFullPathNameW(PCWSTR(wide.as_ptr()), Some(full.as_mut_slice()), None) };
    if full_len == 0 || full_len as usize + 1 > full.len() {
        return Err("pipe derivation: cannot absolutize state directory".into());
    }
    full.truncate(full_len as usize + 1);
    let mut canon = vec![0u16; 32768];
    unsafe {
        PathCchCanonicalizeEx(
            canon.as_mut_slice(),
            PCWSTR(full.as_ptr()),
            PATHCCH_ALLOW_LONG_PATHS,
        )
    }
    .map_err(|error| format!("pipe derivation: cannot canonicalize state directory: {error}"))?;
    let canon_len = canon
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("pipe derivation: canonical path is unterminated")?;
    canon.truncate(canon_len + 1);
    let need = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_LOWERCASE,
            &canon,
            None,
            None,
            None,
            LPARAM(0),
        )
    };
    if need == 0 {
        return Err("pipe derivation: cannot lowercase state directory".into());
    }
    let mut lower = vec![0u16; need as usize];
    let mapped = unsafe {
        LCMapStringEx(
            LOCALE_NAME_INVARIANT,
            LCMAP_LOWERCASE,
            &canon,
            Some(lower.as_mut_slice()),
            None,
            None,
            LPARAM(0),
        )
    };
    if mapped == 0 {
        return Err("pipe derivation: cannot lowercase state directory".into());
    }
    let end = lower
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(lower.len());
    lower.truncate(end);
    while lower.len() > 3 && matches!(lower.last(), Some(0x5C) | Some(0x2F)) {
        lower.pop();
    }
    let lower_le: Vec<u8> = lower.iter().flat_map(|unit| unit.to_le_bytes()).collect();

    let mut digest = Sha256::new();
    digest.update(b"own-domains-pipe-v1\0");
    digest.update(sid.as_bytes());
    digest.update(b"\0");
    digest.update(&lower_le);
    let hash = digest.finalize();
    let hex: String = hash[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(format!("own-domains-{hex}"))
}

#[cfg(test)]
mod tests {
    use super::default_pipe_name;
    use std::path::PathBuf;

    fn named(state_dir: &str) -> String {
        default_pipe_name(&PathBuf::from(state_dir)).unwrap()
    }

    #[test]
    fn derivation_ignores_case_separators_and_trailing_slashes() {
        let base = std::env::temp_dir().join("od-case-sep");
        let lower = base.to_string_lossy().to_lowercase();
        let upper = base.to_string_lossy().to_uppercase();
        let expected = named(&lower);
        assert!(expected.starts_with("own-domains-"));
        assert_eq!(expected.len(), "own-domains-".len() + 32);
        assert_eq!(named(&upper), expected);
        assert_eq!(named(&lower.replace('\\', "/")), expected);
        assert_eq!(named(&format!("{lower}\\")), expected);
    }

    #[test]
    fn derivation_supports_long_paths() {
        let long: String = "d".repeat(200);
        let dir = std::env::temp_dir().join(format!("od-long-{long}"));
        let first = default_pipe_name(&dir).unwrap();
        assert_eq!(default_pipe_name(&dir).unwrap(), first);
    }
}
