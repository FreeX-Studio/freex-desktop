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
//! the desktop app. FreeX Web hands over the stream URL with [`mpv_play`]; the app starts mpv in
//! its own window and talks to it over mpv's JSON IPC (a named pipe on Windows, a Unix socket
//! elsewhere). The sign-in token is sent over that pipe, never on the command line. Position
//! changes and the end of playback come back to the page as `freex-mpv` events, and FreeX Web
//! saves the position on the server just as with its own player.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime, State, Url, Webview};
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
    let fullscreen = state.settings.lock().unwrap().mpv_fullscreen;

    let mut cmd = tokio::process::Command::new(&mpv);
    cmd.arg(format!("--input-ipc-server={ipc}"))
        .arg("--idle=once") // quit when the file is done
        .arg("--force-window=immediate")
        .arg("--keep-open=no")
        .arg("--no-terminal")
        .arg("--ytdl=no")
        .arg("--hwdec=auto-safe")
        .arg("--save-position-on-quit=no") // FreeX keeps the position
        .arg(format!("--force-media-title={}", request.title))
        .arg(format!("--start={:.3}", request.start_ms as f64 / 1000.0))
        .arg(match request.audio {
            Some(a) => format!("--aid={a}"),
            None => "--aid=auto".into(),
        })
        .arg(match request.subtitle {
            Some(s) => format!("--sid={s}"),
            None => "--sid=no".into(),
        });
    for file in &subtitle_files {
        cmd.arg(format!("--sub-files-append={file}"));
    }
    if fullscreen {
        cmd.arg("--fs");
    }
    cmd.kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd.spawn().map_err(|e| format!("mpv could not be started: {e}"))?;

    let (tx, rx) = mpsc::unbounded_channel();
    // Order matters: the header must be set before the file is loaded.
    let header = request
        .token
        .as_deref()
        .filter(|t| !t.is_empty() && !t.contains(['\r', '\n', ',']))
        .map(|t| vec![format!("X-FreeX-Token: {t}")])
        .unwrap_or_default();
    let _ = tx.send(json!({"command": ["set_property", "http-header-fields", header]}));
    for (n, name) in ["time-pos", "duration", "pause"].iter().enumerate() {
        let _ = tx.send(json!({"command": ["observe_property", n + 1, name]}));
    }
    let _ = tx.send(json!({"command": ["loadfile", url.as_str()]}));
    *state.mpv.current.lock().unwrap() = Some(Running { id, commands: tx });

    let item_id = request.item_id.clone();
    tauri::async_runtime::spawn(async move {
        let result = run(&app, child, &ipc, rx, &item_id).await;
        if let Some(state) = tauri::Manager::try_state::<AppState>(&app) {
            state.mpv.finished(id);
        }
        let _ = app.emit_to("main", EVENT, result);
    });
    Ok(())
}

#[tauri::command]
pub fn mpv_stop(state: State<'_, AppState>) {
    state.mpv.stop();
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
    let _ = app.emit_to("main", EVENT, MpvEvent { kind: "started", ..event.clone() });

    let mut last_emit = Instant::now() - Duration::from_secs(10);
    let mut ended_reason: Option<String> = None;
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
                            _ => {}
                        }
                        if last_emit.elapsed() >= Duration::from_secs(1) {
                            last_emit = Instant::now();
                            let _ = app.emit_to("main", EVENT, MpvEvent { kind: "progress", ..event.clone() });
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
    if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
    event.eof = ended_reason.as_deref() == Some("eof");
    if event.eof && event.duration_ms > 0 {
        event.position_ms = event.duration_ms;
    }
    event
}

#[cfg(test)]
mod tests {
    use super::same_server;
    use tauri::Url;

    #[test]
    fn only_streams_of_the_own_server_are_accepted() {
        let page = Url::parse("http://nas:32500/play/abc").unwrap();
        assert!(same_server(&page, "http://nas:32500/api/items/abc/stream").is_ok());
        assert!(same_server(&page, "http://evil:32500/x").is_err());
        assert!(same_server(&page, "file:///C:/Windows/win.ini").is_err());
        assert!(same_server(&page, "https://nas:32500/x").is_err());
    }
}
