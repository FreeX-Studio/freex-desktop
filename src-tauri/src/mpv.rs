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

//! Playback in mpv.
//!
//! mpv plays practically every file directly, so the server never has to convert anything for
//! the desktop app. FreeX Web hands over the stream URL with [`mpv_play`]; the app starts mpv and
//! talks to it over mpv's JSON IPC (a named pipe on Windows, a Unix socket elsewhere). The
//! sign-in token is sent over that pipe, never on the command line. Position changes and the end
//! of playback come back to the page as `freex-mpv` events, and FreeX Web saves the position on
//! the server just as with its own player.
//!
//! Where the platform allows it (Windows, Linux on X11), mpv draws into the app's own window
//! (`--wid`), so the video plays inside FreeX like in a native player. mpv's full screen is then
//! the full screen of the app window, and Esc first leaves full screen and then stops. The
//! keyboard stays with the web view (GTK keeps the X11 focus on its own window), so the page
//! hands key presses to mpv while it plays there ([`mpv_key`], see `native.js`). On
//! Wayland, which cannot embed a foreign window, and when turned off in the settings, mpv opens
//! its own window as before.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime, State, Url, Webview};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::server::{origin, AppState};

/// Name of the event FreeX Web listens to.
pub const EVENT: &str = "freex-mpv";

/// The running mpv, if any.
#[derive(Default)]
pub struct Mpv {
    current: Mutex<Option<Running>>,
}

struct Running {
    id: u64,
    commands: mpsc::UnboundedSender<Value>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Mpv {
    /// Asks a running mpv to quit; it then reports the final position as usual.
    pub fn stop(&self) {
        if let Some(running) = self.current.lock().unwrap().take() {
            let _ = running.commands.send(json!({"command": ["quit"]}));
        }
    }

    /// Sends a command to the running mpv, if any.
    fn send(&self, command: Value) {
        if let Some(running) = self.current.lock().unwrap().as_ref() {
            let _ = running.commands.send(command);
        }
    }

    fn finished(&self, id: u64) {
        let mut current = self.current.lock().unwrap();
        if current.as_ref().map(|r| r.id) == Some(id) {
            *current = None;
        }
    }
}

/// mpv next to the app (`mpv\mpv.exe`, installed with it), otherwise from the PATH.
pub fn find() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "mpv.exe" } else { "mpv" };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("mpv").join(exe)));
    if let Some(path) = bundled.filter(|p| p.is_file()) {
        return Some(path);
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

/// Native handle of the app window that mpv can draw into (`--wid`): a Win32 window or an X11
/// window. `None` on Wayland (no embedding of foreign windows) and other platforms.
fn embed_target<R: Runtime>(app: &AppHandle<R>) -> Option<i64> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let window = app.get_webview_window("main")?;
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as i64),
        RawWindowHandle::Xlib(h) => i64::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(i64::from(h.window.get())),
        _ => None,
    }
}

/// True if mpv can draw into the app window on this system.
pub fn can_embed<R: Runtime>(app: &AppHandle<R>) -> bool {
    embed_target(app).is_some()
}

/// What FreeX Web asks to play.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    /// Item id, only passed back in the events.
    pub item_id: String,
    /// Stream URL of the file on the page's own server.
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub start_ms: u64,
    /// Sign-in token, sent as `X-FreeX-Token` with every request of mpv.
    #[serde(default)]
    pub token: Option<String>,
    /// mpv audio track (1-based), or the file's default.
    #[serde(default)]
    pub audio: Option<u32>,
    /// mpv subtitle track (1-based); none switches subtitles off.
    #[serde(default)]
    pub subtitle: Option<u32>,
    /// External subtitle files (URLs on the same server), appended after the embedded tracks.
    #[serde(default)]
    pub subtitle_files: Vec<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MpvEvent {
    /// `started`, `progress` or `ended`.
    kind: &'static str,
    item_id: String,
    position_ms: u64,
    duration_ms: u64,
    paused: bool,
    /// Ended because the file played to its end.
    eof: bool,
    /// Ended with an error (file could not be opened, mpv failed to start).
    error: Option<String>,
}

/// Only URLs of the server the page itself comes from, so a page cannot make mpv open anything
/// else (local files, other hosts, other protocols).
fn same_server(page: &Url, url: &str) -> Result<Url, String> {
    let url = Url::parse(url).map_err(|_| "invalid url".to_string())?;
    if (url.scheme() != "http" && url.scheme() != "https") || origin(&url) != origin(page) {
        return Err("only streams of this server can be played".into());
    }
    Ok(url)
}

