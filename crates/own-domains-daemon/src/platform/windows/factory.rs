use super::{
    SystemProcessHost, SystemReadiness, SystemSetup, WindowsAdapters, WindowsRuntimeBackend,
    resolve_paths,
};
use crate::{RuntimeBackend, UnavailableRuntimeBackend, probe::CaddyAdminClient};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) fn runtime_backend(
    state_dir: &Path,
    resources: Option<PathBuf>,
    sidecars: Option<PathBuf>,
    settings_only: bool,
) -> Result<Arc<dyn RuntimeBackend>, Box<dyn std::error::Error>> {
    if settings_only {
        return Ok(Arc::new(UnavailableRuntimeBackend));
    }
    let current_dir = std::env::current_dir()?;
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or("the daemon executable has no parent directory")?
        .to_path_buf();
    // Installed bundle layout (NSIS): sidecars live under
    // `<exe-dir>/sidecars/<triple>/` with triple-infixed names. Derive that
    // automatically so GUI/CLI launches, which pass no explicit locations,
    // resolve the staged sidecars. Explicit `--resources`/`--sidecars`
    // overrides still win; dev trees without a bundle fall back to the
    // exe-adjacent plain names in `resolve_paths`.
    let resources = resources.or_else(|| {
        if exe_dir.join("sidecars").is_dir() {
            Some(exe_dir.clone())
        } else {
            None
        }
    });
    let paths = resolve_paths(
        state_dir,
        &current_dir,
        &exe_dir,
        resources.as_deref(),
        sidecars,
    )?;
    let admin =
        CaddyAdminClient::new_tcp(SocketAddr::from((Ipv4Addr::LOCALHOST, paths.admin_port)))?;
    let backend = WindowsRuntimeBackend::with_adapters(
        paths,
        WindowsAdapters {
            process_host: Box::leak(Box::new(SystemProcessHost)),
            readiness: Box::leak(Box::new(SystemReadiness)),
            caddy: Box::leak(Box::new(admin)),
            setup: Box::leak(Box::new(SystemSetup::new())),
        },
    );
    Ok(Arc::new(backend))
}
