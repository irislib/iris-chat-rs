use super::*;
use std::time::{Duration, Instant};

#[track_caller]
fn pump_until(stage: &str, mut ready: impl FnMut() -> bool) {
    let context = gtk::glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "dialog UI timed out: {stage}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn dialog() -> (adw::Dialog, gtk::Box) {
    let dialog = adw::Dialog::builder()
        .content_width(320)
        .content_height(240)
        .presentation_mode(adw::DialogPresentationMode::Floating)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_margin_top(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.append(&gtk::Button::with_label("Inside the dialog"));
    dialog.set_child(Some(&content));
    (dialog, content)
}

pub fn verify_ui() {
    let window = adw::Window::builder()
        .default_width(960)
        .default_height(700)
        .build();
    window.set_content(Some(&gtk::Button::with_label("Behind the dialog")));
    window.present();
    let (profile, content) = dialog();
    present(&profile, Some(window.upcast_ref()));
    pump_until("profile presentation", || {
        content.width() > 0 && profile.is_mapped()
    });
    let click = window
        .observe_controllers()
        .iter::<gtk::glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventController>().ok())
        .find(|controller| controller.name().as_deref() == Some(CONTROLLER_NAME))
        .unwrap()
        .downcast::<gtk::GestureClick>()
        .unwrap();
    assert_eq!(click.propagation_phase(), gtk::PropagationPhase::Capture);
    assert_eq!(click.button(), gtk::gdk::BUTTON_PRIMARY);
    let bounds = content.compute_bounds(&window).unwrap();
    click.emit_by_name::<()>(
        "pressed",
        &[
            &1i32,
            &f64::from(bounds.x() + 20.0),
            &f64::from(bounds.y() + 20.0),
        ],
    );
    assert!(!dismiss_at(
        window.upcast_ref(),
        f64::from(bounds.x() - 10.0),
        f64::from(bounds.y() + 20.0)
    ));
    assert_eq!(window.visible_dialog(), Some(profile.clone()));

    profile.set_can_close(false);
    assert!(!dismiss_at(window.upcast_ref(), 4.0, 4.0));
    profile.set_can_close(true);

    let menu = gtk::MenuButton::new();
    let popover = gtk::Popover::new();
    popover.set_child(Some(&gtk::Button::with_label("Menu choice")));
    menu.set_popover(Some(&popover));
    content.append(&menu);
    pump_until("menu button layout", || {
        menu.is_mapped() && menu.width() > 0
    });
    menu.popup();
    pump_until("menu popup", || popover.is_mapped());
    assert!(!dismiss_at(window.upcast_ref(), 4.0, 4.0));
    popover.popdown();
    pump_until("menu popdown", || !popover.is_mapped());

    let (nested, nested_content) = dialog();
    present(&nested, Some(window.upcast_ref()));
    pump_until("nested dialog presentation", || {
        nested_content.width() > 0 && nested.is_mapped()
    });
    // Keep this early click: on libadwaita 1.5 the dialog can be allocated
    // before its content maps, and the dismissal must survive its pending open.
    click.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
    pump_until("nested dialog outside-click dismissal", || {
        window.visible_dialog() == Some(profile.clone()) && !nested.is_mapped()
    });
    assert!(
        profile.is_mapped(),
        "outside click must only close the top dialog"
    );

    let (confirmation, confirmation_content) = dialog();
    confirmation.present(Some(&window));
    pump_until("confirmation presentation", || {
        confirmation_content.width() > 0 && confirmation.is_mapped()
    });
    assert!(!dismiss_at(window.upcast_ref(), 4.0, 4.0));
    // This protected dialog uses the native close API, not click-away. Wait
    // until its content is presented before simulating its own close control.
    pump_until("confirmation content presentation", || {
        confirmation_content.is_mapped()
    });
    confirmation.close();
    pump_until("confirmation dismissal", || {
        window.visible_dialog() == Some(profile.clone()) && !confirmation.is_mapped()
    });

    assert_eq!(
        window
            .observe_controllers()
            .iter::<gtk::glib::Object>()
            .filter_map(Result::ok)
            .filter_map(|controller| controller.downcast::<gtk::EventController>().ok())
            .filter(|controller| controller.name().as_deref() == Some(CONTROLLER_NAME))
            .count(),
        1,
        "reopening dialogs must reuse one backdrop handler"
    );
    click.emit_by_name::<()>("pressed", &[&1i32, &4.0f64, &4.0f64]);
    pump_until("profile outside-click dismissal", || {
        window.visible_dialog().is_none()
    });
    window.close();
}
