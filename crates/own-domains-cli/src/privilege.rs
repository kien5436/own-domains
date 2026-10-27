#[cfg(unix)]
use std::ffi::CString;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivilegedIdentity {
    pub user_name: String,
    pub user_id: u32,
}

pub fn validate_privileged_setup_identity(
    apply: bool,
    effective_user_id: u32,
    sudo_user: Option<&str>,
    sudo_uid: Option<&str>,
) -> Result<Option<PrivilegedIdentity>, String> {
    if !apply {
        return Ok(None);
    }
    if effective_user_id != 0 {
        return Err("setup --apply must run as root through sudo".into());
    }
    let user_name = sudo_user
        .filter(|user| {
            !user.is_empty()
                && user
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        })
        .ok_or_else(|| "setup --apply requires a valid SUDO_USER".to_string())?;
    let user_id = sudo_uid
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|user_id| *user_id != 0)
        .ok_or_else(|| "setup --apply requires a non-root numeric SUDO_UID".to_string())?;

    Ok(Some(PrivilegedIdentity {
        user_name: user_name.into(),
        user_id,
    }))
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct UserRecord {
    pub(crate) name: String,
    pub(crate) user_id: u32,
    pub(crate) group_id: u32,
    pub(crate) home: PathBuf,
}

#[cfg(unix)]
pub(crate) fn user_by_uid(user_id: u32) -> Result<UserRecord, String> {
    let record = unsafe { libc::getpwuid(user_id) };
    if record.is_null() {
        return Err(format!("unable to resolve user id {user_id}"));
    }
    let record = unsafe { &*record };
    let name = unsafe { std::ffi::CStr::from_ptr(record.pw_name) }
        .to_str()
        .map_err(|_| "user name is not valid UTF-8".to_string())?
        .to_string();
    let home = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) }
        .to_str()
        .map_err(|_| "user home directory is not valid UTF-8".to_string())?;
    Ok(UserRecord {
        name,
        user_id: record.pw_uid,
        group_id: record.pw_gid,
        home: PathBuf::from(home),
    })
}

#[cfg(unix)]
fn user_by_name(user_name: &str) -> Result<UserRecord, String> {
    let name = CString::new(user_name).map_err(|_| "user name contains a NUL byte".to_string())?;
    let record = unsafe { libc::getpwnam(name.as_ptr()) };
    if record.is_null() {
        return Err(format!("unable to resolve user {user_name}"));
    }
    let record = unsafe { &*record };
    let home = unsafe { std::ffi::CStr::from_ptr(record.pw_dir) }
        .to_str()
        .map_err(|_| "user home directory is not valid UTF-8".to_string())?;
    Ok(UserRecord {
        name: user_name.to_string(),
        user_id: record.pw_uid,
        group_id: record.pw_gid,
        home: PathBuf::from(home),
    })
}

#[cfg(unix)]
pub(crate) fn setup_user(apply: bool) -> Result<(UserRecord, Option<PrivilegedIdentity>), String> {
    let identity = validate_privileged_setup_identity(
        apply,
        unsafe { libc::geteuid() },
        std::env::var("SUDO_USER").ok().as_deref(),
        std::env::var("SUDO_UID").ok().as_deref(),
    )?;
    if let Some(identity) = &identity {
        let user = user_by_name(&identity.user_name)?;
        if user.user_id != identity.user_id {
            return Err("SUDO_UID does not match SUDO_USER".into());
        }
        return Ok((user, Some(identity.clone())));
    }
    if let Ok(name) = std::env::var("SUDO_USER")
        && !name.is_empty()
    {
        return Ok((user_by_name(&name)?, None));
    }
    Ok((user_by_uid(unsafe { libc::geteuid() })?, None))
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub(crate) fn user_by_uid(user_id: u32) -> Result<UserRecord, String> {
    Err(format!(
        "unable to resolve user id {user_id} on this platform"
    ))
}

#[cfg(not(unix))]
pub(crate) fn setup_user(_apply: bool) -> Result<(UserRecord, Option<PrivilegedIdentity>), String> {
    Err("user lookup is not implemented yet on Windows".to_string())
}
