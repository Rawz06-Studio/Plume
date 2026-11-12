//! Response bodies: what kind of content they hold (from Content-Type, then by sniffing)
//! and how to show it — JSON tree, highlighted text, image, hex dump.
//!
//! Text is shown row by row (only the visible rows are laid out), so multi-megabyte
//! bodies scroll smoothly.

use super::theme::{Palette, pal};
use crate::http;
use eframe::egui::{
    self, Color32, CornerRadius, FontId, Rect, Sense, TextFormat, TextStyle, Ui, text::LayoutJob, vec2,
};
use serde_json::Value;
use std::collections::HashSet;
use std::ops::Range;

/// Text views are truncated past this size (Copy still copies everything).
pub const MAX_DISPLAY: usize = 2 * 1024 * 1024;
/// Hex dumps show this many bytes at most.
const MAX_HEX: usize = 64 * 1024;
/// Arrays longer than this are shown in groups of this many items.
const GROUP: usize = 100;
/// "Expand all" stops after this many rows.
const MAX_EXPAND: usize = 20_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Json,
    Html,
    Xml,
    Text,
    Image,
    Binary,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Tree,
    Pretty,
    Raw,
    Preview,
    Hex,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Tree => "Tree",
            View::Pretty => "Pretty",
            View::Raw => "Raw",
            View::Preview => "Preview",
            View::Hex => "Hex",
        }
    }
}

impl Kind {
    /// Views offered for this kind, the default first.
    pub fn views(self) -> &'static [View] {
        match self {
            Kind::Json => &[View::Tree, View::Pretty, View::Raw],
            Kind::Html => &[View::Preview, View::Raw],
            Kind::Image => &[View::Preview, View::Hex],
            Kind::Xml | Kind::Text => &[View::Raw, View::Hex],
            Kind::Binary => &[View::Hex],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Json => "JSON",
            Kind::Html => "HTML",
            Kind::Xml => "XML",
            Kind::Text => "Text",
            Kind::Image => "Image",
            Kind::Binary => "Binary",
        }
    }
}

pub struct Body {
    pub kind: Kind,
    /// Media type without parameters, e.g. `application/json` (may be empty).
    pub media_type: String,
    pub json: Option<Value>,
    raw: Code,
    pretty: Option<Code>,
    hex: Option<Code>,
    tree: Tree,
    image: Option<Result<egui::TextureHandle, String>>,
}

impl Body {
    pub fn new(resp: &http::Response) -> Self {
        let media_type = resp
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
            .unwrap_or_default();
        let bytes = &resp.body;
        let json = if bytes.is_empty() { None } else { serde_json::from_slice::<Value>(bytes).ok() };
        let kind = detect(&media_type, bytes, json.is_some());

        let raw_text = truncated_text(bytes);
        let lang = match kind {
            Kind::Json => Lang::Json,
            Kind::Html | Kind::Xml => Lang::Markup,
            _ => Lang::Plain,
        };
        let raw = Code::new(raw_text, lang, true);
        let pretty = json
            .as_ref()
            .and_then(|v| serde_json::to_string_pretty(v).ok())
            .map(|s| Code::new(cut(s, MAX_DISPLAY), Lang::Json, true));
        let hex = kind.views().contains(&View::Hex).then(|| Code::new(hex_dump(bytes), Lang::Plain, false));
        let mut tree = Tree::default();
        if let Some(v) = &json {
            tree.open_defaults(v);
        }
        Self { kind, media_type, json, raw, pretty, hex, tree, image: None }
    }

