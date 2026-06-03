use adw::prelude::*;
use gtk::gio;

pub(super) struct LinkifiedText {
    pub markup: String,
    pub urls: Vec<String>,
}

pub(super) fn linkified_text(text: &str) -> LinkifiedText {
    let mut markup = String::new();
    let mut urls = Vec::new();

    for part in text.split_inclusive(char::is_whitespace) {
        let token = part.trim_end();
        let whitespace = &part[token.len()..];
        if let Some((prefix, visible, suffix, href)) = split_link_token(token) {
            markup.push_str(&glib::markup_escape_text(prefix));
            push_link_markup(&mut markup, visible, &href);
            markup.push_str(&glib::markup_escape_text(suffix));
            if !urls.iter().any(|url| url == &href) {
                urls.push(href);
            }
        } else {
            markup.push_str(&glib::markup_escape_text(token));
        }
        markup.push_str(&glib::markup_escape_text(whitespace));
    }

    LinkifiedText { markup, urls }
}

fn split_link_token(token: &str) -> Option<(&str, &str, &str, String)> {
    let start = token
        .char_indices()
        .find_map(|(idx, ch)| ch.is_ascii_alphanumeric().then_some(idx))?;
    let (prefix, candidate) = token.split_at(start);
    if !(candidate.starts_with("https://")
        || candidate.starts_with("http://")
        || candidate.starts_with("www."))
    {
        return None;
    }

    let visible = candidate
        .trim_end_matches(|ch| matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']'));
    if visible.is_empty() {
        return None;
    }
    let suffix = &candidate[visible.len()..];
    Some((prefix, visible, suffix, normalized_link_href(visible)))
}

fn normalized_link_href(token: &str) -> String {
    if token.starts_with("www.") {
        format!("https://{token}")
    } else {
        token.to_string()
    }
}

fn push_link_markup(markup: &mut String, visible: &str, href: &str) {
    markup.push_str(&format!(
        "<a href=\"{}\">{}</a>",
        glib::markup_escape_text(href),
        glib::markup_escape_text(visible)
    ));
}

pub(super) fn install_link_actions(label: &gtk::Label, urls: Vec<String>) {
    if urls.is_empty() {
        return;
    }

    label.set_cursor_from_name(Some("pointer"));
    label.connect_activate_link(move |_, uri| {
        open_url(uri);
        gtk::glib::Propagation::Stop
    });

    let popover = build_link_popover(urls);
    popover.set_parent(label);

    let popover_for_click = popover.clone();
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    gesture.connect_pressed(move |gesture, _, x, y| {
        popover_for_click
            .set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover_for_click.popup();
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    label.add_controller(gesture);
}

fn build_link_popover(urls: Vec<String>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_margin_top(6);
    column.set_margin_bottom(6);
    column.set_margin_start(6);
    column.set_margin_end(6);

    let multiple = urls.len() > 1;
    for url in urls {
        let display = compact_url(&url);
        let open_label = if multiple {
            format!("Open {display}")
        } else {
            "Open link".to_string()
        };
        let open = gtk::Button::with_label(&open_label);
        open.add_css_class("flat");
        open.set_halign(gtk::Align::Fill);
        let url_for_open = url.clone();
        let popover_for_open = popover.clone();
        open.connect_clicked(move |_| {
            open_url(&url_for_open);
            popover_for_open.popdown();
        });
        column.append(&open);

        let copy_label = if multiple {
            format!("Copy {display}")
        } else {
            "Copy link".to_string()
        };
        let copy = gtk::Button::with_label(&copy_label);
        copy.add_css_class("flat");
        copy.set_halign(gtk::Align::Fill);
        let url_for_copy = url.clone();
        let popover_for_copy = popover.clone();
        copy.connect_clicked(move |_| {
            crate::platform::clipboard::copy(&url_for_copy);
            popover_for_copy.popdown();
        });
        column.append(&copy);
    }

    popover.set_child(Some(&column));
    popover
}

fn compact_url(value: &str) -> String {
    if value.chars().count() <= 18 {
        value.to_string()
    } else {
        let prefix = value.chars().take(8).collect::<String>();
        let suffix = value
            .chars()
            .rev()
            .take(6)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("{prefix}…{suffix}")
    }
}

fn open_url(url: &str) {
    if let Err(error) = gio::AppInfo::launch_default_for_uri(url, gio::AppLaunchContext::NONE) {
        eprintln!("Failed to open link {url}: {error}");
    }
}
