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

//! FreeX Desktop: the desktop app for FreeX Media Server, for Windows and Linux.
//!
//! One window. It starts with the bundled server picker and then shows FreeX Web straight from
//! the chosen server, so the app always has the interface of the server it talks to. On top of
//! that it behaves like a native app: remembered servers (also in the tray menu), playback of
//! any file in mpv inside the app window without conversion, media keys, real full screen (F11),
//! a dark title bar, no browser menus or shortcuts, and a single running instance.

// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mpv;
mod server;
mod store;

use server::AppState;
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, RunEvent, Runtime, Theme, WebviewUrl,
    WebviewWindowBuilder, WindowEvent,
};

/// Id of the tray icon.
const TRAY: &str = "freex";

/// Makes the web view behave like an app window (see the file); runs in every page.
const NATIVE_JS: &str = include_str!("native.js");

fn main() {
    #[cfg(target_os = "linux")]
    linux_environment();

    let app = tauri::Builder::default()
        // A second start just brings the open window to the front.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .invoke_handler(tauri::generate_handler![
            server::servers_list,
            server::server_probe,
            server::server_open,
            server::server_remove,
            server::set_auto_connect,
            server::desktop_info,
            server::set_player,
            server::show_servers,
            mpv::mpv_play,
            mpv::mpv_stop,
            mpv::mpv_key,
        ])
        .setup(|app| {
            let config = app.path().app_config_dir()?;
            app.manage(AppState::new(config.join("settings.json")));
            create_main_window(app.handle())?;
            // Without a tray (e.g. a Linux desktop without status icons) the app works as well.
            if let Err(e) = create_tray(app.handle()) {
                eprintln!("FreeX Desktop: no tray icon: {e}");
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("FreeX Desktop could not start");

    app.run(|app, event| match event {
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { .. },
            ..
        } if label == "main" => {
            save_geometry(app);
        }
        RunEvent::Exit => {
            if let Some(state) = app.try_state::<AppState>() {
                state.mpv.stop();
            }
        }
        _ => {}
    });
}

fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let nav_app = app.clone();
    let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("FreeX")
        .inner_size(1280.0, 800.0)
        .min_inner_size(720.0, 480.0)
        .background_color(tauri::window::Color(0x12, 0x18, 0x21, 0xff))
        // FreeX is dark, so is the title bar.
        .theme(Some(Theme::Dark))
        .visible(false)
        .initialization_script(NATIVE_JS)
        .zoom_hotkeys_enabled(true);
    // Windows 11 style scroll bars that only show while scrolling.
    #[cfg(windows)]
    let builder = builder.scroll_bar_style(tauri::webview::ScrollBarStyle::FluentOverlay);
    let window = builder
        // Only the picker and known servers are shown in the window; every other link opens
        // in the default browser (trailers, TMDB pages, help links).
        .on_navigation(move |url| {
            let allowed = nav_app
                .try_state::<AppState>()
                .map(|s| s.allowed_in_window(url))
                .unwrap_or(false);
            if !allowed && (url.scheme() == "http" || url.scheme() == "https") {
                let _ = open::that_detached(url.as_str());
            }
            allowed
        })
        .on_new_window(|url, _features| {
            if url.scheme() == "http" || url.scheme() == "https" {
                let _ = open::that_detached(url.as_str());
            }
            NewWindowResponse::Deny
        })
        .build()?;

    let state = app.state::<AppState>();
    *state.local_url.lock().unwrap() = window.url().ok();
    let geometry = state.settings.lock().unwrap().window;
    if let Some(g) = geometry.filter(|g| g.width >= 400 && g.height >= 300) {
        let _ = window.set_size(PhysicalSize::new(g.width, g.height));
        // Only restore the position if it is still on one of the screens.
        let on_screen = window
            .available_monitors()
            .unwrap_or_default()
            .iter()
            .any(|m| {
                let (p, s) = (m.position(), m.size());
                g.x + 40 >= p.x
                    && g.y >= p.y
                    && g.x + 40 < p.x + s.width as i32
                    && g.y + 40 < p.y + s.height as i32
            });
        if on_screen {
            let _ = window.set_position(PhysicalPosition::new(g.x, g.y));
        }
        if g.maximized {
            let _ = window.maximize();
        }
    } else {
        let _ = window.center();
    }
    window.show()?;
    Ok(())
}

