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

//! The app's own settings: the known servers, the player choice and the window geometry.
//!
//! Everything lives in one small JSON file (`settings.json`) in the app's config folder,
//! on Windows `%APPDATA%\de.mirranet.freex.desktop`. Sign-ins are not stored here: they are
//! kept by the web view, per server, exactly as in a browser.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A FreeX Media Server the user has connected to.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    /// Stable id, derived from the origin.
    pub id: String,
    /// Server name as reported by `/api/status`.
    pub name: String,
    /// Origin of the server, e.g. `http://192.168.1.10:32500` (no path, no trailing slash).
    pub url: String,
    /// Last time the server was opened, in ms since the epoch.
    #[serde(default)]
    pub last_used: u64,
}

/// Size and position of the main window, restored on the next start.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub maximized: bool,
}

/// Which player plays videos: the one of FreeX Web or mpv.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Player {
    #[default]
    Builtin,
    Mpv,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub servers: Vec<Server>,
    /// Id of the server opened last; opened directly on the next start when `auto_connect` is set.
    #[serde(default)]
    pub last_server: Option<String>,
    #[serde(default = "yes")]
    pub auto_connect: bool,
    #[serde(default)]
    pub player: Player,
    /// mpv starts in full screen.
    #[serde(default = "yes")]
    pub mpv_fullscreen: bool,
    #[serde(default)]
    pub window: Option<WindowGeometry>,
}

fn yes() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            servers: Vec::new(),
            last_server: None,
            auto_connect: true,
            player: Player::Builtin,
            mpv_fullscreen: true,
            window: None,
        }
    }
}

impl Settings {
    /// Reads the settings; a missing or broken file gives the defaults.
    pub fn load(path: &PathBuf) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes the settings atomically (temporary file, then rename).
    pub fn save(&self, path: &PathBuf) {
        let Ok(json) = serde_json::to_vec_pretty(self) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn server_by_origin(&self, origin: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.url == origin)
    }

    /// Adds the server or updates its name; marks it as used now and returns it.
    pub fn remember(&mut self, url: &str, name: &str) -> Server {
        let now = now_ms();
        let id = server_id(url);
        match self.servers.iter_mut().find(|s| s.id == id) {
            Some(s) => {
                s.name = name.to_string();
                s.last_used = now;
            }
            None => self.servers.push(Server {
                id: id.clone(),
                name: name.to_string(),
                url: url.to_string(),
                last_used: now,
            }),
        }
        self.last_server = Some(id.clone());
        self.servers.sort_by(|a, b| b.last_used.cmp(&a.last_used));
        self.server(&id).cloned().expect("just added")
    }

    pub fn forget(&mut self, id: &str) {
        self.servers.retain(|s| s.id != id);
        if self.last_server.as_deref() == Some(id) {
            self.last_server = None;
        }
    }
}

/// A short, stable id for an origin (FNV-1a, hex), so ids never contain URL characters.
pub fn server_id(origin: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in origin.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
