#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod http;
mod menu;
mod preview;
mod store;
mod ui;
mod updater;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, Rect, RichText, Stroke, Vec2, vec2,
};
use menu::Action;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use store::{Env, Envs, Header, METHODS, Request, Settings, Store, Theme, Var};
use ui::body::{self, Kind, View};
use ui::icons::Icon;
use ui::theme::{self, Palette, pal};
use ui::widgets as w;

/// On macOS the content extends under the title bar; leave room for the window buttons.
const TITLEBAR_INSET: f32 = if cfg!(target_os = "macos") { 38.0 } else { 10.0 };
const TOOLBAR_HEIGHT: f32 = 52.0;

/// A REST client needs no discrete GPU. By default wgpu asks for the high-performance one,
/// which on laptops wakes up the dedicated GPU and loads its (large) driver: prefer the
/// integrated GPU instead, and on Windows use DirectX 12 only (the Vulkan loader pulls in
/// every installed overlay layer). `WGPU_BACKEND` / `WGPU_POWER_PREF` still override this.
fn gpu_options() -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew, wgpu};
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.power_preference = wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::LowPower);
    if cfg!(windows) {
        setup.instance_descriptor.backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12 | wgpu::Backends::GL);
    }
    eframe::WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..Default::default() }
}

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Plume")
        .with_inner_size([1120.0, 740.0])
        .with_min_inner_size([760.0, 460.0])
        .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default());
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        viewport = viewport.with_transparent(true);
    }
    if cfg!(target_os = "macos") {
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    let options = eframe::NativeOptions { viewport, wgpu_options: gpu_options(), ..Default::default() };
    eframe::run_native("Plume", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[derive(PartialEq, Clone, Copy)]
enum ReqTab {
    Headers,
    Body,
}

#[derive(PartialEq, Clone, Copy)]
enum RespTab {
    Body,
    Headers,
}

struct Shown {
    resp: http::Response,
    body: body::Body,
    /// Unique per response (tells the HTML preview when to reload).
    seq: u64,
}

enum Outcome {
    Ok(Box<Shown>),
    Err(String),
}

enum Update {
    Idle,
    Checking { manual: bool },
    UpToDate,
    Available(updater::Release),
    Installing,
    Installed,
    Failed(String),
}

struct App {
    store: Store,
    saved: Vec<Request>,
    /// The request being edited (empty id = new, not saved yet).
    cur: Request,
    /// Unsaved edits of other requests, kept in memory while switching.
    drafts: HashMap<String, Request>,
    responses: HashMap<String, Outcome>,
    pending: Option<(String, Receiver<Result<http::Response, String>>)>,
    envs: Envs,
    settings: Settings,
    search: String,
    req_tab: ReqTab,
    resp_tab: RespTab,
    show_envs: bool,
    show_settings: bool,
    show_about: bool,
    /// Where each secondary window opens (computed once, when it is opened).
    window_pos: HashMap<&'static str, egui::Pos2>,
    /// When each secondary window appeared: it is repainted for a moment, because macOS
    /// may show a new window after its first frame was drawn (leaving it blank).
    window_opened: HashMap<&'static str, Instant>,
    icon: Option<egui::TextureHandle>,
    #[cfg(target_os = "macos")]
    native_menu: menu::NativeMenu,
    edit_env: usize,
    confirm_delete: bool,
    message: Option<(String, bool, Instant)>,
    update: Update,
    update_rx: Option<Receiver<Update>>,
    exe: Option<PathBuf>,
    /// Dark mode the window material was last applied with (Mica needs it on Windows).
    material_dark: Option<bool>,
    /// Share of the height given to the request editor (the response gets the rest).
    split: f32,
    /// Body view picked for each kind of content (JSON → Tree, HTML → Preview…).
    views: HashMap<Kind, View>,
    seq: u64,
    /// Where the HTML preview goes this frame (set while drawing the response).
    preview_at: Option<Rect>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    preview: preview::Preview,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let ctx = &cc.egui_ctx;
        theme::install_fonts(ctx);
        let store = Store::open();
        let saved = store.load_requests();
        let envs = store.load_envs();
        let settings = store.load_settings();
        let mut app = Self {
            store,
            saved,
            cur: Request::default(),
            drafts: HashMap::new(),
            responses: HashMap::new(),
            pending: None,
            envs,
            settings,
            search: String::new(),
            req_tab: ReqTab::Body,
            resp_tab: RespTab::Body,
            show_envs: false,
            show_settings: false,
            show_about: false,
            window_pos: HashMap::new(),
            window_opened: HashMap::new(),
            icon: None,
            #[cfg(target_os = "macos")]
            native_menu: menu::NativeMenu::install(ctx),
            edit_env: 0,
            confirm_delete: false,
            message: None,
            update: Update::Idle,
            update_rx: None,
            exe: std::env::current_exe().ok(),
            material_dark: None,
            split: 0.45,
            views: HashMap::new(),
            seq: 0,
            preview_at: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            preview: Default::default(),
        };
        app.apply_theme(ctx);
        let dark = ctx.system_theme().unwrap_or(egui::Theme::Dark) == egui::Theme::Dark;
        theme::apply_material(cc, app.effective_dark(dark));
        theme::install_style(ctx);
        if app.settings.check_updates && !cfg!(debug_assertions) {
            app.check_updates(ctx, false);
        }
        // Debug builds only: open (and send) a saved request at launch, for UI screenshots.
        #[cfg(debug_assertions)]
        if let Ok(id) = std::env::var("PLUME_DEBUG_OPEN") {
            app.switch_to(&id);
            app.send(ctx);
            app.show_settings = std::env::var("PLUME_DEBUG_SETTINGS").is_ok();
            app.show_envs = std::env::var("PLUME_DEBUG_ENVS").is_ok();
            app.show_about = std::env::var("PLUME_DEBUG_ABOUT").is_ok();
            if let Ok(v) = std::env::var("PLUME_DEBUG_VIEW") {
                let view = [View::Tree, View::Pretty, View::Raw, View::Preview, View::Hex]
                    .into_iter()
                    .find(|x| x.label() == v)
                    .unwrap_or(View::Raw);
                for k in [Kind::Json, Kind::Html, Kind::Xml, Kind::Text, Kind::Image, Kind::Binary] {
                    app.views.insert(k, view);
                }
            }
            if std::env::var("PLUME_DEBUG_HEADERS").is_ok() {
                app.req_tab = ReqTab::Headers;
                app.resp_tab = RespTab::Headers;
            }
        }
        app
    }

    fn effective_dark(&self, system_dark: bool) -> bool {
        match self.settings.theme {
            Theme::Auto => system_dark,
            Theme::Light => false,
            Theme::Dark => true,
        }
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            Theme::Auto => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
    }

    fn flash(&mut self, text: impl Into<String>, error: bool) {
        self.message = Some((text.into(), error, Instant::now()));
    }

    fn saved_version(&self, id: &str) -> Request {
        self.saved.iter().find(|r| r.id == id).cloned().unwrap_or_default()
    }

    fn is_dirty(&self, r: &Request) -> bool {
        *r != self.saved_version(&r.id)
    }

    fn switch_to(&mut self, id: &str) {
        if self.cur.id == id {
            return;
        }
        let old = std::mem::take(&mut self.cur);
        if self.is_dirty(&old) {
            self.drafts.insert(old.id.clone(), old);
        } else {
            self.drafts.remove(&old.id);
        }
        self.cur = self.drafts.remove(id).unwrap_or_else(|| self.saved_version(id));
        self.confirm_delete = false;
    }

    fn new_request(&mut self, from: Option<Request>) {
        self.switch_to("");
        self.cur = from.unwrap_or_default();
        self.cur.id.clear();
        self.responses.remove("");
    }

    fn save(&mut self) {
        if self.cur.name.trim().is_empty() {
            self.cur.name = if self.cur.url.is_empty() { "Untitled".into() } else { self.cur.url.clone() };
        }
        let old_id = self.cur.id.clone();
        let mut r = self.cur.clone();
        match self.store.save_request(&mut r, &self.saved) {
            Ok(()) => {
                if old_id.is_empty() {
                    if let Some(resp) = self.responses.remove("") {
                        self.responses.insert(r.id.clone(), resp);
                    }
                    if let Some((pid, _)) = &mut self.pending
                        && pid.is_empty()
                    {
                        *pid = r.id.clone();
                    }
                }
                self.saved.retain(|s| s.id != r.id);
                self.saved.push(r.clone());
                store::sort(&mut self.saved);
                self.cur = r;
                self.flash("Saved", false);
            }
            Err(e) => self.flash(format!("Could not save: {e}"), true),
        }
    }

    fn delete(&mut self) {
        let id = self.cur.id.clone();
        if id.is_empty() {
            self.cur = Request::default();
            return;
        }
        if let Err(e) = self.store.delete_request(&id) {
            self.flash(format!("Could not delete: {e}"), true);
            return;
        }
        self.saved.retain(|s| s.id != id);
        self.responses.remove(&id);
        self.cur = self.drafts.remove("").unwrap_or_default();
        self.confirm_delete = false;
    }

    fn send(&mut self, ctx: &egui::Context) {
        if self.cur.url.trim().is_empty() || self.pending.is_some() {
            return;
        }
        let e = &self.envs;
        let out = http::Outgoing {
            method: self.cur.method.clone(),
            url: e.resolve(&self.cur.url),
            headers: self
                .cur
                .headers
                .iter()
                .filter(|h| h.enabled && !h.key.trim().is_empty())
                .map(|h| (e.resolve(h.key.trim()), e.resolve(&h.value)))
                .collect(),
            body: e.resolve(&self.cur.body),
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(http::send(out));
            ctx.request_repaint();
        });
        self.pending = Some((self.cur.id.clone(), rx));
    }

    fn check_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if matches!(self.update, Update::Checking { .. } | Update::Installing | Update::Installed) {
            return;
        }
        self.update = Update::Checking { manual };
        self.spawn_update(ctx, move || match updater::check() {
            Ok(Some(release)) => Update::Available(release),
            Ok(None) => Update::UpToDate,
            Err(e) if manual => Update::Failed(e),
            Err(_) => Update::Idle, // stay quiet when the automatic check fails (offline…)
        });
    }

    fn install_update(&mut self, ctx: &egui::Context, url: String) {
        self.update = Update::Installing;
        self.spawn_update(ctx, move || match updater::install(&url) {
            Ok(()) => Update::Installed,
            Err(e) => Update::Failed(e),
        });
    }

    fn spawn_update(&mut self, ctx: &egui::Context, job: impl FnOnce() -> Update + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.update_rx = Some(rx);
    }

    fn restart(&self, ctx: &egui::Context) {
        if let Some(exe) = &self.exe
            && std::process::Command::new(exe).spawn().is_ok()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.update_rx
            && let Ok(state) = rx.try_recv()
        {
            self.update = state;
            self.update_rx = None;
        }
        let Some((id, rx)) = &self.pending else { return };
        let Ok(result) = rx.try_recv() else { return };
        let outcome = match result {
            Ok(resp) => {
                self.seq += 1;
                Outcome::Ok(Box::new(Shown { body: body::Body::new(&resp), resp, seq: self.seq }))
            }
            Err(e) => Outcome::Err(e),
        };
        self.responses.insert(id.clone(), outcome);
        self.pending = None;
    }

    // ---------- Sidebar ----------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let top = Rect::from_min_size(ui.max_rect().min, vec2(ui.available_width(), TITLEBAR_INSET));
        if cfg!(target_os = "macos") {
            w::window_drag_area(ui, top, "sidebar_drag");
        }
        ui.add_space(TITLEBAR_INSET);

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let width = ui.available_width() - 34.0;
            w::search_field(ui, &mut self.search, width);
            if w::icon_button(ui, Icon::Plus, "New request  ⌘N").clicked() {
                self.new_request(None);
            }
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            w::caption(ui, "Requests");
        });
        ui.add_space(2.0);

        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let q = self.search.to_lowercase();
            // The unsaved request, if any.
            let scratch = if self.cur.id.is_empty() { Some(&self.cur) } else { self.drafts.get("") };
            if let Some(r) = scratch.filter(|r| self.is_dirty(r) || self.cur.id.is_empty()) {
                let name = if r.name.is_empty() { "New request" } else { &r.name };
                let color = method_color(&r.method, &p);
                if w::sidebar_row(ui, &r.method, color, name, true, self.cur.id.is_empty()).clicked() {
                    clicked = Some(String::new());
                }
            }
            for r in &self.saved {
                let hay = format!("{} {} {}", r.name, r.method, r.url).to_lowercase();
                if !q.is_empty() && !hay.contains(&q) {
                    continue;
                }
                let current = self.cur.id == r.id;
                let shown = if current { &self.cur } else { self.drafts.get(&r.id).unwrap_or(r) };
                let color = method_color(&shown.method, &p);
                let resp = w::sidebar_row(ui, &shown.method, color, &shown.name, shown != r, current);
                if resp.on_hover_text(&r.url).clicked() {
                    clicked = Some(r.id.clone());
                }
            }
            if self.saved.is_empty() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Saved requests appear here.").color(p.weak));
                });
            }
        });
        if let Some(id) = clicked {
            self.switch_to(&id);
        }
    }

    // ---------- Toolbar ----------

    fn toolbar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let p = pal(ui);
        if cfg!(target_os = "macos") {
            w::window_drag_area(ui, rect, "toolbar_drag");
        }
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.separator));
        let inner = rect.shrink2(vec2(16.0, 0.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner).layout(Layout::right_to_left(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            self.env_picker(ui);
            ui.add_space(6.0);

            if !self.cur.id.is_empty() {
                if self.confirm_delete {
                    if w::button(ui, "Delete?", Some(Icon::Trash), true).clicked() {
                        self.delete();
                    }
                } else if w::icon_button(ui, Icon::Trash, "Delete").clicked() {
                    self.confirm_delete = true;
                }
                if w::icon_button(ui, Icon::Copy, "Duplicate").clicked() {
                    self.duplicate();
                }
            }
            let dirty = self.is_dirty(&self.cur);
            if w::button(ui, "Save", None, dirty).on_hover_text("⌘S").clicked() {
                self.save();
            }
            if let Some((msg, err, at)) = &self.message {
                let age = at.elapsed();
                if *err || age < Duration::from_millis(2500) {
                    ui.label(RichText::new(msg).size(12.0).color(if *err { p.red } else { p.weak }));
                    if !*err {
                        ui.ctx().request_repaint_after(Duration::from_millis(2600) - age);
                    }
                }
            }
            self.update_notice(ui);

            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.cur.name)
                        .hint_text("Untitled request")
                        .font(egui::FontId::proportional(15.0))
                        .align(egui::Align2::LEFT_CENTER)
                        .frame(Frame::NONE)
                        .desired_width(ui.available_width()),
                );
            });
        });
    }

    fn env_picker(&mut self, ui: &mut egui::Ui) {
        let before = self.envs.active.clone();
        let label = self.envs.active.clone().unwrap_or_else(|| "No environment".into());
        egui::ComboBox::from_id_salt("env").icon(w::chevron).selected_text(label).width(150.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut self.envs.active, None, "No environment");
            for env in &self.envs.envs {
                ui.selectable_value(&mut self.envs.active, Some(env.name.clone()), &env.name);
            }
            ui.separator();
            if ui.selectable_label(false, "Manage environments…").clicked() {
                self.open_window(ui.ctx(), Action::Environments);
            }
        });
        if before != self.envs.active {
            let _ = self.store.save_envs(&self.envs);
        }
    }

    /// Discreet notice in the toolbar when an update is available or in progress.
    fn update_notice(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let p = pal(ui);
        match &self.update {
            Update::Available(rel) => {
                let rel = rel.clone();
                let label = format!("Update to {}", rel.version);
                let resp = w::button(ui, &label, Some(Icon::Download), true).on_hover_text("See what's new on GitHub");
                if resp.clicked() {
                    match rel.asset_url {
                        Some(url) => self.install_update(&ctx, url),
                        None => ctx.open_url(egui::OpenUrl::new_tab(&rel.page_url)),
                    }
                }
            }
            Update::Installing => {
                ui.label(RichText::new("Updating…").size(12.0).color(p.weak));
                ui.spinner();
            }
            Update::Installed => {
                if w::accent_button(ui, "Restart to update", None, true).clicked() {
                    self.restart(&ctx);
                }
            }
            Update::Failed(e) if !self.show_settings && !self.show_about => {
                ui.label(RichText::new("Update failed").size(12.0).color(p.red)).on_hover_text(e);
            }
            _ => {}
        }
    }

    // ---------- Request ----------

    fn url_bar(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let mut send = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let bar_w = ui.available_width() - 92.0;
            Frame::new()
                .fill(p.input)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(3, 2))
                .show(ui, |ui| {
                    ui.set_width(bar_w - 8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let color = method_color(&self.cur.method, &p);
                        egui::ComboBox::from_id_salt("method")
                            .icon(w::chevron)
                            .width(84.0)
                            .selected_text(RichText::new(&self.cur.method).monospace().color(color))
                            .show_ui(ui, |ui| {
                                for m in METHODS {
                                    let text = RichText::new(m).monospace().color(method_color(m, &p));
                                    ui.selectable_value(&mut self.cur.method, m.to_string(), text);
                                }
                            });
                        let url = ui.add(
                            egui::TextEdit::singleline(&mut self.cur.url)
                                .hint_text("https://api.example.com/users")
                                .font(egui::TextStyle::Monospace)
                                .frame(Frame::NONE)
                                .margin(Margin::symmetric(6, 6))
                                .align(egui::Align2::LEFT_CENTER)
                                .desired_width(ui.available_width()),
                        );
                        if url.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            send = true;
                        }
                    });
                });
            let enabled = self.pending.is_none() && !self.cur.url.trim().is_empty();
            if w::accent_button(ui, "Send", Some(Icon::Send), enabled).on_hover_text("⌘↵").clicked() {
                send = true;
            }
        });
        if self.cur.url.contains("{{") {
            let resolved = self.envs.resolve(&self.cur.url);
            ui.horizontal(|ui| {
                ui.add_space(96.0);
                ui.label(RichText::new(format!("→ {resolved}")).monospace().size(11.0).color(p.weak));
            });
        }
        if send {
            self.send(ui.ctx());
        }
    }

    fn request_editor(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            let n = self.cur.headers.iter().filter(|h| h.enabled && !h.key.is_empty()).count();
            let headers = if n > 0 { format!("Headers  {n}") } else { "Headers".into() };
            w::segmented(ui, &mut self.req_tab, &[(ReqTab::Body, "Body".into()), (ReqTab::Headers, headers)]);
            if self.req_tab == ReqTab::Body && !self.cur.body.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if w::button(ui, "Format JSON", None, true).clicked() {
                        self.format_json();
                    }
                });
            }
        });
        ui.add_space(8.0);
        match self.req_tab {
            ReqTab::Body => code_frame(&p).show(ui, |ui| {
                let h = ui.available_height().max(60.0);
                egui::ScrollArea::vertical().id_salt("req_body").auto_shrink(false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cur.body)
                            .code_editor()
                            .frame(Frame::NONE)
                            .hint_text("Request body — JSON, text…")
                            .desired_width(f32::INFINITY)
                            .min_size(vec2(0.0, h - 4.0)),
                    );
                });
            }),
            ReqTab::Headers => Frame::NONE.show(ui, |ui| headers_editor(ui, &mut self.cur.headers)),
        };
    }

    // ---------- Response ----------

    fn response_view(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.add_space(12.0);
        if self.pending.as_ref().is_some_and(|(id, _)| *id == self.cur.id) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Sending…").color(p.weak));
            });
            return;
        }
        let Some(outcome) = self.responses.get_mut(&self.cur.id) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Send a request to see the response here.   ⌘↵").color(p.weak));
            });
            return;
        };
        let shown = match outcome {
            Outcome::Err(e) => {
                ui.horizontal(|ui| {
                    w::pill(ui, "Error", p.red);
                    ui.label(RichText::new(e.as_str()).color(p.text));
                });
                return;
            }
            Outcome::Ok(s) => s,
        };
        let r = &shown.resp;
        let kind = shown.body.kind;
        let view = self.views.get(&kind).copied().unwrap_or(kind.views()[0]);
        ui.horizontal(|ui| {
            w::pill(ui, &format!("{} {}", r.status, r.reason), status_color(r.status, &p));
            ui.label(RichText::new(format!("{} ms", r.elapsed.as_millis())).color(p.weak));
            ui.label(RichText::new("·").color(p.weak));
            ui.label(RichText::new(human_size(r.body.len())).color(p.weak));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::icon_button(ui, Icon::Copy, "Copy").clicked() {
                    let text = match self.resp_tab {
                        RespTab::Body => shown.body.copy_text(view, &r.body),
                        RespTab::Headers => r.headers.iter().map(|(k, v)| format!("{k}: {v}\n")).collect(),
                    };
                    ui.ctx().copy_text(text);
                    self.message = Some(("Copied".into(), false, Instant::now()));
                }
                let headers = format!("Headers  {}", r.headers.len());
                w::segmented(ui, &mut self.resp_tab, &[(RespTab::Body, "Body".into()), (RespTab::Headers, headers)]);
            });
        });
        ui.add_space(8.0);

        if self.resp_tab == RespTab::Body && !r.body.is_empty() {
            // Kind of content on the left, view switcher and tools on the right.
            ui.horizontal(|ui| {
                let label = if shown.body.media_type.is_empty() {
                    kind.label().to_string()
                } else {
                    format!("{}  ·  {}", kind.label(), shown.body.media_type)
                };
                ui.label(RichText::new(label).size(12.0).color(p.weak));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if kind.views().len() > 1 {
                        let mut v = view;
                        let options: Vec<_> = kind.views().iter().map(|v| (*v, v.label().to_string())).collect();
                        if w::segmented(ui, &mut v, &options) {
                            self.views.insert(kind, v);
                        }
                    }
                    if kind == Kind::Html
                        && w::button(ui, "Open in Browser", None, true).clicked()
                        && let Err(e) = preview::open_in_browser(
                            ui.ctx(),
                            &preview::with_base(&String::from_utf8_lossy(&r.body), &r.url),
                        )
                    {
                        self.message = Some((e, true, Instant::now()));
                    }
                    shown.body.view_tools(ui, view);
                });
            });
            ui.add_space(6.0);
        }

        code_frame(&p).show(ui, |ui| match self.resp_tab {
            RespTab::Body => {
                if let Some(rect) = shown.body.show(ui, view, &r.body) {
                    if preview::SUPPORTED {
                        self.preview_at = Some(rect);
                        ui.allocate_rect(rect, egui::Sense::hover());
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new("No embedded preview on this platform — use Open in Browser.")
                                    .color(p.weak),
                            );
                        });
                    }
                }
            }
            RespTab::Headers => {
                egui::ScrollArea::vertical().id_salt("resp_headers").auto_shrink(false).show(ui, |ui| {
                    egui::Grid::new("resp_headers_grid").num_columns(2).spacing([24.0, 6.0]).striped(false).show(
                        ui,
                        |ui| {
                            for (k, v) in &r.headers {
                                ui.label(RichText::new(k).monospace().color(p.weak));
                                ui.add(egui::Label::new(RichText::new(v).monospace()).wrap());
                                ui.end_row();
                            }
                        },
                    );
                });
            }
        });
    }

    /// Shows the native HTML preview where the response asked for it, or hides it.
    fn sync_preview(&mut self, frame: &eframe::Frame) {
        let at = self.preview_at.take();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        match (at, self.responses.get(&self.cur.id)) {
            (Some(rect), Some(Outcome::Ok(shown))) => {
                let html = preview::with_base(&String::from_utf8_lossy(&shown.resp.body), &shown.resp.url);
                self.preview.show(frame, rect, shown.seq, &html);
                if let Some(e) = self.preview.error.take() {
                    self.message = Some((format!("Preview unavailable: {e}"), true, Instant::now()));
                    self.preview.error = Some(e);
                }
            }
            _ => self.preview.hide(),
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = (at, frame);
    }

    // ---------- Windows ----------

    // ---------- Actions ----------

    fn act(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::About | Action::Settings | Action::Environments => self.open_window(ctx, action),
            Action::CheckUpdates => {
                self.open_window(ctx, Action::About);
                if matches!(self.update, Update::Failed(_) | Update::UpToDate) {
                    self.update = Update::Idle;
                }
                self.check_updates(ctx, true);
            }
            Action::NewRequest => self.new_request(None),
            Action::Save => {
                if self.is_dirty(&self.cur) {
                    self.save();
                }
            }
            Action::Duplicate => self.duplicate(),
            Action::Delete => {
                if !self.cur.id.is_empty() {
                    self.confirm_delete = true; // confirmed with the toolbar button
                }
            }
            Action::Send => self.send(ctx),
            Action::FormatJson => self.format_json(),
            Action::GitHub => ctx.open_url(egui::OpenUrl::new_tab(format!("https://github.com/{}", updater::REPO))),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn menu_state(&self) -> menu::Enabled {
        menu::Enabled {
            save: self.is_dirty(&self.cur),
            saved_request: !self.cur.id.is_empty(),
            send: self.pending.is_none() && !self.cur.url.trim().is_empty(),
        }
    }

    fn duplicate(&mut self) {
        if self.cur.id.is_empty() {
            return;
        }
        let mut copy = self.cur.clone();
        copy.name = format!("{} copy", copy.name);
        self.new_request(Some(copy));
    }

    fn format_json(&mut self) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&self.cur.body) {
            self.cur.body = serde_json::to_string_pretty(&v).unwrap();
        }
    }

    // ---------- Secondary windows (real OS windows) ----------

    fn open_window(&mut self, ctx: &egui::Context, which: Action) {
        let (key, size, _) = window_spec(which);
        // Centered over the main window, computed once so it doesn't follow it around.
        if let Some(main) = ctx.input(|i| i.viewport().outer_rect) {
            let pos = main.center() - Vec2::from(size) / 2.0 - vec2(0.0, 40.0);
            self.window_pos.insert(key, pos.max(main.min));
        }
        match which {
            Action::About => self.show_about = true,
            Action::Settings => self.show_settings = true,
            Action::Environments => self.show_envs = true,
            _ => {}
        }
        ctx.send_viewport_cmd_to(egui::ViewportId::from_hash_of(key), egui::ViewportCommand::Focus);
    }

    fn windows(&mut self, ctx: &egui::Context) {
        for which in [Action::Settings, Action::About, Action::Environments] {
            let open = match which {
                Action::About => self.show_about,
                Action::Settings => self.show_settings,
                _ => self.show_envs,
            };
            let (key, size, resizable) = window_spec(which);
            if !open {
                self.window_opened.remove(key);
                continue;
            }
            if self.window_opened.entry(key).or_insert_with(Instant::now).elapsed() < Duration::from_secs(1) {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            let mut builder = egui::ViewportBuilder::default()
                .with_title(key)
                .with_inner_size(size)
                .with_resizable(resizable)
                .with_minimize_button(false)
                .with_maximize_button(false);
            if resizable {
                builder = builder.with_min_inner_size([size[0] * 0.8, size[1] * 0.7]);
            }
            if let Some(pos) = self.window_pos.get(key) {
                builder = builder.with_position(*pos);
            }
            let close = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(key), builder, |ui, _| {
                let fill = ui.visuals().window_fill;
                egui::CentralPanel::default().frame(Frame::new().fill(fill).inner_margin(Margin::same(20))).show(
                    ui,
                    |ui| match which {
                        Action::About => self.about_content(ui),
                        Action::Settings => self.settings_content(ui),
                        _ => self.envs_content(ui),
                    },
                );
                ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape))
            });
            if close {
                match which {
                    Action::About => self.show_about = false,
                    Action::Settings => self.show_settings = false,
                    _ => self.show_envs = false,
                }
            }
        }
    }

    fn envs_content(&mut self, ui: &mut egui::Ui) {
        let before = self.envs.clone();
        self.envs_editor(ui);
        if self.envs != before
            && let Err(e) = self.store.save_envs(&self.envs)
        {
            self.flash(format!("Could not save environments: {e}"), true);
        }
    }

    fn envs_editor(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        ui.horizontal(|ui| {
            if !self.envs.envs.is_empty() {
                self.edit_env = self.edit_env.min(self.envs.envs.len() - 1);
                egui::ComboBox::from_id_salt("edit_env")
                    .icon(w::chevron)
                    .width(180.0)
                    .selected_text(&self.envs.envs[self.edit_env].name)
                    .show_ui(ui, |ui| {
                        for (i, e) in self.envs.envs.iter().enumerate() {
                            ui.selectable_value(&mut self.edit_env, i, &e.name);
                        }
                    });
            }
            if w::button(ui, "New environment", Some(Icon::Plus), true).clicked() {
                let n = self.envs.envs.len() + 1;
                self.envs.envs.push(Env { name: format!("env{n}"), vars: vec![Var::default()] });
                self.edit_env = self.envs.envs.len() - 1;
            }
        });
        if self.envs.envs.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Create an environment (local, staging, prod…), then use {{name}} in the URL, headers or body.",
                )
                .color(p.weak),
            );
            return;
        }
        ui.add_space(10.0);
        let i = self.edit_env;
        let old_name = self.envs.envs[i].name.clone();
        let mut remove_env = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Name").color(p.weak));
            ui.add(w::text_field(&mut self.envs.envs[i].name).desired_width(220.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::icon_button(ui, Icon::Trash, "Delete environment").clicked() {
                    remove_env = true;
                }
            });
        });
        if self.envs.active.as_deref() == Some(old_name.as_str()) && self.envs.envs[i].name != old_name {
            self.envs.active = Some(self.envs.envs[i].name.clone());
        }
        ui.add_space(10.0);
        w::caption(ui, "Variables");
        let vars = &mut self.envs.envs[i].vars;
        let mut remove = None;
        for (j, v) in vars.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let avail = ui.available_width() - 28.0 - 2.0 * ui.spacing().item_spacing.x;
                ui.add(
                    w::text_field(&mut v.key)
                        .hint_text("name")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(avail * 0.35),
                );
                ui.add(
                    w::text_field(&mut v.value)
                        .hint_text("value")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(avail * 0.65),
                );
                if w::icon_button(ui, Icon::Close, "Remove").clicked() {
                    remove = Some(j);
                }
            });
        }
        if let Some(j) = remove {
            vars.remove(j);
        }
        ui.add_space(4.0);
        if w::button(ui, "Add variable", Some(Icon::Plus), true).clicked() {
            vars.push(Var::default());
        }
        if remove_env {
            let removed = self.envs.envs.remove(i);
            if self.envs.active.as_deref() == Some(removed.name.as_str()) {
                self.envs.active = None;
            }
            self.edit_env = 0;
        }
    }

    fn settings_content(&mut self, ui: &mut egui::Ui) {
        let before = self.settings.clone();
        let mut check_now = false;
        let p = pal(ui);
        egui::Grid::new("settings").num_columns(2).spacing([20.0, 14.0]).show(ui, |ui| {
            ui.label(RichText::new("Appearance").color(p.weak));
            w::segmented(
                ui,
                &mut self.settings.theme,
                &[(Theme::Auto, "Auto".into()), (Theme::Light, "Light".into()), (Theme::Dark, "Dark".into())],
            );
            ui.end_row();

            ui.label(RichText::new("Updates").color(p.weak));
            ui.horizontal(|ui| {
                w::checkbox(ui, &mut self.settings.check_updates);
                ui.label("Check on startup");
            });
            ui.end_row();

            ui.label(RichText::new("Version").color(p.weak));
            ui.horizontal(|ui| {
                ui.label(updater::VERSION);
                match &self.update {
                    Update::Checking { manual: true } => {
                        ui.spinner();
                    }
                    Update::UpToDate => {
                        ui.label(RichText::new("Up to date").color(p.green));
                    }
                    Update::Available(rel) => {
                        ui.label(RichText::new(format!("{} available", rel.version)).color(p.accent));
                    }
                    Update::Failed(e) => {
                        ui.label(RichText::new("Check failed").color(p.red)).on_hover_text(e);
                    }
                    _ => {}
                }
                if w::button(ui, "Check now", None, true).clicked() {
                    check_now = true;
                }
            });
            ui.end_row();

            ui.label(RichText::new("Data").color(p.weak));
            let path = self.store.root().display().to_string();
            ui.add(egui::Label::new(RichText::new(&path).monospace().size(11.5)).truncate()).on_hover_text(path);
            ui.end_row();
        });
        ui.add_space(10.0);
        ui.hyperlink_to("Plume on GitHub", format!("https://github.com/{}", updater::REPO));
        let ctx = ui.ctx().clone();
        if check_now {
            if matches!(self.update, Update::Failed(_) | Update::UpToDate) {
                self.update = Update::Idle;
            }
            self.check_updates(&ctx, true);
        }
        if self.settings != before {
            if self.settings.theme != before.theme {
                self.apply_theme(&ctx);
            }
            if let Err(e) = self.store.save_settings(&self.settings) {
                self.flash(format!("Could not save settings: {e}"), true);
            }
        }
    }

    fn about_content(&mut self, ui: &mut egui::Ui) {
        let p = pal(ui);
        let icon = self.icon.get_or_insert_with(|| {
            let data = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default();
            let image =
                egui::ColorImage::from_rgba_unmultiplied([data.width as usize, data.height as usize], &data.rgba);
            ui.ctx().load_texture("plume-icon", image, egui::TextureOptions::LINEAR)
        });
        let icon = (icon.id(), vec2(96.0, 96.0));
        let ctx = ui.ctx().clone();
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            ui.add(egui::Image::new(icon));
            ui.add_space(6.0);
            ui.label(RichText::new("Plume").size(22.0).strong());
            ui.label(RichText::new(format!("Version {}", updater::VERSION)).color(p.weak));
            ui.add_space(10.0);
            ui.label(RichText::new("A tiny, native REST client.\nNo Electron, no account, no cloud.").color(p.text));
            ui.add_space(14.0);
            match &self.update {
                Update::Checking { .. } => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Checking for updates…").color(p.weak));
                    });
                }
                Update::UpToDate => {
                    ui.label(RichText::new("Plume is up to date.").color(p.green));
                }
                Update::Available(rel) => {
                    let rel = rel.clone();
                    if w::accent_button(ui, &format!("Update to {}", rel.version), Some(Icon::Download), true).clicked()
                    {
                        match rel.asset_url {
                            Some(url) => self.install_update(&ctx, url),
                            None => ctx.open_url(egui::OpenUrl::new_tab(&rel.page_url)),
                        }
                    }
                }
                Update::Installing => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Updating…").color(p.weak));
                    });
                }
                Update::Installed => {
                    if w::accent_button(ui, "Restart to update", None, true).clicked() {
                        self.restart(&ctx);
                    }
                }
                Update::Failed(e) => {
                    ui.label(RichText::new("Could not check for updates.").color(p.red)).on_hover_text(e);
                }
                Update::Idle => {
                    if w::button(ui, "Check for Updates", None, true).clicked() {
                        self.check_updates(&ctx, true);
                    }
                }
            }
            ui.add_space(14.0);
            // Center the two links: measure them first.
            let font = egui::TextStyle::Body.resolve(ui.style());
            let width: f32 = ["GitHub", "·", "MIT License"]
                .iter()
                .map(|t| ui.painter().layout_no_wrap(t.to_string(), font.clone(), p.text).size().x)
                .sum::<f32>()
                + 2.0 * ui.spacing().item_spacing.x;
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.hyperlink_to("GitHub", format!("https://github.com/{}", updater::REPO));
                ui.label(RichText::new("·").color(p.weak));
                ui.hyperlink_to("MIT License", format!("https://github.com/{}/blob/main/LICENSE", updater::REPO));
            });
            ui.add_space(4.0);
            ui.label(RichText::new("© 2026 Rawz06 Studio").size(11.0).color(p.weak));
        });
    }
}