#[tauri::command]
pub async fn mpv_play<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, AppState>,
    request: PlayRequest,
) -> Result<(), String> {
    let page = webview.url().map_err(|e| e.to_string())?;
    let url = same_server(&page, &request.url)?;
    let subtitle_files = request
        .subtitle_files
        .iter()
        .map(|s| same_server(&page, s).map(|u| u.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mpv = find().ok_or("mpv not found")?;
    state.mpv.stop();

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let ipc = ipc_path(id);
    let (fullscreen, embed) = {
        let settings = state.settings.lock().unwrap();
        (settings.mpv_fullscreen, settings.mpv_embed)
    };
    let wid = if embed { embed_target(&app) } else { None };
    let window = app.get_webview_window("main");
    // Embedded, mpv's full screen is the app window's: remember how it was to restore it, and
    // keep a window that is already full screen that way.
    let embedded = match (&window, wid) {
        (Some(w), Some(_)) => Some(Embedded {
            window_was_fullscreen: w.is_fullscreen().unwrap_or(false),
        }),
        _ => None,
    };
    let fullscreen = fullscreen || embedded.as_ref().is_some_and(|e| e.window_was_fullscreen);
    if let (Some(w), Some(_), true) = (&window, &embedded, fullscreen) {
        let _ = w.set_fullscreen(true);
    }

    let mut cmd = tokio::process::Command::new(&mpv);
    cmd.args(mpv_args(&ipc, &request, &subtitle_files, wid, fullscreen));
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            if let (Some(w), Some(e)) = (&window, &embedded) {
                let _ = w.set_fullscreen(e.window_was_fullscreen);
            }
            return Err(format!("mpv could not be started: {e}"));
        }
    };

    let (tx, rx) = mpsc::unbounded_channel();
    // Order matters: the header must be set before the file is loaded.
    let header = request
        .token
        .as_deref()
        .filter(|t| !t.is_empty() && !t.contains(['\r', '\n', ',']))
        .map(|t| vec![format!("X-FreeX-Token: {t}")])
        .unwrap_or_default();
    let _ = tx.send(json!({"command": ["set_property", "http-header-fields", header]}));
    for (n, name) in OBSERVED.iter().enumerate() {
        let _ = tx.send(json!({"command": ["observe_property", n + 1, name]}));
    }
    if embedded.is_some() {
        // Esc leaves full screen first and then stops (decided in `run`); older mpv versions
        // without `keybind` just keep their default.
        let _ = tx.send(json!({"command": ["keybind", "ESC", "script-message freex-escape"]}));
    }
    let _ = tx.send(json!({"command": ["loadfile", url.as_str()]}));
    *state.mpv.current.lock().unwrap() = Some(Running { id, commands: tx });

    let item_id = request.item_id.clone();
    if embedded.is_some() {
        mark_embedded(&app, true);
    }
    tauri::async_runtime::spawn(async move {
        let result = run(&app, child, &ipc, rx, &item_id, embedded.is_some()).await;
        #[cfg(not(windows))]
        let _ = std::fs::remove_file(&ipc);
        if let (Some(w), Some(e)) = (app.get_webview_window("main"), embedded) {
            mark_embedded(&app, false);
            let _ = w.set_fullscreen(e.window_was_fullscreen);
            let _ = w.set_focus();
        }
        if let Some(state) = tauri::Manager::try_state::<AppState>(&app) {
            state.mpv.finished(id);
        }
        let _ = app.emit_to("main", EVENT, result);
    });
    Ok(())
}

/// Properties of mpv the app follows, in the order of their observer ids.
const OBSERVED: [&str; 4] = ["time-pos", "duration", "pause", "fullscreen"];

/// State of an mpv that draws into the app window.
struct Embedded {
    window_was_fullscreen: bool,
}

