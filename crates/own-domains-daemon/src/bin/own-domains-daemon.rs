use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{
    DaemonState, IpcServer, ProcessLock, RuntimeBackend, UnavailableRuntimeBackend,
};
use std::{path::PathBuf, sync::Arc};

#[cfg(target_os = "macos")]
use own_domains_daemon::platform::macos::{
    MacOsArchitecture, MacOsRuntimeBackend, RuntimePaths, SidecarManifest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut state_dir = PathBuf::from(".own-domains");
    let mut socket = None;
    let mut resources = None;
    let mut sidecars = None;
    let mut settings_only = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--state-dir" => {
                state_dir = PathBuf::from(args.next().ok_or("--state-dir requires a path")?)
            }
            "--socket" => {
                socket = Some(PathBuf::from(
                    args.next().ok_or("--socket requires a path")?,
                ))
            }
            "--resources" => {
                resources = Some(PathBuf::from(
                    args.next().ok_or("--resources requires a path")?,
                ))
            }
            "--sidecars" => {
                sidecars = Some(PathBuf::from(
                    args.next().ok_or("--sidecars requires a path")?,
                ))
            }
            "--settings-only" => settings_only = true,
            other => return Err(format!("unknown option: {other}").into()),
        }
    }
    let socket = socket.unwrap_or_else(|| state_dir.join("daemon.sock"));
    let _lock = ProcessLock::acquire(&state_dir)?;
    let runtime = runtime_backend(&state_dir, resources, sidecars, settings_only)?;
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&state_dir)),
        runtime,
    ));
    let server = IpcServer::bind(&socket, state)?;
    install_shutdown_signals(socket)?;
    server.run()?;
    Ok(())
}

fn runtime_backend(
    state_dir: &std::path::Path,
    resources: Option<PathBuf>,
    sidecars: Option<PathBuf>,
    settings_only: bool,
) -> Result<Arc<dyn RuntimeBackend>, Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        if !settings_only {
            let state_dir = if state_dir.is_absolute() {
                state_dir.to_path_buf()
            } else {
                std::env::current_dir()?.join(state_dir)
            };
            let resources = match resources {
                Some(resources) => resources,
                None => default_resource_directory(
                    std::env::current_exe()?
                        .parent()
                        .ok_or("the daemon executable has no parent directory")?,
                ),
            };
            let manifest_path = sidecars.unwrap_or_else(|| {
                sidecar_manifest_path(&resources, macos_target_triple(macos_architecture()))
            });
            let manifest = std::fs::read(&manifest_path).map_err(|error| {
                format!(
                    "unable to read the sidecar manifest {}: {error}",
                    manifest_path.display()
                )
            })?;
            let sidecars: SidecarManifest = serde_json::from_slice(&manifest).map_err(|error| {
                format!(
                    "unable to parse the sidecar manifest {}: {error}",
                    manifest_path.display()
                )
            })?;
            let backup_directory = state_dir.join("backups/setup-1");
            let architecture = macos_architecture();
            let paths = RuntimePaths::new(state_dir, resources, backup_directory, architecture)?;
            return Ok(Arc::new(MacOsRuntimeBackend::try_new(paths, sidecars)?));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (state_dir, resources, sidecars, settings_only);
    }
    Ok(Arc::new(UnavailableRuntimeBackend))
}

#[cfg(target_os = "macos")]
fn default_resource_directory(executable_directory: &std::path::Path) -> PathBuf {
    let resources = executable_directory.join("../Resources");
    if resources.join("sidecars").is_dir() {
        resources
    } else {
        executable_directory.to_path_buf()
    }
}

#[cfg(target_os = "macos")]
fn sidecar_manifest_path(resources: &std::path::Path, target: &str) -> PathBuf {
    let sidecars = resources.join("sidecars");
    let manifest = sidecars.join(format!("manifest-{target}.json"));
    if manifest.is_file() {
        manifest
    } else {
        sidecars.join("manifest.json")
    }
}

#[cfg(target_os = "macos")]
fn macos_target_triple(architecture: MacOsArchitecture) -> &'static str {
    match architecture {
        MacOsArchitecture::Arm64 => "aarch64-apple-darwin",
        MacOsArchitecture::X86_64 => "x86_64-apple-darwin",
    }
}

#[cfg(target_os = "macos")]
fn macos_architecture() -> MacOsArchitecture {
    #[cfg(target_arch = "aarch64")]
    {
        MacOsArchitecture::Arm64
    }
    #[cfg(target_arch = "x86_64")]
    {
        MacOsArchitecture::X86_64
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        compile_error!("the macOS runtime requires an Apple silicon or Intel build")
    }
}

#[cfg(unix)]
fn install_shutdown_signals(socket: PathBuf) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicI32, Ordering};

    static SIGNAL_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

    extern "C" fn handle_termination(_signal: libc::c_int) {
        let fd = SIGNAL_WRITE_FD.load(Ordering::Relaxed);
        if fd < 0 {
            return;
        }
        let byte = 1_u8;
        unsafe {
            libc::write(fd, (&byte as *const u8).cast(), 1);
        }
    }

    let mut pipe = [0_i32; 2];
    if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    SIGNAL_WRITE_FD.store(pipe[1], Ordering::SeqCst);
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_termination as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_termination as *const () as libc::sighandler_t,
        );
    }
    std::thread::spawn(move || {
        let mut byte = [0_u8; 1];
        loop {
            let read = unsafe { libc::read(pipe[0], byte.as_mut_ptr().cast(), 1) };
            if read <= 0 {
                return;
            }
            IpcServer::request_shutdown(&socket);
        }
    });
    Ok(())
}

#[cfg(not(unix))]
fn install_shutdown_signals(_socket: PathBuf) -> std::io::Result<()> {
    Ok(())
}