    /// The text the Copy button copies for `view`.
    pub fn copy_text(&self, view: View, bytes: &[u8]) -> String {
        match view {
            View::Pretty => self.pretty.as_ref().map(|c| c.text.clone()).unwrap_or_default(),
            View::Hex => self.hex.as_ref().map(|c| c.text.clone()).unwrap_or_default(),
            _ => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    /// Toolbar extras for the current view (expand/collapse for the tree).
    pub fn view_tools(&mut self, ui: &mut Ui, view: View) {
        if view == View::Tree
            && let Some(json) = &self.json
        {
            if super::widgets::button(ui, "Collapse", None, true).clicked() {
                self.tree.collapse_all();
            }
            if super::widgets::button(ui, "Expand all", None, true).clicked() {
                self.tree.expand_all(json);
            }
        }
    }

    /// Draws the body with `view`. Returns the rect the HTML preview should cover, when the
    /// view is `Preview` on an HTML body (the webview is native and drawn by the caller).
    pub fn show(&mut self, ui: &mut Ui, view: View, bytes: &[u8]) -> Option<Rect> {
        let p = pal(ui);
        if bytes.is_empty() {
            ui.centered_and_justified(|ui| ui.label(egui::RichText::new("Empty body").color(p.weak)));
            return None;
        }
        match (view, self.kind) {
            (View::Tree, _) if self.json.is_some() => {
                let json = self.json.as_ref().expect("checked");
                self.tree.show(ui, json, &p);
            }
            (View::Pretty, _) if self.pretty.is_some() => self.pretty.as_mut().expect("checked").show(ui, &p),
            (View::Hex, _) if self.hex.is_some() => self.hex.as_mut().expect("checked").show(ui, &p),
            (View::Preview, Kind::Image) => self.show_image(ui, bytes, &p),
            (View::Preview, Kind::Html) => return Some(ui.available_rect_before_wrap()),
            _ => self.raw.show(ui, &p),
        }
        None
    }

    fn show_image(&mut self, ui: &mut Ui, bytes: &[u8], p: &Palette) {
        let image = self.image.get_or_insert_with(|| {
            let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            Ok(ui.ctx().load_texture("response-image", color, egui::TextureOptions::LINEAR))
        });
        match image {
            Ok(tex) => {
                let size = tex.size_vec2();
                egui::ScrollArea::both().id_salt("resp_image").auto_shrink(false).show(ui, |ui| {
                    ui.label(egui::RichText::new(format!("{} × {} px", size.x, size.y)).color(p.weak));
                    ui.add_space(6.0);
                    let max = ui.available_width().max(64.0);
                    let shown = if size.x > max { size * (max / size.x) } else { size };
                    ui.add(egui::Image::new((tex.id(), shown)).corner_radius(CornerRadius::same(4)));
                });
            }
            Err(e) => {
                ui.label(egui::RichText::new(format!("Can't decode this image: {e}")).color(p.weak));
            }
        }
    }
}

fn detect(media_type: &str, bytes: &[u8], is_json: bool) -> Kind {
    let m = media_type;
    if m.contains("json") {
        return if is_json || bytes.is_empty() { Kind::Json } else { Kind::Text };
    }
    if m == "text/html" || m == "application/xhtml+xml" {
        return Kind::Html;
    }
    if m.contains("xml") {
        return Kind::Xml;
    }
    if m.starts_with("image/") && image::guess_format(bytes).is_ok() {
        return Kind::Image;
    }
    if m.starts_with("text/") || m.contains("javascript") || m.contains("yaml") || m.contains("urlencoded") {
        return Kind::Text;
    }
    // No (or a generic) Content-Type: look at the bytes.
    if is_json {
        return Kind::Json;
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).trim_start().to_ascii_lowercase();
    if head.starts_with("<!doctype html") || head.starts_with("<html") {
        return Kind::Html;
    }
    if head.starts_with("<?xml") {
        return Kind::Xml;
    }
    if image::guess_format(bytes).is_ok() {
        return Kind::Image;
    }
    if looks_like_text(bytes) { Kind::Text } else { Kind::Binary }
}

fn looks_like_text(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    if head.contains(&0) {
        return false;
    }
    match std::str::from_utf8(head) {
        Ok(_) => true,
        // A multi-byte character cut at the end of the sample is fine.
        Err(e) => e.error_len().is_none(),
    }
}

fn truncated_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_DISPLAY)]).into_owned();
    if bytes.len() > MAX_DISPLAY {
        format!("{text}\n\n… truncated for display. Copy copies everything.")
    } else {
        text
    }
}

