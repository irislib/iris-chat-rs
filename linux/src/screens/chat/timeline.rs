use super::*;
use std::collections::HashMap;

pub(super) struct Timeline {
    pub viewport: crate::widgets::message_timeline::MessageTimeline,
    rows: HashMap<String, (RowKey, gtk::Widget)>,
    empty: gtk::Label,
    context: Option<(CurrentChatSnapshot, PreferencesSnapshot)>,
}
#[derive(PartialEq)]
struct RowKey {
    message: ChatMessageSnapshot,
    day: Option<String>,
    start: bool,
    end: bool,
    footer: bool,
}
impl Timeline {
    pub fn new(chat_id: &str, manager: &Rc<AppManager>) -> Self {
        let manager = Rc::downgrade(manager);
        let chat_id = chat_id.to_owned();
        let empty = gtk::Label::new(Some("No messages yet"));
        empty.add_css_class("dim-label");
        empty.set_vexpand(true);
        empty.set_valign(gtk::Align::Center);
        Self {
            empty,
            viewport: crate::widgets::message_timeline::MessageTimeline::new(move || {
                if let Some(manager) = manager.upgrade() {
                    manager.load_older_messages(&chat_id);
                }
            }),
            rows: HashMap::new(),
            context: None,
        }
    }
    pub fn update(
        &mut self,
        chat: &CurrentChatSnapshot,
        prefs: &PreferencesSnapshot,
        manager: &Rc<AppManager>,
    ) {
        let mut context = chat.clone();
        context.messages.clear();
        context.draft.clear();
        context.typing_indicators.clear();
        context.direct_chat_capability = None;
        let context = (context, prefs.clone());
        if self.context.as_ref() != Some(&context) {
            self.rows.clear();
            self.context = Some(context);
        }
        let mut rows = Vec::with_capacity(chat.messages.len());
        let mut retained = HashMap::with_capacity(chat.messages.len());
        for (index, message) in chat.messages.iter().enumerate() {
            let previous = index.checked_sub(1).and_then(|i| chat.messages.get(i));
            let next = chat.messages.get(index + 1);
            let day = day_label_secs(message.created_at_secs);
            let key = RowKey {
                message: message.clone(),
                day: previous
                    .filter(|p| day_label_secs(p.created_at_secs) == day)
                    .is_none()
                    .then_some(day),
                start: previous.is_none_or(|p| grouping::cluster_break(p, message, &chat.kind)),
                end: next.is_none_or(|n| grouping::cluster_break(message, n, &chat.kind)),
                footer: grouping::show_footer(message, next, &chat.kind),
            };
            let row = match self.rows.remove(&message.id) {
                Some((old, widget)) if old == key => widget,
                _ => {
                    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    if let Some(day) = &key.day {
                        row.append(&day_chip(day));
                    }
                    row.append(&render_message(
                        message,
                        chat,
                        key.start,
                        key.end,
                        key.footer,
                        unix_now(),
                        prefs,
                        manager,
                    ));
                    row.upcast()
                }
            };
            rows.push((message.id.clone(), row.clone()));
            retained.insert(message.id.clone(), (key, row));
        }
        self.rows = retained;
        if rows.is_empty() {
            rows.push(("empty".into(), self.empty.clone().upcast()));
        }
        self.viewport.update(rows);
    }
}
