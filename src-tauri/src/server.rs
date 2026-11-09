/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! Servers: finding a FreeX Media Server behind what the user typed, opening it in the main
//! window and the commands its pages may call.
//!
//! The main window shows either the bundled server picker (`ui/`) or FreeX Web, loaded directly
//! from the chosen server. Pages of a server only get a small runtime capability for their own
//! origin (see [`grant`]): they can ask for app info, switch the player, go back to the picker,
//! hand a video (and key presses) to mpv and make the window full screen. Nothing else of the app is reachable
//! from there.

use crate::mpv;
use crate::store::{Player, Server, Settings};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::CapabilityBuilder;
use tauri::{AppHandle, Manager, Runtime, State, Url, Webview};

/// Default port of FreeX Media Server, tried first when the user typed none.
pub const DEFAULT_PORT: u16 = 32500;

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub settings_path: PathBuf,
    /// Origins whose pages already have their capability.
    granted: Mutex<HashSet<String>>,
    /// URL of the bundled picker (`tauri://localhost/` or `http://tauri.localhost/`).
    pub local_url: Mutex<Option<Url>>,
    /// Id of the server shown in the window; none while the picker is shown.
    open_server: Mutex<Option<String>>,
    pub mpv: mpv::Mpv,
}

impl AppState {
    pub fn new(settings_path: PathBuf) -> AppState {
        AppState {
            settings: Mutex::new(Settings::load(&settings_path)),
            settings_path,
            granted: Mutex::new(HashSet::new()),
            local_url: Mutex::new(None),
            open_server: Mutex::new(None),
            mpv: mpv::Mpv::default(),
        }
    }

    /// Changes the settings and writes them to disk.
    pub fn update<T>(&self, change: impl FnOnce(&mut Settings) -> T) -> T {
        let mut settings = self.settings.lock().unwrap();
        let result = change(&mut settings);
        settings.save(&self.settings_path);
        result
    }

    /// Id of the server shown in the window, if any.
    pub fn current_server(&self) -> Option<String> {
        self.open_server.lock().unwrap().clone()
    }

    /// True if the URL may be shown in the main window: the picker or a known server.
    pub fn allowed_in_window(&self, url: &Url) -> bool {
        if is_local(url) {
            return true;
        }
        let origin = origin(url);
        self.settings
            .lock()
            .unwrap()
            .server_by_origin(&origin)
            .is_some()
    }
}

/// The app's own pages: `tauri://localhost` on Linux, `http(s)://tauri.localhost` on Windows.
pub fn is_local(url: &Url) -> bool {
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost")
}

/// Scheme, host and port of a URL, without a trailing slash (`http://host:32500`).
pub fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

// ---------------------------------------------------------------------------- probing

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// Origin to open.
    pub url: String,
    pub name: String,
    pub setup_required: bool,
    /// Version of the server; only reported to signed-in users, so usually empty here.
    pub version: String,
}

/// The URLs to try for what the user typed. Without a scheme, the default port is tried first,
/// then HTTPS and plain HTTP on their standard ports.
fn candidates(input: &str) -> Result<Vec<Url>, String> {
    let input = input.trim().trim_end_matches('/');
    if input.is_empty() {
        return Err("empty".into());
    }
    if input.contains("://") {
        let url = Url::parse(input).map_err(|_| "invalid".to_string())?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err("invalid".into());
        }
        if url.host_str().is_none() {
            return Err("invalid".into());
        }
        return Ok(vec![url]);
    }
    let plain = Url::parse(&format!("http://{input}")).map_err(|_| "invalid".to_string())?;
    if plain.host_str().is_none() {
        return Err("invalid".into());
    }
    if plain.port().is_some() {
        return Ok(vec![plain]);
    }
    let mut with_port = plain.clone();
    let _ = with_port.set_port(Some(DEFAULT_PORT));
    let https = Url::parse(&format!("https://{input}")).map_err(|_| "invalid".to_string())?;
    Ok(vec![with_port, https, plain])
}

