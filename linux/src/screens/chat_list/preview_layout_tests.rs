use super::*;
use std::time::{Duration, Instant};

pub fn verify_ui(manager: Rc<AppManager>) {
    let state = manager.current_state();
    let original = state.chat_list.first().expect("test chat").clone();
    let window = gtk::Window::builder()
        .title("Chat previews")
        .default_width(352)
        .default_height(240)
        .build();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let list = gtk::ListBox::new();
    let next = gtk::Label::new(Some("Following chat"));
    next.set_height_request(40);
    root.append(&list);
    root.append(&next);
    window.set_child(Some(&root));
    let css = gtk::CssProvider::new();
    css.load_from_data(".preview-layout-large label { font-size: 24px; }");
    gtk::style_context_add_provider_for_display(
        &window.display(),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    window.present();
    let samples = [
        "".to_string(),
        "Short preview".into(),
        "A preview that wraps onto another line. ".repeat(5),
        "One\nTwo\nThree".into(),
    ];
    for width in [280, 352, 520] {
        for large in [false, true] {
            if large {
                list.add_css_class("preview-layout-large");
            } else {
                list.remove_css_class("preview-layout-large");
            }
            root.set_size_request(width, -1);
            window.set_default_size(width, 240);
            let mut expected = None;
            for preview in &samples {
                for draft in [false, true] {
                    for typing in [false, true] {
                        let mut chat = original.clone();
                        chat.last_message_preview = Some(preview.clone());
                        chat.draft = if draft {
                            preview.clone()
                        } else {
                            String::new()
                        };
                        chat.is_typing = typing;
                        chat.unread_count = if draft { 123 } else { 0 };
                        let row = row_for(
                            &chat,
                            &state.preferences,
                            unix_now(),
                            &manager,
                            &Timestamps::default(),
                        );
                        list.append(&row);
                        let deadline = Instant::now() + Duration::from_secs(10);
                        loop {
                            while gtk::glib::MainContext::default().pending() {
                                gtk::glib::MainContext::default().iteration(false);
                            }
                            if row.height() > 0 {
                                break;
                            }
                            assert!(
                                Instant::now() < deadline,
                                "chat preview row did not allocate"
                            );
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        let geometry = (row.height(), next.compute_bounds(&root).unwrap().y());
                        let baseline = *expected.get_or_insert(geometry);
                        assert_eq!(
                            geometry.0, baseline.0,
                            "GTK preview changed row height at width {width}, large={large}"
                        );
                        assert!(
                            (geometry.1 - baseline.1).abs() <= 0.5,
                            "GTK preview moved the following row"
                        );
                        list.remove(&row);
                    }
                }
            }
        }
    }
    window.close();
    gtk::style_context_remove_provider_for_display(&window.display(), &css);
    println!("PASS: GTK preview/typing/draft/newline rows and following position stay stable at 3 widths and 2 font sizes");
}
