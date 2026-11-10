use super::*;

/// Predicts the screen stack that [`AppCore::push_screen`] will produce, so a
/// shell can show the destination before the core confirms it. `None` means
/// the push has no predictable result and the shell must wait for the core.
#[uniffi::export]
pub fn optimistic_push_screen_stack(
    has_account: bool,
    mut screen_stack: Vec<Screen>,
    screen: Screen,
) -> Option<Vec<Screen>> {
    if !has_account {
        return match screen {
            Screen::Welcome => Some(Vec::new()),
            Screen::CreateAccount
            | Screen::RestoreAccount
            | Screen::RemoteSigner
            | Screen::AddDevice => Some(vec![screen]),
            _ => None,
        };
    }

    match screen {
        Screen::ChatList => Some(Vec::new()),
        Screen::NewChat
        | Screen::NewGroup
        | Screen::CreateInvite
        | Screen::JoinInvite
        | Screen::Settings
        | Screen::DeviceRoster => Some(vec![screen]),
        Screen::Chat { chat_id } => {
            let chat_id = chat_id.trim();
            (!chat_id.is_empty()).then(|| {
                vec![Screen::Chat {
                    chat_id: chat_id.to_string(),
                }]
            })
        }
        Screen::DirectChatInfo { chat_id } => {
            let chat_id = chat_id.trim();
            if chat_id.is_empty() {
                return None;
            }
            let info = Screen::DirectChatInfo {
                chat_id: chat_id.to_string(),
            };
            if screen_stack.last() != Some(&info) {
                screen_stack.push(info);
            }
            Some(screen_stack)
        }
        Screen::GroupDetails { group_id } => {
            let group_id = group_id.trim();
            if group_id.is_empty() {
                return None;
            }
            let group_chat_id = group_chat_id(group_id);
            if active_chat_in_stack(&screen_stack) != Some(group_chat_id.as_str()) {
                screen_stack = vec![Screen::Chat {
                    chat_id: group_chat_id,
                }];
            }
            let details = Screen::GroupDetails {
                group_id: group_id.to_string(),
            };
            if screen_stack.last() != Some(&details) {
                screen_stack.push(details);
            }
            Some(screen_stack)
        }
        Screen::CreateAccount
        | Screen::RestoreAccount
        | Screen::RemoteSigner
        | Screen::AddDevice
        | Screen::DeviceRevoked
        | Screen::Welcome => None,
    }
}

/// Whether a shell should drop its optimistic stack and show the core's:
/// the account is gone, the core caught up, or the shell's timeout `expired`.
#[uniffi::export]
pub fn pending_navigation_settled(
    has_account: bool,
    pending_stack: Vec<Screen>,
    core_stack: Vec<Screen>,
    expired: bool,
) -> bool {
    !has_account || expired || pending_stack == core_stack
}

fn active_chat_in_stack(screen_stack: &[Screen]) -> Option<&str> {
    screen_stack.iter().rev().find_map(|screen| match screen {
        Screen::Chat { chat_id } => Some(chat_id.as_str()),
        _ => None,
    })
}

