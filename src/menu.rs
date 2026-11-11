//! App actions and menus: the native menu bar on macOS, an in-window menu bar elsewhere.

use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    About,
    Settings,
    CheckUpdates,
    NewRequest,
    Save,
    Duplicate,
    Delete,
    Send,
    FormatJson,
    Environments,
    GitHub,
    Quit,
}

#[cfg(target_os = "macos")]
const ALL: [Action; 12] = [
    Action::About,
    Action::Settings,
    Action::CheckUpdates,
    Action::NewRequest,
    Action::Save,
    Action::Duplicate,
    Action::Delete,
    Action::Send,
    Action::FormatJson,
    Action::Environments,
    Action::GitHub,
    Action::Quit,
];

impl Action {
    #[cfg(target_os = "macos")]
    fn id(self) -> &'static str {
        match self {
            Action::About => "about",
            Action::Settings => "settings",
            Action::CheckUpdates => "check-updates",
            Action::NewRequest => "new-request",
            Action::Save => "save",
            Action::Duplicate => "duplicate",
            Action::Delete => "delete",
            Action::Send => "send",
            Action::FormatJson => "format-json",
            Action::Environments => "environments",
            Action::GitHub => "github",
            Action::Quit => "quit",
        }
    }

    /// Keyboard shortcuts handled by egui (on macOS the menu bar catches them first).
    pub fn from_keys(i: &mut egui::InputState) -> Option<Self> {
        use egui::{Key, Modifiers};
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        [
            (cmd_shift, Key::F, Action::FormatJson),
            (cmd, Key::Enter, Action::Send),
            (cmd, Key::S, Action::Save),
            (cmd, Key::N, Action::NewRequest),
            (cmd, Key::D, Action::Duplicate),
            (cmd, Key::E, Action::Environments),
            (cmd, Key::Comma, Action::Settings),
        ]
        .into_iter()
        .find(|(m, k, _)| i.consume_key(*m, *k))
        .map(|(_, _, a)| a)
    }
}

/// Which actions currently make sense (used to grey out menu items).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Enabled {
    pub save: bool,
    pub saved_request: bool,
    pub send: bool,
}

// ---------- macOS: native menu bar ----------

#[cfg(target_os = "macos")]
pub use macos::NativeMenu;

#[cfg(target_os = "macos")]
mod macos {
    use super::{ALL, Action, Enabled};
    use eframe::egui;
    use muda::accelerator::{Accelerator, Code, Modifiers};
    use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use std::sync::mpsc::{self, Receiver};

    pub struct NativeMenu {
        _menu: Menu,
        save: MenuItem,
        duplicate: MenuItem,
        delete: MenuItem,
        send: MenuItem,
        enabled: Enabled,
        rx: Receiver<Action>,
    }

    fn item(a: Action, text: &str, key: Option<(Modifiers, Code)>) -> MenuItem {
        MenuItem::with_id(a.id(), text, true, key.map(|(m, c)| Accelerator::new(m, c)))
    }

