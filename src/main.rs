#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod http;
mod store;
mod updater;

use eframe::egui::{self, Color32, Key, Modifiers, RichText};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use store::{Env, Envs, Header, METHODS, Request, Settings, Store, Theme, Var};

/// Response bodies larger than this are truncated on screen (huge text edits are slow).
const MAX_DISPLAY: usize = 2 * 1024 * 1024;

const RED: Color32 = Color32::from_rgb(220, 80, 80);
const GREEN: Color32 = Color32::from_rgb(70, 170, 110);
const BLUE: Color32 = Color32::from_rgb(70, 130, 220);
const ORANGE: Color32 = Color32::from_rgb(220, 150, 50);
const PURPLE: Color32 = Color32::from_rgb(160, 110, 210);

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Plume")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([700.0, 400.0])
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).unwrap_or_default()),
        ..Default::default()
    };
    eframe::run_native("Plume", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
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
    message: Option<(String, bool)>,
    update: Update,
    update_rx: Option<Receiver<Update>>,
    exe: Option<PathBuf>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
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
        };
        app.apply_theme(ctx);
        if app.settings.check_updates && !cfg!(debug_assertions) {
            app.check_updates(ctx, false);
        }
        app
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            Theme::Auto => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
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
                self.message = Some(("Saved".into(), false));
            }
            Err(e) => self.message = Some((format!("Could not save: {e}"), true)),
        }
    }

    fn delete(&mut self) {
        let id = self.cur.id.clone();
        if id.is_empty() {
            self.cur = Request::default();
            return;
        }
        if let Err(e) = self.store.delete_request(&id) {
            self.message = Some((format!("Could not delete: {e}"), true));
            return;
        }
        self.saved.retain(|s| s.id != id);
        self.responses.remove(&id);
        self.cur = self.drafts.remove("").unwrap_or_default();
        self.confirm_delete = false;
    }

    fn send(&mut self, ctx: &egui::Context) {
        if self.cur.url.trim().is_empty() {
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

    // ---------- UI ----------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Plume").strong().size(16.0));
            self.update_banner(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings (⌘,)").clicked() {
                    self.show_settings = true;
                }
                if ui.button("Environments…").clicked() {
                    self.show_envs = true;
                }
                let before = self.envs.active.clone();
                egui::ComboBox::from_id_salt("env")
                    .selected_text(self.envs.active.clone().unwrap_or_else(|| "No environment".into()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.envs.active, None, "No environment");
                        for env in &self.envs.envs {
                            ui.selectable_value(&mut self.envs.active, Some(env.name.clone()), &env.name);
                        }
                    });
                if before != self.envs.active {
                    let _ = self.store.save_envs(&self.envs);
                }
            });
        });
    }

    /// Discreet notice next to the title when an update is available or in progress.
    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        match &self.update {
            Update::Available(rel) => {
                let rel = rel.clone();
                ui.separator();
                ui.label(RichText::new(format!("Plume {} is available", rel.version)).color(BLUE));
                match rel.asset_url {
                    Some(url) => {
                        if ui.small_button("Update").clicked() {
                            self.install_update(&ctx, url);
                        }
                    }
                    None => {
                        if ui.small_button("Download").clicked() {
                            ctx.open_url(egui::OpenUrl::new_tab(&rel.page_url));
                        }
                    }
                }
                ui.hyperlink_to(RichText::new("What's new").small(), &rel.page_url);
            }
            Update::Installing => {
                ui.separator();
                ui.spinner();
                ui.label("Updating…");
            }
            Update::Installed => {
                ui.separator();
                ui.label(RichText::new("Update installed").color(GREEN));
                if ui.small_button("Restart").clicked() {
                    self.restart(&ctx);
                }
            }
            Update::Failed(e) if !self.show_settings => {
                ui.separator();
                ui.label(RichText::new("Update failed").color(RED)).on_hover_text(e);
            }
            _ => {}
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        if ui.button("＋ New request").on_hover_text("⌘N").clicked() {
            self.new_request(None);
        }
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search…").desired_width(f32::INFINITY));
        ui.add_space(4.0);

        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let q = self.search.to_lowercase();
            // The unsaved request, if any.
            let scratch = if self.cur.id.is_empty() { Some(&self.cur) } else { self.drafts.get("") };
            if let Some(r) = scratch.filter(|r| self.is_dirty(r) || self.cur.id.is_empty()) {
                let name = if r.name.is_empty() { "New request" } else { &r.name };
                if list_item(ui, &r.method, name, true, self.cur.id.is_empty()).clicked() {
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
                let dirty = shown != r;
                if list_item(ui, &shown.method, &shown.name, dirty, current).on_hover_text(&r.url).clicked() {
                    clicked = Some(r.id.clone());
                }
            }
            if self.saved.is_empty() {
                ui.add_space(8.0);
                ui.weak("No saved requests yet.");
            }
        });
        if let Some(id) = clicked {
            self.switch_to(&id);
        }
    }

    fn request_editor(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        // Name + actions
        ui.horizontal(|ui| {
            let dirty = self.is_dirty(&self.cur);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.cur.id.is_empty() {
                    let label = if self.confirm_delete { "Confirm?" } else { "Delete" };
                    if ui.button(label).clicked() {
                        if self.confirm_delete {
                            self.delete();
                        } else {
                            self.confirm_delete = true;
                        }
                    }
                    if ui.button("Duplicate").clicked() {
                        let mut copy = self.cur.clone();
                        copy.name = format!("{} (copy)", copy.name);
                        self.new_request(Some(copy));
                    }
                }
                if ui.add_enabled(dirty, egui::Button::new("Save")).on_hover_text("⌘S").clicked() {
                    self.save();
                }
                if let Some((msg, err)) = &self.message {
                    let color = if *err { RED } else { ui.visuals().weak_text_color() };
                    ui.label(RichText::new(msg).color(color));
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.cur.name)
                        .hint_text("Request name")
                        .font(egui::TextStyle::Heading)
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width()),
                );
            });
        });
        ui.add_space(4.0);

        // Method + URL + Send
        let mut send = false;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("method")
                .width(90.0)
                .selected_text(RichText::new(&self.cur.method).color(method_color(&self.cur.method)).strong())
                .show_ui(ui, |ui| {
                    for m in METHODS {
                        ui.selectable_value(
                            &mut self.cur.method,
                            m.to_string(),
                            RichText::new(m).color(method_color(m)),
                        );
                    }
                });
            let busy = self.pending.is_some();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!busy, egui::Button::new("Send")).on_hover_text("⌘↵").clicked() {
                    send = true;
                }
                let url = ui.add(
                    egui::TextEdit::singleline(&mut self.cur.url)
                        .hint_text("https://api.example.com/users  —  {{variables}} welcome")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                );
                if url.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    send = true;
                }
            });
        });
        if send && self.pending.is_none() {
            self.send(ui.ctx());
        }
        if self.cur.url.contains("{{") {
            let resolved = self.envs.resolve(&self.cur.url);
            ui.label(RichText::new(format!("→ {resolved}")).monospace().small().weak());
        }
        ui.add_space(6.0);

        // Body / Headers tabs
        ui.horizontal(|ui| {
            let n = self.cur.headers.iter().filter(|h| h.enabled && !h.key.is_empty()).count();
            ui.selectable_value(&mut self.req_tab, ReqTab::Body, "Body");
            let label = if n > 0 { format!("Headers ({n})") } else { "Headers".into() };
            ui.selectable_value(&mut self.req_tab, ReqTab::Headers, label);
            if self.req_tab == ReqTab::Body && !self.cur.body.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Format JSON").clicked()
                        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&self.cur.body)
                    {
                        self.cur.body = serde_json::to_string_pretty(&v).unwrap();
                    }
                });
            }
        });
        ui.separator();
        match self.req_tab {
            ReqTab::Body => {
                egui::ScrollArea::vertical().id_salt("req_body").show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.cur.body)
                            .code_editor()
                            .hint_text("Request body (JSON, text…)")
                            .desired_width(f32::INFINITY)
                            .desired_rows(12),
                    );
                });
            }
            ReqTab::Headers => headers_editor(ui, &mut self.cur.headers),
        }
    }

    fn response_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        if self.pending.as_ref().is_some_and(|(id, _)| *id == self.cur.id) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Sending…");
            });
            return;
        }
        let Some(outcome) = self.responses.get(&self.cur.id) else {
            ui.weak("No response yet. Press ⌘↵ to send.");
            return;
        };
        let shown = match outcome {
            Outcome::Err(e) => {
                ui.label(RichText::new(format!("Error: {e}")).color(RED));
                return;
            }
            Outcome::Ok(s) => s,
        };
        let r = &shown.resp;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} {}", r.status, r.reason)).strong().color(status_color(r.status)));
            ui.label(format!("{} ms", r.elapsed.as_millis()));
            ui.label(human_size(r.body.len()));
            ui.separator();
            ui.selectable_value(&mut self.resp_tab, RespTab::Body, "Body");
            ui.selectable_value(&mut self.resp_tab, RespTab::Headers, format!("Headers ({})", r.headers.len()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Copy").clicked() {
                    let text = match self.resp_tab {
                        RespTab::Body => String::from_utf8_lossy(&r.body).into_owned(),
                        RespTab::Headers => r.headers.iter().map(|(k, v)| format!("{k}: {v}\n")).collect(),
                    };
                    ui.ctx().copy_text(text);
                }
            });
        });
        ui.separator();
        match self.resp_tab {
            RespTab::Body => {
                egui::ScrollArea::both().id_salt("resp_body").auto_shrink(false).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut shown.text.as_str()).code_editor().desired_width(f32::INFINITY),
                    );
                });
            }
            RespTab::Headers => {
                egui::ScrollArea::vertical().id_salt("resp_headers").auto_shrink(false).show(ui, |ui| {
                    egui::Grid::new("resp_headers_grid").num_columns(2).striped(true).show(ui, |ui| {
                        for (k, v) in &r.headers {
                            ui.label(RichText::new(k).monospace().strong());
                            ui.label(RichText::new(v).monospace());
                            ui.end_row();
                        }
                    });
                });
            }
        }
    }

    fn envs_window(&mut self, ctx: &egui::Context) {
        if !self.show_envs {
            return;
        }
        let before = self.envs.clone();
        let mut open = true;
        egui::Window::new("Environments").open(&mut open).default_width(520.0).collapsible(false).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if !self.envs.envs.is_empty() {
                    self.edit_env = self.edit_env.min(self.envs.envs.len() - 1);
                    egui::ComboBox::from_id_salt("edit_env")
                        .selected_text(&self.envs.envs[self.edit_env].name)
                        .show_ui(ui, |ui| {
                            for (i, e) in self.envs.envs.iter().enumerate() {
                                ui.selectable_value(&mut self.edit_env, i, &e.name);
                            }
                        });
                }
                if ui.button("＋ Environment").clicked() {
                    let n = self.envs.envs.len() + 1;
                    self.envs.envs.push(Env { name: format!("env{n}"), vars: vec![Var::default()] });
                    self.edit_env = self.envs.envs.len() - 1;
                }
            });
            if self.envs.envs.is_empty() {
                ui.add_space(6.0);
                ui.weak(
                    "Create an environment (e.g. local, staging, prod), then use {{name}} in the URL, headers or body.",
                );
                return;
            }
            ui.separator();
            let i = self.edit_env;
            let old_name = self.envs.envs[i].name.clone();
            let mut remove_env = false;
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut self.envs.envs[i].name);
                if ui.button("Delete").clicked() {
                    remove_env = true;
                }
            });
            if self.envs.active.as_deref() == Some(old_name.as_str()) && self.envs.envs[i].name != old_name {
                self.envs.active = Some(self.envs.envs[i].name.clone());
            }
            ui.add_space(4.0);
            let vars = &mut self.envs.envs[i].vars;
            let mut remove = None;
            egui::Grid::new("vars").num_columns(3).spacing([6.0, 4.0]).show(ui, |ui| {
                for (j, v) in vars.iter_mut().enumerate() {
                    ui.add(
                        egui::TextEdit::singleline(&mut v.key)
                            .hint_text("name")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(140.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut v.value)
                            .hint_text("value")
                            .font(egui::TextStyle::Monospace)
                            .desired_width(300.0),
                    );
                    if ui.small_button("✕").clicked() {
                        remove = Some(j);
                    }
                    ui.end_row();
                }
            });
            if let Some(j) = remove {
                vars.remove(j);
            }
            if ui.button("＋ Variable").clicked() {
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
            self.message = Some((format!("Could not save environments: {e}"), true));
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let before = self.settings.clone();
        let mut open = true;
        let mut check_now = false;
        egui::Window::new("Settings").open(&mut open).default_width(360.0).resizable(false).collapsible(false).show(
            ctx,
            |ui| {
                egui::Grid::new("settings").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                    ui.label("Theme");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.settings.theme, Theme::Auto, "Auto");
                        ui.selectable_value(&mut self.settings.theme, Theme::Light, "Light");
                        ui.selectable_value(&mut self.settings.theme, Theme::Dark, "Dark");
                    });
                    ui.end_row();

                    ui.label("Updates");
                    ui.checkbox(&mut self.settings.check_updates, "Check on startup");
                    ui.end_row();

                    ui.label("Version");
                    ui.horizontal(|ui| {
                        ui.label(updater::VERSION);
                        match &self.update {
                            Update::Checking { manual: true } => {
                                ui.spinner();
                            }
                            Update::UpToDate => {
                                ui.label(RichText::new("Up to date").color(GREEN));
                            }
                            Update::Available(rel) => {
                                ui.label(RichText::new(format!("{} available", rel.version)).color(BLUE));
                            }
                            Update::Failed(e) => {
                                ui.label(RichText::new("Check failed").color(RED)).on_hover_text(e);
                            }
                            _ => {}
                        }
                        if ui.small_button("Check now").clicked() {
                            check_now = true;
                        }
                    });
                    ui.end_row();

                    ui.label("Data");
                    ui.label(RichText::new(self.store.root().display().to_string()).monospace().small());
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.hyperlink_to(
                    RichText::new("Plume on GitHub").small(),
                    format!("https://github.com/{}", updater::REPO),
                );
            },
        );
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
                self.message = Some((format!("Could not save settings: {e}"), true));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        let ctx = ui.ctx().clone();

        let (send, save, new, settings) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::Comma),
            )
        });
        if send && self.pending.is_none() {
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

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("list")
            .resizable(true)
            .default_size(240.0)
            .size_range(160.0..=480.0)
            .show(ui, |ui| self.sidebar(ui));
        let half = ui.available_height() * 0.5;
        egui::Panel::bottom("response")
            .resizable(true)
            .default_size(half)
            .min_size(80.0)
            .show(ui, |ui| self.response_view(ui));
        egui::CentralPanel::default().show(ui, |ui| self.request_editor(ui));

        self.envs_window(&ctx);
        self.settings_window(&ctx);
    }
}

