use super::*;

impl AppCore {
    pub(super) fn push_screen(&mut self, screen: Screen) {
        if self.state.account.is_none() {
            match screen {
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

            self.rebuild_state();
            self.persist_best_effort();
            self.emit_state();
            return;
        }

        match screen {
            Screen::CreateAccount | Screen::RestoreAccount | Screen::AddDevice => return,
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
            Screen::AwaitingDeviceApproval | Screen::DeviceRevoked | Screen::Welcome => return,
        }

        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn navigate_back(&mut self) {
        self.reset_pending_invite_acceptance();
        if self.screen_stack.pop().is_none() {
            return;
        }
        self.sync_active_chat_from_router();
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn update_screen_stack(&mut self, stack: Vec<Screen>) {
        if self.state.account.is_none() {
            self.screen_stack = stack
                .into_iter()
                .filter(|screen| {
                    matches!(
                        screen,
                        Screen::CreateAccount | Screen::RestoreAccount | Screen::AddDevice
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
            self.active_chat_id = None;
            self.rebuild_state();
            self.persist_best_effort();
            self.emit_state();
            return;
        }

        let mut normalized_stack = Vec::new();
        for screen in stack {
            match screen {
                Screen::Welcome
                | Screen::CreateAccount
                | Screen::RestoreAccount
                | Screen::AddDevice
                | Screen::ChatList
                | Screen::AwaitingDeviceApproval
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
        self.rebuild_state();
        self.persist_best_effort();
        self.emit_state();
    }

    pub(super) fn is_device_roster_open(&self) -> bool {
        matches!(self.screen_stack.last(), Some(Screen::DeviceRoster))
    }

    pub(super) fn is_chat_visible(&self, chat_id: &str) -> bool {
        matches!(
            self.screen_stack.last(),
            Some(Screen::Chat { chat_id: current }) if current == chat_id
        )
    }

    pub(super) fn sync_active_chat_from_router(&mut self) {
        match self
            .screen_stack
            .iter()
            .rev()
            .find_map(|screen| match screen {
                Screen::Chat { chat_id } => Some(chat_id.clone()),
                _ => None,
            }) {
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
