//! Native-feeling controls: icon buttons, accent button, segmented control,
//! checkbox, sidebar rows, search field.

use super::icons::{self, Icon};
use super::theme::pal;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Margin, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, Ui,
    Vec2, text::LayoutJob, vec2,
};

/// A borderless 28×28 button showing a line icon, with a subtle hover background.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    let p = pal(ui);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.control_hover);
    }
    icons::paint(ui.painter(), rect.shrink(6.0), icon, if resp.hovered() { p.text } else { p.weak });
    resp.on_hover_text(tooltip)
}

/// Filled button in the platform accent color (the primary action).
pub fn accent_button(ui: &mut Ui, text: &str, icon: Option<Icon>, enabled: bool) -> Response {
    let p = pal(ui);
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, p.on_accent);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 24.0, 30.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let mut fill = p.accent;
    if !enabled {
        fill = fill.gamma_multiply(0.4);
    } else if resp.is_pointer_button_down_on() {
        fill = fill.gamma_multiply(0.8);
    } else if resp.hovered() {
        fill = fill.gamma_multiply(0.92);
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    let mut x = rect.left() + 12.0;
    if let Some(icon) = icon {
        icons::paint(
            painter,
            Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), Vec2::splat(14.0)),
            icon,
            p.on_accent,
        );
        x += icon_w;
    }
    painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, p.on_accent);
    resp
}

/// A plain button with an optional icon, styled like a native secondary button.
pub fn button(ui: &mut Ui, text: &str, icon: Option<Icon>, enabled: bool) -> Response {
    let p = pal(ui);
    let font = TextStyle::Button.resolve(ui.style());
    let color = if enabled { p.text } else { p.weak };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 22.0, 28.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let fill = if enabled && resp.hovered() { p.control_hover } else { p.control };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), fill);
    let mut x = rect.left() + 11.0;
    if let Some(icon) = icon {
        icons::paint(
            painter,
            Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), Vec2::splat(14.0)),
            icon,
            color,
        );
        x += icon_w;
    }
    painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
    resp
}

/// macOS/Fluent-style segmented control. Returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, String)]) -> bool {
    let p = pal(ui);
    let font = TextStyle::Button.resolve(ui.style());
    let galleys: Vec<_> =
        options.iter().map(|(_, label)| ui.painter().layout_no_wrap(label.clone(), font.clone(), p.text)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 24.0).collect();
    let total = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(7), p.control);

    let mut changed = false;
    let mut x = rect.left() + 2.0;
    for (((option, _), galley), w) in options.iter().zip(galleys).zip(widths) {
        let seg = Rect::from_min_size(egui::pos2(x, rect.top() + 2.0), vec2(w, rect.height() - 4.0));
        let resp = ui.interact(seg, ui.make_persistent_id(("seg", seg.min.x as i32, seg.min.y as i32)), Sense::click());
        let selected = *value == *option;
        if selected {
            let fill = if p.dark { Color32::from_white_alpha(38) } else { Color32::WHITE };
            let shadow = Rect::from_min_size(seg.min + vec2(0.0, 0.5), seg.size());
            ui.painter().rect_filled(
                shadow,
                CornerRadius::same(5),
                Color32::from_black_alpha(if p.dark { 40 } else { 18 }),
            );
            ui.painter().rect_filled(seg, CornerRadius::same(5), fill);
        } else if resp.hovered() {
            ui.painter().rect_filled(seg, CornerRadius::same(5), p.control);
        }
        let color = if selected { p.text } else { p.weak };
        ui.painter().galley(seg.center() - galley.size() / 2.0, galley, color);
        if resp.clicked() && !selected {
            *value = *option;
            changed = true;
        }
        x += w;
    }
    changed
}

