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

fn main() {
    // Every app command gets its own permission ("allow-<command>"), so the pages of a FreeX
    // server can be granted exactly the commands they need and nothing else.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "servers_list",
            "server_probe",
            "server_open",
            "server_remove",
            "set_auto_connect",
            "desktop_info",
            "set_player",
            "show_servers",
            "mpv_play",
            "mpv_stop",
            "mpv_key",
        ]),
    ))
    .expect("failed to run tauri-build");
}
