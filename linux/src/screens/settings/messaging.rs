use super::*;

pub(super) fn messaging_group(
    prefs: &PreferencesSnapshot,
    manager: &Rc<AppManager>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Messaging").build();

    let accept_requests = adw::SwitchRow::builder()
        .title("Accept message requests from unknowns")
        .build();
    accept_requests.set_active(prefs.accept_unknown_direct_messages);
    {
        let manager = manager.clone();
        accept_requests.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetAcceptUnknownDirectMessages {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&accept_requests);

    let hide_blocked = adw::SwitchRow::builder()
        .title("Hide past group messages from blocked people")
        .build();
    hide_blocked.set_widget_name("settings-hide-blocked-history");
    hide_blocked.set_active(prefs.hide_blocked_group_messages);
    {
        let manager = manager.clone();
        hide_blocked.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetHideBlockedGroupMessages {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&hide_blocked);

    let typing = adw::SwitchRow::builder().title("Typing indicators").build();
    typing.set_active(prefs.send_typing_indicators);
    {
        let manager = manager.clone();
        typing.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetTypingIndicatorsEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&typing);

    let receipts = adw::SwitchRow::builder().title("Read receipts").build();
    receipts.set_active(prefs.send_read_receipts);
    {
        let manager = manager.clone();
        receipts.connect_active_notify(move |row| {
            manager.dispatch(AppAction::SetReadReceiptsEnabled {
                enabled: row.is_active(),
            });
        });
    }
    group.add(&receipts);

    let deletions = adw::SwitchRow::builder()
        .title("Allow others to delete their messages")
        .build();
    deletions.set_active(prefs.allow_message_deletion_by_others);
    let manager = manager.clone();
    deletions.connect_active_notify(move |row| {
        manager.dispatch(AppAction::SetAllowMessageDeletionByOthers {
            enabled: row.is_active(),
        });
    });
    group.add(&deletions);

    group
}

pub(super) fn blocked_people_group(
    state: &AppState,
    manager: &Rc<AppManager>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Blocked people")
        .build();
    group.set_widget_name("settings-blocked-people");
    if state.blocked_people.is_empty() {
        group.add(&adw::ActionRow::builder().title("No blocked people").build());
    }
    for person in &state.blocked_people {
        let row = adw::ActionRow::builder()
            .title(&person.display_label)
            .subtitle(&person.user_id)
            .build();
        row.set_title_lines(1);
        row.set_subtitle_lines(1);
        row.set_use_markup(false);
        let unblock = gtk::Button::with_label("Unblock");
        unblock.set_valign(gtk::Align::Center);
        unblock.set_widget_name(&format!("settings-unblock-{}", person.owner_pubkey_hex));
        let manager = manager.clone();
        let owner = person.owner_pubkey_hex.clone();
        unblock.connect_clicked(move |_| {
            manager.dispatch(AppAction::SetUserBlocked {
                owner_pubkey_hex: owner.clone(),
                blocked: false,
            })
        });
        row.add_suffix(&unblock);
        group.add(&row);
    }
    group
}