/// Rounded checkbox filled with the accent color when checked.
pub fn checkbox(ui: &mut Ui, checked: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(vec2(18.0, 28.0), Sense::click());
    if resp.clicked() {
        *checked = !*checked;
        resp.mark_changed();
    }
    let p = pal(ui);
    let bx = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
    if *checked {
        ui.painter().rect_filled(bx, CornerRadius::same(4), p.accent);
        icons::paint(ui.painter(), bx.shrink(2.0), Icon::Check, p.on_accent);
    } else {
        ui.painter().rect_filled(bx, CornerRadius::same(4), p.input);
        ui.painter().rect_stroke(bx, CornerRadius::same(4), Stroke::new(1.0, p.border), StrokeKind::Inside);
    }
    resp
}

/// One request in the sidebar: method badge, name (truncated), unsaved dot.
pub fn sidebar_row(
    ui: &mut Ui,
    method: &str,
    method_color: Color32,
    name: &str,
    dirty: bool,
    selected: bool,
) -> Response {
    let p = pal(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(6), p.selection);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), p.control);
    }
    let y = rect.center().y;
    painter.text(
        egui::pos2(rect.left() + 10.0, y),
        Align2::LEFT_CENTER,
        short_method(method),
        FontId::monospace(10.5),
        method_color,
    );

    let left = rect.left() + 52.0;
    let right = rect.right() - if dirty { 22.0 } else { 10.0 };
    let mut job = LayoutJob::single_section(
        name.to_string(),
        egui::TextFormat::simple(TextStyle::Body.resolve(ui.style()), p.text),
    );
    job.wrap.max_width = (right - left).max(10.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter.galley(egui::pos2(left, y - galley.size().y / 2.0), galley, p.text);
    if dirty {
        painter.circle_filled(egui::pos2(rect.right() - 12.0, y), 3.5, p.orange);
    }
    resp
}

fn short_method(m: &str) -> &str {
    match m {
        "DELETE" => "DEL",
        "OPTIONS" => "OPT",
        "PATCH" => "PATCH",
        _ => m,
    }
}

/// Rounded search field with a magnifier icon.
pub fn search_field(ui: &mut Ui, text: &mut String, width: f32) -> Response {
    let p = pal(ui);
    egui::Frame::new()
        .fill(p.input)
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(8, 0))
        .show(ui, |ui| {
            ui.set_width(width - 16.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 28.0), Sense::hover());
                icons::paint(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(13.0)), Icon::Search, p.weak);
                ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text("Search")
                        .frame(egui::Frame::NONE)
                        .align(Align2::LEFT_CENTER)
                        .min_size(vec2(0.0, 28.0))
                        .desired_width(f32::INFINITY),
                )
            })
            .inner
        })
        .inner
}

/// Text field with a native-looking rounded frame and a focus ring.
pub fn text_field<'t>(text: &'t mut dyn egui::TextBuffer) -> egui::TextEdit<'t> {
    egui::TextEdit::singleline(text)
        .margin(Margin::symmetric(8, 5))
        .align(Align2::LEFT_CENTER)
        .min_size(vec2(0.0, 28.0))
}

/// Small rounded label with a tinted background, e.g. an HTTP status.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(16.0, 8.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), color.gamma_multiply(0.16));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

/// Small uppercase-style section caption, like sidebar headers on macOS.
pub fn caption(ui: &mut Ui, text: &str) {
    let p = pal(ui);
    ui.label(egui::RichText::new(text).size(11.0).color(p.weak));
}

/// Makes an empty area drag the window (the title bar is hidden on macOS).
pub fn window_drag_area(ui: &mut Ui, rect: Rect, id: &str) {
    let resp = ui.interact(rect, ui.make_persistent_id(id), Sense::click_and_drag());
    if resp.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
}

/// Thin chevron for combo boxes, instead of egui's filled triangle.
pub fn chevron(ui: &Ui, rect: Rect, _visuals: &egui::style::WidgetVisuals, _open: bool) {
    let p = pal(ui);
    let c = rect.center();
    let stroke = Stroke::new(1.4, p.weak);
    ui.painter().line_segment([c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0)], stroke);
    ui.painter().line_segment([c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)], stroke);
}
