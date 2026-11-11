//! Native look: system font, platform accent color, light/dark palettes and the
//! window material (vibrancy on macOS, Mica on Windows).

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Vec2};
use std::sync::atomic::{AtomicBool, Ordering};

/// True when the window has a native translucent material behind egui.
static GLASS: AtomicBool = AtomicBool::new(false);

pub fn glass() -> bool {
    GLASS.load(Ordering::Relaxed)
}

/// Colors that egui's `Visuals` has no slot for.
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub accent: Color32,
    pub on_accent: Color32,
    pub sidebar: Color32,
    pub content: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub input: Color32,
    pub border: Color32,
    pub separator: Color32,
    pub selection: Color32,
    pub code: Color32,
    pub green: Color32,
    pub orange: Color32,
    pub red: Color32,
    pub purple: Color32,
    pub blue: Color32,
}

const fn a(r: u8, g: u8, b: u8, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const(r, g, b, alpha)
}

pub fn palette(dark: bool) -> Palette {
    let glass = glass();
    let accent = accent(dark);
    if dark {
        Palette {
            dark,
            accent,
            on_accent: if cfg!(windows) { Color32::BLACK } else { Color32::WHITE },
            sidebar: if glass { a(30, 30, 32, 120) } else { Color32::from_rgb(37, 37, 40) },
            content: if glass { a(28, 28, 30, 245) } else { Color32::from_rgb(28, 28, 30) },
            text: Color32::from_rgb(242, 242, 247),
            weak: Color32::from_rgb(152, 152, 159),
            control: a(255, 255, 255, 18),
            control_hover: a(255, 255, 255, 30),
            input: a(255, 255, 255, 12),
            border: a(255, 255, 255, 22),
            separator: a(255, 255, 255, 20),
            selection: a(255, 255, 255, 26),
            code: a(255, 255, 255, 7),
            green: Color32::from_rgb(48, 209, 88),
            orange: Color32::from_rgb(255, 159, 10),
            red: Color32::from_rgb(255, 69, 58),
            purple: Color32::from_rgb(191, 90, 242),
            blue: Color32::from_rgb(10, 132, 255),
        }
    } else {
        Palette {
            dark,
            accent,
            on_accent: Color32::WHITE,
            sidebar: if glass { a(246, 246, 248, 110) } else { Color32::from_rgb(243, 243, 245) },
            content: if glass { a(255, 255, 255, 248) } else { Color32::WHITE },
            text: Color32::from_rgb(29, 29, 31),
            weak: Color32::from_rgb(110, 110, 115),
            control: a(0, 0, 0, 12),
            control_hover: a(0, 0, 0, 22),
            input: a(0, 0, 0, 8),
            border: a(0, 0, 0, 20),
            separator: a(0, 0, 0, 18),
            selection: a(0, 0, 0, 18),
            code: a(0, 0, 0, 6),
            green: Color32::from_rgb(36, 160, 70),
            orange: Color32::from_rgb(222, 128, 0),
            red: Color32::from_rgb(228, 52, 42),
            purple: Color32::from_rgb(160, 70, 210),
            blue: Color32::from_rgb(0, 122, 255),
        }
    }
}

/// Default accent of each platform (macOS blue, Windows 11 accent blue).
fn accent(dark: bool) -> Color32 {
    match (cfg!(windows), dark) {
        (true, false) => Color32::from_rgb(0, 95, 184),
        (true, true) => Color32::from_rgb(96, 205, 255),
        (false, false) => Color32::from_rgb(0, 122, 255),
        (false, true) => Color32::from_rgb(10, 132, 255),
    }
}

pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

/// Applies the native window material. Must run once the window exists.
pub fn apply_material(window: &impl raw_window_handle::HasWindowHandle, dark: bool) {
    #[cfg(target_os = "macos")]
    {
        let _ = dark;
        GLASS.store(macos::add_vibrancy(window), Ordering::Relaxed);
    }
    #[cfg(target_os = "windows")]
    {
        let ok = window_vibrancy::apply_mica(window, Some(dark));
        GLASS.store(ok.is_ok(), Ordering::Relaxed);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (window, dark);
    }
}