// ---------- Small widgets and helpers ----------

fn list_item(ui: &mut egui::Ui, method: &str, name: &str, dirty: bool, selected: bool) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    let mono = egui::FontId::monospace(11.0);
    let body = egui::TextStyle::Body.resolve(ui.style());
    let text_color = ui.visuals().text_color();
    job.append(&format!("{:<6}", short_method(method)), 0.0, egui::TextFormat::simple(mono, method_color(method)));
    job.append(name, 4.0, egui::TextFormat::simple(body.clone(), text_color));
    if dirty {
        job.append(" •", 0.0, egui::TextFormat::simple(body, ORANGE));
    }
    ui.add_sized([ui.available_width(), 22.0], egui::Button::selectable(selected, job).truncate())
}

fn headers_editor(ui: &mut egui::Ui, headers: &mut Vec<Header>) {
    egui::ScrollArea::vertical().id_salt("req_headers").show(ui, |ui| {
        let mut remove = None;
        let w = (ui.available_width() - 70.0).max(200.0);
        egui::Grid::new("headers").num_columns(4).spacing([6.0, 4.0]).show(ui, |ui| {
            for (i, h) in headers.iter_mut().enumerate() {
                ui.checkbox(&mut h.enabled, "");
                ui.add(
                    egui::TextEdit::singleline(&mut h.key)
                        .hint_text("Header")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(w * 0.35),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut h.value)
                        .hint_text("Value")
                        .font(egui::TextStyle::Monospace)
                        .desired_width(w * 0.65),
                );
                if ui.small_button("✕").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            headers.remove(i);
        }
        if ui.button("＋ Header").clicked() {
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
        format!("{text}\n\n… truncated for display ({}). “Copy” copies everything.", human_size(bytes.len()))
    } else {
        text
    }
}

fn short_method(m: &str) -> &str {
    match m {
        "DELETE" => "DEL",
        "OPTIONS" => "OPT",
        _ => m,
    }
}

fn method_color(m: &str) -> Color32 {
    match m {
        "GET" => GREEN,
        "POST" => ORANGE,
        "PUT" => BLUE,
        "PATCH" => PURPLE,
        "DELETE" => RED,
        _ => Color32::GRAY,
    }
}

fn status_color(s: u16) -> Color32 {
    match s {
        200..=299 => GREEN,
        300..=399 => BLUE,
        400..=499 => ORANGE,
        _ => RED,
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