    impl NativeMenu {
        pub fn install(ctx: &egui::Context) -> Self {
            let cmd = Modifiers::META;
            let sep = PredefinedMenuItem::separator;

            let app = Submenu::with_items(
                "Plume",
                true,
                &[
                    &item(Action::About, "About Plume", None),
                    &item(Action::CheckUpdates, "Check for Updates…", None),
                    &sep(),
                    &item(Action::Settings, "Settings…", Some((cmd, Code::Comma))),
                    &sep(),
                    &PredefinedMenuItem::services(None),
                    &sep(),
                    &PredefinedMenuItem::hide(Some("Hide Plume")),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &sep(),
                    &PredefinedMenuItem::quit(Some("Quit Plume")),
                ],
            )
            .expect("app menu");

            let save = item(Action::Save, "Save", Some((cmd, Code::KeyS)));
            let duplicate = item(Action::Duplicate, "Duplicate", Some((cmd, Code::KeyD)));
            let delete = item(Action::Delete, "Delete Request", None);
            let file = Submenu::with_items(
                "File",
                true,
                &[
                    &item(Action::NewRequest, "New Request", Some((cmd, Code::KeyN))),
                    &sep(),
                    &save,
                    &duplicate,
                    &sep(),
                    &delete,
                    &sep(),
                    &PredefinedMenuItem::close_window(None),
                ],
            )
            .expect("file menu");

            let send = item(Action::Send, "Send", Some((cmd, Code::Enter)));
            let request = Submenu::with_items(
                "Request",
                true,
                &[
                    &send,
                    &item(Action::FormatJson, "Format JSON Body", Some((cmd | Modifiers::SHIFT, Code::KeyF))),
                    &sep(),
                    &item(Action::Environments, "Environments…", Some((cmd, Code::KeyE))),
                ],
            )
            .expect("request menu");

            let window = Submenu::with_items(
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(None),
                    &PredefinedMenuItem::maximize(Some("Zoom")),
                    &PredefinedMenuItem::fullscreen(None),
                    &sep(),
                    &PredefinedMenuItem::bring_all_to_front(None),
                ],
            )
            .expect("window menu");

            let help = Submenu::with_items("Help", true, &[&item(Action::GitHub, "Plume on GitHub", None)])
                .expect("help menu");

            let menu = Menu::with_items(&[&app, &file, &request, &window, &help]).expect("menu bar");
            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            help.set_as_help_menu_for_nsapp();

            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(a) = ALL.into_iter().find(|a| a.id() == event.id().as_ref()) {
                    let _ = tx.send(a);
                    ctx.request_repaint();
                }
            }));

            Self { _menu: menu, save, duplicate, delete, send, enabled: Enabled::default(), rx }
        }

        pub fn poll(&self) -> Option<Action> {
            self.rx.try_recv().ok()
        }

        pub fn set_enabled(&mut self, e: Enabled) {
            if e == self.enabled {
                return;
            }
            self.save.set_enabled(e.save);
            self.duplicate.set_enabled(e.saved_request);
            self.delete.set_enabled(e.saved_request);
            self.send.set_enabled(e.send);
            self.enabled = e;
        }
    }
}

// ---------- Windows / Linux: menu bar inside the window ----------

/// Draws "File · Request · Help" at the top of the window, Windows 11 style.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn in_window_bar(ui: &mut egui::Ui, e: Enabled) -> Option<Action> {
    let mut action = None;
    let mut entry = |ui: &mut egui::Ui, a: Action, text: &str, shortcut: &str, enabled: bool| {
        let button = egui::Button::new(text).shortcut_text(shortcut);
        if ui.add_enabled(enabled, button).clicked() {
            action = Some(a);
        }
    };
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            entry(ui, Action::NewRequest, "New request", "Ctrl+N", true);
            ui.separator();
            entry(ui, Action::Save, "Save", "Ctrl+S", e.save);
            entry(ui, Action::Duplicate, "Duplicate", "Ctrl+D", e.saved_request);
            entry(ui, Action::Delete, "Delete request", "", e.saved_request);
            ui.separator();
            entry(ui, Action::Settings, "Settings", "Ctrl+,", true);
            ui.separator();
            entry(ui, Action::Quit, "Exit", "Alt+F4", true);
        });
        ui.menu_button("Request", |ui| {
            entry(ui, Action::Send, "Send", "Ctrl+Enter", e.send);
            entry(ui, Action::FormatJson, "Format JSON body", "Ctrl+Shift+F", true);
            ui.separator();
            entry(ui, Action::Environments, "Environments…", "Ctrl+E", true);
        });
        ui.menu_button("Help", |ui| {
            entry(ui, Action::CheckUpdates, "Check for updates", "", true);
            entry(ui, Action::GitHub, "Plume on GitHub", "", true);
            ui.separator();
            entry(ui, Action::About, "About Plume", "", true);
        });
    });
    action
}
