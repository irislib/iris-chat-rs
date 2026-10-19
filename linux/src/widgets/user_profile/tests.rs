use super::*;
use iris_chat_core::AppAction;
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

pub fn verify_ui(manager: Rc<AppManager>) {
    let before = manager.current_state().account.expect("test account");
    let owner = before.public_key_hex.clone();
    let label = name(&manager, &owner);
    let picture = avatar(&manager, &owner, 40, false);
    let image = || {
        picture
            .first_child()
            .unwrap()
            .downcast::<gtk::Overlay>()
            .unwrap()
            .child()
            .unwrap()
            .downcast::<adw::Avatar>()
            .unwrap()
    };
    assert_eq!(label.text(), before.display_name);
    assert_eq!(
        image().text().as_deref(),
        Some(before.display_name.as_str())
    );
    // Drop a subscriber before updating: stale weak bindings must be harmless.
    drop(name(&manager, &owner));
    let updates = manager.update_rx();
    for expected in ["Current core name", before.display_name.as_str()] {
        manager.dispatch(AppAction::UpdateProfileMetadata {
            name: expected.into(),
            picture_url: before.picture_url.clone(),
            about: before.about.clone(),
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            while let Ok(update) = updates.try_recv() {
                manager.apply_update(update);
            }
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            if label.text() == expected && image().text().as_deref() == Some(expected) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "ID-bound GTK profile widgets did not update"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    println!(
        "PASS: GTK name/avatar share live core profiles by ID; expired subscribers are removed"
    );
}
