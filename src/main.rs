#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod http;
mod store;
mod ui;
mod updater;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Key, Layout, Margin, Modifiers, Rect, RichText, Stroke, vec2,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use store::{Env, Envs, Header, METHODS, Request, Settings, Store, Theme, Var};
use ui::icons::Icon;
use ui::theme::{self, Palette, pal};
use ui::widgets as w;

/// Response bodies larger than this are truncated on screen (huge text edits are slow).
const MAX_DISPLAY: usize = 2 * 1024 * 1024;

/// On macOS the content extends under the title bar; leave room for the window buttons.
const TITLEBAR_INSET: f32 = if cfg!(target_os = "macos") { 38.0 } else { 10.0 };
const TOOLBAR_HEIGHT: f32 = 52.0;

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
    let options = eframe::NativeOptions { viewport, ..Default::default() };
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
    text: String,
}

enum Outcome {
    Ok(Shown),
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
            edit_env: 0,
            confirm_delete: false,
            message: None,
            update: Update::Idle,
            update_rx: None,
            exe: std::env::current_exe().ok(),
            material_dark: None,
            split: 0.45,
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
                let text = display_body(&resp);
                Outcome::Ok(Shown { resp, text })
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
            if w::icon_button(ui, Icon::Gear, "Settings  ⌘,").clicked() {
                self.show_settings = true;
            }
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
                    let mut copy = self.cur.clone();
                    copy.name = format!("{} copy", copy.name);
                    self.new_request(Some(copy));
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
                self.show_envs = true;
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
            Update::Failed(e) if !self.show_settings => {
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
                    if w::button(ui, "Format JSON", None, true).clicked()
                        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&self.cur.body)
                    {
                        self.cur.body = serde_json::to_string_pretty(&v).unwrap();
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
        let Some(outcome) = self.responses.get(&self.cur.id) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Send a request to see the response here.   ⌘↵").color(p.weak));
            });
            return;
        };
        let shown = match outcome {
            Outcome::Err(e) => {
                ui.horizontal(|ui| {
                    w::pill(ui, "Error", p.red);
                    ui.label(RichText::new(e).color(p.text));
                });
                return;
            }
            Outcome::Ok(s) => s,
        };
        let r = &shown.resp;
        ui.horizontal(|ui| {
            w::pill(ui, &format!("{} {}", r.status, r.reason), status_color(r.status, &p));
            ui.label(RichText::new(format!("{} ms", r.elapsed.as_millis())).color(p.weak));
            ui.label(RichText::new("·").color(p.weak));
            ui.label(RichText::new(human_size(r.body.len())).color(p.weak));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if w::icon_button(ui, Icon::Copy, "Copy").clicked() {
                    let text = match self.resp_tab {
                        RespTab::Body => String::from_utf8_lossy(&r.body).into_owned(),
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
        code_frame(&p).show(ui, |ui| match self.resp_tab {
            RespTab::Body => {
                egui::ScrollArea::both().id_salt("resp_body").auto_shrink(false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut shown.text.as_str())
                            .code_editor()
                            .frame(Frame::NONE)
                            .desired_width(f32::INFINITY),
                    );
                });
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

    // ---------- Windows ----------

    fn envs_window(&mut self, ctx: &egui::Context) {
        if !self.show_envs {
            return;
        }
        let before = self.envs.clone();
        let mut open = true;
        egui::Window::new("Environments")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .default_width(560.0)
            .collapsible(false)
            .show(ctx, |ui| {
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
                            w::text_field(&mut v.key).hint_text("name").font(egui::TextStyle::Monospace).desired_width(avail * 0.35),
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
            });
        self.show_envs = open;
        if self.envs != before
            && let Err(e) = self.store.save_envs(&self.envs)
        {
            self.flash(format!("Could not save environments: {e}"), true);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let before = self.settings.clone();
        let mut open = true;
        let mut check_now = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 80.0])
            .default_width(420.0)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
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
                    ui.add(egui::Label::new(RichText::new(&path).monospace().size(11.5)).truncate())
                        .on_hover_text(path);
                    ui.end_row();
                });
                ui.add_space(10.0);
                ui.hyperlink_to("Plume on GitHub", format!("https://github.com/{}", updater::REPO));
            });
        self.show_settings = open;
        if check_now {
            if matches!(self.update, Update::Failed(_) | Update::UpToDate) {
                self.update = Update::Idle;
            }
            self.check_updates(ctx, true);
        }
        if self.settings != before {
            if self.settings.theme != before.theme {
                self.apply_theme(ctx);
            }
            if let Err(e) = self.store.save_settings(&self.settings) {
                self.flash(format!("Could not save settings: {e}"), true);
            }
        }
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

        let (send, save, new, settings) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::Comma),
            )
        });
        if send {
            self.send(&ctx);
        }
        if save && self.is_dirty(&self.cur) {
            self.save();
        }
        if new {
            self.new_request(None);
        }
        if settings {
            self.show_settings = !self.show_settings;
        }

        let p = pal(ui);
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

        self.envs_window(&ctx);
        self.settings_window(&ctx);
    }
}

// ---------- Helpers ----------

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

fn display_body(resp: &http::Response) -> String {
    let bytes = &resp.body;
    if bytes.is_empty() {
        return String::new();
    }
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes)
        && let Ok(pretty) = serde_json::to_string_pretty(&v)
        && pretty.len() <= MAX_DISPLAY
    {
        return pretty;
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_DISPLAY)]).into_owned();
    if bytes.len() > MAX_DISPLAY {
        format!("{text}\n\n… truncated for display ({}). Copy copies everything.", human_size(bytes.len()))
    } else {
        text
    }
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
