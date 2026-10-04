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
