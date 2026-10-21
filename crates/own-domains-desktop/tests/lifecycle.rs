use own_domains_desktop::bridge::BridgeError;
use own_domains_desktop::lifecycle::{DesktopLifecycle, LifecycleBackend, WindowActions};
use std::sync::{Arc, Mutex};
#[derive(Clone)]
struct Fake(Arc<Mutex<Vec<&'static str>>>, bool);
impl LifecycleBackend for Fake {
    fn start(&self) -> Result<(), BridgeError> {
        self.0.lock().unwrap().push("start");
        Ok(())
    }
    fn stop(&self) -> Result<(), BridgeError> {
        self.0.lock().unwrap().push("stop");
        if self.1 {
            Err(BridgeError::Transport {
                message: "routing may still be active".into(),
            })
        } else {
            Ok(())
        }
    }
}
impl WindowActions for Fake {
    fn show(&self) {
        self.0.lock().unwrap().push("show");
    }
    fn hide(&self) {
        self.0.lock().unwrap().push("hide");
    }
    fn exit(&self) {
        self.0.lock().unwrap().push("exit");
    }
}
#[test]
fn close_and_second_launch_only_change_window_visibility() {
    let fake = Fake(Default::default(), false);
    let lifecycle = DesktopLifecycle::new(fake.clone(), fake.clone());
    lifecycle.close();
    lifecycle.show();
    assert_eq!(*fake.0.lock().unwrap(), vec!["hide", "show"]);
}
#[test]
fn quit_stops_before_exit_and_failed_shutdown_restores_window() {
    for fail in [false, true] {
        let fake = Fake(Default::default(), fail);
        let lifecycle = DesktopLifecycle::new(fake.clone(), fake.clone());
        assert_eq!(lifecycle.quit().is_err(), fail);
        assert_eq!(
            *fake.0.lock().unwrap(),
            if fail {
                vec!["stop", "show"]
            } else {
                vec!["stop", "exit"]
            }
        );
    }
}