impl AppCore {
    pub(super) fn push_screen(&mut self, screen: Screen) {
        if self.state.account.is_none() {
            if !matches!(screen, Screen::RemoteSigner) {
                self.cancel_remote_signer_login();
            }
            match screen {
                Screen::RemoteSigner => {
                    self.start_remote_signer_login(None);
                    return;
                }
                Screen::Welcome => {
                    self.stop_pending_linked_device();
                    self.screen_stack.clear();
                    self.active_chat_id = None;
                }
                Screen::CreateAccount | Screen::RestoreAccount | Screen::AddDevice => {
                    self.screen_stack = vec![screen];
                    self.active_chat_id = None;
                    if matches!(self.screen_stack.last(), Some(Screen::AddDevice))
                        && self.pending_linked_device.is_none()
                    {
                        self.start_linked_device("");
                        return;
                    }
                }
                _ => return,
            }

            self.rebuild_persist_and_emit_state();
            return;
        }

        match screen {
            Screen::CreateAccount
            | Screen::RestoreAccount
            | Screen::AddDevice
            | Screen::RemoteSigner => return,
            Screen::ChatList => {
                self.screen_stack.clear();
                self.active_chat_id = None;
            }
            Screen::NewChat => {
                if !self.can_use_chats() {
                    self.state.toast =
                        Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
                    self.emit_state();
                    return;
                }
                self.screen_stack = vec![Screen::NewChat];
                self.active_chat_id = None;
            }
            Screen::NewGroup => {
                if !self.can_use_chats() {
                    self.state.toast =
                        Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
                    self.emit_state();
                    return;
                }
                self.screen_stack = vec![Screen::NewGroup];
                self.active_chat_id = None;
            }
            Screen::CreateInvite => {
                if !self.can_use_chats() {
                    self.state.toast =
                        Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
                    self.emit_state();
                    return;
                }
                self.screen_stack = vec![Screen::CreateInvite];
                self.active_chat_id = None;
                self.create_public_invite();
                return;
            }
            Screen::JoinInvite => {
                if !self.can_use_chats() {
                    self.state.toast =
                        Some(chat_unavailable_message(self.logged_in.as_ref()).to_string());
                    self.emit_state();
                    return;
                }
                self.screen_stack = vec![Screen::JoinInvite];
                self.active_chat_id = None;
            }
            Screen::Settings => {
                self.screen_stack = vec![Screen::Settings];
                self.active_chat_id = None;
            }
            Screen::Chat { chat_id } => {
                self.open_chat(&chat_id);
                return;
            }
            Screen::DirectChatInfo { chat_id } => {
                let Some(chat_id) = self.normalize_chat_id(&chat_id) else {
                    return;
                };
                if !matches!(chat_kind_for_id(&chat_id), ChatKind::Direct) {
                    return;
                }
                self.ensure_thread_record(&chat_id, unix_now().get());
                if !matches!(
                    self.screen_stack.last(),
                    Some(Screen::DirectChatInfo { chat_id: current }) if current == &chat_id
                ) {
                    self.screen_stack.push(Screen::DirectChatInfo { chat_id });
                }
            }
            Screen::GroupDetails { group_id } => {
                let Some(group_id) = normalize_group_id(&group_id) else {
                    return;
                };
                let group_chat_id = group_chat_id(&group_id);
                if self.active_chat_id.as_deref() != Some(group_chat_id.as_str()) {
                    self.open_chat(&group_chat_id);
                }
                if !matches!(
                    self.screen_stack.last(),
                    Some(Screen::GroupDetails { group_id: current }) if current == &group_id
                ) {
                    self.screen_stack.push(Screen::GroupDetails { group_id });
                }
            }
            Screen::DeviceRoster => {
                self.screen_stack = vec![Screen::DeviceRoster];
                self.active_chat_id = None;
                self.fetch_pending_device_invites_for_local_owner();
            }
            Screen::DeviceRevoked | Screen::Welcome => return,
        }

        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn navigate_back(&mut self) {
        self.reset_pending_invite_acceptance();
        if matches!(self.screen_stack.last(), Some(Screen::RemoteSigner)) {
            self.cancel_remote_signer_login();
        }
        if self.screen_stack.pop().is_none() {
            return;
        }
        self.sync_active_chat_from_router();
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn update_screen_stack(&mut self, stack: Vec<Screen>) {
        if self.state.account.is_none() {
            self.screen_stack = stack
                .into_iter()
                .filter(|screen| {
                    matches!(
                        screen,
                        Screen::CreateAccount
                            | Screen::RestoreAccount
                            | Screen::AddDevice
                            | Screen::RemoteSigner
                    )
                })
                .collect();
            if !self
                .screen_stack
                .iter()
                .any(|screen| matches!(screen, Screen::AddDevice))
            {
                self.stop_pending_linked_device();
            }
            if !self
                .screen_stack
                .iter()
                .any(|screen| matches!(screen, Screen::RemoteSigner))
            {
                self.cancel_remote_signer_login();
            }
            self.active_chat_id = None;
            self.rebuild_persist_and_emit_state();
            return;
        }

        let mut normalized_stack = Vec::new();
        for screen in stack {
            match screen {
                Screen::Welcome
                | Screen::CreateAccount
                | Screen::RestoreAccount
                | Screen::RemoteSigner
                | Screen::AddDevice
                | Screen::ChatList
                | Screen::DeviceRevoked => {}
                Screen::Settings => normalized_stack.push(Screen::Settings),
                Screen::CreateInvite => {
                    if self.can_use_chats() {
                        normalized_stack.push(Screen::CreateInvite);
                    }
                }
                Screen::JoinInvite => {
                    if self.can_use_chats() {
                        normalized_stack.push(Screen::JoinInvite);
                    }
                }
                Screen::NewChat => {
                    if self.can_use_chats() {
                        normalized_stack.push(Screen::NewChat);
                    }
                }
                Screen::NewGroup => {
                    if self.can_use_chats() {
                        normalized_stack.push(Screen::NewGroup);
                    }
                }
                Screen::DeviceRoster => normalized_stack.push(Screen::DeviceRoster),
                Screen::Chat { chat_id } => {
                    if self.can_use_chats() {
                        if let Some(chat_id) = self.normalize_chat_id(&chat_id) {
                            normalized_stack.push(Screen::Chat { chat_id });
                        }
                    }
                }
                Screen::DirectChatInfo { chat_id } => {
                    if self.can_use_chats() {
                        if let Some(chat_id) = self.normalize_chat_id(&chat_id) {
                            if matches!(chat_kind_for_id(&chat_id), ChatKind::Direct) {
                                self.ensure_thread_record(&chat_id, unix_now().get());
                                normalized_stack.push(Screen::DirectChatInfo { chat_id });
                            }
                        }
                    }
                }
                Screen::GroupDetails { group_id } => {
                    if self.can_use_chats() {
                        if let Some(group_id) = normalize_group_id(&group_id) {
                            normalized_stack.push(Screen::GroupDetails { group_id });
                        }
                    }
                }
            }
        }

        self.screen_stack = normalized_stack;
        self.sync_active_chat_from_router();
        self.rebuild_persist_and_emit_state();
    }

    pub(super) fn is_device_roster_open(&self) -> bool {
        matches!(self.screen_stack.last(), Some(Screen::DeviceRoster))
    }

    pub(super) fn is_chat_visible(&self, chat_id: &str) -> bool {
        self.app_in_foreground
            && matches!(
                self.screen_stack.last(),
                Some(Screen::Chat { chat_id: current }) if current == chat_id
            )
    }

    pub(super) fn sync_active_chat_from_router(&mut self) {
        match active_chat_in_stack(&self.screen_stack).map(str::to_string) {
            Some(chat_id) => {
                self.active_chat_id = Some(chat_id.clone());
                if let Some(thread) = self.threads.get_mut(&chat_id) {
                    thread.unread_count = 0;
                }
            }
            _ => {
                self.active_chat_id = None;
            }
        }
    }
}

#[cfg(test)]
mod optimistic_navigation_tests {
    use super::*;

    fn chat(chat_id: &str) -> Screen {
        Screen::Chat {
            chat_id: chat_id.to_string(),
        }
    }

    fn group_details(group_id: &str) -> Screen {
        Screen::GroupDetails {
            group_id: group_id.to_string(),
        }
    }

    fn direct_info(chat_id: &str) -> Screen {
        Screen::DirectChatInfo {
            chat_id: chat_id.to_string(),
        }
    }

    #[test]
    fn logged_out_pushes_only_predict_onboarding_screens() {
        let push = |screen| optimistic_push_screen_stack(false, vec![Screen::AddDevice], screen);
        assert_eq!(push(Screen::Welcome), Some(vec![]));
        assert_eq!(
            push(Screen::RestoreAccount),
            Some(vec![Screen::RestoreAccount])
        );
        assert_eq!(push(Screen::RemoteSigner), Some(vec![Screen::RemoteSigner]));
        assert_eq!(push(Screen::Settings), None);
        assert_eq!(push(chat("a")), None);
    }

    #[test]
    fn logged_in_root_screens_replace_the_stack() {
        let push = |screen| optimistic_push_screen_stack(true, vec![chat("a")], screen);
        assert_eq!(push(Screen::ChatList), Some(vec![]));
        assert_eq!(push(Screen::Settings), Some(vec![Screen::Settings]));
        assert_eq!(push(chat(" b ")), Some(vec![chat("b")]));
        assert_eq!(push(chat("  ")), None);
        assert_eq!(push(Screen::Welcome), None);
        assert_eq!(push(Screen::AddDevice), None);
        assert_eq!(push(Screen::DeviceRevoked), None);
    }

    #[test]
    fn direct_chat_info_is_pushed_once() {
        let stack = vec![chat("a")];
        let pushed = optimistic_push_screen_stack(true, stack, direct_info(" a ")).unwrap();
        assert_eq!(pushed, vec![chat("a"), direct_info("a")]);
        assert_eq!(
            optimistic_push_screen_stack(true, pushed.clone(), direct_info("a")),
            Some(pushed)
        );
        assert_eq!(
            optimistic_push_screen_stack(true, vec![], direct_info(" ")),
            None
        );
    }

    #[test]
    fn group_details_opens_the_group_chat_unless_it_is_active() {
        let opened = vec![chat("group:g"), group_details("g")];
        assert_eq!(
            optimistic_push_screen_stack(true, vec![chat("a")], group_details(" g ")),
            Some(opened.clone())
        );
        assert_eq!(
            optimistic_push_screen_stack(true, vec![chat("group:g")], group_details("g")),
            Some(opened.clone())
        );
        assert_eq!(
            optimistic_push_screen_stack(true, opened.clone(), group_details("g")),
            Some(opened)
        );
        assert_eq!(
            optimistic_push_screen_stack(true, vec![], group_details("")),
            None
        );
    }

    #[test]
    fn pending_navigation_settles_on_logout_match_or_expiry() {
        let pending = || vec![chat("a")];
        assert!(!pending_navigation_settled(true, pending(), vec![], false));
        assert!(pending_navigation_settled(
            true,
            pending(),
            pending(),
            false
        ));
        assert!(pending_navigation_settled(true, pending(), vec![], true));
        assert!(pending_navigation_settled(false, pending(), vec![], false));
    }
}
