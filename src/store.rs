//! Data model and storage: one JSON file per request, one file for environments,
//! one file for settings.
//!
//! ~/.plume/requests/<id>.json
//! ~/.plume/envs.json
//! ~/.plume/settings.json
//!
//! The storage folder can be overridden with the PLUME_DIR environment variable.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Header {
    #[serde(default = "yes")]
    pub enabled: bool,
    pub key: String,
    pub value: String,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// File name (without .json). Empty until the request is saved.
    #[serde(skip)]
    pub id: String,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<Header>,
    #[serde(default)]
    pub body: String,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            method: "GET".into(),
            url: String::new(),
            headers: Vec::new(),
            body: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Var {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Env {
    pub name: String,
    #[serde(default)]
    pub vars: Vec<Var>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Envs {
    /// Name of the active environment (None = no environment).
    #[serde(default)]
    pub active: Option<String>,
    #[serde(default)]
    pub envs: Vec<Env>,
}

impl Envs {
    pub fn active_env(&self) -> Option<&Env> {
        let name = self.active.as_ref()?;
        self.envs.iter().find(|e| &e.name == name)
    }

    /// Replaces `{{var}}` with its value from the active environment.
    /// Unknown variables are left untouched.
    pub fn resolve(&self, input: &str) -> String {
        let Some(env) = self.active_env() else {
            return input.to_string();
        };
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            match after.find("}}") {
                Some(end) => {
                    let key = after[..end].trim();
                    match env.vars.iter().find(|v| v.key == key) {
                        Some(v) => out.push_str(&v.value),
                        None => out.push_str(&rest[start..start + 2 + end + 2]),
                    }
                    rest = &after[end + 2..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "yes")]
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: Theme::Auto, check_updates: true }
    }
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open() -> Self {
        let root = std::env::var_os("PLUME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".plume"));
        let _ = fs::create_dir_all(root.join("requests"));
        Self { root }
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn requests_dir(&self) -> PathBuf {
        self.root.join("requests")
    }

    pub fn load_requests(&self) -> Vec<Request> {
        let mut list = Vec::new();
        let Ok(entries) = fs::read_dir(self.requests_dir()) else {
            return list;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            match serde_json::from_str::<Request>(&text) {
                Ok(mut r) => {
                    r.id = path.file_stem().unwrap().to_string_lossy().into_owned();
                    list.push(r);
                }
                Err(e) => eprintln!("plume: skipping {} ({e})", path.display()),
            }
        }
        sort(&mut list);
        list
    }

    /// Saves the request, giving it an id derived from its name if it has none.
    pub fn save_request(&self, req: &mut Request, existing: &[Request]) -> std::io::Result<()> {
        if req.id.is_empty() {
            let base = slug(&req.name);
            let mut id = base.clone();
            let mut n = 2;
            while existing.iter().any(|r| r.id == id) || self.request_path(&id).exists() {
                id = format!("{base}-{n}");
                n += 1;
            }
            req.id = id;
        }
        write_json(&self.request_path(&req.id), req)
    }

    pub fn delete_request(&self, id: &str) -> std::io::Result<()> {
        fs::remove_file(self.request_path(id))
    }

    fn request_path(&self, id: &str) -> PathBuf {
        self.requests_dir().join(format!("{id}.json"))
    }

    pub fn load_envs(&self) -> Envs {
        read_json(&self.root.join("envs.json"))
    }

    pub fn save_envs(&self, envs: &Envs) -> std::io::Result<()> {
        write_json(&self.root.join("envs.json"), envs)
    }

    pub fn load_settings(&self) -> Settings {
        read_json(&self.root.join("settings.json"))
    }

    pub fn save_settings(&self, settings: &Settings) -> std::io::Result<()> {
        write_json(&self.root.join("settings.json"), settings)
    }
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &PathBuf) -> T {
    fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_json<T: Serialize>(path: &PathBuf, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(value).expect("serialization");
    fs::write(path, json + "\n")
}

pub fn sort(list: &mut [Request]) {
    list.sort_by_key(|r| r.name.to_lowercase());
}

fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "request".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_vars() {
        let envs = Envs {
            active: Some("local".into()),
            envs: vec![Env { name: "local".into(), vars: vec![Var { key: "base".into(), value: "http://x".into() }] }],
        };
        assert_eq!(envs.resolve("{{base}}/a/{{ base }}"), "http://x/a/http://x");
        assert_eq!(envs.resolve("{{nope}}/{{base"), "{{nope}}/{{base");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Get users / list"), "get-users-list");
        assert_eq!(slug("  "), "request");
    }

    #[test]
    fn settings_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Settings::default());
        let s: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(s.theme, Theme::Dark);
    }
}
