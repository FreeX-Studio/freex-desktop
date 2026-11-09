<p align="center">
  <img src="docs/brand/freex-logo.svg" alt="FreeX" width="128">
</p>

<h1 align="center">FreeX Desktop</h1>

<p align="center">
  <strong>Your Media. Your Server. Your Freedom.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: LGPL-3.0" src="https://img.shields.io/badge/license-LGPL--3.0-blue"></a>
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078d4">
  <img alt="Tauri 2" src="https://img.shields.io/badge/tauri-2-ffc131">
</p>

FreeX Desktop is the Windows app for [FreeX Media Server](https://github.com/FreeX-Studio/freex-media-server). It connects
to your server and shows FreeX Web, the interface of the server itself, in its own window – so the app always
matches the version of the server it talks to. On top of the browser experience it adds:

- **Your servers in one place** – add a server by its address (`192.168.1.10`, `nas`, `https://media.example.org`),
  switch between several servers, and connect to the last one right away on start. A server on the same PC is
  found automatically.
- **Plays every file directly with mpv** – optionally, videos open in the bundled [mpv](https://mpv.io) player,
  which decodes practically every format itself. The server does not have to convert anything; your position
  and watch state are saved on the server as usual, and the playback shows up in the activity view.
- **Real full screen** for the built-in player, a tray icon, one running instance and remembered window size.

The family: **FreeX Media Server** (core), **FreeX Web** (built into the server), **FreeX Desktop** (this
repository), and the planned **FreeX iOS**, **FreeX Android** and **FreeX TV**.

## Install

Run `freex-desktop-<version>-setup.exe` from the build. It installs the app with mpv into
`C:\Program Files\FreeX Desktop`, adds FreeX to the start menu (and, if chosen, to the desktop) and installs the
Microsoft Edge WebView2 Runtime if it is missing (only needed on older Windows 10 installations).

Start FreeX, enter the address of your server and sign in. Sign-ins are kept per server, as in a browser.

| What | Where |
|---|---|
| Program | `C:\Program Files\FreeX Desktop` (with `mpv\`) |
| Settings (servers, player, window) | `%APPDATA%\de.mirranet.freex.desktop\settings.json` |
| Web view data (sign-ins, cache) | `%LOCALAPPDATA%\de.mirranet.freex.desktop` |

Updates install over the old version and keep all settings.

## Using it

- **Switch server:** account menu → *Switch server*, *Settings → FreeX Desktop*, or the tray icon (which also
  works when a server is unreachable).
- **Player:** *Settings → FreeX Desktop* (or the server picker). *FreeX player* is the player of FreeX Web with
  skip intro, up next and Watch Together. *mpv* plays every file directly in its own window; when an episode has
  played to the end, the next one starts after a short countdown. Watch Together always uses the FreeX player.
- mpv reads your own `mpv.conf` and scripts as usual (`%APPDATA%\mpv`). Keys: Space pause, ← → seek,
  F full screen, Q stop (the position is saved).

## Requirements

- Windows 10 (1809 or later) or Windows 11, 64-bit
- FreeX Media Server 0.14.0 or newer (for the mpv player; older servers work with the FreeX player)

## Building

The app is built with [Tauri 2](https://tauri.app) (Rust) and cross-compiled on Linux, so it fits into the same
Jenkins as the server:

```bash
sudo apt install build-essential gcc-mingw-w64-x86-64 nsis curl unzip
rustup target add x86_64-pc-windows-gnu
windows/build.sh            # dist/freex-desktop-<version>-setup.exe
windows/build.sh --app      # only dist/freex-desktop.exe
```

The script downloads mpv and Microsoft's WebView2 bootstrapper once into `~/.cache/freex-build`; set `MPV_SHA256`
and `WEBVIEW2_SHA256` to pin them. The `Jenkinsfile` runs the script and archives both files. A distribution
toolchain without the Windows standard library but with `rust-src` also works (the script then builds the
standard library from source).

### Project structure

| Path | |
|---|---|
| `src-tauri/src/main.rs` | Window, tray icon, single instance, window geometry |
| `src-tauri/src/server.rs` | Finding a server, opening it, the commands its pages may call |
| `src-tauri/src/mpv.rs` | Playback in mpv over its JSON IPC |
| `src-tauri/src/store.rs` | `settings.json` |
| `ui/` | The server picker (plain HTML, CSS and JavaScript) |
| `windows/` | Installer (NSIS) and build script |

### How it works

The window starts with the bundled picker. When a server is opened, the app grants the pages of exactly that
origin a small capability at runtime – app info, player choice, back to the picker, mpv, window full screen – and
loads FreeX Web from the server. FreeX Web notices the app (`window.__TAURI__`, see `desktop.js` in the server)
and adds the desktop features. Links to other sites open in the default browser.

For mpv, FreeX Web announces the playback with `POST /api/items/{id}/playback` and `"player": "external"` (the
server then always plans direct play) and hands the stream URL to the app. The app only accepts URLs of the
page's own server, starts mpv with a JSON IPC pipe, sends the sign-in token over that pipe (never on the command
line), and reports the position back as `freex-mpv` events, which FreeX Web saves on the server.

## License

FreeX Desktop is free software under the [GNU Lesser General Public License v3.0](LICENSE). The installer bundles
mpv (GPL v2+), see [`windows/THIRD-PARTY.txt`](windows/THIRD-PARTY.txt). The picker uses the font Manrope (SIL
Open Font License).