/// The command line of mpv. Everything about the user (the sign-in token) goes over the IPC.
fn mpv_args(
    ipc: &str,
    request: &PlayRequest,
    subtitle_files: &[String],
    wid: Option<i64>,
    fullscreen: bool,
) -> Vec<String> {
    let mut args = vec![
        format!("--input-ipc-server={ipc}"),
        "--idle=once".into(), // quit when the file is done
        "--force-window=immediate".into(),
        "--keep-open=no".into(),
        "--no-terminal".into(),
        "--ytdl=no".into(),
        "--hwdec=auto-safe".into(),
        "--save-position-on-quit=no".into(), // FreeX keeps the position
        format!("--force-media-title={}", request.title),
        "--title=${media-title} – FreeX".into(),
        format!("--start={:.3}", request.start_ms as f64 / 1000.0),
        match request.audio {
            Some(a) => format!("--aid={a}"),
            None => "--aid=auto".into(),
        },
        match request.subtitle {
            Some(s) => format!("--sid={s}"),
            None => "--sid=no".into(),
        },
    ];
    for file in subtitle_files {
        args.push(format!("--sub-files-append={file}"));
    }
    if let Some(wid) = wid {
        args.push(format!("--wid={wid}"));
    } else if cfg!(target_os = "linux") {
        // Groups mpv's own window with FreeX in the task bar.
        args.push("--x11-name=freex-desktop".into());
    }
    if fullscreen {
        args.push("--fs".into());
    }
    if cfg!(windows) {
        // Play/pause from the keyboard's media keys and the Windows media overlay. On Linux the
        // package recommends mpv-mpris for the same.
        args.push("--media-controls=yes".into());
    }
    args
}

#[tauri::command]
pub fn mpv_stop(state: State<'_, AppState>) {
    state.mpv.stop();
}

/// A key pressed in the app window while mpv plays inside it, in mpv's key names (`SPACE`,
/// `Ctrl+LEFT`, `f`). mpv handles it like a key pressed in its own window.
#[tauri::command]
pub fn mpv_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    if key.is_empty() || key.len() > 40 || key.chars().any(char::is_whitespace) {
        return Err("invalid key".into());
    }
    state.mpv.send(json!({"command": ["keypress", key]}));
    Ok(())
}

/// Tells the page in the main window whether mpv plays inside it (see `native.js`).
fn mark_embedded<R: Runtime>(app: &AppHandle<R>, on: bool) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval(format!("window.__FREEX_MPV_EMBEDDED__ = {on};"));
    }
}

#[cfg(windows)]
fn ipc_path(id: u64) -> String {
    format!(r"\\.\pipe\freex-mpv-{}-{id}", std::process::id())
}

#[cfg(not(windows))]
fn ipc_path(id: u64) -> String {
    std::env::temp_dir()
        .join(format!("freex-mpv-{}-{id}.sock", std::process::id()))
        .to_string_lossy()
        .into_owned()
}

/// Connects to mpv's IPC; mpv creates it shortly after starting.
#[cfg(windows)]
async fn connect(path: &str) -> std::io::Result<impl AsyncRead + AsyncWrite> {
    use tokio::net::windows::named_pipe::ClientOptions;
    ClientOptions::new().open(path)
}

#[cfg(not(windows))]
async fn connect(path: &str) -> std::io::Result<impl AsyncRead + AsyncWrite> {
    tokio::net::UnixStream::connect(path).await
}

