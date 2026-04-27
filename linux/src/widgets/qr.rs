use adw::prelude::*;
use qrcode::QrCode;

pub fn build(text: &str, size: i32) -> gtk::Widget {
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_halign(gtk::Align::Center);

    let code = QrCode::new(text.as_bytes());

    area.set_draw_func(move |_, ctx, w, h| {
        let dim = w.min(h) as f64;
        ctx.set_source_rgb(1.0, 1.0, 1.0);
        let _ = ctx.paint();

        let Ok(code) = code.as_ref() else {
            return;
        };

        let modules = code.width();
        if modules == 0 {
            return;
        }
        let quiet = 4usize;
        let total = modules + quiet * 2;
        let scale = dim / total as f64;

        ctx.set_source_rgb(0.0, 0.0, 0.0);
        let bits: Vec<bool> = code
            .to_colors()
            .into_iter()
            .map(|c| matches!(c, qrcode::Color::Dark))
            .collect();
        for y in 0..modules {
            for x in 0..modules {
                if bits[y * modules + x] {
                    let px = (quiet + x) as f64 * scale;
                    let py = (quiet + y) as f64 * scale;
                    ctx.rectangle(px, py, scale, scale);
                }
            }
        }
        let _ = ctx.fill();
    });

    area.upcast()
}
