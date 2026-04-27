use std::rc::Rc;

use adw::prelude::*;
use ndr_demo_core::{AppAction, AppState, DeviceEntrySnapshot, DeviceRosterSnapshot};

use crate::app_manager::AppManager;
use crate::screens::screen_container;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let container = screen_container();
    container.set_vexpand(true);

    let Some(roster) = state.device_roster.as_ref() else {
        let empty = gtk::Label::new(Some("No roster available."));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        empty.set_valign(gtk::Align::Center);
        container.append(&empty);
        return container.upcast();
    };

    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

    let inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
    inner.set_margin_top(12);
    inner.set_margin_bottom(12);
    inner.set_margin_start(12);
    inner.set_margin_end(12);

    inner.append(&owner_card(roster));
    if roster.can_manage_devices {
        inner.append(&authorize_card(state, manager));
    }
    inner.append(&devices_card(state, roster, manager));

    scrolled.set_child(Some(&inner));
    container.append(&scrolled);

    container.upcast()
}

fn owner_card(roster: &DeviceRosterSnapshot) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Owner devices").build();

    let owner = adw::ActionRow::builder()
        .title("Owner npub")
        .subtitle(&roster.owner_npub)
        .build();
    group.add(&owner);

    let device = adw::ActionRow::builder()
        .title("This device")
        .subtitle(&roster.current_device_npub)
        .build();
    group.add(&device);

    group.upcast()
}

fn authorize_card(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder().title("Authorize a new device").build();

    let entry = adw::EntryRow::builder()
        .title("Device ID or approval code")
        .build();
    let busy = state.busy.updating_roster;
    let submit = gtk::Button::with_label(if busy { "Authorizing…" } else { "Authorize" });
    submit.add_css_class("suggested-action");
    submit.set_valign(gtk::Align::Center);
    submit.set_sensitive(!busy);

    let entry_for_btn = entry.clone();
    let manager_for_btn = manager.clone();
    submit.connect_clicked(move |btn| {
        let value = entry_for_btn.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        btn.set_sensitive(false);
        entry_for_btn.set_text("");
        manager_for_btn.dispatch(AppAction::AddAuthorizedDevice {
            device_input: value,
        });
    });
    entry.add_suffix(&submit);

    let manager_for_apply = manager.clone();
    entry.connect_apply(move |row| {
        let value = row.text().trim().to_string();
        if value.is_empty() {
            return;
        }
        row.set_text("");
        manager_for_apply.dispatch(AppAction::AddAuthorizedDevice {
            device_input: value,
        });
    });

    group.add(&entry);
    group.upcast()
}

fn devices_card(
    state: &AppState,
    roster: &DeviceRosterSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Linked devices")
        .description(format!("{} registered", roster.devices.len()))
        .build();

    if roster.devices.is_empty() {
        let row = adw::ActionRow::builder()
            .title("No registered devices")
            .subtitle("Authorized devices will appear here.")
            .build();
        group.add(&row);
        return group.upcast();
    }

    for device in &roster.devices {
        group.add(&device_row(state, roster, device, manager));
    }
    group.upcast()
}

fn device_row(
    state: &AppState,
    roster: &DeviceRosterSnapshot,
    device: &DeviceEntrySnapshot,
    manager: &Rc<AppManager>,
) -> adw::ActionRow {
    let title = if device.is_current_device {
        "This device".to_string()
    } else {
        "Linked device".to_string()
    };
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(&device.device_npub)
        .build();

    let status = gtk::Label::new(Some(if device.is_authorized {
        "Authorized"
    } else {
        "Pending"
    }));
    status.add_css_class("caption");
    status.add_css_class(if device.is_authorized {
        "success"
    } else {
        "warning"
    });
    status.set_valign(gtk::Align::Center);
    row.add_suffix(&status);

    if roster.can_manage_devices && !device.is_current_device {
        if !device.is_authorized {
            let approve = gtk::Button::with_label("Approve");
            approve.add_css_class("suggested-action");
            approve.set_valign(gtk::Align::Center);
            approve.set_sensitive(!state.busy.updating_roster);
            let manager_for_btn = manager.clone();
            let device_pubkey_hex = device.device_pubkey_hex.clone();
            approve.connect_clicked(move |_| {
                manager_for_btn.dispatch(AppAction::AddAuthorizedDevice {
                    device_input: device_pubkey_hex.clone(),
                });
            });
            row.add_suffix(&approve);
        }

        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_tooltip_text(Some("Remove device"));
        remove.set_valign(gtk::Align::Center);
        remove.set_sensitive(!state.busy.updating_roster);
        let manager_for_btn = manager.clone();
        let device_pubkey_hex = device.device_pubkey_hex.clone();
        remove.connect_clicked(move |_| {
            manager_for_btn.dispatch(AppAction::RemoveAuthorizedDevice {
                device_pubkey_hex: device_pubkey_hex.clone(),
            });
        });
        row.add_suffix(&remove);
    }

    row
}