fn cut(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n\n… truncated for display. Copy copies everything.");
    }
    s
}

fn hex_dump(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let shown = &bytes[..bytes.len().min(MAX_HEX)];
    let mut out = String::with_capacity(shown.len() * 4 + 64);
    for (i, chunk) in shown.chunks(16).enumerate() {
        let _ = write!(out, "{:08x}  ", i * 16);
        for j in 0..16 {
            match chunk.get(j) {
                Some(b) => {
                    let _ = write!(out, "{b:02x} ");
                }
                None => out.push_str("   "),
            }
            if j == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        out.extend(chunk.iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' }));
        out.push_str("|\n");
    }
    if bytes.len() > MAX_HEX {
        let _ = write!(out, "\n… first {} KB of {} bytes", MAX_HEX / 1024, bytes.len());
    }
    out
}

// ---------- Highlighted text, laid out row by row ----------

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Plain,
    Json,
    Markup,
}

#[derive(Clone, Copy, PartialEq)]
enum Tok {
    Key,
    Str,
    Num,
    Lit,
    Punct,
    Tag,
    Attr,
    Comment,
}

fn tok_color(t: Tok, p: &Palette) -> Color32 {
    match t {
        Tok::Key | Tok::Tag => p.purple,
        Tok::Str => p.green,
        Tok::Num => p.blue,
        Tok::Lit | Tok::Attr => p.orange,
        Tok::Punct | Tok::Comment => p.weak,
    }
}

struct Code {
    text: String,
    /// Sorted, non-overlapping highlighted ranges (bytes); gaps use the text color.
    spans: Vec<(u32, u32, Tok)>,
    wrap: bool,
    /// Visual rows (byte ranges), rebuilt when the number of columns changes.
    rows: Vec<Range<usize>>,
    cols: usize,
}

impl Code {
    fn new(text: String, lang: Lang, wrap: bool) -> Self {
        let spans = match lang {
            Lang::Plain => Vec::new(),
            Lang::Json => json_spans(&text),
            Lang::Markup => markup_spans(&text),
        };
        Self { text, spans, wrap, rows: Vec::new(), cols: 0 }
    }

    fn layout_rows(&mut self, cols: usize) {
        if cols == self.cols && !self.rows.is_empty() {
            return;
        }
        self.cols = cols;
        self.rows.clear();
        let mut start = 0;
        for line in self.text.split_inclusive('\n') {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let content = content.strip_suffix('\r').unwrap_or(content);
            let mut row_start = start;
            let mut n = 0;
            for (i, _) in content.char_indices() {
                if n == cols {
                    self.rows.push(row_start..start + i);
                    row_start = start + i;
                    n = 0;
                }
                n += 1;
            }
            self.rows.push(row_start..start + content.len());
            start += line.len();
        }
    }

    fn show(&mut self, ui: &mut Ui, p: &Palette) {
        let font = TextStyle::Monospace.resolve(ui.style());
        let char_w = ui.painter().layout_no_wrap("0".repeat(10), font.clone(), p.text).size().x / 10.0;
        let row_h = ui.text_style_height(&TextStyle::Monospace);
        let cols = if self.wrap {
            ((ui.available_width() - 14.0) / char_w.max(1.0)).floor().max(20.0) as usize
        } else {
            usize::MAX
        };
        self.layout_rows(cols);

        let area = if self.wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
        area.id_salt(("code", self.text.len(), self.wrap)).auto_shrink(false).show_rows(
            ui,
            row_h,
            self.rows.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for r in &self.rows[range] {
                    let job = self.row_job(r.clone(), &font, p);
                    ui.add(egui::Label::new(job).selectable(true).wrap_mode(egui::TextWrapMode::Extend));
                }
            },
        );
    }

    fn row_job(&self, row: Range<usize>, font: &FontId, p: &Palette) -> LayoutJob {
        let mut job = LayoutJob::default();
        let fmt = |c: Color32| TextFormat::simple(font.clone(), c);
        let text = &self.text;
        // First span that ends after the row start.
        let mut i = self.spans.partition_point(|s| (s.1 as usize) <= row.start);
        let mut pos = row.start;
        while pos < row.end {
            match self.spans.get(i) {
                Some(&(s, e, t)) if (s as usize) < row.end => {
                    let (s, e) = ((s as usize).max(pos), (e as usize).min(row.end));
                    if s > pos {
                        job.append(&text[pos..s], 0.0, fmt(p.text));
                    }
                    job.append(&text[s..e], 0.0, fmt(tok_color(t, p)));
                    pos = e;
                    i += 1;
                }
                _ => {
                    job.append(&text[pos..row.end], 0.0, fmt(p.text));
                    pos = row.end;
                }
            }
        }
        if job.is_empty() {
            job.append(" ", 0.0, fmt(p.text));
        }
        job
    }
}