fn save_geometry(app: &AppHandle) {
    let (Some(window), Some(state)) = (app.get_webview_window("main"), app.try_state::<AppState>())
    else {
        return;
    };
    let maximized = window.is_maximized().unwrap_or(false);
    if window.is_fullscreen().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(size), Ok(pos)) = (window.outer_size(), window.outer_position()) else {
        return;
    };
    state.update(|s| {
        let old = s.window.unwrap_or_default();
        // While maximized only the flag changes, so un-maximizing restores the old size.
        s.window = Some(if maximized {
            store::WindowGeometry {
                maximized: true,
                ..old
            }
        } else {
            let inner = window.inner_size().unwrap_or(size);
            store::WindowGeometry {
                width: inner.width,
                height: inner.height,
                x: pos.x,
                y: pos.y,
                maximized: false,
            }
        });
    });
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn german() -> bool {
    sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("de"))
}

/// The tray menu: the known servers (the open one checked) to switch with one click, the server
/// picker and quitting. Built again whenever the list of servers changes.
fn tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let tr = |de: &'static str, en: &'static str| if german() { de } else { en };
    let (servers, current) = match app.try_state::<AppState>() {
        Some(state) => (
            state.settings.lock().unwrap().servers.clone(),
            state.current_server(),
        ),
        None => (Vec::new(), None),
    };
    let mut items: Vec<Box<dyn IsMenuItem<R>>> = vec![
        Box::new(MenuItem::with_id(
            app,
            "show",
            tr("FreeX öffnen", "Open FreeX"),
            true,
            None::<&str>,
        )?),
        Box::new(PredefinedMenuItem::separator(app)?),
    ];
    for s in servers.iter().take(8) {
        let open = current.as_deref() == Some(s.id.as_str());
        items.push(Box::new(CheckMenuItem::with_id(
            app,
            format!("server:{}", s.id),
            &s.name,
            true,
            open,
            None::<&str>,
        )?));
    }
    items.push(Box::new(MenuItem::with_id(
        app,
        "servers",
        tr("Server auswählen …", "Choose a server …"),
        true,
        None::<&str>,
    )?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(MenuItem::with_id(
        app,
        "quit",
        tr("Beenden", "Quit"),
        true,
        None::<&str>,
    )?));
    let refs: Vec<&dyn IsMenuItem<R>> = items.iter().map(|i| i.as_ref()).collect();
    Menu::with_items(app, &refs)
}

/// Shows the current list of servers in the tray menu.
pub fn refresh_tray<R: Runtime>(app: &AppHandle<R>) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY), tray_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

/// Tray icon with the things that also work when a server is unreachable.
fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = tray_menu(app)?;
    let mut tray = TrayIconBuilder::with_id(TRAY)
        .tooltip("FreeX")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            match id {
                "show" => show_main(app),
                "servers" => {
                    show_main(app);
                    if let Some(state) = app.try_state::<AppState>() {
                        state.mpv.stop();
                        let _ = server::open_picker(app, &state, None);
                    }
                }
                "quit" => {
                    save_geometry(app);
                    app.exit(0);
                }
                _ => {
                    if let Some(server) = id.strip_prefix("server:") {
                        show_main(app);
                        if let Some(state) = app.try_state::<AppState>() {
                            state.mpv.stop();
                            // The picker checks the server first and shows a message if it is down.
                            let _ = server::open_picker(app, &state, Some(server));
                        }
                    }
                    // A check item toggles itself when clicked; the menu shows the truth again.
                    refresh_tray(app);
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Linux: WebKitGTK's DMA-BUF renderer shows an empty window with the proprietary NVIDIA driver;
/// there the older renderer is used (unless the user decided otherwise).
#[cfg(target_os = "linux")]
fn linux_environment() {
    const DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    if std::env::var_os(DMABUF).is_none()
        && std::path::Path::new("/proc/driver/nvidia/version").exists()
    {
        // Still single-threaded: nothing else reads the environment yet.
        std::env::set_var(DMABUF, "1");
    }
}
