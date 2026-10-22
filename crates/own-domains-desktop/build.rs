fn main() {
    #[cfg(feature = "desktop")]
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "daemon_request",
            "daemon_subscribe",
            "daemon_unsubscribe",
            "daemon_start",
            "daemon_stop",
            "desktop_quit",
            "desktop_error",
        ]),
    ))
    .expect("Tauri build failed");
}
