use super::*;

const PAGES: [SettingsPage; 12] = [
    SettingsPage::Profile,
    SettingsPage::General,
    SettingsPage::Devices,
    SettingsPage::Messaging,
    SettingsPage::Notifications,
    SettingsPage::Media,
    SettingsPage::Nearby,
    SettingsPage::MessageServers,
    SettingsPage::Updates,
    SettingsPage::About,
    SettingsPage::Support,
    SettingsPage::AccountData,
];

struct Rendered {
    account: Option<iris_chat_core::AccountSnapshot>,
    preferences: PreferencesSnapshot,
    devices: Option<iris_chat_core::DeviceRosterSnapshot>,
    roster_busy: bool,
    network: Option<iris_chat_core::NetworkStatusSnapshot>,
}

pub(crate) struct SettingsView {
    pub root: gtk::Box,
    stack: gtk::Stack,
    split: adw::NavigationSplitView,
    menu: gtk::ScrolledWindow,
    media: Option<MediaGroup>,
    rendered: Option<Rendered>,
}

impl SettingsView {
    pub fn new(state: &AppState, manager: &Rc<AppManager>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_vexpand(true);
        root.set_hexpand(true);
        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        stack.set_hhomogeneous(false);
        stack.set_vhomogeneous(false);
        let menu = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let split = adw::NavigationSplitView::new();
        split.set_widget_name("settings-navigation");
        split.set_vexpand(true);
        split.set_sidebar_width_unit(adw::LengthUnit::Px);
        split.set_min_sidebar_width(220.0);
        split.set_max_sidebar_width(260.0);
        split.set_sidebar_width_fraction(0.33);
        split.set_sidebar(Some(&adw::NavigationPage::new(&menu, "Settings")));
        let toolbar = adw::ToolbarView::new();
        let header = adw::HeaderBar::new();
        header.set_show_start_title_buttons(false);
        header.set_show_end_title_buttons(false);
        header.set_visible(false);
        let title = gtk::Label::new(None);
        header.set_title_widget(Some(&title));
        let title = title.downgrade();
        stack.connect_visible_child_name_notify(move |stack| {
            if let (Some(title), Some(page)) = (
                title.upgrade(),
                PAGES
                    .iter()
                    .find(|page| Some(page.id()) == stack.visible_child_name().as_deref()),
            ) {
                title.set_label(page.title());
            }
        });
        toolbar.add_top_bar(&header);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&stack)
            .build();
        scroll.set_widget_name("settings-detail-scroll");
        scroll.set_vexpand(true);
        toolbar.set_content(Some(&scroll));
        split.set_content(Some(&adw::NavigationPage::new(&toolbar, "Settings")));
        let header = header.downgrade();
        split.connect_collapsed_notify(move |split| {
            if let Some(header) = header.upgrade() {
                header.set_visible(split.is_collapsed());
            }
        });
        let responsive = adw::BreakpointBin::new();
        responsive.set_vexpand(true);
        responsive.set_size_request(300, 200);
        responsive.set_child(Some(&split));
        let breakpoint =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 679sp").unwrap());
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        responsive.add_breakpoint(breakpoint);
        root.append(&responsive);
        let mut view = Self {
            root,
            stack,
            split,
            menu,
            media: None,
            rendered: None,
        };
        view.update(state, manager);
        view
    }

    pub fn update(&mut self, state: &AppState, manager: &Rc<AppManager>) {
        let account_changed = self
            .rendered
            .as_ref()
            .is_none_or(|old| old.account != state.account);
        let preferences_changed = self
            .rendered
            .as_ref()
            .is_none_or(|old| old.preferences != state.preferences);
        let selected = self
            .stack
            .visible_child_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| {
                if state.account.is_some() {
                    SettingsPage::Profile
                } else {
                    SettingsPage::Messaging
                }
                .id()
                .to_owned()
            });
        if account_changed {
            self.menu
                .set_child(Some(&settings_menu(state, &self.stack, &self.split)));
        }
        for page in PAGES {
            if matches!(page, SettingsPage::Media) {
                if let Some(media) = self.media.as_mut() {
                    media.update(&state.preferences);
                } else {
                    let media = media_group(&state.preferences, manager);
                    self.stack.add_named(
                        &settings_detail_page(vec![media.group.clone()]),
                        Some(page.id()),
                    );
                    self.media = Some(media);
                }
                continue;
            }
            let changed = self.rendered.as_ref().is_none_or(|old| match page {
                SettingsPage::Profile => account_changed || preferences_changed,
                SettingsPage::General
                | SettingsPage::Messaging
                | SettingsPage::Notifications
                | SettingsPage::Media
                | SettingsPage::Nearby
                | SettingsPage::MessageServers => preferences_changed,
                SettingsPage::Devices => {
                    old.devices != state.device_roster
                        || old.roster_busy != state.busy.updating_roster
                }
                SettingsPage::About => old.network != state.network_status,
                _ => false,
            });
            if changed {
                if let Some(previous) = self.stack.child_by_name(page.id()) {
                    self.stack.remove(&previous);
                }
                self.stack
                    .add_named(&page_widget(page, state, manager), Some(page.id()));
            }
        }
        self.stack.set_visible_child_name(&selected);
        self.rendered = Some(Rendered {
            account: state.account.clone(),
            preferences: state.preferences.clone(),
            devices: state.device_roster.clone(),
            roster_busy: state.busy.updating_roster,
            network: state.network_status.clone(),
        });
    }
}

#[cfg(feature = "ui-tests")]
mod tests;
#[cfg(feature = "ui-tests")]
pub use tests::verify_ui;