fn json_spans(text: &str) -> Vec<(u32, u32, Tok)> {
    let b = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let start = i;
        let tok = match c {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                let mut j = i;
                while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                    j += 1;
                }
                if b.get(j) == Some(&b':') { Tok::Key } else { Tok::Str }
            }
            b'-' | b'0'..=b'9' => {
                while i < b.len() && matches!(b[i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    i += 1;
                }
                Tok::Num
            }
            b't' | b'f' | b'n' => {
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                Tok::Lit
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                i += 1;
                Tok::Punct
            }
            _ => {
                i += 1;
                continue;
            }
        };
        push_span(&mut spans, start, i, tok);
    }
    spans
}

fn markup_spans(text: &str) -> Vec<(u32, u32, Tok)> {
    let b = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if text[i..].starts_with("<!--") {
            let end = text[i..].find("-->").map_or(b.len(), |e| i + e + 3);
            push_span(&mut spans, i, end, Tok::Comment);
            i = end;
            continue;
        }
        // Tag name, e.g. `<div`, `</div`, `<!DOCTYPE`, `<?xml`.
        let start = i;
        i += 1;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && !(b[i] == b'/' && i > start + 1) {
            i += 1;
        }
        push_span(&mut spans, start, i, Tok::Tag);
        // Attributes until `>`.
        while i < b.len() && b[i] != b'>' {
            match b[i] {
                b'"' | b'\'' => {
                    let q = b[i];
                    let s = i;
                    i += 1;
                    while i < b.len() && b[i] != q {
                        i += 1;
                    }
                    i = (i + 1).min(b.len());
                    push_span(&mut spans, s, i, Tok::Str);
                }
                c if c.is_ascii_alphabetic() => {
                    let s = i;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'-' | b'_' | b':' | b'.')) {
                        i += 1;
                    }
                    push_span(&mut spans, s, i, Tok::Attr);
                }
                _ => i += 1,
            }
        }
        if i < b.len() {
            let s = if i > 0 && b[i - 1] == b'/' { i - 1 } else { i };
            push_span(&mut spans, s, i + 1, Tok::Tag);
            i += 1;
        }
    }
    spans
}

fn push_span(spans: &mut Vec<(u32, u32, Tok)>, s: usize, e: usize, t: Tok) {
    let s = s.max(spans.last().map_or(0, |l| l.1 as usize));
    if e > s {
        spans.push((s as u32, e as u32, t));
    }
}

// ---------- JSON tree ----------

enum Label {
    None,
    Key(String),
    Index(usize),
    Group(usize, usize),
}

struct Row {
    depth: usize,
    /// JSON pointer of the value (of the array, for a group).
    pointer: String,
    /// jq-style path shown by "Copy path".
    path: String,
    label: Label,
}

impl Row {
    fn open_key(&self) -> String {
        match self.label {
            Label::Group(s, _) => format!("{}#{s}", self.pointer),
            _ => self.pointer.clone(),
        }
    }
}

#[derive(Default)]
struct Tree {
    open: HashSet<String>,
    rows: Vec<Row>,
    dirty: bool,
}

fn is_container(v: &Value) -> bool {
    matches!(v, Value::Object(_) | Value::Array(_)) && !is_empty_container(v)
}

