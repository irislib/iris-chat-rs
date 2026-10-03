use super::*;

#[derive(Clone)]
pub(super) struct Content {
    pub root: gtk::Box,
    chat: Rc<RefCell<Option<screens::chat::ChatView>>>,
    section: Rc<RefCell<SectionFocus>>,
}

#[derive(Default)]
struct SectionFocus {
    remembered_chat: Option<(String, String)>,
    pending_composer: Option<bool>,
}

impl Content {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("iris-root");
        root.set_vexpand(true);
        Self {
            root,
            chat: Rc::new(RefCell::new(None)),
            section: Rc::new(RefCell::new(SectionFocus::default())),
        }
    }

    pub fn install_section_shortcuts(
        &self,
        window: &adw::ApplicationWindow,
        manager: &Rc<AppManager>,
    ) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let content = self.clone();
        let manager = manager.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let modifiers = modifiers
                & (gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::META_MASK);
            if !matches!(key, gtk::gdk::Key::t | gtk::gdk::Key::T)
                || (modifiers != gtk::gdk::ModifierType::CONTROL_MASK
                    && modifiers
                        != (gtk::gdk::ModifierType::CONTROL_MASK
                            | gtk::gdk::ModifierType::SHIFT_MASK))
            {
                return glib::Propagation::Proceed;
            }
            let state = manager.current_state();
            let Some(account) = state.account.as_ref() else {
                return glib::Propagation::Proceed;
            };
            let composer = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            let mut section = content.section.borrow_mut();
            if let Some(chat) = state.current_chat.as_ref() {
                section.remembered_chat =
                    Some((account.public_key_hex.clone(), chat.chat_id.clone()));
            }
            if composer {
                if crate::widgets::keyboard_list::focus_composer(content.root.upcast_ref()) {
                    return glib::Propagation::Stop;
                }
                let Some((owner, chat_id)) =
                    section.remembered_chat.as_ref().filter(|(owner, id)| {
                        *owner == account.public_key_hex
                            && state.chat_list.iter().any(|chat| chat.chat_id == *id)
                    })
                else {
                    section.remembered_chat = None;
                    return glib::Propagation::Proceed;
                };
                let chat_id = chat_id.clone();
                section.pending_composer = Some(true);
                drop(section);
                manager.dispatch(AppAction::OpenChat { chat_id });
            } else {
                if crate::widgets::keyboard_list::focus_list(content.root.upcast_ref()) {
                    return glib::Propagation::Stop;
                }
                section.pending_composer = Some(false);
                drop(section);
                manager.dispatch(AppAction::UpdateScreenStack { stack: vec![] });
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys);
    }

    fn focus_pending_section(&self) {
        let mut section = self.section.borrow_mut();
        let focused = match section.pending_composer {
            Some(true) => crate::widgets::keyboard_list::focus_composer(self.root.upcast_ref()),
            Some(false) => crate::widgets::keyboard_list::focus_list(self.root.upcast_ref()),
            None => false,
        };
        if focused {
            section.pending_composer = None;
        }
    }

    pub fn replace(&self, widget: &impl IsA<gtk::Widget>) {
        let focus = crate::widgets::keyboard_list::FocusBookmark::capture(self.root.upcast_ref());
        self.chat.borrow_mut().take();
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        self.root.append(widget);
        if let Some(focus) = focus {
            focus.restore(self.root.upcast_ref());
        }
    }

    pub fn update(&self, screen: &Screen, state: &AppState, manager: &Rc<AppManager>) {
        let same_account =
            self.section
                .borrow()
                .remembered_chat
                .as_ref()
                .is_none_or(|(owner, _)| {
                    state
                        .account
                        .as_ref()
                        .is_some_and(|account| account.public_key_hex == *owner)
                });
        if state.account.is_none() || !same_account {
            *self.section.borrow_mut() = SectionFocus::default();
        }
        if let Screen::Chat { chat_id } | Screen::DirectChatInfo { chat_id } = screen {
            let mut chat = self.chat.borrow_mut();
            if let Some(view) = chat.as_mut().filter(|view| view.chat_id == *chat_id) {
                view.update(state, manager);
                self.focus_pending_section();
                return;
            }
        }

        let mut chat = match screen {
            Screen::Chat { chat_id } | Screen::DirectChatInfo { chat_id } => {
                Some(screens::chat::ChatView::new(chat_id))
            }
            _ => None,
        };
        let widget = if let Some(chat) = chat.as_mut() {
            chat.update(state, manager);
            chat.root.clone().upcast()
        } else {
            screens::render(screen, state, manager)
        };
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(560)
            .build();
        clamp.set_child(Some(&widget));
        clamp.set_vexpand(true);
        self.replace(&clamp);
        *self.chat.borrow_mut() = chat;
        self.focus_pending_section();
    }
}
