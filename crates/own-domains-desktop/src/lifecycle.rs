use crate::bridge::BridgeError;
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