impl eframe::App for App {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if theme::glass() { [0.0; 4] } else { visuals.panel_fill.to_normalized_gamma_f32() }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();

        // Mica takes the light/dark variant explicitly: re-apply when it changes.
        let dark = ui.visuals().dark_mode;
        if cfg!(target_os = "windows") && self.material_dark != Some(dark) {
            theme::apply_material(frame, dark);
            self.material_dark = Some(dark);
        }
        let _ = &frame;

        let mut actions: Vec<Action> = ctx.input_mut(|i| std::iter::from_fn(|| Action::from_keys(i)).collect());
        #[cfg(target_os = "macos")]
        {
            actions.extend(std::iter::from_fn(|| self.native_menu.poll()));
            self.native_menu.set_enabled(self.menu_state());
        }

        let p = pal(ui);
        if !cfg!(target_os = "macos") {
            let state = self.menu_state();
            egui::Panel::top("menubar")
                .frame(Frame::new().fill(p.sidebar).inner_margin(Margin::symmetric(8, 3)))
                .show(ui, |ui| actions.extend(menu::in_window_bar(ui, state)));
        }
        for action in actions {
            self.act(action, &ctx);
        }

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(250.0)
            .size_range(200.0..=440.0)
            .frame(Frame::new().fill(p.sidebar).inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 10 }))
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default().frame(Frame::new().fill(p.content)).show(ui, |ui| {
            let full = ui.max_rect();
            let bar = Rect::from_min_size(full.min, vec2(full.width(), TOOLBAR_HEIGHT));
            self.toolbar(ui, bar);

            let body = Rect::from_min_max(egui::pos2(full.left(), bar.bottom()), full.max).shrink2(vec2(18.0, 0.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                ui.add_space(14.0);
                self.url_bar(ui);
                ui.add_space(12.0);
                // Request on top, response below, split by a draggable divider.
                let area = ui.available_rect_before_wrap();
                let split_y =
                    area.top() + (area.height() * self.split).clamp(140.0, (area.height() - 120.0).max(140.0));
                let top = Rect::from_min_max(area.min, egui::pos2(area.right(), split_y - 8.0));
                let bottom = Rect::from_min_max(
                    egui::pos2(area.left(), split_y + 4.0),
                    egui::pos2(area.right(), area.bottom() - 14.0),
                );

                let handle =
                    Rect::from_center_size(egui::pos2(area.center().x, split_y - 2.0), vec2(area.width(), 8.0));
                let resp = ui.interact(handle, ui.make_persistent_id("split"), egui::Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
                if resp.dragged() && area.height() > 0.0 {
                    self.split = (self.split + resp.drag_delta().y / area.height()).clamp(0.2, 0.8);
                }
                let line = if resp.hovered() || resp.dragged() { p.accent } else { p.separator };
                ui.painter().hline(area.x_range(), split_y - 2.0, Stroke::new(1.0, line));

                ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| self.request_editor(ui));
                ui.scope_builder(egui::UiBuilder::new().max_rect(bottom), |ui| self.response_view(ui));
            });
        });

        self.sync_preview(frame);
        self.windows(&ctx);
    }
}

