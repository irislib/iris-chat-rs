use super::*;

#[derive(Clone)]
pub(super) struct Content {
    pub root: gtk::Box,
    chat: Rc<RefCell<Option<screens::chat::ChatView>>>,
}

impl Content {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("iris-root");
        root.set_vexpand(true);
        Self {
            root,
            chat: Rc::new(RefCell::new(None)),
        }
    }

    pub fn replace(&self, widget: &impl IsA<gtk::Widget>) {
        self.chat.borrow_mut().take();
        while let Some(child) = self.root.first_child() {
            self.root.remove(&child);
        }
        self.root.append(widget);
    }

    pub fn update(&self, screen: &Screen, state: &AppState, manager: &Rc<AppManager>) {
        if let Screen::Chat { chat_id } | Screen::DirectChatInfo { chat_id } = screen {
            let mut chat = self.chat.borrow_mut();
            if let Some(view) = chat.as_mut().filter(|view| view.chat_id == *chat_id) {
                view.update(state, manager);
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
    }
}