fn is_empty_container(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

fn child_path(path: &str, key: &str) -> String {
    let ident = !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if ident { format!("{path}.{key}") } else { format!("{path}[{}]", serde_json::to_string(key).unwrap_or_default()) }
}

impl Tree {
    /// The root and its direct children start open.
    fn open_defaults(&mut self, root: &Value) {
        self.open.insert(String::new());
        if let Value::Object(m) = root {
            for (k, v) in m.iter().take(50) {
                if is_container(v) && m.len() <= 20 {
                    self.open.insert(format!("/{}", escape_pointer(k)));
                }
            }
        }
        self.dirty = true;
    }

    fn collapse_all(&mut self) {
        self.open.clear();
        self.open.insert(String::new());
        self.dirty = true;
    }

    fn expand_all(&mut self, root: &Value) {
        let mut count = 0;
        let mut stack = vec![(String::new(), root)];
        while let Some((ptr, v)) = stack.pop() {
            if count > MAX_EXPAND {
                break;
            }
            match v {
                Value::Object(m) => {
                    self.open.insert(ptr.clone());
                    count += m.len();
                    for (k, c) in m {
                        stack.push((format!("{ptr}/{}", escape_pointer(k)), c));
                    }
                }
                Value::Array(a) => {
                    self.open.insert(ptr.clone());
                    count += a.len();
                    if a.len() > GROUP {
                        for s in (0..a.len()).step_by(GROUP) {
                            self.open.insert(format!("{ptr}#{s}"));
                        }
                    }
                    for (i, c) in a.iter().enumerate() {
                        stack.push((format!("{ptr}/{i}"), c));
                    }
                }
                _ => {}
            }
        }
        self.dirty = true;
    }

    fn rebuild(&mut self, root: &Value) {
        self.rows.clear();
        if is_container(root) {
            self.push_children(root, 0, "", ".");
        } else {
            self.rows.push(Row { depth: 0, pointer: String::new(), path: ".".into(), label: Label::None });
        }
        self.dirty = false;
    }

    fn push_children(&mut self, v: &Value, depth: usize, pointer: &str, path: &str) {
        let base = if path == "." { "" } else { path };
        match v {
            Value::Object(m) => {
                for (k, c) in m {
                    let ptr = format!("{pointer}/{}", escape_pointer(k));
                    let p = child_path(base, k);
                    self.push_node(c, depth, ptr, p, Label::Key(k.clone()));
                }
            }
            Value::Array(a) if a.len() > GROUP => {
                for s in (0..a.len()).step_by(GROUP) {
                    let e = (s + GROUP).min(a.len()) - 1;
                    let row =
                        Row { depth, pointer: pointer.to_string(), path: path.to_string(), label: Label::Group(s, e) };
                    let open = self.open.contains(&row.open_key());
                    self.rows.push(row);
                    if open {
                        for (i, c) in a.iter().enumerate().take(e + 1).skip(s) {
                            self.push_node(
                                c,
                                depth + 1,
                                format!("{pointer}/{i}"),
                                format!("{base}[{i}]"),
                                Label::Index(i),
                            );
                        }
                    }
                }
            }
            Value::Array(a) => {
                for (i, c) in a.iter().enumerate() {
                    self.push_node(c, depth, format!("{pointer}/{i}"), format!("{base}[{i}]"), Label::Index(i));
                }
            }
            _ => {}
        }
    }

    fn push_node(&mut self, v: &Value, depth: usize, pointer: String, path: String, label: Label) {
        let open = is_container(v) && self.open.contains(&pointer);
        self.rows.push(Row { depth, pointer: pointer.clone(), path: path.clone(), label });
        if open {
            self.push_children(v, depth + 1, &pointer, &path);
        }
    }

    fn show(&mut self, ui: &mut Ui, root: &Value, p: &Palette) {
        if self.dirty || self.rows.is_empty() {
            self.rebuild(root);
        }
        let row_h = 22.0;
        let mut toggle = None;
        let mut expand = None;
        egui::ScrollArea::vertical().id_salt("json_tree").auto_shrink(false).show_rows(
            ui,
            row_h,
            self.rows.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in &self.rows[range] {
                    let Some(v) = root.pointer(&row.pointer) else { continue };
                    let is_group = matches!(row.label, Label::Group(..));
                    let container = is_group || is_container(v);
                    let open = container && self.open.contains(&row.open_key());
                    let resp = tree_row(ui, row, v, container, open, row_h, p);
                    if container && resp.clicked() {
                        toggle = Some(row.open_key());
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Copy value").clicked() {
                            let text = match (v, is_group) {
                                (Value::String(s), false) => s.clone(),
                                (Value::Array(a), true) => {
                                    let Label::Group(s, e) = row.label else { unreachable!() };
                                    serde_json::to_string_pretty(&a[s..=e]).unwrap_or_default()
                                }
                                _ => serde_json::to_string_pretty(v).unwrap_or_default(),
                            };
                            ui.ctx().copy_text(text);
                        }
                        if !is_group && ui.button("Copy path").clicked() {
                            ui.ctx().copy_text(row.path.clone());
                        }
                        if let Label::Key(k) = &row.label
                            && ui.button("Copy key").clicked()
                        {
                            ui.ctx().copy_text(k.clone());
                        }
                        if container && !is_group && ui.button("Expand all inside").clicked() {
                            expand = Some(row.pointer.clone());
                        }
                    });
                }
            },
        );
        if let Some(key) = toggle {
            if !self.open.remove(&key) {
                self.open.insert(key);
            }
            self.dirty = true;
        }
        if let Some(ptr) = expand
            && let Some(v) = root.pointer(&ptr)
        {
            let mut sub = Tree::default();
            sub.expand_all(v);
            self.open.extend(sub.open.into_iter().map(|k| format!("{ptr}{k}")));
            self.dirty = true;
        }
    }
}