// ---------- Helpers ----------

/// Title (also the window id), inner size and resizability of each secondary window.
fn window_spec(which: Action) -> (&'static str, [f32; 2], bool) {
    match which {
        Action::About => ("About Plume", [340.0, 380.0], false),
        Action::Settings => ("Settings", [480.0, 260.0], false),
        _ => ("Environments", [620.0, 420.0], true),
    }
}

fn code_frame(p: &Palette) -> Frame {
    Frame::new()
        .fill(p.code)
        .stroke(Stroke::new(1.0, p.separator))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
}

/// Header rows: checkbox, name (⅓), value (⅔), remove button — using the full width.
fn headers_editor(ui: &mut egui::Ui, headers: &mut Vec<Header>) {
    egui::ScrollArea::vertical().id_salt("req_headers").auto_shrink(false).show(ui, |ui| {
        let mut remove = None;
        for (i, h) in headers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                w::checkbox(ui, &mut h.enabled);
                let gaps = 3.0 * ui.spacing().item_spacing.x;
                let avail = (ui.available_width() - 28.0 - gaps).max(200.0);
                ui.add(
                    w::text_field(&mut h.key)
                        .hint_text("Header")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(avail * 0.34),
                );
                ui.add(
                    w::text_field(&mut h.value)
                        .hint_text("Value")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(avail * 0.66),
                );
                if w::icon_button(ui, Icon::Close, "Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            headers.remove(i);
        }
        ui.add_space(4.0);
        if w::button(ui, "Add header", Some(Icon::Plus), true).clicked() {
            headers.push(Header { enabled: true, ..Default::default() });
        }
    });
}

fn method_color(m: &str, p: &Palette) -> Color32 {
    match m {
        "GET" => p.green,
        "POST" => p.orange,
        "PUT" => p.blue,
        "PATCH" => p.purple,
        "DELETE" => p.red,
        _ => p.weak,
    }
}

fn status_color(s: u16, p: &Palette) -> Color32 {
    match s {
        200..=299 => p.green,
        300..=399 => p.blue,
        400..=499 => p.orange,
        _ => p.red,
    }
}

fn human_size(n: usize) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_decodes() {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
    }
}
