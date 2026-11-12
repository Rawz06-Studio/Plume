<div align="center">

<img src="assets/icon.svg" width="128" height="128" alt="Plume logo">

# Plume

**A tiny, native REST client. No Electron, no account, no cloud, no AI.**

Open it, pick a request, hit send. That's it.

[![CI](https://github.com/Rawz06-Studio/Plume/actions/workflows/ci.yml/badge.svg)](https://github.com/Rawz06-Studio/Plume/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Rawz06-Studio/Plume?sort=semver)](https://github.com/Rawz06-Studio/Plume/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)

</div>

---

## Why

Postman and friends grew into platforms: workspaces, sync, sign-in walls, AI sidebars, and a few hundred megabytes of Chromium to send a `GET`. Plume does the opposite. It's a single ~10 MB native binary that starts instantly and stores everything as plain JSON files you own.

## Features

- **Requests** — method, URL, headers, body. Response status, timing, size and headers.
- **Response viewer** — picked from the `Content-Type`:
  - **JSON**: a collapsible tree (right-click a value to copy it or its path, e.g. `.data[0].email`), pretty-printed with colors, or raw.
  - **HTML**: rendered by the system webview (macOS, Windows; JavaScript off, links don't navigate), source view, or open in your browser.
  - **Images** shown as images; **XML** and text highlighted; anything else as a hex dump.
- **Saved requests** — a flat, searchable list. One readable JSON file per request, easy to commit with your project.
- **Environments** — `local`, `staging`, `prod`… with simple `{{variable}}` substitution in the URL, headers and body.
- **Unsaved drafts** — edit several requests, switch between them; unsaved changes are kept (marked with •) until you save.
- **Themes** — Auto (follows the system), Light or Dark.
- **Self-update** — Plume checks GitHub releases on startup and updates itself in one click. Can be turned off.

And that's deliberately all.

## Install

Grab the binary for your platform from the [latest release](https://github.com/Rawz06-Studio/Plume/releases/latest):

| Platform | File |
| --- | --- |
| macOS (Apple Silicon & Intel) | `Plume-macos-universal.zip` (app) |
| Linux x86_64 | `plume-linux-x86_64` |
| Linux ARM64 | `plume-linux-aarch64` |
| Windows x86_64 | `Plume-Setup-x86_64.exe` (installer) or `Plume-windows-x86_64.zip` (portable) |

<details>
<summary><b>macOS: “Plume can't be opened”</b></summary>

Builds are not notarized by Apple. After unzipping, remove the quarantine flag once:

```bash
xattr -dr com.apple.quarantine /Applications/Plume.app
```
</details>

<details>
<summary><b>Windows: installer and SmartScreen</b></summary>

The installer needs no admin rights: it installs Plume for the current user in `%LOCALAPPDATA%\Programs\Plume`, adds it to the Start menu and registers an uninstaller. Your requests in `%USERPROFILE%\.plume` are kept when uninstalling.

Builds are not code-signed yet, so SmartScreen may show “Windows protected your PC”: click **More info** → **Run anyway**. If that button is missing, right-click the file → **Properties** → tick **Unblock**.
</details>

<details>
<summary><b>Linux</b></summary>

```bash
chmod +x plume-linux-x86_64 && ./plume-linux-x86_64
```
</details>

### From source

Requires a recent [Rust toolchain](https://rustup.rs).

```bash
git clone https://github.com/Rawz06-Studio/Plume.git
cd Plume
cargo run --release
```

On Linux you may need: `libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.

## Usage

| Shortcut | Action |
| --- | --- |
| <kbd>⌘</kbd> <kbd>↵</kbd> | Send request |
| <kbd>⌘</kbd> <kbd>N</kbd> | New request |
| <kbd>⌘</kbd> <kbd>S</kbd> | Save request |
| <kbd>⌘</kbd> <kbd>D</kbd> | Duplicate request |
| <kbd>⌘</kbd> <kbd>E</kbd> | Environments |
| <kbd>⇧</kbd> <kbd>⌘</kbd> <kbd>F</kbd> | Format JSON body |
| <kbd>⌘</kbd> <kbd>,</kbd> | Settings |

Every action is also in the menus: the native menu bar on macOS (**Plume**, **File**, **Request**, **Window**, **Help**), and a **File · Request · Help** bar at the top of the window on Windows and Linux. Settings, Environments and About open in their own windows.

On Linux and Windows, use <kbd>Ctrl</kbd> instead of <kbd>⌘</kbd>.

A few conveniences:

- A URL without a scheme gets `http://`.
- A JSON body without a `Content-Type` header is sent as `application/json`.
- Responses over 2 MB are truncated on screen; **Copy** always copies the full body.

### Environments

Open **Environments…**, create one, add variables, and select it in the top-right picker. Then reference variables anywhere:

```
GET {{baseUrl}}/users/{{userId}}
Authorization: Bearer {{token}}
```

The resolved URL is shown under the URL field. Unknown variables are left as-is.

## Storage

Everything lives in `~/.plume/` as plain JSON:

```
~/.plume/
├── requests/
│   ├── list-users.json
│   └── create-user.json
├── envs.json
└── settings.json
```

A request file looks like this:

```json
{
  "name": "Create user",
  "method": "POST",
  "url": "{{baseUrl}}/users",
  "headers": [
    { "enabled": true, "key": "Authorization", "value": "Bearer {{token}}" }
  ],
  "body": "{\n  \"name\": \"Ada\"\n}"
}
```

Want requests per project, versioned next to your code? Point Plume at another folder:

```bash
PLUME_DIR=./api plume
```

> **Tip:** `envs.json` often contains secrets. Keep it out of git if you version a Plume folder.

## Releasing

Releases are made by GitHub Actions only — never by hand:

- **Every push to `main` that passes CI** publishes a new patch release (0.1.0 → 0.1.1).
- **Actions → Release → Run workflow** publishes a `minor` (→ 0.2.0) or `major` (→ 1.0.0) release.

Each release bumps the version in `Cargo.toml` and `Cargo.lock`, commits `Release vX.Y.Z` to `main`, tags it, builds macOS (universal `.app`), Linux (x86_64, ARM64) and Windows (installer + portable zip), and publishes the executables with checksums and generated notes. Changes to Markdown files alone don't trigger a release.

Installed copies of Plume pick it up on their next launch.

## Icon

The logo lives in [`assets/icon.svg`](assets/icon.svg). After editing it, regenerate the PNG, `.icns` and `.ico` files:

```bash
scripts/icons.sh
```

## Built with

[Rust](https://www.rust-lang.org) · [egui](https://github.com/emilk/egui) · [ureq](https://github.com/algesten/ureq)

## License

[MIT](LICENSE) © Rawz06 Studio
