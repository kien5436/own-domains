use own_domains_desktop::bridge::BridgeError;
use own_domains_desktop::lifecycle::{
    DesktopLifecycle, LifecycleBackend, QUIT_MENU_ID, SHOW_MENU_ID, WindowActions,
    handle_menu_action,
};
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
                message: "partial cleanup: Caddy is still listening on 8080".into(),
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
fn stop_only_stops_the_backend_without_exiting_or_hiding() {
    let fake = Fake(Default::default(), false);
    let lifecycle = DesktopLifecycle::new(fake.clone(), fake.clone());
    lifecycle.stop().unwrap();
    assert_eq!(*fake.0.lock().unwrap(), vec!["stop"]);
}
#[test]
fn quit_menu_action_stops_before_exit() {
    let fake = Fake(Default::default(), false);
    let result = handle_menu_action(QUIT_MENU_ID, fake.clone(), fake.clone());
    assert!(result.unwrap().is_ok());
    assert_eq!(*fake.0.lock().unwrap(), vec!["stop", "exit"]);
}

#[test]
fn show_menu_action_only_restores_the_window() {
    let fake = Fake(Default::default(), false);
    let result = handle_menu_action(SHOW_MENU_ID, fake.clone(), fake.clone());
    assert!(result.unwrap().is_ok());
    assert_eq!(*fake.0.lock().unwrap(), vec!["show"]);
}

#[test]
fn unknown_menu_action_is_ignored() {
    let fake = Fake(Default::default(), false);
    assert!(handle_menu_action("unknown", fake.clone(), fake.clone()).is_none());
    assert!(fake.0.lock().unwrap().is_empty());
}

#[test]
fn quit_stops_before_exit_and_failed_shutdown_restores_window() {
    for fail in [false, true] {
        let fake = Fake(Default::default(), fail);
        let lifecycle = DesktopLifecycle::new(fake.clone(), fake.clone());
        match lifecycle.quit() {
            Ok(()) => assert!(!fail),
            Err(BridgeError::Transport { message }) => {
                assert!(fail);
                assert_eq!(message, "partial cleanup: Caddy is still listening on 8080");
            }
            Err(other) => panic!("unexpected quit error: {other:?}"),
        }
        assert_eq!(
            *fake.0.lock().unwrap(),
            if fail {
                vec!["stop", "show"]
            } else {
                vec!["stop", "exit"]
            }
        );
        assert!(!fake.0.lock().unwrap().contains(&"exit") || !fail);
    }
}
