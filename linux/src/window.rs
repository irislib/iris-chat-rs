use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use ndr_demo_core::{AppAction, AppState, AppUpdate, Screen};

use crate::app_manager::AppManager;
use crate::screens;

pub fn build_ui(app: &adw::Application) {
    let manager = Rc::new(AppManager::new());

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(420)
        .default_height(740)
        .title("Iris Chat")
        .build();

    let header = adw::HeaderBar::new();
    let title_label = gtk::Label::new(None);
    title_label.add_css_class("heading");
    header.set_title_widget(Some(&title_label));

    let back_button = gtk::Button::from_icon_name("go-previous-symbolic");
    back_button.set_tooltip_text(Some("Back"));
    back_button.set_visible(false);
    {
        let manager = manager.clone();
        back_button.connect_clicked(move |_| {
            manager.dispatch(AppAction::UpdateScreenStack { stack: Vec::new() });
        });
    }
    header.pack_start(&back_button);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);

    let content_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content_slot.set_vexpand(true);
    toolbar.set_content(Some(&content_slot));

    window.set_content(Some(&toolbar));

    let current = Rc::new(RefCell::new(manager.initial_state()));
    apply_state(
        &content_slot,
        &back_button,
        &title_label,
        &manager,
        &current.borrow(),
    );

    let update_rx = manager.update_rx();
    let content_for_updates = content_slot.clone();
    let back_for_updates = back_button.clone();
    let title_for_updates = title_label.clone();
    let manager_for_updates = manager.clone();
    let current_for_updates = current.clone();
    glib::MainContext::default().spawn_local(async move {
        while let Ok(update) = update_rx.recv().await {
            if let AppUpdate::FullState(state) = update {
                let mut slot = current_for_updates.borrow_mut();
                if state.rev >= slot.rev {
                    *slot = state;
                    apply_state(
                        &content_for_updates,
                        &back_for_updates,
                        &title_for_updates,
                        &manager_for_updates,
                        &slot,
                    );
                }
            }
        }
    });

    window.present();
}

fn apply_state(
    slot: &gtk::Box,
    back: &gtk::Button,
    title: &gtk::Label,
    manager: &Rc<AppManager>,
    state: &AppState,
) {
    while let Some(child) = slot.first_child() {
        slot.remove(&child);
    }

    let screen = current_screen(state);
    back.set_visible(!state.router.screen_stack.is_empty());
    title.set_label(screens::title(&screen));

    let widget = screens::render(&screen, state, manager);
    slot.append(&widget);
}

fn current_screen(state: &AppState) -> Screen {
    state
        .router
        .screen_stack
        .last()
        .cloned()
        .unwrap_or_else(|| state.router.default_screen.clone())
}