fn tree_row(ui: &mut Ui, row: &Row, v: &Value, container: bool, open: bool, h: f32, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let painter = ui.painter_at(rect);
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(4), p.control);
    }
    let font = TextStyle::Monospace.resolve(ui.style());
    let y = rect.center().y;
    let mut x = rect.left() + 4.0 + row.depth as f32 * 16.0;
    if container {
        let c = egui::pos2(x + 6.0, y);
        let s = egui::Stroke::new(1.4, p.weak);
        if open {
            painter.line_segment([c + vec2(-3.5, -1.5), c + vec2(0.0, 2.0)], s);
            painter.line_segment([c + vec2(0.0, 2.0), c + vec2(3.5, -1.5)], s);
        } else {
            painter.line_segment([c + vec2(-1.5, -3.5), c + vec2(2.0, 0.0)], s);
            painter.line_segment([c + vec2(2.0, 0.0), c + vec2(-1.5, 3.5)], s);
        }
    }
    x += 16.0;

    let mut job = LayoutJob::default();
    let fmt = |c: Color32| TextFormat::simple(font.clone(), c);
    match &row.label {
        Label::Key(k) => {
            job.append(k, 0.0, fmt(p.purple));
            job.append(": ", 0.0, fmt(p.weak));
        }
        Label::Index(i) => {
            job.append(&i.to_string(), 0.0, fmt(p.weak));
            job.append(": ", 0.0, fmt(p.weak));
        }
        Label::Group(s, e) => job.append(&format!("[{s} … {e}]"), 0.0, fmt(p.weak)),
        Label::None => {}
    }
    if !matches!(row.label, Label::Group(..)) {
        match v {
            Value::Object(m) => {
                let n = m.len();
                job.append(if n == 0 { "{}" } else { "{…}" }, 0.0, fmt(p.weak));
                if n > 0 {
                    job.append(&format!("  {n} {}", if n == 1 { "key" } else { "keys" }), 0.0, fmt(p.weak));
                }
            }
            Value::Array(a) => {
                let n = a.len();
                job.append(if n == 0 { "[]" } else { "[…]" }, 0.0, fmt(p.weak));
                if n > 0 {
                    job.append(&format!("  {n} {}", if n == 1 { "item" } else { "items" }), 0.0, fmt(p.weak));
                }
            }
            Value::String(s) => {
                let shown: String = s.chars().take(300).collect();
                let shown = shown.replace('\n', "⏎");
                job.append(
                    &format!("\"{shown}{}\"", if s.chars().count() > 300 { "…" } else { "" }),
                    0.0,
                    fmt(p.green),
                );
            }
            Value::Number(n) => job.append(&n.to_string(), 0.0, fmt(p.blue)),
            Value::Bool(b) => job.append(&b.to_string(), 0.0, fmt(p.orange)),
            Value::Null => job.append("null", 0.0, fmt(p.orange)),
        }
    }
    job.wrap.max_width = (rect.right() - x - 4.0).max(10.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter.galley(egui::pos2(x, y - galley.size().y / 2.0), galley, p.text);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(ct: &str, body: &[u8]) -> http::Response {
        http::Response {
            url: "http://x".into(),
            status: 200,
            reason: "OK".into(),
            headers: if ct.is_empty() { vec![] } else { vec![("Content-Type".into(), ct.into())] },
            body: body.to_vec(),
            elapsed: Default::default(),
        }
    }

    #[test]
    fn detects_kinds() {
        assert_eq!(Body::new(&resp("application/json; charset=utf-8", b"{\"a\":1}")).kind, Kind::Json);
        assert_eq!(Body::new(&resp("application/problem+json", b"{}")).kind, Kind::Json);
        assert_eq!(Body::new(&resp("text/html", b"<p>hi</p>")).kind, Kind::Html);
        assert_eq!(Body::new(&resp("", b"<!DOCTYPE html><html></html>")).kind, Kind::Html);
        assert_eq!(Body::new(&resp("", b"[1,2]")).kind, Kind::Json);
        assert_eq!(Body::new(&resp("application/xml", b"<a/>")).kind, Kind::Xml);
        assert_eq!(Body::new(&resp("text/plain", b"hello")).kind, Kind::Text);
        assert_eq!(Body::new(&resp("application/octet-stream", &[0, 1, 2, 255])).kind, Kind::Binary);
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
        assert_eq!(Body::new(&resp("image/png", png)).kind, Kind::Image);
    }

    #[test]
    fn json_highlighting() {
        let text = "{\n  \"id\": 42,\n  \"ok\": true\n}";
        let spans = json_spans(text);
        let kinds: Vec<_> = spans.iter().map(|s| (&text[s.0 as usize..s.1 as usize], s.2)).collect();
        assert!(kinds.contains(&("\"id\"", Tok::Key)));
        assert!(kinds.contains(&("42", Tok::Num)));
        assert!(kinds.contains(&("true", Tok::Lit)));
    }

    #[test]
    fn rows_wrap_on_char_boundaries() {
        let mut code = Code::new("ééééé\nab".into(), Lang::Plain, true);
        code.layout_rows(2);
        let rows: Vec<_> = code.rows.iter().map(|r| &code.text[r.clone()]).collect();
        assert_eq!(rows, ["éé", "éé", "é", "ab"]);
    }

    #[test]
    fn tree_paths_and_groups() {
        let v: Value = serde_json::json!({ "user": { "first name": "A", "tags": (0..250).collect::<Vec<_>>() } });
        let mut t = Tree::default();
        t.open_defaults(&v);
        t.open.insert("/user/tags".into());
        t.rebuild(&v);
        let paths: Vec<_> = t.rows.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&".user[\"first name\"]"));
        // 250 items → 3 groups, all closed.
        assert_eq!(t.rows.iter().filter(|r| matches!(r.label, Label::Group(..))).count(), 3);
        t.expand_all(&v);
        t.rebuild(&v);
        assert!(t.rows.iter().any(|r| r.path == ".user.tags[249]"));
    }

    #[test]
    fn hex() {
        let dump = hex_dump(b"Hello");
        assert!(dump.starts_with("00000000  48 65 6c 6c 6f"));
        assert!(dump.trim_end().ends_with("|Hello|"));
    }
}