/// Drives one mpv until it exits and returns the final event.
async fn run<R: Runtime>(
    app: &AppHandle<R>,
    mut child: tokio::process::Child,
    ipc: &str,
    mut commands: mpsc::UnboundedReceiver<Value>,
    item_id: &str,
    embedded: bool,
) -> MpvEvent {
    let mut event = MpvEvent {
        kind: "ended",
        item_id: item_id.to_string(),
        position_ms: 0,
        duration_ms: 0,
        paused: false,
        eof: false,
        error: None,
    };

    // Wait for the pipe (or for mpv to give up early, e.g. on a broken option).
    let deadline = Instant::now() + Duration::from_secs(15);
    let stream = loop {
        match connect(ipc).await {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => {
                if let Ok(Some(status)) = child.try_wait() {
                    event.error = Some(format!("mpv exited ({status})"));
                    return event;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                let _ = child.kill().await;
                event.error = Some(format!("no connection to mpv: {e}"));
                return event;
            }
        }
    };
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let _ = app.emit_to(
        "main",
        EVENT,
        MpvEvent {
            kind: "started",
            ..event.clone()
        },
    );

    let mut last_emit = Instant::now() - Duration::from_secs(10);
    let mut ended_reason: Option<String> = None;
    let mut fullscreen = false;
    if embedded {
        focus_embedded(app);
    }
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(command) => {
                    let mut line = command.to_string();
                    line.push('\n');
                    if writer.write_all(line.as_bytes()).await.is_err() {
                        break;
                    }
                }
                // The sender is gone only when the app shuts down.
                None => {
                    let _ = writer.write_all(b"{\"command\":[\"quit\"]}\n").await;
                    break;
                }
            },
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                match msg.get("event").and_then(Value::as_str) {
                    Some("property-change") => {
                        let data = msg.get("data");
                        match msg.get("name").and_then(Value::as_str) {
                            Some("time-pos") => {
                                if let Some(s) = data.and_then(Value::as_f64) {
                                    event.position_ms = (s.max(0.0) * 1000.0) as u64;
                                }
                            }
                            Some("duration") => {
                                if let Some(s) = data.and_then(Value::as_f64) {
                                    event.duration_ms = (s.max(0.0) * 1000.0) as u64;
                                }
                            }
                            Some("pause") => {
                                event.paused = data.and_then(Value::as_bool).unwrap_or(false);
                                last_emit = Instant::now() - Duration::from_secs(10); // report at once
                            }
                            Some("fullscreen") => {
                                fullscreen = data.and_then(Value::as_bool).unwrap_or(false);
                                if embedded {
                                    if let Some(w) = app.get_webview_window("main") {
                                        let _ = w.set_fullscreen(fullscreen);
                                    }
                                }
                            }
                            _ => {}
                        }
                        if last_emit.elapsed() >= Duration::from_secs(1) {
                            last_emit = Instant::now();
                            let _ = app.emit_to("main", EVENT, MpvEvent { kind: "progress", ..event.clone() });
                        }
                    }
                    Some("client-message") => {
                        let args = msg.get("args").and_then(Value::as_array);
                        if args.and_then(|a| a.first()).and_then(Value::as_str) == Some("freex-escape") {
                            let command = if fullscreen {
                                json!({"command": ["set_property", "fullscreen", false]})
                            } else {
                                json!({"command": ["quit"]})
                            };
                            let mut line = command.to_string();
                            line.push('\n');
                            if writer.write_all(line.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some("end-file") => {
                        let reason = msg.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
                        if reason == "error" {
                            let detail = msg.get("file_error").and_then(Value::as_str).unwrap_or("unknown error");
                            event.error = Some(format!("mpv could not play the file: {detail}"));
                        }
                        ended_reason = Some(reason);
                    }
                    _ => {}
                }
            }
        }
    }

    // mpv quits by itself after the file (--idle=once); give it a moment, then make sure.
    if tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
    }
    event.eof = ended_reason.as_deref() == Some("eof");
    if event.eof && event.duration_ms > 0 {
        event.position_ms = event.duration_ms;
    }
    event
}

/// Gives the keyboard to mpv's window inside the app window, so its keys work at once.
#[cfg(windows)]
fn focus_embedded<R: Runtime>(app: &AppHandle<R>) {
    use windows::core::w;
    use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowExW;
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(parent) = window.hwnd() else { return };
    let parent = parent.0 as isize;
    let _ = window.set_focus();
    let _ = window.run_on_main_thread(move || {
        let parent = windows::Win32::Foundation::HWND(parent as *mut _);
        // mpv registers its window class as "mpv".
        if let Ok(child) = unsafe { FindWindowExW(Some(parent), None, w!("mpv"), None) } {
            let _ = unsafe { SetFocus(Some(child)) };
        }
    });
}

/// X11 hands the keyboard to mpv's window while the mouse is over it; the app window only
/// needs the focus.
#[cfg(not(windows))]
fn focus_embedded<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::{mpv_args, same_server, PlayRequest};
    use tauri::Url;

    fn request() -> PlayRequest {
        PlayRequest {
            item_id: "abc".into(),
            url: "http://nas:32500/api/items/abc/stream".into(),
            title: "Film".into(),
            start_ms: 61_500,
            token: Some("secret".into()),
            audio: Some(2),
            subtitle: None,
            subtitle_files: vec![],
        }
    }

    #[test]
    fn the_command_line_embeds_when_asked_and_never_holds_the_token() {
        let args = mpv_args("ipc", &request(), &[], Some(4242), true);
        assert!(args.contains(&"--wid=4242".to_string()));
        assert!(args.contains(&"--fs".to_string()));
        assert!(args.contains(&"--start=61.500".to_string()));
        assert!(args.contains(&"--aid=2".to_string()));
        assert!(args.contains(&"--sid=no".to_string()));
        assert!(!args.iter().any(|a| a.contains("secret")));
        let own_window = mpv_args("ipc", &request(), &[], None, false);
        assert!(!own_window
            .iter()
            .any(|a| a.starts_with("--wid") || a == "--fs"));
    }

    #[test]
    fn only_streams_of_the_own_server_are_accepted() {
        let page = Url::parse("http://nas:32500/play/abc").unwrap();
        assert!(same_server(&page, "http://nas:32500/api/items/abc/stream").is_ok());
        assert!(same_server(&page, "http://evil:32500/x").is_err());
        assert!(same_server(&page, "file:///C:/Windows/win.ini").is_err());
        assert!(same_server(&page, "https://nas:32500/x").is_err());
    }
}
