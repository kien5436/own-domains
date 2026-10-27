use own_domains_core::{DomainService, SettingsStore};
use own_domains_daemon::{DaemonState, IpcServer, ProcessLock};
use own_domains_protocol::Endpoint;
use std::{path::PathBuf, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut state_dir = PathBuf::from(".own-domains");
    let mut socket = None;
    let mut pipe = None;
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
            "--pipe" => pipe = Some(args.next().ok_or("--pipe requires a name")?),
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
    let endpoint = match (socket, pipe) {
        (Some(_), Some(_)) => return Err("use either --socket or --pipe".into()),
        (_, Some(name)) => Endpoint::named_pipe(name)?,
        (Some(path), None) => Endpoint::unix(path),
        (None, None) => Endpoint::default_for_state_dir(&state_dir),
    };
    let _lock = ProcessLock::acquire(&state_dir)?;
    let runtime = own_domains_daemon::platform::runtime_backend(
        &state_dir,
        resources,
        sidecars,
        settings_only,
    )?;
    let state = Arc::new(DaemonState::new(
        DomainService::new(SettingsStore::new(&state_dir)),
        runtime,
    ));
    let server = IpcServer::bind(&endpoint, state)?;
    install_shutdown_signals(endpoint)?;
    server.run()?;
    Ok(())
}

#[cfg(unix)]
fn install_shutdown_signals(endpoint: Endpoint) -> std::io::Result<()> {
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
            IpcServer::request_shutdown(&endpoint);
        }
    });
    Ok(())
}

#[cfg(not(unix))]
fn install_shutdown_signals(_endpoint: Endpoint) -> std::io::Result<()> {
    Ok(())
}
