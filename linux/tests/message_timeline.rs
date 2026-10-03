use adw::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

#[path = "../src/widgets/message_timeline.rs"]
mod viewport;

pub fn run() {
    let requests = Rc::new(Cell::new(0));
    let count = requests.clone();
    let timeline = viewport::MessageTimeline::new(move || count.set(count.get() + 1));
    let window = adw::Window::builder()
        .default_width(420)
        .default_height(420)
        .build();
    window.set_content(Some(&timeline.scroll));
    let rows: Vec<_> = (0..240)
        .map(|i| {
            let row = gtk::Label::new(Some(&format!(
                "Synthetic message {i}: {}",
                "longer text ".repeat(i % 7)
            )));
            row.set_wrap(true);
            row.set_xalign(0.0);
            row.set_height_request(24 + (i % 5) as i32 * 13);
            (i.to_string(), row.upcast::<gtk::Widget>())
        })
        .collect();
    timeline.update(rows[160..].to_vec());
    window.present();
    settle();
    let adj = timeline.scroll.vadjustment();
    near(
        adj.value(),
        adj.upper() - adj.page_size(),
        "Initial page opens at latest",
    );
    adj.set_value(80.0);
    settle();
    assert!(
        requests.get() > 0,
        "Near-top scrolling requests older history"
    );
    let anchor = rows[163].1.clone();
    let position = || f64::from(anchor.compute_bounds(&timeline.scroll).unwrap().y());
    let before = position();
    timeline.update(rows[80..].to_vec());
    settle();
    near(
        position(),
        before,
        "Second page retains visible row position",
    );
    let before = position();
    timeline.update(rows.clone());
    settle();
    near(
        position(),
        before,
        "Third page retains position beyond 80 messages",
    );
    assert_eq!(anchor, rows[163].1, "Existing rows are retained");
    let incoming = gtk::Label::new(Some("New incoming message"));
    incoming.set_height_request(90);
    let mut updated = rows.clone();
    updated.push(("new".into(), incoming.upcast()));
    let before = position();
    timeline.update(updated.clone());
    settle();
    near(
        position(),
        before,
        "Incoming message does not move browsed history",
    );
    // Asynchronous image/expanded text growth above the viewport.
    rows[5].1.set_height_request(180);
    settle();
    near(
        position(),
        before,
        "Late row growth preserves visible anchor",
    );
    timeline.follow_latest();
    settle();
    near(
        adj.value(),
        adj.upper() - adj.page_size(),
        "Own send returns to latest",
    );
    let extra = gtk::Label::new(Some("Another incoming message"));
    extra.set_height_request(80);
    updated.push(("another".into(), extra.upcast()));
    timeline.update(updated);
    settle();
    near(
        adj.value(),
        adj.upper() - adj.page_size(),
        "At-bottom arrival follows latest",
    );
    let weak = timeline.scroll.downgrade();
    window.set_content(gtk::Widget::NONE);
    window.close();
    drop(timeline);
    drop(rows);
    drop(anchor);
    drop(adj);
    drop(window);
    settle();
    assert!(
        weak.upgrade().is_none(),
        "Timeline callbacks must release disposed chat widgets"
    );
    println!("PASS: GTK 240 mixed-height rows, two older pages, stable anchors, incoming/own-send, late growth, disposal");
}
fn near(actual: f64, expected: f64, context: &str) {
    assert!(
        (actual - expected).abs() <= 2.0,
        "{context}: {actual} vs {expected}"
    );
}
fn settle() {
    let context = gtk::glib::MainContext::default();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(120);
    while std::time::Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
