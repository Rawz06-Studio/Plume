//! HTML preview: the system webview (WebKit on macOS, WebView2 on Windows) laid over the
//! response area. JavaScript is off and links don't navigate: it's a look, not a browser.
//! Elsewhere (Linux) there is no embedded preview; "Open in Browser" still works.

use eframe::egui;
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// `<base href>` so relative CSS and images load from the server that answered.
pub fn with_base(html: &str, url: &str) -> String {
    let base = format!("<base href=\"{}\">", url.replace('"', "%22"));
    let lower = html.get(..html.len().min(4096)).unwrap_or("").to_ascii_lowercase();
    match lower.find("<head").and_then(|i| lower[i..].find('>').map(|j| i + j + 1)) {
        Some(at) => format!("{}{base}{}", &html[..at], &html[at..]),
        None => format!("{base}{html}"),
    }
}

fn create_preview_file(html: &str) -> io::Result<PathBuf> {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    for _ in 0..10 {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "plume-preview-{}-{timestamp}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => {
                let path = dir.join("preview.html");
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                if let Err(error) = options.open(&path).and_then(|mut file| file.write_all(html.as_bytes())) {
                    let _ = fs::remove_dir_all(&dir);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(io::ErrorKind::AlreadyExists, "could not create a unique preview directory"))
}

/// Writes the page to a temporary file and opens it in the default browser.
pub fn open_in_browser(ctx: &egui::Context, html: &str) -> Result<(), String> {
    let path = create_preview_file(html).map_err(|e| e.to_string())?;
    let path = path.to_string_lossy().replace('\\', "/");
    let url = if path.starts_with('/') { format!("file://{path}") } else { format!("file:///{path}") };
    ctx.open_url(egui::OpenUrl::new_tab(url));
    Ok(())
}

pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use native::Preview;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native {
    use eframe::egui;
    use raw_window_handle::HasWindowHandle;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect, WebView, WebViewBuilder};

    #[derive(Default)]
    pub struct Preview {
        view: Option<WebView>,
        /// Which response is loaded (see `show`).
        loaded: Option<u64>,
        visible: bool,
        bounds: Option<egui::Rect>,
        pub error: Option<String>,
    }

    fn bounds(r: egui::Rect) -> Rect {
        Rect {
            position: LogicalPosition::new(r.left() as f64, r.top() as f64).into(),
            size: LogicalSize::new(r.width().max(1.0) as f64, r.height().max(1.0) as f64).into(),
        }
    }

    impl Preview {
        /// Shows response `key` (its `html`) over `rect`, in points.
        pub fn show(&mut self, window: &impl HasWindowHandle, rect: egui::Rect, key: u64, html: &str) {
            if self.error.is_some() {
                return;
            }
            match &self.view {
                None => {
                    let built = WebViewBuilder::new()
                        .with_html(html)
                        .with_bounds(bounds(rect))
                        .with_javascript_disabled()
                        .with_devtools(false)
                        // Only the page itself loads; clicked links go nowhere.
                        .with_navigation_handler(|url| !url.starts_with("http"))
                        .build_as_child(window);
                    match built {
                        Ok(v) => {
                            self.view = Some(v);
                            self.loaded = Some(key);
                            self.visible = true;
                            self.bounds = Some(rect);
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                Some(view) => {
                    if self.loaded != Some(key) {
                        let _ = view.load_html(html);
                        self.loaded = Some(key);
                    }
                    if self.bounds != Some(rect) {
                        let _ = view.set_bounds(bounds(rect));
                        self.bounds = Some(rect);
                    }
                    if !self.visible {
                        let _ = view.set_visible(true);
                        self.visible = true;
                    }
                }
            }
        }

        pub fn hide(&mut self) {
            if let Some(view) = &self.view
                && self.visible
            {
                let _ = view.set_visible(false);
                let _ = view.focus_parent();
                self.visible = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{create_preview_file, with_base};
    use std::fs;

    #[test]
    fn preview_files_are_unique_and_private() {
        let first = create_preview_file("<p>first</p>").unwrap();
        let second = create_preview_file("<p>second</p>").unwrap();

        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&first).unwrap(), "<p>first</p>");
        assert_eq!(fs::read_to_string(&second).unwrap(), "<p>second</p>");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(first.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(&first).unwrap().permissions().mode() & 0o777, 0o600);
        }

        fs::remove_dir_all(first.parent().unwrap()).unwrap();
        fs::remove_dir_all(second.parent().unwrap()).unwrap();
    }

    #[test]
    fn base_goes_in_head() {
        assert_eq!(
            with_base("<html><head><title>x</title>", "http://a/b"),
            "<html><head><base href=\"http://a/b\"><title>x</title>"
        );
        assert_eq!(with_base("<p>hi</p>", "http://a"), "<base href=\"http://a\"><p>hi</p>");
    }
}