/// Asks one URL for `/api/status`; `Some` if a FreeX Media Server answered.
fn ask(agent: &ureq::Agent, base: &Url) -> Option<Probe> {
    let status = base.join("/api/status").ok()?;
    let response = agent
        .get(status.as_str())
        .set("Accept", "application/json")
        .call()
        .ok()?;
    // Follow redirects (e.g. HTTP -> HTTPS) to the origin that really answered.
    let final_url = Url::parse(response.get_url()).ok()?;
    let json: serde_json::Value = response.into_json().ok()?;
    let name = json.get("name")?.as_str()?.to_string();
    let setup_required = json.get("setupRequired")?.as_bool()?;
    let version = json
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some(Probe {
        url: origin(&final_url),
        name,
        setup_required,
        version,
    })
}

/// Finds the FreeX Media Server behind what the user typed (`nas`, `192.168.1.10:32500`,
/// `https://media.example.org`).
#[tauri::command]
pub async fn server_probe(address: String) -> Result<Probe, String> {
    let urls = candidates(&address)?;
    tauri::async_runtime::spawn_blocking(move || {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .redirects(5)
            .user_agent(concat!("FreeX-Desktop/", env!("CARGO_PKG_VERSION")))
            .build();
        urls.iter()
            .find_map(|url| ask(&agent, url))
            .ok_or_else(|| "unreachable".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------- opening

/// Lets the pages of one server call the few commands meant for them.
fn grant<R: Runtime>(app: &AppHandle<R>, state: &AppState, origin: &str) -> Result<(), String> {
    let mut granted = state.granted.lock().unwrap();
    if granted.contains(origin) {
        return Ok(());
    }
    let capability = CapabilityBuilder::new(format!("server-{}", crate::store::server_id(origin)))
        .remote(format!("{origin}/*"))
        .local(false)
        .window("main")
        .permission("allow-desktop-info")
        .permission("allow-show-servers")
        .permission("allow-set-player")
        .permission("allow-mpv-play")
        .permission("allow-mpv-stop")
        .permission("allow-mpv-key")
        .permission("core:event:allow-listen")
        .permission("core:event:allow-unlisten")
        .permission("core:window:allow-set-fullscreen")
        .permission("core:window:allow-is-fullscreen");
    app.add_capability(capability).map_err(|e| e.to_string())?;
    granted.insert(origin.to_string());
    Ok(())
}

/// Opens a server (an origin from [`server_probe`]) in the main window and remembers it.
#[tauri::command]
pub fn server_open<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    url: String,
    name: String,
) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|_| "invalid".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("invalid".into());
    }
    let origin = origin(&parsed);
    let server = state.update(|s| s.remember(&origin, &name));
    grant(&app, &state, &origin)?;
    let window = app.get_webview_window("main").ok_or("no window")?;
    window
        .navigate(Url::parse(&format!("{origin}/")).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let _ = window.set_title(&format!("{name} – FreeX"));
    *state.open_server.lock().unwrap() = Some(server.id);
    crate::refresh_tray(&app);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerList {
    servers: Vec<Server>,
    last_server: Option<String>,
    auto_connect: bool,
}

#[tauri::command]
pub fn servers_list(state: State<'_, AppState>) -> ServerList {
    let settings = state.settings.lock().unwrap();
    ServerList {
        servers: settings.servers.clone(),
        last_server: settings.last_server.clone(),
        auto_connect: settings.auto_connect,
    }
}

#[tauri::command]
pub fn server_remove<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>, id: String) {
    state.update(|s| s.forget(&id));
    crate::refresh_tray(&app);
}

#[tauri::command]
pub fn set_auto_connect(state: State<'_, AppState>, enabled: bool) {
    state.update(|s| s.auto_connect = enabled);
}

/// Goes back to the server picker, which then does not connect automatically.
#[tauri::command]
pub fn show_servers<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.mpv.stop();
    open_picker(&app, &state, None)
}

/// Shows the server picker; with `connect` (a server id) it connects to that server at once.
pub fn open_picker<R: Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    connect: Option<&str>,
) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no window")?;
    let url = picker_url(state.local_url.lock().unwrap().as_ref(), connect);
    let _ = window.set_title("FreeX");
    window.navigate(url).map_err(|e| e.to_string())?;
    *state.open_server.lock().unwrap() = None;
    crate::refresh_tray(app);
    Ok(())
}

