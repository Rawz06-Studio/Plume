//! Self-update from GitHub releases.
//!
//! The release workflow uploads one raw binary per platform (see `asset_name`).
//! Checking reads the latest release from the GitHub API; installing downloads the
//! matching binary and swaps it in place of the running executable.

use std::time::Duration;

pub const REPO: &str = "Rawz06-Studio/Plume";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    /// Download URL of the binary for this platform, if the release has one.
    pub asset_url: Option<String>,
}

/// Name of the release asset that matches the platform this binary was built for.
pub fn asset_name() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("plume-macos-universal")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("plume-linux-x86_64")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("plume-linux-aarch64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("plume-windows-x86_64.exe")
    } else {
        None
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("Plume/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Returns the latest release if it is newer than the running version.
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut resp =
        agent().get(&url).header("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Ok(None), // no release published yet
        s => return Err(format!("GitHub answered HTTP {s}")),
    }
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;

    let tag = json["tag_name"].as_str().unwrap_or_default();
    if !is_newer(tag, VERSION) {
        return Ok(None);
    }
    let asset_url = asset_name().and_then(|name| {
        json["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name))?["browser_download_url"]
            .as_str()
            .map(String::from)
    });
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page_url: json["html_url"].as_str().unwrap_or_default().to_string(),
        asset_url,
    }))
}

/// Downloads the new binary and replaces the running executable with it.
/// The new version is used on the next launch.
pub fn install(asset_url: &str) -> Result<(), String> {
    let mut resp = agent().get(asset_url).call().map_err(|e| e.to_string())?;
    if resp.status().as_u16() != 200 {
        return Err(format!("download failed: HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.body_mut().with_config().limit(300 * 1024 * 1024).read_to_vec().map_err(|e| e.to_string())?;
    if bytes.len() < 100 * 1024 {
        return Err("downloaded file is too small to be a Plume binary".into());
    }

    let tmp = std::env::temp_dir().join(format!("plume-update-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    let result = self_replace::self_replace(&tmp).map_err(|e| format!("could not replace binary: {e}"));
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Compares `x.y.z` versions (a leading `v` and any `-suffix` are ignored).
fn is_newer(candidate: &str, current: &str) -> bool {
    fn parse(v: &str) -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    }
    let (mut a, mut b) = (parse(candidate), parse(current));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a > b
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn versions() {
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("", "0.1.0"));
    }

    /// Hits the real GitHub API: `cargo test -- --ignored live_check`
    #[test]
    #[ignore]
    fn live_check() {
        println!("{:?}", super::check());
    }
}
