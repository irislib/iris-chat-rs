use adw::prelude::*;

const DISMISSIBLE: &str = "dismiss-on-backdrop";
const CONTROLLER_NAME: &str = "iris-dialog-backdrop";

#[cfg(feature = "ui-tests")]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;

/// Present a lightweight dialog with the same click-away behavior as a popover.
pub fn present(dialog: &adw::Dialog, parent: Option<&gtk::Window>) {
    dialog.add_css_class(DISMISSIBLE);
    dialog.present(parent);
    let Some(window) = dialog.root().and_downcast::<gtk::Window>() else {
        return;
    };
    // One controller per window prevents a click from dismissing two stacked
    // dialogs and avoids accumulating handlers as profiles are opened again.
    if window
        .observe_controllers()
        .iter::<gtk::glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventController>().ok())
        .any(|controller| controller.name().as_deref() == Some(CONTROLLER_NAME))
    {
        return;
    }
    let click = gtk::GestureClick::new();
    click.set_name(Some(CONTROLLER_NAME));
    click.set_button(gtk::gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = window.downgrade();
    click.connect_pressed(move |gesture, _, x, y| {
        if let Some(window) = weak.upgrade() {
            if dismiss_at(&window, x, y) {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        }
    });
    window.add_controller(click);
}

fn visible_dialog(window: &gtk::Window) -> Option<adw::Dialog> {
    if let Some(window) = window.downcast_ref::<adw::ApplicationWindow>() {
        window.visible_dialog()
    } else {
        window
            .downcast_ref::<adw::Window>()
            .and_then(|window| window.visible_dialog())
    }
}

fn dismiss_at(window: &gtk::Window, x: f64, y: f64) -> bool {
    let Some(dialog) = visible_dialog(window) else {
        return false;
    };
    if !dialog.has_css_class(DISMISSIBLE) || !dialog.can_close() {
        return false;
    }
    let Some(content) = dialog.child() else {
        return false;
    };
    // Let an open menu consume its own outside click first.
    if has_open_popover(&content) {
        return false;
    }
    if window
        .pick(x, y, gtk::PickFlags::DEFAULT)
        .is_some_and(|target| target == content || target.is_ancestor(&content))
    {
        return false;
    }
    let Some(bounds) = content.compute_bounds(window) else {
        return false;
    };
    // Margins belong to the dialog surface too, not to its backdrop.
    if x >= f64::from(bounds.x()) - f64::from(content.margin_start())
        && x < f64::from(bounds.x() + bounds.width()) + f64::from(content.margin_end())
        && y >= f64::from(bounds.y()) - f64::from(content.margin_top())
        && y < f64::from(bounds.y() + bounds.height()) + f64::from(content.margin_bottom())
    {
        return false;
    }
    if content.is_mapped() {
        return dialog.close();
    }

    // libadwaita 1.5 maps the dialog before opening its sheet on a later frame.
    // Closing in that gap is undone by the pending open. Retain the click and
    // close once the content is mapped, without dismissing a newer top dialog.
    let window = window.downgrade();
    dialog.add_tick_callback(move |dialog, _| {
        let Some(window) = window.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        if visible_dialog(&window).as_ref() != Some(dialog)
            || !dialog.has_css_class(DISMISSIBLE)
            || !dialog.can_close()
        {
            return gtk::glib::ControlFlow::Break;
        }
        let Some(content) = dialog.child() else {
            return gtk::glib::ControlFlow::Break;
        };
        if !content.is_mapped() {
            return gtk::glib::ControlFlow::Continue;
        }
        dialog.close();
        gtk::glib::ControlFlow::Break
    });
    true
}

fn has_open_popover(widget: &gtk::Widget) -> bool {
    if widget.is::<gtk::Popover>() && widget.is_mapped() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if has_open_popover(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}
