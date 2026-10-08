use super::*;
use std::cell::Cell;

#[derive(Clone)]
pub(super) struct Content {
    pub root: gtk::Box,
    pub detail: gtk::Box,
    split: adw::NavigationSplitView,
    sidebar_page: adw::NavigationPage,
    detail_page: adw::NavigationPage,
    list: Rc<RefCell<Option<screens::chat_list::ChatListView>>>,
    sidebar_toolbar: adw::ToolbarView,
    sidebar_account: Rc<RefCell<Option<(AccountSnapshot, iris_chat_core::PreferencesSnapshot)>>>,
    sidebar_settings: gtk::Button,
    navigating: Rc<Cell<bool>>,
    screen: Rc<RefCell<Option<Screen>>>,
    chat: Rc<RefCell<Option<screens::chat::ChatView>>>,
    settings: Rc<RefCell<Option<screens::settings::SettingsView>>>,
    form: Rc<RefCell<Option<(Screen, bool, Option<String>)>>>,
    section: Rc<RefCell<SectionFocus>>,
}

#[derive(Default)]
struct SectionFocus {
    remembered_chat: Option<(String, String)>,
    pending_composer: Option<bool>,
    pending_tick: bool,
}

impl Content {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("iris-root");
        root.set_vexpand(true);
        let split = adw::NavigationSplitView::new();
        split.set_sidebar_width_unit(adw::LengthUnit::Px);
        split.set_min_sidebar_width(280.0);
        split.set_max_sidebar_width(352.0);
        split.set_sidebar_width_fraction(0.32);
        split.set_vexpand(true);
        let sidebar_toolbar = adw::ToolbarView::new();
        let sidebar_page = adw::NavigationPage::new(&sidebar_toolbar, "Chats");
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        detail.set_vexpand(true);
        detail.set_hexpand(true);
        let detail_page = adw::NavigationPage::new(&detail, "Iris Chat");
        split.set_content(Some(&detail_page));
        split.set_show_content(true);
        let responsive = adw::BreakpointBin::new();
        responsive.set_size_request(360, 300);
        responsive.set_child(Some(&split));
        let breakpoint =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 759sp").unwrap());
        let compact = split.downgrade();
        breakpoint.connect_apply(move |_| {
            if let Some(split) = compact.upgrade() {
                set_collapsed(&split, true);
            }
        });
        let wide = split.downgrade();
        breakpoint.connect_unapply(move |_| {
            if let Some(split) = wide.upgrade() {
                set_collapsed(&split, false);
            }
        });
        responsive.add_breakpoint(breakpoint);
        root.append(&responsive);
        Self {
            root,
            detail,
            split,
            sidebar_page,
            detail_page,
            sidebar_toolbar,
            list: Rc::new(RefCell::new(None)),
            sidebar_account: Rc::new(RefCell::new(None)),
            sidebar_settings: gtk::Button::new(),
            navigating: Rc::new(Cell::new(false)),
            screen: Rc::new(RefCell::new(None)),
            chat: Rc::new(RefCell::new(None)),
            settings: Rc::new(RefCell::new(None)),
            form: Rc::new(RefCell::new(None)),
            section: Rc::new(RefCell::new(SectionFocus::default())),
        }
    }

    pub fn install_toolbar(
        &self,
        toolbar: &adw::ToolbarView,
        header: &adw::HeaderBar,
        back: &gtk::Button,
    ) {
        // Move the detail content into its toolbar; both panes own a native header.
        self.detail_page.set_child(Some(toolbar));
        header.set_show_back_button(false);
        let header = header.clone();
        let back = back.clone();
        let screen = self.screen.clone();
        self.split.connect_collapsed_notify(move |split| {
            // Narrow windows need room for the contact name and call actions.
            // Keep Close available; wider windows use the desktop's controls.
            header.set_decoration_layout(split.is_collapsed().then_some(":close"));
            back.set_visible(screen.borrow().as_ref().is_some_and(|screen| match screen {
                Screen::ChatList | Screen::Welcome => false,
                Screen::Chat { .. } => split.is_collapsed(),
                _ => true,
            }));
        });
    }

    pub fn show_back(&self, screen: &Screen, has_stack: bool) -> bool {
        has_stack && (!matches!(screen, Screen::Chat { .. }) || self.split.is_collapsed())
    }

    fn ensure_sidebar(&self, manager: &Rc<AppManager>) {
        if self.list.borrow().is_some() {
            return;
        }
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&gtk::Label::new(Some("Chats"))));
        self.sidebar_settings.add_css_class("flat");
        self.sidebar_settings.add_css_class("circular");
        self.sidebar_settings.set_tooltip_text(Some("Settings"));
        let mgr = manager.clone();
        self.sidebar_settings.connect_clicked(move |_| {
            mgr.dispatch(AppAction::PushScreen {
                screen: Screen::Settings,
            })
        });
        header.pack_start(&self.sidebar_settings);
        let new_chat = gtk::Button::from_icon_name("list-add-symbolic");
        new_chat.set_tooltip_text(Some("New chat"));
        new_chat.add_css_class("circular");
        let mgr = manager.clone();
        new_chat.connect_clicked(move |_| {
            mgr.dispatch(AppAction::PushScreen {
                screen: Screen::NewChat,
            })
        });
        header.pack_end(&new_chat);
        self.sidebar_toolbar.add_top_bar(&header);
        let list = screens::chat_list::ChatListView::new(manager);
        self.sidebar_toolbar.set_content(Some(&list.root));
        *self.list.borrow_mut() = Some(list);
        let navigating = self.navigating.clone();
        let manager = manager.clone();
        self.split.connect_show_content_notify(move |split| {
            if !navigating.get() && split.is_collapsed() && !split.shows_content() {
                manager.dispatch(AppAction::UpdateScreenStack { stack: vec![] });
            }
        });
    }

    pub fn install_section_shortcuts(
        &self,
        window: &impl IsA<gtk::Widget>,
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
                if (!content.split.is_collapsed() || content.split.shows_content())
                    && crate::widgets::keyboard_list::focus_composer(content.root.upcast_ref())
                {
                    return glib::Propagation::Stop;
                }
                let Some((_owner, chat_id)) =
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
                let preferred_chat = section
                    .remembered_chat
                    .as_ref()
                    .filter(|(owner, _)| *owner == account.public_key_hex)
                    .map(|(_, id)| id.as_str());
                if (!content.split.is_collapsed() || !content.split.shows_content())
                    && crate::widgets::keyboard_list::focus_list(
                        content.root.upcast_ref(),
                        preferred_chat,
                    )
                {
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
        if focus_section(self.root.upcast_ref(), &self.section) {
            return;
        }
        let mut section = self.section.borrow_mut();
        if section.pending_composer.is_none() || section.pending_tick {
            return;
        }
        section.pending_tick = true;
        let weak = Rc::downgrade(&self.section);
        let started = std::time::Instant::now();
        self.root.add_tick_callback(move |root, _| {
            let Some(section) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // Wait for the pane animation, not for a future state change to
            // make a missing/restricted composer available and steal focus.
            if started.elapsed() >= std::time::Duration::from_secs(1) {
                let mut section = section.borrow_mut();
                section.pending_composer = None;
                section.pending_tick = false;
                return glib::ControlFlow::Break;
            }
            if focus_section(root.upcast_ref(), &section) {
                section.borrow_mut().pending_tick = false;
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub fn replace(&self, widget: &impl IsA<gtk::Widget>) {
        let focus = crate::widgets::keyboard_list::FocusBookmark::capture(self.detail.upcast_ref());
        self.chat.borrow_mut().take();
        self.settings.borrow_mut().take();
        self.form.borrow_mut().take();
        while let Some(child) = self.detail.first_child() {
            self.detail.remove(&child);
        }
        self.detail.append(widget);
        if let Some(focus) = focus {
            focus.restore(self.detail.upcast_ref());
        }
    }

    pub fn update(&self, screen: &Screen, state: &AppState, manager: &Rc<AppManager>) {
        *self.screen.borrow_mut() = Some(screen.clone());
        self.navigating.set(true);
        if state.account.is_some() {
            self.ensure_sidebar(manager);
            self.split.set_sidebar(Some(&self.sidebar_page));
            let avatar_key = state
                .account
                .clone()
                .map(|account| (account, state.preferences.clone()));
            if *self.sidebar_account.borrow() != avatar_key {
                if let Some(account) = state.account.as_ref() {
                    self.sidebar_settings
                        .set_child(Some(&build_own_avatar(account, state)));
                }
                *self.sidebar_account.borrow_mut() = avatar_key;
            }
            self.list
                .borrow_mut()
                .as_mut()
                .unwrap()
                .update(state, manager);
        } else {
            self.split.set_sidebar(None::<&adw::NavigationPage>);
            if let Some(list) = self.list.borrow_mut().as_mut() {
                list.update(state, manager);
            }
            self.sidebar_account.borrow_mut().take();
        }
        self.split
            .set_show_content(!matches!(screen, Screen::ChatList));
        self.navigating.set(false);
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
            self.chat.borrow_mut().take();
            self.settings.borrow_mut().take();
        }
        if let (Some(account), Some(chat)) = (&state.account, &state.current_chat) {
            self.section.borrow_mut().remembered_chat =
                Some((account.public_key_hex.clone(), chat.chat_id.clone()));
        }
        if matches!(screen, Screen::Settings) {
            if let Some(view) = self.settings.borrow_mut().as_mut() {
                view.update(state, manager);
                self.focus_pending_section();
                return;
            }
        }
        if let Screen::Chat { chat_id } | Screen::DirectChatInfo { chat_id } = screen {
            let mut chat = self.chat.borrow_mut();
            if let Some(view) = chat.as_mut().filter(|view| view.chat_id == *chat_id) {
                view.update(state, manager);
                self.focus_pending_section();
                return;
            }
        }

        // Background network/profile snapshots must not replace local form
        // editors. Rebuild these pages only when their own operation changes.
        let form = match screen {
            Screen::Welcome | Screen::ChatList => Some(false),
            Screen::CreateAccount => Some(state.busy.creating_account),
            Screen::RestoreAccount => Some(state.busy.restoring_session),
            Screen::NewChat => Some(state.busy.creating_chat || state.busy.accepting_invite),
            Screen::NewGroup => Some(state.busy.creating_group),
            Screen::JoinInvite => Some(state.busy.accepting_invite),
            _ => None,
        }
        .map(|busy| {
            (
                screen.clone(),
                busy,
                state.account.as_ref().map(|a| a.public_key_hex.clone()),
            )
        });
        if form.is_some() && self.form.borrow().as_ref() == form.as_ref() {
            self.focus_pending_section();
            return;
        }
        let mut chat = match screen {
            Screen::Chat { chat_id } | Screen::DirectChatInfo { chat_id } => {
                Some(screens::chat::ChatView::new(chat_id))
            }
            _ => None,
        };
        let settings = matches!(screen, Screen::Settings)
            .then(|| screens::settings::SettingsView::new(state, manager));
        let wide = chat.is_some() || settings.is_some();
        let widget = if let Some(chat) = chat.as_mut() {
            chat.update(state, manager);
            chat.root.clone().upcast()
        } else if let Some(settings) = settings.as_ref() {
            settings.root.clone().upcast()
        } else if matches!(screen, Screen::ChatList) {
            let empty = adw::StatusPage::builder()
                .icon_name("chat-symbolic")
                .title("Select a chat")
                .build();
            empty.upcast()
        } else {
            screens::render(screen, state, manager)
        };
        let clamp = adw::Clamp::builder()
            .maximum_size(if wide { 1100 } else { 600 })
            .tightening_threshold(if wide { 1000 } else { 560 })
            .build();
        clamp.set_child(Some(&widget));
        clamp.set_vexpand(true);
        self.replace(&clamp);
        *self.chat.borrow_mut() = chat;
        *self.settings.borrow_mut() = settings;
        *self.form.borrow_mut() = form;
        self.focus_pending_section();
    }
}

fn focus_section(root: &gtk::Widget, section: &Rc<RefCell<SectionFocus>>) -> bool {
    let mut section = section.borrow_mut();
    let focused = match section.pending_composer {
        Some(true) => crate::widgets::keyboard_list::focus_composer(root),
        Some(false) => crate::widgets::keyboard_list::focus_list(
            root,
            section.remembered_chat.as_ref().map(|(_, id)| id.as_str()),
        ),
        None => return true,
    };
    if focused {
        section.pending_composer = None;
    }
    focused
}

fn set_collapsed(split: &adw::NavigationSplitView, collapsed: bool) {
    // Changing the split reparents the pages. Preserve the focused editor's
    // selection across GTK's unmap/map cycle, just as a normal resize does.
    let input = split
        .root()
        .and_then(|root| root.focus())
        .and_downcast::<gtk::TextView>();
    let selection = input.as_ref().map(|input| {
        let buffer = input.buffer();
        (
            buffer.cursor_position(),
            buffer.iter_at_mark(&buffer.selection_bound()).offset(),
        )
    });
    split.set_collapsed(collapsed);
    if let (Some(input), Some((cursor, bound))) = (input, selection) {
        let started = std::time::Instant::now();
        input.add_tick_callback(move |input, _| {
            if started.elapsed() >= std::time::Duration::from_secs(1) {
                return glib::ControlFlow::Break;
            }
            if !input.is_mapped() {
                return glib::ControlFlow::Continue;
            }
            input.grab_focus();
            let buffer = input.buffer();
            buffer.select_range(
                &buffer.iter_at_offset(cursor),
                &buffer.iter_at_offset(bound),
            );
            glib::ControlFlow::Break
        });
    }
}
