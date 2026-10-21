use crate::{
    bridge::{BridgeError, DesktopBridge, DesktopPaths},
    lifecycle::{DesktopLifecycle, LifecycleBackend, WindowActions},
    subscription::Subscriptions,
};
use own_domains_client::{Client, ClientError, connect_or_launch, stop_and_wait};
use own_domains_protocol::{Command, CommandResult};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

struct DesktopState {
    paths: DesktopPaths,
    executable: PathBuf,
    subscriptions: Mutex<Subscriptions>,
    operation: Mutex<()>,
    last_error: Mutex<Option<BridgeError>>,
}
#[derive(Clone)]
struct Host(tauri::AppHandle);
impl Host {
    fn report(&self, error: BridgeError) {
        *self
            .0
            .state::<DesktopState>()
            .last_error
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(error.clone());
        let _ = self.0.emit_to("main", "desktop-error", error);
        self.show();
    }
}
impl WindowActions for Host {
    fn show(&self) {
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }
    fn hide(&self) {
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.hide();
        }
    }
    fn exit(&self) {
        self.0.exit(0);
    }
}
impl LifecycleBackend for Host {
    fn start(&self) -> Result<(), BridgeError> {
        let state = self.0.state::<DesktopState>();
        let _guard = state.operation.lock().unwrap_or_else(|p| p.into_inner());
        let client = connect_or_launch(
            &state.paths.state_dir,
            &state.paths.socket,
            &state.executable,
            Duration::from_secs(5),
        )
        .map_err(lifecycle_error)?;
        client.request(Command::Start)?;
        *state.last_error.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(())
    }
    fn stop(&self) -> Result<(), BridgeError> {
        let state = self.0.state::<DesktopState>();
        let _guard = state.operation.lock().unwrap_or_else(|p| p.into_inner());
        match Client::connect(&state.paths.socket) {
            Ok(client) => stop_and_wait(&client, &state.paths.socket, Duration::from_secs(5))
                .map_err(lifecycle_error)?,
            Err(ClientError::DaemonUnavailable { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        state
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear()?;
        Ok(())
    }
}
fn lifecycle_error(error: own_domains_client::LifecycleError) -> BridgeError {
    match error {
        own_domains_client::LifecycleError::Client(error) => error.into(),
        other => BridgeError::Transport {
            message: other.to_string(),
        },
    }
}
fn require_main(window: &tauri::WebviewWindow) -> Result<(), BridgeError> {
    if window.label() != "main" {
        return Err(BridgeError::InvalidMessage {
            message: "Only the main window can manage the daemon".into(),
        });
    }
    Ok(())
}
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, BridgeError> + Send + 'static,
) -> Result<T, BridgeError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| BridgeError::Transport {
            message: error.to_string(),
        })?
}
#[tauri::command]
async fn daemon_request(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    command: Command,
) -> Result<CommandResult, BridgeError> {
    require_main(&window)?;
    if matches!(command, Command::Start | Command::Stop) {
        return Err(BridgeError::InvalidMessage {
            message: "Use the desktop lifecycle commands for start and quit".into(),
        });
    }
    blocking(move || {
        DesktopBridge::connect(&app.state::<DesktopState>().paths.socket)?.request(command)
    })
    .await
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventMessage {
    subscription_id: u64,
    envelope: own_domains_protocol::EventEnvelope,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DisconnectedMessage {
    subscription_id: u64,
    error: BridgeError,
}
#[tauri::command]
async fn daemon_subscribe(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    after_sequence: Option<u64>,
) -> Result<u64, BridgeError> {
    require_main(&window)?;
    blocking(move || {
        let state = app.state::<DesktopState>();
        let bridge = DesktopBridge::connect(&state.paths.socket)?;
        let emitter = app.clone();
        let sink = Arc::new(move |subscription_id, event| match event {
            Ok(envelope) => {
                let _ = emitter.emit_to(
                    "main",
                    "daemon-event",
                    EventMessage {
                        subscription_id,
                        envelope,
                    },
                );
            }
            Err(error) => {
                let _ = emitter.emit_to(
                    "main",
                    "daemon-disconnected",
                    DisconnectedMessage {
                        subscription_id,
                        error,
                    },
                );
            }
        });
        state
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .replace(&bridge, after_sequence, sink)
    })
    .await
}
#[tauri::command]
async fn daemon_unsubscribe(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    subscription_id: u64,
) -> Result<(), BridgeError> {
    require_main(&window)?;
    blocking(move || {
        app.state::<DesktopState>()
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(subscription_id)
    })
    .await
}
#[tauri::command]
async fn daemon_start(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), BridgeError> {
    require_main(&window)?;
    blocking(move || Host(app).start()).await
}
#[tauri::command]
async fn desktop_quit(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<(), BridgeError> {
    require_main(&window)?;
    blocking(move || {
        let host = Host(app);
        DesktopLifecycle::new(host.clone(), host).quit()
    })
    .await
}
#[tauri::command]
fn desktop_error(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<BridgeError>, BridgeError> {
    require_main(&window)?;
    Ok(app
        .state::<DesktopState>()
        .last_error
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone())
}
fn configuration() -> Result<(DesktopPaths, PathBuf), String> {
    let mut state_dir = std::env::var_os("OWN_DOMAINS_STATE_DIR").map(PathBuf::from);
    let mut socket = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--state-dir" => {
                state_dir = Some(PathBuf::from(
                    args.next().ok_or("--state-dir requires a path")?,
                ))
            }
            "--socket" => {
                socket = Some(PathBuf::from(
                    args.next().ok_or("--socket requires a path")?,
                ))
            }
            other => return Err(format!("unknown desktop argument: {other}")),
        }
    }
    let paths = DesktopPaths::new(state_dir.as_deref(), socket.as_deref())
        .map_err(|error| format!("{error:?}"))?;
    let executable = match std::env::var_os("OWN_DOMAINS_DAEMON") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("own-domains-daemon"),
    };
    if !executable.is_absolute() {
        return Err("OWN_DOMAINS_DAEMON must be an absolute path".into());
    }
    Ok((paths, executable))
}
pub fn run() {
    let (paths, executable) = configuration().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            Host(app.clone()).show();
        }))
        .manage(DesktopState {
            paths,
            executable,
            subscriptions: Mutex::new(Subscriptions::default()),
            operation: Mutex::new(()),
            last_error: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            daemon_request,
            daemon_subscribe,
            daemon_unsubscribe,
            daemon_start,
            desktop_quit,
            desktop_error
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let host = Host(window.app_handle().clone());
                DesktopLifecycle::new(host.clone(), host).close();
            }
        })
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Show Own Domains", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            // Small built-in RGBA icon keeps tray availability independent of bundle assets.
            let icon = tauri::image::Image::new_owned([32, 90, 200, 255].repeat(16 * 16), 16, 16);
            TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        Host(tray.app_handle().clone()).show();
                    }
                })
                .on_menu_event(|app, event| {
                    let host = Host(app.clone());
                    if event.id.as_ref() == "show" {
                        host.show();
                    }
                    if event.id.as_ref() == "quit" {
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) =
                                DesktopLifecycle::new(host.clone(), host.clone()).quit()
                            {
                                host.report(error);
                            }
                        });
                    }
                })
                .build(app)?;
            let host = Host(app.handle().clone());
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = host.start() {
                    host.report(error);
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Unable to build the desktop application");
    app.run(|handle, event| {
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
            let host = Host(handle.clone());
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(error) = DesktopLifecycle::new(host.clone(), host.clone()).quit() {
                    host.report(error);
                }
            });
        }
    });
}
