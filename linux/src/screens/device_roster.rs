use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{
    decode_device_approval_qr, is_valid_peer_input, normalize_peer_input, AppAction, AppState,
    DeviceEntrySnapshot, DeviceRosterSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::chat_list::{relative_time, unix_now};
use crate::screens::scan_qr_button;

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&content(state, manager)));
    scrolled.upcast()
}

pub(crate) fn content(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 16);
    inner.set_margin_top(20);
    inner.set_margin_bottom(20);
    inner.set_margin_start(16);
    inner.set_margin_end(16);

    let Some(roster) = state.device_roster.as_ref() else {
        let empty = gtk::Label::new(Some("Devices unavailable."));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        empty.set_valign(gtk::Align::Center);
        inner.append(&empty);
        return inner.upcast();
    };

    inner.append(&owner_card(roster));
    if roster.can_manage_devices {
        inner.append(&authorize_card(state, roster, manager));
    }
    inner.append(&devices_card(state, roster, manager));

    inner.upcast()
}

fn owner_card(_roster: &DeviceRosterSnapshot) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Linked devices")
        .description("These devices can use your profile.")
        .build();

    let device = adw::ActionRow::builder().title("This device").build();
    group.add(&device);

    group.upcast()
}

fn authorize_card(
    state: &AppState,
    roster: &DeviceRosterSnapshot,
    manager: &Rc<AppManager>,
) -> gtk::Widget {
    let group = adw::PreferencesGroup::builder()
        .title("Link another device")
        .description("Scan the code from the device you want to link, or paste it.")
        .build();

    let entry = adw::EntryRow::builder().title("Link code").build();
    let busy = state.busy.updating_roster;

    let roster_for_changed = roster.clone();
    let manager_for_changed = manager.clone();
    entry.connect_changed(move |row| {
        if !busy
            && dispatch_authorized_device_input(
                row.text().as_str(),
                &roster_for_changed,
                &manager_for_changed,
            )
        {
            row.set_text("");
        }
    });

    let roster_for_scan = roster.clone();
    let manager_for_scan = manager.clone();
    let scan = scan_qr_button("Scan code", move |text| {
        dispatch_authorized_device_input(&text, &roster_for_scan, &manager_for_scan);
    });
    scan.add_css_class("suggested-action");
    scan.set_sensitive(!busy);

    let manager_for_apply = manager.clone();
    let roster_for_apply = roster.clone();
    entry.connect_apply(move |row| {
        if dispatch_authorized_device_input(
            row.text().as_str(),
            &roster_for_apply,
            &manager_for_apply,
        ) {
            row.set_text("");
        }
    });

    group.add(&scan);
    group.add(&entry);
    group.upcast()
}

fn dispatch_authorized_device_input(
    raw_input: &str,
    roster: &DeviceRosterSnapshot,
    manager: &Rc<AppManager>,
) -> bool {
    let Some(value) = resolve_device_authorization_input(raw_input, roster) else {
        return false;
    };
    manager.dispatch(AppAction::AddAuthorizedDevice {
        device_input: value,
    });
    true
}

fn resolve_device_authorization_input(
    raw_input: &str,
    roster: &DeviceRosterSnapshot,
) -> Option<String> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(payload) = decode_device_approval_qr(trimmed.to_string()) {
        let normalized_owner = normalize_peer_input(payload.owner_input);
        let owner_inputs = [
            normalize_peer_input(roster.owner_npub.clone()),
            normalize_peer_input(roster.owner_public_key_hex.clone()),
        ];
        if !normalized_owner.is_empty() && !owner_inputs.contains(&normalized_owner) {
            return None;
        }

        let normalized_device = normalize_peer_input(payload.device_input);
        if !is_valid_peer_input(normalized_device.clone()) {
            return None;
        }
        if normalized_owner.is_empty() {
            return Some(trimmed.to_string());
        }
        return Some(normalized_device);
    }

    let normalized_device = normalize_peer_input(trimmed.to_string());
    is_valid_peer_input(normalized_device.clone()).then_some(normalized_device)
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
            .title("No linked devices")
            .subtitle("Linked devices will appear here.")
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
        non_empty(device.device_label.as_deref())
            .unwrap_or("Linked device")
            .to_string()
    };
    let row = adw::ActionRow::builder().title(title).build();
    let mut subtitles = Vec::new();
    if device.is_current_device {
        if let Some(label) = non_empty(device.device_label.as_deref()) {
            subtitles.push(label.to_string());
        }
    }
    if let Some(client) = non_empty(device.client_label.as_deref()) {
        subtitles.push(client.to_string());
    }
    if let Some(secs) = device.added_at_secs {
        let ago = relative_time(secs, unix_now());
        if !ago.is_empty() {
            subtitles.push(format!("Added {} ago", ago));
        }
    }
    if !subtitles.is_empty() {
        row.set_subtitle(&subtitles.join(" - "));
    }

    let status = gtk::Label::new(Some(if device.is_authorized {
        "Linked"
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
            let approve = gtk::Button::with_label("Link");
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

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}
