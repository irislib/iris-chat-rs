use super::*;
use std::cell::Cell;

// Keep the controls out of the message's measured layout. They use the existing
// flexible space beside the bubble, so revealing them never rewraps text or
// shifts the timeline, and touch layouts do not reserve an empty action gutter.
pub fn install_hover_actions(
    row: &gtk::Box,
    bubble: &gtk::Box,
    message: &ChatMessageSnapshot,
    chat: &CurrentChatSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Overlay {
    let slot = gtk::Overlay::new();
    slot.set_hexpand(true);
    slot.set_overflow(gtk::Overflow::Visible);
    slot.set_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));

    let dock = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    dock.set_widget_name("messageActionDock");
    dock.add_css_class("message-action-dock");
    dock.set_halign(if message.is_outgoing {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    dock.set_valign(gtk::Align::Center);
    if message.is_outgoing {
        dock.set_margin_end(6);
    } else {
        dock.set_margin_start(6);
    }
    dock.set_visible(false);
    slot.add_overlay(&dock);
    slot.set_measure_overlay(&dock, false);
    slot.set_clip_overlay(&dock, false);

    let reveal = Rc::new(Reveal {
        dock: dock.downgrade(),
        pointer_inside: Cell::new(false),
        focus_inside: Cell::new(false),
        open_popovers: Cell::new(0),
    });
    let menu = build_message_popover(message, chat, manager);
    menu.set_widget_name("messageActionsMenu");
    menu.set_parent(bubble);
    retain_while_open(&menu, &reveal);

    if !message.deleted_for_everyone && !is_removed_group(chat) {
        let react = dock_button("face-smile-symbolic", "React", "messageReactButton");
        let chooser = build_reaction_emoji_popover(message, manager, &menu);
        chooser.set_widget_name("messageReactionPicker");
        chooser.set_parent(&react);
        let cleanup = chooser.downgrade();
        react.connect_destroy(move |_| {
            if let Some(chooser) = cleanup.upgrade() {
                chooser.unparent();
            }
        });
        retain_while_open(&chooser, &reveal);
        let weak = chooser.downgrade();
        react.connect_clicked(move |_| {
            if let Some(chooser) = weak.upgrade() {
                chooser.popup();
            }
        });
        dock.append(&react);
    }

    let info = dock_button("help-about-symbolic", "Info", "messageInfoButton");
    let target = message.clone();
    let chat = chat.clone();
    let manager = manager.clone();
    info.connect_clicked(move |button| {
        let parent = button
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        present_message_info(parent.as_ref(), &target, &chat, &manager);
    });
    dock.append(&info);

    let more = dock_button("view-more-symbolic", "More actions", "messageMoreButton");
    let weak = menu.downgrade();
    more.connect_clicked(move |_| {
        if let Some(menu) = weak.upgrade() {
            menu.set_pointing_to(None);
            menu.popup();
        }
    });
    dock.append(&more);

    let motion = gtk::EventControllerMotion::new();
    let state = reveal.clone();
    motion.connect_enter(move |_, _, _| {
        state.pointer_inside.set(true);
        state.update();
    });
    let state = reveal.clone();
    motion.connect_leave(move |_| {
        state.pointer_inside.set(false);
        state.update();
    });
    row.add_controller(motion);

    // Focusing the bubble reveals the same actions for keyboard users. Keep it
    // visible as focus travels from the bubble to its action buttons/popovers.
    bubble.set_focusable(true);
    bubble.set_focus_on_click(false);
    let focus = gtk::EventControllerFocus::new();
    let state = reveal.clone();
    focus.connect_enter(move |_| {
        state.focus_inside.set(true);
        state.update();
    });
    let state = reveal.clone();
    focus.connect_leave(move |_| {
        state.focus_inside.set(false);
        state.update();
    });
    row.add_controller(focus);

    let weak = menu.downgrade();
    let click = gtk::GestureClick::new();
    click.set_button(3);
    click.connect_pressed(move |_, _, x, y| popup_at(&weak, x, y));
    bubble.add_controller(click);

    let weak = menu.downgrade();
    let long_press = gtk::GestureLongPress::new();
    long_press.set_touch_only(true);
    long_press.connect_pressed(move |_, x, y| popup_at(&weak, x, y));
    bubble.add_controller(long_press);

    let weak = menu.downgrade();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gtk::gdk::Key::Menu
            || (key == gtk::gdk::Key::F10 && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK))
        {
            if let Some(menu) = weak.upgrade() {
                menu.set_pointing_to(None);
                menu.popup();
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    bubble.add_controller(keys);
    slot
}

fn dock_button(icon: &str, label: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.set_widget_name(name);
    button.add_css_class("flat");
    button.add_css_class("circular");
    button.set_tooltip_text(Some(label));
    button.set_focus_on_click(false);
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.show_pointer_cursor();
    button
}

struct Reveal {
    dock: glib::WeakRef<gtk::Box>,
    pointer_inside: Cell<bool>,
    focus_inside: Cell<bool>,
    open_popovers: Cell<usize>,
}

impl Reveal {
    fn update(&self) {
        if let Some(dock) = self.dock.upgrade() {
            dock.set_visible(
                self.pointer_inside.get()
                    || self.focus_inside.get()
                    || self.open_popovers.get() > 0,
            );
        }
    }
}

fn retain_while_open(popover: &gtk::Popover, state: &Rc<Reveal>) {
    let on_open = state.clone();
    popover.connect_map(move |_| {
        on_open.open_popovers.set(on_open.open_popovers.get() + 1);
        on_open.update();
    });
    let on_close = state.clone();
    popover.connect_unmap(move |_| {
        on_close
            .open_popovers
            .set(on_close.open_popovers.get().saturating_sub(1));
        on_close.update();
    });
}

fn popup_at(popover: &glib::WeakRef<gtk::Popover>, x: f64, y: f64) {
    if let Some(popover) = popover.upgrade() {
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    }
}

#[cfg(feature = "ui-tests")]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
