//! Small line icons drawn with the painter (no icon font needed, crisp at any scale).

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Plus,
    Close,
    Search,
    Gear,
    Copy,
    Trash,
    Check,
    Send,
    Download,
}

/// Draws `icon` centered in `rect`, on a 16×16 design grid scaled to the rect.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 16.0;
    let o = rect.center() - vec2(8.0, 8.0) * s;
    let p = |x: f32, y: f32| -> Pos2 { o + vec2(x, y) * s };
    let stroke = Stroke::new(1.4 * s.max(1.0), color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);

    match icon {
        Icon::Plus => {
            line(p(8.0, 3.0), p(8.0, 13.0));
            line(p(3.0, 8.0), p(13.0, 8.0));
        }
        Icon::Close => {
            line(p(4.0, 4.0), p(12.0, 12.0));
            line(p(12.0, 4.0), p(4.0, 12.0));
        }
        Icon::Search => {
            painter.circle_stroke(p(7.0, 7.0), 4.2 * s, stroke);
            line(p(10.2, 10.2), p(13.5, 13.5));
        }
        Icon::Gear => {
            let c = p(8.0, 8.0);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = Vec2::angled(a);
                painter.line_segment([c + d * 4.4 * s, c + d * 6.6 * s], Stroke::new(2.0 * s.max(1.0), color));
            }
            painter.circle_stroke(c, 4.4 * s, stroke);
            painter.circle_stroke(c, 1.8 * s, stroke);
        }
        Icon::Copy => {
            let r = CornerRadius::same((2.0 * s) as u8);
            painter.rect_stroke(Rect::from_min_max(p(5.5, 5.5), p(13.0, 13.0)), r, stroke, StrokeKind::Middle);
            line(p(3.0, 10.5), p(3.0, 4.5));
            line(p(3.0, 3.0), p(10.5, 3.0));
            painter.circle_filled(p(3.0, 3.0), 0.7 * s, color);
        }
        Icon::Trash => {
            line(p(2.5, 4.5), p(13.5, 4.5));
            line(p(6.0, 4.5), p(6.5, 2.5));
            line(p(6.5, 2.5), p(9.5, 2.5));
            line(p(9.5, 2.5), p(10.0, 4.5));
            painter.add(Shape::line(vec![p(4.0, 4.5), p(4.8, 13.5), p(11.2, 13.5), p(12.0, 4.5)], stroke));
            line(p(6.8, 7.0), p(7.0, 11.0));
            line(p(9.2, 7.0), p(9.0, 11.0));
        }
        Icon::Check => {
            painter.add(Shape::line(vec![p(3.5, 8.5), p(6.5, 11.5), p(12.5, 4.5)], Stroke::new(1.8 * s, color)));
        }
        Icon::Send => {
            painter.add(Shape::line(vec![p(8.0, 13.0), p(8.0, 3.5)], stroke));
            painter.add(Shape::line(vec![p(4.0, 7.5), p(8.0, 3.5), p(12.0, 7.5)], stroke));
        }
        Icon::Download => {
            line(p(8.0, 2.5), p(8.0, 10.0));
            painter.add(Shape::line(vec![p(4.5, 6.8), p(8.0, 10.3), p(11.5, 6.8)], stroke));
            line(p(3.0, 13.0), p(13.0, 13.0));
        }
    }
    let _ = pos2;
}