/// Where Tauri serves the bundled pages when the window's own URL is not known.
const LOCAL_ORIGIN: &str = if cfg!(any(windows, target_os = "android")) {
    "http://tauri.localhost/"
} else {
    "tauri://localhost/"
};

/// URL of the picker in "choose" mode, optionally connecting to a server at once. `seen` is the
/// URL the window started with; it is only used if it really is one of the app's own pages
/// (right after the window was created, WebView2 may still report `about:blank`).
fn picker_url(seen: Option<&Url>, connect: Option<&str>) -> Url {
    let base = seen
        .filter(|u| is_local(u))
        .cloned()
        .unwrap_or_else(|| Url::parse(LOCAL_ORIGIN).expect("valid local origin"));
    let mut url = base.join("/index.html").expect("valid picker path");
    url.query_pairs_mut().append_pair("pick", "1");
    if let Some(id) = connect {
        url.query_pairs_mut().append_pair("connect", id);
    }
    url
}

// ---------------------------------------------------------------------------- app info

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopInfo {
    app: &'static str,
    version: &'static str,
    os: &'static str,
    player: Player,
    mpv_available: bool,
    mpv_fullscreen: bool,
    /// mpv plays inside the app window (setting).
    mpv_embed: bool,
    /// The system allows mpv inside the app window (not on Wayland).
    mpv_can_embed: bool,
    /// The server the asking page belongs to (none for the picker).
    server: Option<Server>,
}

#[tauri::command]
pub fn desktop_info<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, AppState>,
) -> DesktopInfo {
    let mpv_can_embed = mpv::can_embed(&app);
    let settings = state.settings.lock().unwrap();
    let server = webview
        .url()
        .ok()
        .filter(|u| !is_local(u))
        .and_then(|u| settings.server_by_origin(&origin(&u)).cloned());
    DesktopInfo {
        app: "FreeX Desktop",
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        player: settings.player,
        mpv_available: mpv::find().is_some(),
        mpv_fullscreen: settings.mpv_fullscreen,
        mpv_embed: settings.mpv_embed,
        mpv_can_embed,
        server,
    }
}

/// Chooses the player for videos; `mpv_fullscreen` and `mpv_embed` are optional.
#[tauri::command]
pub fn set_player(
    state: State<'_, AppState>,
    player: Player,
    mpv_fullscreen: Option<bool>,
    mpv_embed: Option<bool>,
) {
    state.update(|s| {
        s.player = player;
        if let Some(fs) = mpv_fullscreen {
            s.mpv_fullscreen = fs;
        }
        if let Some(embed) = mpv_embed {
            s.mpv_embed = embed;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{candidates, picker_url, LOCAL_ORIGIN};
    use tauri::Url;

    #[test]
    fn picker_url_ignores_a_window_url_that_is_not_the_app() {
        let blank = Url::parse("about:blank").unwrap();
        let expected = format!("{LOCAL_ORIGIN}index.html?pick=1");
        assert_eq!(picker_url(Some(&blank), None).as_str(), expected);
        assert_eq!(picker_url(None, None).as_str(), expected);
        let seen = Url::parse("http://tauri.localhost/index.html").unwrap();
        assert_eq!(
            picker_url(Some(&seen), Some("00ff")).as_str(),
            "http://tauri.localhost/index.html?pick=1&connect=00ff"
        );
    }

    #[test]
    fn candidates_for_a_host_name_try_the_default_port_first() {
        let c: Vec<String> = candidates("nas")
            .unwrap()
            .iter()
            .map(|u| u.to_string())
            .collect();
        assert_eq!(c, ["http://nas:32500/", "https://nas/", "http://nas/"]);
    }

    #[test]
    fn candidates_keep_an_explicit_port_or_scheme() {
        assert_eq!(
            candidates("10.0.0.2:8080").unwrap()[0].as_str(),
            "http://10.0.0.2:8080/"
        );
        assert_eq!(candidates(" https://media.example.org/ ").unwrap().len(), 1);
        assert!(candidates("ftp://x").is_err());
        assert!(candidates("").is_err());
    }
}
