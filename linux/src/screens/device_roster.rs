use std::rc::Rc;

use adw::prelude::*;
use iris_chat_core::{
    decode_device_approval_qr, is_valid_peer_input, normalize_peer_input, AppAction, AppState,
    DeviceEntrySnapshot, DeviceRosterSnapshot,
};

use crate::app_manager::AppManager;
use crate::screens::chat_list::{relative_time, unix_now};

pub fn render(state: &AppState, manager: &Rc<AppManager>) -> gtk::Widget {
    let scrolled = gtk::ScrolledWindow::new();
    scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
    scrolled.set_vexpand(true);

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
        scrolled.set_child(Some(&inner));
        return scrolled.upcast();
    };

    inner.append(&owner_card(roster));
    if roster.can_manage_devices {
        inner.append(&authorize_card(state, roster, manager));
    }
    inner.append(&devices_card(state, roster, manager));

    scrolled.set_child(Some(&inner));
    scrolled.upcast()
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
        .build();

    let entry = adw::EntryRow::builder().title("Link code").build();
    let busy = state.busy.updating_roster;
    let submit = gtk::Button::with_label(if busy { "Linking…" } else { "Link device" });
    submit.add_css_class("suggested-action");
    submit.set_valign(gtk::Align::Center);
    submit.set_sensitive(false);

    let roster_for_changed = roster.clone();
    let submit_for_changed = submit.clone();
    entry.connect_changed(move |row| {
        submit_for_changed.set_sensitive(
            !busy
                && resolve_device_authorization_input(row.text().as_str(), &roster_for_changed)
                    .is_some(),
        );
    });

    let entry_for_btn = entry.clone();
    let roster_for_btn = roster.clone();
    let manager_for_btn = manager.clone();
    submit.connect_clicked(move |btn| {
        let Some(value) =
            resolve_device_authorization_input(entry_for_btn.text().as_str(), &roster_for_btn)
        else {
            return;
        };
        btn.set_sensitive(false);
        entry_for_btn.set_text("");
        manager_for_btn.dispatch(AppAction::AddAuthorizedDevice {
            device_input: value,
        });
    });
    entry.add_suffix(&submit);

    let manager_for_apply = manager.clone();
    let roster_for_apply = roster.clone();
    entry.connect_apply(move |row| {
        let Some(value) =
            resolve_device_authorization_input(row.text().as_str(), &roster_for_apply)
        else {
            return;
        };
        row.set_text("");
        manager_for_apply.dispatch(AppAction::AddAuthorizedDevice {
            device_input: value,
        });
    });

    group.add(&entry);
    group.upcast()
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
        if !owner_inputs.contains(&normalized_owner) {
            return None;
        }

        let normalized_device = normalize_peer_input(payload.device_input);
        return is_valid_peer_input(normalized_device.clone()).then_some(normalized_device);
    }

    is_likely_link_invite(trimmed).then(|| trimmed.to_string())
}

fn is_likely_link_invite(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    if !lower.starts_with("https://chat.iris.to/#") && !lower.starts_with("https://chat.iris.to/?")
    {
        return false;
    }
    let decoded = percent_decode_lossy(input);
    decoded.contains("\"purpose\":\"link\"")
        && decoded.contains("\"ephemeralKey\"")
        && decoded.contains("\"sharedSecret\"")
}

fn percent_decode_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                out.push(high << 4 | low);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
        "Linked device".to_string()
    };
    let row = adw::ActionRow::builder().title(title).build();
    if let Some(secs) = device.added_at_secs {
        let ago = relative_time(secs, unix_now());
        if !ago.is_empty() {
            row.set_subtitle(&format!("Added {} ago", ago));
        }
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
