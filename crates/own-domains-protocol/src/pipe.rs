//! Windows named-pipe byte stream and endpoint helpers.
//!
//! The pipe is always byte mode (`PIPE_TYPE_BYTE`), so the shared JSON-line
//! codec in `transport` works unchanged. Every operation runs overlapped with
//! an explicit wait: there is no documented pipe read/write timeout, and
//! `SetCommTimeouts` does not apply to named pipes.

use std::{
    io::{self, Read, Write},
    path::Path,
    time::Duration,
};
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, HANDLE, HLOCAL, LPARAM, LocalFree,
            WAIT_OBJECT_0, WAIT_TIMEOUT,
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
            let mut raw = vec![0u8; needed as usize];
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
    timeout_ms: u32,
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
            timeout_ms: timeout.as_millis().min(u32::MAX as u128) as u32,
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
            let mut overlapped = OVERLAPPED::default();
            overlapped.hEvent = self.event;
            let mut done = 0u32;
            match ReadFile(
                self.handle,
                Some(buf),
                Some(&mut done),
                Some(std::ptr::addr_of_mut!(overlapped)),
            ) {
                Ok(()) => Ok(done as usize),
                Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                    let waited = WaitForSingleObject(self.event, self.timeout_ms);
                    if waited == WAIT_OBJECT_0 {
                        GetOverlappedResult(self.handle, &overlapped, &mut done, false)
                            .map_err(win_error)?;
                        Ok(done as usize)
                    } else if waited == WAIT_TIMEOUT {
                        let _ = CancelIoEx(self.handle, Some(&overlapped));
                        let _ = GetOverlappedResult(self.handle, &overlapped, &mut done, true);
                        Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "pipe read timed out",
                        ))
                    } else {
                        let _ = CancelIoEx(self.handle, Some(&overlapped));
                        Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "pipe wait failed",
                        ))
                    }
                }
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
            let mut overlapped = OVERLAPPED::default();
            overlapped.hEvent = self.event;
            let mut done = 0u32;
            match WriteFile(
                self.handle,
                Some(buf),
                Some(&mut done),
                Some(std::ptr::addr_of_mut!(overlapped)),
            ) {
                Ok(()) => Ok(done as usize),
                Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                    let waited = WaitForSingleObject(self.event, self.timeout_ms);
                    if waited == WAIT_OBJECT_0 {
                        GetOverlappedResult(self.handle, &overlapped, &mut done, false)
                            .map_err(win_error)?;
                        Ok(done as usize)
                    } else if waited == WAIT_TIMEOUT {
                        let _ = CancelIoEx(self.handle, Some(&overlapped));
                        let _ = GetOverlappedResult(self.handle, &overlapped, &mut done, true);
                        Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "pipe write timed out",
                        ))
                    } else {
                        let _ = CancelIoEx(self.handle, Some(&overlapped));
                        Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "pipe wait failed",
                        ))
                    }
                }
                Err(error) => Err(win_error(error)),
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Whether a pipe server currently owns `full_name` (`\\.\pipe\...`).
///
/// `WaitNamedPipeW` with a zero timeout never blocks: success and
/// pipe-busy/access-denied all mean present; only file-not-found means absent.
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
        UI::Shell::{PATHCCH_OPTIONS, PathCchCanonicalizeEx},
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
            PATHCCH_OPTIONS(0),
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
