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
        let parent = row
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        if !busy
            && dispatch_authorized_device_input(
                row.text().as_str(),
                &roster_for_changed,
                &manager_for_changed,
                parent.as_ref(),
            )
        {
            row.set_text("");
        }
    });

    let roster_for_scan = roster.clone();
    let manager_for_scan = manager.clone();
    let scan = scan_qr_button("Scan code", move |text| {
        dispatch_authorized_device_input(&text, &roster_for_scan, &manager_for_scan, None);
    });
    scan.add_css_class("suggested-action");
    scan.set_sensitive(!busy);

    let manager_for_apply = manager.clone();
    let roster_for_apply = roster.clone();
    entry.connect_apply(move |row| {
        let parent = row
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        if dispatch_authorized_device_input(
            row.text().as_str(),
            &roster_for_apply,
            &manager_for_apply,
            parent.as_ref(),
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
    parent: Option<&gtk::Window>,
) -> bool {
    let Some(resolved) = resolve_device_authorization_input(raw_input, roster) else {
        return false;
    };
    if resolved.requires_confirmation {
        present_link_device_confirmation(parent, manager.clone(), resolved);
        return true;
    }
    manager.dispatch(AppAction::AddAuthorizedDevice {
        device_input: resolved.device_input,
    });
    true
}

#[derive(Clone)]
struct ResolvedDeviceAuthorizationInput {
    device_input: String,
    requires_confirmation: bool,
    device_label: Option<String>,
    client_label: Option<String>,
}

fn resolve_device_authorization_input(
    raw_input: &str,
    roster: &DeviceRosterSnapshot,
) -> Option<ResolvedDeviceAuthorizationInput> {
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
            return Some(ResolvedDeviceAuthorizationInput {
                device_input: trimmed.to_string(),
                requires_confirmation: true,
                device_label: payload.device_label.clone(),
                client_label: payload.client_label.clone(),
            });
        }
        return Some(ResolvedDeviceAuthorizationInput {
            device_input: normalized_device,
            requires_confirmation: true,
            device_label: payload.device_label.clone(),
            client_label: payload.client_label.clone(),
        });
    }

    let normalized_device = normalize_peer_input(trimmed.to_string());
    is_valid_peer_input(normalized_device.clone()).then_some(ResolvedDeviceAuthorizationInput {
        device_input: normalized_device,
        requires_confirmation: false,
        device_label: None,
        client_label: None,
    })
}

fn present_link_device_confirmation(
    parent: Option<&gtk::Window>,
    manager: Rc<AppManager>,
    input: ResolvedDeviceAuthorizationInput,
) {
    let dialog = adw::Dialog::builder()
        .title(link_device_confirmation_title(&input))
        .content_width(340)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    let title = gtk::Label::new(Some(&link_device_confirmation_title(&input)));
    title.add_css_class("title-2");
    title.set_halign(gtk::Align::Start);
    content.append(&title);

    let message = gtk::Label::new(Some(&link_device_confirmation_message(&input)));
    message.set_wrap(true);
    message.set_xalign(0.0);
    message.add_css_class("dim-label");
    content.append(&message);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    buttons.set_halign(gtk::Align::End);

    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("pill");
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    buttons.append(&cancel);

    let link = gtk::Button::with_label("Link device");
    link.add_css_class("pill");
    link.add_css_class("suggested-action");
    {
        let dialog = dialog.clone();
        let device_input = input.device_input.clone();
        link.connect_clicked(move |_| {
            manager.dispatch(AppAction::AddAuthorizedDevice {
                device_input: device_input.clone(),
            });
            dialog.close();
        });
    }
    buttons.append(&link);

    content.append(&buttons);
    dialog.set_child(Some(&content));
    dialog.present(parent);
}

fn link_device_confirmation_title(input: &ResolvedDeviceAuthorizationInput) -> String {
    let name = link_device_confirmation_name(input);
    if name == "this device" {
        "Link this device?".to_string()
    } else {
        format!("Link {name}?")
    }
}

fn link_device_confirmation_message(input: &ResolvedDeviceAuthorizationInput) -> String {
    if let Some(client) = non_empty(input.client_label.as_deref()) {
        format!("{client} will be able to use your profile.")
    } else {
        "This device will be able to use your profile.".to_string()
    }
}

fn link_device_confirmation_name(input: &ResolvedDeviceAuthorizationInput) -> String {
    non_empty(input.device_label.as_deref())
        .or_else(|| non_empty(input.client_label.as_deref()))
        .unwrap_or("this device")
        .to_string()
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