/// Uses the platform UI font (SF Pro / Segoe UI) and monospace font when available.
pub fn install_fonts(ctx: &egui::Context) {
    let (ui, mono): (&[&str], &[&str]) = if cfg!(target_os = "macos") {
        (
            &["/System/Library/Fonts/SFNS.ttf"],
            &["/System/Library/Fonts/SFNSMono.ttf", "/System/Library/Fonts/Menlo.ttc"],
        )
    } else if cfg!(windows) {
        (
            &["C:\\Windows\\Fonts\\SegUIVar.ttf", "C:\\Windows\\Fonts\\segoeui.ttf"],
            &["C:\\Windows\\Fonts\\CascadiaMono.ttf", "C:\\Windows\\Fonts\\consola.ttf"],
        )
    } else {
        (&[], &[])
    };
    for (paths, family) in [(ui, FontFamily::Proportional), (mono, FontFamily::Monospace)] {
        if let Some(bytes) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
            ctx.add_font(egui::epaint::text::FontInsert::new(
                &format!("system-{family:?}"),
                egui::FontData::from_owned(bytes),
                vec![egui::epaint::text::InsertFontFamily {
                    family,
                    priority: egui::epaint::text::FontPriority::Highest,
                }],
            ));
        }
    }
}

pub fn install_style(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let p = palette(theme == egui::Theme::Dark);
        ctx.style_mut_of(theme, |s| style(s, &p));
    }
}

fn style(s: &mut egui::Style, p: &Palette) {
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Heading, FontId::proportional(17.0)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();

    let sp = &mut s.spacing;
    sp.item_spacing = Vec2::new(8.0, 6.0);
    sp.button_padding = Vec2::new(10.0, 4.0);
    sp.interact_size = Vec2::new(28.0, 26.0);
    sp.window_margin = Margin::same(16);
    sp.menu_margin = Margin::same(6);
    sp.combo_width = 90.0;
    sp.icon_width = 16.0;
    sp.icon_width_inner = 10.0;
    sp.scroll.bar_width = 8.0;
    sp.scroll.floating = true;

    let v = &mut s.visuals;
    v.dark_mode = p.dark;
    v.override_text_color = None;
    v.panel_fill = p.content;
    v.window_fill = if p.dark { Color32::from_rgb(38, 38, 41) } else { Color32::from_rgb(250, 250, 252) };
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 10], blur: 40, spread: 0, color: a(0, 0, 0, if p.dark { 120 } else { 50 }) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 20, spread: 0, color: a(0, 0, 0, if p.dark { 100 } else { 40 }) };
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.faint_bg_color = p.code;
    v.code_bg_color = p.code;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(if p.dark { 0.45 } else { 0.25 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(1.5, p.accent);
    v.warn_fg_color = p.orange;
    v.error_fg_color = p.red;
    v.striped = false;
    v.window_highlight_topmost = false;
    v.indent_has_left_vline = false;

    let r = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.content;
    w.noninteractive.weak_bg_fill = p.content;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;

    w.hovered.bg_fill = p.control_hover;
    w.hovered.weak_bg_fill = p.control_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.selection;
    w.active.weak_bg_fill = p.selection;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open = w.hovered;
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
        NSVisualEffectView, NSWindowOrderingMode,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// Puts an NSVisualEffectView *behind* the content view (which hosts egui's Metal layer).
    /// Adding it inside the content view would cover what egui draws.
    pub fn add_vibrancy(window: &impl HasWindowHandle) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let Ok(handle) = window.window_handle() else { return false };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else { return false };
        // SAFETY: the handle comes from a live winit window, on the main thread.
        let content: &NSView = unsafe { appkit.ns_view.cast().as_ref() };
        let Some(frame_view) = (unsafe { content.superview() }) else { return false };

        let effect = NSVisualEffectView::initWithFrame(mtm.alloc(), frame_view.bounds());
        effect.setMaterial(NSVisualEffectMaterial::Sidebar);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        effect.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        frame_view.addSubview_positioned_relativeTo(&effect, NSWindowOrderingMode::Below, Some(content));
        true
    }
}
