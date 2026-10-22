use crate::bridge::BridgeError;
pub const SHOW_MENU_ID: &str = "show";
pub const QUIT_MENU_ID: &str = "quit";
pub trait LifecycleBackend {
    fn start(&self) -> Result<(), BridgeError>;
    fn stop(&self) -> Result<(), BridgeError>;
}
pub trait WindowActions {
    fn show(&self);
    fn hide(&self);
    fn exit(&self);
}
pub struct DesktopLifecycle<B, W> {
    backend: B,
    window: W,
}
impl<B: LifecycleBackend, W: WindowActions> DesktopLifecycle<B, W> {
    pub fn new(backend: B, window: W) -> Self {
        Self { backend, window }
    }
    pub fn start(&self) -> Result<(), BridgeError> {
        self.backend.start()
    }
    pub fn stop(&self) -> Result<(), BridgeError> {
        self.backend.stop()
    }
    pub fn close(&self) {
        self.window.hide();
    }
    pub fn show(&self) {
        self.window.show();
    }
    pub fn quit(&self) -> Result<(), BridgeError> {
        match self.backend.stop() {
            Ok(()) => {
                self.window.exit();
                Ok(())
            }
            Err(error) => {
                self.window.show();
                Err(error)
            }
        }
    }
}

pub fn handle_menu_action<B: LifecycleBackend, W: WindowActions>(
    id: &str,
    backend: B,
    window: W,
) -> Option<Result<(), BridgeError>> {
    match id {
        SHOW_MENU_ID => {
            window.show();
            Some(Ok(()))
        }
        QUIT_MENU_ID => Some(DesktopLifecycle::new(backend, window).quit()),
        _ => None,
    }
}
